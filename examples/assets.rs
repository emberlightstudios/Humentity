mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::{add_humentity_plugin, cam_controls, setup_env};

use ahash::AHashMap;

const PREFAB: &str = "prefab";
// See assets folder for file names
const SKIN: &str = "young_caucasian_female";
const EYES: &str = "high-poly-eyes";
const EYE_TEXTURE: &str = "blue_eye";
const EYEBROW: &str = "eyebrow001";
const EYEBROW_TEXTURE: &str = "eyebrow001";
const EYELASH: &str = "false_eyelashes";
const EYELASH_TEXTURE: &str = "false_eyelashes";
const BRA: &str = "simple_bra";
const PANTIES: &str = "simple_briefs";
const HAIR: &str = "ponytail01";
const HAIR_TEXTURE: &str = "ponytail01";

fn main() {
    let mut app = App::new();
    add_humentity_plugin(&mut app);

    app.add_plugins(DefaultPlugins)
        .add_systems(Startup, setup_env)
        .add_systems(Startup, setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_human)
        .add_systems(Update, (cam_controls, add_materials))
        .run();
}

fn add_materials(
    mut asset_registry: ResMut<CharacterAssetRegistry>,
    parts: Query<
        (Entity, &CharacterPart),
        (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>),
    >,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (entity, part) in parts {
        let mat = match part {
            CharacterPart::BodyMesh(_) => {
                let skin_albedo: Handle<Image> = part.get_texture_handle(
                    SKIN,
                    CharacterAssetTextureType::Albedo,
                    &asset_server,
                    &asset_registry,
                );
                materials.add(StandardMaterial {
                    base_color_texture: Some(skin_albedo),
                    ..default()
                })
            }
            CharacterPart::BodyPart(EYES) => {
                let albedo: Handle<Image> = part.get_texture_handle(
                    EYE_TEXTURE,
                    CharacterAssetTextureType::Albedo,
                    &asset_server,
                    &asset_registry,
                );
                materials.add(StandardMaterial {
                    base_color_texture: Some(albedo),
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                })
            }
            CharacterPart::BodyPart(EYEBROW) => {
                let albedo: Handle<Image> = part.get_texture_handle(
                    EYEBROW_TEXTURE,
                    CharacterAssetTextureType::Albedo,
                    &asset_server,
                    &asset_registry,
                );
                materials.add(StandardMaterial {
                    base_color_texture: Some(albedo),
                    base_color: Color::BLACK,
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                })
            }
            CharacterPart::BodyPart(HAIR) => {
                let albedo: Handle<Image> = part.get_texture_handle(
                    HAIR_TEXTURE,
                    CharacterAssetTextureType::Albedo,
                    &asset_server,
                    &asset_registry,
                );
                materials.add(StandardMaterial {
                    base_color_texture: Some(albedo),
                    //base_color: Color::LinearRgba(LinearRgba::RED),
                    alpha_mode: AlphaMode::Blend,
                    clearcoat_perceptual_roughness: 0.1,
                    clearcoat: 0.2,
                    perceptual_roughness: 0.3,
                    reflectance: 0.1,
                    metallic: 0.,
                    ..default()
                })
            }
            CharacterPart::BodyPart(EYELASH) => {
                let albedo: Handle<Image> = part.get_texture_handle(
                    EYELASH_TEXTURE,
                    CharacterAssetTextureType::Albedo,
                    &asset_server,
                    &asset_registry,
                );
                materials.add(StandardMaterial {
                    base_color_texture: Some(albedo),
                    base_color: Color::LinearRgba(LinearRgba::RED),
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                })
            }
            _ => {
                // I didnt' make any textures for the basic clothes
                // Should just return a dummy handle and log an error
                let _albedo: Handle<Image> = part.get_texture_handle(
                    EYELASH_TEXTURE,
                    CharacterAssetTextureType::Albedo,
                    &asset_server,
                    &mut asset_registry,
                );
                materials.add(StandardMaterial {
                    base_color: Color::LinearRgba(LinearRgba::RED),
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                })
            }
        };
        commands.entity(entity).insert(MeshMaterial3d(mat));
    }
}

fn add_human(mut commands: Commands) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("female", 1.);

    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morph_targets),
        Visibility::Visible,
        children![
            (
                CharacterPart::BodyMesh("female_muscle_13442"),
                InheritedVisibility::default(),
            ),
            (
                CharacterPart::BodyPart(EYES),
                InheritedVisibility::default(),
            ),
            (
                CharacterPart::BodyPart(EYEBROW),
                InheritedVisibility::default(),
            ),
            (
                CharacterPart::BodyPart(EYELASH),
                InheritedVisibility::default(),
            ),
            (
                CharacterPart::BodyPart(HAIR),
                InheritedVisibility::default(),
            ),
            (
                CharacterPart::Equipment(BRA),
                InheritedVisibility::default(),
            ),
            (
                CharacterPart::Equipment(PANTIES),
                InheritedVisibility::default(),
            )
        ],
    ));
}

fn setup_prefabs(mut commands: Commands, morphs: Res<MakeHumanMorphs>) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.);
    let base_shape = CharacterShapeArchetype::new(
        "female".to_string(),
        morphs.compute_target_weights(&morph_targets),
    );
    let mut prefabs = AHashMap::default();
    prefabs.insert(
        PREFAB,
        CharacterArchetypePrefab::new(
            vec![base_shape],
            CharacterAnimationArchetype::default(), // No animation in this example
        ),
    );

    commands.insert_resource(CharacterArchetypePrefabs::new(prefabs));
}
