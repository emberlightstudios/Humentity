use crate::prelude::*;
use ahash::AHashMap;
use bevy::{ecs::intern::Internable, prelude::*};
use serde::{Deserialize, Serialize, ser::SerializeStruct};
use serde::{Deserializer, Serializer};

/// One blendable body/face shape in a [`CharacterTemplate`].
///
/// Weights are free: the game decides the convention. The usual split is
/// macro shapes (baby, bodybuilder) with weights summing to 1, plus feature
/// shapes (long nose, big ears) with independent 0..1 weights added on top,
/// e.g. 50% baby + 50% bodybuilder + 100% nose + 100% ears. humentity never
/// normalizes; any auto-normalize for macro groups lives in game code, not here.
///
/// Each shape's morphed-body delta (`helper_deltas`) is baked once on the
/// template and shared by every character. Per-character blends compute
/// `base + Σ weight × delta` on demand, so no 150KB helper vec is duplicated
/// per character. Deltas are sparse: a nose shape only touches nose verts.
/// `None` means not yet baked (morph targets not loaded); the bake fills it,
/// then characters blend from it.
///
/// Build-once part: never push new shapes into a template or edit `morphs`
/// after the template is added to the asset store. Make a new template
/// instead; runtime edits serve stale cached meshes with no error.
#[derive(Clone, Debug)]
pub struct CharacterMorphShape {
    pub name: &'static str,
    pub morphs: MorphTargets,
    pub helper_deltas: Option<Vec<TargetDelta>>,
}

impl CharacterMorphShape {
    pub fn new(name: impl AsRef<str>, morphs: MorphTargets) -> Self {
        Self {
            name: NAME_INTERNER.intern(name.as_ref()).leak(),
            morphs,
            helper_deltas: None,
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
            helper_deltas: None,
        })
    }
}

/// A collection of base shapes and animation properties.  The shapes will be baked into a
/// new Mesh as morph targets. Loadable from `.toml` files via [`CharacterTemplateAssetLoader`].
///
/// Build-once: create the template fully, add it to the asset store, then never
/// mutate it. Mesh builds are cached by template handle and delta bakes only
/// fill empty slots, so pushing/clearing `shapes` (or editing a shape's
/// `morphs`) after meshes exist serves stale cached meshes with no error.
/// A variant is a new template, not an edit of an existing one.
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

    /// Blend per-character helpers from baked template deltas.
    ///
    /// `base + Σ weight × delta`: macro shapes blend with weights summing to
    /// 1, feature shapes add on top with independent weights. Missing weights
    /// read as 0. Shapes with `None` deltas are skipped (not yet baked).
    pub fn blend_helpers(
        &self,
        morph_values: &MorphTargets,
        basemesh_vertices: &[Vec3],
    ) -> Vec<Vec3> {
        let mut helpers = basemesh_vertices.to_vec();
        for shape in self.shapes.iter() {
            let shape_weight = morph_values.get(shape.name).copied().unwrap_or(0.0);
            if shape_weight == 0.0 {
                continue;
            }
            let Some(deltas) = shape.helper_deltas.as_ref() else {
                continue;
            };
            for delta in deltas.iter() {
                helpers[delta.vertex as usize] += delta.offset * shape_weight;
            }
        }
        helpers
    }
}

/// Bake one shape's sparse morphed-body delta from resolved morph targets.
///
/// Runs once per template shape after [`resolve_template_morphs`] (morphs are
/// direct targets by then). The delta is `morphed − base`: zero everywhere the
/// shape doesn't touch, so a nose shape only carries nose verts.
fn bake_shape_deltas(
    shape_morphs: &MorphTargets,
    targets: &AHashMap<&'static str, TargetAsset>,
    basemesh_vert_count: usize,
) -> Vec<TargetDelta> {
    use crate::loaders::TargetDelta as ShapeDelta;
    let mut accumulated: AHashMap<u16, Vec3> = AHashMap::default();
    for (&target_name, &target_weight) in shape_morphs.iter() {
        let Some(target) = targets.get(target_name) else {
            panic!(
                "template shape references morph target '{target_name}' with no loaded target: gate template creation on HumentityAssetsReady"
            );
        };
        for delta in target.deltas.iter() {
            debug_assert!(
                (delta.vertex as usize) < basemesh_vert_count,
                "morph target '{target_name}' vertex {} out of range for {basemesh_vert_count} basemesh verts",
                delta.vertex,
            );
            *accumulated.entry(delta.vertex).or_insert(Vec3::ZERO) += delta.offset * target_weight;
        }
    }
    accumulated
        .into_iter()
        .map(|(vert_index, vert_offset)| ShapeDelta {
            vertex: vert_index,
            offset: vert_offset,
        })
        .collect()
}

/// Resolves macro morph sliders (e.g. "muscle") into direct morph targets
/// whenever a new CharacterTemplate asset is added. Templates must only be
/// created after `HumentityAssetsReady`: macro/composite data has to be loaded
/// before resolution can succeed, and an unresolvable shape is a hard error,
/// not a silent skip — downstream helpers and mesh builds would retry forever
/// on macro names that are not real targets.
pub(crate) fn resolve_template_morphs(
    mut events: MessageReader<AssetEvent<CharacterTemplate>>,
    morphs: Res<MakeHumanMorphs>,
    mut templates: ResMut<Assets<CharacterTemplate>>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id }) = event else {
            continue;
        };
        // Untracked: the template is still under construction, and the
        // `Modified` event this would otherwise emit must not trip the
        // build-once freeze below.
        let Some(template) = templates.get_mut_untracked(*id) else {
            continue;
        };
        for shape in template.shapes.iter_mut() {
            shape.morphs = morphs
                .compute_target_weights(&shape.morphs)
                .expect("CharacterTemplate created before morph data was ready: gate template creation on HumentityAssetsReady");
        }
        // Delta baking lives in `bake_template_deltas` (guarded by
        // `has_targets_for_shapes` + basemesh readiness). Baking here would
        // panic on templates added before the targets folder finishes loading.
    }
}

/// Panics if a `CharacterTemplate` is modified after upload.
///
/// Templates are build-once: mesh builds are cached by template handle and
/// delta bakes only fill empty slots, so a runtime edit would silently serve
/// stale meshes. Internal init writes use `get_mut_untracked` precisely so
/// they never emit `Modified` — only an external edit trips this.
pub(crate) fn enforce_template_frozen(mut events: MessageReader<AssetEvent<CharacterTemplate>>) {
    for event in events.read() {
        if matches!(event, AssetEvent::Modified { .. }) {
            panic!(
                "CharacterTemplate modified after upload: templates are build-once, create a new template instead of editing one in the asset store"
            );
        }
    }
}

/// Retries template delta bakes for templates whose basemesh wasn't loaded at
/// resolve time. Runs until every shape on every template carries deltas.
pub(crate) fn bake_template_deltas(
    morphs: Res<MakeHumanMorphs>,
    basemesh: Res<crate::basemesh::BaseMesh>,
    asset_server: Res<AssetServer>,
    mut templates: ResMut<Assets<CharacterTemplate>>,
) {
    if basemesh.vertices.is_empty() || !morphs.is_ready(&asset_server) {
        return;
    }
    let vert_count = basemesh.vertices.len();
    let Ok(targets) = morphs.targets.read() else {
        return;
    };
    // Two-phase: `iter_mut` would emit `Modified` per template and trip the
    // build-once freeze, and there is no untracked iterator — so collect ids
    // under a shared borrow, then write via `get_mut_untracked`.
    let pending: Vec<_> = templates.ids().collect();
    for id in pending {
        let needs_bake = templates.get(id).is_some_and(|template| {
            template.shapes.iter().any(|shape| {
                shape.helper_deltas.is_none()
                    && morphs.has_targets_for_shapes(std::slice::from_ref(shape))
            })
        });
        if !needs_bake {
            continue;
        }
        let Some(template) = templates.get_mut_untracked(id) else {
            continue;
        };
        for shape in template.shapes.iter_mut() {
            if shape.helper_deltas.is_none()
                && morphs.has_targets_for_shapes(std::slice::from_ref(shape))
            {
                shape.helper_deltas = Some(bake_shape_deltas(&shape.morphs, &targets, vert_count));
            }
        }
    }
}
