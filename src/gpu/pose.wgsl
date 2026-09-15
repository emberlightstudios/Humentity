struct PoseUniforms {
    num_bones: u32,
    instance_count: u32,
    _pad0: u32,
    _pad1: u32,
    // Clip tables sized for MAX_GPU_CLIPS (64). Keep in sync with config.rs.
    offsets: array<u32, 64>,
    frames: array<u32, 64>,
    durations: array<f32, 64>,
    modes: array<u32, 64>,
};

@group(0) @binding(0) var<storage, read> parents: array<i32>;
@group(0) @binding(1) var<storage, read> frames: array<mat4x4f>;
@group(0) @binding(2) var<storage, read> inv_bind: array<mat4x4f>;
@group(0) @binding(3) var<storage, read_write> joints: array<mat4x4f>;
@group(0) @binding(4) var<storage, read> uniforms: PoseUniforms;
// Per instance: weights[4] + clocks[4] + bank indices[4] (MAX_BLEND_CLIPS = 4).
@group(0) @binding(5) var<storage, read> instance_data: array<f32>;

fn instance_weight(instance: u32, slot: u32) -> f32 {
    return instance_data[instance * 12u + slot];
}

fn instance_time(instance: u32, slot: u32) -> f32 {
    return instance_data[instance * 12u + 4u + slot];
}

// Blend slot -> global bank clip. Clamped so a stale slot can never read OOB.
fn instance_bank(instance: u32, slot: u32) -> u32 {
    return min(u32(instance_data[instance * 12u + 8u + slot]), 63u);
}

fn mat_mix(a: mat4x4f, b: mat4x4f, f: f32) -> mat4x4f {
    return mat4x4f(mix(a[0], b[0], f), mix(a[1], b[1], f), mix(a[2], b[2], f), mix(a[3], b[3], f));
}

fn frame_at(bank: u32, bone: u32, frame: u32) -> mat4x4f {
    return frames[(uniforms.offsets[bank] + frame) * uniforms.num_bones + bone];
}

fn clip_local(slot: u32, bone: u32, instance: u32) -> mat4x4f {
    let bank = instance_bank(instance, slot);
    let duration = max(uniforms.durations[bank], 0.0001);
    let nf = max(uniforms.frames[bank], 1u);
    let nff = f32(nf);
    let t = instance_time(instance, slot);
    var ft: f32;
    var frame0: u32;
    var frame1: u32;
    if (uniforms.modes[bank] == 1u) {
        let clamped = min(t, duration - 0.0001);
        ft = clamped / duration * nff;
        frame0 = min(u32(ft), nf - 1u);
        frame1 = min(frame0 + 1u, nf - 1u);
    } else {
        let wrapped = fract(t / duration) * nff;
        frame0 = u32(wrapped) % nf;
        frame1 = (frame0 + 1u) % nf;
        ft = wrapped;
    }
    let f = fract(ft);
    return mat_mix(frame_at(bank, bone, frame0), frame_at(bank, bone, frame1), f);
}

fn blended_local(bone: u32, instance: u32, wsum: f32) -> mat4x4f {
    var local = mat4x4f(vec4f(0.0), vec4f(0.0), vec4f(0.0), vec4f(0.0));
    for (var s = 0u; s < 4u; s++) {
        local += clip_local(s, bone, instance) * (instance_weight(instance, s) / wsum);
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
    for (var s = 0u; s < 4u; s++) {
        wsum += instance_weight(instance, s);
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
