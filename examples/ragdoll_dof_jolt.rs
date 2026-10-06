//! Lets you inspect one degree of freedom of the jolt ragdoll at a time.
//!
//! Same flow as `ragdoll_dof` (avian): the character floats in its bind pose
//! (no floor, no floor collision). With a single key press you cycle to the
//! next joint degree of freedom. Only that one DOF is made physical: the rest
//! of the body stays in the bind pose (kinematic), and the single joint's limb
//! is driven through its limit back and forth so you can judge whether the
//! joint settings are appropriate.
//!
//! Joints come in two families:
//!   - **Swing-twist joints** (shoulders, hips, spine, neck, wrists, ankles):
//!     3 rotational DOF — `swing` (a cone, 2 DOF) and `twist` about the bone axis (1 DOF).
//!     Each is exposed as two separate DOFs: `Swing` and `Twist`.
//!   - **Hinge joints** (elbows, knees): 1 rotational DOF — a hinge `angle`.
//!
//! Controls:
//!   - `[` / `]`   cycle to the previous / next degree of freedom
//!   - `,` / `.`   jump to the previous / next *joint* (its first DOF)
//!   - `G`         toggle gravity
//!
//! The on-screen overlay shows the active DOF, its current limit, and the controls.
//!
//! Jolt drive notes (vs the avian version):
//! - Spherical DOFs write `JoltAngularVelocity` (world-frame prescribed sweep,
//!   same as avian writes `AngularVelocity`); change detection pushes it before
//!   the step, and the sync writes the measured result back.
mod shared;

use ahash::AHashMap;
use bevy::prelude::*;
use bevy_jolt::{
    CollisionLayers, JointKind, JoltAngularVelocity, JoltJoint, JoltLinearVelocity,
    JoltMotorDrive, JoltPlugin, JoltTeleport,
};
use humentity::prelude::*;
use shared::setup_app;
use std::f32::consts::FRAC_PI_2;

const RAGDOLL_TEAM: u16 = 3;
const WORLD_TEAM: u16 = 0;
const CHARACTER_TEAM: u16 = 1;

/// Angular speed (rad/s) of the oscillation that sweeps each DOF through its limit.
const OMEGA: f32 = 2.0;
/// Which axis of a joint is currently being exercised.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Axis {
    Swing,
    Twist,
    Hinge,
}

/// A single selectable degree of freedom. `bone` is the *child* bone of the joint.
struct Dof {
    bone: ColliderBone,
    axis: Axis,
}

impl Dof {
    fn label(&self) -> String {
        let axis = match self.axis {
            Axis::Swing => "SWING",
            Axis::Twist => "TWIST",
            Axis::Hinge => "HINGE",
        };
        format!("{:?} — {}", self.bone, axis)
    }

    fn limit_text(&self, mobility: f32) -> String {
        let limit = resolve_joint_limit(self.bone, None, mobility);
        match self.axis {
            Axis::Twist => format!("twist ±{:.0}°", limit.twist.to_degrees()),
            Axis::Swing => format!("swing ±{:.0}°", limit.swing.to_degrees()),
            Axis::Hinge => format!(
                "angle {:.0}° .. {:.0}°",
                limit.angle_min.to_degrees(),
                limit.angle_max.to_degrees()
            ),
        }
    }
}

/// Every selectable DOF, in display order.
fn dofs() -> Vec<Dof> {
    let mut dof_list = Vec::new();
    // Swing-twist joints expose swing and twist separately.
    for &bone in &[
        ColliderBone::Head,
        ColliderBone::Chest,
        ColliderBone::UpperRightArm,
        ColliderBone::UpperLeftArm,
        ColliderBone::UpperRightLeg,
        ColliderBone::UpperLeftLeg,
        ColliderBone::LeftHand,
        ColliderBone::RightHand,
        ColliderBone::LeftFoot,
        ColliderBone::RightFoot,
    ] {
        dof_list.push(Dof {
            bone,
            axis: Axis::Swing,
        });
        dof_list.push(Dof {
            bone,
            axis: Axis::Twist,
        });
    }
    // Hinge joints (elbows, knees) have a single hinge DOF.
    for &bone in &[
        ColliderBone::LowerRightArm,
        ColliderBone::LowerLeftArm,
        ColliderBone::LowerRightLeg,
        ColliderBone::LowerLeftLeg,
    ] {
        dof_list.push(Dof {
            bone,
            axis: Axis::Hinge,
        });
    }
    dof_list
}

/// `bone` plus every collider bone descended from it (inclusive). This is the set of
/// bones that become dynamic: the active joint is the top of the chain, and all
/// descendants are held rigid by locking their joints, so the whole limb moves as a
/// unit about the single active DOF.
fn subchain(bone: ColliderBone) -> Vec<ColliderBone> {
    let mut children: AHashMap<ColliderBone, Vec<ColliderBone>> = default();
    for &child in COLLIDERS.iter() {
        if let Some(parent) = get_collider_parent(child) {
            children.entry(parent).or_default().push(child);
        }
    }
    let mut out = vec![bone];
    let mut stack = vec![bone];
    while let Some(top) = stack.pop() {
        if let Some(kids) = children.get(&top) {
            for kid in kids {
                out.push(*kid);
                stack.push(*kid);
            }
        }
    }
    out
}

/// Tracks which DOF is active and the oscillation phase.
#[derive(Resource, Default)]
struct ActiveDof {
    index: usize,
    phase: f32,
    applied: bool,
}

/// Captured local bind-pose transforms of every skeleton bone, taken once the
/// skeleton is fitted. Used to snap non-active bones back to the bind pose.
#[derive(Resource, Default)]
struct BindPose(AHashMap<Entity, Transform>);

/// Marker for the on-screen status text.
#[derive(Component)]
struct DofText;

fn main() {
    let mut app = setup_app();

    let collision_layers = CollisionLayers::new(4);

    app.add_plugins(JoltPlugin::new().with_collision_layers(collision_layers))
        .insert_resource(JoltGravity(Vec3::ZERO))
        .insert_resource(ActiveDof::default())
        .insert_resource(BindPose::default())
        .add_systems(Startup, spawn_status_text)
        .add_systems(Update, add_human.run_if(resource_exists::<HumentityAssetsReady>))
        .add_systems(
            Update,
            (
                apply_startup_gravity,
                ground_character,
                capture_bind_pose,
                cycle_dof,
                apply_active_dof.before(HumentityRagdollSystemSet),
                reset_inactive_bones,
                drive_dof,
                gravity_toggle,
            )
                .chain(),
        )
        .run();
}

/// Startup gravity for the Jolt world: zero-G so the floating bind pose holds
/// until the user toggles gravity. Applied once the world resource exists.
#[derive(Resource, Clone, Copy)]
struct JoltGravity(Vec3);

fn apply_startup_gravity(
    gravity: Res<JoltGravity>,
    world: Option<ResMut<bevy_jolt::JoltPhysicsWorld>>,
) {
    let Some(mut world) = world else {
        return;
    };
    world.set_world_gravity(gravity.0);
}

/// Offsets the character by its bind-pose lowest vertex height so it sits at a
/// fixed, known height (y = 0 at the feet). Blends the morphed helpers on
/// demand from the template's baked deltas once they land. Idempotent: it
/// just re-asserts the (constant) transform once the vertices are known.
fn ground_character(
    mut commands: Commands,
    characters: Query<(Entity, &CharacterShape)>,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    basemesh: Res<BaseMesh>,
) {
    for (entity, character_shape) in &characters {
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
        if blended_helpers.is_empty() {
            continue;
        }
        let min_y = blended_helpers.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
        // +0.05 clearance above y = 0.
        let height = -min_y + 0.05;
        commands
            .entity(entity)
            .insert(Transform::from_xyz(0.0, height, 0.0));
    }
}

/// Snapshots every skeleton bone's local transform once the skeleton is fitted, so
/// inactive bones can be snapped back to the bind pose.
fn capture_bind_pose(
    characters: Query<&CharacterSkeleton, Added<SkeletonReady>>,
    bones: Query<&Transform>,
    mut bind_pose: ResMut<BindPose>,
) {
    for skeleton in &characters {
        for &bone_entity in skeleton.bone_map.values() {
            if let Ok(bone_transform) = bones.get(bone_entity) {
                bind_pose.0.insert(bone_entity, *bone_transform);
            }
        }
    }
}

/// Each frame, writes the bind pose back to every skeleton bone that is not part of
/// the currently active DOF's dynamic chain, so the rest of the body stays in the
/// bind pose while only the active joint moves.
fn reset_inactive_bones(
    active: Res<ActiveDof>,
    bind_pose: Res<BindPose>,
    character: Query<(&CharacterSkeleton, &CharacterColliders)>,
    mut bones: Query<&mut Transform, With<SkeletalBone>>,
) {
    if !active.applied || bind_pose.0.is_empty() {
        return;
    }
    let Ok((skeleton, colliders)) = character.single() else {
        return;
    };

    // Entities whose transforms the ragdoll drives this frame.
    let mut active_entities = std::collections::HashSet::new();
    let chain = subchain(dofs()[active.index].bone);
    for bone in chain {
        if let Some(&bone_entity) = colliders.bone_entities.get(&bone) {
            active_entities.insert(bone_entity);
        }
    }

    for &bone_entity in skeleton.bone_map.values() {
        if active_entities.contains(&bone_entity) {
            continue;
        }
        if let (Ok(mut bone_transform), Some(bind)) = (
            bones.get_mut(bone_entity),
            bind_pose.0.get(&bone_entity),
        ) {
            *bone_transform = *bind;
        }
    }
}

fn cycle_dof(input: Res<ButtonInput<KeyCode>>, mut active: ResMut<ActiveDof>) {
    let list = dofs();
    let dof_count = list.len();
    let mut pressed: Option<&str> = None;
    if input.just_pressed(KeyCode::BracketRight) {
        active.index = (active.index + 1) % dof_count;
        active.applied = false;
        pressed = Some("]");
    }
    if input.just_pressed(KeyCode::BracketLeft) {
        active.index = (active.index + dof_count - 1) % dof_count;
        active.applied = false;
        pressed = Some("[");
    }
    // Jump to the next joint (the first DOF of the next joint in the list).
    if input.just_pressed(KeyCode::Period) {
        let mut next = (active.index + 1) % dof_count;
        while next != active.index && list[next].bone == list[active.index].bone {
            next = (next + 1) % dof_count;
        }
        active.index = next;
        active.applied = false;
        pressed = Some(".");
    }
    if input.just_pressed(KeyCode::Comma) {
        let mut prev = (active.index + dof_count - 1) % dof_count;
        while prev != active.index && list[prev].bone == list[active.index].bone {
            prev = (prev + dof_count - 1) % dof_count;
        }
        active.index = prev;
        active.applied = false;
        pressed = Some(",");
    }
    if pressed.is_some() {
        // Start each DOF at the sweep end nearest bind (zero initial drive):
        // knees rest at max (straight), elbows at min, sphericals at center.
        let dof = &dofs()[active.index];
        active.phase = match dof.axis {
            Axis::Hinge => {
                let limit = default_joint_limit(dof.bone);
                if limit.angle_min.abs() < limit.angle_max.abs() {
                    -FRAC_PI_2
                } else {
                    FRAC_PI_2
                }
            }
            Axis::Swing | Axis::Twist => FRAC_PI_2,
        };
    }
}

/// Reconfigures the ragdoll so only the active joint is free. Runs whenever the
/// selection changes (or once colliders have spawned).
fn apply_active_dof(
    mut active: ResMut<ActiveDof>,
    bind_pose: Res<BindPose>,
    mut character: Query<(
        &mut CharacterRagdoll,
        &mut RagdollJointLimitOverrides,
        &CharacterSkeleton,
        &CharacterColliders,
    )>,
    mut transforms: Query<&mut Transform>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    collider_offsets: Query<&ColliderOffset, With<ColliderBone>>,
    body_ids: Query<&bevy_jolt::JoltBodyId>,
    mut collider_linears: Query<&mut JoltLinearVelocity, With<ColliderBone>>,
    mut collider_angulars: Query<&mut JoltAngularVelocity, With<ColliderBone>>,
    mut commands: Commands,
) {
    // Snapshot the whole skeleton and its collider bodies back to the bind pose so
    // the newly-selected DOF always starts from a clean rest pose. This covers bones
    // shared with the previous selection (which `reset_inactive_bones` leaves alone)
    // and re-seats the dynamic bodies so `sync_bones_to_ragdoll` can't drag the
    // displaced pose back into the bones. Scoped so the immutable query borrow is
    // released before we take the mutable `CharacterRagdoll` below.
    {
        let Ok((_, _, skeleton, colliders)) = character.single() else {
            return;
        };

        // Wait until colliders exist; otherwise the joints would never spawn.
        if colliders.collider_entities.is_empty() {
            return;
        }

        if active.applied {
            return;
        }

        if !bind_pose.0.is_empty() {
            // Seat every collider entity at its bind-pose *world* transform,
            // rebuilt by composing the captured bind locals top-down under the
            // skeleton entity's global. Bind locals are displacement-proof,
            // unlike live GlobalTransforms, so the previously driven DOF can't
            // pollute the reset. Velocity is zeroed so a fast-swinging limb
            // doesn't fling on reset.
            let skeleton_global = globals
                .get(skeleton.skeleton_entity)
                .copied()
                .unwrap_or(GlobalTransform::IDENTITY);
            for (&bone, &collider_entity) in &colliders.collider_entities {
                let Some(&bone_entity) = colliders.bone_entities.get(&bone) else {
                    continue;
                };
                // Collect the bone-to-skeleton entity chain; refuse partial
                // chains so one missing bind entry can't seat a body halfway.
                let mut lineage = vec![bone_entity];
                let rooted = loop {
                    let Some(&top) = lineage.last() else {
                        break false;
                    };
                    let Ok(child_of) = parents.get(top) else {
                        break false;
                    };
                    if child_of.parent() == skeleton.skeleton_entity {
                        break true;
                    }
                    lineage.push(child_of.parent());
                };
                if !rooted {
                    continue;
                }
                let mut world = Transform::from(skeleton_global);
                let mut complete = true;
                for &entity in lineage.iter().rev() {
                    if let Some(bind) = bind_pose.0.get(&entity) {
                        world = world * *bind;
                    } else if let Ok(local) = transforms.get(entity) {
                        world = world * *local;
                    } else {
                        complete = false;
                        break;
                    }
                }
                if !complete {
                    continue;
                }
                let Ok(offset) = collider_offsets.get(collider_entity) else {
                    continue;
                };
                let body_world = world * offset.collider_to_bone;
                if let Ok(mut collider_transform) = transforms.get_mut(collider_entity) {
                    collider_transform.translation = body_world.translation;
                    collider_transform.rotation = body_world.rotation;
                }
                // Teleport the Jolt body (when baked) into the bind pose.
                if body_ids.get(collider_entity).is_ok() {
                    commands.trigger(JoltTeleport {
                        body_entity: collider_entity,
                        target_position: body_world.translation,
                        target_rotation: body_world.rotation,
                    });
                }
                if let Ok(mut linear) = collider_linears.get_mut(collider_entity) {
                    linear.linear_velocity = Vec3::ZERO;
                }
                if let Ok(mut angular) = collider_angulars.get_mut(collider_entity) {
                    angular.angular_velocity = Vec3::ZERO;
                }
            }
            // Then snap every bone's local transform to the bind pose so the visible
            // mesh (and inactive bones, which `sync_bones_to_ragdoll` skips) are clean.
            for &bone_entity in skeleton.bone_map.values() {
                if let (Ok(mut bone_transform), Some(bind)) =
                    (transforms.get_mut(bone_entity), bind_pose.0.get(&bone_entity))
                {
                    *bone_transform = *bind;
                }
            }
        }
    }

    let Ok((mut ragdoll, mut overrides, _skeleton, _colliders)) = character.single_mut() else {
        return;
    };

    let list = dofs();
    let dof = &list[active.index];

    // 1. Make the active joint's limb (and its descendants) dynamic; everything else
    //    stays kinematic in the bind pose.
    let chain = subchain(dof.bone);

    // 2. Build limits: the active DOF is free, every other axis (and every
    //    descendant joint) is locked to zero so the limb holds its bind pose.
    let mut next = RagdollJointLimitOverrides::default();
    let def = default_joint_limit(dof.bone);
    match dof.axis {
        Axis::Hinge => {
            next.revolute
                .insert(dof.bone, (def.angle_min, def.angle_max));
        }
        Axis::Twist => {
            next.spherical.insert(dof.bone, (0.0, def.twist));
        }
        Axis::Swing => {
            next.spherical.insert(dof.bone, (def.swing, 0.0));
        }
    }
    for &child in &chain[1..] {
        if matches!(
            child,
            ColliderBone::LowerRightArm
                | ColliderBone::LowerLeftArm
                | ColliderBone::LowerRightLeg
                | ColliderBone::LowerLeftLeg
        ) {
            next.revolute.insert(child, (0.0, 0.0));
        } else {
            next.spherical.insert(child, (0.0, 0.0));
        }
    }
    *overrides = next;
    *ragdoll = CharacterRagdoll::Partial(chain);

    active.applied = true;
}

/// Drives the active DOF through its limit each frame.
///
/// Swing/twist DOFs sweep at a prescribed angular velocity about the live joint
/// frame (the joint's twist axis for twist, an orthogonal swing axis for
/// swing). Open-loop torque is banned here: constant torque spins
/// feather-weight extremities up until the sim explodes. Prescribing the sweep
/// velocity bounds the motion by construction, and the amplitude follows the
/// resolved limit so each DOF parks at its bite. Hinge DOFs attach a
/// `JoltMotorDrive` velocity target swept between the resolved limits.
fn drive_dof(
    time: Res<Time>,
    mut active: ResMut<ActiveDof>,
    character: Query<(&CharacterColliders, Option<&RagdollMobility>)>,
    joint_query: Query<(Entity, &JoltJoint)>,
    mut bodies: Query<(&Transform, &mut JoltAngularVelocity), With<ColliderBone>>,
    mut commands: Commands,
    mut text: Query<&mut Text, With<DofText>>,
) {
    let Ok((colliders, mobility)) = character.single() else {
        return;
    };
    if !active.applied || colliders.collider_entities.is_empty() {
        return;
    }

    let list = dofs();
    let dof = &list[active.index];
    let Some(&active_collider) = colliders.collider_entities.get(&dof.bone) else {
        error!(
            "drive_dof: no collider entity for active bone {:?} (colliders spawned: {})",
            dof.bone,
            colliders.collider_entities.len()
        );
        return;
    };

    let mobility_value = mobility.map_or(1.0, |mob| mob.0);

    active.phase += time.delta_secs() * OMEGA;
    let sweep = active.phase.sin();

    match dof.axis {
        Axis::Hinge => {
            let limit = resolve_joint_limit(dof.bone, None, mobility_value);
            let mid = (limit.angle_min + limit.angle_max) * 0.5;
            let half = (limit.angle_max - limit.angle_min) * 0.5;
            // Velocity target sweeping the hinge through its range: positive
            // half the sweep, negative the other half.
            let target = half * OMEGA * active.phase.cos();
            let _ = (mid, sweep);
            for (joint_entity, joint) in joint_query.iter() {
                let is_active = match joint.kind {
                    JointKind::Hinge { .. } => joint.body_b == active_collider,
                    _ => false,
                };
                if !is_active {
                    continue;
                }
                commands
                    .entity(joint_entity)
                    .insert(JoltMotorDrive(target));
            }
        }
        Axis::Twist | Axis::Swing => {
            // Child-local drive axis from the live joint frame, rotated into
            // world: `JoltAngularVelocity` is world-frame, so writing the local
            // axis raw cross-wires twist and swing (e.g. head swing yaws).
            // Right after a selection change the joint may not have respawned
            // yet; skip the frame in that case.
            let mut axis = None;
            for (_, joint) in joint_query.iter() {
                if joint.body_b != active_collider {
                    continue;
                }
                if let JointKind::SwingTwist {
                    twist_axis2,
                    plane_axis2,
                    ..
                } = joint.kind
                {
                    axis = Some(if dof.axis == Axis::Twist {
                        twist_axis2.as_vec3()
                    } else {
                        plane_axis2.as_vec3()
                    });
                }
                break;
            }
            if let Some(axis) = axis.filter(|axis| axis.is_finite()) {
                let limit = resolve_joint_limit(dof.bone, None, mobility_value);
                let range = if dof.axis == Axis::Twist {
                    limit.twist
                } else {
                    limit.swing
                };
                let target = range * OMEGA * active.phase.cos();
                if target.is_finite() {
                    if let Ok((body_transform, mut angular)) = bodies.get_mut(active_collider) {
                        let axis = body_transform.rotation * axis;
                        let spin: Vec3 = angular.angular_velocity;
                        angular.angular_velocity = spin - axis * spin.dot(axis) + axis * target;
                    }
                }
            }
        }
    }

    if let Ok(mut text) = text.single_mut() {
        *text = Text::new(status_string(active.index, mobility_value));
    }
}

fn gravity_toggle(
    input: Res<ButtonInput<KeyCode>>,
    mut gravity: ResMut<JoltGravity>,
    world: Option<ResMut<bevy_jolt::JoltPhysicsWorld>>,
) {
    if input.just_pressed(KeyCode::KeyG) {
        gravity.0 = if gravity.0 == Vec3::ZERO {
            Vec3::NEG_Y * 9.81
        } else {
            Vec3::ZERO
        };
        if let Some(mut world) = world {
            world.set_world_gravity(gravity.0);
        }
    }
}

fn status_string(index: usize, mobility: f32) -> String {
    let list = dofs();
    let dof = &list[index];
    format!(
        "[ / ]: cycle DOF   , / .: jump joint   G: gravity\n\nActive DOF ({} / {}):\n  {}\n  limit: {}",
        index + 1,
        list.len(),
        dof.label(),
        dof.limit_text(mobility),
    )
}

fn spawn_status_text(mut commands: Commands) {
    commands.spawn((
        DofText,
        Text::new(status_string(0, 1.0)),
        TextLayout::justify(Justify::Right),
        TextFont::from_font_size(22.0),
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.),
            right: Val::Px(12.),
            ..default()
        },
    ));
}

fn add_human(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    let template_handle = template_assets.add(CharacterTemplate::new([]));

    let basemesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: MeshBuildLod::Cpu(0),
    });

    // The character is offset to a fixed height once its bind-pose vertices are
    // ready (see `ground_character`); it starts at the origin.
    commands.spawn((
        Transform::IDENTITY,
        CharacterShape(shape_assets.add(template_handle)),
        BuildCpuSkeleton,
        CharacterRagdoll::None,
        CharacterColliders::new(None),
        RagdollJointLimitOverrides::default(),
        RagdollCollisionLayers::new(
            RAGDOLL_TEAM,
            (1 << WORLD_TEAM) | (1 << CHARACTER_TEAM) | (1 << RAGDOLL_TEAM),
        ),
        RagdollMobility(1.0),
        RagdollDamping::default(),
        children![(CharacterPart {
            mesh: basemesh,
            skeleton_lod: 0
        },)],
    ));
}
