use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::shader::ShaderRef;

const SHADER_ASSET_PATH: &str = "humentity://shaders/human.wgsl";


#[derive(Default, Clone, ShaderType)]
pub struct CharacterMaterialExtensionUniform {
    pub some_int: u32,
}

#[derive(Asset, Clone, Reflect, AsBindGroup)]
#[data(50, CharacterMaterialExtensionUniform, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
#[bind_group_data(CharacterMaterialKey)]
pub struct CharacterMaterialExtension {
    pub some_int: u32,
    pub noise: bool,
}

#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct CharacterMaterialKey {
    noise: bool,
}

impl From<&CharacterMaterialExtension> for CharacterMaterialKey {
    fn from(material: &CharacterMaterialExtension) -> CharacterMaterialKey {
        Self {
            noise: material.noise
        }
    }
}

impl MaterialExtension for CharacterMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> std::result::Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.noise {
            if let Ok(frag) = descriptor.fragment_mut() {
                frag.shader_defs.push("NOISE".into());
            }
        }
        Ok(())
    }
}

impl<'a> From<&'a CharacterMaterialExtension> for CharacterMaterialExtensionUniform {
    fn from(value: &'a CharacterMaterialExtension) -> Self {
        CharacterMaterialExtensionUniform {
            some_int: value.some_int,
        }
    }
}
