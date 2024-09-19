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
        LoadHumanParams,
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

#[derive(Event)]
pub struct LoadHumanParams {
    pub shapekeys: HashMap<String, f32>,
    pub skin_albedo: String,
    pub rig: RigType,
    pub transform: Transform,
}

pub(crate) fn load_human_entity(
    trigger: Trigger<LoadHumanParams>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inv_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    targets: Query<&MorphTarget>,
    asset_server: Res<AssetServer>,
    base_mesh: Res<BaseMesh>,
    rigs: Res<RigData>,
    vg: Res<VertexGroups>,
) {
    let albedo = asset_server.load("skin_textures/albedo/".to_string() + &trigger.event().skin_albedo);
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(albedo),
        ..default()
    });
    let human = trigger.entity();
    let (helpers, mesh) = bake_morphs_to_mesh(
        &trigger.event().shapekeys,
        &base_mesh,
        &targets,
        &mut meshes
    );
    let (mesh_handle, skinned_mesh) = apply_rig(
        &human,
        trigger.event().rig,
        mesh,
        &base_mesh,
        &rigs,
        &mut inv_bindposes,
        &mut commands,
        &mut meshes,
        vg,
        helpers,
    );
    commands.entity(human).insert(skinned_mesh);
    commands.entity(human).insert(PbrBundle {
        mesh: mesh_handle,
        transform: trigger.event().transform,
        material: material,
        ..default()
    });
}

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
        app.observe(load_human_entity);
        if self.debug {
            app.add_systems(Update, bone_debug_draw);
        }
    }
}