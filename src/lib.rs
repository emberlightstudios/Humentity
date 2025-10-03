mod basemesh;
mod morphs;
mod rigs;
mod global_config;
mod assets;
mod animation;
mod mesh_ops;

use bevy::{prelude::*, mesh::skinning::SkinnedMeshInverseBindposes};
use bevy::ecs::schedule::common_conditions::run_once;
use bevy_obj::ObjPlugin;
use assets::{HumanAssetRegistry, HumanAssetTextures};
use fxhash::{FxHashMap, FxHashSet};
use morphs::{MorphTargets};

pub(crate) use basemesh::{BaseMesh, VertexGroups, BODY_SCALE};
pub(crate) use assets::{HelperMap, HumanMeshAsset};

pub use rigs::RigType;
pub use global_config::HumentityGlobalConfig;
pub use animation::{AnimationLibrarySet, AnimationLibrarySettings};

use crate::basemesh::HelperMeshHandle;

pub mod prelude {
    pub use crate::{
        Humentity,
        HumentityLoading,
        HumentityGlobalConfig,
        HumanConfig,
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
        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin);
        }
        app.insert_resource(HumentityLoading);
        app.init_resource::<BaseMesh>();
        app.init_resource::<HumanAssetRegistry>();
        app.init_resource::<MorphTargets>();
        app.init_resource::<rigs::RigData>();
        app.init_resource::<AnimationLibrarySet>();

        app.add_systems(Update, (
            (
                basemesh::create_body_mesh
                    .run_if(resource_exists::<HelperMeshHandle>),
                assets::generate_asset_vertex_maps
                    .run_if(run_once),
                animation::load_animations
                    .run_if(run_once),
            ).run_if(resource_exists::<HumentityLoading>),

            on_human_added,
        ));

        if self.debug {
            app.add_systems(Update, rigs::bone_debug_draw);
        }
    }
}

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource)]
pub struct HumentityLoading;

/*--------------+
 |  Components  |
 +--------------*/

#[derive(Component)]
pub struct HumanConfig {
    pub morph_targets: FxHashMap<String, f32>,
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
            morph_targets: FxHashMap::<String, f32>::default(),
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
fn on_human_added(
    new_humans: Query<(Entity, &HumanConfig, &Transform), Added<HumanConfig>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inv_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    global_config: Res<HumentityGlobalConfig>,
    registry: Res<HumanAssetRegistry>,
    base_mesh: Res<BaseMesh>,
    targets: Res<MorphTargets>,
    asset_server: Res<AssetServer>,
    rigs: Res<rigs::RigData>,
    vg: Res<VertexGroups>,
    asset_textures: Res<HumanAssetTextures>,
) {
    let path = &global_config.core_assets_path;
    let transparent_slots = &global_config.transparent_slots;

    new_humans.iter().for_each(|(human, config, transform)| {
        // Body Material
        let albedo = asset_server.load(path.join("skin_textures/albedo/".to_string() + &config.skin_albedo));
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(albedo),
            perceptual_roughness: 1.,
            ..default()
        });

        let helpers = morphs::adjust_helpers_to_morphs(
            &config.morph_targets,
            &targets,
            &base_mesh
        );
        let (skinned_mesh, sorted_bones) = rigs::build_rig(
            &human,
            config.rig,
            &rigs,
            &mut inv_bindposes,
            &mut commands,
            &vg,
            &helpers,
            transform,
        );

        let mut delete_verts = FxHashSet::<u16>::default();

        // Body Parts
        for bp in config.body_parts.iter() {
            let err_msg = format!("FAILED TO FIND BODY PART {}", bp);
            let asset = registry.body_parts.get(bp).expect(&err_msg);
            //delete_verts.extend(&asset.delete_verts);
            let mesh = morphs::bake_asset_morphs(
                &config.morph_targets, 
                &targets,
                &mut meshes,
                &helpers,
                &asset,
            );
            let mesh_handle = rigs::set_asset_rig_arrays(
                config.rig,
                mesh,
                &rigs,
                &asset.mhid_lookup,
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
            let material = materials.add(material);

            commands.entity(human).insert(
                children![
                    (
                        skinned_mesh.clone(),
                        Mesh3d(mesh_handle),
                        MeshMaterial3d(material)
                    )
                ]
            );
        }

        // Equipment
        for eq in config.equipment.iter() {
            let err_msg = format!("FAILED TO FIND EQUIPMENT {}", eq);
            let asset = registry.equipment.get(eq).expect(&err_msg);
            delete_verts.extend(&asset.delete_verts);
            let mesh = morphs::bake_asset_morphs(
                &config.morph_targets, 
                &targets,
                &mut meshes,
                &helpers,
                &asset,
            );
            let mesh_handle = rigs::set_asset_rig_arrays(
                config.rig,
                mesh,
                &rigs,
                &asset.mhid_lookup,
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

            commands.entity(human).insert(
                children! [
                    (
                        skinned_mesh.clone(),
                        Mesh3d(mesh_handle),
                        MeshMaterial3d(materials.add(material)),
                    )
                ]
            );
        }

        // Body Mesh
        // Delete verts
        let mesh = assets::delete_mesh_verts(&mut meshes, &base_mesh, delete_verts);
        // Apply Morphs
        let mesh = morphs::bake_body_morphs(&mesh, &base_mesh.mhid_lookup, &helpers);
        // Apply Rig
        let mesh_handle = rigs::set_basemesh_rig_arrays(
            config.rig,
            mesh,
            &rigs,
            &base_mesh.mhid_lookup,
            &mut meshes,
            &sorted_bones,
        );

        
        // Spawn avatar as separate entity
        commands.entity(human).insert(
            children! [
                (
                    skinned_mesh.clone(),
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material),
                )
            ]
        );
        commands.entity(human).insert(children![(AnimationPlayer::default())]);
    })

}

