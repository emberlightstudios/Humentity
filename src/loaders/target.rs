use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader}, ecs::intern::Internable, prelude::*
};

use crate::NAME_INTERNER;

const SCALE_FACTOR: f32 = 0.1;

#[derive(Clone, Debug)]
pub struct TargetDelta {
    pub vertex: u16,
    pub offset: Vec3,
}

#[derive(Asset, TypePath, Clone, Debug)]
pub struct TargetAsset {
    pub name: &'static str,
    pub deltas: Vec<TargetDelta>,
}

#[derive(Default, TypePath)]
pub struct TargetAssetLoader;

impl AssetLoader for TargetAssetLoader {
    type Asset = TargetAsset;
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
        let raw_text = String::from_utf8(bytes).map_err(|err| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
        })?;

        let mut deltas = Vec::new();
        for line in raw_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let mut parts = line.split_whitespace();
            let Some(vertex) = parts.next().and_then(|v| v.parse::<u16>().ok()) else {
                continue;
            };
            let Some(x) = parts.next().and_then(|v| v.parse::<f32>().ok()) else {
                continue;
            };
            let Some(y) = parts.next().and_then(|v| v.parse::<f32>().ok()) else {
                continue;
            };
            let Some(z) = parts.next().and_then(|v| v.parse::<f32>().ok()) else {
                continue;
            };

            deltas.push(TargetDelta {
                vertex,
                offset: Vec3::new(x, y, z) * SCALE_FACTOR,
            });
        }

        let name = _load_context.path().path().file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown_target");
        let name = NAME_INTERNER.intern(name).leak();

        Ok(TargetAsset { deltas, name })
    }

    fn extensions(&self) -> &[&str] {
        &["target"]
    }
}
