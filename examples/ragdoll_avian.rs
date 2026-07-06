mod shared;

use avian3d::prelude::*;
use bevy::{
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use humentity::prelude::*;
use shared::{setup_app, CharacterPart};

const RAGDOLL_LAYER: u32 = 1 << 3;
const WORLD_LAYER: u32 = 1 << 0;
const CHARACTER_LAYER: u32 = 1 << 1;

fn main() {
    let mut app = setup_app();

    app
        .add_plugins((
            PhysicsPlugins::default(),
            PhysicsDebugPlugin,
        ))
        .insert_resource(SubstepCount(10))
        .add_systems(Startup, (floor, spawn_ui))
        .add_observer(add_human)
        .add_systems(
            Update,
            (
                toggle,
                toggle_sync,
                sleep_ragdoll,
                setup_graph,
                start_clip,
            ),
        )
        .run();
}

#[derive(Component)]
struct AnimationController(AnimationNodeIndex);

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut character: Query<(
        Entity,
        &mut CharacterRagdoll,
        &mut CharacterColliders,
        &SkeletonEntities,
        &SkinnedMesh,
    )>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut bones: Query<(&mut Transform, Option<&ChildOf>), With<SkeletalBone>>,
    mut players: Query<&mut AnimationPlayer>,
    mut collider_data: Query<&mut LinearVelocity>,
) {
    if !input.just_pressed(KeyCode::Space) {
        return;
    }

    let Ok((_entity, mut ragdoll, colliders, related, skm)) = character.single_mut()
    else {
        return;
    };

    match &*ragdoll {
        CharacterRagdoll::Full => {
            // Toggle ragdoll off
            *ragdoll = CharacterRagdoll::None;

            // Reset bones to bind poses
            if let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes) {
                let model_bind_poses: Vec<Mat4> =
                    inv_bindposes.iter().map(|m| m.inverse()).collect();

                for (i, &joint_entity) in skm.joints.iter().enumerate() {
                    if let Ok((mut transform, child_of)) = bones.get_mut(joint_entity) {
                        if let Some(parent) = child_of
                            && let Some(parent_idx) =
                                skm.joints.iter().position(|&e| e == parent.parent())
                        {
                            *transform = Transform::from_matrix(
                                model_bind_poses[parent_idx].inverse() * model_bind_poses[i],
                            );
                        } else {
                            *transform = Transform::from_matrix(model_bind_poses[i]);
                        }
                    }
                }
            }

            if let Ok(mut player) = players.get_mut(related.rig) {
                player.stop(AnimationNodeIndex::new(0));
                player.play(AnimationNodeIndex::new(0)).repeat();
            }
        }
        _ => {
            // Toggle ragdoll on
            *ragdoll = CharacterRagdoll::Full;

            // Stop animation so it doesn't compete with physics
            if let Ok(mut player) = players.get_mut(related.rig) {
                player.stop(AnimationNodeIndex::new(0));
            }

            // Give each collider a small nudge so the collapse is interesting.
            // Varies with time so each activation is unique.
            for (bone, &collider_entity) in colliders.collider_entities.iter() {
                if let Ok(mut vel) = collider_data.get_mut(collider_entity) {
                    let i = *bone as usize;
                    let t = time.elapsed_secs();
                    let seed = (t * 100.0).floor() / 100.0;
                    let h = ((i as f32 + seed) * 0x9e3779b9u32 as f32).sin();
                    let dir = Vec3::new(
                        ((h * 43758.5453).fract() - 0.5) * 2.0,
                        ((h * 27118.3129).fract()) * 0.5,
                        ((h * 30903.5511).fract() - 0.5) * 2.0,
                    )
                    .normalize_or_zero();
                    vel.0 += dir * 2.0;
                }
            }
        }
    }
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("SPACE: toggle full ragdoll\nC: toggle collider sync"),
        TextFont::from_font_size(24.0),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.),
            right: Val::Px(12.),
            ..default()
        },
    ));
}

#[derive(Component)]
struct RagdollSleepTimer(Timer);

fn sleep_ragdoll(
    time: Res<Time>,
    mut commands: Commands,
    mut timers: Query<(Entity, &mut RagdollSleepTimer, &CharacterColliders)>,
    changed_ragdolls: Query<(Entity, &CharacterRagdoll), Changed<CharacterRagdoll>>,
) {
    for (entity, ragdoll) in changed_ragdolls.iter() {
        match ragdoll {
            CharacterRagdoll::Full | CharacterRagdoll::Partial(_) => {
                commands.entity(entity).insert(RagdollSleepTimer(
                    Timer::from_seconds(2.0, TimerMode::Once),
                ));
            }
            CharacterRagdoll::None => {
                commands.entity(entity).remove::<RagdollSleepTimer>();
            }
        }
    }

    for (entity, mut timer, colliders) in timers.iter_mut() {
        timer.0.tick(time.delta());
        if timer.0.just_finished() {
            for &collider_entity in colliders.collider_entities.values() {
                commands.entity(collider_entity).insert(Sleeping);
            }
            commands.entity(entity).remove::<RagdollSleepTimer>();
        }
    }
}

fn toggle_sync(
    input: Res<ButtonInput<KeyCode>>,
    mut characters: Query<&mut ColliderSync>,
) {
    if input.just_pressed(KeyCode::KeyC) {
        for mut sync in characters.iter_mut() {
            sync.0 = !sync.0;
        }
    }
}

fn floor(mut commands: Commands) {
    commands.spawn((
        Collider::cuboid(100.0, 0.1, 100.0),
        Friction::new(0.5),
        Restitution::new(0.1),
        RigidBody::Static,
        Transform::IDENTITY,
    ));
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
    let template_handle = template_assets.add(CharacterTemplate::new([], RigType::Default));

    let basemesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    let texture =
        asset_server.load::<Image>("skin_textures/albedo/young_caucasian_female.png");
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
        Transform::from_xyz(0.0, 0.0, 0.0),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(
            template_handle,
            MorphTargets::default(),
        ))),
        CharacterRagdoll::None,
        CharacterColliders::new(None),
        RagdollCollisionLayers(CollisionLayers::new(
            RAGDOLL_LAYER,
            WORLD_LAYER | CHARACTER_LAYER | RAGDOLL_LAYER,
        )),
        RagdollMobility(1.0),
        // Ragdolls tend to twitch without higher density settings in my findings
        RagdollDensity(10.0),
        RagdollDamping::default(),
        RagdollCompliance(0.2),
        children![(CharacterPart(basemesh), MeshMaterial3d(mat))],
    ));
}

#[derive(Resource)]
struct RetargetedAnimations {
    _clips: Handle<RetargetedAnimationAsset>,
}

fn setup_graph(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    retargeted_clips: Res<Assets<RetargetedAnimationAsset>>,
    mut character_player: Query<
        (Entity, &mut AnimationPlayer),
        Without<AnimationGraphHandle>,
    >,
) {
    let Ok((entity, _player)) = character_player.single_mut() else {
        return;
    };
    let Some((_id, clips_map)) = retargeted_clips.iter().next() else {
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
    anim: Single<(&mut AnimationPlayer, &AnimationController)>,
    mut started: Local<bool>,
) {
    if !*started {
        let (mut player, controller) = anim.into_inner();
        player.play(controller.0).repeat();
        *started = true;
    }
}
