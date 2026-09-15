//! Baked GPU animation resources: the clip bank, material handle, render-world
//! buffer handles, and the ready marker.

use bevy::{
    prelude::*,
    render::{extract_resource::ExtractResource, storage::ShaderBuffer},
};

use super::{config::MAX_BLEND_CLIPS, material::CrowdMaterial};

#[derive(Resource)]
pub struct GpuAnimationBank {
    pub num_bones: usize,
    pub num_frames: usize,
    pub duration: f32,
    pub clip_names: Vec<String>,
}

#[derive(Resource)]
pub struct GpuAnimationHandles {
    pub material: Handle<CrowdMaterial>,
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
    pub num_frames: u32,
    pub instance_count: u32,
    pub clip_count: u32,
    pub durations: [f32; MAX_BLEND_CLIPS],
    pub modes: [u32; MAX_BLEND_CLIPS],
}

#[derive(Resource)]
pub struct GpuAnimationReady;
