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
    animation::{local_from_model_space, parent_space_direction_fix},
    basemesh::VertexGroups,
    rigs::{fitted_model_space_bindposes, RigSpec},
    NAME_INTERNER,
};

/// Fit one template shape's morphed body to a GPU shape skeleton.
///
/// `bones` is the LOD bone order (bank order); `model_space` is this shape's
/// fitted model-space bindposes (positions from this shape's morphed helpers,
/// reference rotations restored). Reference data (parents, locals, root Y)
/// comes from `rig`, so the fit can store this shape's
/// `fitted_root_y / reference_root_y` — the same factor the CPU
/// `rescale_root_bone_translation` applies — plus the per-bone dynamic
/// corrections (length ratio + direction fix, same math as the CPU
/// `rescale_dynamic_retargeting`) that the pose shader applies per frame.
///
/// The corrections are true-parent-relative and exact for full skeletons
/// (what [`GpuSkeletonLod`](super::config::GpuSkeletonLod) crowds use: no
/// merged bones, so the survivor parent below always equals the true parent).
pub fn fit_shape_skeleton(
    shape: &'static str,
    bones: &[&'static str],
    model_space: &AHashMap<&'static str, Transform>,
    rig: &RigSpec,
) -> GpuShapeSkeleton {
    let reference_rig = rig.reference_rig();
    let bone_parents = &reference_rig.bone_parents;
    let reference_root_y = reference_rig
        .bone_names
        .first()
        .map(|root_bone| reference_rig.model_space_bindpose[*root_bone].translation.y)
        .unwrap_or(1.0);
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
    let mut translation_ratios = Vec::with_capacity(bones.len());
    let mut translation_direction_adjust = Vec::with_capacity(bones.len());
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
        // Dynamic correction for this bone (same math as the CPU fit cache):
        // scale the clip offset by the fitted/reference length ratio, then
        // rotate it from the reference direction into the fitted direction.
        // Root, zero-length, and unknown bones stay identity so the offset
        // passes through untouched.
        let (length_ratio, direction_fix) = dynamic_correction_for_bone(bone, model_space, rig);
        translation_ratios.push(length_ratio);
        translation_direction_adjust.push(Vec4::new(
            direction_fix.x,
            direction_fix.y,
            direction_fix.z,
            direction_fix.w,
        ));
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
        translation_ratios,
        translation_direction_adjust,
    }
}

/// Length ratio + parent-space direction fix for one bone, mirroring the CPU
/// `fit_skeleton_to_shape` correction cache. Returns identity (1.0,
/// [`Quat::IDENTITY`]) for the root, zero-length bones, and anything without
/// reference data, so the pose shader passes those offsets through untouched.
fn dynamic_correction_for_bone(
    bone_name: &'static str,
    fitted_model_space: &AHashMap<&'static str, Transform>,
    rig: &RigSpec,
) -> (f32, Quat) {
    let reference_rig = rig.reference_rig();
    let Some(bone_config_entry) = rig.config.bones.get(bone_name) else {
        return (1.0, Quat::IDENTITY);
    };
    if bone_config_entry.parent.is_empty() {
        return (1.0, Quat::IDENTITY);
    }
    let Some(reference_local) = reference_rig.local_bindpose.get(bone_name) else {
        return (1.0, Quat::IDENTITY);
    };
    if reference_local.translation.length() < 1e-3 {
        return (1.0, Quat::IDENTITY);
    }
    let Some(fitted_local) =
        local_from_model_space(bone_name, &bone_config_entry.parent, fitted_model_space)
    else {
        return (1.0, Quat::IDENTITY);
    };
    let length_ratio = fitted_local.translation.length() / reference_local.translation.length();
    let parent_name: &'static str = NAME_INTERNER.intern(&bone_config_entry.parent).leak();
    let direction_fix =
        parent_space_direction_fix(reference_rig, fitted_model_space, bone_name, parent_name);
    (length_ratio, direction_fix)
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
    fit_shape_skeleton(shape, bones, &model_space, rig)
}
