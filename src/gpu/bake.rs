//! One-time bake: samples the selected clips onto the LOD skeleton and
//! uploads the parent table, local-frame table, inverse bindposes, seed pose,
//! uniforms, and per-instance state.

use ahash::AHashMap;
use bevy::{
    animation::{animated_field, AnimationClip, AnimationTargetId},
    asset::RenderAssetUsages,
    pbr::ExtendedMaterial,
    prelude::*,
    render::storage::ShaderBuffer,
};

use crate::{prelude::*, rigs::RigBundleRes};

use super::{
    bank::{GpuAnimationBank, GpuAnimationHandles, GpuAnimationReady, GpuRenderHandles},
    config::{
        GpuBlendClips, GpuBlendWeights, GpuClipMode, GpuClipModes, GpuCrowdConfig, GpuSkeletonLod,
        MAX_BLEND_CLIPS,
    },
    material::{CrowdMaterial, GpuCrowdExtension, GpuCrowdUniform},
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

pub(super) fn bake_gpu_animation(
    mut commands: Commands,
    config: Res<GpuCrowdConfig>,
    blend_clips: Res<GpuBlendClips>,
    blend_weights: Res<GpuBlendWeights>,
    clip_modes: Res<GpuClipModes>,
    skeleton_lod: Res<GpuSkeletonLod>,
    rig_data: Res<RigData>,
    rig_bundle: Res<RigBundleRes>,
    retargeted: Res<Assets<RetargetedAnimationAsset>>,
    clips: Res<Assets<AnimationClip>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<CrowdMaterial>>,
    mut done: Local<bool>,
    mut warned: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(rig) = rig_data.0.as_ref() else {
        return;
    };
    let wanted: Vec<&str> = blend_clips
        .names
        .iter()
        .take(MAX_BLEND_CLIPS)
        .map(String::as_str)
        .collect();
    if wanted.is_empty() {
        return;
    }
    let Some(map) = retargeted
        .iter()
        .find_map(|(_id, map)| map.clips.contains_key(wanted[0]).then_some(map))
    else {
        return;
    };
    let mut resolved: Vec<(&str, AnimationClip, f32)> = Vec::new();
    for name in wanted {
        let Some(handle) = map.clips.get(name) else {
            if !*warned {
                *warned = true;
                warn!("GPU animation bake: clip '{name}' not found, skipping");
            }
            continue;
        };
        let Some(clip) = clips.get(handle) else {
            return;
        };
        resolved.push((name, clip.clone(), clip.duration().max(0.01)));
    }
    if resolved.is_empty() {
        return;
    }

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
    *done = true;

    let num_bones = bones.len() as u32;
    let num_frames = config.frames as u32;
    let instances = config.instances as u32;
    let num_clips = resolved.len() as u32;
    let mut durations = [0.0f32; MAX_BLEND_CLIPS];
    for (index, (_, _, duration)) in resolved.iter().enumerate() {
        durations[index] = *duration;
    }

    let targets: Vec<AnimationTargetId> = bones
        .iter()
        .map(|bone| AnimationTargetId::from_names(joint_path(bone, rig).iter()))
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

    let mut local_frames = Vec::with_capacity(bones.len() * config.frames * resolved.len());
    for (_, clip, duration) in &resolved {
        for frame in 0..config.frames {
            let time = frame as f32 / config.frames as f32 * *duration;
            for (index, bone) in bones.iter().enumerate() {
                let bind = reference.local_bindpose[bone];
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
                local_frames.push(
                    Transform {
                        translation,
                        rotation,
                        scale,
                    }
                    .to_matrix(),
                );
            }
        }
    }

    let inv_bind: Vec<Mat4> = bones
        .iter()
        .map(|bone| reference.model_space_bindpose[bone].to_matrix().inverse())
        .collect();

    // TEMP DEBUG: limb-bone rotation variation across the loop. Constant
    // values mean clip sampling missed and fell back to bindpose.
    for f in [0, 16, 32, 48] {
        let m = local_frames[f * bones.len() + 4];
        let (sx, sy, sz) = (m.x_axis.length(), m.y_axis.length(), m.z_axis.length());
        info!(
            "bake clip0 frame {f}: lowerleg01.L row0 ({:.3}, {:.3}, {:.3}) scale ({:.3}, {:.3}, {:.3})",
            m.x_axis.x, m.x_axis.y, m.x_axis.z, sx, sy, sz
        );
    }

    let weight_sum: f32 = blend_weights.0.iter().sum::<f32>().max(1e-5);
    let blended_frame0 = |bone: usize| {
        let mut local = Mat4::ZERO;
        for c in 0..resolved.len() {
            local += local_frames[c * bones.len() + bone] * (blend_weights.0[c] / weight_sum);
        }
        local
    };
    let mut first_pose = vec![Mat4::IDENTITY; bones.len()];
    for (bone_index, matrix) in first_pose.iter_mut().enumerate() {
        let mut model = blended_frame0(bone_index);
        let mut parent = parents[bone_index];
        let mut guard = 0;
        while parent >= 0 && guard < 128 {
            model = blended_frame0(parent as usize) * model;
            parent = parents[parent as usize];
            guard += 1;
        }
        *matrix = model * inv_bind[bone_index];
    }
    let mut joints_data = Vec::with_capacity(bones.len() * config.instances);
    for _ in 0..config.instances {
        joints_data.extend_from_slice(&first_pose);
    }

    let modes = [
        mode_flag(clip_modes.0[0]),
        mode_flag(clip_modes.0[1]),
        mode_flag(clip_modes.0[2]),
        mode_flag(clip_modes.0[3]),
    ];
    let mut instance_floats = Vec::with_capacity(config.instances * MAX_BLEND_CLIPS * 2);
    for i in 0..config.instances {
        for c in 0..MAX_BLEND_CLIPS {
            instance_floats.push(if (c as u32) < num_clips {
                blend_weights.0[c]
            } else {
                0.0
            });
        }
        for (c, duration) in durations.iter().enumerate() {
            let duration = duration.max(0.01);
            let frac = ((i as f32 * 0.618_034) + (c as f32 * 0.381_966)) % 1.0;
            instance_floats.push(frac * duration);
        }
    }

    let parents_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&parents),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let frames_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&local_frames),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let inv_bind_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&inv_bind),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let joints_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&joints_data),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let uniforms_handle = buffers.add(ShaderBuffer::new(
        &pose_uniform_bytes(
            num_bones, num_frames, instances, durations, modes, num_clips,
        ),
        RenderAssetUsages::RENDER_WORLD,
    ));
    let instance_data_handle = buffers.add(ShaderBuffer::new(
        bytemuck::cast_slice(&instance_floats),
        RenderAssetUsages::RENDER_WORLD,
    ));

    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial::from_color(Color::WHITE),
        extension: GpuCrowdExtension {
            joints: joints_handle.clone(),
            crowd: GpuCrowdUniform {
                num_bones,
                num_instances: instances,
                pad0: 0,
                pad1: 0,
            },
        },
    });

    commands.insert_resource(GpuAnimationBank {
        num_bones: bones.len(),
        num_frames: config.frames,
        duration: durations[0],
        clip_names: resolved
            .iter()
            .map(|(name, _, _)| name.to_string())
            .collect(),
    });
    commands.insert_resource(GpuRenderHandles {
        parents: parents_handle,
        frames: frames_handle,
        inv_bind: inv_bind_handle,
        joints: joints_handle,
        uniforms: uniforms_handle,
        instance_data: instance_data_handle,
        num_bones,
        num_frames,
        instance_count: instances,
        clip_count: num_clips,
        durations,
        modes,
    });
    commands.insert_resource(GpuInstanceAnims {
        weights: vec![blend_weights.0; config.instances],
        targets: vec![blend_weights.0; config.instances],
        times: (0..config.instances)
            .map(|i| {
                let mut t = [0.0; MAX_BLEND_CLIPS];
                for (c, duration) in durations.iter().enumerate() {
                    let frac = ((i as f32 * 0.618_034) + (c as f32 * 0.381_966)) % 1.0;
                    t[c] = frac * duration.max(0.01);
                }
                t
            })
            .collect(),
        rates: vec![[1.0; MAX_BLEND_CLIPS]; config.instances],
        entries: [0.0; MAX_BLEND_CLIPS],
        done: vec![[false; MAX_BLEND_CLIPS]; config.instances],
    });
    commands.insert_resource(GpuAnimationHandles { material });
    commands.insert_resource(GpuAnimationReady);
    info!(
        "GPU animation baked: {} bones x {} frames x {} clips [{}] durations {:?}, {} instances ({} joint matrices), skeleton LOD {}",
        num_bones,
        num_frames,
        num_clips,
        resolved
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
            .join(", "),
        durations,
        instances,
        num_bones * instances,
        skeleton_lod.0,
    );
}

const fn mode_flag(mode: GpuClipMode) -> u32 {
    match mode {
        GpuClipMode::Loop => 0,
        GpuClipMode::OnceHold => 1,
    }
}

fn pose_uniform_bytes(
    num_bones: u32,
    num_frames: u32,
    instances: u32,
    durations: [f32; MAX_BLEND_CLIPS],
    modes: [u32; MAX_BLEND_CLIPS],
    clip_count: u32,
) -> [u8; 48] {
    let mut words = [0u32; 12];
    words[0] = num_bones;
    words[1] = num_frames;
    words[2] = instances;
    words[3] = clip_count;
    for i in 0..MAX_BLEND_CLIPS {
        words[4 + i] = durations[i].to_bits();
        words[8 + i] = modes[i];
    }
    bytemuck::cast(words)
}
