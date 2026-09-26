//! Base bake + background clip bakes for the GPU crowd.
//!
//! The first pass samples nothing: it snapshots the LOD skeleton (bones,
//! animation targets, bindposes), uploads the static buffers (`parents`,
//! `inv_bind`, the frame-0 bindpose clip, `joints`, `uniforms`,
//! `instance_data`), creates the [`GpuAnimationBank`] registry, and seeds
//! per-instance state. Clip loads are always manual: callers queue them via
//! [`GpuAnimationBank::request_load`](super::bank::GpuAnimationBank::request_load),
//! follow-up passes dispatch one background [`AsyncComputeTaskPool`] task per
//! queued load; completions are collected on the main thread, appended to the
//! shared `frames` buffer, and published through the bank tables +
//! [`GpuRenderHandles`]. Unloads clear the bank slot and the uniform row
//! (neighbours never move).

use ahash::AHashMap;
use bevy::{
    animation::{AnimationClip, AnimationTargetId, animated_field},
    asset::RenderAssetUsages,
    prelude::*,
    render::storage::ShaderBuffer,
    tasks::AsyncComputeTaskPool,
};

use crate::{prelude::*, rigs::{RigBundleRes, skeleton_rear_offset_meters}};

use super::{
    bank::{
        BIND_POSE_CLIP, BIND_POSE_SLOT, BakedClip, BankSlot, GpuAnimationBank, GpuAnimationReady,
        GpuBakeJobs, GpuRenderHandles,
    },
    config::{
        BLEND_CAP, CLIP_CAP, GpuBlendWeights, GpuCrowdConfig, GpuSkeletonLod, SHAPE_CAP,
        pose_grid_side,
    },
    state::GpuInstanceAnims,
};

fn joint_path(bone: &str, rig: &RigSpec) -> Vec<Name> {
    let reference = rig.reference_rig();
    let mut chain = vec![bone.to_string()];
    let mut parent = reference
        .bone_parents
        .get(bone)
        .cloned()
        .unwrap_or_default();
    while !parent.is_empty() && parent != "Human.rig" {
        chain.push(parent.clone());
        parent = reference
            .bone_parents
            .get(parent.as_str())
            .cloned()
            .unwrap_or_default();
    }
    let mut path = vec![Name::new("Human.rig")];
    for name in chain.iter().rev() {
        path.push(Name::new(name.clone()));
    }
    path
}

/// Pure computation: samples `clip` onto the snapshot LOD skeleton at the
/// bank's fixed rate. Runs on a background thread with no ECS access.
///
/// Frames use inclusive-endpoint sampling: frame `i` of `n` sits at
/// `i / (n - 1) * duration`, so the last frame lands exactly on `duration`
/// (clamped to the final key) and the pose shader's `frame(nf-1) -> frame(0)`
/// wrap blends the true loop endpoints instead of a near-miss pair.
fn sample_clip_frames(
    clip: &AnimationClip,
    duration: f32,
    bones: &[&'static str],
    targets: &[AnimationTargetId],
    binds: &[Transform],
    sample_rate: f32,
) -> Vec<Mat4> {
    let frames = ((duration * sample_rate.max(1.0)).ceil() as usize).max(1);
    let mut out = Vec::with_capacity(bones.len() * frames);
    for frame in 0..frames {
        let time = if frames == 1 {
            0.0
        } else {
            frame as f32 / (frames - 1) as f32 * duration
        };
        for (index, _) in bones.iter().enumerate() {
            let bind = binds[index];
            let mut translation = clip
                .sample_clamped(
                    animated_field!(Transform::translation),
                    targets[index],
                    time,
                )
                .unwrap_or(bind.translation);
            if index == 0 {
                // Match the CPU path (`rescale_root_bone_translation`):
                // locomotion comes from gameplay movement, never the clip.
                translation.x = 0.0;
                translation.z = 0.0;
            }
            let rotation = clip
                .sample_clamped(animated_field!(Transform::rotation), targets[index], time)
                .unwrap_or(bind.rotation);
            let scale = clip
                .sample_clamped(animated_field!(Transform::scale), targets[index], time)
                .unwrap_or(bind.scale);
            out.push(
                Transform {
                    translation,
                    rotation,
                    scale,
                }
                .to_matrix(),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translation_clip(times: &[f32]) -> AnimationClip {
        let mut clip = AnimationClip::default();
        let target = AnimationTargetId::from_name(&Name::new("bone"));
        clip.add_curve_to_target(
            target,
            AnimatableCurve::new(
                animated_field!(Transform::translation),
                AnimatableKeyframeCurve::new(
                    times
                        .iter()
                        .enumerate()
                        .map(|(i, t)| (*t, Vec3::new(0.0, i as f32, 0.0))),
                )
                .unwrap(),
            ),
        );
        clip
    }

    /// The wrap blend `frame(nf-1) -> frame(0)` must span the true loop
    /// endpoints. With exclusive sampling the last frame sits a full step
    /// before `duration`, so a clip whose only motion is at the end key
    /// never appears in the bake and the seam jumps.
    #[test]
    fn baked_last_frame_samples_duration_endpoint() {
        let clip = translation_clip(&[0.0, 2.0]);
        let bones = ["bone"];
        let targets = [AnimationTargetId::from_name(&Name::new("bone"))];
        let binds = [Transform::IDENTITY];
        let frames = sample_clip_frames(&clip, 2.0, &bones, &targets, &binds, 30.0);
        assert_eq!(frames.len() / bones.len(), 60);
        let last = frames.last().unwrap().to_scale_rotation_translation().2;
        assert!(
            (last.y - 1.0).abs() < 1e-4,
            "last baked frame must hold the duration endpoint, got {last:?}"
        );
    }
}
/// First pass: uploads static buffers + frame-0 bindpose seed, creates the
/// bank, and seeds per-instance state. Clip loads are manual via
/// [`GpuAnimationBank::request_load`](super::bank::GpuAnimationBank::request_load).
pub(super) fn bake_gpu_animation(
    mut commands: Commands,
    config: Res<GpuCrowdConfig>,
    blend_weights: Res<GpuBlendWeights>,
    skeleton_lod: Res<GpuSkeletonLod>,
    rig_data: Res<RigData>,
    rig_bundle: Res<RigBundleRes>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut base_done: Local<bool>,
) {
    if *base_done {
        return;
    }
    let Some(rig) = rig_data.0.as_ref() else {
        return;
    };
    let reference = rig.reference_rig();
    let Some(bundle) = rig_bundle.bundle.as_ref() else {
        return;
    };
    let Some(data) = bundle.lod_data.get(skeleton_lod.0) else {
        return;
    };
    let bones: Vec<&'static str> = data.bone_names.clone();
    *base_done = true;

    let targets: Vec<AnimationTargetId> = bones
        .iter()
        .map(|bone| AnimationTargetId::from_names(joint_path(bone, rig).iter()))
        .collect();
    let binds: Vec<Transform> = bones
        .iter()
        .map(|bone| reference.local_bindpose[bone])
        .collect();

    let lod_index: AHashMap<&'static str, usize> = bones
        .iter()
        .enumerate()
        .map(|(i, &bone)| (bone, i))
        .collect();
    let mut parents = Vec::with_capacity(bones.len());
    for bone in bones.iter() {
        let parent = reference
            .bone_parents
            .get(*bone)
            .cloned()
            .unwrap_or_default();
        if parent.is_empty() || parent == "Human.rig" {
            parents.push(-1i32);
        } else {
            parents.push(
                lod_index
                    .get(parent.as_str())
                    .copied()
                    .map(|i| i as i32)
                    .unwrap_or(-1),
            );
        }
    }

    // Frame 0 is the bindpose clip: plain rest bends in the same shape as
    // every baked clip frame. The pose shader walks parents and applies the
    // undo itself, so storing anything already finished here double-applies
    // both. Bank slot 0 permanently points at this frame.
    let seed: Vec<Mat4> = binds.iter().map(|bind| bind.to_matrix()).collect();
    let fix = Mat4::from_cols(
        Vec4::new(-1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    );
    let mut rest_finals: Vec<Mat4> = Vec::with_capacity(bones.len());
    for (index, _) in bones.iter().enumerate() {
        let mut model = binds[index].to_matrix();
        let mut parent = parents[index];
        let mut guard = 0;
        while parent >= 0 && guard < 128 {
            model = binds[parent as usize].to_matrix() * model;
            parent = parents[parent as usize];
            guard += 1;
        }
        rest_finals.push(
            fix * model
                * reference.model_space_bindpose[bones[index]]
                    .to_matrix()
                    .inverse(),
        );
    }
    let mut joints_data = Vec::with_capacity(bones.len() * config.instances);
    for _ in 0..config.instances {
        joints_data.extend_from_slice(&rest_finals);
    }

    let num_bones = bones.len() as u32;
    let instances = config.instances as u32;
    let (blend_slots, max_clips, max_shapes) = config.normalized();
    let mut bank =
        GpuAnimationBank::new(bones, targets, binds.clone(), config.sample_rate, max_clips);
    bank.frames.extend_from_slice(&seed);
    let (offsets, counts, durations, modes) = bank.tables();

    let parents_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&parents),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let frames_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&seed),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let inv_bind: Vec<Mat4> = bank
        .bones
        .iter()
        .map(|bone| reference.model_space_bindpose[bone].to_matrix().inverse())
        .collect();
    let inv_bind_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&inv_bind),
        RenderAssetUsages::RENDER_WORLD,
    ));
    // Shape slices: slice 0 is the reference skeleton (translations mirror the
    // reference locals baked into the clip frames, so shape-0 instances render
    // exactly as before). `upload_shape_buffers` appends registered shapes.
    let reference_translations: Vec<Vec4> = binds
        .iter()
        .map(|bind| {
            Vec4::new(
                bind.translation.x,
                bind.translation.y,
                bind.translation.z,
                0.0,
            )
        })
        .collect();
    let shape_translations_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&reference_translations),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let shape_inv_binds_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&inv_bind),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let joints_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&joints_data),
        RenderAssetUsages::RENDER_WORLD,
    ));
    // Skeleton-root rearward Z shift (default rig only): published into the
    // pose uniforms so the GPU crowd sits on the capsule like CPU characters.
    let root_z_offset = skeleton_rear_offset_meters(&reference.rig_name);
    let uniforms_handle = buffers.add(ShaderBuffer::new(
        &pose_uniform_bytes(
            num_bones,
            instances,
            offsets.clone(),
            counts.clone(),
            durations.clone(),
            modes.clone(),
            1,
            root_z_offset,
        ),
        RenderAssetUsages::RENDER_WORLD,
    ));
    // Per instance: blend_slots weights + clocks + bank indices, padded to
    // the shader's fixed stride. Slots start at bank 0 with zero weight
    // until the caller wires them via `set_slot`.
    let mut instance_floats = Vec::with_capacity(config.instances * BLEND_CAP * 3);
    for _ in 0..config.instances {
        let mut seed = vec![0.0; BLEND_CAP];
        let take = blend_weights.0.len().min(blend_slots);
        seed[..take].copy_from_slice(&blend_weights.0[..take]);
        instance_floats.extend_from_slice(&seed);
        instance_floats.extend_from_slice(&[0.0; BLEND_CAP]);
        instance_floats.extend_from_slice(&[0.0; BLEND_CAP]);
    }
    let instance_data_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&instance_floats),
        RenderAssetUsages::RENDER_WORLD,
    ));
    // Per instance: one shape-ceiling weight row (zeros = reference).
    let shape_weight_floats = vec![0.0f32; config.instances * SHAPE_CAP];
    let shape_weights_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&shape_weight_floats),
        RenderAssetUsages::RENDER_WORLD,
    ));
    // Per instance: one root-bone Y scale (1.0 = reference proportions).
    let root_scale_floats = vec![1.0f32; config.instances];
    let root_scales_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&root_scale_floats),
        RenderAssetUsages::RENDER_WORLD,
    ));
    // Reference root bind-pose Y (local == model space: the root has no
    // parent). The shader measures clip-Y deltas from this.
    let root_bind_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&[binds[0].translation.y]),
        RenderAssetUsages::RENDER_WORLD,
    ));
    commands.insert_resource(GpuRenderHandles {
        parents: parents_handle,
        frames: frames_handle,
        inv_bind: inv_bind_handle,
        shape_translations: shape_translations_handle,
        shape_inv_binds: shape_inv_binds_handle,
        joints: joints_handle,
        uniforms: uniforms_handle,
        instance_data: instance_data_handle,
        shape_weights: shape_weights_handle,
        root_scales: root_scales_handle,
        root_bind: root_bind_handle,
        root_z_offset,
        num_bones,
        instance_count: instances,
        shape_count: 1,
        clip_offsets: offsets,
        clip_frames: counts,
        durations,
        modes,
    });
    let mut seed_weights = vec![0.0; blend_slots];
    seed_weights
        .iter_mut()
        .zip(blend_weights.0.iter())
        .for_each(|(dst, src)| *dst = *src);
    commands.insert_resource(GpuInstanceAnims {
        weights: vec![seed_weights.clone(); config.instances],
        targets: vec![seed_weights; config.instances],
        times: vec![vec![0.0; blend_slots]; config.instances],
        rates: vec![vec![1.0; blend_slots]; config.instances],
        slots: vec![vec![0; blend_slots]; config.instances],
        entries: vec![0.0; blend_slots],
        done: vec![vec![false; blend_slots]; config.instances],
        shape_weights: vec![vec![0.0; max_shapes]; config.instances],
        root_scales: vec![1.0; config.instances],
        blend_slots,
        max_clips,
        max_shapes,
    });
    commands.insert_resource(bank);
    commands.insert_resource(GpuAnimationReady);
    info!(
        "GPU base baked: {} bones, {} instances, skeleton LOD {} @ {:.1} Hz",
        num_bones, instances, skeleton_lod.0, config.sample_rate,
    );
}

/// Dispatches one background bake task per queued load whose clip asset is
/// available. Clones the clip so the task owns its samples with no ECS access.
pub(super) fn submit_clip_bakes(
    bank: Option<ResMut<GpuAnimationBank>>,
    jobs: Res<GpuBakeJobs>,
    retargeted: Res<Assets<RetargetedAnimationAsset>>,
    clips: Res<Assets<AnimationClip>>,
) {
    let Some(mut bank) = bank else {
        return;
    };
    let tx = jobs.sender.clone();
    let pool = AsyncComputeTaskPool::get();
    let mut i = 0;
    while i < bank.pending.len() {
        let req = bank.pending[i].clone();
        if req.name == BIND_POSE_CLIP {
            bank.pending.remove(i);
            continue;
        }
        let Some(map) = retargeted
            .iter()
            .find_map(|(_id, map)| map.clips.contains_key(req.name.as_str()).then_some(map))
        else {
            i += 1;
            continue;
        };
        let Some(handle) = map.clips.get(req.name.as_str()) else {
            warn!("GPU clip bake: '{}' not found, dropping request", req.name);
            bank.pending.remove(i);
            continue;
        };
        let Some(clip) = clips.get(handle) else {
            i += 1;
            continue;
        };
        let clip = clip.clone();
        let bones = bank.bones.clone();
        let targets = bank.targets.clone();
        let binds = bank.binds.clone();
        let rate = bank.sample_rate;
        bank.pending.remove(i);
        bank.baking.push(req.name.clone());
        let tx = tx.clone();
        pool.spawn(async move {
            let duration = clip.duration().max(0.01);
            let frames = sample_clip_frames(&clip, duration, &bones, &targets, &binds, rate);
            let _ = tx.send(BakedClip {
                name: req.name,
                mode: req.mode,
                duration,
                frames,
            });
        })
        .detach();
    }
}

/// Appends completed bakes to the packed `frames` shadow, splices unloads out
/// of it, then re-uploads the whole shadow — deferred while bake work is
/// still pending so a burst of clips sends once. Publishes the new slots
/// through the bank tables + handles + uniforms.
pub(super) fn collect_clip_bakes(
    bank: Option<ResMut<GpuAnimationBank>>,
    jobs: Res<GpuBakeJobs>,
    mut handles: Option<ResMut<GpuRenderHandles>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut anims: Option<ResMut<GpuInstanceAnims>>,
) {
    let Some(mut bank) = bank else {
        return;
    };
    // Stage 1: collect completed background bakes without touching slots,
    // shadow, or tables. Bank offsets must never lead the GPU buffer.
    let mut newly_staged = false;
    for baked in jobs.receiver.try_iter() {
        bank.baking.retain(|b| b != &baked.name);
        // Unloaded while baking, a duplicate, or the reserved bindpose name:
        // drop the bytes.
        if baked.name == BIND_POSE_CLIP
            || bank.slot_of(&baked.name).is_some()
            || bank.staged.iter().any(|s| s.name == baked.name)
        {
            continue;
        }
        bank.staged.push(baked);
        newly_staged = true;
    }
    let unload_requested = !bank.unload_queue.is_empty();
    // Stage 2: commit only at a quiet moment — no undispatched requests, no
    // in-flight bakes, and nothing newly arrived this pass (which implies a
    // bake just finished and siblings may follow). A burst of clips then
    // re-uploads exactly once.
    let work_pending = !bank.pending.is_empty()
        || !bank.baking.is_empty()
        || !jobs.receiver.is_empty()
        || newly_staged;
    if work_pending || (bank.staged.is_empty() && !unload_requested) {
        return;
    }
    let bones = bank.bones.len().max(1);
    let mut changed = false;
    for baked in std::mem::take(&mut bank.staged) {
        if baked.name == BIND_POSE_CLIP || bank.slot_of(&baked.name).is_some() {
            continue;
        }
        let Some(slot) = bank.free_slot() else {
            warn!(
                "GPU clip bake: bank full, dropping '{}' (unload a clip, then retry)",
                baked.name
            );
            continue;
        };
        let frame_count = (baked.frames.len() / bones) as u32;
        let offset = bank.frame_total;
        bank.frame_total += frame_count;
        bank.slots[slot] = Some((
            baked.name.clone(),
            BankSlot {
                offset_frames: offset,
                frame_count,
                duration: baked.duration,
                mode: baked.mode,
            },
        ));
        bank.frames.extend_from_slice(&baked.frames);
        changed = true;
        info!(
            "GPU clip '{}' ready: bank slot {slot}, {} frames at offset {offset}",
            baked.name, frame_count
        );
    }
    // Unloads: splice the clip's bytes out of the packed shadow and rewrite
    // later offsets, so the buffer stays packed with no dead bytes. Rebind
    // per-instance slots that pointed at the freed bank index to the bindpose
    // clip (weight decides). Slot 0 is the permanent bindpose clip and is
    // never spliced, so frame 0 never moves.
    for name in std::mem::take(&mut bank.unload_queue) {
        let Some(slot) = bank.slot_of(&name) else {
            continue;
        };
        if slot == BIND_POSE_SLOT {
            continue;
        }
        let Some((_, meta)) = bank.slots[slot].take() else {
            continue;
        };
        let start = meta.offset_frames as usize * bones;
        let len = meta.frame_count as usize * bones;
        bank.frames.drain(start..start + len);
        for entry in bank.slots.iter_mut().flatten() {
            if entry.1.offset_frames > meta.offset_frames {
                entry.1.offset_frames -= meta.frame_count;
            }
        }
        bank.frame_total -= meta.frame_count;
        changed = true;
        if let Some(anims) = anims.as_mut() {
            for slots in anims.slots.iter_mut() {
                for s in slots.iter_mut() {
                    if *s as usize == slot {
                        *s = BIND_POSE_SLOT as u32;
                    }
                }
            }
        }
        info!("GPU clip '{name}' unloaded: bank slot {slot} cleared, packed buffer rebuilt");
    }
    if !changed {
        return;
    }
    // Publish through the extracted mirrors + the uniforms buffer, so the
    // compute pipeline picks the rows up on its next bind-group rebuild.
    // The frames upload is the full shadow (seed + every clip): extraction
    // empties `data` on upload, so incremental read-extend-write would drop
    // the prefix.
    let (offsets, counts, durations, modes) = bank.tables();
    if let Some(handles) = handles.as_mut() {
        handles.clip_offsets = offsets.clone();
        handles.clip_frames = counts.clone();
        handles.durations = durations.clone();
        handles.modes = modes.clone();
        if let Some(mut frames) = buffers.get_mut(&handles.frames) {
            frames.data = Some(bytemuck::cast_slice(&bank.frames).to_vec());
        }
        if let Some(mut uniforms) = buffers.get_mut(&handles.uniforms) {
            uniforms.data = Some(
                pose_uniform_bytes(
                    handles.num_bones,
                    handles.instance_count,
                    offsets,
                    counts,
                    durations,
                    modes,
                    handles.shape_count,
                    handles.root_z_offset,
                )
                .to_vec(),
            );
        }
    }
}

/// Uploads registered [`GpuCrowdShapes`](super::config::GpuCrowdShapes) to the
/// shape buffers. Slice 0 is the reference skeleton rebuilt from the bank +
/// reference rig; slice `i + 1` holds `shapes[i]`. Rebuilds when the registry
/// version moves (register/re-fit), so population is a game-side `register`
/// call away. Mismatched lengths are rejected with a warning and the buffers
/// left alone.
///
/// Slice 0 is never read back from `buffer.data`: Bevy 0.19 `take_gpu_data`
/// empties `ShaderBuffer::data` on upload, so a read-back upload silently
/// never fires and every non-zero shape reads out of bounds.
pub(super) fn upload_shape_buffers(
    bank: Option<Res<GpuAnimationBank>>,
    shapes: Option<Res<super::config::GpuCrowdShapes>>,
    rig_data: Option<Res<RigData>>,
    mut handles: Option<ResMut<GpuRenderHandles>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut uploaded: Local<u64>,
) {
    let (Some(bank), Some(shapes), Some(rig_data), Some(handles)) =
        (bank, shapes, rig_data, handles.as_mut())
    else {
        return;
    };
    if shapes.version == *uploaded {
        return;
    }
    let num_bones = bank.bones.len();
    if num_bones == 0 {
        return;
    }
    for shape in shapes.shapes.iter() {
        if shape.translations.len() != num_bones || shape.inv_binds.len() != num_bones {
            warn!(
                "GPU shapes: '{}' has {}/{} bones, expected {num_bones}; skipping upload",
                shape.shape,
                shape.translations.len(),
                shape.inv_binds.len(),
            );
            return;
        }
    }
    let Some(rig) = rig_data.0.as_ref() else {
        return;
    };
    let reference = rig.reference_rig();
    let reference_translations: Vec<Vec4> = bank
        .binds
        .iter()
        .map(|bind| {
            Vec4::new(
                bind.translation.x,
                bind.translation.y,
                bind.translation.z,
                0.0,
            )
        })
        .collect();
    if reference_translations.len() != num_bones {
        return;
    }
    let reference_inv_binds: Vec<Mat4> = bank
        .bones
        .iter()
        .map(|bone| reference.model_space_bindpose[bone].to_matrix().inverse())
        .collect();
    let mut translations = Vec::with_capacity((shapes.shapes.len() + 1) * num_bones);
    let mut inv_binds = Vec::with_capacity((shapes.shapes.len() + 1) * num_bones);
    translations.extend_from_slice(&reference_translations);
    inv_binds.extend_from_slice(&reference_inv_binds);
    for shape in shapes.shapes.iter() {
        translations.extend_from_slice(&shape.translations);
        inv_binds.extend_from_slice(&shape.inv_binds);
    }
    if let Some(mut buffer) = buffers.get_mut(&handles.shape_translations) {
        buffer.data = Some(bytemuck::cast_slice(&translations).to_vec());
    }
    if let Some(mut buffer) = buffers.get_mut(&handles.shape_inv_binds) {
        buffer.data = Some(bytemuck::cast_slice(&inv_binds).to_vec());
    }
    handles.shape_count = shapes.shapes.len() as u32 + 1;
    // Publish the new shape count so the pose shader can clamp shape indices
    // while a register is still in flight.
    let (offsets, counts, durations, modes) = (
        handles.clip_offsets.clone(),
        handles.clip_frames.clone(),
        handles.durations.clone(),
        handles.modes.clone(),
    );
    if let Some(mut uniforms) = buffers.get_mut(&handles.uniforms) {
        uniforms.data = Some(
            pose_uniform_bytes(
                handles.num_bones,
                handles.instance_count,
                offsets,
                counts,
                durations,
                modes,
                handles.shape_count,
                handles.root_z_offset,
            )
            .to_vec(),
        );
    }
    *uploaded = shapes.version;
    info!(
        "GPU shapes uploaded: {} shape(s) + reference, {num_bones} bones each",
        shapes.shapes.len(),
    );
}

/// Packs the pose-shader uniforms: sizes, then the clip tables padded to the
/// shader ceilings, then the skeleton-root rearward Z shift (appended after
/// the tables so every existing uniform offset stays put).
fn pose_uniform_bytes(
    num_bones: u32,
    instances: u32,
    clip_offsets: Vec<u32>,
    clip_frames: Vec<u32>,
    durations: Vec<f32>,
    modes: Vec<u32>,
    shape_count: u32,
    root_z_offset: f32,
) -> [u8; 1088] {
    let mut words = [0u32; 272];
    words[0] = num_bones;
    words[1] = instances;
    words[2] = pose_grid_side(instances);
    words[3] = shape_count.max(1);
    // Words 4..260 are the clip tables (4 x CLIP_CAP); word 260 carries the
    // skeleton-root rearward Z shift, every later word stays reserved zero.
    words[260] = root_z_offset.to_bits();
    for i in 0..CLIP_CAP {
        words[4 + i] = clip_offsets[i];
        words[68 + i] = clip_frames[i];
        words[132 + i] = durations[i].to_bits();
        words[196 + i] = modes[i];
    }
    bytemuck::cast(words)
}
