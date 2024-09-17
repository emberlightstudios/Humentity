use bevy::prelude::*;
use humentity::{prelude::*, HumentityState};
use std::collections::HashMap;

#[derive(Component)]
struct HumanEntityTag;

fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // circular base
    commands.spawn(PbrBundle {
        mesh: meshes.add(Circle::new(4.0)),
        material: materials.add(Color::WHITE),
        transform: Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ..default()
    });
    // point light
    commands.spawn(PointLightBundle {
        point_light: PointLight {
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 8.0, 5.0),
        ..default()
    });
    // camera
    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(-0.5, 1.5, 2.5).looking_at(Vec3::ZERO + Vec3::Y * 1.0, Vec3::Y),
        //transform: Transform::from_xyz(-2.5, 2.5, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()   
    });
    // set up human params
    let mut shapekeys = HashMap::<String, f32>::new();
    shapekeys.insert("african-male-old".to_string(), 1.0);
    let params = LoadHumanParams {
        shapekeys: shapekeys,
        skin_albedo: "young_african_male_diffuse.png".to_string(),
        transform: Transform::from_xyz(0.0, 0.0, 0.0),
        rig: RigType::Mixamo,
    };
    // spawn human
    let human: Entity = commands.spawn(HumanEntityTag).id();
    commands.trigger_targets(params, human);
    
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(Humentity::default())
        .add_systems(OnEnter(HumentityState::Ready), setup_env)
        .run();
}