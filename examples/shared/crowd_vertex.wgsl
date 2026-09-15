// Shared example entry: pure call-through to the shared skinning function.
// Custom work goes after the call, before returning.
struct GpuVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(6) joint_indices: vec4<u32>,
    @location(7) joint_weights: vec4<f32>,
};

@vertex
fn vertex(vertex: GpuVertex) -> VertexOutput {
    return gpu_skin_vertex(
        vertex.instance_index,
        vertex.position,
        vertex.normal,
        vertex.uv,
        vertex.joint_indices,
        vertex.joint_weights,
    );
}
