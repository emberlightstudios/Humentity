//! Box3D ragdoll demo: mirrors `ragdoll_jolt.rs` on the `boxddd` backend.
//!
//! A human idles under animation as kinematic hitboxes; SPACE flips every
//! collider part to dynamic (`CharacterRagdoll::Full`) so the body collapses
//! onto the floor. SPACE again snaps the bones back to the bind pose and
//! resumes the idle clip.

mod shared;

use bevy::prelude::*;
use bevy_boxddd::prelude::*;
use humentity::prelude::*;
use shared::setup_app;

const RAGDOLL_CATEGORY: u16 = 3;
const WORLD_CATEGORY: u16 = 0;
const CHARACTER_CATEGORY: u16 = 1;
const ALL_CATEGORIES: u32 =
    (1 << WORLD_CATEGORY) | (1 << CHARACTER_CATEGORY) | (1 << RAGDOLL_CATEGORY);

fn main() {
    let mut app = setup_app();

    app.add_plugins((BoxdddPhysicsPlugin::new(boxddd::FoundationConfig::default()),))
        .add_systems(Startup, (floor, spawn_ui))
        .add_systems(
            Update,
            (
                add_human.run_if(resource_exists::<HumentityAssetsReady>),
                toggle,
                setup_graph,
                start_clip,
            ),
        )
        .run();
}

#[derive(Component)]
struct AnimationController(AnimationNodeIndex);

fn add_human(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    let template_handle = template_assets.add(CharacterTemplate::new([]));

    let basemesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: MeshBuildLod::Cpu(0),
    });

    let clips = asset_server.load::<RotationOnlyAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RotationOnlyAnimations { _clips: clips });

    commands.spawn((
        Transform::from_xyz(0.0, 0.0, 0.0),
        AnimationPlayer::default(),
        CharacterShape(shape_assets.add(template_handle)),
        BuildCpuSkeleton,
        RootOnlyRetargeting,
        CharacterRagdoll::None,
        CharacterColliders::new(None),
        RagdollCollisionLayers::new(RAGDOLL_CATEGORY, ALL_CATEGORIES),
        children![(CharacterPart {
            mesh: basemesh,
            skeleton_lod: 0
        },)],
    ));
}

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut character: Query<(
        Entity,
        &mut CharacterRagdoll,
        &mut CharacterColliders,
        &mut AnimationPlayer,
        Option<&AnimationController>,
    )>,
    body_ids: Query<&BoxdddBody>,
    mut commands: Commands,
    mut physics_context: NonSendMut<BoxdddPhysicsContext>,
) {
    if !input.just_pressed(KeyCode::Space) {
        return;
    }

    let Ok((entity, mut ragdoll, colliders, mut player, controller)) = character.single_mut()
    else {
        return;
    };

    let clip_index = controller.map_or_else(|| AnimationNodeIndex::new(0), |c| c.0);

    match &*ragdoll {
        CharacterRagdoll::Full => {
            // Toggle ragdoll off: snap bones back to the fitted bind pose so
            // stale ragdoll translations don't survive under the rotation-only
            // clip, then restart the clip.
            *ragdoll = CharacterRagdoll::None;

            commands.trigger(ResetToBindPose(entity));
            player.stop(clip_index);
            player.play(clip_index).repeat();
        }
        CharacterRagdoll::None => {
            // Toggle ragdoll on
            *ragdoll = CharacterRagdoll::Full;

            // Stop animation so it doesn't compete with physics
            player.stop(clip_index);

            // Give each collider a small nudge so the collapse is interesting.
            // Varies with time so each activation is unique.
            let Some(world) = physics_context.world_mut() else {
                return;
            };
            for (bone, &collider_entity) in colliders.collider_entities.iter() {
                let Ok(body) = body_ids.get(collider_entity) else {
                    continue;
                };
                let bone_index = *bone as usize;
                let elapsed = time.elapsed_secs();
                let seed = (elapsed * 100.0).floor() / 100.0;
                let hash = ((bone_index as f32 + seed) * 0x9e3779b9u32 as f32).sin();
                let dir = Vec3::new(
                    ((hash * 43_758.547).fract() - 0.5) * 2.0,
                    ((hash * 27_118.313).fract()) * 0.5,
                    ((hash * 30_903.55).fract() - 0.5) * 2.0,
                )
                .normalize_or_zero();
                let _ = world.apply_linear_impulse_to_center(body.0, boxddd_vec3(dir * 2.0), true);
            }
        }
    }
}

fn boxddd_vec3(bevy_vector: Vec3) -> boxddd::Vec3 {
    boxddd::Vec3::new(bevy_vector.x, bevy_vector.y, bevy_vector.z)
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("SPACE: toggle full ragdoll (boxddd)"),
        TextLayout::justify(Justify::Right),
        TextFont::from_font_size(24.0),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.),
            right: Val::Px(12.),
            ..default()
        },
    ));
}

fn floor(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(200.0, 0.2, 200.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.2, 0.2, 0.25))),
        Transform::IDENTITY,
        RigidBody::Static,
        Collider::cuboid(100.0, 0.1, 100.0),
    ));
}

#[derive(Resource)]
struct RotationOnlyAnimations {
    _clips: Handle<RotationOnlyAnimationAsset>,
}

fn setup_graph(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    rotation_only_clips: Res<Assets<RotationOnlyAnimationAsset>>,
    mut character_player: Query<(Entity, &mut AnimationPlayer), Without<AnimationGraphHandle>>,
) {
    let Ok((entity, _player)) = character_player.single_mut() else {
        return;
    };
    let Some((_id, clips_map)) = rotation_only_clips.iter().next() else {
        return;
    };
    let clip_handle = clips_map.clips.get("Idle-loop").unwrap();
    let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());

    commands.entity(entity).insert((
        AnimationController(index),
        AnimationGraphHandle(graphs.add(graph)),
    ));
}

fn start_clip(
    anim: Single<(&mut AnimationPlayer, &AnimationController), Added<AnimationController>>,
) {
    let (mut player, controller) = anim.into_inner();
    player.play(controller.0).repeat();
}
