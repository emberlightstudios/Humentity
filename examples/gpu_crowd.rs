//! Basic GPU crowd: 8000 characters posed entirely on the GPU.
//!
//! The idle clip is baked once to local bone matrices on the single crowd skeleton. Every
//! frame a compute shader poses all instances x bones and the joint buffer
//! feeds the skinning vertex shader bindlessly. The main world holds only
//! static `Transform`s: no skeletons, no `AnimationPlayer`, no transform
//! propagation for the crowd.

mod shared;

use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    mesh::{MeshTag, VertexAttributeValues},
    prelude::*,
};
use humentity::load_and_insert_humentity_assets;
use humentity::prelude::*;
use shared::{CustomCrowdMaterial, GPU_SKELETON_LOD, custom_crowd_material, setup_app_gpu};

const INSTANCES: usize = 8_000;

fn main() {
    let mut app = setup_app_gpu(INSTANCES, 30.0);
    app.add_systems(Startup, (load_assets, setup_scene, setup_fps_text))
        .add_systems(
            Update,
            (
                trigger_crowd_build.run_if(resource_added::<HumentityAssetsReady>),
                spawn_crowd,
                update_fps_text,
            ),
        )
        .run();
}

#[derive(Resource)]
struct CrowdBuild {
    part: Handle<MhcloAsset>,
    template: Handle<CharacterTemplate>,
    _clips: Handle<RetargetedAnimationAsset>,
}

#[derive(Component)]
struct FpsText;

fn load_assets(asset_server: Res<AssetServer>, mut commands: Commands) {
    load_and_insert_humentity_assets(
        &mut commands,
        &asset_server,
        "base.obj",
        "basemesh_vertex_groups.json",
        "target.json",
        "macro.macro",
        "targets",
        "rigs/rig.default.json",
        "rigs/weights.default.json",
        "skeletons/default.glb",
    );
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(80.0, 80.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.2, 0.2, 0.25))),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 3000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(10.0, 20.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 18.0, -30.0).looking_at(Vec3::new(0.0, 1.0, 8.0), Vec3::Y),
    ));
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
        _clips: clips,
    });
    info!("crowd build triggered");
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
    // TEMP DEBUG: verify mesh joint indices fit the baked LOD bone count.
    if let Some(VertexAttributeValues::Uint16x4(indices)) =
        source.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
    {
        let max_idx = indices.iter().flatten().copied().max().unwrap_or(0);
        info!(
            "crowd mesh verts {} max joint index {max_idx}",
            indices.len()
        );
    }
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

fn setup_fps_text(mut commands: Commands) {
    commands.spawn((
        FpsText,
        Text::new("FPS: --"),
        TextLayout::justify(Justify::Right),
        TextFont {
            font_size: FontSize::Px(29.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(4.0),
            right: Val::Px(4.0),
            ..default()
        },
    ));
}

fn update_fps_text(diagnostics: Res<DiagnosticsStore>, mut query: Query<&mut Text, With<FpsText>>) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    for mut text in &mut query {
        **text = format!("GPU crowd FPS: {fps:.1}");
    }
}
