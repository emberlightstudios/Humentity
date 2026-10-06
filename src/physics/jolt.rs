//! Minimal Jolt ragdoll backend: measure collider shapes from mesh helpers,
//! spawn one [`JoltRagdoll`](bevy_jolt::JoltRagdoll) spec per character, and
//! sync both directions (bones drive kinematic hitboxes, dynamics drive bones).
//!
//! Three pieces of game-facing API live here: [`CharacterRagdoll`] (flip
//! between hitbox follow and full simulation), [`CharacterColliders`] (bone ↔
//! part mapping), and [`RagdollCollisionLayers`] (Jolt team wiring). Joint
//! limits are fixed anatomical constants in this module. Everything else —
//! mass stabilization, priorities, no-collide, body creation, pose sync —
//! lives in `bevy_jolt`.

use ahash::AHashMap;
use bevy::ecs::intern::Internable;
use bevy::math::{Mat3, Quat, Vec3};
use bevy::prelude::*;
use bevy_jolt::{JoltRagdoll, JoltRagdollParts, RagdollJoint, RagdollShape};

use super::{
    COLLIDERS, ColliderBone, DEFAULT_RIG_COLLIDER_BONE_NAMES, HEAD_VERTICES, LEFT_FOOT_VERTICES,
    LEFT_HAND_VERTICES, LOWER_LEFT_ARM_VERTICES, LOWER_LEFT_LEG_VERTICES, LOWER_RIGHT_ARM_VERTICES,
    LOWER_RIGHT_LEG_VERTICES, PELVIS_VERTICES, RIGHT_FOOT_VERTICES, RIGHT_HAND_VERTICES,
    TORSO_VERTICES, UPPER_LEFT_ARM_VERTICES, UPPER_LEFT_LEG_VERTICES, UPPER_RIGHT_ARM_VERTICES,
    UPPER_RIGHT_LEG_VERTICES, collider_index, get_collider_parent,
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

/// Jolt team wiring: `membership` picks this character's team, the mask picks
/// which teams it collides with (bit `i` = team `i`).
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

/// Whether ragdoll physics is active: `None` rides the animation as kinematic
/// hitboxes, `Full` simulates every part as dynamic.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub enum CharacterRagdoll {
    #[default]
    None,
    Full,
}

/// Bone ↔ baked-part mapping, built at spawn and filled when the spec bakes.
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

/// Which character owns a collider part or ragdoll spec.
#[derive(Component)]
#[relationship(relationship_target = ColliderList)]
pub struct ColliderForCharacter(pub Entity);

/// Auto-maintained list of collider part entities belonging to a character.
#[derive(Component)]
#[relationship_target(relationship = ColliderForCharacter)]
pub struct ColliderList(Vec<Entity>);

#[derive(Component)]
pub(crate) struct NeedsColliders;

/// Which character owns a ragdoll spec entity (set at spawn, read by the
/// bake tagger).
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct SpecOwner(pub(crate) Entity);

/// Seated offsets in spec order, waiting for the bake tagger.
#[derive(Component)]
pub(crate) struct PendingOffsets {
    pub(crate) offsets: Vec<ColliderOffset>,
}

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

const fn joint_limit(bone: ColliderBone) -> JointLimit {
    match bone {
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

fn ragdoll_joint_for(bone: ColliderBone, anchor: Vec3) -> RagdollJoint {
    let limit = joint_limit(bone);
    match bone {
        ColliderBone::LowerRightArm
        | ColliderBone::LowerLeftArm
        | ColliderBone::LowerRightLeg
        | ColliderBone::LowerLeftLeg => RagdollJoint::Hinge {
            anchor,
            min: limit.angle_min,
            max: limit.angle_max,
        },
        _ => RagdollJoint::SwingTwist {
            anchor,
            normal_half_cone: limit.swing,
            plane_half_cone: limit.swing,
            twist_min: -limit.twist,
            twist_max: limit.twist,
        },
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
    let container_entity = container.map(|container| container.0);
    for (
        character_entity,
        character_shape,
        skeleton,
        mut colliders,
        character_to_world,
        character_scale,
        collision_layers,
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
        let object_layer = collision_layers.map_or(0, |layers| layers.membership);

        // Measure shapes + offsets at the bind pose, then build the spec.
        // Bodies bake kinematic (hitbox mode): flips only change motion.
        let mut parts = Vec::with_capacity(COLLIDERS.len());
        let mut part_order = Vec::with_capacity(COLLIDERS.len());
        let mut offsets = Vec::with_capacity(COLLIDERS.len());
        let mut part_index_of: AHashMap<ColliderBone, usize> = AHashMap::default();
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
            // Seat at the bind pose: spawn runs before first animation, so
            // model-space placement is exact with no bone reads.
            let part_world = character_world * collider_to_model;
            // Anchor at the bind-pose joint origin, in the same frame.
            let anchor = (character_world * joint_to_model).translation;
            let parent_part = get_collider_parent(collider)
                .and_then(|parent| part_index_of.get(&parent).copied());
            part_index_of.insert(collider, parts.len());
            part_order.push(collider);
            offsets.push(offset);
            parts.push(bevy_jolt::RagdollPart {
                shape,
                part_position: part_world.translation,
                part_rotation: part_world.rotation,
                parent_part,
                joint: ragdoll_joint_for(collider, anchor),
            });
        }
        let spec_id = commands
            .spawn((
                Name::new("RagdollSpec"),
                JoltRagdoll {
                    parts,
                    object_layer,
                    density_kg_per_m3: 1000.0,
                    motion: bevy_jolt::JoltMotion::Kinematic,
                },
                ColliderForCharacter(character_entity),
                SpecOwner(character_entity),
            ))
            .id();
        if let Some(container_entity) = container_entity {
            commands.entity(container_entity).add_child(spec_id);
        }
        colliders.ragdoll_entity = Some(spec_id);
        colliders.part_order = part_order;
        commands
            .entity(character_entity)
            .insert(PendingOffsets { offsets });
        commands.entity(character_entity).remove::<NeedsColliders>();
        spawned_character_count += 1;
    }
}

/// Flips ragdoll motion without respawning: `Full` simulates (dynamic),
/// anything else rides the bones (kinematic hitboxes).
pub(crate) fn set_ragdoll_state(
    characters: Query<(&CharacterRagdoll, &CharacterColliders), Changed<CharacterRagdoll>>,
    handles: Query<&bevy_jolt::JoltRagdollHandle>,
    ragdoll_parts: Query<&bevy_jolt::JoltRagdollParts>,
    mut commands: Commands,
    mut physics_world: ResMut<bevy_jolt::JoltPhysicsWorld>,
) {
    for (ragdoll, colliders) in characters.iter() {
        let Some(spec_entity) = colliders.ragdoll_entity else {
            continue; // Transient: spec still baking, retry next flip.
        };
        let Ok(handle) = handles.get(spec_entity) else {
            continue; // Transient: bake observer hasn't run yet.
        };
        let world = &mut *physics_world;
        match ragdoll {
            CharacterRagdoll::Full => {
                // Simulate: drop targets so nothing drives the bodies.
                if let Ok(baked) = ragdoll_parts.get(spec_entity) {
                    for &part_entity in &baked.part_entities {
                        commands
                            .entity(part_entity)
                            .remove::<bevy_jolt::JoltKinematicTarget>();
                    }
                }
                world.ragdoll_set_motion(handle.raw(), bevy_jolt::JoltMotion::Dynamic);
            }
            // Kinematic follow (hitbox mode): bodies ride the bones.
            _ => {
                world.ragdoll_set_motion(handle.raw(), bevy_jolt::JoltMotion::Kinematic);
            }
        }
    }
}

/// Drives kinematic ragdoll parts toward their bones every tick (hitbox
/// mode). Jolt derives velocity from the delta, so followers shove dynamics
/// aside. Dynamic parts (ragdoll mode) are skipped. Runs in `FixedUpdate`,
/// before the step.
pub(crate) fn sync_colliders(
    characters: Query<(&CharacterRagdoll, &CharacterColliders)>,
    bones: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    mut parts: Query<(
        &mut Transform,
        &mut bevy_jolt::JoltKinematicTarget,
        &ColliderOffset,
        &BoneForCollider,
        &ColliderForCharacter,
    )>,
) {
    for (ragdoll, colliders) in characters.iter() {
        if matches!(ragdoll, CharacterRagdoll::Full) {
            continue;
        }
        for &part_entity in colliders.collider_entities.values() {
            let Ok((mut part_transform, mut target, offset, bone_link, _)) =
                parts.get_mut(part_entity)
            else {
                continue; // Transient: tagger hasn't run, or no target yet.
            };
            let Ok(joint_to_world) = bones.get(bone_link.0) else {
                continue; // Transient: bone gone before its part despawns.
            };
            let target_world = Transform::from(*joint_to_world) * offset.collider_to_bone;
            part_transform.translation = target_world.translation;
            part_transform.rotation = target_world.rotation;
            if target.target_position != target_world.translation
                || target.target_rotation != target_world.rotation
            {
                target.target_position = target_world.translation;
                target.target_rotation = target_world.rotation;
            }
        }
    }
}

/// Ensures every ragdoll part carries a `JoltKinematicTarget`. Runs after
/// `sync_colliders`; separate query because that system only matches
/// entities that already have the component.
pub(crate) fn ensure_kinematic_targets(
    characters: Query<(&CharacterRagdoll, &CharacterColliders)>,
    tagged: Query<
        (Entity, &Transform),
        (With<ColliderBone>, Without<bevy_jolt::JoltKinematicTarget>),
    >,
    mut commands: Commands,
) {
    for (ragdoll, colliders) in characters.iter() {
        if matches!(ragdoll, CharacterRagdoll::Full) {
            continue;
        }
        for &part_entity in colliders.collider_entities.values() {
            if let Ok((_, part_transform)) = tagged.get(part_entity) {
                commands
                    .entity(part_entity)
                    .insert(bevy_jolt::JoltKinematicTarget {
                        target_position: part_transform.translation,
                        target_rotation: part_transform.rotation,
                    });
            }
        }
    }
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
            commands
                .entity(*part_entity)
                .insert((bone, offset, ColliderForCharacter(owner.0)));
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
/// characters' ragdoll spec entities. Plain identity: no physics.
#[derive(Resource)]
pub struct CharacterPhysicsContainer(pub Entity);

fn collider_shape(
    collider: ColliderBone,
    helpers: &[Vec3],
    joint_to_model: &Transform,
    scale: f32,
) -> (RagdollShape, Transform) {
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

fn head_shape(helpers: &[Vec3], scale: f32) -> (RagdollShape, Transform) {
    let head_center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
    let head_radius = (helpers[HEAD_VERTICES[0]] - head_center).length() * scale;
    (
        RagdollShape::Sphere {
            radius: head_radius,
        },
        Transform::from_translation(head_center),
    )
}

fn midsection_shape(
    helpers: &[Vec3],
    joint: ColliderBone,
    bind_rotation: Quat,
    scale: f32,
) -> (RagdollShape, Transform) {
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
            RagdollShape::Box {
                half_extents: Vec3::new(
                    (xmax - xmin) * scale * 0.5,
                    (zmax - zmin) * scale * 0.5,
                    (ymax - ymin) * scale * 0.5,
                ),
            },
            Transform::from_translation(center)
                .with_rotation(bind_rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
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
            Transform::from_translation(center).with_rotation(bind_rotation),
        )
    }
}

fn limb_shape(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (RagdollShape, Transform) {
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
    let center = (segment_start + segment_end) * 0.5;
    let limb_direction = (segment_start - segment_end).normalize();
    let limb_side = limb_direction.cross(Vec3::NEG_Z).normalize();
    let limb_forward = limb_direction.cross(limb_side);

    (
        RagdollShape::Capsule {
            cylinder_half_height: (segment_start - segment_end).length() * scale * 0.5,
            radius,
        },
        Transform::from_translation(center).with_rotation(Quat::from_mat3(&Mat3::from_cols(
            limb_forward,
            limb_direction,
            limb_side,
        ))),
    )
}

fn extremity_shape(helpers: &[Vec3], joint: ColliderBone, scale: f32) -> (RagdollShape, Transform) {
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
        RagdollShape::Box {
            half_extents: Vec3::new(x * 0.5, y * 0.5, z * 0.5),
        },
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
        .remove::<NeedsColliders>();
}
