use ahash::AHashMap;
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader}, ecs::intern::Internable, prelude::*
};
use serde::Deserialize;

use crate::NAME_INTERNER;

#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct MacroDataAsset {
    pub macrotargets: AHashMap<&'static str, MacroBounds>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct MacroDataAssetString {
    pub macrotargets: AHashMap<String, MacroBounds>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MacroBounds {
    pub parts: Vec<MacroBound>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MacroBound {
    pub lowest: f32,
    pub highest: f32,
    pub low: String,
    pub high: String,
}

#[derive(Default, TypePath)]
pub struct MacroDataAssetLoader;

impl AssetLoader for MacroDataAssetLoader {
    type Asset = MacroDataAsset;
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

        let values = serde_json::from_slice::<MacroDataAssetString>(&bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()));

        values.iter().map(|v| {
            let interned = v
                .macrotargets
                .iter()
                .map(|(k, bounds)| (NAME_INTERNER.intern(k).leak(), bounds.clone()))
                .collect();
            MacroDataAsset {
                macrotargets: interned,
            }
        }).next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "No data found in macro asset"))
    }

    fn extensions(&self) -> &[&str] {
        // Use .macro to avoid conflict with RigConfigAssetLoader when load_folder discovers files
        // (load_folder has no type, so extension is the only loader hint)
        &["macro"]
    }
}
