use bevy::{
    asset::{io::Reader, AssetLoader, LoadContext},
    prelude::*,
};
use serde::Deserialize;
use ahash::AHashMap;

#[derive(Asset, TypePath, Clone, Debug)]
pub struct BaseMeshAsset(pub Vec<Vec3>);

#[derive(Default, TypePath)]
pub struct BaseMeshAssetLoader;

impl AssetLoader for BaseMeshAssetLoader {
    type Asset = BaseMeshAsset;
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

        Ok(BaseMeshAsset(vertices))
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
