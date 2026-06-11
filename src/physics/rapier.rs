use ahash::AHashMap;
use bevy::{
    math::{Quat, Vec3},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use bevy_rapier3d::prelude::*;

use crate::{
    morphs::MakeHumanMorphs,
    prelude::{BaseMesh, CharacterShape, CharacterShapeAsset, CharacterTemplate},
    rigs::{RigData, RigType, SkeletalBone},
};

use super::{
    get_collider_parent, ColliderBone, NeedsColliders, COLLIDERS, DEFAULT_RIG_COLLIDER_BONE_NAMES,
    HEAD_VERTICES, LEFT_FOOT_VERTICES, LEFT_HAND_VERTICES, LOWER_LEFT_ARM_VERTICES,
    LOWER_LEFT_LEG_VERTICES, LOWER_RIGHT_ARM_VERTICES, LOWER_RIGHT_LEG_VERTICES, PELVIS_VERTICES,
    RIGHT_FOOT_VERTICES, RIGHT_HAND_VERTICES, TORSO_VERTICES, UPPER_LEFT_ARM_VERTICES,
    UPPER_LEFT_LEG_VERTICES, UPPER_RIGHT_ARM_VERTICES, UPPER_RIGHT_LEG_VERTICES,
};

#[derive(Component, Default)]
pub struct CharacterColliders {
    pub bones_subset: Option<Vec<ColliderBone>>,
    pub(crate) collider_entities: AHashMap<ColliderBone, Entity>,
    pub(crate) bone_entities: AHashMap<ColliderBone, Entity>,
    pub(crate) joint_entities: Vec<Entity>,
}

impl CharacterColliders {
    pub fn new(bones_subset: Option<Vec<ColliderBone>>) -> Self {
        Self {
            bones_subset,
            collider_entities: AHashMap::default(),
            bone_entities: AHashMap::default(),
            joint_entities: Vec::new(),
        }
    }
}

/// Insert this component on the character entity to activate the ragdoll.
/// Remove it to return colliders to kinematic mode.
/// The optional timer auto-sleeps the ragdoll when it expires.
#[derive(Component)]
pub struct Ragdoll {
    pub sleep_timer: Option<Timer>,
}

#[derive(Component)]
pub(crate) struct ColliderOffset(pub(crate) Transform);

#[derive(Component)]
pub(crate) struct KinematicCollider;

#[derive(Component)]
#[relationship(relationship_target = ColliderList)]
pub(crate) struct ColliderForCharacter(pub(crate) Entity);

#[derive(Component)]
#[relationship_target(relationship = ColliderForCharacter)]
pub(crate) struct ColliderList(Vec<Entity>);

#[derive(Component)]
#[relationship(relationship_target = JointList)]
pub(crate) struct JointForCharacter(pub(crate) Entity);

#[derive(Component)]
#[relationship_target(relationship = JointForCharacter)]
pub(crate) struct JointList(Vec<Entity>);

pub(crate) fn mark_needs_colliders(
    mut commands: Commands,
    characters: Query<
        Entity,
        (
            Or<(Added<CharacterColliders>, Added<SkinnedMesh>)>,
            With<CharacterColliders>,
            With<SkinnedMesh>,
        ),
    >,
) {
    for entity in characters.iter() {
        commands.entity(entity).insert(NeedsColliders);
    }
}

pub(crate) fn spawn_colliders(
    mut commands: Commands,
    mut characters: Query<
        (
            Entity,
            &CharacterShape,
            &SkinnedMesh,
            &mut CharacterColliders,
        ),
        With<NeedsColliders>,
    >,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    basemesh: Res<BaseMesh>,
    mh_morphs: Res<MakeHumanMorphs>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    rig_data: Res<RigData>,
) {
    for (character_entity, character_shape, skm, mut colliders) in characters.iter_mut() {
        let Some(asset) = shape_assets.get(&character_shape.0) else {
            continue;
        };
        let Some(template) = templates.get(&asset.template) else {
            continue;
        };
        let rig_type = template.rig;
        let collider_bone_map = match rig_type {
            RigType::Default => DEFAULT_RIG_COLLIDER_BONE_NAMES,
            _ => continue,
        };

        let Ok(helpers) = template.get_helpers(
            &asset.template_morph_targets,
            &basemesh.vertices,
            &mh_morphs,
        ) else {
            continue;
        };
        let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes) else {
            continue;
        };
        let reference_rig = &rig_data[&template.rig].reference_rig;

        let bone_entities: AHashMap<&str, Entity> = reference_rig
            .bone_names
            .iter()
            .cloned()
            .zip(skm.joints.iter().cloned())
            .collect();

        let inv_bindposes_map: AHashMap<&str, Transform> = reference_rig
            .bone_names
            .iter()
            .cloned()
            .zip(inv_bindposes.iter().map(|m| Transform::from_matrix(*m)))
            .collect();

        let target_bones: Vec<ColliderBone> = match &colliders.bones_subset {
            Some(bones) if !bones.is_empty() => bones.clone(),
            Some(_) => vec![],
            None => COLLIDERS.to_vec(),
        };

        if target_bones.is_empty() {
            continue;
        }

        for &collider in &target_bones {
            let i_collider = COLLIDERS.iter().position(|&c| c == collider).unwrap();
            let (geometry, collider_to_model) =
                get_collider_geometry(collider, &helpers, i_collider, &inv_bindposes_map);

            let joint_name = collider_bone_map[i_collider];
            let model_to_joint = inv_bindposes_map[joint_name];
            let collider_to_joint = model_to_joint * collider_to_model;

            let collider_entity = commands
                .spawn((
                    RigidBody::KinematicPositionBased,
                    KinematicCollider,
                    collider,
                    geometry,
                    ColliderOffset(collider_to_joint),
                    ColliderMassProperties::Density(1.0),
                    CollisionGroups::new(Group::GROUP_1, Group::GROUP_2),
                    Transform::IDENTITY,
                    ColliderForCharacter(character_entity),
                ))
                .id();

            colliders
                .collider_entities
                .insert(collider, collider_entity);
            colliders
                .bone_entities
                .insert(collider, bone_entities[joint_name]);
        }

        commands.entity(character_entity).remove::<NeedsColliders>();
    }
}

pub(crate) fn sync_colliders(
    characters: Query<&CharacterColliders>,
    bones: Query<&GlobalTransform>,
    mut collider_data: Query<(&mut Transform, &ColliderOffset), With<KinematicCollider>>,
) {
    for colliders in characters.iter() {
        let target_bones: Vec<ColliderBone> = match &colliders.bones_subset {
            Some(bones) if !bones.is_empty() => bones.clone(),
            Some(_) => vec![],
            None => COLLIDERS.to_vec(),
        };
        for bone_type in target_bones {
            let Some(collider_entity) = colliders.collider_entities.get(&bone_type) else {
                continue;
            };
            let Ok((mut transform, offset)) = collider_data.get_mut(*collider_entity) else {
                continue;
            };
            let Some(bone_entity) = colliders.bone_entities.get(&bone_type) else {
                continue;
            };
            let Ok(joint_to_world) = bones.get(*bone_entity) else {
                continue;
            };
            *transform = Transform::from(*joint_to_world) * offset.0;
        }
    }
}

pub(crate) fn activate_ragdoll(
    mut commands: Commands,
    mut to_activate: Query<(Entity, &mut CharacterColliders), Added<Ragdoll>>,
    bones: Query<&GlobalTransform>,
    collider_offsets: Query<&ColliderOffset>,
) {
    for (character_entity, mut char_colliders) in to_activate.iter_mut() {
        for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
            commands
                .entity(collider_entity)
                .insert(RigidBody::Dynamic)
                .remove::<KinematicCollider>()
                .insert(Damping {
                    linear_damping: 2.0,
                    angular_damping: 2.0,
                });
        }

        let joints_to_spawn: Vec<(ColliderBone, Entity, Entity, Vec3, Vec3, Vec3, Quat)> =
            char_colliders
                .collider_entities
                .iter()
                .filter_map(|(bone, &child)| {
                    let parent_bone_type = get_collider_parent(*bone)?;
                    let parent = *char_colliders.collider_entities.get(&parent_bone_type)?;

                    let child_bone = *char_colliders.bone_entities.get(bone)?;
                    let parent_bone = *char_colliders.bone_entities.get(&parent_bone_type)?;

                    let child_bone_world = bones.get(child_bone).ok()?;
                    let parent_bone_world = bones.get(parent_bone).ok()?;
                    let anchor_world = child_bone_world.translation();

                    let parent_offset = collider_offsets.get(parent).ok()?;
                    let child_offset = collider_offsets.get(child).ok()?;

                    let parent_collider = Transform::from(*parent_bone_world) * parent_offset.0;
                    let child_collider = Transform::from(*child_bone_world) * child_offset.0;

                    let parent_matrix = parent_collider.to_matrix();
                    let child_matrix = child_collider.to_matrix();

                    let local_anchor1 = parent_matrix.inverse().transform_point3(anchor_world);
                    let local_anchor2 = child_matrix.inverse().transform_point3(anchor_world);

                    let parent_rot = parent_collider.rotation;
                    let child_rot = child_collider.rotation;

                    // Axis for body2's revolute joint frame so both bodies
                    // rotate around the same world-space axis.
                    let world_axis = parent_rot * Vec3::Z;
                    let local_axis2 = child_rot.inverse() * world_axis;

                    // Basis for body2's spherical joint frame so the
                    // bindpose relative rotation is the rest position.
                    let local_basis2 = child_rot.inverse() * parent_rot;

                    Some((
                        *bone,
                        parent,
                        child,
                        local_anchor1,
                        local_anchor2,
                        local_axis2,
                        local_basis2,
                    ))
                })
                .collect();

        for (bone, parent, child, local_anchor1, local_anchor2, local_axis2, local_basis2) in
            joints_to_spawn
        {
            let joint = spawn_ragdoll_joint(
                &mut commands,
                bone,
                parent,
                child,
                local_anchor1,
                local_anchor2,
                local_axis2,
                local_basis2,
                character_entity,
            );
            char_colliders.joint_entities.push(joint);
        }
    }
}

pub(crate) fn deactivate_ragdoll(
    mut commands: Commands,
    mut removed_ragdoll: RemovedComponents<Ragdoll>,
    mut all_colliders: Query<&mut CharacterColliders>,
) {
    for entity in removed_ragdoll.read() {
        let Ok(mut char_colliders) = all_colliders.get_mut(entity) else {
            continue;
        };

        for &joint in &char_colliders.joint_entities {
            commands.entity(joint).despawn();
        }
        char_colliders.joint_entities.clear();

        for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
            commands
                .entity(collider_entity)
                .insert(RigidBody::KinematicPositionBased)
                .insert(KinematicCollider)
                .remove::<Damping>();
        }
    }
}

pub(crate) fn force_sleep_ragdoll(
    time: Res<Time>,
    mut ragdolls: Query<(&mut Ragdoll, &CharacterColliders)>,
    mut sleep_query: Query<&mut Sleeping>,
    mut velocity_query: Query<&mut Velocity>,
) {
    for (mut ragdoll, char_colliders) in ragdolls.iter_mut() {
        let Some(timer) = &mut ragdoll.sleep_timer else {
            continue;
        };
        timer.tick(time.delta());
        if timer.just_finished() {
            ragdoll.sleep_timer = None;
            for (_, &collider_entity) in char_colliders.collider_entities.iter() {
                if let Ok(mut sleeping) = sleep_query.get_mut(collider_entity) {
                    sleeping.normalized_linear_threshold = 100.0;
                    sleeping.angular_threshold = 100.0;
                }
                if let Ok(mut velocity) = velocity_query.get_mut(collider_entity) {
                    velocity.linear = Vec3::ZERO;
                    velocity.angular = Vec3::ZERO;
                }
            }
        }
    }
}

pub(crate) fn sync_bones_to_ragdoll(
    characters: Query<&CharacterColliders, With<Ragdoll>>,
    colliders: Query<(&Transform, &ColliderOffset), (With<ColliderBone>, Without<SkeletalBone>)>,
    mut bones: Query<
        (&mut Transform, Option<&ChildOf>),
        (With<SkeletalBone>, Without<ColliderBone>),
    >,
    global_transforms: Query<&GlobalTransform>,
) {
    for char_colliders in characters.iter() {
        let mut desired_joint_world = AHashMap::<ColliderBone, Transform>::default();
        for (bone_type, &collider_entity) in char_colliders.collider_entities.iter() {
            let Ok((collider_transform, offset)) = colliders.get(collider_entity) else {
                continue;
            };
            let joint_to_world =
                *collider_transform * Transform::from_matrix(offset.0.to_matrix().inverse());
            desired_joint_world.insert(*bone_type, joint_to_world);
        }

        let mut applied_joint_world = AHashMap::<Entity, Transform>::default();
        for bone_type in COLLIDERS {
            let Some(&joint_to_world) = desired_joint_world.get(&bone_type) else {
                continue;
            };
            let Some(&bone_entity) = char_colliders.bone_entities.get(&bone_type) else {
                continue;
            };

            let Ok((mut local_transform, parent)) = bones.get_mut(bone_entity) else {
                continue;
            };

            let local = if let Some(parent) = parent {
                let parent_entity = parent.parent();
                if let Some(parent_to_world) = applied_joint_world.get(&parent_entity) {
                    Transform::from_matrix(parent_to_world.to_matrix().inverse()) * joint_to_world
                } else if let Ok(parent_to_world) = global_transforms.get(parent_entity) {
                    Transform::from_matrix(parent_to_world.to_matrix().inverse()) * joint_to_world
                } else {
                    joint_to_world
                }
            } else {
                joint_to_world
            };

            // Noise gate: only write if the change exceeds a threshold,
            // preventing micro-jitter from the solver from twitching the visual skeleton.
            let dp = (local.translation - local_transform.translation).length();
            let dr = local.rotation.angle_between(local_transform.rotation);
            if dp > 0.0005 || dr > 0.0005 {
                *local_transform = local;
            }
            applied_joint_world.insert(bone_entity, joint_to_world);
        }
    }
}

fn get_collider_geometry(
    collider: ColliderBone,
    helpers: &[Vec3],
    i_collider: usize,
    inv_bindposes: &AHashMap<&str, Transform>,
) -> (Collider, Transform) {
    match collider {
        ColliderBone::Head => get_head_collider(helpers),
        ColliderBone::Chest | ColliderBone::Pelvis => {
            let bone_name = DEFAULT_RIG_COLLIDER_BONE_NAMES[i_collider];
            let inv_bindpose_rot = inv_bindposes[bone_name].rotation;
            get_midsection_collider(helpers, collider, inv_bindpose_rot)
        }
        ColliderBone::UpperRightArm
        | ColliderBone::UpperLeftArm
        | ColliderBone::LowerRightArm
        | ColliderBone::LowerLeftArm
        | ColliderBone::UpperRightLeg
        | ColliderBone::UpperLeftLeg
        | ColliderBone::LowerRightLeg
        | ColliderBone::LowerLeftLeg => get_limb_collider(helpers, collider),
        ColliderBone::LeftHand
        | ColliderBone::RightHand
        | ColliderBone::LeftFoot
        | ColliderBone::RightFoot => get_extremity_collider(helpers, collider),
    }
}

fn get_head_collider(helpers: &[Vec3]) -> (Collider, Transform) {
    let center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let radius = (helpers[HEAD_VERTICES[0]] - center).length();
    (Collider::ball(radius), Transform::from_translation(center))
}

fn get_midsection_collider(
    helpers: &[Vec3],
    joint: ColliderBone,
    inv_bindpose_rot: Quat,
) -> (Collider, Transform) {
    let ref_verts = match joint {
        ColliderBone::Chest => TORSO_VERTICES,
        ColliderBone::Pelvis => PELVIS_VERTICES,
        _ => unreachable!(),
    };

    let mut verts = [Vec3::ZERO; 8];
    for (i, mhv) in ref_verts.iter().enumerate() {
        verts[i] = helpers[*mhv];
        verts[i + 4] = Vec3::new(-verts[i].x, verts[i].y, verts[i].z);
    }
    let center = verts.iter().sum::<Vec3>() / 8.;

    let xmin = verts.iter().map(|v| v.x).reduce(f32::min).unwrap();
    let xmax = verts.iter().map(|v| v.x).reduce(f32::max).unwrap();
    let ymin = verts.iter().map(|v| v.y).reduce(f32::min).unwrap();
    let ymax = verts.iter().map(|v| v.y).reduce(f32::max).unwrap();
    let zmin = verts.iter().map(|v| v.z).reduce(f32::min).unwrap();
    let zmax = verts.iter().map(|v| v.z).reduce(f32::max).unwrap();

    (
        Collider::cuboid(
            (xmax - xmin) / 2.0,
            (ymax - ymin) / 2.0,
            (zmax - zmin) / 2.0,
        ),
        Transform::from_translation(center).with_rotation(inv_bindpose_rot),
    )
}

fn get_limb_collider(helpers: &[Vec3], joint: ColliderBone) -> (Collider, Transform) {
    let ref_verts = match joint {
        ColliderBone::LowerLeftArm => LOWER_LEFT_ARM_VERTICES,
        ColliderBone::LowerRightArm => LOWER_RIGHT_ARM_VERTICES,
        ColliderBone::UpperLeftArm => UPPER_LEFT_ARM_VERTICES,
        ColliderBone::UpperRightArm => UPPER_RIGHT_ARM_VERTICES,
        ColliderBone::LowerLeftLeg => LOWER_LEFT_LEG_VERTICES,
        ColliderBone::LowerRightLeg => LOWER_RIGHT_LEG_VERTICES,
        ColliderBone::UpperLeftLeg => UPPER_LEFT_LEG_VERTICES,
        ColliderBone::UpperRightLeg => UPPER_RIGHT_LEG_VERTICES,
        _ => unreachable!(),
    };
    let verts: Vec<Vec3> = ref_verts.iter().map(|&i| helpers[i]).collect();

    let p1 = (verts[0] + verts[1]) * 0.5;
    let p2 = (verts[2] + verts[3]) * 0.5;
    let r = (verts[0] - verts[1]).length() * 0.5;
    let c = (p1 + p2) * 0.5;
    let dir = (p1 - p2).normalize();
    let up = dir.cross(Vec3::NEG_Z).normalize();
    let fwd = dir.cross(up);

    let half_height = (p1 - p2).length() / 2.0;

    (
        Collider::capsule_y(half_height, r),
        Transform::from_translation(c)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(fwd, dir, up))),
    )
}

fn get_extremity_collider(helpers: &[Vec3], joint: ColliderBone) -> (Collider, Transform) {
    let ref_verts = match joint {
        ColliderBone::LeftHand => LEFT_HAND_VERTICES,
        ColliderBone::RightHand => RIGHT_HAND_VERTICES,
        ColliderBone::LeftFoot => LEFT_FOOT_VERTICES,
        ColliderBone::RightFoot => RIGHT_FOOT_VERTICES,
        _ => unreachable!(),
    };
    let verts: Vec<Vec3> = ref_verts.iter().map(|&i| helpers[i]).collect();

    let x = (verts[0] - verts[1]).length();
    let y = (verts[2] - verts[3]).length();
    let z = (verts[4] - verts[5]).length();

    let center = verts.iter().sum::<Vec3>() / verts.len() as f32;

    let x_axis = verts[1] - verts[0];
    let y_axis = (verts[3] - verts[2]).normalize();
    let z_axis = x_axis.cross(y_axis).normalize();
    let x_axis = y_axis.cross(z_axis);

    (
        Collider::cuboid(x / 2.0, y / 2.0, z / 2.0),
        Transform::from_translation(center)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(x_axis, y_axis, z_axis))),
    )
}

fn spawn_ragdoll_joint(
    commands: &mut Commands,
    bone: ColliderBone,
    parent: Entity,
    child: Entity,
    local_anchor1: Vec3,
    local_anchor2: Vec3,
    local_axis2: Vec3,
    local_basis2: Quat,
    character: Entity,
) -> Entity {
    let entity = match bone {
        ColliderBone::LowerRightArm | ColliderBone::LowerLeftArm => commands
            .spawn((
                ImpulseJoint::new(
                    parent,
                    RevoluteJointBuilder::new(Vec3::Z)
                        .local_axis2(local_axis2)
                        .limits([-0.17, 2.5])
                        .local_anchor1(local_anchor1)
                        .local_anchor2(local_anchor2),
                ),
                JointForCharacter(character),
            ))
            .id(),
        ColliderBone::LowerRightLeg | ColliderBone::LowerLeftLeg => commands
            .spawn((
                ImpulseJoint::new(
                    parent,
                    RevoluteJointBuilder::new(Vec3::Z)
                        .local_axis2(local_axis2)
                        .limits([-2.4, 0.0])
                        .local_anchor1(local_anchor1)
                        .local_anchor2(local_anchor2),
                ),
                JointForCharacter(character),
            ))
            .id(),
        _ => {
            let (swing, twist) = get_spherical_limits(bone);
            let mut joint = SphericalJoint::new();
            joint
                .data
                .set_local_basis2(local_basis2)
                .set_limits(JointAxis::AngX, [-swing, swing])
                .set_limits(JointAxis::AngY, [-twist, twist])
                .set_limits(JointAxis::AngZ, [-swing, swing])
                .set_local_anchor1(local_anchor1)
                .set_local_anchor2(local_anchor2);
            commands
                .spawn((
                    ImpulseJoint::new(parent, joint),
                    JointForCharacter(character),
                ))
                .id()
        }
    };
    commands.entity(entity).set_parent_in_place(child);
    entity
}

const fn get_spherical_limits(bone: ColliderBone) -> (f32, f32) {
    match bone {
        ColliderBone::Chest => (0.5, 0.3),
        ColliderBone::Head => (0.5, 0.3),
        ColliderBone::UpperRightArm | ColliderBone::UpperLeftArm => (1.5, 0.5),
        ColliderBone::UpperRightLeg | ColliderBone::UpperLeftLeg => (1.5, 0.3),
        ColliderBone::LeftHand | ColliderBone::RightHand => (0.3, 0.3),
        ColliderBone::LeftFoot | ColliderBone::RightFoot => (0.2, 0.1),
        _ => unreachable!(),
    }
}
