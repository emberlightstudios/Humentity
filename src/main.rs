use bevy::prelude::*;
use humentity::{prelude::*, HumentityState};
use std::collections::HashMap;


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
                    rig: RigType::Mixamo,
                    body_parts: vec![
                        "LeftEye-EyeballLowPoly".to_string(),
                        "LeftEyelash".to_string(),
                        "LeftEyebrow-001".to_string(),
                        "RightEye-EyeballLowPoly".to_string(),
                        "RightEyelash".to_string(),
                        "RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                    ],
                }
            }
            1 => {
                shapekeys.insert("asian-male-child".to_string(), 1.0);
                config = HumanConfig {

                    morph_targets: shapekeys,
                    skin_albedo: "young_asian_male_diffuse3.png".to_string(),
                    rig: RigType::Mixamo,
                    body_parts: vec![
                        "LeftEye-EyeballLowPoly".to_string(),
                        "LeftEyelash".to_string(),
                        "LeftEyebrow-001".to_string(),
                        "RightEye-EyeballLowPoly".to_string(),
                        "RightEyelash".to_string(),
                        "RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                    ],
                }
            }
            2 => {
                shapekeys.insert("caucasian-female-young".to_string(), 1.0);
                config = HumanConfig {
                    morph_targets: shapekeys,
                    skin_albedo: "middleage_caucasian_female_diffuse.png".to_string(),
                    rig: RigType::Mixamo,
                    body_parts: vec![
                        "LeftEye-EyeballLowPoly".to_string(),
                        "LeftEyelash".to_string(),
                        "LeftEyebrow-001".to_string(),
                        "RightEye-EyeballLowPoly".to_string(),
                        "RightEyelash".to_string(),
                        "RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                        "female_panties_01".to_string(),
                    ],
                }
            }
            3 => {
                shapekeys.insert("african-male-old".to_string(), 1.0);
                config = HumanConfig {
                    morph_targets: shapekeys,
                    skin_albedo: "old_african_male_diffuse.png".to_string(),
                    rig: RigType::Mixamo,
                    body_parts: vec![
                        "LeftEye-EyeballLowPoly".to_string(),
                        "LeftEyelash".to_string(),
                        "LeftEyebrow-001".to_string(),
                        "RightEye-EyeballLowPoly".to_string(),
                        "RightEyelash".to_string(),
                        "RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                    ],
                }
            }
            _  => { panic!{"uninitialized human"}; }
        }
        let transform = Transform::from_xyz(*i as f32 - 1.5, 0.0, 0.0);
        commands.spawn((
            SpawnTransform(transform),
            config,
        ));
    }
}


fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(HumentityGlobalConfig::default())
        .add_plugins(Humentity::default())
        .add_systems(OnEnter(HumentityState::Ready), setup_env)
        .run();
}