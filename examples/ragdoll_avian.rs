mod shared;

use std::f32::consts::PI;

use avian3d::prelude::*;
use bevy::{
    diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use humentity::prelude::*;
use shared::{setup_app, CharacterPart};

fn main() {
    let mut app = setup_app();

    app.insert_resource(SubstepCount(3))
        .add_plugins((
        PhysicsPlugins::default(),
        PhysicsDebugPlugin,
        FrameTimeDiagnosticsPlugin::default(),
        LogDiagnosticsPlugin::default(),
    ))
    .add_systems(Startup, floor)
    .add_observer(add_human)
    .add_systems(
        Update,
        (
            toggle,
            setup_graph,
            start_clip,
        ),
    )
    .run();
}

fn toggle(
    input: Res<ButtonInput<KeyCode>>,
    related: Single<&RelatedEntities>,
    mut ragdoll: Single<&mut CharacterRagdoll>,
    mut colliders: Single<&mut CharacterColliders>,
    human: Single<(&CharacterShape, &SkinnedMesh)>,
    inv_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut bones: Query<&mut Transform, With<SkeletalBone>>,
    mut players: Query<&mut AnimationPlayer>,
    controllers: Query<&AnimationController>,
) {
    if input.just_pressed(KeyCode::Space) {
        if **ragdoll == CharacterRagdoll::Full {
            **ragdoll = CharacterRagdoll::None;
            colliders.bones_subset = None;

            let (_, skm) = *human;
            if let Some(inv_bindposes) = inv_bindposes.get(&skm.inverse_bindposes)
                && let Ok(mut pelvis) = bones.get_mut(skm.joints[0])
            {
                pelvis.translation = Transform::from_matrix(inv_bindposes[0].inverse()).translation;
            }
        } else {
            if let Ok(mut player) = players.get_mut(related.rig)
                && let Ok(controller) = controllers.get(related.rig)
            {
                player.stop(controller.0);
            }

            **ragdoll = CharacterRagdoll::Full;
            colliders.bones_subset = Some(vec![]);
        }
    }
}

fn floor(
    mut commands: Commands,
) {
    commands.spawn((
        Collider::cuboid(100.0, 0.1, 100.0),
        Friction::new(0.5),
        Restitution::new(0.5),
        RigidBody::Static,
        Transform::IDENTITY,
    ));
}

fn add_human(
    _trigger: On<MorphsReady>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
) {

    let template_handle = template_assets.add(CharacterTemplate::new(
        [],
        RigType::Default,
    ));

    let basemesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    let texture = asset_server.load::<Image>("skin_textures/albedo/young_caucasian_female.png");
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(texture),
        ..default()
    });

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: basemesh.clone(),
        template_handle: template_handle.clone(),
    });

    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(RetargetedAnimations { _clips: clips });

    commands.spawn((
        Transform::from_rotation(Quat::from_rotation_y(PI / 4.)),
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(template_handle, MorphTargets::default()))),
        CharacterRagdoll::None,
        CharacterColliders::new(None),
        children![(
            CharacterPart(basemesh),
            MeshMaterial3d(mat),
        )]
    ));
}

#[derive(Resource)]
struct RetargetedAnimations {
    _clips: Handle<RetargetedAnimationAsset>,
}

#[derive(Component)]
struct AnimationController(AnimationNodeIndex);

fn setup_graph(
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    retargeted_clips: Res<Assets<RetargetedAnimationAsset>>,
    mut character_player: Query<(Entity, &mut AnimationPlayer), Without<AnimationGraphHandle>>,
) {
    let Ok((entity, _player)) = character_player.single_mut() else { return };
    let Some((_id, clips_map)) = retargeted_clips.iter().next() else { return };
    let clip_handle = clips_map.clips.get("Idle-loop").unwrap();
    let (graph, index) = AnimationGraph::from_clip(clip_handle.clone());
    commands.entity(entity).insert((
        AnimationController(index),
        AnimationGraphHandle(graphs.add(graph)),
    ));
}

fn start_clip(
    ragdoll: Single<&CharacterRagdoll>,
    mut anim: Single<(&mut AnimationPlayer, &AnimationController)>,
) {
    if **ragdoll == CharacterRagdoll::Full {
        return;
    }
    let index = anim.1.0;
    anim.0.play(index).repeat();
}