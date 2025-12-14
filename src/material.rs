use bevy::{pbr::MaterialExtension, prelude::*, render::render_resource::*, shader::ShaderRef};

const SHADER_ASSET_PATH: &str = "humentity://shaders/human.wgsl";

/*--------------+
|   Material   |
+--------------*/
#[allow(dead_code)]
#[derive(Default, Clone)] //, ShaderType)]
pub struct CharacterMaterialExtensionData {}

#[derive(Asset, Clone, Reflect, AsBindGroup)]
//#[data(50, HumanMaterialExtensionData, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct CharacterMaterialExtension {}

impl MaterialExtension for CharacterMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }
}
