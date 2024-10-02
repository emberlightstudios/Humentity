use bevy::{
    prelude::*,
    gltf::Gltf,
};
use std::{
    collections::HashMap,
    fs::read_dir,
    path::PathBuf,
};
use crate::{
    HumentityGlobalConfig,
    LoadingPhase,
    LoadingState,
    RigType,
};

#[allow(dead_code)]
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct AnimationLibrarySettings {
    pub paths: Vec<PathBuf>,
    pub rig_type: RigType,
    //pub added_root_bone: bool,
}

impl Default for AnimationLibrarySettings {
    fn default() -> Self {
        AnimationLibrarySettings {
            paths: Vec::<PathBuf>::new(),
            rig_type: RigType::Mixamo,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct AnimationLibrary(pub HashMap<String, Handle<AnimationClip>>);

/*-----------+
 | Resources |
 +-----------*/
#[derive(Resource, Debug)]
pub struct AnimationLibrarySet{
    gltf_handles: HashMap<String, Handle<Gltf>>,
    pub libraries: HashMap<String, AnimationLibrary>,
}

impl FromWorld for AnimationLibrarySet {
    fn from_world(world: &mut World) -> Self {
        let config = world.get_resource::<HumentityGlobalConfig>().expect("No global config loaded");
        let asset_server = world.get_resource::<AssetServer>().expect("No asset server loaded");
        let mut handles = HashMap::<String, Handle<Gltf>>::new();

        // We will search the folder(s) provided for glb/gltf files 
        for path in config.animation_libraries.paths.iter() {
            for entry in read_dir(path.clone()).unwrap() {
                let file = entry.expect("Unspecified file Error");
                if !file.file_type().unwrap().is_file() { continue; }
                let path = file.path();
                let Some(extension) = path.extension() else { continue };
                if !(extension == "glb" || extension == "gltf") { continue }
                let name = path.file_stem().unwrap().to_string_lossy();

                // We have to feed the path relative to "assets" into the asset_loader
                let components: Vec<&str> = path.components().map(|c| c.as_os_str().to_str().unwrap()).collect();
                if let Some(index) = components.iter().position(|&comp| comp == "assets") {
                    // Create a relative path starting from assets
                    let relative_path: PathBuf = components[index + 1..].iter().collect::<PathBuf>();
                    let handle = asset_server.load(relative_path);
                    handles.insert(name.to_string(), handle);
                } else {
                    println!("The directory '{}' was not found in the path.", "assets");
                }
            }
        }

        AnimationLibrarySet {
            gltf_handles: handles,
            libraries: HashMap::<String, AnimationLibrary>::new(),
        }
    }
}

/*---------+
 | Systems |
 +---------*/
pub(crate) fn load_animations(
    gltfs: Res<Assets<Gltf>>,
    mut animations: ResMut<AnimationLibrarySet>,
    mut loading_state: ResMut<LoadingState>,
) {
    if let Some(&done) = loading_state.0.get(&LoadingPhase::SetUpAnimationLibraries) { if done { return; } }
    for (_name, handle) in animations.gltf_handles.iter() {
        let Some(_gltf) = gltfs.get(&*handle) else { return; };
    }
    for (name, handle) in animations.gltf_handles.clone().iter_mut() {
        let gltf = gltfs.get(handle).unwrap();
        let animation_clips: Vec<(&Box<str>, &Handle<AnimationClip>)> = gltf.named_animations.iter()
            .map(|animation| animation.clone())
            .collect();
        let library: HashMap<String, Handle<AnimationClip>> = animation_clips.iter().map(|(s, &ref c)| (s.to_string(), c.clone())).collect();
        animations.libraries.insert(name.to_string(), AnimationLibrary(library));
    }
    loading_state.0.insert(LoadingPhase::SetUpAnimationLibraries, true);
}