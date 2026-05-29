mod shared;

use std::f32::consts::PI;

use bevy::{mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes}, prelude::*};
use bevy_mod_physx::prelude::{self as bpx, *};
use humentity::prelude::*;
use shared::{setup_app, CharacterPart};

fn main() {
    let mut app = setup_app();

    app.add_plugins(
        PhysicsPlugins.set(PhysicsCore::default()),
    )
    .add_systems(Startup, floor)
    .add_observer(add_human)
    .add_systems(
        Update,
        (
            toggle,
            setup_graph,
            start_clip,
        ),
    )
    .run();
}

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    related: Single<&RelatedEntities>,
    mut hitbox: Single<&mut CharacterColliders<HitboxCollider>>,
    mut ragdoll: Single<&mut CharacterColliders<RagdollCollider>>,
    human: Single<(&CharacterShape, &SkinnedMesh)>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut bones: Query<&mut Transform, With<SkeletalBone>>,
    mut players: Query<&mut AnimationPlayer>,
    controllers: Query<&AnimationController>,
) {
    if input.just_pressed(KeyCode::Space) {
        let ragdoll_active = !ragdoll
            .bones_subset
            .as_ref()
            .map_or(false, |v| v.is_empty());

        if ragdoll_active {
            ragdoll.bones_subset = Some(vec![]);
            hitbox.bones_subset = None;

            let (_, skm) = *human;
            if let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes)
                && let Ok(mut pelvis) = bones.get_mut(skm.joints[0])
            {
                pelvis.translation = Transform::from_matrix(inv_bindposes[0].inverse()).translation;
            }
        } else {
            if let Ok(mut player) = players.get_mut(related.rig)
                && let Ok(controller) = controllers.get(related.rig)
            {
                player.stop(controller.0);
                info!("STOP");
            }

            hitbox.bones_subset = Some(vec![]);
            ragdoll.bones_subset = None;
        }
    }
}

fn floor(
    mut commands: Commands,
    mut geometries: ResMut<Assets<Geometry>>,
    mut materials: ResMut<Assets<bpx::Material>>,
    mut physics: ResMut<Physics>,
) {
    commands.spawn((
        Shape {
            geometry: geometries.add(Plane3d::default()),
            material: materials.add(bpx::Material::new(&mut physics, 0.5, 0.5, 0.5)),
            ..Default::default()
        },
        RigidBody::Static,
        Transform::IDENTITY,
    ));
}

#[derive(Resource)]
struct RetargetedAnimations {
    _clips: Handle<RetargetedAnimationAsset>,
}

fn add_human(
    _trigger: On<MorphsReady>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {

    let template_handle = template_assets.add(CharacterTemplate::new(
        [],
        RigType::Default,
    ));

    let hitbox_filter = ShapeFilterData {
        simulation_filter_data: [1, 1, 0, 0],
        ..default()
    };

    let basemesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    let texture = asset_server.load::<Image>("skin_textures/albedo/young_caucasian_female.png");
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(texture),
        ..default()
    });

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh.clone(),
        template_handle: template_handle.clone(),
    });

    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnimations { _clips: clips });

    commands.spawn((
        Transform::from_rotation(Quat::from_rotation_y(PI / 4.)),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(template_handle, MorphTargets::default()))),
        CharacterColliders::<HitboxCollider>::new(hitbox_filter, None),
        children![(
            CharacterPart(basemesh),
            MeshMaterial3d(mat),
        )]
    ));
}

#[derive(Component)]
struct AnimationController(AnimationNodeIndex);

fn setup_graph(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    retargeted_clips: Res<Assets<RetargetedAnimationAsset>>,
    mut character_player: Query<(Entity, &mut AnimationPlayer), Without<AnimationGraphHandle>>,
) {
    let Ok((entity, _player)) = character_player.single_mut() else { return };
    let Some((_id, clips_map)) = retargeted_clips.iter().next() else { return };
    let clip_handle = clips_map.clips.get("Idle-loop").unwrap();
    let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());
    commands.entity(entity).insert((
        AnimationController(index),
        AnimationGraphHandle(graphs.add(graph)),
    ));
}

fn start_clip(
    ragdoll: Single<&CharacterColliders<RagdollCollider>>,
    mut anim: Single<(&mut AnimationPlayer, &AnimationController)>,
) {
    let ragdoll_active = !ragdoll
        .bones_subset
        .as_ref()
        .map_or(false, |v| v.is_empty());
    if ragdoll_active {
        return;
    }
    let index = anim.1.0;
    anim.0.play(index).repeat();
}
