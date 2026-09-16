//! Base bake + background clip bakes for the GPU crowd.
//!
//! The first pass samples nothing: it snapshots the LOD skeleton (bones,
//! animation targets, bindposes), uploads the static buffers (`parents`,
//! `inv_bind`, a frame-0 bindpose seed, `joints`, `uniforms`,
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
    animation::{animated_field, AnimationClip, AnimationTargetId},
    asset::RenderAssetUsages,
    prelude::*,
    render::storage::ShaderBuffer,
    tasks::AsyncComputeTaskPool,
};

use crate::{prelude::*, rigs::RigBundleRes};

use super::{
    bank::{BakedClip, BankSlot, GpuAnimationBank, GpuAnimationReady, GpuBakeJobs, GpuRenderHandles},
    config::{GpuBlendWeights, GpuCrowdConfig, GpuSkeletonLod, MAX_BLEND_CLIPS, MAX_GPU_CLIPS},
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

    // Frame 0 is the bindpose seed so cleared uniform rows read a valid pose.
    let mut seed: Vec<Mat4> = Vec::with_capacity(bones.len());
    for (index, _) in bones.iter().enumerate() {
        let mut model = binds[index].to_matrix();
        let mut parent = parents[index];
        let mut guard = 0;
        while parent >= 0 && guard < 128 {
            model = binds[parent as usize].to_matrix() * model;
            parent = parents[parent as usize];
            guard += 1;
        }
        seed.push(model * reference.model_space_bindpose[bones[index]].to_matrix().inverse());
    }
    let mut joints_data = Vec::with_capacity(bones.len() * config.instances);
    for _ in 0..config.instances {
        joints_data.extend_from_slice(&seed);
    }

    let num_bones = bones.len() as u32;
    let instances = config.instances as u32;
    let mut bank = GpuAnimationBank::new(bones, targets, binds, config.sample_rate);
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
    let joints_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&joints_data),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let uniforms_handle = buffers.add(ShaderBuffer::new(
        &pose_uniform_bytes(num_bones, instances, offsets, counts, durations, modes),
        RenderAssetUsages::RENDER_WORLD,
    ));
    // Per instance: weights[4] + clocks[4] + bank indices[4]. Slots start at
    // bank 0 with zero weight until the caller wires them via `set_slot`.
    let mut instance_floats = Vec::with_capacity(config.instances * MAX_BLEND_CLIPS * 3);
    for _ in 0..config.instances {
        instance_floats.extend_from_slice(&blend_weights.0);
        instance_floats.extend_from_slice(&[0.0; MAX_BLEND_CLIPS]);
        instance_floats.extend_from_slice(&[0.0; MAX_BLEND_CLIPS]);
    }
    let instance_data_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&instance_floats),
        RenderAssetUsages::RENDER_WORLD,
    ));

    commands.insert_resource(GpuRenderHandles {
        parents: parents_handle,
        frames: frames_handle,
        inv_bind: inv_bind_handle,
        joints: joints_handle,
        uniforms: uniforms_handle,
        instance_data: instance_data_handle,
        num_bones,
        instance_count: instances,
        clip_offsets: offsets,
        clip_frames: counts,
        durations,
        modes,
    });
    commands.insert_resource(GpuInstanceAnims {
        weights: vec![blend_weights.0; config.instances],
        targets: vec![blend_weights.0; config.instances],
        times: vec![[0.0; MAX_BLEND_CLIPS]; config.instances],
        rates: vec![[1.0; MAX_BLEND_CLIPS]; config.instances],
        slots: vec![[0; MAX_BLEND_CLIPS]; config.instances],
        entries: [0.0; MAX_BLEND_CLIPS],
        done: vec![[false; MAX_BLEND_CLIPS]; config.instances],
    });
    commands.insert_resource(bank);
    commands.insert_resource(GpuAnimationReady);
    info!(
        "GPU base baked: {} bones, {} instances, skeleton LOD {} @ {:.1} Hz",
        num_bones,
        instances,
        skeleton_lod.0,
        config.sample_rate,
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
        // Unloaded while baking, or a duplicate: drop the bytes.
        if bank.slot_of(&baked.name).is_some()
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
        if bank.slot_of(&baked.name).is_some() {
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
    // per-instance slots that pointed at the freed bank index to 0 (bindpose
    // seed; weight decides).
    for name in std::mem::take(&mut bank.unload_queue) {
        let Some(slot) = bank.slot_of(&name) else {
            continue;
        };
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
                        *s = 0;
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
        handles.clip_offsets = offsets;
        handles.clip_frames = counts;
        handles.durations = durations;
        handles.modes = modes;
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
                )
                .to_vec(),
            );
        }
    }
}

fn pose_uniform_bytes(
    num_bones: u32,
    instances: u32,
    clip_offsets: [u32; MAX_GPU_CLIPS],
    clip_frames: [u32; MAX_GPU_CLIPS],
    durations: [f32; MAX_GPU_CLIPS],
    modes: [u32; MAX_GPU_CLIPS],
) -> [u8; 1088] {
    let mut words = [0u32; 272];
    words[0] = num_bones;
    words[1] = instances;
    words[2] = 0;
    words[3] = 0;
    for i in 0..MAX_GPU_CLIPS {
        words[4 + i] = clip_offsets[i];
        words[68 + i] = clip_frames[i];
        words[132 + i] = durations[i].to_bits();
        words[196 + i] = modes[i];
    }
    bytemuck::cast(words)
}
