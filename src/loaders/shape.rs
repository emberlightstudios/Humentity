//! Character shape assets.
//!
//! A shape is a named set of MakeHuman morph slider values (`gender`, `age`,
//! `muscle`, …), baked once into a sparse delta over **basemesh** vertices
//! ([`TargetDelta`]). It references no template and no mesh part: parts refit
//! onto the deformed basemesh through their own mhclo, so one shape serves every
//! template.
//!
//! A [`CharacterTemplate`](crate::template::CharacterTemplate) references shapes
//! by handle; it holds no morph data of its own.
//!
//! Shapes are build-once. `morphs` are resolved from macro sliders to direct
//! morph targets on load, then the delta is baked. Editing a loaded shape trips
//! [`enforce_shapes_frozen`]: cached meshes carry the old delta as a morph
//! target and nothing would rebuild them.

use std::sync::Arc;

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};

use crate::{loaders::TargetDelta, morphs::MorphTargets};

/// One named shape: morph slider values plus their baked basemesh delta.
#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct CharacterShape {
    /// File stem for loaded shapes, the given name for code-constructed ones.
    pub name: String,
    /// Morph slider values, replaced by direct morph targets on load.
    pub morphs: MorphTargets,
    /// `morphed − base` over basemesh vertices, sparse. `None` until baked.
    pub(crate) helper_deltas: Option<Arc<Vec<TargetDelta>>>,
}

impl CharacterShape {
    pub fn new(name: impl Into<String>, morphs: MorphTargets) -> Self {
        Self {
            name: name.into(),
            morphs,
            helper_deltas: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_baked(&self) -> bool {
        self.helper_deltas.is_some()
    }

    pub(crate) fn helper_deltas(&self) -> Option<&[TargetDelta]> {
        self.helper_deltas.as_ref().map(|deltas| deltas.as_slice())
    }
}

/// Shape files are TOML text in a `*.shape` file, so the extension cannot
/// collide with `*.toml` templates.
#[derive(Default, TypePath)]
pub struct CharacterShapeAssetLoader;

impl AssetLoader for CharacterShapeAssetLoader {
    type Asset = CharacterShape;
    type Settings = ();
    type Error = Arc<std::io::Error>;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        #[derive(serde::Deserialize)]
        struct RawShape {
            morphs: MorphTargets,
        }

        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(Arc::new)?;
        let text = String::from_utf8(bytes)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let raw: RawShape = toml::from_str(&text)
            .map_err(|e| Arc::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;

        // The file is `<shape name>.shape.toml`, so the stem still carries the
        // format suffix; the name is what a profile's weights are keyed by.
        let asset_path = load_context.path();
        let name = asset_path
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.strip_suffix(".shape"))
            .unwrap_or("unnamed")
            .to_string();

        Ok(CharacterShape::new(name, raw.morphs))
    }
}

