use crate::{morphs::adjust_helpers_to_morphs, prelude::*};
use bevy::{ecs::intern::Internable, prelude::*};
use serde::{ser::SerializeStruct, Deserialize, Serialize};
use serde::{Deserializer, Serializer};

/// In order to dynamically reshape humans at runtime, we can define a CharacterArchetype which is a mesh
/// cached from a given set of MorphTargets.  Archetypes are added as new distinct shapekeys to the base
/// mesh, and the rest of the makehuman shapekeys are removed.  Use this for distinct faces or body types.
/// You can also blend between them, since they are just shapekeys.
#[derive(Clone, Debug)]
pub struct CharacterMorphShape {
    pub name: &'static str,
    pub morphs: MorphTargets,
}

impl CharacterMorphShape {
    pub fn new(name: impl AsRef<str>, morphs: MorphTargets) -> Self {
        Self {
            name: NAME_INTERNER.intern(name.as_ref()).leak(),
            morphs,
        }
    }
}

impl Serialize for CharacterMorphShape {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as a map with "name" and "morphs"
        let mut state: <S as Serializer>::SerializeStruct =
            serializer.serialize_struct("CharacterShapeArchetype", 2)?;
        state.serialize_field("name", self.name)?;
        state.serialize_field("morphs", &self.morphs)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for CharacterMorphShape {
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
        Ok(CharacterMorphShape {
            name,
            morphs: tmp.morphs,
        })
    }
}

/// A collection of base shapes and animation properties.  The shapes will be baked into a
/// new Mesh as morph targets. Loadable from `.toml` files via [`CharacterTemplateAssetLoader`].
#[derive(Asset, TypePath, Default, Serialize, Deserialize, Clone, Debug)]
pub struct CharacterTemplate {
    /// Set by the asset loader from the file stem. Empty for code-constructed templates.
    #[serde(skip)]
    pub name: &'static str,
    pub shapes: Vec<CharacterMorphShape>,
}

impl CharacterTemplate {
    pub fn new(shapes: impl IntoIterator<Item = CharacterMorphShape>) -> Self {
        Self {
            name: "",
            shapes: shapes.into_iter().collect(),
        }
    }

    pub(crate) fn get_helpers(
        &self,
        morph_values: &MorphTargets,
        basemesh_vertices: &[Vec3],
        mh_morphs: &MakeHumanMorphs,
    ) -> Result<Vec<Vec3>, crate::morphs::MorphError> {
        let mut mh_morph_values = MorphTargets::default();
        for shape in self.shapes.iter() {
            let Some(weight) = morph_values.get(shape.name) else {
                continue;
            };
            for (&k, v) in shape.morphs.iter() {
                let entry = mh_morph_values.entry(k).or_insert(0.);
                *entry += *v * weight;
            }
        }
        adjust_helpers_to_morphs(&mh_morph_values, &mh_morphs.targets, basemesh_vertices)
    }
}

/// Resolves macro morph sliders (e.g. "muscle") into direct morph targets
/// whenever a new CharacterTemplate asset is added.
pub(crate) fn resolve_template_morphs(
    mut events: MessageReader<AssetEvent<CharacterTemplate>>,
    morphs: Res<MakeHumanMorphs>,
    mut templates: ResMut<Assets<CharacterTemplate>>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id }) = event else {
            continue;
        };
        let Some(mut template) = templates.get_mut(*id) else {
            continue;
        };
        for shape in template.shapes.iter_mut() {
            if let Ok(resolved) = morphs.compute_target_weights(&shape.morphs) {
                shape.morphs = resolved;
            }
        }
    }
}

/// Overrides the template shapes for a part
#[derive(Component, Deref, Clone, Eq, PartialEq, Hash, Debug)]
pub struct TemplateOverride(pub Handle<CharacterTemplate>);
