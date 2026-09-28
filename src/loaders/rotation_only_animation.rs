use ahash::AHashMap;
use bevy::{
    animation::{AnimationTargetId, animated_field},
    asset::{AssetLoader, LoadContext, LoadedAsset, io::Reader},
    ecs::intern::Internable,
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::{
    NAME_INTERNER,
    animation::{BakedBoneCorrection, ShapeBakedCorrections},
};
use ahash::AHashSet;
use gltf::Skin;

/// Root-only clips: rotation and scale on every bone, translation on the root
/// only. Non-root translations are dropped at import; put
/// [`RootOnlyRetargeting`](crate::animation::RootOnlyRetargeting) on the
/// character so the root fix runs (its only runtime cost).
#[derive(Asset, TypePath, Clone)]
pub struct RotationOnlyAnimationAsset {
    pub clips: AHashMap<&'static str, Handle<AnimationClip>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TypePath)]
pub struct RotationOnlyAnimationSettings;

#[derive(Default, TypePath)]
pub struct RotationOnlyAnimationAssetLoader;

impl AssetLoader for RotationOnlyAnimationAssetLoader {
    type Asset = RotationOnlyAnimationAsset;
    type Settings = RotationOnlyAnimationSettings;
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let clips = get_animation_clips_from_bytes(&bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

        let mut clip_handles = AHashMap::default();
        for (name, clip) in clips {
            // Labels share one namespace per file across every loader, so
            // this plain clip name must never equal a baked loader label for
            // the same file, or one clip silently replaces the other.
            let handle = load_context
                .add_loaded_labeled_asset(name, LoadedAsset::new_with_dependencies(clip));
            clip_handles.insert(name, handle);
        }

        Ok(RotationOnlyAnimationAsset {
            clips: clip_handles,
        })
    }
}

/// Shape-baked clips: every translation track (root + bones) is rewritten to
/// each listed shape at import, so no retarget system runs and no marker is
/// needed. Exact for those shapes, wrong for any other. List every shape up
/// front with [`shape_baked_corrections_for_shape`](crate::animation::shape_baked_corrections_for_shape)
/// results passed as loader settings via `load_builder().with_settings(...)`.
/// Root is owned here: XZ is zeroed and Y offsets are rescaled from the
/// fitted bind pose. Never put a retarget marker on characters playing baked
/// clips, or the root scale applies twice.
///
/// One file yields one outer asset holding every shape's clips (`{clip}.{suffix}`
/// labels, legacy `{clip}.baked` when the suffix is empty), so loading the same
/// file once serves all shapes with no outer path-dedup collision. Adding a
/// shape later means reloading the file with the longer list.
#[derive(Asset, TypePath, Clone)]
pub struct ShapeBakedAnimationAsset {
    pub clips: AHashMap<&'static str, Handle<AnimationClip>>,
}

/// One shape's entry in a multi-shape bake: clips land under
/// `{clip}.{shape_suffix}` labels (`{clip}.baked` when empty).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShapeBakeRequest {
    pub shape_suffix: String,
    pub shape_corrections: ShapeBakedCorrections,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TypePath)]
pub struct ShapeBakedAnimationSettings {
    /// Every shape to bake in this load. Empty fails the load: `Default`
    /// (required by Bevy's `Settings`) is an explicit missing value, never
    /// silent identity — with no corrections the loader would flatten root Y
    /// to 0 and drop every bone track.
    #[serde(default)]
    pub shape_bakes: Vec<ShapeBakeRequest>,
}

#[derive(Default, TypePath)]
pub struct ShapeBakedAnimationAssetLoader;

/// Bake every clip in raw glTF `bytes` to `shape_corrections`, returning
/// plain clips keyed by clip name. Single-shape helper for one-off code bakes
/// with `Assets::add`; the loader parses once and runs
/// `bake_shape_clips_from_document` per listed shape instead, so multi-shape
/// loads go through settings rather than repeated calls here.
pub fn bake_shape_clips_from_bytes(
    bytes: &[u8],
    shape_corrections: &ShapeBakedCorrections,
) -> Result<AHashMap<&'static str, AnimationClip>, std::io::Error> {
    let invalid = |message: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
    let (document, buffers, _) = gltf::import_slice(bytes).map_err(|err| invalid(&err.to_string()))?;
    if document.skins().len() > 1 {
        return Err(invalid("More than one skin present in file"));
    }
    let Some(skin) = document.skins().next() else {
        return Err(invalid("No skins available"));
    };
    let root_node = find_root_joints(&skin);
    let joint_targets = build_joint_paths(&root_node);
    let root_bone_name: &str = root_node.name().unwrap_or("");
    let bone_corrections: AHashMap<&str, &BakedBoneCorrection> = shape_corrections
        .baked_bone_corrections
        .iter()
        .map(|correction| (correction.bone_name.as_str(), correction))
        .collect();
    bake_shape_clips_from_document(&document, &buffers, &joint_targets, root_bone_name, shape_corrections, &bone_corrections)
}

fn bake_shape_clips_from_document(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    joint_targets: &AHashMap<Name, Vec<Name>>,
    root_bone_name: &str,
    shape_corrections: &ShapeBakedCorrections,
    bone_corrections: &AHashMap<&str, &BakedBoneCorrection>,
) -> Result<AHashMap<&'static str, AnimationClip>, std::io::Error> {
    let mut baked_clips = AHashMap::default();
    for animation in document.animations() {
        let Some(clip_name) = animation.name() else {
            continue;
        };
        let mut clip = AnimationClip::default();
        for channel in animation.channels() {
            let sampler = channel.sampler();
            let input_accessor = sampler.input();
            let Some(input_view) = input_accessor.view() else {
                continue;
            };
            let buffer = &buffers[input_view.buffer().index()];
            let start = input_accessor.offset() + input_view.offset();
            let end = start + input_accessor.count() * std::mem::size_of::<f32>();
            let Some(input_bytes) = buffer.get(start..end) else {
                continue;
            };
            let times: Vec<f32> = bytemuck::cast_slice(input_bytes).to_vec();
            let output_accessor = sampler.output();
            let Some(output_view) = output_accessor.view() else {
                continue;
            };
            let buffer = &buffers[output_view.buffer().index()];
            let target = channel.target();
            let target_property = target.property();
            let target_node = target.node();
            let Some(target_name) = target_node.name() else {
                continue;
            };
            let animated_bone_name: &'static str = NAME_INTERNER.intern(target_name).leak();
            let target_key = Name::new(animated_bone_name);
            let Some(joint_path) = joint_targets.get(&target_key) else {
                continue;
            };
            let target_id = AnimationTargetId::from_names(joint_path.iter());
            let floats_per_element = match target_property {
                gltf::animation::Property::Translation | gltf::animation::Property::Scale => 3,
                gltf::animation::Property::Rotation => 4,
                _ => continue,
            };
            let start = output_view.offset() + output_accessor.offset();
            let end = start
                + output_accessor.count() * floats_per_element * std::mem::size_of::<f32>();
            let Some(output_bytes) = buffer.get(start..end) else {
                continue;
            };
            let floats: &[f32] = bytemuck::cast_slice(output_bytes);
            match target_property {
                gltf::animation::Property::Translation => {
                    if animated_bone_name == root_bone_name {
                        let baked = bake_root_translation(
                            &times,
                            floats,
                            floats_per_element,
                            shape_corrections,
                        );
                        let Ok(curve) = AnimatableKeyframeCurve::new(baked) else {
                            continue;
                        };
                        clip.add_curve_to_target(
                            target_id,
                            AnimatableCurve::new(animated_field!(Transform::translation), curve),
                        );
                    } else {
                        let Some(correction) = bone_corrections.get(animated_bone_name) else {
                            continue;
                        };
                        let baked = bake_bone_translation(
                            &times,
                            floats,
                            floats_per_element,
                            correction,
                        );
                        let Ok(curve) = AnimatableKeyframeCurve::new(baked) else {
                            continue;
                        };
                        clip.add_curve_to_target(
                            target_id,
                            AnimatableCurve::new(animated_field!(Transform::translation), curve),
                        );
                    }
                }
                gltf::animation::Property::Rotation => {
                    let baked = times.into_iter().zip(
                        floats.chunks(floats_per_element).map(|chunk| {
                            Quat::from_array([chunk[0], chunk[1], chunk[2], chunk[3]]).normalize()
                        }),
                    );
                    let Ok(curve) = AnimatableKeyframeCurve::new(baked) else {
                        continue;
                    };
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(animated_field!(Transform::rotation), curve),
                    );
                }
                gltf::animation::Property::Scale => {
                    let baked = times.into_iter().zip(
                        floats.chunks(floats_per_element).map(|chunk| {
                            Vec3::from_array([chunk[0], chunk[1], chunk[2]])
                        }),
                    );
                    let Ok(curve) = AnimatableKeyframeCurve::new(baked) else {
                        continue;
                    };
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(animated_field!(Transform::scale), curve),
                    );
                }
                _ => continue,
            }
        }
        let leaked_name: &'static str = NAME_INTERNER.intern(clip_name).leak();
        baked_clips.insert(leaked_name, clip);
    }
    Ok(baked_clips)
}

fn bake_root_translation(
    times: &[f32],
    floats: &[f32],
    floats_per_element: usize,
    shape_corrections: &ShapeBakedCorrections,
) -> Vec<(f32, Vec3)> {
    let baked_root_scale = shape_corrections.baked_root_scale;
    let baked_bind_y = shape_corrections.baked_bind_pose_y;
    let reference_bind_y = shape_corrections.reference_bind_pose_y;
    times
        .iter()
        .copied()
        .zip(floats.chunks(floats_per_element).map(move |chunk| {
            let reference_translation = Vec3::from_array([chunk[0], chunk[1], chunk[2]]);
            let root_offset_y = reference_translation.y - reference_bind_y;
            Vec3::new(
                0.0,
                if root_offset_y.abs() > 1e-3 {
                    baked_bind_y + root_offset_y * baked_root_scale
                } else {
                    baked_bind_y
                },
                0.0,
            )
        }))
        .collect()
}

fn bake_bone_translation(
    times: &[f32],
    floats: &[f32],
    floats_per_element: usize,
    correction: &BakedBoneCorrection,
) -> Vec<(f32, Vec3)> {
    let ratio = correction.translation_length_ratio;
    let direction = correction.translation_direction_adjust;
    times
        .iter()
        .copied()
        .zip(floats.chunks(floats_per_element).map(move |chunk| {
            // Same math as dynamic: direction * raw * ratio.
            direction * Vec3::from_array([chunk[0], chunk[1], chunk[2]]) * ratio
        }))
        .collect()
}

impl AssetLoader for ShapeBakedAnimationAssetLoader {
    type Asset = ShapeBakedAnimationAsset;
    type Settings = ShapeBakedAnimationSettings;
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        if settings.shape_bakes.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "ShapeBakedAnimationSettings.shape_bakes is empty: list every shape with its corrections from shape_baked_corrections_for_shape via load_builder().with_settings(...)",
            ));
        };
        // Parse once, bake once per listed shape: one outer load serves all
        // shapes, so the asset server never dedups a second shape onto the
        // first shape's bake.
        let invalid =
            |message: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
        let (document, buffers, _) =
            gltf::import_slice(&bytes).map_err(|err| invalid(&err.to_string()))?;
        if document.skins().len() > 1 {
            return Err(invalid("More than one skin present in file"));
        }
        let Some(skin) = document.skins().next() else {
            return Err(invalid("No skins available"));
        };
        let root_node = find_root_joints(&skin);
        let joint_targets = build_joint_paths(&root_node);
        let root_bone_name: &str = root_node.name().unwrap_or("");
        let mut clip_handles = AHashMap::default();
        for bake in &settings.shape_bakes {
            let shape_corrections = &bake.shape_corrections;
            let bone_corrections: AHashMap<&str, &BakedBoneCorrection> = shape_corrections
                .baked_bone_corrections
                .iter()
                .map(|correction| (correction.bone_name.as_str(), correction))
                .collect();
            let baked_clips = bake_shape_clips_from_document(
                &document,
                &buffers,
                &joint_targets,
                root_bone_name,
                shape_corrections,
                &bone_corrections,
            )?;
            for (clip_name, clip) in baked_clips {
                // Bevy IDs a labeled sub-asset from file path plus label only,
                // so filing under the plain clip name would share an ID with
                // the root-only loader's clip from this file and overwrite it
                // (this happened: the root-only baby played baked curves, got
                // fixed twice, and sank to the floor). The suffix keeps shapes
                // apart within this one load: `Idle-loop.baby` vs
                // `Idle-loop.adult`. Map keys stay `(clip, suffix)` so every
                // shape's clips are reachable from the single outer asset.
                let suffix = if bake.shape_suffix.is_empty() {
                    "baked".to_string()
                } else {
                    bake.shape_suffix.clone()
                };
                let label: &'static str = NAME_INTERNER
                    .intern(&format!("{clip_name}.{suffix}"))
                    .leak();
                let handle = load_context.add_loaded_labeled_asset(
                    label,
                    LoadedAsset::new_with_dependencies(clip),
                );
                clip_handles.insert(label, handle);
            }
        }

        Ok(ShapeBakedAnimationAsset {
            clips: clip_handles,
        })
    }
}
/// Root-only clip import: keeps rotation and scale tracks on every bone, but
/// translation tracks on the root bone only. Non-root translations are dropped
/// at import, so the only runtime cost is the root fix on characters carrying
/// [`RootOnlyRetargeting`](crate::animation::RootOnlyRetargeting).
/// Rotation-driven clips (locomotion, idle) look right; clips whose motion
/// lives in bone translations lose it.
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
            let target_name = target_node.name().unwrap_or_else(|| {
                panic!(
                    "clip channel targets glTF node #{} with no name: every animated node must be named after its joint (check the armature in Blender)",
                    target_node.index()
                )
            });
            let target_name = Name::new(NAME_INTERNER.intern(target_name).leak());
            let Some(joint_path) = joint_targets.get(&target_name) else {
                panic!(
                    "clip channel targets joint '{}' which is not under the skin root: clips must be authored on the reference rig hierarchy (remove extra armatures from the glTF)",
                    target_name.as_str(),
                )
            };
            let target_id = AnimationTargetId::from_names(joint_path.iter());
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
    let name = node.name().unwrap_or_else(|| {
        panic!(
            "clip glTF node #{} has no name: every joint targeted by an animation channel must be named (check the armature in Blender)",
            node.index()
        )
    }).to_string();
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
    joints.into_iter().rfind(|j| !seen_as_child.contains(&j.index())).unwrap_or_else(|| {
        panic!(
            "clip glTF skin has {} joint(s) but none is unparented: at least one joint must not appear as another joint's child (check the armature hierarchy in Blender)",
            joint_indices.len(),
        )
    })
}

