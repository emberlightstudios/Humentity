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

/// Maximum number of template shapes blended per crowd instance in the pose
/// shader. Per-instance shape weights (`shape_weights` buffer) ride one row
/// of this width; the mesh's morph-target order and the registered
/// [`GpuCrowdShapes`] order must agree so weight `i` hits shape slice `i + 1`.
/// Must stay in sync with the hardcoded loop bound in `pose.wgsl`.
pub const MAX_GPU_SHAPES: usize = 8;


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

/// Pose compute workgroup tile. Must stay in sync with `@workgroup_size` in
/// `pose.wgsl`. Instances tile across X/Y, bones across Z, so no single
/// dispatch dimension grows with `instances * bones`.
pub const POSE_WORKGROUP_X: u32 = 8;
pub const POSE_WORKGROUP_Y: u32 = 8;
pub const POSE_WORKGROUP_Z: u32 = 4;

/// Side of the square instance grid the pose shader walks. Instance
/// `i` lives at `x = i % side`, `y = i / side`.
pub fn pose_grid_side(instances: u32) -> u32 {
    if instances == 0 {
        return 1;
    }
    (instances as f64).sqrt().ceil() as u32
}

/// One fitted skeleton for one template shape. The game computes these with
/// [`fit_shape_skeleton`](super::shapes::fit_shape_skeleton) and registers them
/// via [`GpuCrowdShapes::register`]; every crowd instance then blends them
/// with per-instance weights via
/// [`GpuInstanceAnims::set_shape_weights`](super::state::GpuInstanceAnims::set_shape_weights).
/// Weight slot `i` addresses `shapes[i]` (slice `i + 1` on the GPU; slice 0
/// is always the reference skeleton).
pub struct GpuShapeSkeleton {
    /// Shape name, matching the `CharacterTemplate` shape it was fitted from.
    pub shape: &'static str,
    /// Fitted local rest translations in LOD bone order, padded to `Vec4`
    /// (w unused). The pose shader swaps these in over the reference
    /// translations baked into the shared clip frames.
    pub translations: Vec<Vec4>,
    /// Fitted model-space inverse bindposes in LOD bone order (replaces the reference undo).
    pub inv_binds: Vec<Mat4>,
    /// Root-bone Y scale for this shape (`fitted_root_y / reference_root_y`,
    /// same factor the CPU `rescale_root_bone_translation` applies). The
    /// shader blends these by the instance shape weights.
    pub root_scale: f32,
}

/// Per-shape fitted skeletons for the GPU crowd.
///
/// Shape 0 is the reference skeleton (no fit needed). Register fitted shapes
/// before spawning: instances default to shape 0, which reads the pose
/// shader's shape-0 slice — the reference translations + `inv_bind` buffer.
/// Helpers, template lookups, and weight bookkeeping stay game-side; the
/// crate only owns buffers.
#[derive(Resource, Default)]
pub struct GpuCrowdShapes {
    /// Registered shapes in upload order. Index + 1 is the shape index
    /// instances select; index 0 is always the reference skeleton.
    pub shapes: Vec<GpuShapeSkeleton>,
    /// Bumped on every register; the upload system rebuilds GPU buffers when
    /// this moves.
    pub(crate) version: u64,
}
impl GpuCrowdShapes {
    /// Register a fitted shape skeleton. Returns its slice index (1-based;
    /// 0 is always the reference skeleton). The weight slot addressing it is
    /// the slice index minus one. Duplicate shape names replace the existing
    /// entry in place so re-fits don't grow the buffer. Registrations beyond
    /// [`MAX_GPU_SHAPES`] are rejected with a warning and read as reference.
    pub fn register(&mut self, shape: GpuShapeSkeleton) -> u32 {
        if let Some(i) = self.shapes.iter().position(|s| s.shape == shape.shape) {
            self.shapes[i] = shape;
            self.version += 1;
            return (i + 1) as u32;
        }
        if self.shapes.len() >= MAX_GPU_SHAPES {
            bevy::log::warn!(
                "GPU shapes: '{}' rejected, already holding MAX_GPU_SHAPES ({MAX_GPU_SHAPES})",
                shape.shape,
            );
            return 0;
        }
        self.shapes.push(shape);
        self.version += 1;
        self.shapes.len() as u32
    }

    /// Weight slot for `shape`, or `None` when unregistered. Slot `i`
    /// addresses `shapes[i]` (slice `i + 1` on the GPU).
    pub fn slot_of(&self, shape: &str) -> Option<usize> {
        self.shapes.iter().position(|s| s.shape == shape)
    }

    /// Number of non-reference shapes registered.
    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    /// True when no fitted shapes are registered.
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }
}
