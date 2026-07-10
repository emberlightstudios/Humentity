mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::{setup_app, CharacterPart};

const EYES: &str = "body_parts/Eyes/Eyeballs/high-poly-eyes.mhclo";
const EYEBROW: &str = "body_parts/eyebrows/eyebrows001/eyebrow001.mhclo";
const EYELASH: &str = "body_parts/Eyelashes/false_eyelashes/false_eyelashes.mhclo";
const HAIR: &str = "body_parts/Hair/ponytail01/ponytail01.mhclo";
const BRA: &str = "clothes/underwear/simple_bra/simple_bra.mhclo";
const PANTIES: &str = "clothes/underwear/simple_briefs/simple_briefs.mhclo";

fn main() {
    let mut app = setup_app();

    app.add_observer(add_human).run();
}

fn add_human(
    _trigger: On<MorphsReady>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.);

    let shape = CharacterMorphShape::new("female", morph_targets);

    let template_handle = template_assets.add(CharacterTemplate::new([shape], RigType::Default));

    let basemesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");
    let eyes = asset_server.load::<MhcloAsset>(EYES);
    let eyebrow = asset_server.load::<MhcloAsset>(EYEBROW);
    let eyelash = asset_server.load::<MhcloAsset>(EYELASH);
    let hair = asset_server.load::<MhcloAsset>(HAIR);
    let bra = asset_server.load::<MhcloAsset>(BRA);
    let panties = asset_server.load::<MhcloAsset>(PANTIES);

    for part in [&basemesh, &eyes, &eyebrow, &eyelash, &hair, &bra, &panties] {
        mesh_builder.trigger(LoadAssetMeshJob::Single {
            part: part.clone(),
            template_handle: template_handle.clone(),
        });
    }

    let skin_albedo = asset_server.load::<Image>("skin_textures/albedo/young_caucasian_female.png");
    let eyes_albedo = asset_server.load::<Image>("body_parts/Eyes/Eyeballs/albedo/blue_eye.png");
    let eyebrow_albedo =
        asset_server.load::<Image>("body_parts/eyebrows/eyebrows001/albedo/eyebrow001.png");
    let eyelash_albedo = asset_server
        .load::<Image>("body_parts/Eyelashes/false_eyelashes/albedo/false_eyelashes.png");
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

    // A CharacterShapeAsset can be constructed from just a template handle without specifying morph
    // targets.  Single-shape templates like this one bake that shape down into the base mesh. 
    // In that case there are no morph targets on the final mesh.
    let shape_handle = shape_assets.add(template_handle.clone());

    commands.spawn((
        Name::new("Character"),
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        CharacterShape(shape_handle),
        InheritedVisibility::default(),
        children![
            (
                CharacterPart(basemesh),
                Name::new("basemesh"),
                MeshMaterial3d(skin_mat)
            ),
            (
                CharacterPart(eyes),
                Name::new("eyes"),
                MeshMaterial3d(eyes_mat)
            ),
            (
                CharacterPart(eyebrow),
                Name::new("eyebrow"),
                MeshMaterial3d(eyebrow_mat)
            ),
            (
                CharacterPart(eyelash),
                Name::new("eyelash"),
                MeshMaterial3d(eyelash_mat)
            ),
            (
                CharacterPart(hair),
                Name::new("hair"),
                MeshMaterial3d(hair_mat)
            ),
            (
                CharacterPart(bra),
                Name::new("bra"),
                MeshMaterial3d(clothes_mat.clone())
            ),
            (
                CharacterPart(panties),
                Name::new("panties"),
                MeshMaterial3d(clothes_mat)
            ),
        ],
    ));
}
