//! Makehuman allows character customization through the use of morph targets,
//! also knows as blendshapes or shapekeys.  One possible architecture for this
//! crate could be to accept a set of morph values and bake the resulting
//! mesh down to a new fixed mesh.  One problem with this approach is that it
//! breaks instancing/batching between different humans, and therefore
//! performance degrades, as well memory usage explodes since each individual
//! mesh, skinnedmesh, etc. must occcupy it's own space in the AssetServer/GPU buffers.
//! To overcome these problems Humentity uses a "template" system.
//!
//! Makehuman has something like 1000 distinct morph targets.  This
//! is too many to be used at runtime.  The Humentity template system
//! allows you to bake an entire set of makehuman morph weights down to a
//! single morph target in bevy. In order to make variable humans we can define
//! a few basic human archetypes, and perhaps a set of distinct faces that we can
//! use to blend between at runtime.  This allows us to dramatically reduce the
//! number of morph targets while still allowing at least some runtime mesh
//! customization, and keeping instancing/batching intact, since each template
//! is still the same mesh handle (assuming they all use the same material also).

mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::setup_app;

const TEMPLATE_NAME: &str = "ExampleHumanTemplate";
const BABY: &str = "baby";
const BODYBUILDER: &str = "bodybuilder";


fn main() {
    let mut app = setup_app();

    app
        .add_systems(
            Update,
            add_humans
                .run_if(resource_exists::<MakeHumanMorphs>)
                .run_if(not(resource_exists::<CharacterTemplates>))
        )
        .run();
}

fn add_humans(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    morphs: Res<MakeHumanMorphs>,
) {
    // When feeding in morphs you can ignore the categories here.
    // They are only for helping you organize a UI

    // Let's create a template that can take different shapes
    // If race is not specified, defaults to caucasian (caucasian = 1, african = 0, asian = 0)
    // If gender is not specified, defaults to male (gender = 1)
    // If age is not specified, defaults to (young) adult (age = 0.5)
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("age", 0.);

    // This fn call is necessary to deconstruct compound "morph" values
    // down to the level of individual makehuman morph targets.
    // Many of the available morphs (see line 83) actually drive multiple
    // makehuman morph targets at once.
    let baby_morphs = match morphs.compute_target_weights(&morph_targets) {
        Ok(morphs) => morphs,
        Err(err) => {
            error!("Error computing morph targets for baby: {err}");
            return;
        } 
    };

    let loaded = morphs.targets.read().unwrap();
    for k in baby_morphs.keys() {
        if !loaded.contains_key(k) {
            return;
        }
    }

    morph_targets.clear();
    // These are desinged in makehuman such that you don't have to normalize their sum.
    morph_targets.insert("weight", 1.);
    morph_targets.insert("muscle", 1.);
    let bodybuilder_morphs = match morphs.compute_target_weights(&morph_targets) {
        Ok(morphs) => morphs,
        Err(err) => {
            error!("Error computing morph targets for bodybuilder: {err}");
            return;
        } 
    };
    let loaded = morphs.targets.read().unwrap();
    for k in bodybuilder_morphs.keys() {
        if !loaded.contains_key(k) {
            return;
        }
    }

    commands.insert_resource(
        CharacterTemplates::new([(
            TEMPLATE_NAME,
            CharacterTemplate::new(
                [
                    CharacterMorphShapes::new(BODYBUILDER, bodybuilder_morphs),
                    CharacterMorphShapes::new(BABY, baby_morphs),
                ],
                RigType::Default,
            ),
        )]),
    );

    // Previously defined shapes will now appear as morph targets on the template's mesh
    // The HumanShapeConfig type controls template access and applies our morph targets.
    let basemesh_part =
        CharacterPart(asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy"));

    // Trigger the mesh to build with the new morph targets.
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh_part.clone(),
        template_name: TEMPLATE_NAME,
    });

    // Spawn some characters with different morph values.  They will all share the same mesh handle, but look different!
    
    // The base mesh
    let mut morphs = MorphTargets::default();
    morphs.insert(BABY, 0.);
    morphs.insert(BODYBUILDER, 0.);
    commands.spawn((
        Name::new("Basemesh"),
        Transform::from_translation(Vec3::new(-2., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(TEMPLATE_NAME, morphs.clone()),
        children![(basemesh_part.clone())],
    ));

    // A baby
    morphs.insert(BABY, 1.);
    morphs.insert(BODYBUILDER, 0.);
    commands.spawn((
        Name::new("Baby"),
        Transform::from_translation(Vec3::new(-1., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(TEMPLATE_NAME, morphs.clone()),
        children![(
            basemesh_part.clone(),
            Name::new("mesh"),
        )],
    ));

    // A bodybuilder
    morphs.insert(BABY, 0.);
    morphs.insert(BODYBUILDER, 1.);
    commands.spawn((
        Name::new("Bodybuilder"),
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(TEMPLATE_NAME, morphs.clone()),
        children![(
            Name::new("mesh"),
            basemesh_part.clone(),
        )],
    ));

    // Half baby/half bodybuilder, ha!
    // Note that the shapekey weights sum to 1
    morphs.insert(BABY, 0.5);
    morphs.insert(BODYBUILDER, 0.5);
    commands.spawn((
        Name::new("Hybrid normalized"),
        Transform::from_translation(Vec3::new(1., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(TEMPLATE_NAME, morphs.clone()),
        children![(
            basemesh_part.clone(),
            Name::new("mesh"),
        )],
    ));

    // You have to be careful with normalization of mixed shapekeys sometimes
    // or you might end up with artifacts!
    // Here is a baby/bodybuilder mix, without normalizing
    morphs.insert(BABY, 1.);
    morphs.insert(BODYBUILDER, 1.);
    commands.spawn((
        Name::new("Hybrid unnormalized"),
        Transform::from_translation(Vec3::new(2., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(TEMPLATE_NAME, morphs),
        children![(
            basemesh_part,
            Name::new("mesh"),
        )],
    ));
}
