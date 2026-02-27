mod shared;
use avian3d::prelude::*;
use bevy::{mesh::skinning::SkinnedMesh, prelude::*};
use humentity::prelude::*;
use shared::{add_humentity_plugin, add_material, cam_controls, setup_env};

fn main() {
    let mut app = App::new();
    add_humentity_plugin(&mut app);

    app.add_plugins((
        DefaultPlugins,
        PhysicsPlugins::default(),
        PhysicsDebugPlugin::default(),
    ))
    .add_systems(Startup, (setup_env, floor))
    .add_systems(Startup, setup_prefabs)
    .add_systems(OnEnter(HumentityLoadState::Ready), add_human)
    .add_systems(
        Update,
        (
            cam_controls,
            add_material,
            toggle,
            setup_graph,
            start_clip,
            oscillate,
        ),
    )
    .run();
}

fn oscillate(mut transforms: Query<&mut Transform, With<CharacterColliders>>, time: Res<Time>) {
    return;
    for mut transform in &mut transforms {
        transform.translation.x = time.elapsed_secs().sin() * 0.5;
    }
}

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    mut ragdoll: Single<&mut CharacterRagdoll, With<CharacterColliders>>,
) {
    if input.just_pressed(KeyCode::Space) {
        if **ragdoll == CharacterRagdoll::None {
            **ragdoll = CharacterRagdoll::Full;
        } else {
            **ragdoll = CharacterRagdoll::None;
        };
    }
}

fn floor(mut commands: Commands) {
    commands.spawn((
        Collider::cuboid(100.0, 0.1, 100.0),
        Friction::new(0.5),
        Restitution::new(0.5),
        RigidBody::Static,
        Transform::IDENTITY,
    ));
}

fn add_human(mut commands: Commands) {
    commands.spawn((
        Transform::IDENTITY,
        CharacterShapeConfig::default(),
        CharacterColliders::new(true),
        CharacterRagdoll::None,
        //children![(CharacterPart::BodyMesh("basemesh"))],
    ));
}

fn setup_prefabs(mut commands: Commands) {
    // No shape morphs, just the basemesh
    // Just for the examples.
    commands.insert_resource(CharacterArchetypePrefabs::new([(
        "",
        CharacterArchetypePrefab::new(
            [],
            CharacterAnimationArchetype::new(RigType::Default, ["assets/animation/idle.glb"]),
        ),
    )]));
}

#[derive(Component)]
struct AnimationController(AnimationNodeIndex);

fn setup_graph(
    player: Query<Entity, With<AnimationPlayer>>,
    human: Single<&RelatedEntities, Added<SkinnedMesh>>,
    animations: Res<CharacterAnimationClips>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut commands: Commands,
) {
    let Ok(anim_player) = player.get(human.rig) else {
        return;
    };
    let animations = &animations[&RigType::Default];
    let clip = &animations["Idle-loop"];
    let (graph, index) = AnimationGraph::from_clip(clip.clone());
    let graph_handle = graphs.add(graph.clone());
    commands.entity(anim_player).insert((
        AnimationController(index),
        AnimationGraphHandle(graph_handle.clone()),
    ));
}

fn start_clip(mut anim: Single<(&mut AnimationPlayer, &AnimationController)>) {
    let idx = anim.1 .0.clone();
    //anim.0.play(idx);
}
