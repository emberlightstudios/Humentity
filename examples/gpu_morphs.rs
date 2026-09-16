//! GPU morphs: `morphs_and_templates` bodies posed entirely on the GPU.
//!
//! Same baby/bodybuilder template, but crowd-style: one shared GPU mesh,
//! the idle clip baked once on the single crowd skeleton, per-entity
//! [`MeshMorphWeights`] for the body variation. If morphs work through the
//! GPU skinning vertex shader, the five characters look different despite
//! sharing one mesh handle and one material.

mod shared;

use bevy::{
    mesh::{MeshTag, morph::MeshMorphWeights},
    prelude::*,
};
use humentity::prelude::*;
use shared::{CameraFraming, CustomCrowdMaterial, GPU_SKELETON_LOD, custom_crowd_material, setup_app_gpu};

const BABY: &str = "baby";
const BODYBUILDER: &str = "bodybuilder";
const INSTANCES: usize = 5;

fn main() {
    let mut app = setup_app_gpu(INSTANCES, 30.0, CameraFraming::Close);
    app.add_systems(
        Update,
        (
            trigger_morph_build.run_if(resource_added::<HumentityAssetsReady>),
            request_clip_bakes,
            spawn_crowd,
        ),
    )
    .run();
}

#[derive(Resource)]
struct MorphBuild {
    part: Handle<MhcloAsset>,
    template: Handle<CharacterTemplate>,
    clips: Handle<RetargetedAnimationAsset>,
}

fn trigger_morph_build(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut templates: ResMut<Assets<CharacterTemplate>>,
) {
    let mut baby_targets = MorphTargets::default();
    baby_targets.insert("age", 0.);

    let mut bodybuilder_targets = MorphTargets::default();
    bodybuilder_targets.insert("muscle", 1.);
    bodybuilder_targets.insert("weight", 1.);

    let template = templates.add(CharacterTemplate::new([
        CharacterMorphShape::new(BODYBUILDER, bodybuilder_targets),
        CharacterMorphShape::new(BABY, baby_targets),
    ]));
    let part = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: part.clone(),
        template_handle: template.clone(),
        skeleton_lod: GPU_SKELETON_LOD,
    });
    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(MorphBuild {
        part,
        template,
        clips,
    });
    info!("gpu morph build triggered");
}

/// Manual clip loads: retries every frame until the bank accepts every clip
/// in the retargeted asset. Nothing else queues loads.
fn request_clip_bakes(
    bank: Option<ResMut<GpuAnimationBank>>,
    build: Option<Res<MorphBuild>>,
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
    build: Option<Res<MorphBuild>>,
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
    if !mesh.has_morph_targets() {
        warn!("gpu_morphs: mesh has no morph targets, weights will do nothing");
    }
    let mesh = meshes.add(mesh);
    let material = custom_crowd_material(&handles, &mut materials);
    if let Some(anims) = anims.as_mut() {
        for index in 0..INSTANCES {
            anims.set_slot(index, 0, idle as u32);
        }
    }
    *spawned = true;
    // Template order is [bodybuilder, baby]; weights follow shape order.
    let cases = [
        ("Basemesh", [0.0, 0.0], -2.0),
        ("Baby", [0.0, 1.0], -1.0),
        ("Bodybuilder", [1.0, 0.0], 0.0),
        ("Hybrid normalized", [0.5, 0.5], 1.0),
        ("Hybrid unnormalized", [1.0, 1.0], 2.0),
    ];
    for (index, (name, weights, x)) in cases.into_iter().enumerate() {
        commands.spawn((
            Name::new(name),
            Transform::from_translation(Vec3::new(x, 0., 0.)),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            MeshTag(index as u32),
            MeshMorphWeights::Value {
                weights: weights.to_vec(),
            },
        ));
    }
    info!("spawned {INSTANCES} GPU-posed morph characters sharing one mesh and one material");
}
