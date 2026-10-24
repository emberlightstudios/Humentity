//! Character templates: an ordered list of shape handles and nothing else.
//!
//! The mesh build turns each shape into one morph target, in this order, so the
//! order is the contract between a template and a character's weight vector.
//! Shapes carry all the morph data; see [`crate::loaders::CharacterShape`].
//!
//! Build-once: create the template fully, add it to the asset store, then never
//! mutate it. Mesh builds are cached by template handle, so pushing or clearing
//! shapes after meshes exist serves stale cached meshes with no error. A variant
//! is a new template, not an edit.

use std::sync::Arc;

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;

use crate::loaders::CharacterShape;

#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct CharacterTemplate {
    /// Set by the asset loader from the file stem. Empty for code-constructed templates.
    pub name: String,
    pub shapes: Vec<Handle<CharacterShape>>,
}

impl CharacterTemplate {
    pub fn new(shapes: impl IntoIterator<Item = Handle<CharacterShape>>) -> Self {
        Self {
            name: String::new(),
            shapes: shapes.into_iter().collect(),
        }
    }

    /// Every shape handle resolved, in template order. `None` until they all
    /// load — a partial list would misalign weights, so this never skips one.
    pub fn resolved_shapes<'a>(
        &'a self,
        shapes: &'a Assets<CharacterShape>,
    ) -> Option<Vec<&'a CharacterShape>> {
        self.shapes
            .iter()
            .map(|handle| shapes.get(handle))
            .collect()
    }
}

#[derive(Default, TypePath)]
pub struct CharacterTemplateAssetLoader;

impl AssetLoader for CharacterTemplateAssetLoader {
    type Asset = CharacterTemplate;
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
        struct RawTemplate {
            shapes: Vec<String>,
        }

        let raw: RawTemplate = toml::from_str(&text)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let shapes = raw
            .shapes
            .iter()
            .map(|shape_path| load_context.load::<CharacterShape>(shape_path.clone()))
            .collect();
        let mut template = CharacterTemplate {
            name: String::new(),
            shapes,
        };

        let name = load_context
            .path()
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unnamed");
        template.name = name.to_string();
        Ok(template)
    }

}
