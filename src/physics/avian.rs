use ahash::AHashMap;
use avian3d::prelude::*;
use bevy::{
    math::{Quat, Vec3},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use bevy::ecs::intern::Internable;

use crate::{
    helpers::Helpers,
    prelude::{
        CharacterShape, DisableSkeletonLod,
        EnableSkeletonLod, ResetSkeletonToBindPose, SkeletonLodDisabled, SkeletonLodMap,
        SkeletonsReady,
    },
    rigs::SkeletalBone,
    NAME_INTERNER,
};

use super::*;

/// Collision layers for ragdoll/hitbox colliders on this character.
///
/// Set this on your character entity to control which physics layers
/// the collider bones belong to and which layers they collide with.
/// When changed at runtime, all existing collider entities are updated
/// automatically.
#[derive(Component, Clone, Copy, Debug)]
pub struct RagdollCollisionLayers(pub CollisionLayers);

/// Describes whether ragdoll physics is active
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
#[require(RagdollDensity, RagdollDamping)]
pub enum CharacterRagdoll {
    #[default]
    None,
    Full,
    /// Only the listed bones are made dynamic; all others stay kinematic.
    ///
    /// ## Warning: kinematic colliders whose corresponding skeleton bone is a child
    ///     of a bone whose cooresponding collider is dynamic
    ///
    /// If a kinematic collider's corresponding skeletal bone is a descendant of a
    /// dynamically controlled bone in the skeleton hierarchy, then directly setting
    /// `Position`/`Rotation` or transform component values
    /// on the kinematic collider can cause unpredictable behavior including crashes.
    ///
    /// For example:
    /// Ragdolling a branch (e.g. arms) while the root of the skeletal tree is kinematic should be safe
    /// Ragdolling the root of the tree while trying to control child bones kinematically can be dangerous.
    /// For example, a character whose head is kinematic attached to a dynamic body.
    ///
    /// This occurs because:
    /// 1. `sync_bones_to_ragdoll` (PostUpdate) writes the dynamic collider's physics
    ///    position back to its skeletal bone's local transform.
    /// 2. `TransformSystems::Propagate` (PostUpdate) recomputes all descendant bone
    ///    globals — including the kinematic bone — using the dynamic parent's new local.
    Partial(Vec<ColliderBone>),
}

#[derive(Component, Default)]
pub struct CharacterColliders {
    pub bones_subset: Option<Vec<ColliderBone>>,
    pub collider_entities: AHashMap<ColliderBone, Entity>,
    /// Primary bone entity for each collider bone — points to the best active LOD.
    /// Updated when skeleton LODs change.
    pub bone_entities: AHashMap<ColliderBone, Entity>,
    /// Per-LOD bone entity mapping for each collider bone.
    /// Indexed by `[collider_index_in_COLLIDERS][lod_index]`.
    /// `None` if the bone doesn't exist in that LOD variant.
    /// Built once at collider spawn time, read-only afterwards.
    pub lod_bone_entities: Vec<Vec<Option<Entity>>>,
    pub(crate) joint_entities: Vec<Entity>,
}

impl CharacterColliders {
    pub fn new(bones_subset: Option<Vec<ColliderBone>>) -> Self {
        Self {
            bones_subset,
            collider_entities: AHashMap::default(),
            bone_entities: AHashMap::default(),
            lod_bone_entities: Vec::new(),
            joint_entities: Vec::new(),
        }
    }
}

/// Tracks which skeleton LODs are currently active (not disabled).
/// Indexed by LOD variant index. Updated on EnableSkeletonLod / DisableSkeletonLod events.
/// Used by ragdoll sync systems to short-circuit disabled LODs.
#[derive(Component, Debug, Clone)]
pub struct ActiveSkeletonLods(pub Vec<bool>);

#[derive(Component)]
pub(crate) struct NeedsColliders;

/// Offset from a collider's parent bone to the collider itself.
///
/// Stores both the forward transform (collider → joint) and its
/// precomputed inverse (joint → collider) to avoid matrix inversion
/// in the per-frame sync hot path.
#[derive(Component)]
pub struct ColliderOffset {
    pub collider_to_bone: Transform,
    pub bone_to_collider: Transform,
}

/// Marker for colliders that are currently in kinematic (animation-following) mode.
#[derive(Component)]
pub struct KinematicCollider;

/// Links a collider entity to its owning character entity.
#[derive(Component)]
#[relationship(relationship_target = ColliderList)]
pub struct ColliderForCharacter(pub Entity);

/// Auto-maintained list of collider entities belonging to a character.
#[derive(Component)]
#[relationship_target(relationship = ColliderForCharacter)]
pub struct ColliderList(Vec<Entity>);

/// Links a joint entity to its owning character entity.
#[derive(Component)]
#[relationship(relationship_target = JointList)]
pub(crate) struct JointForCharacter(pub(crate) Entity);

/// Auto-maintained list of joint entities belonging to a character.
#[derive(Component)]
#[relationship_target(relationship = JointForCharacter)]
pub(crate) struct JointList(Vec<Entity>);

/// Marker for characters whose skeleton is fitted but colliders haven't been spawned yet.
pub(crate) fn mark_needs_colliders(
    mut commands: Commands,
    characters: Query<
        Entity,
        (
            Or<(Added<CharacterColliders>, Added<SkeletonsReady>)>,
            With<CharacterColliders>,
            With<SkeletonsReady>,
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
            &SkeletonLodMap,
            &mut CharacterColliders,
            &RagdollDensity,
            Option<&RagdollCollisionLayers>,
            Option<&Helpers>,
        ),
        (With<NeedsColliders>, With<SkeletonsReady>),
    >,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    skeleton_skins: Query<&SkinnedMesh, (Without<Mesh3d>, With<ChildOf>, Allow<SkeletonLodDisabled>)>,
    children_query: Query<&Children, Allow<SkeletonLodDisabled>>,
    joint_names: Query<&Name, (With<SkeletalBone>, Allow<SkeletonLodDisabled>)>,
) {
    const BATCH_SIZE: usize = 2;
    let mut char_count = 0;
    for (character_entity, _character_shape, lod_map, mut colliders, density, collision_layers, computed_helpers) in
        characters.iter_mut()
    {
        if char_count >= BATCH_SIZE {
            break;
        }
        let Some(collision_layers) = collision_layers else {
            continue;
        };
        let density = density.0;

        let Some(h) = computed_helpers else {
            continue;
        };
        let helpers = &h.0;

        // Resolve SkinnedMesh from the highest-detail (LOD 0) skeleton for
        // collider geometry and inverse bindposes (which are LOD-independent).
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

        let collider_bone_map = DEFAULT_RIG_COLLIDER_BONE_NAMES;

        let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes) else {
            continue;
        };

        let bone_entities: AHashMap<&str, Entity> = skm
            .joints
            .iter()
            .filter_map(|&joint| {
                joint_names
                    .get(joint)
                    .ok()
                    .map(|name| (NAME_INTERNER.intern(name.as_str()).leak(), joint))
            })
            .collect();

        let inv_bindposes_map: AHashMap<&str, Transform> = skm
            .joints
            .iter()
            .zip(inv_bindposes.iter())
            .filter_map(|(&joint, ibp)| {
                joint_names.get(joint).ok().map(|name| {
                    (
                        NAME_INTERNER.intern(name.as_str()).leak(),
                        Transform::from_matrix(*ibp),
                    )
                })
            })
            .collect();

        let target_bones: Vec<ColliderBone> = match &colliders.bones_subset {
            Some(bones) if !bones.is_empty() => bones.clone(),
            Some(_) => vec![],
            None => COLLIDERS.to_vec(),
        };

        if target_bones.is_empty() {
            continue;
        }

        // ── Build per-LOD bone entity mapping ──────────────────────────
        let mut lod_bone_maps: Vec<AHashMap<&str, Entity>> = Vec::new();

        // Determine actual number of LOD variants from sorted lod_map keys.
        let mut sorted_lod_keys: Vec<usize> = lod_map.0.keys().copied().collect();
        sorted_lod_keys.sort();
        let lod_num_variants = sorted_lod_keys.len();

        for &lod_idx in &sorted_lod_keys {
            let Some(&skeleton_entity) = lod_map.0.get(&lod_idx) else {
                lod_bone_maps.push(AHashMap::default());
                continue;
            };
            let mut name_to_entity: AHashMap<&str, Entity> = AHashMap::default();
            for child in children_query.iter_descendants(skeleton_entity) {
                if let Ok(s) = skeleton_skins.get(child) {
                    for &joint in &s.joints {
                        if let Ok(name) = joint_names.get(joint) {
                            name_to_entity
                                .insert(NAME_INTERNER.intern(name.as_str()).leak(), joint);
                        }
                    }
                    break;
                }
            }
            lod_bone_maps.push(name_to_entity);
        }

        // Per-collider, per-LOD bone entity lookup
        let mut lod_bone_entities: Vec<Vec<Option<Entity>>> =
            vec![vec![None; lod_num_variants]; COLLIDERS.len()];

        for &collider in &COLLIDERS {
            let i_collider = collider_index(collider);
            let joint_name = collider_bone_map[i_collider];
            for (lod_pos, name_to_entity) in lod_bone_maps.iter().enumerate() {
                lod_bone_entities[i_collider][lod_pos] =
                    name_to_entity.get(joint_name).copied();
            }
        }

        // ── Spawn collider entities (geometry from LOD 0, offsets LOD-independent) ──
        for &collider in &target_bones {
            let i_collider = collider_index(collider);
            let (geometry, collider_to_model) =
                get_collider_geometry(collider, helpers, i_collider, &inv_bindposes_map);

            let joint_name = collider_bone_map[i_collider];
            let model_to_joint = inv_bindposes_map[joint_name];
            let collider_to_joint = model_to_joint * collider_to_model;
            let joint_to_collider = Transform::from_matrix(collider_to_joint.to_matrix().inverse());

            let collider_entity = commands
                .spawn((
                    RigidBody::Kinematic,
                    KinematicCollider,
                    collider,
                    geometry,
                    ColliderOffset {
                        collider_to_bone: collider_to_joint,
                        bone_to_collider: joint_to_collider,
                    },
                    ColliderDensity(density),
                    collision_layers.0,
                    Transform::IDENTITY,
                    ColliderForCharacter(character_entity),
                    LinearDamping(0.1),
                    AngularDamping(0.1),
                ))
                .id();

            colliders
                .collider_entities
                .insert(collider, collider_entity);
            colliders
                .bone_entities
                .insert(collider, bone_entities[joint_name]);
        }

        colliders.lod_bone_entities = lod_bone_entities;

        // Insert ActiveSkeletonLods — all start enabled, observers correct this on events.
        commands
            .entity(character_entity)
            .insert(ActiveSkeletonLods(vec![true; lod_num_variants]))
            .remove::<NeedsColliders>();
        char_count += 1;
    }
}

/// This function syncs kinematic character colliders to align with the skeletal bones.
/// For each collider, iterates active LODs to find the first one that has the corresponding
/// bone, then reads its GlobalTransform.
pub(crate) fn sync_colliders(
    characters: Query<(&CharacterColliders, &ActiveSkeletonLods)>,
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    mut collider_data: Query<
        (&mut Position, &mut Rotation, &ColliderOffset),
        With<KinematicCollider>,
    >,
) {
    for (colliders, active_lods) in characters.iter() {
        for (&bone_type, collider_entity) in colliders.collider_entities.iter() {
            let Ok((mut position, mut rotation, offset)) = collider_data.get_mut(*collider_entity)
            else {
                continue;
            };

            let i_collider = collider_index(bone_type);

            // Find the first active LOD that has this bone.
            let mut found = false;
            for (lod_idx, &is_active) in active_lods.0.iter().enumerate() {
                if !is_active {
                    continue;
                }
                if let Some(Some(bone_entity)) = colliders.lod_bone_entities.get(i_collider)
                    .and_then(|lod_vec| lod_vec.get(lod_idx))
                {
                    if let Ok(joint_to_world) = bones.get(*bone_entity) {
                        let world = Transform::from(*joint_to_world) * offset.collider_to_bone;
                        *position = Position(world.translation);
                        *rotation = Rotation(world.rotation);
                        found = true;
                        break;
                    }
                }
            }

            // Fallback: try the primary bone_entities map
            if !found {
                if let Some(bone_entity) = colliders.bone_entities.get(&bone_type) {
                    if let Ok(joint_to_world) = bones.get(*bone_entity) {
                        let world = Transform::from(*joint_to_world) * offset.collider_to_bone;
                        *position = Position(world.translation);
                        *rotation = Rotation(world.rotation);
                    }
                }
            }
        }
    }
}

pub(crate) fn set_ragdoll_state(
    mut characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &mut CharacterColliders,
            &RagdollDamping,
            &ActiveSkeletonLods,
            &SkeletonLodMap,
        ),
        Changed<CharacterRagdoll>,
    >,
    mut commands: Commands,
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    collider_offsets: Query<&ColliderOffset>,
    mobility_query: Query<&RagdollMobility>,
) {
    for (character_entity, ragdoll, mut char_colliders, damping, active_lods, lod_map) in
        characters.iter_mut()
    {
        let damping = damping.0;
        let joint_damping = JointDamping {
            linear: damping,
            angular: damping,
        };

        let partial_bones: Option<&[ColliderBone]> = match ragdoll {
            CharacterRagdoll::Partial(bones) => Some(bones.as_slice()),
            _ => None,
        };

        // Clear existing joints before any ragdoll transition
        for &joint in &char_colliders.joint_entities {
            commands.entity(joint).despawn();
        }
        char_colliders.joint_entities.clear();

        match ragdoll {
            CharacterRagdoll::Full => {
                for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
                    commands
                        .entity(collider_entity)
                        .insert(RigidBody::Dynamic)
                        .remove::<KinematicCollider>();
                }
            }
            CharacterRagdoll::Partial(bones) => {
                for (bone, &collider_entity) in char_colliders.collider_entities.iter() {
                    if bones.contains(bone) {
                        commands
                            .entity(collider_entity)
                            .insert(RigidBody::Dynamic)
                            .remove::<KinematicCollider>();
                    } else {
                        commands
                            .entity(collider_entity)
                            .insert(RigidBody::Kinematic)
                            .insert(KinematicCollider);
                    }
                }
            }
            CharacterRagdoll::None => {
                for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
                    commands
                        .entity(collider_entity)
                        .insert(RigidBody::Kinematic)
                        .insert(KinematicCollider);
                }
                for &lod_idx in lod_map.0.keys() {
                    commands.trigger(ResetSkeletonToBindPose {
                        character: character_entity,
                        lod: lod_idx,
                    });
                }
                continue;
            }
        }

        // Helper: find the best bone entity for a ColliderBone across active LODs.
        let find_bone = |bone: ColliderBone, colliders: &CharacterColliders, active_lods: &ActiveSkeletonLods| -> Option<Entity> {
            let i_collider = collider_index(bone);
            for (lod_idx, &is_active) in active_lods.0.iter().enumerate() {
                if is_active && let Some(Some(entity)) = colliders.lod_bone_entities.get(i_collider)
                    .and_then(|v| v.get(lod_idx))
                {
                    return Some(*entity);
                }
            }
            // Fallback to primary bone_entities
            colliders.bone_entities.get(&bone).copied()
        };

        let joints_to_spawn: Vec<(
            ColliderBone,
            Entity,
            Entity,
            Vec3,
            Quat,
            Transform,
            Transform,
        )> = char_colliders
            .collider_entities
            .iter()
            .filter_map(|(bone, &child)| {
                if let Some(bones) = partial_bones
                    && !bones.contains(bone)
                    && !get_collider_parent(*bone).is_some_and(|p| bones.contains(&p))
                {
                    return None;
                }
                let parent_bone_type = get_collider_parent(*bone)?;
                let parent = *char_colliders.collider_entities.get(&parent_bone_type)?;

                let child_bone = find_bone(*bone, &char_colliders, active_lods)?;
                let parent_bone = find_bone(parent_bone_type, &char_colliders, active_lods)?;

                let child_bone_world = bones.get(child_bone).ok()?;
                let parent_bone_world = bones.get(parent_bone).ok()?;
                let anchor_world = child_bone_world.translation();

                let parent_offset = collider_offsets.get(parent).ok()?;
                let child_offset = collider_offsets.get(child).ok()?;

                let parent_collider =
                    Transform::from(*parent_bone_world) * parent_offset.collider_to_bone;
                let child_collider =
                    Transform::from(*child_bone_world) * child_offset.collider_to_bone;

                // Basis for body2's joint frame so the bindpose relative
                // rotation is the rest position and axes align.
                let local_basis2 = child_collider.rotation.inverse() * parent_collider.rotation;

                Some((
                    *bone,
                    parent,
                    child,
                    anchor_world,
                    local_basis2,
                    parent_collider,
                    child_collider,
                ))
            })
            .collect();

        let r = mobility_query
            .get(character_entity)
            .ok()
            .map_or(1.0, |m| m.0.clamp(0.0, 1.0));
        for (bone, parent, child, anchor, local_basis2, parent_collider, child_collider) in
            joints_to_spawn
        {
            // Ensure colliders are at their correct positions before creating joints,
            // in case sync_colliders hasn't run yet (e.g. newly-spawned colliders).
            commands.entity(parent).insert((
                Position(parent_collider.translation),
                Rotation(parent_collider.rotation),
            ));
            commands.entity(child).insert((
                Position(child_collider.translation),
                Rotation(child_collider.rotation),
            ));
            // If we don't increase the damping at the knee the whole ragdoll collapses very quickly at
            // the knees.  I guess we need this to support all the weight above
            let knee_damping = JointDamping {
                linear: damping * 20.0,
                angular: damping * 20.0,
            };
            let joint = spawn_ragdoll_joint(
                &mut commands,
                bone,
                parent,
                child,
                anchor,
                local_basis2,
                joint_damping,
                knee_damping,
                r,
                character_entity,
            );
            char_colliders.joint_entities.push(joint);
        }
    }
}

pub(crate) fn sync_bones_to_ragdoll(
    characters: Query<(&CharacterRagdoll, &CharacterColliders)>,
    colliders: Query<
        (&Position, &Rotation, &ColliderOffset),
        (With<ColliderBone>, Without<SkeletalBone>, Without<RigidBodyDisabled>),
    >,
    mut bones: Query<
        (&mut Transform, Option<&ChildOf>),
        (With<SkeletalBone>, Without<ColliderBone>),
    >,
    global_transforms: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
) {
    for (ragdoll, char_colliders) in characters.iter() {
        if matches!(ragdoll, CharacterRagdoll::None) {
            continue;
        }

        let partial_bones: Option<&[ColliderBone]> = match ragdoll {
            CharacterRagdoll::Partial(bones) => Some(bones.as_slice()),
            _ => None,
        };

        let mut applied_joint_world = AHashMap::<Entity, Transform>::default();

        for (&bone_type, &collider_entity) in char_colliders.collider_entities.iter() {
            if let Some(bones) = partial_bones
                && !bones.contains(&bone_type)
            {
                continue;
            }
            let Ok((position, rotation, offset)) = colliders.get(collider_entity) else {
                continue;
            };

            let joint_to_world =
                Transform::from_translation(position.0).with_rotation(rotation.0)
                    * offset.bone_to_collider;
            let i_collider = collider_index(bone_type);

            // Write ragdoll position to every LOD that has this bone.
            let lod_count = char_colliders
                .lod_bone_entities
                .get(i_collider)
                .map_or(0, |v| v.len());
            for lod_idx in 0..lod_count {
                let Some(Some(bone_entity)) = char_colliders
                    .lod_bone_entities
                    .get(i_collider)
                    .and_then(|v| v.get(lod_idx))
                else {
                    continue;
                };
                write_bone_to_skeleton(
                    *bone_entity,
                    joint_to_world,
                    &mut bones,
                    &global_transforms,
                    &mut applied_joint_world,
                );
            }

            // Fallback: primary bone_entities (may differ from LOD entities after remap).
            if let Some(&bone_entity) = char_colliders.bone_entities.get(&bone_type) {
                if !applied_joint_world.contains_key(&bone_entity) {
                    write_bone_to_skeleton(
                        bone_entity,
                        joint_to_world,
                        &mut bones,
                        &global_transforms,
                        &mut applied_joint_world,
                    );
                }
            }
        }
    }
}

fn write_bone_to_skeleton(
    bone_entity: Entity,
    joint_to_world: Transform,
    bones: &mut Query<
        (&mut Transform, Option<&ChildOf>),
        (With<SkeletalBone>, Without<ColliderBone>),
    >,
    global_transforms: &Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    applied_joint_world: &mut AHashMap<Entity, Transform>,
) {
    let Ok((mut local_transform, parent)) = bones.get_mut(bone_entity) else {
        return;
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

    let dp = (local.translation - local_transform.translation).length();
    let dr = local.rotation.angle_between(local_transform.rotation);
    if dp > 0.0001 || dr > 0.0001 {
        const LERP_FACTOR: f32 = 0.85;
        local_transform.translation = local_transform
            .translation
            .lerp(local.translation, LERP_FACTOR);
        local_transform.rotation =
            local_transform.rotation.slerp(local.rotation, LERP_FACTOR);
    }
    applied_joint_world.insert(bone_entity, joint_to_world);
}

/// Observer: when a skeleton LOD is enabled, update `ActiveSkeletonLods` and
/// remap `bone_entities` to the best available LOD.
pub(crate) fn on_enable_skeleton_lod_ragdoll(
    trigger: On<EnableSkeletonLod>,
    mut characters: Query<(
        &mut CharacterColliders,
        &mut ActiveSkeletonLods,
        &SkeletonLodMap,
    )>,
) {
    let event = trigger.event();
    let Ok((mut colliders, mut active_lods, lod_map)) = characters.get_mut(event.character) else {
        return;
    };
    let mut sorted_keys: Vec<usize> = lod_map.0.keys().copied().collect();
    sorted_keys.sort();
    let lod_pos = sorted_keys.iter().position(|&k| k == event.lod);
    if let Some(pos) = lod_pos {
        if pos < active_lods.0.len() {
            active_lods.0[pos] = true;
        }
    }
    remap_bone_entities(&mut colliders, &active_lods);
}

/// Observer: when a skeleton LOD is disabled, update `ActiveSkeletonLods` and
/// remap `bone_entities` to the best available LOD.
pub(crate) fn on_disable_skeleton_lod_ragdoll(
    trigger: On<DisableSkeletonLod>,
    mut characters: Query<(
        &mut CharacterColliders,
        &mut ActiveSkeletonLods,
        &SkeletonLodMap,
    )>,
) {
    let event = trigger.event();
    let Ok((mut colliders, mut active_lods, lod_map)) = characters.get_mut(event.character) else {
        return;
    };
    let mut sorted_keys: Vec<usize> = lod_map.0.keys().copied().collect();
    sorted_keys.sort();
    let lod_pos = sorted_keys.iter().position(|&k| k == event.lod);
    if let Some(pos) = lod_pos {
        if pos < active_lods.0.len() {
            active_lods.0[pos] = false;
        }
    }
    remap_bone_entities(&mut colliders, &active_lods);
}

/// Remap `bone_entities` to point to the first active LOD that has each bone.
fn remap_bone_entities(
    colliders: &mut CharacterColliders,
    active_lods: &ActiveSkeletonLods,
) {
    for (collider_idx, collider) in COLLIDERS.iter().enumerate() {
        let mut found = false;
        for (lod_idx, &is_active) in active_lods.0.iter().enumerate() {
            if !is_active {
                continue;
            }
            if let Some(Some(entity)) = colliders
                .lod_bone_entities
                .get(collider_idx)
                .and_then(|v| v.get(lod_idx))
            {
                colliders.bone_entities.insert(*collider, *entity);
                found = true;
                break;
            }
        }
        if !found {
            // No active LOD has this bone — remove from primary map.
            colliders.bone_entities.remove(collider);
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
            let bind_rot = inv_bindpose_rot.inverse();
            get_midsection_collider(helpers, collider, bind_rot)
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
    bind_rot: Quat,
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
        Transform::from_translation(center).with_rotation(bind_rot),
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
    local_basis2: Quat,
    joint_damping: JointDamping,
    knee_damping: JointDamping,
    r: f32,
    character: Entity,
) -> Entity {
    match bone {
        ColliderBone::LowerRightArm | ColliderBone::LowerLeftArm => commands
            .spawn((
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_local_basis2(local_basis2)
                    .with_angle_limits(-0.17 * r, 2.5 * r)
                    .with_point_compliance(0.0)
                    .with_align_compliance(0.0),
                JointCollisionDisabled,
                joint_damping,
                JointForCharacter(character),
            ))
            .id(),
        ColliderBone::LowerRightLeg | ColliderBone::LowerLeftLeg => commands
            .spawn((
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_local_basis2(local_basis2)
                    .with_angle_limits(-2.4 * r, 0.0)
                    .with_point_compliance(0.0)
                    .with_align_compliance(0.0),
                JointCollisionDisabled,
                knee_damping,
                JointForCharacter(character),
            ))
            .id(),
        ColliderBone::Head
        | ColliderBone::Chest
        | ColliderBone::Pelvis
        | ColliderBone::UpperRightArm
        | ColliderBone::UpperLeftArm
        | ColliderBone::UpperRightLeg
        | ColliderBone::UpperLeftLeg
        | ColliderBone::LeftHand
        | ColliderBone::RightHand
        | ColliderBone::LeftFoot
        | ColliderBone::RightFoot => {
            let (swing, twist) = get_spherical_limits(bone);
            commands
                .spawn((
                    SphericalJoint::new(parent, child)
                        .with_anchor(anchor)
                        .with_local_basis2(local_basis2)
                        .with_swing_limits(-swing * r, swing * r)
                        .with_twist_limits(-twist * r, twist * r)
                        .with_point_compliance(0.0)
                        .with_swing_compliance(0.0)
                        .with_twist_compliance(0.0),
                    JointCollisionDisabled,
                    joint_damping,
                    JointForCharacter(character),
                ))
                .id()
        }
    }
}

/// Event to atomically disable physics on a character's collider entities.
/// Despawns all ragdoll joints and inserts `RigidBodyDisabled`/`ColliderDisabled`
/// on every collider, preventing the island race condition that occurs when
/// joint despawn and component insertion happen in separate command flushes.
#[derive(Event, Debug, Clone)]
pub struct DisablePhysics {
    pub character: Entity,
}

pub(crate) fn on_disable_physics(
    trigger: On<DisablePhysics>,
    mut characters: Query<&mut CharacterColliders>,
    mut commands: Commands,
) {
    let event = trigger.event();
    let Ok(mut colliders) = characters.get_mut(event.character) else {
        return;
    };

    for joint in colliders.joint_entities.drain(..) {
        commands.entity(joint).despawn();
    }

    for &collider_entity in colliders.collider_entities.values() {
        commands
            .entity(collider_entity)
            .insert(RigidBodyDisabled)
            .insert(ColliderDisabled);
    }
}

/// Propagates [`RagdollCollisionLayers`] changes from the character entity
/// to all of its spawned collider entities at runtime.
pub(crate) fn update_collision_layers(
    characters: Query<(&RagdollCollisionLayers, &ColliderList), Changed<RagdollCollisionLayers>>,
    mut commands: Commands,
) {
    for (layers, collider_list) in characters.iter() {
        for &collider_entity in collider_list.0.iter() {
            commands.entity(collider_entity).insert(layers.0);
        }
    }
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
