//! Stress test: spawns an NxN grid of LOD characters to measure performance.

mod shared;
use bevy::{
    camera::visibility::VisibilityRange,
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
};
use humentity::prelude::*;
use shared::setup_app;

// On my machine I can accomodate this many animated characters while staying near 60fps.
// This is an improvement after implementing skeleton lod as I think transform propagation
// was one of the biggest bottlenecks.
// 24*24 = 576 characters
// Of course further optimization (e.g. frs)
const N: usize = 24;

#[derive(Component, Debug, Default)]
struct CameraDistance(f32);

#[derive(Component)]
struct FpsText;

#[derive(Resource)]
struct RetargetedAnims {
    _clips: Handle<RetargetedAnimationAsset>,
}

fn main() {
    let mut app = setup_app();

    app.add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_systems(
            Update,
            add_humans.run_if(resource_added::<HumentityAssetsReady>),
        )
        .add_systems(Startup, setup_fps_text)
        .add_systems(
            Update,
            (
                update_camera_distance,
                sync_skeleton_lod_to_visibility,
                play_idle_animation.run_if(resource_added::<RetargetedAnims>),
                update_fps_text,
            ),
        )
        .run();
}

fn setup_fps_text(mut commands: Commands) {
    commands.spawn((
        FpsText,
        Text::new("FPS: --"),
        TextFont {
            font_size: FontSize::Px(30.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(5.0),
            left: Val::Px(5.0),
            ..default()
        },
    ));
}

fn update_fps_text(
    diagnostics: Res<DiagnosticsStore>,
    characters: Query<Entity, With<CharacterShape>>,
    mut query: Query<&mut Text, With<FpsText>>,
) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    let count = characters.iter().count();

    for mut text in &mut query {
        **text = format!("FPS: {fps:.1}  Characters: {count}");
    }
}

fn add_humans(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 1.0);

    let shape = CharacterMorphShape::new("bigboobs", morph_targets);
    let template_handle = template_assets.add(CharacterTemplate::new([shape]));

    let lod0 = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");
    let lod1 = asset_server.load::<MhcloAsset>("proxymeshes/proxy4817/proxy4817.proxy");
    let lod2 = asset_server.load::<MhcloAsset>("proxymeshes/proxy1605/proxy1605.proxy");
    let lod3 = asset_server.load::<MhcloAsset>("proxymeshes/proxy741/proxy741.proxy");

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod0.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: 0,
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod1.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: 0,
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod2.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: 1,
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod3.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: 2,
    });

    let mut morphs = MorphTargets::default();
    morphs.insert("bigboobs", 1.);

    let white = materials.add(StandardMaterial::from_color(Color::WHITE));

    let _clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnims { _clips });

    let half = N as f32 / 2.0;
    for row in 0..N {
        for col in 0..N {
            let x = col as f32 - half;
            let z = row as f32 - half;
            commands.spawn((
                Name::new(format!("Char {row},{col}")),
                Transform::from_translation(Vec3::new(x, 0., z)),
                CharacterShape(shape_assets.add(CharacterShapeAsset::new(
                    template_handle.clone(),
                    morphs.clone(),
                ))),
                HelperVertexPositions::default(),
                InheritedVisibility::default(),
                CameraDistance::default(),
                AnimationPlayer::default(),
                children![
                    (
                        CharacterPart {
                            mesh: lod0.clone(),
                            skeleton_lod: 0
                        },
                        Name::new("basemesh"),
                        MeshMaterial3d(white.clone()),
                        VisibilityRange {
                            start_margin: 0.0..0.0,
                            end_margin: 2.0..3.0,
                            use_aabb: false,
                        }
                    ),
                    (
                        CharacterPart {
                            mesh: lod1.clone(),
                            skeleton_lod: 0
                        },
                        Name::new("proxy4817"),
                        MeshMaterial3d(white.clone()),
                        VisibilityRange {
                            start_margin: 2.0..3.0,
                            end_margin: 7.0..8.0,
                            use_aabb: false,
                        }
                    ),
                    (
                        CharacterPart {
                            mesh: lod2.clone(),
                            skeleton_lod: 1
                        },
                        Name::new("proxy1605"),
                        MeshMaterial3d(white.clone()),
                        VisibilityRange {
                            start_margin: 7.0..8.0,
                            end_margin: 14.0..15.0,
                            use_aabb: false,
                        }
                    ),
                    (
                        CharacterPart {
                            mesh: lod3.clone(),
                            skeleton_lod: 2
                        },
                        Name::new("proxy741"),
                        MeshMaterial3d(white.clone()),
                        VisibilityRange {
                            start_margin: 14.0..15.0,
                            end_margin: 20.0..30.0,
                            use_aabb: false,
                        }
                    )
                ],
            ));
        }
    }
}

fn update_camera_distance(
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut entities: Query<(&GlobalTransform, &mut CameraDistance)>,
) {
    let Ok(cam_gt) = cameras.single() else {
        return;
    };
    let cam_pos = cam_gt.translation();
    for (gt, mut dist) in &mut entities {
        dist.0 = gt.translation().distance(cam_pos);
    }
}

/// How far (in world units) beyond a mesh part's visibility range its skeleton
/// LOD is kept active.
///
/// Skeleton LOD enables/disables bone sub-trees, but a re-enabled bone's frozen
/// `GlobalTransform` isn't corrected until a frame or more later: Bevy resets a
/// disabled bone's `GlobalTransform` to `Transform::IDENTITY`, and the skinned
/// mesh extraction only re-samples a joint after transform propagation
/// recomputes it. If we flipped the skeleton LOD exactly when a mesh became
/// visible, that new part would render for at least one frame with stale
/// `IDENTITY` joint matrices, detaching the mesh from its bones.
///
/// By activating a part's skeleton LOD a little *before* it comes into range,
/// the joints get a frame or two to settle before the part is actually drawn.
/// We expand both ends of the range so joints are also released late while
/// receding; over-enabling is harmless (each part only skins its own joint
/// subset), whereas under-enabling is what causes the rendering artifacts.
const SKELETON_LOD_ACTIVATION_BUFFER: f32 = 1.0;

fn sync_skeleton_lod_to_visibility(
    characters: Query<(Entity, &CameraDistance, Option<&SkeletonLodState>), With<SkeletonsReady>>,
    parts: Query<(&VisibilityRange, &CharacterPart, &ChildOf)>,
    mut commands: Commands,
) {
    for (entity, cam_dist, existing) in &characters {
        let mut active = [false; MAX_LODS];
        for (lod, active_lod) in active.iter_mut().enumerate() {
            *active_lod = parts.iter().any(|(range, cp, child_of)| {
                child_of.parent() == entity
                    && cp.skeleton_lod == lod
                    && cam_dist.0 >= range.start_margin.start - SKELETON_LOD_ACTIVATION_BUFFER
                    && cam_dist.0 < range.end_margin.end + SKELETON_LOD_ACTIVATION_BUFFER
            });
        }

        let changed = existing.is_none_or(|s| s.active != active);
        if changed {
            commands.entity(entity).insert(SkeletonLodState { active });
        }
    }
}

fn play_idle_animation(
    clips: Res<Assets<RetargetedAnimationAsset>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut characters: Query<
        (Entity, &mut AnimationPlayer),
        (Without<AnimationGraphHandle>, With<CharacterShape>),
    >,
    mut commands: Commands,
) {
    let Some((_id, clips_map)) = clips.iter().next() else {
        return;
    };
    let Some(clip_handle) = clips_map.clips.get("Idle-loop") else {
        return;
    };
    for (entity, mut player) in &mut characters {
        let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());
        commands
            .entity(entity)
            .insert(AnimationGraphHandle(graphs.add(graph)));
        player.play(index).repeat();
    }
}
