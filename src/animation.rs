use std::{
    f32::consts::PI,
    path::{Path, PathBuf},
};

use ahash::{AHashMap, AHashSet};
use bevy::{
    animation::{animated_field, AnimationTargetId},
    ecs::intern::Internable,
    prelude::*,
};
use gltf::Skin;

use crate::{
    prelude::*,
    rigs::{BoneTranslationData, RigType, RootBone, RootBonePrevious},
    spawn::RelatedEntities,
    HumentityGlobalConfig,
};

#[derive(Resource, Deref, DerefMut)]
pub struct CharacterAnimationClips(
    AHashMap<RigType, AHashMap<&'static str, Handle<AnimationClip>>>,
);

/// This system (if enabled in the config) will adjust translation tracks in aniamtion clips
/// in realtime using data cached on the human config.
pub(crate) fn rescale_bone_translations(
    prefabs: Res<CharacterArchetypePrefabs>,
    humans: Query<(Entity, &CharacterShapeConfig), Without<FitSkeleton>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut transforms: Query<&mut Transform>,
) {
    for (entity, human) in humans {
        let ref_translations = &prefabs[human.prefab].rig.bone_translations;
        let BoneTranslationData::Full(shape_translations) = &human.bone_translations else {
            continue;
        };
        let rotation_deltas = &human.bone_delta_rotations;

        for child in children.iter_descendants(entity) {
            let Ok(name) = names.get(child) else { continue };
            let name = name.as_str();
            let Some(ref_trans) = ref_translations.get(name) else {
                continue;
            };
            let ref_trans = ref_trans.length();
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

/// This system (if enabled in the config) will adjust translation tracks in aniamtion clips
/// in realtime using data cached on the human config.  This one affects only the root bone.
pub(crate) fn rescale_root_bone_translation(
    prefabs: Res<CharacterArchetypePrefabs>,
    humans: Query<(&RelatedEntities, &CharacterShapeConfig), Without<FitSkeleton>>,
    mut transforms: Query<&mut Transform>,
) {
    for (related, human) in humans {
        let &root_bone = &prefabs[human.prefab].rig.bone_order[0];
        let BoneTranslationData::Root(shape_trans) = &human.bone_translations else {
            continue;
        };
        let ref_trans = &prefabs[human.prefab].rig.bone_translations;
        let Ok(mut root) = transforms.get_mut(related.root_bone) else {
            continue;
        };
        root.translation = root.translation * shape_trans.length() / ref_trans[root_bone].length();
    }
}

pub(crate) fn root_motion(
    mut humans: Query<(&RelatedEntities, &RootMotion, &mut Transform), With<CharacterShapeConfig>>,
    mut root_transforms: Query<
        (&mut Transform, &mut RootBonePrevious),
        (With<RootBone>, Without<CharacterShapeConfig>),
    >,
    players: Query<&AnimationPlayer>,
    time: Res<Time>,
) {
    for (related, root_motion, mut human_transform) in humans.iter_mut() {
        let Ok((mut root_bone_transform, mut previous)) =
            root_transforms.get_mut(related.root_bone)
        else {
            continue;
        };

        // Blending between clips causes issues due to different root motion behavior.
        // I think I would have to track changes at the level of individual clips.
        // For now, let's only apply root motion if our animation state isn't changing.
        let mut weights = vec![];
        let Ok(player) = players.get(related.rig) else {
            continue;
        };
        for (_i, a) in player.playing_animations() {
            weights.push(a.weight());
        }
        let mut skip = true;
        if previous.prev_weights.len() == weights.len() {
            skip = weights
                .iter()
                .enumerate()
                .any(|(i, v)| (*v - previous.prev_weights[i]).abs() > 1e-3);
        }

        previous.prev_weights = weights;

        // Check for exceptionally large offsets this frame, expected from reset of root position
        // when the clip loops back to the beginning
        let root = root_bone_transform.translation;
        let mut offset = root - previous.translation;
        if !root_motion.y_translate {
            offset.y = 0.;
        }
        let offset_sq = offset.length_squared();
        previous.translation = root;

        // This will skip root motion this frame.  Could cause some stutter.
        // Ideally we would add some small offset.  Might have to cache last frames offset.
        let t2 = time.delta_secs() * time.delta_secs();
        if !skip && offset_sq < 10. * t2 {
            human_transform.translation += offset;
        }

        // Reset root bone position
        if root_motion.y_translate {
            root_bone_transform.translation = Vec3::ZERO;
        } else {
            root_bone_transform.translation.x = 0.;
            root_bone_transform.translation.z = 0.;
        }

        // Do the same for yaw rotation
        if root_motion.yaw {
            // The Euler convention may be different for different rigs.
            // I think it depends on the roll on the root bone. This looks good for default rig.
            let (yaw, pitch, roll) = root_bone_transform.rotation.to_euler(EulerRot::YZX);
            let mut delta_yaw = yaw - previous.yaw;
            while delta_yaw > PI {
                delta_yaw -= 2.0 * PI;
            }
            while delta_yaw < -PI {
                delta_yaw += 2.0 * PI;
            }
            previous.yaw = yaw;

            if !skip && delta_yaw * delta_yaw < 1000. * t2 {
                human_transform.rotate_y(delta_yaw);
            }
            root_bone_transform.rotation = Quat::from_euler(EulerRot::YZX, 0.0, pitch, roll);
        }
    }
}

pub(crate) fn rebuild_animations(
    prefabs: Res<CharacterArchetypePrefabs>,
    mut clips_assets: ResMut<Assets<AnimationClip>>,
    mut commands: Commands,
    config: Res<HumentityGlobalConfig>,
) {
    let rig_types = prefabs
        .iter()
        .map(|(_, p)| p.rig.rig_type)
        .collect::<Vec<_>>();

    let mut rig_clips =
        AHashMap::<RigType, AHashMap<&'static str, Handle<AnimationClip>>>::default();
    for rig in rig_types.into_iter() {
        let glbs = prefabs
            .iter()
            .filter(|(_, p)| p.rig.rig_type == rig)
            .flat_map(|(_, &ref p)| p.rig.animation_glbs.iter())
            .collect::<Vec<_>>();

        let mut clip_handles = AHashMap::<&'static str, Handle<AnimationClip>>::new();
        for &glb in glbs {
            let path = PathBuf::from(glb);
            let clips = get_animation_clips(path, config.translation_tracks)
                .expect("Failed to retarget animation clips");
            let mut handles: AHashMap<&'static str, Handle<AnimationClip>> = AHashMap::default();
            for (name, clip) in clips.into_iter() {
                handles.insert(name, clips_assets.add(clip));
            }
            clip_handles.extend(handles);
        }
        rig_clips.insert(rig, clip_handles);
    }
    commands.insert_resource(CharacterAnimationClips(rig_clips));
    commands.set_state(HumentityLoadState::Ready);
}

/// Returns a tuple of HashMaps, one for (model space) rotations, the other for (bone space) translations
pub(crate) fn get_skeleton_transforms(
    world: &mut World,
    rig: RigType,
) -> Result<(AHashMap<&'static str, Quat>, AHashMap<&'static str, Vec3>), BevyError> {
    let config = world
        .get_resource::<HumentityPathsConfig>()
        .expect("Humentity not loaded");
    let mut path = config.core_assets_path.clone();
    match rig {
        RigType::Default => path = path.join("skeletons/default.glb"),
        RigType::Mixamo => path = path.join("skeletons/mixamo.glb"),
        RigType::GameEngine => path = path.join("skeletons/game_engine.glb"),
        //_ => unimplemented!("Add skeleton glb file for this skeleton")
    }
    let (document, ..) = gltf::import(path)?;
    if document.skins().len() > 1 {
        return Err(BevyError::from("More than one skin present in file"));
    };
    let Some(skin) = document.skins().next() else {
        return Err(BevyError::from("No skins available"));
    };
    let mut transforms = AHashMap::<&'static str, Transform>::default();
    let mut node_indices = AHashMap::<&'static str, usize>::default();

    // Get joint local transforms
    for joint in skin.joints() {
        let name = joint.name().expect("No name for bone in skeleton file?");
        node_indices.insert(NAME_INTERNER.intern(name).leak(), joint.index());
        let (pos, rot, scale) = joint.transform().decomposed();
        let transform = Transform {
            translation: Vec3::from_array(pos),
            rotation: Quat::from_array(rot).normalize(),
            scale: Vec3::from_array(scale),
        };
        transforms.insert(NAME_INTERNER.intern(name).leak(), transform);
    }

    // Convert to global transforms
    let mut global_transforms = AHashMap::<&'static str, Transform>::default();
    let root = &find_root_joints(&skin);
    compute_global_transform(
        root,
        &transforms,
        &mut global_transforms,
        Transform::IDENTITY,
    )?;

    Ok((
        global_transforms
            .iter()
            .map(|(&n, t)| (n, t.rotation))
            .collect::<AHashMap<&'static str, Quat>>(),
        transforms
            .iter()
            .map(|(&n, t)| (n, t.translation))
            .collect::<AHashMap<&'static str, Vec3>>(),
    ))
}

pub(crate) fn get_animation_clips(
    path: impl AsRef<Path>,
    translation_tracks: TranslationTracks,
) -> Result<AHashMap<&'static str, AnimationClip>, BevyError> {
    let (document, buffers, _) = gltf::import(path)?;
    if document.skins().len() > 1 {
        return Err(BevyError::from("More than one skin present in file"));
    };
    let Some(skin) = document.skins().next() else {
        return Err(BevyError::from("No skins available"));
    };
    let mut transforms = AHashMap::<&'static str, Transform>::default();
    let mut node_indices = AHashMap::<&'static str, usize>::default();

    // Get joint local transforms
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

    // Convert to global transforms
    let mut global_transforms = AHashMap::default();
    let root = &find_root_joints(&skin);
    compute_global_transform(
        root,
        &transforms,
        &mut global_transforms,
        Transform::IDENTITY,
    )?;

    // Get bone paths
    let joint_targets = build_joint_paths(root);

    // Output clips
    let mut new_clips = AHashMap::default();

    // Build new clips
    for animation in document.animations() {
        let mut clip = AnimationClip::default();
        let clip_name = animation
            .name()
            .ok_or(BevyError::from("Animation clip has no name"))?;

        for channel in animation.channels() {
            // Get input values (t1, t2, ...)
            let sampler = channel.sampler();
            let input_accessor = sampler.input();
            let input_view = input_accessor
                .view()
                .ok_or(BevyError::from("Failed to get input_view for animation"))?;
            let buffer = &buffers[input_view.buffer().index()];

            let start = input_accessor.offset() + input_view.offset();
            let end = start + input_accessor.count() * std::mem::size_of::<f32>();
            let data = &buffer[start..end];
            let times: &[f32] = bytemuck::cast_slice(data);
            let times = times.iter().map(|x| *x).collect::<Vec<_>>();

            // Get output values (pos1, pos2, ...) or (quat1, quat2, ...)
            let output_accessor = sampler.output();
            let output_view = output_accessor
                .view()
                .ok_or(BevyError::from("Missing output view"))?;
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
                _ => continue, // Morph target weights
            };
            let end =
                start + output_accessor.count() * floats_per_element * std::mem::size_of::<f32>();
            let values = &buffer[start..end];
            let floats: &[f32] = bytemuck::cast_slice(values);

            // Add new curve to new clip
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

                    let values: Vec<Vec3> = floats
                        .chunks(floats_per_element)
                        .map(|chunk| Vec3::from_array([chunk[0], chunk[1], chunk[2]]))
                        .collect();
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(
                            animated_field!(Transform::translation),
                            AnimatableKeyframeCurve::new(times.into_iter().zip(values.into_iter()))
                                .expect("Failed to construct curve"),
                        ),
                    );
                }
                gltf::animation::Property::Scale => {
                    let values: Vec<Vec3> = floats
                        .chunks(floats_per_element)
                        .map(|chunk| Vec3::from_array([chunk[0], chunk[1], chunk[2]]))
                        .collect();
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(
                            animated_field!(Transform::scale),
                            AnimatableKeyframeCurve::new(times.into_iter().zip(values.into_iter()))
                                .expect("Failed to construct curve"),
                        ),
                    );
                }
                gltf::animation::Property::Rotation => {
                    let values: Vec<Quat> = floats
                        .chunks(floats_per_element)
                        .map(|chunk| {
                            Quat::from_array([chunk[0], chunk[1], chunk[2], chunk[3]]).normalize()
                        })
                        .collect();
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(
                            animated_field!(Transform::rotation),
                            AnimatableKeyframeCurve::new(times.into_iter().zip(values.into_iter()))
                                .expect("Failed to construct curve"),
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
    let name = root.name().ok_or(BevyError::from("No name for bone"))?;
    let name = NAME_INTERNER.intern(name).leak();
    let local = local_transforms
        .get(&name)
        .ok_or(BevyError::from("Missing local transform"))?;
    let global = Transform::from_matrix(parent_global.to_matrix() * local.to_matrix());
    global_transforms.insert(name, global);

    for child in root.children() {
        compute_global_transform(&child, local_transforms, global_transforms, global)?;
    }
    Ok(())
}

/// Builds a map of joint name → full path (from root to that joint)
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
        .filter(|j| !seen_as_child.contains(&j.index()))
        .last()
        .expect("Unable to find root node")
}
