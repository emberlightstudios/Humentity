use bevy::pbr::MaterialExtension;
use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::shader::ShaderRef;

const SHADER_ASSET_PATH: &str = "humentity://shaders/human.wgsl";

//#[derive(Default, Clone, ShaderType)]
//pub struct CharacterMaterialExtensionData {
//    pub overlay1: u32,
//    pub overlay2: u32,
//    pub overlay3: u32,
//    pub overlay4: u32,
//}

#[derive(Asset, Clone, Reflect, AsBindGroup)]
//#[data(50, CharacterMaterialExtensionData, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct CharacterMaterialExtension {
}

impl MaterialExtension for CharacterMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }
}

//impl<'a> From<&'a CharacterMaterialExtension> for CharacterMaterialExtensionData {
//    fn from(value: &'a CharacterMaterialExtension) -> Self {
//        CharacterMaterialExtensionData {
//            overlay1: value.overlays[0].unwrap_or_default(),
//            overlay2: value.overlays[1].unwrap_or_default(),
//            overlay3: value.overlays[2].unwrap_or_default(),
//            overlay4: value.overlays[3].unwrap_or_default(),
//        }
//    }
//}
