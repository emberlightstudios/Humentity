use bevy::{
    asset::{io::Reader, AssetLoader, LoadContext},
    prelude::*,
};
use ahash::AHashMap;
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
        let content = String::from_utf8(bytes).map_err(|err| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
        })?;

        let mut vertices = Vec::new();
        for line in content.lines() {
            if !line.starts_with("v ") {
                continue;
            }
            let mut values = line.split_whitespace().skip(1);
            let Some(x) = values.next().and_then(|v| v.parse::<f32>().ok()) else {
                continue;
            };
            let Some(y) = values.next().and_then(|v| v.parse::<f32>().ok()) else {
                continue;
            };
            let Some(z) = values.next().and_then(|v| v.parse::<f32>().ok()) else {
                continue;
            };
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
        &["json"]
    }
}
