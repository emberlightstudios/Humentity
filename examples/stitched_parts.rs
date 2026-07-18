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
use shared::setup_app;

fn main() {
    let mut app = setup_app();

    app.add_observer(add_humans).run();
}

fn add_humans(
    _trigger: On<MorphsReady>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.0);
    let shape = CharacterMorphShape::new("woman", morph_targets);

    // Expression morph — only defined on the head-portion template so it never touches the body mesh
    let mut expression_targets = MorphTargets::default();
    expression_targets.insert("jawOpen", 1.0);
    let expression_shape = CharacterMorphShape::new("jawOpen", expression_targets);

    let body_template_handle = template_assets.add(CharacterTemplate::new([shape.clone()]));
    let head_template_handle =
        template_assets.add(CharacterTemplate::new([shape, expression_shape]));

    // Split pieces for stitched demonstration
    let headless =
        asset_server.load::<MhcloAsset>("proxymeshes/basemesh_headless/basemesh_headless.mhclo");
    let head = asset_server.load::<MhcloAsset>("proxymeshes/basemesh_head/basemesh_head.proxy");

    // Stitched: head + body reconnected with continuous normals.
    // The head uses a separate template so expression morphs are baked only into the head mesh.
    mesh_builder.trigger(LoadAssetMeshJob::Stitched {
        parts: StitchedParts(vec![
            StitchedPart::from(headless.clone()),
            StitchedPart::from(head.clone()).with_template_override(head_template_handle.clone()),
        ]),
        template_handle: body_template_handle.clone(),
    });

    let white = materials.add(StandardMaterial::from_color(Color::WHITE));

    let mut morphs = MorphTargets::default();
    morphs.insert("woman", 1.);
    morphs.insert("jawOpen", 0.5);

    commands.spawn((
        Name::new("Stitched"),
        Transform::from_translation(Vec3::new(0., 0., -1.)),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(body_template_handle, morphs))),
        InheritedVisibility::default(),
        children![
            (
                CharacterPart {
                    mesh: headless,
                    lod: 0
                },
                Name::new("headless"),
                MeshMaterial3d(white.clone())
            ),
            (
                CharacterPart { mesh: head, lod: 0 },
                Name::new("head"),
                TemplateOverride(head_template_handle),
                MeshMaterial3d(white)
            ),
        ],
    ));
}
