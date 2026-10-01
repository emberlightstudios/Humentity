#import humentity::crowd_skin
#import bevy_pbr::prepass_io::VertexOutput

// Posed depth/shadow entry: calls the shared skinning module, then builds
// `prepass_io::VertexOutput` so the depth prepass and shadow passes pose the
// mesh instead of drawing bind pose. Both the depth prepass and the shadow
// passes run this entry (Bevy 0.19 exposes no separate shadow vertex hook on
// `MaterialExtension`).
//
// v1 limits: motion vectors read as zero (previous pose not tracked), no
// tangent/color support (crowd meshes carry neither).
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
#ifdef MORPH_TARGETS
    let render_vertex = vertex_in.vertex_index;
#else
    let render_vertex = 0u;
#endif
    let skinned = crowd_skin::crowd_skin_pose(
        vertex_in.instance_index,
        render_vertex,
        vertex_in.position,
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
        vertex_in.normal,
#else
        vec3<f32>(0.0, 1.0, 0.0),
#endif
#else
        vec3<f32>(0.0, 1.0, 0.0),
#endif
        vertex_in.joint_indices,
        vertex_in.joint_weights,
    );
    var out: VertexOutput;
    out.world_position = skinned.world_position;
    out.position = skinned.clip_position;
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif

#ifdef VERTEX_UVS_A
    out.uv = vertex_in.uv;
#endif

#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    out.world_normal = skinned.world_normal;
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = crowd_skin::crowd_skin_world_tangent(
        skinned,
        vertex_in.tangent,
        vertex_in.instance_index,
    );
#endif
#endif

#ifdef MOTION_VECTOR_PREPASS
    // Previous pose is not tracked; motion vectors read as zero.
    out.previous_world_position = out.world_position;
#endif

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = skinned.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = skinned.visibility_range_dither;
#endif
    return out;
}
