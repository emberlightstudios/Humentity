//! GPU locomotion blend: 25,000 characters blending forward walk with strafe
//! right entirely on the GPU.
//!
//! The `normal-walk` and `normal-walk-strafe-right` clips from
//! `assets/animation/movement_normal.glb` are baked once on the single crowd
//! skeleton; every frame the pose shader blends them with per-instance
//! [`GpuInstanceAnims`] weights. This example sweeps the weights on a sine so
//! the blend is visible, and shows the live mix in the FPS readout.

mod shared;

use bevy::{mesh::MeshTag, prelude::*};
use humentity::prelude::*;
use shared::{CameraFraming, CustomCrowdMaterial, GPU_SKELETON_LOD, custom_crowd_material, setup_app_gpu};
use shared::FpsText;

const WALK: &str = "normal-walk";
const STRAFE_RIGHT: &str = "normal-walk-strafe-right";
const INSTANCES: usize = 25_000;

fn main() {
    let mut app = setup_app_gpu(INSTANCES, 30.0, CameraFraming::Far);
    app.insert_resource(GpuBlendWeights([0.5, 0.5, 0.0, 0.0]));
    app.add_systems(
        Update,
        (
            trigger_crowd_build.run_if(resource_exists::<HumentityAssetsReady>),
            request_clip_bakes,
            spawn_crowd,
            sweep_blend_weights,
            update_blend_text,
        ),
    )
    .run();
}

#[derive(Resource)]
struct CrowdBuild {
    part: Handle<MhcloAsset>,
    template: Handle<CharacterTemplate>,
    clips: Handle<RetargetedAnimationAsset>,
}

fn trigger_crowd_build(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut templates: ResMut<Assets<CharacterTemplate>>,
    build: Option<Res<CrowdBuild>>,
) {
    if build.is_some() {
        return;
    }
    let template = templates.add(CharacterTemplate::new([CharacterMorphShape::new(
        "neutral",
        MorphTargets::default(),
    )]));
    let part = asset_server.load::<MhcloAsset>("proxymeshes/proxy741/proxy741.proxy");
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: part.clone(),
        template_handle: template.clone(),
        skeleton_lod: GPU_SKELETON_LOD,
    });
    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/movement_normal.glb");
    commands.insert_resource(CrowdBuild {
        part,
        template,
        clips,
    });
    info!("locomotion crowd build triggered");
}

fn spawn_crowd(
    mut commands: Commands,
    build: Option<Res<CrowdBuild>>,
    handles: Option<Res<GpuRenderHandles>>,
    bank: Option<Res<GpuAnimationBank>>,
    cached: Res<CachedMhcloMeshHandles>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CustomCrowdMaterial>>,
    mut anims: Option<ResMut<GpuInstanceAnims>>,
    mut spawned: Local<bool>,
) {
    if *spawned {
        return;
    }
    let (Some(build), Some(handles), Some(bank)) = (build, handles, bank) else {
        return;
    };
    // Walk on slot 0, strafe on slot 1; wait until both are resident.
    let (Some(walk_slot), Some(strafe_slot)) = (bank.slot_of(WALK), bank.slot_of(STRAFE_RIGHT))
    else {
        return;
    };
    let Some(source_handle) =
        cached.get(&(build.part.clone(), build.template.clone(), GPU_SKELETON_LOD))
    else {
        return;
    };
    let Some(source) = meshes.get(source_handle) else {
        return;
    };
    let Some(mesh) = make_gpu_mesh(source) else {
        return;
    };
    let mesh = meshes.add(mesh);
    let material = custom_crowd_material(&handles, &mut materials);
    if let Some(anims) = anims.as_mut() {
        for index in 0..INSTANCES {
            anims.set_slot(index, 0, walk_slot as u32);
            anims.set_slot(index, 1, strafe_slot as u32);
        }
    }
    *spawned = true;
    let count = INSTANCES;
    let side = (count as f32).sqrt().ceil() as usize;
    let spacing = 1.6;
    let offset = side as f32 * spacing * 0.5;
    for index in 0..count {
        let row = index / side;
        let col = index % side;
        commands.spawn((
            Name::new(format!("GpuWalk {index}")),
            Transform::from_xyz(
                col as f32 * spacing - offset,
                0.0,
                row as f32 * spacing - offset + 8.0,
            ),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            MeshTag(index as u32),
        ));
    }
    info!("spawned {count} GPU-blended characters sharing one mesh and one material");
}

fn sweep_blend_weights(time: Res<Time>, anims: Option<ResMut<GpuInstanceAnims>>) {
    let Some(mut anims) = anims else {
        return;
    };
    let walk = 0.5 + 0.5 * (time.elapsed_secs() * std::f32::consts::TAU / 10.0).sin();
    let target = [walk, 1.0 - walk, 0.0, 0.0];
    for t in anims.targets.iter_mut() {
        *t = target;
    }
}

/// Manual clip loads: when a retargeted clip asset finishes loading, queue
/// every clip it contains for background baking. Nothing else queues loads.
fn request_clip_bakes(
    bank: Option<ResMut<GpuAnimationBank>>,
    build: Option<Res<CrowdBuild>>,
    clips: Res<Assets<RetargetedAnimationAsset>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(mut bank), Some(build)) = (bank, build) else {
        return;
    };
    let Some(asset) = clips.get(&build.clips) else {
        return;
    };
    // Level-driven, not event-driven: a bank that is missing (base bake not
    // done) or full must not lose the load, so keep asking until accepted.
    let mut retry = false;
    for name in asset.clips.keys() {
        if !bank.is_loaded(name)
            && !bank.is_baking(name)
            && !bank.request_load(name.to_string(), GpuClipMode::Loop)
        {
            retry = true;
        }
    }
    *done = !retry;
}

/// Shows the live walk/strafe mix on the shared [`FpsText`] readout.
fn update_blend_text(
    anims: Option<Res<GpuInstanceAnims>>,
    mut query: Query<&mut Text, With<FpsText>>,
) {
    let Some(mix) = anims.and_then(|a| a.targets.first().copied()) else {
        return;
    };
    for mut text in &mut query {
        **text = format!(
            "{}  walk {:.0}% / strafe {:.0}%",
            text.as_str(),
            mix[0] * 100.0,
            mix[1] * 100.0
        );
    }
}
