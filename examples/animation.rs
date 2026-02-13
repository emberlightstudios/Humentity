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
use ahash::AHashMap;
use bevy::{mesh::skinning::SkinnedMesh, prelude::*, scene::SceneInstanceReady};
use humentity::prelude::*;
use shared::{add_humentity_plugin, add_material, cam_controls};

fn main() {
    let mut app = App::new();
    add_humentity_plugin(&mut app);

    app.add_plugins(DefaultPlugins)
        .add_systems(Startup, setup_env)
        .add_systems(
            Update,
            (
                cam_controls,
                add_material,
                setup_graph_on_new_human,
                start_graph,
            )
                .run_if(in_state(HumentityLoadState::Ready)),
        )
        .add_systems(Startup, setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_human)
        .run();
}

#[derive(Component)]
struct TestAnimation(Handle<AnimationGraph>, AnimationNodeIndex);

fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // Spawn idle animation straight from glb
    let (graph, index) = AnimationGraph::from_clip(
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("animation/idle.glb")),
    );
    let graph_handle = graphs.add(graph);
    let animation = TestAnimation(graph_handle, index);
    commands
        .spawn((
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("animation/idle.glb"))),
            animation,
        ))
        .observe(start_animation_clip_on_imported_glb);

    // circular base
    let mesh = meshes.add(Circle::new(4.0));
    let material = materials.add(Color::WHITE);

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material.clone()),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
    ));

    // A light:
    commands.spawn((
        PointLight {
            intensity: 15_000_0.0,
            radius: 20.,
            range: 20.,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.0, 1.0, -3.0),
    ));

    // A camera:
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 3.0, -4.0).looking_at(Vec3::Y * 0.7, Vec3::Y),
    ));
}

fn setup_prefabs(mut commands: Commands, morphs: Res<MakeHumanMorphs>) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("age", 0.);

    let baby = CharacterShapeArchetype::new(
        "baby",
        morphs.compute_target_weights(&morph_targets),
    );

    let mut prefabs = AHashMap::default();
    prefabs.insert(
        "ExampleHumanPrefab",
        CharacterArchetypePrefab::new(
            vec![baby],
            CharacterAnimationArchetype::new(
                RigType::Default,
                ["assets/animation/idle.glb"],
            ),
        ),
    );

    commands.insert_resource(CharacterArchetypePrefabs::new(prefabs));
}

fn start_animation_clip_on_imported_glb(
    _: On<SceneInstanceReady>,
    mut players: Query<(Entity, &mut AnimationPlayer), Added<AnimationPlayer>>,
    animations: Query<(Entity, &TestAnimation)>,
    children: Query<&Children>,
    mut commands: Commands,
) {
    if players.count() == 0 {
        return;
    }
    for (e, anim) in animations.iter() {
        let (e, _) = children
            .iter_descendants(e)
            .map(|e| players.get(e))
            .filter_map(|r| r.ok())
            .last()
            .unwrap();
        let (e, mut p) = players.get_mut(e).unwrap();
        p.play(anim.1).repeat();
        commands
            .entity(e)
            .insert(AnimationGraphHandle(anim.0.clone()));
    }
}

fn add_human(mut commands: Commands) {
    let mut morphs = MorphTargets::default();
    morphs.insert("baby", 1.);
    commands.spawn((
        Transform::from_translation(Vec3::new(1., 0., 1.)),
        CharacterShapeConfig::new("ExampleHumanPrefab", morphs),
        children![(
            CharacterPart::BodyMesh("basemesh"),
        )],
    ));
}

fn setup_graph_on_new_human(
    player: Query<Entity, With<AnimationPlayer>>,
    humans: Query<Entity, (With<CharacterShapeConfig>, Added<SkinnedMesh>)>,
    children: Query<&Children>,
    animations: Res<CharacterAnimationClips>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut commands: Commands,
) {
    let Ok(human) = humans.single() else { return };
    // included clip is authored on the default rig
    let animations = &animations[&RigType::Default];
    let clip = &animations["Idle-loop"];
    let (graph, index) = AnimationGraph::from_clip(clip.clone());
    let graph_handle = graphs.add(graph.clone());
    for child in children.iter_descendants(human) {
        if let Ok(player) = player.get(child) {
            commands.entity(player).insert((
                TestAnimation(graph_handle.clone(), index),
                AnimationGraphHandle(graph_handle.clone()),
            ));
        }
    }
}

fn start_graph(
    mut players: Query<(&mut AnimationPlayer, &TestAnimation), Added<AnimationGraphHandle>>,
) {
    for (mut p, anim) in players.iter_mut() {
        p.play(anim.1).repeat();
    }
}
