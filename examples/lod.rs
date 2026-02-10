//! Makehuman comes with several lower poly proxy meshes.  These
//! can be used for lods with the VisibilityRanges component
//! In this example we switch lods early just for clarity.

mod shared;
use ahash::AHashMap;
use bevy::{camera::visibility::VisibilityRange, prelude::*};
use humentity::prelude::*;
use shared::{add_humentity_plugin, cam_controls, setup_env};

const PREFAB: &str = "ExamplePrefab";
// I'm testing the topology I swear
const SHAPE_NAME: &str = "bigboobs";

fn main() {
    let mut app = App::new();
    add_humentity_plugin(&mut app);

    app.add_plugins(DefaultPlugins)
        .add_systems(Startup, setup_env)
        .add_systems(Update, (cam_controls, add_material))
        .add_systems(Startup, setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_humans)
        .run();
}

// this is just a marker component to control which mesh has lods.
#[derive(Component)]
struct LevelOfDetail;

fn add_humans(mut commands: Commands) {
    // Previously defined shapes will now appear as morph targets on the prefab's mesh
    // The HumanConfig type controls prefab access and applies our morph targets.

    // I'm testing the topology I swear
    let mut morphs = MorphTargets::default();
    morphs.insert(SHAPE_NAME, 1.);

    // Base mesh will be lod0
    let lod0 = "basemesh";

    // I generated this from basemesh with a decimate modifier 
    // in collapse mode, topology is a bit chaotic
    let lod1 = "proxy6025";

    // These were built in
    let lod2 = "proxy1605";
    let lod3 = "proxy741";

    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., -1.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        LevelOfDetail, // Just a marker component for this example
        InheritedVisibility::default(),
        children![
            (
                CharacterPart::BodyMesh(lod0),
                VisibilityRange {
                    start_margin: 0.0..0.0,
                    end_margin: 2.0..3.0,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart::BodyMesh(lod1),
                VisibilityRange {
                    start_margin: 2.0..3.0,
                    end_margin: 7.0..8.0,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart::BodyMesh(lod2),
                VisibilityRange {
                    start_margin: 7.0..8.,
                    end_margin: 14.0..15.0,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart::BodyMesh(lod3),
                VisibilityRange {
                    start_margin: 14.0..15.0,
                    end_margin: 20.0..30.0,
                    use_aabb: false,
                }
            )
        ],
    ));

    // Just for comparison we'll spawn the proxies used here

    // Basemesh ~13k verts
    commands.spawn((
        Transform::from_translation(Vec3::new(-1.5, 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        InheritedVisibility::default(),
        children![(CharacterPart::BodyMesh(lod0))],
    ));

    // mid poly 6025 verts
    // Topology is not ideal.  I made it with decimate modifier on basemesh in blender
    commands.spawn((
        Transform::from_translation(Vec3::new(-0.5, 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        InheritedVisibility::default(),
        children![(CharacterPart::BodyMesh(lod1))],
    ));

    //  low poly-count 1605 verts
    commands.spawn((
        Transform::from_translation(Vec3::new(0.5, 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        InheritedVisibility::default(),
        children![(CharacterPart::BodyMesh(lod2))],
    ));

    // very low poly-count 741 verts
    commands.spawn((
        Transform::from_translation(Vec3::new(1.5, 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        InheritedVisibility::default(),
        children![(CharacterPart::BodyMesh(lod3))],
    ));

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
    morph_targets.insert("muscle", 1.);
    morph_targets.insert("gender", 0.);
    morph_targets.insert("cupsize", 1.);
    morph_targets.insert("firmness", 1.);

    // Deconstruct compound sliders
    let morphs = morphs.compute_target_weights(&morph_targets);
    info!("{:#?}", morphs);

    let shape = CharacterShapeArchetype::new(
        SHAPE_NAME.to_string(),
        morphs
    );

    let mut prefabs = AHashMap::default();
    prefabs.insert(
        PREFAB,
        CharacterArchetypePrefab::new(
            vec![shape],
            CharacterAnimationArchetype::default(), // No animation in this example
        ),
    );

    commands.insert_resource(CharacterArchetypePrefabs::new(prefabs));
}
