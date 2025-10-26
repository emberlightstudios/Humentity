use bevy::prelude::Resource;
use std::path::PathBuf;
use ahash::AHashSet;


#[derive(Resource, Clone)]
pub struct HumentityPathsConfig {
    pub(crate) core_assets_path: PathBuf,
    pub(crate) proxymesh_paths: AHashSet<PathBuf>,
    pub(crate) body_part_paths: AHashSet<PathBuf>,
    pub(crate) equipment_paths: AHashSet<PathBuf>,
    pub(crate) target_paths: AHashSet<PathBuf>,
}

impl HumentityPathsConfig {
    pub fn new(path: impl AsRef<str>) -> Self {
        let path = PathBuf::from(path.as_ref());

        HumentityPathsConfig {
            core_assets_path: path.join("assets"),
            proxymesh_paths: vec![PathBuf::from("proxymeshes")].into_iter().collect(),
            body_part_paths: vec![PathBuf::from("body_parts")].into_iter().collect(),
            equipment_paths: vec![PathBuf::from("clothes")].into_iter().collect(),
            target_paths: vec![PathBuf::from("targets")].into_iter().collect(),
        }
    }
}

impl HumentityPathsConfig {
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
}
