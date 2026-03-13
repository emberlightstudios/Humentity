use std::sync::{Arc, RwLock};

use crate::{
    loaders::{CompositeTargetsAsset, MacroDataAsset, TargetAsset}, morphs, prelude::*
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

        let mut morph_targets = morph_targets.clone();

        // --- 1️⃣ Normalize race sliders ---
        let mut total_race: f32 = 0.;
        for race in self.macros.morph_map["race"].iter() {
            if let Some(value) = morph_targets.0.get(race) {
                total_race += value;
            }
        }
        if total_race > 0. {
            for race in self.macros.morph_map["race"].iter() {
                let value = morph_targets.0.entry(race).or_default();
                *value /= total_race;
            }
        } else {
            // Default to Caucasian=1 if not specified
            morph_targets.insert("caucasian", 1.0);
            morph_targets.insert("african", 0.0);
            morph_targets.insert("asian", 0.0);
        };

        // --- 2️⃣ Split out macros ---
        // Handle defaults for macros
        if !morph_targets.contains_key("gender") {
            morph_targets.insert("gender", 1.0); // Male
        }
        if !morph_targets.contains_key("age") {
            morph_targets.insert("age", 0.5); // Young
        }
        if !morph_targets.contains_key("weight") {
            morph_targets.insert("weight", 0.5);
        }
        if !morph_targets.contains_key("muscle") {
            morph_targets.insert("muscle", 0.5);
        }
        if !morph_targets.contains_key("proportions") {
            morph_targets.insert("proportions", 0.5);
        }

        // --- 3️⃣ Compute macro morphs ---
        let macro_values = compute_macro_weights(&self.macros, &morph_targets);
        
        for &race in self.macros.morph_map["race"].iter() {
            let race_value = morph_targets.get(&race).copied().unwrap_or(0.0);
            for &gender in self.macros.morph_map["gender"].iter() {
                let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
                for &age in self.macros.morph_map["age"].iter() {
                    let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                    let name = NAME_INTERNER
                        .intern(&format!("{race}-{gender}-{age}"))
                        .leak();
                    let value = race_value * gender_value * age_value;
                    if value != 0.0 {
                        result.insert(name, value);
                    }
                }
            }
        }

        // universal-gender-age-muscle-weight targets
        for &gender in self.macros.morph_map["gender"].iter() {
            let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
            for &age in self.macros.morph_map["age"].iter() {
                let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                for &muscle in self.macros.morph_map["muscle"].iter() {
                    let muscle_value = macro_values.get(&muscle).copied().unwrap_or(0.0);
                    for &weight in self.macros.morph_map["weight"].iter() {
                        let weight_value = macro_values.get(&weight).copied().unwrap_or(0.0);
                        let name = NAME_INTERNER
                            .intern(&format!("universal-{gender}-{age}-{muscle}-{weight}"))
                            .leak();
                        let value = gender_value * age_value * muscle_value * weight_value;
                        if value != 0.0 {
                            result.insert(name, value);
                        }
                    }
                }
            }
        }

        // gender-age-muscle-weight-height targets
        for &gender in self.macros.morph_map["gender"].iter() {
            let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
            for &age in self.macros.morph_map["age"].iter() {
                let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                for &muscle in self.macros.morph_map["muscle"].iter() {
                    let muscle_value = macro_values.get(&muscle).copied().unwrap_or(0.0);
                    for &weight in self.macros.morph_map["weight"].iter() {
                        let weight_value = macro_values.get(&weight).copied().unwrap_or(0.0);
                        for &height in self.macros.morph_map["height"].iter() {
                            let height_value = macro_values.get(&height).copied().unwrap_or(0.0);
                            let name = NAME_INTERNER
                                .intern(&format!("{gender}-{age}-{muscle}-{weight}-{height}"))
                                .leak();
                            let value = gender_value
                                * age_value
                                * muscle_value
                                * weight_value
                                * height_value;
                            if value != 0.0 {
                                result.insert(name, value);
                            }
                        }
                    }
                }
            }
        }

        // gender-age-muscle-weight-proportions targets
        for &gender in self.macros.morph_map["gender"].iter() {
            let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
            for &age in self.macros.morph_map["age"].iter() {
                if age == "baby" {
                    continue;
                }
                let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                for &muscle in self.macros.morph_map["muscle"].iter() {
                    let muscle_value = macro_values.get(&muscle).copied().unwrap_or(0.0);
                    for &weight in self.macros.morph_map["weight"].iter() {
                        let weight_value = macro_values.get(&weight).copied().unwrap_or(0.0);
                        for &proportions in self.macros.morph_map["proportions"].iter() {
                            let proportions_value = macro_values.get(&proportions).copied().unwrap_or(0.0);
                            let name = NAME_INTERNER
                                .intern(&format!("{gender}-{age}-{muscle}-{weight}-{proportions}"))
                                .leak();
                            let value = gender_value
                                * age_value
                                * muscle_value
                                * weight_value
                                * proportions_value;
                            if value != 0.0 {
                                result.insert(name, value);
                            }
                        }
                    }
                }
            }
        }

        // gender-age-muscle-weight-cup-firmness targets
        for &gender in self.macros.morph_map["gender"].iter() {
            if gender == "male" {
                continue;
            }
            let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
            for &age in self.macros.morph_map["age"].iter() {
                if age == "baby" {
                    continue;
                }
                let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                for &muscle in self.macros.morph_map["muscle"].iter() {
                    let muscle_value = macro_values.get(&muscle).copied().unwrap_or(0.0);
                    for &weight in self.macros.morph_map["weight"].iter() {
                        let weight_value = macro_values.get(&weight).copied().unwrap_or(0.0);
                        for &cupsize in self.macros.morph_map["cupsize"].iter() {
                            let cupsize_value = macro_values.get(&cupsize).copied().unwrap_or(0.0);
                            for &firmness in self.macros.morph_map["firmness"].iter() {
                                if firmness == "averagefirmness" && cupsize == "averagecup" {
                                    continue;
                                }
                                let firmness_value = macro_values.get(&firmness).copied().unwrap_or(0.0);
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
                                if value != 0.0 {
                                    result.insert(name, value);
                                }
                            }
                        }
                    }
                }
            }
        }

        let macro_flat_morphs = self.macros.morph_map
            .values()
            .flat_map(|v| v.iter())
            .collect::<Vec<_>>();

        for macro_key in macro_flat_morphs {
            morph_targets.remove(macro_key);
        }
        for macro_key in self.macros.morph_map.keys() {
            morph_targets.remove(macro_key);
        }

        // race-gender-age targets

        // -----------------------------------
        // 2. Resolve composite morph sliders
        // -----------------------------------
        let flattened_composite_morphs = self.composites
            .iter()
            .flat_map(|(_category_name, category)| {
                category.morphs.iter().cloned().map(|m| (m.name, m))
            })
            .collect::<AHashMap<_, _>>();
        
        let mut to_remove = Vec::new();
        for (&slider_name, value) in morph_targets.iter() {
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
                    to_remove.push(slider_name);
                } else if let Some(targets) = &morph.targets {
                    // No opposites: directly apply
                    for target in targets {
                        *result
                            .entry(NAME_INTERNER.intern(target).leak())
                            .or_insert(0.0) += *value;
                    }
                    to_remove.push(slider_name);
                }
            }
        }

        // Remove handled composite morphs to see if any unknown sliders remain that don't map to targets
        let morph_targets = morph_targets.iter()
            .filter(|(k, _)| !to_remove.contains(*k));

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

fn compute_macro_weights(
    macros: &MacroDataAsset,
    slider_values: &MorphTargets,
) -> AHashMap<&'static str, f32> {
    let mut result = AHashMap::default();

    for (&macro_name, value) in slider_values.iter() {
        if let Some(bounds) = macros.macrotargets.get(macro_name) {
            for part in &bounds.parts {
                if *value >= part.lowest && *value <= part.highest {
                    let range = part.highest - part.lowest;
                    let t = (value - part.lowest) / range;

                    let (low, high) = (&part.low[..], &part.high[..]);
                    let low = NAME_INTERNER.intern(low).leak();
                    let high = NAME_INTERNER.intern(high).leak();
                    if !low.is_empty() {
                        *result.entry(low).or_insert(0.0) += 1.0 - t;
                    }
                    if !high.is_empty() {
                        *result.entry(high).or_insert(0.0) += t;
                    }
                    break;
                }
            }
        }
    }

    result
}
