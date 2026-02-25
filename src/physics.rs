use ahash::AHashMap;
use avian3d::prelude::*;
use bevy::{
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};

use crate::{
    morphs::MakeHumanMorphs,
    prefab::CharacterArchetypePrefabs,
    prelude::{BaseMesh, CharacterShapeConfig, RelatedEntities},
    rigs::{RigType, SkeletalBone, SkeletonCaches},
    spawn_skeleton::FitSkeleton,
    MODEL_ROTATION_FIX,
};

/// Use to find radius and center of sphere
const HEAD_VERTICES: [usize; 2] = [5063, 5389];
/// These are for the right side fo the body only.
/// Augment with 4 more verts with x -> -x
/// Use to find box shape
const TORSO_VERTICES: [usize; 4] = [1553, 3753, 4097, 4049];
const PELVIS_VERTICES: [usize; 4] = [4174, 4243, 4353, 4170];
// Use first 2 to find center and radius of capsule top
// Use last 2 to find center and radius of capsule bottom
const UPPER_LEG_VERTICES: [usize; 4] = [4407, 4268, 4567, 4565];
const LOWER_LEG_VERTICES: [usize; 4] = [4662, 4664, 6385, 6375];
const UPPER_ARM_VERTICES: [usize; 4] = [1630, 1432, 3330, 3323];
const LOWER_ARM_VERTICES: [usize; 4] = [3412, 3877, 3552, 3906];
/// Use cuboids. Get dimensions from  2x, 2y, 2z
const HAND_VERTICES: [usize; 6] = [2776, 3189, 2119, 3909, 3247, 3650];
const FOOT_VERTICES: [usize; 6] = [6251, 6705, 4972, 5845, 6214, 6298];

/// Stores transform for collider in bone space
#[derive(Component, Clone, Deref)]
pub(crate) struct PoseOffset(Transform);

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

/// An hierarchical ordering for colliders, does not flow back up the tree
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

/// The skeletal bones we want the colliders to latch onto. Make sure order is the same as above
const DEFAULT_RIG_COLLIDER_BONE_NAMES: [&'static str; 15] = [
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
    //"metacarpal2.L",
    //"metacarpal2.R",
    "wrist.L",
    "wrist.R",
    "head",
];

/// Get a collider's parent in the hierarchy
fn get_collider_parent(bone: CharacterColliderBone) -> Option<CharacterColliderBone> {
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

/// Provides body colliders for characters
/// TODO : Relationship to match entities so we can delete with despawn_related
#[derive(Component, Default)]
pub struct CharacterColliders {
    /// Should collider transforms currently synced to bones
    pub sync_to_bones: bool,
    collider_entities: AHashMap<CharacterColliderBone, Entity>,
    bone_entities: AHashMap<CharacterColliderBone, Entity>,
}

impl CharacterColliders {
    pub fn new(sync_to_bones: bool) -> Self {
        Self {
            sync_to_bones,
            ..Default::default()
        }
    }
}

// Temp marker component
#[derive(Component)]
pub(crate) struct NeedsColliders;

/// Ragdoll for characters
#[derive(Component, Eq, PartialEq, Clone)]
#[require(CharacterColliders)]
pub enum CharacterRagdoll {
    None,
    Full,
    Partial(Vec<CharacterColliderBone>),
}

/*--- Systems ---*/
pub(crate) fn mark_entity_needs_colliders(
    trigger: On<Add, CharacterColliders>,
    mut commands: Commands,
) {
    commands.entity(trigger.entity).insert(NeedsColliders);
}

/// Create colliders from character shape
pub(crate) fn spawn_colliders(
    mut needs_colliders: Query<
        (
            Entity,
            &CharacterShapeConfig,
            &RelatedEntities,
            &mut CharacterColliders,
            &SkinnedMesh,
        ),
        (
            Without<FitSkeleton>,
            Without<SkeletalBone>,
            With<NeedsColliders>,
        ),
    >,
    global_transforms: Query<&GlobalTransform>,
    prefabs: Res<CharacterArchetypePrefabs>,
    basemesh: Res<BaseMesh>,
    mh_morphs: Res<MakeHumanMorphs>,
    collider_mat: Res<ColliderMaterial>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    skeleton_caches: Res<SkeletonCaches>,
    mut geometries: ResMut<Assets<Geometry>>,
    mut commands: Commands,
) {
    for (character_entity, shape_config, related, mut colliders, skm) in needs_colliders.iter_mut()
    {
        let rig_type = prefabs[&shape_config.prefab].rig.rig_type;

        let collider_bone_map = match rig_type {
            RigType::Default => DEFAULT_RIG_COLLIDER_BONE_NAMES,
            _ => todo!("Implement this for the other rigs"),
        };

        let prefab = &prefabs[&shape_config.prefab];
        let helpers = prefab.get_helpers(&shape_config.prefab_morph_targets, &basemesh, &mh_morphs);
        let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes) else {
            return;
        };
        let sk_cache = &skeleton_caches[&prefab.rig.rig_type];

        let bone_entities = sk_cache
            .bone_order
            .iter()
            .cloned()
            .zip(skm.joints.iter().cloned())
            .collect::<AHashMap<&str, Entity>>();

        let inv_bindposes = sk_cache
            .bone_order
            .iter()
            .cloned()
            .zip(inv_bindposes.iter().map(|m| Transform::from_matrix(*m)))
            .collect::<AHashMap<&str, Transform>>();

        let mut collider_to_world_transforms = AHashMap::default();

        for (i_collider, collider) in COLLIDERS.iter().enumerate() {
            let (geometry, collider_to_backwards_model) = match collider {
                CharacterColliderBone::Head => get_head_collider(&helpers, &mut geometries),
                CharacterColliderBone::Chest | CharacterColliderBone::Pelvis => {
                    let bone_name = collider_bone_map[i_collider];
                    let inv_bindpose_rot = inv_bindposes[bone_name].rotation;
                    get_midsection_collider(&helpers, *collider, &mut geometries, inv_bindpose_rot)
                }
                CharacterColliderBone::UpperRightArm
                | CharacterColliderBone::UpperLeftArm
                | CharacterColliderBone::LowerRightArm
                | CharacterColliderBone::LowerLeftArm
                | CharacterColliderBone::UpperRightLeg
                | CharacterColliderBone::UpperLeftLeg
                | CharacterColliderBone::LowerRightLeg
                | CharacterColliderBone::LowerLeftLeg => {
                    get_limb_collider(&helpers, *collider, &mut geometries)
                }
                CharacterColliderBone::LeftHand
                | CharacterColliderBone::RightHand
                | CharacterColliderBone::LeftFoot
                | CharacterColliderBone::RightFoot => {
                    get_extremity_collider(&helpers, *collider, &mut geometries)
                }
            };

            let bone_name = collider_bone_map[i_collider];
            let Ok(bone_to_world) = global_transforms.get(bone_entities[bone_name]) else {
                continue;
            };
            let Ok(model_to_world) = global_transforms.get(related.rig) else {
                continue;
            };
            let model_to_world = Transform::from(model_to_world.clone());
            let world_to_bone = Transform::from_matrix(bone_to_world.to_matrix().inverse());

            // local transform
            let collider_to_world = model_to_world
                * Transform::from_rotation(MODEL_ROTATION_FIX.inverse())
                * collider_to_backwards_model;
            collider_to_world_transforms.insert(*collider, collider_to_world);
            // offset for placement from bone global
            let collider_to_bone = world_to_bone * collider_to_world;

            let collider_entity = commands
                .spawn((
                    RigidBody::Dynamic,
                    collider_to_world,
                    *collider,
                    Visibility::default(),
                    Shape {
                        geometry,
                        material: collider_mat.0.clone(),
                        ..Default::default()
                    },
                    PoseOffset(collider_to_bone),
                    colliders.filter.clone(),
                    MassProperties::density(1000.),
                ))
                .id();

            colliders
                .collider_entities
                .insert(*collider, collider_entity);
            colliders
                .bone_entities
                .insert(*collider, bone_entities[bone_name]);

            if *collider == CharacterColliderBone::Pelvis {
                commands.entity(collider_entity).insert(ArticulationRoot {
                    fix_base: true,
                    drive_limits_are_forces: true,
                    ..Default::default()
                });
                info!(
                    "[RAGDOLL] {:?} is ArticulationRoot at {:?}",
                    collider, collider_to_world.translation
                );
            } else {
                let parent_collider = get_collider_parent(*collider).unwrap();
                let parent_collider_to_world = collider_to_world_transforms[&parent_collider];
                let parent_pose = world_to_bone * parent_collider_to_world;
                let parent = colliders.collider_entities[&parent_collider];
                let child_pose = collider_to_bone;

                let child_pose_inv = Transform::from_matrix(child_pose.to_matrix().inverse());
                let parent_pose_inv = Transform::from_matrix(parent_pose.to_matrix().inverse());

                info!(
                    "[RAGDOLL] {:?} child_pose (collider->bone): trans={:?} rot={:?}",
                    collider, child_pose.translation, child_pose.rotation
                );
                info!(
                    "[RAGDOLL] {:?} child_pose_inv (bone->collider): trans={:?} rot={:?}",
                    collider, child_pose_inv.translation, child_pose_inv.rotation
                );
                info!(
                    "[RAGDOLL] {:?} parent_pose (parent_collider->bone): trans={:?} rot={:?}",
                    collider, parent_pose.translation, parent_pose.rotation
                );
                info!(
                    "[RAGDOLL] {:?} parent_pose_inv: trans={:?} rot={:?}",
                    collider, parent_pose_inv.translation, parent_pose_inv.rotation
                );

                commands
                    .entity(collider_entity)
                    .insert(get_articulation_joint(
                        *collider,
                        parent,
                        parent_pose_inv,
                        child_pose_inv,
                    ));
            }
        }

        commands.entity(character_entity).remove::<NeedsColliders>();
    }
}

fn get_articulation_joint(
    collider: CharacterColliderBone,
    parent: Entity,
    parent_pose: Transform,
    child_pose: Transform,
) -> ArticulationJoint {
    match collider {
        //CharacterColliderBone::Head => {
        _ => ArticulationJoint {
            parent,
            parent_pose,
            child_pose,
            joint_type: PxArticulationJointType::Spherical,
            motion_swing1: ArticulationJointMotion::Free,
            motion_swing2: ArticulationJointMotion::Free,
            motion_twist: ArticulationJointMotion::Free,
            motion_x: ArticulationJointMotion::Locked,
            motion_y: ArticulationJointMotion::Locked,
            motion_z: ArticulationJointMotion::Locked,
            friction_coefficient: 1.1,
            ..default()
        }, //_ => {
           //    ArticulationJoint {
           //        parent,
           //        parent_pose,
           //        child_pose,
           //        joint_type: PxArticulationJointType::Fix,
           //        ..default()
           //    }
           //}
    }
}

/// Syncs colliders to bone transforms during animation (i.e. not simulating physics)
pub(crate) fn sync_colliders(
    characters: Query<(&CharacterColliders, Option<&CharacterRagdoll>)>,
    global_transforms: Query<
        &GlobalTransform,
        Or<(With<SkeletalBone>, Without<CharacterColliderBone>)>,
    >,
    mut collider_transforms: Query<
        (&mut Transform, &GlobalTransform, &PoseOffset),
        (Without<SkeletalBone>, With<CharacterColliderBone>),
    >,
) {
    return;
    for (colliders, ragdoll) in characters {
        if !colliders.sync_to_bones {
            continue;
        };
        if colliders.collider_entities.is_empty() {
            continue;
        };

        if let Some(CharacterRagdoll::Full) = ragdoll {
            continue;
        }

        for collider in COLLIDERS.iter() {
            let Some(&collider_entity) = colliders.collider_entities.get(collider) else {
                continue;
            };

            if let Some(CharacterRagdoll::Partial(collider_bones)) = ragdoll {
                if collider_bones.contains(collider) {
                    continue;
                }
            }

            let Ok((mut collider_transform, _, offset)) =
                collider_transforms.get_mut(collider_entity)
            else {
                continue;
            };
            let bone_entity = colliders.bone_entities[collider];
            let Ok(bone_to_world) = global_transforms.get(bone_entity) else {
                continue;
            };
            let collider_to_bone = **offset;
            let collider_to_world = Transform::from(bone_to_world.clone()) * collider_to_bone;

            *collider_transform = collider_to_world;
        }
    }
}

/// Manage physics state for ragdolls.
pub(crate) fn on_ragdoll(
    ragdolls: Query<
        (&CharacterColliders, &CharacterRagdoll),
        (Changed<CharacterRagdoll>, Without<NeedsColliders>),
    >,
    transforms: Query<&GlobalTransform, Or<(With<CharacterColliderBone>, With<SkeletalBone>)>>,
    mut commands: Commands,
) {
    return;
    for (colliders, ragdoll) in ragdolls.iter() {
        match ragdoll {
            CharacterRagdoll::Full => {
                for collider in COLLIDERS.iter() {
                    let Some(&entity) = colliders.collider_entities.get(collider) else {
                        continue;
                    };
                }
            }
            CharacterRagdoll::None => {
                for collider in COLLIDERS.iter() {
                    let Some(&entity) = colliders.collider_entities.get(collider) else {
                        continue;
                    };
                }
            }
            CharacterRagdoll::Partial(character_collider_bones) => {
                let roots: Vec<_> = character_collider_bones
                    .iter()
                    .filter(|&c| {
                        let parent = get_collider_parent(*c);
                        parent.is_none() || !character_collider_bones.contains(&parent.unwrap())
                    })
                    .cloned()
                    .collect();

                for collider in COLLIDERS.iter() {
                    let Some(&entity) = colliders.collider_entities.get(collider) else {
                        continue;
                    };
                }
            }
        }
    }
}

/*--- Utility functions ---*/
fn get_head_collider(
    helpers: &[Vec3],
    geometry: &mut Assets<Geometry>,
) -> (Handle<Geometry>, Transform) {
    let center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let radius = (helpers[HEAD_VERTICES[0]] - center).length();
    (
        geometry.add(Sphere::new(radius)),
        Transform::from_translation(MODEL_ROTATION_FIX * center),
    )
}

fn get_midsection_collider(
    helpers: &[Vec3],
    joint: CharacterColliderBone,
    geometry: &mut Assets<Geometry>,
    inv_bindpose_rot: Quat,
) -> (Handle<Geometry>, Transform) {
    let ref_verts = match joint {
        CharacterColliderBone::Chest => TORSO_VERTICES,
        CharacterColliderBone::Pelvis => PELVIS_VERTICES,
        _ => unimplemented!("wrong joint input"),
    };

    let mut verts = [Vec3::ZERO; 8];
    for (i, mhv) in ref_verts.iter().enumerate() {
        verts[i] = helpers[*mhv];
        verts[i + 4] = Vec3::new(-verts[i].x, verts[i].y, verts[i].z)
    }
    let center = verts.iter().sum::<Vec3>() / 8.;

    let xmin = verts
        .iter()
        .map(|v| v.x)
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap();
    let xmax = verts
        .iter()
        .map(|v| v.x)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap();
    let ymin = verts
        .iter()
        .map(|v| v.y)
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap();
    let ymax = verts
        .iter()
        .map(|v| v.y)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap();
    let zmin = verts
        .iter()
        .map(|v| v.z)
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap();
    let zmax = verts
        .iter()
        .map(|v| v.z)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap();
    (
        geometry.add(Cuboid::new(xmax - xmin, ymax - ymin, zmax - zmin)),
        Transform::from_translation(MODEL_ROTATION_FIX * center).with_rotation(
            MODEL_ROTATION_FIX *
                inv_bindpose_rot * //.inverse() *   // Why inverse bindpose, not bindpose? idk
                MODEL_ROTATION_FIX.inverse(),
        ),
    )
}

fn get_limb_collider(
    helpers: &[Vec3],
    joint: CharacterColliderBone,
    geometry: &mut Assets<Geometry>,
) -> (Handle<Geometry>, Transform) {
    let ref_verts = match joint {
        CharacterColliderBone::LowerLeftArm | CharacterColliderBone::LowerRightArm => {
            LOWER_ARM_VERTICES
        }
        CharacterColliderBone::UpperLeftArm | CharacterColliderBone::UpperRightArm => {
            UPPER_ARM_VERTICES
        }
        CharacterColliderBone::LowerLeftLeg | CharacterColliderBone::LowerRightLeg => {
            LOWER_LEG_VERTICES
        }
        CharacterColliderBone::UpperLeftLeg | CharacterColliderBone::UpperRightLeg => {
            UPPER_LEG_VERTICES
        }
        _ => unimplemented!("wrong joint input"),
    };
    let mut verts = [Vec3::ZERO; 4];
    for (i, &mhv) in ref_verts.iter().enumerate() {
        verts[i] = helpers[mhv];
        if matches!(joint, CharacterColliderBone::LowerRightArm)
            || matches!(joint, CharacterColliderBone::UpperRightArm)
            || matches!(joint, CharacterColliderBone::LowerRightLeg)
            || matches!(joint, CharacterColliderBone::UpperRightLeg)
        {
            verts[i].x = -verts[i].x;
        }
    }

    let p1 = (verts[0] + verts[1]) * 0.5;
    let p2 = (verts[2] + verts[3]) * 0.5;
    let r = (verts[0] - verts[1]).length() * 0.5;
    let c = 0.5 * (p1 + p2);
    let dir = (p1 - p2).normalize();
    let up = dir.cross(Vec3::NEG_Z).normalize();
    let fwd = dir.cross(up);

    (
        // Subtract just a small amount of capsule length
        geometry.add(Capsule3d::new(r, (p1 - p2).length())),
        // we have to account for the mesh facing wrong direction
        Transform::from_translation(MODEL_ROTATION_FIX * c).with_rotation(
            MODEL_ROTATION_FIX
                * Quat::from_mat3(&Mat3::from_cols(dir, up, fwd))
                * MODEL_ROTATION_FIX.inverse(),
        ),
    )
}

fn get_extremity_collider(
    helpers: &[Vec3],
    joint: CharacterColliderBone,
    geometry: &mut Assets<Geometry>,
) -> (Handle<Geometry>, Transform) {
    let ref_verts = match joint {
        CharacterColliderBone::LeftHand | CharacterColliderBone::RightHand => HAND_VERTICES,
        CharacterColliderBone::LeftFoot | CharacterColliderBone::RightFoot => FOOT_VERTICES,
        _ => unimplemented!("wrong joint input"),
    };
    let mut verts = [Vec3::ZERO; 6];
    for (i, &mhv) in ref_verts.iter().enumerate() {
        verts[i] = helpers[mhv];
        if matches!(joint, CharacterColliderBone::RightHand)
            || matches!(joint, CharacterColliderBone::RightFoot)
        {
            verts[i].x = -verts[i].x;
        }
    }

    let x = (verts[0] - verts[1]).length();
    let y = (verts[2] - verts[3]).length();
    let z = (verts[4] - verts[5]).length();
    let cube = Cuboid::new(x, y, z);

    let center = verts.iter().sum::<Vec3>() / verts.len() as f32;

    let x = verts[1] - verts[0];
    let y = (verts[3] - verts[2]).normalize();
    let z = x.cross(y).normalize();
    let x = y.cross(z);

    (
        geometry.add(cube),
        Transform::from_translation(MODEL_ROTATION_FIX * center).with_rotation(
            MODEL_ROTATION_FIX
                * Quat::from_mat3(&Mat3::from_cols(x, y, z))
                * MODEL_ROTATION_FIX.inverse(),
        ),
    )
}

pub(crate) fn debug_ragdoll_positions(
    characters: Query<&CharacterColliders>,
    transforms: Query<&GlobalTransform, With<CharacterColliderBone>>,
) {
    use bevy::prelude::*;

    let mut hand_l: Option<Vec3> = None;
    let mut hand_r: Option<Vec3> = None;
    let mut lower_arm_l: Option<Vec3> = None;
    let mut lower_arm_r: Option<Vec3> = None;
    let mut upper_arm_l: Option<Vec3> = None;
    let mut upper_arm_r: Option<Vec3> = None;
    let mut chest: Option<Vec3> = None;
    let mut pelvis: Option<Vec3> = None;

    for colliders in &characters {
        for (bone, entity) in colliders.collider_entities.iter() {
            if let Ok(transform) = transforms.get(*entity) {
                let pos = transform.translation();
                match bone {
                    CharacterColliderBone::LeftHand => hand_l = Some(pos),
                    CharacterColliderBone::RightHand => hand_r = Some(pos),
                    CharacterColliderBone::LowerLeftArm => lower_arm_l = Some(pos),
                    CharacterColliderBone::LowerRightArm => lower_arm_r = Some(pos),
                    CharacterColliderBone::UpperLeftArm => upper_arm_l = Some(pos),
                    CharacterColliderBone::UpperRightArm => upper_arm_r = Some(pos),
                    CharacterColliderBone::Chest => chest = Some(pos),
                    CharacterColliderBone::Pelvis => pelvis = Some(pos),
                    _ => {}
                }
            }
        }
    }

    if let (Some(chest), Some(pelvis)) = (chest, pelvis) {
        info!("[RAGDOLL DEBUG] Pelvis: {:.3}, Chest: {:.3}", pelvis, chest);
    }
    if let (Some(upper_arm_l), Some(lower_arm_l), Some(hand_l)) = (upper_arm_l, lower_arm_l, hand_l)
    {
        info!(
            "[RAGDOLL DEBUG] L Arm: Upper={:.3}, Lower={:.3}, Hand={:.3}",
            upper_arm_l, lower_arm_l, hand_l
        );
    }
    if let (Some(upper_arm_r), Some(lower_arm_r), Some(hand_r)) = (upper_arm_r, lower_arm_r, hand_r)
    {
        info!(
            "[RAGDOLL DEBUG] R Arm: Upper={:.3}, Lower={:.3}, Hand={:.3}",
            upper_arm_r, lower_arm_r, hand_r
        );
    }
}
