use bevy::{
    prelude::*,
    render::{
        mesh::{
            VertexAttributeValues,
            PrimitiveTopology,
            Indices,
        },
        render_asset::RenderAssetUsages,
    },
};
use std::{
    collections::{ HashSet, HashMap },
    fs::File,
    path::Path,
    io::{ BufReader, BufRead },
};
use crate::BaseMesh;

pub(crate) fn parse_obj_vertices<T: AsRef<Path>>(filename: T) -> Vec<Vec3> {
    let path = filename.as_ref();
    let err_msg = format!("Couldn't open file {:?}", path);
    let file = File::open(path).expect(&err_msg);
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

pub(crate) fn get_vertex_positions(mesh: &Mesh) -> Vec<Vec3> {
    let Some(VertexAttributeValues::Float32x3(verts)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else { panic!("FAILED TO LOAD MESH VERTEX POSITIONS") };
    let d: Vec<Vec3> = verts.iter()
            .map(|arr| Vec3::new(arr[0], arr[1], arr[2])).collect(); 
    d
}

pub(crate) fn get_vertex_normals(mesh: &Mesh) -> Vec<Vec3> {
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
            else { panic!("FAILED TO LOAD MESH VERTEX NORMALS") };
    let d: Vec<Vec3> = normals.iter()
            .map(|arr| Vec3::new(arr[0], arr[1], arr[2])).collect(); 
    d
}

pub(crate) fn get_uv_coords(mesh: &Mesh) -> Vec<Vec2> {
    let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
            else { panic!("FAILED TO LOAD MESH UV DATA") };
    let d: Vec<Vec2> = uv.iter()
            .map(|arr| Vec2::new(arr[0], arr[1])).collect(); 
    d
}

/*
pub(crate) fn get_joint_indices(mesh: &Mesh) -> Vec<UVec4> {
    let Some(VertexAttributeValues::Uint32x4(ind)) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
            else { panic!("FAILED TO LOAD MESH JOINT INDICES") };
    let d: Vec<UVec4> = ind.iter()
            .map(|arr| UVec4::new(arr[0], arr[1], arr[2], arr[3])).collect(); 
    d
}

pub(crate) fn get_joint_weights(mesh: &Mesh) -> Vec<Vec4> {
    let Some(VertexAttributeValues::Float32x4(wts)) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
            else { panic!("FAILED TO LOAD MESH JOINT INDICES") };
    let d: Vec<Vec4> = wts.iter()
            .map(|arr| Vec4::new(arr[0], arr[1], arr[2], arr[3])).collect(); 
    d
}
*/

// Maps mh vertex ids to vec of bevy ids
pub(crate) fn generate_vertex_map(
    mh_vertices: &Vec<Vec3>,
    vertices: &Vec<Vec3>
) -> HashMap<u16, Vec<u16>> {
    let mut vertex_map = HashMap::<u16, Vec<u16>>::new();
    let mut assigned = std::collections::HashSet::<usize>::new();
    let mut matched = std::collections::HashSet::<usize>::new();

    for (i, mh_vertex) in mh_vertices.iter().enumerate() {
        vertex_map.insert(i as u16, Vec::<u16>::new());
        let vec = vertex_map.get_mut(&(i as u16)).unwrap();
        for (j, vtx) in vertices.iter().enumerate() {
            if vtx == mh_vertex {
                if assigned.contains(&j) {
                    panic!("DUPLICATE VERTICES WHEN MAKING VTX MAP") 
                }
                assigned.insert(j);
                matched.insert(j);
                vec.push(j as u16);
            }
        }
    }
    if matched.len() < vertices.len() {
        panic!("FAILED TO MATCH VERTEX IN VERTEX MAP");
    }
    vertex_map
}
    
// Maps bevy vertex ids to mh id
pub(crate) fn generate_inverse_vertex_map(
    map: &HashMap<u16, Vec<u16>>,
) -> HashMap<u16, u16> {
    let mut inv_vertex_map = HashMap::<u16, u16>::new();
    for (mhv, verts) in map.iter() {
        for vert in verts.iter() { inv_vertex_map.insert(*vert, *mhv); }
    }
    inv_vertex_map
}

pub(crate) fn delete_mesh_verts(
    meshes: &mut ResMut<Assets<Mesh>>,
    base_mesh: &Res<BaseMesh>,
    delete_verts: HashSet<u16>,
) -> Mesh {
    let mesh = meshes.get(&base_mesh.mesh_handle).unwrap().clone();
    let inv_vertex_map = generate_inverse_vertex_map(&base_mesh.vertex_map);

    let vertices = get_vertex_positions(&mesh);
    let normals = get_vertex_normals(&mesh);
    let uv = get_uv_coords(&mesh);
    let indices = mesh.indices().expect("FAILED TO GET MESH FACES");

    let verts = vertices.len() - delete_verts.len();  // Roughly
    let mut new_vertices = Vec::<Vec3>::with_capacity(verts);
    let mut new_normals = Vec::<Vec3>::with_capacity(verts);
    let mut new_uv = Vec::<Vec2>::with_capacity(verts);
    let mut new_indices = Vec::<u16>::with_capacity(verts);
    // map new vertex indices to original before deleting verts
    let mut indices_map = HashMap::<u16, u16>::with_capacity(verts);

    for (&vtx, &mh_vert) in inv_vertex_map.iter() {
        if !delete_verts.contains(&mh_vert) {
            indices_map.insert(vtx, new_vertices.len() as u16);
            new_vertices.push(vertices[vtx as usize]);
            new_normals.push(normals[vtx as usize]);
            new_uv.push(uv[vtx as usize]);
        }
    }
    
    let indices_vec: Vec<u16> = indices.iter().map(|x| x as u16).collect();
    // Find new face indices
    for face in indices_vec.chunks(3) {
        // Check if all vertices still exist in new mesh verts
        if !face.iter().all(|&i| indices_map.contains_key(&(i as u16))) { continue; }
        // Map face to new vertex indices
        new_indices.extend_from_slice(face);
    }
    new_indices = new_indices.iter().map(|x| *indices_map.get(x).unwrap()).collect();

    let mut new_mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, new_vertices)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, new_normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, new_uv)
        .with_inserted_indices(Indices::U16(new_indices));
    new_mesh.compute_smooth_normals();
    let _ = new_mesh.generate_tangents();
    new_mesh
}