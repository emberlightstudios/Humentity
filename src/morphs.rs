use bevy::prelude::*;
use bevy::render::{
    mesh::{ 
        VertexAttributeValues,
        PrimitiveTopology,
    },
    render_asset::RenderAssetUsages,
};
use std::{
    collections::{ HashMap, HashSet },
    fs::File,
    io::{ BufReader, BufRead },
};
use walkdir::WalkDir;
use crate::{ 
    BaseMesh,
    HumentityGlobalConfig,
    BODY_SCALE,
};


pub struct MorphTarget(HashMap<u16, Vec3>);

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource)]
pub struct MorphTargets {
    names: HashMap<String, MorphTarget>,
    categories: HashMap<String, HashSet<MorphTarget>>,
}

impl FromWorld for MorphTargets {
    fn from_world(world: &mut World) -> Self {
        // Create Morph Target Entities from all the .target files
        let Some(config) = world.get_resource_mut::<HumentityGlobalConfig>() else {
            panic!("No global Humentity config loaded");
        };
        let mut names = HashMap::<String, MorphTarget>::new();
        let mut categories = HashMap::<String, HashSet<MorphTarget>>::new();
        let mut category = "".to_string();
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
                    names.insert(stem.to_string(), MorphTarget(offsets));
                }
                else {
                    category = path.file_stem().unwrap().to_string_lossy().into();
                    categories.insert(category, HashSet::<MorphTarget>::new());
                }
            };
        };
        MorphTargets {
            names: names,
            categories: categories,
        }
    }
}

/*-------------+
 |  Functions  |
 +-------------*/
pub(crate) fn bake_morphs_to_mesh(
    shapekeys: &HashMap<String, f32>,
    base_mesh: &Res<BaseMesh>,
    targets: &Res<MorphTargets>,
    meshes: &mut ResMut<Assets<Mesh>>,
) -> (Vec<Vec3>, Mesh) {
    let mesh = meshes.get(&base_mesh.body_handle).unwrap().clone();
    let Some(VertexAttributeValues::Float32x3(vertices)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("MESH VERTICES FAILURE") };
    let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else { panic!("MESH UV FAILURE") };
    let Some(indices) = mesh.indices() else { panic!("MESH FACE INDICES FAILURE") };
    let mut vertices_vec = vertices.to_vec();
    let mut helpers = base_mesh.vertices.clone();
    for (target_name, &value) in shapekeys.iter() {
        for (name, target) in targets.names.iter() {
            if *name != *target_name { continue; }
            for (&vertex, &offset) in target.0.iter() {
                if let Some(vtx_list) = base_mesh.body_vertex_map.get(&(vertex as u16)) {
                    for vtx in vtx_list.iter() {
                        vertices_vec[*vtx as usize][0] += offset.x * value;
                        vertices_vec[*vtx as usize][1] += offset.y * value;
                        vertices_vec[*vtx as usize][2] += offset.z * value;
                    }
                }
                helpers[vertex as usize] += offset * value;
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices_vec.clone())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv.clone())
    .with_inserted_indices(indices.clone());
    mesh.compute_smooth_normals();
    let _ = mesh.generate_tangents();
    (helpers, mesh)
}
