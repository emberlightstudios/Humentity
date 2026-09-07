//! Makehuman allows character customization through the use of morph targets,
//! also knows as blendshapes or shapekeys.  One possible architecture for this
//! crate could be to accept a set of morph values and bake the resulting
//! mesh down to a new fixed mesh.  One problem with this approach is that it
//! breaks instancing/batching between different humans, and therefore
//! performance degrades, as well memory usage explodes since each individual
//! mesh, skinnedmesh, etc. must occcupy it's own space in the AssetServer/GPU buffers.
//! To overcome these problems Humentity uses a "template" system.
//!
//! Makehuman has something like 1000 distinct morph targets.  This
//! is too many to be on a mesh at runtime.  The Humentity template system
//! allows you to bake an arbitrary set of makehuman morph weights down to a
//! single morph target in bevy. In order to make variable humans we can define
//! a few basic human archetypes, and perhaps a set of distinct faces that we can
//! use to blend between at runtime.  This allows us to dramatically reduce the
//! number of morph targets while still allowing at least some runtime mesh
//! customization, and keeping instancing/batching intact, since each template
//! is still the same mesh handle (assuming they all use the same material also).

mod shared;

use bevy::prelude::*;
use humentity::prelude::*;
use shared::setup_app;

const BABY: &str = "baby";
const BODYBUILDER: &str = "bodybuilder";

fn main() {
    let mut app = setup_app();

    app.add_systems(Startup, add_humans)
        .add_observer(on_skeletons_ready)
        .run();
}

/// Skeletons start fully enabled by default; here we narrow the active LOD set
/// to LOD 0 so bone sub-trees it doesn't reference get disabled.
fn on_skeletons_ready(trigger: On<Add, SkeletonsReady>, mut commands: Commands) {
    let mut active = [false; MAX_LODS];
    active[0] = true;
    commands
        .entity(trigger.entity)
        .insert(SkeletonLodState { active });
}

fn add_humans(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {
    let mut baby_targets = MorphTargets::default();
    baby_targets.insert("age", 0.);

    let mut bodybuilder_targets = MorphTargets::default();
    bodybuilder_targets.insert("muscle", 1.);
    bodybuilder_targets.insert("weight", 1.);

    // Templates can be created at runtime or loaded from toml
    let template_handle = template_assets.add(CharacterTemplate::new([
        CharacterMorphShape::new(BODYBUILDER, bodybuilder_targets),
        CharacterMorphShape::new(BABY, baby_targets),
    ]));

    // Previously defined shapes will now appear as morph targets on the template's mesh
    // The HumanShapeConfig type controls template access and applies our morph targets.
    let basemesh_part = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    // Trigger the mesh to build with the new morph targets.
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh_part.clone(),
        template_handle: template_handle.clone(),
        skeleton_lod: 0,
    });

    // Spawn some characters with different morph values.  They will all share the same mesh handle, but look different!
    // CharacterShapeCOnfig can be loaded from toml or created at runtime

    // The base mesh
    commands.spawn((
        Name::new("Basemesh"),
        Transform::from_translation(Vec3::new(-2., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShape(shape_assets.add(template_handle.clone())),
        HelperVertexPositions::default(),
        children![
            (CharacterPart {
                mesh: basemesh_part.clone(),
                skeleton_lod: 0
            })
        ],
    ));

    // A baby
    let mut morphs = MorphTargets::default();
    morphs.insert(BABY, 1.);
    morphs.insert(BODYBUILDER, 0.);
    commands.spawn((
        Name::new("Baby"),
        Transform::from_translation(Vec3::new(-1., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(
            template_handle.clone(),
            morphs.clone(),
        ))),
        HelperVertexPositions::default(),
        children![(
            CharacterPart {
                mesh: basemesh_part.clone(),
                skeleton_lod: 0
            },
            Name::new("mesh"),
        )],
    ));

    // A bodybuilder
    morphs.insert(BABY, 0.);
    morphs.insert(BODYBUILDER, 1.);
    commands.spawn((
        Name::new("Bodybuilder"),
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(
            template_handle.clone(),
            morphs.clone(),
        ))),
        HelperVertexPositions::default(),
        children![(
            Name::new("mesh"),
            CharacterPart {
                mesh: basemesh_part.clone(),
                skeleton_lod: 0
            }
        )],
    ));

    // Half baby/half bodybuilder, ha!
    // Note that the shapekey weights sum to 1
    morphs.insert(BABY, 0.5);
    morphs.insert(BODYBUILDER, 0.5);
    commands.spawn((
        Name::new("Hybrid normalized"),
        Transform::from_translation(Vec3::new(1., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(
            template_handle.clone(),
            morphs.clone(),
        ))),
        HelperVertexPositions::default(),
        children![(
            CharacterPart {
                mesh: basemesh_part.clone(),
                skeleton_lod: 0
            },
            Name::new("mesh"),
        )],
    ));

    // You have to be careful with normalization of mixed shapekeys sometimes
    // or you might end up with artifacts!
    // Here is a baby/bodybuilder mix, without normalizing
    morphs.insert(BABY, 1.);
    morphs.insert(BODYBUILDER, 1.);
    commands.spawn((
        Name::new("Hybrid unnormalized"),
        Transform::from_translation(Vec3::new(2., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(template_handle, morphs))),
        HelperVertexPositions::default(),
        children![(
            CharacterPart {
                mesh: basemesh_part,
                skeleton_lod: 0
            },
            Name::new("mesh"),
        )],
    ));
}
