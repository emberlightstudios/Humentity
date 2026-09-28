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
    animation::{BakedBoneCorrection, ShapeBakedCorrections, get_animation_clips_from_bytes},
};

/// Root-only clips: rotation and scale on every bone, translation on the root
/// only. Non-root translations are dropped at import; put
/// [`RootOnlyRetargeting`](crate::animation::RootOnlyRetargeting) on the
/// character so the root fix runs (its only runtime cost).
#[derive(Asset, TypePath, Clone)]
pub struct RetargetedAnimationAsset {
    pub clips: AHashMap<&'static str, Handle<AnimationClip>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TypePath)]
pub struct RetargetedAnimationSettings;

#[derive(Default, TypePath)]
pub struct RetargetedAnimationAssetLoader;

impl AssetLoader for RetargetedAnimationAssetLoader {
    type Asset = RetargetedAnimationAsset;
    type Settings = RetargetedAnimationSettings;
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

        Ok(RetargetedAnimationAsset {
            clips: clip_handles,
        })
    }
}

/// Shape-baked clips: every translation track (root + bones) is rewritten to
/// one shape at import, so no retarget system runs and no marker is needed.
/// Exact for that shape, wrong for any other. Bake once per shape with
/// [`shape_baked_corrections_for_shape`](crate::animation::shape_baked_corrections_for_shape)
/// (or a code path that feeds [`ShapeBakedAnimationSettings`]), passing the
/// result as loader settings via `load_builder().with_settings(...)`.
/// Root is owned here: XZ is zeroed and Y offsets are rescaled from the
/// fitted bind pose. Never put a retarget marker on characters playing baked
/// clips, or the root scale applies twice.
#[derive(Asset, TypePath, Clone)]
pub struct ShapeBakedAnimationAsset {
    pub clips: AHashMap<&'static str, Handle<AnimationClip>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TypePath)]
pub struct ShapeBakedAnimationSettings {
    /// Fitted corrections for the target shape. Empty means identity: curves
    /// pass through with only root XZ zeroed.
    #[serde(default)]
    pub shape_corrections: ShapeBakedCorrections,
}

#[derive(Default, TypePath)]
pub struct ShapeBakedAnimationAssetLoader;

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
        let (document, buffers, _) = gltf::import_slice(&bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;
        if document.skins().len() > 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "More than one skin present in file",
            ));
        }
        let Some(skin) = document.skins().next() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "No skins available",
            ));
        };

        let root_node = crate::animation::find_root_joints(&skin);
        let joint_targets = crate::animation::build_joint_paths(&root_node);
        let root_bone_name: &str = root_node.name().unwrap_or("");

        let bone_corrections: AHashMap<&str, &BakedBoneCorrection> = settings
            .shape_corrections
            .baked_bone_corrections
            .iter()
            .map(|correction| (correction.bone_name.as_str(), correction))
            .collect();

        let mut clip_handles = AHashMap::default();
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
                    gltf::animation::Property::Translation
                    | gltf::animation::Property::Scale => 3,
                    gltf::animation::Property::Rotation => 4,
                    _ => continue,
                };
                let start = output_view.offset() + output_accessor.offset();
                let end = start
                    + output_accessor.count()
                        * floats_per_element
                        * std::mem::size_of::<f32>();
                let Some(output_bytes) = buffer.get(start..end) else {
                    continue;
                };
                let floats: &[f32] = bytemuck::cast_slice(output_bytes);
                match target_property {
                    gltf::animation::Property::Translation => {
                        if animated_bone_name == root_bone_name {
                            let baked_root_scale =
                                settings.shape_corrections.baked_root_scale;
                            let baked_bind_y =
                                settings.shape_corrections.baked_bind_pose_y;
                            let reference_bind_y = settings
                                .shape_corrections
                                .reference_bind_pose_y;
                            let baked = times.into_iter().zip(
                                floats.chunks(floats_per_element).map(move |chunk| {
                                    let reference_translation =
                                        Vec3::from_array([chunk[0], chunk[1], chunk[2]]);
                                    let root_offset_y =
                                        reference_translation.y - reference_bind_y;
                                    Vec3::new(
                                        0.0,
                                        if root_offset_y.abs() > 1e-3 {
                                            baked_bind_y + root_offset_y * baked_root_scale
                                        } else {
                                            baked_bind_y
                                        },
                                        0.0,
                                    )
                                }),
                            );
                            let Ok(curve) = AnimatableKeyframeCurve::new(baked) else {
                                continue;
                            };
                            clip.add_curve_to_target(
                                target_id,
                                AnimatableCurve::new(
                                    animated_field!(Transform::translation),
                                    curve,
                                ),
                            );
                        } else {
                            let Some(correction) =
                                bone_corrections.get(animated_bone_name)
                            else {
                                continue;
                            };
                            let ratio = correction.translation_length_ratio;
                            let direction = correction.translation_direction_adjust;
                            let baked = times.into_iter().zip(
                                floats.chunks(floats_per_element).map(move |chunk| {
                                    // Same math as dynamic: direction * raw * ratio.
                                    direction * Vec3::from_array([
                                        chunk[0], chunk[1], chunk[2],
                                    ]) * ratio
                                }),
                            );
                            let Ok(curve) = AnimatableKeyframeCurve::new(baked) else {
                                continue;
                            };
                            clip.add_curve_to_target(
                                target_id,
                                AnimatableCurve::new(
                                    animated_field!(Transform::translation),
                                    curve,
                                ),
                            );
                        }
                    }
                    gltf::animation::Property::Rotation => {
                        let baked = times.into_iter().zip(
                            floats.chunks(floats_per_element).map(|chunk| {
                                Quat::from_array([chunk[0], chunk[1], chunk[2], chunk[3]])
                                    .normalize()
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
            // Bevy IDs a labeled sub-asset from file path plus label only, so
            // filing under the plain clip name would share an ID with the
            // root-only loader's clip from this file and overwrite it (this
            // happened: the root-only baby played baked curves, got fixed
            // twice, and sank to the floor). The map key stays the clip name.
            let baked_label: &'static str =
                NAME_INTERNER.intern(&format!("{clip_name}.baked")).leak();
            let handle = load_context.add_loaded_labeled_asset(
                baked_label,
                LoadedAsset::new_with_dependencies(clip),
            );
            clip_handles.insert(leaked_name, handle);
        }

        Ok(ShapeBakedAnimationAsset {
            clips: clip_handles,
        })
    }
}

