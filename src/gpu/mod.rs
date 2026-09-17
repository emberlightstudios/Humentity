//! GPU animation pipeline for humentity skeletons.
//!
//! Bevy poses every skeleton on the CPU (`AnimationPlayer` + transform
//! propagation) and only skins on the GPU. This plugin moves posing to a
//! compute shader: clips are baked once to local bone matrices, a compute
//! dispatch evaluates all instances x bones every frame, and the resulting
//! joint buffer is consumed bindlessly by a custom skinning vertex shader.
//! No per-bone entities, no `AnimationPlayer`, no transform propagation for
//! the crowd.
//!
//! The bake poses the skeleton LOD selected by [`GpuSkeletonLod`], which must
//! match the `skeleton_lod` the crowd meshes were built with so joint indices
//! line up. Lower LODs shrink the joints buffer and the per-frame dispatch.

mod bake;
mod bank;
mod config;
mod material;
mod pipeline;
mod shapes;
mod state;

use bake::{bake_gpu_animation, collect_clip_bakes, submit_clip_bakes, upload_shape_buffers};
pub use bank::{
    BIND_POSE_CLIP, BIND_POSE_SLOT, GpuAnimationBank, GpuAnimationReady, GpuRenderHandles,
};
pub use config::{
    GpuBlendWeights, GpuClipMode, GpuCrowdConfig, GpuCrowdShapes, GpuShapeSkeleton, GpuSkeletonLod,
    POSE_WORKGROUP_X, POSE_WORKGROUP_Y, POSE_WORKGROUP_Z, pose_grid_side,
};
pub use material::{
    ATTRIBUTE_GPU_JOINT_INDEX, ATTRIBUTE_GPU_JOINT_WEIGHT, CrowdMaterial, GpuCrowdExtension,
    GpuCrowdUniform, make_gpu_mesh, specialize_gpu_vertex_layout,
};
pub use shapes::{fit_shape_skeleton, fit_shape_skeleton_from_helpers};
pub use state::{GpuInstanceAnims, GpuOneShotDone};

use std::marker::PhantomData;

use bevy::{
    asset::{load_internal_asset, uuid::Uuid},
    pbr::MaterialPlugin,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems, extract_resource::ExtractResourcePlugin,
    },
};

pub const POSE_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x677075616e696d4750753030317567),
    PhantomData,
);
pub const CROWD_SKIN_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x677075616e696d4750753030327567),
    PhantomData,
);

/// The shared skinning snippet: joint/uniform bindings plus the single
/// `gpu_skin_vertex` function. Custom vertex shaders compose this with their
/// own `@vertex` entry that calls the function, then adds work on top of the
/// returned `VertexOutput`. Entries must not re-import what the snippet
/// already imports (`VertexOutput`, `position_world_to_clip`); duplicate
/// imports fail as ambiguous.
pub const fn gpu_skin_wgsl() -> &'static str {
    include_str!("skin.wgsl")
}

/// Default `@vertex` entry: pure call-through to `gpu_skin_vertex`, no extras.
const CROWD_SKIN_ENTRY: &str = r#"
struct GpuVertex {
    @builtin(instance_index) instance_index: u32,
    @builtin(vertex_index) vertex_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(6) joint_indices: vec4<u32>,
    @location(7) joint_weights: vec4<f32>,
};

@vertex
fn vertex(vertex: GpuVertex) -> VertexOutput {
    return gpu_skin_vertex(
        vertex.instance_index,
        vertex.vertex_index,
        vertex.position,
        vertex.normal,
        vertex.uv,
        vertex.joint_indices,
        vertex.joint_weights,
    );
}
"#;

pub struct HumentityGpuPlugin {
    pub instances: usize,
    pub sample_rate: f32,
    pub blend_slots: usize,
    pub max_clips: usize,
    pub max_shapes: usize,
}

impl Default for HumentityGpuPlugin {
    fn default() -> Self {
        let defaults = GpuCrowdConfig::default();
        Self {
            instances: defaults.instances,
            sample_rate: defaults.sample_rate,
            blend_slots: defaults.blend_slots,
            max_clips: defaults.max_clips,
            max_shapes: defaults.max_shapes,
        }
    }
}

impl Plugin for HumentityGpuPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, POSE_SHADER, "pose.wgsl", Shader::from_wgsl);
        // The default skin shader is composed from the shared snippet so the
        // function stays the single source of truth.
        let _ = app.world_mut().resource_mut::<Assets<Shader>>().insert(
            CROWD_SKIN_SHADER.id(),
            Shader::from_wgsl(
                format!("{}\n{}", gpu_skin_wgsl(), CROWD_SKIN_ENTRY),
                "crowd_skin.wgsl",
            ),
        );
        app.add_plugins(MaterialPlugin::<CrowdMaterial>::default());
        app.add_plugins(ExtractResourcePlugin::<GpuRenderHandles>::default());
        app.insert_resource(GpuCrowdConfig {
            instances: self.instances,
            sample_rate: self.sample_rate,
            blend_slots: self.blend_slots,
            max_clips: self.max_clips,
            max_shapes: self.max_shapes,
        });
        app.init_resource::<GpuBlendWeights>();
        app.init_resource::<GpuSkeletonLod>();
        app.insert_resource(config::GpuCrowdShapes::with_capacity(self.max_shapes));
        app.init_resource::<bank::GpuBakeJobs>();
        app.add_message::<GpuOneShotDone>();
        app.add_systems(
            Update,
            (
                bake_gpu_animation,
                submit_clip_bakes,
                collect_clip_bakes,
                upload_shape_buffers,
                state::update_instance_clocks,
            )
                .chain(),
        );
        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .add_systems(RenderStartup, pipeline::init_pose_pipeline)
            .add_systems(
                Render,
                (pipeline::prepare_pose_bind_group, pipeline::dispatch_pose)
                    .chain()
                    .in_set(RenderSystems::PrepareBindGroups),
            );
    }
}
