//! GPU locomotion blend: 6000 characters blending forward walk with strafe
//! right entirely on the GPU.
//!
//! The `normal-walk` and `normal-walk-strafe-right` clips from
//! `assets/animation/movement_normal.glb` are baked once on the single crowd
//! skeleton; every frame the pose shader blends them with per-instance
//! [`GpuInstanceAnims`] weights. This example sweeps the weights on a sine so
//! the blend is visible, and shows the live mix in the FPS readout.

mod shared;

use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    mesh::MeshTag,
    prelude::*,
};
use humentity::load_and_insert_humentity_assets;
use humentity::prelude::*;
use shared::{CustomCrowdMaterial, GPU_SKELETON_LOD, custom_crowd_material, setup_app_gpu};

const WALK: &str = "normal-walk";
const STRAFE_RIGHT: &str = "normal-walk-strafe-right";
const INSTANCES: usize = 6_000;

fn main() {
    let mut app = setup_app_gpu(INSTANCES, 30.0);
    app.insert_resource(GpuBlendClips {
        names: vec![WALK.to_string(), STRAFE_RIGHT.to_string()],
    });
    app.insert_resource(GpuBlendWeights([0.5, 0.5, 0.0, 0.0]));
    app.add_systems(Startup, (load_assets, setup_scene, setup_fps_text))
        .add_systems(
            Update,
            (
                trigger_crowd_build.run_if(resource_exists::<HumentityAssetsReady>),
                spawn_crowd,
                sweep_blend_weights,
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

fn setup_fps_text(mut commands: Commands) {
    commands.spawn((
        FpsText,
        Text::new("FPS: --"),
        TextLayout::justify(Justify::Right),
        TextFont {
            font_size: FontSize::Px(30.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(5.0),
            right: Val::Px(5.0),
            ..default()
        },
    ));
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
    let part = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: part.clone(),
        template_handle: template.clone(),
        skeleton_lod: GPU_SKELETON_LOD,
    });
    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/movement_normal.glb");
    commands.insert_resource(CrowdBuild {
        part,
        template,
        _clips: clips,
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

fn update_fps_text(
    diagnostics: Res<DiagnosticsStore>,
    anims: Option<Res<GpuInstanceAnims>>,
    mut query: Query<&mut Text, With<FpsText>>,
) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let (walk, strafe) = anims
        .and_then(|a| a.targets.first().copied())
        .map(|t| (t[0], t[1]))
        .unwrap_or((0.5, 0.5));
    for mut text in &mut query {
        **text = format!(
            "FPS: {fps:.1}  walk {:.0}% / strafe {:.0}%",
            walk * 100.0,
            strafe * 100.0
        );
    }
}
