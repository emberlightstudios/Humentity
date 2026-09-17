//! Render-world pose pipeline: the compute pipeline that evaluates every
//! instance x bone each frame, its bind group, and the per-frame dispatch.

use bevy::{
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::GpuShaderBuffer,
    },
};

use super::{
    bank::GpuRenderHandles,
    config::{POSE_WORKGROUP_X, POSE_WORKGROUP_Y, POSE_WORKGROUP_Z, pose_grid_side},
    POSE_SHADER,
};

#[derive(Resource)]
pub(super) struct CrowdPosePipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

#[derive(Resource)]
pub(super) struct CrowdPoseBindGroup(BindGroup);

pub(super) fn init_pose_pipeline(mut commands: Commands, pipeline_cache: Res<PipelineCache>) {
    let entries = vec![
        BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 1,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 2,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 3,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 4,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 5,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 6,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 7,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 8,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 9,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 10,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
    ];
    let layout = BindGroupLayoutDescriptor::new("crowd_pose", &entries);
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        layout: vec![layout.clone()],
        shader: POSE_SHADER,
        entry_point: Some("main".into()),
        ..default()
    });
    commands.insert_resource(CrowdPosePipeline { layout, pipeline });
}

pub(super) fn prepare_pose_bind_group(
    mut commands: Commands,
    pipeline: Res<CrowdPosePipeline>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    handles: Option<Res<GpuRenderHandles>>,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
) {
    let Some(handles) = handles else {
        return;
    };
    let (
        Some(parents),
        Some(frames),
        Some(inv_bind),
        Some(shape_translations),
        Some(shape_inv_binds),
        Some(joints),
        Some(uniforms),
        Some(instance_data),
        Some(shape_weights),
        Some(root_scales),
        Some(root_bind),
    ) = (
        gpu_buffers.get(&handles.parents),
        gpu_buffers.get(&handles.frames),
        gpu_buffers.get(&handles.inv_bind),
        gpu_buffers.get(&handles.shape_translations),
        gpu_buffers.get(&handles.shape_inv_binds),
        gpu_buffers.get(&handles.joints),
        gpu_buffers.get(&handles.uniforms),
        gpu_buffers.get(&handles.instance_data),
        gpu_buffers.get(&handles.shape_weights),
        gpu_buffers.get(&handles.root_scales),
        gpu_buffers.get(&handles.root_bind),
    )
    else {
        return;
    };
    let layout = pipeline_cache.get_bind_group_layout(&pipeline.layout);
    let bind_group = render_device.create_bind_group(
        None,
        &layout,
        &BindGroupEntries::sequential((
            parents.buffer.as_entire_buffer_binding(),
            frames.buffer.as_entire_buffer_binding(),
            inv_bind.buffer.as_entire_buffer_binding(),
            joints.buffer.as_entire_buffer_binding(),
            uniforms.buffer.as_entire_buffer_binding(),
            instance_data.buffer.as_entire_buffer_binding(),
            shape_translations.buffer.as_entire_buffer_binding(),
            shape_inv_binds.buffer.as_entire_buffer_binding(),
            shape_weights.buffer.as_entire_buffer_binding(),
            root_scales.buffer.as_entire_buffer_binding(),
            root_bind.buffer.as_entire_buffer_binding(),
        )),
    );
    commands.insert_resource(CrowdPoseBindGroup(bind_group));
}

pub(super) fn dispatch_pose(
    pipeline: Res<CrowdPosePipeline>,
    bind_group: Option<Res<CrowdPoseBindGroup>>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    handles: Option<Res<GpuRenderHandles>>,
) {
    let Some(handles) = handles else {
        return;
    };
    let Some(bind_group) = bind_group else {
        return;
    };
    let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline) else {
        return;
    };
    let side = pose_grid_side(handles.instance_count);
    let groups_x = side.div_ceil(POSE_WORKGROUP_X);
    let groups_y = side.div_ceil(POSE_WORKGROUP_Y);
    let groups_z = handles.num_bones.div_ceil(POSE_WORKGROUP_Z);
    let mut encoder =
        render_device
            .wgpu_device()
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("crowd_pose"),
            });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_bind_group(0, &bind_group.0, &[]);
        pass.set_pipeline(compute_pipeline);
        pass.dispatch_workgroups(groups_x, groups_y, groups_z);
    }
    render_queue.submit(std::iter::once(encoder.finish()));
}

