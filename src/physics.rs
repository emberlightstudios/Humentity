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

/// Collider physics material resource
#[derive(Resource)]
pub(crate) struct ColliderMaterial {
    pub friction: Friction,
    pub restitution: Restitution,
}

impl Default for ColliderMaterial {
    fn default() -> Self {
        Self {
            friction: Friction::new(0.0),
            restitution: Restitution::new(0.0),
        }
    }
}

/*--- Systems ---*/
pub(crate) fn mark_entity_needs_colliders(
    trigger: On<Add, CharacterColliders>,
    mut commands: Commands,
) {
    commands.entity(trigger.entity).insert(NeedsColliders);
}

pub(crate) fn create_collider_physics_material(mut commands: Commands) {
    commands.insert_resource(ColliderMaterial::default());
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
            let (collider_shape, collider_to_backwards_model) = match collider {
                CharacterColliderBone::Head => get_head_collider(&helpers),
                CharacterColliderBone::Chest | CharacterColliderBone::Pelvis => {
                    let bone_name = collider_bone_map[i_collider];
                    let inv_bindpose_rot = inv_bindposes[bone_name].rotation;
                    get_midsection_collider(&helpers, *collider, inv_bindpose_rot)
                }
                CharacterColliderBone::UpperRightArm
                | CharacterColliderBone::UpperLeftArm
                | CharacterColliderBone::LowerRightArm
                | CharacterColliderBone::LowerLeftArm
                | CharacterColliderBone::UpperRightLeg
                | CharacterColliderBone::UpperLeftLeg
                | CharacterColliderBone::LowerRightLeg
                | CharacterColliderBone::LowerLeftLeg => get_limb_collider(&helpers, *collider),
                CharacterColliderBone::LeftHand
                | CharacterColliderBone::RightHand
                | CharacterColliderBone::LeftFoot
                | CharacterColliderBone::RightFoot => get_extremity_collider(&helpers, *collider),
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

            // Pelvis is the root - make it kinematic to anchor the ragdoll
            let rigid_body = if *collider == CharacterColliderBone::Pelvis {
                RigidBody::Kinematic
            } else {
                RigidBody::Dynamic
            };

            let collider_entity = commands
                .spawn((
                    rigid_body,
                    collider_to_world,
                    *collider,
                    Visibility::default(),
                    collider_shape,
                    collider_mat.friction,
                    collider_mat.restitution,
                    PoseOffset(collider_to_bone),
                    Mass(1000.0),
                ))
                .id();

            colliders
                .collider_entities
                .insert(*collider, collider_entity);
            colliders
                .bone_entities
                .insert(*collider, bone_entities[bone_name]);
        }

        // Spawn joints as separate entities in Avian
        for (i_collider, collider) in COLLIDERS.iter().enumerate() {
            if *collider == CharacterColliderBone::Pelvis {
                continue;
            }

            let parent_collider = get_collider_parent(*collider).unwrap();
            let child_entity = colliders.collider_entities[collider];
            let parent_entity = colliders.collider_entities[&parent_collider];

            // Get actual collider positions (not bone positions)
            let child_collider_transform = collider_to_world_transforms[collider];
            let parent_collider_transform = collider_to_world_transforms[&parent_collider];

            // Anchor at midpoint between parent and child colliders
            let anchor = (parent_collider_transform.translation
                + child_collider_transform.translation)
                / 2.0;

            spawn_ragdoll_joint(
                &mut commands,
                *collider,
                parent_entity,
                child_entity,
                anchor,
            );

            info!(
                "[RAGDOLL] {:?} joint: parent={:?}, child={:?}, anchor={:?}",
                collider, parent_entity, child_entity, anchor
            );
        }

        commands.entity(character_entity).remove::<NeedsColliders>();
    }
}

#[derive(Component)]
struct RagdollJoint(CharacterColliderBone);

fn spawn_ragdoll_joint(
    commands: &mut Commands,
    collider: CharacterColliderBone,
    parent: Entity,
    child: Entity,
    anchor: Vec3,
) {
    match collider {
        // Elbows - Revolute (hinge)
        CharacterColliderBone::LowerRightArm | CharacterColliderBone::LowerLeftArm => {
            commands.spawn((
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_hinge_axis(Vec3::NEG_Z),
                JointCollisionDisabled,
                RagdollJoint(collider),
            ));
        }
        // Knees - Revolute (hinge)
        CharacterColliderBone::LowerRightLeg | CharacterColliderBone::LowerLeftLeg => {
            commands.spawn((
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_hinge_axis(Vec3::Z),
                JointCollisionDisabled,
                RagdollJoint(collider),
            ));
        }
        // Everything else - Spherical (ball socket)
        _ => {
            commands.spawn((
                SphericalJoint::new(parent, child).with_anchor(anchor),
                JointCollisionDisabled,
                RagdollJoint(collider),
            ));
        }
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
fn get_head_collider(helpers: &[Vec3]) -> (Collider, Transform) {
    let center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let radius = (helpers[HEAD_VERTICES[0]] - center).length();
    (
        Collider::sphere(radius),
        Transform::from_translation(MODEL_ROTATION_FIX * center),
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
        Collider::cuboid(
            (xmax - xmin) / 2.0,
            (ymax - ymin) / 2.0,
            (zmax - zmin) / 2.0,
        ),
        Transform::from_translation(MODEL_ROTATION_FIX * center).with_rotation(
            MODEL_ROTATION_FIX *
                inv_bindpose_rot * //.inverse() *   // Why inverse bindpose, not bindpose? idk
                MODEL_ROTATION_FIX.inverse(),
        ),
    )
}

fn get_limb_collider(helpers: &[Vec3], joint: CharacterColliderBone) -> (Collider, Transform) {
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
    let length = (p1 - p2).length() - r * 2.0;
    let c = 0.5 * (p1 + p2);
    let dir = (p1 - p2).normalize();
    let up = dir.cross(Vec3::NEG_Z).normalize();
    let fwd = dir.cross(up);

    (
        Collider::capsule(r, length),
        Transform::from_translation(MODEL_ROTATION_FIX * c).with_rotation(
            MODEL_ROTATION_FIX
                * Quat::from_mat3(&Mat3::from_cols(dir, up, fwd))
                * MODEL_ROTATION_FIX.inverse(),
        ),
    )
}

fn get_extremity_collider(helpers: &[Vec3], joint: CharacterColliderBone) -> (Collider, Transform) {
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

    let center = verts.iter().sum::<Vec3>() / verts.len() as f32;

    let x_axis = verts[1] - verts[0];
    let y_axis = (verts[3] - verts[2]).normalize();
    let z_axis = x_axis.cross(y_axis).normalize();
    let x_axis = y_axis.cross(z_axis);

    (
        Collider::cuboid(x / 2.0, y / 2.0, z / 2.0),
        Transform::from_translation(MODEL_ROTATION_FIX * center).with_rotation(
            MODEL_ROTATION_FIX
                * Quat::from_mat3(&Mat3::from_cols(x_axis, y_axis, z_axis))
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
