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
    rigs::{RigData, RigType},
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
}

impl CharacterColliders {
    pub fn new(include_all: bool) -> Self {
        Self {
            bones_subset: if include_all { None } else { Some(vec![]) },
            collider_entities: AHashMap::default(),
            bone_entities: AHashMap::default(),
        }
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
    characters: Query<(&CharacterRagdoll, &CharacterColliders), Changed<CharacterRagdoll>>,
    mut commands: Commands,
) {
    for (ragdoll, char_colliders) in characters.iter() {
        for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
            match *ragdoll {
                CharacterRagdoll::Full => {
                    commands
                        .entity(collider_entity)
                        .insert(RigidBody::Dynamic)
                        .remove::<KinematicCollider>();
                }
                CharacterRagdoll::None => {
                    commands
                        .entity(collider_entity)
                        .insert(RigidBody::Kinematic)
                        .insert(KinematicCollider);
                }
            }
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
