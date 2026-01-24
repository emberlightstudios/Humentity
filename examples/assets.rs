mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::{cam_controls, setup_env, add_humentity_plugin};

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

    app
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup_env)
        .add_systems(OnExit(HumentityLoadState::LoadingCoreAssets), setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_human)
        .add_systems(Update, (cam_controls, add_materials))
        .run();
}

fn add_materials(
    skins: Res<CharacterBodyTextures>,
    mut human_assets: ResMut<CharacterAssetRegistry>,
    parts: Query<
        (Entity, &CharacterPart),
        (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>),
    >,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (entity, part) in parts {
        match part {
            CharacterPart::BaseMesh | CharacterPart::ProxyMesh(_) => {
                let skin: Handle<Image> = skins.albedo_maps[SKIN].load_asset(&asset_server);
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(skin),
                        ..default()
                    })));
            }
            CharacterPart::BodyPart(EYES) => {
                let asset = human_assets.get_mut(part).unwrap();
                let albedo: Handle<Image> = asset.get_texture_handle(EYE_TEXTURE, CharacterAssetTextureType::Albedo, &asset_server);
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(albedo),
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    })));
            }
            CharacterPart::BodyPart(EYEBROW) => {
                let asset = human_assets.get_mut(part).unwrap();
                let albedo: Handle<Image> = asset.get_texture_handle(EYEBROW_TEXTURE, CharacterAssetTextureType::Albedo, &asset_server);
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(albedo),
                        base_color: Color::BLACK,
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    })));
            }
            CharacterPart::BodyPart(HAIR) => {
                let asset = human_assets.get_mut(part).unwrap();
                let albedo: Handle<Image> = asset.get_texture_handle(HAIR_TEXTURE, CharacterAssetTextureType::Albedo, &asset_server);
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(albedo),
                        //base_color: Color::LinearRgba(LinearRgba::RED),
                        alpha_mode: AlphaMode::Blend,
                        clearcoat_perceptual_roughness: 0.1,
                        clearcoat: 0.2,
                        perceptual_roughness: 0.3,
                        reflectance: 0.1,
                        metallic: 0.,
                        ..default()
                    })));
            }
            CharacterPart::BodyPart(EYELASH) => {
                let asset = &human_assets[part];
                let albedo: Handle<Image> = asset.paths.albedo_maps[&EYELASH_TEXTURE]
                    .load_asset(&*asset_server);
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(albedo),
                        base_color: Color::LinearRgba(LinearRgba::RED),
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    })));
            }
            _ => {
                // I didnt' make any textures for the basic clothes
                let mat = materials.add(StandardMaterial::from_color(Color::BLACK));
                commands.entity(entity).insert(MeshMaterial3d(mat.clone()));
            }
        }
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
            (CharacterPart::BaseMesh, InheritedVisibility::default(),),
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
    let base_shape =
        CharacterShapeArchetype::new("female".to_string(), morphs.compute_target_weights(&morph_targets));
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
