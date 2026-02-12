use std::sync::Arc;

use crate::{
    animation::get_skeleton_transforms,
    morphs::adjust_helpers_to_morphs,
    prelude::*,
    rigs::{get_bone_order, SkeletonCache, SkeletonCaches},
};
use ahash::AHashMap;
use bevy::{ecs::intern::Internable, prelude::*};
use serde::{Deserialize, Serialize, ser::SerializeStruct};
use serde::{Deserializer, Serializer};

/// In order to dynamically reshape humans at runtime, we can define a CharacterArchetype which is a mesh
/// cached from a given set of MorphTargets.  Archetypes are added as new distinct shapekeys to the base
/// mesh, and the rest of the makehuman shapekeys are removed.  Use this for distinct faces or body types.
/// You can also blend between them, since they are just shapekeys.
#[derive(Clone, Debug)]
pub struct CharacterShapeArchetype {
    pub name: &'static str,
    pub morphs: MorphTargets,
}

impl CharacterShapeArchetype {
    pub fn new(name: impl AsRef<str>, morphs: MorphTargets) -> Self {
        Self {
            name: NAME_INTERNER.intern(name.as_ref()).leak(),
            morphs,
        }
    }
}

impl Serialize for CharacterShapeArchetype {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as a map with "name" and "morphs"
        let mut state: <S as Serializer>::SerializeStruct = serializer.serialize_struct("CharacterShapeArchetype", 2)?;
        state.serialize_field("name", self.name)?;
        state.serialize_field("morphs", &self.morphs)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for CharacterShapeArchetype {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Tmp {
            name: String,
            morphs: MorphTargets,
        }

        let tmp = Tmp::deserialize(deserializer)?;

        // Convert name to &'static str via leak (safe if fixed names)
        let name: &'static str = NAME_INTERNER.intern(&tmp.name).leak();
        Ok(CharacterShapeArchetype { name, morphs: tmp.morphs })
    }
}

/// Encapsulates animation properties associated with an archetype/prefab.
#[derive(Default, Clone)]
pub struct CharacterAnimationArchetype {
    pub animation_glbs: Vec<&'static str>,
    pub rig_type: RigType,
}


impl Serialize for CharacterAnimationArchetype {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as a map with "name" and "morphs"
        let mut state: <S as Serializer>::SerializeStruct = serializer
            .serialize_struct("CharacterAnimationArchetype", 2)?;
        state.serialize_field("animation_glbs", &self.animation_glbs)?;
        state.serialize_field("rig_type", &self.rig_type)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for CharacterAnimationArchetype {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Tmp {
            animation_glbs: Vec<String>,
            rig_type: RigType,
        }

        let tmp = Tmp::deserialize(deserializer)?;

        // Convert name to &'static str via leak (safe if fixed names)
        let glbs: Vec<&'static str> = tmp.animation_glbs
            .iter()
            .map(|s| NAME_INTERNER.intern(s).leak())
            .collect();
        Ok(CharacterAnimationArchetype { animation_glbs: glbs, rig_type: tmp.rig_type })
    }
}

impl CharacterAnimationArchetype {
    pub fn new(rig_type: RigType, animation_glbs: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self {
            animation_glbs: animation_glbs
                .into_iter()
                .map(|s| NAME_INTERNER.intern(s.as_ref()).leak())
                .collect::<Vec<_>>(),
            rig_type,
        }
    }
}

/// A collection of base shapes and animation properties.  The shapes will be baked into a
/// new Mesh as morph targets.
#[derive(Default, Serialize, Deserialize, Clone)]
pub struct CharacterArchetypePrefab {
    pub shapes: Vec<CharacterShapeArchetype>,
    pub rig: CharacterAnimationArchetype,
}

impl CharacterArchetypePrefab {
    pub fn new(
        shapes: impl IntoIterator<Item = CharacterShapeArchetype>,
        rig: CharacterAnimationArchetype,
    ) -> Self {
        Self {
            shapes: shapes.into_iter().collect(),
            rig,
        }
    }

    pub(crate) fn get_helpers(
        &self,
        morph_values: &MorphTargets,
        basemesh: &BaseMesh,
        morph_targets: &MakeHumanMorphs,
    ) -> Vec<Vec3> {
        let mut mh_morphs = MorphTargets::default();
        for shape in self.shapes.iter() {
            let name: &str = NAME_INTERNER.intern(shape.name).leak();
            let Some(weight) = morph_values.get(name) else {
                continue;
            };
            for (&k, v) in shape.morphs.iter() {
                let entry = mh_morphs.entry(k).or_insert(0.);
                *entry += *v * weight;
            }
        }
        adjust_helpers_to_morphs(&mh_morphs, &morph_targets.targets, basemesh)
    }
}

/// Overrides the prefab shapes for a part
#[derive(Component, Deref, Serialize, Clone, Eq, PartialEq, Hash)]
pub struct PrefabOverride(pub &'static str);

impl<'de> Deserialize<'de> for PrefabOverride {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where D: Deserializer<'de> {
        let s = String::deserialize(deserializer)?;
        Ok(Self(NAME_INTERNER.intern(&s).leak()))
    }
}

#[derive(Resource, Deref, DerefMut, Default)]
pub struct CharacterArchetypePrefabs(AHashMap<&'static str, CharacterArchetypePrefab>);

impl CharacterArchetypePrefabs {
    pub fn new(
        prefabs: impl IntoIterator<Item = (&'static str, CharacterArchetypePrefab)>,
    ) -> Self {
        Self(
            prefabs
                .into_iter()
                .collect::<AHashMap<&'static str, CharacterArchetypePrefab>>(),
        )
    }

    /// This is just for testing, no shapes are added
    pub fn basemesh() -> Self {
        let mut prefabs = AHashMap::default();
        prefabs.insert("", CharacterArchetypePrefab::default());
        Self(prefabs)
    }
}

pub(crate) fn create_character_prefab_rig_scenes(world: &mut World) {
    // Only run if prefab rig scenes are None
    let prefabs = world
        .get_resource::<CharacterArchetypePrefabs>()
        .expect("No human prefabs resource found");

    let prefab_data = prefabs
        .iter()
        .map(|(&n, p)| (n, p.rig.rig_type))
        .collect::<Vec<_>>();

    for (name, rig_type) in prefab_data {
        let (bone_rotations, bone_translations) = get_skeleton_transforms(world, rig_type)
            .expect("Failed to get skeleton rotations from glb file");

        let bone_order = get_bone_order(world, rig_type);
        let base_mesh = world.get_resource::<BaseMesh>().unwrap();
        let helpers = &base_mesh.0.clone();
        let scene = crate::rigs::build_human_rig_scene(
            helpers,
            rig_type,
            &bone_rotations,
            &bone_order,
            world,
        );

        let mut prefabs = world.resource_mut::<CharacterArchetypePrefabs>();
        let prefab = prefabs.get_mut(&name).unwrap();
        let rig_type = prefab.rig.rig_type;

        let mut skeleton_caches = world.resource_mut::<SkeletonCaches>();
        if !skeleton_caches.contains_key(&rig_type) {
            skeleton_caches.insert(
                rig_type,
                Arc::new(SkeletonCache {
                    bone_order: bone_order.clone(),
                    bone_model_space_rots: bone_rotations,
                    bone_local_translations: bone_translations,
                    scene,
                }),
            );
        }
    }
    let mut state = world.resource_mut::<NextState<HumentityLoadState>>();
    state.set(HumentityLoadState::AnimationProcessing);
}
