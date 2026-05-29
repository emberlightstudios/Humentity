use std::sync::{Arc, RwLock};

use crate::{
    loaders::{CompositeTargetsAsset, MacroDataAsset, TargetAsset}, prelude::*
};
use ahash::AHashMap;
use bevy::{asset::{LoadState, LoadedFolder}, ecs::intern::Internable, prelude::*};
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
    #[error("RwLock was poisoned")]
    LockPoisoned,
}

/// A type for storing generic morph target weights
#[derive(Component, Deref, DerefMut, Clone, Default, Debug)]
pub struct MorphTargets(AHashMap<&'static str, f32>);

impl Serialize for MorphTargets {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
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
    pub categories: RwLock<AHashMap<&'static str, Vec<&'static str>>>,
    macros: Arc<RwLock<Option<MacroDataAsset>>>,
    composites: Arc<RwLock<Option<CompositeTargetsAsset>>>,
    #[allow(dead_code)]
    composite_handle: Handle<CompositeTargetsAsset>,
    #[allow(dead_code)]
    macro_handle: Handle<MacroDataAsset>,
    targets_handle: Handle<LoadedFolder>,
}

impl MakeHumanMorphs {
    pub fn new(
        asset_server: &AssetServer,
        composite_path: &'static str,
        macro_path: &'static str,
        targets_folder: &'static str,
    ) -> Self {
        Self {
            targets: default(),
            macros: default(),
            composites: default(),
            categories: default(),
            composite_handle: asset_server.load::<CompositeTargetsAsset>(composite_path),
            macro_handle: asset_server.load::<MacroDataAsset>(macro_path),
            targets_handle: asset_server.load_folder(targets_folder),
        }
    }

    pub fn is_ready(&self, asset_server: &AssetServer) -> bool {
        if !self.macros.read().unwrap().is_some() || !self.composites.read().unwrap().is_some() {
            return false;
        }
        matches!(asset_server.get_load_state(&self.targets_handle), Some(LoadState::Loaded))
    }

    pub fn get_min_values(&self) -> AHashMap<&'static str, f32> {
        let categories = self.categories.read().unwrap();
        let mut result = AHashMap::default();
        for (&category, morph_names) in categories.iter() {
            for &morph in morph_names.iter() {
                if category == "macro"
                    || (category == "head" && morph.split('-').count() == 2)
                    || category == "expressions"
                {
                    result.insert(morph, 0.);
                } else {
                    result.insert(morph, -1.);
                }
            }
        }
        result
    }

    pub fn get_morph_names(&self) -> AHashMap<&'static str, Vec<&'static str>> {
        self.categories.read().unwrap().clone()
    }

    pub fn compute_target_weights(&self, morph_targets: &MorphTargets) -> Result<MorphTargets, MorphError> {
        let targets = self.targets.read().map_err(|_| MorphError::LockPoisoned)?;
        if targets
            .keys()
            .all(|&t| morph_targets.contains_key(t))
        {
            return Ok(morph_targets.clone());
        }

        let macros = self.macros.read().map_err(|_| MorphError::LockPoisoned)?;
        let Some(macros) = macros.as_ref() else {
            return Err(MorphError::MacrosNotLoaded);
        };

        let composites = self.composites.read().map_err(|_| MorphError::LockPoisoned)?;
        let Some(composites) = composites.as_ref() else {
            return Err(MorphError::CompositeNotLoaded);
        };

        let mut result = MorphTargets::default();

        if macros.macrotargets.is_empty() {
            return Err(MorphError::MacrosNotLoaded);
        }

        if composites.is_empty() {
            return Err(MorphError::CompositeNotLoaded);
        }

        if morph_targets.keys().all(|&t| targets.contains_key(t)) {
            return Ok(morph_targets.clone());
        }

        let mut morph_targets = morph_targets.clone();

        // --- 1️⃣ Normalize race sliders ---
        let mut total_race: f32 = 0.;
        for race in macros.morph_map["race"].iter() {
            if let Some(value) = morph_targets.0.get(race) {
                total_race += value;
            }
        }
        if total_race > 0. {
            for race in macros.morph_map["race"].iter() {
                let value = morph_targets.0.entry(race).or_default();
                *value /= total_race;
            }
        } else {
            morph_targets.insert("caucasian", 1.0);
            morph_targets.insert("african", 0.0);
            morph_targets.insert("asian", 0.0);
        };

        // --- 2️⃣ Split out macros ---
        if !morph_targets.contains_key("gender") {
            morph_targets.insert("gender", 1.0);
        }
        if !morph_targets.contains_key("age") {
            morph_targets.insert("age", 0.5);
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
        if !morph_targets.contains_key("cupsize") {
            morph_targets.insert("cupsize", 0.0);
        }
        if !morph_targets.contains_key("firmness") {
            morph_targets.insert("firmness", 0.0);
        }

        // --- 3️⃣ Compute macro morphs ---
        let macro_values = compute_macro_weights(macros, &morph_targets);

        for &race in macros.morph_map["race"].iter() {
            let race_value = morph_targets.get(&race).copied().unwrap_or(0.0);
            for &gender in macros.morph_map["gender"].iter() {
                let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
                for &age in macros.morph_map["age"].iter() {
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

        for &gender in macros.morph_map["gender"].iter() {
            let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
            for &age in macros.morph_map["age"].iter() {
                let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                for &muscle in macros.morph_map["muscle"].iter() {
                    let muscle_value = macro_values.get(&muscle).copied().unwrap_or(0.0);
                    for &weight in macros.morph_map["weight"].iter() {
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

        for &gender in macros.morph_map["gender"].iter() {
            let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
            for &age in macros.morph_map["age"].iter() {
                let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                for &muscle in macros.morph_map["muscle"].iter() {
                    let muscle_value = macro_values.get(&muscle).copied().unwrap_or(0.0);
                    for &weight in macros.morph_map["weight"].iter() {
                        let weight_value = macro_values.get(&weight).copied().unwrap_or(0.0);
                        for &height in macros.morph_map["height"].iter() {
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

        for &gender in macros.morph_map["gender"].iter() {
            let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
            for &age in macros.morph_map["age"].iter() {
                if age == "baby" {
                    continue;
                }
                let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                for &muscle in macros.morph_map["muscle"].iter() {
                    let muscle_value = macro_values.get(&muscle).copied().unwrap_or(0.0);
                    for &weight in macros.morph_map["weight"].iter() {
                        let weight_value = macro_values.get(&weight).copied().unwrap_or(0.0);
                        for &proportions in macros.morph_map["proportions"].iter() {
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

        for &gender in macros.morph_map["gender"].iter() {
            if gender == "male" {
                continue;
            }
            let gender_value = macro_values.get(&gender).copied().unwrap_or(0.0);
            for &age in macros.morph_map["age"].iter() {
                if age == "baby" {
                    continue;
                }
                let age_value = macro_values.get(&age).copied().unwrap_or(0.0);
                for &muscle in macros.morph_map["muscle"].iter() {
                    let muscle_value = macro_values.get(&muscle).copied().unwrap_or(0.0);
                    for &weight in macros.morph_map["weight"].iter() {
                        let weight_value = macro_values.get(&weight).copied().unwrap_or(0.0);
                        for &cupsize in macros.morph_map["cupsize"].iter() {
                            let cupsize_value = macro_values.get(&cupsize).copied().unwrap_or(0.0);
                            for &firmness in macros.morph_map["firmness"].iter() {
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

        let macro_flat_morphs = macros.morph_map
            .values()
            .flat_map(|v| v.iter())
            .collect::<Vec<_>>();

        for macro_key in macro_flat_morphs {
            morph_targets.remove(macro_key);
        }
        for macro_key in macros.morph_map.keys() {
            morph_targets.remove(macro_key);
        }

        // -----------------------------------
        // 2. Resolve composite morph sliders
        // -----------------------------------
        let flattened_composite_morphs = composites.values().flat_map(|category| {
                category.morphs.iter().cloned().map(|m| (m.name, m))
            })
            .collect::<AHashMap<_, _>>();

        let mut to_remove = Vec::new();
        for (&slider_name, value) in morph_targets.iter() {
            if let Some(morph) = flattened_composite_morphs.get(slider_name) {
                if let Some(opps) = &morph.opposites {
                    if morph.has_left_and_right {
                        if *value > 0.0 {
                            *result
                                .entry(NAME_INTERNER.intern(opps.positive_right).leak())
                                .or_insert(0.0) += value.abs();
                            *result
                                .entry(NAME_INTERNER.intern(opps.positive_left).leak())
                                .or_insert(0.0) += value.abs();
                        } else {
                            *result
                                .entry(NAME_INTERNER.intern(opps.negative_right).leak())
                                .or_insert(0.0) += value.abs();
                            *result
                                .entry(NAME_INTERNER.intern(opps.negative_left).leak())
                                .or_insert(0.0) += value.abs();
                        }
                    } else if *value > 0.0 {
                        *result
                            .entry(NAME_INTERNER.intern(opps.positive_unsided).leak())
                            .or_insert(0.0) += value.abs();
                    } else {
                        *result
                            .entry(NAME_INTERNER.intern(opps.negative_unsided).leak())
                            .or_insert(0.0) += value.abs();
                    }
                    to_remove.push(slider_name);
                } else if let Some(targets) = &morph.targets {
                    for target in targets {
                        *result
                            .entry(NAME_INTERNER.intern(target).leak())
                            .or_insert(0.0) += *value;
                    }
                    to_remove.push(slider_name);
                }
            }
        }

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

/// Fired once as a trigger when all morph assets have finished loading.
#[derive(Event)]
pub struct MorphsReady;

pub(crate) fn check_morphs_ready(
    morphs: Res<MakeHumanMorphs>,
    asset_server: Res<AssetServer>,
    mut ready: Local<bool>,
    mut commands: Commands,
) {
    if *ready {
        return;
    }
    if morphs.is_ready(&asset_server) {
        *ready = true;
        commands.trigger(MorphsReady);
    }
}

pub(crate) fn populate_morph_resource(
    morphs: Res<MakeHumanMorphs>,
    macro_assets: Res<Assets<MacroDataAsset>>,
    composite_assets: Res<Assets<CompositeTargetsAsset>>,
) {
    if morphs.macros.read().unwrap().is_none()
        && let Some((_, asset)) = macro_assets.iter().next() {
            let data = asset.clone();
            let mut cats = morphs.categories.write().unwrap();
            let mut macro_sliders = vec!["caucasian", "asian", "african"];
            macro_sliders.extend(data.macrotargets.keys());
            cats.insert("macro", macro_sliders);
            *morphs.macros.write().unwrap() = Some(data);
        }

    if morphs.composites.read().unwrap().is_none()
        && let Some((_, asset)) = composite_assets.iter().next() {
            let data = asset.clone();
            let mut cats = morphs.categories.write().unwrap();
            for (&category, category_morphs) in data.iter() {
                if category_morphs.morphs.is_empty() {
                    continue;
                }
                let morph_names: Vec<&'static str> = category_morphs.morphs.iter().map(|m| m.name).collect();
                cats.insert(category, morph_names);
            }
            *morphs.composites.write().unwrap() = Some(data);
        }
}

pub(crate) fn sync_loaded_morph_targets(
    morphs: Res<MakeHumanMorphs>,
    target_assets: Res<Assets<TargetAsset>>,
    asset_server: Res<AssetServer>,
    mut target_events: MessageReader<AssetEvent<TargetAsset>>,
) {
    for ev in target_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev {
            let asset = target_assets.get(*id).unwrap();
            let name = asset.name;
            let mut targets = morphs.targets.write().unwrap();
            targets.insert(name, asset.clone());

            if let Some(path) = asset_server.get_path(*id) {
                let folder = path.path().parent().and_then(|p| p.file_stem()).and_then(|s| s.to_str());
                let category = match folder {
                    Some("expressions") => Some("expressions"),
                    Some("asym") => Some("asymmetry"),
                    _ => None,
                };
                if let Some(cat) = category {
                    morphs.categories.write().unwrap()
                        .entry(cat)
                        .or_insert(vec![])
                        .push(name);
                }
            }
        }
    }
}

pub fn adjust_helpers_to_morphs(
    morph_values: &MorphTargets,
    mh_morphs: &Arc<RwLock<AHashMap<&'static str, TargetAsset>>>,
    basemesh_vertices: &[Vec3],
) -> Result<Vec<Vec3>, MorphError> {
    let mut helpers = basemesh_vertices.to_vec();
    for (&target_name, &value) in morph_values.iter() {
        let targets = mh_morphs.read().map_err(|_| MorphError::LockPoisoned)?;
        let target = targets.get(target_name)
            .ok_or(MorphError::TargetNotFound(target_name))?;
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

                    let (low, high) = (part.low, part.high);
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
