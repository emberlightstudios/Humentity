use bevy::{
    asset::AssetPath,
    asset::{AssetLoader, LoadContext, io::Reader},
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
        let raw_text = String::from_utf8(bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

        let asset_label = load_context
            .path()
            .path()
            .to_string_lossy()
            .into_owned();
        let mut obj_file = AssetPath::default();
        let mut tags = Vec::new();
        let mut z_depth = 0_i8;
        let mut x_scale = None;
        let mut y_scale = None;
        let mut z_scale = None;
        let mut helper_map = Vec::new();
        let mut delete_verts = BTreeSet::<u16>::new();
        let mut section = FileSection::Header;
        let mut line_number = 0_usize;

        for raw_line in raw_text.lines() {
            line_number += 1;
            let line = raw_line.trim();
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
                    "obj_file" => {
                        let obj_token = parts.next().unwrap_or_else(|| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has 'obj_file' with no path (got '{raw_line}')"
                            )
                        });
                        let parent = load_context.path().parent().unwrap_or_else(|| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} cannot resolve obj_file '{obj_token}': asset has no parent folder"
                            )
                        });
                        obj_file = parent.resolve_str(obj_token).unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} cannot resolve obj_file '{obj_token}' (got '{raw_line}')"
                            )
                        });
                    }
                    "tag" => {
                        let tag_text = parts.collect::<Vec<_>>().join(" ");
                        if tag_text.is_empty() {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has 'tag' with no text (got '{raw_line}')"
                            );
                        }
                        tags.push(tag_text);
                    }
                    "x_scale" | "y_scale" | "z_scale" => {
                        let scale_values =
                            line.split_whitespace().skip(1).collect::<Vec<_>>();
                        if scale_values.len() < 3 {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has '{key}' with {} values, expected 3 ('<min> <max> <scale>', got '{raw_line}')",
                                scale_values.len(),
                            );
                        }
                        let min: u16 = scale_values[0].parse().unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has a corrupt '{key}' min '{}' (got '{raw_line}')",
                                scale_values[0],
                            )
                        });
                        let max: u16 = scale_values[1].parse().unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has a corrupt '{key}' max '{}' (got '{raw_line}')",
                                scale_values[1],
                            )
                        });
                        let scale: f32 = scale_values[2].parse().unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has a corrupt '{key}' scale '{}' (got '{raw_line}')",
                                scale_values[2],
                            )
                        });
                        let axis = Some(MhcloScaleAxis { min, max, scale });
                        match key {
                            "x_scale" => x_scale = axis,
                            "y_scale" => y_scale = axis,
                            _ => z_scale = axis,
                        }
                    }
                    "z_depth" => {
                        let depth_token = parts.next().unwrap_or_else(|| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has 'z_depth' with no value (got '{raw_line}')"
                            )
                        });
                        z_depth = depth_token.parse().unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has a corrupt z_depth '{depth_token}' (got '{raw_line}')"
                            )
                        });
                    }
                    _ => {}
                },
                FileSection::Vertices => {
                    let vert_tokens = line.split_whitespace().collect::<Vec<_>>();
                    if vert_tokens.is_empty() || vert_tokens[0] == "material" {
                        continue;
                    }

                    if vert_tokens.len() == 1 {
                        let vertex: u16 = vert_tokens[0].parse().unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has a corrupt single-vertex entry '{}' (got '{raw_line}')",
                                vert_tokens[0],
                            )
                        });
                        helper_map.push(MhcloVertexMap::SingleVertex(vertex));
                        continue;
                    }

                    if vert_tokens.len() != 9 {
                        panic!(
                            "mhclo '{asset_label}' line {line_number} has {} values, expected 1 or 9 ('<v>' or '<v0> <v1> <v2> <w0> <w1> <w2> <ox> <oy> <oz>', got '{raw_line}')",
                            vert_tokens.len(),
                        );
                    }
                    let parsed_vertex = |token_text: &str| -> u16 {
                        token_text.parse().unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has a corrupt helper vertex '{token_text}' (got '{raw_line}')"
                            )
                        })
                    };
                    let parsed_weight = |token_text: &str| -> f32 {
                        token_text.parse().unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has a corrupt helper weight '{token_text}' (got '{raw_line}')"
                            )
                        })
                    };
                    let parsed_offset = |token_text: &str| -> f32 {
                        token_text.parse().unwrap_or_else(|_| {
                            panic!(
                                "mhclo '{asset_label}' line {line_number} has a corrupt helper offset '{token_text}' (got '{raw_line}')"
                            )
                        })
                    };
                    let (v0, v1, v2) = (
                        parsed_vertex(vert_tokens[0]),
                        parsed_vertex(vert_tokens[1]),
                        parsed_vertex(vert_tokens[2]),
                    );
                    let (w0, w1, w2) = (
                        parsed_weight(vert_tokens[3]),
                        parsed_weight(vert_tokens[4]),
                        parsed_weight(vert_tokens[5]),
                    );
                    let (ox, oy, oz) = (
                        parsed_offset(vert_tokens[6]),
                        parsed_offset(vert_tokens[7]),
                        parsed_offset(vert_tokens[8]),
                    );
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
                FileSection::DeleteVertices => {
                    let delete_tokens = line.split_whitespace().collect::<Vec<_>>();
                    let mut range_start: Option<u16> = None;
                    let mut grouping = false;

                    for delete_token in delete_tokens {
                        if grouping {
                            let range_begin = range_start.unwrap_or_else(|| {
                                panic!(
                                    "mhclo '{asset_label}' line {line_number} has '-' with no range start (got '{raw_line}')"
                                )
                            });
                            let range_end: u16 = delete_token.parse().unwrap_or_else(|_| {
                                panic!(
                                    "mhclo '{asset_label}' line {line_number} has a corrupt delete range end '{delete_token}' (got '{raw_line}')"
                                )
                            });
                            for vert_index in range_begin..=range_end {
                                delete_verts.insert(vert_index);
                            }
                            range_start = None;
                            grouping = false;
                        } else if delete_token != "-" {
                            if let Some(pending_vert) = range_start {
                                delete_verts.insert(pending_vert);
                            }
                            range_start = Some(delete_token.parse().unwrap_or_else(|_| {
                                panic!(
                                    "mhclo '{asset_label}' line {line_number} has a corrupt delete vertex '{delete_token}' (got '{raw_line}')"
                                )
                            }));
                        } else {
                            grouping = true;
                        }
                    }

                    if grouping {
                        panic!(
                            "mhclo '{asset_label}' line {line_number} ends with a dangling '-' and no range end (got '{raw_line}')"
                        );
                    }
                    if let Some(pending_vert) = range_start {
                        delete_verts.insert(pending_vert);
                    }
                }
            }
        }

        Ok(MhcloAsset {
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
