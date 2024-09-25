use bevy::{
    prelude::*,
    render::mesh::{
        skinning::{ SkinnedMesh, SkinnedMeshInverseBindposes},
        VertexAttributeValues,
    },
    color::palettes::css::RED,
};
use serde::Deserialize;
use serde_json;
use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
};
use crate::{
    get_vertex_positions,
    HelperMap,
    VertexGroups
};

#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub enum RigType {
    None,
    Default,
    Mixamo,
    GameEngine,
}

/*---------+
 |  JSON   |
 +---------*/
#[derive(Deserialize, Debug)]
struct BoneTransform {
    cube_name: Option<String>,
    //default_position: Vec3,
    strategy: String,
    vertex_indices: Option<Vec<u16>>,
    vertex_index: Option<u16>,
}

#[derive(Deserialize, Debug)]
struct BoneData {
    head: BoneTransform,
    //inherit_scale: String,
    parent: String,
    //roll: f32,
    tail: BoneTransform,
}

#[derive(Deserialize, Debug)]
struct WeightsFile {
    weights: HashMap<String, Vec<(u16, f32)>>
}

// Contains an extra layer for some reason.  Usual config is in the bones key
#[derive(Deserialize, Debug)]
struct MixamoRigConfig {
    bones: HashMap<String, BoneData>
}

/*-----------+
 | Resources |
 +-----------*/
#[derive(Resource)]
pub(crate) struct RigData {
    weights: HashMap<RigType, HashMap<String, HashMap<u16, f32>>>,
    configs: HashMap<RigType, HashMap<String, BoneData>>,
}

impl FromWorld for RigData {
    fn from_world(_world: &mut World) -> Self {
        let mut type_strings = HashMap::<RigType, &str>::new();
        type_strings.insert(RigType::Default, "default");
        type_strings.insert(RigType::Mixamo, "mixamo");
        type_strings.insert(RigType::GameEngine, "game_engine");

        let mut rig_weights = HashMap::<RigType, HashMap<String, HashMap<u16, f32>>>::new();
        let mut rig_configs = HashMap::<RigType, HashMap<String, BoneData>>::new();

        for (rig_type, name) in type_strings.iter() {
            let err_msg = "FAILED TO OPEN WEIGHTS FILE : ".to_string() + name;
            let weights_file = File::open("assets/rigs/weights.".to_string() + type_strings.get(rig_type).unwrap() + ".json").expect(&err_msg);
            let weights_reader = BufReader::new(weights_file);
            let err_msg = "FAILED TO READ WEIGHTS JSON : ".to_string() + name;
            let weights: WeightsFile = serde_json::from_reader(weights_reader).expect(&err_msg);
            let mut weights_hashmap = HashMap::<String, HashMap<u16, f32>>::new();
            for (bone, wts) in weights.weights.iter() {
                let hashmap: HashMap<u16, f32> = wts.iter().cloned().collect();
                weights_hashmap.insert(bone.to_string(), hashmap);
            }
            rig_weights.insert(*rig_type, weights_hashmap);

            let err_msg = "FAILED TO OPEN CONFIG FILE : ".to_string() + name;
            let config_file = File::open("assets/rigs/rig.".to_string() + type_strings.get(rig_type).unwrap() + ".json").expect(&err_msg);
            let config_reader = BufReader::new(config_file);
            let err_msg = "FAILED TO READ CONFIG JSON : ".to_string() + name;
            if *rig_type == RigType::Mixamo {
                let config: MixamoRigConfig = serde_json::from_reader(config_reader).expect(&err_msg);
                rig_configs.insert(*rig_type, config.bones);
            } else {
                let config: HashMap<String, BoneData> = serde_json::from_reader(config_reader).expect(&err_msg);
                rig_configs.insert(*rig_type, config);
            }
        }
        RigData {
            weights: rig_weights,
            configs: rig_configs,
        }
    }
}

/*------------+
 | Components |
 +------------*/
 #[allow(dead_code)]
 #[derive(Component)]
 pub(crate) struct Bone(String);

/*---------+
 | Systems |
 +---------*/
 pub(crate) fn bone_debug_draw(
    query: Query<(&Transform, &Parent), With<Bone>>,
    transforms: Query<&Transform, With<Bone>>,
    mut gizmos: Gizmos,
 ) {
    query.iter().for_each(|(transform, parent)| {
        let start = transform.translation;
        if let Ok(end) = transforms.get(parent.get()) {
            gizmos.line(start, end.translation, RED);
        }
    })
 }

/*-----------+
 | Functions |
 +-----------*/
pub(crate) fn build_rig(
    human: &Entity,
    rig: RigType,
    rigs: &Res<RigData>,
    inv_bindpose_assets: &mut ResMut<Assets<SkinnedMeshInverseBindposes>>,
    commands: &mut Commands,
    vg: &Res<VertexGroups>,
    helpers: &Vec<Vec3>,
    spawn_transform: Transform,
) -> (SkinnedMesh, Vec<String>) {
    let config_res = rigs.configs.get(&rig).unwrap();

    // Spawn bone entities
    // Use human as root of skeleton
    commands.entity(*human).insert(Bone("Root".to_string()));
    let mut bone_entities = HashMap::<String, Entity>::with_capacity(config_res.len());
    for (name, bone) in config_res.iter() {
        if bone.parent == "" && name.eq_ignore_ascii_case("root"){
            bone_entities.insert(name.to_string(), *human);
        } else {
            bone_entities.insert(name.to_string(), commands.spawn(Bone(name.to_string())).id());
        }
    }

    // For finding in-degree of each bone in the tree
    let mut in_degree = HashMap::<String, usize>::with_capacity(config_res.len());

    // Set up parent child relationships
    for (name, bone) in config_res.iter() {
        in_degree.insert(name.to_string(), 0);
        let &child = bone_entities.get(name).unwrap();
        if let Some(parent) = bone_entities.get(&bone.parent) {
            commands.entity(*parent).push_children(&[child]);
        }
    }

    // Find in-degree of the bones
    for (name, bone) in config_res.iter() {
        let mut parent = bone.parent.clone();
        while parent != "" {
            *in_degree.entry(name.to_string()).or_insert(0) += 1;
            parent = config_res.get(&parent).unwrap().parent.clone();
        }
    }

    // Get bone vecs sorted by degree
    let mut in_degree_vec: Vec<(String, usize)> = in_degree.into_iter().collect();
    in_degree_vec.sort_by(|a, b| a.1.cmp(&b.1));
    let mut sorted_bones: Vec<String> = in_degree_vec.into_iter().map(|(k, _)| k.clone()).collect();
    let mut joints: Vec<Entity> = sorted_bones.iter().map(|name| {
        *bone_entities.get(name).unwrap()
    }).collect();

    // Set transforms and inverse bind poses
    let mut inv_bindposes = Vec::<Mat4>::with_capacity(joints.len());
    let mut matrices = HashMap::<String, Mat4>::with_capacity(joints.len());
    for name in sorted_bones.iter() {
        let bone = config_res.get(name).unwrap();
        let &entity = bone_entities.get(name).unwrap();
        let transform: Transform;
        if !name.eq_ignore_ascii_case("root") {
            transform = spawn_transform * get_bone_transform(
                &bone.head,
                &bone.tail,
                &vg,
                &helpers
            );
        } else {
            // Force root bone at origin, default rig has it at hips
            transform = spawn_transform;
        }
        let parent = &bone.parent;
        // No idea why this works.  Shouldn't need to multiply by parent
        // Typically you would do this with local transforms to bring them
        // to the global space.  But these are already global so it makes no 
        // sense but seems to work for some reason.
        let mut xform_mat = transform.compute_matrix();
        if parent != "" {
            let parent_mat = *matrices.get(parent).unwrap();
            xform_mat = parent_mat * xform_mat;
        }
        matrices.insert(name.to_string(), xform_mat);
        inv_bindposes.push(xform_mat.inverse());
        commands.entity(entity).insert(TransformBundle {
            local: transform,
            ..default()
        });
    }

    // Mixamo rig has hips as root. Insert human as root bone.
    let root_str = &sorted_bones[0].clone();
    if root_str.ends_with("Hips") {
        sorted_bones.insert(0, "Root".to_string());
        bone_entities.insert("Root".to_string(), *human);
        let &old_root = bone_entities.get(root_str).unwrap();
        commands.entity(*human).push_children(&[old_root]);
        commands.entity(*human).insert(TransformBundle{
            local: spawn_transform,
            ..default()
        });
        joints.insert(0, *human);
        inv_bindposes.insert(0, Mat4::IDENTITY)
    }

    let inverse_bindposes = inv_bindpose_assets.add(inv_bindposes);
    (SkinnedMesh {
        inverse_bindposes: inverse_bindposes.clone(),
        joints: joints,
    }, sorted_bones)
}

pub(crate) fn set_basemesh_rig_arrays(
    rig: RigType,
    mesh: Mesh,
    rigs: &Res<RigData>,
    vertex_map: &HashMap<u16, Vec<u16>>,
    meshes: &mut ResMut<Assets<Mesh>>,
    sorted_bones: &Vec<String>,
) -> Handle<Mesh> {
    // Build bone index and weight arrays
    let weights_res = rigs.weights.get(&rig).expect("No weights for rig?");
    let mut new_mesh = mesh.clone();
    let vertices = get_vertex_positions(&mesh);
    let mut indices = vec![[0; 4]; vertices.len()];
    let mut weights = vec![[0.0; 4]; vertices.len()];

    for (bone_index, bone_name) in sorted_bones.iter().enumerate() {
        let Some(bone_weights) = weights_res.get(bone_name) else { continue };
        // loop over vertex, bone weight pairs from config
        for (&mh_id, &wt) in bone_weights.iter() {
            // loop over bevy vertex ids mapping to this mh vertex
            for vertex in vertex_map.get(&mh_id).unwrap().iter() {
                // Get the vertex(u16) -> weights(f32) map for this bone
                // get the array at the vertex index to get the [u16;4] array we need to insert into
                let mut indices_vec = indices[*vertex as usize];
                // find smallest weight which is also < wt
                let Some(vec_index) = indices_vec.iter()
                    .enumerate()
                    .filter_map(|(index, &value)| if (value as f32) < wt { Some(index) } else { None })
                    .min() else { continue };
                // Set the bone index in this vector
                indices_vec[vec_index] = bone_index as u16;
                // insert into indices array 
                indices[*vertex as usize] = indices_vec;
                // use the same vertex vec index to set the weights also
                let mut weights_vec = weights[*vertex as usize];
                weights_vec[vec_index] = *bone_weights.get(&mh_id).expect("Failed to get vertex bone weight");
                weights[*vertex as usize] = weights_vec;
            }
        }
    }

    // Make sure weights sum to 1 for each vertex
    for i in 0..weights.iter().len() {
        let wvec = weights[i];
        let norm = wvec[0] + wvec[1] + wvec[2] + wvec[3];
        if norm == 0.0 { panic!("div by 0 ");}
        weights[i] = [
            wvec[0] / norm,
            wvec[1] / norm,
            wvec[2] / norm,
            wvec[3] / norm,
        ];
    }

    new_mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(indices));
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, VertexAttributeValues::Float32x4(weights));
    meshes.add(new_mesh)
}

pub(crate) fn set_asset_rig_arrays(
    rig: RigType,
    mesh: Mesh,
    rigs: &Res<RigData>,
    vertex_map: &HashMap<u16, Vec<u16>>,
    meshes: &mut ResMut<Assets<Mesh>>,
    helper_maps: &Vec<HelperMap>,
    sorted_bones: &Vec<String>,
) -> Handle<Mesh> {
    let weights_res = rigs.weights.get(&rig).expect("No weights for rig?");
    let mut new_mesh = mesh.clone();
    let vertices = get_vertex_positions(&mesh);

    // Build hashmaps to store bone info for each obj vertex id
    let mut indices_map = HashMap::<u16, Vec<usize>>::with_capacity(vertices.len());
    let mut weights_map = HashMap::<u16, Vec<f32>>::with_capacity(vertices.len());

    // loop over obj vertices
    for obj_id in vertex_map.keys() {
        // Create vec in the map for bone indices and weights
        let mut indices_vec = Vec::<usize>::new();
        let mut weights_vec = Vec::<f32>::new();
        // Get helper map for this obj_id
        let helper_map = &helper_maps[*obj_id as usize];
        // loop over bones and find any matching helper indices
        for (bone_index, bone_name) in sorted_bones.iter().enumerate() {
            let Some(bone_weights) = weights_res.get(bone_name) else { continue };
            // For single vertex mapping just apply data for that vertex
            if let Some(v) = helper_map.single_vertex {
                let Some(helper_wt) = bone_weights.get(&v) else { continue; };
                if *helper_wt <= 0.0 { continue };
                indices_vec.push(bone_index);
                weights_vec.push(*helper_wt);
            } else { 
                // Triangle.  Have to weight the base vertices
                let triangle = helper_map.triangle.as_ref().unwrap();
                for (i, mh_id) in triangle.helper_verts.iter().enumerate() {
                    let Some(helper_wt) = bone_weights.get(&mh_id) else { continue; };
                    if *helper_wt <= 0.0 { continue };
                    // Will aggregate below.  For now just allow duplicate entries
                    // e.g. same bone can have weights on all 3 verts of triangle
                    indices_vec.push(bone_index);
                    weights_vec.push(*helper_wt * triangle.helper_weights[i]);
                }
            }
        }
        indices_map.insert(*obj_id, indices_vec);
        weights_map.insert(*obj_id, weights_vec);
    }

    // The acutal arrays that go into the mesh data
    let mut indices = vec![[0; 4]; vertices.len()];
    let mut weights = vec![[0.0; 4]; vertices.len()];

    // Take top 4 weights
    for (obj_id, verts) in vertex_map.iter() {
        for vtx in verts.iter() {
            let vtx_indices = indices_map.get(obj_id).expect("Error getting vertex bone indices");
            let vtx_weights = weights_map.get(obj_id).expect("Error getting vertex bone weights");

            // Deduplicate vertices by summing weights 
            let mut aggregate = HashMap::<usize, f32>::new();
            for (&ind, &wt) in vtx_indices.iter().zip(vtx_weights.iter()) {
                let wtsum = aggregate.entry(ind).or_insert(0.0);
                *wtsum += wt;
            }
            let (mut vtx_indices, mut vtx_weights): (Vec<usize>, Vec<f32>) = aggregate.into_iter().unzip();

            // 4 bone limit for bevy animation
            if vtx_indices.len() > 4 {
                // Sort indices based on the values
                let mut ordering: Vec<usize> = (0..weights.len()).collect();
                ordering.sort_by(|&i, &j| weights[j].partial_cmp(&weights[i]).unwrap());
                // Get top 4
                let top_weights: Vec<usize> = ordering.iter().take(4).copied().collect();
                println!("{}", vtx_weights.len());
                let new_vtx_weights: Vec<f32> = top_weights.iter().map(|&i| vtx_weights[i]).collect();
                let new_vtx_indices: Vec<usize> = top_weights.iter().map(|&i| vtx_indices[i]).collect();
                vtx_indices = new_vtx_indices;
                vtx_weights = new_vtx_weights;
            }
            // Insert bone indices into final array
            let mut indices_array = [0 as u16; 4];
            for (i, &val) in vtx_indices.iter().enumerate() {
                indices_array[i] = val as u16
            };
            indices[*vtx as usize] = indices_array;
            // Normalize weights and insert
            let mut weights_array = [0.0; 4];
            let sum: f32 = vtx_weights.iter().sum();
            for (i, &val) in vtx_weights.iter().enumerate() {
                weights_array[i] = val / sum;
            };
            weights[*vtx as usize] = weights_array;
        }
    }

    new_mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(indices));
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, VertexAttributeValues::Float32x4(weights));
    meshes.add(new_mesh)
}

fn get_bone_transform(
    bone_head: &BoneTransform,
    bone_tail: &BoneTransform,
    vg: &Res<VertexGroups>,
    mh_vertices: &Vec<Vec3>,
) -> Transform {
    let (v1, v2) = get_bone_vertices(bone_head, vg);
    let (v3, v4) = get_bone_vertices(bone_tail, vg);
    let start = (mh_vertices[v1 as usize] + mh_vertices[v2 as usize]) * 0.5;
    let end = (mh_vertices[v3 as usize] + mh_vertices[v4 as usize]) * 0.5;
    Transform::from_translation(start)
        .with_rotation(Quat::from_rotation_arc(Vec3::Y, (end - start).normalize()))
}

fn get_bone_vertices(
    bone: &BoneTransform,
    vg: &Res<VertexGroups>,
) -> (u16, u16) {
    let v1: u16;
    let v2: u16;
    if bone.strategy == "MEAN" {
        v1 = bone.vertex_indices.as_ref().unwrap()[0];
        v2 = bone.vertex_indices.as_ref().unwrap()[1];
    } else if bone.strategy == "CUBE" {
        let joint = bone.cube_name.as_ref().unwrap();
        v1 = vg.0.get(joint).unwrap()[0][0] as u16;
        v2 = vg.0.get(joint).unwrap()[0][1] as u16;
    } else if bone.strategy == "VERTEX" {
        v1 = bone.vertex_index.unwrap();
        v2 = bone.vertex_index.unwrap();
    } else { panic!("Unrecognized bone strategy {}", bone.strategy) }
    (v1, v2)
}