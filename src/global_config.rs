use bevy::prelude::Resource;
use std::{
    path::PathBuf,
    env,
    collections::HashSet
};

#[derive(Resource, Clone)]
pub struct HumentityGlobalConfig {
    pub body_part_paths: HashSet<PathBuf>,
    pub equipment_paths: HashSet<PathBuf>,
    pub target_paths: HashSet<PathBuf>,
}

impl Default for HumentityGlobalConfig {
    fn default() -> Self {
        let path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        HumentityGlobalConfig {
            body_part_paths: vec![path.join("assets/body_parts")].into_iter().collect(),
            equipment_paths: vec![path.join("assets/clothes")].into_iter().collect(),
            target_paths: vec![path.join("assets/targets")].into_iter().collect(),
        }
    }
}

impl HumentityGlobalConfig {
    pub fn with_body_parts_paths(&mut self, paths: Vec<PathBuf>) -> Self {
        for path in paths.iter() {
            self.body_part_paths.insert(path.to_path_buf());
        }
        self.clone()
    }

    pub fn with_equipment_paths(&mut self, paths: Vec<PathBuf>) -> Self {
        for path in paths.iter() {
            self.equipment_paths.insert(path.to_path_buf());
        }
        self.clone()
    }

    pub fn with_target_paths(&mut self, paths: Vec<PathBuf>) -> Self {
        for path in paths.iter() {
            self.target_paths.insert(path.to_path_buf());
        }
        self.clone()
    }
}
