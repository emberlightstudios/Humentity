/// Mesh stitching allows you to create CharacterParts which are themselves composed of smaller parts.
/// This could be used as in this example to put facial expression shapes only on the face mesh.
/// You could also set up a dismemberment system. I'm sure there are other use cases.
/// 
/// The problem with splitting meshes into multiple pieces is that when you cut a mesh at an edge loop,
/// most 3d modelling software will autmoatically recompute normals at the loop and create a discontinuity
/// of mesh normals across the seam.  The normals will no longer be smooth and lighting will make a line
/// obvious where you cut.  This is the main problem intended to be solved by mesh stitching.
/// 
/// Here I will show how mesh stitching works by comparing the same mesh parts with and without
/// stitching.  There is a head only part and a headless body part that together form the basemesh.
/// I will use separate prefabs for stitched and unstitched versions because mesh handles are cached
/// per prefab and I don't want any interference between them.  The stitched and unstitched meshes
/// may share vertices but they are not the same mesh.  Only the stitched meshes have continuous
/// normals across the seams betewen the pieces.
/// 
/// Note that the normal smoothing algorithm requires that the vert positions are bitwise identical on both 
/// sides of your edge loop cuts.


// 4 prefabs in total, 2 pieces * (stitched + non-stitched)
 
const STITCHED_BODY: &str = "stitched_body";
const STITCHED_FACE: &str = "stitched_face";
const UNSTITCHED_BODY: &str = "unstitched_body";
const UNSTITCHED_FACE: &str = "unstitched_face";

mod shared;

use ahash::AHashMap;
use bevy::prelude::*;
use humentity::prelude::*;
use shared::{add_humentity_plugin, cam_controls, setup_env};


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
        UNSTITCHED_BODY,
        CharacterArchetypePrefab::new(
            shapes.clone(),
            CharacterAnimationArchetype::default(), 
        ),
    );
    prefabs.insert(
        STITCHED_BODY,
        CharacterArchetypePrefab::new(
            shapes.clone(),
            CharacterAnimationArchetype::default(), 
        ),
    );

    // We keep the gender shapes on the face also to align it with the body. We add more shapes for expressions
    // But the expression morphs can be face only, hence 2 different prefabs, one for body another for face.
    morph_targets.clear();
    // These are not composite/macro morphs so we don't need to compute_target_weights on them.
    morph_targets.insert("jawOpen", 1.0);
    morph_targets.insert("mouthFrownLeft", 1.0);
    morph_targets.insert("mouthFrownRight", 1.0);
    shapes.push(CharacterShapeArchetype::new(
        "scream", morph_targets.clone()
    ));

    morph_targets.clear();
    morph_targets.insert("mouthSmileLeft", 1.0);
    morph_targets.insert("mouthSmileRight", 1.0);
    morph_targets.insert("jawOpen", 0.3);
    shapes.push(CharacterShapeArchetype::new(
        "smile", morph_targets.clone()
    ));

    prefabs.insert(
        UNSTITCHED_FACE,
        CharacterArchetypePrefab::new(
            shapes.clone(),
            CharacterAnimationArchetype::default(), 
        ),
    );
    prefabs.insert(
        STITCHED_FACE,
        CharacterArchetypePrefab::new(
            shapes.clone(),
            CharacterAnimationArchetype::default(), 
        ),
    );

    commands.insert_resource(CharacterArchetypePrefabs::new(prefabs));
}

fn add_humans(
    mut commands: Commands,
) {
    let mut morphs = MorphTargets::default();
    morphs.insert("man", 1.);
    morphs.insert("scream", 1.);

    // Here we don't stitch.  If you look closely at the man's neckline you will see
    // the discontinuity in mesh normals, hence the facial expression.
    commands.spawn((
        Transform::from_translation(Vec3::new(-1., 0., -1.)),
        // This stores the "default" prefab for the character.  Parts will use this unless overridden.
        CharacterShapeConfig::new(UNSTITCHED_BODY, morphs.clone()),
        children![(
            CharacterPart::BodyMesh("basemesh_headless"),
        ), (
            CharacterPart::BodyMesh("basemesh_head"),
            // We need to override the prefab used by the head part because only this contains 
            // the facial expression shapekeys
            PrefabOverride(UNSTITCHED_FACE),
        )],
    ));

    // Here we use stitched parts to fix the normals.  This makes the woman happy and smiling.
    morphs.clear();
    morphs.insert("woman", 1.);
    morphs.insert("smile", 1.);
    commands.spawn((
        Transform::from_translation(Vec3::new(1., 0., -1.)),
        CharacterShapeConfig::new(STITCHED_BODY, morphs),
        children![(
            StitchedParts(vec![
                StitchedPart::from(CharacterPart::BodyMesh("basemesh_headless")),
                StitchedPart::from(CharacterPart::BodyMesh("basemesh_head"))
                    .with_prefab_override(STITCHED_FACE),  // Again we need to override for the face
            ]),
        )],
    ));
}