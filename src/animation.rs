use std::path::{Path, PathBuf};
use bevy::{animation::{animated_field, AnimationTargetId}, ecs::intern::Internable, prelude::*};
use ahash::{AHashMap, AHashSet};
use gltf::Skin;

use crate::{HumentityGlobalConfig, prelude::*, rigs::RigType};

#[derive(Resource, Deref, DerefMut)]
pub struct CharacterAnimationClips(AHashMap::<&'static str, Handle<AnimationClip>>);

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
        let shape_translations = &human.bone_translations;
        for child in children.iter_descendants(entity) {
            let Ok(name) = names.get(child) else { continue };
            let name = name.as_str();
            let Some(ref_trans) = ref_translations.get(name) else { continue };
            let ref_trans = ref_trans.length();
            if ref_trans < 1e-3 { continue }
            let Some(shape_trans) = shape_translations.get(name) else { continue };
            let Ok(mut transform) = transforms.get_mut(child) else { continue };
            let rot = if let Some(rot) = human.bone_delta_rotations.get(name) { rot } else { &Quat::IDENTITY };
            transform.translation = rot * transform.translation * shape_trans.length() / ref_trans;
        }
    }
}

pub(crate) fn rebuild_animations(
    prefabs: Res<CharacterArchetypePrefabs>,
    mut clips_assets: ResMut<Assets<AnimationClip>>,
    mut commands: Commands,
    config: Res<HumentityGlobalConfig>,
) {
    let glbs = prefabs
        .iter()
        .flat_map(|(_, &ref p)| p.rig.animation_glbs.iter())
        .collect::<Vec<_>>();

    let mut clip_handles = AHashMap::<&'static str, Handle<AnimationClip>>::new();
    for &glb in glbs {
        let path = PathBuf::from(glb);
        let clips = get_animation_clips(path, config.translation_animation_tracks)
            .expect("Failed to retarget animation clips");
        let mut handles: AHashMap<&'static str, Handle<AnimationClip>> = AHashMap::default();
        for (name, clip) in clips.into_iter() {
            handles.insert(name, clips_assets.add(clip));
        }
        clip_handles.extend(handles);
    }
    commands.insert_resource(CharacterAnimationClips(clip_handles));
    commands.set_state(HumentityLoadState::Ready);
}

/// Returns a tuple of HashMaps, one for (model space) rotations, the other for (bone space) translations
pub(crate) fn get_skeleton_transforms(
    world: &mut World, rig: RigType
) -> Result<(AHashMap<&'static str, Quat>, AHashMap<&'static str, Vec3>), BevyError> {
    let config = world.get_resource::<HumentityPathsConfig>()
        .expect("Humentity not loaded");
    let mut path = config.core_assets_path.clone();
    match rig {
        RigType::Default => path = path.join("skeletons/default.glb"),
        RigType::Mixamo => path = path.join("skeletons/mixamo.glb"),
        RigType::GameEngine => path = path.join("skeletons/game_engine.glb"),
        //_ => unimplemented!("Add skeleton glb file for this skeleton")
    }
    let (document, ..) = gltf::import(path)?;
    if document.skins().len() > 1 { return Err(BevyError::from("More than one skin present in file")) };
    let Some(skin) = document.skins().next() else { return Err(BevyError::from("No skins available")) };
    let mut transforms = AHashMap::<&'static str, Transform>::default();
    let mut node_indices = AHashMap::<&'static str, usize>::default();

    // Get joint local transforms
    for joint in skin.joints() {
        let name = joint.name().expect("No name for bone in skeleton file?");
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
    let mut global_transforms = AHashMap::<&'static str, Transform>::default();
    let root = &find_root_joints(&skin);
    compute_global_transform(root, &transforms, &mut global_transforms, Transform::IDENTITY)?;

    Ok(
        (
            global_transforms
                .iter()
                .map(|(&n, t)| (n, t.rotation))
                .collect::<AHashMap<&'static str, Quat>>(),
            transforms
                .iter()
                .map(|(&n, t)| (n, t.translation))
                .collect::<AHashMap<&'static str, Vec3>>()
        )
    )
}

pub(crate) fn get_animation_clips(
    path: impl AsRef<Path>,
    translation_tracks: bool,
) -> Result<AHashMap<&'static str, AnimationClip>, BevyError> {
    let (document, buffers, _) = gltf::import(path)?;
    if document.skins().len() > 1 { return Err(BevyError::from("More than one skin present in file")) };
    let Some(skin) = document.skins().next() else { return Err(BevyError::from("No skins available")) };
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
    compute_global_transform(root, &transforms, &mut global_transforms, Transform::IDENTITY)?;

    // Get bone paths
    let joint_targets = build_joint_paths(&find_root_joints(&skin));

    // Output clips
    let mut new_clips = AHashMap::default();

    // Build new clips
    for animation in document.animations() {
        let mut clip = AnimationClip::default();
        let clip_name = animation.name()
            .ok_or(BevyError::from("Animation clip has no name"))?;

        for channel in animation.channels() {
            // Get input values (t1, t2, ...)
            let sampler = channel.sampler();
            let input_accessor = sampler.input();
            let input_view = input_accessor.view()
                .ok_or(BevyError::from("Failed to get input_view for animation"))?;
            let buffer = &buffers[input_view.buffer().index()];

            let start = input_accessor.offset() + input_view.offset();
            let end = start + input_accessor.count() * std::mem::size_of::<f32>();
            let data = &buffer[start..end];
            let times: &[f32] = bytemuck::cast_slice(data);
            let times = times.iter().map(|x| *x).collect::<Vec<_>>();

            // Get output values (pos1, pos2, ...) or (quat1, quat2, ...)
            let output_accessor = sampler.output();
            let output_view = output_accessor.view()
                .ok_or(BevyError::from("Missing output view"))?;
            let buffer = &buffers[output_view.buffer().index()];

            let target = channel.target();
            let target_property = target.property(); 
            let target_node = target.node();
            let target_name = target_node.name()
                .expect("Failed to match node name");
            let target_name = Name::new(NAME_INTERNER.intern(target_name).leak());
            let target_id = AnimationTargetId::from_names(joint_targets[&target_name].iter());

            let start = output_view.offset() + output_accessor.offset();
            let floats_per_element = match target_property {
                gltf::animation::Property::Translation | gltf::animation::Property::Scale => 3,
                gltf::animation::Property::Rotation => 4,
                _ => { continue } // Morph target weights
            };
            let end = start + output_accessor.count() * floats_per_element * std::mem::size_of::<f32>();
            let values = &buffer[start..end];
            let floats: &[f32] = bytemuck::cast_slice(values);

            // Add new curve to new clip
            match target_property {
                gltf::animation::Property::Translation => {
                    if !translation_tracks { continue };
                    let values: Vec<Vec3> = floats
                        .chunks(floats_per_element)
                        .map(|chunk| Vec3::from_array([chunk[0], chunk[1], chunk[2]]))
                        .collect();
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(
                            animated_field!(Transform::translation),
                            AnimatableKeyframeCurve::new(times.into_iter().zip(values.into_iter()))
                                .expect("Failed to construct curve")
                        )
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
                                .expect("Failed to construct curve")
                        )
                    );
                }
                gltf::animation::Property::Rotation => {
                    let values: Vec<Quat> = floats
                        .chunks(floats_per_element)
                        .map(|chunk| Quat::from_array([chunk[0], chunk[1], chunk[2], chunk[3]]))
                        .collect();
                    clip.add_curve_to_target(
                        target_id,
                        AnimatableCurve::new(
                            animated_field!(Transform::rotation),
                            AnimatableKeyframeCurve::new(times.into_iter().zip(values.into_iter()))
                                .expect("Failed to construct curve")
                        )
                    );
                }
                _ => { continue }
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
    let local = local_transforms.get(&name).ok_or(BevyError::from("Missing local transform"))?;
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
        .map(|(k, v)|
            ( 
                Name::new(NAME_INTERNER.intern(&k).leak()),
                v
                    .into_iter()
                    .map(|n| Name::new(NAME_INTERNER.intern(&n).leak()))
                    .collect::<Vec<_>>()
            )
        )
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