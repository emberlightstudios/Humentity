mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::setup_app;

const EYES: &str = "body_parts/Eyes/Eyeballs/high-poly-eyes.mhclo";
const EYEBROW: &str = "body_parts/eyebrows/eyebrows001/eyebrow001.mhclo";
const EYELASH: &str = "body_parts/Eyelashes/false_eyelashes/false_eyelashes.mhclo";
const HAIR: &str = "body_parts/Hair/ponytail01/ponytail01.mhclo";
const BRA: &str = "clothes/underwear/simple_bra/simple_bra.mhclo";
const PANTIES: &str = "clothes/underwear/simple_briefs/simple_briefs.mhclo";
const FEMALE_SHAPE: &str = "female";

fn main() {
    let mut app = setup_app();

    app.add_systems(Startup, spawn_front_of_character_light)
        .add_systems(
            Update,
            add_human.run_if(resource_added::<HumentityAssetsReady>),
        )
        .run();
}

/// A point light on the camera side of the character, so the face is lit from
/// the front rather than only by the shared directional sun in `setup_env`.
/// The character stands at the origin and `CameraFraming::Close` looks at it
/// from `z = -4`, so the light sits at negative `z`.
fn spawn_front_of_character_light(mut commands: Commands) {
    commands.spawn((
        Name::new("Front of character point light"),
        PointLight {
            intensity: 200_000.0,
            range: 8.0,
            ..default()
        },
        Transform::from_xyz(0.5, 1.6, -1.8),
    ));
}

fn add_human(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShape>>,
    mut profile_assets: ResMut<Assets<CharacterProfile>>,
) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.);

    let shape_handle = shape_assets.add(CharacterShape::new(FEMALE_SHAPE, morph_targets));

    let template_handle = template_assets.add(CharacterTemplate::new([shape_handle]));

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
            skeleton_lod: MeshBuildLod::Cpu(0),
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
    // Wet look: the cornea is a tight, bright highlight, so almost no
    // roughness. A matte iris reads as a painted ball. `ior` is not a knob
    // here — 0.20 takes dielectric F0 from `reflectance` alone
    // (`calculate_F0_dielectric`) and reads `ior` only for transmission.
    let eyes_mat = materials.add(StandardMaterial {
        base_color_texture: Some(eyes_albedo),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.02,
        reflectance: 0.6,
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
    // Hair is not a dielectric shell: no clearcoat, high roughness and almost
    // no reflectance, so it reads as strands rather than moulded plastic.
    let hair_mat = materials.add(StandardMaterial {
        base_color_texture: Some(hair_albedo),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.75,
        reflectance: 0.04,
        metallic: 0.,
        ..default()
    });
    let clothes_mat = materials.add(StandardMaterial {
        base_color: Color::LinearRgba(LinearRgba::RED),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    // A template's shapes are baked as morph-target deltas against the rest body,
    // so a shape must be weighted to show at all: an unshaped profile leaves the
    // weight map empty, and every missing weight reads as 0, which renders the
    // unshaped rest body.
    let mut shape_weights = MorphTargets::default();
    shape_weights.insert(FEMALE_SHAPE, 1.);
    let profile_handle = profile_assets.add(CharacterProfile::new(template_handle, shape_weights));

    commands.spawn((
        Name::new("Character"),
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        CharacterRef(profile_handle),
        BuildCpuSkeleton,
        InheritedVisibility::default(),
        children![
            (
                CharacterPart {
                    mesh: basemesh,
                    skeleton_lod: 0
                },
                Name::new("basemesh"),
                MeshMaterial3d(skin_mat)
            ),
            (
                CharacterPart {
                    mesh: eyes,
                    skeleton_lod: 0
                },
                Name::new("eyes"),
                MeshMaterial3d(eyes_mat)
            ),
            (
                CharacterPart {
                    mesh: eyebrow,
                    skeleton_lod: 0
                },
                Name::new("eyebrow"),
                MeshMaterial3d(eyebrow_mat)
            ),
            (
                CharacterPart {
                    mesh: eyelash,
                    skeleton_lod: 0
                },
                Name::new("eyelash"),
                MeshMaterial3d(eyelash_mat)
            ),
            (
                CharacterPart {
                    mesh: hair,
                    skeleton_lod: 0
                },
                Name::new("hair"),
                MeshMaterial3d(hair_mat)
            ),
            (
                CharacterPart {
                    mesh: bra,
                    skeleton_lod: 0
                },
                Name::new("bra"),
                MeshMaterial3d(clothes_mat.clone())
            ),
            (
                CharacterPart {
                    mesh: panties,
                    skeleton_lod: 0
                },
                Name::new("panties"),
                MeshMaterial3d(clothes_mat)
            ),
        ],
    ));
}
