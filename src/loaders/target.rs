use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    ecs::intern::Internable,
    prelude::*,
};

use crate::NAME_INTERNER;

const SCALE_FACTOR: f32 = 0.1;
const DELTA_SQ_THRESHOLD: f32 = 1e-6;

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
        let raw_text = String::from_utf8(bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;
        let mut deltas = Vec::new();
        let asset_label = _load_context.path().path().to_string_lossy().into_owned();
        for (line_number, raw_line) in raw_text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let mut parts = line.split_whitespace();
            let vertex_text = parts.next().unwrap_or_else(|| {
                panic!(
                    "target '{asset_label}' line {} has no vertex index: expected '<u16> <x> <y> <z>' (got '{raw_line}')",
                    line_number + 1,
                )
            });
            let vertex: u16 = vertex_text.parse().unwrap_or_else(|_| {
                panic!(
                    "target '{asset_label}' line {} has a corrupt vertex index '{vertex_text}': expected '<u16> <x> <y> <z>' (got '{raw_line}')",
                    line_number + 1,
                )
            });
            let parse_axis = |axis_name: &str, axis_text: Option<&str>| -> f32 {
                let axis_text = axis_text.unwrap_or_else(|| {
                    panic!(
                        "target '{asset_label}' line {} is missing {axis_name}: expected '<u16> <x> <y> <z>' (got '{raw_line}')",
                        line_number + 1,
                    )
                });
                axis_text.parse().unwrap_or_else(|_| {
                    panic!(
                        "target '{asset_label}' line {} has a corrupt {axis_name} '{axis_text}': expected a float (got '{raw_line}')",
                        line_number + 1,
                    )
                })
            };
            let x = parse_axis("x", parts.next());
            let y = parse_axis("y", parts.next());
            let z = parse_axis("z", parts.next());

            let offset = Vec3::new(x, y, z) * SCALE_FACTOR;
            if offset.length_squared() > DELTA_SQ_THRESHOLD {
                deltas.push(TargetDelta { vertex, offset });
            }
        }

        let name = _load_context
            .path()
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown_target");
        let name = NAME_INTERNER.intern(name).leak();

        Ok(TargetAsset { deltas, name })
    }

    fn extensions(&self) -> &[&str] {
        &["target"]
    }
}
