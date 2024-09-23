use bevy::prelude::*;
use std::{
    collections::HashMap,
    fs::File,
    io::{ BufReader, BufRead },
};
use walkdir::WalkDir;
use crate::{ 
    get_vertex_positions,
    BaseMesh,
    HumentityGlobalConfig,
    BODY_SCALE,
    HumanMeshAsset,
};

pub struct MorphTarget(HashMap<u16, Vec3>);

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource)]
pub struct MorphTargets {
    names: HashMap<String, MorphTarget>,
}

impl FromWorld for MorphTargets {
    fn from_world(world: &mut World) -> Self {
        // Create Morph Target Entities from all the .target files
        let Some(config) = world.get_resource_mut::<HumentityGlobalConfig>() else {
            panic!("No global Humentity config loaded");
        };
        let mut names = HashMap::<String, MorphTarget>::new();
        for target_path in config.target_paths.clone().iter() {
            for entry in WalkDir::new(target_path).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                let mut offsets = HashMap::<u16, Vec3>::new();
                if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("target") {
                    let Some(filename) = path.file_name().unwrap().to_str() else { continue };
                    let Some(stem) = path.file_stem().unwrap().to_str() else { continue };
                    let err_msg = "Couldn't open target file ".to_string() + filename;
                    let file = File::open(path).expect(&err_msg);
                    for line_result in BufReader::new(file).lines() {
                        let Ok(line) = line_result else { break };
                        let mut line_elements = line.split_whitespace();
                        let Some(vert_str) = line_elements.next() else { continue };
                        let Ok(vert) = vert_str.parse::<u16>() else { continue };
                        let coords: Vec<f32> = line_elements
                                              .filter_map(|x| x.parse().ok())
                                              .collect();
                        offsets.insert(vert, Vec3::from_slice(&coords[..]) * BODY_SCALE);
                    }
                    names.insert(stem.to_string(), MorphTarget(offsets.clone()));
                }
                else {
                }
            };
        };
        MorphTargets {
            names: names,
        }
    }
}

/*-------------+
 |  Functions  |
 +-------------*/
pub(crate) fn adjust_helpers_to_morphs(
    shapekeys: &HashMap<String, f32>,
    targets: &Res<MorphTargets>,
    base_mesh: &Res<BaseMesh>,
) -> Vec<Vec3> {
    let mut helpers = base_mesh.vertices.clone();
    for (target_name, &value) in shapekeys.iter() {
        let err_msg = format!("Failed to find morph {}", target_name);
        let target = targets.names.get(target_name).expect(&err_msg);
        for (&vertex, &offset) in target.0.iter() {
            helpers[vertex as usize] += offset * value;
        }
    }
    helpers
}

pub(crate) fn bake_body_morphs(
    meshes: &mut ResMut<Assets<Mesh>>,
    helpers: &Vec<Vec3>,
    base_mesh: &Res<BaseMesh>,
) -> Mesh {
    let mesh = meshes.get(&base_mesh.mesh_handle).unwrap().clone();
    let mut vertices = get_vertex_positions(&mesh);
    for (mh_vert, vtx_list) in base_mesh.vertex_map.iter() {
        for vtx in vtx_list.iter() {
            vertices[*vtx as usize] = helpers[*mh_vert as usize];
        }
    }
    let mut new_mesh = mesh.clone()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    new_mesh.compute_smooth_normals();
    let _ = new_mesh.generate_tangents();
    new_mesh
}


pub(crate) fn bake_asset_morphs(
    shapekeys: &HashMap<String, f32>,
    targets: &Res<MorphTargets>,
    meshes: &mut ResMut<Assets<Mesh>>,
    helpers: &Vec<Vec3>,
    asset: &HumanMeshAsset,
) -> Mesh {
    let mesh = meshes.get(&asset.mesh_handle).unwrap().clone();
    let mut vertices = get_vertex_positions(&mesh);
    for (target_name, &value) in shapekeys.iter() {
        let err_msg = format!("Failed to find morph {}", target_name);
        let target = targets.names.get(target_name).expect(&err_msg);

        for (asset_vert, vtx_list) in asset.vertex_map.iter() {
            let helper_map = &asset.helper_maps[*asset_vert as usize];
            for vtx in vtx_list.iter() {
                if helper_map.single_vertex.is_some() {
                    let offset = *target.0.get(asset_vert).unwrap();
                    vertices[*vtx as usize] += offset * value;
                } else { // Triangulation
                    let triangle = helper_map.triangle.as_ref().unwrap();
                    let mut position = Vec3::ZERO;
                    for i in 0..3 {
                        let mh_vert = triangle.helper_verts[i];
                        let wt = triangle.helper_weights[i];
                        position += *helpers.get(mh_vert as usize).unwrap() * wt;
                    }
                    position += asset.get_scale(helpers) * triangle.helper_offset;
                    let offset = position - vertices[*vtx as usize];
                    vertices[*vtx as usize] += offset * value;
                }
            }
        }
    }
    let mut new_mesh = mesh.clone()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    new_mesh.compute_smooth_normals();
    let _ = new_mesh.generate_tangents();
    new_mesh
}

