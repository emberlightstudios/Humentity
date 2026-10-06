//! Jolt ragdoll backend: collider/shape/offset mapping onto `bevy_jolt`'s
//! `JoltRagdoll` one-shot builder.
//!
//! Same architecture (bones-only hierarchy, separate collider entities,
//! `ColliderOffset` sync both directions), same public names, so game code
//! swaps backends by flipping the `avian`/`jolt` feature.
//!
//! This module owns three things and nothing else:
//!
//! - collider/shape measurement from mesh helpers (`get_collider_geometry`
//!   and friends) plus the bone↔collider offset frames;
//! - the per-bone joint limit tables (`default_joint_limit`,
//!   `resolve_joint_limit`, [`RagdollJointLimitOverrides`]);
//! - spawning one [`JoltRagdoll`](bevy_jolt::JoltRagdoll) spec per character
//!   and tagging its baked parts so [`sync_bones_to_ragdoll`] can read them.
//!
//! Everything else — mass stabilization, constraint priorities,
//! parent-child no-collide, body creation, pose sync — lives inside Jolt's
//! `RagdollSettings` path in `bevy_jolt`. There is deliberately no damping,
//! density, or motion-flip tuning here: bodies bake at Jolt defaults and
//! settle through stabilization, not per-body knobs.

use ahash::AHashMap;
use bevy::ecs::intern::Internable;
use bevy::math::{Mat3, Quat, Vec3};
use bevy_jolt::{
    JoltRagdoll, JoltRagdollParts, RagdollJoint, RagdollShape,
};

use crate::{
    helpers::{RefitCharacter, TeardownCharacter},
    prelude::{
        CharacterShape, CharacterShapeAsset, CharacterSkeleton, SkeletonLodDisabled,
        SkeletonReady,
    },
    rigs::{RigData, SkeletalBone},
    spawn_skeleton::CharacterScale,
    NAME_INTERNER,
};
use super::*;

/// Collision layers for ragdoll/hitbox colliders on this character.
///
/// Same role as the avian `RagdollCollisionLayers`, but Jolt layers are team
/// indices (`u16`), not bitmasks: `membership` picks this character's team,
/// `collides_with_mask` is the bitmask of teams it collides with (bit `i` =
/// team `i`).
///
/// NOTE: the layer table is fixed when `JoltPlugin` creates the world, so
/// every team named here must exist in the table the app passes to
/// `JoltPlugin::with_collision_layers`.
#[derive(Component, Clone, Copy, Debug)]
pub struct RagdollCollisionLayers {
    pub membership: u16,
    pub collides_with_mask: u32,
}

impl RagdollCollisionLayers {
    pub const fn new(membership: u16, collides_with_mask: u32) -> Self {
        Self {
            membership,
            collides_with_mask,
        }
    }
}

/// Maps an avian-style membership bit (e.g. `1 << 3`) to a Jolt team index.
/// Layer tables are small and dense, so the bit position is the index.
pub const fn jolt_layer_for(membership_bit: u32) -> u16 {
    membership_bit.trailing_zeros() as u16
}

/// Describes whether ragdoll physics is active
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub enum CharacterRagdoll {
    #[default]
    None,
    Full,
    /// Only the listed bones are made dynamic; all others stay kinematic.
    ///
    /// ## Warning: kinematic colliders whose corresponding skeleton bone is a child
    /// of a bone whose corresponding collider is dynamic
    ///
    /// If a kinematic collider's corresponding skeletal bone is a descendant of a
    /// dynamically controlled bone in the skeleton hierarchy, then directly driving
    /// the kinematic collider can cause unpredictable behavior including crashes.
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

/// Single global, static (identity, world-root) entity that holds all characters'
/// ragdoll spec entities. Keeps them grouped under one world-root entity in
/// the inspector rather than polluting the root with one entity per character.
/// Plain identity entity: no physics, so jolt ignores it.
#[derive(Resource)]
pub struct CharacterPhysicsContainer(pub Entity);

#[derive(Component, Default)]
pub struct CharacterColliders {
    pub bones_subset: Option<Vec<ColliderBone>>,
    /// Collider bone type → bone entity in the character's single fixed skeleton.
    /// Built once at collider spawn time, read-only afterwards.
    pub bone_entities: AHashMap<ColliderBone, Entity>,
    /// Collider bone type → baked part entity (filled when the spec bakes).
    pub collider_entities: AHashMap<ColliderBone, Entity>,
    /// The live ragdoll spec entity (the `JoltRagdoll` holder). None when off.
    pub ragdoll_entity: Option<Entity>,
    /// Spec order: collider bone per part index. Needed to tag baked parts.
    pub part_order: Vec<ColliderBone>,
}

impl CharacterColliders {
    pub fn new(bones_subset: Option<Vec<ColliderBone>>) -> Self {
        Self {
            bones_subset,
            bone_entities: AHashMap::default(),
            collider_entities: AHashMap::default(),
            ragdoll_entity: None,
            part_order: Vec::new(),
        }
    }
}

/// The resolved rotational limits for a single ragdoll joint.
///
/// Swing-twist joints (shoulders, hips, spine, neck, hands, feet) use `swing`
/// (cone half-angle in radians) and `twist` (twist half-angle in radians).
/// Hinge joints (elbows, knees) use `angle_min`/`angle_max` (radians).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RagdollJointLimit {
    pub swing: f32,
    pub twist: f32,
    pub angle_min: f32,
    pub angle_max: f32,
}

/// Returns the default [`RagdollJointLimit`] for a collider bone, before any
/// [`RagdollMobility`] scaling or [`RagdollJointLimitOverrides`] are applied.
pub fn default_joint_limit(bone: ColliderBone) -> RagdollJointLimit {
    match bone {
        ColliderBone::LowerRightArm | ColliderBone::LowerLeftArm => RagdollJointLimit {
            angle_min: -0.17,
            angle_max: 2.5,
            ..default()
        },
        ColliderBone::LowerRightLeg | ColliderBone::LowerLeftLeg => RagdollJointLimit {
            angle_min: -2.4,
            angle_max: 0.0,
            ..default()
        },
        _ => {
            let (swing, twist) = get_spherical_limits(bone);
            RagdollJointLimit {
                swing,
                twist,
                ..default()
            }
        }
    }
}

/// Resolves the effective [`RagdollJointLimit`] for a collider bone: an explicit
/// override wins, otherwise [`default_joint_limit`]; either way the result is
/// scaled by `mobility` (clamped to 0..=1).
pub fn resolve_joint_limit(
    bone: ColliderBone,
    overrides: Option<&RagdollJointLimitOverrides>,
    mobility: f32,
) -> RagdollJointLimit {
    let mobility_ratio = mobility.clamp(0.0, 1.0);
    let mut limit = overrides
        .and_then(|overrides| overrides.get(bone))
        .unwrap_or_else(|| default_joint_limit(bone));
    limit.swing *= mobility_ratio;
    limit.twist *= mobility_ratio;
    limit.angle_min *= mobility_ratio;
    limit.angle_max *= mobility_ratio;
    limit
}

/// Per-bone joint limit overrides, resolved into the spec at spawn.
///
/// Apply to your character entity. Any bone absent from the relevant map falls
/// back to [`default_joint_limit`] scaled by [`RagdollMobility`].
///
/// NOTE: Jolt bakes limits at creation, so changing these respawns the whole
/// ragdoll spec — same as flipping the ragdoll off and on.
#[derive(Component, Clone, Default, Debug)]
pub struct RagdollJointLimitOverrides {
    /// Swing-twist joints: bone → (swing half-angle, twist half-angle), radians.
    pub spherical: AHashMap<ColliderBone, (f32, f32)>,
    /// Hinge joints: bone → (min angle, max angle), radians.
    pub revolute: AHashMap<ColliderBone, (f32, f32)>,
}

impl RagdollJointLimitOverrides {
    fn get(&self, bone: ColliderBone) -> Option<RagdollJointLimit> {
        match bone {
            ColliderBone::LowerRightArm
            | ColliderBone::LowerLeftArm
            | ColliderBone::LowerRightLeg
            | ColliderBone::LowerLeftLeg => {
                self.revolute
                    .get(&bone)
                    .map(|&(min, max)| RagdollJointLimit {
                        angle_min: min,
                        angle_max: max,
                        ..default()
                    })
            }
            _ => self
                .spherical
                .get(&bone)
                .map(|&(swing, twist)| RagdollJointLimit {
                    swing,
                    twist,
                    ..default()
                }),
        }
    }
}

#[derive(Component)]
pub(crate) struct NeedsColliders;

/// Offset from a collider's parent bone to the collider itself.
///
/// Stores both the forward transform (collider → joint) and its
/// precomputed inverse (joint → collider) to avoid matrix inversion
/// in the per-frame sync hot path.
#[derive(Component, Clone, Copy, Debug)]
pub struct ColliderOffset {
    pub collider_to_bone: Transform,
    pub bone_to_collider: Transform,
}

/// Links a collider entity to the skeleton bone it follows.
#[derive(Component, Debug, Clone, Copy)]
pub struct BoneForCollider(pub Entity);

/// Links a collider entity to its owning character entity.
#[derive(Component)]
#[relationship(relationship_target = ColliderList)]
pub struct ColliderForCharacter(pub Entity);

/// Auto-maintained list of collider entities belonging to a character.
#[derive(Component)]
#[relationship_target(relationship = ColliderForCharacter)]
pub struct ColliderList(Vec<Entity>);

/// Marker for characters whose skeleton is fitted but colliders haven't been spawned yet.
pub(crate) fn mark_needs_colliders(
 mut commands: Commands,
 characters: Query<
 Entity,
 (
 Or<(Added<CharacterColliders>, Added<SkeletonReady>)>,
 With<CharacterColliders>,
 With<SkeletonReady>,
 ),
 >,
) {
 for character_entity in characters.iter() {
 commands.entity(character_entity).insert(NeedsColliders);
 }
}

/// Measures every collider shape + offset from the fitted bind pose and caches
/// them on the character. The bodies themselves spawn later, when the ragdoll
/// flips on — [`set_ragdoll_state`] builds the `JoltRagdoll` spec from these.
#[derive(Component, Clone, Debug)]
pub(crate) struct ColliderMeasurement {
 pub(crate) bone: ColliderBone,
 pub(crate) shape: RagdollShape,
 /// Fixed joint frame: collider origin relative to its bone (bind pose).
 /// Part pose at flip = live bone world × this; the stored offset is also
 /// what [`sync_bones_to_ragdoll`] needs, so no second pass computes it.
 pub(crate) offset: ColliderOffset,
}

/// Cached collider measurements, filled by [`spawn_colliders`]. The ragdoll
/// spec builds from these on flip.
#[derive(Component, Default)]
pub(crate) struct MeasuredColliders {
 pub(crate) measurements: Vec<ColliderMeasurement>,
}

pub(crate) fn spawn_colliders(
 mut commands: Commands,
 mut characters: Query<
 (
 Entity,
 &CharacterShape,
 &CharacterSkeleton,
 &mut CharacterColliders,
 Option<&CharacterScale>,
 ),
 (With<NeedsColliders>, With<SkeletonReady>),
 >,
 shape_assets: Res<Assets<CharacterShapeAsset>>,
 templates: Res<Assets<crate::template::CharacterTemplate>>,
 basemesh: Res<crate::basemesh::BaseMesh>,
 rig_data: Res<RigData>,
 ) {
 const BATCH_SIZE: usize = 2;
 let mut spawned_character_count = 0;
    for (
        character_entity,
        character_shape,
        skeleton,
        mut colliders,
        character_scale,
    ) in characters.iter_mut()
    {
        if spawned_character_count >= BATCH_SIZE {
            break;
        }

        let Some(shape_asset) = shape_assets.get(&character_shape.0) else {
            continue;
        };
        let Some(template) = templates.get(&shape_asset.template) else {
            continue;
        };
        if basemesh.vertices.is_empty()
            || template.shapes.iter().any(|shape| shape.helper_deltas.is_none())
        {
            continue;
        }
        let blended_helpers =
            template.blend_helpers(&shape_asset.template_morph_targets, &basemesh.vertices);
        let helpers = &blended_helpers;

        let Some(rig_spec) = rig_data.0.as_ref() else {
            continue;
        };
        let reference_order = &rig_spec.reference_rig.bone_names;

        // Build inverse-bindpose map (name → inv bindpose) from the single skeleton.
        let mut inv_bindposes_map: AHashMap<&str, Transform> = AHashMap::default();
        for (bone_index, &bone_name) in reference_order.iter().enumerate() {
            inv_bindposes_map.insert(
                bone_name,
                Transform::from_matrix(skeleton.model_space_inv_bindposes[bone_index]),
            );
        }

        let collider_bone_map = DEFAULT_RIG_COLLIDER_BONE_NAMES;

        let target_bones: Vec<ColliderBone> = match &colliders.bones_subset {
            Some(bones) if !bones.is_empty() => bones.clone(),
            Some(_) => vec![],
            None => COLLIDERS.to_vec(),
        };

        if target_bones.is_empty() {
            continue;
        }

        // Build collider-bone → bone-entity mapping from the single skeleton.
        colliders.bone_entities.clear();
        for &collider in &COLLIDERS {
            let collider_slot = collider_index(collider);
            let joint_name = collider_bone_map[collider_slot];
            if let Some(&bone_entity) = skeleton
                .bone_map
                .get(NAME_INTERNER.intern(joint_name).leak())
            {
                colliders.bone_entities.insert(collider, bone_entity);
            }
        }

 // Measure shapes + offsets (LOD-independent) and cache them. The
 // offset is the fixed joint frame: collider origin relative to its
 // bone in the bind pose, so the flip and the sync share one frame.
 let collider_scale = character_scale.map_or(1.0, |scale| scale.0);
 let mut measurements = Vec::with_capacity(target_bones.len());
 for &collider in &target_bones {
 let collider_slot = collider_index(collider);
 let (shape, collider_to_model) = get_collider_geometry(
 collider,
 helpers,
 collider_slot,
 &inv_bindposes_map,
 collider_scale,
 );
 let joint_name = collider_bone_map[collider_slot];
 let model_to_joint = inv_bindposes_map[joint_name];
 let collider_to_joint = model_to_joint * collider_to_model;
 let joint_to_collider =
 Transform::from_matrix(collider_to_joint.to_matrix().inverse());
 measurements.push(ColliderMeasurement {
 bone: collider,
 shape,
 offset: ColliderOffset {
 collider_to_bone: collider_to_joint,
 bone_to_collider: joint_to_collider,
 },
 });
 }
        commands
            .entity(character_entity)
            .insert(MeasuredColliders { measurements });
        commands
            .entity(character_entity)
            .remove::<NeedsColliders>();
        spawned_character_count += 1;
    }
}


/// Spawns (or despawns) the `JoltRagdoll` spec when `CharacterRagdoll` flips.
/// Full builds one spec from the cached measurements at the current bone
/// poses; None despawns it. Partial is not supported on this backend: Jolt
/// bakes the whole chain in one call, so Partial falls back to Full.
pub(crate) fn set_ragdoll_state(
    mut characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &mut CharacterColliders,
            Option<&RagdollCollisionLayers>,
            Option<&MeasuredColliders>,
            Option<&RagdollJointLimitOverrides>,
            Option<&RagdollMobility>,
        ),
        Changed<CharacterRagdoll>,
    >,
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    mut commands: Commands,
    container: Option<Res<CharacterPhysicsContainer>>,
) {
    let container_entity = container.map(|container| container.0);
    for (
        character_entity,
        ragdoll,
        mut character_colliders,
        collision_layers,
        measured,
        overrides,
        mobility,
    ) in characters.iter_mut()
    {
        // Tear down any live spec first: flips and limit changes rebuild.
        if let Some(ragdoll_entity) = character_colliders.ragdoll_entity.take() {
            commands.entity(ragdoll_entity).despawn();
        }
        character_colliders.collider_entities.clear();
        character_colliders.part_order.clear();

        if matches!(ragdoll, CharacterRagdoll::None) {
            continue;
        }
        let Some(measured) = measured else {
            continue; // Transient: measurements still pending, retry next flip.
        };
        let mobility = mobility.map_or(1.0, |mob| mob.0);
        let object_layer = collision_layers.map_or(0, |layers| layers.membership);

 let mut parts = Vec::with_capacity(measured.measurements.len());
 let mut part_order = Vec::with_capacity(measured.measurements.len());
 let mut offsets = Vec::with_capacity(measured.measurements.len());
 // Part index per collider bone: parents-first order comes free —
 // `COLLIDERS` lists parents before children, and measurements follow it.
 let mut part_index_of: AHashMap<ColliderBone, usize> = AHashMap::default();
 for measurement in &measured.measurements {
 let Some(&bone_entity) = character_colliders.bone_entities.get(&measurement.bone) else {
 continue; // Transient: bone missing from the map, skip this part.
 };
 let Ok(joint_to_world) = bones.get(bone_entity) else {
 continue; // Transient: bone gone, skip this part.
 };
 let joint_world = Transform::from(*joint_to_world);
 // Collider origin in world space: live bone pose × fixed joint
 // frame. The offset was measured in the bind pose, so this seats
 // the part exactly where the old collider entities sat.
 let part_world = joint_world * measurement.offset.bone_to_collider;
 let limit = resolve_joint_limit(measurement.bone, overrides, mobility);
 let joint = ragdoll_joint_for(measurement.bone, limit);
 let parent_part = get_collider_parent(measurement.bone)
 .and_then(|parent| part_index_of.get(&parent).copied());
 // Root parts (no parent in the set) hang parentless.
 part_index_of.insert(measurement.bone, parts.len());
 part_order.push(measurement.bone);
 offsets.push(measurement.offset);
 parts.push(bevy_jolt::RagdollPart {
 shape: measurement.shape,
 part_position: part_world.translation,
 part_rotation: part_world.rotation,
 parent_part,
 joint,
 });
 }
 if parts.is_empty() {
 continue;
 }
 let spec = JoltRagdoll {
 parts,
 object_layer,
 density_kg_per_m3: 1000.0,
 };
 let spec_id = commands
 .spawn((
 Name::new("RagdollSpec"),
 spec,
 ColliderForCharacter(character_entity),
 SpecOwner(character_entity),
 ))
 .id();
 if let Some(container_entity) = container_entity {
 commands.entity(container_entity).add_child(spec_id);
 }
 character_colliders.ragdoll_entity = Some(spec_id);
 character_colliders.part_order = part_order;
 // Stash offsets on the character for the bake tagger (spec order).
 commands
 .entity(character_entity)
 .insert(PendingOffsets { offsets });
 }
}

/// Which character owns a ragdoll spec entity. Set at spawn so the bake
/// tagger can file part entities back into `CharacterColliders`.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct SpecOwner(pub(crate) Entity);

/// Seated offsets in spec order, waiting for the bake tagger.
#[derive(Component)]
pub(crate) struct PendingOffsets {
 pub(crate) offsets: Vec<ColliderOffset>,
}

/// Tags baked ragdoll parts with their collider mapping once `bevy_jolt`
/// expands the spec: `ColliderBone` + the measured `ColliderOffset` +
/// `BoneForCollider`, so [`sync_bones_to_ragdoll`] reads them like any
/// other collider.
pub(crate) fn tag_ragdoll_parts(
 mut commands: Commands,
 specs: Query<(Entity, &SpecOwner, &JoltRagdollParts)>,
 mut characters: Query<(&mut CharacterColliders, Option<&PendingOffsets>)>,
) {
 for (_spec_entity, owner, baked) in specs.iter() {
 let Ok((mut colliders, pending)) = characters.get_mut(owner.0) else {
 continue; // Transient: character torn down before bake flushed.
 };
 let Some(pending) = pending else {
 continue; // Already tagged (or spec from elsewhere), skip.
 };
 for (part_index, part_entity) in baked.part_entities.iter().enumerate() {
 let (Some(&bone), Some(&offset)) = (
 colliders.part_order.get(part_index),
 pending.offsets.get(part_index),
 ) else {
 continue;
 };
 commands.entity(*part_entity).insert((
 bone,
 offset,
 ColliderForCharacter(owner.0),
 ));
 if let Some(&bone_entity) = colliders.bone_entities.get(&bone) {
 commands
 .entity(*part_entity)
 .insert(BoneForCollider(bone_entity));
 }
 colliders.collider_entities.insert(bone, *part_entity);
 }
 commands.entity(owner.0).remove::<PendingOffsets>();
 }
}

fn ragdoll_joint_for(bone: ColliderBone, limit: RagdollJointLimit) -> RagdollJoint {
    match bone {
        ColliderBone::LowerRightArm
        | ColliderBone::LowerLeftArm
        | ColliderBone::LowerRightLeg
        | ColliderBone::LowerLeftLeg => RagdollJoint::Hinge {
            // Hinges flex about body-local Z; twist about Y is the normal.
            // Seated rotations bake the rest pose, so identity axes read zero.
            hinge_axis: Dir3::Z,
            normal_axis: Dir3::Y,
            min: limit.angle_min,
            max: limit.angle_max,
        },
        _ => RagdollJoint::SwingTwist {
            twist_axis: Dir3::Y,
            plane_axis: Dir3::X,
            normal_half_cone: limit.swing,
            plane_half_cone: limit.swing,
            twist_min: -limit.twist,
            twist_max: limit.twist,
        },
    }
}

pub(crate) fn sync_bones_to_ragdoll(
    mut characters: Query<(&CharacterRagdoll, &mut CharacterColliders)>,
    colliders: Query<
        (&Transform, &ColliderOffset),
        (With<ColliderBone>, Without<SkeletalBone>),
    >,
    mut bones: Query<
        (&mut Transform, Option<&ChildOf>),
        (With<SkeletalBone>, Without<ColliderBone>, Allow<SkeletonLodDisabled>),
    >,
    global_transforms: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
) {
    for (ragdoll, character_colliders) in characters.iter_mut() {
        if matches!(ragdoll, CharacterRagdoll::None) {
            continue;
        }

        let partial_bones: Option<&[ColliderBone]> = match ragdoll {
            CharacterRagdoll::Partial(bones) => Some(bones.as_slice()),
            _ => None,
        };

        // Fixed-size parent world cache indexed by `collider_index`: parents
        // come before children in `COLLIDERS` order, so a child's skeletal
        // parent (when itself collider-tracked) is already cached. A linear
        // reverse lookup maps the skeletal parent entity back to its collider
        // slot; untracked parents fall back to `GlobalTransform` as before.
        let mut applied_joint_world: [Option<Transform>; 15] = [None; 15];
        for &bone_type in COLLIDERS.iter() {
            if let Some(bones) = partial_bones
                && !bones.contains(&bone_type)
            {
                continue;
            }

            let Some(&collider_entity) = character_colliders.collider_entities.get(&bone_type) else {
                continue;
            };
            let Ok((collider_transform, offset)) = colliders.get(collider_entity) else {
                continue;
            };

            let joint_to_world = *collider_transform * offset.bone_to_collider;

            let Some(&bone_entity) = character_colliders.bone_entities.get(&bone_type) else {
                continue;
            };
            // Convert joint_to_world (world space) to local using the single
            // skeleton's parent chain.
            let local = {
                let (_, parent) = bones.get(bone_entity).expect("bone entity should be valid");
                if let Some(parent) = parent {
                    let parent_entity = parent.parent();
                    let mut cached = None;
                    for (slot, cached_world) in applied_joint_world.iter().enumerate() {
                        if let Some(cached_world) = cached_world
                            && character_colliders.bone_entities.get(&COLLIDERS[slot])
                                == Some(&parent_entity)
                        {
                            cached = Some(*cached_world);
                            break;
                        }
                    }
                    if let Some(parent_to_world) = cached {
                        Transform::from_matrix(parent_to_world.to_matrix().inverse())
                            * joint_to_world
                    } else if let Ok(parent_to_world) = global_transforms.get(parent_entity) {
                        Transform::from_matrix(parent_to_world.to_matrix().inverse())
                            * joint_to_world
                    } else {
                        joint_to_world
                    }
                } else {
                    joint_to_world
                }
            };

            applied_joint_world[collider_index(bone_type)] = Some(joint_to_world);

            // Write directly to the single skeleton's bone, lerping for smoothness.
            if let Ok((mut transform, _)) = bones.get_mut(bone_entity) {
                let position_delta = (local.translation - transform.translation).length();
                let rotation_delta = local.rotation.angle_between(transform.rotation);
                if position_delta > 0.0001 || rotation_delta > 0.0001 {
                    const LERP_FACTOR: f32 = 0.85;
                    transform.translation = transform
                        .translation
                        .lerp(local.translation, LERP_FACTOR);
                    transform.rotation = transform.rotation.slerp(local.rotation, LERP_FACTOR);
                }
            }
        }
    }
}

fn get_collider_geometry(
    collider: ColliderBone,
    helpers: &[Vec3],
    collider_slot: usize,
    inv_bindposes: &AHashMap<&str, Transform>,
    scale: f32,
) -> (RagdollShape, Transform) {
    match collider {
        ColliderBone::Head => get_head_collider(helpers, scale),
        ColliderBone::Chest | ColliderBone::Pelvis => {
            let bone_name = DEFAULT_RIG_COLLIDER_BONE_NAMES[collider_slot];
            let inv_bindpose_rot = inv_bindposes[bone_name].rotation;
            let bind_rot = inv_bindpose_rot.inverse();
            get_midsection_collider(helpers, collider, bind_rot, scale)
        }
        ColliderBone::UpperRightArm
        | ColliderBone::UpperLeftArm
        | ColliderBone::LowerRightArm
        | ColliderBone::LowerLeftArm
        | ColliderBone::UpperRightLeg
        | ColliderBone::UpperLeftLeg
        | ColliderBone::LowerRightLeg
        | ColliderBone::LowerLeftLeg => get_limb_collider(helpers, collider, scale),
        ColliderBone::LeftHand
        | ColliderBone::RightHand
        | ColliderBone::LeftFoot
        | ColliderBone::RightFoot => get_extremity_collider(helpers, collider, scale),
    }
}

fn get_head_collider(helpers: &[Vec3], scale: f32) -> (RagdollShape, Transform) {
    let center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let radius = (helpers[HEAD_VERTICES[0]] - center).length() * scale;
    (
        RagdollShape::Sphere { radius },
        Transform::from_translation(center),
    )
}

fn get_midsection_collider(
    helpers: &[Vec3],
    joint: ColliderBone,
    bind_rot: Quat,
    scale: f32,
) -> (RagdollShape, Transform) {
    let ref_verts = match joint {
        ColliderBone::Chest => TORSO_VERTICES,
        ColliderBone::Pelvis => PELVIS_VERTICES,
        _ => unreachable!(),
    };

    let mut verts = [Vec3::ZERO; 8];
    for (vert_index, mesh_vert) in ref_verts.iter().enumerate() {
        verts[vert_index] = helpers[*mesh_vert];
        verts[vert_index + 4] = Vec3::new(-verts[vert_index].x, verts[vert_index].y, verts[vert_index].z);
    }
    let center = verts.iter().sum::<Vec3>() / 8.;

    let xmin = verts.iter().map(|vert| vert.x).reduce(f32::min).unwrap();
    let xmax = verts.iter().map(|vert| vert.x).reduce(f32::max).unwrap();
    let ymin = verts.iter().map(|vert| vert.y).reduce(f32::min).unwrap();
    let ymax = verts.iter().map(|vert| vert.y).reduce(f32::max).unwrap();
    let zmin = verts.iter().map(|vert| vert.z).reduce(f32::min).unwrap();
    let zmax = verts.iter().map(|vert| vert.z).reduce(f32::max).unwrap();

    // Chest: explicit 90° flip about local X, swapping forward and up vs the
    // inherited bind frame. The y/z extents swap with the axes so the box
    // keeps its measured shape.
    if joint == ColliderBone::Chest {
        (
            RagdollShape::Box {
                half_extents: Vec3::new(
                    (xmax - xmin) * scale * 0.5,
                    (zmax - zmin) * scale * 0.5,
                    (ymax - ymin) * scale * 0.5,
                ),
            },
            Transform::from_translation(center).with_rotation(
                bind_rot * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            ),
        )
    } else {
        (
            RagdollShape::Box {
                half_extents: Vec3::new(
                    (xmax - xmin) * scale * 0.5,
                    (ymax - ymin) * scale * 0.5,
                    (zmax - zmin) * scale * 0.5,
                ),
            },
            Transform::from_translation(center).with_rotation(bind_rot),
        )
    }
}

fn get_limb_collider(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (RagdollShape, Transform) {
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
    let verts: Vec<Vec3> = ref_verts.iter().map(|&vert| helpers[vert]).collect();

    let segment_start = (verts[0] + verts[1]) * 0.5;
    let segment_end = (verts[2] + verts[3]) * 0.5;
    let radius = (verts[0] - verts[1]).length() * 0.5 * scale;
    let center = (segment_start + segment_end) * 0.5;
    let dir = (segment_start - segment_end).normalize();
    let up = dir.cross(Vec3::NEG_Z).normalize();
    let fwd = dir.cross(up);

    (
        RagdollShape::Capsule {
            cylinder_half_height: (segment_start - segment_end).length() * scale * 0.5,
            radius,
        },
        Transform::from_translation(center)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(fwd, dir, up))),
    )
}

fn get_extremity_collider(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (RagdollShape, Transform) {
    let ref_verts = match joint {
        ColliderBone::LeftHand => LEFT_HAND_VERTICES,
        ColliderBone::RightHand => RIGHT_HAND_VERTICES,
        ColliderBone::LeftFoot => LEFT_FOOT_VERTICES,
        ColliderBone::RightFoot => RIGHT_FOOT_VERTICES,
        _ => unreachable!(),
    };
    let verts: Vec<Vec3> = ref_verts.iter().map(|&vert| helpers[vert]).collect();

    let x = (verts[0] - verts[1]).length() * scale;
    let y = (verts[2] - verts[3]).length() * scale;
    let z = (verts[4] - verts[5]).length() * scale;

    let center = verts.iter().sum::<Vec3>() / verts.len() as f32;

    let x_axis = verts[1] - verts[0];
    let y_axis = (verts[3] - verts[2]).normalize();
    let z_axis = x_axis.cross(y_axis).normalize();
    let x_axis = y_axis.cross(z_axis);

    (
        RagdollShape::Box {
            half_extents: Vec3::new(x * 0.5, y * 0.5, z * 0.5),
        },
        Transform::from_translation(center)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(x_axis, y_axis, z_axis))),
    )
}

/// Observer: on [`TeardownCharacter`] or [`RefitCharacter`], tear down all
/// humentity-owned physics state so only a bare state blob remains. This pairs
/// with the skeleton/mesh cleanup in `spawn_skeleton`.
///
/// Despawns the live ragdoll spec (bodies + constraints go with it) and
/// removes `CharacterColliders` and `NeedsColliders` from the character
/// root. User-config components (`CharacterRagdoll`, `RagdollMobility`,
/// `RagdollCollisionLayers`) are left intact so the character can be
/// re-activated later.
pub(crate) fn on_teardown_character(
    trigger: On<TeardownCharacter>,
    characters: Query<(), With<CharacterShape>>,
    mut colliders: Query<&mut CharacterColliders>,
    mut commands: Commands,
) {
    teardown_character_physics(
        trigger.event().0,
        &characters,
        &mut colliders,
        &mut commands,
    );
}

/// Observer for [`RefitCharacter`]: same physics teardown, so the collider
/// spawn re-runs from current morph weights after the fit rebuilds.
pub(crate) fn on_refit_character(
    trigger: On<RefitCharacter>,
    characters: Query<(), With<CharacterShape>>,
    mut colliders: Query<&mut CharacterColliders>,
    mut commands: Commands,
) {
    teardown_character_physics(
        trigger.event().0,
        &characters,
        &mut colliders,
        &mut commands,
    );
}

fn teardown_character_physics(
    character_entity: Entity,
    characters: &Query<(), With<CharacterShape>>,
    colliders: &mut Query<&mut CharacterColliders>,
    commands: &mut Commands,
) {
    if characters.get(character_entity).is_err() {
        return; // Transient: teardown raced character despawn.
    }
    let Ok(mut character_colliders) = colliders.get_mut(character_entity) else {
        return;
    };
    if let Some(ragdoll_entity) = character_colliders.ragdoll_entity.take() {
        commands.entity(ragdoll_entity).despawn();
    }
    character_colliders.collider_entities.clear();
    character_colliders.part_order.clear();
    commands
        .entity(character_entity)
        .remove::<CharacterColliders>()
        .remove::<NeedsColliders>()
        .remove::<MeasuredColliders>();
}

/// Rebuilds the ragdoll spec live when limits, mobility, or layers change.
/// Jolt bakes everything at creation, so tuning = respawn (same path as a
/// flip, minus the state change).
pub(crate) fn apply_joint_limit_overrides(
    mut characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &mut CharacterColliders,
        ),
        Or<(
            Changed<RagdollJointLimitOverrides>,
            Changed<RagdollMobility>,
            Changed<RagdollCollisionLayers>,
        )>,
    >,
    mut commands: Commands,
) {
 for (character_entity, ragdoll, character_colliders) in characters.iter_mut() {
        if matches!(ragdoll, CharacterRagdoll::None) {
            continue;
        }
        // Skip the respawn when nothing was ever spawned: the spec builds on
        // the next ragdoll flip, already carrying the new limits.
        if character_colliders.ragdoll_entity.is_none() {
            continue;
        }
        // Re-trigger the state system by touching the marker: remove + re-add
        // forces a `Changed` on the next tick.
        commands
            .entity(character_entity)
            .remove::<CharacterRagdoll>();
        commands
            .entity(character_entity)
            .insert(ragdoll.clone());
    }
}

const fn get_spherical_limits(bone: ColliderBone) -> (f32, f32) {
    match bone {
        ColliderBone::Chest | ColliderBone::Pelvis => (0.5, 0.3),
        ColliderBone::Head => (0.5, 0.3),
        ColliderBone::UpperRightArm | ColliderBone::UpperLeftArm => (1.5, 0.5),
        ColliderBone::UpperRightLeg | ColliderBone::UpperLeftLeg => (1.5, 0.3),
        ColliderBone::LeftHand | ColliderBone::RightHand => (0.3, 0.3),
        ColliderBone::LeftFoot | ColliderBone::RightFoot => (0.2, 0.1),
        _ => unreachable!(),
    }
}
