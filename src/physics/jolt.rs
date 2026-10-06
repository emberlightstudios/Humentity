//! Jolt ragdoll backend: mirrors `super::avian` with `bevy_jolt` bodies.
//!
//! Same architecture (bones-only hierarchy, separate world-root collider
//! entities, `ColliderOffset` sync both directions), same public names, so
//! game code swaps backends by flipping the `avian`/`jolt` feature.
//!
//! Jolt mapping notes (verified against `bevy_jolt` source):
//! - `RigidBody::Kinematic/Dynamic` flip → direct `JoltBody.motion` writes
//!   (`sync_jolt_motion` in `body_forces.rs` pushes `Changed` before the
//!   step); kinematic follow → `JoltKinematicTarget`
//! - Pose read/write → `Transform` + `JoltPhysicsWorld` FFI
//!   (`body_full_transform` / `teleport_body`); the crate's own
//!   `sync_body_transforms` also writes `Transform`, so our kinematic sync
//!   must run before the step, in `FixedUpdate`.
//! - `Collider::{sphere, cuboid, capsule}` → `JoltShape::{sphere,
//!   box_shape, capsule}`. Avian and Jolt agree on semantics: sphere takes a
//!   radius, box takes half extents, capsule takes cylinder half-height plus
//!   radius (parry excludes the hemispheres; Jolt's `CapsuleShapeSettings`
//!   likewise takes the cylinder section's half-height).
//! - `SphericalJoint` (swing/twist) → `JoltJoint::swing_twist`; `RevoluteJoint`
//!   (Z-axis hinge) → `JoltJoint::hinge_limited`. Both use
//!   `JointSpace::World`: the avian anchor is already a world position and
//!   the rest pose is baked by seating the bodies first.
//! - `JointCollisionDisabled` → `JoltWorld::set_bodies_no_collide` per pair.
//! - `RigidBodyDisabled`/`ColliderDisabled` (sleep parking) → direct
//!   `JoltSleeping.sleeping` writes; wake on ragdoll-off the same way.
//! - `RagdollCollisionLayers(CollisionLayers)` → `JoltBody::kinematic(layer)`
//!   / `dynamic(layer)` (`object_layer: u16`). Layers are per-body u16 team
//!   ids, not bitmasks; the layer table itself is fixed at `JoltPlugin`
//!   world creation, so this backend maps the avian membership bit to a
//!   layer index (see `jolt_layer_for`).

use ahash::AHashMap;
use bevy::ecs::intern::Internable;
use bevy::math::{Quat, Vec3};
use bevy_jolt::{
    JointSpace, JoltBody, JoltBodyId, JoltJoint, JoltKinematicTarget, JoltPhysicsWorld,
    JoltShape, JoltSleeping,
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
/// team `i`). When changed at runtime, all existing collider bodies move to
/// the new team.
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
#[require(RagdollDensity, RagdollDamping)]
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
/// collider and joint entities. Keeps them grouped under one world-root entity in
/// the inspector rather than polluting the root with one entity per collider/joint.
/// Plain identity entity: no `JoltBody`/`JoltShape`, so jolt ignores it.
#[derive(Resource)]
pub struct CharacterPhysicsContainer(pub Entity);

/// Linear + angular damping for one jolt collider body (0 = Jolt default glide).
///
/// `bevy_jolt` bakes bodies with no damping control, so this marker rides on
/// the collider entity and `apply_collider_damping` pushes it into Jolt once
/// the body owns a `JoltBodyId`. Mirrors avian's `LinearDamping(0.1)` /
/// `AngularDamping(0.1)` on every collider.
#[derive(Component, Clone, Copy, Debug)]
pub struct JoltDamping {
    pub linear_damping: f32,
    pub angular_damping: f32,
}

impl JoltDamping {
    pub const fn new(linear_damping: f32, angular_damping: f32) -> Self {
        Self {
            linear_damping,
            angular_damping,
        }
    }
}

/// Pushes [`JoltDamping`] into Jolt once the collider body is baked.
/// Body bake lands a flush after the collider spawns, so this polls until
/// `JoltBodyId` exists, then removes the marker. Missing bodies on live
/// colliders just mean "not baked yet".
pub(crate) fn apply_collider_damping(
    pending: Query<(Entity, &JoltDamping, &JoltBodyId)>,
    mut physics_world: ResMut<JoltPhysicsWorld>,
    mut commands: Commands,
) {
    for (collider_entity, damping, body_id) in pending.iter() {
        physics_world.set_body_damping(
            body_id.body_id_raw,
            damping.linear_damping,
            damping.angular_damping,
        );
        commands.entity(collider_entity).remove::<JoltDamping>();
    }
}

#[derive(Component, Default)]
pub struct CharacterColliders {
    pub bones_subset: Option<Vec<ColliderBone>>,
    pub collider_entities: AHashMap<ColliderBone, Entity>,
    /// Collider bone type → bone entity in the character's single fixed skeleton.
    /// Built once at collider spawn time, read-only afterwards.
    pub bone_entities: AHashMap<ColliderBone, Entity>,
    pub joint_entities: Vec<Entity>,
    /// Joint entity → the collider bone it constrains. Used to live-apply
    /// [`RagdollJointLimitOverrides`] without respawning joints.
    pub joint_bone: AHashMap<Entity, ColliderBone>,
}

impl CharacterColliders {
    pub fn new(bones_subset: Option<Vec<ColliderBone>>) -> Self {
        Self {
            bones_subset,
            collider_entities: AHashMap::default(),
            bone_entities: AHashMap::default(),
            joint_entities: Vec::new(),
            joint_bone: AHashMap::default(),
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
///
/// Single source of truth shared by joint spawn (`set_ragdoll_state`), live
/// tuning (`apply_joint_limit_overrides`), and debug tooling.
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

/// Per-bone joint limit overrides applied when ragdoll joints are spawned and
/// re-applied live whenever this component changes on a character.
///
/// Apply to your character entity. Any bone absent from the relevant map falls
/// back to [`default_joint_limit`] scaled by [`RagdollMobility`].
///
/// Use this to fine-tune joint limits while the ragdoll is active and watch for
/// visual artifacts as each degree of freedom reaches its limit.
///
/// NOTE: Jolt bakes limits at constraint creation, so unlike avian this backend
/// applies overrides by respawning the joints, not by patching them live.
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
#[derive(Component)]
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

pub(crate) fn spawn_colliders(
    mut commands: Commands,
    mut characters: Query<
        (
            Entity,
            &CharacterShape,
            &CharacterSkeleton,
            &mut CharacterColliders,
            &RagdollDensity,
            Option<&RagdollCollisionLayers>,
            Option<&CharacterScale>,
        ),
        (With<NeedsColliders>, With<SkeletonReady>),
    >,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<crate::template::CharacterTemplate>>,
    basemesh: Res<crate::basemesh::BaseMesh>,
    rig_data: Res<RigData>,
    container: Option<Res<CharacterPhysicsContainer>>,
) {
    // Create the single global container on first use, then reuse it for every
    // character so all colliders/joints share one world-root parent.
    let container_entity = match container {
        Some(container) => container.0,
        None => {
            let container_id = commands
                .spawn((Name::new("CharacterPhysics"), Transform::IDENTITY))
                .id();
            commands.insert_resource(CharacterPhysicsContainer(container_id));
            container_id
        }
    };

    const BATCH_SIZE: usize = 2;
    let mut spawned_character_count = 0;
    for (
        character_entity,
        character_shape,
        skeleton,
        mut colliders,
        density,
        collision_layers,
        character_scale,
    ) in characters.iter_mut()
    {
        if spawned_character_count >= BATCH_SIZE {
            break;
        }
        let Some(collision_layers) = collision_layers else {
            continue;
        };
        let density = density.0;

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

        // ── Spawn collider entities (offsets LOD-independent) ──
        let collider_scale = character_scale.map_or(1.0, |scale| scale.0);
        for &collider in &target_bones {
            let collider_slot = collider_index(collider);
            let (geometry, collider_to_model) = get_collider_geometry(
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
            let bone_entity = colliders.bone_entities.get(&collider).copied();
            let mut collider_commands = commands.spawn((
                Name::new(
                    NAME_INTERNER
                        .intern(&format!("Collider {joint_name}"))
                        .leak(),
                ),
                JoltBody::kinematic(collision_layers.membership).with_density(density),
                collider,
                geometry,
                ColliderOffset {
                    collider_to_bone: collider_to_joint,
                    bone_to_collider: joint_to_collider,
                },
                JoltDamping::new(0.1, 0.1),
                Transform::IDENTITY,
                ColliderForCharacter(character_entity),
            ));

            if let Some(bone_entity) = bone_entity {
                collider_commands.insert(BoneForCollider(bone_entity));
            }

            let collider_entity = collider_commands.id();
            commands.entity(container_entity).add_child(collider_entity);
            colliders
                .collider_entities
                .insert(collider, collider_entity);
        }

        commands
            .entity(character_entity)
            .remove::<NeedsColliders>();
        spawned_character_count += 1;
    }
}

/// Syncs kinematic character colliders to follow the skeletal bones.
///
/// Iterates all colliders directly and follows the linked bone's `GlobalTransform`.
/// Runs in `FixedUpdate` before the Jolt step (which lives in the `JoltStep`
/// schedule right after): writes `JoltKinematicTarget` (driven via
/// `MoveKinematic`, so followers shove dynamics aside) plus the `Transform`
/// the body bake reads for not-yet-baked bodies.
pub(crate) fn sync_colliders(
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    mut collider_data: Query<(
        &mut Transform,
        &mut JoltKinematicTarget,
        &ColliderOffset,
        &JoltBody,
        &BoneForCollider,
    )>,
) {
    for (mut collider_transform, mut kinematic_target, offset, body, bone_link) in
        collider_data.iter_mut()
    {
        if body.motion != bevy_jolt::JoltMotion::Kinematic {
            continue;
        }
        let Ok(joint_to_world) = bones.get(bone_link.0) else {
            continue; // Transient: bone gone before its collider despawns.
        };

        let target_world = Transform::from(*joint_to_world) * offset.collider_to_bone;
        // Keep the Bevy transform seated so freshly baked bodies spawn in
        // place and `set_ragdoll_state` reads current rest frames.
        collider_transform.translation = target_world.translation;
        collider_transform.rotation = target_world.rotation;
        // `ensure_kinematic_targets` covers colliders that lack the component;
        // only write here when the target actually moved.
        if kinematic_target.target_position != target_world.translation
            || kinematic_target.target_rotation != target_world.rotation
        {
            kinematic_target.target_position = target_world.translation;
            kinematic_target.target_rotation = target_world.rotation;
        }
    }
}

/// Ensures every kinematic collider entity carries a `JoltKinematicTarget`.
/// Runs after `sync_colliders`; separate query because `sync_colliders` only
/// matches entities that already have the component.
pub(crate) fn ensure_kinematic_targets(
    kinematic_colliders: Query<
        (Entity, &Transform, &JoltBody),
        (With<ColliderBone>, Without<JoltKinematicTarget>),
    >,
    mut commands: Commands,
) {
    for (collider_entity, collider_transform, body) in kinematic_colliders.iter() {
        if body.motion != bevy_jolt::JoltMotion::Kinematic {
            continue;
        }
        commands.entity(collider_entity).insert(JoltKinematicTarget {
            target_position: collider_transform.translation,
            target_rotation: collider_transform.rotation,
        });
    }
}

pub(crate) fn set_ragdoll_state(
    mut characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &mut CharacterColliders,
            &RagdollDamping,
        ),
        Changed<CharacterRagdoll>,
    >,
    mut commands: Commands,
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    collider_transforms: Query<&Transform, With<ColliderBone>>,
    collider_shapes: Query<&JoltShape>,
    collider_offsets: Query<&ColliderOffset>,
    body_ids: Query<&JoltBodyId>,
    mut collider_bodies: Query<(&mut JoltBody, Option<&mut JoltSleeping>)>,
    mobility_query: Query<&RagdollMobility>,
    overrides_query: Query<&RagdollJointLimitOverrides>,
    mut physics_world: ResMut<JoltPhysicsWorld>,
    container: Option<Res<CharacterPhysicsContainer>>,
) {
    let container_entity = container.map(|container| container.0);
    for (character_entity, ragdoll, mut character_colliders, damping) in characters.iter_mut() {
        let overrides = overrides_query.get(character_entity).ok();
        let mobility = mobility_query.get(character_entity).map_or(1.0, |mob| mob.0);
        let joint_damping = damping.0;

        let partial_bones: Option<&[ColliderBone]> = match ragdoll {
            CharacterRagdoll::Partial(bones) => Some(bones.as_slice()),
            _ => None,
        };
        // Joint teardown + respawn lives in `respawn_ragdoll_joints` after the
        // motion flip; the `None` arm clears joints itself before `continue`.
        // Motion goes straight into `JoltBody.motion`: `sync_jolt_motion`
        // pushes `Changed` values before the step, same tick, no trigger.
        // Sleeping state goes into `JoltSleeping.sleeping` the same way.

        match ragdoll {
            CharacterRagdoll::Full => {
                for &collider_entity in character_colliders.collider_entities.values() {
                    if let Ok((mut collider_body, sleeping)) =
                        collider_bodies.get_mut(collider_entity)
                    {
                        collider_body.motion = bevy_jolt::JoltMotion::Dynamic;
                        if let Some(mut sleeping) = sleeping {
                            sleeping.sleeping = false;
                        }
                    }
                    commands
                        .entity(collider_entity)
                        .remove::<JoltKinematicTarget>();
                }
            }
            CharacterRagdoll::Partial(bones) => {
                for (bone, &collider_entity) in character_colliders.collider_entities.iter() {
                    if bones.contains(bone) {
                        if let Ok((mut collider_body, sleeping)) =
                            collider_bodies.get_mut(collider_entity)
                        {
                            collider_body.motion = bevy_jolt::JoltMotion::Dynamic;
                            if let Some(mut sleeping) = sleeping {
                                sleeping.sleeping = false;
                            }
                        }
                        commands
                            .entity(collider_entity)
                            .remove::<JoltKinematicTarget>();
                    } else if let Ok((mut collider_body, _)) =
                        collider_bodies.get_mut(collider_entity)
                    {
                        collider_body.motion = bevy_jolt::JoltMotion::Kinematic;
                    }
                }
            }
            CharacterRagdoll::None => {
                for &collider_entity in character_colliders.collider_entities.values() {
                    if let Ok((mut collider_body, sleeping)) =
                        collider_bodies.get_mut(collider_entity)
                    {
                        collider_body.motion = bevy_jolt::JoltMotion::Kinematic;
                        if let Some(mut sleeping) = sleeping {
                            sleeping.sleeping = false;
                        }
                    }
                }
                for &joint_entity in &character_colliders.joint_entities {
                    commands.entity(joint_entity).despawn();
                }
                character_colliders.joint_entities.clear();
                character_colliders.joint_bone.clear();
                continue;
            }
        }

        respawn_ragdoll_joints(
            character_entity,
            &mut character_colliders,
            partial_bones,
            overrides,
            mobility,
            joint_damping,
            &mut commands,
            &bones,
            &collider_transforms,
            &collider_shapes,
            &collider_offsets,
            &body_ids,
            &mut physics_world,
            container_entity,
        );
    }
}

/// Despawns all joints on a character and respawns them from the current
/// collider poses with freshly resolved limits. Shared by `set_ragdoll_state`
/// (after the motion flip) and `apply_joint_limit_overrides` (limits changed,
/// motion untouched).
#[allow(clippy::too_many_arguments)]
fn respawn_ragdoll_joints(
    character_entity: Entity,
    character_colliders: &mut CharacterColliders,
    partial_bones: Option<&[ColliderBone]>,
    overrides: Option<&RagdollJointLimitOverrides>,
    mobility: f32,
    joint_damping: f32,
    commands: &mut Commands,
    bones: &Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    collider_transforms: &Query<&Transform, With<ColliderBone>>,
    collider_shapes: &Query<&JoltShape>,
    collider_offsets: &Query<&ColliderOffset>,
    body_ids: &Query<&JoltBodyId>,
    physics_world: &mut JoltPhysicsWorld,
    container_entity: Option<Entity>,
) {
    for &joint_entity in &character_colliders.joint_entities {
        commands.entity(joint_entity).despawn();
    }
    character_colliders.joint_entities.clear();
    character_colliders.joint_bone.clear();

    let joints_to_spawn: Vec<(
        ColliderBone,
        Entity,
        Entity,
        Vec3,
        Transform,
        Transform,
        RagdollJointLimit,
    )> = character_colliders
        .collider_entities
        .iter()
        .filter_map(|(bone, &child)| {
            if let Some(bones) = partial_bones
                && !bones.contains(bone)
                && !get_collider_parent(*bone).is_some_and(|parent| bones.contains(&parent))
            {
                return None;
            }
            let parent_bone_type = get_collider_parent(*bone)?;
            let parent = *character_colliders.collider_entities.get(&parent_bone_type)?;

            let child_bone = *character_colliders.bone_entities.get(bone)?;
            let parent_bone = *character_colliders.bone_entities.get(&parent_bone_type)?;
            // All `?` skips below are transient mid-spawn states (colliders or
            // bone links not yet in place); the next `set_ragdoll_state` run
            // retries them.
            let parent_offset = collider_offsets.get(parent).ok()?;
            let child_offset = collider_offsets.get(child).ok()?;

            // Joint rest frames come from the collider entities as they stand:
            // tooling seats these at the bind pose (direct writes, visible
            // immediately) before flipping `CharacterRagdoll`, while bone
            // `GlobalTransform`s still hold last frame's displaced pose until
            // the next `PostUpdate` propagate — reading them here baked the
            // swung pose into every respawned joint. Bone globals remain only
            // as a fallback for freshly spawned colliders with no pose yet.
            let parent_collider = match collider_transforms.get(parent) {
                Ok(collider_transform) => *collider_transform,
                Err(_) => {
                    let world = bones.get(parent_bone).ok()?;
                    Transform::from(*world) * parent_offset.collider_to_bone
                }
            };
            let child_collider = match collider_transforms.get(child) {
                Ok(collider_transform) => *collider_transform,
                Err(_) => {
                    let world = bones.get(child_bone).ok()?;
                    Transform::from(*world) * child_offset.collider_to_bone
                }
            };

            // Anchor: the child bone's origin is the anatomical pivot for
            // limbs (knee, elbow, ankle, ...), but `spine03` sits high in
            // the torso (bind z ~ 0.20 vs pelvis ~ 0.06), so a bone-derived
            // pelvis->chest anchor hinges the chest at its top. Chest
            // instead pivots at the waist: top-center of the parent
            // (pelvis) collider. Its local +Y is model-up: midsection
            // boxes are measured axis-aligned in model space.
            // Anchors come from bone globals (which carry CharacterScale
            // via the skeleton root). Collider entities have no scale, so a
            // `child_collider * collider_to_bone` product mixes scaled and
            // unscaled frames — at scale 2 it lands halfway and joints
            // float apart. Bone globals stay in one scaled frame.
            let anchor_world = if *bone == ColliderBone::Chest {
                collider_shapes
                    .get(parent)
                    .ok()
                    .and_then(|shape| match shape.0.as_ref() {
                        bevy_jolt::PhysicsShape::Box { half_extents } => Some(*half_extents),
                        _ => None,
                    })
                    .map(|half_extents| {
                        // Cuboid half extents are built scaled for the
                        // character (collider geometry carries the
                        // CharacterScale), so this stays at the scaled
                        // waist without extra math.
                        parent_collider.transform_point(Vec3::new(
                            0.0,
                            half_extents.y,
                            0.0,
                        ))
                    })
                    .unwrap_or_else(|| {
                        bones
                            .get(child_bone)
                            .map(|world| world.translation())
                            .unwrap_or(child_collider.translation)
                    })
            } else {
                bones
                    .get(child_bone)
                    .map(|world| world.translation())
                    .unwrap_or(child_collider.translation)
            };
            // Resolve the limit for this joint: explicit override wins,
            // otherwise the anatomical default; scaled by mobility.
            let limit = resolve_joint_limit(*bone, overrides, mobility);

            Some((
                *bone,
                parent,
                child,
                anchor_world,
                parent_collider,
                child_collider,
                limit,
            ))
        })
        .collect();

    for (bone, parent, child, anchor, parent_collider, child_collider, limit) in joints_to_spawn
    {
        if collider_transforms.get(parent).is_err() {
            commands.entity(parent).insert(parent_collider);
        }
        if collider_transforms.get(child).is_err() {
            commands.entity(child).insert(child_collider);
        }
        // Bodies missing their ids are not baked yet; the pair files once both
        // endpoints exist (see `ensure_joint_no_collide` below).
        if let (Ok(parent_id), Ok(child_id)) = (body_ids.get(parent), body_ids.get(child)) {
            physics_world.set_bodies_no_collide(parent_id.body_id_raw, child_id.body_id_raw);
            // Jolt has no joint damping: hold the relative motion by damping
            // both endpoints instead. Knees carry the weight above, like avian's
            // 20x knee damping.
            let endpoint_damping = match bone {
                ColliderBone::LowerRightLeg | ColliderBone::LowerLeftLeg => joint_damping * 20.0,
                _ => joint_damping,
            };
            physics_world.set_body_damping(
                parent_id.body_id_raw,
                endpoint_damping,
                endpoint_damping,
            );
            physics_world.set_body_damping(
                child_id.body_id_raw,
                endpoint_damping,
                endpoint_damping,
            );
        }
        let joint = spawn_ragdoll_joint(
            commands,
            bone,
            parent,
            child,
            anchor,
            limit,
            parent_collider.rotation,
            child_collider.rotation,
            character_entity,
            container_entity,
        );
        // Pairs whose bodies are not baked yet file later: `ensure_joint_no_collide`
        // polls these markers until both endpoints own a `JoltBodyId`.
        if body_ids.get(parent).is_err() || body_ids.get(child).is_err() {
            commands.entity(joint).insert(NeedsNoCollide { parent, child });
        }
        character_colliders.joint_entities.push(joint);
        character_colliders.joint_bone.insert(joint, bone);
    }
}

/// Marker for a ragdoll joint whose linked bodies were not baked yet when it
/// spawned, so the no-collide pair could not file. Removed once filed.
#[derive(Component)]
pub(crate) struct NeedsNoCollide {
    parent: Entity,
    child: Entity,
}

/// Files pending no-collide pairs once both linked bodies own a `JoltBodyId`.
/// Body bake lands a flush after the collider spawns, so joints spawned in the
/// same tick usually miss it; this polls instead of blocking the ragdoll flip.
pub(crate) fn ensure_joint_no_collide(
    pending: Query<(Entity, &NeedsNoCollide)>,
    body_ids: Query<&JoltBodyId>,
    bodies: Query<(), With<JoltBody>>,
    mut commands: Commands,
    mut physics_world: ResMut<JoltPhysicsWorld>,
) {
    for (joint_entity, pending_pair) in pending.iter() {
        // An endpoint with neither body marker nor id is gone for good
        // (despawned mid-bake): drop the marker instead of retrying forever.
        let endpoint_gone =
            |body_entity: Entity| body_ids.get(body_entity).is_err() && !bodies.contains(body_entity);
        if endpoint_gone(pending_pair.parent) || endpoint_gone(pending_pair.child) {
            commands.entity(joint_entity).remove::<NeedsNoCollide>();
            continue;
        }
        let (Ok(parent_id), Ok(child_id)) = (
            body_ids.get(pending_pair.parent),
            body_ids.get(pending_pair.child),
        ) else {
            continue; // Transient: bodies still baking, retry next tick.
        };
        physics_world.set_bodies_no_collide(parent_id.body_id_raw, child_id.body_id_raw);
        commands.entity(joint_entity).remove::<NeedsNoCollide>();
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
) -> (JoltShape, Transform) {
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

fn get_head_collider(helpers: &[Vec3], scale: f32) -> (JoltShape, Transform) {
    let center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let radius = (helpers[HEAD_VERTICES[0]] - center).length() * scale;
    (
        JoltShape::sphere(radius),
        Transform::from_translation(center),
    )
}

fn get_midsection_collider(
    helpers: &[Vec3],
    joint: ColliderBone,
    bind_rot: Quat,
    scale: f32,
) -> (JoltShape, Transform) {
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
            JoltShape::box_shape(Vec3::new(
                (xmax - xmin) * scale * 0.5,
                (zmax - zmin) * scale * 0.5,
                (ymax - ymin) * scale * 0.5,
            )),
            Transform::from_translation(center).with_rotation(
                bind_rot * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            ),
        )
    } else {
        (
            JoltShape::box_shape(Vec3::new(
                (xmax - xmin) * scale * 0.5,
                (ymax - ymin) * scale * 0.5,
                (zmax - zmin) * scale * 0.5,
            )),
            Transform::from_translation(center).with_rotation(bind_rot),
        )
    }
}

fn get_limb_collider(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (JoltShape, Transform) {
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
        JoltShape::capsule((segment_start - segment_end).length() * scale * 0.5, radius),
        Transform::from_translation(center)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(fwd, dir, up))),
    )
}

fn get_extremity_collider(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (JoltShape, Transform) {
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
        JoltShape::box_shape(Vec3::new(x * 0.5, y * 0.5, z * 0.5)),
        Transform::from_translation(center)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(x_axis, y_axis, z_axis))),
    )
}

/// Human limb twist runs along body-local +Y (limb segments are measured with
/// +Y down the bone, matching avian's Y-twist spherical joints). Hinge flexion
/// runs about body-local +Z (matching avian's default Z hinge axis). Both are
/// derived from the seated body rotations in `spawn_ragdoll_joint`.
///
/// Baked rest orientation of a ragdoll joint: the relative rotation between
/// the two linked collider bodies as seated when the joint spawned.
///
/// Debug tooling reads this to compare the live relative angle against the
/// rest pose (in degrees) and see whether each joint sits inside its limits.
#[derive(Component, Clone, Copy, Debug)]
pub struct JointRestPose {
    pub parent_entity: Entity,
    pub child_entity: Entity,
    pub parent_rest_rotation: Quat,
    pub child_rest_rotation: Quat,
    pub rest_relative: Quat,
}

impl JointRestPose {
    /// Live relative rotation between the two bodies, in the same frame as
    /// `rest_relative`: identity means the joint sits exactly at rest.
    pub fn live_relative(
        &self,
        parent_rotation: Quat,
        child_rotation: Quat,
    ) -> Quat {
        parent_rotation.inverse() * self.rest_relative * child_rotation
    }

    /// Angle in degrees between the live relative rotation and rest.
    /// 0 = seated pose; compare against the joint's swing/twist limits.
    pub fn angle_from_rest_degrees(
        &self,
        parent_rotation: Quat,
        child_rotation: Quat,
    ) -> f32 {
        let live = self.live_relative(parent_rotation, child_rotation);
        2.0 * live.w.clamp(-1.0, 1.0).acos().to_degrees()
    }
}

fn spawn_ragdoll_joint(
    commands: &mut Commands,
    bone: ColliderBone,
    parent: Entity,
    child: Entity,
    anchor: Vec3,
    limit: RagdollJointLimit,
    parent_rest_rotation: Quat,
    child_rest_rotation: Quat,
    character: Entity,
    container_entity: Option<Entity>,
) -> Entity {
    let joint_name: &'static str = NAME_INTERNER
        .intern(&format!(
            "Joint {}",
            DEFAULT_RIG_COLLIDER_BONE_NAMES[collider_index(bone)]
        ))
        .leak();
    // Rest orientation: Jolt's world-space joints take one anchor plus
    // per-body axes. The seated body rotations ARE the rest pose, so derive
    // every axis from them: the seated pose sits at zero inside the limits.
    // Hinges flex about the body-local Z (matching avian's default hinge
    // axis); swing-twist runs twist along body-local Y (limb segments are
    // measured with +Y down the bone, matching avian's Y-twist joints).
    let hinge_axis_parent = Dir3::new(parent_rest_rotation * Vec3::Z)
        .unwrap_or(Dir3::Z);
    let hinge_normal_parent = Dir3::new(parent_rest_rotation * Vec3::Y)
        .unwrap_or(Dir3::Y);
    let hinge_axis_child = Dir3::new(child_rest_rotation * Vec3::Z)
        .unwrap_or(Dir3::Z);
    let hinge_normal_child = Dir3::new(child_rest_rotation * Vec3::Y)
        .unwrap_or(Dir3::Y);
    let twist_axis_parent = Dir3::new(parent_rest_rotation * Vec3::Y)
        .unwrap_or(Dir3::Y);
    let plane_axis_parent = Dir3::new(parent_rest_rotation * Vec3::X)
        .unwrap_or(Dir3::X);
    let twist_axis_child = Dir3::new(child_rest_rotation * Vec3::Y)
        .unwrap_or(Dir3::Y);
    let plane_axis_child = Dir3::new(child_rest_rotation * Vec3::X)
        .unwrap_or(Dir3::X);
    let rest_relative = parent_rest_rotation.inverse() * child_rest_rotation;
    let rest_pose = JointRestPose {
        parent_entity: parent,
        child_entity: child,
        parent_rest_rotation,
        child_rest_rotation,
        rest_relative,
    };
    let joint = match bone {
        ColliderBone::LowerRightArm
        | ColliderBone::LowerLeftArm
        | ColliderBone::LowerRightLeg
        | ColliderBone::LowerLeftLeg => commands
            .spawn((
                Name::new(joint_name),
                JoltJoint::hinge_limited(
                    parent,
                    child,
                    anchor,
                    hinge_axis_parent,
                    hinge_normal_parent,
                    hinge_axis_child,
                    hinge_normal_child,
                    limit.angle_min,
                    limit.angle_max,
                    JointSpace::World,
                ),
                rest_pose,
                JointForCharacter(character),
            ))
            .id(),
        _ => commands
            .spawn((
                Name::new(joint_name),
                JoltJoint::swing_twist(
                    parent,
                    child,
                    anchor,
                    twist_axis_parent,
                    plane_axis_parent,
                    twist_axis_child,
                    plane_axis_child,
                    limit.swing,
                    limit.swing,
                    -limit.twist,
                    limit.twist,
                    JointSpace::World,
                ),
                rest_pose,
                JointForCharacter(character),
            ))
            .id(),
    };
    if let Some(container_entity) = container_entity {
        commands.entity(container_entity).add_child(joint);
    }
    joint
}

/// Event to atomically disable physics on a character's collider entities.
/// Despawns all ragdoll joints and sleeps every collider, so the body rests
/// in place without simulating. Waking happens on the next ragdoll-off flip.
#[derive(Event, Debug, Clone)]
pub struct DisablePhysics {
    pub character: Entity,
}

pub(crate) fn on_disable_physics(
    trigger: On<DisablePhysics>,
    mut characters: Query<&mut CharacterColliders>,
    mut sleeping_flags: Query<&mut JoltSleeping>,
    mut commands: Commands,
) {
    let event = trigger.event();
    let Ok(mut colliders) = characters.get_mut(event.character) else {
        return; // Transient: character torn down before the event flushes.
    };

    for joint in colliders.joint_entities.drain(..) {
        commands.entity(joint).despawn();
    }

    for &collider_entity in colliders.collider_entities.values() {
        if let Ok(mut sleeping) = sleeping_flags.get_mut(collider_entity) {
            sleeping.sleeping = true;
        }
    }
}
/// Observer: on [`TeardownCharacter`] or [`RefitCharacter`], tear down all
/// humentity-owned physics state so only a bare state blob remains. This pairs
/// with the skeleton/mesh cleanup in `spawn_skeleton`.
///
/// Despawns every jolt collider and joint (tracked via the bevy_relationships
/// lists) and removes `CharacterColliders` and `NeedsColliders` from the character
/// root. User-config components (`CharacterRagdoll`, `RagdollDensity`,
/// `RagdollDamping`, `RagdollMobility`, `RagdollCollisionLayers`) are left intact so
/// the character can be re-activated later.
pub(crate) fn on_teardown_character(
    trigger: On<TeardownCharacter>,
    characters: Query<(), With<CharacterShape>>,
    collider_lists: Query<&ColliderList>,
    joint_lists: Query<&JointList>,
    mut commands: Commands,
) {
    teardown_character_physics(trigger.event().0, &characters, &collider_lists, &joint_lists, &mut commands);
}

/// Observer for [`RefitCharacter`]: same physics teardown, so the collider
/// spawn re-runs from current morph weights after the fit rebuilds.
pub(crate) fn on_refit_character(
    trigger: On<RefitCharacter>,
    characters: Query<(), With<CharacterShape>>,
    collider_lists: Query<&ColliderList>,
    joint_lists: Query<&JointList>,
    mut commands: Commands,
) {
    teardown_character_physics(trigger.event().0, &characters, &collider_lists, &joint_lists, &mut commands);
}

fn teardown_character_physics(
    character_entity: Entity,
    characters: &Query<(), With<CharacterShape>>,
    collider_lists: &Query<&ColliderList>,
    joint_lists: &Query<&JointList>,
    commands: &mut Commands,
) {
    if characters.get(character_entity).is_err() {
        return; // Transient: teardown raced character despawn.
    }

    if let Ok(list) = collider_lists.get(character_entity) {
        for &collider_entity in list.0.iter() {
            commands.entity(collider_entity).despawn();
        }
    }
    if let Ok(list) = joint_lists.get(character_entity) {
        for &joint_entity in list.0.iter() {
            commands.entity(joint_entity).despawn();
        }
    }

    commands
        .entity(character_entity)
        .remove::<CharacterColliders>()
        .remove::<NeedsColliders>();
}

/// Re-applies [`RagdollJointLimitOverrides`] to live joints whenever the
/// overrides component changes on a character. Jolt bakes limits at creation,
/// so this respawns the joints through the shared `respawn_ragdoll_joints`
/// helper (same path as `set_ragdoll_state`, minus the motion flip).
pub(crate) fn apply_joint_limit_overrides(
    mut characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &mut CharacterColliders,
            Option<&RagdollJointLimitOverrides>,
            Option<&RagdollMobility>,
            Option<&RagdollDamping>,
        ),
        Or<(
            Changed<RagdollJointLimitOverrides>,
            Changed<RagdollMobility>,
            Changed<RagdollDamping>,
        )>,
    >,
    mut commands: Commands,
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    collider_transforms: Query<&Transform, With<ColliderBone>>,
    collider_shapes: Query<&JoltShape>,
    collider_offsets: Query<&ColliderOffset>,
    body_ids: Query<&JoltBodyId>,
    mut physics_world: ResMut<JoltPhysicsWorld>,
    container: Option<Res<CharacterPhysicsContainer>>,
) {
    let container_entity = container.map(|container| container.0);
    for (character_entity, ragdoll, mut character_colliders, overrides, mobility, damping) in
        characters.iter_mut()
    {
        let partial_bones: Option<&[ColliderBone]> = match ragdoll {
            CharacterRagdoll::None => continue,
            CharacterRagdoll::Partial(bones) => Some(bones.as_slice()),
            CharacterRagdoll::Full => None,
        };
        // Skip the respawn when nothing was ever spawned: joints appear on
        // the next ragdoll flip, already carrying the new limits.
        if character_colliders.joint_entities.is_empty()
            && character_colliders.collider_entities.is_empty()
        {
            continue;
        }
        respawn_ragdoll_joints(
            character_entity,
            &mut character_colliders,
            partial_bones,
            overrides,
            mobility.map_or(1.0, |mob| mob.0),
            damping.map_or_else(|| RagdollDamping::default().0, |damping| damping.0),
            &mut commands,
            &bones,
            &collider_transforms,
            &collider_shapes,
            &collider_offsets,
            &body_ids,
            &mut physics_world,
            container_entity,
        );
    }
}

/// Propagates [`RagdollCollisionLayers`] changes from the character entity
/// to all of its spawned collider bodies at runtime, by re-issuing the body
/// descriptors with the new team index.
pub(crate) fn update_collision_layers(
    characters: Query<(&RagdollCollisionLayers, &ColliderList), Changed<RagdollCollisionLayers>>,
    body_query: Query<&JoltBody>,
    mut commands: Commands,
) {
    for (layers, collider_list) in characters.iter() {
        for &collider_entity in collider_list.0.iter() {
            let Ok(body) = body_query.get(collider_entity) else {
                continue; // Transient: collider despawned before layers flush.
            };
            let mut rebased_body = *body;
            rebased_body.object_layer = layers.membership;
            commands.entity(collider_entity).insert(rebased_body);
        }
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
