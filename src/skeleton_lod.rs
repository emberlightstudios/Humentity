use std::sync::Arc;

use ahash::AHashMap;
use bevy::{
    ecs::intern::Internable,
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::{
    loaders::{ReferenceRigAsset, RigWeightsAsset},
    NAME_INTERNER,
};

/// Configuration for which bones to merge.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoneMergeConfig {
    /// Remove all descendants of these anchor bones and merge their
    /// weights into the anchor.  The anchor bone itself survives.
    /// e.g., `["head"]` removes face, `["wrist.L", "wrist.R"]` removes fingers.
    /// Resolved at runtime via `allChildrenOf`.
    pub without_children_of: Vec<String>,
}

impl BoneMergeConfig {
    /// No merges — full skeleton.
    pub const fn full() -> Self {
        Self {
            without_children_of: vec![],
        }
    }

    /// Add more subtrees to remove.
    pub fn without_children_of(mut self, bones: &[&str]) -> Self {
        self.without_children_of
            .extend(bones.iter().map(|s| s.to_string()));
        self
    }
}

/// Returns all descendant bones of `parent_bone` (BFS through bone_parents).
/// Does NOT include `parent_bone` itself.
pub fn all_children_of(
    bone_parents: &AHashMap<&'static str, String>,
    parent_bone: &str,
) -> Vec<&'static str> {
    let mut result = Vec::new();
    let mut stack: Vec<&str> = bone_parents
        .iter()
        .filter(|(_, p)| p.as_str() == parent_bone)
        .map(|(&name, _)| name)
        .collect();

    while let Some(current) = stack.pop() {
        result.push(current);
        for (&name, parent) in bone_parents.iter() {
            if parent.as_str() == current && !result.contains(&name) {
                stack.push(name);
            }
        }
    }

    result
}

/// Per-LOD merge data for a rig. Consumed by mesh building (weight painting)
/// and by part skinning. It deliberately does NOT carry any skeleton scene or
/// bindpose data: a character uses a single fixed skeleton, so LOD is expressed
/// purely by disabling bone sub-trees (see `SkeletonLodConfig`).
#[derive(Clone)]
pub struct SkeletonLodData {
    /// Surviving bones after merge, in reference rig order.
    pub bone_names: Vec<&'static str>,
    /// Merged rig weights for surviving bones (removed bones' weights are
    /// forwarded to their surviving anchors).
    pub merged_weights: AHashMap<&'static str, AHashMap<u16, f32>>,
}

/// Complete rig bundle: merge data for all LOD levels of a rig.
#[derive(Clone)]
pub struct RigBundle {
    pub lod_data: Vec<SkeletonLodData>,
}

/// Maximum number of LOD levels supported by the skeleton LOD system.
pub const MAX_LODS: usize = 4;

/// Resource specifying which merge configs to use for the rig.
/// Insert this resource before `build_rig_scenes` runs to enable skeleton LOD.
///
/// If not inserted (or empty), rigs get only the full (unmerged) skeleton variant.
#[derive(Resource, Clone)]
pub struct SkeletonLodConfig(pub [BoneMergeConfig; MAX_LODS], pub usize);

impl Default for SkeletonLodConfig {
    fn default() -> Self {
        Self(core::array::from_fn(|_| BoneMergeConfig::full()), 1)
    }
}

impl SkeletonLodConfig {
    /// Create a config from a slice of merge configs. Panics if more than
    /// [`MAX_LODS`] configs are provided.
    pub fn new(configs: &[BoneMergeConfig]) -> Self {
        assert!(
            configs.len() <= MAX_LODS,
            "SkeletonLodConfig supports at most {} LOD levels, got {}",
            MAX_LODS,
            configs.len()
        );
        let count = configs.len().max(1);
        let mut lods = core::array::from_fn(|_| BoneMergeConfig::full());
        for (i, c) in configs.iter().enumerate() {
            lods[i] = c.clone();
        }
        Self(lods, count)
    }
}

// ─── Merge Algorithm ──────────────────────────────────────────────────

/// Step 1: Resolve the set of bones to remove and their merge targets.
fn resolve_remove_set(
    bone_parents: &AHashMap<&'static str, String>,
    config: &BoneMergeConfig,
) -> AHashMap<&'static str, &'static str> {
    let mut remove_set = AHashMap::default();

    for anchor in &config.without_children_of {
        let anchor_leaked = NAME_INTERNER.intern(anchor).leak();
        let children = all_children_of(bone_parents, anchor);
        for child in children {
            remove_set.insert(child, anchor_leaked);
        }
    }

    remove_set
}

/// Step 2: Re-parent surviving bones and preserve model-space bindposes.
/// With subtree-only removal, every surviving bone's original parent is also
/// surviving, so no reparenting or transform composition is needed.
fn merge_hierarchy(
    original_bone_names: &[&'static str],
    original_bone_parents: &AHashMap<&'static str, String>,
    original_local_bindpose: &AHashMap<&'static str, Transform>,
    remove_set: &AHashMap<&'static str, &'static str>,
) -> (
    Vec<&'static str>,
    AHashMap<&'static str, &'static str>,
    AHashMap<&'static str, Transform>,
) {
    let mut new_bone_to_parent: AHashMap<&'static str, &'static str> = AHashMap::default();
    let mut new_local_bindpose: AHashMap<&'static str, Transform> = AHashMap::default();

    for &bone in original_bone_names {
        if remove_set.contains_key(bone) {
            continue;
        }

        // Parent is always a surviving bone (subtree removal guarantees this).
        if let Some(parent) = original_bone_parents.get(bone)
            && !parent.is_empty()
        {
            let parent_leaked = NAME_INTERNER.intern(parent).leak();
            new_bone_to_parent.insert(bone, parent_leaked);
        }

        // Local bindpose unchanged — no removed ancestors to compose.
        if let Some(&local) = original_local_bindpose.get(bone) {
            new_local_bindpose.insert(bone, local);
        }
    }

    let new_bone_names: Vec<&'static str> = original_bone_names
        .iter()
        .filter(|&&name| !remove_set.contains_key(name))
        .copied()
        .collect();

    (new_bone_names, new_bone_to_parent, new_local_bindpose)
}

/// Step 3: Merge rig weights.
fn merge_weights(
    original_weights: &AHashMap<&'static str, AHashMap<u16, f32>>,
    remove_set: &AHashMap<&'static str, &'static str>,
) -> AHashMap<&'static str, AHashMap<u16, f32>> {
    let mut merged: AHashMap<&'static str, AHashMap<u16, f32>> = AHashMap::default();

    // Copy surviving bones
    for (&bone, weights) in original_weights {
        if !remove_set.contains_key(bone) {
            merged.insert(bone, weights.clone());
        }
    }

    // Forward removed bones' weights to their (always surviving) merge target.
    for (&removed_bone, &target) in remove_set {
        let source = original_weights
            .get(removed_bone)
            .cloned()
            .unwrap_or_default();
        let target_weights = merged.entry(target).or_default();
        for (&mhid, &weight) in &source {
            *target_weights.entry(mhid).or_insert(0.0) += weight;
        }
    }

    // Re-normalize per-vertex weights
    let all_mhids: Vec<u16> = {
        let mut set = std::collections::HashSet::new();
        for weights in merged.values() {
            for &mhid in weights.keys() {
                set.insert(mhid);
            }
        }
        set.into_iter().collect()
    };

    for mhid in all_mhids {
        let total: f32 = merged.values().filter_map(|w| w.get(&mhid)).sum();
        if total > 0.0 {
            for weights in merged.values_mut() {
                if let Some(w) = weights.get_mut(&mhid) {
                    *w /= total;
                }
            }
        }
    }

    merged
}

/// Build merge data for every LOD level of a rig. No skeleton scenes are
/// produced — a character uses a single fixed skeleton, and each LOD only
/// contributes the surviving bone names + merged weights used for mesh painting.
pub(crate) fn build_lod_data(
    reference_rig: &Arc<ReferenceRigAsset>,
    weights: &Arc<RigWeightsAsset>,
    configs: &[BoneMergeConfig],
) -> Vec<SkeletonLodData> {
    let mut lod_data = Vec::with_capacity(configs.len());

    for config in configs {
        let remove_set = resolve_remove_set(&reference_rig.bone_parents, config);

        let new_bone_names: Vec<&'static str> = if remove_set.is_empty() {
            // Full skeleton — no merge
            reference_rig.bone_names.clone()
        } else {
            let (bone_names, _, _) = merge_hierarchy(
                &reference_rig.bone_names,
                &reference_rig.bone_parents,
                &reference_rig.local_bindpose,
                &remove_set,
            );
            bone_names
        };

        let merged_weights = merge_weights(&weights.weights, &remove_set);

        lod_data.push(SkeletonLodData {
            bone_names: new_bone_names,
            merged_weights,
        });
    }

    lod_data
}
