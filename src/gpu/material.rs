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

use super::{CROWD_FORWARD_SHADER, CROWD_PREPASS_SHADER};

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
    pub pad_a: u32,
    pub pad_b: u32,
    pub pad_c: u32,
}

impl MaterialExtension for GpuCrowdExtension {

    fn vertex_shader() -> ShaderRef {
        CROWD_FORWARD_SHADER.into()
    }

    // Posed depth/shadow entry: Bevy 0.19.1+ binds the real material layout
    // for custom `prepass_vertex_shader` (`prepass_reads_material`, #24843),
    // so the joint buffer at group `MATERIAL_BIND_GROUP` (bindings 100/101)
    // validates in the depth prepass and shadow passes. Requires
    // `bevy_pbr >= 0.19.1`; on 0.19.0 the depth-only path pushes an empty
    // layout and the custom entry cannot bind.
    fn prepass_vertex_shader() -> ShaderRef {
        CROWD_PREPASS_SHADER.into()
    }

    // Deferred geometry pass poses like the prepass: same `prepass_io`
    // output, same joint buffer. Without this a deferred camera falls back
    // to Bevy's default (unposed) vertex entry.
    fn deferred_vertex_shader() -> ShaderRef {
        CROWD_PREPASS_SHADER.into()
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
/// vertex shader — the default or a custom one calling the shared
/// `humentity::crowd_skin` module — receives them. Custom
/// [`MaterialExtension`]s reuse this instead of duplicating the layout walk.
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

/// Rebuilds a humentity mesh for the GPU pipeline: position, normal, uv, plus
/// tangents when the source mesh carries them, plus the custom GPU joint
/// attributes. Standard joint attributes are dropped so Bevy treats the mesh as unskinned (no CPU skin buffer, no
/// `SkinnedMesh`), while the custom vertex shader still skins from the GPU
/// joint buffer. Morph targets (and names) are preserved so per-entity
/// `MeshMorphWeights` keep working through the GPU vertex shader.
/// The source mesh must be painted for the same skeleton LOD the bake poses
/// (see [`GpuSkeletonLod`](super::config::GpuSkeletonLod)) so joint indices
/// line up.
pub fn make_gpu_mesh(source: &Mesh) -> Option<Mesh> {
    let positions = source.attribute(Mesh::ATTRIBUTE_POSITION)?.clone();
    let normals = source.attribute(Mesh::ATTRIBUTE_NORMAL)?.clone();
    let uvs = source.attribute(Mesh::ATTRIBUTE_UV_0)?.clone();
    let joint_index = source.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)?.clone();
    let joint_weight = source.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)?.clone();
    let cpu_tangents = source.attribute(Mesh::ATTRIBUTE_TANGENT).cloned();
    // Examples read the converted mesh back on the CPU (`has_morph_targets` in
    // `gpu_morphs::spawn_crowd` runs after render extraction), so keep the
    // data in the main world. `RENDER_WORLD` alone extracts the attributes
    // away and that read panics.
    let mut mesh = Mesh::new(
        source.primitive_topology(),
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if let Some(tangent_values) = cpu_tangents {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangent_values);
    }
    mesh.insert_attribute(ATTRIBUTE_GPU_JOINT_INDEX, joint_index);
    mesh.insert_attribute(ATTRIBUTE_GPU_JOINT_WEIGHT, joint_weight);
    if let Some(indices) = source.indices() {
        mesh.insert_indices(indices.clone());
    }
    if let Some(targets) = source.morph_targets() {
        mesh.set_morph_targets(targets.clone());
    }
    if let Some(names) = source.morph_target_names() {
        mesh.set_morph_target_names(names.to_vec());
    }
    Some(mesh)
}
