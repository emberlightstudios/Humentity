// Demonstrates a partial ragdoll (on the arms) attached to a set of kinematic colliders
// It's a bit jittery
mod shared;

use std::f32::consts::PI;

use avian3d::prelude::*;
use bevy::{
    animation::AnimationTargetId,
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use humentity::prelude::*;
use shared::setup_app;

const RAGDOLL_LAYER: u32 = 1 << 3;
const WORLD_LAYER: u32 = 1 << 0;
const CHARACTER_LAYER: u32 = 1 << 1;

fn main() {
    let mut app = setup_app();

    app.add_plugins((PhysicsPlugins::default(), PhysicsDebugPlugin))
        //.insert_resource(SubstepCount(10))
        .add_systems(Startup, (floor, spawn_ui))
        .add_observer(add_human)
        .add_systems(Update, (toggle, setup_graph, start_clip, oscillate))
        .run();
}

#[derive(Component)]
struct AnimationController(AnimationNodeIndex);

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    human: Single<(&CharacterShape, &SkinnedMesh)>,
    character: Single<(
        &mut CharacterRagdoll,
        &mut CharacterColliders,
        &mut AnimationPlayer,
        &AnimationGraphHandle,
        &AnimationController,
    )>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut bones: Query<(&mut Transform, Option<&ChildOf>), With<SkeletalBone>>,
) {
    let (mut ragdoll, mut colliders, mut player, graph, controller) = character.into_inner();

    if input.just_pressed(KeyCode::Space) {
        info!("PRESSED");
        if matches!(&*ragdoll, CharacterRagdoll::Partial(_)) {
            *ragdoll = CharacterRagdoll::None;
            colliders.bones_subset = None;

            if let Some(mut graph) = graphs.get_mut(&graph.0) {
                if let Some(node) = graph.graph.node_weight_mut(controller.0) {
                    node.mask = 0;
                }
            }

            let (_, skm) = *human;
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

            player.stop(controller.0);
            player.play(controller.0).repeat();
        } else {
            *ragdoll = CharacterRagdoll::Partial(vec![
                ColliderBone::UpperRightArm,
                ColliderBone::UpperLeftArm,
                ColliderBone::LowerRightArm,
                ColliderBone::LowerLeftArm,
                ColliderBone::RightHand,
                ColliderBone::LeftHand,
            ]);
            colliders.bones_subset = None;

            if let Some(mut graph) = graphs.get_mut(&graph.0) {
                if let Some(node) = graph.graph.node_weight_mut(controller.0) {
                    node.mask = 1;
                }
            }
        }
    }
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("Press SPACEBAR to toggle ragdoll arms"),
        TextFont::from_font_size(24.0),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.),
            right: Val::Px(12.),
            ..default()
        },
    ));
}

fn floor(mut commands: Commands) {
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
    let template_handle = template_assets.add(CharacterTemplate::new([]));

    let basemesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    let texture = asset_server.load::<Image>("skin_textures/albedo/young_caucasian_female.png");
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(texture),
        ..default()
    });

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh.clone(),
        template_handle: template_handle.clone(),
        lod: 0,
    });

    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnimations { _clips: clips });

    commands.spawn((
        Transform::from_rotation(Quat::from_rotation_y(PI / 4.))
            .with_translation(Vec3::new(1., 0., 0.)),
        AnimationPlayer::default(),
        CharacterShape(shape_assets.add(template_handle)),
        CharacterRagdoll::None,
        CharacterColliders::new(Some(vec![
            ColliderBone::Chest,
            ColliderBone::Pelvis,
            ColliderBone::UpperRightLeg,
            ColliderBone::UpperLeftLeg,
            ColliderBone::UpperRightArm,
            ColliderBone::UpperLeftArm,
            ColliderBone::LowerRightArm,
            ColliderBone::LowerLeftArm,
            ColliderBone::RightHand,
            ColliderBone::LeftHand,
        ])),
        RagdollCollisionLayers(CollisionLayers::new(
            RAGDOLL_LAYER,
            WORLD_LAYER | CHARACTER_LAYER | RAGDOLL_LAYER,
        )),
        RagdollMobility(1.0),
        RagdollDensity(10.0),
        RagdollDamping(25.0),
        children![(CharacterPart { mesh: basemesh, lod: 0 }, MeshMaterial3d(mat),)],
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
    mut character_player: Query<Entity, (With<AnimationPlayer>, Without<AnimationGraphHandle>)>,
    skeleton_bones: Query<(&Name, &AnimationTargetId), With<SkeletalBone>>,
) {
    let Ok(entity) = character_player.single_mut() else {
        return;
    };
    let Some((_id, clips_map)) = retargeted_clips.iter().next() else {
        return;
    };
    let clip_handle = clips_map.clips.get("Idle-loop").unwrap();
    let (mut graph, index) = AnimationGraph::from_clip(clip_handle.clone());

    let arm_bone_names = [
        "upperarm01.L",
        "upperarm01.R",
        "lowerarm01.L",
        "lowerarm01.R",
        "wrist.L",
        "wrist.R",
    ];
    for (name, target_id) in skeleton_bones.iter() {
        if arm_bone_names.contains(&name.as_str()) {
            graph.mask_groups.insert(*target_id, 1);
        }
    }

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

fn oscillate(time: Res<Time>, mut query: Query<&mut Transform, With<CharacterRagdoll>>) {
    let amplitude = 0.15;
    let frequency = 0.5;
    let rot_speed = 0.5;
    for mut transform in query.iter_mut() {
        transform.translation.z =
            (time.elapsed_secs() * frequency * std::f32::consts::TAU).sin() * amplitude;
        transform.rotate_y(time.delta_secs() * rot_speed);
    }
}
