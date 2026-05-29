//! Makehuman comes with several lower poly proxy meshes.  These
//! can be used for lods with the VisibilityRanges component
//! In this example we switch lods early just for clarity.

mod shared;

use bevy::{camera::visibility::VisibilityRange, prelude::*};
use humentity::prelude::*;
use shared::{setup_app, CharacterPart};

const SHAPE_NAME: &str = "bigboobs";

fn main() {
    let mut app = setup_app();

    app.add_observer(add_humans).run();
}

fn add_humans(
    _trigger: On<MorphsReady>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    morphs: Res<MakeHumanMorphs>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("gender", 0.0);
    morph_targets.insert("cupsize", 1.0);

    let resolved = morphs.compute_target_weights(&morph_targets).unwrap();

    let shape = CharacterMorphShape::new(SHAPE_NAME, resolved);

    let template_handle = template_assets.add(CharacterTemplate::new([shape], RigType::Default));

    let lod0 = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");
    let lod1 = asset_server.load::<MhcloAsset>("proxymeshes/proxy4817/proxy4817.proxy");
    let lod2 = asset_server.load::<MhcloAsset>("proxymeshes/proxy1605/proxy1605.proxy");
    let lod3 = asset_server.load::<MhcloAsset>("proxymeshes/proxy741/proxy741.proxy");

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod0.clone(),
        template_handle: template_handle.clone(),
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod1.clone(),
        template_handle: template_handle.clone(),
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod2.clone(),
        template_handle: template_handle.clone(),
    });
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: lod3.clone(),
        template_handle: template_handle.clone(),
    });

    let mut morphs = MorphTargets::default();
    morphs.insert(SHAPE_NAME, 1.);

    let white = materials.add(StandardMaterial::from_color(Color::WHITE));
    let black = materials.add(StandardMaterial::from_color(Color::BLACK));

    // LOD character with all proxy meshes as children, each with a VisibilityRange
    commands.spawn((
        Name::new("LOD Character"),
        Transform::from_translation(Vec3::new(0., 0., -1.)),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(
            template_handle.clone(),
            morphs.clone(),
        ))),
        InheritedVisibility::default(),
        children![
            (
                CharacterPart(lod0.clone()),
                Name::new("basemesh"),
                MeshMaterial3d(white.clone()),
                VisibilityRange {
                    start_margin: 0.0..0.0,
                    end_margin: 2.0..3.0,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart(lod1.clone()),
                Name::new("proxy4817"),
                MeshMaterial3d(white.clone()),
                VisibilityRange {
                    start_margin: 2.0..3.0,
                    end_margin: 7.0..8.0,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart(lod2.clone()),
                Name::new("proxy1605"),
                MeshMaterial3d(white.clone()),
                VisibilityRange {
                    start_margin: 7.0..8.0,
                    end_margin: 14.0..15.0,
                    use_aabb: false,
                }
            ),
            (
                CharacterPart(lod3.clone()),
                Name::new("proxy741"),
                MeshMaterial3d(white),
                VisibilityRange {
                    start_margin: 14.0..15.0,
                    end_margin: 20.0..30.0,
                    use_aabb: false,
                }
            )
        ],
    ));

    // Reference characters showing individual proxy meshes
    for (proxy, name, x) in [
        (lod0, "basemesh ref", -1.5),
        (lod1, "proxy4817 ref", -0.5),
        (lod2, "proxy1605 ref", 0.5),
        (lod3, "proxy741 ref", 1.5),
    ] {
        commands.spawn((
            Name::new(name),
            Transform::from_translation(Vec3::new(x, 0., 0.)),
            CharacterShape(shape_assets.add(CharacterShapeAsset::new(
                template_handle.clone(),
                morphs.clone(),
            ))),
            InheritedVisibility::default(),
            children![(
                CharacterPart(proxy),
                Name::new("mesh"),
                MeshMaterial3d(black.clone())
            )],
        ));
    }
}
