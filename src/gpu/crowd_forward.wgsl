#import humentity::crowd_skin
#import bevy_pbr::forward_io::VertexOutput

// Default color-pass entry: pure call-through to the shared skinning module,
// no extras. Custom vertex shaders do the same and add work on top.
struct CrowdForwardVertex {
    @builtin(instance_index) instance_index: u32,
    @builtin(vertex_index) vertex_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(6) joint_indices: vec4<u32>,
    @location(7) joint_weights: vec4<f32>,
};

@vertex
fn vertex(vertex_in: CrowdForwardVertex) -> VertexOutput {
    let skinned = crowd_skin::crowd_skin_pose(
        vertex_in.instance_index,
        vertex_in.vertex_index,
        vertex_in.position,
        vertex_in.normal,
        vertex_in.joint_indices,
        vertex_in.joint_weights,
    );
    var out: VertexOutput;
    out.position = skinned.clip_position;
    out.world_position = skinned.world_position;
    out.world_normal = skinned.world_normal;
#ifdef VERTEX_UVS_A
    out.uv = vertex_in.uv;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = skinned.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = skinned.visibility_range_dither;
#endif
    return out;
}
