use ahash::AHashMap;
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    ecs::intern::Internable,
    prelude::*,
};
use serde::Deserialize;

use crate::NAME_INTERNER;

#[derive(Clone, Debug, Deserialize)]
pub struct BoneTransformSpec {
    pub cube_name: Option<String>,
    pub strategy: String,
    pub vertex_indices: Option<Vec<u16>>,
    pub vertex_index: Option<u16>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BoneJsonConfig {
    pub parent: String,
    pub head: BoneTransformSpec,
    pub tail: BoneTransformSpec,
    #[serde(default)]
    pub roll: f32,
}

#[derive(Asset, TypePath, Clone, Debug)]
pub struct RigConfigAsset {
    pub bones: AHashMap<&'static str, BoneJsonConfig>,
    pub rig_name: String,
}

#[derive(Deserialize)]
struct MixamoRigConfigAsset {
    bones: AHashMap<String, BoneJsonConfig>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RigConfigFile {
    Direct(AHashMap<String, BoneJsonConfig>),
    Mixamo(MixamoRigConfigAsset),
}

#[derive(Default, TypePath)]
pub struct RigConfigAssetLoader;

impl AssetLoader for RigConfigAssetLoader {
    type Asset = RigConfigAsset;
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
        let parsed = serde_json::from_slice::<RigConfigFile>(&bytes)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

        let bones = match parsed {
            RigConfigFile::Direct(bones) => bones,
            RigConfigFile::Mixamo(mixamo) => mixamo.bones,
        };

        let path = load_context.path().to_string();
        let rig_name = if path.contains("mixamo") {
            "mixamo".to_string()
        } else if path.contains("game_engine") {
            "game_engine".to_string()
        } else if path.contains("default") {
            "default".to_string()
        } else {
            "unknown".to_string()
        };

        let bones = bones
            .into_iter()
            .map(|(name, config)| (NAME_INTERNER.intern(&name).leak(), config))
            .collect();

        Ok(RigConfigAsset { bones, rig_name })
    }

    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}
