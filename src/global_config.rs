use bevy::prelude::Resource;
use std::{
    path::PathBuf,
    env,
    collections::HashSet
};

#[derive(Resource, Clone)]
pub struct HumentityGlobalConfig {
    pub(crate) core_assets_path: PathBuf,
    pub(crate) body_part_paths: HashSet<PathBuf>,
    pub(crate) equipment_paths: HashSet<PathBuf>,
    pub(crate) target_paths: HashSet<PathBuf>,
    pub(crate) animation_library_paths: HashSet<PathBuf>,
    pub(crate) body_part_slots: Vec<String>,
    pub(crate) transparent_slots: Vec<String>,
    pub(crate) equipment_slots: Vec<String>,
}

impl Default for HumentityGlobalConfig {
    fn default() -> Self {
        let mut path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        // It is assumed this is only one level deep in your source. 
        if !path.to_str().unwrap().ends_with("humentity") { path = path.join("src/humentity") }
        let body_parts_slots = vec![
            "LeftEye",
            "LeftEyebrow",
            "LeftEyelash",
            "RightEye",
            "RightEyebrow",
            "RightEyelash",
            "Tongue",
            "Teeth",
            "Hair",
        ];
        let transparent_slots = vec![
            "LeftEyebrow",
            "LeftEyelash",
            "RightEyebrow",
            "RightEyelash",
            "Hair",
        ];
        let equipment_slots = vec![
            "Head",
            "Chest",
            "Abdomen",
            "Pelvis",
            "RightArm",
            "LeftArm",
            "RightHand",
            "LeftHand",
            "RightLeg",
            "LeftLeg",
            "RightFoot",
            "LeftFoot",
        ];
        let animation_library_paths = HashSet::<PathBuf>::new();

        HumentityGlobalConfig {
            core_assets_path: path.join("assets"),
            body_part_paths: vec![path.join("assets/body_parts")].into_iter().collect(),
            equipment_paths: vec![path.join("assets/clothes")].into_iter().collect(),
            target_paths: vec![path.join("assets/targets")].into_iter().collect(),
            animation_library_paths: animation_library_paths,
            body_part_slots: body_parts_slots.iter().map(|s| s.to_string()).collect(),
            equipment_slots: equipment_slots.iter().map(|s| s.to_string()).collect(),
            transparent_slots: transparent_slots.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl HumentityGlobalConfig {
    pub fn with_added_body_parts_paths<I>(self, paths: I) -> Self
    where I: IntoIterator<Item = PathBuf> {
        let mut new = self;
        for path in paths.into_iter() { new.body_part_paths.insert(path.to_path_buf()); }
        new
    }

    pub fn with_added_equipment_paths<I>(self, paths: I) -> Self
    where I: IntoIterator<Item = PathBuf> {
        let mut new = self;
        for path in paths.into_iter() { new.equipment_paths.insert(path.to_path_buf()); }
        new
    }

    pub fn with_added_target_paths<I>(self, paths: I) -> Self
    where I: IntoIterator<Item = PathBuf> {
        let mut new = self;
        for path in paths.into_iter() { new.target_paths.insert(path.to_path_buf()); }
        new
    }

    pub fn with_animation_library_paths<I>(self, paths: I) -> Self
    where I: IntoIterator<Item = PathBuf> {
        let mut new = self;
        new.animation_library_paths = paths.into_iter().collect();
        new
    }

    pub fn with_body_part_slots<I>(self, slots: I) -> Self
    where I: IntoIterator<Item = String> {
        let mut new = self;
        new.body_part_slots = slots.into_iter().collect();
        new
    }

    pub fn with_equipment_slots<I>(self, slots: I) -> Self
    where I: IntoIterator<Item = String> {
        let mut new = self;
        new.equipment_slots = slots.into_iter().collect();
        new
    }
}
