//! GPU crowd configuration: instance counts, clip selection, blend-weight
//! templates, playback modes, and skeleton LOD selection.

use bevy::prelude::*;

#[derive(Resource, Clone, Copy)]
pub struct GpuCrowdConfig {
    pub instances: usize,
    pub frames: usize,
}

impl Default for GpuCrowdConfig {
    fn default() -> Self {
        Self {
            instances: 1000,
            frames: 64,
        }
    }
}

/// Which skeleton LOD the bake poses. Must match the `skeleton_lod` the crowd
/// meshes were built with so joint indices line up. `0` is the full skeleton;
/// higher LODs pose fewer bones.
#[derive(Resource, Clone, Copy, Default)]
pub struct GpuSkeletonLod(pub usize);

/// Maximum number of clips blended in the pose shader.
pub const MAX_BLEND_CLIPS: usize = 4;

/// Which animation clips to bake (up to [`MAX_BLEND_CLIPS`]), resolved by name
/// against the loaded [`RetargetedAnimationAsset`](crate::prelude::RetargetedAnimationAsset).
/// Defaults to the idle loop so the crowd example needs no configuration.
#[derive(Resource, Clone)]
pub struct GpuBlendClips {
    pub names: Vec<String>,
}

impl Default for GpuBlendClips {
    fn default() -> Self {
        Self {
            names: vec!["Idle-loop".to_string()],
        }
    }
}

/// Per-clip blend weights template used once at bake to seed every instance.
/// Runtime weights are per-instance in [`GpuInstanceAnims`](super::state::GpuInstanceAnims);
/// drive those to crossfade.
#[derive(Resource, Clone, Copy)]
pub struct GpuBlendWeights(pub [f32; MAX_BLEND_CLIPS]);

impl Default for GpuBlendWeights {
    fn default() -> Self {
        Self([1.0, 0.0, 0.0, 0.0])
    }
}

/// Loop vs one-shot behaviour per baked clip slot.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuClipMode {
    #[default]
    Loop,
    OnceHold,
}

/// Static per-clip modes, resolved by baked slot order.
#[derive(Resource, Clone, Copy)]
pub struct GpuClipModes(pub [GpuClipMode; MAX_BLEND_CLIPS]);

impl Default for GpuClipModes {
    fn default() -> Self {
        Self([GpuClipMode::Loop; MAX_BLEND_CLIPS])
    }
}
