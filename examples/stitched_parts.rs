/// Mesh stitching allows you to create CharacterParts which are themselves composed of smaller parts.
/// This demonstrates solving the normal-discontinuity problem at seam cuts, and also shows how
/// per-part morph targets let you put facial expression morphs only on the head mesh, not the body.
///
/// The problem with splitting meshes into multiple pieces is that when you cut a mesh at an edge loop,
/// most 3d modelling software will autmoatically recompute normals at the loop and create a discontinuity
/// of mesh normals across the seam.  The normals will no longer be smooth and lighting will make a line
/// obvious where you cut.  This is the main problem intended to be solved by mesh stitching.
///
/// Additionally, because each stitched part can use a different prefab, expression morphs
/// (which only deform face helpers) can be baked only into the head mesh and not the body.
/// This avoids unnecessary shapekeys on the body and prevents any seam displacement.
///
/// Note that the normal smoothing algorithm requires that the vert positions are bitwise identical on both
/// sides of your edge loop cuts.

mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::setup_app;

const STITCHED: &str = "stitched";
const HEAD_PREFAB: &str = "head_prefab";

fn main() {
    let mut app = setup_app();

    app.add_systems(
        Update,
        add_humans
            .run_if(resource_exists::<MakeHumanMorphs>)
            .run_if(not(resource_exists::<CharacterArchetypePrefabs>)),
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
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.0);

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

    let shape = CharacterShapeArchetype::new("woman", resolved);

    // Expression morph — only defined on the head-portion prefab so it never touches the body mesh
    let mut expression_targets = MorphTargets::default();
    expression_targets.insert("jawOpen", 1.0);
    let expression_shape = CharacterShapeArchetype::new("jawOpen", expression_targets);

    commands.insert_resource(CharacterArchetypePrefabs::new([
        (STITCHED, CharacterArchetypePrefab::new([shape.clone()], RigType::Default)),
        (HEAD_PREFAB, CharacterArchetypePrefab::new([shape, expression_shape], RigType::Default)),
    ]));

    // Split pieces for stitched demonstration
    let headless = asset_server.load::<MhcloAsset>(
        "proxymeshes/basemesh_headless/basemesh_headless.mhclo",
    );
    let head = asset_server.load::<MhcloAsset>(
        "proxymeshes/basemesh_head/basemesh_head.proxy",
    );

    // Stitched: head + body reconnected with continuous normals.
    // The head uses a separate prefab so expression morphs are baked only into the head mesh.
    mesh_builder.trigger(LoadAssetMeshJob::Stitched {
        parts: StitchedParts(vec![
            StitchedPart::from(headless.clone()),
            StitchedPart::from(head.clone()).with_prefab_override(HEAD_PREFAB),
        ]),
        prefab_name: STITCHED,
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
            (CharacterPart(head), Name::new("head"), PrefabOverride(HEAD_PREFAB), MeshMaterial3d(white)),
        ],
    ));
}
