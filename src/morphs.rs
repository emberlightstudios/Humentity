use bevy::prelude::*;
use std::{
    fs::File,
    io::{ BufReader, BufRead },
    path::PathBuf,
};
use fxhash::{FxHashMap};
use serde::Deserialize;
use serde_json;
use walkdir::WalkDir;
use crate::{assets::HumanMeshAsset, basemesh::BODY_SCALE, mesh_ops::get_vertex_positions, prelude::*};

/*--------------+
 |  JSON Types  |
 +--------------*/
#[derive(Deserialize, Debug)]
struct MacroData {
    macrotargets: FxHashMap<String, MacroBounds>,
    combinations: FxHashMap<String, Vec<String>>,
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
struct MorphCategoriesJSON(FxHashMap<String, MorphCategoryJSON>);


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
pub struct MorphSliders(FxHashMap<String, Vec<CompositeMorph>>);

#[derive(Resource)]
struct MacroSliders(MacroData);

#[derive(Resource)]
pub struct MorphTargets(FxHashMap<String, FxHashMap<u16, Vec3>>);


impl FromWorld for MorphTargets {
    fn from_world(world: &mut World) -> Self {
        // Create Morph Target Entities from all the .target files
        let core_path: PathBuf;
        let config = world.get_resource::<HumentityGlobalConfig>()
            .expect("No global Humentity config loaded");
        core_path = config.core_assets_path.clone();
        let target_paths = config.target_paths.clone();
        let mut names = FxHashMap::<String, FxHashMap<u16, Vec3>>::default();
        for target_path in target_paths.iter() {
            for entry in WalkDir::new(core_path.join(target_path)).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                let mut offsets = FxHashMap::<u16, Vec3>::default();
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
        let mut categories = FxHashMap::<String, Vec<CompositeMorph>>::default();
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
    shapekeys: &FxHashMap<String, f32>,
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
    mhid_lookup: &Vec<u16>,
    helpers: &Vec<Vec3>,
) -> Mesh {
    let mut vertices = get_vertex_positions(&mesh);
    for (vert, mh_vert) in mhid_lookup.iter().enumerate() {
        vertices[vert] = helpers[*mh_vert as usize];
    }
    mesh.clone()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_computed_area_weighted_normals()
        .with_generated_tangents().unwrap()
}


pub(crate) fn bake_asset_morphs(
    morphs: &FxHashMap<String, f32>,
    targets: &Res<MorphTargets>,
    meshes: &mut ResMut<Assets<Mesh>>,
    helpers: &Vec<Vec3>,
    asset: &HumanMeshAsset,
) -> Mesh {
    let mesh = meshes.get(&asset.mesh_handle).unwrap().clone();
    let mut vertices = get_vertex_positions(&mesh);
    for (target_name, &value) in morphs.iter() {
        let err_msg = format!("Failed to find morph {}", target_name);
        let target = targets.0.get(target_name).expect(&err_msg);

        for (vert, mh_vert) in asset.mhid_lookup.iter().enumerate() {
            let helper_map = &asset.helper_maps[*mh_vert as usize];
            if let Some(mh_vtx) = helper_map.single_vertex {
                let offset = *target.get(&mh_vtx).unwrap();
                vertices[vert] += offset * value;
            } else { // Triangulation
                let triangle = helper_map.triangle.as_ref().unwrap();
                let mut position = Vec3::ZERO;
                for i in 0..3 {
                    let mh_vert = triangle.helper_verts[i];
                    let wt = triangle.helper_weights[i];
                    position += *helpers.get(mh_vert as usize).unwrap() * wt;
                }
                position += asset.get_offset_scale(helpers) * triangle.helper_offset;
                let offset = position - vertices[vert];
                vertices[vert] += offset * value;
            }
        }
    }
    mesh.clone()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_computed_area_weighted_normals()
        .with_generated_tangents().unwrap()
}

