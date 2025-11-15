mod shared;
use bevy::prelude::*;
use humentity::prelude::*;
use avian3d::prelude::*;
use shared::{setup_env, cam_controls, add_material};

fn main() {
    App::new()
        .add_plugins((
            Humentity {
                paths: HumentityPathsConfig::from_crate_path("./"),
                config: HumentityGlobalConfig {
                    translation_tracks: TranslationTracks::None,
                    ..default()
                }
            },
            DefaultPlugins,
            PhysicsPlugins::default(),
            PhysicsDebugPlugin,
        ))
        .add_systems(Startup, (setup_env, floor))
        .add_systems(OnExit(HumentityLoadState::LoadingCoreAssets), setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_human)
        .add_systems(Update, (cam_controls, add_material, toggle))
        .run();
}

fn toggle(input: Res<ButtonInput<KeyCode>>, mut ragdolls: Query<&mut CharacterRagdoll>) {
    if input.just_pressed(KeyCode::Space) {
        if let Ok(mut ragdoll) = ragdolls.single_mut() {
            ragdoll.active = !ragdoll.active;
        }
    }
}

fn floor(mut commands: Commands) {
    commands.spawn((
        Collider::cuboid(6., 1., 6.),
        RigidBody::Static,
        Transform::from_translation(Vec3::Y * -0.5),
    ));
}

fn add_human(
    mut commands: Commands,
) {
    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0.2, 0.)),
        CharacterShapeConfig::default(),
        InheritedVisibility::default(),
        CharacterRagdoll::new(false),
        children![(
            CharacterPart::BaseMesh
        )],
    ));
}

fn setup_prefabs(mut commands: Commands) {
    // No shape morphs, just the basemesh
    // Just for the examples.
    commands.insert_resource(CharacterArchetypePrefabs::basemesh());
}