use bevy::{
    prelude::*,
    gltf::Gltf,
    asset::AssetEvent,
};
use std::{
    collections::HashMap,
    fs::read_dir,
};
use crate::{
    HumentityGlobalConfig,
    LoadingPhase,
    LoadingState,
};

#[allow(dead_code)]
pub(crate) struct AnimationLibrary(HashMap<String, Handle<AnimationClip>>);

/*-----------+
 | Resources |
 +-----------*/
#[derive(Resource)]
pub(crate) struct AnimationLibrarySet{
    gltf_handles: HashMap<String, Handle<Gltf>>,
    pub(crate) libraries: HashMap<String, AnimationLibrary>,
}

impl FromWorld for AnimationLibrarySet {
    fn from_world(world: &mut World) -> Self {
        let config = world.get_resource::<HumentityGlobalConfig>().expect("No global config loaded");
        let asset_server = world.get_resource::<AssetServer>().expect("No asset server loaded");
        let mut handles = HashMap::<String, Handle<Gltf>>::new();

        for path in config.animation_library_paths.iter() {
            for entry in read_dir(path).unwrap() {
                let file = entry.expect("Unspecified file Error");
                if !file.file_type().unwrap().is_file() { continue; }
                let path = file.path();
                let Some(extension) = path.extension() else { continue };
                if extension == "glb" || extension == "gltf" {
                    let name = path.file_stem().unwrap().to_string_lossy();
                    let handle = asset_server.load(path.clone());
                    handles.insert(name.to_string(), handle);
                }
            }
        }

        AnimationLibrarySet {
            gltf_handles: handles,
            libraries: HashMap::<String, AnimationLibrary>::new(),
        }
    }
}

pub(crate) fn load_animations(
    gltfs: Res<Assets<Gltf>>,
    mut animations: ResMut<AnimationLibrarySet>,
    mut loading_state: ResMut<LoadingState>,
    mut events: EventReader<AssetEvent<Gltf>>,
) {
    for ev in events.read() {
        println!("{:?}", ev);
    }
    if let Some(&done) = loading_state.0.get(&LoadingPhase::SetUpAnimationLibraries) { if done { return; } }
    for (_name, handle) in animations.gltf_handles.clone().iter_mut() {
        let Some(_gltf) = gltfs.get(&*handle) else { return; };
    }

    for (name, handle) in animations.gltf_handles.clone().iter_mut() {
        let gltf = gltfs.get(handle).unwrap();
        let animation_clips: Vec<(&Box<str>, &Handle<AnimationClip>)> = gltf.named_animations.iter()
            .map(|animation| animation.clone())
            .collect();
        let library: HashMap<String, Handle<AnimationClip>> = animation_clips.iter().map(|(s, &ref c)| (s.to_string(), c.clone())).collect();
        println!("{} {:?}", name, library);
        animations.libraries.insert(name.to_string(), AnimationLibrary(library));
    }
    loading_state.0.insert(LoadingPhase::SetUpAnimationLibraries, true);
}