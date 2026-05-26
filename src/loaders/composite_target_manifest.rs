use ahash::AHashMap;
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader}, ecs::intern::Internable, prelude::*
};
use serde::Deserialize;
use crate::prelude::NAME_INTERNER;

#[derive(Asset, TypePath, Clone, Debug, Deref, Default)]
pub struct CompositeTargetsAsset(pub AHashMap<&'static str, CategoryMorphsAsset>);

#[derive(Clone, Debug)]
pub struct CategoryMorphsAsset {
    pub morphs: Vec<CompositeTarget>,
}

#[derive(Clone, Debug)]
pub struct CompositeTarget {
    pub has_left_and_right: bool,
    pub name: &'static str,
    pub opposites: Option<OppositesAsset>,
    pub targets: Option<Vec<&'static str>>,
}

#[derive(Clone, Debug)]
pub struct OppositesAsset {
    pub negative_left: &'static str,
    pub negative_right: &'static str,
    pub negative_unsided: &'static str,
    pub positive_left: &'static str,
    pub positive_right: &'static str,
    pub positive_unsided: &'static str,
}

// Temp types with String for deserialization
#[derive(Clone, Debug, Deserialize, Deref)]
pub struct CompositeMorphsAssetString(pub AHashMap<String, CategoryMorphsAssetString>);

#[derive(Clone, Debug, Deserialize)]
pub struct CategoryMorphsAssetString {
    #[serde(rename = "categories")]
    pub morphs: Vec<CompositeMorphAssetString>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CompositeMorphAssetString {
    pub has_left_and_right: bool,
    pub name: String,
    pub opposites: Option<OppositesAssetString>,
    pub targets: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct OppositesAssetString {
    pub negative_left: String,
    pub negative_right: String,
    pub negative_unsided: String,
    pub positive_left: String,
    pub positive_right: String,
    pub positive_unsided: String,
}

#[derive(Default, TypePath)]
pub struct TargetManifestAssetLoader;

impl AssetLoader for TargetManifestAssetLoader {
    type Asset = CompositeTargetsAsset;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;

        let values = serde_json::from_slice::<CompositeMorphsAssetString>(&bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

        let interned = values
            .iter()
            .map(|(category, morphs)| {
                let interned_category = NAME_INTERNER.intern(category).leak();
                let interned_morphs = morphs
                    .morphs
                    .iter()
                    .map(|asset| {
                        let interned_name = NAME_INTERNER.intern(&asset.name).leak();
                        let interned_opposites = asset.opposites.as_ref().map(|op| OppositesAsset {
                            negative_left: NAME_INTERNER.intern(&op.negative_left).leak(),
                            negative_right: NAME_INTERNER.intern(&op.negative_right).leak(),
                            negative_unsided: NAME_INTERNER.intern(&op.negative_unsided).leak(),
                            positive_left: NAME_INTERNER.intern(&op.positive_left).leak(),
                            positive_right: NAME_INTERNER.intern(&op.positive_right).leak(),
                            positive_unsided: NAME_INTERNER.intern(&op.positive_unsided).leak(),
                        });
                        CompositeTarget {
                            has_left_and_right: asset.has_left_and_right,
                            name: interned_name,
                            opposites: interned_opposites,
                            targets: asset.targets.as_ref().map(|v| {
                                v.iter()
                                    .map(|t| NAME_INTERNER.intern(t).leak())
                                    .collect()
                            }),
                        }
                    })
                    .collect::<Vec<_>>();
                (interned_category, CategoryMorphsAsset { morphs: interned_morphs })
            })
            .collect::<AHashMap<_, _>>();

        Ok(CompositeTargetsAsset(interned))
    }

    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}
