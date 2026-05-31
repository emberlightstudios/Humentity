#![allow(dead_code)]
use bevy::{
    input::mouse::MouseMotion, mesh::morph::MeshMorphWeights, mesh::skinning::SkinnedMesh,
    prelude::*,
};
use bevy_egui::prelude::*;
use bevy_inspector_egui::quick::WorldInspectorPlugin;
use humentity::prelude::*;

/// Marks an entity as representing a character mesh piece.
#[derive(Component, Clone, Debug, Eq, PartialEq, Hash, Deref)]
pub struct CharacterPart(pub Handle<MhcloAsset>);

/// Add just the [HumentityPlugin] without egui/inspector plugins.
/// Use this in examples that add their own UI or physics plugins.
pub fn add_humentity_plugin(app: &mut App) {
    app.add_plugins(HumentityPlugin);
}

pub fn setup_app() -> App {
    // I moved target.json and macro.macro to the root of the assets folder because when trying to load
    // the target folders, the asset server tried to load them there also.

    let mut app = App::new();

    app.add_plugins((DefaultPlugins, HumentityPlugin))
        .add_plugins((EguiPlugin::default(), WorldInspectorPlugin::new()))
        .add_systems(Startup, load_assets)
        .add_systems(Startup, setup_env)
        .add_systems(Update, (update_mesh_when_ready, cam_controls, add_material));

    app
}

/// These assets are necessary to get the plugin to work.
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

fn update_mesh_when_ready(
    character_parts: Query<(Entity, &ChildOf, &CharacterPart), Without<Mesh3d>>,
    characters: Query<(&CharacterShape, &SkinnedMesh)>,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    template_overrides: Query<&TemplateOverride>,
    cached_meshes: Res<CachedMhcloMeshHandles>,
    meshes: Res<Assets<Mesh>>,
    template_assets: Res<Assets<CharacterTemplate>>,
    mut commands: Commands,
) {
    for (entity, parent, part) in character_parts.iter() {
        let mhclo_handle = part.0.clone();
        let Ok((character_shape, skm)) = characters.get(parent.parent()) else {
            continue;
        };
        let Some(asset) = shape_assets.get(&character_shape.0) else {
            continue;
        };
        let template = &asset.template;

        // After you trigger a mesh build it will be put in this cache
        if let Some(mesh_handle) = cached_meshes.get(&(mhclo_handle, template.clone())) {
            commands.entity(entity).insert((
                Mesh3d(mesh_handle.clone()),
                Transform::IDENTITY, // I think this is necessary
                skm.clone(),         // This is necessary to bind the mesh to the skeleton
            ));
            let mesh = meshes.get(mesh_handle).unwrap();
            if mesh.has_morph_targets() {
                let active_template = template_overrides
                    .get(entity)
                    .ok()
                    .map_or(template.clone(), |o| o.0.clone());
                if let Some(template_data) = template_assets.get(&active_template) {
                    let morph_weights = template_data
                        .shapes
                        .iter()
                        .map(|s| *asset.template_morph_targets.get(s.name).unwrap_or(&0.))
                        .collect::<Vec<_>>();
                    commands
                        .entity(entity)
                        .insert(MeshMorphWeights::new(morph_weights).unwrap());
                }
            }
        }
    }
}

pub fn cam_controls(
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    kb_input: Res<ButtonInput<KeyCode>>,
    mut _pitch: Local<f32>,
    mut _yaw: Local<f32>,
    mut init: Local<bool>,
) {
    if !*init {
        *init = true;
        *_yaw = std::f32::consts::PI;
    }
    const MS: f32 = 1e-1;
    const LS: f32 = 5e-3;
    let Ok(transform) = cam.single().cloned() else {
        return;
    };
    let Ok(mut cam) = cam.single_mut() else {
        return;
    };
    for _ev in mouse_motion.read() {
        //*_yaw -= ev.delta.x * LS;
        //*_pitch -= ev.delta.y * LS;
    }
    cam.rotation = Quat::from_euler(EulerRot::YXZ, *_yaw, *_pitch, 0.);
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
        Name::new("Floor"),
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
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-1.0, 2.0, -4.0).looking_at(Vec3::Y * 0.7, Vec3::Y),
    ));
}
