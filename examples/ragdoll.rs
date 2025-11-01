mod shared;
use bevy::prelude::*;
use humentity::prelude::*;
use avian3d::prelude::*;
use shared::{setup_env, cam_controls, add_material};
use ahash::AHashMap;

const PREFAB: &str = "prefab";
const TIME_SCALE: f32 = 0.01;

fn main() {
    App::new()
        .add_plugins((
            Humentity::new(HumentityPathsConfig::new("./")),
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

fn toggle(input: Res<ButtonInput<KeyCode>>, mut ragdolls: Query<&mut HumanRagdoll>) {
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
        HumanShapeConfig::new(PREFAB, MorphTargets::default()),
        InheritedVisibility::default(),
        HumanRagdoll::new(false),
        children![(
            HumanPart::BaseMesh
        )],
    ));
}

fn setup_prefabs(mut commands: Commands, morphs: Res<HumanMorphs>) {
    let morph_targets = MorphTargets::default();
    let base_shape = HumanShapeArchetype::new(
        "default",
        morphs.compute_target_weights(&morph_targets),
    );

    let mut prefabs = AHashMap::default();
    prefabs.insert(
        PREFAB,
        HumanArchetypePrefab::new(
            vec![base_shape],
            HumanAnimationArchetype::default(), // No animation in this example
        )
    );

    commands.insert_resource(HumanArchetypePrefabs::new(prefabs));
}