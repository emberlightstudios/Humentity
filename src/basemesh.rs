use bevy::{prelude::*, asset::{RenderAssetUsages, AssetPath}, mesh::{Indices, Mesh}};
use std::{
    io::BufReader,
    fs::File,
};
use fxhash::FxHashMap;
use serde::Deserialize;
use serde_json;

use crate::mesh_ops::{generate_mhid_lookup, generate_vertex_map, get_uv_coords, get_vertex_positions, parse_obj_vertices};
use crate::prelude::HumentityGlobalConfig;
use crate::{HumentityLoading};

pub(crate) const BODY_VERTICES: u16 = 13380u16;
pub(crate) const BODY_SCALE: f32 = 0.1;

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource, Deserialize, Debug)]
pub(crate) struct VertexGroups(pub(crate) FxHashMap<String, Vec<[usize; 2]>>);

#[derive(Resource, Debug)]
pub struct BaseMesh{
    pub(crate) mesh_handle: Handle<Mesh>,
    pub(crate) vertices: Vec<Vec3>,
    pub(crate) mhid_lookup: Vec<u16>,
}

#[derive(Resource, Debug, Clone)]
pub(crate) struct HelperMeshHandle(Handle<Mesh>);

// Load base mesh with helpers and vertex group data
impl FromWorld for BaseMesh {
    fn from_world(world: &mut World) -> Self {
        let config = world.get_resource::<HumentityGlobalConfig>().expect("NO CONFIG LOADED");
        let path = config.core_assets_path.clone();
        if !path.join("base.obj").exists() {
            panic!("base.obj not found.  Did you provide the correct path to the Humentity crate?")
        }
        // Get mh vertices from base mesh and helper files
        let mh_vertices = parse_obj_vertices(path.join("base.obj"));

        // Load obj into asset server
        let asset_server = world.resource::<AssetServer>();
        let base_handle: Handle<Mesh> = asset_server.load("humentity://base.obj");

        let file = File::open(path.join("basemesh_vertex_groups.json"))
            .expect("FAILED TO LOAD VERTEX GROUOPS");
        let reader = BufReader::new(file);
        let vg: VertexGroups = serde_json::from_reader(reader).unwrap();

        world.insert_resource(vg);
        world.insert_resource(HelperMeshHandle(base_handle.clone()));

        BaseMesh{
            mesh_handle: base_handle,
            vertices: mh_vertices,
            mhid_lookup: vec![],
        }

    }
}
        
// Remove helper vertices to generate body only mesh
pub(crate) fn create_body_mesh(
    mut base_mesh: ResMut<BaseMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    helper_handle: Option<Res<HelperMeshHandle>>,
) {
    let Some(helper_handle) = helper_handle else { return; };
    let Some(mesh) = meshes.get_mut(&helper_handle.0) else { return };

    // Get mesh arrays
    let raw_indices = mesh.indices().expect("FAILED TO LOAD MESH INDICES");
    let vtx_data = get_vertex_positions(&mesh);
    let uv_data = get_uv_coords(&mesh);
    let vertex_map = generate_vertex_map(&base_mesh.vertices, &vtx_data);
    let mhid_lookup = generate_mhid_lookup(&vertex_map);

    // Create mesh without helpers
    let mesh = generate_mesh_without_helpers(
        &mhid_lookup,
        vtx_data,
        uv_data,
        raw_indices
    )
        .with_computed_area_weighted_normals()
        .with_generated_tangents().unwrap();

    let vtx_data = get_vertex_positions(&mesh);
    let vertex_map = generate_vertex_map(&base_mesh.vertices[..BODY_VERTICES as usize], &vtx_data);

    // Save values in base mesh resource
    base_mesh.mesh_handle = meshes.add(mesh);
    base_mesh.mhid_lookup = generate_mhid_lookup(&vertex_map);

    commands.remove_resource::<HelperMeshHandle>();
    commands.remove_resource::<HumentityLoading>();
} 

fn generate_mesh_without_helpers(
    mhid_lookup: &Vec<u16>,
    vtx_data: Vec<Vec3>,
    uv_data: Vec<Vec2>,
    indices_data: &Indices,
) -> Mesh {
    // Final buffers
    let mut vertices = Vec::<Vec3>::new();
    let mut uv = Vec::<Vec2>::new();

    // Mapping old vertex index -> new vertex index
    let mut new_vert_indices = std::collections::HashMap::<u16, u16>::default();

    // Build new vertex buffer, and UVs
    for (vertex, &mh_id) in mhid_lookup.iter().enumerate() {
        if mh_id < BODY_VERTICES {
            new_vert_indices.insert(vertex as u16, vertices.len() as u16);
            vertices.push(vtx_data[vertex as usize]);
            uv.push(uv_data[vertex as usize]);
        }
    }

    // Build new index buffer
    let indices_data: &Vec<u16> = match indices_data {
        Indices::U16(indices_data) => indices_data,
        Indices::U32(indices_data) => &indices_data.iter().map(|&i| i as u16).collect(),
    };
    let mut new_indices = Vec::<u16>::new();
    for chunk in indices_data.iter().as_slice().chunks(3) {
        // Only include triangles where all vertices exist in new_vert_indices
        if chunk.iter().all(|x| new_vert_indices.contains_key(x)) {
            new_indices.extend(chunk.iter().map(|x| new_vert_indices[x]));
        }
    }

    // Create the new mesh
    Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U16(new_indices))
}
