mod shared;

use std::f32::consts::PI;

use bevy::{
    diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use bevy_rapier3d::prelude::*;
use humentity::prelude::*;
use shared::{setup_app, CharacterPart};

fn main() {
    let mut app = setup_app();

    app.add_plugins((
        RapierPhysicsPlugin::<NoUserData>::default(),
        RapierDebugRenderPlugin::default(),
        FrameTimeDiagnosticsPlugin::default(),
        LogDiagnosticsPlugin::default(),
    ))
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
    human: Single<(&CharacterShape, &SkinnedMesh)>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut bones: Query<(&mut Transform, Option<&ChildOf>), With<SkeletalBone>>,
    mut players: Query<&mut AnimationPlayer>,
    controllers: Query<&AnimationController>,
    character: Single<(Entity, &mut CharacterColliders, Option<&Ragdoll>), With<CharacterShape>>,
    mut commands: Commands,
) {
    let (entity, mut colliders, ragdoll) = character.into_inner();
    if input.just_pressed(KeyCode::Space) {
        if ragdoll.is_some() {
            commands.entity(entity).remove::<Ragdoll>();
            colliders.bones_subset = None;

            // Reset joints back to bindpose
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

            commands.entity(entity).insert(Ragdoll {
                sleep_timer: Some(Timer::from_seconds(3.0, TimerMode::Once)),
            });
            colliders.bones_subset = Some(vec![]);
        }
    }
}

fn floor(
    mut commands: Commands,
) {
    commands.spawn((
        Collider::cuboid(100.0, 0.1, 100.0),
        CollisionGroups::new(
            Group::GROUP_2,
            Group::all(),
        ),
        Friction::new(0.5),
        Restitution::new(0.0),
        RigidBody::Fixed,
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
        CharacterColliders::new(None),

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
    ragdoll: Single<Option<&Ragdoll>, With<CharacterColliders>>,
    mut anim: Single<(&mut AnimationPlayer, &AnimationController)>,
    mut started: Local<bool>,
) {
    if ragdoll.is_some() {
        *started = false;
        return;
    }
    let index = anim.1.0;
    if !*started {
        anim.0.play(index).repeat();
        *started = true;
    }
}
