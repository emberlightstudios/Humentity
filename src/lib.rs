mod basemesh;
mod morphs;
mod rigs;

use basemesh::{
    fix_helper_mesh,
    create_body_mesh,
    create_body_vertex_map,
};
use bevy::{prelude::*, render::mesh::skinning::SkinnedMeshInverseBindposes};
use bevy_obj::ObjPlugin;
use rigs::{
    RigData,
    bone_debug_draw,
};
use std::collections::HashMap;
pub(crate) use crate::basemesh::{
    BaseMesh,
    VertexGroups,
    BODY_SCALE,
    BODY_VERTICES,
};
use rigs::apply_rig;
pub(crate) use morphs::{
    MorphTarget,
    MorphTargetType,
    bake_morphs_to_mesh,
};
pub use rigs::RigType;

pub mod prelude {
    pub use crate::{
        Humentity,
        HumanConfig,
        SpawnTransform,
        RigType,
    };
}

#[derive(States, PartialEq, Eq, Hash, Debug, Clone)]
pub enum HumentityState {
    Idle,
    FixingHelperMesh,
    LoadingBodyMesh,
    LoadingBodyVertexMap,
    Ready
}

#[derive(Component)]
pub struct SpawnTransform(pub Transform);

#[derive(Component)]
pub struct HumanConfig {
    // Could be f16 (unstable type warning)
    pub morph_targets: HashMap<String, f32>,
    pub rig: RigType,
    pub skin_albedo: String,
}

fn on_human_added(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inv_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    base_mesh: Res<BaseMesh>,
    targets: Query<&MorphTarget>,
    asset_server: Res<AssetServer>,
    rigs: Res<RigData>,
    vg: Res<VertexGroups>,
    new_humans: Query<(Entity, &HumanConfig, &SpawnTransform), Added<HumanConfig>>,
) {
    new_humans.iter().for_each(|(human, config, spawn_transform)| {
        let albedo = asset_server.load("skin_textures/albedo/".to_string() + &config.skin_albedo);
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(albedo),
            ..default()
        });
        let (helpers, mesh) = bake_morphs_to_mesh(
            &config.morph_targets,
            &base_mesh,
            &targets,
            &mut meshes
        );
        let (mesh_handle, skinned_mesh) = apply_rig(
            &human,
            config.rig,
            mesh,
            &base_mesh,
            &rigs,
            &mut inv_bindposes,
            &mut commands,
            &mut meshes,
            &vg,
            helpers,
            spawn_transform.0,
        );
        // Spawn avatar as separate entity
        commands.spawn((
            skinned_mesh,
            PbrBundle {
                mesh: mesh_handle,
                material: material,
                ..default()
            },
        ));
        commands.entity(human).remove::<SpawnTransform>();
    })
}

// TODO
// transform needs to be controlled by skeleton root
// should convert to bundle instead of listening for triggers
// mhclo
// animation
// presets
// 

pub struct Humentity{
    debug: bool,
}

impl Default for Humentity {
    fn default() -> Self {
        Humentity {
            debug: true,
        }
    }
}

impl Plugin for Humentity {
    fn build(&self, app: &mut App) {
        app.add_plugins(ObjPlugin{ compute_smooth_normals: true });
        app.insert_state(HumentityState::Idle);
        app.init_resource::<BaseMesh>();
        app.init_resource::<RigData>();
        app.add_systems(Update, (
            fix_helper_mesh,
        ).run_if(in_state(HumentityState::FixingHelperMesh)));
        app.add_systems(Update, (
            create_body_mesh,
        ).run_if(in_state(HumentityState::LoadingBodyMesh)));
        app.add_systems(Update, (
            create_body_vertex_map,
        ).run_if(in_state(HumentityState::LoadingBodyVertexMap)));
        app.add_systems(Update, (
            on_human_added,
        ).run_if(in_state(HumentityState::Ready)));
        if self.debug {
            app.add_systems(Update, bone_debug_draw);
        }
    }
}