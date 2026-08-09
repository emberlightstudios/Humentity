//! bevy_mod_physx hasn't been updated in a while.
//! You'll have to update it to use the latest version of bevy before this will work

mod shared;

use std::f32::consts::PI;

use bevy::{
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use bevy_mod_physx::prelude::{self as bpx, *};
use humentity::prelude::*;
use shared::setup_app;

fn main() {
    let mut app = setup_app();

    app.add_plugins(PhysicsPlugins.set(PhysicsCore::default()))
        .insert_resource(DebugRenderSettings {
            ..DebugRenderSettings::enable()
        })
        .add_systems(Startup, floor)
        .add_systems(Update, add_human.run_if(resource_added::<HumentityAssetsReady>))
        .add_systems(Update, (toggle, setup_graph, start_clip))
        .run();
}

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    mut hitbox: Single<&mut PhysxCharacterColliders<HitboxCollider>>,
    mut ragdoll: Single<&mut PhysxCharacterColliders<RagdollCollider>>,
    mut human: Single<(&SkinnedMesh, &mut AnimationPlayer)>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut bones: Query<(&mut Transform, Option<&ChildOf>), With<SkeletalBone>>,
) {
    if input.just_pressed(KeyCode::Space) {
        let ragdoll_active = !ragdoll
            .bones_subset
            .as_ref()
            .map_or(false, |v| v.is_empty());

        if ragdoll_active {
            ragdoll.bones_subset = Some(vec![]);
            hitbox.bones_subset = None;

            let ((skm, mut_player)) = human.into_inner();
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

            player.play(controller.0).repeat();
        } else {
            player.stop(controller.0);
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
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {
    let template_handle = template_assets.add(CharacterTemplate::new([]));

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
        skeleton_lod: 0,
    });

    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnimations { _clips: clips });

    commands.spawn((
        Transform::from_rotation(Quat::from_rotation_y(PI / 4.)),
        AnimationPlayer::default(),
        CharacterShape(shape_assets.add(template_handle)),
        HelperVertexPositions::default(),
        PhysxCharacterColliders::<HitboxCollider>::new(hitbox_filter, None),
        children![(CharacterPart { mesh: basemesh, skeleton_lod: 0 }, MeshMaterial3d(mat),)],
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
    ragdoll: Single<&PhysxCharacterColliders<RagdollCollider>>,
    mut anim: Single<(&mut AnimationPlayer, &AnimationController)>,
    mut started: Local<bool>,
) {
    let ragdoll_active = !ragdoll
        .bones_subset
        .as_ref()
        .map_or(false, |v| v.is_empty());
    if ragdoll_active {
        *started = false;
        return;
    }
    let index = anim.1.0;
    if !*started {
        anim.0.play(index).repeat();
        *started = true;
    }
}
