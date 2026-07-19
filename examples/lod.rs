//! Makehuman comes with several lower poly proxy meshes.  These
//! can be used for lods with the VisibilityRanges component.
//! We also demonstrate the use of skeleton lod here.

mod shared;
use ahash::AHashSet;
use bevy::{camera::visibility::VisibilityRange, prelude::*};
use humentity::prelude::*;
use shared::setup_app;
use std::collections::HashMap;

const SHAPE_NAME: &str = "bigboobs";

#[derive(Component, Debug, Default)]
struct CameraDistance(f32);

#[derive(Component)]
struct AnimIndex(AnimationNodeIndex);

fn main() {
    let mut app = setup_app();

    app.add_plugins(BoneDebugPlugin)
        .add_observer(add_humans)
        .add_observer(on_skeletons_ready)
        .add_systems(
            Update,
            (
                update_camera_distance,
                sync_skeleton_lod_to_visibility,
                play_idle_animation.run_if(resource_added::<RetargetedAnims>),
            ),
        )
        .run();
}

fn on_skeletons_ready(
    trigger: On<Add, SkeletonsReady>,
    lod_map_query: Query<&SkeletonLodMap>,
    filter_query: Query<Option<&SkeletonLodFilter>>,
    mut commands: Commands,
) {
    let entity = trigger.entity;
    let Ok(lod_map) = lod_map_query.get(entity) else {
        return;
    };
    let allowed = filter_query.get(entity).ok().flatten();
    for &lod in lod_map.0.keys() {
        if let Some(SkeletonLodFilter(Some(set))) = allowed {
            if !set.contains(&lod) {
                continue;
            }
        }
        commands.trigger(EnableSkeletonLod {
            character: entity,
            lod,
        });
    }
}

#[derive(Resource)]
struct RetargetedAnims {
    _clips: Handle<RetargetedAnimationAsset>,
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
            .insert((AnimIndex(index), AnimationGraphHandle(graphs.add(graph))));
        player.play(index).repeat();
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

/// Primitive distance management for skeleton lod
fn sync_skeleton_lod_to_visibility(
    characters: Query<(Entity, &CameraDistance, &SkeletonLodMap), With<SkeletonsReady>>,
    parts: Query<(&VisibilityRange, &CharacterPart, &ChildOf)>,
    mut prev: Local<HashMap<(Entity, usize), bool>>,
    mut commands: Commands,
) {
    for (entity, cam_dist, lod_map) in &characters {
        for &lod in lod_map.0.keys() {
            let Some((range, _, _)) = parts
                .iter()
                .find(|(_, cp, child_of)| child_of.parent() == entity && cp.lod == lod)
            else {
                continue;
            };

            let in_range =
                cam_dist.0 >= range.start_margin.start && cam_dist.0 < range.end_margin.end;

            let key = (entity, lod);
            let was_in_range = prev.get(&key).copied().unwrap_or(!in_range);

            if was_in_range && !in_range {
                commands.trigger(DisableSkeletonLod {
                    character: entity,
                    lod,
                });
            } else if !was_in_range && in_range {
                commands.trigger(EnableSkeletonLod {
                    character: entity,
                    lod,
                });
            }

            prev.insert(key, in_range);
        }
    }
}

fn add_humans(
    _trigger: On<MorphsReady>,
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
        lod: 0,
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod1.clone(),
        template_handle: template_handle.clone(),
        lod: 1,
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod2.clone(),
        template_handle: template_handle.clone(),
        lod: 2,
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod3.clone(),
        template_handle: template_handle.clone(),
        lod: 3,
    });

    let mut morphs = MorphTargets::default();
    morphs.insert(SHAPE_NAME, 1.);

    let white = materials.add(StandardMaterial::from_color(Color::WHITE));
    let black = materials.add(StandardMaterial::from_color(Color::BLACK));

    let _clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnims { _clips: _clips });

    // LOD character with all proxy meshes as children, each with a VisibilityRange
    commands.spawn((
        Name::new("LOD Character"),
        Transform::from_translation(Vec3::new(0., 0., -1.)),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(
            template_handle.clone(),
            morphs.clone(),
        ))),
        InheritedVisibility::default(),
        CameraDistance::default(),
        AnimationPlayer::default(),
        children![
            (
                CharacterPart {
                    mesh: lod0.clone(),
                    lod: 0
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
                    lod: 1
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
                    lod: 2
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
                    lod: 3
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
    for ((proxy, name, x), lod) in [
        ((lod0, "basemesh ref", -1.5), 0),
        ((lod1, "proxy4817 ref", -0.5), 1),
        ((lod2, "proxy1605 ref", 0.5), 2),
        ((lod3, "proxy741 ref", 1.5), 3),
    ] {
        commands.spawn((
            Name::new(name),
            Transform::from_translation(Vec3::new(x, 0., 0.)),
            CharacterShape(shape_assets.add(CharacterShapeAsset::new(
                template_handle.clone(),
                morphs.clone(),
            ))),
            InheritedVisibility::default(),
            SkeletonLodFilter(Some(AHashSet::from([lod]))),
            children![(
                CharacterPart { mesh: proxy, lod },
                Name::new("mesh"),
                MeshMaterial3d(black.clone())
            )],
        ));
    }
}
