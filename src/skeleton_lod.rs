use std::sync::Arc;

use ahash::AHashMap;
use bevy::{
    animation::AnimationTargetId,
    ecs::intern::Internable,
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
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
    /// Resolved at runtime via `all_children_of`.
    pub without_children_of: Vec<String>,

    /// Remove these bones entirely.  Children are reparented to the
    /// bone's parent (walking up the chain if the parent is also removed).
    /// Weights are merged into the surviving ancestor.
    /// e.g., `["spine03", "spine04"]`, `["upperarm02.L"]`.
    pub without_bones: Vec<String>,
}

impl BoneMergeConfig {
    /// No merges — full skeleton.
    pub const fn full() -> Self {
        Self {
            without_children_of: vec![],
            without_bones: vec![],
        }
    }

    /// Add more subtrees to remove.
    pub fn without_children_of(mut self, bones: &[&str]) -> Self {
        self.without_children_of
            .extend(bones.iter().map(|s| s.to_string()));
        self
    }

    /// Add more bones to remove.
    pub fn without_bones(mut self, bones: &[&str]) -> Self {
        self.without_bones
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

/// A skeleton LOD variant — a reduced-bone subset of the original rig.
#[derive(Clone)]
pub struct SkeletonLodVariant {
    pub merge_config: BoneMergeConfig,
    pub scene: Handle<DynamicWorld>,
    /// Surviving bones after merge (subset of original bone_names).
    pub bone_names: Vec<&'static str>,
    /// Updated parent chain for merged hierarchy.
    pub bone_to_parent: AHashMap<&'static str, &'static str>,
    /// Local bindposes for surviving bones (after merge composition).
    pub local_bindpose: AHashMap<&'static str, Transform>,
    /// Model-space bindposes for surviving bones.
    pub model_space_bindpose: AHashMap<&'static str, Transform>,
    /// Inverse bindposes for surviving bones.
    pub inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    /// AnimationTargetIds computed from ORIGINAL reference rig paths.
    pub animation_target_ids: AHashMap<&'static str, AnimationTargetId>,
    /// Merged rig weights for surviving bones.
    pub merged_weights: AHashMap<&'static str, AHashMap<u16, f32>>,
}

/// Complete rig bundle: all LOD variants for a rig.
#[derive(Clone)]
pub struct RigBundle {
    pub lod_variants: Vec<SkeletonLodVariant>,
}

/// Resource specifying which merge configs to use for the rig.
/// Insert this resource before `build_rig_scenes` runs to enable skeleton LOD.
///
/// If not inserted (or empty), rigs get only the full (unmerged) skeleton variant.
#[derive(Resource, Clone, Default)]
pub struct SkeletonLodConfig(pub Vec<BoneMergeConfig>);

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

    for bone_name in &config.without_bones {
        let bone_leaked = NAME_INTERNER.intern(bone_name).leak();
        if let Some(parent) = bone_parents.get(bone_name.as_str()) {
            if !parent.is_empty() {
                let parent_leaked = NAME_INTERNER.intern(parent).leak();
                remove_set.insert(bone_leaked, parent_leaked);
            }
        }
    }

    remove_set
}

/// Step 2: Re-parent and compose local transforms.
/// Handles chained removals — if multiple ancestors are removed,
/// walks the full chain and composes all their transforms.
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

        // Walk up original parent chain, collecting removed ancestors.
        let mut removed_chain: Vec<&str> = Vec::new();
        let mut current_parent = original_bone_parents
            .get(bone)
            .map(|s| s.as_str())
            .unwrap_or("");

        while !current_parent.is_empty() && remove_set.contains_key(current_parent) {
            removed_chain.push(current_parent);
            current_parent = original_bone_parents
                .get(current_parent)
                .map(|s| s.as_str())
                .unwrap_or("");
        }

        // Set parent to nearest surviving ancestor.
        if !current_parent.is_empty() {
            let parent_leaked = NAME_INTERNER.intern(current_parent).leak();
            new_bone_to_parent.insert(bone, parent_leaked);
        }

        // Compose transforms: innermost removed ancestor first.
        // new_local(bone) = local(chain[0]) * local(chain[1]) * ... * local(bone)
        let mut composed = original_local_bindpose
            .get(bone)
            .copied()
            .unwrap_or(Transform::IDENTITY);
        for &removed in &removed_chain {
            if let Some(removed_local) = original_local_bindpose.get(removed) {
                composed = Transform::from_matrix(removed_local.to_matrix() * composed.to_matrix());
            }
        }
        new_local_bindpose.insert(bone, composed);
    }

    let new_bone_names: Vec<&'static str> = original_bone_names
        .iter()
        .filter(|&&name| !remove_set.contains_key(name))
        .copied()
        .collect();

    (new_bone_names, new_bone_to_parent, new_local_bindpose)
}

/// Step 3: Compute AnimationTargetIds from the ORIGINAL reference rig paths.
fn compute_animation_target_ids(
    surviving_bones: &[&'static str],
    original_bone_parents: &AHashMap<&'static str, String>,
) -> AHashMap<&'static str, AnimationTargetId> {
    let mut result = AHashMap::default();

    for &bone in surviving_bones {
        let mut path = Vec::<Name>::new();
        path.push(Name::from(bone));

        let mut current = bone;
        while let Some(parent_str) = original_bone_parents.get(current) {
            if parent_str.is_empty() {
                break;
            }
            path.push(Name::new(parent_str.clone()));
            current = NAME_INTERNER.intern(parent_str).leak();
        }
        // Reverse to root-first
        path.reverse();
        let id = AnimationTargetId::from_names(path.iter());
        result.insert(bone, id);
    }

    result
}

/// Step 4: Merge rig weights.
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

    // Topological sort: process children before parents.
    // If bone A targets bone B and B is also removed, A must be merged first
    // so its weights accumulate into merged[B] before B is forwarded.
    let sorted_removed = topological_sort_removed(remove_set);

    for removed_bone in sorted_removed {
        if let Some(&target) = remove_set.get(removed_bone) {
            // Read accumulated weights (may include merges from child bones),
            // fall back to original weights.
            // Accumulated weights from children that merged into this bone,
            // PLUS this bone's own original weights (both must be forwarded).
            let mut source = merged.remove(removed_bone).unwrap_or_default();
            if let Some(orig) = original_weights.get(removed_bone) {
                for (&mhid, &weight) in orig {
                    *source.entry(mhid).or_insert(0.0) += weight;
                }
            }
            let target_weights = merged.entry(target).or_default();
            for (&mhid, &weight) in &source {
                *target_weights.entry(mhid).or_insert(0.0) += weight;
            }
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

/// Topological sort of removed bones: children before parents.
/// Ensures that when a bone is both a merge target and removed,
/// its accumulated weights flow to the correct ancestor.
fn topological_sort_removed<'a>(remove_set: &AHashMap<&'a str, &'a str>) -> Vec<&'a str> {
    // in_degree[bone] = number of removed bones that must be merged before this one.
    // A removed bone A targeting removed bone B means B depends on A → B's in_degree++.
    let mut in_degree: AHashMap<&str, usize> = AHashMap::default();
    for &bone in remove_set.keys() {
        in_degree.entry(bone).or_insert(0);
    }
    for (_removed_bone, &target) in remove_set {
        if remove_set.contains_key(target) {
            *in_degree.entry(target).or_insert(0) += 1;
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(&bone, _)| bone)
        .collect();
    let mut sorted = Vec::new();

    while let Some(bone) = queue.pop() {
        sorted.push(bone);
        // bone was just processed — find entries where bone is the SOURCE
        // and the target is also removed; decrement target's in_degree.
        for (&other, &target) in remove_set {
            if other == bone && remove_set.contains_key(target) {
                let deg = in_degree.get_mut(target).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push(target);
                }
            }
        }
    }

    sorted
}

/// Build a merged skeleton DynamicWorld scene.
fn build_merged_skeleton_scene(
    bone_names: &[&'static str],
    bone_to_parent: &AHashMap<&'static str, &'static str>,
    local_bindpose: &AHashMap<&'static str, Transform>,
    animation_target_ids: &AHashMap<&'static str, AnimationTargetId>,
    inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    world: &mut World,
) -> Handle<DynamicWorld> {
    let registry = world.resource::<AppTypeRegistry>();
    let mut scene_world = World::new();
    scene_world.insert_resource(registry.clone());

    let rig_entity = scene_world
        .spawn((
            Name::new("Human.rig"),
            Transform::IDENTITY,
            crate::rigs::SkeletalBone,
        ))
        .id();

    let mut bone_entities = AHashMap::<&'static str, Entity>::default();

    for (i, &name) in bone_names.iter().enumerate() {
        let Some(target_id) = animation_target_ids.get(name).copied() else {
            continue;
        };
        let entity = scene_world
            .spawn((Name::new(name), target_id, crate::rigs::SkeletalBone))
            .id();

        if i == 0 {
            scene_world.entity_mut(entity).insert(crate::rigs::RootBone);
        }
        bone_entities.insert(name, entity);
    }

    // Wire up parent-child relationships
    for &name in bone_names {
        let &child = match bone_entities.get(name) {
            Some(e) => e,
            None => continue,
        };
        if let Some(&parent_name) = bone_to_parent.get(name) {
            if let Some(&parent) = bone_entities.get(parent_name) {
                scene_world.entity_mut(parent).add_child(child);
            }
        }
    }

    // Attach root bones to rig entity — any bone whose parent is not itself
    // a bone in this scene (e.g. "" or "Human.rig") belongs under rig_entity.
    for &name in bone_names {
        let is_root = match bone_to_parent.get(name) {
            None => true,
            Some(parent_name) => !bone_entities.contains_key(parent_name),
        };
        if is_root {
            if let Some(&entity) = bone_entities.get(name) {
                scene_world.entity_mut(rig_entity).add_child(entity);
            }
        }
    }

    // Set local transforms on bone entities
    for &name in bone_names {
        let entity = bone_entities[name];
        let local = local_bindpose
            .get(name)
            .copied()
            .unwrap_or(Transform::IDENTITY);
        scene_world.entity_mut(entity).insert(local);
    }

    let joint_entities: Vec<Entity> = bone_names.iter().map(|n| bone_entities[n]).collect();

    let skinned_mesh = SkinnedMesh {
        inverse_bindposes,
        joints: joint_entities,
    };
    scene_world.entity_mut(rig_entity).insert(skinned_mesh);

    let mut ds = world.resource_mut::<Assets<DynamicWorld>>();
    ds.add(DynamicWorld::from_world(&scene_world))
}

/// Build all LOD variants for a rig.
pub(crate) fn build_lod_variants(
    reference_rig: &Arc<ReferenceRigAsset>,
    weights: &Arc<RigWeightsAsset>,
    configs: &[BoneMergeConfig],
    world: &mut World,
) -> Vec<SkeletonLodVariant> {
    let mut variants = Vec::with_capacity(configs.len());

    for config in configs {
        let remove_set = resolve_remove_set(&reference_rig.bone_parents, config);

        let (new_bone_names, new_bone_to_parent, new_local_bindpose) = if remove_set.is_empty() {
            // Full skeleton — no merge
            let bone_to_parent: AHashMap<&'static str, &'static str> = reference_rig
                .bone_parents
                .iter()
                .map(|(&name, parent)| {
                    let parent_leaked = if parent.is_empty() {
                        ""
                    } else {
                        NAME_INTERNER.intern(parent).leak()
                    };
                    (name, parent_leaked)
                })
                .collect();
            (
                reference_rig.bone_names.clone(),
                bone_to_parent,
                reference_rig.local_bindpose.clone(),
            )
        } else {
            merge_hierarchy(
                &reference_rig.bone_names,
                &reference_rig.bone_parents,
                &reference_rig.local_bindpose,
                &remove_set,
            )
        };

        let model_space: AHashMap<&'static str, Transform> = new_bone_names
            .iter()
            .filter_map(|&name| {
                reference_rig
                    .model_space_bindpose
                    .get(name)
                    .map(|&v| (name, v))
            })
            .collect();

        let animation_target_ids =
            compute_animation_target_ids(&new_bone_names, &reference_rig.bone_parents);

        let merged_weights = merge_weights(&weights.weights, &remove_set);

        // Compute inverse bindposes
        let mut inv_bindposes_assets = world.resource_mut::<Assets<SkinnedMeshInverseBindposes>>();
        let inv_bindposes_vec: Vec<Mat4> = new_bone_names
            .iter()
            .map(|name| {
                model_space
                    .get(name)
                    .map(|global| global.to_matrix().inverse())
                    .unwrap_or(Mat4::IDENTITY)
            })
            .collect();
        let inverse_bindposes = inv_bindposes_assets.add(inv_bindposes_vec);

        let scene = build_merged_skeleton_scene(
            &new_bone_names,
            &new_bone_to_parent,
            &new_local_bindpose,
            &animation_target_ids,
            inverse_bindposes.clone(),
            world,
        );

        variants.push(SkeletonLodVariant {
            merge_config: config.clone(),
            scene,
            bone_names: new_bone_names,
            bone_to_parent: new_bone_to_parent,
            local_bindpose: new_local_bindpose,
            model_space_bindpose: model_space,
            inverse_bindposes,
            animation_target_ids,
            merged_weights,
        });
    }

    variants
}
