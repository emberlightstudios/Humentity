//! This is a simple example showing the animation features.
//! The base makehuman mesh with helpers has an idle loop animation
//! Humentity rewrites the animation to try to make it compatible with
//! different sized humans, e.g. the baby mesh.  
//!
//! Important notes:
//!  - AnimationTargetId matching requires you to leave the base object name as its
//!    default from blender.  This is "Human.rig" after you add a rig.  Do not change it.
//!  - Retargeting assumes that all animation clips are authored on a humanoid with the
//!    shape of the base mesh with no morphs applied.  Remove all morphs from your human
//!    before authoring animation clips.

mod shared;

use std::f32::consts::PI;

use bevy::{prelude::*, scene::SceneInstanceReady};
use humentity::prelude::*;
use shared::setup_app;

const TEMPLATE_NAME: &str = "ExampleHumanTemplate";
const BABY: &str = "baby";

fn main() {
    let mut app = setup_app();

    app
        .add_plugins(BoneDebugPlugin)
        .add_systems(
        Update,
        (
            add_humans
                .run_if(resource_exists::<MakeHumanMorphs>)
                .run_if(not(resource_exists::<CharacterTemplates>)),
            play_graph,
            add_graph,
        ),
    )
    .run();
}

// Hold handle refs to keep the clip assets alive
#[derive(Resource)]
struct RetargetedAnimations {
    clips: Handle<RetargetedAnimationAsset>,
}

#[derive(Component, Clone)]
struct AnimationIndex(AnimationNodeIndex);

fn add_humans(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    morphs: Res<MakeHumanMorphs>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // Set up the dynamic character resources
    if !morphs.is_ready(&asset_server) {
        return;
    }

    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("age", 0.);

    let baby_morphs = morphs.compute_target_weights(&morph_targets).unwrap();

    commands.insert_resource(CharacterTemplates::new([
        (
            TEMPLATE_NAME,
            CharacterTemplate::new(
                [CharacterMorphShapes::new(BABY, baby_morphs)],
                RigType::Default,
            ),
        )
    ]));
    info!("GLB scene loaded");

    // Spawn the raw GLB animation scene for comparison, includes basemesh+helpers
    let clip_handle = asset_server.load(GltfAssetLabel::Animation(0).from_asset("animation/idle.glb"));
    let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());
    let graph_handle = graphs.add(graph);

    commands.spawn((
        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("animation/idle.glb"))),
        Transform::from_translation(Vec3::new(-1., 0., 0.))
            .with_rotation(Quat::from_rotation_y(PI)),
        AnimationIndex(index),
        AnimationGraphHandle(graph_handle.clone()),
    )).observe(on_gltf_scene_ready);


    // Load the dynamic avatar with morphs and a retargeted clip

    // Create the morphable base mesh
    let basemesh_part =
        CharacterPart(asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy"));

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh_part.clone(),
        template_name: TEMPLATE_NAME,
    });

    // Spawn the character with baby morphs
    let mut morphs = MorphTargets::default();
    morphs.insert(BABY, 1.);
    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        Name::new("Retargeted"),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(TEMPLATE_NAME, morphs),
        children![(
            Name::new("Mesh"),
            basemesh_part,
        )],
    ));

    // Load the animation clip with translation tracks removed for different human shapes.
    // Since this also loads via gltf we have to pass the type explicitly
    // The clips are loaded on the RetargetedAnimationAsset as a hashmap
    // Use loader settings to retain translation tracks (experimental/broken)
    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnimations{ clips });

    info!("Baby created");
}

fn on_gltf_scene_ready(
    trigger: On<SceneInstanceReady>,
    q: Query<(&AnimationIndex, &AnimationGraphHandle)>,
    mut players: Query<&mut AnimationPlayer>,
    children: Query<&Children>,
    mut commands: Commands,
) {
    let (index, graph) = q.get(trigger.entity).unwrap();
    for child in children.iter_descendants(trigger.entity) {
        if let Ok(mut player) = players.get_mut(child) {
            commands.entity(child).insert(graph.clone());
            player.play(index.0).repeat();
            return;
        }
    }
}

fn add_graph(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    retargeted_clips: Res<Assets<RetargetedAnimationAsset>>,
    mut character_player: Query<(Entity, &mut AnimationPlayer), Without<AnimationGraphHandle>>,
) {
    let Ok((entity, mut player)) = character_player.single_mut() else { return };
    if let Some((_id, clips_map)) = retargeted_clips.iter().next() {
        let clip_handle = clips_map.clips.get("Idle-loop").unwrap();
        let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());
        commands.entity(entity.clone()).insert((
            AnimationIndex(index),
            AnimationGraphHandle(graphs.add(graph)),
        ));
        player.play(index).repeat();
        info!("Adding graph handle to retargeted character");
    }
}

fn play_graph(
    mut players: Query<(&mut AnimationPlayer, &AnimationIndex), Added<AnimationGraphHandle>>,
) {
    for (mut player,node_index) in players.iter_mut() {
        info!("Starting clip");
        player.play(node_index.0).repeat();
    }
}
