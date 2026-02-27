mod shared;
use std::f32::consts::PI;

use bevy::{mesh::skinning::SkinnedMesh, prelude::*};
use bevy_mod_physx::{
    physx_sys::PxSolverType,
    prelude::{self as bpx, *},
};
use humentity::prelude::*;
use shared::{add_humentity_plugin, add_material, cam_controls, setup_env};

fn main() {
    let mut app = App::new();
    add_humentity_plugin(&mut app);

    let mut settings = DebugRenderSettings::enable();
    settings.joint_local_frames = 0.05;

    app.add_plugins((
        DefaultPlugins,
        PhysicsPlugins.set(
            PhysicsCore {
                //scene: bpx::SceneDescriptor {
                //    solver_type: PxSolverType::Tgs,
                //    ..default()
                //},
                ..default()
            }
            .with_pvd(),
        ),
    ))
    .insert_resource(settings)
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
        ),
    )
    .run();
}

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    mut hitbox: Single<&mut CharacterColliders<HitboxCollider>>,
    mut ragdoll: Single<&mut CharacterColliders<RagdollCollider>>,
) {
    if input.just_pressed(KeyCode::Space) {
        // Toggle between ragdoll active and hitbox active
        let ragdoll_active = !ragdoll.bones_subset.as_ref().map_or(true, |v| v.is_empty());

        if ragdoll_active {
            // Switch to hitbox: ragdoll gets empty, hitbox gets all bones
            ragdoll.bones_subset = Some(vec![]);
            hitbox.bones_subset = None;
        } else {
            // Switch to ragdoll: hitbox gets empty, ragdoll gets all bones
            hitbox.bones_subset = Some(vec![]);
            ragdoll.bones_subset = None;
        }
    }
}

fn floor(
    mut commands: Commands,
    mut geometries: ResMut<Assets<Geometry>>,
    mut materials: ResMut<Assets<bpx::Material>>,
    mut physics: ResMut<Physics>,
) {
    commands.spawn((
        Shape {
            geometry: geometries.add(Plane3d::default()),
            material: materials.add(bpx::Material::new(&mut physics, 0.5, 0.5, 0.5)),
            ..Default::default()
        },
        RigidBody::Static,
        Transform::IDENTITY,
    ));
}

fn add_human(mut commands: Commands) {
    // Different filter layers so hitbox and ragdoll don't collide with each other
    // Using PhysX filter: group in word0, mask in word1
    // Hitbox: group=1, mask=1 (only collides with hitbox)
    // Ragdoll: group=2, mask=2 (only collides with ragdoll)
    let hitbox_filter = ShapeFilterData {
        simulation_filter_data: [1, 1, 0, 0],
        ..default()
    };
    let ragdoll_filter = ShapeFilterData {
        simulation_filter_data: [2, 2, 0, 0],
        ..default()
    };

    commands.spawn((
        Transform::from_rotation(Quat::from_rotation_y(PI / 4.)),
        CharacterShapeConfig::default(),
        // Start with hitbox colliders (all bones), ragdoll has no bones
        CharacterColliders::<HitboxCollider>::new(hitbox_filter, None),
        CharacterColliders::<RagdollCollider>::new(ragdoll_filter, Some(vec![])),
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
    anim.0.play(idx).repeat();
}
