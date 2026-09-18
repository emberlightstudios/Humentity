use ahash::AHashMap;
use avian3d::prelude::*;
use bevy::{
    math::{Quat, Vec3},
    prelude::*,
};
use bevy::ecs::intern::Internable;
use crate::{
    helpers::HelperVertexPositions,
    prelude::{CharacterShape, CharacterSkeleton, SkeletonLodDisabled, SkeletonsReady},
    rigs::{RigData, SkeletalBone},
    spawn_skeleton::CharacterScale,
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

/// Single global, static (identity, world-root) entity that holds all characters'
/// collider and joint entities. Keeps them grouped under one world-root entity in
/// the inspector rather than polluting the root with one entity per collider/joint.
/// Plain identity entity: no `RigidBody`/`Collider`, so avian ignores it.
#[derive(Resource)]
pub struct CharacterPhysicsContainer(pub Entity);

#[derive(Component, Default)]
pub struct CharacterColliders {
    pub bones_subset: Option<Vec<ColliderBone>>,
    pub collider_entities: AHashMap<ColliderBone, Entity>,
    /// Collider bone type → bone entity in the character's single fixed skeleton.
    /// Built once at collider spawn time, read-only afterwards.
    pub bone_entities: AHashMap<ColliderBone, Entity>,
    pub(crate) joint_entities: Vec<Entity>,
    /// Joint entity → the collider bone it constrains. Used to live-apply
    /// [`RagdollJointLimitOverrides`] without respawning joints.
    pub(crate) joint_bone: AHashMap<Entity, ColliderBone>,
    /// Canonical local-space bone transforms computed from the collider positions
    /// during active ragdoll, written back to the single skeleton's bones.
    pub(crate) bone_transforms: [Transform; COLLIDERS.len()],
}

impl CharacterColliders {
    pub fn new(bones_subset: Option<Vec<ColliderBone>>) -> Self {
        Self {
            bones_subset,
            collider_entities: AHashMap::default(),
            bone_entities: AHashMap::default(),
            joint_entities: Vec::new(),
            joint_bone: AHashMap::default(),
            bone_transforms: [Transform::IDENTITY; COLLIDERS.len()],
        }
    }
}

/// The resolved rotational limits for a single ragdoll joint.
///
/// Spherical joints (shoulders, hips, spine, neck, hands, feet) use `swing`
/// (cone half-angle in radians) and `twist` (twist half-angle in radians).
/// Revolute joints (elbows, knees) use `angle_min`/`angle_max` (radians).
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
    let r = mobility.clamp(0.0, 1.0);
    let mut limit = overrides
        .and_then(|o| o.get(bone))
        .unwrap_or_else(|| default_joint_limit(bone));
    limit.swing *= r;
    limit.twist *= r;
    limit.angle_min *= r;
    limit.angle_max *= r;
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
#[derive(Component, Clone, Default, Debug)]
pub struct RagdollJointLimitOverrides {
    /// Spherical joints: bone → (swing half-angle, twist half-angle), radians.
    pub spherical: AHashMap<ColliderBone, (f32, f32)>,
    /// Revolute joints: bone → (min angle, max angle), radians.
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
            &CharacterSkeleton,
            &mut CharacterColliders,
            &RagdollDensity,
            Option<&RagdollCollisionLayers>,
            Option<&HelperVertexPositions>,
            Option<&CharacterScale>,
        ),
        (With<NeedsColliders>, With<SkeletonsReady>),
    >,
    rig_data: Res<RigData>,
    container: Option<Res<CharacterPhysicsContainer>>,
) {
    // Create the single global container on first use, then reuse it for every
    // character so all colliders/joints share one world-root parent.
    let container_entity = match container {
        Some(container) => container.0,
        None => {
            let entity = commands
                .spawn((Name::new("CharacterPhysics"), Transform::IDENTITY))
                .id();
            commands.insert_resource(CharacterPhysicsContainer(entity));
            entity
        }
    };

    const BATCH_SIZE: usize = 2;
    let mut char_count = 0;
    for (
        character_entity,
        _character_shape,
        skeleton,
        mut colliders,
        density,
        collision_layers,
        computed_helpers,
        character_scale,
    ) in characters.iter_mut()
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

        let Some(rig_spec) = rig_data.0.as_ref() else {
            continue;
        };
        let reference_order = &rig_spec.reference_rig.bone_names;

        // Build inverse-bindpose map (name → inv bindpose) from the single skeleton.
        let mut inv_bindposes_map: AHashMap<&str, Transform> = AHashMap::default();
        for (i, &name) in reference_order.iter().enumerate() {
            inv_bindposes_map.insert(
                name,
                Transform::from_matrix(skeleton.model_space_inv_bindposes[i]),
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
            let i_collider = collider_index(collider);
            let joint_name = collider_bone_map[i_collider];
            if let Some(&entity) = skeleton
                .bone_map
                .get(NAME_INTERNER.intern(joint_name).leak())
            {
                colliders.bone_entities.insert(collider, entity);
            }
        }

        // ── Spawn collider entities (offsets LOD-independent) ──
        // Uniform scale only: a non-uniform CharacterScale stretches bones in
        // ways spheres/capsules can't match, so use the x axis.
        let collider_scale = character_scale.map_or(1.0, |s| s.0.x);
        for &collider in &target_bones {
            let i_collider = collider_index(collider);
            let (geometry, collider_to_model) = get_collider_geometry(
                collider,
                helpers,
                i_collider,
                &inv_bindposes_map,
                collider_scale,
            );

            let joint_name = collider_bone_map[i_collider];
            let model_to_joint = inv_bindposes_map[joint_name];
            let collider_to_joint = model_to_joint * collider_to_model;
            let joint_to_collider = Transform::from_matrix(collider_to_joint.to_matrix().inverse());
            let bone_entity = colliders.bone_entities.get(&collider).copied();
            let collider_entity = commands
                .spawn((
                    Name::new(
                        NAME_INTERNER
                            .intern(&format!("Collider {joint_name}"))
                            .leak(),
                    ),
                    RigidBody::Kinematic,
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
                    SleepingDisabled,
                ))
                .id();

            if let Some(bone_entity) = bone_entity {
                commands
                    .entity(collider_entity)
                    .insert(BoneForCollider(bone_entity));
            }

            commands.entity(container_entity).add_child(collider_entity);
            colliders
                .collider_entities
                .insert(collider, collider_entity);
        }

        commands
            .entity(character_entity)
            .remove::<NeedsColliders>();
        char_count += 1;
    }
}

/// This function syncs kinematic character colliders to align with the skeletal bones.
/// Iterates all colliders directly and follows the linked bone's `GlobalTransform`.
pub(crate) fn sync_colliders(
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    mut collider_data: Query<(
        &mut Position,
        &mut Rotation,
        &ColliderOffset,
        &RigidBody,
        &BoneForCollider,
    )>,
) {
    for (mut position, mut rotation, offset, rigidbody, bone_link) in collider_data.iter_mut() {
        if *rigidbody != RigidBody::Kinematic {
            continue;
        }
        let Ok(joint_to_world) = bones.get(bone_link.0) else {
            continue;
        };

        let world = Transform::from(*joint_to_world) * offset.collider_to_bone;
        *position = Position(world.translation);
        *rotation = Rotation(world.rotation);
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
    bodies: Query<(&Position, &Rotation), With<ColliderBone>>,
    collider_shapes: Query<&Collider>,
    collider_offsets: Query<&ColliderOffset>,
    mobility_query: Query<&RagdollMobility>,
    overrides_query: Query<&RagdollJointLimitOverrides>,
    container: Option<Res<CharacterPhysicsContainer>>,
) {
    let container_entity = container.map(|c| c.0);
    for (character_entity, ragdoll, mut char_colliders, damping) in characters.iter_mut() {
        let damping = damping.0;
        let joint_damping = JointDamping {
            linear: damping,
            angular: damping,
        };
        let overrides = overrides_query.get(character_entity).ok();

        let partial_bones: Option<&[ColliderBone]> = match ragdoll {
            CharacterRagdoll::Partial(bones) => Some(bones.as_slice()),
            _ => None,
        };
        // Clear existing joints before any ragdoll transition
        for &joint in &char_colliders.joint_entities {
            commands.entity(joint).despawn();
        }
        char_colliders.joint_entities.clear();
        char_colliders.joint_bone.clear();

        match ragdoll {
            CharacterRagdoll::Full => {
                for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
                    commands.entity(collider_entity).insert(RigidBody::Dynamic);
                }
            }
            CharacterRagdoll::Partial(bones) => {
                for (bone, &collider_entity) in char_colliders.collider_entities.iter() {
                    commands
                        .entity(collider_entity)
                        .insert(if bones.contains(bone) {
                            RigidBody::Dynamic
                        } else {
                            RigidBody::Kinematic
                        });
                }
            }
            CharacterRagdoll::None => {
                for (_bone, &collider_entity) in char_colliders.collider_entities.iter() {
                    commands
                        .entity(collider_entity)
                        .insert(RigidBody::Kinematic);
                }
                continue;
            }
        }

        let joints_to_spawn: Vec<(
            ColliderBone,
            Entity,
            Entity,
            Vec3,
            Quat,
            Transform,
            Transform,
            RagdollJointLimit,
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

                let child_bone = *char_colliders.bone_entities.get(bone)?;
                let parent_bone = *char_colliders.bone_entities.get(&parent_bone_type)?;

                let parent_offset = collider_offsets.get(parent).ok()?;
                let child_offset = collider_offsets.get(child).ok()?;

                // Joint rest frames come from the collider bodies as they stand:
                // tooling seats these at the bind pose (direct writes, visible
                // immediately) before flipping `CharacterRagdoll`, while bone
                // `GlobalTransform`s still hold last frame's displaced pose until
                // the next `PostUpdate` propagate — reading them here baked the
                // swung pose into every respawned joint. Bone globals remain only
                // as a fallback for freshly spawned colliders with no body yet.
                let parent_collider = match bodies.get(parent) {
                    Ok((position, rotation)) => {
                        Transform::from_translation(position.0).with_rotation(rotation.0)
                    }
                    Err(_) => {
                        let world = bones.get(parent_bone).ok()?;
                        Transform::from(*world) * parent_offset.collider_to_bone
                    }
                };
                let child_collider = match bodies.get(child) {
                    Ok((position, rotation)) => {
                        Transform::from_translation(position.0).with_rotation(rotation.0)
                    }
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
                // via the skeleton root). Collider bodies have no scale, so a
                // `child_collider * collider_to_bone` product mixes scaled and
                // unscaled frames — at scale 2 it lands halfway and joints
                // float apart. Bone globals stay in one scaled frame.
                let anchor_world = if *bone == ColliderBone::Chest {
                    collider_shapes
                        .get(parent)
                        .ok()
                        .and_then(|shape| shape.shape().as_cuboid())
                        .map(|cuboid| {
                            // Cuboid half extents are built scaled for the
                            // character (collider geometry carries the
                            // CharacterScale), so this stays at the scaled
                            // waist without extra math.
                            parent_collider.transform_point(Vec3::new(
                                0.0,
                                cuboid.half_extents.y,
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
                // basis2 aligns the two *body* frames so the as-seated relative
                // rotation is the rest position (frame1 keeps identity basis).
                let local_basis2 =
                    child_collider.rotation.inverse() * parent_collider.rotation;

                // Resolve the limit for this joint: explicit override wins,
                // otherwise the anatomical default; scaled by mobility.
                let mobility = mobility_query.get(character_entity).map_or(1.0, |m| m.0);
                let limit = resolve_joint_limit(*bone, overrides, mobility);

                Some((
                    *bone,
                    parent,
                    child,
                    anchor_world,
                    local_basis2,
                    parent_collider,
                    child_collider,
                    limit,
                ))
            })
            .collect();

        for (bone, parent, child, anchor, local_basis2, parent_collider, child_collider, limit) in
            joints_to_spawn
        {
            // Seat bodies that have no transform yet (newly spawned colliders).
            // Seated bodies are already correct and must not be rewritten:
            // identical values would still trip change detection.
            if bodies.get(parent).is_err() {
                commands.entity(parent).insert((
                    Position(parent_collider.translation),
                    Rotation(parent_collider.rotation),
                ));
            }
            if bodies.get(child).is_err() {
                commands.entity(child).insert((
                    Position(child_collider.translation),
                    Rotation(child_collider.rotation),
                ));
            }
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
                limit,
                joint_damping,
                knee_damping,
                character_entity,
                container_entity,
            );
            char_colliders.joint_entities.push(joint);
            char_colliders.joint_bone.insert(joint, bone);
        }
    }
}

pub(crate) fn sync_bones_to_ragdoll(
    mut characters: Query<(&CharacterRagdoll, &mut CharacterColliders)>,
    colliders: Query<
        (&Position, &Rotation, &ColliderOffset),
        (With<ColliderBone>, Without<SkeletalBone>),
    >,
    mut bones: Query<
        (&mut Transform, Option<&ChildOf>),
        (With<SkeletalBone>, Without<ColliderBone>, Allow<SkeletonLodDisabled>),
    >,
    global_transforms: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
) {
    for (ragdoll, mut char_colliders) in characters.iter_mut() {
        if matches!(ragdoll, CharacterRagdoll::None) {
            continue;
        }

        let partial_bones: Option<&[ColliderBone]> = match ragdoll {
            CharacterRagdoll::Partial(bones) => Some(bones.as_slice()),
            _ => None,
        };

        // Write each collider's position back to its single skeleton bone as a
        // local (parent-relative) transform, lerping for smoothness.
        let mut applied_joint_world = AHashMap::<Entity, Transform>::default();
        for (i_collider, &bone_type) in COLLIDERS.iter().enumerate() {
            if let Some(bones) = partial_bones
                && !bones.contains(&bone_type)
            {
                continue;
            }

            let Some(&collider_entity) = char_colliders.collider_entities.get(&bone_type) else {
                continue;
            };
            let Ok((position, rotation, offset)) = colliders.get(collider_entity) else {
                continue;
            };

            let joint_to_world =
                Transform::from_translation(position.0).with_rotation(rotation.0) * offset.bone_to_collider;

            let Some(&bone_entity) = char_colliders.bone_entities.get(&bone_type) else {
                continue;
            };

            // Convert joint_to_world (world space) to local using the single
            // skeleton's parent chain.
            let local = {
                let (_, parent) = bones.get(bone_entity).expect("bone entity should be valid");
                if let Some(parent) = parent {
                    let parent_entity = parent.parent();
                    if let Some(parent_to_world) = applied_joint_world.get(&parent_entity) {
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

            char_colliders.bone_transforms[i_collider] = local;
            applied_joint_world.insert(bone_entity, joint_to_world);

            // Write directly to the single skeleton's bone, lerping for smoothness.
            if let Ok((mut transform, _)) = bones.get_mut(bone_entity) {
                let dp = (local.translation - transform.translation).length();
                let dr = local.rotation.angle_between(transform.rotation);
                if dp > 0.0001 || dr > 0.0001 {
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
    i_collider: usize,
    inv_bindposes: &AHashMap<&str, Transform>,
    scale: f32,
) -> (Collider, Transform) {
    match collider {
        ColliderBone::Head => get_head_collider(helpers, scale),
        ColliderBone::Chest | ColliderBone::Pelvis => {
            let bone_name = DEFAULT_RIG_COLLIDER_BONE_NAMES[i_collider];
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

fn get_head_collider(helpers: &[Vec3], scale: f32) -> (Collider, Transform) {
    let center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let radius = (helpers[HEAD_VERTICES[0]] - center).length() * scale;
    (
        Collider::sphere(radius),
        Transform::from_translation(center),
    )
}

fn get_midsection_collider(
    helpers: &[Vec3],
    joint: ColliderBone,
    bind_rot: Quat,
    scale: f32,
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

    // Chest: explicit 90° flip about local X, swapping forward and up vs the
    // inherited bind frame. The y/z extents swap with the axes so the box
    // keeps its measured shape.
    if joint == ColliderBone::Chest {
        (
            Collider::cuboid(
                (xmax - xmin) * scale,
                (zmax - zmin) * scale,
                (ymax - ymin) * scale,
            ),
            Transform::from_translation(center).with_rotation(
                bind_rot * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            ),
        )
    } else {
        (
            Collider::cuboid(
                (xmax - xmin) * scale,
                (ymax - ymin) * scale,
                (zmax - zmin) * scale,
            ),
            Transform::from_translation(center).with_rotation(bind_rot),
        )
    }
}

fn get_limb_collider(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (Collider, Transform) {
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
    let r = (verts[0] - verts[1]).length() * 0.5 * scale;
    let c = (p1 + p2) * 0.5;
    let dir = (p1 - p2).normalize();
    let up = dir.cross(Vec3::NEG_Z).normalize();
    let fwd = dir.cross(up);

    (
        Collider::capsule(r, (p1 - p2).length() * scale),
        Transform::from_translation(c)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(fwd, dir, up))),
    )
}

fn get_extremity_collider(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (Collider, Transform) {
    let ref_verts = match joint {
        ColliderBone::LeftHand => LEFT_HAND_VERTICES,
        ColliderBone::RightHand => RIGHT_HAND_VERTICES,
        ColliderBone::LeftFoot => LEFT_FOOT_VERTICES,
        ColliderBone::RightFoot => RIGHT_FOOT_VERTICES,
        _ => unreachable!(),
    };
    let verts: Vec<Vec3> = ref_verts.iter().map(|&i| helpers[i]).collect();

    let x = (verts[0] - verts[1]).length() * scale;
    let y = (verts[2] - verts[3]).length() * scale;
    let z = (verts[4] - verts[5]).length() * scale;

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
    limit: RagdollJointLimit,
    joint_damping: JointDamping,
    knee_damping: JointDamping,
    character: Entity,
    container_entity: Option<Entity>,
) -> Entity {
    let joint_name: &'static str = NAME_INTERNER
        .intern(&format!(
            "Joint {}",
            DEFAULT_RIG_COLLIDER_BONE_NAMES[collider_index(bone)]
        ))
        .leak();
    let joint_entity = match bone {
        ColliderBone::LowerRightArm | ColliderBone::LowerLeftArm => commands
            .spawn((
                Name::new(joint_name),
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_local_basis2(local_basis2)
                    .with_angle_limits(limit.angle_min, limit.angle_max)
                    .with_point_compliance(0.0)
                    .with_align_compliance(0.0),
                JointCollisionDisabled,
                joint_damping,
                JointForCharacter(character),
            ))
            .id(),
        ColliderBone::LowerRightLeg | ColliderBone::LowerLeftLeg => commands
            .spawn((
                Name::new(joint_name),
                RevoluteJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_local_basis2(local_basis2)
                    .with_angle_limits(limit.angle_min, limit.angle_max)
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
        | ColliderBone::RightFoot => commands
            .spawn((
                Name::new(joint_name),
                SphericalJoint::new(parent, child)
                    .with_anchor(anchor)
                    .with_local_basis2(local_basis2)
                    .with_swing_limits(-limit.swing, limit.swing)
                    .with_twist_limits(-limit.twist, limit.twist)
                    .with_point_compliance(0.0)
                    .with_swing_compliance(0.0)
                    .with_twist_compliance(0.0),
                JointCollisionDisabled,
                joint_damping,
                JointForCharacter(character),
            ))
            .id(),
    };
    if let Some(container_entity) = container_entity {
        commands.entity(container_entity).add_child(joint_entity);
    }
    joint_entity
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

/// Observer: when a character's `HelperVertexPositions` are removed, tear down
/// all humentity-owned physics state so only a bare state blob remains. This pairs
/// with the skeleton/mesh cleanup in `spawn_skeleton::on_character_helpers_removed`.
///
/// Despawns every avian collider and joint (tracked via the bevy_relationships
/// lists) and removes `CharacterColliders` and `NeedsColliders` from the character
/// root. User-config components (`CharacterRagdoll`, `RagdollDensity`,
/// `RagdollDamping`, `RagdollMobility`, `RagdollCollisionLayers`) are left intact so
/// the character can be re-activated later.
pub(crate) fn on_character_helpers_removed(
    trigger: On<Remove, HelperVertexPositions>,
    characters: Query<(), With<CharacterShape>>,
    collider_lists: Query<&ColliderList>,
    joint_lists: Query<&JointList>,
    mut commands: Commands,
) {
    let entity = trigger.entity;
    if characters.get(entity).is_err() {
        return;
    }

    if let Ok(list) = collider_lists.get(entity) {
        for &collider_entity in list.0.iter() {
            commands.entity(collider_entity).despawn();
        }
    }
    if let Ok(list) = joint_lists.get(entity) {
        for &joint_entity in list.0.iter() {
            commands.entity(joint_entity).despawn();
        }
    }

    commands
        .entity(entity)
        .remove::<CharacterColliders>()
        .remove::<NeedsColliders>();
}

/// Re-applies [`RagdollJointLimitOverrides`] to live joints whenever the
/// overrides component changes on a character. Lets you tune joint limits
/// interactively while the ragdoll is active without respawning the joints.
pub(crate) fn apply_joint_limit_overrides(
    characters: Query<
        (
            Option<&RagdollJointLimitOverrides>,
            &CharacterColliders,
            Option<&RagdollMobility>,
        ),
        Or<(
            Changed<RagdollJointLimitOverrides>,
            Changed<RagdollMobility>,
        )>,
    >,
    mut revolute: Query<&mut RevoluteJoint>,
    mut spherical: Query<&mut SphericalJoint>,
) {
    for (overrides, colliders, mobility) in &characters {
        let m = mobility.map_or(1.0, |mob| mob.0);
        for (&joint_entity, &bone) in &colliders.joint_bone {
            // Absent overrides fall back to the mobility-scaled default so the
            // live joint always matches what a respawn would produce.
            let limit = resolve_joint_limit(bone, overrides, m);
            if let Ok(mut joint) = revolute.get_mut(joint_entity) {
                joint.angle_limit = Some(AngleLimit::new(limit.angle_min, limit.angle_max));
            } else if let Ok(mut joint) = spherical.get_mut(joint_entity) {
                joint.swing_limit = Some(AngleLimit::new(-limit.swing, limit.swing));
                joint.twist_limit = Some(AngleLimit::new(-limit.twist, limit.twist));
            }
        }
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
        ColliderBone::Chest | ColliderBone::Pelvis => (0.5, 0.3),
        ColliderBone::Head => (0.5, 0.3),
        ColliderBone::UpperRightArm | ColliderBone::UpperLeftArm => (1.5, 0.5),
        ColliderBone::UpperRightLeg | ColliderBone::UpperLeftLeg => (1.5, 0.3),
        ColliderBone::LeftHand | ColliderBone::RightHand => (0.3, 0.3),
        ColliderBone::LeftFoot | ColliderBone::RightFoot => (0.2, 0.1),
        _ => unreachable!(),
    }
}
