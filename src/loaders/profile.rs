//! A character's shape selection: one template plus a weight per template shape.
//!
//! Weights are authored by shape name and resolved to a `Vec<f32>` in template
//! order, which is the order of the baked morph targets. A weight key that names
//! no shape on the template is an error, not a silent zero.

use std::sync::Arc;

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    mesh::morph::MeshMorphWeights,
    prelude::*,
};

use crate::{
    loaders::{CharacterShape, CharacterTemplate},
    morphs::MorphTargets,
};

/// `*.profile.toml`: a template path plus per-shape weights. Profiles are not
/// build-once — they only select weights, and every mesh build reads them anew.
#[derive(Default, TypePath)]
pub struct CharacterProfileAssetLoader;

impl AssetLoader for CharacterProfileAssetLoader {
    type Asset = CharacterProfile;
    type Settings = ();
    type Error = Arc<std::io::Error>;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        #[derive(serde::Deserialize)]
        struct RawProfile {
            template: String,
            #[serde(default)]
            weights: MorphTargets,
        }

        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(Arc::new)?;
        let text = String::from_utf8(bytes)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let raw: RawProfile = toml::from_str(&text)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;

        let template = load_context.load::<CharacterTemplate>(raw.template);
        Ok(CharacterProfile::new(template, raw.weights))
    }

}

#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct CharacterProfile {
    pub template: Handle<CharacterTemplate>,
    /// Shape name → weight. A shape on the template with no entry weighs 0.
    pub weights: MorphTargets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    TemplateNotLoaded,
    ShapesNotLoaded,
    UnknownShape(String),
}

impl CharacterProfile {
    pub fn new(template: Handle<CharacterTemplate>, weights: MorphTargets) -> Self {
        Self { template, weights }
    }

    /// The rest body: every shape at weight 0. Spelled out at the call site so
    /// "no shape selected" can never be mistaken for "weights forgotten".
    pub fn unshaped(template: Handle<CharacterTemplate>) -> Self {
        Self {
            template,
            weights: MorphTargets::default(),
        }
    }

    /// Weights in template order. Template and every shape must be loaded.
    pub fn resolve_weights(
        &self,
        templates: &Assets<CharacterTemplate>,
        shapes: &Assets<CharacterShape>,
    ) -> Result<Vec<f32>, ProfileError> {
        let template = templates
            .get(&self.template)
            .ok_or(ProfileError::TemplateNotLoaded)?;
        let resolved = template
            .resolved_shapes(shapes)
            .ok_or(ProfileError::ShapesNotLoaded)?;

        for &weighted_shape_name in self.weights.keys() {
            if !resolved
                .iter()
                .any(|shape| shape.name() == weighted_shape_name)
            {
                return Err(ProfileError::UnknownShape(weighted_shape_name.to_string()));
            }
        }

        Ok(resolved
            .iter()
            .map(|shape| self.weights.get(shape.name()).copied().unwrap_or(0.0))
            .collect())
    }

    /// Ready-made component for the mesh entity, or `None` until the template
    /// and its shapes load.
    pub fn mesh_morph_weights(
        &self,
        templates: &Assets<CharacterTemplate>,
        shapes: &Assets<CharacterShape>,
    ) -> Option<MeshMorphWeights> {
        let weights = self.resolve_weights(templates, shapes).ok()?;
        Some(MeshMorphWeights::Value { weights })
    }
}
