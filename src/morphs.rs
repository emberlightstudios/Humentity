use bevy::prelude::*;
use bevy::render::{
    mesh::{ 
        VertexAttributeValues,
        PrimitiveTopology,
    },
    render_asset::RenderAssetUsages,
};
use std::collections::HashMap;
use crate::BaseMesh;

pub enum MorphTargetType {
    Macro,
}

/*-------------+
 |  Compnents  |
 +-------------*/
#[derive(Component)]
pub struct MorphTarget {
    pub name: String,
    pub morph_type: MorphTargetType,
    pub(crate) offsets: HashMap<u16, Vec3>,
}

pub(crate) fn bake_morphs_to_mesh(
    shapekeys: &HashMap<String, f32>,
    base_mesh: &Res<BaseMesh>,
    targets: &Query<&MorphTarget>,
    meshes: &mut ResMut<Assets<Mesh>>,
) -> (Vec<Vec3>, Mesh) {
    let mesh = meshes.get(&base_mesh.body_handle).unwrap().clone();
    let Some(VertexAttributeValues::Float32x3(vertices)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("MESH VERTICES FAILURE") };
    let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else { panic!("MESH UV FAILURE") };
    let Some(indices) = mesh.indices() else { panic!("MESH FACE INDICES FAILURE") };
    let mut vertices_vec = vertices.to_vec();
    let mut helpers = base_mesh.vertices.clone();
    for (target_name, &value) in shapekeys.iter() {
        for target in targets.iter() {
            if target.name != *target_name { continue; }
            for (&vertex, &offset) in target.offsets.iter() {
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
