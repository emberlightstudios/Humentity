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
    prelude::{BaseMesh, CharacterShape, CharacterShapeAsset},
    rigs::{RigData, SkeletalBone},
    spawn_skeleton::{SkeletonLodMap, SkeletonsReady},
    template::CharacterTemplate,
};

use super::{
    get_collider_parent, ColliderBone, COLLIDERS, DEFAULT_RIG_COLLIDER_BONE_NAMES, HEAD_VERTICES,
    LEFT_FOOT_VERTICES, LEFT_HAND_VERTICES, LOWER_LEFT_ARM_VERTICES, LOWER_LEFT_LEG_VERTICES,
    LOWER_RIGHT_ARM_VERTICES, LOWER_RIGHT_LEG_VERTICES, PELVIS_VERTICES, RIGHT_FOOT_VERTICES,
    RIGHT_HAND_VERTICES, TORSO_VERTICES, UPPER_LEFT_ARM_VERTICES, UPPER_LEFT_LEG_VERTICES,
    UPPER_RIGHT_ARM_VERTICES, UPPER_RIGHT_LEG_VERTICES,
};

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

/// Tracks which character a physx entity belongs to
#[derive(Component)]
#[relationship(relationship_target = ColliderList)]
pub struct ColliderForCharacter(pub Entity);

/// Tracks which colliders belong to this character
#[derive(Component)]
#[relationship_target(relationship = ColliderForCharacter)]
pub struct ColliderList(Vec<Entity>);

/// Provides body colliders for characters
#[derive(Component, Default)]
pub struct PhysxCharacterColliders<C: ColliderType + Send + Sync = HitboxCollider> {
    /// Physx collider filters/layers
    pub filter: ShapeFilterData,
    /// Subset of bones to create colliders for. None = all bones
    pub bones_subset: Option<Vec<ColliderBone>>,
    /// Map from ColliderBone to collider entity. Remove entries here to detach limbs.
    pub collider_entities: AHashMap<ColliderBone, Entity>,
    /// Map from ColliderBone to the skeletal bone entity it tracks.
    pub bone_entities: AHashMap<ColliderBone, Entity>,
    _phantom: PhantomData<C>,
}

impl<C: ColliderType + Default + Send + Sync + 'static> PhysxCharacterColliders<C> {
    pub fn new(filter: ShapeFilterData, bones_subset: Option<Vec<ColliderBone>>) -> Self {
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

/// Resource for configuring the default ragdoll collider filter.
/// Insert this resource to override the default filter used by `auto_add_ragdoll_colliders`.
#[derive(Resource, Clone)]
pub struct RagdollColliderFilter(pub ShapeFilterData);

impl Default for RagdollColliderFilter {
    fn default() -> Self {
        Self(ShapeFilterData {
            simulation_filter_data: [1 << 1, 1 << 1, 0, 0],
            ..default()
        })
    }
}

/// Automatically adds `CharacterColliders<RagdollCollider>` to characters
/// once their skeletons have been fitted (SkeletonsReady present).
pub(crate) fn auto_add_ragdoll_colliders(
    characters: Query<
        Entity,
        (
            With<CharacterShape>,
            With<SkeletonsReady>,
            Without<PhysxCharacterColliders<RagdollCollider>>,
        ),
    >,
    filter: Option<Res<RagdollColliderFilter>>,
    mut commands: Commands,
) {
    let filter = filter.as_deref().cloned().unwrap_or_default();
    for entity in characters.iter() {
        commands
            .entity(entity)
            .insert(PhysxCharacterColliders::<RagdollCollider>::new(
                filter.0,
                Some(vec![]),
            ));
    }
}

/*--- Systems ---*/
pub(crate) fn create_collider_physics_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<bpx::Material>>,
    mut physics: ResMut<Physics>,
) {
    let handle = materials.add(bpx::Material::new(&mut physics, 0.5, 0.5, 0.3));
    commands.insert_resource(ColliderMaterial(handle));
}

pub(crate) fn mark_entity_needs_colliders<C: ColliderType + Send + Sync + 'static>(
    trigger: On<Add, PhysxCharacterColliders<C>>,
    mut commands: Commands,
) {
    commands
        .entity(trigger.entity)
        .insert(NeedsColliders::<C>(PhantomData));
}

pub(crate) fn on_colliders_changed<C: ColliderType + Send + Sync + 'static>(
    mut colliders: Query<
        (Entity, &mut PhysxCharacterColliders<C>),
        Changed<PhysxCharacterColliders<C>>,
    >,
    mut commands: Commands,
) {
    for (entity, mut collider) in colliders.iter_mut() {
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

        for (_, child_entity) in collider.collider_entities.drain() {
            commands.entity(child_entity).remove::<RigidBody>();
            commands.entity(child_entity).remove::<Kinematic>();
            commands.entity(child_entity).despawn();
        }
        collider.bone_entities.clear();

        commands
            .entity(entity)
            .insert(NeedsColliders::<C>(PhantomData));
    }
}

fn get_collider_geometry(
    collider: ColliderBone,
    helpers: &[Vec3],
    i_collider: usize,
    inv_bindposes: &AHashMap<&str, Transform>,
    collider_bone_map: &[&str; 15],
    geometries: &mut ResMut<Assets<Geometry>>,
) -> (Handle<Geometry>, Transform) {
    match collider {
        ColliderBone::Head => get_head_collider(helpers, geometries),
        ColliderBone::Chest | ColliderBone::Pelvis => {
            let bone_name = collider_bone_map[i_collider];
            let inv_bindpose_rot = inv_bindposes[bone_name].rotation;
            get_midsection_collider(helpers, collider, geometries, inv_bindpose_rot)
        }
        ColliderBone::UpperRightArm
        | ColliderBone::UpperLeftArm
        | ColliderBone::LowerRightArm
        | ColliderBone::LowerLeftArm
        | ColliderBone::UpperRightLeg
        | ColliderBone::UpperLeftLeg
        | ColliderBone::LowerRightLeg
        | ColliderBone::LowerLeftLeg => get_limb_collider(helpers, collider, geometries),
        ColliderBone::LeftHand
        | ColliderBone::RightHand
        | ColliderBone::LeftFoot
        | ColliderBone::RightFoot => get_extremity_collider(helpers, collider, geometries),
    }
}

/// Create kinematic colliders (hitbox/hurtbox) - dynamic rigid body with Kinematic component
pub(crate) fn spawn_kinematic_colliders<C: ColliderType + Send + Sync + 'static>(
    mut needs_colliders: Query<
        (
            Entity,
            &CharacterShape,
            &mut PhysxCharacterColliders<C>,
            &SkeletonLodMap,
        ),
        (
            With<SkeletonsReady>,
            Without<SkeletalBone>,
            With<NeedsColliders<C>>,
        ),
    >,
    _global_transforms: Query<&GlobalTransform>,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    basemesh: Res<BaseMesh>,
    mh_morphs: Res<MakeHumanMorphs>,
    collider_mat: Res<ColliderMaterial>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    rig_data: Res<RigData>,
    mut geometries: ResMut<Assets<Geometry>>,
    skeleton_skins: Query<&SkinnedMesh, (Without<Mesh3d>, With<ChildOf>)>,
    children_query: Query<&Children>,
    mut commands: Commands,
) {
    for (character_entity, character_shape, mut colliders, lod_map) in needs_colliders.iter_mut() {
        // Resolve SkinnedMesh from the highest-detail (LOD 0) skeleton
        let Some(&lod0_entity) = lod_map.0.get(&0) else {
            continue;
        };
        let mut skm: Option<SkinnedMesh> = None;
        for child in children_query.iter_descendants(lod0_entity) {
            if let Ok(s) = skeleton_skins.get(child) {
                skm = Some(s.clone());
                break;
            }
        }
        let Some(skm) = skm else {
            continue;
        };
        let Some(asset) = shape_assets.get(&character_shape.0) else {
            continue;
        };
        let Some(template) = templates.get(&asset.template) else {
            continue;
        };
        let collider_bone_map = DEFAULT_RIG_COLLIDER_BONE_NAMES;
        let helpers = match template.get_helpers(
            &asset.template_morph_targets,
            &basemesh.vertices,
            &mh_morphs,
        ) {
            Ok(h) => h,
            Err(e) => {
                error!("Failed to compute morph helpers for colliders: {}", e);
                continue;
            }
        };
        let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes) else {
            continue;
        };
        let Some(rig_spec) = rig_data.0.as_ref() else {
            continue;
        };
        let reference_rig = &rig_spec.reference_rig;

        let bone_entities = reference_rig
            .bone_names
            .iter()
            .cloned()
            .zip(skm.joints.iter().cloned())
            .collect::<AHashMap<&str, Entity>>();

        let inv_bindposes_map = reference_rig
            .bone_names
            .iter()
            .cloned()
            .zip(inv_bindposes.iter().map(|m| Transform::from_matrix(*m)))
            .collect::<AHashMap<&str, Transform>>();

        let target_bones: Vec<ColliderBone> = match &colliders.bones_subset {
            Some(bones) if !bones.is_empty() => bones.clone(),
            Some(_) => vec![],
            None => COLLIDERS.to_vec(),
        };

        let existing_bones: Vec<ColliderBone> =
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
            let model_to_joint = inv_bindposes_map[joint_name];
            let collider_to_joint = model_to_joint * collider_to_model;
            let collider_to_world = Transform::IDENTITY * collider_to_model;

            let collider_entity = commands
                .spawn((
                    Name::from(format!("KinematicCollider_{:?}", collider)),
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
                    colliders.filter,
                    ColliderForCharacter(character_entity),
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
            &CharacterShape,
            &mut PhysxCharacterColliders<RagdollCollider>,
            &SkeletonLodMap,
        ),
        (
            With<SkeletonsReady>,
            Without<SkeletalBone>,
            With<NeedsColliders<RagdollCollider>>,
        ),
    >,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    global_transforms: Query<&GlobalTransform>,
    templates: Res<Assets<CharacterTemplate>>,
    basemesh: Res<BaseMesh>,
    mh_morphs: Res<MakeHumanMorphs>,
    collider_mat: Res<ColliderMaterial>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    rig_data: Res<RigData>,
    mut geometries: ResMut<Assets<Geometry>>,
    skeleton_skins: Query<&SkinnedMesh, (Without<Mesh3d>, With<ChildOf>)>,
    children_query: Query<&Children>,
    mut commands: Commands,
) {
    for (character_entity, character_shape, mut colliders, lod_map) in needs_colliders.iter_mut() {
        // Resolve SkinnedMesh from the highest-detail (LOD 0) skeleton
        let Some(&lod0_entity) = lod_map.0.get(&0) else {
            continue;
        };
        let mut skm: Option<SkinnedMesh> = None;
        for child in children_query.iter_descendants(lod0_entity) {
            if let Ok(s) = skeleton_skins.get(child) {
                skm = Some(s.clone());
                break;
            }
        }
        let Some(skm) = skm else {
            continue;
        };
        let Some(asset) = shape_assets.get(&character_shape.0) else {
            continue;
        };
        let Some(template) = templates.get(&asset.template) else {
            continue;
        };
        let collider_bone_map = DEFAULT_RIG_COLLIDER_BONE_NAMES;
        let helpers = match template.get_helpers(
            &asset.template_morph_targets,
            &basemesh.vertices,
            &mh_morphs,
        ) {
            Ok(h) => h,
            Err(e) => {
                error!("Failed to compute morph helpers for ragdoll: {}", e);
                return;
            }
        };
        let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes) else {
            return;
        };
        let Some(rig_spec) = rig_data.0.as_ref() else {
            return;
        };
        let reference_rig = &rig_spec.reference_rig;

        let bone_entities = reference_rig
            .bone_names
            .iter()
            .cloned()
            .zip(skm.joints.iter().cloned())
            .collect::<AHashMap<&str, Entity>>();

        let inv_bindposes_map = reference_rig
            .bone_names
            .iter()
            .cloned()
            .zip(inv_bindposes.iter().map(|m| Transform::from_matrix(*m)))
            .collect::<AHashMap<&str, Transform>>();

        let mut collider_to_world_transforms = AHashMap::default();

        let target_bones: Vec<ColliderBone> = match &colliders.bones_subset {
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
            let model_to_joint_bind = inv_bindposes_map[bone_name];
            let collider_to_joint = model_to_joint_bind * collider_bind_to_model;

            let Ok(joint_to_world) = global_transforms.get(bone_entities[bone_name]) else {
                continue;
            };
            let joint_to_world = Transform::from(*joint_to_world);
            let world_to_joint = Transform::from_matrix(joint_to_world.to_matrix().inverse());
            let collider_to_world = joint_to_world * collider_to_joint;

            collider_to_world_transforms.insert(*collider, collider_to_world);

            let collider_entity = commands
                .spawn((
                    Name::from(format!("Ragdoll_{:?}", collider)),
                    RigidBody::ArticulationLink,
                    collider_to_world,
                    *collider,
                    Shape {
                        geometry,
                        material: collider_mat.0.clone(),
                        ..Default::default()
                    },
                    PoseOffset(collider_to_joint),
                    colliders.filter,
                    MassProperties::density(1000.),
                    ColliderForCharacter(character_entity),
                ))
                .id();

            colliders
                .collider_entities
                .insert(*collider, collider_entity);
            colliders
                .bone_entities
                .insert(*collider, bone_entities[bone_name]);

            if *collider == ColliderBone::Pelvis {
                commands.entity(collider_entity).insert(ArticulationRoot {
                    fix_base: false,
                    ..Default::default()
                });
            } else {
                let parent_collider = get_collider_parent(*collider).unwrap();
                if !colliders.collider_entities.contains_key(&parent_collider) {
                    commands.entity(collider_entity).insert(ArticulationRoot {
                        fix_base: false,
                        ..Default::default()
                    });
                } else {
                    let parent_collider_to_model = collider_to_world_transforms[&parent_collider];
                    let parent_collider_to_joint = world_to_joint * parent_collider_to_model;

                    let parent = colliders.collider_entities[&parent_collider];
                    let child_pose = collider_to_joint;

                    let child_pose = Transform::from_matrix(child_pose.to_matrix().inverse());
                    let parent_pose =
                        Transform::from_matrix(parent_collider_to_joint.to_matrix().inverse());

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
        }

        commands
            .entity(character_entity)
            .remove::<NeedsColliders<RagdollCollider>>();
    }
}

fn get_articulation_joint(
    collider: ColliderBone,
    parent: Entity,
    parent_pose: Transform,
    child_pose: Transform,
) -> ArticulationJoint {
    match collider {
        ColliderBone::Pelvis => ArticulationJoint {
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
        ColliderBone::Chest => ArticulationJoint {
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
        ColliderBone::Head => ArticulationJoint {
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
        ColliderBone::UpperRightArm | ColliderBone::UpperLeftArm => ArticulationJoint {
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
            friction_coefficient: 0.05,
            ..default()
        },
        ColliderBone::LowerRightArm | ColliderBone::LowerLeftArm => ArticulationJoint {
            parent,
            parent_pose,
            child_pose,
            joint_type: PxArticulationJointType::Revolute,
            motion_twist: ArticulationJointMotion::Limited { min: 0.0, max: 2.5 },
            friction_coefficient: 0.05,
            ..default()
        },
        ColliderBone::UpperRightLeg | ColliderBone::UpperLeftLeg => ArticulationJoint {
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
        },
        ColliderBone::LowerRightLeg | ColliderBone::LowerLeftLeg => ArticulationJoint {
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
        },
        ColliderBone::LeftHand | ColliderBone::RightHand => ArticulationJoint {
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
        ColliderBone::LeftFoot | ColliderBone::RightFoot => ArticulationJoint {
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
    characters: Query<&PhysxCharacterColliders<C>>,
    global_transforms: Query<&GlobalTransform, Or<(With<SkeletalBone>, Without<ColliderBone>)>>,
    mut collider_transforms: Query<
        (&mut Kinematic, &GlobalTransform, &PoseOffset),
        (Without<SkeletalBone>, With<ColliderBone>),
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
            let collider_to_world = Transform::from(*joint_to_world) * collider_to_joint;

            collider_transform.target = collider_to_world;
        }
    }
}

/// Syncs skeletal bones to ragdoll collider transforms when ragdoll is active.
pub(crate) fn sync_skeleton_to_ragdoll(
    characters: Query<&PhysxCharacterColliders<RagdollCollider>>,
    collider_transforms: Query<
        (&Transform, &PoseOffset),
        (With<ColliderBone>, Without<SkeletalBone>),
    >,
    mut bones: Query<
        (&mut Transform, Option<&ChildOf>),
        (With<SkeletalBone>, Without<ColliderBone>),
    >,
    global_transforms: Query<&GlobalTransform>,
) {
    for colliders in characters.iter() {
        if colliders.collider_entities.is_empty() {
            continue;
        }

        let ragdoll_active = !matches!(
            &colliders.bones_subset,
            Some(active_bones) if active_bones.is_empty()
        );
        if !ragdoll_active {
            continue;
        }

        let mut desired_joint_world = AHashMap::<ColliderBone, Transform>::default();
        for (collider_bone, collider_entity) in colliders.collider_entities.iter() {
            let Ok((collider_to_world, collider_to_joint)) =
                collider_transforms.get(*collider_entity)
            else {
                continue;
            };

            let joint_to_world = *collider_to_world
                * Transform::from_matrix(collider_to_joint.to_matrix().inverse());
            desired_joint_world.insert(*collider_bone, joint_to_world);
        }

        let mut applied_joint_world = AHashMap::<Entity, Transform>::default();
        for collider_bone in COLLIDERS {
            let Some(&joint_to_world) = desired_joint_world.get(&collider_bone) else {
                continue;
            };
            let Some(&bone_entity) = colliders.bone_entities.get(&collider_bone) else {
                continue;
            };

            let Ok((mut local_transform, parent)) = bones.get_mut(bone_entity) else {
                continue;
            };

            let local = if let Some(parent) = parent {
                if let Some(parent_to_world) = applied_joint_world.get(&parent.parent()) {
                    Transform::from_matrix(parent_to_world.to_matrix().inverse()) * joint_to_world
                } else if let Ok(parent_to_world) = global_transforms.get(parent.parent()) {
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

/*--- Utility functions ---*/
fn get_head_collider(
    helpers: &[Vec3],
    geometry: &mut Assets<Geometry>,
) -> (Handle<Geometry>, Transform) {
    let center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let radius = (helpers[HEAD_VERTICES[0]] - center).length();
    (
        geometry.add(Sphere::new(radius)),
        Transform::from_translation(center),
    )
}

fn get_midsection_collider(
    helpers: &[Vec3],
    joint: ColliderBone,
    geometry: &mut Assets<Geometry>,
    inv_bindpose_rot: Quat,
) -> (Handle<Geometry>, Transform) {
    let ref_verts = match joint {
        ColliderBone::Chest => TORSO_VERTICES,
        ColliderBone::Pelvis => PELVIS_VERTICES,
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
        Transform::from_translation(center).with_rotation(inv_bindpose_rot),
    )
}

fn get_limb_collider(
    helpers: &[Vec3],
    joint: ColliderBone,
    geometry: &mut Assets<Geometry>,
) -> (Handle<Geometry>, Transform) {
    let ref_verts = match joint {
        ColliderBone::LowerLeftArm => LOWER_LEFT_ARM_VERTICES,
        ColliderBone::LowerRightArm => LOWER_RIGHT_ARM_VERTICES,
        ColliderBone::UpperLeftArm => UPPER_LEFT_ARM_VERTICES,
        ColliderBone::UpperRightArm => UPPER_RIGHT_ARM_VERTICES,
        ColliderBone::LowerLeftLeg => LOWER_LEFT_LEG_VERTICES,
        ColliderBone::LowerRightLeg => LOWER_RIGHT_LEG_VERTICES,
        ColliderBone::UpperLeftLeg => UPPER_LEFT_LEG_VERTICES,
        ColliderBone::UpperRightLeg => UPPER_RIGHT_LEG_VERTICES,
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
        Transform::from_translation(c)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(dir, up, fwd))),
    )
}

fn get_extremity_collider(
    helpers: &[Vec3],
    joint: ColliderBone,
    geometry: &mut Assets<Geometry>,
) -> (Handle<Geometry>, Transform) {
    let ref_verts = match joint {
        ColliderBone::LeftHand => LEFT_HAND_VERTICES,
        ColliderBone::RightHand => RIGHT_HAND_VERTICES,
        ColliderBone::LeftFoot => LEFT_FOOT_VERTICES,
        ColliderBone::RightFoot => RIGHT_FOOT_VERTICES,
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
        Transform::from_translation(center)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(x, y, z))),
    )
}
