//! Stress test: spawns an NxN grid of animated LOD characters with avian ragdoll physics.
//! SPACE toggles ragdoll on ALL characters simultaneously.

mod shared;
use ahash::AHashMap;
use avian3d::prelude::*;
use bevy::{
    camera::visibility::VisibilityRange,
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
};
use humentity::prelude::*;
use shared::setup_app;

const N: usize = 10;

const RAGDOLL_LAYER: u32 = 1 << 3;
const WORLD_LAYER: u32 = 1 << 0;
const CHARACTER_LAYER: u32 = 1 << 1;

#[derive(Component, Debug, Default)]
struct CameraDistance(f32);

#[derive(Component)]
struct FpsText;

#[derive(Component)]
struct AnimCtrl(AnimationNodeIndex);

#[derive(Resource)]
struct RetargetedAnims {
    _clips: Handle<RetargetedAnimationAsset>,
}

#[derive(Component)]
struct RagdollSleepTimer(Timer);

impl Default for RagdollSleepTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(2., TimerMode::Once))
    }
}

fn main() {
    let mut app = setup_app();

    app.add_plugins((
        PhysicsPlugins::default(),
        FrameTimeDiagnosticsPlugin::default(),
        PhysicsDebugPlugin::default(),
    ))
    .insert_resource(SubstepCount(10))
    .add_systems(Startup, (physics_floor, spawn_ui))
    .add_observer(add_humans)
    .add_systems(
        Update,
        (
            update_camera_distance,
            sync_skeleton_lod_to_visibility,
            setup_graph.run_if(resource_added::<RetargetedAnims>),
            start_clip,
            toggle,
            sleep_ragdoll,
            update_fps_text,
        ),
    )
    .run();
}

fn physics_floor(mut commands: Commands) {
    commands.spawn((
        Collider::cuboid(60.0, 0.1, 60.0),
        Friction::new(0.5),
        Restitution::new(0.1),
        RigidBody::Static,
        Transform::IDENTITY,
    ));
}

fn spawn_ui(mut commands: Commands) {
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
    commands.spawn((
        Text::new("SPACE: toggle ragdoll on all characters"),
        TextFont::from_font_size(24.0),
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.),
            right: Val::Px(12.),
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
    _trigger: On<MorphsReady>,
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
                InheritedVisibility::default(),
                CameraDistance::default(),
                AnimationPlayer::default(),
                CharacterRagdoll::None,
                CharacterColliders::new(None),
                RagdollCollisionLayers(CollisionLayers::new(
                    RAGDOLL_LAYER,
                    WORLD_LAYER | CHARACTER_LAYER | RAGDOLL_LAYER,
                )),
                RagdollMobility(1.0),
                RagdollDensity(10.0),
                RagdollDamping::default(),
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

fn sync_skeleton_lod_to_visibility(
    characters: Query<(Entity, &CameraDistance, &SkeletonLodMap), With<SkeletonsReady>>,
    parts: Query<(&VisibilityRange, &CharacterPart, &ChildOf)>,
    mut prev: Local<AHashMap<(Entity, usize), bool>>,
    mut commands: Commands,
) {
    for (entity, cam_dist, lod_map) in &characters {
        for &lod in lod_map.0.keys() {
            let in_range = parts.iter().any(|(range, cp, child_of)| {
                child_of.parent() == entity
                    && cp.skeleton_lod == lod
                    && cam_dist.0 >= range.start_margin.start
                    && cam_dist.0 < range.end_margin.end
            });

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

fn setup_graph(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    retargeted_clips: Res<Assets<RetargetedAnimationAsset>>,
    mut character_player: Query<(Entity, &mut AnimationPlayer), Without<AnimationGraphHandle>>,
) {
    let Some((_id, clips_map)) = retargeted_clips.iter().next() else {
        return;
    };
    let Some(clip_handle) = clips_map.clips.get("Idle-loop") else {
        return;
    };
    for (entity, _player) in &mut character_player {
        let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());
        commands
            .entity(entity)
            .insert((AnimCtrl(index), AnimationGraphHandle(graphs.add(graph))));
    }
}

fn start_clip(mut characters: Query<(&mut AnimationPlayer, &AnimCtrl), Added<AnimCtrl>>) {
    for (mut player, ctrl) in &mut characters {
        player.play(ctrl.0).repeat();
    }
}

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut characters: Query<(
        Entity,
        &mut CharacterRagdoll,
        &CharacterColliders,
        &mut AnimationPlayer,
        Option<&AnimCtrl>,
    )>,
    mut collider_data: Query<&mut LinearVelocity>,
) {
    if !input.just_pressed(KeyCode::Space) {
        return;
    }

    for (entity, mut ragdoll, colliders, mut player, ctrl) in &mut characters {
        let clip_index = ctrl.map_or_else(|| AnimationNodeIndex::new(0), |c| c.0);

        match &*ragdoll {
            CharacterRagdoll::Full => {
                *ragdoll = CharacterRagdoll::None;

                player.stop(clip_index);
                player.play(clip_index).repeat();
            }
            _ => {
                *ragdoll = CharacterRagdoll::Full;
                player.stop(clip_index);

                for (bone, &collider_entity) in colliders.collider_entities.iter() {
                    if let Ok(mut vel) = collider_data.get_mut(collider_entity) {
                        let i = *bone as u32;
                        let e = entity.to_bits() as u32;
                        let t = (time.elapsed_secs() * 100.0) as u32;
                        let mut h = i
                            .wrapping_mul(0x9e3779b9)
                            .wrapping_add(e.wrapping_mul(0x85ebca6b))
                            .wrapping_add(t.wrapping_mul(0xc2b2ae35));
                        h ^= h >> 16;
                        h = h.wrapping_mul(0x85ebca6b);
                        h ^= h >> 13;
                        h = h.wrapping_mul(0xc2b2ae35);
                        h ^= h >> 16;
                        let hf = h as f32 / u32::MAX as f32;
                        let dir = Vec3::new(
                            ((hf * 43_758.547).fract() - 0.5) * 2.0,
                            ((hf * 27_118.313).fract()) * 0.5,
                            ((hf * 30_903.55).fract() - 0.5) * 2.0,
                        )
                        .normalize_or_zero();
                        vel.0 += dir * 2.0;
                    }
                }
            }
        }
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
