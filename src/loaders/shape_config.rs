use std::sync::Arc;

use ahash::AHashMap;
use bevy::{asset::{io::Reader, AssetLoader, LoadContext}, mesh::morph::MeshMorphWeights, prelude::*};
use serde::{Serialize, Deserialize};

use crate::morphs::MorphTargets;
use crate::rigs::BoneTranslationData;
use crate::template::CharacterTemplate;

/// Defines the shape of a character.  Can be loaded as an asset from `.shape.toml` files,
/// or constructed in code and added to the asset store.  Place a [`CharacterShape`] component
/// on the root entity of a character to reference this asset.
#[derive(Asset, TypePath, Clone, Default, Debug, Serialize, Deserialize)]
pub struct CharacterShapeAsset {
    pub template_morph_targets: MorphTargets,
    #[serde(skip)]
    pub template: Handle<CharacterTemplate>,
    #[serde(skip)]
    #[allow(dead_code)]
    pub(crate) bone_translations: BoneTranslationData,
    #[serde(skip)]
    #[allow(dead_code)]
    pub(crate) bone_delta_rotations: AHashMap<&'static str, Quat>,
}

impl CharacterShapeAsset {
    pub fn get_morph_weights_component(&self, templates: &Assets<CharacterTemplate>) -> Option<MeshMorphWeights> {
        let template = templates.get(&self.template)?;
        let morph_weights = template
            .shapes
            .iter()
            .map(|s| *self.template_morph_targets.get(s.name).unwrap_or(&0.))
            .collect::<Vec<_>>();
        Some(MeshMorphWeights::Value { weights: morph_weights })
    }

    pub fn new(template: Handle<CharacterTemplate>, morphs: MorphTargets) -> Self {
        Self {
            template,
            template_morph_targets: morphs,
            bone_translations: BoneTranslationData::None,
            bone_delta_rotations: AHashMap::<&'static str, Quat>::default(),
        }
    }
}

impl From<Handle<CharacterTemplate>> for CharacterShapeAsset {
    fn from(template: Handle<CharacterTemplate>) -> Self {
        Self { template, ..default() }
    }
}

#[derive(Default, TypePath)]
pub struct CharacterShapeConfigLoader;

impl AssetLoader for CharacterShapeConfigLoader {
    type Asset = CharacterShapeAsset;
    type Settings = ();
    type Error = Arc<std::io::Error>;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(Arc::new)?;
        let text = String::from_utf8(bytes)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;

        #[derive(serde::Deserialize)]
        struct Raw {
            template: String,
            template_morph_targets: MorphTargets,
        }

        let raw: Raw = toml::from_str(&text)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;

        let template: Handle<CharacterTemplate> = load_context.load(raw.template);

        Ok(CharacterShapeAsset::new(template, raw.template_morph_targets))
    }

    fn extensions(&self) -> &[&str] {
        &["shape.toml"]
    }
}
