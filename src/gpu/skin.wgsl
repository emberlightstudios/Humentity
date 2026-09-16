#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    forward_io::VertexOutput,
    morph::{layer_count, weight_at, morph_position, morph_normal},
    view_transformations::position_world_to_clip,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> joints: array<mat4x4f>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> crowd: CrowdUniform;

struct CrowdUniform {
    num_bones: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

// Poses one vertex from the GPU joint buffer: fetches the four influencing
// joints for this character's crowd row, blends them, and returns the skinned
// vertex in world space. `instance_index` is the render batch slot (world
// transform, normals); the crowd row comes from `MeshTag` via `get_tag`, so
// batch order never has to match compute order. Custom vertex shaders call
// this, then add their own work on top of the returned `VertexOutput`.
// `vertex_index` is the render `@builtin(vertex_index)` of this vertex: the
// morph helpers index by position in the vertex buffer, and the mesh vertex
// shader runs before indexing effects, so the raw buffer index is the right
// key. Under `MORPH_TARGETS` offsets are applied first (Bevy's `mesh.wgsl`
// order: morph, then skin), otherwise this is a pure skinning call.
fn gpu_skin_vertex(
    instance_index: u32,
    vertex_index: u32,
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    joint_indices: vec4<u32>,
    joint_weights: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
#ifdef MORPH_TARGETS
    let vertex = vertex_index - mesh[instance_index].first_vertex_index;
    var morphed_position = position;
    var morphed_normal = normal;
    let weight_count = layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i++) {
        let weight = weight_at(i, instance_index);
        if (weight == 0.0) {
            continue;
        }
        morphed_position += weight * morph_position(vertex, i, instance_index);
        morphed_normal += weight * morph_normal(vertex, i, instance_index);
    }
#else
    let morphed_position = position;
    let morphed_normal = normal;
#endif
    let crowd_index = mesh_functions::get_tag(instance_index);
    let base = crowd_index * crowd.num_bones;
    let j0 = joints[base + joint_indices.x];
    let j1 = joints[base + joint_indices.y];
    let j2 = joints[base + joint_indices.z];
    let j3 = joints[base + joint_indices.w];
    let w = joint_weights;
    let skin = j0 * w.x + j1 * w.y + j2 * w.z + j3 * w.w;
    let mesh_world = mesh_functions::get_world_from_local(instance_index);
    let world_from_local = mesh_world * skin;
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(morphed_position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    let skinned_normal = (skin * vec4<f32>(morphed_normal, 0.0)).xyz;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(skinned_normal, instance_index);
    out.uv = uv;
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = instance_index;
#endif
    return out;
}
