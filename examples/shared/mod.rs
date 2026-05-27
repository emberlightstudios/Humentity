#![allow(dead_code)]
use bevy::{
    input::mouse::MouseMotion, mesh::skinning::SkinnedMesh, prelude::*,
};
use humentity::prelude::*;
use bevy_inspector_egui::quick::WorldInspectorPlugin;
use bevy_egui::prelude::*;

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
        .add_systems(
            Update,
            (
                update_mesh_when_ready.run_if(resource_exists::<CharacterTemplates>),
                cam_controls,
                add_material,
            ),
        );

    app
}

/// These assets are necessary to get the plugin to work.
fn load_assets(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(MakeHumanMorphs::new(
        &asset_server,
        "target.json",
        "macro.macro",
        "targets",
    ));
    commands.insert_resource(RigData::new(
        &asset_server,
        "rigs/rig.default.json",
        "rigs/weights.default.json",
        "skeletons/default.glb",
    ));
    commands.insert_resource(BaseMesh::new(&asset_server, "base.obj"));
    commands.insert_resource(VertexGroups::new(&asset_server, "basemesh_vertex_groups.json"));
}

fn update_mesh_when_ready(
    character_parts: Query<(Entity, &ChildOf, &CharacterPart), Without<Mesh3d>>,
    characters: Query<(&CharacterShapeConfig, &SkinnedMesh)>,
    template_overrides: Query<&TemplateOverride>,
    cached_meshes: Res<CachedMhcloMeshHandles>,
    meshes: Res<Assets<Mesh>>,
    templates: Res<CharacterTemplates>,
    mut commands: Commands,
) {
    for (entity, parent, part) in character_parts.iter() {
        let mhclo_handle = part.0.clone();
        let Ok((shape_config, skm)) = characters.get(parent.parent()) else {
            continue;
        };
        let template = shape_config.template;

        // After you trigger a mesh build it will be put in this cache
        if let Some(mesh_handle) = cached_meshes.get(&(mhclo_handle, template)) {
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
                    .map_or(template, |o| o.0);
                commands
                    .entity(entity)
                    .insert(shape_config.get_morph_weights_component(&templates[active_template]));
            }
        }
    }
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
        *yaw = std::f32::consts::PI;
    }
    const MS: f32 = 1e-1;
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
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-1.0, 2.0, -4.0).looking_at(Vec3::Y * 0.7, Vec3::Y),
    ));
}
