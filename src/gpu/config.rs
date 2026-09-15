//! GPU crowd configuration: instance counts, blend-weight templates,
//! playback modes, and skeleton LOD selection.

use bevy::prelude::*;

#[derive(Resource, Clone, Copy)]
pub struct GpuCrowdConfig {
    pub instances: usize,
    /// Fixed sampling rate in frames per second. Each clip bakes to
    /// `ceil(duration * sample_rate)` frames, so short clips stay small and
    /// long clips keep time resolution.
    pub sample_rate: f32,
}

impl Default for GpuCrowdConfig {
    fn default() -> Self {
        Self {
            instances: 1000,
            sample_rate: 30.0,
        }
    }
}

/// Which skeleton variant the bake poses. Must match the `skeleton_lod` the crowd
/// meshes were built with so joint indices line up. `0` selects the first
/// [`SkeletonLodConfig`](crate::prelude::SkeletonLodConfig) entry.
#[derive(Resource, Clone, Copy, Default)]
pub struct GpuSkeletonLod(pub usize);

/// Maximum number of clips blended per instance in the pose shader.
pub const MAX_BLEND_CLIPS: usize = 4;

/// Maximum number of clips resident in the global GPU clip bank.
/// Per-instance blend slots (`MAX_BLEND_CLIPS`) index into this bank, so the
/// crowd can pick any 4 of up to 64 loaded clips. Must stay in sync with the
/// hardcoded table sizes in `pose.wgsl`.
pub const MAX_GPU_CLIPS: usize = 64;

/// Clip loads/unloads are manual via
/// [`GpuAnimationBank`](super::bank::GpuAnimationBank) `request_load` /
/// `request_unload`.
/// Per-instance blend weights template used once at bake to seed every instance.
/// Runtime weights are per-instance in [`GpuInstanceAnims`](super::state::GpuInstanceAnims);
/// drive those to crossfade.
#[derive(Resource, Clone, Copy)]
pub struct GpuBlendWeights(pub [f32; MAX_BLEND_CLIPS]);

impl Default for GpuBlendWeights {
    fn default() -> Self {
        Self([1.0, 0.0, 0.0, 0.0])
    }
}

/// Loop vs one-shot behaviour per bank clip.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuClipMode {
    #[default]
    Loop,
    OnceHold,
}

