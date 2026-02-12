//! Makehuman comes with several lower poly proxy meshes.  These
//! can be used for lods with the VisibilityRanges component
//! In this example we switch lods early just for clarity.

mod shared;

use ahash::AHashMap;
use bevy::prelude::*;
use humentity::prelude::*;
use shared::{add_humentity_plugin, cam_controls, setup_env};

const BODY_PREFAB: &str = "Body";
const FACE_PREFAB: &str = "Face";

fn main() {
    let mut app = App::new();
    add_humentity_plugin(&mut app);
    app.add_plugins(DefaultPlugins)
        .add_systems(Startup, setup_env)
        .add_systems(Update, (
            cam_controls,
            add_mat,
        ))
        .add_systems(Startup, setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_humans)
        .run();
}

fn add_mat(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    meshes: Query<Entity, (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>)>,
) {
    for mesh in meshes {
        let white = materials.add(StandardMaterial::from_color(Color::WHITE));
        commands.entity(mesh).insert(MeshMaterial3d(white));
    }
}

fn add_humans(
    mut commands: Commands,
) {
    let mut morphs = MorphTargets::default();
    morphs.insert("man", 1.);
    morphs.insert("mouthOpen", 1.);

    // Here we don't stitch.  Look closely at the neckline and you will see problems with normals.
    commands.spawn((
        Transform::from_translation(Vec3::new(-1., 0., -1.)),
        CharacterShapeConfig::new(BODY_PREFAB, morphs.clone()),
        children![(
            CharacterPart::BodyMesh("basemesh_headless"),
        ), (
            CharacterPart::BodyMesh("basemesh_head"),
            PrefabOverride(FACE_PREFAB),
        )],
    ));

    // Here we use stitched parts to fix the normals.
    // We can only fix normals at the seams if the verts on the seams at each mesh
    // have EXACTLY the same positions on each mesh
    morphs.clear();
    morphs.insert("woman", 1.);
    morphs.insert("smiling", 1.);
    commands.spawn((
        Transform::from_translation(Vec3::new(1., 0., -1.)),
        CharacterShapeConfig::new(BODY_PREFAB, morphs),
        children![(
            StitchedParts(vec![
                StitchedPart::from(CharacterPart::BodyMesh("basemesh_headless")),
                StitchedPart::from(CharacterPart::BodyMesh("basemesh_head"))
                    .with_override(FACE_PREFAB),
            ]),
        )],
    ));
}

fn setup_prefabs(mut commands: Commands, mh_morphs: Res<MakeHumanMorphs>) {
    let mut morph_targets = MorphTargets::default();
    let mut shapes = vec![];

    morph_targets.insert("gender", 0.0);
    let morphs = mh_morphs.compute_target_weights(&morph_targets);
    shapes.push(CharacterShapeArchetype::new(
        "woman", morphs,
    ));

    morph_targets.clear();
    morph_targets.insert("gender", 1.0);
    let morphs = mh_morphs.compute_target_weights(&morph_targets);
    shapes.push(CharacterShapeArchetype::new(
        "man", morphs,
    ));

    let mut prefabs = AHashMap::default();
    prefabs.insert(
        BODY_PREFAB,
        CharacterArchetypePrefab::new(
            shapes.clone(),
            CharacterAnimationArchetype::default(), 
        ),
    );

    // We keep the gender shapes on the face also to align it with the body
    // We add more shapes for expressions
    morph_targets.clear();
    // But the expression morphs are face only.
    // These are not composite/macro morphs so we don't need to compute_target_weights on them.
    morph_targets.insert("jawOpen", 1.0);
    shapes.push(CharacterShapeArchetype::new(
        "mouthOpen", morph_targets.clone()
    ));

    morph_targets.clear();
    morph_targets.insert("mouthSmileLeft", 1.0);
    morph_targets.insert("mouthSmileRight", 1.0);
    shapes.push(CharacterShapeArchetype::new(
        "smiling", morph_targets.clone()
    ));

    prefabs.insert(
        FACE_PREFAB,
        CharacterArchetypePrefab::new(
            shapes,
            CharacterAnimationArchetype::default(), 
        ),
    );

    commands.insert_resource(CharacterArchetypePrefabs::new(prefabs));
}
