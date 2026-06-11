mod shared;

use std::f32::consts::PI;

use avian3d::prelude::*;
use bevy::{
    diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use humentity::prelude::*;
use shared::{setup_app, CharacterPart};

fn main() {
    let mut app = setup_app();

    app
        .add_plugins((
            PhysicsPlugins::default(),
            PhysicsDebugPlugin,
            //FrameTimeDiagnosticsPlugin::default(),
            //LogDiagnosticsPlugin::default(),
        ))
        .add_systems(Startup, floor)
        .add_observer(add_human)
        .add_systems(
            Update,
            (
                toggle,
                setup_graph,
                start_clip,
                auto_sleep_ragdoll,
            ),
        )
        .run();
}

#[derive(Component)]
struct SleepTimer(Timer);

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    related: Single<&RelatedEntities>,
    character: Single<(Entity, &mut CharacterRagdoll, &mut CharacterColliders)>,
    human: Single<(&CharacterShape, &SkinnedMesh)>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut bones: Query<(&mut Transform, Option<&ChildOf>), With<SkeletalBone>>,
    mut players: Query<&mut AnimationPlayer>,
    controllers: Query<&AnimationController>,
) {
    let (entity, mut ragdoll, mut colliders) = character.into_inner();
    if input.just_pressed(KeyCode::Space) {
        if matches!(&*ragdoll, CharacterRagdoll::Full) {
            *ragdoll = CharacterRagdoll::None;
            colliders.bones_subset = None;
            commands.entity(entity).remove::<SleepTimer>();

            let (_, skm) = *human;
            if let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes) {
                let model_bind_poses: Vec<Mat4> = inv_bindposes
                    .iter()
                    .map(|m| m.inverse())
                    .collect();

                for (i, &joint_entity) in skm.joints.iter().enumerate() {
                    if let Ok((mut transform, child_of)) = bones.get_mut(joint_entity) {
                        if let Some(parent) = child_of
                            && let Some(parent_idx) = skm.joints
                                .iter()
                                .position(|&e| e == parent.parent())
                        {
                            *transform = Transform::from_matrix(
                                model_bind_poses[parent_idx].inverse()
                                    * model_bind_poses[i],
                            );
                        } else {
                            *transform = Transform::from_matrix(model_bind_poses[i]);
                        }
                    }
                }
            }

            if let Ok(mut player) = players.get_mut(related.rig)
                && let Ok(controller) = controllers.get(related.rig)
            {
                player.play(controller.0).repeat();
            }
        } else {
            if let Ok(mut player) = players.get_mut(related.rig)
                && let Ok(controller) = controllers.get(related.rig)
            {
                player.stop(controller.0);
            }

            *ragdoll = CharacterRagdoll::Full;
            colliders.bones_subset = Some(vec![]);
            commands.entity(entity).insert(SleepTimer(Timer::from_seconds(3.0, TimerMode::Once)));
        }
    }
}

fn floor(
    mut commands: Commands,
) {
    commands.spawn((
        Collider::cuboid(100.0, 0.1, 100.0),
        Friction::new(0.5),
        Restitution::new(0.5),
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

    let template_handle = template_assets.add(CharacterTemplate::new(
        [],
        RigType::Default,
    ));

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
        CharacterRagdoll::None,
        CharacterColliders::new(None),
        RagdollMobility(0.1),
        RagdollDensity(100.0),
        RagdollDamping(5.0),
        children![(
            CharacterPart(basemesh),
            MeshMaterial3d(mat),
        )]
    ));
}

#[derive(Resource)]
struct RetargetedAnimations {
    _clips: Handle<RetargetedAnimationAsset>,
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
    ragdoll: Single<&CharacterRagdoll>,
    mut anim: Single<(&mut AnimationPlayer, &AnimationController)>,
    mut started: Local<bool>,
) {
    if !matches!(*ragdoll, CharacterRagdoll::None) {
        *started = false;
        return;
    }
    let index = anim.1.0;
    if !*started {
        anim.0.play(index).repeat();
        *started = true;
    }
}

fn auto_sleep_ragdoll(
    time: Res<Time>,
    mut characters: Query<(Entity, &mut SleepTimer, &CharacterColliders), With<CharacterRagdoll>>,
    mut sleep_query: Query<&mut SleepThreshold>,
    mut linear_velocity_query: Query<&mut LinearVelocity>,
    mut angular_velocity_query: Query<&mut AngularVelocity>,
    mut commands: Commands,
) {
    return;
    for (entity, mut timer, colliders) in characters.iter_mut() {
        timer.0.tick(time.delta());
        if !timer.0.just_finished() {
            continue;
        }
        commands.entity(entity).remove::<SleepTimer>();
        for (_, &collider_entity) in colliders.collider_entities.iter() {
            if let Ok(mut threshold) = sleep_query.get_mut(collider_entity) {
                threshold.linear = 100.0;
                threshold.angular = 100.0;
            }
            if let Ok(mut linear_velocity) = linear_velocity_query.get_mut(collider_entity) {
                linear_velocity.0 = Vec3::ZERO;
            }
            if let Ok(mut angular_velocity) = angular_velocity_query.get_mut(collider_entity) {
                angular_velocity.0 = Vec3::ZERO;
            }
        }
    }
}