//! Makehuman allows character customization through the use of morph targets,
//! also knows as blendshapes or shapekeys.  One possible architecture for this 
//! crate could be to accept a set of morph values and bake the resulting
//! mesh down to a new fixed mesh.  One problem with this approach is that it
//! breaks instancing/batching between different humans, and therefore 
//! performance degrades, as well memory usage explodes since each individual
//! mesh, skinnedmesh, etc. must occcupy it's own space in the AssetServer/GPU buffers. 
//! To overcome these problems Humentity uses a "prefab" system.
//!
//! Makehuman has something like 1000 distinct morph targets.  This
//! is too many to be used at runtime.  While possible, it is likely to lead
//! to performance degradation in the shader.  The Humentity prefab system
//! allows you to bake an entire set of makehuman morph weights down to a 
//! single morph target in bevy. In order to make variable humans we can define
//! a few basic human archetypes, and perhaps a set of distinct faces that we can
//! use to blend between at runtime.  This allows us to dramatically reduce the
//! number of morph targets while still allowing at least some runtime mesh
//! customization, and keeping instancing/batching intact, since each prefab
//! is still the same mesh handle (assuming they all use the same material also). 

mod shared;

use bevy::prelude::*;
use shared::cam_controls;
use ahash::AHashMap;
use humentity::prelude::*;

fn main() {
    let mut app = App::new();
    app
        .add_plugins((
            // Point to the humentity crate location
            Humentity::new(HumentityPathsConfig::new("./")),
            DefaultPlugins,
        ))
        .add_systems(Startup, setup_env)
        .add_systems(
            OnExit(HumentityLoadState::LoadingCoreAssets), 
            setup_prefabs
        )
        .add_systems(
            OnEnter(HumentityLoadState::Ready),
            add_humans
        )
        .add_systems(Update, (cam_controls, add_material))
        .run();
}

fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
        Transform::from_xyz(0.0, 1.0, 3.0),
    ));

    // A camera:
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 3.0, 4.0).looking_at(Vec3::Y * 0.7, Vec3::Y),
    ));
}

fn setup_prefabs(mut commands: Commands, morphs: Res<HumanMorphs>) {
    // When feeding in morphs you can ignore the categories here.
    // They are only for helping you organize a UI
    //info!("Available morphs: {:#?}", morphs.get_morph_names());

    // Let's create a prefab that can take different shapes
    // If race is not specified, defaults to caucasian (caucasian = 1, african = 0, asian = 0)
    // If gender is not specified, defaults to male (gender = 1)
    // If age is not specified, defaults to (young) adult (age = 0.5)
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert(Name::new("age"), 0.);

    // We'll give our prefab 2 shapes, a baby archetype and a bodybuilder archetype
    let baby_shape = HumanShapeArchetype::new(
        Name::new("baby"),
        // This fn call is necessary to deconstruct compound "morph" values
        // down to the level of individual makehuman morph targets.
        // Many of the available morphs (see line 83) actually drive multiple
        // makehuman morph targets at once.
        morphs.compute_target_weights(&morph_targets),
    );

    morph_targets.clear();
    // These are desinged in makehuman such that you don't have to normalize their sum.
    morph_targets.insert(Name::new("weight"), 1.);
    morph_targets.insert(Name::new("muscle"), 1.);
    let bodybuilder_shape = HumanShapeArchetype::new(
        Name::new("bodybuilder"),
        morphs.compute_target_weights(&morph_targets),
    );

    // You could use this, e.g. to define distinct face presets on a body also. Since they
    // become morph targets you can generate essentially infinite face shapes from the vector
    // space spanned by these basis morphs.  

    let mut prefabs = AHashMap::default();

    // You can have more than one prefab, but for this example just one.
    // Prefabs have a name also
    prefabs.insert(
        Name::new("ExampleHumanPrefab"),
        HumanArchetypePrefab::new(
            vec![baby_shape, bodybuilder_shape],
            HumanAnimationArchetype::default(), // No animation in this example
        )
    );

    commands.insert_resource(HumanArchetypePrefabs::new(prefabs));
}

fn add_humans(
    mut commands: Commands,
) {
    // Previously defined shapes will now appear as morph targets on the prefab's mesh
    // The HumanShapeConfig type controls prefab access and applies our morph targets.
    let prefab_name = Name::new("ExampleHumanPrefab");
    let baby = Name::new("baby");
    let bodybuilder = Name::new("bodybuilder");

    // The base mesh
    let mut morphs = MorphTargets::default();
    morphs.insert(baby.clone(), 0.);
    morphs.insert(bodybuilder.clone(), 0.);
    commands.spawn((
        Transform::from_translation(Vec3::new(-2., 0., 0.)),
        InheritedVisibility::default(),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        children![(
            // This is broken for some reason.  I can't figure it out.  The mesh renders
            // at the wrong location, or not at all.  Makes no sense.
            HumanPart::BaseMesh 
        )]
    ));

    // A baby
    morphs.insert(baby.clone(), 1.);
    morphs.insert(bodybuilder.clone(), 0.);
    commands.spawn((
        Transform::from_translation(Vec3::new(-1., 0., 0.)),
        InheritedVisibility::default(),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        children![(
            HumanPart::BaseMesh
        )]
    ));

    // A bodybuilder
    morphs.insert(baby.clone(), 0.);
    morphs.insert(bodybuilder.clone(), 1.);
    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        InheritedVisibility::default(),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        children![(
            HumanPart::BaseMesh
        )]
    ));

    // Half baby/half bodybuilder, ha!
    // Note that the shapekey weights sum to 1
    morphs.insert(baby.clone(), 0.5);
    morphs.insert(bodybuilder.clone(), 0.5);
    commands.spawn((
        Transform::from_translation(Vec3::new(1., 0., 0.)),
        InheritedVisibility::default(),
        HumanShapeConfig::new(prefab_name.clone(), morphs.clone()),
        children![(
            HumanPart::BaseMesh
        )]
    ));

    // You have to be careful with normalization of mixed shapekeys sometimes 
    // or you might end up with artifacts!
    // Here is a baby/bodybuilder mix, without normalizing
    morphs.insert(baby, 1.);
    morphs.insert(bodybuilder, 1.);
    commands.spawn((
        Transform::from_translation(Vec3::new(2., 0., 0.)),
        InheritedVisibility::default(),
        HumanShapeConfig::new(prefab_name, morphs),
        children![(
            HumanPart::BaseMesh
        )]
    ));
}

// Adds a simple black material to humans
fn add_material(
    humans: Query<Entity, (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let mat = materials.add(StandardMaterial::from_color(Color::BLACK));
    for human in humans {
        commands.entity(human).insert(MeshMaterial3d(mat.clone()));
    }
}