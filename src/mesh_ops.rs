use crate::assets::CharacterPart;
use ahash::{AHashMap, AHashSet};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};


/// A message to be sent when a mesh is ready
#[derive(Message)]
pub struct CharacterAssetMeshReady {
    pub part: CharacterPart,
    pub prefab: &'static str,
}

pub fn parse_obj_vertices<T: AsRef<Path>>(filename: T) -> Vec<Vec3> {
    let path = filename.as_ref();
    let file = File::open(path).unwrap_or_else(|_| panic!("Couldn't open file {:?}", path));
    let mut vertices = Vec::<Vec3>::new();
    for line_result in BufReader::new(file).lines() {
        let Ok(line) = line_result else { break };
        if line.starts_with("v ") {
            let coords: Vec<f32> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|x| x.parse().ok())
                .collect();
            vertices.push(Vec3::new(coords[0], coords[1], coords[2]));
        }
    }
    vertices
}

pub fn get_vertex_positions(mesh: &Mesh) -> Vec<Vec3> {
    let Some(VertexAttributeValues::Float32x3(verts)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("FAILED TO LOAD MESH VERTEX POSITIONS")
    };
    verts
        .iter()
        .map(|arr| Vec3::new(arr[0], arr[1], arr[2]))
        .collect::<Vec<Vec3>>()
}

pub(crate) fn get_vertex_tangents(mesh: &Mesh) -> Result<Vec<Vec3>, BevyError> {
    let Some(VertexAttributeValues::Float32x4(verts)) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT)
    else {
        return Err(BevyError::from("NO TANGENTS PRESENT ON MESH"));
    };
    Ok(verts
        .iter()
        .map(|arr| Vec3::new(arr[0], arr[1], arr[2]))
        .collect::<Vec<Vec3>>())
}

pub(crate) fn get_vertex_normals(mesh: &Mesh) -> Vec<Vec3> {
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("FAILED TO LOAD MESH VERTEX NORMALS")
    };
    normals
        .iter()
        .map(|arr| Vec3::new(arr[0], arr[1], arr[2]))
        .collect::<Vec<Vec3>>()
}

pub(crate) fn get_uv_coords(mesh: &Mesh) -> Vec<Vec2> {
    let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        panic!("FAILED TO LOAD MESH UV DATA")
    };
    uv.iter()
        .map(|arr| Vec2::new(arr[0], arr[1]))
        .collect::<Vec<Vec2>>()
}

//pub(crate) fn get_joint_indices(mesh: &Mesh) -> Vec<UVec4> {
//    let Some(VertexAttributeValues::Uint32x4(ind)) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
//            else { panic!("FAILED TO LOAD MESH JOINT INDICES") };
//    let d: Vec<UVec4> = ind.iter()
//            .map(|arr| UVec4::new(arr[0], arr[1], arr[2], arr[3])).collect();
//    d
//}
//
//pub(crate) fn get_joint_weights(mesh: &Mesh) -> Vec<Vec4> {
//    let Some(VertexAttributeValues::Float32x4(wts)) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
//            else { panic!("FAILED TO LOAD MESH JOINT INDICES") };
//    let d: Vec<Vec4> = wts.iter()
//            .map(|arr| Vec4::new(arr[0], arr[1], arr[2], arr[3])).collect();
//    d
//}

/// Some vertices from the makehuman obj file get duplicated in Bevy's Mesh GPU data.
/// These duplicates show up at UV seams. We auto-generate normals for morphed
/// meshes. This leads to artifacts at the seams because the algorithm does not see
/// a smooth surface across the seam but rather 2 distinct surfaces edges which just
/// happen to terminate along the same seam. mhid_lookup maps bevy verts back to the
/// original obj vert indices.  Now we can partition the bevy verts to find groups 
/// of duplicates which share exactly the same position in 3d space, i.e. verts on the
/// seams. Finding the mean normal vector for each group SHOULD be the corrent normal
/// for a smooth surface across the seam, which SHOULD fix the normals at the seams
/// as well as the interpolated normals in triangles which border the seams. 
pub fn fix_normals(mesh: &mut Mesh, mhid_lookup: &[u16]) {
    let mut normals = get_vertex_normals(mesh);
    let mut groups: AHashMap<u16, Vec<usize>> = AHashMap::default();

    #[allow(clippy::needless_range_loop)]
    for bevy_idx in 0..normals.len() {
        groups
            .entry(mhid_lookup[bevy_idx])
            .or_default()
            .push(bevy_idx);
    }

    // Average normals per group with duplicates
    for group in groups.values()
        .filter(|&v| v.len() > 1)
    {
        let mut sum = Vec3::ZERO;
        for &i in group {
            sum += normals[i];
        }
        let avg = sum.normalize_or_zero();
        for &i in group {
            normals[i] = avg;
        }
    }

    // Write back to the mesh
    let normals = normals
        .iter()
        .map(|n| [n.x, n.y, n.z])
        .collect::<Vec<[f32; 3]>>();
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);

    // Regenerate tangents
    mesh.generate_tangents().ok();
}

// Maps mh vertex ids to vec of bevy ids
pub fn generate_vertex_map(
    mh_vertices: &[Vec3],
    vertices: &[Vec3],
) -> AHashMap<u16, Vec<u16>> {
    let mut vertex_map = AHashMap::<u16, Vec<u16>>::default();
    let mut matched = AHashSet::<usize>::default();

    for (i, mh_vertex) in mh_vertices.iter().enumerate() {
        let vec = vertex_map.entry(i as u16).or_insert(vec![]);
        for (j, vtx) in vertices.iter().enumerate() {
            if vtx == mh_vertex {
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
pub fn generate_mhid_lookup(map: AHashMap<u16, Vec<u16>>) -> Vec<u16> {
    let max_vert = map
        .values()
        .flat_map(|v| v.iter())
        .max()
        .copied()
        .unwrap_or(0);

    let mut lkup: Vec<u16> = vec![0; max_vert as usize + 1];
    for (&mhv, verts) in map.iter() {
        for &vert in verts.iter() {
            lkup[vert as usize] = mhv;
        }
    }
    lkup
}
