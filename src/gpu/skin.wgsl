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
    let joint_row_base_index = crowd_index * crowd.num_bones;
    let joint_matrix_0 = joints[joint_row_base_index + joint_indices.x];
    let joint_matrix_1 = joints[joint_row_base_index + joint_indices.y];
    let joint_matrix_2 = joints[joint_row_base_index + joint_indices.z];
    let joint_matrix_3 = joints[joint_row_base_index + joint_indices.w];
    let joint_blend_weights = joint_weights;
    let skin_matrix = joint_matrix_0 * joint_blend_weights.x + joint_matrix_1 * joint_blend_weights.y + joint_matrix_2 * joint_blend_weights.z + joint_matrix_3 * joint_blend_weights.w;
    let mesh_world = mesh_functions::get_world_from_local(instance_index);
    let world_from_local = mesh_world * skin_matrix;
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(morphed_position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    let skinned_normal = (skin_matrix * vec4<f32>(morphed_normal, 0.0)).xyz;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(skinned_normal, instance_index);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(instance_index, mesh_world[3]);
#endif
    return out;
}
