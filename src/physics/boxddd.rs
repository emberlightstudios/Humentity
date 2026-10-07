//! Box3D (`bevy_boxddd`) ragdoll backend: measure collider shapes from
//! mesh helpers, spawn one body per character part, and sync both directions
//! (bones drive kinematic hitboxes, dynamics drive bones).
//!
//! Three pieces of game-facing API live here: [`CharacterRagdoll`] (flip
//! between hitbox follow and full simulation), [`CharacterColliders`]
//! (bone ↔ part mapping), and [`RagdollCollisionLayers`] (category/mask
//! wiring). Joint limits are fixed anatomical constants in this module.
//!
//! Bodies and shapes are plain `bevy_boxddd` components (created by its
//! plugin); joints are created directly on the native world because the
//! declarative [`Joint`](bevy_boxddd::Joint) has no anchors or limits. The
//! native joint id lives in [`NativeRagdollJoint`] (deliberately NOT
//! `bevy_boxddd::BoxdddJoint`, which would make the plugin destroy an
//! untracked joint), and teardown destroys it explicitly.

use ahash::AHashMap;
use bevy::ecs::intern::Internable;
use bevy::prelude::*;
use bevy_boxddd::{
    BoxdddPhysicsContext, Collider as BoxdddCollider, PhysicsMaterial, RigidBody as BoxdddRigidBody,
};
use bevy_boxddd::{boxddd::Filter, to_boxddd_quat, to_boxddd_vec3};

use super::{
    COLLIDERS, ColliderBone, DEFAULT_RIG_COLLIDER_BONE_NAMES, HEAD_VERTICES, LEFT_FOOT_VERTICES,
    LEFT_HAND_VERTICES, LOWER_LEFT_ARM_VERTICES, LOWER_LEFT_LEG_VERTICES, LOWER_RIGHT_ARM_VERTICES,
    LOWER_RIGHT_LEG_VERTICES, PELVIS_VERTICES, RIGHT_FOOT_VERTICES, RIGHT_HAND_VERTICES,
    RagdollDamping, RagdollDensity, TORSO_VERTICES, UPPER_LEFT_ARM_VERTICES,
    UPPER_LEFT_LEG_VERTICES, UPPER_RIGHT_ARM_VERTICES, UPPER_RIGHT_LEG_VERTICES, collider_index,
    get_collider_parent,
};
use crate::{
    NAME_INTERNER,
    helpers::{RefitCharacter, TeardownCharacter},
    prelude::{
        CharacterShape, CharacterShapeAsset, CharacterSkeleton, SkeletonLodDisabled, SkeletonReady,
    },
    rigs::{RigData, SkeletalBone},
    spawn_skeleton::CharacterScale,
};

/// Box3D category/mask wiring: `membership` picks this character's category
/// bit, the mask picks which categories it collides with (bit `i` =
/// category `i`).
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

/// Whether ragdoll physics is active: `None` rides the animation as kinematic
/// hitboxes, `Full` simulates every part as dynamic.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub enum CharacterRagdoll {
    #[default]
    None,
    Full,
}

/// Fired when a character's ragdoll state flips (kinematic hitboxes ↔ full
/// simulation). Observers run after the native body types have switched, so
/// impulses applied here land on bodies already in their new state.
#[derive(Event, Debug, Clone)]
pub struct RagdollStateChanged {
    pub character: Entity,
    pub state: CharacterRagdoll,
}

/// Bone ↔ part mapping, built at spawn.
#[derive(Component, Default)]
pub struct CharacterColliders {
    pub bone_entities: AHashMap<ColliderBone, Entity>,
    pub collider_entities: AHashMap<ColliderBone, Entity>,
    pub ragdoll_entity: Option<Entity>,
    pub part_order: Vec<ColliderBone>,
}

impl CharacterColliders {
    pub fn new(_bones_subset: Option<Vec<ColliderBone>>) -> Self {
        Self::default()
    }
}

/// Offset between a collider body and its skeleton joint, both directions.
/// Precomputed once so the per-frame sync never inverts a matrix.
#[derive(Component, Clone, Copy, Debug)]
pub struct ColliderOffset {
    pub collider_to_bone: Transform,
    pub bone_to_collider: Transform,
}

/// Which skeleton bone a collider part follows.
#[derive(Component, Debug, Clone, Copy)]
pub struct BoneForCollider(pub Entity);

/// Which character owns a collider part or ragdoll joint.
#[derive(Component)]
#[relationship(relationship_target = ColliderList)]
pub struct ColliderForCharacter(pub Entity);

/// Auto-maintained list of collider part entities belonging to a character.
#[derive(Component)]
#[relationship_target(relationship = ColliderForCharacter)]
pub struct ColliderList(Vec<Entity>);

#[derive(Component)]
pub(crate) struct NeedsColliders;

/// Anchor bone entities in collider order, waiting for the joint spawner.
/// The spawner reads their live `GlobalTransform`, so joints are created at
/// the current animated pose — not the bind pose the bodies were measured in.
#[derive(Component)]
pub(crate) struct PendingJoints {
    pub(crate) anchor_bones: Vec<Option<Entity>>,
}

/// Marker: this joint entity still needs its native Box3D joint created
/// (both part bodies must exist first).
#[derive(Component)]
pub(crate) struct PendingNativeJoint;

/// Anchor + descriptor for one ragdoll joint, waiting on native bodies.
/// The bone entities give the anatomical (skeleton) frame each side's joint
/// frame is built from — not the body orientation, which includes
/// shape-fitting flips (e.g. the chest's 90° X flip).
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct JointAnchor {
    pub(crate) part_entity: Entity,
    pub(crate) parent_entity: Entity,
    pub(crate) part_bone: Option<Entity>,
    pub(crate) parent_bone: Option<Entity>,
    pub(crate) anchor_bone: Option<Entity>,
    pub(crate) anchor: Vec3,
    pub(crate) descriptor: RagdollJointDescriptor,
}

/// Which native joint family + limits to build for one collider bone.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RagdollJointDescriptor {
    Hinge { min: f32, max: f32 },
    SwingTwist { swing: f32, twist: f32 },
}

/// Native Box3D joint created for a ragdoll joint entity. Deliberately NOT
/// `bevy_boxddd::BoxdddJoint`: the plugin destroys untracked native joints
/// on cleanup, so humentity owns the lifetime and destroys it on teardown.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct NativeRagdollJoint(pub(crate) bevy_boxddd::boxddd::JointId);

/// Fixed anatomical joint limits (radians). Hinges (elbows, knees) use
/// `angle_min`/`angle_max`; everything else is a swing-twist cone (`swing`)
/// plus twist range (`twist`).
#[derive(Clone, Copy, Debug)]
struct JointLimit {
    swing: f32,
    twist: f32,
    angle_min: f32,
    angle_max: f32,
}

const fn joint_limit(collider_bone: ColliderBone) -> JointLimit {
    match collider_bone {
        ColliderBone::LowerRightArm | ColliderBone::LowerLeftArm => JointLimit {
            swing: 0.0,
            twist: 0.0,
            angle_min: -0.17,
            angle_max: 2.5,
        },
        ColliderBone::LowerRightLeg | ColliderBone::LowerLeftLeg => JointLimit {
            swing: 0.0,
            twist: 0.0,
            angle_min: -2.4,
            angle_max: 0.0,
        },
        ColliderBone::Chest | ColliderBone::Pelvis | ColliderBone::Head => JointLimit {
            swing: 0.5,
            twist: 0.3,
            angle_min: 0.0,
            angle_max: 0.0,
        },
        ColliderBone::UpperRightArm | ColliderBone::UpperLeftArm => JointLimit {
            swing: 1.5,
            twist: 0.5,
            angle_min: 0.0,
            angle_max: 0.0,
        },
        ColliderBone::UpperRightLeg | ColliderBone::UpperLeftLeg => JointLimit {
            swing: 1.5,
            twist: 0.3,
            angle_min: 0.0,
            angle_max: 0.0,
        },
        ColliderBone::LeftHand | ColliderBone::RightHand => JointLimit {
            swing: 0.3,
            twist: 0.3,
            angle_min: 0.0,
            angle_max: 0.0,
        },
        ColliderBone::LeftFoot | ColliderBone::RightFoot => JointLimit {
            swing: 0.2,
            twist: 0.1,
            angle_min: 0.0,
            angle_max: 0.0,
        },
    }
}

/// Which skeleton bone's origin anchors a collider's joint to its parent.
/// A bone's origin sits at its start (the joint that moves it), so for limbs
/// the collider's own bone is the pivot; the chest pivots at `spine03`, the
/// head at `neck02`, and the pelvis root has no joint. `None` = no joint.
pub(crate) const fn joint_anchor_bone(collider_bone: ColliderBone) -> Option<&'static str> {
    match collider_bone {
        ColliderBone::Pelvis => None,
        ColliderBone::Chest => Some("spine03"),
        ColliderBone::Head => Some("neck02"),
        ColliderBone::UpperRightArm => Some("upperarm01.R"),
        ColliderBone::UpperLeftArm => Some("upperarm01.L"),
        ColliderBone::LowerRightArm => Some("lowerarm01.R"),
        ColliderBone::LowerLeftArm => Some("lowerarm01.L"),
        ColliderBone::RightHand => Some("wrist.R"),
        ColliderBone::LeftHand => Some("wrist.L"),
        ColliderBone::UpperRightLeg => Some("upperleg01.R"),
        ColliderBone::UpperLeftLeg => Some("upperleg01.L"),
        ColliderBone::LowerRightLeg => Some("lowerleg01.R"),
        ColliderBone::LowerLeftLeg => Some("lowerleg01.L"),
        ColliderBone::RightFoot => Some("foot.R"),
        ColliderBone::LeftFoot => Some("foot.L"),
    }
}

const fn joint_descriptor_for(collider_bone: ColliderBone) -> RagdollJointDescriptor {
    let limit = joint_limit(collider_bone);
    match collider_bone {
        ColliderBone::LowerRightArm
        | ColliderBone::LowerLeftArm
        | ColliderBone::LowerRightLeg
        | ColliderBone::LowerLeftLeg => RagdollJointDescriptor::Hinge {
            min: limit.angle_min,
            max: limit.angle_max,
        },
        _ => RagdollJointDescriptor::SwingTwist {
            swing: limit.swing,
            twist: limit.twist,
        },
    }
}

fn collider_filter(layers: Option<&RagdollCollisionLayers>) -> Filter {
    let (membership, collides_with_mask) = layers.map_or((0, u32::MAX), |ragdoll_layers| {
        (ragdoll_layers.membership, ragdoll_layers.collides_with_mask)
    });
    Filter {
        category_bits: 1u64 << membership.min(63),
        mask_bits: u64::from(collides_with_mask),
        group_index: 0,
    }
}

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
            &GlobalTransform,
            Option<&CharacterScale>,
            Option<&RagdollCollisionLayers>,
            Option<&RagdollDamping>,
            Option<&RagdollDensity>,
        ),
        (With<NeedsColliders>, With<SkeletonReady>),
    >,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<crate::template::CharacterTemplate>>,
    basemesh: Res<crate::basemesh::BaseMesh>,
    rig_data: Res<RigData>,
    container: Option<Res<CharacterPhysicsContainer>>,
) {
    const BATCH_SIZE: usize = 2;
    let mut spawned_character_count = 0;
    // Create the single global container on first use, then reuse it for every
    // character so all ragdoll parts share one world-root parent.
    let container_entity = match container {
        Some(container) => container.0,
        None => {
            let container_spawned = commands
                .spawn((Name::new("CharacterPhysics"), Transform::IDENTITY))
                .id();
            commands.insert_resource(CharacterPhysicsContainer(container_spawned));
            container_spawned
        }
    };
    for (
        character_entity,
        character_shape,
        skeleton,
        mut colliders,
        character_to_world,
        character_scale,
        collision_layers,
        ragdoll_damping,
        ragdoll_density,
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
            || template
                .shapes
                .iter()
                .any(|shape| shape.helper_deltas.is_none())
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

        // Forward joint→model frames, in reference order. `skeleton` stores
        // the inverses, so invert once here.
        let mut joint_to_model_map: AHashMap<&str, Transform> = AHashMap::default();
        for (bone_index, &bone_name) in reference_order.iter().enumerate() {
            joint_to_model_map.insert(
                bone_name,
                Transform::from_matrix(skeleton.model_space_inv_bindposes[bone_index].inverse()),
            );
        }

        // Collider-bone → skeleton bone-entity mapping.
        colliders.bone_entities.clear();
        for &collider in &COLLIDERS {
            let joint_name = DEFAULT_RIG_COLLIDER_BONE_NAMES[collider_index(collider)];
            if let Some(&bone_entity) = skeleton
                .bone_map
                .get(NAME_INTERNER.intern(joint_name).leak())
            {
                colliders.bone_entities.insert(collider, bone_entity);
            }
        }

        // Seat under the skeleton root frame (facing fix + rear shift), the
        // same composition the skeleton build uses.
        let rig_name = rig_spec.reference_rig.rig_name.as_str();
        let model_root = Transform::from_rotation(crate::MODEL_ROTATION_FIX).with_translation(
            Vec3::new(0.0, 0.0, crate::rigs::skeleton_rear_offset_meters(rig_name)),
        );
        let character_world = Transform::from(*character_to_world) * model_root;
        let collider_scale = character_scale.map_or(1.0, |scale| scale.0);
        let shape_filter = collider_filter(collision_layers);
        let body_damping = ragdoll_damping.map_or(RagdollDamping::default().0, |damping| damping.0);
        let shape_density =
            ragdoll_density.map_or(RagdollDensity::default().0, |density| density.0);

        // Measure shapes + offsets at the bind pose: spawn runs before first
        // animation, so model-space placement is exact with no bone reads.
        // Bodies spawn kinematic (hitbox mode): flips only change the body
        // type, never the shape. Each part carries exactly one shape, so the
        // body frame IS the shape frame (shape-local transform is identity).
        let mut part_order = Vec::with_capacity(COLLIDERS.len());
        let mut anchor_bones = Vec::with_capacity(COLLIDERS.len());
        for &collider in &COLLIDERS {
            let collider_slot = collider_index(collider);
            let joint_name = DEFAULT_RIG_COLLIDER_BONE_NAMES[collider_slot];
            let Some(&joint_to_model) = joint_to_model_map.get(joint_name) else {
                panic!("collider bone {joint_name} missing from reference rig");
            };
            let (shape, collider_to_model) =
                collider_shape(collider, helpers, &joint_to_model, collider_scale);
            let collider_to_joint = Transform::from_matrix(
                joint_to_model.to_matrix().inverse() * collider_to_model.to_matrix(),
            );
            let offset = ColliderOffset {
                collider_to_bone: collider_to_joint,
                bone_to_collider: Transform::from_matrix(collider_to_joint.to_matrix().inverse()),
            };
            // Seat at the bind pose.
            let part_world = character_world * collider_to_model;
            // Anchor bone entity (limbs pivot at their own bone start, chest
            // at spine03, head at neck02, pelvis has no joint). The spawner
            // reads its live transform, so joints land on the animated pose.
            let anchor_bone = joint_anchor_bone(collider).and_then(|anchor_name| {
                skeleton
                    .bone_map
                    .get(NAME_INTERNER.intern(anchor_name).leak())
                    .copied()
            });
            part_order.push(collider);
            anchor_bones.push(anchor_bone);
            let part_entity = commands
                .spawn((
                    Name::new(format!("RagdollPart:{collider:?}")),
                    part_world,
                    BoxdddRigidBody::Kinematic,
                    bevy_boxddd::BodySettings {
                        linear_damping: body_damping,
                        angular_damping: body_damping,
                        ..default()
                    },
                    shape,
                    PhysicsMaterial {
                        density: shape_density,
                        filter: shape_filter,
                        ..default()
                    },
                    offset,
                    collider,
                    ColliderForCharacter(character_entity),
                ))
                .id();
            if let Some(&bone_entity) = colliders.bone_entities.get(&collider) {
                commands
                    .entity(part_entity)
                    .insert(BoneForCollider(bone_entity));
            }
            commands.entity(container_entity).add_child(part_entity);
            colliders.collider_entities.insert(collider, part_entity);
        }
        colliders.part_order = part_order;
        commands
            .entity(character_entity)
            .insert(PendingJoints { anchor_bones });
        commands.entity(character_entity).remove::<NeedsColliders>();
        spawned_character_count += 1;
    }
}

/// Spawns one joint entity per non-root collider once the plugin has created
/// every part body: elbows/knees become limited revolute joints about the
/// part-local Z axis, everything else becomes cone+twist limited spherical
/// joints. Native creation happens in [`create_native_joints`].
pub(crate) fn spawn_joints(
    mut commands: Commands,
    mut characters: Query<(Entity, &mut CharacterColliders, &PendingJoints)>,
    bodies: Query<&bevy_boxddd::BoxdddBody>,
    bone_transforms: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    container: Option<Res<CharacterPhysicsContainer>>,
) {
    let Some(container) = container else {
        return;
    };
    for (character_entity, colliders, pending) in characters.iter_mut() {
        // Wait until every part body exists natively: joint creation needs
        // both native body ids, and the plugin creates bodies asynchronously.
        if colliders
            .collider_entities
            .values()
            .any(|part_entity| bodies.get(*part_entity).is_err())
        {
            continue;
        }
        for (part_index, &collider) in colliders.part_order.iter().enumerate() {
            let Some(parent_collider) = get_collider_parent(collider) else {
                continue; // Pelvis root: no joint.
            };
            let (Some(&part_entity), Some(&parent_entity)) = (
                colliders.collider_entities.get(&collider),
                colliders.collider_entities.get(&parent_collider),
            ) else {
                continue; // Transient: part missing, skip this joint.
            };
            let Some((anchor_bone, anchor)) = pending
                .anchor_bones
                .get(part_index)
                .and_then(|anchor_bone| *anchor_bone)
                .and_then(|anchor_entity| {
                    bone_transforms
                        .get(anchor_entity)
                        .ok()
                        .map(|anchor_to_world| (anchor_entity, anchor_to_world.translation()))
                })
            else {
                continue; // No anchor bone (pelvis root) or bone gone.
            };
            let joint_entity = commands
                .spawn((
                    Name::new(format!("RagdollJoint:{collider:?}")),
                    JointAnchor {
                        part_entity,
                        parent_entity,
                        part_bone: colliders.bone_entities.get(&collider).copied(),
                        parent_bone: colliders.bone_entities.get(&parent_collider).copied(),
                        anchor_bone: Some(anchor_bone),
                        anchor,
                        descriptor: joint_descriptor_for(collider),
                    },
                    PendingNativeJoint,
                    ColliderForCharacter(character_entity),
                ))
                .id();
            commands.entity(container.0).add_child(joint_entity);
        }
        commands.entity(character_entity).remove::<PendingJoints>();
    }
}

/// Local joint frame: the shared world anchor expressed in a body frame whose
/// orientation is the part's bind orientation, so the bind pose reads zero.
/// Returns `None` for a non-finite rotation (retry next frame).
fn local_frame_for(
    body_transform: &Transform,
    anchor_world: Vec3,
    reference_rotation: Quat,
) -> Option<bevy_boxddd::boxddd::Transform> {
    let local_translation =
        body_transform.rotation.inverse() * (anchor_world - body_transform.translation);
    let local_rotation = body_transform.rotation.inverse() * reference_rotation;
    Some(bevy_boxddd::boxddd::Transform::new(
        to_boxddd_vec3(local_translation),
        to_boxddd_quat(local_rotation).ok()?,
    ))
}

/// Builds a hinge reference orientation from a limb bone's live world
/// orientation: Y along the limb (the bone's long axis), Z mediolateral
/// (world X projected perpendicular to Y), X completing a right-handed
/// frame. Returns `None` when Y is parallel to world X (degenerate).
fn limb_basis_rotation(bone_world: Quat) -> Option<Quat> {
    let limb_axis = bone_world * Vec3::Y;
    let side_axis = (Vec3::X - limb_axis * limb_axis.x).normalize_or_zero();
    if side_axis == Vec3::ZERO {
        return None;
    }
    let forward_axis = limb_axis.cross(side_axis);
    Some(Quat::from_mat3(&Mat3::from_cols(
        forward_axis,
        limb_axis,
        side_axis,
    )))
}

/// Fixed quarter-turn about X mapping the limb basis onto the spherical
/// convention: former Y (along the limb) lands on Z, former Z lands on −Y.
/// Both Box3D cone and twist are Z-referenced; bevy_jolt references both to
/// body-local Y, so this is the rotation that carries the tuned limits over.
fn limb_quarter_turn() -> Quat {
    Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
}

/// Creates native Box3D joints for entities whose part bodies both exist.
/// Runs after the plugin's body creation (same `Update` slot, ordered after
/// [`spawn_joints`]).
pub(crate) fn create_native_joints(
    mut commands: Commands,
    mut physics_context: NonSendMut<BoxdddPhysicsContext>,
    joints: Query<(Entity, &JointAnchor), With<PendingNativeJoint>>,
    bodies: Query<(&bevy_boxddd::BoxdddBody, &Transform)>,
    bone_transforms: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
) {
    let Some(world) = physics_context.world_mut() else {
        return;
    };
    for (joint_entity, anchor) in joints.iter() {
        let (Ok((part_body, part_transform)), Ok((parent_body, parent_transform))) = (
            bodies.get(anchor.part_entity),
            bodies.get(anchor.parent_entity),
        ) else {
            continue; // Transient: a part body isn't created yet.
        };
        // Both sides share the anchor bone's live orientation as their
        // reference, so the two frames agree and the spawn pose reads zero
        // joint angle. Translations are untouched (anchor per side, already
        // correct). Falls back per side when a bone mapping is missing.
        let shared_reference = anchor
            .anchor_bone
            .and_then(|bone_entity| bone_transforms.get(bone_entity).ok())
            .map(|bone_to_world| bone_to_world.compute_transform().rotation);
        // All joint frames build their reference orientation from the limb
        // basis: Y along the limb (the anchor bone's long axis, verified by
        // probe), Z mediolateral (world X projected ⊥ Y), X third. Hinges
        // revolve about frame Z, so this puts their axis on the anatomical
        // flexion axis. Sphericals measure cone and twist about frame Z;
        // bevy_jolt (which these limits were tuned for) references both to
        // body-local Y, i.e. along the limb — so sphericals rotate the limb
        // basis a quarter-turn about X, putting former Y (limb) onto Z.
        let limb_reference = anchor
            .anchor_bone
            .and_then(|bone_entity| bone_transforms.get(bone_entity).ok())
            .map(|bone_to_world| bone_to_world.compute_transform().rotation)
            .and_then(limb_basis_rotation);
        let joint_reference = match anchor.descriptor {
            RagdollJointDescriptor::Hinge { .. } => limb_reference,
            RagdollJointDescriptor::SwingTwist { .. } => {
                limb_reference.map(|limb| limb * limb_quarter_turn())
            }
        };
        let part_reference = joint_reference
            .or(shared_reference)
            .or(anchor.part_bone.and_then(|bone_entity| {
                bone_transforms
                    .get(bone_entity)
                    .ok()
                    .map(|bone_to_world| bone_to_world.compute_transform().rotation)
            }))
            .unwrap_or(part_transform.rotation);
        let parent_reference = joint_reference
            .or(shared_reference)
            .or(anchor.parent_bone.and_then(|bone_entity| {
                bone_transforms
                    .get(bone_entity)
                    .ok()
                    .map(|bone_to_world| bone_to_world.compute_transform().rotation)
            }))
            .unwrap_or(parent_transform.rotation);
        let (Some(part_local), Some(parent_local)) = (
            local_frame_for(part_transform, anchor.anchor, part_reference),
            local_frame_for(parent_transform, anchor.anchor, parent_reference),
        ) else {
            continue; // Non-finite rotation; retry next frame.
        };
        let native_result = match anchor.descriptor {
            RagdollJointDescriptor::Hinge { min, max } => world.create_revolute_joint(
                bevy_boxddd::boxddd::RevoluteJointDef::new(parent_body.0, part_body.0)
                    .local_frame_a(parent_local)
                    .local_frame_b(part_local)
                    .limit(true, min, max)
                    .collide_connected(false),
            ),
            RagdollJointDescriptor::SwingTwist { swing, twist } => world.create_spherical_joint(
                bevy_boxddd::boxddd::SphericalJointDef::new(parent_body.0, part_body.0)
                    .local_frame_a(parent_local)
                    .local_frame_b(part_local)
                    .cone_limit(true, swing)
                    .twist_limit(true, -twist, twist)
                    .collide_connected(false),
            ),
        };
        match native_result {
            Ok(native_joint) => {
                commands
                    .entity(joint_entity)
                    .remove::<PendingNativeJoint>()
                    .insert(NativeRagdollJoint(native_joint));
            }
            // Invalid def (shouldn't happen with measured frames): drop the
            // joint rather than retrying every frame.
            Err(_) => {
                commands.entity(joint_entity).despawn();
            }
        }
    }
}

/// Flips ragdoll motion without respawning: `Full` simulates (dynamic),
/// anything else rides the bones (kinematic hitboxes). The plugin does not
/// react to `RigidBody` changes after body creation, so the native body type
/// is switched directly; the component insert keeps Bevy-side state (and the
/// plugin's default transform-sync direction) consistent.
pub(crate) fn set_ragdoll_state(
    characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &CharacterColliders,
            Option<&RagdollDamping>,
        ),
        Changed<CharacterRagdoll>,
    >,
    bodies: Query<&bevy_boxddd::BoxdddBody>,
    mut commands: Commands,
    mut physics_context: NonSendMut<BoxdddPhysicsContext>,
) {
    let Some(world) = physics_context.world_mut() else {
        return;
    };
    for (character_entity, ragdoll, colliders, ragdoll_damping) in characters.iter() {
        let (body_component, body_type) = match ragdoll {
            CharacterRagdoll::Full => (
                BoxdddRigidBody::Dynamic,
                bevy_boxddd::boxddd::BodyType::Dynamic,
            ),
            CharacterRagdoll::None => (
                BoxdddRigidBody::Kinematic,
                bevy_boxddd::boxddd::BodyType::Kinematic,
            ),
        };
        let body_damping = ragdoll_damping.map_or(RagdollDamping::default().0, |damping| damping.0);
        for &part_entity in colliders.collider_entities.values() {
            let Ok(body) = bodies.get(part_entity) else {
                continue; // Transient: body not created yet.
            };
            if world.set_body_type(body.0, body_type).is_err() {
                continue;
            }
            let _ = world.set_body_linear_damping(body.0, body_damping);
            let _ = world.set_body_angular_damping(body.0, body_damping);
            commands.entity(part_entity).insert((
                body_component,
                bevy_boxddd::BodySettings {
                    linear_damping: body_damping,
                    angular_damping: body_damping,
                    ..default()
                },
            ));
        }
        commands.trigger(RagdollStateChanged {
            character: character_entity,
            state: ragdoll.clone(),
        });
    }
}

/// Re-applies [`RagdollDamping`] to every part body when the component
/// changes, without requiring a ragdoll flip.
pub(crate) fn apply_damping(
    characters: Query<(&CharacterColliders, &RagdollDamping), Changed<RagdollDamping>>,
    bodies: Query<&bevy_boxddd::BoxdddBody>,
    mut commands: Commands,
    mut physics_context: NonSendMut<BoxdddPhysicsContext>,
) {
    let Some(world) = physics_context.world_mut() else {
        return;
    };
    for (colliders, ragdoll_damping) in characters.iter() {
        for &part_entity in colliders.collider_entities.values() {
            let Ok(body) = bodies.get(part_entity) else {
                continue; // Transient: body not created yet.
            };
            let _ = world.set_body_linear_damping(body.0, ragdoll_damping.0);
            let _ = world.set_body_angular_damping(body.0, ragdoll_damping.0);
            commands
                .entity(part_entity)
                .insert(bevy_boxddd::BodySettings {
                    linear_damping: ragdoll_damping.0,
                    angular_damping: ragdoll_damping.0,
                    ..default()
                });
        }
    }
}

/// Drives kinematic ragdoll parts toward their bones every tick (hitbox
/// mode). The plugin's `BevyToPhysics` sync writes the pose into Box3D before
/// the step, so followers shove dynamics aside. Dynamic parts (ragdoll mode)
/// are skipped.
pub(crate) fn sync_colliders(
    characters: Query<(&CharacterRagdoll, &CharacterColliders)>,
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    mut parts: Query<(&mut Transform, &ColliderOffset, &BoneForCollider)>,
) {
    for (ragdoll, colliders) in characters.iter() {
        if matches!(ragdoll, CharacterRagdoll::Full) {
            continue;
        }
        for &part_entity in colliders.collider_entities.values() {
            let Ok((mut part_transform, offset, bone_link)) = parts.get_mut(part_entity) else {
                continue; // Transient: part not spawned yet.
            };
            let Ok(joint_to_world) = bones.get(bone_link.0) else {
                continue; // Transient: bone gone before its part despawns.
            };
            let target_world = Transform::from(*joint_to_world) * offset.collider_to_bone;
            part_transform.translation = target_world.translation;
            part_transform.rotation = target_world.rotation;
        }
    }
}

pub(crate) fn sync_bones_to_ragdoll(
    mut characters: Query<(&CharacterRagdoll, &mut CharacterColliders)>,
    colliders: Query<(&Transform, &ColliderOffset), (With<ColliderBone>, Without<SkeletalBone>)>,
    mut bones: Query<
        (&mut Transform, Option<&ChildOf>),
        (
            With<SkeletalBone>,
            Without<ColliderBone>,
            Allow<SkeletonLodDisabled>,
        ),
    >,
    global_transforms: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
) {
    for (ragdoll, character_colliders) in characters.iter_mut() {
        if matches!(ragdoll, CharacterRagdoll::None) {
            continue;
        }
        for &bone_type in COLLIDERS.iter() {
            let Some(&collider_entity) = character_colliders.collider_entities.get(&bone_type)
            else {
                continue;
            };
            let Ok((collider_transform, offset)) = colliders.get(collider_entity) else {
                continue;
            };
            let joint_to_world = *collider_transform * offset.bone_to_collider;
            let Some(&bone_entity) = character_colliders.bone_entities.get(&bone_type) else {
                continue;
            };
            let local = {
                let (_, parent) = bones.get(bone_entity).expect("bone entity should be valid");
                if let Some(parent) = parent {
                    let parent_entity = parent.parent();
                    if let Ok(parent_to_world) = global_transforms.get(parent_entity) {
                        Transform::from_matrix(parent_to_world.to_matrix().inverse())
                            * joint_to_world
                    } else {
                        joint_to_world
                    }
                } else {
                    joint_to_world
                }
            };
            if let Ok((mut transform, _)) = bones.get_mut(bone_entity) {
                let position_delta = (local.translation - transform.translation).length();
                let rotation_delta = local.rotation.angle_between(transform.rotation);
                if position_delta > 0.0001 || rotation_delta > 0.0001 {
                    const LERP_FACTOR: f32 = 0.85;
                    transform.translation =
                        transform.translation.lerp(local.translation, LERP_FACTOR);
                    transform.rotation = transform.rotation.slerp(local.rotation, LERP_FACTOR);
                }
            }
        }
    }
}

/// Single global, static (identity, world-root) entity holding all
/// characters' ragdoll part entities. Plain identity: no physics.
#[derive(Resource)]
pub struct CharacterPhysicsContainer(pub Entity);

fn collider_shape(
    collider: ColliderBone,
    helpers: &[Vec3],
    joint_to_model: &Transform,
    scale: f32,
) -> (BoxdddCollider, Transform) {
    match collider {
        ColliderBone::Head => head_shape(helpers, scale),
        ColliderBone::Chest | ColliderBone::Pelvis => {
            midsection_shape(helpers, collider, joint_to_model.rotation.inverse(), scale)
        }
        ColliderBone::UpperRightArm
        | ColliderBone::UpperLeftArm
        | ColliderBone::LowerRightArm
        | ColliderBone::LowerLeftArm
        | ColliderBone::UpperRightLeg
        | ColliderBone::UpperLeftLeg
        | ColliderBone::LowerRightLeg
        | ColliderBone::LowerLeftLeg => limb_shape(helpers, collider, scale),
        ColliderBone::LeftHand
        | ColliderBone::RightHand
        | ColliderBone::LeftFoot
        | ColliderBone::RightFoot => extremity_shape(helpers, collider, scale),
    }
}

fn head_shape(helpers: &[Vec3], scale: f32) -> (BoxdddCollider, Transform) {
    let head_center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let head_radius = (helpers[HEAD_VERTICES[0]] - head_center).length() * scale;
    (
        BoxdddCollider::Sphere {
            radius: head_radius,
            center: Vec3::ZERO,
        },
        Transform::from_translation(head_center),
    )
}

fn midsection_shape(
    helpers: &[Vec3],
    joint: ColliderBone,
    bind_rotation: Quat,
    scale: f32,
) -> (BoxdddCollider, Transform) {
    let ref_verts = match joint {
        ColliderBone::Chest => TORSO_VERTICES,
        ColliderBone::Pelvis => PELVIS_VERTICES,
        _ => unreachable!("midsection shape only fits chest and pelvis"),
    };

    let mut verts = [Vec3::ZERO; 8];
    for (vert_index, mesh_vert) in ref_verts.iter().enumerate() {
        verts[vert_index] = helpers[*mesh_vert];
        verts[vert_index + 4] = Vec3::new(
            -verts[vert_index].x,
            verts[vert_index].y,
            verts[vert_index].z,
        );
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
            BoxdddCollider::cuboid(
                (xmax - xmin) * scale * 0.5,
                (zmax - zmin) * scale * 0.5,
                (ymax - ymin) * scale * 0.5,
            ),
            Transform::from_translation(center)
                .with_rotation(bind_rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        )
    } else {
        (
            BoxdddCollider::cuboid(
                (xmax - xmin) * scale * 0.5,
                (ymax - ymin) * scale * 0.5,
                (zmax - zmin) * scale * 0.5,
            ),
            Transform::from_translation(center).with_rotation(bind_rotation),
        )
    }
}

fn limb_shape(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (BoxdddCollider, Transform) {
    let ref_verts = match joint {
        ColliderBone::LowerLeftArm => LOWER_LEFT_ARM_VERTICES,
        ColliderBone::LowerRightArm => LOWER_RIGHT_ARM_VERTICES,
        ColliderBone::UpperLeftArm => UPPER_LEFT_ARM_VERTICES,
        ColliderBone::UpperRightArm => UPPER_RIGHT_ARM_VERTICES,
        ColliderBone::LowerLeftLeg => LOWER_LEFT_LEG_VERTICES,
        ColliderBone::LowerRightLeg => LOWER_RIGHT_LEG_VERTICES,
        ColliderBone::UpperLeftLeg => UPPER_LEFT_LEG_VERTICES,
        ColliderBone::UpperRightLeg => UPPER_RIGHT_LEG_VERTICES,
        _ => unreachable!("limb shape only fits arms and legs"),
    };
    let verts: Vec<Vec3> = ref_verts.iter().map(|&vert| helpers[vert]).collect();

    let segment_start = (verts[0] + verts[1]) * 0.5;
    let segment_end = (verts[2] + verts[3]) * 0.5;
    let radius = (verts[0] - verts[1]).length() * 0.5 * scale;
    let segment_vector = segment_start - segment_end;
    let center = (segment_start + segment_end) * 0.5;
    let limb_direction = segment_vector.normalize();
    let limb_side = limb_direction.cross(Vec3::NEG_Z).normalize();
    let limb_forward = limb_direction.cross(limb_side);

    // Capsule axis is the frame's Y axis (`limb_direction`), so the local
    // endpoints sit at ±half the segment length on Y.
    let half_length = segment_vector.length() * scale * 0.5;
    (
        BoxdddCollider::Capsule {
            point1: Vec3::new(0.0, -half_length, 0.0),
            point2: Vec3::new(0.0, half_length, 0.0),
            radius,
        },
        Transform::from_translation(center).with_rotation(Quat::from_mat3(&Mat3::from_cols(
            limb_forward,
            limb_direction,
            limb_side,
        ))),
    )
}

fn extremity_shape(
    helpers: &[Vec3],
    joint: ColliderBone,
    scale: f32,
) -> (BoxdddCollider, Transform) {
    let ref_verts = match joint {
        ColliderBone::LeftHand => LEFT_HAND_VERTICES,
        ColliderBone::RightHand => RIGHT_HAND_VERTICES,
        ColliderBone::LeftFoot => LEFT_FOOT_VERTICES,
        ColliderBone::RightFoot => RIGHT_FOOT_VERTICES,
        _ => unreachable!("extremity shape only fits hands and feet"),
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
        BoxdddCollider::cuboid(x * 0.5, y * 0.5, z * 0.5),
        Transform::from_translation(center)
            .with_rotation(Quat::from_mat3(&Mat3::from_cols(x_axis, y_axis, z_axis))),
    )
}

/// Observer: on [`TeardownCharacter`] or [`RefitCharacter`], tear down all
/// humentity-owned physics state so only a bare state blob remains.
pub(crate) fn on_teardown_character(
    trigger: On<TeardownCharacter>,
    characters: Query<(), With<CharacterShape>>,
    mut colliders: Query<&mut CharacterColliders>,
    mut commands: Commands,
    joints: Query<(Entity, Option<&NativeRagdollJoint>, &ColliderForCharacter), With<JointAnchor>>,
    mut physics_context: NonSendMut<BoxdddPhysicsContext>,
) {
    teardown_character_physics(
        trigger.event().0,
        &characters,
        &mut colliders,
        &mut commands,
        &joints,
        &mut physics_context,
    );
}

/// Observer for [`RefitCharacter`]: same physics teardown, so the collider
/// spawn re-runs from current morph weights after the fit rebuilds.
pub(crate) fn on_refit_character(
    trigger: On<RefitCharacter>,
    characters: Query<(), With<CharacterShape>>,
    mut colliders: Query<&mut CharacterColliders>,
    mut commands: Commands,
    joints: Query<(Entity, Option<&NativeRagdollJoint>, &ColliderForCharacter), With<JointAnchor>>,
    mut physics_context: NonSendMut<BoxdddPhysicsContext>,
) {
    teardown_character_physics(
        trigger.event().0,
        &characters,
        &mut colliders,
        &mut commands,
        &joints,
        &mut physics_context,
    );
}

fn teardown_character_physics(
    character_entity: Entity,
    characters: &Query<(), With<CharacterShape>>,
    colliders: &mut Query<&mut CharacterColliders>,
    commands: &mut Commands,
    joints: &Query<(Entity, Option<&NativeRagdollJoint>, &ColliderForCharacter), With<JointAnchor>>,
    physics_context: &mut NonSendMut<BoxdddPhysicsContext>,
) {
    if characters.get(character_entity).is_err() {
        return; // Transient: teardown raced character despawn.
    }
    let Ok(mut character_colliders) = colliders.get_mut(character_entity) else {
        return;
    };
    // Destroy native joints explicitly: they were created directly on the
    // Box3D world (the plugin never tracked them). Joint entities despawn
    // with them, pending ones included.
    for (joint_entity, native, owner) in joints.iter() {
        if owner.0 != character_entity {
            continue;
        }
        if let (Some(native), Some(world)) = (native, physics_context.world_mut()) {
            let _ = world.destroy_joint(native.0, true);
        }
        commands.entity(joint_entity).despawn();
    }
    // Despawn part bodies; the plugin destroys their native bodies on removal.
    for &part_entity in character_colliders.collider_entities.values() {
        commands.entity(part_entity).despawn();
    }
    if let Some(ragdoll_entity) = character_colliders.ragdoll_entity.take() {
        commands.entity(ragdoll_entity).despawn();
    }
    character_colliders.collider_entities.clear();
    character_colliders.part_order.clear();
    commands
        .entity(character_entity)
        .remove::<CharacterColliders>()
        .remove::<NeedsColliders>()
        .remove::<PendingJoints>();
}
