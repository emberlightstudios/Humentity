mod shared;
use ahash::AHashMap;
use bevy::{pbr::ExtendedMaterial, prelude::*};
use humentity::prelude::*;
use shared::{cam_controls, setup_env};

fn main() {
    App::new()
        .add_plugins((
            Humentity::new(HumentityPathsConfig::new("./")), // Must come before DefaultPlugins
            DefaultPlugins,
            MaterialPlugin::<ExtendedMaterial<StandardMaterial, HumanMaterialExtension>>::default(),
        ))
        .add_systems(Startup, setup_env)
        .add_systems(OnExit(HumentityLoadState::LoadingCoreAssets), setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_humans)
        .add_systems(Update,
            (
                setup_material,
            )
        )
        .add_systems(Update, cam_controls)
        .run();

}

fn setup_material(
    mut commands: Commands,
    humans: Query<(Entity, &HumanPart), (With<Mesh3d>, Without<MeshMaterial3d<ExtendedMaterial<StandardMaterial, HumanMaterialExtension>>>)>,
    human_materials: Res<HumanMaterials>,
    mut human_material_assets: ResMut<Assets<ExtendedMaterial<StandardMaterial, HumanMaterialExtension>>>,
) {
    //info!("{:#?}", human_materials.keys());
    for (entity, part) in humans.iter() {
        let name = "middleage_asian_female";
        // This should always be true here, but in general we only want to put this material
        // on the base mesh or the proxy meshes, not any other parts/assets.
        if matches!(part, HumanPart::BaseMesh) {  
            let material = human_materials.get(name).unwrap();
            let material = human_material_assets.add(material.clone());
            commands.entity(entity).insert(MeshMaterial3d(material.clone()));
        }
    }
}

fn add_humans(
    mut commands: Commands,
) {
    let prefab_name = "ExampleHumanPrefab";

    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        HumanShapeConfig::new(prefab_name, MorphTargets::default()),
        InheritedVisibility::default(),
        children![(HumanPart::BaseMesh)],
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
        "ExampleHumanPrefab",
        HumanArchetypePrefab::new(
            vec![base_shape],
            HumanAnimationArchetype::default(), // No animation in this example
        )
    );

    commands.insert_resource(HumanArchetypePrefabs::new(prefabs));
}