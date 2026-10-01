// Posed depth/shadow entry for GPU crowds: same joint-buffer skinning as
// `skin.wgsl`, but writes `prepass_io::VertexOutput` so the depth prepass and
// shadow passes can pose the mesh instead of drawing bind pose.
//
// Standalone on purpose: composing the shared `skin.wgsl` snippet would drag
// in `forward_io::VertexOutput` and collide with the prepass output type.
// Keep the joint fetch below in sync with `skin.wgsl`.
//
// v1 limits: motion vectors read as zero (previous pose not tracked), no
// tangent/color support (crowd meshes carry neither).
#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    prepass_io::VertexOutput,
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

// Input locations mirror `prepass_io::Vertex` (0/1/2/3/4); joints ride the
// crowd custom attributes at 6/7 (crowd meshes are never SKINNED and carry no
// color, so neither slot collides).
struct CrowdPrepassVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_UVS_A
    @location(1) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(2) uv_b: vec2<f32>,
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    @location(3) normal: vec3<f32>,
#endif
#ifdef VERTEX_TANGENTS
    @location(4) tangent: vec4<f32>,
#endif
#endif
    @location(6) joint_indices: vec4<u32>,
    @location(7) joint_weights: vec4<f32>,
#ifdef MORPH_TARGETS
    @builtin(vertex_index) vertex_index: u32,
#endif
};

@vertex
fn vertex(vertex_in: CrowdPrepassVertex) -> VertexOutput {
    var out: VertexOutput;
    let instance_index = vertex_in.instance_index;

    var skinned_position = vertex_in.position;
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    var skinned_normal = vertex_in.normal;
#endif
#endif

#ifdef MORPH_TARGETS
    let first_vertex = mesh[instance_index].first_vertex_index;
    let morph_vertex_index = vertex_in.vertex_index - first_vertex;
    let weight_count = layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i++) {
        let morph_weight = weight_at(i, instance_index);
        if (morph_weight == 0.0) {
            continue;
        }
        skinned_position += morph_weight * morph_position(morph_vertex_index, i, instance_index);
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
        skinned_normal += morph_weight * morph_normal(morph_vertex_index, i, instance_index);
#endif
#endif
    }
#endif

    let crowd_index = mesh_functions::get_tag(instance_index);
    let joint_row_base_index = crowd_index * crowd.num_bones;
    let joint_matrix_0 = joints[joint_row_base_index + vertex_in.joint_indices.x];
    let joint_matrix_1 = joints[joint_row_base_index + vertex_in.joint_indices.y];
    let joint_matrix_2 = joints[joint_row_base_index + vertex_in.joint_indices.z];
    let joint_matrix_3 = joints[joint_row_base_index + vertex_in.joint_indices.w];
    let skin_matrix = joint_matrix_0 * vertex_in.joint_weights.x + joint_matrix_1 * vertex_in.joint_weights.y + joint_matrix_2 * vertex_in.joint_weights.z + joint_matrix_3 * vertex_in.joint_weights.w;
    let mesh_world = mesh_functions::get_world_from_local(instance_index);
    let world_from_local = mesh_world * skin_matrix;

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(skinned_position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif

#ifdef VERTEX_UVS_A
    out.uv = vertex_in.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex_in.uv_b;
#endif

#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    let posed_normal = (skin_matrix * vec4<f32>(skinned_normal, 0.0)).xyz;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(posed_normal, instance_index);
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex_in.tangent, instance_index);
#endif
#endif

#ifdef MOTION_VECTOR_PREPASS
    // Previous pose is not tracked; motion vectors read as zero.
    out.previous_world_position = out.world_position;
#endif

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(instance_index, mesh_world[3]);
#endif
    return out;
}
