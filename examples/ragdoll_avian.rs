mod shared;

use avian3d::prelude::*;
use bevy::{math::Isometry3d, prelude::*};
use humentity::prelude::*;
use shared::setup_app;

const RAGDOLL_LAYER: u32 = 1 << 3;
const WORLD_LAYER: u32 = 1 << 0;
const CHARACTER_LAYER: u32 = 1 << 1;

fn main() {
    let mut app = setup_app();

    app.add_plugins((
        PhysicsPlugins::default(),
        //PhysicsDebugPlugin
    ))
    .insert_resource(SubstepCount(10))
    .add_systems(Startup, (floor, spawn_ui, joint_gizmos_on_top))
    .add_systems(
        Update,
        add_human.run_if(resource_exists::<HumentityAssetsReady>),
    )
    .add_systems(Update, (toggle, sleep_ragdoll, setup_graph, start_clip, dump_collider_states, draw_joint_anchors))
    .run();
}

#[derive(Component)]
struct AnimationController(AnimationNodeIndex);

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

    let clips = asset_server.load::<RotationOnlyAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RotationOnlyAnimations { _clips: clips });

    commands.spawn((
        Transform::from_xyz(0.0, 0.0, 0.0),
        AnimationPlayer::default(),
        CharacterShape(shape_assets.add(template_handle)),
        BuildCpuSkeleton,
        RootOnlyRetargeting,
        CharacterRagdoll::None,
        CharacterColliders::new(None),
        RagdollCollisionLayers(CollisionLayers::new(
            RAGDOLL_LAYER,
            WORLD_LAYER | CHARACTER_LAYER | RAGDOLL_LAYER,
        )),
        RagdollMobility(1.0),
        // Ragdolls tend to twitch without higher density settings in my findings
        RagdollDensity(10.0),
        children![(CharacterPart {
            mesh: basemesh,
            skeleton_lod: 0
        },)],
    ));
}

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut character: Query<(
        Entity,
        &mut CharacterRagdoll,
        &mut CharacterColliders,
        &mut AnimationPlayer,
        Option<&AnimationController>,
    )>,
    mut collider_data: Query<&mut LinearVelocity>,
    mut commands: Commands,
) {
    if !input.just_pressed(KeyCode::Space) {
        return;
    }

    let Ok((entity, mut ragdoll, colliders, mut player, controller)) = character.single_mut()
    else {
        return;
    };

    let clip_index = controller.map_or_else(|| AnimationNodeIndex::new(0), |c| c.0);

    match &*ragdoll {
        CharacterRagdoll::Full => {
            // Toggle ragdoll off: snap bones back to the fitted bind pose so
            // stale ragdoll translations don't survive under the rotation-only
            // clip, then restart the clip.
            *ragdoll = CharacterRagdoll::None;

            commands.trigger(ResetToBindPose(entity));
            player.stop(clip_index);
            player.play(clip_index).repeat();
        }
        _ => {
            // Toggle ragdoll on
            *ragdoll = CharacterRagdoll::Full;

            // Stop animation so it doesn't compete with physics
            player.stop(clip_index);

            // Give each collider a small nudge so the collapse is interesting.
            // Varies with time so each activation is unique.
            for (bone, &collider_entity) in colliders.collider_entities.iter() {
                if let Ok(mut vel) = collider_data.get_mut(collider_entity) {
                    let i = *bone as usize;
                    let t = time.elapsed_secs();
                    let seed = (t * 100.0).floor() / 100.0;
                    let h = ((i as f32 + seed) * 0x9e3779b9u32 as f32).sin();
                    let dir = Vec3::new(
                        ((h * 43_758.547).fract() - 0.5) * 2.0,
                        ((h * 27_118.313).fract()) * 0.5,
                        ((h * 30_903.55).fract() - 0.5) * 2.0,
                    )
                    .normalize_or_zero();
                    vel.0 += dir * 2.0;
                }
            }
        }
    }
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("SPACE: toggle full ragdoll (avian) | J: dump collider states"),
        TextLayout::justify(Justify::Right),
        TextFont::from_font_size(24.0),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.),
            right: Val::Px(12.),
            ..default()
        },
    ));
}

#[derive(Component)]
struct RagdollSleepTimer(Timer);

impl Default for RagdollSleepTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(2., TimerMode::Once))
    }
}

fn sleep_ragdoll(
    time: Res<Time>,
    mut commands: Commands,
    mut timers: Query<(Entity, &mut RagdollSleepTimer)>,
    changed_ragdolls: Query<
        (Entity, &CharacterRagdoll, &CharacterColliders),
        Changed<CharacterRagdoll>,
    >,
) {
    for (entity, ragdoll, colliders) in changed_ragdolls.iter() {
        match ragdoll {
            CharacterRagdoll::Full | CharacterRagdoll::Partial(_) => {
                commands.entity(entity).insert(RagdollSleepTimer::default());
            }
            CharacterRagdoll::None => {
                commands.entity(entity).remove::<RagdollSleepTimer>();
                for &collider_entity in colliders.collider_entities.values() {
                    commands
                        .entity(collider_entity)
                        .remove::<RigidBodyDisabled>()
                        .remove::<ColliderDisabled>();
                }
            }
        }
    }

    for (entity, mut timer) in timers.iter_mut() {
        timer.0.tick(time.delta());
        if timer.0.just_finished() {
            commands.trigger(DisablePhysics { character: entity });
            commands.entity(entity).remove::<RagdollSleepTimer>();
        }
    }
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

#[derive(Resource)]
struct RotationOnlyAnimations {
    _clips: Handle<RotationOnlyAnimationAsset>,
}

fn setup_graph(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    rotation_only_clips: Res<Assets<RotationOnlyAnimationAsset>>,
    mut character_player: Query<(Entity, &mut AnimationPlayer), Without<AnimationGraphHandle>>,
) {
    let Ok((entity, _player)) = character_player.single_mut() else {
        return;
    };
    let Some((_id, clips_map)) = rotation_only_clips.iter().next() else {
        return;
    };
    let clip_handle = clips_map.clips.get("Idle-loop").unwrap();
    let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());

    commands.entity(entity).insert((
        AnimationController(index),
        AnimationGraphHandle(graphs.add(graph)),
    ));
}

fn start_clip(
    anim: Single<(&mut AnimationPlayer, &AnimationController), Added<AnimationController>>,
) {
    let (mut player, controller) = anim.into_inner();
    player.play(controller.0).repeat();
}

/// Prints every collider's live pose + avian sleep state. Press J while the
/// ragdoll is active: parts that never sleep point at unsettled joints.
/// Avian colliders carry `SleepingDisabled`, so the useful signal is the
/// marker's presence: `sleep-off` (marker removed, free to settle) vs
/// `no-sleep` (marker still on, pinned awake). Mirrors the jolt example's
/// `dump_joint_angles` (same key, same line shape).
/// Example line: `knee.L live y=0.412 sleep-off`.
fn dump_collider_states(
    input: Res<ButtonInput<KeyCode>>,
    transform_query: Query<&Transform>,
    sleeping_query: Query<&SleepingDisabled>,
    character_query: Query<&CharacterColliders>,
) {
    if !input.just_pressed(KeyCode::KeyJ) {
        return;
    }
    let Ok(colliders) = character_query.single() else {
        // Character not spawned yet (assets still loading): nothing to dump.
        return;
    };
    for (collider_bone, &collider_entity) in colliders.collider_entities.iter() {
        let Ok(collider_transform) = transform_query.get(collider_entity) else {
            // Collider despawned mid-transition (ragdoll teardown): skip it.
            continue;
        };
        let sleep_state = if sleeping_query.contains(collider_entity) {
            "no-sleep"
        } else {
            "sleep-off"
        };
        println!(
            "{collider_bone:?} live y={:.3} {}",
            collider_transform.translation.y, sleep_state,
        );
    }
}

/// Draws one sphere per joint at the live constraint position, rendered on
/// top of everything (`depth_bias: -1`). Revolute joints (elbows, knees)
/// draw orange, spherical joints cyan, so a glance shows which constraint
/// type guards each pivot. Mirrors the jolt example's `draw_joint_anchors`,
/// adapted to avian joint components: the queries read `RevoluteJoint` and
/// `SphericalJoint` directly, and limits come from `resolve_joint_limit`
/// (same source the backend bakes from).
///
/// The sphere follows the child collider: the bake anchor is bind-pose (the
/// child's seated position when the joint spawned), so each frame
/// re-expresses the anchor in the child collider's current frame via the
/// collider's own `Transform` (avian colliders carry no `GlobalTransform`;
/// they are world-root children under the physics container).
///
/// Spherical joints also get a wireframe swing cone plus a twist-axis arrow,
/// both in the *parent* side's live frame. Revolute joints get their axis
/// arrow plus the min/max sweep arc.
/// Draws one sphere per joint at the live constraint position, rendered on
/// top of everything (`depth_bias: -1`). Revolute joints (elbows, knees)
/// draw orange, spherical joints cyan, so a glance shows which constraint
/// type guards each pivot. Mirrors the jolt example's `draw_joint_anchors`,
/// adapted to avian joint components: the queries read `RevoluteJoint` and
/// `SphericalJoint` directly, and limits come from `resolve_joint_limit`
/// (same source the backend bakes from).
///
/// Before the flip there are no joint components yet (the backend spawns
/// them in `set_ragdoll_state`), so the preview branch derives the same
/// anchor the backend will bake — child bone origin, chest at the pelvis
/// top-center — and draws the same shapes from the colliders' live frames.
/// After the flip the joint branch below reads the real components.
fn draw_joint_anchors(
    revolute_query: Query<(&RevoluteJoint, &Name)>,
    spherical_query: Query<(&SphericalJoint, &Name)>,
    collider_poses: Query<&Transform>,
    bone_worlds: Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    collider_shapes: Query<&Collider>,
    preview_roots: Query<(
        &CharacterColliders,
        Option<&RagdollJointLimitOverrides>,
        Option<&RagdollMobility>,
    )>,
    mut gizmos: Gizmos,
) {
    for (character_colliders, limit_overrides, mobility) in &preview_roots {
        let mobility_scale = mobility.map_or(1.0, |ragdoll_mobility| ragdoll_mobility.0);
        for (collider_bone, &child_collider) in character_colliders.collider_entities.iter() {
            let Some(parent_bone) = get_collider_parent(*collider_bone) else {
                // Root collider (pelvis): no joint above it, nothing to draw.
                continue;
            };
            let Some(&parent_collider) = character_colliders.collider_entities.get(&parent_bone)
            else {
                // Parent collider not spawned (partial subset): skip this joint.
                continue;
            };
            let (Ok(child_pose), Ok(parent_pose)) = (
                collider_poses.get(child_collider),
                collider_poses.get(parent_collider),
            ) else {
                // Colliders despawned mid-transition (ragdoll teardown): skip.
                continue;
            };
            let joint_limit = resolve_joint_limit(*collider_bone, limit_overrides, mobility_scale);
            // Revolute joints guard elbows and knees; everything else is
            // spherical. The backend spawns exactly one of the two per bone,
            // so matching on the bone keeps the queries disjoint.
            let hinge_bone = matches!(
                collider_bone,
                ColliderBone::LowerRightArm
                    | ColliderBone::LowerLeftArm
                    | ColliderBone::LowerRightLeg
                    | ColliderBone::LowerLeftLeg
            );
            if hinge_bone {
                if let Some((hinge_joint, _)) = revolute_query
                    .iter()
                    .find(|(revolute_joint, _)| revolute_joint.body2 == child_collider)
                {
                    let live_anchor = joint_live_anchor(
                        hinge_joint.frame1.anchor,
                        hinge_joint.frame2.anchor,
                        parent_pose,
                        child_pose,
                    );
                    draw_hinge_shapes(
                        &mut gizmos,
                        live_anchor,
                        child_pose,
                        parent_pose,
                        joint_limit.angle_min,
                        joint_limit.angle_max,
                    );
                } else {
                    // Joint not spawned yet (ragdoll off): preview the anchor
                    // the backend will bake, in the colliders' live frames.
                    let Some(preview_anchor) = preview_joint_anchor(
                        *collider_bone,
                        character_colliders,
                        &bone_worlds,
                        &collider_shapes,
                        parent_pose,
                    ) else {
                        continue;
                    };
                    draw_hinge_shapes(
                        &mut gizmos,
                        preview_anchor,
                        child_pose,
                        parent_pose,
                        joint_limit.angle_min,
                        joint_limit.angle_max,
                    );
                }
            } else if let Some((swing_twist_joint, _)) = spherical_query
                .iter()
                .find(|(spherical_joint, _)| spherical_joint.body2 == child_collider)
            {
                let live_anchor = joint_live_anchor(
                    swing_twist_joint.frame1.anchor,
                    swing_twist_joint.frame2.anchor,
                    parent_pose,
                    child_pose,
                );
                draw_swing_twist_shapes(
                    &mut gizmos,
                    live_anchor,
                    child_pose,
                    parent_pose,
                    joint_limit.swing,
                );
            } else {
                // Joint not spawned yet (ragdoll off): preview the anchor the
                // backend will bake, in the colliders' live frames.
                let Some(preview_anchor) = preview_joint_anchor(
                    *collider_bone,
                    character_colliders,
                    &bone_worlds,
                    &collider_shapes,
                    parent_pose,
                ) else {
                    continue;
                };
                draw_swing_twist_shapes(
                    &mut gizmos,
                    preview_anchor,
                    child_pose,
                    parent_pose,
                    joint_limit.swing,
                );
            }
        }
    }
}

/// Anchor the backend will bake for this joint: the child bone's origin
/// (anatomical pivot for limbs), except the chest which pivots at the waist
/// (top-center of the parent pelvis collider). Mirrors `set_ragdoll_state`.
fn preview_joint_anchor(
    joint_bone: ColliderBone,
    character_colliders: &CharacterColliders,
    bone_worlds: &Query<&GlobalTransform, Allow<SkeletonLodDisabled>>,
    collider_shapes: &Query<&Collider>,
    parent_pose: &Transform,
) -> Option<Vec3> {
    if joint_bone == ColliderBone::Chest {
        let parent_bone = get_collider_parent(joint_bone)?;
        let &parent_collider = character_colliders.collider_entities.get(&parent_bone)?;
        let parent_shape = collider_shapes.get(parent_collider).ok()?;
        let half_up = parent_shape.shape().as_cuboid().map(|cuboid| cuboid.half_extents.y)?;
        return Some(parent_pose.transform_point(Vec3::new(0.0, half_up, 0.0)));
    }
    let &child_bone_entity = character_colliders.bone_entities.get(&joint_bone)?;
    let child_bone_world = bone_worlds.get(child_bone_entity).ok()?;
    Some(child_bone_world.translation())
}

/// Hinge branch of `draw_joint_anchors`: orange sphere at the anchor,
/// hinge-axis arrow, plus the allowed min/max sweep arc fanning out from the
/// parent frame, with a white arrow for the child's current direction.
fn draw_hinge_shapes(
    gizmos: &mut Gizmos,
    joint_anchor: Vec3,
    child_pose: &Transform,
    parent_pose: &Transform,
    sweep_min: f32,
    sweep_max: f32,
) {
    let hinge_color = Color::srgb(1.0, 0.55, 0.1);
    let live_anchor = joint_anchor;
    gizmos.sphere(Isometry3d::from_translation(live_anchor), 0.03, hinge_color);
    gizmos.line(
        live_anchor,
        parent_pose.transform_point(Vec3::ZERO),
        hinge_color.with_alpha(0.5),
    );
    // Hinge axis arrow (body-local Z) plus the allowed sweep.
    let hinge_axis = child_pose.rotation * Vec3::Z;
    gizmos.arrow(
        live_anchor - hinge_axis * 0.08,
        live_anchor + hinge_axis * 0.08,
        hinge_color,
    );
    // Zero of the sweep is the seated pose, where child and parent frames
    // coincided: min/max rays fan out from the parent frame's X about its Z.
    let hinge_radius = 0.15;
    let parent_hinge_axis = parent_pose.rotation * Vec3::Z;
    let sweep_zero = parent_pose.rotation * Vec3::X;
    let min_ray = Quat::from_axis_angle(parent_hinge_axis, sweep_min) * sweep_zero;
    let max_ray = Quat::from_axis_angle(parent_hinge_axis, sweep_max) * sweep_zero;
    gizmos.line(
        live_anchor,
        live_anchor + min_ray * hinge_radius,
        hinge_color.with_alpha(0.8),
    );
    gizmos.line(
        live_anchor,
        live_anchor + max_ray * hinge_radius,
        hinge_color.with_alpha(0.8),
    );
    let sweep_steps = 16;
    let mut previous_arc_point = live_anchor + min_ray * hinge_radius;
    for sweep_step in 1..=sweep_steps {
        let sweep_fraction = sweep_step as f32 / sweep_steps as f32;
        let sweep_ray = Quat::from_axis_angle(
            parent_hinge_axis,
            sweep_min + (sweep_max - sweep_min) * sweep_fraction,
        ) * sweep_zero;
        let arc_point = live_anchor + sweep_ray * hinge_radius;
        gizmos.line(previous_arc_point, arc_point, hinge_color);
        previous_arc_point = arc_point;
    }
    // Current child X: where the limb points inside the sweep.
    gizmos.arrow(
        live_anchor,
        live_anchor + (child_pose.rotation * Vec3::X) * 0.12,
        Color::WHITE,
    );
}

/// Swing-twist branch of `draw_joint_anchors`: cyan sphere at the anchor,
/// twist arrow along the child's live twist axis, plus a wireframe swing
/// cone opening over the limb in the parent's live frame.
fn draw_swing_twist_shapes(
    gizmos: &mut Gizmos,
    joint_anchor: Vec3,
    child_pose: &Transform,
    parent_pose: &Transform,
    swing_half_angle: f32,
) {
    let joint_color = Color::srgb(0.1, 0.9, 1.0);
    let live_anchor = joint_anchor;
    gizmos.sphere(Isometry3d::from_translation(live_anchor), 0.03, joint_color);
    gizmos.line(
        live_anchor,
        parent_pose.transform_point(Vec3::ZERO),
        joint_color.with_alpha(0.5),
    );
    // Cone axis: parent frame's local Y (the bake's twist axis) aimed at the
    // child body, so the cone opens over the limb it guards.
    let child_center = child_pose.transform_point(Vec3::ZERO);
    let mut cone_axis = parent_pose.rotation * Vec3::Y;
    if cone_axis.dot(child_center - live_anchor) < 0.0 {
        cone_axis = -cone_axis;
    }
    // Twist arrow along the child's live twist axis.
    let mut child_twist = child_pose.rotation * Vec3::Y;
    if child_twist.dot(child_center - live_anchor) < 0.0 {
        child_twist = -child_twist;
    }
    gizmos.arrow(
        live_anchor,
        live_anchor + child_twist * 0.15,
        Color::WHITE,
    );
    // Elliptical rim: clean orthonormal frame off the cone axis, semi-axes
    // from each half-cone angle. Sized by slant (anchor → rim stays
    // `cone_length`), so wide cones read as wider fans, not funnels.
    // Avian's swing limit is a single symmetric cone: both semi-axes use it.
    let cone_length = 0.2;
    let parent_plane_axis = parent_pose.rotation * Vec3::X;
    let rim_center = live_anchor + cone_axis * cone_length * swing_half_angle.cos();
    let rim_u = (parent_plane_axis - cone_axis * parent_plane_axis.dot(cone_axis))
        .normalize_or_zero();
    let rim_v = cone_axis.cross(rim_u).normalize_or_zero();
    let rim_steps = 24;
    let mut previous_rim_point =
        rim_center + rim_u * (swing_half_angle.sin() * cone_length);
    for rim_step in 0..=rim_steps {
        let rim_angle = rim_step as f32 / rim_steps as f32 * std::f32::consts::TAU;
        let rim_point = rim_center
            + rim_u * (rim_angle.cos() * swing_half_angle.sin() * cone_length)
            + rim_v * (rim_angle.sin() * swing_half_angle.sin() * cone_length);
        gizmos.line(previous_rim_point, rim_point, joint_color);
        // Spokes at the four cardinal points tie rim to apex.
        if rim_step % (rim_steps / 4) == 0 {
            gizmos.line(
                live_anchor,
                rim_point,
                joint_color.with_alpha(0.6),
            );
        }
        previous_rim_point = rim_point;
    }
}

/// Live joint position from both bodies' local anchors: each side maps its
/// own local anchor through its live pose, then the midpoint is the
/// constraint position. `FromGlobal` anchors are bind-pose world points (the
/// joint has not stepped yet): they only read correctly before the first
/// step converts them, so the midpoint keeps the gizmo near the joint even
/// in that window.
fn joint_live_anchor(
    parent_anchor: JointAnchor,
    child_anchor: JointAnchor,
    parent_pose: &Transform,
    child_pose: &Transform,
) -> Vec3 {
    let parent_point = match parent_anchor {
        JointAnchor::Local(parent_local) => parent_pose.transform_point(parent_local),
        JointAnchor::FromGlobal(parent_world) => parent_world,
    };
    let child_point = match child_anchor {
        JointAnchor::Local(child_local) => child_pose.transform_point(child_local),
        JointAnchor::FromGlobal(child_world) => child_world,
    };
    (parent_point + child_point) * 0.5
}

/// Renders joint gizmos on top of all geometry. Bevy has no per-draw depth
/// toggle; `depth_bias: -1` pulls every gizmo to the front instead.
fn joint_gizmos_on_top(mut gizmo_store: ResMut<GizmoConfigStore>) {
    let (gizmo_config, _) = gizmo_store.config_mut::<DefaultGizmoConfigGroup>();
    gizmo_config.depth_bias = -1.0;
}
