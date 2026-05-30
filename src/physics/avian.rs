use ahash::AHashMap;
use avian3d::prelude::*;
use bevy::{
    math::{Quat, Vec3},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};

use crate::{
    morphs::MakeHumanMorphs,
    prelude::{BaseMesh, CharacterShape, CharacterShapeAsset, CharacterTemplate},
    rigs::{RigData, RigType, SkeletalBone},
};

use super::{
    get_collider_parent, ColliderBone, COLLIDERS, DEFAULT_RIG_COLLIDER_BONE_NAMES, HEAD_VERTICES,
    LEFT_FOOT_VERTICES, LEFT_HAND_VERTICES, LOWER_LEFT_ARM_VERTICES, LOWER_LEFT_LEG_VERTICES,
    LOWER_RIGHT_ARM_VERTICES, LOWER_RIGHT_LEG_VERTICES, PELVIS_VERTICES, RIGHT_FOOT_VERTICES,
    RIGHT_HAND_VERTICES, TORSO_VERTICES, UPPER_LEFT_ARM_VERTICES, UPPER_LEFT_LEG_VERTICES,
    UPPER_RIGHT_ARM_VERTICES, UPPER_RIGHT_LEG_VERTICES,
};

/// Describes whether ragdoll physics is active
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CharacterRagdoll {
    #[default]
    None,
    Full,
}

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

/// Offset from a collider's parent bone to the collider itself.
#[derive(Component)]
pub(crate) struct ColliderOffset(pub(crate) Transform);

/// Marker for colliders that are currently in kinematic (animation-following) mode.
#[derive(Component)]
pub(crate) struct KinematicCollider;

/// Links a collider entity to its owning character entity.
#[derive(Component)]
#[relationship(relationship_target = ColliderList)]
pub(crate) struct ColliderForCharacter(pub(crate) Entity);

/// Auto-maintained list of collider entities belonging to a character.
#[derive(Component)]
#[relationship_target(relationship = ColliderForCharacter)]
pub(crate) struct ColliderList(Vec<Entity>);

/// Links a joint entity to its owning character entity.
#[derive(Component)]
#[relationship(relationship_target = JointList)]
pub(crate) struct JointForCharacter(pub(crate) Entity);

/// Auto-maintained list of joint entities belonging to a character.
#[derive(Component)]
#[relationship_target(relationship = JointForCharacter)]
pub(crate) struct JointList(Vec<Entity>);

/// Marker for characters whose skeleton is fitted but colliders haven't been spawned yet.
#[derive(Component)]
pub(crate) struct NeedsColliders;

pub(crate) fn mark_needs_colliders(
    mut commands: Commands,
    characters: Query<
        Entity,
        (
            With<CharacterColliders>,
            With<SkinnedMesh>,
            Without<NeedsColliders>,
        ),
    >,
    collider_data: Query<&CharacterColliders>,
) {
    for entity in characters.iter() {
        if collider_data
            .get(entity)
            .is_ok_and(|c| c.collider_entities.is_empty())
        {
            commands.entity(entity).insert(NeedsColliders);
        }
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
                    RigidBody::Kinematic,
                    KinematicCollider,
                    collider,
                    geometry,
                    ColliderOffset(collider_to_joint),
                    ColliderDensity(1.0),
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

/// This function syncs kinematic character colliders to align with the skeletal bones
pub(crate) fn sync_colliders(
    characters: Query<&CharacterColliders>,
    bones: Query<&GlobalTransform>,
    mut collider_data: Query<
        (&mut Position, &mut Rotation, &ColliderOffset),
        With<KinematicCollider>,
    >,
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
            let Ok((mut position, mut rotation, offset)) = collider_data.get_mut(*collider_entity)
            else {
                continue;
            };
            let Some(bone_entity) = colliders.bone_entities.get(&bone_type) else {
                continue;
            };
            let Ok(joint_to_world) = bones.get(*bone_entity) else {
                continue;
            };
            let world = Transform::from(*joint_to_world) * offset.0;
            *position = Position(world.translation);
            *rotation = Rotation(world.rotation);
        }
    }
}

pub(crate) fn set_ragdoll_state(
    mut characters: Query<
        (Entity, &CharacterRagdoll, &mut CharacterColliders),
        Changed<CharacterRagdoll>,
    >,
    mut commands: Commands,
    bones: Query<&GlobalTransform>,
) {
    for (character_entity, ragdoll, mut char_colliders) in characters.iter_mut() {
        match *ragdoll {
            CharacterRagdoll::Full => {
                for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
                    commands
                        .entity(collider_entity)
                        .insert(RigidBody::Dynamic)
                        .remove::<KinematicCollider>()
                        .insert(LinearDamping(1.0))
                        .insert(AngularDamping(1.0));
                }

                let joints_to_spawn: Vec<(ColliderBone, Entity, Entity, Vec3)> = char_colliders
                    .collider_entities
                    .iter()
                    .filter_map(|(bone, &child)| {
                        let parent_bone = get_collider_parent(*bone)?;
                        let parent = *char_colliders.collider_entities.get(&parent_bone)?;
                        let bone_entity = *char_colliders.bone_entities.get(bone)?;
                        let bone_transform = bones.get(bone_entity).ok()?;
                        Some((*bone, parent, child, bone_transform.translation()))
                    })
                    .collect();

                for (bone, parent, child, anchor) in joints_to_spawn {
                    let joint = spawn_ragdoll_joint(
                        &mut commands,
                        bone,
                        parent,
                        child,
                        anchor,
                        character_entity,
                    );
                    char_colliders.joint_entities.push(joint);
                }
            }
            CharacterRagdoll::None => {
                for &joint in &char_colliders.joint_entities {
                    commands.entity(joint).despawn();
                }
                char_colliders.joint_entities.clear();

                for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
                    commands
                        .entity(collider_entity)
                        .insert(RigidBody::Kinematic)
                        .insert(KinematicCollider)
                        .remove::<LinearDamping>()
                        .remove::<AngularDamping>();
                }
            }
        }
    }
}

pub(crate) fn sync_bones_to_ragdoll(
    characters: Query<(&CharacterRagdoll, &CharacterColliders)>,
    colliders: Query<(&Transform, &ColliderOffset), (With<ColliderBone>, Without<SkeletalBone>)>,
    mut bones: Query<
        (&mut Transform, Option<&ChildOf>),
        (With<SkeletalBone>, Without<ColliderBone>),
    >,
    global_transforms: Query<&GlobalTransform>,
) {
    for (ragdoll, char_colliders) in characters.iter() {
        if *ragdoll != CharacterRagdoll::Full {
            continue;
        }

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

            *local_transform = local;
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
    (
        Collider::sphere(radius),
        Transform::from_translation(center),
    )
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
        Collider::cuboid(xmax - xmin, ymax - ymin, zmax - zmin),
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

    (
        Collider::capsule(r, (p1 - p2).length()),
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
        Collider::cuboid(x, y, z),
        Transform::from_translation(center)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(x_axis, y_axis, z_axis))),
    )
}

fn spawn_ragdoll_joint(
    commands: &mut Commands,
    bone: ColliderBone,
    parent: Entity,
    child: Entity,
    anchor: Vec3,
    character: Entity,
) -> Entity {
    match bone {
        ColliderBone::LowerRightArm | ColliderBone::LowerLeftArm => commands
            .spawn((
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_angle_limits(0.0, 2.5)
                    .with_point_compliance(0.0005)
                    .with_align_compliance(0.05)
                    .with_limit_compliance(0.05),
                JointCollisionDisabled,
                JointForCharacter(character),
            ))
            .id(),
        ColliderBone::LowerRightLeg | ColliderBone::LowerLeftLeg => commands
            .spawn((
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_angle_limits(-1.5, 0.0)
                    .with_point_compliance(0.0005)
                    .with_align_compliance(0.05)
                    .with_limit_compliance(0.05),
                JointCollisionDisabled,
                JointForCharacter(character),
            ))
            .id(),
        _ => {
            let (swing, twist) = get_spherical_limits(bone);
            commands
                .spawn((
                    SphericalJoint::new(parent, child)
                        .with_anchor(anchor)
                        .with_swing_limits(-swing, swing)
                        .with_twist_limits(-twist, twist)
                        .with_point_compliance(0.0005)
                        .with_swing_compliance(0.05)
                        .with_twist_compliance(0.05),
                    JointCollisionDisabled,
                    JointForCharacter(character),
                ))
                .id()
        }
    }
}

const fn get_spherical_limits(bone: ColliderBone) -> (f32, f32) {
    match bone {
        ColliderBone::Pelvis => (0.3, 0.3),
        ColliderBone::Chest => (0.5, 0.3),
        ColliderBone::Head => (0.5, 0.3),
        ColliderBone::UpperRightArm | ColliderBone::UpperLeftArm => (1.5, 0.5),
        ColliderBone::UpperRightLeg | ColliderBone::UpperLeftLeg => (1.5, 0.3),
        ColliderBone::LeftHand | ColliderBone::RightHand => (0.8, 0.8),
        ColliderBone::LeftFoot | ColliderBone::RightFoot => (0.5, 0.2),
        _ => unreachable!(),
    }
}
