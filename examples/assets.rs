mod shared;

use bevy::prelude::*;
use humentity::{HumentityGlobalConfig, prelude::*};
use shared::{setup_env, cam_controls, add_material};

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
    App::new()
        .add_plugins((
            Humentity {
                paths: HumentityPathsConfig::from_crate_path("./"),
                config: HumentityGlobalConfig {
                    translation_tracks: TranslationTracks::None,
                    ..default()
                }
            },
            DefaultPlugins,
        ))
        .add_systems(Startup, setup_env)
        .add_systems(OnExit(HumentityLoadState::LoadingCoreAssets), setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_human)
        .add_systems(Update, (cam_controls, add_materials, add_material))
        .run();
}

fn add_materials(
    skins: Res<CharacterBodyTextures>,
    human_assets: Res<CharacterAssetRegistry>,
    parts: Query<(Entity, &CharacterPart), (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>)>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (entity, part) in parts {
        match part {
            CharacterPart::BaseMesh | CharacterPart::ProxyMesh(_) => {
                let skin = asset_server.load(skins.albedo_maps[SKIN].clone());
                commands.entity(entity).insert(
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(skin.clone()),
                        ..default()
                    }))
                );
            },
            CharacterPart::BodyPart(EYES) => {
                let asset = &human_assets.assets[part];
                let albedo: Handle<Image> = asset_server.load(asset.paths.albedo_maps[&EYE_TEXTURE].clone());
                commands.entity(entity).insert(
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(albedo),
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    }))
                );
            }
            CharacterPart::BodyPart(EYEBROW) => {
                let asset = &human_assets.assets[part];
                let albedo: Handle<Image> = asset_server.load(asset.paths.albedo_maps[&EYEBROW_TEXTURE].clone());
                commands.entity(entity).insert(
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(albedo),
                        base_color: Color::BLACK,
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    }))
                );
            }
            CharacterPart::BodyPart(HAIR) => {
                let asset = &human_assets.assets[part];
                let albedo: Handle<Image> = asset_server.load(asset.paths.albedo_maps[&HAIR_TEXTURE].clone());
                commands.entity(entity).insert(
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(albedo),
                        //base_color: Color::LinearRgba(LinearRgba::RED),
                        alpha_mode: AlphaMode::Blend,
                        clearcoat_perceptual_roughness: 0.1,
                        clearcoat: 0.2,
                        perceptual_roughness: 0.3,
                        reflectance: 0.1,
                        metallic: 0.,
                        ..default()
                    }))
                );
            }
            CharacterPart::BodyPart(EYELASH) => {
                let asset = &human_assets.assets[part];
                let albedo: Handle<Image> = asset_server.load(asset.paths.albedo_maps[&EYELASH_TEXTURE].clone());
                commands.entity(entity).insert(
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(albedo),
                        base_color: Color::LinearRgba(LinearRgba::RED),
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    }))
                );
            }
            _ => {
                // I didnt' make any textures for the basic clothes
                let mat = materials.add(StandardMaterial::from_color(Color::BLACK));
                commands.entity(entity).insert(
                    MeshMaterial3d(mat.clone())
                );
            }
        }
    }
}

fn add_human(
    mut commands: Commands,
) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("female", 1.);

    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morph_targets),
        Visibility::Visible,
        children![(
            CharacterPart::BaseMesh,
            InheritedVisibility::default(),
        ), (
            CharacterPart::BodyPart(EYES),
            InheritedVisibility::default(),
        ), (
            CharacterPart::BodyPart(EYEBROW),
            InheritedVisibility::default(),
        ), (
            CharacterPart::BodyPart(EYELASH),
            InheritedVisibility::default(),
        ), (
            CharacterPart::BodyPart(HAIR),
            InheritedVisibility::default(),
        ), (
            CharacterPart::Equipment(BRA),
            InheritedVisibility::default(),
        ), (
            CharacterPart::Equipment(PANTIES),
            InheritedVisibility::default(),
        )],
    ));
}

fn setup_prefabs(mut commands: Commands, morphs: Res<MakeHumanMorphs>) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.);
    let base_shape = CharacterShapeArchetype::new(
        "female",
        morphs.compute_target_weights(&morph_targets),
    );
    let mut prefabs = AHashMap::default();
    prefabs.insert(
        PREFAB,
        CharacterArchetypePrefab::new(
            vec![base_shape],
            CharacterAnimationArchetype::default(), // No animation in this example
        )
    );

    commands.insert_resource(CharacterArchetypePrefabs::new(prefabs));
}
