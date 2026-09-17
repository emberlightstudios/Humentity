//! Per-instance animation state, owned by the CPU. The GPU only samples.
//!
//! [`GpuInstanceAnims`] holds live weights, blend targets, per-slot clocks,
//! playback rates, the blend-slot -> clip-bank wiring, and the per-instance
//! shape-weight rows for every crowd member. [`update_instance_clocks`]
//! advances clocks, eases weights toward targets, and uploads the packed
//! buffers the pose shader reads.
//!
//! Blend slots index the global [`GpuAnimationBank`](super::bank::GpuAnimationBank):
//! slot `s` of instance `i` plays bank clip `slots[i][s]`. Rewire with
//! [`set_slot`](GpuInstanceAnims::set_slot) once the clip reports loaded.
//! Shape weights mirror the mesh's [`MeshMorphWeights`](bevy::mesh::morph::MeshMorphWeights):
//! weight `i` blends registered shape `i` (`GpuCrowdShapes.shapes[i]`,
//! slice `i + 1` on the GPU). Drive with
//! [`set_shape_weights`](GpuInstanceAnims::set_shape_weights).

use bevy::{prelude::*, render::storage::ShaderBuffer};

use super::{
    bank::GpuRenderHandles,
    config::{MAX_BLEND_CLIPS, MAX_GPU_CLIPS, MAX_GPU_SHAPES},
};

/// Per-instance animation state. `weights` ease toward `targets` each frame;
/// `times` advance by `rates` while active. Rising edge (`0` -> `>0`) resets
/// `times[s]` to `entries[s]` so a clip restarts at its beginning instead of
/// inheriting phase. `slots` maps each blend slot to a bank clip index.
/// `shape_weights` blends the fitted shape skeletons per instance with the
/// same semantics as the mesh morph weights (weight `i` = shape `i` at full
/// strength; zeros = reference; see [`GpuCrowdShapes`](super::config::GpuCrowdShapes)).
/// `root_scales` holds the CPU `SkeletonRootBone.root_scale` factor per
/// instance so the pose shader applies the same root-Y correction as
/// `rescale_root_bone_translation`.
#[derive(Resource)]
pub struct GpuInstanceAnims {
    pub weights: Vec<[f32; MAX_BLEND_CLIPS]>,
    pub targets: Vec<[f32; MAX_BLEND_CLIPS]>,
    pub times: Vec<[f32; MAX_BLEND_CLIPS]>,
    pub rates: Vec<[f32; MAX_BLEND_CLIPS]>,
    pub slots: Vec<[u32; MAX_BLEND_CLIPS]>,
    pub entries: [f32; MAX_BLEND_CLIPS],
    pub done: Vec<[bool; MAX_BLEND_CLIPS]>,
    pub shape_weights: Vec<[f32; MAX_GPU_SHAPES]>,
    pub root_scales: Vec<f32>,
}

impl GpuInstanceAnims {
    pub fn set_target(&mut self, instance: usize, target: [f32; MAX_BLEND_CLIPS]) {
        if let Some(slot) = self.targets.get_mut(instance) {
            *slot = target;
        }
    }

    /// Points blend `slot` at bank clip `bank` and restarts its clock, so a
    /// freshly wired clip starts at its beginning. No-op on bad indices.
    pub fn set_slot(&mut self, instance: usize, slot: usize, bank: u32) {
        if slot >= MAX_BLEND_CLIPS || bank as usize >= MAX_GPU_CLIPS {
            return;
        }
        let Some(slots) = self.slots.get_mut(instance) else {
            return;
        };
        slots[slot] = bank;
        if let Some(times) = self.times.get_mut(instance) {
            times[slot] = self.entries[slot];
        }
        if let Some(done) = self.done.get_mut(instance) {
            done[slot] = false;
        }
    }

    pub fn trigger_one_shot(&mut self, instance: usize, slot: usize) {
        if instance >= self.times.len() || slot >= MAX_BLEND_CLIPS {
            return;
        }
        self.times[instance][slot] = self.entries[slot];
        self.done[instance][slot] = false;
        self.targets[instance][slot] = 1.0;
    }

    pub fn set_rate(&mut self, instance: usize, slot: usize, rate: f32) {
        if let Some(rates) = self.rates.get_mut(instance)
            && slot < MAX_BLEND_CLIPS
        {
            rates[slot] = rate;
        }
    }

    /// Sets the full shape-weight row for `instance`, mirroring the entity's
    /// morph weights. Truncates rows longer than [`MAX_GPU_SHAPES`]; shorter
    /// rows leave the remaining slots untouched. No-op on bad indices.
    pub fn set_shape_weights(&mut self, instance: usize, weights: &[f32]) {
        let Some(row) = self.shape_weights.get_mut(instance) else {
            return;
        };
        for (slot, w) in row.iter_mut().zip(weights.iter()) {
            *slot = *w;
        }
    }

    /// Sets one shape-weight slot for `instance`. No-op on bad indices.
    pub fn set_shape_weight(&mut self, instance: usize, slot: usize, weight: f32) {
        if slot >= MAX_GPU_SHAPES {
            return;
        }
        if let Some(row) = self.shape_weights.get_mut(instance) {
            row[slot] = weight;
        }
    }

    /// Sets the root-bone Y scale for `instance`, mirroring the CPU
    /// `SkeletonRootBone.root_scale`. No-op on bad indices.
    pub fn set_root_scale(&mut self, instance: usize, scale: f32) {
        if let Some(slot) = self.root_scales.get_mut(instance) {
            *slot = scale;
        }
    }
}

/// Fired once when a `OnceHold` clip reaches its duration. `slot` is the
/// blend slot, not the bank index.
#[derive(Message)]
pub struct GpuOneShotDone {
    pub instance: usize,
    pub slot: usize,
}

pub(super) fn update_instance_clocks(
    time: Res<Time>,
    handles: Option<Res<GpuRenderHandles>>,
    anims: Option<ResMut<GpuInstanceAnims>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut done_events: MessageWriter<GpuOneShotDone>,
) {
    let (Some(handles), Some(mut anims)) = (handles, anims) else {
        return;
    };
    let dt = time.delta_secs();
    let count = handles.instance_count as usize;
    for i in 0..count {
        for s in 0..MAX_BLEND_CLIPS {
            let was_active = anims.weights[i][s] > 1e-4;
            let will_be_active = anims.targets[i][s] > 1e-4;
            if !was_active && will_be_active && anims.weights[i][s] <= 1e-4 {
                anims.times[i][s] = anims.entries[s];
                anims.done[i][s] = false;
            }
            let blend_speed = 4.0 * dt;
            let current = anims.weights[i][s];
            let target = anims.targets[i][s];
            anims.weights[i][s] = if (target - current).abs() <= blend_speed {
                target
            } else {
                current + (target - current).signum() * blend_speed
            };
            if anims.weights[i][s] > 1e-4 {
                let bank = (anims.slots[i][s] as usize).min(MAX_GPU_CLIPS - 1);
                let duration = handles.durations[bank].max(0.01);
                if handles.modes[bank] == 1 {
                    if !anims.done[i][s] {
                        anims.times[i][s] += dt * anims.rates[i][s];
                        if anims.times[i][s] >= duration {
                            anims.times[i][s] = duration;
                            anims.done[i][s] = true;
                            done_events.write(GpuOneShotDone { instance: i, slot: s });
                        }
                    }
                } else {
                    anims.times[i][s] += dt * anims.rates[i][s];
                    if anims.times[i][s] >= duration || anims.times[i][s] < 0.0 {
                        anims.times[i][s] = anims.times[i][s].rem_euclid(duration);
                    }
                }
            }
        }
    }
    // Layout must match pose.wgsl: weights[4] + clocks[4] + bank indices[4].
    let mut flat = Vec::with_capacity(count * MAX_BLEND_CLIPS * 3);
    for i in 0..count {
        flat.extend_from_slice(&anims.weights[i]);
        flat.extend_from_slice(&anims.times[i]);
        flat.extend_from_slice(&anims.slots[i].map(|b| b as f32));
    }
    if let Some(mut buffer) = buffers.get_mut(&handles.instance_data) {
        buffer.data = Some(bytemuck::cast_slice(&flat).to_vec());
    }
    // Shape weights ride a separate buffer: one MAX_GPU_SHAPES row per
    // instance, same semantics as the entity's morph weights.
    let mut shape_flat = Vec::with_capacity(count * MAX_GPU_SHAPES);
    for i in 0..count {
        if let Some(row) = anims.shape_weights.get(i) {
            shape_flat.extend_from_slice(row);
        } else {
            shape_flat.extend_from_slice(&[0.0; MAX_GPU_SHAPES]);
        }
    }
    if let Some(mut buffer) = buffers.get_mut(&handles.shape_weights) {
        buffer.data = Some(bytemuck::cast_slice(&shape_flat).to_vec());
    }
    // Root scales ride their own buffer: one float per instance.
    let mut root_flat = Vec::with_capacity(count);
    for i in 0..count {
        root_flat.push(anims.root_scales.get(i).copied().unwrap_or(1.0));
    }
    if let Some(mut buffer) = buffers.get_mut(&handles.root_scales) {
        buffer.data = Some(bytemuck::cast_slice(&root_flat).to_vec());
    }
}
