/// Mesh stitching allows you to create CharacterParts which are themselves composed of smaller parts.
/// This demonstrates solving the normal-discontinuity problem at seam cuts, and also shows how
/// per-part morph targets let you put facial expression morphs only on the head mesh, not the body.
///
/// The problem with splitting meshes into multiple pieces is that normals at the seams are not continuous.
/// The leads to lighting artifacts at the seam.  This is the main problem intended to be solved by mesh stitching.
///
/// Additionally, we can isolate shapes to smaller meshes.  In this example we put facial expressions on a separate
/// template that only the head mesh uses because the body mesh doesn't care about facial expressions. 
/// This helps optimize vram usage.
/// 
/// Note that the normal smoothing algorithm requires that the vert positions are bitwise identical on both
/// sides of your edge loop cuts.

mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::{setup_app, CharacterPart};

const STITCHED: &str = "stitched";
const HEAD_TEMPLATE: &str = "head_template";

fn main() {
    let mut app = setup_app();

    app.add_systems(
        Update,
        add_humans
            .run_if(resource_exists::<MakeHumanMorphs>)
            .run_if(not(resource_exists::<CharacterTemplates>)),
    )
    .run();
}

fn add_humans(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    morphs: Res<MakeHumanMorphs>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !morphs.is_ready(&asset_server) {
        return;
    }

    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.0);

    let resolved = morphs.compute_target_weights(&morph_targets).unwrap();

    let shape = CharacterMorphShapes::new("woman", resolved);

    // Expression morph — only defined on the head-portion template so it never touches the body mesh
    let mut expression_targets = MorphTargets::default();
    expression_targets.insert("jawOpen", 1.0);
    let expression_shape = CharacterMorphShapes::new("jawOpen", expression_targets);

    commands.insert_resource(CharacterTemplates::new([
        (STITCHED, CharacterTemplate::new([shape.clone()], RigType::Default)),
        (HEAD_TEMPLATE, CharacterTemplate::new([shape, expression_shape], RigType::Default)),
    ]));

    // Split pieces for stitched demonstration
    let headless = asset_server.load::<MhcloAsset>(
        "proxymeshes/basemesh_headless/basemesh_headless.mhclo",
    );
    let head = asset_server.load::<MhcloAsset>(
        "proxymeshes/basemesh_head/basemesh_head.proxy",
    );

    // Stitched: head + body reconnected with continuous normals.
    // The head uses a separate template so expression morphs are baked only into the head mesh.
    mesh_builder.trigger(LoadAssetMeshJob::Stitched {
        parts: StitchedParts(vec![
            StitchedPart::from(headless.clone()),
            StitchedPart::from(head.clone()).with_template_override(HEAD_TEMPLATE),
        ]),
        template_name: STITCHED,
    });

    let white = materials.add(StandardMaterial::from_color(Color::WHITE));

    let mut morphs = MorphTargets::default();
    morphs.insert("woman", 1.);
    morphs.insert("jawOpen", 0.5);

    commands.spawn((
        Name::new("Stitched"),
        Transform::from_translation(Vec3::new(0., 0., -1.)),
        CharacterShapeConfig::new(STITCHED, morphs),
        InheritedVisibility::default(),
        children![
            (CharacterPart(headless), Name::new("headless"), MeshMaterial3d(white.clone())),
            (CharacterPart(head), Name::new("head"), TemplateOverride(HEAD_TEMPLATE), MeshMaterial3d(white)),
        ],
    ));
}
