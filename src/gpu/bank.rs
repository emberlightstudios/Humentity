//! GPU clip bank: manual load/unload registry for baked animation clips.
//!
//! The bank owns the global clip registry (up to [`MAX_GPU_CLIPS`] clips).
//! Per-instance blend slots ([`MAX_BLEND_CLIPS`]) index into the bank, so the
//! crowd can blend any 4 of the loaded clips. Loads are manual via
//! [`GpuAnimationBank::request_load`]: each request spawns one background bake
//! task, and the baked frames are appended to the shared `frames` buffer when
//! ready. Unloads ([`GpuAnimationBank::request_unload`]) clear the slot's
//! uniform row; the buffer bytes stay in place (fragmentation, compaction is
//! future work), so neighbouring clips are unaffected.
//!
//! A cleared uniform row reads frame 0, which the base bake seeds with the
//! bindpose — an unloaded slot falls back to the bindpose. Callers must still
//! drive the slot weight to 0 so it contributes nothing.

use bevy::{
    animation::AnimationTargetId,
    prelude::*,
    render::{extract_resource::ExtractResource, storage::ShaderBuffer},
};
use crossbeam_channel::{Receiver, Sender};

use super::config::{GpuClipMode, MAX_GPU_CLIPS};

/// Metadata for one resident bank slot. The clip's bytes live at
/// `offset_frames * num_bones` in the shared `frames` buffer and are never
/// moved, so unload never disturbs neighbours.
#[derive(Clone, Copy)]
pub(crate) struct BankSlot {
    pub offset_frames: u32,
    pub frame_count: u32,
    pub duration: f32,
    pub mode: GpuClipMode,
}

/// A queued manual load request, awaiting asset resolution + background bake.
#[derive(Clone)]
pub(crate) struct PendingLoad {
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
    pub(crate) bones: Vec<&'static str>,
    pub(crate) targets: Vec<AnimationTargetId>,
    pub(crate) binds: Vec<Transform>,
    pub(crate) sample_rate: f32,
    /// Next free frame in the shared buffer. Starts at 1: frame 0 is the
    /// bindpose seed, so cleared uniform rows read a valid pose.
    pub(crate) frame_total: u32,
    /// CPU-side shadow of the shared `frames` buffer (seed + every appended
    /// clip). Bevy 0.19 `take_gpu_data` empties `ShaderBuffer::data` on
    /// upload, so the buffer itself cannot be used as the append source.
    pub(crate) frames: Vec<Mat4>,
    pub(crate) slots: Vec<Option<(String, BankSlot)>>,
    pub(crate) pending: Vec<PendingLoad>,
    pub(crate) baking: Vec<String>,
    pub(crate) unload_queue: Vec<String>,
}

impl GpuAnimationBank {
    pub(crate) fn new(
        bones: Vec<&'static str>,
        targets: Vec<AnimationTargetId>,
        binds: Vec<Transform>,
        sample_rate: f32,
    ) -> Self {
        Self {
            bones,
            targets,
            binds,
            sample_rate,
            frame_total: 1,
            frames: Vec::new(),
            slots: (0..MAX_GPU_CLIPS).map(|_| None).collect(),
            pending: Vec::new(),
            baking: Vec::new(),
            unload_queue: Vec::new(),
        }
    }

    /// Queues a background bake of `name` with the given playback mode.
    /// Returns false when the clip is already resident, queued, or baking, or
    /// when every bank slot is occupied (unload something, then retry).
    pub fn request_load(&mut self, name: impl Into<String>, mode: GpuClipMode) -> bool {
        let name = name.into();
        if let Some(idx) = self.slot_of(&name) {
            // Already resident: cancel a queued unload so the load wins.
            self.unload_queue.retain(|n| n != &name);
            let _ = idx;
            return true;
        }
        if self.pending.iter().any(|p| p.name == name)
            || self.baking.iter().any(|b| b == &name)
        {
            return false;
        }
        if self.slots.iter().all(|s| s.is_some()) {
            return false;
        }
        self.pending.push(PendingLoad { name, mode });
        true
    }

    /// Unloads `name`. Resident clips are queued for uniform clearing (their
    /// buffer bytes stay until a future compaction); queued-but-undispatched
    /// requests are just dequeued. Returns false when the clip is unknown or
    /// currently baking (bakes cannot be cancelled — retry once it lands).
    ///
    /// Cleared slots read the bindpose seed; drive the slot weight to 0.
    pub fn request_unload(&mut self, name: &str) -> bool {
        if self.slot_of(name).is_some() {
            if !self.unload_queue.iter().any(|n| n == name) {
                self.unload_queue.push(name.to_string());
            }
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
        self.slots.iter().position(|s| s.is_none())
    }

    /// Uniform tables in shader order (offsets, counts, durations, modes).
    /// Cleared rows are zero, which reads the bindpose seed frame.
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
    pub joints: Handle<ShaderBuffer>,
    pub uniforms: Handle<ShaderBuffer>,
    pub instance_data: Handle<ShaderBuffer>,
    pub num_bones: u32,
    pub instance_count: u32,
    pub clip_offsets: [u32; MAX_GPU_CLIPS],
    pub clip_frames: [u32; MAX_GPU_CLIPS],
    pub durations: [f32; MAX_GPU_CLIPS],
    pub modes: [u32; MAX_GPU_CLIPS],
}

#[derive(Resource)]
pub struct GpuAnimationReady;
