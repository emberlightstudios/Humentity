
use bevy::{input::mouse::MouseMotion, prelude::*, scene::SceneInstanceReady, window::PresentMode};
use humentity::prelude::*;
use fxhash::FxHashMap;

#[derive(Component)]
struct TestAnimation(Handle<AnimationGraph>, AnimationNodeIndex);


fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    morphs: Res<HumanMorphs>,
) {

    // Spawn idle animation straight from glb
    let (graph, index) = AnimationGraph::from_clip(
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("animation/idle.glb"))
    );
    let graph_handle = graphs.add(graph);
    let animation = TestAnimation(graph_handle, index);
    commands.spawn((
        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("animation/idle.glb"),
        )),
        animation,
    )).observe(animation_clip_on_imported_glb);
    
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
        Transform::from_xyz(0.0, 1.0, 3.0),
    ));

    // A camera:
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 3.0, 4.0).looking_at(Vec3::Y * 0.7, Vec3::Y),
    ));

    let mut morphs = MorphTargetValues::default();
    morphs.insert(Name::new("african-male-baby"), 1.0);

    // Spawn a dynamically generated human
    commands.spawn((
        Name::new("Muh Human"),
        Transform::from_xyz(1.5, 0.0, 0.0),
        InheritedVisibility::VISIBLE,
        HumanShapeConfig {
            prefab_morph_targets: morphs,
            rig_archetype: Name::new("Rig1"),
        },
        children![
            (
                HumanPart::new(HumanPart::ProxyMesh, Name::new("proxy741")),
            )
        ],
    ));
}

fn create_rig_prefabs(
    mut commands: Commands,
) {
    commands.insert_resource(HumanArchetypePrefabs::new(
        [
            (
                Name::new("Rig1"),
                HumanArchetype::new(
                    RigType::Default,
                    MorphTargetValues::default(),
                    ["assets/animation/Idle.glb"]
                )
            )
        ]
    ));
}

fn animation_clip_on_imported_glb(
    _: On<SceneInstanceReady>,
    mut players: Query<(Entity, &mut AnimationPlayer), Added<AnimationPlayer>>,
    animations: Query<(Entity, &TestAnimation)>,
    children: Query<&Children>,
    mut commands: Commands,
) {
    if players.count() == 0 { return }
    for (e, anim) in animations.iter() {
        let (e, _) = children.iter_descendants(e)
            .map(|e| players.get(e))
            .filter_map(|r| r.ok())
            .last()
            .unwrap();
        let (e, mut p) = players.get_mut(e).unwrap();
        p.play(anim.1).repeat();
        commands.entity(e).insert(
            AnimationGraphHandle(anim.0.clone())
        );
    }
}

fn setup_graph_on_new_human(
    rigs: Res<HumanArchetypePrefabs>,
    spawned_rigs: Query<(Entity, &ChildOf), Added<HumanRigScene>>,
    humans: Query<(Entity, &HumanShapeConfig)>,
    children: Query<&Children>,
    player: Query<Entity, With<AnimationPlayer>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut commands: Commands,
) {
    if spawned_rigs.count() == 0 { return; }
    for (entity, child_of) in spawned_rigs.iter() {
        assert!(humans.get(child_of.parent()).is_ok());
        let (_config_entity, config) = humans.get(child_of.parent()).unwrap();
        let clips = &rigs[&config.rig_archetype].animations;
        let clip = &clips[&Name::new("Idle-loop")];
        let (graph, index) = AnimationGraph::from_clip(clip.clone());
        let graph_handle = graphs.add(graph.clone());
        for child in children.iter_descendants(entity) {
            if let Ok(player) = player.get(child) {
                commands.entity(player).insert((
                    TestAnimation(graph_handle.clone(), index),
                    AnimationGraphHandle(graph_handle.clone()),
                ));
            }
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

fn add_material(
    humans: Query<Entity, (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for human in humans.iter() {
        let mat = materials.add(StandardMaterial::from_color(Color::BLACK));
        commands.entity(human).insert(MeshMaterial3d(mat));
    }
}

fn cam_controls(
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    kb_input: Res<ButtonInput<KeyCode>>,
    mut pitch: Local<f32>,
    mut yaw: Local<f32>,
) {
    const MS: f32 = 5e-3;
    const LS: f32 = 5e-3;
    let Ok(transform) = cam.single().cloned() else { return };
    let Ok(mut cam) = cam.single_mut() else { return };
    for ev in mouse_motion.read() {
        *yaw -= ev.delta.x * LS;
        *pitch -= ev.delta.y * LS;
    }
    //cam.rotation = Quat::from_euler(EulerRot::YXZ, *yaw, *pitch, 0.);
    let mut mv = Vec3::ZERO;
    if kb_input.pressed(KeyCode::KeyD) { mv.x += MS }
    if kb_input.pressed(KeyCode::KeyA) { mv.x -= MS }
    if kb_input.pressed(KeyCode::KeyS) { mv.z += MS }
    if kb_input.pressed(KeyCode::KeyW) { mv.z -= MS }
    if kb_input.pressed(KeyCode::KeyQ) { mv.y -= MS }
    if kb_input.pressed(KeyCode::KeyE) { mv.y += MS }
    cam.translation += Transform::from_rotation(transform.rotation) * mv;
}


fn main() {
    App::new()
        .add_plugins(Humentity{
            debug: true,
            config: HumentityPathsConfig::new("../humentity")
        })
        .add_plugins((
            DefaultPlugins
                .set(AssetPlugin {
                    unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
                    ..Default::default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Animations".to_string(),
                        resolution: [800, 600].into(),
                        present_mode: PresentMode::Immediate, // <- disables VSync
                        ..default()
                    }),
                    ..default()
                }),
            //FrameTimeDiagnosticsPlugin::default(),
            //LogDiagnosticsPlugin::default(),
        ))
        .add_systems(Update, (
            (
                create_rig_prefabs,
            ).run_if(resource_removed::<HumentityLoading>),
            (
                setup_env,
            ).run_if(resource_added::<HumanArchetypePrefabs>),
            (
                add_material,
                cam_controls,
                setup_graph_on_new_human,
                start_graph,
            ).run_if(resource_exists::<HumanArchetypePrefabs>),
        ))
        .run();
}

