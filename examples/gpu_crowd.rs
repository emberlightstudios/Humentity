//! Basic GPU crowd: many characters posed entirely on the GPU.
//!
//! The idle clip (full translation tracks, Bevy's native glTF loader) is baked
//! once to local bone matrices on the single crowd skeleton. Every frame a
//! compute shader poses all instances x bones with per-shape dynamic retarget
//! (length ratio + direction fix from each registered shape) and the joint
//! buffer feeds the skinning vertex shader bindlessly. The main world holds
//! only static `Transform`s: no skeletons, no `AnimationPlayer`, no transform
//! propagation for the crowd.

mod shared;

use bevy::{
    camera::visibility::VisibilityRange,
    mesh::{MeshTag, morph::MeshMorphWeights},
    prelude::*,
};
use humentity::prelude::*;
use shared::{CameraFraming, CustomCrowdMaterial, custom_crowd_material, setup_app_gpu};

// This type of crowd rendering is largely gpu bound and poly count matters enormously here.
// You may get a few thousand basemesh instances at acceptable framerates, but if you really
// want to crank up the crowd size you will need to use lower poly meshes like we do here.
const INSTANCES: usize = 80_000;

fn main() {
    let mut app = setup_app_gpu(INSTANCES, 30.0, CameraFraming::Far);
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
    lod_parts: [Handle<MhcloAsset>; 4],
    template: Handle<CharacterTemplate>,
    idle_clip: Handle<AnimationClip>,
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
    let lod_part_handles = [
        asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy"),
        asset_server.load::<MhcloAsset>("proxymeshes/proxy4817/proxy4817.proxy"),
        asset_server.load::<MhcloAsset>("proxymeshes/proxy1605/proxy1605.proxy"),
        asset_server.load::<MhcloAsset>("proxymeshes/proxy741/proxy741.proxy"),
    ];
    for lod_part_handle in &lod_part_handles {
        mesh_builder.trigger(LoadAssetMeshJob::Single {
            part: lod_part_handle.clone(),
            template_handle: template.clone(),
            skeleton_lod: MeshBuildLod::Gpu,
        });
    }
    // Native glTF clip: every translation track intact, so the pose shader
    // can retarget bone translations per shape per frame.
    let idle_clip =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("animation/idle.glb"));
    commands.insert_resource(CrowdBuild {
        lod_parts: lod_part_handles,
        template,
        idle_clip,
    });
    info!("crowd build triggered");
}

/// Manual clip load: retries every frame until the bank accepts the idle
/// clip. Nothing else queues loads.
fn request_clip_bakes(
    bank: Option<ResMut<GpuAnimationBank>>,
    build: Option<Res<CrowdBuild>>,
    clips: Res<Assets<AnimationClip>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(mut bank), Some(build)) = (bank, build) else {
        return;
    };
    if clips.get(&build.idle_clip).is_none() {
        return;
    }
    // Level-driven, not event-driven: a bank that is missing (base bake not
    // done) or full must not lose the load, so keep asking until accepted.
    if !bank.is_loaded("Idle-loop")
        && !bank.is_baking("Idle-loop")
        && !bank.request_load_handle("Idle-loop", GpuClipMode::Loop, build.idle_clip.clone())
    {
        return;
    }
    *done = true;
}

fn spawn_crowd(
    mut commands: Commands,
    build: Option<Res<CrowdBuild>>,
    handles: Option<Res<GpuRenderHandles>>,
    bank: Option<Res<GpuAnimationBank>>,
    cached: Res<CachedMhcloMeshHandles>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<CustomCrowdMaterial>>,
    asset_server: Res<AssetServer>,
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
    // The builder caches each converted GPU mesh under `MeshBuildLod::Gpu`,
    // so the spawn system reads them directly instead of converting per spawn.
    let mut lod_mesh_handles = Vec::with_capacity(4);
    for lod_part in &build.lod_parts {
        let Some(lod_mesh_handle) = cached
            .get(&(lod_part.clone(), build.template.clone(), MeshBuildLod::Gpu))
            .cloned()
        else {
            return;
        };
        if meshes.get(&lod_mesh_handle).is_none() {
            return;
        }
        lod_mesh_handles.push(lod_mesh_handle);
    }
    let crowd_material_handle = custom_crowd_material(&handles, &mut materials);
    let skin_albedo_texture: Handle<Image> =
        asset_server.load("skin_textures/albedo/young_caucasian_female.png");
    if let Some(mut crowd_material) = materials.get_mut(&crowd_material_handle) {
        crowd_material.base.base_color_texture = Some(skin_albedo_texture);
    }
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
        let crowd_transform = Transform::from_xyz(
            col as f32 * spacing - offset,
            0.0,
            row as f32 * spacing - offset + 8.0,
        );
        for (lod_level, lod_mesh_handle) in lod_mesh_handles.iter().enumerate() {
            let lod_visibility = match lod_level {
                0 => VisibilityRange {
                    start_margin: 0.0..0.0,
                    end_margin: 2.0..3.0,
                    use_aabb: false,
                },
                1 => VisibilityRange {
                    start_margin: 2.0..3.0,
                    end_margin: 7.0..8.0,
                    use_aabb: false,
                },
                2 => VisibilityRange {
                    start_margin: 7.0..8.0,
                    end_margin: 14.0..15.0,
                    use_aabb: false,
                },
                _ => VisibilityRange {
                    start_margin: 14.0..15.0,
                    end_margin: 20.0..200.0,
                    use_aabb: false,
                },
            };
            commands.spawn((
                Name::new(format!("GpuChar {index} lod{lod_level}")),
                crowd_transform,
                Mesh3d(lod_mesh_handle.clone()),
                MeshMaterial3d(crowd_material_handle.clone()),
                MeshTag(index as u32),
                // Neutral morph entry: the GPU meshes carry morph targets via
                // `make_gpu_mesh`, and `SetMeshBindGroup` derives the bind group
                // key per entity — without this it resolves to `NoMorphTargets`
                // (`model_only`) while the layout expects the morphed group.
                MeshMorphWeights::Value { weights: vec![0.0] },
                lod_visibility,
            ));
        }
    }
    info!("spawned {count} GPU-posed characters x 4 LODs sharing one material");
}
