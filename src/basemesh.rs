use bevy::prelude::*;
use bevy::render::mesh::{
    Mesh, VertexAttributeValues, Indices,
};
use std::{
    io::{ BufReader, BufRead },
    fs::File,
    collections::HashMap,
};
use walkdir::WalkDir;
use crate::{
    HumentityState,
    MorphTarget,
    MorphTargetType
};
use serde::Deserialize;
use serde_json;

enum LoadingStep {
    LoadingMeshes,
    BuildingVertexMap,
    Finished,
}

pub(crate) const BODY_VERTICES: u16 = 13380u16;
pub(crate) const BODY_SCALE: f32 = 0.1;

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource, Deserialize, Debug)]
pub(crate) struct VertexGroups(pub(crate) HashMap<String, Vec<[usize; 2]>>);

#[derive(Resource, Debug, Clone)]
pub(crate) struct BaseMesh{
    pub(crate) handle: Handle<Mesh>,
    pub(crate) body_handle: Handle<Mesh>,
    pub(crate) vertices: Vec<Vec3>,
    pub(crate) vertex_map: HashMap<u16, Vec<u16>>,
    pub(crate) inv_vertex_map: HashMap<u16, u16>,
    pub(crate) body_vertex_map: HashMap<u16, Vec<u16>>,
    pub(crate) body_inv_vertex_map: HashMap<u16, u16>,
}

impl FromWorld for BaseMesh {
    fn from_world(world: &mut World) -> Self {
        // Get mh vertices from base mesh and helper files
        let mh_vertices = parse_obj_vertices("assets/base.obj");

        // Load obj into asset server
        let asset_server = world.resource::<AssetServer>();
        let base_handle: Handle<Mesh> = asset_server.load("base.obj");

        let err_msg = "FAILED TO LOAD VERTEX GROUOPS";
        let file = File::open("assets/basemesh_vertex_groups.json").expect(&err_msg);
        let reader = BufReader::new(file);
        let vg: VertexGroups = serde_json::from_reader(reader).unwrap();

        world.insert_resource(vg);

        // Create Morph Target Entities from all the .target files
        let path = "assets/targets";
        for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
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
                world.spawn(MorphTarget {
                    name: stem.to_string(),
                    morph_type: MorphTargetType::Macro,
                    offsets: offsets,
                });
            }
        };
        let mut next = world.get_resource_mut::<NextState<HumentityState>>().expect("No HumentityState registered");
        next.set(HumentityState::FixingHelperMesh);
        BaseMesh{
            handle: base_handle.clone(),
            body_handle: base_handle,
            vertices: mh_vertices,
            vertex_map: HashMap::<u16, Vec<u16>>::new(),
            inv_vertex_map: HashMap::<u16, u16>::new(),
            body_vertex_map: HashMap::<u16, Vec<u16>>::new(),
            body_inv_vertex_map: HashMap::<u16, u16>::new(),
        }
    }
}
        
/*-----------+
 |  Systems  |
 +-----------*/

// Rescale and set feet on ground
pub(crate) fn fix_helper_mesh(
    mut meshes: ResMut<Assets<Mesh>>,
    vg_res: Option<Res<VertexGroups>>,
    base_mesh_res: Option<ResMut<BaseMesh>>,
    mut next: ResMut<NextState<HumentityState>>,
) {
    // Have to make sure these resources loaded correctly already
    let Some(mut base_mesh) = base_mesh_res else { return };
    let Some(mesh) = meshes.get_mut(&base_mesh.handle) else { return };
    let Some(vg) = vg_res else { return };

    // Get feet-on-ground offset
    let v1: usize = vg.0.get("joint-ground").unwrap()[0][0];
    let v2: usize = vg.0.get("joint-ground").unwrap()[0][1];
    let offset = (base_mesh.vertices[v1] + base_mesh.vertices[v2]) * 0.5;

    // Fix mh_vertices cache
    for i in 0..base_mesh.vertices.len() {
        base_mesh.vertices[i] = (base_mesh.vertices[i] - offset) * BODY_SCALE;
    }

    // Loaded mesh vertices
    let Some(VertexAttributeValues::Float32x3(raw_vtx_data)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else { panic!("FAILED TO LOAD MESH VERTEX DATA") };
    let mut vtx_data: Vec<Vec3> = raw_vtx_data.iter().map(|arr| Vec3::new(arr[0], arr[1], arr[2])).collect(); 
    for i in 0..vtx_data.len() {
        vtx_data[i] = (vtx_data[i] - offset) * BODY_SCALE;
    }

    // Reinsert corrected base mesh
    let mut new_mesh = mesh.clone();
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vtx_data);
    base_mesh.handle = meshes.add(new_mesh);

    // Continue to next loading step
    next.set(HumentityState::LoadingBodyMesh);
}

// Remove helper vertices to generate body only mesh
pub(crate) fn create_body_mesh(
    mut next: ResMut<NextState<HumentityState>>,
    mut base_mesh: ResMut<BaseMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(mesh) = meshes.get(&base_mesh.handle) else { return };

    // Get mesh arrays
    let Some(VertexAttributeValues::Float32x3(raw_vtx_data)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else { panic!("FAILED TO LOAD MESH VERTEX DATA") };
    let Some(VertexAttributeValues::Float32x3(raw_normal_data)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
            else { panic!("FAILED TO LOAD MESH NORMAL DATA") };
    let Some(VertexAttributeValues::Float32x2(raw_uv_data)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
            else { panic!("FAILED TO LOAD MESH UV DATA") };
    let Some(raw_indices) = mesh.indices() else { panic!("FAILED TO LOAD MESH INDICES") };

    let vtx_data: Vec<Vec3> = raw_vtx_data.iter().map(|arr| Vec3::new(arr[0], arr[1], arr[2])).collect(); 
    let normal_data: Vec<Vec3> = raw_normal_data.iter().map(|arr| Vec3::new(arr[0], arr[1], arr[2])).collect(); 
    let uv_data: Vec<Vec2> = raw_uv_data.iter().map(|arr| Vec2::new(arr[0], arr[1])).collect(); 

    let vertex_map = generate_vertex_map(&base_mesh.vertices, &vtx_data);
    let inv_vertex_map = generate_inverse_vertex_map(&vertex_map);
    
    let mut new_mesh = mesh.clone();
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vtx_data.clone());

    // Create mesh without helpers
    let body_mesh = generate_mesh_without_helpers(
        &new_mesh,
        &inv_vertex_map,
        vtx_data.clone(),
        normal_data,
        uv_data,
        raw_indices
    );

    // Save values in base mesh resource
    base_mesh.handle = meshes.add(new_mesh);
    base_mesh.body_handle = meshes.add(body_mesh);
    base_mesh.vertex_map = vertex_map;
    base_mesh.inv_vertex_map = inv_vertex_map;
    next.set(HumentityState::LoadingBodyVertexMap);
} 

// Load body mesh to calculate vertex maps
pub(crate) fn create_body_vertex_map(
    mut base_mesh: ResMut<BaseMesh>,
    meshes: Res<Assets<Mesh>>,
    mut next: ResMut<NextState<HumentityState>>,
) {
    let Some(body_mesh) = meshes.get(&base_mesh.body_handle) else { return };
    let Some(VertexAttributeValues::Float32x3(raw_vtx_data)) = body_mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else { panic!("FAILED TO LOAD MESH VERTEX DATA") };
    let vertices: Vec<Vec3> = raw_vtx_data.iter().map(|arr| Vec3::new(arr[0], arr[1], arr[2])).collect(); 
    let body_vertex_map = generate_vertex_map(
        &base_mesh.vertices,
        &vertices
    );
    let body_inv_vertex_map = generate_inverse_vertex_map(&body_vertex_map);
    base_mesh.body_vertex_map = body_vertex_map;
    base_mesh.body_inv_vertex_map = body_inv_vertex_map;
    next.set(HumentityState::Ready);
}

/*---------------------+
 |  Utility Functions  |
 +---------------------*/
fn parse_obj_vertices(filename: &str) -> Vec<Vec3> {
    let err_msg = "Couldn't open file ".to_string() + filename;
    let file = File::open(filename).expect(&err_msg);
    let mut vertices = Vec::<Vec3>::new();
    for line_result in BufReader::new(file).lines() {
        let Ok(line) = line_result else { break };
        if line.starts_with("v ") {
            let coords: Vec<f32> = line.split_whitespace()
                             .skip(1)
                             .filter_map(|x| x.parse().ok())
                             .collect();
            vertices.push(Vec3::new(coords[0], coords[1], coords[2]));
        }
    }
    vertices
}

fn generate_mesh_without_helpers(
    original_mesh: &Mesh,
    inv_vertex_map: &HashMap<u16, u16>,
    vtx_data: Vec<Vec3>,
    normal_data: Vec<Vec3>,
    uv_data: Vec<Vec2>,
    indices_data: &Indices,
) -> Mesh {
    let mut vertices = Vec::<Vec3>::new();
    let mut normals = Vec::<Vec3>::new();
    let mut uv = Vec::<Vec2>::new();
    let mut indices = Vec::<usize>::new();

    // For remapping face indices buffer
    // Some vertices will be skipped, changing the vertex indices
    // So face indices will have to be changed as well
    let mut new_vert_indices = HashMap::<u16, u16>::new();
    let mut i = 0;

    for (vertex, mhv) in inv_vertex_map.iter() {
        if *mhv < BODY_VERTICES {
            new_vert_indices.insert(*vertex, i);
            vertices.push(vtx_data[*vertex as usize]);
            normals.push(normal_data[*vertex as usize]);
            uv.push(uv_data[*vertex as usize]);
            i += 1;
        }
    }
    let index_vec: Vec<usize> = indices_data.iter().collect();
    for chunk in index_vec.chunks(3) {
        if chunk.iter().all(|&x| *inv_vertex_map.get(&(x as u16)).unwrap() < BODY_VERTICES) {
            indices.extend_from_slice(chunk);
        }
    }
    let mut u16indices = Vec::<u16>::with_capacity(indices.len());
    for x in indices { u16indices.push(x as u16); }
    // Since some vertices have been removed the face indices will change
    //  we have to reindex them
    u16indices = u16indices.iter().map(|x| *new_vert_indices.get(x).unwrap()).collect();

    let mut body_mesh = original_mesh.clone()
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices.clone())
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    .with_inserted_indices(Indices::U16(u16indices));
    let _ = body_mesh.generate_tangents();
    body_mesh
}

fn generate_vertex_map(
    mh_vertices: &Vec<Vec3>,
    vertices: &Vec<Vec3>
) -> HashMap<u16, Vec<u16>> {
    let mut vertex_map = HashMap::<u16, Vec<u16>>::new();
    let mut assigned = std::collections::HashSet::<usize>::new();
    for (i, mh_vertex) in mh_vertices.iter().enumerate() {
        vertex_map.insert(i as u16, Vec::<u16>::new());
        let vec = vertex_map.get_mut(&(i as u16)).unwrap();
        for (j, vtx) in vertices.iter().enumerate() {
            if vtx == mh_vertex {
                if assigned.contains(&j) {
                    panic!("DUPLICATE VERTICES WHEN MAKING VTX MAP") 
                }
                assigned.insert(j);
                vec.push(j as u16);
            }
        }
    }
    vertex_map
}
    
fn generate_inverse_vertex_map(
    vertex_map: &HashMap<u16, Vec<u16>>,
) -> HashMap<u16, u16> {
    let mut inv_vertex_map = HashMap::<u16, u16>::new();
    for (mhv, verts) in vertex_map.iter() {
        for vert in verts.iter() { inv_vertex_map.insert(*vert, *mhv); }
    }
    inv_vertex_map
}
