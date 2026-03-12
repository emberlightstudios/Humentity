use ahash::AHashMap;
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader}, ecs::intern::Internable, prelude::*
};
use serde::Deserialize;

use crate::NAME_INTERNER;

#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct MacroDataAsset {
    /// Raw macro data
    pub macrotargets: AHashMap<&'static str, MacroBounds>,
    /// Convenience fied mapping macro names to the morph names they affect
    pub morph_map: AHashMap<&'static str, Vec<&'static str>>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct MacroDataAssetString {
    pub macrotargets: AHashMap<String, MacroBoundsString>,
}

#[derive(Clone, Debug)]
pub struct MacroBounds {
    pub parts: Vec<MacroBound>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MacroBoundsString {
    pub parts: Vec<MacroBoundString>,
}

#[derive(Clone, Debug)]
pub struct MacroBound {
    pub lowest: f32,
    pub highest: f32,
    pub low: &'static str,
    pub high: &'static str,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MacroBoundString {
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
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

        let mut morph_map = AHashMap::<&'static str, Vec<&'static str>>::default();
        let mut macros = MacroDataAsset::default();
        for (macro_name, bounds) in values.macrotargets.iter() {
            let name = NAME_INTERNER.intern(macro_name).leak();
            let parts = bounds.parts.iter().map(|part| {
                let low = NAME_INTERNER.intern(&part.low).leak();
                let high = NAME_INTERNER.intern(&part.high).leak();
                MacroBound {
                    lowest: part.lowest,
                    highest: part.highest,
                    low,
                    high,
                }
            }).collect();

            macros.macrotargets.insert(name, MacroBounds { parts });

            let mut morph_names: Vec<&'static str> = macros.macrotargets[name]
                .parts.iter().flat_map(|part| [part.low, part.high]).collect();
            morph_names.dedup();
            if let Some(pos) = morph_names.iter().position(|x| x.is_empty()) {
                morph_names.remove(pos);
            }
            morph_map.insert(name, morph_names);
        }
        morph_map.insert("race", vec!["african", "asian", "caucasian"]);
        macros.morph_map = morph_map;

        Ok(macros)
    }

    fn extensions(&self) -> &[&str] {
        // Use .macro to avoid conflict with RigConfigAssetLoader when load_folder discovers files
        // (load_folder has no type, so extension is the only loader hint)
        &["macro"]
    }
}
