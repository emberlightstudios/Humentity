use crate::{rigs::SkeletonRootBone, NAME_INTERNER};
use ahash::{AHashMap, AHashSet};
use bevy::{
    animation::{animated_field, AnimationTargetId},
    ecs::intern::Internable,
    prelude::*,
};
use gltf::Skin;
use serde::{Deserialize, Serialize};

/// humentity's PostUpdate bone-authoritative pass that runs after Bevy's
/// `AnimationSystems` and before `TransformSystems::Propagate`. It covers both
/// animation post-processing (`rescale_root_bone_translation`) and ragdoll
/// bone→skeleton sync (`sync_bones_to_ragdoll`). Add your own systems with
/// `.after(HumentitySkeletonSystemSet)` to run after this pass.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HumentitySkeletonSystemSet;

/// Which translation tracks should be kept on animation clips
#[derive(Copy, Clone, Default, Debug, Serialize, Deserialize)]
pub enum TranslationTracks {
    #[default]
    Root,
    //Full,
    None,
}

/*
/// This system (if enabled in the config) will adjust translation tracks in aniamtion clips
/// in realtime using data cached on the human config.
#[allow(dead_code)]
pub(crate) fn rescale_bone_translations(
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    humans: Query<(Entity, &CharacterShape), With<SkeletonsReady>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut transforms: Query<&mut Transform>,
    rig_data: Res<RigData>,
) {
    for (entity, human) in humans {
        let Some(asset) = shape_assets.get(&human.0) else {
            continue;
        };
        let Some(template) = templates.get(&asset.template) else {
            continue;
        };
        let rig_type = &template.rig;
        let rig_spec = &rig_data[rig_type];
        let ref_translations = &rig_spec.reference_rig.local_bindpose;
        let BoneTranslationData::Full(shape_translations) = &asset.bone_translations else {
            continue;
        };
        let rotation_deltas = &asset.bone_delta_rotations;

        for child in children.iter_descendants(entity) {
            let Ok(name) = names.get(child) else { continue };
            let name = name.as_str();
            let Some(ref_trans) = ref_translations.get(name) else {
                continue;
            };
            let ref_trans = ref_trans.translation.length();
            if ref_trans < 1e-3 {
                continue;
            }
            let Some(shape_trans) = shape_translations.get(name) else {
                continue;
            };
            let Ok(mut transform) = transforms.get_mut(child) else {
                continue;
            };
            let rot = if let Some(rot) = rotation_deltas.get(name) {
                rot
            } else {
                &Quat::IDENTITY
            };
            transform.translation = rot * transform.translation * shape_trans.length() / ref_trans;
        }
    }
}
*/

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
            t.translation.x = 0.;
            t.translation.z = 0.;
        }
    }
}

pub(crate) fn get_animation_clips_from_bytes(
    bytes: &[u8],
    translation_tracks: TranslationTracks,
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
                    if matches!(translation_tracks, TranslationTracks::None) {
                        continue;
                    };
                    if matches!(translation_tracks, TranslationTracks::Root)
                        && target_name.as_str() != root.name().unwrap()
                    {
                        continue;
                    };

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
