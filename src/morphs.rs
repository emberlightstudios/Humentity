use bevy::{asset::RenderAssetUsages, ecs::intern::Internable, prelude::*};
use std::{
    fs::File,
    io::{ BufReader, BufRead },
    path::PathBuf,
};
use ahash::{AHashMap};
use serde::Deserialize;
use serde_json;
use walkdir::WalkDir;
use crate::{assets::HumanAssetData, basemesh::BODY_SCALE, mesh_ops::{get_uv_coords, get_vertex_positions, get_vertex_tangents}, prelude::*};

/*--------------+
 |  Components  |
 +--------------*/
#[derive(Component, Deref, DerefMut, Clone, Default)]
pub struct MorphTargets(AHashMap<Name, f32>);

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource)]
pub struct HumanMorphs {
    macro_morphs: MacroData,
    composite_morphs: CompositeMorphs,
    targets: AHashMap<Name, AHashMap<u16, Vec3>>,
}

impl FromWorld for HumanMorphs {
    fn from_world(world: &mut World) -> Self {
        // Create Morph Target Entities from all the .target files
        let core_path: PathBuf;
        let config = world.get_resource::<HumentityPathsConfig>()
            .expect("No global Humentity config loaded");
        core_path = config.core_assets_path.clone();
        let target_paths = config.target_paths.clone();
        let mut targets = AHashMap::<Name, AHashMap<u16, Vec3>>::default();
        for target_path in target_paths.iter() {
            for entry in WalkDir::new(core_path.join(target_path)).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                let mut offsets = AHashMap::<u16, Vec3>::default();
                if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("target") {
                    let Some(filename) = path.file_name().unwrap().to_str() else { continue };
                    let Some(stem) = path.file_stem().unwrap().to_str() else { continue };
                    let err_msg = "Couldn't open target file ".to_string() + filename;
                    let file = File::open(path).expect(&err_msg);
                    for line_result in BufReader::new(file).lines() {
                        let Ok(line) = line_result else { break };
                        let mut line_elements = line.split_whitespace();
                        let Some(vert_str) = line_elements.next() else { continue };
                        let Ok(vert) = vert_str.parse::<u16>() else { continue };
                        let coords: Vec<f32> = line_elements
                                              .filter_map(|x| x.parse().ok())
                                              .collect();
                        offsets.insert(vert, Vec3::from_slice(&coords[..]) * BODY_SCALE);
                    }
                    targets.insert(Name::new(NAME_INTERNER.intern(stem).leak()), offsets.clone());
                }
            };
        };
        let file = File::open(core_path.join("targets/macrodetails/macro.json")).expect("FAILED TO OPEN macro.json");
        let reader = BufReader::new(file);
        let macro_sliders: MacroData = serde_json::from_reader(reader).expect("FAILED TO PARSE macro.json");

        let file = File::open(core_path.join("targets/target.json")).expect("FAILED TO OPEN target.json");
        let reader = BufReader::new(file);
        let mut composite_morphs: CompositeMorphs = serde_json::from_reader(reader).expect("FAILED TO PARSE target.json");
        for (_category, targets) in composite_morphs.0.iter_mut() {
            for target in targets.morphs.iter_mut() {
                if target.opposites.is_some() {
                    target.targets = None;
                } else {
                    target.opposites = None;
                    if target.targets.iter().len() > 1 { panic! {"Should not have more than 1 target without opposites"} }
                }
            }
        }
        HumanMorphs { targets, composite_morphs, macro_morphs: macro_sliders }
    }
}

impl HumanMorphs {
    /// Gets all available morph names
    pub fn get_morph_names(&self) -> AHashMap<Name, Vec<Name>> {
        let mut sliders = AHashMap::<Name, Vec<Name>>::default();
        let mut macro_sliders = vec![Name::new("caucasian"), Name::new("asian"), Name::new("african")];
        macro_sliders.extend(self.macro_morphs.macrotargets
            .keys()
            .map(|n| n.clone())
            .collect::<Vec<Name>>()
        );
        sliders.insert(Name::new("macro"), macro_sliders);

        for (category, morphs) in self.composite_morphs.0.iter() {
            if morphs.morphs.is_empty() { continue }
            sliders.insert(
                category.clone(),
                morphs.morphs
                    .iter()
                    .map(|m| m.name.clone())
                    .collect::<Vec<Name>>()
            );
        }

        sliders.insert(Name::new("asymmetry"), self.get_asymetry_target_names());

        sliders
    }

    /// Get asymmetry targets, not composite.  Everything else should be macro or composite
    pub fn get_asymetry_target_names(&self) -> Vec<Name> {
        self.targets
            .iter()
            .filter(|(name, _)| name.as_str().starts_with("asym"))
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    }

    /// Given a single unified slider map, resolve all macro and composite morphs to final target weights.
    pub fn compute_target_weights(&self, morph_targets: &MorphTargets) -> MorphTargets {
        let mut result = AHashMap::default();

        // --- 1️⃣ Separate race sliders ---
        let race_sliders: AHashMap<_, _> = morph_targets
            .iter()
            .filter(|(k, _)| ["african", "asian", "caucasian"].contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        let total_race: f32 = race_sliders.values().sum();
        let race_weights: AHashMap<_, _> = if total_race > 0.0 {
            race_sliders
                .iter()
                .map(|(k, v)| (k.clone(), v / total_race))
                .collect()
        } else {
            // Default to Caucasian=1 if not specified
            let mut m = AHashMap::default();
            m.insert(Name::from("caucasian"), 1.0);
            m
        };

        // --- 2️⃣ Split out macros ---
        let mut macro_inputs = AHashMap::default();

        for (k, v) in morph_targets.iter() {
            if self.macro_morphs.macrotargets.contains_key(k) {
                macro_inputs.insert(k.clone(), *v);
            }
        }

        // Handle defaults for macros
        let name = Name::new("gender");
        if !macro_inputs.contains_key(&name) {
            macro_inputs.insert(name, 1.0);  // Male
        }
        let name = Name::new("age");
        if !macro_inputs.contains_key(&name) {
            macro_inputs.insert(name, 0.5);    // Young
        }

        // --- 3️⃣ Compute macro morphs ---
        let macro_morphs = Self::compute_macro_weights(
            &self.macro_morphs, &macro_inputs);

        let mut macro_combos = AHashMap::<&str, &[&str]>::default();
        macro_combos.insert("race", &["caucasian", "asian", "african"]);
        macro_combos.insert("gender", &["male", "female"]);
        macro_combos.insert("age", &["baby", "child", "young", "old"]);
        macro_combos.insert("muscle", &["minmuscle", "averagemuscle", "maxmuscle"]);
        macro_combos.insert("weight", &["minweight", "averageweight", "maxweight"]);
        macro_combos.insert("proportions", &["uncommonproportions", "idealproportions"]);
        macro_combos.insert("height", &["minheight", "maxheight"]);
        macro_combos.insert("cupsize", &["averagecup", "maxcup"]);
        macro_combos.insert("firmness", &["minfirmness", "averagefirmness", "maxfirmness"]);


        let gender_values = macro_morphs
            .iter()
            .filter(|(&ref n, &_)| macro_combos["gender"].contains(&n.as_str()))
            .map(|(n, v)| (n.clone(), *v))
            .collect::<AHashMap<Name, f32>>();
        let age_values = macro_morphs
            .iter()
            .filter(|(&ref n, &_)| macro_combos["age"].contains(&n.as_str()))
            .map(|(n, v)| (n.clone(), *v))
            .collect::<AHashMap<Name, f32>>();
        let muscle_values = macro_morphs
            .iter()
            .filter(|(&ref n, &_)| macro_combos["muscle"].contains(&n.as_str()))
            .map(|(n, v)| (n.clone(), *v))
            .collect::<AHashMap<Name, f32>>();
        let weight_values = macro_morphs
            .iter()
            .filter(|(&ref n, &_)| macro_combos["weight"].contains(&n.as_str()))
            .map(|(n, v)| (n.clone(), *v))
            .collect::<AHashMap<Name, f32>>();
        let proportions_values = macro_morphs
            .iter()
            .filter(|(&ref n, &_)| macro_combos["proportions"].contains(&n.as_str()))
            .map(|(n, v)| (n.clone(), *v))
            .collect::<AHashMap<Name, f32>>();
        let height_values = macro_morphs
            .iter()
            .filter(|(&ref n, &_)| macro_combos["height"].contains(&n.as_str()))
            .map(|(n, v)| (n.clone(), *v))
            .collect::<AHashMap<Name, f32>>();
        let cupsize_values = macro_morphs
            .iter()
            .filter(|(&ref n, &_)| macro_combos["cupsize"].contains(&n.as_str()))
            .map(|(n, v)| (n.clone(), *v))
            .collect::<AHashMap<Name, f32>>();
        let firmness_values = macro_morphs
            .iter()
            .filter(|(&ref n, &_)| macro_combos["firmness"].contains(&n.as_str()))
            .map(|(n, v)| (n.clone(), *v))
            .collect::<AHashMap<Name, f32>>();

        // race-gender-age targets
        for (race, race_value) in race_weights.iter() {
            for (gender, gender_value) in gender_values.iter() {
                for (age, age_value) in age_values.iter() {
                    let name = NAME_INTERNER.intern(&format!("{race}-{gender}-{age}")).leak();
                    let value = race_value * gender_value * age_value;
                    result.insert(Name::new(name), value);
                }
            }
        }
        
        // universal-gender-age-muscle-weight targets
        for (gender, gender_value) in gender_values.iter() {
            for (age, age_value) in age_values.iter() {
                for (muscle, muscle_value) in muscle_values.iter() {
                    for (weight, weight_value) in weight_values.iter() {
                        let name = NAME_INTERNER.intern(&format!("universal-{gender}-{age}-{muscle}-{weight}")).leak();
                        let value = gender_value * age_value * muscle_value * weight_value;
                        result.insert(Name::new(name), value);
                    }
                }
            }
        }

        // gender-age-muscle-weight-height targets
        for (gender, gender_value) in gender_values.iter() {
            for (age, age_value) in age_values.iter() {
                for (muscle, muscle_value) in muscle_values.iter() {
                    for (weight, weight_value) in weight_values.iter() {
                        for (height, height_value) in height_values.iter() {
                            let name = NAME_INTERNER.intern(&format!("{gender}-{age}-{muscle}-{weight}-{height}")).leak();
                            let value = gender_value * age_value * muscle_value * weight_value * height_value;
                            result.insert(Name::new(name), value);
                        }
                    }
                }
            }
        }

        // gender-age-muscle-weight-proportions targets
        for (gender, gender_value) in gender_values.iter() {
            for (age, age_value) in age_values.iter() {
                for (muscle, muscle_value) in muscle_values.iter() {
                    for (weight, weight_value) in weight_values.iter() {
                        for (proportions, proportions_value) in proportions_values.iter() {
                            let name = NAME_INTERNER.intern(&format!("{gender}-{age}-{muscle}-{weight}-{proportions}")).leak();
                            let value = gender_value * age_value * muscle_value * weight_value * proportions_value;
                            result.insert(Name::new(name), value);
                        }
                    }
                }
            }
        }

        // gender-age-muscle-weight-proportions targets
        for (gender, gender_value) in gender_values.iter() {
            for (age, age_value) in age_values.iter() {
                for (muscle, muscle_value) in muscle_values.iter() {
                    for (weight, weight_value) in weight_values.iter() {
                        for (cupsize, cupsize_value) in cupsize_values.iter() {
                            for (firmness, firmness_value) in firmness_values.iter() {
                                let name = NAME_INTERNER.intern(&format!("{gender}-{age}-{muscle}-{weight}-{cupsize}-{firmness}")).leak();
                                let value = gender_value * age_value * muscle_value * weight_value * cupsize_value * firmness_value;
                                result.insert(Name::new(name), value);
                            }
                        }
                    }
                }
            }
        }

        // -----------------------------------
        // 2. Resolve composite morph sliders
        // -----------------------------------
        for (slider_name, value) in morph_targets.iter() {
            // Find the composite morph definition that matches this slider
            if let Some(morph_list) = self.composite_morphs.0.get(slider_name) {
                for morph in morph_list.morphs.iter() {
                    // Some morphs just directly reference targets
                    if let Some(targets) = &morph.targets {
                        // Apply evenly or using sign if opposites are present
                        if let Some(opps) = &morph.opposites {
                            if morph.has_left_and_right {
                                if *value > 0.0 {
                                    *result.entry(opps.positive_right.clone()).or_insert(0.0) += value.abs();
                                    *result.entry(opps.positive_left.clone()).or_insert(0.0) += value.abs();
                                } else {
                                    *result.entry(opps.negative_right.clone()).or_insert(0.0) += value.abs();
                                    *result.entry(opps.negative_left.clone()).or_insert(0.0) += value.abs();
                                }
                            } else {
                                if *value > 0.0 {
                                    *result.entry(opps.positive_unsided.clone()).or_insert(0.0) += value.abs();
                                } else {
                                    *result.entry(opps.negative_unsided.clone()).or_insert(0.0) += value.abs();
                                }
                            }
                        } else {
                            // No opposites: directly apply
                            for target in targets {
                                *result.entry(target.clone()).or_insert(0.0) += *value;
                            }
                        }
                    }
                }
            }
        }

        // --------------------------------------
        // 3. Resolve direct target morph sliders
        // --------------------------------------
        for (slider_name, value) in morph_targets
            .iter()
            .filter(|(name, _)| name.as_str().starts_with("asym"))
        {
            *result.entry(slider_name.clone()).or_insert(0.0) += *value;
        }

        // ---------------------------------------
        // 4. Normalize or clamp values if needed
        // ---------------------------------------
        for val in result.values_mut() {
            if !val.is_finite() {
                *val = 0.0;
            }
            *val = val.clamp(-1.0, 1.0);
        }

        MorphTargets(result)
    }

    /// Helper for step 3
    fn compute_macro_weights(
        macros: &MacroData,
        slider_values: &AHashMap<Name, f32>,
    ) -> AHashMap<Name, f32> {
        let mut result = AHashMap::default();

        for (macro_name, value) in slider_values {
            if let Some(bounds) = macros.macrotargets.get(macro_name) {
                for part in &bounds.parts {
                    if *value > part.lowest && *value <= part.highest {
                        let range = part.highest - part.lowest;
                        let mut t = if range > 0.0 {
                            (value - part.lowest) / range
                        } else {
                            panic!("Invalid macro bounds");
                        };
                        if t < 0.005 { t = 0. }
                        if t > 0.995 { t = 1. }

                        match (&part.low[..], &part.high[..]) {
                            ("", "") => {}
                            (low, "") if !low.is_empty() => {
                                *result.entry(Name::from(low.to_string())).or_insert(0.0) += 1.0 - t;
                            }
                            ("", high) if !high.is_empty() => {
                                *result.entry(Name::from(high.to_string())).or_insert(0.0) += t;
                            }
                            (low, high) => {
                                *result.entry(Name::from(low.to_string())).or_insert(0.0) += 1.0 - t;
                                *result.entry(Name::from(high.to_string())).or_insert(0.0) += t;
                            }
                        }

                        break;
                    }
                }
            }
        }

        result
            .into_iter()
            .filter(|(_, v)| *v != 0.)
            .collect::<AHashMap::<Name, f32>>()
    }
}

/*-------------+
 |  Functions  |
 +-------------*/
pub(crate) fn adjust_helpers_to_morphs(
    morph_values: &MorphTargets,
    morph_targets: &HumanMorphs,
    basemesh: &crate::basemesh::BaseMesh,
) -> Vec<Vec3> {
    let mut helpers = basemesh.vertices.clone();
    for (target_name, &value) in morph_values.iter() {
        let err_msg = format!("Failed to find morph {}", target_name);
        let target = morph_targets.targets.get(target_name).expect(&err_msg);
        for (&vertex, &offset) in target.iter() {
            helpers[vertex as usize] += offset * value;
        }
    }
    helpers
}

pub(crate) fn asset_mesh_from_helpers(
    helpers: &Vec<Vec3>,
    morph_values: &AHashMap<Name, f32>,
    morph_targets: &HumanMorphs,
    meshes: &mut Assets<Mesh>,
    asset_data: &HumanAssetData,
) -> Handle<Mesh> {
    let mesh = meshes.get(&asset_data.base_mesh_handle).unwrap().clone();
    let mut vertices = get_vertex_positions(&mesh);
    for (target_name, &value) in morph_values.iter() {
        let err_msg = format!("Failed to find morph {}", target_name);
        let target = morph_targets.targets.get(target_name).expect(&err_msg);

        for (vert, mh_vert) in asset_data.mhid_lookup.iter().enumerate() {
            let helper_map = &asset_data.helper_map[*mh_vert as usize];
            if let Some(mh_vtx) = helper_map.single_vertex {
                if let Some(offset) = target.get(&mh_vtx) {
                    vertices[vert] += offset * value;
                }
            } else { // Triangulation
                let triangle = helper_map.triangle.as_ref().unwrap();
                let mut position = Vec3::ZERO;
                for i in 0..3 {
                    let mh_vert = triangle.helper_verts[i];
                    let wt = triangle.helper_weights[i];
                    position += *helpers.get(mh_vert as usize).unwrap() * wt;
                }
                position += asset_data.get_offset_scale(helpers) * triangle.helper_offset;
                let offset = position - vertices[vert];
                vertices[vert] += offset * value;
            }
        }
    }
    let mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, get_uv_coords(&mesh))
        .with_inserted_indices(mesh.indices().unwrap().clone())
        .with_computed_area_weighted_normals()
        .with_generated_tangents()
        .unwrap();

    meshes.add(mesh)
}

/*--------------+
 |  JSON Types  |
 +--------------*/
#[derive(Deserialize, Debug)]
struct MacroData {
    macrotargets: AHashMap<Name, MacroBounds>,
}

#[derive(Deserialize, Debug)]
struct MacroBounds {
    parts: Vec<MacroBound>,
}

#[derive(Deserialize, Debug)]
struct MacroBound {
    lowest: f32,
    highest: f32,
    low: Name,
    high: Name,
}

#[derive(Deserialize, Debug)]
struct CompositeMorphs(AHashMap<Name, CategoryMorphs>);

#[derive(Deserialize, Debug)]
struct CategoryMorphs{
    #[serde(rename = "categories")]
    morphs: Vec<CompositeMorph>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
struct CompositeMorph {
    has_left_and_right: bool,
    name: Name,
    opposites: Option<Opposites>,
    targets: Option<Vec<Name>>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
struct Opposites {
    negative_left: Name,
    negative_right: Name,
    negative_unsided: Name,
    positive_left: Name,
    positive_right: Name,
    positive_unsided: Name,
}
