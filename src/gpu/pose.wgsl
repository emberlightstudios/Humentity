struct PoseUniforms {
    num_bones: u32,
    instance_count: u32,
    grid_side: u32,
    shape_count: u32,
    // Clip tables sized to the pose-shader ceiling (64). Keep in sync with config.rs.
    offsets: array<u32, 64>,
    frames: array<u32, 64>,
    durations: array<f32, 64>,
    modes: array<u32, 64>,
    // Skeleton-root rearward Z shift in reference meters (default rig only,
    // zero otherwise). Baked from the rig name; mirrors the CPU
    // skeleton-entity translation. Applied post-`fix` in world space.
    root_z_offset: f32,
};

// Baked reference bind-pose Y of the root bone, in reference model space.
// The CPU rescale compares animation Y against the fitted bind-pose Y; the
// GPU equivalent compares against root_bind_y * root_scale. 1:1 with
// `rescale_root_bone_translation`.
struct RootBindInfo {
    bind_y: f32,
};

@group(0) @binding(0) var<storage, read> parents: array<i32>;
@group(0) @binding(1) var<storage, read> frames: array<mat4x4f>;
@group(0) @binding(2) var<storage, read> inv_bind: array<mat4x4f>;
@group(0) @binding(3) var<storage, read_write> joints: array<mat4x4f>;
@group(0) @binding(4) var<storage, read> uniforms: PoseUniforms;
// Per instance: BLEND_CAP weights + clocks + bank indices (padded to the
// ceiling stride; smaller configs zero-pad on upload).
@group(0) @binding(5) var<storage, read> instance_data: array<f32>;
// Per-shape local rest translations: shape_count * num_bones vec4s. Slice 0
// mirrors the reference translations baked into the clip frames.
@group(0) @binding(6) var<storage, read> shape_translations: array<vec4f>;
// Per-shape inverse bindposes, same layout as shape_translations.
@group(0) @binding(7) var<storage, read> shape_inv_binds: array<mat4x4f>;
// Per-instance shape weights: instance_count * SHAPE_CAP (8) floats, padded to
// the ceiling stride. Slot i blends registered shape i (slice i + 1); same
// semantics as the entity's morph weights, so hybrids ride the true blended
// skeleton.
@group(0) @binding(8) var<storage, read> shape_weights: array<f32>;
// Per-instance root-bone Y scales: one float per instance, mirroring the CPU
// `SkeletonRootBone.root_scale` (fitted_root_y / reference_root_y).
@group(0) @binding(9) var<storage, read> root_scales: array<f32>;
// Reference root bind-pose Y (model space): `binds[0]` local Y with no
// ancestors, since the root has no parent.
@group(0) @binding(10) var<storage, read> root_bind: RootBindInfo;

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

fn instance_shape_weight(instance: u32, shape: u32) -> f32 {
    return shape_weights[instance * 8u + shape];
}

// Blended rest translation for this instance + bone: reference plus the
// weight-scaled deltas of every registered shape. Exact for hybrids because
// every fitted shape restores reference rotations, so only translations
// differ and those are linear in the morph weights — the same blend the mesh
// morph targets apply to the verts.
fn blended_translation(instance: u32, bone: u32) -> vec3f {
    let ref_t = shape_translations[bone].xyz;
    var out = ref_t;
    for (var i = 0u; i < 8u; i++) {
        if (i + 1u >= uniforms.shape_count) {
            break;
        }
        let w = instance_shape_weight(instance, i);
        if (w != 0.0) {
            let t = shape_translations[(i + 1u) * uniforms.num_bones + bone].xyz;
            out += w * (t - ref_t);
        }
    }
    return out;
}

// Blended inverse bindpose, same weight blend as the translations. Valid for
// the same reason: shared rotations, translation-only deltas, so the lerp of
// rigid inverses stays a rigid inverse.
fn blended_inv_bind(instance: u32, bone: u32) -> mat4x4f {
    let ref_ib = shape_inv_binds[bone];
    var out = ref_ib;
    for (var i = 0u; i < 8u; i++) {
        if (i + 1u >= uniforms.shape_count) {
            break;
        }
        let w = instance_shape_weight(instance, i);
        if (w != 0.0) {
            let ib = shape_inv_binds[(i + 1u) * uniforms.num_bones + bone];
            out += w * (ib - ref_ib);
        }
    }
    return out;
}

fn mat_mix(a: mat4x4f, b: mat4x4f, f: f32) -> mat4x4f {
    return mat4x4f(mix(a[0], b[0], f), mix(a[1], b[1], f), mix(a[2], b[2], f), mix(a[3], b[3], f));
}

fn frame_at(bank: u32, bone: u32, frame: u32) -> mat4x4f {
    return frames[(uniforms.offsets[bank] + frame) * uniforms.num_bones + bone];
}

// Frame interpolation (time -> frame pair + fraction) shared by every clip
// read, so the root pre-pass and the main blend sample identically.
fn frame_pair(bank: u32, instance: u32, slot: u32) -> vec3u {
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
    return vec3u(frame0, frame1, bitcast<u32>(fract(ft)));
}

// Root Y for this instance at this frame blend, with the CPU rescale applied:
// the baked clip translation is reference-proportioned, so re-apply its delta
// scaled by the instance's fitted root scale. Matches
// `rescale_root_bone_translation` 1:1: delta measured against the fitted bind
// (reference bind times scale), deadzone, then X/Z zeroing.
fn rescaled_root_y(clip_y: f32, instance: u32) -> vec3f {
    let fitted_bind_y = root_bind.bind_y * root_scales[instance];
    let delta = clip_y - fitted_bind_y;
    var y = fitted_bind_y;
    if (abs(delta) > 0.001) {
        y = clip_y * root_scales[instance];
    }
    return vec3f(0.0, y, 0.0);
}

fn clip_local(slot: u32, bone: u32, instance: u32, fitted: vec3f, is_root: bool) -> mat4x4f {
    let bank = instance_bank(instance, slot);
    let pair = frame_pair(bank, instance, slot);
    let f = bitcast<f32>(pair.z);
    // Swap the baked reference translation for this instance's blended rest
    // translation. Rotations/scales play through untouched: clips are authored
    // for reference rotations, which every fitted shape restores. The root
    // gets the rescaled clip Y instead of the fitted rest (same fix the CPU
    // post-update applies).
    let a = frame_at(bank, bone, pair.x);
    let b = frame_at(bank, bone, pair.y);
    var trans = fitted;
    if (is_root) {
        trans = rescaled_root_y(mix(a[3].y, b[3].y, f), instance);
    }
    let a_fitted = mat4x4f(a[0], a[1], a[2], vec4f(trans, 1.0));
    let b_fitted = mat4x4f(b[0], b[1], b[2], vec4f(trans, 1.0));
    return mat_mix(a_fitted, b_fitted, f);
}

fn blended_local(bone: u32, instance: u32, fitted: vec3f, wsum: f32, is_root: bool) -> mat4x4f {
    var local = mat4x4f(vec4f(0.0), vec4f(0.0), vec4f(0.0), vec4f(0.0));
    for (var s = 0u; s < 4u; s++) {
        local += clip_local(s, bone, instance, fitted, is_root) * (instance_weight(instance, s) / wsum);
    }
    return local;
}

@compute @workgroup_size(8, 8, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // Instances tile across X/Y on a square grid of `grid_side`; bones run
    // down Z. This keeps every dispatch dimension small: X/Y grow with
    // sqrt(instances), Z with bones.
    let instance = gid.x + gid.y * uniforms.grid_side;
    let bone = gid.z;
    if (instance >= uniforms.instance_count || bone >= uniforms.num_bones) {
        return;
    }
    let idx = instance * uniforms.num_bones + bone;
    let fitted = blended_translation(instance, bone);
    let is_root = bone == 0u;

    var wsum = 0.0;
    for (var s = 0u; s < 4u; s++) {
        wsum += instance_weight(instance, s);
    }
    wsum = max(wsum, 0.00001);

    var model = blended_local(bone, instance, fitted, wsum, is_root);
    var p = parents[bone];
    var guard = 0;
    while (p >= 0 && guard < 128) {
        let pb = u32(p);
        model = blended_local(pb, instance, blended_translation(instance, pb), wsum, pb == 0u) * model;
        p = parents[pb];
        guard += 1;
    }
    // MODEL_ROTATION_FIX: model verts face +Z, so CPU skeletons carry a PI
    // rotation about Y on the skeleton entity to face -Z. The GPU has no
    // skeleton entity, so the pose root carries the same rotation: premultiply
    // the model chain once, exactly like the skeleton-entity global on CPU.
    let fix = mat4x4f(
        vec4f(-1.0, 0.0, 0.0, 0.0),
        vec4f(0.0, 1.0, 0.0, 0.0),
        vec4f(0.0, 0.0, -1.0, 0.0),
        vec4f(0.0, 0.0, 0.0, 1.0),
    );
    var joint = fix * model * blended_inv_bind(instance, bone);
    // Default-rig rearward shift: the CPU applies this as the skeleton-entity
    // translation (post-`fix` world space), so it adds here and not to a
    // bone-local translation (which `fix` would flip). Every joint carries
    // it, exactly like the entity translation propagating to every bone.
    joint[3] += vec4f(0.0, 0.0, uniforms.root_z_offset, 0.0);
    joints[idx] = joint;
}
