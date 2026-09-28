use crate::{
    NAME_INTERNER,
    basemesh::{BaseMesh, VertexGroups},
    loaders::{BoneJsonConfig, CharacterShapeAsset, ReferenceRigAsset},
    morphs::{MakeHumanMorphs, MorphError},
    rigs::{RigSpec, SkeletonRootBone, fitted_model_space_bindposes},
    spawn_mesh::CharacterShape,
    spawn_skeleton::{CharacterSkeleton, SkeletonLodDisabled},
    template::CharacterTemplate,
};
use ahash::{AHashMap, AHashSet};
use bevy::{
    animation::{AnimationTargetId, animated_field},
    ecs::intern::Internable,
    prelude::*,
};
use gltf::Skin;
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
///   ([`crate::loaders::RetargetedAnimationAsset`]); only
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
    morphs: &MakeHumanMorphs,
    rig_spec: &RigSpec,
    vertex_groups: &VertexGroups,
) -> Result<ShapeBakedCorrections, MorphError> {
    let template = templates
        .get(&shape_asset.template)
        .ok_or(MorphError::TargetNotFound("template missing for shape"))?;
    let helpers = template.get_helpers(
        &shape_asset.template_morph_targets,
        basemesh_vertices,
        morphs,
    )?;
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
    let reference_length = reference_local.translation.length();
    if reference_length < 1e-3 {
        return Some((1.0, Quat::IDENTITY));
    }
    let translation_length_ratio = fitted_local.translation.length() / reference_length;
    let leaked_parent: &'static str = NAME_INTERNER.intern(parent_name).leak();
    let translation_direction_adjust = parent_space_direction_fix(
        reference_rig,
        fitted_model_space,
        bone_name,
        leaked_parent,
    );
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
    Some(Transform::from_matrix(parent_matrix.inverse() * child_matrix))
}

pub(crate) fn parent_space_direction_fix(
    reference_rig: &ReferenceRigAsset,
    fitted_model_space: &AHashMap<&'static str, Transform>,
    bone_name: &'static str,
    parent_name: &'static str,
) -> Quat {
    let (Some(reference_bone), Some(reference_parent), Some(fitted_bone), Some(fitted_parent)) = (
        reference_rig.model_space_bindpose.get(bone_name),
        reference_rig.model_space_bindpose.get(parent_name),
        fitted_model_space.get(bone_name),
        fitted_model_space.get(parent_name),
    ) else {
        return Quat::IDENTITY;
    };
    let reference_segment = reference_bone.translation - reference_parent.translation;
    let fitted_segment = fitted_bone.translation - fitted_parent.translation;
    if reference_segment.length_squared() < 1e-12 || fitted_segment.length_squared() < 1e-12 {
        return Quat::IDENTITY;
    }
    let model_space_delta = Quat::from_rotation_arc(
        reference_segment.normalize(),
        fitted_segment.normalize(),
    );
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
    characters: Query<
        (&CharacterSkeleton, Option<&AnimationPlayer>),
        With<DynamicRetargeting>,
    >,
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

/// Root-only clip import: keeps rotation and scale tracks on every bone, but
/// translation tracks on the root bone only. Non-root translations are dropped
/// at import, so the only runtime cost is the root fix on characters carrying
/// [`RootOnlyRetargeting`]. Rotation-driven clips (locomotion, idle) look
/// right; clips whose motion lives in bone translations lose it.
pub(crate) fn get_animation_clips_from_bytes(
    bytes: &[u8],
) -> Result<AHashMap<&'static str, AnimationClip>, BevyError> {
    let (document, buffers, _) = gltf::import_slice(bytes)?;
    if document.skins().len() > 1 {
        return Err(BevyError::from("More than one skin present in file"));
    };
    let Some(skin) = document.skins().next() else {
        return Err(BevyError::from("No skins available"));
    };
    let mut transforms = AHashMap::<&'static str, Transform>::default();
    let mut node_indices = AHashMap::<&'static str, usize>::default();

    for joint in skin.joints() {
        let name = joint.name().unwrap_or("");
        node_indices.insert(NAME_INTERNER.intern(name).leak(), joint.index());
        let (pos, rot, scale) = joint.transform().decomposed();
        let transform = Transform {
            translation: Vec3::from_array(pos),
            rotation: Quat::from_array(rot),
            scale: Vec3::from_array(scale),
        };
        transforms.insert(NAME_INTERNER.intern(name).leak(), transform);
    }

    let mut global_transforms = AHashMap::default();
    let root = &find_root_joints(&skin);
    compute_global_transform(
        root,
        &transforms,
        &mut global_transforms,
        Transform::IDENTITY,
    )?;
    let root_bone_name = root
        .name()
        .map(|root_name| NAME_INTERNER.intern(root_name).leak())
        .unwrap_or("");

    let joint_targets = build_joint_paths(root);

    let mut new_clips = AHashMap::default();

    for animation in document.animations() {
        let mut clip = AnimationClip::default();
        let clip_name = animation
            .name()
            .ok_or_else(|| BevyError::from("Animation clip has no name"))?;

        for channel in animation.channels() {
            let sampler = channel.sampler();
            let input_accessor = sampler.input();
            let input_view = input_accessor
                .view()
                .ok_or_else(|| BevyError::from("Failed to get input_view for animation"))?;
            let buffer = &buffers[input_view.buffer().index()];

            let start = input_accessor.offset() + input_view.offset();
            let end = start + input_accessor.count() * std::mem::size_of::<f32>();
            let data = &buffer[start..end];
            let times: Vec<f32> = bytemuck::cast_slice(data).to_vec();

            let output_accessor = sampler.output();
            let output_view = output_accessor
                .view()
                .ok_or_else(|| BevyError::from("Missing output view"))?;
            let buffer = &buffers[output_view.buffer().index()];

            let target = channel.target();
            let target_property = target.property();
            let target_node = target.node();
            let target_name = target_node.name().expect("Failed to match node name");
            let target_name = Name::new(NAME_INTERNER.intern(target_name).leak());
            let target_id = AnimationTargetId::from_names(joint_targets[&target_name].iter());

            let start = output_view.offset() + output_accessor.offset();
            let floats_per_element = match target_property {
                gltf::animation::Property::Translation | gltf::animation::Property::Scale => 3,
                gltf::animation::Property::Rotation => 4,
                _ => continue,
            };
            let end =
                start + output_accessor.count() * floats_per_element * std::mem::size_of::<f32>();
            let values = &buffer[start..end];
            let floats: &[f32] = bytemuck::cast_slice(values);

            match target_property {
                gltf::animation::Property::Translation => {
                    // Root only: non-root translations are dropped at import.
                    // Cheaper curves and nothing to fix at runtime, but any
                    // bone-translation motion is gone for good.
                    let animated_bone_name: &str = target_name.as_str();
                    if animated_bone_name != root_bone_name {
                        continue;
                    }
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(
                            animated_field!(Transform::translation),
                            AnimatableKeyframeCurve::new(
                                times.into_iter().zip(
                                    floats.chunks(floats_per_element).map(|chunk| {
                                        Vec3::from_array([chunk[0], chunk[1], chunk[2]])
                                    }),
                                ),
                            )?,
                        ),
                    );
                }
                gltf::animation::Property::Scale => {
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(
                            animated_field!(Transform::scale),
                            AnimatableKeyframeCurve::new(
                                times.into_iter().zip(
                                    floats.chunks(floats_per_element).map(|chunk| {
                                        Vec3::from_array([chunk[0], chunk[1], chunk[2]])
                                    }),
                                ),
                            )?,
                        ),
                    );
                }
                gltf::animation::Property::Rotation => {
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(
                            animated_field!(Transform::rotation),
                            AnimatableKeyframeCurve::new(times.into_iter().zip(
                                floats.chunks(floats_per_element).map(|chunk| {
                                    Quat::from_array([chunk[0], chunk[1], chunk[2], chunk[3]])
                                        .normalize()
                                }),
                            ))?,
                        ),
                    );
                }
                _ => continue,
            }
        }
        new_clips.insert(NAME_INTERNER.intern(clip_name).leak(), clip);
    }

    Ok(new_clips)
}

fn compute_global_transform(
    root: &gltf::Node,
    local_transforms: &AHashMap<&'static str, Transform>,
    global_transforms: &mut AHashMap<&'static str, Transform>,
    parent_global: Transform,
) -> Result<(), BevyError> {
    let name = root
        .name()
        .ok_or_else(|| BevyError::from("No name for bone"))?;
    let name = NAME_INTERNER.intern(name).leak();
    let local = local_transforms
        .get(&name)
        .ok_or_else(|| BevyError::from("Missing local transform"))?;
    let global = Transform::from_matrix(parent_global.to_matrix() * local.to_matrix());
    global_transforms.insert(name, global);

    for child in root.children() {
        compute_global_transform(&child, local_transforms, global_transforms, global)?;
    }
    Ok(())
}

/// Builds a map of joint name -> full path (from root to that joint)
pub fn build_joint_paths(root: &gltf::Node) -> AHashMap<Name, Vec<Name>> {
    let mut paths = AHashMap::default();
    let mut current_path = vec!["Human.rig".to_string()];
    collect_paths_recursive(root, &mut current_path, &mut paths);
    paths
        .into_iter()
        .map(|(k, v)| {
            (
                Name::new(NAME_INTERNER.intern(&k).leak()),
                v.into_iter()
                    .map(|n| Name::new(NAME_INTERNER.intern(&n).leak()))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<AHashMap<Name, Vec<Name>>>()
}

fn collect_paths_recursive(
    node: &gltf::Node,
    current_path: &mut Vec<String>,
    paths: &mut AHashMap<String, Vec<String>>,
) {
    let name = node.name().unwrap().to_string();
    current_path.push(name.clone());

    // Store a clone of the current path for this node
    paths.insert(name, current_path.clone());

    // Recurse into children
    for child in node.children() {
        collect_paths_recursive(&child, current_path, paths);
    }

    current_path.pop();
}

/// Return the root joint node for a skin (there can be multiple; the last
/// unparented joint wins). Shared by the root-only import and the bake loader
/// so both agree on which bone owns the root.
pub(crate) fn find_root_joints<'a>(skin: &Skin<'a>) -> gltf::Node<'a> {
    // Collect joints and their indices
    let joints: Vec<gltf::Node> = skin.joints().collect();
    let joint_indices: AHashSet<usize> = joints.iter().map(|n| n.index()).collect();

    // Track which joint indices appear as children of other joints
    let mut seen_as_child: AHashSet<usize> = AHashSet::default();
    for joint in &joints {
        for child in joint.children() {
            if joint_indices.contains(&child.index()) {
                seen_as_child.insert(child.index());
            }
        }
    }

    // Roots are joints that were never seen as children
    joints
        .into_iter()
        .rfind(|j| !seen_as_child.contains(&j.index()))
        .expect("Unable to find root node")
}
