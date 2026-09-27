use crate::{
    NAME_INTERNER,
    rigs::SkeletonRootBone,
    spawn_mesh::CharacterShape,
    spawn_skeleton::{CharacterSkeleton, SkeletonLodDisabled},
};
use ahash::{AHashMap, AHashSet};
use bevy::{
    animation::{animated_field, AnimationTargetId},
    ecs::intern::Internable,
    prelude::*,
};
use gltf::Skin;

/// humentity's PostUpdate bone-authoritative pass that runs after Bevy's
/// `AnimationSystems` and before `TransformSystems::Propagate`. It covers
/// animation post-processing (`rescale_root_bone_translation` for the root,
/// `rescale_full_bone_translations` for `Full` non-root tracks) and ragdoll
/// bone→skeleton sync (`sync_bones_to_ragdoll`). Add your own systems with
/// `.after(HumentitySkeletonSystemSet)` to run after this pass.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HumentitySkeletonSystemSet;

/// Marker for the translation-track workflow. Clips always keep every bone's
/// translation tracks; non-root tracks are rescaled per frame by
/// `rescale_full_bone_translations` only when the `dynamic_translation_tracks`
/// plugin flag is on. The root bone is always handled by
/// `rescale_root_bone_translation`.
#[derive(Copy, Clone, Debug)]
pub struct TranslationTracks;

pub(crate) fn rescale_root_bone_translation(
    root_info: Query<(&SkeletonRootBone, &ChildOf)>,
    animation_players: Query<&AnimationPlayer>,
    mut transforms: Query<&mut Transform>,
) {
    for (info, child_of) in &root_info {
        let Ok(player) = animation_players.get(child_of.parent()) else {
            continue;
        };
        if player.playing_animations().next().is_none() {
            continue;
        }
        let Ok(mut t) = transforms.get_mut(info.entity) else {
            continue;
        };
        let delta = t.translation.y - info.bind_pose_y;
        if delta.abs() > 1e-3 {
            t.translation.y *= info.root_scale;
        }
        t.translation.x = 0.;
        t.translation.z = 0.;
    }
}

/// Cached per-bone correction that retargets a reference-proportioned clip
/// translation onto the fitted shape. Computed once per fit in
/// `fit_skeleton_to_shape`; applied per frame by
/// `rescale_full_bone_translations`.
#[derive(Clone, Copy, Debug)]
pub struct BoneTranslationCorrection {
    /// Bone entity in the character's fixed skeleton.
    pub bone_entity: Entity,
    /// Fitted bind-pose local translation. Bones still sitting at rest are
    /// skipped per frame (deadzone), so bones the clip leaves at rest cost a
    /// single cheap check and no rescale.
    pub fitted_rest_translation: Vec3,
    /// Fitted local length / reference local length.
    pub translation_length_ratio: f32,
    /// Re-aligns the clip translation direction from the reference bone
    /// direction to the fitted shape's direction, expressed in parent space.
    pub translation_direction_adjust: Quat,
}

/// Post-process for non-root translation tracks: rescales every non-root
/// bone's clip translation to the fitted shape. Runs only when the
/// `dynamic_translation_tracks` plugin flag is on.
///
/// Clips are authored on the reference rig, so their translation tracks are
/// reference-proportioned. The cached [`BoneTranslationCorrection`] rescales
/// them per bone after Bevy's `AnimationSystems` run.
///
/// Only bones the clip actually drives differ from their fitted rest, so bones
/// at rest are skipped via deadzone. The root bone is excluded entirely:
/// locomotion comes from gameplay movement, and
/// `rescale_root_bone_translation` owns it.
pub(crate) fn rescale_full_bone_translations(
    characters: Query<(&CharacterSkeleton, Option<&AnimationPlayer>)>,
    mut bone_transforms: Query<
        &mut Transform,
        (Without<CharacterShape>, Allow<SkeletonLodDisabled>),
    >,
) {
    for (character_skeleton, character_animation_player) in &characters {
        if character_skeleton.translation_corrections.is_empty() {
            continue;
        }
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
        for translation_correction in &character_skeleton.translation_corrections {
            let Ok(mut bone_transform) =
                bone_transforms.get_mut(translation_correction.bone_entity)
            else {
                continue;
            };
            let animated_translation = bone_transform.translation;
            let rest_offset =
                animated_translation - translation_correction.fitted_rest_translation;
            if rest_offset.length_squared() <= 1e-6 {
                continue;
            }
            bone_transform.translation = translation_correction.translation_direction_adjust
                * animated_translation
                * translation_correction.translation_length_ratio;
        }
    }
}

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
                    // Translation tracks are always kept. Whether they are
                    // rescaled per frame depends on the
                    // `dynamic_translation_tracks` plugin flag
                    // (`rescale_full_bone_translations`); without it the root
                    // system still handles the root bone. The extra curves cost
                    // ~nothing at load; the system is the real cost.
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

/// Return all root joint nodes for a skin (there can be multiple).
fn find_root_joints<'a>(skin: &Skin<'a>) -> gltf::Node<'a> {
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
