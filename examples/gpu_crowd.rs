//! Basic GPU crowd: 8000 characters posed entirely on the GPU.
//!
//! The idle clip is baked once to local bone matrices on the single crowd skeleton. Every
//! frame a compute shader poses all instances x bones and the joint buffer
//! feeds the skinning vertex shader bindlessly. The main world holds only
//! static `Transform`s: no skeletons, no `AnimationPlayer`, no transform
//! propagation for the crowd.

mod shared;

use bevy::{mesh::MeshTag, prelude::*};
use humentity::prelude::*;
use shared::{CustomCrowdMaterial, GPU_SKELETON_LOD, custom_crowd_material, setup_app_gpu};

const INSTANCES: usize = 8_000;

fn main() {
    let mut app = setup_app_gpu(INSTANCES, 30.0);
    app.add_systems(
        Update,
        (
            trigger_crowd_build.run_if(resource_added::<HumentityAssetsReady>),
            request_clip_bakes,
            spawn_crowd,
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
) {
    let template = templates.add(CharacterTemplate::new([CharacterMorphShape::new(
        "neutral",
        MorphTargets::default(),
    )]));
    let part = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: part.clone(),
        template_handle: template.clone(),
        skeleton_lod: GPU_SKELETON_LOD,
    });
    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(CrowdBuild {
        part,
        template,
        clips,
    });
    info!("crowd build triggered");
}

/// Manual clip loads: retries every frame until the bank accepts every clip
/// in the retargeted asset. Nothing else queues loads.
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
    // Idle must be resident before the crowd binds blend slot 0 to it.
    let Some(idle) = bank.slot_of("Idle-loop") else {
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
            anims.set_slot(index, 0, idle as u32);
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
            Name::new(format!("GpuChar {index}")),
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
    info!("spawned {count} GPU-posed characters sharing one mesh and one material");
}

