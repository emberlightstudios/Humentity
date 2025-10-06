use bevy::{input::mouse::MouseMotion, prelude::*};
use bevy_obj::ObjPlugin;
use humentity::prelude::*;
use std::{path::Path};
use fxhash::FxHashMap;


fn main() {
    let mut app = App::new();
    app
        .add_plugins((
            Humentity {
                config: HumentityGlobalConfig::new("./")
                    .with_animation_libraries(AnimationLibrarySettings {
                        paths: vec![Path::new(".").to_path_buf()],
                        rig_type: RigType::Mixamo,
                    }),
                debug: true,
            },
            DefaultPlugins,
        ))
        .add_systems(Update, (
            setup_env.run_if(resource_removed::<HumentityLoading>),
            animate,
            cam_controls,
        ));

    app.run();
}

fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // circular base
    let mesh = meshes.add(Circle::new(4.0));
    let material = materials.add(Color::WHITE);

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
    ));

    // A light:
    commands.spawn((
        PointLight {
            intensity: 15_000_0.0,
            radius: 20.,
            range: 20.,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.0, 1.0, 3.0),
    ));

    // A camera:
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 2.0, 4.0).looking_at(Vec3::Y, Vec3::Y),
    ));

    // set up humans
    for i in [0,1,2,3].iter() {
        let mut shapekeys = FxHashMap::<String, f32>::default();
        let config: HumanConfig;
        match i {
            0 => {
                shapekeys.insert("african-female-baby".to_string(), 1.0);
                config = HumanConfig {
                    morph_targets: shapekeys,
                    skin_albedo: "young_african_female_diffuse.png".to_string(),
                    body_parts: vec![
                        //"LeftEyeballLowPoly".to_string(),
                        //"LeftEyelash".to_string(),
                        //"LeftEyebrow-001".to_string(),
                        //"RightEyeballLowPoly".to_string(),
                        //"RightEyelash".to_string(),
                        //"RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                        //"SimpleBra".to_string(),
                        //"SimpleBriefs".to_string(),
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
                        //"LeftEyeballLowPoly".to_string(),
                        //"LeftEyelash".to_string(),
                        //"LeftEyebrow-001".to_string(),
                        //"RightEyeballLowPoly".to_string(),
                        //"RightEyelash".to_string(),
                        //"RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                        //"SimpleBriefs".to_string(),
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
                        //"LeftEyeballLowPoly".to_string(),
                        //"FalseLeftEyelash".to_string(),
                        //"LeftEyebrow-001".to_string(),
                        //"RightEyeballLowPoly".to_string(),
                        //"FalseRightEyelash".to_string(),
                        //"RightEyebrow-001".to_string(),
                        //"Ponytail01".to_string(),
                    ],
                    equipment: vec![
                        //"SimpleBra".to_string(),
                        //"SimpleBriefs".to_string(),
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
                        //"LeftEyeballLowPoly".to_string(),
                        //"LeftEyelash".to_string(),
                        //"LeftEyebrow-001".to_string(),
                        //"RightEyeballLowPoly".to_string(),
                        //"RightEyelash".to_string(),
                        //"RightEyebrow-001".to_string(),
                    ],
                    equipment: vec![
                        //"SimpleBriefs".to_string(),
                    ],
                    ..default()
                }
            }
            _  => { panic!{"uninitialized human"}; }
        }
        let transform = Transform::from_xyz(*i as f32 - 1.5, 0.0, 0.0);
        commands.spawn((
            transform,
            config,
            InheritedVisibility::VISIBLE,
            AnimationPlayer::default(),
        ));
    }
}

fn animate(
    animations: Res<AnimationLibrarySet>,
) {
    for (name, library) in animations.libraries.iter() {
        println!("Library {name}");
        for (name, clip) in library.iter() {
            println!("clip {name}");
        }
    }
}

fn cam_controls(
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    kb_input: Res<ButtonInput<KeyCode>>,
    mut pitch: Local<f32>,
    mut yaw: Local<f32>,
) {
    const MS: f32 = 5e-3;
    const LS: f32 = 5e-3;
    let Ok(transform) = cam.single().cloned() else { return };
    let Ok(mut cam) = cam.single_mut() else { return };
    for ev in mouse_motion.read() {
        *yaw -= ev.delta.x * LS;
        *pitch -= ev.delta.y * LS;
    }
    cam.rotation = Quat::from_euler(EulerRot::YXZ, *yaw, *pitch, 0.);
    let mut mv = Vec3::ZERO;
    if kb_input.pressed(KeyCode::KeyD) { mv.x += MS }
    if kb_input.pressed(KeyCode::KeyA) { mv.x -= MS }
    if kb_input.pressed(KeyCode::KeyS) { mv.z += MS }
    if kb_input.pressed(KeyCode::KeyW) { mv.z -= MS }
    if kb_input.pressed(KeyCode::KeyQ) { mv.y -= MS }
    if kb_input.pressed(KeyCode::KeyE) { mv.y += MS }
    cam.translation += Transform::from_rotation(transform.rotation) * mv;
}