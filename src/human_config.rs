use bevy::{mesh::skinning::SkinnedMeshInverseBindposes, prelude::*};
use fxhash::{FxHashMap, FxHashSet};
use crate::{assets::{self, HumanAssetTextures}, basemesh::VertexGroups, mesh_ops::get_vertex_positions, morphs, prelude::*, rigs};

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
pub(crate) fn on_human_added(
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
    rigs: Res<RigData>,
    vg: Res<VertexGroups>,
    asset_textures: Res<HumanAssetTextures>,
) {
    if new_humans.count() == 0 { return }
    let transparent_slots = &global_config.transparent_slots;

    new_humans.iter().for_each(|(human, config, transform)| {
        // Body Material
        let albedo = asset_server.load("humentity://skin_textures/albedo/".to_string() + &config.skin_albedo);
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

