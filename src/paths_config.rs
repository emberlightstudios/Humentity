use bevy::prelude::Resource;
use std::path::{Path, PathBuf};
use ahash::AHashSet;



/// Metadata about where an asset should be loaded from. Anything that should be
/// loaded with the AssetServer needs to specify which asset source it resides in,
/// None for the default.  Textures and .obj files need this.  Targets do not.
#[derive(Clone, Hash, Eq, PartialEq)]
pub struct HumentityAssetPath {
    /// The path to the folder, relative to your working directory
    pub(crate) path: PathBuf,
    /// The name of the custom asset source (if not the default ./assets folder)
    pub(crate) source_id: Option<HumentityAssetSourceId>,
}

/// After creating a custom asset source it is not so easy to get the path to it
/// back out from the asset server.  Just pass it in here.
#[derive(Clone, Hash, Eq, PartialEq, Default)]
pub struct HumentityAssetSourceId {
    /// The AssetSourceId you used when you created it
    pub id: &'static str,
    /// The path to the source root, relative to your working directory
    pub root_path: PathBuf,
}

impl HumentityAssetSourceId {
    pub fn new(id: &'static str, root_path: impl AsRef<Path>) -> Self {
        Self { id, root_path: root_path.as_ref().to_path_buf() }
    }
}

impl HumentityAssetPath {
    pub fn from_custom_asset_source(path: impl AsRef<Path>, source_id: &HumentityAssetSourceId) -> Self {
        Self { path: path.as_ref().to_path_buf(), source_id: Some(source_id.clone()) }
    }

    pub fn from_default_asset_source(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_path_buf(), source_id: None }
    }
}

#[derive(Resource, Clone)]
pub struct HumentityPathsConfig {
    /// The path to the folder where base.obj exists.  Should be in assets/ inside this crate
    pub(crate) core_assets_path: PathBuf,
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

impl Default for HumentityPathsConfig {
    /// This will give you a paths config that will work only when you are running from inside
    /// the humentity crate.   From outside you will need to configure these paths yourself.
    fn default() -> Self {
        let mut proxymesh_paths = AHashSet::default();
        let mut body_part_paths = AHashSet::default();
        let mut equipment_paths = AHashSet::default();
        let mut skin_texture_paths = AHashSet::default();
        let mut target_paths = AHashSet::default();
        let humentity_source = HumentityAssetSourceId::new("humentity", "./assets");

        proxymesh_paths.insert(HumentityAssetPath::from_custom_asset_source("./proxymeshes", &humentity_source));
        body_part_paths.insert(HumentityAssetPath::from_custom_asset_source("./body_parts", &humentity_source));
        equipment_paths.insert(HumentityAssetPath::from_custom_asset_source("./clothes", &humentity_source));
        skin_texture_paths.insert(HumentityAssetPath::from_custom_asset_source("./skin_textures", &humentity_source));
        target_paths.insert(PathBuf::from("./assets/targets"));
        Self {
            core_assets_path: PathBuf::from("./assets"),
            proxymesh_paths,
            body_part_paths,
            equipment_paths,
            skin_texture_paths,
            target_paths
        }
    }
}

impl HumentityPathsConfig {
    /// Pass the path to the humentity crate root to load the included assets
    pub fn from_crate_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf().join("./assets");
        HumentityPathsConfig {
            core_assets_path: path,
            ..Default::default()
        }
    }
}