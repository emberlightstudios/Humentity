use bevy::prelude::*;
use bevy::pbr::MaterialExtension;
use bevy::shader::ShaderRef;
use bevy::render::render_resource::*;


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
