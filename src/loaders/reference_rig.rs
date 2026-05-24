use ahash::{AHashMap, AHashSet};
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    ecs::intern::Internable,
    prelude::*,
};

use crate::{NAME_INTERNER, rigs::RigType};

#[derive(Asset, TypePath, Clone)]
pub struct ReferenceRigAsset {
    pub bone_names: Vec<&'static str>,
    pub bone_parents: AHashMap<&'static str, String>,
    pub local_bindpose: AHashMap<&'static str, Transform>,
    pub model_space_bindpose: AHashMap<&'static str, Transform>,
    pub bone_name_to_index: AHashMap<&'static str, usize>,
    pub rig: RigType,
}

#[derive(Default, TypePath)]
pub struct ReferenceRigAssetLoader;

impl AssetLoader for ReferenceRigAssetLoader {
    type Asset = ReferenceRigAsset;
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
        
        let (document, _buffers, _) = gltf::import_slice(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        if document.skins().len() > 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "More than one skin present in file",
            ));
        }

        let Some(skin) = document.skins().next() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "No skins available",
            ));
        };

        let path = load_context.path().to_string();
        let rig = if path.contains("mixamo") {
            RigType::Mixamo
        } else if path.contains("game_engine") {
            RigType::GameEngine
        } else if path.contains("default") {
            RigType::Default
        } else {
            unimplemented!("Unrecognized rig type in path: {}", path);
        };

        let joints: Vec<gltf::Node> = skin.joints().collect();
        let joint_indices: AHashSet<usize> = joints.iter().map(|j| j.index()).collect();

        let mut node_to_parent: AHashMap<usize, usize> = AHashMap::default();
        for node in document.nodes() {
            for child in node.children() {
                if joint_indices.contains(&child.index()) {
                    node_to_parent.insert(child.index(), node.index());
                }
            }
        }

        let mut bone_parents: AHashMap<&'static str, String> = AHashMap::default();
        let mut local_bindpose: AHashMap<&'static str, Transform> = AHashMap::default();
        let mut bone_names: Vec<&'static str> = Vec::with_capacity(joints.len());

        for joint in &joints {
            let name = joint.name().unwrap_or("");
            let name_interned = NAME_INTERNER.intern(name).leak();
            bone_names.push(name_interned);

            let (translation, rotation, scale) = joint.transform().decomposed();
            local_bindpose.insert(
                name_interned,
                Transform {
                    translation: Vec3::from_array(translation),
                    rotation: Quat::from_array(rotation),
                    scale: Vec3::from_array(scale),
                },
            );

            let parent_idx = node_to_parent.get(&joint.index());
            let parent_name = parent_idx
                .and_then(|&idx| {
                    document.nodes().find(|n| n.index() == idx)
                })
                .and_then(|p| p.name())
                .map(|s| s.to_string())
                .unwrap_or_default();
            bone_parents.insert(name_interned, parent_name);
        }

        let bone_name_to_index: AHashMap<&'static str, usize> = bone_names
            .iter()
            .enumerate()
            .map(|(i, &name)| (name, i))
            .collect();

        let model_space_bindpose = compute_model_space_from_local(&bone_names, &bone_parents, &local_bindpose);

        Ok(ReferenceRigAsset {
            bone_names,
            bone_parents,
            local_bindpose,
            model_space_bindpose,
            bone_name_to_index,
            rig,
        })
    }

}

fn compute_model_space_from_local(
    bone_order: &[&'static str],
    bone_parents: &AHashMap<&'static str, String>,
    local_transforms: &AHashMap<&'static str, Transform>,
) -> AHashMap<&'static str, Transform> {
    let mut model_space = AHashMap::default();
    
    for &name in bone_order {
        let local = &local_transforms[name];
        
        if let Some(parent_name) = bone_parents.get(name) {
            if parent_name.is_empty() {
                model_space.insert(name, *local);
            } else {
                let parent_name_interned = NAME_INTERNER.intern(parent_name).leak();
                if let Some(&parent_global) = model_space.get(parent_name_interned) {
                    let global = Transform::from_matrix(parent_global.to_matrix() * local.to_matrix());
                    model_space.insert(name, global);
                } else {
                    model_space.insert(name, *local);
                }
            }
        } else {
            model_space.insert(name, *local);
        }
    }
    
    model_space
}
