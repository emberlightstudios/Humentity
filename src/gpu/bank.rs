//! GPU clip bank: manual load/unload registry for baked animation clips.
//!
//! The bank owns the global clip registry (up to [`MAX_GPU_CLIPS`] clips).
//! Per-instance blend slots ([`MAX_BLEND_CLIPS`]) index into the bank, so the
//! crowd can blend any 4 of the loaded clips. Loads are manual via
//! [`GpuAnimationBank::request_load`]: each request spawns one background bake
//! task, and the baked frames are appended to the packed `frames` shadow when
//! ready. Unloads ([`GpuAnimationBank::request_unload`]) splice the clip's
//! bytes out of the shadow and rewrite later offsets, so the buffer stays
//! packed with no dead bytes. Every mutation re-uploads the whole shadow;
//! uploads are deferred while bake work is still pending so a burst of clips
//! sends once.
//!
//! Bank slot 0 permanently holds the bindpose clip (frame 0, plain rest
//! bends): empty rows read offset 0, so an unloaded slot falls back to
//! rest. Callers must still drive the slot weight to 0 so it contributes
//! nothing.

use bevy::{
    animation::AnimationTargetId,
    prelude::*,
    render::{extract_resource::ExtractResource, storage::ShaderBuffer},
};
use crossbeam_channel::{Receiver, Sender};

use super::config::{GpuClipMode, MAX_GPU_CLIPS};

/// Bank slot permanently holding the bindpose fallback clip. Frame 0 of the
/// shared frames buffer is the bindpose in plain rest-bend shape (the same
/// shape as every baked clip frame); this entry points at it, so empty blend
/// slots (bank 0) and cleared table entries (offset 0) read rest. Never
/// handed out to real clips, never unloaded.
pub const BIND_POSE_SLOT: usize = 0;
/// Reserved clip name for the bindpose fallback in [`BIND_POSE_SLOT`].
pub const BIND_POSE_CLIP: &str = "__bindpose__";

/// Metadata for one resident bank slot. The clip's bytes live at
/// `offset_frames * num_bones` in the packed `frames` shadow; unloads splice
/// bytes out and rewrite later offsets, so offsets are only stable while no
/// load/unload is in flight.
pub(crate) struct BankSlot {
    pub offset_frames: u32,
    pub frame_count: u32,
    pub duration: f32,
    pub mode: GpuClipMode,
}
#[derive(Clone)]
pub(crate) struct PendingGpuAnimationClip {
    pub(crate) name: String,
    pub(crate) mode: GpuClipMode,
}

/// Baked local-frame matrices for one clip, sent from the background task.
pub(crate) struct BakedClip {
    pub name: String,
    pub mode: GpuClipMode,
    pub duration: f32,
    pub frames: Vec<Mat4>,
}

/// Manual clip registry + bake queues. Created once by the base bake with the
/// LOD skeleton snapshot the background tasks sample against.
#[derive(Resource)]
pub struct GpuAnimationBank {
    /// LOD bone order every baked frame follows. Needed game-side to fit
    /// per-shape skeletons in the same order.
    pub bones: Vec<&'static str>,
    pub(crate) targets: Vec<AnimationTargetId>,
    pub(crate) binds: Vec<Transform>,
    pub(crate) sample_rate: f32,
    /// Next free frame in the shared buffer. Starts at 1: frame 0 is the
    /// permanent bindpose clip in slot 0, so cleared table entries read rest.
    pub(crate) frame_total: u32,
    /// CPU-side shadow of the shared `frames` buffer (seed + every appended
    /// clip). Bevy 0.19 `take_gpu_data` empties `ShaderBuffer::data` on
    /// upload, so the buffer itself cannot be used as the append source.
    pub(crate) frames: Vec<Mat4>,
    pub(crate) slots: Vec<Option<(String, BankSlot)>>,
    pub(crate) pending: Vec<PendingGpuAnimationClip>,
    pub(crate) baking: Vec<String>,
    pub(crate) unload_queue: Vec<String>,
    /// Completed bakes waiting for a quiet moment. Committed (slots assigned,
    /// shadow rebuilt) only when no bake work is pending, so a burst of clips
    /// re-uploads once and bank offsets never lead the GPU buffer.
    pub(crate) staged: Vec<BakedClip>,
}

impl GpuAnimationBank {
    pub(crate) fn new(
        bones: Vec<&'static str>,
        targets: Vec<AnimationTargetId>,
        binds: Vec<Transform>,
        sample_rate: f32,
    ) -> Self {
        let mut slots: Vec<Option<(String, BankSlot)>> =
            (0..MAX_GPU_CLIPS).map(|_| None).collect();
        slots[BIND_POSE_SLOT] = Some((
            BIND_POSE_CLIP.to_string(),
            BankSlot {
                offset_frames: 0,
                frame_count: 1,
                duration: 1.0 / sample_rate.max(1.0),
                mode: GpuClipMode::Loop,
            },
        ));
        Self {
            bones,
            targets,
            binds,
            sample_rate,
            frame_total: 1,
            frames: Vec::new(),
            slots,
            pending: Vec::new(),
            baking: Vec::new(),
            unload_queue: Vec::new(),
            staged: Vec::new(),
        }
    }

    /// Queues a background bake of `name` with the given playback mode.
    /// Returns false when the clip is already resident, queued, or baking, or
    /// when every bank slot is occupied (unload something, then retry).
    /// Slot 0 is the permanent bindpose clip: real clips start at slot 1.
    /// Staged-but-uncommitted clips report success without queueing a
    pub fn request_load(&mut self, name: impl Into<String>, mode: GpuClipMode) -> bool {
        let name = name.into();
        if name == BIND_POSE_CLIP {
            return false;
        }
        if let Some(idx) = self.slot_of(&name) {
            // Already resident: cancel a queued unload so the load wins.
            self.unload_queue.retain(|n| n != &name);
            let _ = idx;
            return true;
        }
        if self.staged.iter().any(|s| s.name == name) {
            return true;
        }
        if self.pending.iter().any(|p| p.name == name)
            || self.baking.iter().any(|b| b == &name)
        {
            return false;
        }
        if self.slots.iter().skip(1).all(|s| s.is_some()) {
            return false;
        }
        self.pending.push(PendingGpuAnimationClip { name, mode });
        true
    }
    /// Unloads `name`. Resident clips are queued for a packed rebuild (bytes
    /// spliced out of the shadow, later offsets rewritten, cleared slots read
    /// the frame-0 bindpose clip — drive the slot weight to 0); staged-but-
    /// uncommitted clips are dropped outright; queued-but-undispatched
    /// requests are just dequeued. Returns false when the clip is unknown or
    /// currently baking (bakes cannot be cancelled — retry once it lands).
    pub fn request_unload(&mut self, name: &str) -> bool {
        if name == BIND_POSE_CLIP {
            return false;
        }
        if self.slot_of(name).is_some() {
            if !self.unload_queue.iter().any(|n| n == name) {
                self.unload_queue.push(name.to_string());
            }
            return true;
        }
        if let Some(i) = self.staged.iter().position(|s| s.name == name) {
            self.staged.remove(i);
            return true;
        }
        if let Some(i) = self.pending.iter().position(|p| p.name == name) {
            self.pending.remove(i);
            return true;
        }
        false
    }

    /// True once the clip's frames are appended and its uniform row is live.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.slot_of(name).is_some()
    }

    /// True while a background bake task for `name` is still running.
    pub fn is_baking(&self, name: &str) -> bool {
        self.baking.iter().any(|b| b == name)
    }

    /// Bank index for wiring per-instance blend slots
    /// (see [`GpuInstanceAnims::set_slot`](super::state::GpuInstanceAnims::set_slot)).
    pub fn slot_of(&self, name: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| matches!(s, Some((n, _)) if n == name))
    }

    pub(crate) fn free_slot(&self) -> Option<usize> {
        self.slots.iter().enumerate().skip(1).find(|(_, s)| s.is_none()).map(|(i, _)| i)
    }

    /// Uniform tables in shader order (offsets, counts, durations, modes).
    /// Cleared rows are zero, which reads the frame-0 bindpose clip.
    pub(crate) fn tables(
        &self,
    ) -> (
        [u32; MAX_GPU_CLIPS],
        [u32; MAX_GPU_CLIPS],
        [f32; MAX_GPU_CLIPS],
        [u32; MAX_GPU_CLIPS],
    ) {
        let mut offsets = [0u32; MAX_GPU_CLIPS];
        let mut counts = [0u32; MAX_GPU_CLIPS];
        let mut durations = [0.0f32; MAX_GPU_CLIPS];
        let mut modes = [0u32; MAX_GPU_CLIPS];
        for (slot, entry) in self.slots.iter().enumerate() {
            if let Some((_, meta)) = entry {
                offsets[slot] = meta.offset_frames;
                counts[slot] = meta.frame_count;
                durations[slot] = meta.duration;
                modes[slot] = match meta.mode {
                    GpuClipMode::Loop => 0,
                    GpuClipMode::OnceHold => 1,
                };
            }
        }
        (offsets, counts, durations, modes)
    }
}

/// Channel for completed background clip bakes.
#[derive(Resource)]
pub(crate) struct GpuBakeJobs {
    pub sender: Sender<BakedClip>,
    pub receiver: Receiver<BakedClip>,
}

impl Default for GpuBakeJobs {
    fn default() -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        Self { sender, receiver }
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct GpuRenderHandles {
    pub parents: Handle<ShaderBuffer>,
    pub frames: Handle<ShaderBuffer>,
    pub inv_bind: Handle<ShaderBuffer>,
    /// Per-shape local rest translations: `shape_count * num_bones` `Vec4`s.
    /// Slice 0 mirrors the reference translations baked into the clip frames;
    /// slice `i + 1` holds `GpuCrowdShapes.shapes[i]`.
    pub shape_translations: Handle<ShaderBuffer>,
    /// Per-shape inverse bindposes: `shape_count * num_bones` `Mat4`s, same
    /// layout as `shape_translations`.
    pub shape_inv_binds: Handle<ShaderBuffer>,
    pub joints: Handle<ShaderBuffer>,
    pub uniforms: Handle<ShaderBuffer>,
    pub instance_data: Handle<ShaderBuffer>,
    /// Per-instance shape weights: `instance_count * MAX_GPU_SHAPES` floats,
    /// one row per instance with the same semantics as the entity's morph
    /// weights (slot `i` blends `GpuCrowdShapes.shapes[i]`).
    pub shape_weights: Handle<ShaderBuffer>,
    /// Per-instance root-bone Y scales: `instance_count` floats, mirroring the
    /// CPU `SkeletonRootBone.root_scale` per character.
    pub root_scales: Handle<ShaderBuffer>,
    /// Reference root bind-pose Y: one float, the reference rig's root model-space Y.
    pub root_bind: Handle<ShaderBuffer>,
    pub num_bones: u32,
    pub instance_count: u32,
    pub shape_count: u32,
    pub clip_offsets: [u32; MAX_GPU_CLIPS],
    pub clip_frames: [u32; MAX_GPU_CLIPS],
    pub durations: [f32; MAX_GPU_CLIPS],
    pub modes: [u32; MAX_GPU_CLIPS],
}

#[derive(Resource)]
pub struct GpuAnimationReady;
