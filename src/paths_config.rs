use ahash::AHashSet;
use bevy::prelude::*;
use std::{
    fs::canonicalize,
    path::{Path, PathBuf},
};

/// Stored the path to an asset relative to the asset source root, as well as source data
#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct HumentityAssetPath {
    /// The path to the folder (relative to the source asset directory e.g. ./assets)
    pub(crate) path: PathBuf,
    /// The name of the custom asset source (if not the default ./assets folder)
    pub(crate) source_id: HumentityAssetSourceId,
}

impl HumentityAssetPath {
    pub fn new(path: impl AsRef<Path>, source_id: &HumentityAssetSourceId) -> Self {
        Self {
            path: get_abs_path(source_id.root_path.join(path)),
            source_id: source_id.clone(),
        }
    }

    pub fn full_path(&self) -> PathBuf {
        self.source_id.root_path.join(&self.path)
    }

    pub fn load_asset<T: Asset>(&self, asset_server: &AssetServer) -> Handle<T> {
        let prefix = if let Some(source_name) = self.source_id.id {
            &format!("{}://", source_name)
        } else {
            ""
        };
        let path = self.path.to_str().expect("Unparseable path");
        let path = format!("{prefix}{}", path);
        asset_server.load(path)
    }
}

/// A reference to the asset source where an asset resides
#[derive(Clone, Hash, Eq, PartialEq, Default, Debug)]
pub struct HumentityAssetSourceId {
    /// For the default asset source use None, otherwise the name of the custom source
    pub id: Option<&'static str>,
    /// The path to the source root
    pub root_path: PathBuf,
}

impl HumentityAssetSourceId {
    pub fn new(id: Option<&'static str>, root_path: impl AsRef<Path>) -> Self {
        Self {
            id,
            root_path: get_abs_path(root_path)
        }
    }
}

#[derive(Resource, Clone)]
pub struct HumentityPathsConfig {
    /// The path to the folder where base.obj exists.  Should be in assets/ inside this crate
    pub(crate) core_assets_path: PathBuf,
    /// Paths from which body meshes (aka proxy meshes) should be loaded
    pub(crate) body_mesh_paths: AHashSet<HumentityAssetPath>,
    /// Paths from which body parts should be loaded
    pub(crate) body_part_paths: AHashSet<HumentityAssetPath>,
    /// Paths from which equipment/clothes should be loaded
    pub(crate) equipment_paths: AHashSet<HumentityAssetPath>,
    /// Paths from which skin textures should be loaded
    pub(crate) skin_texture_paths: AHashSet<HumentityAssetPath>,
    /// Paths from which target files should be loaded
    pub(crate) target_paths: AHashSet<PathBuf>,
}

impl HumentityPathsConfig {
    pub fn new(
        core_assets_path: PathBuf,
        body_mesh_paths: impl IntoIterator<Item = HumentityAssetPath>,
        body_part_paths: impl IntoIterator<Item = HumentityAssetPath>,
        equipment_paths: impl IntoIterator<Item = HumentityAssetPath>,
        skin_texture_paths: impl IntoIterator<Item = HumentityAssetPath>,
        target_paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        Self {
            core_assets_path: get_abs_path(core_assets_path),
            body_mesh_paths: body_mesh_paths.into_iter().collect(),
            body_part_paths: body_part_paths.into_iter().collect(),
            equipment_paths: equipment_paths.into_iter().collect(),
            skin_texture_paths: skin_texture_paths.into_iter().collect(),
            target_paths: target_paths
                .into_iter()
                .map(get_abs_path)
                .collect(),
        }
    }
}

fn get_abs_path(path: impl AsRef<Path>) -> PathBuf {
    let path = if path.as_ref().starts_with(".") {
        std::env::current_dir()
            .expect("Failed to get current dir")
            .join(path)
    } else { path.as_ref().to_path_buf() };
    canonicalize(&path)
        .unwrap_or_else(|err| panic!("Failed to canonicalize path: {:#?} \n{}", path, err))
}