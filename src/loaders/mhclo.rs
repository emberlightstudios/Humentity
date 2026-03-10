use bevy::{
    asset::AssetPath,
    asset::{io::Reader, AssetLoader, LoadContext},
    prelude::*,
};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct MhcloScaleAxis {
    pub min: u16,
    pub max: u16,
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub enum MhcloVertexMap {
    SingleVertex(u16),
    Triangle {
        helper_verts: [u16; 3],
        helper_weights: [f32; 3],
        helper_offset: Vec3,
    },
}

#[derive(Asset, TypePath, Clone, Debug)]
pub struct MhcloAsset {
    pub name: String,
    pub obj_file: AssetPath<'static>,
    pub tags: Vec<String>,
    pub z_depth: i8,
    pub x_scale: Option<MhcloScaleAxis>,
    pub y_scale: Option<MhcloScaleAxis>,
    pub z_scale: Option<MhcloScaleAxis>,
    pub helper_map: Vec<MhcloVertexMap>,
    pub delete_verts: Vec<u16>,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum FileSection {
    Header,
    Vertices,
    DeleteVertices,
}

#[derive(Default, TypePath)]
pub struct MhcloAssetLoader;

impl AssetLoader for MhcloAssetLoader {
    type Asset = MhcloAsset;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let raw_text = String::from_utf8(bytes).map_err(|err| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
        })?;

        let mut name = "".to_string();
        let mut obj_file = AssetPath::default();
        let mut tags = Vec::new();
        let mut z_depth = 0_i8;
        let mut x_scale = None;
        let mut y_scale = None;
        let mut z_scale = None;
        let mut helper_map = Vec::new();
        let mut delete_verts = BTreeSet::<u16>::new();
        let mut section = FileSection::Header;

        for line in raw_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with("verts 0") {
                section = FileSection::Vertices;
                continue;
            }
            if line.starts_with("delete_verts") {
                section = FileSection::DeleteVertices;
                continue;
            }

            let mut parts = line.split_whitespace();
            let Some(key) = parts.next() else {
                continue;
            };

            match section {
                FileSection::Header => match key {
                    "name" => {
                        let value = parts.collect::<Vec<_>>().join(" ");
                        if !value.is_empty() {
                            name = line.strip_prefix("name ").unwrap().to_string();
                        }
                    }
                    "obj_file" => {
                        if let Some(value) = parts.next()
                            && let Ok(path) = load_context.path().resolve(value)
                        {
                            obj_file = path;
                        }
                    }
                    "tag" => {
                        let value = parts.collect::<Vec<_>>().join(" ");
                        if !value.is_empty() {
                            tags.push(value);
                        }
                    }
                    "x_scale" => {
                        let values = line.split_whitespace().skip(1).collect::<Vec<_>>();
                        if values.len() >= 3
                            && let (Ok(min), Ok(max), Ok(scale)) = (
                                values[0].parse::<u16>(),
                                values[1].parse::<u16>(),
                                values[2].parse::<f32>(),
                            )
                        {
                            x_scale = Some(MhcloScaleAxis { min, max, scale });
                        }
                    }
                    "y_scale" => {
                        let values = line.split_whitespace().skip(1).collect::<Vec<_>>();
                        if values.len() >= 3
                            && let (Ok(min), Ok(max), Ok(scale)) = (
                                values[0].parse::<u16>(),
                                values[1].parse::<u16>(),
                                values[2].parse::<f32>(),
                            )
                        {
                            y_scale = Some(MhcloScaleAxis { min, max, scale });
                        }
                    }
                    "z_scale" => {
                        let values = line.split_whitespace().skip(1).collect::<Vec<_>>();
                        if values.len() >= 3
                            && let (Ok(min), Ok(max), Ok(scale)) = (
                                values[0].parse::<u16>(),
                                values[1].parse::<u16>(),
                                values[2].parse::<f32>(),
                            )
                        {
                            z_scale = Some(MhcloScaleAxis { min, max, scale });
                        }
                    }
                    "z_depth" => {
                        if let Some(value) = parts.next() && let Ok(parsed) = value.parse::<i8>() {
                            z_depth = parsed;
                        }
                    }
                    _ => {}
                },
                FileSection::Vertices => {
                    let values = line.split_whitespace().collect::<Vec<_>>();
                    if values.is_empty() || values[0] == "material" {
                        continue;
                    }

                    if values.len() == 1 {
                        if let Ok(vertex) = values[0].parse::<u16>() {
                            helper_map.push(MhcloVertexMap::SingleVertex(vertex));
                        }
                        continue;
                    }

                    if values.len() == 9
                        && let (
                            Ok(v0),
                            Ok(v1),
                            Ok(v2),
                            Ok(w0),
                            Ok(w1),
                            Ok(w2),
                            Ok(ox),
                            Ok(oy),
                            Ok(oz),
                        ) = (
                            values[0].parse::<u16>(),
                            values[1].parse::<u16>(),
                            values[2].parse::<u16>(),
                            values[3].parse::<f32>(),
                            values[4].parse::<f32>(),
                            values[5].parse::<f32>(),
                            values[6].parse::<f32>(),
                            values[7].parse::<f32>(),
                            values[8].parse::<f32>(),
                        )
                    {
                        let mut helper_weights = [w0, w1, w2];
                        let sum = helper_weights.iter().sum::<f32>();
                        if sum.abs() > f32::EPSILON {
                            helper_weights[0] /= sum;
                            helper_weights[1] /= sum;
                            helper_weights[2] /= sum;
                        }
                        helper_map.push(MhcloVertexMap::Triangle {
                            helper_verts: [v0, v1, v2],
                            helper_weights,
                            helper_offset: Vec3::new(ox, oy, oz),
                        });
                    }
                }
                FileSection::DeleteVertices => {
                    let values = line.split_whitespace().collect::<Vec<_>>();
                    let mut start: Option<u16> = None;
                    let mut grouping = false;

                    for value in values {
                        if grouping {
                            if let Some(s) = start && let Ok(end) = value.parse::<u16>() {
                                for idx in s..=end {
                                    delete_verts.insert(idx);
                                }
                            }
                            start = None;
                            grouping = false;
                        } else if value != "-" {
                            if let Some(s) = start {
                                delete_verts.insert(s);
                            }
                            start = value.parse::<u16>().ok();
                        } else {
                            grouping = true;
                        }
                    }

                    if let Some(s) = start {
                        delete_verts.insert(s);
                    }
                }
            }
        }

        Ok(MhcloAsset {
            name,
            obj_file,
            tags,
            z_depth,
            x_scale,
            y_scale,
            z_scale,
            helper_map,
            delete_verts: delete_verts.into_iter().collect(),
        })
    }

    fn extensions(&self) -> &[&str] {
        &["mhclo", "proxy"]
    }
}

impl MhcloAsset {
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    pub(crate) fn get_offset_scale_mhclo(&self, helpers: &[Vec3]) -> Vec3 {
        let x_scale = self.x_scale.as_ref();
        let y_scale = self.y_scale.as_ref();
        let z_scale = self.z_scale.as_ref();

        Vec3::new(
            x_scale.map_or(1.0, |s| {
                (helpers[s.max as usize].x - helpers[s.min as usize].x) / s.scale
            }),
            y_scale.map_or(1.0, |s| {
                (helpers[s.max as usize].y - helpers[s.min as usize].y) / s.scale
            }),
            z_scale.map_or(1.0, |s| {
                (helpers[s.max as usize].z - helpers[s.min as usize].z) / s.scale
            }),
        )
    }

}
