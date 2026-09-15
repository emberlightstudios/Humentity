//! Per-instance animation state, owned by the CPU. The GPU only samples.
//!
//! [`GpuInstanceAnims`] holds live weights, blend targets, per-clip clocks,
//! and playback rates for every crowd member. [`update_instance_clocks`]
//! advances clocks, eases weights toward targets, and uploads the packed
//! buffer the pose shader reads.

use bevy::{prelude::*, render::storage::ShaderBuffer};

use super::{bank::GpuRenderHandles, config::MAX_BLEND_CLIPS};

/// Per-instance animation state. `weights` ease toward `targets` each frame;
/// `times` advance by `rates` while active. Rising edge (`0` -> `>0`) resets
/// `times[c]` to `entries[c]` so idle restarts at its beginning instead of
/// inheriting walk phase.
#[derive(Resource)]
pub struct GpuInstanceAnims {
    pub weights: Vec<[f32; MAX_BLEND_CLIPS]>,
    pub targets: Vec<[f32; MAX_BLEND_CLIPS]>,
    pub times: Vec<[f32; MAX_BLEND_CLIPS]>,
    pub rates: Vec<[f32; MAX_BLEND_CLIPS]>,
    pub entries: [f32; MAX_BLEND_CLIPS],
    pub done: Vec<[bool; MAX_BLEND_CLIPS]>,
}

impl GpuInstanceAnims {
    pub fn set_target(&mut self, instance: usize, target: [f32; MAX_BLEND_CLIPS]) {
        if let Some(slot) = self.targets.get_mut(instance) {
            *slot = target;
        }
    }

    pub fn trigger_one_shot(&mut self, instance: usize, clip: usize) {
        if instance >= self.times.len() || clip >= MAX_BLEND_CLIPS {
            return;
        }
        self.times[instance][clip] = self.entries[clip];
        self.done[instance][clip] = false;
        self.targets[instance][clip] = 1.0;
    }

    pub fn set_rate(&mut self, instance: usize, clip: usize, rate: f32) {
        if let Some(rates) = self.rates.get_mut(instance)
            && clip < MAX_BLEND_CLIPS
        {
            rates[clip] = rate;
        }
    }
}

/// Fired once when a `OnceHold` clip reaches its duration.
#[derive(Message)]
pub struct GpuOneShotDone {
    pub instance: usize,
    pub clip: usize,
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
    let clip_count = handles.clip_count as usize;
    for i in 0..count {
        for c in 0..clip_count {
            let was_active = anims.weights[i][c] > 1e-4;
            let will_be_active = anims.targets[i][c] > 1e-4;
            if !was_active && will_be_active && anims.weights[i][c] <= 1e-4 {
                anims.times[i][c] = anims.entries[c];
                anims.done[i][c] = false;
            }
            let blend_speed = 4.0 * dt;
            let current = anims.weights[i][c];
            let target = anims.targets[i][c];
            anims.weights[i][c] = if (target - current).abs() <= blend_speed {
                target
            } else {
                current + (target - current).signum() * blend_speed
            };
            if anims.weights[i][c] > 1e-4 {
                let duration = handles.durations[c].max(0.01);
                if handles.modes[c] == 1 {
                    if !anims.done[i][c] {
                        anims.times[i][c] += dt * anims.rates[i][c];
                        if anims.times[i][c] >= duration {
                            anims.times[i][c] = duration;
                            anims.done[i][c] = true;
                            done_events.write(GpuOneShotDone {
                                instance: i,
                                clip: c,
                            });
                        }
                    }
                } else {
                    anims.times[i][c] += dt * anims.rates[i][c];
                    if anims.times[i][c] >= duration || anims.times[i][c] < 0.0 {
                        anims.times[i][c] = anims.times[i][c].rem_euclid(duration);
                    }
                }
            }
        }
    }
    let mut flat = Vec::with_capacity(count * MAX_BLEND_CLIPS * 2);
    for i in 0..count {
        flat.extend_from_slice(&anims.weights[i]);
        flat.extend_from_slice(&anims.times[i]);
    }
    if let Some(mut buffer) = buffers.get_mut(&handles.instance_data) {
        buffer.data = Some(bytemuck::cast_slice(&flat).to_vec());
    }
}
