mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::setup_app;

const PREFAB: &str = "prefab";
const EYES: &str = "body_parts/Eyes/Eyeballs/high-poly-eyes.mhclo";
const EYEBROW: &str = "body_parts/eyebrows/eyebrows001/eyebrow001.mhclo";
const EYELASH: &str = "body_parts/Eyelashes/false_eyelashes/false_eyelashes.mhclo";
const HAIR: &str = "body_parts/Hair/ponytail01/ponytail01.mhclo";
const BRA: &str = "clothes/underwear/simple_bra/simple_bra.mhclo";
const PANTIES: &str = "clothes/underwear/simple_briefs/simple_briefs.mhclo";

fn main() {
    let mut app = setup_app();

    app.add_systems(
        Update,
        add_human
            .run_if(resource_exists::<MakeHumanMorphs>)
            .run_if(not(resource_exists::<CharacterArchetypePrefabs>)),
    )
    .run();
}

fn add_human(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    morphs: Res<MakeHumanMorphs>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.);

    let resolved = match morphs.compute_target_weights(&morph_targets) {
        Ok(m) => m,
        Err(err) => {
            error!("Error computing morph targets: {err}");
            return;
        }
    };

    for k in resolved.keys() {
        let loaded = morphs.targets.read().unwrap();
        if !loaded.contains_key(k) {
            return;
        }
    }

    let shape = CharacterShapeArchetype::new("female", resolved);

    commands.insert_resource(CharacterArchetypePrefabs::new([(
        PREFAB,
        CharacterArchetypePrefab::new([shape], RigType::Default),
    )]));

    let basemesh = CharacterPart(
        asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy"),
    );
    let eyes = CharacterPart(asset_server.load::<MhcloAsset>(EYES));
    let eyebrow = CharacterPart(asset_server.load::<MhcloAsset>(EYEBROW));
    let eyelash = CharacterPart(asset_server.load::<MhcloAsset>(EYELASH));
    let hair = CharacterPart(asset_server.load::<MhcloAsset>(HAIR));
    let bra = CharacterPart(asset_server.load::<MhcloAsset>(BRA));
    let panties = CharacterPart(asset_server.load::<MhcloAsset>(PANTIES));

    for part in [&basemesh, &eyes, &eyebrow, &eyelash, &hair, &bra, &panties] {
        mesh_builder.trigger(LoadAssetMeshJob::Single {
            part: part.clone(),
            prefab_name: PREFAB,
        });
    }

    let skin_albedo =
        asset_server.load::<Image>("skin_textures/albedo/young_caucasian_female.png");
    let eyes_albedo = asset_server.load::<Image>("body_parts/Eyes/Eyeballs/albedo/blue_eye.png");
    let eyebrow_albedo =
        asset_server.load::<Image>("body_parts/eyebrows/eyebrows001/albedo/eyebrow001.png");
    let eyelash_albedo =
        asset_server.load::<Image>("body_parts/Eyelashes/false_eyelashes/albedo/false_eyelashes.png");
    let hair_albedo =
        asset_server.load::<Image>("body_parts/Hair/ponytail01/albedo/ponytail01.png");

    let skin_mat = materials.add(StandardMaterial {
        base_color_texture: Some(skin_albedo),
        ..default()
    });
    let eyes_mat = materials.add(StandardMaterial {
        base_color_texture: Some(eyes_albedo),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let eyebrow_mat = materials.add(StandardMaterial {
        base_color_texture: Some(eyebrow_albedo),
        base_color: Color::BLACK,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let eyelash_mat = materials.add(StandardMaterial {
        base_color_texture: Some(eyelash_albedo),
        base_color: Color::LinearRgba(LinearRgba::RED),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let hair_mat = materials.add(StandardMaterial {
        base_color_texture: Some(hair_albedo),
        alpha_mode: AlphaMode::Blend,
        clearcoat_perceptual_roughness: 0.1,
        clearcoat: 0.2,
        perceptual_roughness: 0.3,
        reflectance: 0.1,
        metallic: 0.,
        ..default()
    });
    let clothes_mat = materials.add(StandardMaterial {
        base_color: Color::LinearRgba(LinearRgba::RED),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("female", 1.);

    commands.spawn((
        Name::new("Character"),
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        CharacterShapeConfig::new(PREFAB, morph_targets),
        InheritedVisibility::default(),
        children![
            (basemesh, Name::new("basemesh"), MeshMaterial3d(skin_mat)),
            (eyes, Name::new("eyes"), MeshMaterial3d(eyes_mat)),
            (eyebrow, Name::new("eyebrow"), MeshMaterial3d(eyebrow_mat)),
            (eyelash, Name::new("eyelash"), MeshMaterial3d(eyelash_mat)),
            (hair, Name::new("hair"), MeshMaterial3d(hair_mat)),
            (bra, Name::new("bra"), MeshMaterial3d(clothes_mat.clone())),
            (panties, Name::new("panties"), MeshMaterial3d(clothes_mat)),
        ],
    ));
}
