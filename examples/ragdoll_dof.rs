//! Lets you inspect one degree of freedom of the ragdoll at a time.
//!
//! The character stands on the floor in its bind pose. With a single key press you
//! cycle to the next joint degree of freedom. Only that one DOF is made physical:
//! the rest of the body stays in the bind pose (kinematic), and the single joint's
//! limb is driven through its limit back and forth by an oscillating torque (spherical
//! joints) or motor (revolute joints) so you can watch where the limit bites and
//! judge whether it is appropriate.
//!
//! Joints come in two families:
//!   - **Spherical joints** (shoulders, hips, spine, neck, wrists, ankles):
//!     3 rotational DOF — `swing` (a cone, 2 DOF) and `twist` about the bone axis (1 DOF).
//!     Each is exposed as two separate DOFs: `Swing` and `Twist`.
//!   - **Revolute joints** (elbows, knees): 1 rotational DOF — a hinge `angle`.
//!
//! Controls:
//!   - `[` / `]`   cycle to the previous / next degree of freedom
//!   - `,` / `.`   jump to the previous / next *joint* (its first DOF)
//!   - `G`         toggle gravity
//!
//! The on-screen overlay shows the active DOF, its current limit, and the controls.

mod shared;

use ahash::AHashMap;
use avian3d::dynamics::rigid_body::forces::ConstantLocalTorque;
use avian3d::prelude::*;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use humentity::prelude::*;
use shared::setup_app;

const RAGDOLL_LAYER: u32 = 1 << 3;
const WORLD_LAYER: u32 = 1 << 0;
const CHARACTER_LAYER: u32 = 1 << 1;

/// Angular speed (rad/s) of the oscillation that sweeps each DOF through its limit.
const OMEGA: f32 = 1.2;
/// Peak torque (N·m) used to drive spherical (swing/twist) DOFs.
const TORQUE: f32 = 8.0;

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

    fn limit_text(&self) -> String {
        let l = default_joint_limit(self.bone);
        match self.axis {
            Axis::Twist => format!("twist ±{:.0}°", l.twist.to_degrees()),
            Axis::Swing => format!("swing ±{:.0}°", l.swing.to_degrees()),
            Axis::Hinge => format!(
                "angle {:.0}° .. {:.0}°",
                l.angle_min.to_degrees(),
                l.angle_max.to_degrees()
            ),
        }
    }
}

/// Every selectable DOF, in display order.
fn dofs() -> Vec<Dof> {
    let mut v = Vec::new();
    // Spherical joints expose swing and twist separately.
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
        v.push(Dof {
            bone,
            axis: Axis::Swing,
        });
        v.push(Dof {
            bone,
            axis: Axis::Twist,
        });
    }
    // Revolute joints (elbows, knees) have a single hinge DOF.
    for &bone in &[
        ColliderBone::LowerRightArm,
        ColliderBone::LowerLeftArm,
        ColliderBone::LowerRightLeg,
        ColliderBone::LowerLeftLeg,
    ] {
        v.push(Dof {
            bone,
            axis: Axis::Hinge,
        });
    }
    v
}

/// Parent bone of a collider bone (mirrors `get_collider_parent` in the crate).
const fn collider_parent(bone: ColliderBone) -> Option<ColliderBone> {
    match bone {
        ColliderBone::Head => Some(ColliderBone::Chest),
        ColliderBone::Chest => Some(ColliderBone::Pelvis),
        ColliderBone::Pelvis => None,
        ColliderBone::UpperRightArm => Some(ColliderBone::Chest),
        ColliderBone::UpperLeftArm => Some(ColliderBone::Chest),
        ColliderBone::LowerRightArm => Some(ColliderBone::UpperRightArm),
        ColliderBone::LowerLeftArm => Some(ColliderBone::UpperLeftArm),
        ColliderBone::UpperRightLeg => Some(ColliderBone::Pelvis),
        ColliderBone::UpperLeftLeg => Some(ColliderBone::Pelvis),
        ColliderBone::LowerRightLeg => Some(ColliderBone::UpperRightLeg),
        ColliderBone::LowerLeftLeg => Some(ColliderBone::UpperLeftLeg),
        ColliderBone::LeftHand => Some(ColliderBone::LowerLeftArm),
        ColliderBone::RightHand => Some(ColliderBone::LowerRightArm),
        ColliderBone::LeftFoot => Some(ColliderBone::LowerLeftLeg),
        ColliderBone::RightFoot => Some(ColliderBone::LowerRightLeg),
    }
}

/// `bone` plus every collider bone descended from it (inclusive). This is the set of
/// bones that become dynamic: the active joint is the top of the chain, and all
/// descendants are held rigid by locking their joints, so the whole limb moves as a
/// unit about the single active DOF.
fn subchain(bone: ColliderBone) -> Vec<ColliderBone> {
    let mut children: AHashMap<ColliderBone, Vec<ColliderBone>> = default();
    for &b in COLLIDERS.iter() {
        if let Some(p) = collider_parent(b) {
            children.entry(p).or_default().push(b);
        }
    }
    let mut out = vec![bone];
    let mut stack = vec![bone];
    while let Some(b) = stack.pop() {
        if let Some(kids) = children.get(&b) {
            for k in kids {
                out.push(*k);
                stack.push(*k);
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

    app.add_plugins((PhysicsPlugins::default(), PhysicsDebugPlugin))
        .insert_resource(SubstepCount(10))
        .insert_resource(Gravity(Vec3::ZERO))
        .insert_resource(ActiveDof::default())
        .insert_resource(BindPose::default())
        .add_systems(Startup, (floor, spawn_status_text, add_human))
        .add_systems(
            Update,
            (
                ground_character,
                capture_bind_pose,
                cycle_dof,
                apply_active_dof,
                reset_inactive_bones,
                drive_dof,
                gravity_toggle,
            ),
        )
        .run();
}

/// Lowers the character so its bind-pose feet sit on the floor (y = 0). The bind-pose
/// vertices become available asynchronously, so we wait for them and then offset the
/// `CharacterShape` by the lowest vertex height. Idempotent: it just re-asserts the
/// (constant) grounded transform once the vertices are known.
fn ground_character(
    mut commands: Commands,
    characters: Query<(Entity, &HelperVertexPositions), With<CharacterShape>>,
) {
    for (entity, helpers) in &characters {
        if helpers.0.is_empty() {
            continue;
        }
        let min_y = helpers.0.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
        // +0.05 so the bind-pose feet rest on top of the floor slab.
        let y = -min_y + 0.05;
        info!(
            "ground_character: lowering CharacterShape to y = {y:.3} (min vertex y = {min_y:.3})"
        );
        commands
            .entity(entity)
            .insert(Transform::from_xyz(0.0, y, 0.0));
    }
}

/// Snapshots every skeleton bone's local transform once the skeleton is fitted, so
/// inactive bones can be snapped back to the bind pose.
fn capture_bind_pose(
    characters: Query<&CharacterSkeleton, Added<SkeletonsReady>>,
    bones: Query<&Transform>,
    mut bind_pose: ResMut<BindPose>,
) {
    for skeleton in &characters {
        for &bone_entity in skeleton.bone_map.values() {
            if let Ok(t) = bones.get(bone_entity) {
                bind_pose.0.insert(bone_entity, *t);
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
        if let Some(&e) = colliders.bone_entities.get(&bone) {
            active_entities.insert(e);
        }
    }

    for &bone_entity in skeleton.bone_map.values() {
        if active_entities.contains(&bone_entity) {
            continue;
        }
        if let (Ok(mut t), Some(bind)) = (bones.get_mut(bone_entity), bind_pose.0.get(&bone_entity))
        {
            *t = *bind;
        }
    }
}

fn cycle_dof(input: Res<ButtonInput<KeyCode>>, mut active: ResMut<ActiveDof>) {
    let list = dofs();
    let n = list.len();
    let mut pressed: Option<&str> = None;
    if input.just_pressed(KeyCode::BracketRight) {
        active.index = (active.index + 1) % n;
        active.applied = false;
        pressed = Some("]");
    }
    if input.just_pressed(KeyCode::BracketLeft) {
        active.index = (active.index + n - 1) % n;
        active.applied = false;
        pressed = Some("[");
    }
    // Jump to the next joint (the first DOF of the next joint in the list).
    if input.just_pressed(KeyCode::Period) {
        let mut i = (active.index + 1) % n;
        while i != active.index && list[i].bone == list[active.index].bone {
            i = (i + 1) % n;
        }
        active.index = i;
        active.applied = false;
        pressed = Some(".");
    }
    if input.just_pressed(KeyCode::Comma) {
        let mut i = (active.index + n - 1) % n;
        while i != active.index && list[i].bone == list[active.index].bone {
            i = (i + n - 1) % n;
        }
        active.index = i;
        active.applied = false;
        pressed = Some(",");
    }
    if let Some(key) = pressed {
        info!(
            "cycle_dof: pressed '{}' -> index {} / {} ({})",
            key,
            active.index + 1,
            n,
            list[active.index].label()
        );
    }
}

/// Reconfigures the ragdoll so only the active joint is free. Runs whenever the
/// selection changes (or once colliders have spawned).
fn apply_active_dof(
    mut commands: Commands,
    mut active: ResMut<ActiveDof>,
    bind_pose: Res<BindPose>,
    mut character: Query<(
        &mut CharacterRagdoll,
        &mut RagdollJointLimitOverrides,
        &CharacterSkeleton,
        &CharacterColliders,
    )>,
    mut bones: Query<&mut Transform, With<SkeletalBone>>,
    skinned_meshes: Query<(&SkinnedMesh, &GlobalTransform)>,
    inv_bind_assets: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut collider_poses: Query<(&mut Position, &mut Rotation, &ColliderOffset), With<ColliderBone>>,
    mut collider_vels: Query<(&mut LinearVelocity, &mut AngularVelocity), With<ColliderBone>>,
    mut revolute_joints: Query<&mut RevoluteJoint>,
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

        if bind_pose.0.is_empty() {
            // Capture hasn't run yet; nothing to reset against.
        } else {
            // Seat every collider body at its bind-pose *world* transform, taken
            // directly from the `SkinnedMesh` inverse bindposes — the canonical rest
            // pose (mesh-local space), composed with the skinned mesh's world
            // transform. Bevy's skinning is `joint_matrix = joint_world *
            // inverse_bindpose`, so `joint_world_bind = skinned_mesh_global *
            // inverse(inverse_bindpose)`. This is displacement-independent and correct
            // for children of the active bone. Velocity is zeroed so a fast-swinging
            // limb doesn't fling on reset.
            let bone_to_collider: AHashMap<Entity, Entity> = colliders
                .bone_entities
                .iter()
                .filter_map(|(&bone, &bone_entity)| {
                    colliders
                        .collider_entities
                        .get(&bone)
                        .map(|&collider_entity| (bone_entity, collider_entity))
                })
                .collect();

            for (skm, skm_global) in skinned_meshes.iter() {
                let Some(inv_bind) = inv_bind_assets.get(&skm.inverse_bindposes) else {
                    continue;
                };
                for (i, &joint_entity) in skm.joints.iter().enumerate() {
                    let Some(&collider_entity) = bone_to_collider.get(&joint_entity) else {
                        continue;
                    };
                    // `inv_bind[i]` is model→joint in MODEL space. The bind-pose joint
                    // transform is its inverse (joint→model), and the collider body
                    // sits at `collider_to_model = joint→model * collider_to_bone`,
                    // all composed in model space. Lift the result to world via the
                    // skinned mesh's global transform.
                    if let Ok((mut pos, mut rot, offset)) = collider_poses.get_mut(collider_entity)
                    {
                        let joint_model = Transform::from_matrix(inv_bind[i].inverse());
                        let collider_model = joint_model * offset.collider_to_bone;
                        let body_world = skm_global.mul_transform(collider_model);
                        pos.0 = body_world.translation();
                        rot.0 = body_world.rotation();
                    }
                    if let Ok((mut lv, mut av)) = collider_vels.get_mut(collider_entity) {
                        *lv = LinearVelocity::ZERO;
                        *av = AngularVelocity::ZERO;
                    }
                }
            }
            // Then snap every bone's local transform to the bind pose so the visible
            // mesh (and inactive bones, which `sync_bones_to_ragdoll` skips) are clean.
            for &bone_entity in skeleton.bone_map.values() {
                if let (Ok(mut t), Some(bind)) =
                    (bones.get_mut(bone_entity), bind_pose.0.get(&bone_entity))
                {
                    *t = *bind;
                }
            }
        }
    }

    let Ok((mut ragdoll, mut overrides, _skeleton, colliders)) = character.single_mut() else {
        return;
    };

    let list = dofs();
    let dof = &list[active.index];

    // 1. Make the active joint's limb (and its descendants) dynamic; everything else
    //    stays kinematic in the bind pose.
    let chain = subchain(dof.bone);

    info!(
        "apply_active_dof: index {} -> {} (chain: {:?})",
        active.index,
        dof.label(),
        chain
    );

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

    // 3. Clear any drive state left over from a previous selection.
    for &collider_entity in colliders.collider_entities.values() {
        commands
            .entity(collider_entity)
            .remove::<ConstantLocalTorque>();
    }
    for mut joint in revolute_joints.iter_mut() {
        joint.motor.enabled = false;
    }

    active.applied = true;
}

/// Drives the active DOF through its limit each frame.
fn drive_dof(
    time: Res<Time>,
    mut active: ResMut<ActiveDof>,
    mut commands: Commands,
    character: Query<&CharacterColliders>,
    mut revolute_joints: Query<&mut RevoluteJoint>,
    mut text: Query<&mut Text, With<DofText>>,
) {
    let Ok(colliders) = character.single() else {
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

    active.phase += time.delta_secs() * OMEGA;
    let s = active.phase.sin();

    match dof.axis {
        Axis::Hinge => {
            // Use the joint's angular motor, targeting a sine sweep between limits.
            let def = default_joint_limit(dof.bone);
            let mid = (def.angle_min + def.angle_max) * 0.5;
            let half = (def.angle_max - def.angle_min) * 0.5;
            let target = mid + half * s;
            for mut joint in revolute_joints.iter_mut() {
                if joint.body2 == active_collider {
                    joint.motor = AngularMotor::new(MotorModel::SpringDamper {
                        frequency: 2.0,
                        damping_ratio: 1.0,
                    })
                    .with_target_position(target)
                    .with_max_torque(50.0);
                }
            }
        }
        Axis::Twist => {
            // Twist is about the bone's local Y axis.
            commands
                .entity(active_collider)
                .insert(ConstantLocalTorque(Vec3::Y * TORQUE * s));
        }
        Axis::Swing => {
            // Swing is about a local axis perpendicular to the bone (local X).
            commands
                .entity(active_collider)
                .insert(ConstantLocalTorque(Vec3::X * TORQUE * s));
        }
    }

    if let Ok(mut text) = text.single_mut() {
        *text = Text::new(status_string(active.index));
    }
}

fn gravity_toggle(input: Res<ButtonInput<KeyCode>>, mut gravity: ResMut<Gravity>) {
    if input.just_pressed(KeyCode::KeyG) {
        gravity.0 = if gravity.0 == Vec3::ZERO {
            Vec3::NEG_Y * 9.81
        } else {
            Vec3::ZERO
        };
        info!(
            "gravity_toggle: gravity now {}",
            if gravity.0 == Vec3::ZERO { "OFF" } else { "ON" }
        );
    }
}

fn status_string(index: usize) -> String {
    let list = dofs();
    let dof = &list[index];
    format!(
        "[ / ]: cycle DOF   , / .: jump joint   G: gravity\n\nActive DOF ({} / {}):\n  {}\n  limit: {}",
        index + 1,
        list.len(),
        dof.label(),
        dof.limit_text(),
    )
}

fn spawn_status_text(mut commands: Commands) {
    commands.spawn((
        DofText,
        Text::new(status_string(0)),
        TextFont::from_font_size(22.0),
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.),
            left: Val::Px(12.),
            ..default()
        },
    ));
}

fn floor(mut commands: Commands) {
    commands.spawn((
        Collider::cuboid(100.0, 0.1, 100.0),
        Friction::new(0.5),
        Restitution::new(0.1),
        RigidBody::Static,
        Transform::IDENTITY,
    ));
}

fn add_human(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {
    let template_handle = template_assets.add(CharacterTemplate::new([]));

    let basemesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: 0,
    });

    // The character is lowered onto the floor once its bind-pose vertices are ready
    // (see `ground_character`); it starts at the origin.
    commands.spawn((
        Transform::IDENTITY,
        CharacterShape(shape_assets.add(template_handle)),
        HelperVertexPositions::default(),
        CharacterRagdoll::None,
        CharacterColliders::new(None),
        RagdollJointLimitOverrides::default(),
        RagdollCollisionLayers(CollisionLayers::new(
            RAGDOLL_LAYER,
            WORLD_LAYER | CHARACTER_LAYER | RAGDOLL_LAYER,
        )),
        RagdollMobility(1.0),
        RagdollDensity(10.0),
        RagdollDamping::default(),
        children![(CharacterPart {
            mesh: basemesh,
            skeleton_lod: 0
        },)],
    ));
}
