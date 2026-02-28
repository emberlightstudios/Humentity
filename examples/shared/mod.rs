#![allow(dead_code)]
use std::path::PathBuf;

use ahash::AHashSet;
use avian3d::math::PI;
use bevy::{asset::io::AssetSourceBuilder, input::mouse::MouseMotion, prelude::*};
use humentity::prelude::*;

pub fn add_humentity_plugin(app: &mut App) {
    let paths = build_humentity_custom_source_paths(app);

    app.add_plugins(HumentityPlugin {
        paths,
        config: HumentityGlobalConfig {
            debug_draw_bones: true,
            translation_tracks: TranslationTracks::None,
        },
    });
}

pub fn build_humentity_custom_source_paths(app: &mut App) -> HumentityPathsConfig {
    /// We will build the path configs for included assets with a custom source
    const ASSET_SOURCE_ID: &str = "humentity";

    // In this case it's just the default assets folder in this crate.
    // This is just for example
    let path = "./assets";

    app.register_asset_source(
        ASSET_SOURCE_ID,
        AssetSourceBuilder::platform_default(path, None),
    );

    let humentity_source = HumentityAssetSourceId::new(
        // None if using default asset source
        Some(ASSET_SOURCE_ID),
        // This would still be required in case default asset source path has changed
        PathBuf::from(path),
    );

    // Set up paths to all asset types.  Assets in folders will be scanned and imported into the CharacterAssetRegistry

    // ProxyMeshes/body lod
    let mut body_mesh_paths = AHashSet::default();
    // Hair, eyes, eyebrows, etc
    let mut body_part_paths = AHashSet::default();
    // Clothes, armor, etc
    let mut equipment_paths = AHashSet::default();
    // Skin textures
    let mut skin_texture_paths = AHashSet::default();
    // Morph target files, exported from Makehuman/MPFB
    let mut target_paths = AHashSet::default();

    // These paths must be relative to the source root folder
    body_mesh_paths.insert(HumentityAssetPath::new("./proxymeshes", &humentity_source));
    body_part_paths.insert(HumentityAssetPath::new("./body_parts", &humentity_source));
    equipment_paths.insert(HumentityAssetPath::new("./clothes", &humentity_source));
    skin_texture_paths.insert(HumentityAssetPath::new(
        "./skin_textures",
        &humentity_source,
    ));
    target_paths.insert(PathBuf::from(path).join("targets"));

    HumentityPathsConfig::new(
        humentity_source.root_path.clone(),
        body_mesh_paths,
        body_part_paths,
        equipment_paths,
        skin_texture_paths,
        target_paths,
    )
}

pub fn cam_controls(
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    kb_input: Res<ButtonInput<KeyCode>>,
    mut pitch: Local<f32>,
    mut yaw: Local<f32>,
    mut init: Local<bool>,
) {
    if !*init {
        *init = true;
        *pitch = -0.15;
        *yaw = 3.3;
    }
    const MS: f32 = 1e-2;
    const LS: f32 = 5e-3;
    let Ok(transform) = cam.single().cloned() else {
        return;
    };
    let Ok(mut cam) = cam.single_mut() else {
        return;
    };
    for ev in mouse_motion.read() {
        //*yaw -= ev.delta.x * LS;
        //*pitch -= ev.delta.y * LS;
    }
    cam.rotation = Quat::from_euler(EulerRot::YXZ, *yaw, *pitch, 0.);
    let mut mv = Vec3::ZERO;
    if kb_input.pressed(KeyCode::KeyD) {
        mv.x += MS
    }
    if kb_input.pressed(KeyCode::KeyA) {
        mv.x -= MS
    }
    if kb_input.pressed(KeyCode::KeyS) {
        mv.z += MS
    }
    if kb_input.pressed(KeyCode::KeyW) {
        mv.z -= MS
    }
    if kb_input.pressed(KeyCode::KeyQ) {
        mv.y -= MS
    }
    if kb_input.pressed(KeyCode::KeyE) {
        mv.y += MS
    }
    cam.translation += Transform::from_rotation(transform.rotation) * mv;
}

pub fn add_material(
    humans: Query<Entity, (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for human in humans.iter() {
        let mat = materials.add(StandardMaterial::from_color(Color::BLACK));
        commands.entity(human).insert(MeshMaterial3d(mat));
    }
}

pub fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // circular base
    let mesh = meshes.add(Circle::new(3.0));
    let material = materials.add(Color::WHITE);

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material.clone()),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
    ));

    // A light:
    commands.spawn((
        PointLight {
            intensity: 8_000_0.0,
            radius: 19.,
            range: 19.,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(-1.0, 3.0, -5.0),
    ));

    // A camera:
    commands.spawn((Camera3d::default(), Transform::from_xyz(-1.0, 2.0, -4.0)));
}
