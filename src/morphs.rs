use crate::{basemesh::BODY_SCALE, prelude::*};
use ahash::AHashMap;
use bevy::{ecs::intern::Internable, prelude::*};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};
use walkdir::WalkDir;

/*--------------+
|  Components  |
+--------------*/
#[derive(Component, Deref, DerefMut, Clone, Default, Debug)]
pub struct MorphTargets(AHashMap<&'static str, f32>);

impl Serialize for MorphTargets {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Convert keys to owned Strings for serialization
        let map: AHashMap<String, f32> = self.0.iter().map(|(&k, &v)| (k.to_string(), v)).collect();
        map.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MorphTargets {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map: AHashMap<String, f32> = AHashMap::deserialize(deserializer)?;
        let mut result = AHashMap::new();

        for (k, v) in map {
            let key: &'static str = NAME_INTERNER.intern(&k).leak();
            result.insert(key, v);
        }

        Ok(MorphTargets(result))
    }
}

/*-------------+
|  Resources  |
+-------------*/
#[derive(Resource)]
pub struct MakeHumanMorphs {
    macro_morphs: MacroData,
    composite_categories: AHashMap<&'static str, Vec<&'static str>>,
    composite_morphs: AHashMap<&'static str, CompositeMorph>,
    pub targets: AHashMap<&'static str, AHashMap<u16, Vec3>>,
}

impl FromWorld for MakeHumanMorphs {
    fn from_world(world: &mut World) -> Self {
        // Create Morph Target Entities from all the .target files
        
        let config = world
            .get_resource::<HumentityPathsConfig>()
            .expect("No global Humentity config loaded");
        let core_path: PathBuf = config.core_assets_path.clone();
        let target_paths = config.target_paths.clone();
        let mut targets = AHashMap::<&'static str, AHashMap<u16, Vec3>>::default();
        for target_path in target_paths.iter() {
            for entry in WalkDir::new(target_path).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                let mut offsets = AHashMap::<u16, Vec3>::default();
                if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("target") {
                    let Some(filename) = path.file_name().unwrap().to_str() else {
                        continue;
                    };
                    let Some(stem) = path.file_stem().unwrap().to_str() else {
                        continue;
                    };
                    let file =
                        File::open(path).unwrap_or_else(|_| panic!("Couldn't open target file {}", filename));
                    for line_result in BufReader::new(file).lines() {
                        let Ok(line) = line_result else { break };
                        let mut line_elements = line.split_whitespace();
                        let Some(vert_str) = line_elements.next() else {
                            continue;
                        };
                        let Ok(vert) = vert_str.parse::<u16>() else {
                            continue;
                        };
                        let coords: Vec<f32> =
                            line_elements.filter_map(|x| x.parse().ok()).collect();
                        offsets.insert(vert, Vec3::from_slice(&coords[..]) * BODY_SCALE);
                    }
                    targets.insert(NAME_INTERNER.intern(stem).leak(), offsets.clone());
                }
            }
        }
        let file = File::open(core_path.join("targets/macrodetails/macro.json"))
            .expect("FAILED TO OPEN macro.json");
        let reader = BufReader::new(file);
        let macro_sliders: MacroData =
            serde_json::from_reader(reader).expect("FAILED TO PARSE macro.json");

        let file =
            File::open(core_path.join("targets/target.json")).expect("FAILED TO OPEN target.json");
        let reader = BufReader::new(file);
        let mut composite_morph_categories: CompositeMorphs =
            serde_json::from_reader(reader).expect("FAILED TO PARSE target.json");
        for (_category, targets) in composite_morph_categories.0.iter_mut() {
            for target in targets.morphs.iter_mut() {
                if target.opposites.is_some() {
                    target.targets = None;
                } else {
                    target.opposites = None;
                    if target.targets.iter().len() > 1 {
                        panic! {"Should not have more than 1 target without opposites"}
                    }
                }
            }
        }

        let mut composite_categories = AHashMap::new();
        let mut composite_morphs = AHashMap::new();
        for category in composite_morph_categories.keys() {
            let cat = composite_categories
                .entry(NAME_INTERNER.intern(category).leak())
                .or_insert(vec![]);
            for morph in composite_morph_categories[category].morphs.iter() {
                let morph_name = NAME_INTERNER.intern(&morph.name).leak();
                composite_morphs.insert(morph_name, morph.clone());
                cat.push(morph_name);
            }
        }
        MakeHumanMorphs {
            targets,
            composite_categories,
            composite_morphs,
            macro_morphs: macro_sliders,
        }
    }
}

impl MakeHumanMorphs {
    pub fn get_min_values(&self) -> AHashMap<&'static str, f32> {
        let names = self.get_morph_names();
        let mut result = AHashMap::default();
        for (&category, morph_names) in names.iter() {
            for &morph in morph_names.iter() {
                if category == "macro" || (category == "head" && morph.split('-').count() == 2) {
                    result.insert(morph, 0.);
                } else {
                    result.insert(morph, -1.);
                }
            }
        }
        result
    }

    /// Gets all available morph names
    pub fn get_morph_names(&self) -> AHashMap<&'static str, Vec<&'static str>> {
        let mut sliders = AHashMap::<&'static str, Vec<&'static str>>::default();
        let mut macro_sliders = vec!["caucasian", "asian", "african"];
        macro_sliders.extend(
            self.macro_morphs
                .macrotargets
                .keys()
                .map(|n| NAME_INTERNER.intern(n).leak())
        );
        sliders.insert("macro", macro_sliders);
        sliders.extend(self.composite_categories.clone());
        sliders.insert("asymmetry", self.get_asymetry_target_names());

        sliders
    }

    /// Get asymmetry targets, not composite.  Everything else should be macro or composite
    pub fn get_asymetry_target_names(&self) -> Vec<&'static str> {
        self.targets
            .iter()
            .filter(|(&name, _)| name.starts_with("asym"))
            .map(|(&name, _)| name)
            .collect::<Vec<_>>()
    }

    /// Given a single unified slider map, resolve all macro and composite morphs to final target weights.
    pub fn compute_target_weights(&self, morph_targets: &MorphTargets) -> MorphTargets {
        let mut result = MorphTargets::default();

        // --- 1️⃣ Separate race sliders ---
        let race_sliders: AHashMap<_, _> = morph_targets
            .iter()
            .filter(|(&k, _)| ["african", "asian", "caucasian"].contains(&k))
            .map(|(&k, v)| (k, *v))
            .collect();

        let total_race: f32 = race_sliders.values().sum();
        let race_weights: AHashMap<_, _> = if total_race > 0.0 {
            race_sliders
                .iter()
                .map(|(&k, v)| (k, v / total_race))
                .collect()
        } else {
            // Default to Caucasian=1 if not specified
            let mut m = AHashMap::default();
            m.insert("caucasian", 1.0);
            m
        };

        // --- 2️⃣ Split out macros ---
        let mut macro_inputs = AHashMap::default();

        for (&k, v) in morph_targets.iter() {
            if self
                .macro_morphs
                .macrotargets
                .contains_key(&String::from(k))
            {
                macro_inputs.insert(k, *v);
            }
        }

        // Handle defaults for macros
        if !macro_inputs.contains_key("gender") {
            macro_inputs.insert("gender", 1.0); // Male
        }
        if !macro_inputs.contains_key("age") {
            macro_inputs.insert("age", 0.5); // Young
        }

        // --- 3️⃣ Compute macro morphs ---
        let macro_morphs = Self::compute_macro_weights(&self.macro_morphs, &macro_inputs);

        let mut macro_combos = AHashMap::<&str, &[&str]>::default();
        macro_combos.insert("race", &["caucasian", "asian", "african"]);
        macro_combos.insert("gender", &["male", "female"]);
        macro_combos.insert("age", &["baby", "child", "young", "old"]);
        macro_combos.insert("muscle", &["minmuscle", "averagemuscle", "maxmuscle"]);
        macro_combos.insert("weight", &["minweight", "averageweight", "maxweight"]);
        macro_combos.insert("proportions", &["uncommonproportions", "idealproportions"]);
        macro_combos.insert("height", &["minheight", "maxheight"]);
        macro_combos.insert("cupsize", &["averagecup", "maxcup"]);
        macro_combos.insert(
            "firmness",
            &["minfirmness", "averagefirmness", "maxfirmness"],
        );

        let gender_values = macro_morphs
            .iter()
            .filter(|(n, &_)| macro_combos["gender"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let age_values = macro_morphs
            .iter()
            .filter(|(n, &_)| macro_combos["age"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let muscle_values = macro_morphs
            .iter()
            .filter(|(n, &_)| macro_combos["muscle"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let weight_values = macro_morphs
            .iter()
            .filter(|(n, &_)| macro_combos["weight"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let proportions_values = macro_morphs
            .iter()
            .filter(|(n, &_)| macro_combos["proportions"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let height_values = macro_morphs
            .iter()
            .filter(|(n, &_)| macro_combos["height"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let cupsize_values = macro_morphs
            .iter()
            .filter(|(n, &_)| macro_combos["cupsize"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let firmness_values = macro_morphs
            .iter()
            .filter(|(n, &_)| macro_combos["firmness"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();

        // race-gender-age targets
        for (&race, race_value) in race_weights.iter() {
            for (&gender, gender_value) in gender_values.iter() {
                for (&age, age_value) in age_values.iter() {
                    let name = NAME_INTERNER
                        .intern(&format!("{race}-{gender}-{age}"))
                        .leak();
                    let value = race_value * gender_value * age_value;
                    result.insert(name, value);
                }
            }
        }

        // universal-gender-age-muscle-weight targets
        for (&gender, gender_value) in gender_values.iter() {
            for (&age, age_value) in age_values.iter() {
                for (&muscle, muscle_value) in muscle_values.iter() {
                    for (&weight, weight_value) in weight_values.iter() {
                        let name = NAME_INTERNER
                            .intern(&format!("universal-{gender}-{age}-{muscle}-{weight}"))
                            .leak();
                        let value = gender_value * age_value * muscle_value * weight_value;
                        result.insert(name, value);
                    }
                }
            }
        }

        // gender-age-muscle-weight-height targets
        for (&gender, gender_value) in gender_values.iter() {
            for (&age, age_value) in age_values.iter() {
                for (&muscle, muscle_value) in muscle_values.iter() {
                    for (&weight, weight_value) in weight_values.iter() {
                        for (&height, height_value) in height_values.iter() {
                            let name = NAME_INTERNER
                                .intern(&format!("{gender}-{age}-{muscle}-{weight}-{height}"))
                                .leak();
                            let value = gender_value
                                * age_value
                                * muscle_value
                                * weight_value
                                * height_value;
                            result.insert(name, value);
                        }
                    }
                }
            }
        }

        // gender-age-muscle-weight-proportions targets
        for (&gender, gender_value) in gender_values.iter() {
            for (&age, age_value) in age_values.iter() {
                if age == "baby" {
                    continue;
                }
                for (&muscle, muscle_value) in muscle_values.iter() {
                    for (&weight, weight_value) in weight_values.iter() {
                        for (&proportions, proportions_value) in proportions_values.iter() {
                            let name = NAME_INTERNER
                                .intern(&format!("{gender}-{age}-{muscle}-{weight}-{proportions}"))
                                .leak();
                            let value = gender_value
                                * age_value
                                * muscle_value
                                * weight_value
                                * proportions_value;
                            result.insert(name, value);
                        }
                    }
                }
            }
        }

        // gender-age-muscle-weight-cup-firmness targets
        for (&gender, gender_value) in gender_values.iter() {
            if gender == "male" {
                continue;
            }
            for (&age, age_value) in age_values.iter() {
                if age == "baby" {
                    continue;
                }
                for (&muscle, muscle_value) in muscle_values.iter() {
                    for (&weight, weight_value) in weight_values.iter() {
                        for (&cupsize, cupsize_value) in cupsize_values.iter() {
                            for (&firmness, firmness_value) in firmness_values.iter() {
                                if firmness == "averagefirmness" && cupsize == "averagecup" {
                                    continue;
                                }
                                let name = NAME_INTERNER
                                    .intern(&format!(
                                        "{gender}-{age}-{muscle}-{weight}-{cupsize}-{firmness}"
                                    ))
                                    .leak();
                                let value = gender_value
                                    * age_value
                                    * muscle_value
                                    * weight_value
                                    * cupsize_value
                                    * firmness_value;
                                result.insert(name, value);
                            }
                        }
                    }
                }
            }
        }

        // -----------------------------------
        // 2. Resolve composite morph sliders
        // -----------------------------------
        for (&slider_name, value) in morph_targets.iter() {
            // Find the composite morph definition that matches this slider
            if let Some(morph) = self.composite_morphs.get(slider_name) {
                // Some morphs just directly reference targets
                // Apply evenly or using sign if opposites are present
                if let Some(opps) = &morph.opposites {
                    if morph.has_left_and_right {
                        if *value > 0.0 {
                            *result
                                .entry(NAME_INTERNER.intern(&opps.positive_right).leak())
                                .or_insert(0.0) += value.abs();
                            *result
                                .entry(NAME_INTERNER.intern(&opps.positive_left).leak())
                                .or_insert(0.0) += value.abs();
                        } else {
                            *result
                                .entry(NAME_INTERNER.intern(&opps.negative_right).leak())
                                .or_insert(0.0) += value.abs();
                            *result
                                .entry(NAME_INTERNER.intern(&opps.negative_left).leak())
                                .or_insert(0.0) += value.abs();
                        }
                    } else if *value > 0.0 {
                        *result
                            .entry(NAME_INTERNER.intern(&opps.positive_unsided).leak())
                            .or_insert(0.0) += value.abs();
                    } else {
                        *result
                            .entry(NAME_INTERNER.intern(&opps.negative_unsided).leak())
                            .or_insert(0.0) += value.abs();
                    }
                } else if let Some(targets) = &morph.targets {
                    // No opposites: directly apply
                    for target in targets {
                        *result
                            .entry(NAME_INTERNER.intern(target).leak())
                            .or_insert(0.0) += *value;
                    }
                }
            }
        }

        // --------------------------------------
        // 3. Resolve direct target morph sliders
        // --------------------------------------
        for (slider_name, value) in morph_targets.iter() {
            if self.targets.contains_key(slider_name) {
                *result.entry(slider_name).or_insert(0.0) = *value;
            }
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

        result
    }

    /// Helper for step 3
    fn compute_macro_weights(
        macros: &MacroData,
        slider_values: &AHashMap<&'static str, f32>,
    ) -> AHashMap<&'static str, f32> {
        let mut result = AHashMap::default();

        for (&macro_name, value) in slider_values {
            if let Some(bounds) = macros.macrotargets.get(macro_name) {
                for part in &bounds.parts {
                    if *value > part.lowest && *value <= part.highest {
                        let range = part.highest - part.lowest;
                        let mut t = if range > 0.0 {
                            (value - part.lowest) / range
                        } else {
                            panic!("Invalid macro bounds");
                        };
                        if t < 0.005 {
                            t = 0.
                        }
                        if t > 0.995 {
                            t = 1.
                        }

                        match (&part.low[..], &part.high[..]) {
                            ("", "") => {}
                            (low, "") if !low.is_empty() => {
                                *result.entry(low.to_string()).or_insert(0.0) += 1.0 - t;
                            }
                            ("", high) if !high.is_empty() => {
                                *result.entry(high.to_string()).or_insert(0.0) += t;
                            }
                            (low, high) => {
                                *result.entry(low.to_string()).or_insert(0.0) += 1.0 - t;
                                *result.entry(high.to_string()).or_insert(0.0) += t;
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
            .map(|(k, v)| (NAME_INTERNER.intern(&k).leak(), v))
            .collect::<AHashMap<&'static str, f32>>()
    }
}

/*-------------+
|  Functions  |
+-------------*/
pub(crate) fn adjust_helpers_to_morphs(
    morph_values: &MorphTargets,
    morph_targets: &MakeHumanMorphs,
    basemesh: &crate::basemesh::BaseMesh,
) -> Vec<Vec3> {
    let mut helpers = basemesh.vertices.clone();
    for (target_name, &value) in morph_values.iter() {
        let target = morph_targets
            .targets
            .get(target_name)
            .unwrap_or_else(|| panic!("Failed to find morph {}", target_name));
        for (&vertex, &offset) in target.iter() {
            helpers[vertex as usize] += offset * value;
        }
    }
    helpers
}

/*--------------+
|  JSON Types  |
+--------------*/
#[derive(Deserialize, Debug)]
struct MacroData {
    macrotargets: AHashMap<String, MacroBounds>,
}

#[derive(Deserialize, Debug)]
struct MacroBounds {
    parts: Vec<MacroBound>,
}

#[derive(Deserialize, Debug)]
struct MacroBound {
    lowest: f32,
    highest: f32,
    low: String,
    high: String,
}

#[derive(Deserialize, Debug, Deref)]
struct CompositeMorphs(AHashMap<String, CategoryMorphs>);

#[derive(Deserialize, Debug)]
struct CategoryMorphs {
    #[serde(rename = "categories")]
    morphs: Vec<CompositeMorph>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
struct CompositeMorph {
    has_left_and_right: bool,
    name: String,
    opposites: Option<Opposites>,
    targets: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
struct Opposites {
    negative_left: String,
    negative_right: String,
    negative_unsided: String,
    positive_left: String,
    positive_right: String,
    positive_unsided: String,
}
