use ahash::AHashSet;
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// Metadata about where an asset should be loaded from. Anything that should be
/// loaded with the AssetServer needs to specify which asset source it resides in,
/// None for the default.  Textures and .obj files need this.  Targets do not.
#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct HumentityAssetPath {
    /// The path to the folder (relative to the source asset directory e.g. ./assets)
    pub(crate) path: PathBuf,
    /// The name of the custom asset source (if not the default ./assets folder)
    pub(crate) source_id: HumentityAssetSourceId,
}

impl Default for HumentityAssetPath {
    fn default() -> Self {
        Self {
            path: PathBuf::from("."),
            source_id: HumentityAssetSourceId::new(
                None,
                PathBuf::from("./assets")
            ),
        }
    }
}

impl HumentityAssetPath {
    pub fn new(
        path: impl AsRef<Path>,
        source_id: &HumentityAssetSourceId,
    ) -> Self {
        Self { path: path.as_ref().to_path_buf(), source_id: source_id.clone() }
    }

    pub fn load_asset<T: Asset>(&self, asset_server: &AssetServer) -> Handle<T> {
        let prefix = if let Some(source_name) = self.source_id.id {
            &format!("{}://", source_name)
        } else {
            ""
        };
        let path = self.path.to_str()
            .expect("Unparseable path");
        let path = format!("{prefix}{}", path);
        asset_server.load(path)
    }
}

/// After creating a custom asset source it is not so easy to get the path to it
/// back out from the asset server.  Just pass it in here.
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
            root_path: root_path.as_ref().to_path_buf(),
        }
    }
}


#[derive(Resource, Clone)]
pub struct HumentityPathsConfig {
    /// The path to the folder where base.obj exists.  Should be in assets/ inside this crate
    pub core_assets_path: PathBuf,
    /// Paths from which proxy meshes should be loaded
    pub proxymesh_paths: AHashSet<HumentityAssetPath>,
    /// Paths from which body parts should be loaded
    pub body_part_paths: AHashSet<HumentityAssetPath>,
    /// Paths from which equipment/clothes should be loaded
    pub equipment_paths: AHashSet<HumentityAssetPath>,
    /// Paths from which skin textures should be loaded
    pub skin_texture_paths: AHashSet<HumentityAssetPath>,
    /// Paths from which target files should be loaded
    pub target_paths: AHashSet<PathBuf>,
}
