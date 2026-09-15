//! Base bake + background clip bakes for the GPU crowd.
//!
//! The first pass samples nothing: it snapshots the LOD skeleton (bones,
//! animation targets, bindposes), uploads the static buffers (`parents`,
//! `inv_bind`, a frame-0 bindpose seed, `joints`, `uniforms`,
//! `instance_data`), creates the [`GpuAnimationBank`] registry, and queues
//! the [`GpuBlendClips`] names as manual loads. Follow-up passes dispatch one
//! background [`AsyncComputeTaskPool`] task per queued load; completions are
//! collected on the main thread, appended to the shared `frames` buffer, and
//! published through the bank tables + [`GpuRenderHandles`]. Unloads clear the
//! bank slot and the uniform row (neighbours never move).

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
    config::{
        GpuBlendClips, GpuBlendWeights, GpuClipModes, GpuCrowdConfig, GpuSkeletonLod,
        MAX_BLEND_CLIPS, MAX_GPU_CLIPS,
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
        let time = frame as f32 / frames as f32 * duration;
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

/// First pass: uploads static buffers + frame-0 bindpose seed, creates the
/// bank, seeds per-instance state, and queues the initial clip names.
pub(super) fn bake_gpu_animation(
    mut commands: Commands,
    config: Res<GpuCrowdConfig>,
    blend_clips: Res<GpuBlendClips>,
    blend_weights: Res<GpuBlendWeights>,
    clip_modes: Res<GpuClipModes>,
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
    let bones: Vec<&'static str> = if skeleton_lod.0 == 0 {
        reference.bone_names.clone()
    } else {
        let Some(bundle) = rig_bundle.bundle.as_ref() else {
            return;
        };
        let Some(data) = bundle.lod_data.get(skeleton_lod.0) else {
            return;
        };
        data.bone_names.clone()
    };
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

    // TEMP DEBUG: verify the LOD parent table.
    info!("gpu bake parents: {:?}", parents);

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
    for (index, name) in blend_clips.names.iter().take(MAX_GPU_CLIPS).enumerate() {
        bank.request_load(
            name.clone(),
            clip_modes.0.get(index).copied().unwrap_or_default(),
        );
    }
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
    mut jobs: ResMut<GpuBakeJobs>,
    retargeted: Res<Assets<RetargetedAnimationAsset>>,
    clips: Res<Assets<AnimationClip>>,
) {
    let Some(mut bank) = bank else {
        return;
    };
    jobs.ensure_channels();
    let tx = jobs.sender.as_ref().unwrap().clone();
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

/// Appends completed bakes to the shared `frames` buffer, publishes the new
/// slot through the bank tables + handles + uniforms, then processes the
/// unload queue (bank slot + uniform row cleared, bytes left for future
/// compaction).
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
    let Some(rx) = jobs.receiver.as_ref() else {
        return;
    };
    let mut appended = false;
    for baked in rx.try_iter() {
        bank.baking.retain(|b| b != &baked.name);
        // Unloaded while baking, or a duplicate: drop the bytes.
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
        let bones = bank.bones.len().max(1);
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
        let Some(handles_ref) = handles.as_ref() else {
            continue;
        };
        let Some(mut frames) = buffers.get_mut(&handles_ref.frames) else {
            continue;
        };
        let mut bytes = frames.data.clone().unwrap_or_default();
        bytes.extend_from_slice(bytemuck::cast_slice(&baked.frames));
        frames.data = Some(bytes);
        appended = true;
        // TEMP DEBUG: limb-bone variation on the freshly appended clip.
        // Constant values mean sampling missed and fell back to bindpose.
        if bank.bones.len() > 4 {
            let nf = frame_count.max(1);
            for f in [0, nf / 4, nf / 2, nf * 3 / 4] {
                let m = baked.frames[f as usize * bank.bones.len() + 4];
                let (sx, sy, sz) = (m.x_axis.length(), m.y_axis.length(), m.z_axis.length());
                info!(
                    "bake '{}' frame {f}: lowerleg01.L row0 ({:.3}, {:.3}, {:.3}) scale ({:.3}, {:.3}, {:.3})",
                    baked.name, m.x_axis.x, m.x_axis.y, m.x_axis.z, sx, sy, sz
                );
            }
        }
        info!(
            "GPU clip '{}' ready: bank slot {slot}, {} frames at offset {offset}",
            baked.name, frame_count
        );
    }
    // Unloads: clear bank slot + uniform row. Buffer bytes stay (compaction
    // later), so neighbours are unaffected. Rebind per-instance slots that
    // pointed at the freed bank index to 0 (bindpose seed; weight decides).
    let mut unloaded = false;
    for name in std::mem::take(&mut bank.unload_queue) {
        let Some(slot) = bank.slot_of(&name) else {
            continue;
        };
        bank.slots[slot] = None;
        unloaded = true;
        if let Some(anims) = anims.as_mut() {
            for slots in anims.slots.iter_mut() {
                for s in slots.iter_mut() {
                    if *s as usize == slot {
                        *s = 0;
                    }
                }
            }
        }
        info!("GPU clip '{name}' unloaded: bank slot {slot} cleared");
    }
    if !(appended || unloaded) {
        return;
    }
    // Publish through the extracted mirrors + the uniforms buffer, so the
    // compute pipeline picks the rows up on its next bind-group rebuild.
    let (offsets, counts, durations, modes) = bank.tables();
    if let Some(handles) = handles.as_mut() {
        handles.clip_offsets = offsets;
        handles.clip_frames = counts;
        handles.durations = durations;
        handles.modes = modes;
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
