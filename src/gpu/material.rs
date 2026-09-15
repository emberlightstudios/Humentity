//! Crowd skinning material: the [`ExtendedMaterial`] extension that reads the
//! GPU joint buffer, plus [`make_gpu_mesh`] which rebuilds a humentity mesh
//! for the GPU pipeline.

use bevy::{
    asset::RenderAssetUsages,
    mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef, VertexFormat},
    pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::{render_resource::*, storage::ShaderBuffer},
    shader::ShaderRef,
};

use super::CROWD_SKIN_SHADER;

pub type CrowdMaterial = ExtendedMaterial<StandardMaterial, GpuCrowdExtension>;

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct GpuCrowdExtension {
    #[storage(100, read_only)]
    pub joints: Handle<ShaderBuffer>,
    #[uniform(101)]
    pub crowd: GpuCrowdUniform,
}

#[derive(Clone, Copy, ShaderType)]
pub struct GpuCrowdUniform {
    pub num_bones: u32,
    pub num_instances: u32,
    pub pad0: u32,
    pub pad1: u32,
}

impl MaterialExtension for GpuCrowdExtension {
    fn vertex_shader() -> ShaderRef {
        CROWD_SKIN_SHADER.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_gpu_vertex_layout(descriptor, layout)
    }
}

/// Routes the custom GPU joint attributes to shader locations 6/7 so any
/// vertex shader — the default or a custom one calling `gpu_skin_vertex` —
/// receives them. Custom [`MaterialExtension`]s reuse this instead of
/// duplicating the layout walk.
pub fn specialize_gpu_vertex_layout(
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
) -> Result<(), SpecializedMeshPipelineError> {
    let mesh = &layout.0;
    let ids = mesh.attribute_ids();
    let full = mesh.layout();
    for (id, location) in [
        (ATTRIBUTE_GPU_JOINT_INDEX.id, 6u32),
        (ATTRIBUTE_GPU_JOINT_WEIGHT.id, 7u32),
    ] {
        let Some(index) = ids.iter().position(|candidate| *candidate == id) else {
            continue;
        };
        let source = &full.attributes[index];
        if let Some(buffer) = descriptor.vertex.buffers.first_mut() {
            buffer.attributes.push(VertexAttribute {
                format: source.format,
                offset: source.offset,
                shader_location: location,
            });
        }
    }
    Ok(())
}

/// Joint indices for GPU-posed crowds, kept separate from
/// [`Mesh::ATTRIBUTE_JOINT_INDEX`] so the mesh pipeline stays unskinned and no
/// CPU skin buffer is required.
pub const ATTRIBUTE_GPU_JOINT_INDEX: MeshVertexAttribute =
    MeshVertexAttribute::new("GpuJointIndex", 1337, VertexFormat::Uint16x4);
/// Joint weights for GPU-posed crowds. See [`ATTRIBUTE_GPU_JOINT_INDEX`].
pub const ATTRIBUTE_GPU_JOINT_WEIGHT: MeshVertexAttribute =
    MeshVertexAttribute::new("GpuJointWeight", 1338, VertexFormat::Float32x4);

/// Rebuilds a humentity mesh for the GPU pipeline: only position, normal, uv
/// plus the custom GPU joint attributes. Standard joint attributes are
/// dropped so Bevy treats the mesh as unskinned (no CPU skin buffer, no
/// `SkinnedMesh`), while the custom vertex shader still skins from the GPU
/// joint buffer. Morph targets are dropped; the mesh matches the baked
/// bindpose proportions.
///
/// The source mesh must be painted for the same skeleton LOD the bake poses
/// (see [`GpuSkeletonLod`](super::config::GpuSkeletonLod)) so joint indices
/// line up.
pub fn make_gpu_mesh(source: &Mesh) -> Option<Mesh> {
    let positions = source.attribute(Mesh::ATTRIBUTE_POSITION)?.clone();
    let normals = source.attribute(Mesh::ATTRIBUTE_NORMAL)?.clone();
    let uvs = source.attribute(Mesh::ATTRIBUTE_UV_0)?.clone();
    let joint_index = source.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)?.clone();
    let joint_weight = source.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)?.clone();
    let mut mesh = Mesh::new(source.primitive_topology(), RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(ATTRIBUTE_GPU_JOINT_INDEX, joint_index);
    mesh.insert_attribute(ATTRIBUTE_GPU_JOINT_WEIGHT, joint_weight);
    if let Some(indices) = source.indices() {
        mesh.insert_indices(indices.clone());
    }
    Some(mesh)
}
