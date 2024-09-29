use bevy::prelude::Resource;
use std::{
    path::PathBuf,
    env,
    collections::HashSet
};

#[derive(Resource, Clone)]
pub struct HumentityGlobalConfig {
    pub(crate) core_assets_path: PathBuf,
    pub body_part_paths: HashSet<PathBuf>,
    pub equipment_paths: HashSet<PathBuf>,
    pub target_paths: HashSet<PathBuf>,
    pub body_part_slots: Vec<String>,
    pub transparent_slots: Vec<String>,
    pub equipment_slots: Vec<String>,
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

        HumentityGlobalConfig {
            core_assets_path: path.join("assets"),
            body_part_paths: vec![path.join("assets/body_parts")].into_iter().collect(),
            equipment_paths: vec![path.join("assets/clothes")].into_iter().collect(),
            target_paths: vec![path.join("assets/targets")].into_iter().collect(),
            body_part_slots: body_parts_slots.iter().map(|s| s.to_string()).collect(),
            equipment_slots: equipment_slots.iter().map(|s| s.to_string()).collect(),
            transparent_slots: transparent_slots.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl HumentityGlobalConfig {
    pub fn with_added_body_parts_paths(&mut self, paths: Vec<PathBuf>) -> &mut Self {
        for path in paths.iter() { self.body_part_paths.insert(path.to_path_buf()); }
        self
    }

    pub fn with_added_equipment_paths(&mut self, paths: Vec<PathBuf>) -> &mut Self {
        for path in paths.iter() { self.equipment_paths.insert(path.to_path_buf()); }
        self
    }

    pub fn with_added_target_paths(&mut self, paths: Vec<PathBuf>) -> &mut Self {
        for path in paths.iter() { self.target_paths.insert(path.to_path_buf()); }
        self
    }

    pub fn with_body_part_slots(&mut self, slots: Vec<String>) -> &mut Self {
        self.body_part_slots = slots;
        self
    }

    pub fn with_equipment_slots(&mut self, slots: Vec<String>) -> &mut Self {
        self.equipment_slots = slots;
        self
    }
}
