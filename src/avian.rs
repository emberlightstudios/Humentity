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
    spawn_skeleton::FitSkeleton,
};

const HEAD_VERTICES: [usize; 2] = [5063, 5389];
const TORSO_VERTICES: [usize; 4] = [1553, 3753, 4097, 4049];
const PELVIS_VERTICES: [usize; 4] = [4174, 4243, 4353, 4170];
const UPPER_RIGHT_LEG_VERTICES: [usize; 4] = [4407, 4268, 4567, 4565];
const LOWER_RIGHT_LEG_VERTICES: [usize; 4] = [4662, 4664, 6385, 6375];
const UPPER_RIGHT_ARM_VERTICES: [usize; 4] = [1630, 1432, 3330, 3323];
const LOWER_RIGHT_ARM_VERTICES: [usize; 4] = [3412, 3877, 3552, 3906];
const UPPER_LEFT_ARM_VERTICES: [usize; 4] = [8302, 8120, 9998, 9991];
const LOWER_LEFT_ARM_VERTICES: [usize; 4] = [10080, 10542, 10220, 10571];
const UPPER_LEFT_LEG_VERTICES: [usize; 4] = [11025, 10898, 11185, 11183];
const LOWER_LEFT_LEG_VERTICES: [usize; 4] = [11280, 11282, 12982, 12972];
const RIGHT_HAND_VERTICES: [usize; 6] = [2776, 3189, 2119, 3909, 3247, 3650];
const RIGHT_FOOT_VERTICES: [usize; 6] = [6251, 6705, 4972, 5845, 6214, 6298];
const LEFT_HAND_VERTICES: [usize; 6] = [9444, 9857, 8787, 10574, 9915, 10318];
const LEFT_FOOT_VERTICES: [usize; 6] = [12848, 13301, 11590, 12442, 12811, 12895];

/// Describes whether ragdoll physics is active
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CharacterRagdoll {
    #[default]
    None,
    Full,
}

/// The set of colliders on a character
#[derive(Component, Hash, Copy, Clone, Eq, PartialEq, Debug)]
pub enum CharacterColliderBone {
    Head,
    Chest,
    Pelvis,
    UpperRightArm,
    UpperLeftArm,
    LowerRightArm,
    LowerLeftArm,
    UpperRightLeg,
    UpperLeftLeg,
    LowerRightLeg,
    LowerLeftLeg,
    LeftHand,
    RightHand,
    LeftFoot,
    RightFoot,
}

const COLLIDERS: [CharacterColliderBone; 15] = [
    CharacterColliderBone::Pelvis,
    CharacterColliderBone::Chest,
    CharacterColliderBone::UpperLeftLeg,
    CharacterColliderBone::UpperRightLeg,
    CharacterColliderBone::LowerLeftLeg,
    CharacterColliderBone::LowerRightLeg,
    CharacterColliderBone::LeftFoot,
    CharacterColliderBone::RightFoot,
    CharacterColliderBone::UpperLeftArm,
    CharacterColliderBone::UpperRightArm,
    CharacterColliderBone::LowerLeftArm,
    CharacterColliderBone::LowerRightArm,
    CharacterColliderBone::LeftHand,
    CharacterColliderBone::RightHand,
    CharacterColliderBone::Head,
];

const DEFAULT_RIG_COLLIDER_BONE_NAMES: [&str; 15] = [
    "root",
    "spine03",
    "upperleg01.L",
    "upperleg01.R",
    "lowerleg01.L",
    "lowerleg01.R",
    "foot.L",
    "foot.R",
    "upperarm01.L",
    "upperarm01.R",
    "lowerarm01.L",
    "lowerarm01.R",
    "wrist.L",
    "wrist.R",
    "head",
];

#[derive(Component, Default)]
pub struct CharacterColliders {
    pub bones_subset: Option<Vec<CharacterColliderBone>>,
    pub(crate) collider_entities: AHashMap<CharacterColliderBone, Entity>,
    pub(crate) bone_entities: AHashMap<CharacterColliderBone, Entity>,
    pub(crate) joint_entities: Vec<Entity>,
}

impl CharacterColliders {
    pub fn new(include_all: bool) -> Self {
        Self {
            bones_subset: if include_all { None } else { Some(vec![]) },
            collider_entities: AHashMap::default(),
            bone_entities: AHashMap::default(),
            joint_entities: Vec::new(),
        }
    }
}

const fn get_collider_parent(bone: CharacterColliderBone) -> Option<CharacterColliderBone> {
    match bone {
        CharacterColliderBone::Head => Some(CharacterColliderBone::Chest),
        CharacterColliderBone::Chest => Some(CharacterColliderBone::Pelvis),
        CharacterColliderBone::Pelvis => None,
        CharacterColliderBone::UpperRightArm => Some(CharacterColliderBone::Chest),
        CharacterColliderBone::UpperLeftArm => Some(CharacterColliderBone::Chest),
        CharacterColliderBone::LowerRightArm => Some(CharacterColliderBone::UpperRightArm),
        CharacterColliderBone::LowerLeftArm => Some(CharacterColliderBone::UpperLeftArm),
        CharacterColliderBone::UpperRightLeg => Some(CharacterColliderBone::Pelvis),
        CharacterColliderBone::UpperLeftLeg => Some(CharacterColliderBone::Pelvis),
        CharacterColliderBone::LowerRightLeg => Some(CharacterColliderBone::UpperRightLeg),
        CharacterColliderBone::LowerLeftLeg => Some(CharacterColliderBone::UpperLeftLeg),
        CharacterColliderBone::LeftHand => Some(CharacterColliderBone::LowerLeftArm),
        CharacterColliderBone::RightHand => Some(CharacterColliderBone::LowerRightArm),
        CharacterColliderBone::LeftFoot => Some(CharacterColliderBone::LowerLeftLeg),
        CharacterColliderBone::RightFoot => Some(CharacterColliderBone::LowerRightLeg),
    }
}

/// Offset from a collider's parent bone to the collider itself.
#[derive(Component)]
pub(crate) struct ColliderOffset(pub(crate) Transform);

/// Marker for colliders that are currently in kinematic (animation-following) mode.
#[derive(Component)]
pub(crate) struct KinematicCollider;

pub(crate) fn spawn_colliders(
    mut commands: Commands,
    mut characters: Query<
        (
            Entity,
            &CharacterShape,
            &SkinnedMesh,
            &mut CharacterColliders,
        ),
        (Without<FitSkeleton>, Without<RigidBody>),
    >,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    basemesh: Res<BaseMesh>,
    mh_morphs: Res<MakeHumanMorphs>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    rig_data: Res<RigData>,
) {
    for (_character_entity, character_shape, skm, mut colliders) in characters.iter_mut() {
        if !colliders.collider_entities.is_empty() {
            continue;
        }

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

        let target_bones: Vec<CharacterColliderBone> = match &colliders.bones_subset {
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
                ))
                .id();

            colliders
                .collider_entities
                .insert(collider, collider_entity);
            colliders
                .bone_entities
                .insert(collider, bone_entities[joint_name]);
        }
    }
}

pub(crate) fn sync_colliders(
    characters: Query<&CharacterColliders>,
    bones: Query<&GlobalTransform>,
    mut collider_data: Query<(&mut Transform, &ColliderOffset), With<KinematicCollider>>,
) {
    for colliders in characters.iter() {
        let target_bones: Vec<CharacterColliderBone> = match &colliders.bones_subset {
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

pub(crate) fn set_ragdoll_state(
    mut characters: Query<(&CharacterRagdoll, &mut CharacterColliders), Changed<CharacterRagdoll>>,
    mut commands: Commands,
    bones: Query<&GlobalTransform>,
) {
    for (ragdoll, mut char_colliders) in characters.iter_mut() {
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

                let joints_to_spawn: Vec<(CharacterColliderBone, Entity, Entity, Vec3)> =
                    char_colliders
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
                    let joint = spawn_ragdoll_joint(&mut commands, bone, parent, child, anchor);
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
    colliders: Query<
        (&Transform, &ColliderOffset),
        (With<CharacterColliderBone>, Without<SkeletalBone>),
    >,
    mut bones: Query<
        (&mut Transform, Option<&ChildOf>),
        (With<SkeletalBone>, Without<CharacterColliderBone>),
    >,
    global_transforms: Query<&GlobalTransform>,
) {
    for (ragdoll, char_colliders) in characters.iter() {
        if *ragdoll != CharacterRagdoll::Full {
            continue;
        }

        let mut desired_joint_world = AHashMap::<CharacterColliderBone, Transform>::default();
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
    collider: CharacterColliderBone,
    helpers: &[Vec3],
    i_collider: usize,
    inv_bindposes: &AHashMap<&str, Transform>,
) -> (Collider, Transform) {
    match collider {
        CharacterColliderBone::Head => get_head_collider(helpers),
        CharacterColliderBone::Chest | CharacterColliderBone::Pelvis => {
            let bone_name = DEFAULT_RIG_COLLIDER_BONE_NAMES[i_collider];
            let inv_bindpose_rot = inv_bindposes[bone_name].rotation;
            get_midsection_collider(helpers, collider, inv_bindpose_rot)
        }
        CharacterColliderBone::UpperRightArm
        | CharacterColliderBone::UpperLeftArm
        | CharacterColliderBone::LowerRightArm
        | CharacterColliderBone::LowerLeftArm
        | CharacterColliderBone::UpperRightLeg
        | CharacterColliderBone::UpperLeftLeg
        | CharacterColliderBone::LowerRightLeg
        | CharacterColliderBone::LowerLeftLeg => get_limb_collider(helpers, collider),
        CharacterColliderBone::LeftHand
        | CharacterColliderBone::RightHand
        | CharacterColliderBone::LeftFoot
        | CharacterColliderBone::RightFoot => get_extremity_collider(helpers, collider),
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
    joint: CharacterColliderBone,
    inv_bindpose_rot: Quat,
) -> (Collider, Transform) {
    let ref_verts = match joint {
        CharacterColliderBone::Chest => TORSO_VERTICES,
        CharacterColliderBone::Pelvis => PELVIS_VERTICES,
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

fn get_limb_collider(helpers: &[Vec3], joint: CharacterColliderBone) -> (Collider, Transform) {
    let ref_verts = match joint {
        CharacterColliderBone::LowerLeftArm => LOWER_LEFT_ARM_VERTICES,
        CharacterColliderBone::LowerRightArm => LOWER_RIGHT_ARM_VERTICES,
        CharacterColliderBone::UpperLeftArm => UPPER_LEFT_ARM_VERTICES,
        CharacterColliderBone::UpperRightArm => UPPER_RIGHT_ARM_VERTICES,
        CharacterColliderBone::LowerLeftLeg => LOWER_LEFT_LEG_VERTICES,
        CharacterColliderBone::LowerRightLeg => LOWER_RIGHT_LEG_VERTICES,
        CharacterColliderBone::UpperLeftLeg => UPPER_LEFT_LEG_VERTICES,
        CharacterColliderBone::UpperRightLeg => UPPER_RIGHT_LEG_VERTICES,
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

fn get_extremity_collider(helpers: &[Vec3], joint: CharacterColliderBone) -> (Collider, Transform) {
    let ref_verts = match joint {
        CharacterColliderBone::LeftHand => LEFT_HAND_VERTICES,
        CharacterColliderBone::RightHand => RIGHT_HAND_VERTICES,
        CharacterColliderBone::LeftFoot => LEFT_FOOT_VERTICES,
        CharacterColliderBone::RightFoot => RIGHT_FOOT_VERTICES,
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
    bone: CharacterColliderBone,
    parent: Entity,
    child: Entity,
    anchor: Vec3,
) -> Entity {
    match bone {
        CharacterColliderBone::LowerRightArm | CharacterColliderBone::LowerLeftArm => commands
            .spawn((
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_angle_limits(0.0, 2.5)
                    .with_point_compliance(0.0005)
                    .with_align_compliance(0.05)
                    .with_limit_compliance(0.05),
                JointCollisionDisabled,
            ))
            .id(),
        CharacterColliderBone::LowerRightLeg | CharacterColliderBone::LowerLeftLeg => commands
            .spawn((
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_angle_limits(-1.5, 0.0)
                    .with_point_compliance(0.0005)
                    .with_align_compliance(0.05)
                    .with_limit_compliance(0.05),
                JointCollisionDisabled,
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
                ))
                .id()
        }
    }
}

const fn get_spherical_limits(bone: CharacterColliderBone) -> (f32, f32) {
    match bone {
        CharacterColliderBone::Pelvis => (0.3, 0.3),
        CharacterColliderBone::Chest => (0.5, 0.3),
        CharacterColliderBone::Head => (0.5, 0.3),
        CharacterColliderBone::UpperRightArm | CharacterColliderBone::UpperLeftArm => (1.5, 0.5),
        CharacterColliderBone::UpperRightLeg | CharacterColliderBone::UpperLeftLeg => (1.5, 0.3),
        CharacterColliderBone::LeftHand | CharacterColliderBone::RightHand => (0.8, 0.8),
        CharacterColliderBone::LeftFoot | CharacterColliderBone::RightFoot => (0.5, 0.2),
        _ => unreachable!(),
    }
}
