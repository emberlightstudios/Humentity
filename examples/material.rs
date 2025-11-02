//! This example shows how to use the HumanMaterialExtension to create a new material for
//! the skin of a character.  You could also just use the StandardMaterial, but the idea
//! is that this setup should improve GPU batching.  At least that's my intuition.

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
            MaterialPlugin::<ExtendedMaterial<StandardMaterial, CharacterMaterialExtension>>::default(),
        ))
        .add_systems(Startup, setup_env)
        .add_systems(OnExit(HumentityLoadState::LoadingCoreAssets), setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_human)
        .add_systems(Update, add_skin_material)
        .add_systems(Update, cam_controls)
        .run();

}

fn add_skin_material(
    mut commands: Commands,
    humans: Query<(Entity, &CharacterPart), (With<Mesh3d>, Without<MeshMaterial3d<ExtendedMaterial<StandardMaterial, CharacterMaterialExtension>>>)>,
    textures: Res<CharacterBodyTextures>,
    asset_server: Res<AssetServer>,
    mut human_material_assets: ResMut<Assets<ExtendedMaterial<StandardMaterial, CharacterMaterialExtension>>>,
) {
    for (entity, part) in humans.iter() {
        let name = "middleage_asian_female";
        // This should always be true here, but in general we only want to put skin textures
        // on the base mesh or the proxy meshes, not any other parts/assets.
        if matches!(part, CharacterPart::BaseMesh) {  
            let albedo = &textures.albedo_maps[name];
            let material = ExtendedMaterial {
                base: StandardMaterial {
                    base_color_texture: Some(asset_server.load(albedo.clone())),
                    ..default()
                },
                extension: CharacterMaterialExtension {
                    // No data defined yet.
                }
            };
            let material = human_material_assets.add(material.clone());
            commands.entity(entity).insert(MeshMaterial3d(material.clone()));
        }
    }
}

fn add_human(
    mut commands: Commands,
) {

    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        CharacterShapeConfig::default(),
        InheritedVisibility::default(),
        children![(CharacterPart::BaseMesh)],
    ));
}

fn setup_prefabs(mut commands: Commands, morphs: Res<MakeHumanMorphs>) {
    commands.insert_resource(CharacterArchetypePrefabs::default());
}