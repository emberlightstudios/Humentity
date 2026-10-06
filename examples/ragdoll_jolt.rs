mod shared;

use bevy::prelude::*;
use bevy_jolt::prelude::*;
use humentity::prelude::*;
use shared::setup_app;

const RAGDOLL_TEAM: u16 = 3;
const WORLD_TEAM: u16 = 0;
const CHARACTER_TEAM: u16 = 1;

fn main() {
    let mut app = setup_app();

    // Two teams for the demo table: world (0) and ragdoll (3). Everything
    // collides with everything by default; carve out nothing here.
    let collision_layers = CollisionLayers::new(4);

    app.add_plugins((
        JoltPlugin::new().with_collision_layers(collision_layers),
        JoltDebugPlugin,
    ))
    .add_systems(Startup, (floor, spawn_ui))
    .add_systems(
        Update,
        (
            add_human.run_if(resource_exists::<HumentityAssetsReady>),
            toggle,
            sleep_ragdoll,
            setup_graph,
            start_clip,
            dump_joint_angles,
        ),
    )
    .run();
}

/// Prints every ragdoll joint's live angle vs its baked rest pose, with the
/// configured limits alongside. Press J while the ragdoll is active: joints
/// far from rest (or past their limits) point at wrong axes or missing
/// damping. Example line:
/// `knee.L live 42.3° rest ±(swing 85.9°, twist 17.2°) hinge [-137.5°, 0.0°]`.
fn dump_joint_angles(
    input: Res<ButtonInput<KeyCode>>,
    joint_query: Query<(&Name, &JointRestPose, &JoltJoint)>,
    transform_query: Query<&Transform>,
    character_query: Query<&CharacterColliders>,
) {
    if !input.just_pressed(KeyCode::KeyJ) {
        return;
    }
    let Ok(colliders) = character_query.single() else {
        return;
    };
    for &joint_entity in &colliders.joint_entities {
        let Ok((joint_name, rest_pose, _joint)) = joint_query.get(joint_entity) else {
            continue;
        };
        let Ok(parent_transform) = transform_query.get(rest_pose.parent_entity) else {
            continue;
        };
        let Ok(child_transform) = transform_query.get(rest_pose.child_entity) else {
            continue;
        };
        let live_degrees = rest_pose.angle_from_rest_degrees(
            parent_transform.rotation,
            child_transform.rotation,
        );
        let Some(&bone) = colliders.joint_bone.get(&joint_entity) else {
            continue;
        };
        let limit = default_joint_limit(bone);
        println!(
            "{} live {:>6.1}° from rest | swing {:>5.1}° twist {:>5.1}° hinge [{:>6.1}°, {:>5.1}°]",
            joint_name,
            live_degrees,
            limit.swing.to_degrees(),
            limit.twist.to_degrees(),
            limit.angle_min.to_degrees(),
            limit.angle_max.to_degrees(),
        );
    }
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
        RagdollCollisionLayers::new(
            RAGDOLL_TEAM,
            (1 << WORLD_TEAM) | (1 << CHARACTER_TEAM) | (1 << RAGDOLL_TEAM),
        ),
        RagdollMobility(1.0),
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
    mut collider_data: Query<&mut JoltLinearVelocity>,
    mut commands: Commands,
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
        _ => {
            // Toggle ragdoll on
            *ragdoll = CharacterRagdoll::Full;

            // Stop animation so it doesn't compete with physics
            player.stop(clip_index);

            // Give each collider a small nudge so the collapse is interesting.
            // Varies with time so each activation is unique.
            for (bone, &collider_entity) in colliders.collider_entities.iter() {
                if let Ok(mut velocity) = collider_data.get_mut(collider_entity) {
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
                    velocity.linear_velocity += dir * 2.0;
                }
            }
        }
    }
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("SPACE: toggle full ragdoll (jolt) | J: dump joint angles"),
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

#[derive(Component)]
struct RagdollSleepTimer(Timer);

impl Default for RagdollSleepTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(2., TimerMode::Once))
    }
}

fn sleep_ragdoll(
    time: Res<Time>,
    mut commands: Commands,
    mut timers: Query<(Entity, &mut RagdollSleepTimer)>,
    changed_ragdolls: Query<
        (Entity, &CharacterRagdoll, &CharacterColliders),
        Changed<CharacterRagdoll>,
    >,
    mut sleeping_flags: Query<&mut JoltSleeping>,
) {
    for (entity, ragdoll, colliders) in changed_ragdolls.iter() {
        match ragdoll {
            CharacterRagdoll::Full | CharacterRagdoll::Partial(_) => {
                commands.entity(entity).insert(RagdollSleepTimer::default());
            }
            CharacterRagdoll::None => {
                commands.entity(entity).remove::<RagdollSleepTimer>();
                for &collider_entity in colliders.collider_entities.values() {
                    // Wake anything the sleep timer parked; harmless on awake bodies.
                    if let Ok(mut sleeping) = sleeping_flags.get_mut(collider_entity) {
                        sleeping.sleeping = false;
                    }
                }
            }
        }
    }

    for (entity, mut timer) in timers.iter_mut() {
        timer.0.tick(time.delta());
        if timer.0.just_finished() {
            commands.trigger(DisablePhysics { character: entity });
            commands.entity(entity).remove::<RagdollSleepTimer>();
        }
    }
}

fn floor(mut commands: Commands) {
    commands.spawn((
        JoltShape::box_shape(Vec3::new(100.0, 0.1, 100.0)),
        JoltBody::fixed(WORLD_TEAM),
        Transform::IDENTITY,
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
