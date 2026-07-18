use ahash::AHashMap;
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    ecs::intern::Internable,
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::NAME_INTERNER;

#[derive(Asset, TypePath, Clone, Debug)]
pub struct RigWeightsAsset {
    pub weights: AHashMap<&'static str, AHashMap<u16, f32>>,
    pub rig_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WeightsFormat {
    pub weights: AHashMap<String, Vec<(u16, f32)>>,
}

#[derive(Default, TypePath)]
pub struct RigWeightsAssetLoader;

impl AssetLoader for RigWeightsAssetLoader {
    type Asset = RigWeightsAsset;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let weights = serde_json::from_slice::<WeightsFormat>(&bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

        let path = load_context.path().to_string();
        let rig_name = if path.contains("mixamo") {
            "mixamo".to_string()
        } else if path.contains("game_engine") {
            "game_engine".to_string()
        } else if path.contains("default") {
            "default".to_string()
        } else {
            "unknown".to_string()
        };

        let weights = weights
            .weights
            .into_iter()
            .map(|(k, v)| {
                (
                    NAME_INTERNER.intern(&k).leak(),
                    v.iter().copied().collect::<AHashMap<_, _>>(),
                )
            })
            .collect::<AHashMap<_, _>>();

        Ok(RigWeightsAsset { weights, rig_name })
    }

    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}
