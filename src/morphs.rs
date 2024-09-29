use bevy::prelude::*;
use std::{
    collections::{ HashMap, HashSet },
    fs::File,
    io::{ BufReader, BufRead },
    path::PathBuf,
};
use serde::Deserialize;
use serde_json;
use walkdir::WalkDir;
use crate::{ 
    get_vertex_positions,
    BaseMesh,
    HumentityGlobalConfig,
    BODY_SCALE,
    HumanMeshAsset,
};

/*--------------+
 |  JSON Types  |
 +--------------*/
#[derive(Deserialize, Debug)]
struct MacroData {
    macrotargets: HashMap<String, MacroBounds>,
    combinations: HashMap<String, Vec<String>>,
}

#[derive(Deserialize, Debug)]
struct MacroBounds {
    parts: Vec<MacroBound>,
}

#[derive(Deserialize, Debug)]
struct MacroBound {
    lowest: f32,
    highest: f32,
    low: String,
    high: String,
}

#[derive(Deserialize, Debug)]
struct MorphCategoriesJSON(HashMap<String, MorphCategoryJSON>);


#[derive(Deserialize, Debug)]
struct MorphCategoryJSON{
    categories: Vec<CompositeMorph>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
struct CompositeMorph {
    has_left_and_right: bool,
    name: String,
    opposites: Option<Opposites>,
    targets: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
struct Opposites {
    #[serde(rename = "negative-left")]
    negative_left: String,
    #[serde(rename = "negative-right")]
    negative_right: String,
    #[serde(rename = "negative-unsided")]
    negative_unsided: String,
    #[serde(rename = "positive-left")]
    positive_left: String,
    #[serde(rename = "positive-right")]
    positive_right: String,
    #[serde(rename = "positive-unsided")]
    positive_unsided: String,
}

/*-------------+
 |  Resources  |
 +-------------*/
#[allow(dead_code)]
#[derive(Resource)]
pub struct MorphSliders(HashMap<String, Vec<CompositeMorph>>);

#[derive(Resource)]
struct MacroSliders(MacroData);

#[derive(Resource)]
pub struct MorphTargets(HashMap<String, HashMap<u16, Vec3>>);


impl FromWorld for MorphTargets {
    fn from_world(world: &mut World) -> Self {
        // Create Morph Target Entities from all the .target files
        let core_path: PathBuf;
        let target_paths: HashSet<PathBuf>;
        if let Some(config) = world.get_resource::<HumentityGlobalConfig>() {
            core_path = config.core_assets_path.clone();
            target_paths = config.target_paths.clone();
        } else {
            panic!("No global Humentity config loaded");
        };
        let mut names = HashMap::<String, HashMap<u16, Vec3>>::new();
        for target_path in target_paths.iter() {
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
                    names.insert(stem.to_string(), offsets.clone());
                }
                else {
                }
            };
        };
        let file = File::open(core_path.join("targets/macrodetails/macro.json")).expect("FAILED TO OPEN macro.json");
        let reader = BufReader::new(file);
        let macro_json: MacroData = serde_json::from_reader(reader).expect("FAILED TO PARSE macro.json");
        world.insert_resource::<MacroSliders>(MacroSliders(macro_json));

        let file = File::open(core_path.join("targets/target.json")).expect("FAILED TO OPEN target.json");
        let reader = BufReader::new(file);
        let categories_json: MorphCategoriesJSON = serde_json::from_reader(reader).expect("FAILED TO PARSE target.json");
        let mut categories = HashMap::<String, Vec<CompositeMorph>>::new();
        for (category, targets) in categories_json.0.iter() {
            let mut cat = targets.categories.clone();
            for target in cat.iter_mut() {
                if target.opposites.is_some() {
                    target.targets = None;
                } else {
                    target.opposites = None;
                    if target.targets.iter().len() > 1 { panic! {"Should not have more than 1 target without opposites"} }
                }
            }
            categories.insert(category.to_string(), cat);
        }
        world.insert_resource::<MorphSliders>(MorphSliders(categories));

        MorphTargets(names)
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
        let target = targets.0.get(target_name).expect(&err_msg);
        for (&vertex, &offset) in target.iter() {
            helpers[vertex as usize] += offset * value;
        }
    }
    helpers
}

pub(crate) fn bake_body_morphs(
    mesh: &Mesh,
    vertex_map: &HashMap<u16, Vec<u16>>,
    helpers: &Vec<Vec3>,
) -> Mesh {
    let mut vertices = get_vertex_positions(&mesh);
    for (mh_vert, vtx_list) in vertex_map.iter() {
        for vtx in vtx_list.iter() {
            vertices[*vtx as usize] = helpers[*mh_vert as usize];
        }
    }
    mesh.clone()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_computed_smooth_normals()
        .with_generated_tangents().unwrap()
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
        let target = targets.0.get(target_name).expect(&err_msg);

        for (asset_vert, vtx_list) in asset.vertex_map.iter() {
            let helper_map = &asset.helper_maps[*asset_vert as usize];
            for &vtx in vtx_list.iter() {
                if let Some(mh_vtx) = helper_map.single_vertex {
                    let offset = *target.get(&mh_vtx).unwrap();
                    vertices[vtx as usize] += offset * value;
                } else { // Triangulation
                    let triangle = helper_map.triangle.as_ref().unwrap();
                    let mut position = Vec3::ZERO;
                    for i in 0..3 {
                        let mh_vert = triangle.helper_verts[i];
                        let wt = triangle.helper_weights[i];
                        position += *helpers.get(mh_vert as usize).unwrap() * wt;
                    }
                    position += asset.get_offset_scale(helpers) * triangle.helper_offset;
                    let offset = position - vertices[vtx as usize];
                    vertices[vtx as usize] += offset * value;
                }
            }
        }
    }
    mesh.clone()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_computed_smooth_normals()
        .with_generated_tangents().unwrap()
}

