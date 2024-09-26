use bevy::prelude::*;
use bevy::render::mesh::{
    Mesh, Indices,
};
use std::{
    io::BufReader,
    fs::File,
    collections::HashMap,
};
use crate::{
    generate_inverse_vertex_map,
    generate_vertex_map,
    get_uv_coords,
    get_vertex_normals,
    get_vertex_positions,
    parse_obj_vertices,
    HumentityGlobalConfig,
    HumentityState,
}; 
use serde::Deserialize;
use serde_json;

pub(crate) const BODY_VERTICES: u16 = 13380u16;
pub(crate) const BODY_SCALE: f32 = 0.1;

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource, Deserialize, Debug)]
pub(crate) struct VertexGroups(pub(crate) HashMap<String, Vec<[usize; 2]>>);

#[derive(Resource, Debug)]
pub(crate) struct BaseMesh{
    pub(crate) mesh_handle: Handle<Mesh>,
    pub(crate) vertices: Vec<Vec3>,
    pub(crate) vertex_map: HashMap<u16, Vec<u16>>,
}

#[derive(Resource, Debug)]
pub(crate) struct HelperMeshHandle(Handle<Mesh>);

impl FromWorld for BaseMesh {
    fn from_world(world: &mut World) -> Self {
        let config = world.get_resource::<HumentityGlobalConfig>().expect("NO CONFIG LOADED");
        let path = config.core_assets_path.clone();
        println!("{}", path.to_str().unwrap());
        // Get mh vertices from base mesh and helper files
        let mh_vertices = parse_obj_vertices(path.join("base.obj"));

        // Load obj into asset server
        let asset_server = world.resource::<AssetServer>();
        let base_handle: Handle<Mesh> = asset_server.load(path.join("base.obj"));

        let err_msg = "FAILED TO LOAD VERTEX GROUOPS";
        let file = File::open(path.join("basemesh_vertex_groups.json")).expect(&err_msg);
        let reader = BufReader::new(file);
        let vg: VertexGroups = serde_json::from_reader(reader).unwrap();

        world.insert_resource(vg);
        world.insert_resource(HelperMeshHandle(base_handle.clone()));

        let mut next = world.get_resource_mut::<NextState<HumentityState>>().expect("No HumentityState registered");
        next.set(HumentityState::LoadingBodyMesh);
        BaseMesh{
            mesh_handle: base_handle,
            vertices: mh_vertices,
            vertex_map: HashMap::<u16, Vec<u16>>::new(),
        }

    }
}
        
/*-----------+
 |  Systems  |
 +-----------*/
// Remove helper vertices to generate body only mesh
pub(crate) fn create_body_mesh(
    mut next: ResMut<NextState<HumentityState>>,
    mut base_mesh: ResMut<BaseMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    helper_handle: Res<HelperMeshHandle>,
) {
    let Some(mesh) = meshes.get(&helper_handle.0) else { return };

    // Get mesh arrays
    let Some(raw_indices) = mesh.indices() else { panic!("FAILED TO LOAD MESH INDICES") };
    let vtx_data = get_vertex_positions(&mesh);
    let normal_data = get_vertex_normals(&mesh); 
    let uv_data = get_uv_coords(&mesh);

    let vertex_map = generate_vertex_map(&base_mesh.vertices, &vtx_data);
    
    let mut new_mesh = mesh.clone();
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vtx_data.clone());

    // Create mesh without helpers
    let body_mesh = generate_mesh_without_helpers(
        &new_mesh,
        &vertex_map,
        vtx_data.clone(),
        normal_data,
        uv_data,
        raw_indices
    );

    // Save values in base mesh resource
    base_mesh.mesh_handle = meshes.add(body_mesh);
    commands.remove_resource::<HelperMeshHandle>();
    next.set(HumentityState::LoadingBodyVertexMap);
} 

// Load body mesh to calculate vertex maps
pub(crate) fn create_body_vertex_map(
    mut base_mesh: ResMut<BaseMesh>,
    meshes: Res<Assets<Mesh>>,
    mut next: ResMut<NextState<HumentityState>>,
) {
    let Some(body_mesh) = meshes.get(&base_mesh.mesh_handle) else { return };
    let vertices = get_vertex_positions(&body_mesh);
    let body_vertex_map = generate_vertex_map(&base_mesh.vertices, &vertices);
    base_mesh.vertex_map = body_vertex_map;
    next.set(HumentityState::LoadingAssetVertexMaps);
}

/*---------------------+
 |  Utility Functions  |
 +---------------------*/
fn generate_mesh_without_helpers(
    original_mesh: &Mesh,
    vertex_map: &HashMap<u16, Vec<u16>>,
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

    let inv_map = generate_inverse_vertex_map(vertex_map);
    for (vertex, mhv) in inv_map.iter() {
        if *mhv < BODY_VERTICES {
            new_vert_indices.insert(*vertex, vertices.len() as u16);
            vertices.push(vtx_data[*vertex as usize]);
            normals.push(normal_data[*vertex as usize]);
            uv.push(uv_data[*vertex as usize]);
        }
    }
    let index_vec: Vec<usize> = indices_data.iter().collect();
    for chunk in index_vec.chunks(3) {
        if chunk.iter().all(|&x| *inv_map.get(&(x as u16)).unwrap() < BODY_VERTICES) {
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