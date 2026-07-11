use std::sync::Arc;

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::ecs::intern::Internable;
use bevy::prelude::*;

use crate::NAME_INTERNER;
use crate::prelude::*;

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
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(Arc::new)?;
        let text = String::from_utf8(bytes)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let mut template: CharacterTemplate = toml::from_str(&text)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let name = _load_context
            .path()
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed");
        template.name = NAME_INTERNER.intern(name).leak();
        Ok(template)
    }

    fn extensions(&self) -> &[&str] {
        &["toml"]
    }
}
