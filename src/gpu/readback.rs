//! GPU joints readback: optional copy of the posed `joints` buffer to the CPU.
//!
//! Off by default ([`GpuCrowdConfig::readback_joints`](super::config::GpuCrowdConfig)).
//! When enabled, the plugin keeps one
//! [`Readback`](bevy::render::gpu_readback::Readback) entity on the pose
//! `joints` buffer, so Bevy copies it back every frame and fires
//! [`ReadbackComplete`](bevy::render::gpu_readback::ReadbackComplete) on that
//! entity. An observer decodes the bytes into [`GpuJointsReadback`]:
//! `instance_count * num_bones` row-major [`Mat4`]s, 1-2 frames stale, with no
//! main-thread stall. Gameplay (hitboxes) reads the resource, never the GPU.

use bevy::{prelude::*, render::gpu_readback::Readback};

use super::{bank::GpuRenderHandles, config::GpuCrowdConfig};

/// Latest CPU copy of the posed `joints` buffer, row-major
/// `instance * num_bones + bone`. Only populated when
/// [`GpuCrowdConfig::readback_joints`](super::config::GpuCrowdConfig) is true;
/// otherwise stays empty. Data is 1-2 frames stale by design.
#[derive(Resource, Default, Debug, Clone)]
pub struct GpuJointsReadback {
    /// Posed joint matrices in instance-major order.
    pub joints: Vec<Mat4>,
    /// Bones per instance (row stride).
    pub num_bones: usize,
    /// Crowd instances covered.
    pub instance_count: usize,
    /// Completed copies observed (bumps on every landed copy).
    pub frames: u64,
}

impl GpuJointsReadback {
    /// Joint matrix for `instance` + `bone`, or `None` when out of range or
    /// no copy has landed yet.
    pub fn joint(&self, instance: usize, bone: usize) -> Option<Mat4> {
        if bone >= self.num_bones {
            return None;
        }
        self.joints.get(instance * self.num_bones + bone).copied()
    }
}

/// Marker for the entity carrying the persistent joints [`Readback`] request.
#[derive(Component)]
pub(super) struct JointsReadbackRequest;
/// Decodes each completed joints copy into [`GpuJointsReadback`]. The buffer
/// holds `Mat4`s, so the bytes decode straight into column-major joints.
fn on_joints_readback(
    trigger: On<bevy::render::gpu_readback::ReadbackComplete>,
    handles: Option<Res<GpuRenderHandles>>,
    mut readback: ResMut<GpuJointsReadback>,
) {
    let Some(handles) = handles else {
        return;
    };
    let mut joints: Vec<Mat4> = trigger.to_shader_type();
    let bones = handles.num_bones.max(1) as usize;
    let instances = handles.instance_count.max(1) as usize;
    joints.truncate(instances * bones);
    readback.joints = joints;
    readback.num_bones = bones;
    readback.instance_count = instances;
    readback.frames += 1;
}

/// Keeps exactly one readback request on the pose `joints` buffer while the
/// config toggle is on; despawns it when toggled off so copies stop. The
/// `joints` buffer already carries `COPY_SRC` (default `ShaderBuffer` usage
/// is `STORAGE | COPY_SRC | COPY_DST`), so no buffer change is needed.
pub(super) fn maintain_joints_readback(
    mut commands: Commands,
    config: Res<GpuCrowdConfig>,
    handles: Option<Res<GpuRenderHandles>>,
    requests: Query<Entity, With<JointsReadbackRequest>>,
) {
    let wants = config.readback_joints && handles.is_some();
    let has = !requests.is_empty();
    if wants == has {
        return;
    }
    if wants {
        let handles = handles.expect("checked above");
        commands
            .spawn((
                Name::new("GpuJointsReadback"),
                Readback::buffer(handles.joints.clone()),
                JointsReadbackRequest,
            ))
            .observe(on_joints_readback);
    } else {
        for entity in &requests {
            commands.entity(entity).despawn();
        }
    }
}
