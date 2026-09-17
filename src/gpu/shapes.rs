//! Per-shape skeleton fitting for the GPU crowd.
//!
//! The game computes one [`GpuShapeSkeleton`](super::config::GpuShapeSkeleton)
//! per template shape and registers it on
//! [`GpuCrowdShapes`](super::config::GpuCrowdShapes). The pose pipeline then
//! uploads per-shape buffers so each instance poses with its own shape's rest
//! translations and inverse bindposes.
//!
//! The fit mirrors the CPU path (`fit_skeleton_to_shape`): helper verts locate
//! the joints for this shape's morphed body, reference rotations are restored
//! (clips are authored for reference rotations), and locals + inverse
//! bindposes are derived from those. Helpers, template lookups, and weight
//! bookkeeping stay game-side — this function takes plain data.

use ahash::AHashMap;
use bevy::{ecs::intern::Internable, prelude::*};

use super::config::GpuShapeSkeleton;
use crate::{
    NAME_INTERNER,
    basemesh::VertexGroups,
    rigs::{RigSpec, fitted_model_space_bindposes},
};

/// Fit one template shape's morphed body to a GPU shape skeleton.
///
/// `bones` is the LOD bone order (bank order); `model_space` is this shape's
/// fitted model-space bindposes (positions from this shape's morphed helpers,
/// reference rotations restored). `reference_root_y` is the reference rig's
/// root model-space Y, so the fit can store this shape's
/// `fitted_root_y / reference_root_y` — the same factor the CPU
/// `rescale_root_bone_translation` applies.
pub fn fit_shape_skeleton(
    shape: &'static str,
    bones: &[&'static str],
    model_space: &AHashMap<&'static str, Transform>,
    bone_parents: &AHashMap<&'static str, String>,
    reference_root_y: f32,
) -> GpuShapeSkeleton {
    // Nearest surviving LOD ancestor for each bone. Bones whose ancestors
    // merged away re-anchor to the survivor so the parent chain stays
    // continuous; unresolvable bones keep their model-space as the local so
    // they still land near their fitted position.
    let mut survivor_parent: AHashMap<&'static str, Option<&'static str>> = AHashMap::default();
    for &bone in bones.iter() {
        let mut ancestor = bone_parents.get(bone).cloned().unwrap_or_default();
        loop {
            if ancestor.is_empty() || ancestor == "Human.rig" {
                survivor_parent.insert(bone, None);
                break;
            }
            let leaked: &'static str = NAME_INTERNER.intern(&ancestor).leak();
            if bones.contains(&leaked) {
                survivor_parent.insert(bone, Some(leaked));
                break;
            }
            ancestor = bone_parents.get(leaked).cloned().unwrap_or_default();
        }
    }
    let mut translations = Vec::with_capacity(bones.len());
    let mut inv_binds = Vec::with_capacity(bones.len());
    for bone in bones.iter() {
        let child = model_space[bone].to_matrix();
        let local_translation = match survivor_parent[bone] {
            Some(parent) => {
                let local = model_space[parent].to_matrix().inverse() * child;
                local.transform_point3(Vec3::ZERO)
            }
            None => model_space[bone].translation,
        };
        translations.push(Vec4::new(
            local_translation.x,
            local_translation.y,
            local_translation.z,
            0.0,
        ));
        inv_binds.push(model_space[bone].to_matrix().inverse());
    }
    let root_scale = if reference_root_y.abs() > 1e-6 {
        model_space
            .get(bones[0])
            .map(|t| t.translation.y / reference_root_y)
            .unwrap_or(1.0)
    } else {
        1.0
    };
    GpuShapeSkeleton {
        shape,
        translations,
        inv_binds,
        root_scale,
    }
}
/// Convenience wrapper: fit directly from morphed helper verts (same inputs
/// as the CPU fit: helpers + rig + vertex groups).
pub fn fit_shape_skeleton_from_helpers(
    shape: &'static str,
    helpers: &[Vec3],
    bones: &[&'static str],
    rig: &RigSpec,
    vg: &VertexGroups,
) -> GpuShapeSkeleton {
    let model_space = fitted_model_space_bindposes(helpers, rig, vg);
    let reference_root_y = rig.reference_rig().bone_names.first().map_or(1.0, |root| {
        rig.reference_rig().model_space_bindpose[*root].translation.y
    });
    fit_shape_skeleton(
        shape,
        bones,
        &model_space,
        &rig.reference_rig().bone_parents,
        reference_root_y,
    )
}
