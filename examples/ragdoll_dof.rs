//! Lets you inspect one degree of freedom of the ragdoll at a time.
//!
//! The character floats in its bind pose (no floor, no floor collision). With a
//! single key press you cycle to the next joint degree of freedom. Only that one
//! DOF is made physical: the rest of the body stays in the bind pose (kinematic),
//! and the single joint's limb is driven through its limit back and forth by an
//! oscillating torque (spherical joints) or motor (revolute joints) so you can
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
use avian3d::prelude::*;
use bevy::prelude::*;
use humentity::prelude::*;
use shared::setup_app;
use std::f32::consts::FRAC_PI_2;

const RAGDOLL_LAYER: u32 = 1 << 3;
const WORLD_LAYER: u32 = 1 << 0;
const CHARACTER_LAYER: u32 = 1 << 1;

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
        let l = resolve_joint_limit(self.bone, None, mobility);
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

/// `bone` plus every collider bone descended from it (inclusive). This is the set of
/// bones that become dynamic: the active joint is the top of the chain, and all
/// descendants are held rigid by locking their joints, so the whole limb moves as a
/// unit about the single active DOF.
fn subchain(bone: ColliderBone) -> Vec<ColliderBone> {
    let mut children: AHashMap<ColliderBone, Vec<ColliderBone>> = default();
    for &b in COLLIDERS.iter() {
        if let Some(p) = get_collider_parent(b) {
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
        .add_systems(Startup, spawn_status_text)
        .add_systems(Update, add_human.run_if(resource_exists::<HumentityAssetsReady>))
        .add_systems(
            Update,
            (
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

/// Offsets the character by its bind-pose lowest vertex height so it sits at a
/// fixed, known height (y = 0 at the feet). The bind-pose vertices become
/// available asynchronously, so we wait for them. Idempotent: it just re-asserts
/// the (constant) transform once the vertices are known.
fn ground_character(
    mut commands: Commands,
    characters: Query<(Entity, &HelperVertexPositions), With<CharacterShape>>,
) {
    for (entity, helpers) in &characters {
        if helpers.0.is_empty() {
            continue;
        }
        let min_y = helpers.0.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
        // +0.05 clearance above y = 0.
        let y = -min_y + 0.05;
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
    if pressed.is_some() {
        // Start each DOF at the sweep end nearest bind (zero initial drive):
        // knees rest at max (straight), elbows at min, sphericals at center.
        let dof = &dofs()[active.index];
        active.phase = match dof.axis {
            Axis::Hinge => {
                let l = default_joint_limit(dof.bone);
                if l.angle_min.abs() < l.angle_max.abs() {
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
    mut bones: Query<&mut Transform, With<SkeletalBone>>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
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
            // Seat every collider body at its bind-pose *world* transform,
            // rebuilt by composing the captured bind locals top-down under the
            // skeleton entity's global. That global carries MODEL_ROTATION_FIX
            // (pi about Y); mesh-part globals don't, so seating from inverse
            // bindposes mirrored every body left/right. Bind locals are
            // displacement-independent, unlike live GlobalTransforms, so the
            // previously driven DOF can't pollute the reset. Velocity is zeroed
            // so a fast-swinging limb doesn't fling on reset.
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
                    // Bind entries are displacement-proof; intermediate links
                    // like the armature object have none, so take their local
                    // instead — the armature already matches this query, and
                    // nothing but bones ever moves.
                    if let Some(bind) = bind_pose.0.get(&entity) {
                        world = world * *bind;
                    } else if let Ok(local) = bones.get(entity) {
                        world = world * *local;
                    } else {
                        complete = false;
                        break;
                    }
                }
                if !complete {
                    continue;
                }
                if let Ok((mut pos, mut rot, offset)) = collider_poses.get_mut(collider_entity)
                {
                    let body_world = world * offset.collider_to_bone;
                    pos.0 = body_world.translation;
                    rot.0 = body_world.rotation;
                }
                if let Ok((mut lv, mut av)) = collider_vels.get_mut(collider_entity) {
                    *lv = LinearVelocity::ZERO;
                    *av = AngularVelocity::ZERO;
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

    // 3. Clear any motor state left over from a previous selection.
    for mut joint in revolute_joints.iter_mut() {
        joint.motor.enabled = false;
    }

    active.applied = true;
}

/// Drives the active DOF through its limit each frame.
///
/// Spherical DOFs sweep at a prescribed angular velocity about the live joint
/// frame (`local_twist_axis2` for twist, the orthogonal swing axis for swing).
/// Open-loop torque is banned here: constant torque spins feather-weight
/// extremities up until NaN kills avian's AABB pass. Prescribing the sweep
/// velocity bounds the motion by construction, and the amplitude follows the
/// resolved limit so each DOF parks at its bite. Hinge DOFs reuse the joint's
/// angular motor with a target swept between the resolved limits.
fn drive_dof(
    time: Res<Time>,
    mut active: ResMut<ActiveDof>,
    character: Query<(&CharacterColliders, Option<&RagdollMobility>)>,
    mut revolute_joints: Query<&mut RevoluteJoint>,
    spherical_joints: Query<&SphericalJoint>,
    mut bodies: Query<(&Rotation, &mut AngularVelocity), With<ColliderBone>>,
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

    let m = mobility.map_or(1.0, |mob| mob.0);

    active.phase += time.delta_secs() * OMEGA;
    let s = active.phase.sin();

    match dof.axis {
        Axis::Hinge => {
            let limit = resolve_joint_limit(dof.bone, None, m);
            let mid = (limit.angle_min + limit.angle_max) * 0.5;
            let half = (limit.angle_max - limit.angle_min) * 0.5;
            let target = mid + half * s;
            for mut joint in revolute_joints.iter_mut() {
                if joint.body2 == active_collider {
                    joint.motor = AngularMotor::new(MotorModel::SpringDamper {
                        frequency: 3.0,
                        damping_ratio: 1.0,
                    })
                    .with_target_position(target)
                    .with_max_torque(500.0);
                }
            }
        }
        Axis::Twist | Axis::Swing => {
            // Child-local drive axis from the live joint frame, rotated into
            // world: `AngularVelocity` is world-frame, so writing the local
            // axis raw cross-wires twist and swing (e.g. head swing yaws).
            // Right after a selection change the joint may not have respawned
            // yet; skip the frame in that case.
            let mut axis = None;
            for joint in spherical_joints.iter() {
                if joint.body2 != active_collider {
                    continue;
                }
                axis = Some(if dof.axis == Axis::Twist {
                    joint.local_twist_axis2().unwrap_or(Vec3::Y)
                } else {
                    joint
                        .local_basis2()
                        .map_or(Vec3::X, |b| b * joint.twist_axis.any_orthonormal_vector())
                });
                break;
            }
            if let Some(axis) = axis.filter(|a| a.is_finite()) {
                let limit = resolve_joint_limit(dof.bone, None, m);
                let range = if dof.axis == Axis::Twist {
                    limit.twist
                } else {
                    limit.swing
                };
                let target = range * OMEGA * active.phase.cos();
                if target.is_finite() {
                    if let Ok((rot, mut av)) = bodies.get_mut(active_collider) {
                        let axis = rot.0 * axis;
                        let w: Vec3 = av.0;
                        av.0 = w - axis * w.dot(axis) + axis * target;
                    }
                }
            }
        }
    }

    if let Ok(mut text) = text.single_mut() {
        *text = Text::new(status_string(active.index, m));
    }
}

fn gravity_toggle(input: Res<ButtonInput<KeyCode>>, mut gravity: ResMut<Gravity>) {
    if input.just_pressed(KeyCode::KeyG) {
        gravity.0 = if gravity.0 == Vec3::ZERO {
            Vec3::NEG_Y * 9.81
        } else {
            Vec3::ZERO
        };
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
