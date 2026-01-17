//! Makehuman comes with several lower poly proxy meshes.  These
//! can be used for lods with the VisibilityRanges component
//! In this example we switch lods early just for clarity.

mod shared;
use ahash::AHashMap;
use bevy::{camera::visibility::VisibilityRange, prelude::*};
use humentity::prelude::*;
use shared::{cam_controls, setup_env};

const PREFAB: &str = "ExamplePrefab";

fn main() {
    info!("Use WASDQE to move the camera around");
    App::new()
        .add_plugins((
            Humentity {
                paths: HumentityPathsConfig::from_crate_path("./"),
                config: HumentityGlobalConfig {
                    translation_tracks: TranslationTracks::None,
                    ..default()
                },
            },
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

fn add_humans(mut commands: Commands) {
    // Previously defined shapes will now appear as morph targets on the prefab's mesh
    // The HumanConfig type controls prefab access and applies our morph targets.
    let bodybuilder = "bodybuilder";
    let mut morphs = MorphTargets::default();
    morphs.insert(bodybuilder, 1.);

    // Base mesh will be lod0
    let lod1 = "male_generic";
    let lod2 = "male1591";
    let lod3 = "proxy741";

    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 1.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        LevelOfDetail, // Just a marker component for this example
        InheritedVisibility::default(),
        children![
            (
                CharacterPart::BaseMesh,
                VisibilityRange {
                    start_margin: 0.0..0.0,
                    end_margin: 2.0..2.,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart::ProxyMesh(lod1),
                VisibilityRange {
                    start_margin: 2.0..2.0,
                    end_margin: 4.0..4.0,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart::ProxyMesh(lod2),
                VisibilityRange {
                    start_margin: 4.0..4.,
                    end_margin: 6.0..6.,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart::ProxyMesh(lod3),
                VisibilityRange {
                    start_margin: 6.0..6.0,
                    end_margin: 8.0..10.0,
                    use_aabb: false,
                }
            )
        ],
    ));

    // Just for comparison we'll spawn the proxies used here
    // The base mesh (highest poly-count ~15k tris I think)
    commands.spawn((
        Transform::from_translation(Vec3::new(-1.5, 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        InheritedVisibility::default(),
        children![(CharacterPart::BaseMesh)],
    ));

    // male_generic (high poly-count 13k tris)
    commands.spawn((
        Transform::from_translation(Vec3::new(-0.5, 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        InheritedVisibility::default(),
        children![(CharacterPart::ProxyMesh(lod1))],
    ));

    //  male1591 (low poly-count)
    commands.spawn((
        Transform::from_translation(Vec3::new(0.5, 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        InheritedVisibility::default(),
        children![(CharacterPart::ProxyMesh(lod2))],
    ));

    // proxy741 (very low poly-count)
    commands.spawn((
        Transform::from_translation(Vec3::new(1.5, 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        InheritedVisibility::default(),
        children![(CharacterPart::ProxyMesh(lod3))],
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

fn setup_prefabs(mut commands: Commands, morphs: Res<MakeHumanMorphs>) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("asian", 1.);
    morph_targets.insert("weight", 1.);
    morph_targets.insert("muscle", 1.);
    let bodybuilder_shape =
        CharacterShapeArchetype::new("bodybuilder", morphs.compute_target_weights(&morph_targets));

    let mut prefabs = AHashMap::default();
    prefabs.insert(
        PREFAB,
        CharacterArchetypePrefab::new(
            vec![bodybuilder_shape],
            CharacterAnimationArchetype::default(), // No animation in this example
        ),
    );

    commands.insert_resource(CharacterArchetypePrefabs::new(prefabs));
}
