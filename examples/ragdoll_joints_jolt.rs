//! Joint-by-joint ragdoll tuning bench (Jolt backend).
//!
//! The character hangs in the air (no floor) in full dynamic ragdoll mode.
//! One joint at a time is driven through its range by a real Jolt velocity
//! motor: `[` / `]` steps through joints, `,` / `.` steps through the
//! selected joint's axes (hinges have one; swing-twist joints expose twist
//! then swing). The motor sweeps back and forth at a fixed speed, so the
//! limits — not the drive — decide where the limb stops. `G` toggles gravity
//! for a free-hang check, `M` releases the motor so the limb hangs on limits
//! alone.
//!
//! A gizmo sphere marks the driven joint's anchor; a line shows the hinge or
//! twist axis. The overlay names the joint, its axis, its configured limits,
mod shared;

use bevy::prelude::*;
use bevy_jolt::prelude::*;
use bevy_jolt::JoltRagdollHandle;
use humentity::prelude::*;
use shared::setup_app;
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
    let mut app = setup_app();
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
    .add_systems(
        Update,
        (
            add_human.run_if(resource_exists::<HumentityAssetsReady>),
            arm_full_ragdoll,
            cycle_joint,
            gravity_toggle,
            drive_joint,
            draw_joint_gizmos,
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
        Transform::from_xyz(0.0, 1.2, 0.0),
        AnimationPlayer::default(),
        CharacterShape(shape_assets.add(template_handle)),
        BuildCpuSkeleton,
        RootOnlyRetargeting,
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

/// The plugin bakes kinematic; the bench needs dynamic from the start so the
/// motor — not the follow — owns the limb. Runs once the spec has baked.
fn arm_full_ragdoll(
    mut armed: Local<bool>,
    character: Query<(&CharacterColliders, &CharacterRagdoll)>,
    handles: Query<&JoltRagdollHandle>,
    mut physics_world: ResMut<JoltPhysicsWorld>,
) {
    if *armed {
        return;
    }
    let Ok((colliders, _)) = character.single() else {
        return;
    };
    let Some(spec_entity) = colliders.ragdoll_entity else {
        return;
    };
    let Ok(handle) = handles.get(spec_entity) else {
        return;
    };
    physics_world.ragdoll_set_motion(handle.id(), JoltMotion::Dynamic);
    *armed = true;
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

/// Sweeps the selected joint back and forth; releases the motor on `M`.
/// Part index = position of the joint's bone in the character's `part_order`
/// (spec order), which the tagger recorded at bake.
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

/// Sphere on the driven limb plus a line along its axis. The sphere sits on
/// the limb body (the anchor itself lives inside Jolt); the line shows the
/// driven axis in world space.
fn draw_joint_gizmos(
    active: Res<ActiveJoint>,
    character: Query<&CharacterColliders>,
    part_transforms: Query<&Transform>,
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
    let axis_direction = match selected.drive_axis {
        RagdollDriveAxis::Hinge => part_pose.rotation * Vec3::Z,
        RagdollDriveAxis::Twist => part_pose.rotation * Vec3::Y,
        RagdollDriveAxis::Swing => part_pose.rotation * Vec3::X,
    };
    let anchor = part_pose.translation;
    gizmos.sphere(anchor, 0.03, Color::srgb(1.0, 0.2, 0.2));
    gizmos.line(
        anchor - axis_direction * 0.15,
        anchor + axis_direction * 0.15,
        Color::srgb(0.2, 1.0, 0.2),
    );
}

/// `G` toggles gravity for a free-hang check: with gravity on, the undriven
/// joints show whether the limits hold a natural pose.
fn gravity_toggle(input: Res<ButtonInput<KeyCode>>, mut physics_world: ResMut<JoltPhysicsWorld>) {
    if !input.just_pressed(KeyCode::KeyG) {
        return;
    }
    let resting_gravity = physics_world.world_gravity();
    physics_world.set_world_gravity(if resting_gravity.length() > 0.5 {
        Vec3::ZERO
    } else {
        Vec3::new(0.0, -9.81, 0.0)
    });
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
        "{:?} {} [{}/{}]{} | limb y={} | [ ] joint | , . axis | M motor | G gravity",
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
        TextFont::from_font_size(18.0),
        BenchText,
    ));
}
