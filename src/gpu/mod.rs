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
mod readback;
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
pub use readback::GpuJointsReadback;
pub use shapes::{fit_shape_skeleton, fit_shape_skeleton_from_helpers};
pub use state::{GpuInstanceAnims, GpuOneShotDone};

use std::marker::PhantomData;

use bevy::{
    asset::{embedded_asset, load_internal_asset, uuid::Uuid},
    pbr::MaterialPlugin,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems, extract_resource::ExtractResourcePlugin,
    },
    shader::load_shader_library,
};

pub const POSE_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x677075616e696d4750753030317567),
    PhantomData,
);

/// Forward color-pass entry: calls the shared `humentity::crowd_skin` module
/// and builds `forward_io::VertexOutput`.
pub const CROWD_FORWARD_SHADER: &str = "embedded://humentity/gpu/crowd_forward.wgsl";
/// Posed depth/shadow entry (parked): calls the shared `humentity::crowd_skin`
/// module and builds `prepass_io::VertexOutput`. Currently unused — both
/// material hooks return `ShaderRef::Default` until Bevy binds the real
/// material layout for custom prepass shaders (#24843 / `prepass_reads_material`);
/// depth and shadows fall back to bind pose until then.
pub const CROWD_PREPASS_SHADER: &str = "embedded://humentity/gpu/crowd_prepass.wgsl";

/// The shared skinning module (`humentity::crowd_skin`): joint/uniform
/// bindings plus the single `crowd_skin_pose` function, which returns plain
/// posed-vertex data. Both entries call it and build their own output type.
/// Entries must not re-import what the module already imports; duplicate
/// imports fail as ambiguous.
pub struct HumentityGpuPlugin {
    pub instances: usize,
    pub sample_rate: f32,
    pub blend_slots: usize,
    pub max_clips: usize,
    pub max_shapes: usize,
    /// When true, the pose `joints` buffer is copied back to the CPU every
    /// frame into [`GpuJointsReadback`]. Data lands 1-2 frames stale with no
    /// main-thread stall; leave false unless hitboxes or gameplay queries
    /// need posed joints.
    pub readback_joints: bool,
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
            readback_joints: defaults.readback_joints,
        }
    }
}

impl Plugin for HumentityGpuPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, POSE_SHADER, "pose.wgsl", Shader::from_wgsl);
        load_shader_library!(app, "crowd_skin.wgsl");
        embedded_asset!(app, "crowd_forward.wgsl");
        embedded_asset!(app, "crowd_prepass.wgsl");
        app.add_plugins(MaterialPlugin::<CrowdMaterial>::default());
        app.add_plugins(ExtractResourcePlugin::<GpuRenderHandles>::default());
        app.insert_resource(GpuCrowdConfig {
            instances: self.instances,
            sample_rate: self.sample_rate,
            blend_slots: self.blend_slots,
            max_clips: self.max_clips,
            max_shapes: self.max_shapes,
            readback_joints: self.readback_joints,
        });
        app.init_resource::<GpuBlendWeights>();
        app.init_resource::<GpuSkeletonLod>();
        app.insert_resource(config::GpuCrowdShapes::with_capacity(self.max_shapes));
        app.init_resource::<bank::GpuBakeJobs>();
        app.init_resource::<GpuJointsReadback>();
        app.add_message::<GpuOneShotDone>();
        app.add_systems(
            Update,
            (
                bake_gpu_animation,
                submit_clip_bakes,
                collect_clip_bakes,
                upload_shape_buffers,
                state::update_instance_clocks,
                readback::maintain_joints_readback,
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
