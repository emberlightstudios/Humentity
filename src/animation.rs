use crate::{
    NAME_INTERNER,
    basemesh::{BaseMesh, VertexGroups},
    loaders::{BoneJsonConfig, CharacterShapeAsset, ReferenceRigAsset},
    morphs::MorphError,
    rigs::{RigSpec, SkeletonRootBone, fitted_model_space_bindposes},
    spawn_mesh::CharacterShape,
    spawn_skeleton::{CharacterSkeleton, SkeletonLodDisabled},
    template::CharacterTemplate,
};
use ahash::AHashMap;
use bevy::{ecs::intern::Internable, prelude::*};
use serde::{Deserialize, Serialize};

/// humentity's PostUpdate bone-authoritative pass that runs after Bevy's
/// `AnimationSystems` and before `TransformSystems::Propagate`. It covers
/// animation post-processing for the marker on each character, plus ragdoll
/// bone→skeleton sync (`sync_bones_to_ragdoll`). Add your own systems with
/// `.after(HumentitySkeletonSystemSet)` to run after this pass.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HumentitySkeletonSystemSet;

/// Per-character retargeting markers. Put one on the `CharacterShape` entity
/// alongside `CharacterShape`; the matching system runs only for characters
/// that carry it, so different characters can mix styles freely:
///
/// - [`RootOnlyRetargeting`]: clips carry root translation only (rotation +
///   root Y bob). Load clips with the root-only loader
///   ([`crate::loaders::RotationOnlyAnimationAsset`]); only
///   `rescale_root_bone_translation` runs. Cheapest per frame, correct for
///   rotation-driven clips like locomotion, but non-root bone translations
///   are dropped at import and lost.
/// - [`DynamicRetargeting`]: clips keep every translation track. Load clips
///   with Bevy's built-in glTF clip loader (no custom loader);
///   `rescale_dynamic_retargeting` rescales root and all bones per frame from
///   the fit cache. Any clip on any shape, at the cost of a per-frame pass.
///
/// Shape-baked clips ([`crate::loaders::ShapeBakedAnimationAssetLoader`]) need
/// no marker and no system: root and bones are rewritten into the curves at
/// import, so the character animates normally. Never put a marker on a
/// character playing baked clips, or the root scale applies twice.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RootOnlyRetargeting;

/// Full per-frame retarget marker: root + every bone, any clip, any shape.
/// See the marker docs on [`RootOnlyRetargeting`] for the three-way comparison.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DynamicRetargeting;

/// Root-only post-process: rescales root Y to the fitted shape and zeroes
/// root XZ (locomotion comes from gameplay movement). Runs only for
/// characters carrying [`RootOnlyRetargeting`].
pub(crate) fn rescale_root_bone_translation(
    root_info: Query<(&SkeletonRootBone, &ChildOf)>,
    marked_characters: Query<Entity, With<RootOnlyRetargeting>>,
    animation_players: Query<&AnimationPlayer>,
    mut transforms: Query<&mut Transform>,
) {
    for (info, child_of) in &root_info {
        if marked_characters.get(child_of.parent()).is_err() {
            continue;
        }
        let Ok(player) = animation_players.get(child_of.parent()) else {
            continue;
        };
        if player.playing_animations().next().is_none() {
            continue;
        }
        let Ok(mut root_transform) = transforms.get_mut(info.entity) else {
            continue;
        };
        // Offset math: measure from the reference bind the clip was authored
        // against, scale the offset, re-anchor on the fitted bind. Scaling the
        // raw value (`y *= scale`) double-scales: the clip already starts at
        // reference height, so the product lands near the ground for babies.
        let root_offset_y = root_transform.translation.y - info.reference_bind_pose_y;
        if root_offset_y.abs() > 1e-3 {
            root_transform.translation.y = info.bind_pose_y + root_offset_y * info.root_scale;
        } else {
            root_transform.translation.y = info.bind_pose_y;
        }
        root_transform.translation.x = 0.;
        root_transform.translation.z = 0.;
    }
}

/// Cached per-bone correction that retargets a reference-proportioned clip
/// translation onto the fitted shape. Computed once per fit in
/// `fit_skeleton_to_shape`; consumed per frame by
/// `rescale_dynamic_retargeting` and at import by the shape-bake loader.
/// Root has no entity entry here: both dynamic and baked paths own the root
/// separately, so the root scale never applies twice.
#[derive(Clone, Copy, Debug)]
pub struct BoneTranslationCorrection {
    /// Bone entity in the character's fixed skeleton.
    pub bone_entity: Entity,
    /// Fitted local length / reference local length (1.0 when the reference
    /// local is near zero: no scale to derive, pass through).
    pub translation_length_ratio: f32,
    /// Re-aligns the clip translation direction from the reference bone
    /// direction to the fitted shape's direction, expressed in parent space.
    pub translation_direction_adjust: Quat,
}

/// Serializable per-bone correction baked into clip curves at import by the
/// shape-bake loader. Same math as [`BoneTranslationCorrection`], keyed by
/// bone name instead of entity (entities don't exist at import time).
/// Every non-root bone gets an entry, including zero-length reference bones
/// (identity: ratio 1.0, no direction fix, track passes through untouched).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BakedBoneCorrection {
    /// Bone name in the reference rig.
    pub bone_name: String,
    /// Fitted local length / reference local length (1.0 for zero-length
    /// reference bones: pass through, never divide by zero).
    pub translation_length_ratio: f32,
    /// Re-aligns the clip translation direction from the reference bone
    /// direction to the fitted shape's direction, expressed in parent space.
    pub translation_direction_adjust: Quat,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShapeBakedCorrections {
    /// Root Y scale: fitted root height / reference root height.
    pub baked_root_scale: f32,
    /// Fitted root Y; baked curves store fitted-space positions.
    pub baked_bind_pose_y: f32,
    /// Reference root Y; offsets are measured from here, then scaled.
    pub reference_bind_pose_y: f32,
    /// Per-bone corrections for every non-root bone. Root is excluded: the
    /// bake owns the root separately via `baked_root_scale`, so the root
    /// scale never applies twice.
    pub baked_bone_corrections: Vec<BakedBoneCorrection>,
}
pub fn shape_baked_corrections_for_shape(
    shape_asset: &CharacterShapeAsset,
    templates: &Assets<CharacterTemplate>,
    basemesh_vertices: &BaseMesh,
    rig_spec: &RigSpec,
    vertex_groups: &VertexGroups,
) -> Result<ShapeBakedCorrections, MorphError> {
    let template = templates
        .get(&shape_asset.template)
        .ok_or(MorphError::TargetNotFound("template missing for shape"))?;
    if basemesh_vertices.vertices.is_empty()
        || template
            .shapes
            .iter()
            .any(|shape| shape.helper_deltas.is_none())
    {
        return Err(MorphError::TargetNotFound("template deltas not baked yet"));
    }
    let helpers = template.blend_helpers(
        &shape_asset.template_morph_targets,
        &basemesh_vertices.vertices,
    );
    let fitted_model_space = fitted_model_space_bindposes(&helpers, rig_spec, vertex_groups);
    let reference_rig = rig_spec.reference_rig();
    Ok(shape_corrections_from_model_space(
        reference_rig,
        &rig_spec.config.bones,
        &fitted_model_space,
    ))
}
/// Length ratio + parent-space direction fix for one bone, shared by the CPU
/// fit cache, the shape bake, and the GPU shape fit. Returns `None` when the
/// bone is unknown (no reference local or no fitted local): callers skip
/// those. Near-zero reference length yields identity instead of dividing by
/// zero, so zero-length bones retarget as pass-through and every known bone
/// gets an entry — bake, dynamic, and GPU never diverge on short bones.
pub(crate) fn bone_translation_correction(
    reference_rig: &ReferenceRigAsset,
    parent_name: &str,
    bone_name: &'static str,
    fitted_model_space: &AHashMap<&'static str, Transform>,
) -> Option<(f32, Quat)> {
    let reference_local = reference_rig.local_bindpose.get(bone_name)?;
    let fitted_local = local_from_model_space(bone_name, parent_name, fitted_model_space)?;
    let leaked_parent: &'static str = NAME_INTERNER.intern(parent_name).leak();
    // The kept single toes are the one bone whose clips were authored in a
    // different bindpose than the reference rig: walk-clip toe tracks live in
    // the no-toes `movement.glb` frame. Lengths are frame-free scalars; the
    // segment is measured in the bind-pose model frame the fit works in
    // (helper/model space, above the facing flip — never negated, never the
    // bone-local). A local vector here reads a phantom ~111° twist and throws
    // toes behind the foot; the const names say which frame each is in.
    const NO_TOES_TOE_LOCAL_LEN: f32 = 0.1381916;
    const NO_TOES_TOE_MODEL_SEGMENT: Vec3 = Vec3::new(0.0002, -0.0654, 0.1218);
    let (reference_length, translation_direction_adjust) = match bone_name {
        "toe1-1.L" | "toe1-1.R" => (
            NO_TOES_TOE_LOCAL_LEN,
            parent_space_direction_fix_for_segment(
                NO_TOES_TOE_MODEL_SEGMENT,
                bone_name,
                leaked_parent,
                fitted_model_space,
            ),
        ),
        _ => (
            reference_local.translation.length(),
            parent_space_direction_fix(
                reference_rig,
                fitted_model_space,
                bone_name,
                leaked_parent,
            ),
        ),
    };
    if reference_length < 1e-3 {
        return Some((1.0, Quat::IDENTITY));
    }
    let translation_length_ratio = fitted_local.translation.length() / reference_length;
    Some((translation_length_ratio, translation_direction_adjust))
}

pub(crate) fn shape_corrections_from_model_space(
    reference_rig: &ReferenceRigAsset,
    bone_config: &AHashMap<&'static str, BoneJsonConfig>,
    fitted_model_space: &AHashMap<&'static str, Transform>,
) -> ShapeBakedCorrections {
    let root_bone_name = reference_rig.bone_names.first().copied().unwrap_or("");
    let reference_root_y = reference_rig.model_space_bindpose[root_bone_name]
        .translation
        .y;
    let fitted_root_y = fitted_model_space
        .get(root_bone_name)
        .map_or(1.0, |root_pose| root_pose.translation.y);
    let baked_root_scale = if reference_root_y.abs() > 1e-6 {
        fitted_root_y / reference_root_y
    } else {
        1.0
    };
    let mut baked_bone_corrections = Vec::new();
    for &bone_name in &reference_rig.bone_names {
        let Some(bone_config_entry) = bone_config.get(bone_name) else {
            continue;
        };
        if bone_config_entry.parent.is_empty() {
            continue;
        }
        let Some((translation_length_ratio, translation_direction_adjust)) =
            bone_translation_correction(
                reference_rig,
                &bone_config_entry.parent,
                bone_name,
                fitted_model_space,
            )
        else {
            continue;
        };
        baked_bone_corrections.push(BakedBoneCorrection {
            bone_name: bone_name.to_string(),
            translation_length_ratio,
            translation_direction_adjust,
        });
    }
    ShapeBakedCorrections {
        baked_root_scale,
        baked_bind_pose_y: fitted_root_y,
        reference_bind_pose_y: reference_root_y,
        baked_bone_corrections,
    }
}

pub(crate) fn local_from_model_space(
    bone_name: &'static str,
    parent_name: &str,
    fitted_model_space: &AHashMap<&'static str, Transform>,
) -> Option<Transform> {
    let child_matrix = fitted_model_space.get(bone_name)?.to_matrix();
    let parent_matrix = if parent_name.is_empty() {
        Mat4::IDENTITY
    } else {
        let leaked_parent: &'static str = NAME_INTERNER.intern(parent_name).leak();
        fitted_model_space.get(leaked_parent)?.to_matrix()
    };
    Some(Transform::from_matrix(
        parent_matrix.inverse() * child_matrix,
    ))
}

pub(crate) fn parent_space_direction_fix(
    reference_rig: &ReferenceRigAsset,
    fitted_model_space: &AHashMap<&'static str, Transform>,
    bone_name: &'static str,
    parent_name: &'static str,
) -> Quat {
    let (Some(reference_bone), Some(reference_parent)) = (
        reference_rig.model_space_bindpose.get(bone_name),
        reference_rig.model_space_bindpose.get(parent_name),
    ) else {
        return Quat::IDENTITY;
    };
    parent_space_direction_fix_for_segment(
        reference_bone.translation - reference_parent.translation,
        bone_name,
        parent_name,
        fitted_model_space,
    )
}

/// Same fix from an explicit model-frame reference segment. The kept toes use
/// this with the no-toes bindpose segment the clips were authored in; it must
/// be the bind-pose model frame the fit works in (never the bone-local: the
/// arc is measured in model space, so a local vector reads a phantom ~111°
/// twist and throws toes behind the foot).
fn parent_space_direction_fix_for_segment(
    reference_segment: Vec3,
    bone_name: &'static str,
    parent_name: &'static str,
    fitted_model_space: &AHashMap<&'static str, Transform>,
) -> Quat {
    let (Some(fitted_bone), Some(fitted_parent)) = (
        fitted_model_space.get(bone_name),
        fitted_model_space.get(parent_name),
    ) else {
        return Quat::IDENTITY;
    };
    let fitted_segment = fitted_bone.translation - fitted_parent.translation;
    if reference_segment.length_squared() < 1e-12 || fitted_segment.length_squared() < 1e-12 {
        return Quat::IDENTITY;
    }
    let model_space_delta =
        Quat::from_rotation_arc(reference_segment.normalize(), fitted_segment.normalize());
    (fitted_parent.rotation.inverse() * model_space_delta * fitted_parent.rotation).normalize()
}

/// Full per-frame retarget: owns the root (Y rescale + XZ zero, the same math
/// the root-only system does) and every non-root bone (length ratio +
/// direction fix from the fit cache). Runs only for characters carrying
/// [`DynamicRetargeting`]. Clips keep every translation track and are loaded
/// with Bevy's built-in glTF clip loader; nothing happens at import.
///
/// Clips are authored on the reference rig, so every translation track is
/// reference-proportioned. Every corrected bone is rescaled unconditionally,
/// the same math the shape bake applies at import, so baked and dynamic
/// agree on all bones and there is no rest detection to diverge.
pub(crate) fn rescale_dynamic_retargeting(
    characters: Query<(&CharacterSkeleton, Option<&AnimationPlayer>), With<DynamicRetargeting>>,
    root_bones: Query<&SkeletonRootBone>,
    mut bone_transforms: Query<
        &mut Transform,
        (Without<CharacterShape>, Allow<SkeletonLodDisabled>),
    >,
) {
    for (character_skeleton, character_animation_player) in &characters {
        let Some(active_animation_player) = character_animation_player else {
            continue;
        };
        if active_animation_player
            .playing_animations()
            .next()
            .is_none()
        {
            continue;
        }
        if character_skeleton.translation_corrections.is_empty() {
            continue;
        }
        // Root is owned here, not by the root-only system: same offset math,
        // so the root scale never applies twice.
        if let Ok(root_info) = root_bones.get(character_skeleton.skeleton_entity)
            && let Ok(mut root_transform) = bone_transforms.get_mut(root_info.entity)
        {
            let root_offset_y = root_transform.translation.y - root_info.reference_bind_pose_y;
            if root_offset_y.abs() > 1e-3 {
                root_transform.translation.y =
                    root_info.bind_pose_y + root_offset_y * root_info.root_scale;
            } else {
                root_transform.translation.y = root_info.bind_pose_y;
            }
            root_transform.translation.x = 0.;
            root_transform.translation.z = 0.;
        }
        for translation_correction in &character_skeleton.translation_corrections {
            let Ok(mut bone_transform) =
                bone_transforms.get_mut(translation_correction.bone_entity)
            else {
                continue;
            };
            bone_transform.translation = translation_correction.translation_direction_adjust
                * bone_transform.translation
                * translation_correction.translation_length_ratio;
        }
    }
}

#[cfg(test)]
mod kept_toe_correction_tests {
    use super::*;
    use crate::loaders::ReferenceRigAsset;

    /// The kept toe corrects against the no-toes clip frame in the skeleton's
    /// game frame. Fixture uses the real bind-pose numbers: same shape at 3x
    /// scale, so the fitted toe sits where the reference toe sits, longer.
    /// A clip translation down the clip toe axis must come back along the
    /// fitted toe — forward of the foot, on its axis, at fitted length.
    #[test]
    fn kept_toe_translation_stays_forward_on_its_axis() {
        // Real bind-pose numbers: skeleton foot model rotation straight from
        // `skeletons/default.glb`, same shape at 3x scale. The foot is really
        // rotated here — an identity foot puts the skeleton-local vector into
        // a model slot and reads a phantom twist (that mistake failed twice).
        let fitted_foot_rotation =
            Quat::from_array([0.84357, -0.02196, -0.0354, 0.5354]).normalize();
        let fitted_toe_local = Vec3::new(-0.0314, 0.1472, -0.0052).normalize() * 0.4517;
        let fitted_foot = Transform {
            translation: Vec3::ZERO,
            rotation: fitted_foot_rotation,
            scale: Vec3::ONE,
        };
        let fitted_toe = Transform {
            translation: fitted_foot_rotation * fitted_toe_local,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let reference_rig = ReferenceRigAsset {
            bone_names: vec!["foot.L", "toe1-1.L"],
            bone_parents: AHashMap::from_iter([
                ("foot.L", "lowerleg02.L".to_string()),
                ("toe1-1.L", "foot.L".to_string()),
            ]),
            local_bindpose: AHashMap::from_iter([(
                "toe1-1.L",
                Transform::from_translation(Vec3::new(-0.0314, 0.1472, -0.0052)),
            )]),
            model_space_bindpose: AHashMap::from_iter([
                ("foot.L", Transform::IDENTITY),
                (
                    "toe1-1.L",
                    Transform::from_translation(Vec3::new(-0.0314, 0.1472, -0.0052)),
                ),
            ]),
            bone_name_to_index: AHashMap::default(),
            rig_name: "default".to_string(),
        };
        let fitted_model_space: AHashMap<&'static str, Transform> =
            AHashMap::from_iter([("foot.L", fitted_foot), ("toe1-1.L", fitted_toe)]);
        let Some((translation_length_ratio, translation_direction_adjust)) =
            bone_translation_correction(&reference_rig, "foot.L", "toe1-1.L", &fitted_model_space)
        else {
            panic!("kept toe must produce a correction");
        };
        assert!(
            (translation_length_ratio - 0.4517 / 0.1381916).abs() < 0.01,
            "ratio must rescale the clip frame to the fitted toe, got {translation_length_ratio:.4}"
        );
        let walk_clip_toe = Vec3::new(0.0, 0.1382, 0.0);
        let corrected = translation_direction_adjust * walk_clip_toe * translation_length_ratio;
        let sideways = Vec3::new(
            corrected.x - fitted_toe_local.x,
            0.0,
            corrected.z - fitted_toe_local.z,
        )
        .length();
        assert!(
            sideways < 0.1,
            "corrected toe must stay near the fitted toe axis, got sideways {sideways:.4} in {corrected:?}"
        );
    }
}
