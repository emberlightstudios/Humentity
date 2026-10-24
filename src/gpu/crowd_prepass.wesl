import humentity::gpu::crowd_skin;
import bevy_pbr::prepass::io::VertexOutput;

// Posed depth/shadow entry: calls the shared skinning module, then builds
// `prepass::io::VertexOutput` so the depth prepass and shadow passes pose the
// mesh instead of drawing bind pose. Both the depth prepass and the shadow
// passes run this entry (Bevy 0.19 exposes no separate shadow vertex hook on
// `MaterialExtension`).
//
// v1 limits: motion vectors read as zero (previous pose not tracked), no
// color support (crowd meshes carry no vertex colors).
struct CrowdPrepassVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
@if(VERTEX_UVS_A)
    @location(1) uv: vec2<f32>,
@if(VERTEX_UVS_B)
    @location(2) uv_b: vec2<f32>,
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS)
    @location(3) normal: vec3<f32>,
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_TANGENTS)
    @location(4) tangent: vec4<f32>,
    @location(6) joint_indices: vec4<u32>,
    @location(7) joint_weights: vec4<f32>,
@if(MORPH_TARGETS)
    @builtin(vertex_index) vertex_index: u32,
};

@vertex
fn vertex(vertex_in: CrowdPrepassVertex) -> VertexOutput {
@if(MORPH_TARGETS)
    let render_vertex = vertex_in.vertex_index;
@if(!MORPH_TARGETS)
    let render_vertex = 0u;
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS)
    let input_normal = vertex_in.normal;
@if(!(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS))
    let input_normal = vec3<f32>(0.0, 1.0, 0.0);
    let skinned = crowd_skin::crowd_skin_pose(
        vertex_in.instance_index,
        render_vertex,
        vertex_in.position,
        input_normal,
        vertex_in.joint_indices,
        vertex_in.joint_weights,
    );
    var out: VertexOutput;
    out.world_position = skinned.world_position;
    out.position = skinned.clip_position;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
}

@if(VERTEX_UVS_A)
    out.uv = vertex_in.uv;

@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS)
    out.world_normal = skinned.world_normal;
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_TANGENTS)
    out.world_tangent = crowd_skin::crowd_skin_world_tangent(
        skinned,
        vertex_in.tangent,
        render_vertex,
        vertex_in.instance_index,
    );

@if(MOTION_VECTOR_PREPASS)
    // Previous pose is not tracked; motion vectors read as zero.
    out.previous_world_position = out.world_position;

@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = skinned.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = skinned.visibility_range_dither;
    return out;
}
