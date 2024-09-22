mod basemesh;
mod morphs;
mod rigs;
mod global_config;
mod assets;
mod mesh_ops;

use std::collections::HashMap;
use bevy::{prelude::*, render::mesh::skinning::SkinnedMeshInverseBindposes};
use bevy_obj::ObjPlugin;
use basemesh::{
    fix_helper_mesh,
    create_body_mesh,
    create_body_vertex_map,
};
use assets::{
    HumanAssetRegistry,
    fix_asset_meshes,
    generate_asset_vertex_maps,
};
use rigs::{
    RigData,
    bone_debug_draw,
    apply_rig,
};
use morphs::{
    bake_morphs_to_base_mesh,
    bake_morphs_to_asset_mesh,
    MorphTargets,
};

pub(crate) use mesh_ops::{
    get_vertex_positions,
    get_vertex_normals,
    get_uv_coords,
    generate_vertex_map,
    generate_inverse_vertex_map,
    parse_obj_vertices,
    fix_mesh_scale,
};
pub(crate) use basemesh::{
    BaseMesh,
    VertexGroups,
    BODY_SCALE,
    BODY_VERTICES,
};
pub(crate) use assets::HumanMeshAsset;

pub use rigs::RigType;
pub use global_config::HumentityGlobalConfig;

pub mod prelude {
    pub use crate::{
        Humentity,
        HumentityGlobalConfig,
        HumanConfig,
        SpawnTransform,
        RigType,
    };
}

/*----------+
 |  Plugin  |
 +----------*/
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
        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin{ compute_smooth_normals: true });
        }
        app.insert_state(HumentityState::Idle);
        app.init_resource::<MorphTargets>();
        app.init_resource::<HumanAssetRegistry>();
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
            generate_asset_vertex_maps,
        ).run_if(in_state(HumentityState::LoadingAssetVertexMaps)));
        app.add_systems(Update, (
            fix_asset_meshes,
        ).run_if(in_state(HumentityState::FixingAssetMeshes)));
        app.add_systems(Update, (
            on_human_added,
        ).run_if(in_state(HumentityState::Ready)));
        if self.debug {
            app.add_systems(Update, bone_debug_draw);
        }
    }
}

/*----------+
 |  States  |
 +----------*/
#[derive(States, PartialEq, Eq, Hash, Debug, Clone)]
pub enum HumentityState {
    Idle,
    FixingHelperMesh,
    LoadingBodyMesh,
    LoadingBodyVertexMap,
    FixingAssetMeshes,
    LoadingAssetVertexMaps,
    Ready
}

/*--------------+
 |  Components  |
 +--------------*/
#[derive(Component)]
pub struct SpawnTransform(pub Transform);

#[derive(Component)]
pub struct HumanConfig {
    // Could be f16 (unstable type warning)
    pub morph_targets: HashMap<String, f32>,
    pub rig: RigType,
    pub skin_albedo: String,
    pub body_parts: Vec<String>,
    //pub equipment: Vec<String>,
}

/*-----------+
 |  Systems  |
 +-----------*/
fn on_human_added(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inv_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    registry: Res<HumanAssetRegistry>,
    base_mesh: Res<BaseMesh>,
    targets: Res<MorphTargets>,
    asset_server: Res<AssetServer>,
    rigs: Res<RigData>,
    vg: Res<VertexGroups>,
    new_humans: Query<(Entity, &HumanConfig, &SpawnTransform), Added<HumanConfig>>,
) {
    // TODO wrap all these args up in a single struct to pass to every function
    new_humans.iter().for_each(|(human, config, spawn_transform)| {
        // Body Material
        let albedo = asset_server.load("skin_textures/albedo/".to_string() + &config.skin_albedo);
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(albedo),
            ..default()
        });

        // Body Mesh
        let (mesh, helpers) = bake_morphs_to_base_mesh(
            &config.morph_targets, 
            &targets,
            &mut meshes,
            &base_mesh
        );
        let (mesh_handle, skinned_mesh) = apply_rig(
            &human, config.rig, mesh, &rigs, &base_mesh.body_vertex_map,
            &mut inv_bindposes, &mut commands,
            &mut meshes, &vg, &helpers, spawn_transform.0);

        // Spawn avatar as separate entity
        commands.spawn((
            skinned_mesh,
            PbrBundle {
                mesh: mesh_handle,
                material: material.clone(),
                ..default()
            },
        ));
        commands.entity(human).remove::<SpawnTransform>();

        // Body Parts
        for bp in config.body_parts.iter() {
            let bp_asset = registry.body_parts.get(bp).unwrap();
            let mesh = bake_morphs_to_asset_mesh(
                &config.morph_targets,  &targets, &mut meshes,
                &bp_asset, &helpers);
            let mesh_handle = meshes.add(mesh);
            //let (mesh_handle, skinned_mesh) = apply_rig(
            //    &human, config.rig, mesh, &rigs, &base_mesh.body_vertex_map,
            //    &mut inv_bindposes, &mut commands,
            //    &mut meshes, &vg, &helpers, spawn_transform.0);
            commands.spawn(
                //skinned_mesh,
                PbrBundle {
                    mesh: mesh_handle,
                    ..default()
                },
            );
        }
    })
}

