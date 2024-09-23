use bevy::{
    prelude::*,
    render::mesh::VertexAttributeValues,
};
use std::{
    collections::HashMap,
    fs::File,
    path::Path,
    io::{ BufReader, BufRead },
};


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