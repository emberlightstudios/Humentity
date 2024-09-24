mod basemesh;
mod morphs;
mod rigs;
mod global_config;
mod assets;
mod mesh_ops;

use std::collections::{ HashMap, HashSet };
use bevy::{prelude::*, render::mesh::skinning::SkinnedMeshInverseBindposes};
use bevy_obj::ObjPlugin;
use basemesh::{
    create_body_mesh,
    create_body_vertex_map,
};
use assets::{
    HumanAssetRegistry,
    generate_asset_vertex_maps,
    delete_mesh_verts,
};
use rigs::{
    RigData,
    bone_debug_draw,
    build_rig,
    set_basemesh_rig_arrays,
    set_asset_rig_arrays,
};
use morphs::{
    adjust_helpers_to_morphs,
    bake_asset_morphs,
    bake_body_morphs,
    MorphTargets,
};

pub(crate) use mesh_ops::{
    get_vertex_positions,
    get_vertex_normals,
    get_uv_coords,
    generate_vertex_map,
    generate_inverse_vertex_map,
    parse_obj_vertices,
};
pub(crate) use basemesh::{
    BaseMesh,
    VertexGroups,
    BODY_SCALE,
};
pub(crate) use assets::{
    HelperMap,
    HumanMeshAsset,
};

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
            create_body_mesh,
        ).run_if(in_state(HumentityState::LoadingBodyMesh)));
        app.add_systems(Update, (
            create_body_vertex_map,
        ).run_if(in_state(HumentityState::LoadingBodyVertexMap)));
        app.add_systems(Update, (
            generate_asset_vertex_maps,
        ).run_if(in_state(HumentityState::LoadingAssetVertexMaps)));
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
    LoadingBodyMesh,
    LoadingBodyVertexMap,
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
    pub equipment: Vec<String>,
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
    // TODO Can we wrap all these args up in a single struct to pass to every function?
    // tried but couldn't figure out lifetimes

    new_humans.iter().for_each(|(human, config, spawn_transform)| {
        // Body Material
        let albedo = asset_server.load("skin_textures/albedo/".to_string() + &config.skin_albedo);
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(albedo),
            ..default()
        });

        let helpers = adjust_helpers_to_morphs(
            &config.morph_targets,
            &targets,
            &base_mesh
        );
        let (skinned_mesh, sorted_bones) = build_rig(
            &human,
            config.rig,
            &rigs,
            &mut inv_bindposes,
            &mut commands,
            &vg,
            &helpers,
            spawn_transform.0,
        );

        let mut delete_verts = HashSet::<u16>::new();

        // Body Parts
        for bp in config.body_parts.iter() {
            let err_msg = format!("FAILED TO FIND BODY PART {}", bp);
            let asset = registry.body_parts.get(bp).expect(&err_msg);
            delete_verts.extend(&asset.delete_verts);
            let mesh = bake_asset_morphs(
                &config.morph_targets, 
                &targets,
                &mut meshes,
                &helpers,
                &asset,
            );
            let mesh_handle = set_asset_rig_arrays(
                config.rig,
                mesh,
                &rigs,
                &asset.vertex_map,
                &mut meshes,
                &asset.helper_maps,
                &sorted_bones,
            );
            commands.spawn((
                skinned_mesh.clone(),
                PbrBundle {
                    mesh: mesh_handle,
                    ..default()
                },
            ));
        }

        // Equipment
        for eq in config.equipment.iter() {
            let err_msg = format!("FAILED TO FIND EQUIPMENT {}", eq);
            let asset = registry.equipment.get(eq).expect(&err_msg);
            delete_verts.extend(&asset.delete_verts);
            let mesh = bake_asset_morphs(
                &config.morph_targets, 
                &targets,
                &mut meshes,
                &helpers,
                &asset,
            );
            let mesh_handle = set_asset_rig_arrays(
                config.rig,
                mesh,
                &rigs,
                &asset.vertex_map,
                &mut meshes,
                &asset.helper_maps,
                &sorted_bones,
            );
            commands.spawn((
                skinned_mesh.clone(),
                PbrBundle {
                    mesh: mesh_handle,
                    ..default()
                },
            ));
        }

        // Body Mesh
        // Delete verts
        let mesh = delete_mesh_verts(&mut meshes, &base_mesh, delete_verts);
        let vertices = &get_vertex_positions(&mesh);
        let new_vtx_map = generate_vertex_map(&base_mesh.vertices, vertices);

        // Apply Morphs
        let mesh = bake_body_morphs(&mesh,&new_vtx_map,&helpers);
        // Apply Rig
        let mesh_handle = set_basemesh_rig_arrays(
            config.rig,
            mesh,
            &rigs,
            &new_vtx_map,
            &mut meshes,
            &sorted_bones,
        );

        // Spawn avatar as separate entity
        commands.spawn((
            skinned_mesh.clone(),
            PbrBundle {
                mesh: mesh_handle,
                material: material.clone(),
                ..default()
            },
        ));
        commands.entity(human).remove::<SpawnTransform>();
    })
}

