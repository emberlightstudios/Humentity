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
        transform: Transform::from_xyz(-0.5, 1.5, 3.5).looking_at(Vec3::ZERO + Vec3::Y * 1.0, Vec3::Y),
        ..default()   
    });
    // set up humans
    let mut shapekeys = HashMap::<String, f32>::new();
    shapekeys.insert("african-female-baby".to_string(), 1.0);
    let params = LoadHumanParams {
        shapekeys: shapekeys,
        skin_albedo: "young_african_female_diffuse.png".to_string(),
        transform: Transform::from_xyz(-1.5, 0.0, 0.0),
        rig: RigType::Mixamo,
    };
    let human: Entity = commands.spawn(HumanEntityTag).id();
    commands.trigger_targets(params, human);
    
    let mut shapekeys = HashMap::<String, f32>::new();
    shapekeys.insert("asian-male-child".to_string(), 1.0);
    let params = LoadHumanParams {
        shapekeys: shapekeys,
        skin_albedo: "young_asian_male_diffuse3.png".to_string(),
        transform: Transform::from_xyz(-0.5, 0.0, 0.0),
        rig: RigType::Mixamo,
    };
    let human: Entity = commands.spawn(HumanEntityTag).id();
    commands.trigger_targets(params, human);

    let mut shapekeys = HashMap::<String, f32>::new();
    shapekeys.insert("caucasian-female-young".to_string(), 1.0);
    let params = LoadHumanParams {
        shapekeys: shapekeys,
        skin_albedo: "young_caucasian_female_diffuse.png".to_string(),
        transform: Transform::from_xyz(0.5, 0.0, 0.0),
        rig: RigType::Mixamo,
    };
    let human: Entity = commands.spawn(HumanEntityTag).id();
    commands.trigger_targets(params, human);

    let mut shapekeys = HashMap::<String, f32>::new();
    shapekeys.insert("african-male-old".to_string(), 1.0);
    let params = LoadHumanParams {
        shapekeys: shapekeys,
        skin_albedo: "old_african_male_diffuse.png".to_string(),
        transform: Transform::from_xyz(1.5, 0.0, 0.0),
        rig: RigType::Mixamo,
    };
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