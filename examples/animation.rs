//! Animation retargeting showcase: three babies side by side, one per path.
//! These define the CPU posing pipeline only.  GPU posing is separate.
//!
//! All clips are authored on the reference rig (unmorphed base mesh). The baby
//! shape rescales bones, so reference-proportioned translations must be fixed
//! somewhere. Each baby fixes them in a different place, and the marker on
//! each character decides which system (if any) runs for it:
//!
//! 1) Left — `root-only`, the old loader path. The custom
//!    [`RetargetedAnimationAssetLoader`] keeps rotation + scale on every bone
//!    but translation tracks on the root bone only; non-root translations are
//!    dropped at import. [`RootOnlyRetargeting`] on the character runs only
//!    `rescale_root_bone_translation` (root Y rescale + XZ zero). Cheapest per
//!    frame and correct for rotation-driven clips like locomotion and idle.
//!    Drawback: any motion that lives in bone translations (hips sway,
//!    stretchy/animated-length bones) is gone for good — the curves no longer
//!    exist.  You may notice slight skating with the reference clip due to hip
//!    sway being removed from the clip.
//!
//! 2) Middle — `dynamic`, the new live path. No custom loader: Bevy's built-in
//!    glTF clip loader keeps every track untouched, and
//!    [`DynamicRetargeting`] on the character runs `rescale_dynamic_retargeting`,
//!    which fixes root (Y + XZ, same math as root-only) plus every other bone
//!    (length ratio + direction fix from the fit cache) per frame. Handles any
//!    clip on any shape, including shapes that change at runtime. Drawback: a
//!    per-frame pass over every corrected bone on every animated character —
//!    the most expensive of the three, about 20% fps drop in my stress tests.
//!
//! 3) Right — `baked`, the new import-time path. No marker, no system: the
//!    [`ShapeBakedAnimationAssetLoader`] rewrites every translation track
//!    (root XZ zero + Y rescale, plus the same per-bone ratio/direction math
//!    as dynamic) into the curves once, for this baby shape. Zero per-frame
//!    cost and exact for that shape. Drawbacks: one baked asset per shape
//!    (memory × shapes), and playing it on any other shape is wrong.  Will not work
//!    well with a character who is a blend of different macro shapes.  Needs
//!    corrections up front via [`shape_baked_corrections_for_shape`] passed as
//!    loader settings with `load_builder().with_settings(...)`.  Use if you have
//!    a small fixed number of shape archetypes.  Memory usage scales with
//!    num shapes * num clips.
//!
//! A raw GLB scene spawns behind as the unretargeted reference. The baked baby
//! needs the shape's corrections before its clip can load: this example
//! derives them from `CharacterShapeAsset` + `CharacterTemplate` + `BaseMesh`
//! + `MakeHumanMorphs` + `RigData` + `VertexGroups`, retrying until the core
//! assets are ready.
//!
//! Important notes:
//!  - AnimationTargetId matching requires you to leave the base object name as its
//!    default from blender. This is "Human.rig" after you add a rig. Do not change it.
//!  - Retargeting assumes that all animation clips are authored on a humanoid with the
//!    shape of the base mesh with no morphs applied. Remove all morphs from your human
//!    before authoring animation clips.

mod shared;

use std::f32::consts::PI;

use bevy::{prelude::*, world_serialization::WorldInstanceReady};
use humentity::prelude::*;
use shared::setup_app;

const BABY: &str = "baby";

fn main() {
    let mut app = setup_app();

    app.add_systems(
        Update,
        add_humans.run_if(resource_exists::<HumentityAssetsReady>),
    )
    .add_systems(Update, (read_baked_bytes, bake_baked_clip, play_graph, add_graph))
    .run();
}

// Hold handle refs to keep the clip assets alive. One entry per path: each
// baby plays a different clip asset, and its marker decides which fixup (if
// any) runs for it. `baked_shape` remembers the baked baby's shape so the
// corrections can be derived on a later frame once morphs are ready.
#[derive(Resource, Default)]
struct RetargetedAnimations {
    root_only_clips: Option<Handle<RetargetedAnimationAsset>>,
    dynamic_clip: Option<Handle<AnimationClip>>,
    baked_clip: Option<Handle<AnimationClip>>,
    baked_shape: Option<Handle<CharacterShapeAsset>>,
    baked_bytes: Option<Vec<u8>>,
    baked_logged: bool,
}

#[derive(Component, Clone)]
struct AnimationIndex(AnimationNodeIndex);

/// Which baby a graph belongs to. `add_graph` matches each character against
/// this so each baby plays its own clip asset with its own marker-driven fixup.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ShowcaseBaby {
    RootOnly,
    Dynamic,
    Baked,
}

fn add_humans(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut templates: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
    animations: Option<ResMut<RetargetedAnimations>>,
    mut done: Local<bool>,
) {
    let Some(mut animations) = animations else {
        commands.insert_resource(RetargetedAnimations::default());
        return;
    };
    if *done {
        return;
    }
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("age", 0.);

    let template_handle = templates.add(CharacterTemplate::new([CharacterMorphShape::new(
        BABY,
        morph_targets,
    )]));

    // Spawn the raw GLB animation scene for comparison, includes basemesh+helpers
    let clip_handle =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("animation/idle.glb"));
    let (graph, index) = AnimationGraph::from_clip(clip_handle);
    let graph_handle = graphs.add(graph);

    commands
        .spawn((
            WorldAssetRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset("animation/idle.glb")),
            ),
            Transform::from_translation(Vec3::new(0., 0., 3.))
                .with_rotation(Quat::from_rotation_y(PI)),
            AnimationIndex(index),
            AnimationGraphHandle(graph_handle),
        ))
        .observe(on_gltf_scene_ready);

    // Create the morphable base mesh (one part asset, three characters).
    let basemesh_part = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh_part.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: MeshBuildLod::Cpu(0),
    });

    // Spawn three babies side by side. Markers decide the fixup: left owns
    // root only, middle owns root + bones, right owns nothing (baked).
    let mut baby_morphs = MorphTargets::default();
    baby_morphs.insert(BABY, 1.);
    let root_only_shape = shape_assets.add(CharacterShapeAsset::new(
        template_handle.clone(),
        baby_morphs.clone(),
    ));
    let dynamic_shape = shape_assets.add(CharacterShapeAsset::new(
        template_handle.clone(),
        baby_morphs.clone(),
    ));
    let baked_shape_handle =
        shape_assets.add(CharacterShapeAsset::new(template_handle, baby_morphs));
    commands.spawn((
        Transform::from_translation(Vec3::new(-2., 0., 0.)),
        Name::new("RootOnly"),
        InheritedVisibility::default(),
        AnimationPlayer::default(),
        CharacterShape(root_only_shape),
        RootOnlyRetargeting,
        ShowcaseBaby::RootOnly,
        HelperVertexPositions::default(),
        children![(
            Name::new("Mesh"),
            CharacterPart {
                mesh: basemesh_part.clone(),
                skeleton_lod: 0
            },
        )],
    ));
    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        Name::new("Dynamic"),
        InheritedVisibility::default(),
        AnimationPlayer::default(),
        CharacterShape(dynamic_shape),
        DynamicRetargeting,
        ShowcaseBaby::Dynamic,
        HelperVertexPositions::default(),
        children![(
            Name::new("Mesh"),
            CharacterPart {
                mesh: basemesh_part.clone(),
                skeleton_lod: 0
            },
        )],
    ));
    // Baked carries no marker: new clip will be remapped to proper shape during import
    commands.spawn((
        Transform::from_translation(Vec3::new(2., 0., 0.)),
        Name::new("Baked"),
        InheritedVisibility::default(),
        AnimationPlayer::default(),
        CharacterShape(baked_shape_handle.clone()),
        ShowcaseBaby::Baked,
        HelperVertexPositions::default(),
        children![(
            Name::new("Mesh"),
            CharacterPart {
                mesh: basemesh_part,
                skeleton_lod: 0
            },
        )],
    ));

    // Root-only strips non-root translations at import (cheapest runtime,
    // loses bone-translation motion). Dynamic uses Bevy's loader untouched
    // (any clip, any shape, per-frame cost). The baked load below retries
    // until morphs are ready; it rewrites every track to this baby shape at
    // import (zero runtime, one asset per shape).
    if animations.root_only_clips.is_none() {
        animations.root_only_clips =
            Some(asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb"));
    }
    if animations.dynamic_clip.is_none() {
        animations.dynamic_clip =
            Some(asset_server.load(GltfAssetLabel::Animation(0).from_asset("animation/idle.glb")));
    }
    animations.baked_shape = Some(baked_shape_handle.clone());

    *done = true;
    info!("Babies created");
}

/// Reads the raw glTF bytes once via the asset reader, so the bake below can
/// run per shape in code. Retries every frame until the source is readable.
fn read_baked_bytes(
    asset_server: Res<AssetServer>,
    animations: Option<ResMut<RetargetedAnimations>>,
) {
    let Some(mut animations) = animations else {
        return;
    };
    if animations.baked_bytes.is_some() {
        return;
    }
    let reader = asset_server
        .get_source(bevy::asset::io::AssetSourceId::Default)
        .expect("default asset source exists")
        .reader();
    let path = std::path::Path::new("animation/idle.glb");
    let bytes = bevy::tasks::block_on(async {
        let mut file = reader.read(path).await.ok()?;
        let mut bytes = Vec::new();
        bevy::asset::io::Reader::read_to_end(&mut file, &mut bytes)
            .await
            .ok()?;
        Some(bytes)
    });
    animations.baked_bytes = bytes;
}

/// Derives the baked baby's corrections once morphs are ready, then bakes its
/// clip in code and inserts it with `Assets::add`. Each `add` mints a fresh
/// UUID, so a second shape baking the same file can never receive this bake
/// through the asset server's outer path dedup. Retries until every input exists.
fn bake_baked_clip(
    basemesh_vertices: Option<Res<BaseMesh>>,
    morph_assets: Option<Res<MakeHumanMorphs>>,
    templates: Res<Assets<CharacterTemplate>>,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    rig_data: Option<Res<RigData>>,
    vertex_groups: Option<Res<VertexGroups>>,
    animations: Option<ResMut<RetargetedAnimations>>,
    mut clips: ResMut<Assets<AnimationClip>>,
) {
    let Some(mut animations) = animations else {
        return;
    };
    if animations.baked_clip.is_some() {
        return;
    }
    let (Some(bytes), Some(baked_shape_handle)) =
        (animations.baked_bytes.clone(), animations.baked_shape.clone())
    else {
        return;
    };
    let (Some(basemesh_vertices), Some(morph_assets), Some(rig_data), Some(vertex_groups)) =
        (basemesh_vertices, morph_assets, rig_data, vertex_groups)
    else {
        return;
    };
    let Some(shape_asset) = shape_assets.get(&baked_shape_handle).cloned() else {
        return;
    };
    let Some(rig_spec) = rig_data.0.as_ref() else {
        return;
    };
    let Ok(corrections) = shape_baked_corrections_for_shape(
        &shape_asset,
        &templates,
        &basemesh_vertices,
        &morph_assets,
        rig_spec,
        &vertex_groups,
    ) else {
        return;
    };
    let Ok(mut baked) = bake_shape_clips_from_bytes(&bytes, &corrections) else {
        return;
    };
    let Some(clip) = baked.remove("Idle-loop") else {
        warn!("baked baby: Idle-loop missing from baked clips");
        return;
    };
    let handle = clips.add(clip);
    if !animations.baked_logged {
        info!(
            "baked baby clip id {:?} from {} byte(s) of animation/idle.glb",
            handle.id(),
            bytes.len(),
        );
        animations.baked_logged = true;
    }
    animations.baked_clip = Some(handle);
}

fn on_gltf_scene_ready(
    trigger: On<WorldInstanceReady>,
    q: Query<(&AnimationIndex, &AnimationGraphHandle)>,
    mut players: Query<&mut AnimationPlayer>,
    children: Query<&Children>,
    mut commands: Commands,
) {
    let (index, graph) = q.get(trigger.entity).unwrap();
    for child in children.iter_descendants(trigger.entity) {
        if let Ok(mut player) = players.get_mut(child) {
            commands.entity(child).insert(graph.clone());
            player.play(index.0).repeat();
            return;
        }
    }
}

fn add_graph(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    retargeted_clips: Res<Assets<RetargetedAnimationAsset>>,
    clips: Res<Assets<AnimationClip>>,
    animations: Option<Res<RetargetedAnimations>>,
    mut character_player: Query<
        (Entity, &mut AnimationPlayer, &ShowcaseBaby),
        Without<AnimationGraphHandle>,
    >,
) {
    let Some(animations) = animations else {
        return;
    };
    // Each baby plays its own clip asset: root-only comes from its custom
    // asset map, dynamic and baked are plain Bevy clip handles (the baked one
    // was baked in code and `add`ed, so its UUID is unique per shape).
    for (entity, mut player, baby) in &mut character_player {
        let clip_handle = match baby {
            ShowcaseBaby::RootOnly => {
                let Some(handle) = animations.root_only_clips.as_ref() else {
                    continue;
                };
                let Some(clips_map) = retargeted_clips.get(handle) else {
                    continue;
                };
                let Some(root_clip) = clips_map.clips.get("Idle-loop") else {
                    continue;
                };
                root_clip.clone()
            }
            ShowcaseBaby::Dynamic => {
                let Some(dynamic) = animations.dynamic_clip.as_ref() else {
                    continue;
                };
                if clips.get(dynamic).is_none() {
                    continue;
                }
                dynamic.clone()
            }
            ShowcaseBaby::Baked => {
                let Some(baked) = animations.baked_clip.as_ref() else {
                    continue;
                };
                if clips.get(baked).is_none() {
                    continue;
                }
                info!("baked baby playing clip id {:?}", baked.id());
                baked.clone()
            }
        };
        let (graph, index) = AnimationGraph::from_clip(clip_handle);
        commands.entity(entity).insert((
            AnimationIndex(index),
            AnimationGraphHandle(graphs.add(graph)),
        ));
        player.play(index).repeat();
        info!("Adding graph handle to showcase baby");
    }
}

fn play_graph(
    mut players: Query<(&mut AnimationPlayer, &AnimationIndex), Added<AnimationGraphHandle>>,
) {
    for (mut player, node_index) in players.iter_mut() {
        player.play(node_index.0).repeat();
    }
}
