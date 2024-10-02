use bevy::prelude::*;
use humentity::prelude::*;
use std::{
    collections::HashMap,
    path::Path,
};

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
    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_xyz(-0.5, 1.5, 3.5).looking_at(Vec3::ZERO + Vec3::Y * 1.0, Vec3::Y),
            projection: PerspectiveProjection {
                fov: std::f32::consts::PI / 4.0, // Field of view
                near: 0.001, // Near clipping distance
                far: 1000.0, // Far clipping distance
                ..Default::default()
            }.into(),
            ..default()   
        },
    ));

    // set up humans
    for i in [0,1,2,3].iter() {
        let mut shapekeys = HashMap::<String, f32>::new();
        let config: HumanConfig;
        match i {
            0 => {
                shapekeys.insert("african-female-baby".to_string(), 1.0);
                config = HumanConfig {
                    morph_targets: shapekeys,
                    skin_albedo: "young_african_female_diffuse.png".to_string(),
                    body_parts: vec![
                        "LeftEyeballLowPoly".to_string(),
                        "LeftEyelash".to_string(),
                        "LeftEyebrow-001".to_string(),
                        "RightEyeballLowPoly".to_string(),
                        "RightEyelash".to_string(),
                        "RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                        "SimpleBra".to_string(),
                        "SimpleBriefs".to_string(),
                    ],
                    ..default()
                }
            }
            1 => {
                shapekeys.insert("asian-male-child".to_string(), 1.0);
                config = HumanConfig {

                    morph_targets: shapekeys,
                    skin_albedo: "young_asian_male_diffuse3.png".to_string(),
                    body_parts: vec![
                        "LeftEyeballLowPoly".to_string(),
                        "LeftEyelash".to_string(),
                        "LeftEyebrow-001".to_string(),
                        "RightEyeballLowPoly".to_string(),
                        "RightEyelash".to_string(),
                        "RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                        "SimpleBriefs".to_string(),
                    ],
                    ..default()
                }
            }
            2 => {
                shapekeys.insert("caucasian-female-young".to_string(), 1.0);
                config = HumanConfig {
                    morph_targets: shapekeys,
                    skin_albedo: "middleage_caucasian_female_diffuse.png".to_string(),
                    body_parts: vec![
                        "LeftEyeballLowPoly".to_string(),
                        "FalseLeftEyelash".to_string(),
                        "LeftEyebrow-001".to_string(),
                        "RightEyeballLowPoly".to_string(),
                        "FalseRightEyelash".to_string(),
                        "RightEyebrow-001".to_string(),
                        "Ponytail01".to_string(),
                    ],
                    equipment: vec![
                        "SimpleBra".to_string(),
                        "SimpleBriefs".to_string(),
                    ],
                    hair_color: Color::linear_rgb(1.0, 0.2, 0.4),
                    ..default()
                }
            }
            3 => {
                shapekeys.insert("african-male-old".to_string(), 1.0);
                config = HumanConfig {
                    morph_targets: shapekeys,
                    skin_albedo: "old_african_male_diffuse.png".to_string(),
                    body_parts: vec![
                        "LeftEyeballLowPoly".to_string(),
                        "LeftEyelash".to_string(),
                        "LeftEyebrow-001".to_string(),
                        "RightEyeballLowPoly".to_string(),
                        "RightEyelash".to_string(),
                        "RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                        "SimpleBriefs".to_string(),
                    ],
                    ..default()
                }
            }
            _  => { panic!{"uninitialized human"}; }
        }
        let transform = Transform::from_xyz(*i as f32 - 1.5, 0.0, 0.0);
        commands.spawn((
            SpawnTransform(transform),
            config,
            AnimationPlayer::default(),
        ));
    }
}


fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(
            HumentityGlobalConfig::default()
                .with_animation_libraries(AnimationLibrarySettings {
                    paths: vec![Path::new("./assets").to_path_buf()],
                    rig_type: RigType::Mixamo,
                })
        )
        .add_plugins(Humentity{ debug: true })
        .add_systems(OnEnter(HumentityState::Ready), setup_env)
        .run();
}

