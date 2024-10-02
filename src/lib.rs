mod basemesh;
mod morphs;
mod rigs;
mod global_config;
mod assets;
mod animation;
mod mesh_ops;

use bevy::{
    prelude::*,
    render::mesh::skinning::SkinnedMeshInverseBindposes
};
use std::collections::{ HashMap, HashSet };
use bevy_obj::ObjPlugin;
use basemesh::{
    create_body_mesh,
    create_body_vertex_map,
};
use assets::{
    HumanAssetRegistry,
    HumanAssetTextures,
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
use animation::load_animations;

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
pub use animation::{
    AnimationLibrarySet,
    AnimationLibrarySettings,
};

pub mod prelude {
    pub use crate::{
        Humentity,
        HumentityGlobalConfig,
        HumentityState,
        HumanConfig,
        SpawnTransform,
        RigType,
        AnimationLibrarySet,
        AnimationLibrarySettings,
    };
}

/*----------+
 |  Plugin  |
 +----------*/
pub struct Humentity{
    pub debug: bool,
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
        let mut loading_state = HashMap::<LoadingPhase, bool>::new();
        loading_state.insert(LoadingPhase::CreateBodyMesh, false);
        loading_state.insert(LoadingPhase::GenerateBodyVertexMap, false);
        loading_state.insert(LoadingPhase::GenerateAssetVertexMap, false);
        loading_state.insert(LoadingPhase::SetUpAnimationLibraries, false);

        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin{ compute_smooth_normals: true });
        }
        app.insert_state(HumentityState::Loading);
        app.insert_resource(LoadingState(loading_state));
        app.init_resource::<MorphTargets>();
        app.init_resource::<HumanAssetRegistry>();
        app.init_resource::<BaseMesh>();
        app.init_resource::<RigData>();
        app.init_resource::<AnimationLibrarySet>();
        app.add_systems(Update, ((
            loading_state_checker,
            create_body_mesh,
            create_body_vertex_map,
            generate_asset_vertex_maps,
            load_animations,
        )).run_if(in_state(HumentityState::Loading)));
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
    Loading,
    Ready
}

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum LoadingPhase {
    CreateBodyMesh,
    GenerateBodyVertexMap,
    GenerateAssetVertexMap,
    SetUpAnimationLibraries,
}

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource)]
pub(crate) struct LoadingState(HashMap<LoadingPhase, bool>);

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
    pub eye_color: Color,
    pub eyebrow_color: Color,
    pub hair_color: Color,
}

impl Default for HumanConfig {
    fn default() -> Self {
        HumanConfig {
            morph_targets: HashMap::<String, f32>::new(),
            rig: RigType::Mixamo,
            skin_albedo: String::new(),
            body_parts: vec![],
            equipment: vec![],
            eye_color: Color::BLACK,
            eyebrow_color: Color::BLACK,
            hair_color: Color::BLACK,
        }
    }
}

/*-----------+
 |  Systems  |
 +-----------*/
fn loading_state_checker(
    loading_state: Res<LoadingState>,
    mut next: ResMut<NextState<HumentityState>>,
    mut commands: Commands,
) {
    if !loading_state.0.get(&LoadingPhase::CreateBodyMesh).unwrap() { return; }
    if !loading_state.0.get(&LoadingPhase::GenerateBodyVertexMap).unwrap() { return; }
    if !loading_state.0.get(&LoadingPhase::GenerateAssetVertexMap).unwrap() { return; }
    if !loading_state.0.get(&LoadingPhase::SetUpAnimationLibraries).unwrap() { return; }
    commands.remove_resource::<LoadingState>();
    next.set(HumentityState::Ready);
}

fn on_human_added(
    new_humans: Query<(Entity, &HumanConfig, &SpawnTransform), Added<HumanConfig>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inv_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    global_config: Res<HumentityGlobalConfig>,
    registry: Res<HumanAssetRegistry>,
    base_mesh: Res<BaseMesh>,
    targets: Res<MorphTargets>,
    asset_server: Res<AssetServer>,
    rigs: Res<RigData>,
    vg: Res<VertexGroups>,
    asset_textures: Res<HumanAssetTextures>,
) {
    // TODO Can we wrap all these args up in a single struct to pass to every function?
    // tried but couldn't figure out lifetimes
    let path = global_config.core_assets_path.clone();
    let transparent_slots = global_config.transparent_slots.clone();

    new_humans.iter().for_each(|(human, config, spawn_transform)| {
        // Body Material
        let albedo = asset_server.load(path.join("skin_textures/albedo/".to_string() + &config.skin_albedo));
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
            let mut material = StandardMaterial::default();
            if let Some(albedos) = asset_textures.albedo_maps.get(&asset.name) {
                if albedos.len() > 0 { material.base_color_texture = Some(albedos[0].clone()); }
            }
            if let Some(normal) = asset_textures.normal_map.get(&asset.name) {
                material.normal_map_texture = Some(normal.clone());
            }
            if let Some(ao) = asset_textures.ao_map.get(&asset.name) {
                material.occlusion_texture = Some(ao.clone());
            }
            for slot in asset.slots.iter() {
                if transparent_slots.contains(slot) {
                    material.alpha_mode = AlphaMode::Blend;
                    material.reflectance = 0.25;
                    if slot.contains("Eyebrow") { material.base_color = config.eyebrow_color; }
                    else if slot.contains("Eye") && !slot.contains("Eyelash") { material.base_color = config.eye_color; }
                    else if slot.contains("Hair") { material.base_color = config.hair_color; }
                }
            }

            commands.spawn((
                skinned_mesh.clone(),
                PbrBundle {
                    mesh: mesh_handle,
                    material: materials.add(material),
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
            let mut material = StandardMaterial::default();
            if let Some(albedos) = asset_textures.albedo_maps.get(&asset.name) {
                if albedos.len() > 0 { material.base_color_texture = Some(albedos[0].clone()); }
            }
            if let Some(normal) = asset_textures.normal_map.get(&asset.name) {
                material.normal_map_texture = Some(normal.clone());
            }
            if let Some(ao) = asset_textures.ao_map.get(&asset.name) {
                material.occlusion_texture = Some(ao.clone());
            }
            commands.spawn((
                skinned_mesh.clone(),
                PbrBundle {
                    mesh: mesh_handle,
                    material: materials.add(material),
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
        commands.entity(human).insert(AnimationPlayer::default());
    })

}

