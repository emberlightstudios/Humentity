//! Shape and template asset housekeeping: morph resolution, delta baking,
//! build-once freezes, and helper blending.

use std::sync::Arc;

use ahash::AHashMap;
use bevy::prelude::*;

use crate::{
    basemesh::BaseMesh,
    loaders::{CharacterProfile, CharacterShape, CharacterTemplate, TargetAsset, TargetDelta},
    morphs::{MakeHumanMorphs, MorphTargets},
};

/// `base + Σ weight × shape delta`.
///
/// Weights are free: macro shapes usually sum to 1, feature shapes add on top.
/// `shapes` and `weights` are parallel and in template order; unbaked shapes
/// contribute nothing.
pub fn blend_helpers(
    shapes: &[&CharacterShape],
    weights: &[f32],
    basemesh_vertices: &[Vec3],
) -> Vec<Vec3> {
    let mut helpers = basemesh_vertices.to_vec();
    for (shape, &weight) in shapes.iter().zip(weights) {
        if weight == 0.0 {
            continue;
        }
        let Some(deltas) = shape.helper_deltas() else {
            continue;
        };
        for delta in deltas {
            helpers[delta.vertex as usize] += delta.offset * weight;
        }
    }
    helpers
}

/// Resolves macro sliders (e.g. `muscle`) into direct morph targets as shapes
/// load. Shapes must only be created after `HumentityAssetsReady`, or this
/// panics: an unresolvable morph name is an authoring error, not a silent skip.
pub(crate) fn resolve_shape_morphs(
    mut events: MessageReader<AssetEvent<CharacterShape>>,
    morphs: Res<MakeHumanMorphs>,
    mut shapes: ResMut<Assets<CharacterShape>>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id }) = event else {
            continue;
        };
        // Untracked: the shape is still under construction and the `Modified`
        // event would trip the build-once freeze.
        let Some(shape) = shapes.get_mut_untracked(*id) else {
            continue;
        };
        shape.morphs = morphs.compute_target_weights(&shape.morphs).expect(
            "CharacterShape created before morph data was ready: gate shape creation on HumentityAssetsReady",
        );
    }
}

/// Bakes `morphed − base` deltas for shapes whose morph targets and basemesh
/// are loaded. Runs until every shape carries a delta.
pub(crate) fn bake_shape_deltas(
    morphs: Res<MakeHumanMorphs>,
    basemesh: Res<BaseMesh>,
    asset_server: Res<AssetServer>,
    mut shapes: ResMut<Assets<CharacterShape>>,
) {
    if basemesh.vertices.is_empty() || !morphs.is_ready(&asset_server) {
        return;
    }
    let basemesh_vert_count = basemesh.vertices.len();
    let Ok(targets) = morphs.targets.read() else {
        return;
    };
    // Collect under a shared borrow, then write untracked: `iter_mut` emits one
    // `Modified` per shape and would trip the freeze below.
    let pending: Vec<_> = shapes.ids().collect();
    for id in pending {
        let needs_bake = shapes
            .get(id)
            .is_some_and(|shape| !shape.is_baked() && has_targets_for_shape(&targets, shape));
        if !needs_bake {
            continue;
        }
        let Some(shape) = shapes.get_mut_untracked(id) else {
            continue;
        };
        let deltas = bake_shape_deltas_from_targets(&shape.morphs, &targets, basemesh_vert_count);
        shape.helper_deltas = Some(Arc::new(deltas));
    }
}

pub(crate) fn has_targets_for_shape(
    targets: &AHashMap<&'static str, TargetAsset>,
    shape: &CharacterShape,
) -> bool {
    shape.morphs.keys().all(|name| targets.contains_key(name))
}

fn bake_shape_deltas_from_targets(
    shape_morphs: &MorphTargets,
    targets: &AHashMap<&'static str, TargetAsset>,
    basemesh_vert_count: usize,
) -> Vec<TargetDelta> {
    let mut accumulated: AHashMap<u16, Vec3> = AHashMap::default();
    for (&target_name, &target_weight) in shape_morphs.iter() {
        let Some(target) = targets.get(target_name) else {
            panic!(
                "shape references morph target '{target_name}' with no loaded target: gate shape creation on HumentityAssetsReady"
            );
        };
        for delta in target.deltas.iter() {
            debug_assert!(
                (delta.vertex as usize) < basemesh_vert_count,
                "morph target '{target_name}' vertex {} out of range for {basemesh_vert_count} basemesh verts",
                delta.vertex,
            );
            *accumulated.entry(delta.vertex).or_insert(Vec3::ZERO) += delta.offset * target_weight;
        }
    }
    accumulated
        .into_iter()
        .map(|(vert_index, vert_offset)| TargetDelta {
            vertex: vert_index,
            offset: vert_offset,
        })
        .collect()
}

/// Panics if a loaded shape is modified after upload.
///
/// Meshes bake one morph target per shape and cache it by template handle, so an
/// edit would leave every cached mesh serving the old delta with no error. Make
/// a new shape asset instead.
pub(crate) fn enforce_shapes_frozen(mut events: MessageReader<AssetEvent<CharacterShape>>) {
    for event in events.read() {
        if matches!(event, AssetEvent::Modified { .. }) {
            panic!(
                "CharacterShape modified after upload: shapes are build-once, create a new shape instead of editing one in the asset store"
            );
        }
    }
}

/// Panics if a `CharacterTemplate` is modified after upload.
///
/// Templates are build-once: mesh builds are cached by template handle, so a
/// runtime edit would silently serve stale meshes. Internal init writes use
/// `get_mut_untracked` precisely so they never emit `Modified` — only an
/// external edit trips this.
pub(crate) fn enforce_template_frozen(mut events: MessageReader<AssetEvent<CharacterTemplate>>) {
    for event in events.read() {
        if matches!(event, AssetEvent::Modified { .. }) {
            panic!(
                "CharacterTemplate modified after upload: templates are build-once, create a new template instead of editing one in the asset store"
            );
        }
    }
}

/// Blended helpers for a character profile: `base + Σ weight × shape delta`.
///
/// `None` while the template, any of its shapes, or their deltas are not
/// ready. Callers treat that as "not yet", exactly like a missing asset.
pub fn blend_helpers_for_profile(
    profile: &CharacterProfile,
    templates: &Assets<CharacterTemplate>,
    shapes: &Assets<CharacterShape>,
    basemesh_vertices: &[Vec3],
) -> Option<Vec<Vec3>> {
    let template = templates.get(&profile.template)?;
    let resolved_shapes = template.resolved_shapes(shapes)?;
    if basemesh_vertices.is_empty() || resolved_shapes.iter().any(|shape| !shape.is_baked()) {
        return None;
    }
    let weights = profile.resolve_weights(templates, shapes).ok()?;
    Some(blend_helpers(&resolved_shapes, &weights, basemesh_vertices))
}
