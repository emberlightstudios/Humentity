#define_import_path humentity::crowd_skin

#import bevy_pbr::mesh_bindings::mesh
#import bevy_pbr::mesh_functions
#import bevy_pbr::morph
#import bevy_pbr::view_transformations

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> crowd_joints: array<mat4x4f>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> crowd_pose: CrowdSkinUniform;

struct CrowdSkinUniform {
    num_bones: u32,
    pad_a: u32,
    pad_b: u32,
    pad_c: u32,
};

// Posed-vertex data with no `forward_io` / `prepass_io` types, so every entry
// can consume it and build its own output.
struct CrowdSkinnedVertex {
    clip_position: vec4<f32>,
    world_position: vec4<f32>,
    world_normal: vec3<f32>,
    skin_matrix: mat4x4f,
    mesh_world: mat4x4f,
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    instance_index: u32,
#endif
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither: i32,
#endif
};

// Poses one vertex from the GPU joint buffer: morph targets first (Bevy's
// `mesh.wgsl` order: morph, then skin), then the four influencing joints for
// this character's crowd row, blended. `instance_index` is the render batch
// slot (world transform, normals); the crowd row comes from `MeshTag` via
// `get_tag`, so batch order never has to match compute order.
// `vertex_index` is the render `@builtin(vertex_index)` of this vertex: the
// morph helpers index by position in the vertex buffer, and the mesh vertex
// shader runs before indexing effects, so the raw buffer index is the right
// key.
fn crowd_skin_pose(
    instance_index: u32,
    vertex_index: u32,
    position: vec3<f32>,
    normal: vec3<f32>,
    joint_indices: vec4<u32>,
    joint_weights: vec4<f32>,
) -> CrowdSkinnedVertex {
    var skinned: CrowdSkinnedVertex;
#ifdef MORPH_TARGETS
    let first_vertex = mesh[instance_index].first_vertex_index;
    let morph_vertex = vertex_index - first_vertex;
    var morphed_position = position;
    var morphed_normal = normal;
    let weight_count = morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i++) {
        let morph_weight = morph::weight_at(i, instance_index);
        if (morph_weight == 0.0) {
            continue;
        }
        morphed_position += morph_weight * morph::morph_position(morph_vertex, i, instance_index);
        morphed_normal += morph_weight * morph::morph_normal(morph_vertex, i, instance_index);
    }
#else
    let morphed_position = position;
    let morphed_normal = normal;
#endif
    let crowd_index = mesh_functions::get_tag(instance_index);
    let joint_row_base = crowd_index * crowd_pose.num_bones;
    let joint_matrix_0 = crowd_joints[joint_row_base + joint_indices.x];
    let joint_matrix_1 = crowd_joints[joint_row_base + joint_indices.y];
    let joint_matrix_2 = crowd_joints[joint_row_base + joint_indices.z];
    let joint_matrix_3 = crowd_joints[joint_row_base + joint_indices.w];
    let skin_matrix =
        joint_matrix_0 * joint_weights.x + joint_matrix_1 * joint_weights.y +
        joint_matrix_2 * joint_weights.z + joint_matrix_3 * joint_weights.w;
    let mesh_world = mesh_functions::get_world_from_local(instance_index);
    let world_from_local = mesh_world * skin_matrix;
    skinned.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(morphed_position, 1.0)
    );
    skinned.clip_position = view_transformations::position_world_to_clip(skinned.world_position.xyz);
    let posed_normal = (skin_matrix * vec4<f32>(morphed_normal, 0.0)).xyz;
    skinned.world_normal = mesh_functions::mesh_normal_local_to_world(posed_normal, instance_index);
    skinned.skin_matrix = skin_matrix;
    skinned.mesh_world = mesh_world;
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    skinned.instance_index = instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    skinned.visibility_range_dither =
        mesh_functions::get_visibility_range_dither_level(instance_index, mesh_world[3]);
#endif
    return skinned;
}

// Poses a tangent from already-posed data: morph deltas first (Bevy's
// `mesh.wgsl` order: morph, then skin), then the character's skin matrix
// into world space.
fn crowd_skin_world_tangent(
    skinned: CrowdSkinnedVertex,
    tangent: vec4<f32>,
    vertex_index: u32,
    instance_index: u32,
) -> vec4<f32> {
    var morphed_tangent = tangent;
#ifdef MORPH_TARGETS
    let first_vertex = mesh[instance_index].first_vertex_index;
    let morph_vertex = vertex_index - first_vertex;
    let weight_count = morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i++) {
        let morph_weight = morph::weight_at(i, instance_index);
        if (morph_weight == 0.0) {
            continue;
        }
        morphed_tangent += vec4<f32>(morph_weight * morph::morph_tangent(morph_vertex, i, instance_index), 0.0);
    }
#endif
    let world_from_local = skinned.mesh_world * skinned.skin_matrix;
    return mesh_functions::mesh_tangent_local_to_world(world_from_local, morphed_tangent, instance_index);
}
