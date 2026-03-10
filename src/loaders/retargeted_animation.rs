use ahash::AHashMap;
use bevy::{
    asset::{io::Reader, AssetLoader, LoadContext},
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::{TranslationTracks, animation::get_animation_clips_from_bytes};

#[derive(Asset, TypePath, Clone)]
pub struct RetargetedAnimationAsset {
    pub clips: AHashMap<&'static str, AnimationClip>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TypePath)]
pub struct RetargetedAnimationSettings {
    pub translation_tracks: TranslationTracks,
}

#[derive(Default, TypePath)]
pub struct RetargetedAnimationAssetLoader;

impl AssetLoader for RetargetedAnimationAssetLoader {
    type Asset = RetargetedAnimationAsset;
    type Settings = RetargetedAnimationSettings;
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let clips = get_animation_clips_from_bytes(&bytes, settings.translation_tracks)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;
        Ok(RetargetedAnimationAsset { clips })
    }

    fn extensions(&self) -> &[&str] {
        &["glb", "gltf"]
    }
}
