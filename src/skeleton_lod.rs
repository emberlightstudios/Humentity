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
    /// Merge all descendants of a parent bone into one surviving child bone.
    /// Unlike `without_children_of` (which folds a whole sub-tree into the
    /// parent and disables it), the kept bone stays enabled as a child of the
    /// parent so posing still works through it. Every other descendant of the
    /// parent — including the kept bone's own children — is removed and has
    /// its weights folded into the kept bone.
    #[serde(default)]
    pub merge_into_kept_bone: Vec<MergeIntoKeptBone>,
}

/// One `merge_into_kept_bone` entry: keep `kept_bone_name` (a direct child of
/// `parent_bone_name`) and fold every other descendant of the parent into it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergeIntoKeptBone {
    pub parent_bone_name: String,
    pub kept_bone_name: String,
}

impl BoneMergeConfig {
    /// No merges — full skeleton.
    pub const fn full() -> Self {
        Self {
            without_children_of: vec![],
            merge_into_kept_bone: vec![],
        }
    }

    /// Add more subtrees to remove.
    pub fn without_children_of(mut self, bones: &[&str]) -> Self {
        self.without_children_of
            .extend(bones.iter().map(|s| s.to_string()));
        self
    }

    /// Merge a sub-tree into one surviving child bone instead of the parent.
    /// e.g. `merge_into_kept_bone("foot.L", "toe1-1.L")` removes every toe
    /// bone except `toe1-1.L` and folds their weights into it, keeping a
    /// single posable toe per foot.
    pub fn merge_into_kept_bone(mut self, parent_bone_name: &str, kept_bone_name: &str) -> Self {
        self.merge_into_kept_bone.push(MergeIntoKeptBone {
            parent_bone_name: parent_bone_name.to_string(),
            kept_bone_name: kept_bone_name.to_string(),
        });
        self
    }

    /// Default-rig shortcut: merge each foot's toe bones into a single toe
    /// bone (`toe1-1.L` / `toe1-1.R`) kept as a child of the foot, instead of
    /// folding the toes into the foot and losing all toe posing.
    pub fn merge_default_rig_toes(self) -> Self {
        self.merge_into_kept_bone("foot.L", "toe1-1.L")
            .merge_into_kept_bone("foot.R", "toe1-1.R")
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
///
/// Anchors resolve first (later anchors overwrite earlier ones for shared
/// descendants). A kept-bone merge then takes precedence over an anchor on
/// its own parent, but is void when an anchor on an ancestor already removed
/// the parent itself (e.g. a `lowerleg02` anchor overrides the toe merge, so
/// high LODs can still drop the whole foot).
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

    for kept_merge in &config.merge_into_kept_bone {
        let parent_bone_leaked = NAME_INTERNER.intern(&kept_merge.parent_bone_name).leak();
        let kept_bone_leaked = NAME_INTERNER.intern(&kept_merge.kept_bone_name).leak();
        let descendant_bones = all_children_of(bone_parents, parent_bone_leaked);
        if !descendant_bones.contains(&kept_bone_leaked) {
            continue;
        }
        if remove_set.contains_key(parent_bone_leaked) {
            continue;
        }
        remove_set.remove(kept_bone_leaked);
        for descendant_bone in descendant_bones {
            if descendant_bone == kept_bone_leaked {
                continue;
            }
            remove_set.insert(descendant_bone, kept_bone_leaked);
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

#[cfg(test)]
mod kept_bone_merge_tests {
    use super::*;

    fn foot_test_rig() -> (Vec<&'static str>, ReferenceRigAsset, RigWeightsAsset) {
        let bone_names: Vec<&'static str> = vec![
            "foot.L",
            "toe1-1.L",
            "toe1-2.L",
            "toe2-1.L",
            "toe2-2.L",
        ];
        let bone_parents: AHashMap<&'static str, String> = [
            ("foot.L", "lowerleg02.L".to_string()),
            ("toe1-1.L", "foot.L".to_string()),
            ("toe1-2.L", "toe1-1.L".to_string()),
            ("toe2-1.L", "foot.L".to_string()),
            ("toe2-2.L", "toe2-1.L".to_string()),
        ]
        .into_iter()
        .collect();
        let local_bindpose: AHashMap<&'static str, Transform> = bone_names
            .iter()
            .map(|&toe_bone_name| (toe_bone_name, Transform::IDENTITY))
            .collect();
        let reference_rig = ReferenceRigAsset {
            bone_names: bone_names.clone(),
            bone_parents,
            local_bindpose,
            model_space_bindpose: AHashMap::default(),
            bone_name_to_index: bone_names
                .iter()
                .enumerate()
                .map(|(toe_bone_index, &toe_bone_name)| (toe_bone_name, toe_bone_index))
                .collect(),
            rig_name: "default".to_string(),
        };
        let rig_weights_asset = RigWeightsAsset {
            weights: bone_names
                .iter()
                .enumerate()
                .map(|(toe_bone_index, &toe_bone_name)| {
                    (
                        toe_bone_name,
                        [(1000 + toe_bone_index as u16, 1.0)]
                            .into_iter()
                            .collect::<AHashMap<u16, f32>>(),
                    )
                })
                .collect(),
            rig_name: "default".to_string(),
        };
        (bone_names, reference_rig, rig_weights_asset)
    }

    #[test]
    fn toe_merge_keeps_one_toe_bone_per_foot() {
        let (_foot_bone_names, reference_rig, rig_weights_asset) = foot_test_rig();
        let remove_set = resolve_remove_set(
            &reference_rig.bone_parents,
            &BoneMergeConfig::full().merge_default_rig_toes(),
        );
        assert_eq!(remove_set.get("toe2-1.L"), Some(&"toe1-1.L"));
        assert_eq!(remove_set.get("toe2-2.L"), Some(&"toe1-1.L"));
        assert_eq!(remove_set.get("toe1-2.L"), Some(&"toe1-1.L"));
        assert!(!remove_set.contains_key("toe1-1.L"));
        assert!(!remove_set.contains_key("foot.L"));
        let merged_weights = merge_weights(&rig_weights_asset.weights, &remove_set);
        assert!(merged_weights.contains_key("toe1-1.L"));
        assert!(!merged_weights.contains_key("toe2-1.L"));
        assert!(merged_weights.contains_key("foot.L"));
    }

    #[test]
    fn ancestor_anchor_overrides_kept_toe_merge() {
        let remove_set = resolve_remove_set(
            &foot_test_rig().1.bone_parents,
            &BoneMergeConfig::full()
                .merge_default_rig_toes()
                .without_children_of(&["lowerleg02.L"]),
        );
        assert_eq!(remove_set.get("foot.L"), Some(&"lowerleg02.L"));
        assert_eq!(remove_set.get("toe1-1.L"), Some(&"lowerleg02.L"));
        assert_eq!(remove_set.get("toe2-2.L"), Some(&"lowerleg02.L"));
    }

    #[test]
    fn toe_merge_missing_kept_toe_bone_falls_back_to_anchors() {
        let (_foot_bone_names, mut reference_rig, _weights_asset) = foot_test_rig();
        reference_rig.bone_parents.remove("toe1-1.L");
        let remove_set = resolve_remove_set(
            &reference_rig.bone_parents,
            &BoneMergeConfig::full().merge_default_rig_toes(),
        );
        assert!(remove_set.is_empty());
    }
}
