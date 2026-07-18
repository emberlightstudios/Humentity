use bevy::animation::AnimationTargetId;
use bevy::color::palettes::basic::RED;
use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore};
use bevy::prelude::*;

pub struct BoneDebugPlugin;

impl Plugin for BoneDebugPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, configure_gizmo_depth_bias)
            .add_systems(Update, bone_debug_draw);
    }
}

fn configure_gizmo_depth_bias(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.depth_bias = -1.0;
}

fn bone_debug_draw(
    query: Query<(&GlobalTransform, &ChildOf), With<AnimationTargetId>>,
    transforms: Query<&GlobalTransform, With<AnimationTargetId>>,
    mut gizmos: Gizmos,
) {
    query.iter().for_each(|(transform, child_of)| {
        let start = transform.translation();
        if let Ok(end) = transforms.get(child_of.parent()) {
            gizmos.line(start, end.translation(), RED);
        }
    })
}
