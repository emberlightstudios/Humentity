use std::sync::{Arc, RwLock};

use crate::{
    loaders::{CompositeTargetsAsset, MacroDataAsset, TargetAsset},
    prelude::*,
};
use ahash::AHashMap;
use bevy::{ecs::intern::Internable, prelude::*};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MorphError {
    #[error("Macro data not loaded - cannot resolve macros")]
    MacrosNotLoaded,
    #[error("Composite morph data not loaded - cannot resolve")]
    CompositeNotLoaded,
    #[error("Morph target '{0}' not found in loaded targets")]
    TargetNotFound(&'static str),
}

/// A type for storing generic morph target weights
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

#[derive(Resource)]
pub struct MakeHumanMorphs {
    pub targets: Arc<RwLock<AHashMap<&'static str, TargetAsset>>>,
    pub expressions: Vec<&'static str>,
    macros: MacroDataAsset,
    composites: CompositeTargetsAsset,
}

// Temp resource before assets are moved onto MakeHumanMorphs resource
#[derive(Resource)]
pub(crate) struct MorphHandles {
    macro_handle: Handle<MacroDataAsset>,
    composite_handle: Handle<CompositeTargetsAsset>,
}

pub(crate) fn sync_loaded_morph_manifests(
    macro_assets: Res<Assets<MacroDataAsset>>,
    composite_assets: Res<Assets<CompositeTargetsAsset>>,
    mut commands: Commands,
) {
    if let Some((_, macro_asset)) = macro_assets.iter().next() &&
            let Some((_, composite_asset)) = composite_assets.iter().next() {
        commands.insert_resource(MakeHumanMorphs {
            targets: Arc::new(RwLock::new(AHashMap::default())),
            expressions: Vec::new(),
            macros: macro_asset.clone(),
            composites: composite_asset.clone(),
        });
        commands.remove_resource::<MorphHandles>();
    }
}

pub(crate) fn sync_loaded_morph_targets(
    morphs: Res<MakeHumanMorphs>,
    target_assets: Res<Assets<TargetAsset>>,
    mut target_events: MessageReader<AssetEvent<TargetAsset>>,
) {
    for ev in target_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev {
            let asset = target_assets.get(*id).unwrap();
            let name = asset.name;
            let mut targets = morphs.targets.write().unwrap();
            targets.insert(name, asset.clone());
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
        macro_sliders.extend(self.macros.macrotargets.keys());
        sliders.insert("macro", macro_sliders);
        sliders.insert("asymmetry", self.get_asymetry_target_names());
        for (&category, morphs) in self.composites.iter() {
            let morph_names = morphs.morphs.iter().map(|m| m.name).collect();
            sliders.insert(category, morph_names);
        }
        sliders
    }

    /// Get asymmetry targets, not composite.  Everything else should be macro or composite
    pub fn get_asymetry_target_names(&self) -> Vec<&'static str> {
        let targets = self.targets.read().unwrap();
        targets
            .keys()
            .filter(|&name| name.starts_with("asym"))
            .map(|&name| name)
            .collect::<Vec<_>>()
    }

    /// Given a single unified slider map, resolve all macro and composite morphs to final target weights.
    pub fn compute_target_weights(&self, morph_targets: &MorphTargets) -> Result<MorphTargets, MorphError> {

        let targets = self.targets.read().unwrap();
        // If already resolved to raw target asset names, we have nothing to do
        if targets
            .keys()
            .all(|&t| morph_targets.contains_key(t))
        {
            return Ok(morph_targets.clone());
        }

        let mut result = MorphTargets::default();

        // Check for macro inputs
        if self.macros.macrotargets.is_empty() {
            return Err(MorphError::MacrosNotLoaded);
        }

        // Check for composite inputs
        if self.composites.is_empty() {
            return Err(MorphError::CompositeNotLoaded);
        }

        if morph_targets.keys().all(|&t| targets.contains_key(t)) {
            return Ok(morph_targets.clone());
        }

        // --- 1️⃣ Separate race sliders ---
        let race_sliders: AHashMap<_, _> = morph_targets
            .iter()
            .filter(|(k, _)| ["african", "asian", "caucasian"].contains(k))
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
                .macros
                .macrotargets
                .contains_key(k)
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
        if !macro_inputs.contains_key("weight") {
            macro_inputs.insert("weight", 0.5);
        }
        if !macro_inputs.contains_key("muscle") {
            macro_inputs.insert("muscle", 0.5);
        }
        if !macro_inputs.contains_key("proportions") {
            macro_inputs.insert("proportions", 0.5);
        }

        // --- 3️⃣ Compute macro morphs ---
        let macro_morphs = morph_targets
            .iter()
            .filter(|(k, _)| self.macros.macrotargets.contains_key(*k))
            .map(|(&k, v)| (k, *v))
            .collect::<AHashMap<_, _>>();

        let mut macro_combos = AHashMap::<&str, &[&str]>::default();
        macro_combos.insert("race", &["caucasian", "asian", "african"]);
        macro_combos.insert("gender", &["male", "female"]);
        macro_combos.insert("age", &["baby", "child", "young", "old"]);
        macro_combos.insert("muscle", &["minmuscle", "averagemuscle", "maxmuscle"]);
        macro_combos.insert("weight", &["minweight", "averageweight", "maxweight"]);
        macro_combos.insert("proportions", &["uncommonproportions", "idealproportions"]);
        macro_combos.insert("height", &["minheight", "maxheight"]);
        macro_combos.insert("cupsize", &["mincup", "averagecup", "maxcup"]);
        macro_combos.insert("firmness", &["minfirmness", "averagefirmness", "maxfirmness"]);

        let gender_values = macro_morphs
            .iter()
            .filter(|(n, _)| macro_combos["gender"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let age_values = macro_morphs
            .iter()
            .filter(|(n, _)| macro_combos["age"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let muscle_values = macro_morphs
            .iter()
            .filter(|(n, _)| macro_combos["muscle"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let weight_values = macro_morphs
            .iter()
            .filter(|(n, _)| macro_combos["weight"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let proportions_values = macro_morphs
            .iter()
            .filter(|(n, _)| macro_combos["proportions"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let height_values = macro_morphs
            .iter()
            .filter(|(n, _)| macro_combos["height"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let cupsize_values = macro_morphs
            .iter()
            .filter(|(n, _)| macro_combos["cupsize"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();
        let firmness_values = macro_morphs
            .iter()
            .filter(|(n, _)| macro_combos["firmness"].contains(n))
            .map(|(n, v)| (NAME_INTERNER.intern(n).leak(), *v))
            .collect::<AHashMap<&'static str, f32>>();

        fn snap_edges(value: f32) -> f32 {
            if value < 0.005 { 0.0 }
            else if value > 0.995 { 1.0 }
            else { value }
        }

        // race-gender-age targets
        for (&race, race_value) in race_weights.iter() {
            for (&gender, gender_value) in gender_values.iter() {
                for (&age, age_value) in age_values.iter() {
                    let name = NAME_INTERNER
                        .intern(&format!("{race}-{gender}-{age}"))
                        .leak();
                    let value = race_value * gender_value * age_value;
                    result.insert(name, snap_edges(value));
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
                        result.insert(name, snap_edges(value));
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
                            result.insert(name, snap_edges(value));
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
                            result.insert(name, snap_edges(value));
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
                                result.insert(name, snap_edges(value));
                            }
                        }
                    }
                }
            }
        }

        let morph_targets = morph_targets
            .iter()
            .filter(|(k, _)| !macro_morphs.contains_key(*k));

        // -----------------------------------
        // 2. Resolve composite morph sliders
        // -----------------------------------
        let flattened_composite_morphs = self.composites
            .iter()
            .flat_map(|(_category_name, category)| {
                category.morphs.iter().cloned().map(|m| (m.name, m))
            })
            .collect::<AHashMap<_, _>>();
        
        for (&slider_name, value) in morph_targets.clone() {
            // Find the composite morph definition that matches this slider
            if let Some(morph) = flattened_composite_morphs.get(slider_name) {
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

        let morph_targets = morph_targets.clone()
            .filter(|(k, _)| !flattened_composite_morphs.contains_key(*k));

        let mut missing = morph_targets.clone()
            .filter(|(k, _)| !targets.contains_key(*k) )
            .map(|(&k, _)| k);

        if let Some(t) = missing.next() {
            return Err(MorphError::TargetNotFound(t));
        }

        // --------------------------------------
        // 3. Resolve direct target morph sliders
        // --------------------------------------
        for (&slider_name, value) in morph_targets {
            if targets.contains_key(slider_name) {
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

        Ok(result)
    }

}

pub(crate) fn adjust_helpers_to_morphs(
    morph_values: &MorphTargets,
    mh_morphs: &Arc<RwLock<AHashMap<&'static str, TargetAsset>>>,
    basemesh_vertices: &[Vec3],
) -> Result<Vec<Vec3>, MorphError> {
    let mut helpers = basemesh_vertices.to_vec();
    for (&target_name, &value) in morph_values.iter() {
        let targets = mh_morphs.read().unwrap();
        let target = targets.get(target_name)
            .ok_or_else(|| MorphError::TargetNotFound(target_name))?;
        for TargetDelta { vertex, offset } in target.deltas.iter() {
            helpers[*vertex as usize] += offset * value;
        }
    }
    Ok(helpers)
}
