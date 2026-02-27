use std::marker::PhantomData;

use ahash::AHashMap;
use bevy::{
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use bevy_mod_physx::{
    physx_sys::PxArticulationJointType,
    prelude::{self as bpx, *},
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
const UPPER_RIGHT_LEG_VERTICES: [usize; 4] = [4407, 4268, 4567, 4565];
const LOWER_RIGHT_LEG_VERTICES: [usize; 4] = [4662, 4664, 6385, 6375];
const UPPER_RIGHT_ARM_VERTICES: [usize; 4] = [1630, 1432, 3330, 3323];
const LOWER_RIGHT_ARM_VERTICES: [usize; 4] = [3412, 3877, 3552, 3906];
const UPPER_LEFT_ARM_VERTICES: [usize; 4] = [8302, 8120, 9998, 9991];
const LOWER_LEFT_ARM_VERTICES: [usize; 4] = [10080, 10542, 10220, 10571];
const UPPER_LEFT_LEG_VERTICES: [usize; 4] = [11025, 10898, 11185, 11183];
const LOWER_LEFT_LEG_VERTICES: [usize; 4] = [11280, 11282, 12982, 12972];
/// Use cuboids. Get dimensions from  2x, 2y, 2z
const RIGHT_HAND_VERTICES: [usize; 6] = [2776, 3189, 2119, 3909, 3247, 3650];
const RIGHT_FOOT_VERTICES: [usize; 6] = [6251, 6705, 4972, 5845, 6214, 6298];
const LEFT_HAND_VERTICES: [usize; 6] = [9444, 9857, 8787, 10574, 9915, 10318];
const LEFT_FOOT_VERTICES: [usize; 6] = [12848, 13301, 11590, 12442, 12811, 12895];

mod sealed {
    pub trait Sealed {}
}

pub trait ColliderType: sealed::Sealed {}

pub struct HitboxCollider;
impl Default for HitboxCollider {
    fn default() -> Self {
        Self
    }
}
impl sealed::Sealed for HitboxCollider {}
impl ColliderType for HitboxCollider {}

pub struct HurtboxCollider;
impl Default for HurtboxCollider {
    fn default() -> Self {
        Self
    }
}
impl sealed::Sealed for HurtboxCollider {}
impl ColliderType for HurtboxCollider {}

pub struct RagdollCollider;
impl Default for RagdollCollider {
    fn default() -> Self {
        Self
    }
}
impl sealed::Sealed for RagdollCollider {}
impl ColliderType for RagdollCollider {}

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
pub struct CharacterColliders<C: ColliderType + Send + Sync = HitboxCollider> {
    /// Physx collider filters/layers
    pub filter: ShapeFilterData,
    /// Subset of bones to create colliders for. None = all bones
    pub bones_subset: Option<Vec<CharacterColliderBone>>,
    collider_entities: AHashMap<CharacterColliderBone, Entity>,
    bone_entities: AHashMap<CharacterColliderBone, Entity>,
    _phantom: PhantomData<C>,
}

impl<C: ColliderType + Default + Send + Sync + 'static> CharacterColliders<C> {
    pub fn new(filter: ShapeFilterData, bones_subset: Option<Vec<CharacterColliderBone>>) -> Self {
        Self {
            filter,
            bones_subset,
            ..Default::default()
        }
    }
}

// Temp marker component - generic over collider type
#[derive(Component)]
pub(crate) struct NeedsColliders<C: ColliderType>(PhantomData<C>);

// Reusable physics material for colliders
#[derive(Hash, Eq, PartialEq, Clone, Resource)]
pub(crate) struct ColliderMaterial(Handle<bpx::Material>);

/*--- Systems ---*/
pub(crate) fn create_collider_physics_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<bpx::Material>>,
    mut physics: ResMut<Physics>,
) {
    let handle = materials.add(bpx::Material::new(&mut physics, 0., 0., 1.0));
    commands.insert_resource(ColliderMaterial(handle));
}

pub(crate) fn mark_entity_needs_colliders<C: ColliderType + Send + Sync + 'static>(
    trigger: On<Add, CharacterColliders<C>>,
    mut commands: Commands,
) {
    commands
        .entity(trigger.entity)
        .insert(NeedsColliders::<C>(PhantomData));
}

pub(crate) fn on_colliders_changed<C: ColliderType + Send + Sync + 'static>(
    mut colliders: Query<(Entity, &CharacterColliders<C>), Changed<CharacterColliders<C>>>,
    mut commands: Commands,
) {
    for (entity, collider) in colliders.iter_mut() {
        let target_bones: Vec<_> = match &collider.bones_subset {
            Some(bones) if !bones.is_empty() => bones.clone(),
            Some(_) => vec![],
            None => COLLIDERS.to_vec(),
        };

        let existing: Vec<_> = collider.collider_entities.keys().cloned().collect();
        let target_set: std::collections::HashSet<_> = target_bones.iter().collect();
        let existing_set: std::collections::HashSet<_> = existing.iter().collect();

        if existing_set == target_set {
            continue;
        }

        commands
            .entity(entity)
            .insert(NeedsColliders::<C>(PhantomData));
    }
}

fn get_collider_geometry(
    collider: CharacterColliderBone,
    helpers: &[Vec3],
    i_collider: usize,
    inv_bindposes: &AHashMap<&str, Transform>,
    collider_bone_map: &[&str; 15],
    geometries: &mut ResMut<Assets<Geometry>>,
) -> (Handle<Geometry>, Transform) {
    match collider {
        CharacterColliderBone::Head => get_head_collider(helpers, geometries),
        CharacterColliderBone::Chest | CharacterColliderBone::Pelvis => {
            let bone_name = collider_bone_map[i_collider];
            let inv_bindpose_rot = inv_bindposes[bone_name].rotation;
            get_midsection_collider(helpers, collider, geometries, inv_bindpose_rot)
        }
        CharacterColliderBone::UpperRightArm
        | CharacterColliderBone::UpperLeftArm
        | CharacterColliderBone::LowerRightArm
        | CharacterColliderBone::LowerLeftArm
        | CharacterColliderBone::UpperRightLeg
        | CharacterColliderBone::UpperLeftLeg
        | CharacterColliderBone::LowerRightLeg
        | CharacterColliderBone::LowerLeftLeg => get_limb_collider(helpers, collider, geometries),
        CharacterColliderBone::LeftHand
        | CharacterColliderBone::RightHand
        | CharacterColliderBone::LeftFoot
        | CharacterColliderBone::RightFoot => get_extremity_collider(helpers, collider, geometries),
    }
}

/// Create kinematic colliders (hitbox/hurtbox) - dynamic rigid body with Kinematic component
pub(crate) fn spawn_kinematic_colliders<C: ColliderType + Send + Sync + 'static>(
    mut needs_colliders: Query<
        (
            Entity,
            &CharacterShapeConfig,
            &RelatedEntities,
            &mut CharacterColliders<C>,
            &SkinnedMesh,
        ),
        (
            Without<FitSkeleton>,
            Without<SkeletalBone>,
            With<NeedsColliders<C>>,
        ),
    >,
    _global_transforms: Query<&GlobalTransform>,
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
        let model_to_world = Transform::from(
            _global_transforms
                .get(related.rig)
                .expect("Related rig should have a GlobalTransform")
                .clone(),
        );
        let rot_fix = Transform::from_rotation(MODEL_ROTATION_FIX);
        let model_to_world = rot_fix * model_to_world;

        let collider_bone_map = match rig_type {
            RigType::Default => DEFAULT_RIG_COLLIDER_BONE_NAMES,
            _ => continue,
        };

        let prefab = &prefabs[&shape_config.prefab];
        let helpers = prefab.get_helpers(&shape_config.prefab_morph_targets, &basemesh, &mh_morphs);
        let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes) else {
            continue;
        };
        let sk_cache = &skeleton_caches[&prefab.rig.rig_type];

        let bone_entities = sk_cache
            .bone_order
            .iter()
            .cloned()
            .zip(skm.joints.iter().cloned())
            .collect::<AHashMap<&str, Entity>>();

        let inv_bindposes_map = sk_cache
            .bone_order
            .iter()
            .cloned()
            .zip(inv_bindposes.iter().map(|m| Transform::from_matrix(*m)))
            .collect::<AHashMap<&str, Transform>>();

        let target_bones: Vec<CharacterColliderBone> = match &colliders.bones_subset {
            Some(bones) if !bones.is_empty() => bones.clone(),
            Some(_) => vec![],
            None => COLLIDERS.to_vec(),
        };

        let existing_bones: Vec<CharacterColliderBone> =
            colliders.collider_entities.keys().cloned().collect();
        let existing_set: std::collections::HashSet<_> = existing_bones.iter().collect();
        let target_set: std::collections::HashSet<_> = target_bones.iter().collect();

        if existing_set == target_set && !existing_set.is_empty() {
            commands
                .entity(character_entity)
                .remove::<NeedsColliders<C>>();
            continue;
        }

        for (_, entity) in colliders.collider_entities.drain() {
            commands.entity(entity).despawn();
        }
        colliders.bone_entities.clear();

        if target_bones.is_empty() {
            commands
                .entity(character_entity)
                .remove::<NeedsColliders<C>>();
            continue;
        }

        for collider in target_bones.iter() {
            let i_collider = COLLIDERS.iter().position(|&c| c == *collider).unwrap();
            let (geometry, collider_to_model) = get_collider_geometry(
                *collider,
                &helpers,
                i_collider,
                &inv_bindposes_map,
                &collider_bone_map,
                &mut geometries,
            );

            let joint_name = collider_bone_map[i_collider];
            let model_to_joint = inv_bindposes_map[joint_name] * rot_fix;
            let collider_to_joint = model_to_joint * collider_to_model;
            let collider_to_world = model_to_world * collider_to_model;

            let collider_entity = commands
                .spawn((
                    RigidBody::Dynamic,
                    Kinematic::new(collider_to_world),
                    Transform::IDENTITY,
                    *collider,
                    Shape {
                        geometry,
                        material: collider_mat.0.clone(),
                        ..Default::default()
                    },
                    PoseOffset(collider_to_joint),
                    colliders.filter.clone(),
                ))
                .id();

            colliders
                .collider_entities
                .insert(*collider, collider_entity);
            colliders
                .bone_entities
                .insert(*collider, bone_entities[joint_name]);
        }

        commands
            .entity(character_entity)
            .remove::<NeedsColliders<C>>();
    }
}

/// Create ragdoll colliders - articulation links with joints
pub(crate) fn spawn_ragdoll_colliders(
    mut needs_colliders: Query<
        (
            Entity,
            &CharacterShapeConfig,
            &RelatedEntities,
            &mut CharacterColliders<RagdollCollider>,
            &SkinnedMesh,
        ),
        (
            Without<FitSkeleton>,
            Without<SkeletalBone>,
            With<NeedsColliders<RagdollCollider>>,
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
            _ => todo!("impl more rigs"),
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

        let inv_bindposes_map = sk_cache
            .bone_order
            .iter()
            .cloned()
            .zip(inv_bindposes.iter().map(|m| Transform::from_matrix(*m)))
            .collect::<AHashMap<&str, Transform>>();

        let mut collider_to_world_transforms = AHashMap::default();

        let target_bones: Vec<CharacterColliderBone> = match &colliders.bones_subset {
            Some(bones) if !bones.is_empty() => bones.clone(),
            Some(_) => vec![],
            None => COLLIDERS.to_vec(),
        };

        // Despawn any existing colliders first
        for (_, entity) in colliders.collider_entities.drain() {
            commands.entity(entity).despawn();
        }
        colliders.bone_entities.clear();

        // If no bones to create, we're done
        if target_bones.is_empty() {
            commands
                .entity(character_entity)
                .remove::<NeedsColliders<RagdollCollider>>();
            return;
        }

        // Process colliders in COLLIDERS order to ensure parents are handled before children
        for collider in COLLIDERS.iter() {
            if !target_bones.contains(collider) {
                continue;
            }
            let i_collider = COLLIDERS.iter().position(|&c| c == *collider).unwrap();

            let (geometry, collider_bind_to_model) = get_collider_geometry(
                *collider,
                &helpers,
                i_collider,
                &inv_bindposes_map,
                &collider_bone_map,
                &mut geometries,
            );

            let bone_name = collider_bone_map[i_collider];
            let Ok(model_to_world) = global_transforms.get(related.rig) else {
                continue;
            };
            let model_to_world = Transform::from(model_to_world.clone());
            let rot_fix = Transform::from_rotation(MODEL_ROTATION_FIX);
            let model_to_world = rot_fix * model_to_world;

            let model_to_joint_bind = inv_bindposes_map[bone_name] * rot_fix;
            let collider_to_joint = model_to_joint_bind * collider_bind_to_model;

            let world_to_model = Transform::from_matrix(model_to_world.to_matrix().inverse());
            let Ok(joint_to_world) = global_transforms.get(bone_entities[bone_name]) else {
                continue;
            };
            let joint_to_world = Transform::from(joint_to_world.clone());
            let world_to_joint = Transform::from_matrix(joint_to_world.to_matrix().inverse());
            let collider_to_world = joint_to_world * collider_to_joint;

            collider_to_world_transforms.insert(*collider, collider_to_world );

            let collider_entity = commands
                .spawn((
                    RigidBody::ArticulationLink,
                    collider_to_world,
                    *collider,
                    Shape {
                        geometry,
                        material: collider_mat.0.clone(),
                        ..Default::default()
                    },
                    PoseOffset(collider_to_joint),
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
                    ..Default::default()
                });
            } else {
                let parent_collider = get_collider_parent(*collider).unwrap();
                let parent_collider_to_model = collider_to_world_transforms[&parent_collider];
                let parent_collider_to_joint = world_to_joint * parent_collider_to_model;

                let parent = colliders.collider_entities[&parent_collider];
                let child_pose = collider_to_joint;

                let child_pose = Transform::from_matrix(child_pose.to_matrix().inverse());
                let parent_pose = Transform::from_matrix(parent_collider_to_joint.to_matrix().inverse());

                commands
                    .entity(collider_entity)
                    .insert(get_articulation_joint(
                        *collider,
                        parent,
                        parent_pose,
                        child_pose,
                    ));
            }
        }

        commands
            .entity(character_entity)
            .remove::<NeedsColliders<RagdollCollider>>();
    }
}

fn get_articulation_joint(
    collider: CharacterColliderBone,
    parent: Entity,
    parent_pose: Transform,
    child_pose: Transform,
) -> ArticulationJoint {
    match collider {
        CharacterColliderBone::Pelvis => ArticulationJoint {
            parent,
            parent_pose,
            child_pose,
            joint_type: PxArticulationJointType::Spherical,
            motion_swing1: ArticulationJointMotion::Limited {
                min: -0.3,
                max: 0.3,
            },
            motion_swing2: ArticulationJointMotion::Limited {
                min: -0.3,
                max: 0.3,
            },
            motion_twist: ArticulationJointMotion::Limited {
                min: -0.3,
                max: 0.3,
            },
            friction_coefficient: 1.0,
            ..default()
        },
        CharacterColliderBone::Chest => ArticulationJoint {
            parent,
            parent_pose,
            child_pose,
            joint_type: PxArticulationJointType::Spherical,
            motion_swing1: ArticulationJointMotion::Limited {
                min: -0.5,
                max: 0.5,
            },
            motion_swing2: ArticulationJointMotion::Limited {
                min: -0.3,
                max: 0.3,
            },
            motion_twist: ArticulationJointMotion::Limited {
                min: -0.3,
                max: 0.3,
            },
            friction_coefficient: 1.0,
            ..default()
        },
        CharacterColliderBone::Head => ArticulationJoint {
            parent,
            parent_pose,
            child_pose,
            joint_type: PxArticulationJointType::Spherical,
            motion_swing1: ArticulationJointMotion::Limited {
                min: -0.5,
                max: 0.5,
            },
            motion_swing2: ArticulationJointMotion::Limited {
                min: -0.4,
                max: 0.4,
            },
            motion_twist: ArticulationJointMotion::Limited {
                min: -0.3,
                max: 0.3,
            },
            friction_coefficient: 1.0,
            ..default()
        },
        CharacterColliderBone::UpperRightArm | CharacterColliderBone::UpperLeftArm => {
            ArticulationJoint {
                parent,
                parent_pose,
                child_pose,
                joint_type: PxArticulationJointType::Spherical,
                motion_swing1: ArticulationJointMotion::Limited {
                    min: -1.5,
                    max: 1.5,
                },
                motion_swing2: ArticulationJointMotion::Limited {
                    min: -1.5,
                    max: 1.5,
                },
                motion_twist: ArticulationJointMotion::Limited {
                    min: -0.5,
                    max: 0.5,
                },
                friction_coefficient: 1.0,
                ..default()
            }
        }
        CharacterColliderBone::LowerRightArm | CharacterColliderBone::LowerLeftArm => {
            ArticulationJoint {
                parent,
                parent_pose,
                child_pose,
                joint_type: PxArticulationJointType::Revolute,
                motion_twist: ArticulationJointMotion::Limited { min: 0.0, max: 2.5 },
                friction_coefficient: 1.0,
                ..default()
            }
        }
        CharacterColliderBone::UpperRightLeg | CharacterColliderBone::UpperLeftLeg => {
            ArticulationJoint {
                parent,
                parent_pose,
                child_pose,
                joint_type: PxArticulationJointType::Spherical,
                motion_swing1: ArticulationJointMotion::Limited {
                    min: -1.5,
                    max: 1.5,
                },
                motion_swing2: ArticulationJointMotion::Limited {
                    min: -0.5,
                    max: 0.5,
                },
                motion_twist: ArticulationJointMotion::Limited {
                    min: -0.3,
                    max: 0.3,
                },
                friction_coefficient: 1.0,
                ..default()
            }
        }
        CharacterColliderBone::LowerRightLeg | CharacterColliderBone::LowerLeftLeg => {
            ArticulationJoint {
                parent,
                parent_pose,
                child_pose,
                joint_type: PxArticulationJointType::Revolute,
                motion_twist: ArticulationJointMotion::Limited {
                    min: -1.5,
                    max: 0.0,
                },
                friction_coefficient: 1.0,
                ..default()
            }
        }
        CharacterColliderBone::LeftHand | CharacterColliderBone::RightHand => ArticulationJoint {
            parent,
            parent_pose,
            child_pose,
            joint_type: PxArticulationJointType::Spherical,
            motion_swing1: ArticulationJointMotion::Limited {
                min: -0.8,
                max: 0.8,
            },
            motion_swing2: ArticulationJointMotion::Limited {
                min: -0.8,
                max: 0.8,
            },
            motion_twist: ArticulationJointMotion::Limited {
                min: -0.8,
                max: 0.8,
            },
            friction_coefficient: 1.0,
            ..default()
        },
        CharacterColliderBone::LeftFoot | CharacterColliderBone::RightFoot => ArticulationJoint {
            parent,
            parent_pose,
            child_pose,
            joint_type: PxArticulationJointType::Spherical,
            motion_swing1: ArticulationJointMotion::Limited {
                min: -0.5,
                max: 0.5,
            },
            motion_swing2: ArticulationJointMotion::Limited {
                min: -0.3,
                max: 0.3,
            },
            motion_twist: ArticulationJointMotion::Limited {
                min: -0.2,
                max: 0.2,
            },
            friction_coefficient: 1.0,
            ..default()
        },
    }
}

/// Syncs kinematic colliders to bone transforms
pub(crate) fn sync_colliders<C: ColliderType + Send + Sync + 'static>(
    characters: Query<&CharacterColliders<C>>,
    global_transforms: Query<
        &GlobalTransform,
        Or<(With<SkeletalBone>, Without<CharacterColliderBone>)>,
    >,
    mut collider_transforms: Query<
        (&mut Kinematic, &GlobalTransform, &PoseOffset),
        (Without<SkeletalBone>, With<CharacterColliderBone>),
    >,
) {
    for colliders in characters.iter() {
        if colliders.collider_entities.is_empty() {
            continue;
        };

        // Iterate over all bones that have collider entities and sync them
        for (collider, collider_entity) in colliders.collider_entities.iter() {
            let Ok((mut collider_transform, _, offset)) =
                collider_transforms.get_mut(*collider_entity)
            else {
                continue;
            };
            let bone_entity = colliders.bone_entities[collider];
            let Ok(joint_to_world) = global_transforms.get(bone_entity) else {
                continue;
            };
            let collider_to_joint = **offset;
            let collider_to_world = Transform::from(joint_to_world.clone()) * collider_to_joint;

            collider_transform.target = collider_to_world;
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
        CharacterColliderBone::LowerLeftArm => LOWER_LEFT_ARM_VERTICES,
        CharacterColliderBone::LowerRightArm => LOWER_RIGHT_ARM_VERTICES,
        CharacterColliderBone::UpperLeftArm => UPPER_LEFT_ARM_VERTICES,
        CharacterColliderBone::UpperRightArm => UPPER_RIGHT_ARM_VERTICES,
        CharacterColliderBone::LowerLeftLeg => LOWER_LEFT_LEG_VERTICES,
        CharacterColliderBone::LowerRightLeg => LOWER_RIGHT_LEG_VERTICES,
        CharacterColliderBone::UpperLeftLeg => UPPER_LEFT_LEG_VERTICES,
        CharacterColliderBone::UpperRightLeg => UPPER_RIGHT_LEG_VERTICES,
        _ => unimplemented!("wrong joint input"),
    };
    let verts = ref_verts.iter().map(|&i| helpers[i]).collect::<Vec<_>>();

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
        CharacterColliderBone::LeftHand => LEFT_HAND_VERTICES,
        CharacterColliderBone::RightHand => RIGHT_HAND_VERTICES,
        CharacterColliderBone::LeftFoot => LEFT_FOOT_VERTICES,
        CharacterColliderBone::RightFoot => RIGHT_FOOT_VERTICES,
        _ => unimplemented!("wrong joint input"),
    };
    let verts = ref_verts.iter().map(|&i| helpers[i]).collect::<Vec<_>>();

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
