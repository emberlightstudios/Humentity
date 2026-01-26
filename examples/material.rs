//! This example shows how to use the HumanMaterialExtension to create a new material for
//! the skin of a character.  You could also just use the StandardMaterial, but the idea
//! is that this setup should improve GPU batching.  At least that's my intuition.

mod shared;
use bevy::{pbr::ExtendedMaterial, prelude::*};
use humentity::prelude::*;
use shared::{cam_controls, setup_env, add_humentity_plugin};

fn main() {
    let mut app = App::new();
    add_humentity_plugin(&mut app);

    app
        .add_plugins((
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
    humans: Query<
        (Entity, &CharacterPart),
        (
            With<Mesh3d>,
            Without<MeshMaterial3d<ExtendedMaterial<StandardMaterial, CharacterMaterialExtension>>>,
        ),
    >,
    asset_server: Res<AssetServer>,
    mut asset_registry: ResMut<CharacterAssetRegistry>,
    mut human_material_assets: ResMut<
        Assets<ExtendedMaterial<StandardMaterial, CharacterMaterialExtension>>,
    >,
) {
    for (entity, part) in humans.iter() {
        let name = "middleage_asian_female";
        // This should always be true here, but in general we only want to put skin textures
        // on the base mesh or the proxy meshes, not any other parts/assets.
        if matches!(part, CharacterPart::BaseMesh) {
            let albedo = part.get_texture_handle(name, CharacterAssetTextureType::Albedo, &asset_server, &mut asset_registry);
            let material = ExtendedMaterial {
                base: StandardMaterial {
                    base_color_texture: Some(albedo),
                    ..default()
                },
                extension: CharacterMaterialExtension::default(),
            };
            let material = human_material_assets.add(material.clone());
            commands
                .entity(entity)
                .insert(MeshMaterial3d(material.clone()));
        }
    }
}

fn add_human(mut commands: Commands) {
    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        CharacterShapeConfig::default(),
        InheritedVisibility::default(),
        children![(CharacterPart::BaseMesh)],
    ));
}

fn setup_prefabs(mut commands: Commands) {
    // No morph shapes
    commands.insert_resource(CharacterArchetypePrefabs::basemesh());
}
