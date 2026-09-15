struct PoseUniforms {
    num_bones: u32,
    num_frames: u32,
    instance_count: u32,
    clip_count: u32,
    durations: array<f32, 4>,
    modes: array<u32, 4>,
};

@group(0) @binding(0) var<storage, read> parents: array<i32>;
@group(0) @binding(1) var<storage, read> frames: array<mat4x4f>;
@group(0) @binding(2) var<storage, read> inv_bind: array<mat4x4f>;
@group(0) @binding(3) var<storage, read_write> joints: array<mat4x4f>;
@group(0) @binding(4) var<storage, read> uniforms: PoseUniforms;
@group(0) @binding(5) var<storage, read> instance_data: array<f32>;

fn instance_weight(instance: u32, clip: u32) -> f32 {
    return instance_data[instance * 8u + clip];
}

fn instance_time(instance: u32, clip: u32) -> f32 {
    return instance_data[instance * 8u + 4u + clip];
}

fn mat_mix(a: mat4x4f, b: mat4x4f, f: f32) -> mat4x4f {
    return mat4x4f(mix(a[0], b[0], f), mix(a[1], b[1], f), mix(a[2], b[2], f), mix(a[3], b[3], f));
}

fn frame_at(clip: u32, bone: u32, frame: u32) -> mat4x4f {
    return frames[(clip * uniforms.num_frames + frame) * uniforms.num_bones + bone];
}

fn clip_local(clip: u32, bone: u32, instance: u32) -> mat4x4f {
    let duration = max(uniforms.durations[clip], 0.0001);
    let t = instance_time(instance, clip);
    var ft: f32;
    var frame0: u32;
    var frame1: u32;
    if (uniforms.modes[clip] == 1u) {
        let clamped = min(t, duration - 0.0001);
        ft = clamped / duration * f32(uniforms.num_frames);
        frame0 = min(u32(ft), uniforms.num_frames - 1u);
        frame1 = min(frame0 + 1u, uniforms.num_frames - 1u);
    } else {
        let wrapped = fract(t / duration) * f32(uniforms.num_frames);
        frame0 = u32(wrapped) % uniforms.num_frames;
        frame1 = (frame0 + 1u) % uniforms.num_frames;
        ft = wrapped;
    }
    let f = fract(ft);
    return mat_mix(frame_at(clip, bone, frame0), frame_at(clip, bone, frame1), f);
}

fn blended_local(bone: u32, instance: u32, wsum: f32) -> mat4x4f {
    var local = mat4x4f(vec4f(0.0), vec4f(0.0), vec4f(0.0), vec4f(0.0));
    for (var c = 0u; c < uniforms.clip_count; c++) {
        local += clip_local(c, bone, instance) * (instance_weight(instance, c) / wsum);
    }
    return local;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let total = uniforms.instance_count * uniforms.num_bones;
    let idx = gid.x;
    if (idx >= total) {
        return;
    }
    let bone = idx % uniforms.num_bones;
    let instance = idx / uniforms.num_bones;

    var wsum = 0.0;
    for (var c = 0u; c < uniforms.clip_count; c++) {
        wsum += instance_weight(instance, c);
    }
    wsum = max(wsum, 0.00001);

    var model = blended_local(bone, instance, wsum);
    var p = parents[bone];
    var guard = 0;
    while (p >= 0 && guard < 128) {
        let pb = u32(p);
        model = blended_local(pb, instance, wsum) * model;
        p = parents[pb];
        guard += 1;
    }
    joints[idx] = model * inv_bind[bone];
}
