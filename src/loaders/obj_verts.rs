use ahash::AHashMap;
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::{Deserialize, Serialize};

#[derive(Asset, TypePath, Clone, Debug)]
pub struct ObjVertsAsset {
    pub vertices: Vec<Vec3>,
    pub is_basemesh_helpers: bool,
}

#[derive(Asset, TypePath, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ObjVertsSettings {
    #[serde(default)]
    pub is_basemesh_helpers: bool,
}

#[derive(Default, TypePath)]
pub struct ObjVertsAssetLoader;

impl AssetLoader for ObjVertsAssetLoader {
    type Asset = ObjVertsAsset;
    type Settings = ObjVertsSettings;
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let content = String::from_utf8(bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

        let asset_label = _load_context.path().path().to_string_lossy().into_owned();
        let mut vertices = Vec::new();
        for (line_number, line) in content.lines().enumerate() {
            if !line.starts_with("v ") {
                continue;
            }
            let mut values = line.split_whitespace().skip(1);
            let parse_axis = |axis_name: &str, axis_text: Option<&str>| -> f32 {
                let axis_text = axis_text.unwrap_or_else(|| {
                    panic!(
                        "obj '{asset_label}' line {} is missing {axis_name}: every 'v' line needs three floats (got '{line}')",
                        line_number + 1,
                    )
                });
                axis_text.parse().unwrap_or_else(|_| {
                    panic!(
                        "obj '{asset_label}' line {} has a corrupt {axis_name} '{axis_text}': expected a float (got '{line}')",
                        line_number + 1,
                    )
                })
            };
            let x = parse_axis("x", values.next());
            let y = parse_axis("y", values.next());
            let z = parse_axis("z", values.next());
            vertices.push(Vec3::new(x, y, z));
        }

        Ok(ObjVertsAsset {
            vertices,
            is_basemesh_helpers: settings.is_basemesh_helpers,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["obj"]
    }
}

#[derive(Asset, TypePath, Clone, Debug, Deserialize, Deref)]
pub struct VertexGroupsAsset(pub AHashMap<String, Vec<[usize; 2]>>);

#[derive(Default, TypePath)]
pub struct VertexGroupsAssetLoader;

impl AssetLoader for VertexGroupsAssetLoader {
    type Asset = VertexGroupsAsset;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        serde_json::from_slice::<VertexGroupsAsset>(&bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))
    }

    fn extensions(&self) -> &[&str] {
        // No extension: see TargetManifestAssetLoader — JSON is claimed by four
        // loaders, so typed loads are required.
        &[]
    }
}
