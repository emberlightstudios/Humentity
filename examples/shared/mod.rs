use bevy::{input::mouse::MouseMotion, prelude::*};

pub fn cam_controls(
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    kb_input: Res<ButtonInput<KeyCode>>,
    mut pitch: Local<f32>,
    mut yaw: Local<f32>,
) {
    const MS: f32 = 5e-2;
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