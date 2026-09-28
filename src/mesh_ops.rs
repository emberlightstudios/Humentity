use ahash::{AHashMap, AHashSet};
use bevy::{mesh::VertexAttributeValues, prelude::*};

use crate::loaders::MhcloVertexMap;

pub fn get_vertex_positions(mesh: &Mesh) -> Vec<Vec3> {
    let Some(VertexAttributeValues::Float32x3(verts)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("mesh build needs POSITION vertices: the proxy mesh has no position attribute (check the proxy .obj and its ObjVerts snapshot)")
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
        panic!("mesh build needs NORMAL vertices: the proxy mesh has no normal attribute (check the proxy .obj — re-export with normals)")
    };
    normals
        .iter()
        .map(|arr| Vec3::new(arr[0], arr[1], arr[2]))
        .collect::<Vec<Vec3>>()
}

pub(crate) fn get_uv_coords(mesh: &Mesh) -> Vec<Vec2> {
    let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        panic!("mesh build needs UV_0 texcoords: the proxy mesh has no uv attribute (check the proxy .obj — re-export with uvs)")
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

/// Some vertices from the makehuman obj files get duplicated in Bevy's Mesh GPU data.
/// These duplicates show up at UV seams. We auto-generate normals for morphed
/// meshes. This leads to artifacts at the seams because the algorithm does not see
/// a smooth surface across the seam but rather 2 distinct surface edges which just
/// happen to terminate along the same seam, so the normal does not vary smoothly.
/// Using the mean normal vector for each group of duplicates removes these discontinuities.
pub fn fix_normals(mesh: &mut Mesh, groups: &AHashMap<u16, Vec<u16>>) {
    let mut normals = get_vertex_normals(mesh);

    // Average normals per group with duplicates
    for group in groups.values().filter(|&v| v.len() > 1) {
        let mut sum = Vec3::ZERO;
        for &i in group {
            sum += normals[i as usize];
        }
        let avg = sum.normalize_or_zero();
        for &i in group {
            normals[i as usize] = avg;
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

pub fn fix_normals_multiple(meshes: &mut [&mut Mesh]) {
    #[derive(Clone)]
    struct VertexRef {
        mesh_idx: usize,
        vert_idx: usize,
    }

    // Collect positions and normals for all meshes
    let mut all_vertices: Vec<(Vec3, VertexRef)> = Vec::new();
    let mut normals_per_mesh: Vec<Vec<Vec3>> = Vec::with_capacity(meshes.len());

    for (mesh_idx, mesh) in meshes.iter().enumerate() {
        let positions = get_vertex_positions(mesh);
        let normals = get_vertex_normals(mesh);

        normals_per_mesh.push(normals);

        for (vert_idx, &pos) in positions.iter().enumerate() {
            all_vertices.push((pos, VertexRef { mesh_idx, vert_idx }));
        }
    }

    // Brute-force grouping: merge normals for vertices at exact same position
    let mut visited = vec![false; all_vertices.len()];

    for i in 0..all_vertices.len() {
        if visited[i] {
            continue;
        }

        let (pos_i, ref_i) = &all_vertices[i];
        let mut group = vec![ref_i.clone()];
        visited[i] = true;

        for j in (i + 1)..all_vertices.len() {
            if visited[j] {
                continue;
            }
            let (pos_j, ref_j) = &all_vertices[j];
            if *pos_i == *pos_j {
                group.push(ref_j.clone());
                visited[j] = true;
            }
        }

        // Average normals for this group
        if group.len() > 1 {
            let mut sum = Vec3::ZERO;
            for v in &group {
                sum += normals_per_mesh[v.mesh_idx][v.vert_idx];
            }
            let avg = sum.normalize_or_zero();

            for v in &group {
                normals_per_mesh[v.mesh_idx][v.vert_idx] = avg;
            }
        }
    }

    // Write back to meshes
    for (mesh_idx, mesh) in meshes.iter_mut().enumerate() {
        let normals = normals_per_mesh[mesh_idx]
            .iter()
            .map(|n| [n.x, n.y, n.z])
            .collect::<Vec<[f32; 3]>>();

        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.generate_tangents().ok();
    }
}

// Maps mh vertex ids to vec of bevy ids
pub fn generate_vertex_map(mh_vertices: &[Vec3], vertices: &[Vec3]) -> AHashMap<u16, Vec<u16>> {
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
        panic!(
            "proxy mesh topology does not match its vertex snapshot: matched {} of {} mesh vertices by exact position (the mesh and its ObjVerts snapshot must come from the same file)",
            matched.len(),
            vertices.len(),
        );
    }
    vertex_map
}

// Maps bevy vertex ids to mh id
pub fn generate_mhid_lookup(map: &AHashMap<u16, Vec<u16>>) -> Vec<u16> {
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

/// Paints per-vertex joint indices + weights for one LOD bone order: aggregates
/// helper-vertex skin weights through the proxy map, keeps the top 4 per
/// vertex, and writes Bevy's standard skin attributes. Mesh painting, not rig
/// data — lives here with the other mesh builders.
pub(crate) fn set_asset_rig_arrays(
    mesh: &mut Mesh,
    mhid_lookup: &[u16],
    helper_map: &[MhcloVertexMap],
    bone_names: &[&'static str],
    rig_weights: &AHashMap<&'static str, AHashMap<u16, f32>>,
) {
    let vertex_count = mhid_lookup.len();
    // Final fixed-size output arrays
    let mut indices: Vec<[u16; 4]> = vec![[0; 4]; vertex_count];
    let mut weights: Vec<[f32; 4]> = vec![[0.0; 4]; vertex_count];
    // Cache per mhid so duplicates are consistent and O(n)
    let mut mhid_cache: AHashMap<u16, ([u16; 4], [f32; 4])> = AHashMap::default();
    for (vert, &mhid) in mhid_lookup.iter().enumerate() {
        // If we've already computed this mhid, reuse it
        if let Some(&(cached_indices, cached_weights)) = mhid_cache.get(&mhid) {
            indices[vert] = cached_indices;
            weights[vert] = cached_weights;
            continue;
        }
        let helper = &helper_map[mhid as usize];
        // Aggregate bone weights deterministically
        let mut aggregate: AHashMap<u16, f32> = AHashMap::default();
        for (bone_index, &bone_name) in bone_names.iter().enumerate() {
            let Some(bone_weights) = rig_weights.get(bone_name) else {
                continue;
            };
            match helper {
                MhcloVertexMap::SingleVertex(v) => {
                    if let Some(&helper_wt) = bone_weights.get(v)
                        && helper_wt > 0.0
                    {
                        *aggregate.entry(bone_index as u16).or_insert(0.0) += helper_wt;
                    }
                }
                MhcloVertexMap::Triangle {
                    helper_verts,
                    helper_weights,
                    ..
                } => {
                    for (i, mh_id) in helper_verts.iter().enumerate() {
                        if let Some(&helper_wt) = bone_weights.get(mh_id)
                            && helper_wt > 0.0
                        {
                            *aggregate.entry(bone_index as u16).or_insert(0.0) +=
                                helper_wt * helper_weights[i];
                        }
                    }
                }
            }
        }
        // Convert to vec and sort deterministically
        let mut pairs: Vec<(u16, f32)> = aggregate.into_iter().collect();
        pairs.sort_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap().then_with(|| a.0.cmp(&b.0)) // tie-break by bone index
        });
        // Take top 4
        pairs.truncate(4);
        // If empty, just leave zeros
        if pairs.is_empty() {
            continue;
        }
        // Pad to 4 entries if needed
        while pairs.len() < 4 {
            pairs.push(pairs[0]);
        }
        let mut raw_indices = [0u16; 4];
        let mut raw_weights = [0.0f32; 4];
        for i in 0..4 {
            raw_indices[i] = pairs[i].0;
            raw_weights[i] = pairs[i].1;
        }
        // Normalize weights
        let sum: f32 = raw_weights.iter().sum();
        if sum > 0.0 {
            for w in &mut raw_weights {
                *w /= sum;
            }
        }
        indices[vert] = raw_indices;
        weights[vert] = raw_weights;
        // Store in cache for duplicate mhids
        mhid_cache.insert(mhid, (raw_indices, raw_weights));
    }
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(indices),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_WEIGHT,
        VertexAttributeValues::Float32x4(weights),
    );
}
