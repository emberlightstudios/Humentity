//! Makehuman comes with several lower poly proxy meshes.  These
//! can be used for lods with the VisibilityRanges component.
//! We also demonstrate the use of skeleton lod here.
//!
//! NOTE: Overlapping visibility ranges (as used for crossfade dithering between
//! LODs) do NOT work when a depth prepass is enabled. During the overlap the
//! two meshes are drawn at the same depth, and the dithered transition writes
//! inconsistent values to the prepass, producing visible popping/flicker. Keep
//! your LOD visibility ranges non-overlapping when a depth prepass is active.
//!

mod shared;
use bevy::{camera::visibility::VisibilityRange, prelude::*};
use humentity::prelude::*;
use shared::setup_app;

const SHAPE_NAME: &str = "bigboobs";

#[derive(Component, Debug, Default)]
struct CameraDistance(f32);

#[derive(Resource)]
struct RetargetedAnims {
    _clips: Handle<RetargetedAnimationAsset>,
}

fn main() {
    let mut app = setup_app();

    app.add_plugins(BoneDebugPlugin)
        .add_systems(Startup, add_humans)
        .add_systems(
            Update,
            (
                update_camera_distance,
                sync_skeleton_lod_to_visibility,
                sync_reference_lod_state,
                play_idle_animation.run_if(resource_added::<RetargetedAnims>),
            ),
        )
        .run();
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
    morph_targets.insert("gender", 0.0);
    morph_targets.insert("cupsize", 1.0);

    let shape = CharacterMorphShape::new(SHAPE_NAME, morph_targets);

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
    morphs.insert(SHAPE_NAME, 1.);

    let white = materials.add(StandardMaterial::from_color(Color::WHITE));
    let black = materials.add(StandardMaterial::from_color(Color::BLACK));

    let _clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnims { _clips });

    // LOD character with all proxy meshes as children, each with a VisibilityRange
    commands.spawn((
        Name::new("LOD Character"),
        Transform::from_translation(Vec3::new(0., 0., -1.)),
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
                MeshMaterial3d(white),
                VisibilityRange {
                    start_margin: 14.0..15.0,
                    end_margin: 20.0..30.0,
                    use_aabb: false,
                }
            )
        ],
    ));

    // Reference characters showing individual proxy meshes at each LOD level
    for ((proxy, name, x), skeleton_lod) in [
        ((lod0, "basemesh ref", -1.5), 0),
        ((lod1, "proxy4817 ref", -0.5), 0),
        ((lod2, "proxy1605 ref", 0.5), 1),
        ((lod3, "proxy741 ref", 1.5), 2),
    ] {
        commands.spawn((
            Name::new(name),
            Transform::from_translation(Vec3::new(x, 0., 0.)),
            CharacterShape(shape_assets.add(CharacterShapeAsset::new(
                template_handle.clone(),
                morphs.clone(),
            ))),
            HelperVertexPositions::default(),
            InheritedVisibility::default(),
            children![(
                CharacterPart {
                    mesh: proxy,
                    skeleton_lod
                },
                Name::new("mesh"),
                MeshMaterial3d(black.clone())
            )],
        ));
    }
}

/// This will update main main character distance to camera for managing lod state
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

/// Primitive skeleton lod state management based on distance to camera.
/// Writes `SkeletonLodState.active` with each LOD that currently has a mesh in
/// range; the reconcile system disables the bone sub-trees removed by every
/// active LOD.
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

/// Reference characters each show a single proxy mesh at a fixed skeleton LOD.
/// The shared `enable_first_skeleton_on_ready` system would set them all to LOD 0,
/// so this keeps their `SkeletonLodState` in sync with their part's `skeleton_lod`
/// to disable the bone sub-trees relevant to that LOD in the background.
fn sync_reference_lod_state(
    parts: Query<(&CharacterPart, &ChildOf), Without<VisibilityRange>>,
    characters: Query<
        (Entity, Option<&SkeletonLodState>),
        (With<SkeletonsReady>, Without<CameraDistance>),
    >,
    mut commands: Commands,
) {
    for (entity, existing) in &characters {
        let mut active = [false; MAX_LODS];
        let mut found = false;
        for (part, child_of) in &parts {
            if child_of.parent() == entity {
                active[part.skeleton_lod.min(MAX_LODS - 1)] = true;
                found = true;
            }
        }
        if !found {
            continue;
        }
        if existing.is_none_or(|s| s.active != active) {
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
