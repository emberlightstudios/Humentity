use ahash::AHashMap;
use bevy::prelude::*;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::shader::ShaderRef;
use bevy::render::render_resource::*;

use crate::assets::HumanBodyTextures;

const SHADER_ASSET_PATH: &str = "humentity://shaders/human.wgsl";

/*--------------+
 |   Material   |
 +--------------*/
 #[allow(dead_code)]
#[derive(Default, Clone)]//, ShaderType)]
pub struct HumanMaterialExtensionData {

}

#[derive(Asset, Clone, Reflect, AsBindGroup)]
//#[data(50, HumanMaterialExtensionData, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct HumanMaterialExtension {

}

impl MaterialExtension for HumanMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }
}


/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource, Deref)]
pub struct HumanMaterials(AHashMap<&'static str, ExtendedMaterial<StandardMaterial, HumanMaterialExtension>>);

impl HumanMaterials {
    pub fn new(
        textures: &HumanBodyTextures,
    ) -> Self {
        let mut materials = AHashMap::default();
        for (&name, albedo) in textures.albedo_maps.iter() {
            materials.insert(
                name,
                ExtendedMaterial {
                    base: StandardMaterial {
                        base_color_texture: Some(albedo.clone()),
                        ..default()
                    },
                    extension: HumanMaterialExtension {
                    }
                }
            );
        }
        Self(materials)
    }
}

