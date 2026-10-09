mod shared;

use bevy::math::Isometry3d;
use bevy::prelude::*;
use bevy_jolt::{RagdollJoint, prelude::*};
use humentity::prelude::*;
use shared::setup_app;

const RAGDOLL_TEAM: u16 = 3;
const WORLD_TEAM: u16 = 0;
const CHARACTER_TEAM: u16 = 1;

fn main() {
    let mut app = setup_app();

    // Two teams for the demo table: world (0) and ragdoll (3). Everything
    // collides with everything by default; carve out nothing here.
    let collision_layers = CollisionLayers::new(4);

    app.add_plugins((
        JoltPlugin::new().with_collision_layers(collision_layers),
        JoltDebugPlugin,
    ))
    .add_systems(Startup, (floor, spawn_ui, joint_gizmos_on_top))
    .add_systems(
        Update,
        (
            add_human.run_if(resource_exists::<HumentityAssetsReady>),
            toggle,
            setup_graph,
            start_clip,
            dump_joint_angles,
            draw_joint_anchors,
        ),
    )
    .run();
}

/// Prints every ragdoll part's live pose + sleep state. Press J while the
/// ragdoll is active: parts that never sleep point at unsettled joints.
/// Example line: `knee.L live y=0.412 awake`.
fn dump_joint_angles(
    input: Res<ButtonInput<KeyCode>>,
    transform_query: Query<&Transform>,
    sleeping_query: Query<&JoltSleeping>,
    character_query: Query<&CharacterColliders>,
) {
    if !input.just_pressed(KeyCode::KeyJ) {
        return;
    }
    let Ok(colliders) = character_query.single() else {
        return;
    };
    for (bone, &part_entity) in colliders.collider_entities.iter() {
        let Ok(part_transform) = transform_query.get(part_entity) else {
            continue;
        };
        let sleep = sleeping_query
            .get(part_entity)
            .map(|sleeping| if sleeping.sleeping { "asleep" } else { "awake" })
            .unwrap_or("no-state");
        println!(
            "{bone:?} live y={:.3} {}",
            part_transform.translation.y, sleep,
        );
    }
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
    mut collider_data: Query<&mut JoltLinearVelocity>,
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
                if let Ok(mut velocity) = collider_data.get_mut(collider_entity) {
                    let bone_index = *bone as usize;
                    let elapsed = time.elapsed_secs();
                    let seed = (elapsed * 100.0).floor() / 100.0;
                    let hash = ((bone_index as f32 + seed) * 0x9e3779b9u32 as f32).sin();
                    let dir = Vec3::new(
                        ((hash * 43_758.547).fract() - 0.5) * 2.0,
                        ((hash * 27_118.313).fract()) * 0.5,
                        ((hash * 30_903.55).fract() - 0.5) * 2.0,
                    )
                    .normalize_or_zero();
                    velocity.linear_velocity += dir * 2.0;
                }
            }
        }
    }
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("SPACE: toggle full ragdoll (jolt) | J: dump joint angles"),
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

fn floor(mut commands: Commands) {
    commands.spawn((
        JoltShape::box_shape(Vec3::new(100.0, 0.1, 100.0)),
        JoltBody::fixed(WORLD_TEAM),
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

/// Draws one sphere per joint at the live constraint position, rendered on
/// top of everything (`depth_bias: -1`). Hinges draw orange, swing-twist
/// joints cyan, so a glance shows which constraint type guards each pivot.
/// The sphere follows the child part: the baked anchor is bind-pose, so each
/// frame re-expresses the anchor in the child part's current frame via that
/// part's synced `GlobalTransform`.
///
/// Swing-twist joints also get a wireframe swing cone plus a twist-axis
/// arrow, both in the *parent* side's live frame: the bake derives each
/// side's joint frame from its own seated rotation (twist = local Y, plane
/// = local X), so if a cone points somewhere odd, that side's bake frame is
/// the suspect. Hinges get their axis arrow plus the min/max sweep arc.
fn draw_joint_anchors(
    ragdolls: Query<(&JoltRagdoll, &JoltRagdollParts)>,
    part_poses: Query<&GlobalTransform>,
    mut gizmos: Gizmos,
) {
    for (ragdoll_spec, baked_parts) in &ragdolls {
        for (part_index, part_spec) in ragdoll_spec.parts.iter().enumerate() {
            let Some(parent_index) = part_spec.parent_part else {
                continue;
            };
            let Some(&child_entity) = baked_parts.part_entities.get(part_index) else {
                continue;
            };
            let Some(&parent_entity) = baked_parts.part_entities.get(parent_index) else {
                continue;
            };
            let (Ok(child_pose), Ok(parent_pose)) =
                (part_poses.get(child_entity), part_poses.get(parent_entity))
            else {
                continue;
            };
            // Anchor is bind-pose world; re-seat it in the child's live frame.
            // `child_bind_to_world` maps bind-local to bind-world, so its
            // inverse takes the anchor back to bind-local, then the live
            // child pose carries it to the current joint position.
            let child_bind_to_world = Transform {
                translation: part_spec.part_position,
                rotation: part_spec.part_rotation,
                scale: Vec3::ONE,
            };
            let anchor_bind_local = child_bind_to_world
                .compute_affine()
                .inverse()
                .transform_point3(match part_spec.joint {
                    RagdollJoint::Hinge { anchor, .. } => anchor,
                    RagdollJoint::SwingTwist { anchor, .. } => anchor,
                });
            let live_anchor = child_pose.transform_point(anchor_bind_local);
            let child_live_rotation = child_pose.rotation();
            let parent_live_rotation = parent_pose.rotation();
            let parent_spec = &ragdoll_spec.parts[parent_index];
            // Live joint frames mirror the bake: each side's frame is its own
            // seated rotation carrying body-local axes (twist = Y, plane = X
            // for swing-twist; hinge about Z). The live pose rotation stands
            // in for the seated one, so these arrows track the bodies.
            let parent_twist_axis = parent_live_rotation * Vec3::Y;
            let parent_plane_axis = parent_live_rotation * Vec3::X;
            let _ = parent_spec;
            match part_spec.joint {
                RagdollJoint::Hinge { min, max, .. } => {
                    let joint_color = Color::srgb(1.0, 0.55, 0.1);
                    gizmos.sphere(Isometry3d::from_translation(live_anchor), 0.03, joint_color);
                    gizmos.line(
                        live_anchor,
                        parent_pose.transform_point(Vec3::ZERO),
                        joint_color.with_alpha(0.5),
                    );
                    // Hinge axis arrow (body-local Z) plus the allowed sweep.
                    let hinge_axis = child_live_rotation * Vec3::Z;
                    gizmos.arrow(
                        live_anchor - hinge_axis * 0.08,
                        live_anchor + hinge_axis * 0.08,
                        joint_color,
                    );
                    // Zero of the sweep is the seated pose, where child and
                    // parent frames coincided: min/max rays fan out from the
                    // parent frame's X about its Z.
                    let hinge_radius = 0.15;
                    let parent_hinge_axis = parent_live_rotation * Vec3::Z;
                    let sweep_zero = parent_live_rotation * Vec3::X;
                    let min_ray =
                        Quat::from_axis_angle(parent_hinge_axis, min) * sweep_zero;
                    let max_ray =
                        Quat::from_axis_angle(parent_hinge_axis, max) * sweep_zero;
                    gizmos.line(
                        live_anchor,
                        live_anchor + min_ray * hinge_radius,
                        joint_color.with_alpha(0.8),
                    );
                    gizmos.line(
                        live_anchor,
                        live_anchor + max_ray * hinge_radius,
                        joint_color.with_alpha(0.8),
                    );
                    let sweep_steps = 16;
                    let mut previous_point = live_anchor + min_ray * hinge_radius;
                    for step in 1..=sweep_steps {
                        let fraction = step as f32 / sweep_steps as f32;
                        let ray = Quat::from_axis_angle(
                            parent_hinge_axis,
                            min + (max - min) * fraction,
                        ) * sweep_zero;
                        let arc_point = live_anchor + ray * hinge_radius;
                        gizmos.line(previous_point, arc_point, joint_color);
                        previous_point = arc_point;
                    }
                    // Current child X: where the limb points inside the sweep.
                    gizmos.arrow(
                        live_anchor,
                        live_anchor + (child_live_rotation * Vec3::X) * 0.12,
                        Color::WHITE,
                    );
                }
                RagdollJoint::SwingTwist {
                    normal_half_cone,
                    plane_half_cone,
                    ..
                } => {
                    let joint_color = Color::srgb(0.1, 0.9, 1.0);
                    gizmos.sphere(Isometry3d::from_translation(live_anchor), 0.03, joint_color);
                    gizmos.line(
                        live_anchor,
                        parent_pose.transform_point(Vec3::ZERO),
                        joint_color.with_alpha(0.5),
                    );
                    // Cone axis: parent twist axis aimed at the child body, so
                    // the cone opens over the limb it guards.
                    let child_center = child_pose.transform_point(Vec3::ZERO);
                    let mut cone_axis = parent_twist_axis;
                    if cone_axis.dot(child_center - live_anchor) < 0.0 {
                        cone_axis = -cone_axis;
                    }
                    // Twist arrow along the child's live twist axis.
                    let mut child_twist = child_live_rotation * Vec3::Y;
                    if child_twist.dot(child_center - live_anchor) < 0.0 {
                        child_twist = -child_twist;
                    }
                    gizmos.arrow(
                        live_anchor,
                        live_anchor + child_twist * 0.15,
                        Color::WHITE,
                    );
                    // Elliptical rim: clean orthonormal frame off the cone
                    // axis, semi-axes from each half-cone angle.
                    let cone_length = 0.2;
                    let rim_center = live_anchor + cone_axis * cone_length;
                    let rim_u = (parent_plane_axis - cone_axis * parent_plane_axis.dot(cone_axis))
                        .normalize_or_zero();
                    let rim_v = cone_axis.cross(rim_u).normalize_or_zero();
                    let rim_steps = 24;
                    let mut previous_point =
                        rim_center + rim_u * (normal_half_cone.tan() * cone_length);
                    for step in 0..=rim_steps {
                        let angle = step as f32 / rim_steps as f32 * std::f32::consts::TAU;
                        let rim_point = rim_center
                            + rim_u * (angle.cos() * normal_half_cone.tan() * cone_length)
                            + rim_v * (angle.sin() * plane_half_cone.tan() * cone_length);
                        gizmos.line(previous_point, rim_point, joint_color);
                        // Spokes at the four cardinal points tie rim to apex.
                        if step % (rim_steps / 4) == 0 {
                            gizmos.line(
                                live_anchor,
                                rim_point,
                                joint_color.with_alpha(0.6),
                            );
                        }
                        previous_point = rim_point;
                    }
                }
            }
        }
    }
}

/// Renders joint gizmos on top of all geometry. Bevy has no per-draw depth
/// toggle; `depth_bias: -1` pulls every gizmo to the front instead.
fn joint_gizmos_on_top(mut gizmo_store: ResMut<GizmoConfigStore>) {
    let (gizmo_config, _) = gizmo_store.config_mut::<DefaultGizmoConfigGroup>();
    gizmo_config.depth_bias = -1.0;
}
