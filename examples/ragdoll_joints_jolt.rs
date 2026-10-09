//! Joint-by-joint constraint tuning bench (Jolt backend).
//!
//! No animation: everything stays kinematic except the single selected joint,
//! which goes dynamic so its real Jolt motor sweeps it against its real
//! limits. `[` / `]` steps through
//! joints, `,` / `.` steps through the selected joint's axes (hinges have
//! one; swing-twist joints expose twist then swing). `M` pauses the sweep.
//!
//! A gizmo sphere marks the driven joint's anchor; a line shows the hinge or
//! twist axis. The overlay names the joint, its axis, its configured limits,
mod shared;

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;
use bevy_jolt::prelude::*;
use bevy_jolt::{JoltRagdollHandle, RagdollJoint};
use humentity::prelude::*;
use shared::setup_app_without_inspector;
use std::f32::consts::PI;

const RAGDOLL_TEAM: u16 = 3;
const WORLD_TEAM: u16 = 0;
const CHARACTER_TEAM: u16 = 1;

/// Motor sweep speed (rad/s).
const DRIVE_SPEED: f32 = 1.2;
/// Sweep period (s): the motor reverses this often.
const SWEEP_PERIOD: f32 = 3.0;

/// One selectable joint axis, in display order: hinges expose one entry,
/// swing-twist joints expose twist then swing.
#[derive(Clone, Copy, Debug)]
struct JointAxis {
    joint_bone: ColliderBone,
    drive_axis: RagdollDriveAxis,
}

impl JointAxis {
    const fn label(self) -> &'static str {
        match self.drive_axis {
            RagdollDriveAxis::Hinge => "hinge",
            RagdollDriveAxis::Twist => "twist",
            RagdollDriveAxis::Swing => "swing",
        }
    }
}

/// Display order: spine, head, then each limb tip-ward.
fn joint_axes() -> Vec<JointAxis> {
    use ColliderBone::*;
    let hinge = |joint_bone| JointAxis {
        joint_bone,
        drive_axis: RagdollDriveAxis::Hinge,
    };
    let swing_twist = |joint_bone| {
        [
            JointAxis {
                joint_bone,
                drive_axis: RagdollDriveAxis::Twist,
            },
            JointAxis {
                joint_bone,
                drive_axis: RagdollDriveAxis::Swing,
            },
        ]
    };
    let mut axes = Vec::new();
    axes.extend(swing_twist(Chest));
    axes.extend(swing_twist(Head));
    for &joint_bone in &[
        UpperRightArm,
        LowerRightArm,
        RightHand,
        UpperLeftArm,
        LowerLeftArm,
        LeftHand,
        UpperRightLeg,
        LowerRightLeg,
        RightFoot,
        UpperLeftLeg,
        LowerLeftLeg,
        LeftFoot,
    ] {
        match joint_bone {
            LowerRightArm | LowerLeftArm | LowerRightLeg | LowerLeftLeg => {
                axes.push(hinge(joint_bone));
            }
            _ => axes.extend(swing_twist(joint_bone)),
        }
    }
    axes
}

/// Which joint axis the bench is driving, plus sweep state.
#[derive(Resource)]
struct ActiveJoint {
    axes: Vec<JointAxis>,
    axis_index: usize,
    sweep_time: f32,
    motor_on: bool,
}

/// Marker for the on-screen status text.
#[derive(Component)]
struct BenchText;

fn main() {
    let mut app = setup_app_without_inspector();
    let collision_layers = CollisionLayers::new(4);
    app.add_plugins((
        JoltPlugin::new().with_collision_layers(collision_layers),
        JoltDebugPlugin,
    ))
    .insert_resource(ActiveJoint {
        axes: joint_axes(),
        axis_index: 0,
        sweep_time: 0.0,
        motor_on: true,
    })
    .add_systems(Startup, spawn_ui)
    .add_systems(FixedUpdate, follow_driven_parent)
    .add_systems(
        Update,
        (
            add_human.run_if(resource_exists::<HumentityAssetsReady>),
            cycle_joint,
            arm_selected_joint,
            drive_joint,
            draw_joint_gizmos,
            bench_camera,
            debug_dump,
            update_overlay,
        ),
    )
    .run();
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
    commands.spawn((
        Transform::from_xyz(0.0, 0.0, 0.0),
        CharacterShape(shape_assets.add(template_handle)),
        BuildCpuSkeleton,
        CharacterRagdoll::Full,
        CharacterColliders::new(None),
        RagdollCollisionLayers::new(
            RAGDOLL_TEAM,
            (1 << WORLD_TEAM) | (1 << CHARACTER_TEAM) | (1 << RAGDOLL_TEAM),
        ),
        children![(CharacterPart {
            mesh: basemesh,
            skeleton_lod: 0
        },)],
    ));
}

/// Only the driven limb goes dynamic; its children stay kinematic but rigidly
/// follow their parent body each tick, so the motor moves a stiff limb
/// instead of dragging a frozen subtree. Everything else stays frozen at
/// Only the selected body goes dynamic so its real joint motor and limits
/// run against a kinematic anchor; descendants stay kinematic and ride their
/// parent rigidly each tick (see `follow_driven_parent`). Everything else
/// stays frozen at bind. Re-parks the old limb on switch.
fn arm_selected_joint(
    active: Res<ActiveJoint>,
    character: Query<(Entity, &CharacterColliders)>,
    baked: Query<&JoltRagdollParts>,
    body_ids: Query<&JoltBodyId>,
    ragdoll_specs: Query<&JoltRagdoll>,
    mut physics_world: ResMut<JoltPhysicsWorld>,
    mut armed_root: Local<Option<usize>>,
    mut commands: Commands,
) {
    if !active.is_changed() && armed_root.is_some() {
        return;
    }
    let Ok((character_entity, colliders)) = character.single() else {
        return;
    };
    let Some(spec_entity) = colliders.ragdoll_entity else {
        return;
    };
    let Ok(parts) = baked.get(spec_entity) else {
        return;
    };
    let selected = active.axes[active.axis_index];
    let Some(selected_part) = colliders
        .part_order
        .iter()
        .position(|&joint_bone| joint_bone == selected.joint_bone)
    else {
        return;
    };
    if *armed_root == Some(selected_part) {
        return;
    }
    if armed_root.take().is_some() {
        for (part_index, _) in colliders.part_order.iter().enumerate() {
            park_part_at_bind(
                &parts,
                &body_ids,
                &ragdoll_specs,
                &mut physics_world,
                spec_entity,
                part_index,
            );
        }
        commands.trigger(ResetToBindPose(character_entity));
    }
    set_part_motion(
        &parts,
        &body_ids,
        &mut physics_world,
        selected_part,
        JoltMotion::Dynamic,
    );
    for part_index in subtree_parts(&colliders.part_order, selected_part) {
        if part_index == selected_part {
            continue;
        }
        let (Some(&part_entity), Ok(spec)) = (
            parts.part_entities.get(part_index),
            ragdoll_specs.get(spec_entity),
        ) else {
            continue;
        };
        let (Ok(body_id), Some(part)) = (
            body_ids.get(part_entity),
            spec.parts.get(part_index),
        ) else {
            continue;
        };
        physics_world.set_body_velocity(body_id.body_id_raw, Vec3::ZERO, Vec3::ZERO);
        physics_world.teleport_body(
            body_id.body_id_raw,
            part.part_position,
            part.part_rotation,
        );
        physics_world.set_body_motion(body_id.body_id_raw, JoltMotion::Kinematic);
        commands.entity(part_entity).insert(JoltKinematicTarget {
            target_position: part.part_position,
            target_rotation: part.part_rotation,
        });
    }
    *armed_root = Some(selected_part);
}

/// Pins each kinematic descendant of the driven limb to its live parent body:
/// child target = parent live pose * baked parent-to-child offset. Runs in
/// `FixedUpdate` so `apply_jolt_kinematic_targets` picks the targets up
/// before the same step. Only the armed subtree matches, so the frozen rest
/// of the body is untouched.
fn follow_driven_parent(
    character: Query<&CharacterColliders>,
    baked: Query<&JoltRagdollParts>,
    ragdoll_specs: Query<&JoltRagdoll>,
    mut targets: Query<(&mut JoltKinematicTarget, &JoltBodyId)>,
    body_poses: Query<&Transform, With<JoltBodyId>>,
    armed_root: Local<Option<usize>>,
) {
    let Some(armed_part) = *armed_root else {
        return;
    };
    let Ok(colliders) = character.single() else {
        return;
    };
    let Some(spec_entity) = colliders.ragdoll_entity else {
        return;
    };
    let (Ok(parts), Ok(spec)) = (baked.get(spec_entity), ragdoll_specs.get(spec_entity))
    else {
        return;
    };
    for part_index in subtree_parts(&colliders.part_order, armed_part) {
        // The driven root is owned by its motor, not this system.
        if part_index == armed_part {
            continue;
        }
        let Some(part) = spec.parts.get(part_index) else {
            continue;
        };
        let Some(parent_part) = part.parent_part.map(|parent| parent as usize) else {
            continue;
        };
        let (Some(&part_entity), Some(&parent_entity)) = (
            parts.part_entities.get(part_index),
            parts.part_entities.get(parent_part),
        ) else {
            continue;
        };
        let Ok(parent_pose) = body_poses.get(parent_entity) else {
            continue;
        };
        let (Some(parent_baked), Ok((mut target, _))) = (
            spec.parts.get(parent_part),
            targets.get_mut(part_entity),
        ) else {
            continue;
        };
        // Baked relative offset, re-applied to the live parent pose: the
        // child rides its parent rigidly, no joint solving between them.
        let relative_rotation = parent_baked.part_rotation.conjugate() * part.part_rotation;
        let relative_offset = parent_baked.part_rotation.conjugate()
            * (part.part_position - parent_baked.part_position);
        target.target_position =
            parent_pose.translation + (parent_pose.rotation * relative_offset);
        target.target_rotation = parent_pose.rotation * relative_rotation;
    }
}

/// The selected part plus every part hanging off it (children, recursively).
/// Parent indices always come first in `part_order`, so the pelvis-ward walk
/// below reaches every descendant.
fn subtree_parts(part_order: &[ColliderBone], root_part: usize) -> Vec<usize> {
    let Some(&root_bone) = part_order.get(root_part) else {
        return Vec::new();
    };
    let mut collected = vec![root_part];
    for (part_index, &joint_bone) in part_order.iter().enumerate() {
        if part_index == root_part {
            continue;
        }
        // Walk pelvis-ward: a `None` parent means the pelvis root, which is
        // only a descendant when the driven joint is the pelvis itself.
        let mut cursor = Some(joint_bone);
        while let Some(cursor_bone) = cursor {
            if cursor_bone == root_bone {
                collected.push(part_index);
                break;
            }
            cursor = get_collider_parent(cursor_bone);
        }
    }
    collected.sort_unstable();
    collected
}

/// Flips one ragdoll part between frozen (kinematic) and simulated
/// (dynamic), leaving the rest of the ragdoll untouched.
fn set_part_motion(
    baked: &JoltRagdollParts,
    body_ids: &Query<&JoltBodyId>,
    physics_world: &mut ResMut<JoltPhysicsWorld>,
    part_index: usize,
    motion: JoltMotion,
) {
    let Some(&part_entity) = baked.part_entities.get(part_index) else {
        return;
    };
    let Ok(body_id) = body_ids.get(part_entity) else {
        return;
    };
    physics_world.set_body_motion(body_id.body_id_raw, motion);
}

/// Parks one part back at its baked bind pose: zero its velocity, teleport
/// it home, then freeze it kinematic. Called on the old limb when the
/// selection moves, so a switch never leaves a bent limb behind.
fn park_part_at_bind(
    baked: &JoltRagdollParts,
    body_ids: &Query<&JoltBodyId>,
    ragdoll_specs: &Query<&JoltRagdoll>,
    physics_world: &mut ResMut<JoltPhysicsWorld>,
    spec_entity: Entity,
    part_index: usize,
) {
    let (Some(&part_entity), Ok(spec)) = (
        baked.part_entities.get(part_index),
        ragdoll_specs.get(spec_entity),
    ) else {
        return;
    };
    let (Ok(body_id), Some(part)) = (
        body_ids.get(part_entity),
        spec.parts.get(part_index),
    ) else {
        return;
    };
    physics_world.set_body_velocity(body_id.body_id_raw, Vec3::ZERO, Vec3::ZERO);
    physics_world.teleport_body(
        body_id.body_id_raw,
        part.part_position,
        part.part_rotation,
    );
    physics_world.set_body_motion(body_id.body_id_raw, JoltMotion::Kinematic);
}

/// `[` / `]` steps through joints, `,` / `.` steps through the current
/// joint's axes. Any switch releases the previous joint's motor first, so
/// only ever one joint drives. Runs before `drive_joint`.
fn cycle_joint(
    input: Res<ButtonInput<KeyCode>>,
    mut active: ResMut<ActiveJoint>,
    character: Query<&CharacterColliders>,
    handles: Query<&JoltRagdollHandle>,
    mut physics_world: ResMut<JoltPhysicsWorld>,
) {
    let step = if input.just_pressed(KeyCode::BracketRight) {
        Step::NextJoint
    } else if input.just_pressed(KeyCode::BracketLeft) {
        Step::PrevJoint
    } else if input.just_pressed(KeyCode::Period) {
        Step::NextAxis
    } else if input.just_pressed(KeyCode::Comma) {
        Step::PrevAxis
    } else if input.just_pressed(KeyCode::KeyM) {
        active.motor_on = !active.motor_on;
        active.sweep_time = 0.0;
        return;
    } else {
        return;
    };
    release_joint_motor(&active, &character, &handles, &mut physics_world);
    // Distinct joints in display order: stepping joints lands on each
    // joint's first axis; stepping axes moves within the flat list.
    let mut joints: Vec<ColliderBone> = Vec::new();
    for axis in &active.axes {
        if !joints.contains(&axis.joint_bone) {
            joints.push(axis.joint_bone);
        }
    }
    let current_bone = active.axes[active.axis_index].joint_bone;
    let current_joint = joints
        .iter()
        .position(|&bone| bone == current_bone)
        .unwrap_or(0);
    active.axis_index = match step {
        Step::NextJoint => {
            let next_bone = joints[(current_joint + 1) % joints.len()];
            active
                .axes
                .iter()
                .position(|axis| axis.joint_bone == next_bone)
                .unwrap_or(0)
        }
        Step::PrevJoint => {
            let previous_bone = joints[(current_joint + joints.len() - 1) % joints.len()];
            active
                .axes
                .iter()
                .position(|axis| axis.joint_bone == previous_bone)
                .unwrap_or(0)
        }
        Step::NextAxis => (active.axis_index + 1) % active.axes.len(),
        Step::PrevAxis => active
            .axis_index
            .checked_sub(1)
            .unwrap_or(active.axes.len() - 1),
    };
    active.sweep_time = 0.0;
}

/// Which way the selection moves.
#[derive(Clone, Copy)]
enum Step {
    NextJoint,
    PrevJoint,
    NextAxis,
    PrevAxis,
}

/// Releases the currently driven joint's motor, so a switch never leaves two
/// joints driving at once.
fn release_joint_motor(
    active: &ActiveJoint,
    character: &Query<&CharacterColliders>,
    handles: &Query<&JoltRagdollHandle>,
    physics_world: &mut ResMut<JoltPhysicsWorld>,
) {
    let Ok(colliders) = character.single() else {
        return;
    };
    let Some(spec_entity) = colliders.ragdoll_entity else {
        return;
    };
    let Ok(handle) = handles.get(spec_entity) else {
        return;
    };
    let selected = active.axes[active.axis_index];
    if let Some(part_index) = colliders
        .part_order
        .iter()
        .position(|&bone| bone == selected.joint_bone)
    {
        physics_world.ragdoll_motor_off(handle.id(), part_index as u32);
    }
}

/// Sweeps the selected joint back and forth with its real Jolt velocity
/// motor; releases the motor on `M`. Part index = position of the joint's
/// bone in the character's `part_order` (spec order), which the tagger
/// recorded at bake. Only the selected body is dynamic; its kinematic parent
/// anchors the constraint, so the limits — not the drive — decide where the
/// limb stops.
fn drive_joint(
    time: Res<Time>,
    mut active: ResMut<ActiveJoint>,
    character: Query<&CharacterColliders>,
    handles: Query<&JoltRagdollHandle>,
    mut physics_world: ResMut<JoltPhysicsWorld>,
) {
    let Ok(colliders) = character.single() else {
        return;
    };
    let Some(spec_entity) = colliders.ragdoll_entity else {
        return;
    };
    let Ok(handle) = handles.get(spec_entity) else {
        return;
    };
    let selected = active.axes[active.axis_index];
    let Some(part_index) = colliders
        .part_order
        .iter()
        .position(|&bone| bone == selected.joint_bone)
    else {
        return;
    };
    if !active.motor_on {
        physics_world.ragdoll_motor_off(handle.id(), part_index as u32);
        return;
    }
    active.sweep_time += time.delta_secs();
    let phase = (active.sweep_time / SWEEP_PERIOD * 2.0 * PI).sin();
    let direction = if phase >= 0.0 { 1.0 } else { -1.0 };
    physics_world.ragdoll_drive(
        handle.id(),
        part_index as u32,
        selected.drive_axis,
        direction * DRIVE_SPEED,
    );
}

/// Sphere on the driven joint's true anchor plus a line along its axis. The
/// anchor comes from the baked spec (bind-pose joint origin); the body center
/// would lie (e.g. mid-chest instead of the spine).
fn draw_joint_gizmos(
    active: Res<ActiveJoint>,
    character: Query<&CharacterColliders>,
    part_transforms: Query<&Transform>,
    ragdoll_specs: Query<&JoltRagdoll>,
    mut gizmos: Gizmos,
) {
    let Ok(colliders) = character.single() else {
        return;
    };
    let selected = active.axes[active.axis_index];
    let Some(&part_entity) = colliders.collider_entities.get(&selected.joint_bone) else {
        return;
    };
    let Ok(part_pose) = part_transforms.get(part_entity) else {
        return;
    };
    let selected_joint = joint_spec(&colliders, &ragdoll_specs, &selected);
    let axis_direction = match selected.drive_axis {
        RagdollDriveAxis::Hinge => part_pose.rotation * Vec3::Z,
        RagdollDriveAxis::Twist => part_pose.rotation * twist_local(selected_joint),
        RagdollDriveAxis::Swing => part_pose.rotation * plane_local(selected_joint),
    };
    // True joint anchor from the baked spec (bind-pose joint origin), not the
    // body center: e.g. chest pivots at the spine, mid-chest is just where
    // its collider box sits.
    let anchor = joint_anchor(&colliders, &ragdoll_specs, &selected)
        .unwrap_or(part_pose.translation);
    gizmos.line(
        anchor - axis_direction * 0.15,
        anchor + axis_direction * 0.15,
        Color::srgb(0.2, 1.0, 0.2),
    );
}

/// Baked joint spec of the selected joint, if the spec is baked.
fn joint_spec(
    colliders: &CharacterColliders,
    ragdoll_specs: &Query<&JoltRagdoll>,
    selected: &JointAxis,
) -> Option<RagdollJoint> {
    let spec_entity = colliders.ragdoll_entity?;
    let spec = ragdoll_specs.get(spec_entity).ok()?;
    let part_index = colliders
        .part_order
        .iter()
        .position(|&joint_bone| joint_bone == selected.joint_bone)?;
    spec.parts.get(part_index).map(|part| part.joint)
}

/// Twist axis in the child's seated local frame (framed joints carry their
/// own; plain swing-twist uses Y).
fn twist_local(selected_joint: Option<RagdollJoint>) -> Vec3 {
    match selected_joint {
        Some(RagdollJoint::SwingTwistFramed { child_twist, .. }) => child_twist,
        _ => Vec3::Y,
    }
}

/// Plane axis in the child's seated local frame (framed joints carry their
/// own; plain swing-twist uses X).
fn plane_local(selected_joint: Option<RagdollJoint>) -> Vec3 {
    match selected_joint {
        Some(RagdollJoint::SwingTwistFramed { child_plane, .. }) => child_plane,
        _ => Vec3::X,
    }
}

fn joint_anchor(
    colliders: &CharacterColliders,
    ragdoll_specs: &Query<&JoltRagdoll>,
    selected: &JointAxis,
) -> Option<Vec3> {
    let spec_entity = colliders.ragdoll_entity?;
    let spec = ragdoll_specs.get(spec_entity).ok()?;
    let part_index = colliders
        .part_order
        .iter()
        .position(|&joint_bone| joint_bone == selected.joint_bone)?;
    let part = spec.parts.get(part_index)?;
    Some(match part.joint {
        RagdollJoint::Hinge { anchor, .. } => anchor,
        RagdollJoint::SwingTwist { anchor, .. } => anchor,
        RagdollJoint::SwingTwistFramed { anchor, .. } => anchor,
    })
}

fn update_overlay(
    active: Res<ActiveJoint>,
    character: Query<&CharacterColliders>,
    part_transforms: Query<&Transform>,
    mut text: Query<&mut Text, With<BenchText>>,
) {
    let Ok(mut overlay) = text.single_mut() else {
        return;
    };
    let selected = active.axes[active.axis_index];
    let ready = character.single().is_ok_and(|colliders| {
        colliders
            .collider_entities
            .contains_key(&selected.joint_bone)
    });
    let live_height = character
        .single()
        .ok()
        .and_then(|colliders| colliders.collider_entities.get(&selected.joint_bone))
        .and_then(|&part_entity| part_transforms.get(part_entity).ok())
        .map(|part_pose| format!("{:.3}", part_pose.translation.y))
        .unwrap_or_else(|| "—".to_string());
    overlay.0 = format!(
        "{:?} {} [{}/{}]{} | limb y={} | [ ] joint | , . axis | M motor",
        selected.joint_bone,
        selected.label(),
        active.axis_index + 1,
        active.axes.len(),
        if ready { "" } else { " (baking…)" },
        live_height,
    );
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("baking…"),
        TextLayout::justify(Justify::Right),
        TextFont::from_font_size(18.0),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.),
            right: Val::Px(12.),
            ..default()
        },
        BenchText,
    ));
}

/// Press `J`: logs every ragdoll part's live pose plus which joint axis is
/// selected, so a frozen motor shows up as unchanging numbers.
fn debug_dump(
    input: Res<ButtonInput<KeyCode>>,
    active: Res<ActiveJoint>,
    character: Query<(&CharacterColliders, &GlobalTransform)>,
    part_transforms: Query<&Transform>,
) {
    if !input.just_pressed(KeyCode::KeyJ) {
        return;
    }
    let Ok((colliders, character_pose)) = character.single() else {
        info!("bench: no character yet");
        return;
    };
    let selected = active.axes[active.axis_index];
    info!(
        "bench: character at {:?}, driving {:?} {}",
        character_pose.translation(),
        selected.joint_bone,
        selected.label(),
    );
    let mut ordered_parts: Vec<(usize, ColliderBone)> = colliders
        .part_order
        .iter()
        .enumerate()
        .map(|(part_index, &joint_bone)| (part_index, joint_bone))
        .collect();
    ordered_parts.sort_by_key(|&(part_index, _)| part_index);
    for (part_index, joint_bone) in ordered_parts {
        let pose_text = colliders
            .collider_entities
            .get(&joint_bone)
            .and_then(|&part_entity| part_transforms.get(part_entity).ok())
            .map(|part_pose| {
                format!(
                    "pos=({:.3}, {:.3}, {:.3})",
                    part_pose.translation.x, part_pose.translation.y, part_pose.translation.z
                )
            })
            .unwrap_or_else(|| "no body yet".to_string());
        let marker = if joint_bone == selected.joint_bone {
            " <-- driving"
        } else {
            ""
        };
        info!("bench: part {part_index} {joint_bone:?} {pose_text}{marker}");
    }
}

/// Drag-orbit camera: hold left mouse and drag to orbit the character,
/// mouse wheel zooms. The shared `cam_controls` has mouse look commented
/// out, so the bench owns its own orbit rig here.
fn bench_camera(
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    mut mouse_wheel: MessageReader<MouseWheel>,
    mut orbit: Local<BenchOrbit>,
    mut initialized: Local<bool>,
) {
    let Ok(mut camera_pose) = camera.single_mut() else {
        return;
    };
    if !*initialized {
        *initialized = true;
        let offset = camera_pose.translation - orbit.focus;
        orbit.distance = offset.length().max(0.5);
        orbit.yaw = offset.x.atan2(offset.z);
        orbit.pitch = (offset.y / orbit.distance).clamp(-1.0, 1.0).asin();
    }
    if mouse_button.pressed(MouseButton::Left) {
        for motion in mouse_motion.read() {
            orbit.yaw -= motion.delta.x * 0.005;
            orbit.pitch = (orbit.pitch - motion.delta.y * 0.005).clamp(-1.4, 1.4);
        }
    } else {
        mouse_motion.clear();
    }
    for scroll in mouse_wheel.read() {
        orbit.distance = (orbit.distance - scroll.y * 0.2).clamp(0.5, 20.0);
    }
    camera_pose.translation = orbit.focus
        + Vec3::new(
            orbit.distance * orbit.pitch.cos() * orbit.yaw.sin(),
            orbit.distance * orbit.pitch.sin(),
            orbit.distance * orbit.pitch.cos() * orbit.yaw.cos(),
        );
    camera_pose.look_at(orbit.focus, Vec3::Y);
}

/// Orbit state for [`bench_camera`]: focus point plus yaw/pitch/distance.
#[derive(Clone, Copy)]
struct BenchOrbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for BenchOrbit {
    fn default() -> Self {
        Self {
            focus: Vec3::new(0.0, 0.9, 0.0),
            yaw: 0.0,
            pitch: 0.1,
            distance: 4.0,
        }
    }
}

