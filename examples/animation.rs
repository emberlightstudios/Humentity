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

use bevy::{gltf::Gltf, prelude::*};
use humentity::prelude::*;
use shared::setup_app;

const PREFAB_NAME: &str = "ExampleHumanPrefab";
const BABY: &str = "baby";

fn main() {
    let mut app = setup_app();

    app.add_systems(
        Update,
        (
            add_humans
                .run_if(resource_exists::<MakeHumanMorphs>)
                .run_if(not(resource_exists::<CharacterArchetypePrefabs>)),
            clip_loaded,
            add_graph,
            play_graph,
        ),
    )
    .run();
}

#[derive(Resource)]
struct RetargetedAnimations {
    glb_clips: Handle<RetargetedAnimationAsset>,
}

#[derive(Component, Clone)]
struct Animationindex(AnimationNodeIndex);

fn add_humans(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    morphs: Res<MakeHumanMorphs>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // Spawn the raw GLB animation scene for comparison
    // Load as Gltf to force the GLTF loader (not the retargeted one)
    //let clip_handle = asset_server.load::<AnimationClip>("animation/idle.glb");
    //let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());
    //let graph_handle = graphs.add(graph);

    //commands.spawn((
    //    SceneRoot(asset_server.load::<Scene>("animation/idle.glb")),
    //    Transform::from_translation(Vec3::new(-1., 0., 0.))
    //        .with_rotation(Quat::from_rotation_y(PI)),
    //    Animationindex(index),
    //    AnimationGraphHandle(graph_handle.clone()),
    //));//.observe();


    // Spawn the dynamic character with retargeted animation
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("age", 0.);

    let baby_morphs = match morphs.compute_target_weights(&morph_targets) {
        Ok(morphs) => morphs,
        Err(err) => {
            error!("Error computing morph targets for baby: {err}");
            return;
        }
    };

    for k in baby_morphs.keys() {
        let loaded = morphs.targets.read().unwrap();
        if !loaded.contains_key(k) {
            return;
        }
    }

    commands.insert_resource(CharacterArchetypePrefabs::new([(
        PREFAB_NAME,
        CharacterArchetypePrefab::new(
            [CharacterShapeArchetype::new(BABY, baby_morphs)],
            RigType::Default,
        ),
    )]));

    let basemesh_part =
        CharacterPart(asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy"));

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh_part.clone(),
        prefab_name: PREFAB_NAME,
    });

    // Spawn the character with baby morphs
    let mut morphs = MorphTargets::default();
    morphs.insert(BABY, 1.);
    commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(PREFAB_NAME, morphs),
        children![(basemesh_part)],
    ));

    // Load the animation clip with translation traks removed for different human shapes.
    // You can use loader settings to retain translation tracks (still experimental)
    let clip = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnimations { glb_clips: clip });

}

fn clip_loaded(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    retargeted_clips: Res<Assets<RetargetedAnimationAsset>>,
    character: Single<Entity, With<CharacterShapeConfig>>,
    mut msgs: MessageReader<AssetEvent<RetargetedAnimationAsset>>,
) {
    for msg in msgs.read() {
        if let AssetEvent::LoadedWithDependencies { id } = msg {
            let clips_map = retargeted_clips.get(*id).unwrap();
            info!("{:#?}", clips_map.clips.keys());
            let clip_handle = clips_map.clips.get("Idle-loop").unwrap();

            let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());
            commands.entity(*character).insert((
                Animationindex(index),
                AnimationGraphHandle(graphs.add(graph)),
            ));
        }
    }
}

fn add_graph(
    q: Query<Entity, With<AnimationPlayer>>,
    mut commands: Commands,
    children: Query<&Children>,
    anim_data: Query<(Entity, &Animationindex, &AnimationGraphHandle)>,
) {
    for (entity, node_index, graph_handle) in anim_data.iter() {
        for child in children.iter_descendants(entity) {
            if q.get(child).is_ok() {
                commands
                    .entity(child)
                    .insert((node_index.clone(), graph_handle.clone()));
                commands
                    .entity(entity)
                    .remove::<Animationindex>()
                    .remove::<AnimationGraphHandle>();
            }
        }
    }
}

fn play_graph(
    mut players: Query<(&mut AnimationPlayer, &Animationindex), Added<AnimationGraphHandle>>,
) {
    for (mut player, node_index) in players.iter_mut() {
        player.play(node_index.0).repeat();
    }
}
