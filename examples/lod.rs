//! Makehuman comes with several lower poly proxy meshes.  These
//! can be used for lods with the VisibilityRanges component
//! In this example we switch lods early just for clarity. 

mod shared;
use shared::cam_controls;
use ahash::AHashMap;
use bevy::{camera::visibility::VisibilityRange, prelude::*};
use humentity::prelude::*;

fn main() {
    info!("Use WASDQE to move the camera around");
    App::new()
        .add_plugins((
            Humentity::new(HumentityPathsConfig::new("./")),
            DefaultPlugins,
        ))
        .add_systems(Startup, setup_env)
        .add_systems(Update, (cam_controls, add_material))
        .add_systems(OnExit(HumentityLoadState::LoadingCoreAssets), setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_humans)
        .run();
}

// this is just a marker component to control which mesh has lods. 
#[derive(Component)]
struct LevelOfDetail;

fn add_humans(
    mut commands: Commands,
) {
    // Previously defined shapes will now appear as morph targets on the prefab's mesh
    // The HumanConfig type controls prefab access and applies our morph targets.
    let prefab_name = Name::new("ExampleHumanPrefab");
    let bodybuilder = Name::new("bodybuilder");
    let mut morphs = MorphTargets::default();
    morphs.insert(bodybuilder.clone(), 1.);

    // Base mesh will be lod0
    let lod1 = Name::new("male_generic");
    let lod2 = Name::new("male1591");
    let lod3 = Name::new("proxy741");

    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 1.)),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        LevelOfDetail, // Just a marker component for this example
        InheritedVisibility::default(),
        children![(
            HumanPart::BaseMesh,
            VisibilityRange {
                start_margin: 0.0..0.0,
                end_margin: 2.0..2.,
                use_aabb: false,
            }
        ), (
            HumanPart::ProxyMesh(lod1.clone()),
            VisibilityRange {
                start_margin: 2.0..2.0,
                end_margin: 4.0..4.0,
                use_aabb: false,
            }
        ), (
            HumanPart::ProxyMesh(lod2.clone()),
            VisibilityRange {
                start_margin: 4.0..4.,
                end_margin: 6.0..6.,
                use_aabb: false,
            }
        ), (
            HumanPart::ProxyMesh(lod3.clone()),
            VisibilityRange {
                start_margin: 6.0..6.0,
                end_margin: 8.0..10.0,
                use_aabb: false,
            }
        )]
    ));

    // Just for comparison we'll spawn the proxies used here
    // The base mesh (highest poly-count ~19k tris)
    commands.spawn((
        Transform::from_translation(Vec3::new(-1.5, 0., 0.)),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        InheritedVisibility::default(),
        children![(
            HumanPart::BaseMesh
        )]
    ));

    // male_generic (high poly-count 13k tris)
    commands.spawn((
        Transform::from_translation(Vec3::new(-0.5, 0., 0.)),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        InheritedVisibility::default(),
        children![(
            HumanPart::ProxyMesh(lod1)
        )]
    ));

    //  male1591 (low poly-count)
    commands.spawn((
        Transform::from_translation(Vec3::new(0.5, 0., 0.)),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        InheritedVisibility::default(),
        children![(
            HumanPart::ProxyMesh(lod2)
        )]
    ));

    // proxy741 (very low poly-count)
    commands.spawn((
        Transform::from_translation(Vec3::new(1.5, 0., 0.)),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        InheritedVisibility::default(),
        children![(
            HumanPart::ProxyMesh(lod3)
        )]
    ));

    // There are also female specific proxies which may have better topology for breasts
}

// Adds a simple black material to humans
fn add_material(
    humans: Query<(Entity, &ChildOf), (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>)>,
    lod: Query<Entity, With<LevelOfDetail>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let black = materials.add(StandardMaterial::from_color(Color::BLACK));
    let white = materials.add(StandardMaterial::from_color(Color::WHITE));
    for (human, parent) in humans {
        if lod.get(parent.parent()).is_ok() {
            commands.entity(human).insert(MeshMaterial3d(white.clone()));
        } else {
            commands.entity(human).insert(MeshMaterial3d(black.clone()));
        }
    }
}

fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    morphs: Res<HumanMorphs>,
) {
    // circular base
    let mesh = meshes.add(Circle::new(4.0));
    let material = materials.add(Color::WHITE);

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material.clone()),
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
        Transform::from_xyz(0.0, 1.0, 5.0),
    ));

    // A camera:
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 2.0, 4.0).looking_at(Vec3::Y * 0.7, Vec3::Y),
    ));
}

fn setup_prefabs(mut commands: Commands, morphs: Res<HumanMorphs>) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert(Name::new("asian"), 1.);
    morph_targets.insert(Name::new("weight"), 1.);
    morph_targets.insert(Name::new("muscle"), 1.);
    let bodybuilder_shape = HumanShapeArchetype::new(
        Name::new("bodybuilder"),
        morphs.compute_target_weights(&morph_targets),
    );

    let mut prefabs = AHashMap::default();
    prefabs.insert(
        Name::new("ExampleHumanPrefab"),
        HumanArchetypePrefab::new(
            vec![bodybuilder_shape],
            HumanAnimationArchetype::default(), // No animation in this example
        )
    );

    commands.insert_resource(HumanArchetypePrefabs::new(prefabs));
}