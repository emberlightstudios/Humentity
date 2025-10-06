use bevy::{
    color::palettes::css::RED, mesh::{skinning::{SkinnedMesh, SkinnedMeshInverseBindposes}, VertexAttributeValues}, prelude::* 
};
use serde::Deserialize;
use serde_json;
use std::{fs::File, io::BufReader};
use fxhash::FxHashMap;

use crate::{assets::HelperMap, basemesh::VertexGroups, mesh_ops::get_vertex_positions, prelude::*};

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
    roll: f32,
    tail: BoneTransform,
}

#[derive(Deserialize, Debug)]
struct WeightsFile {
    weights: FxHashMap<String, Vec<(u16, f32)>>
}

// Contains an extra layer for some reason.  Usual config is in the bones key
#[derive(Deserialize, Debug)]
struct MixamoRigConfig {
    bones: FxHashMap<String, BoneData>
}

/*-----------+
 | Resources |
 +-----------*/
#[derive(Resource)]
pub struct RigData {
    weights: FxHashMap<RigType, FxHashMap<String, FxHashMap<u16, f32>>>,
    configs: FxHashMap<RigType, FxHashMap<String, BoneData>>,
}

impl FromWorld for RigData {
    fn from_world(world: &mut World) -> Self {
        let config = world.get_resource::<HumentityGlobalConfig>().unwrap();
        let path = config.core_assets_path.clone();
        let mut type_strings = FxHashMap::<RigType, &str>::default();
        type_strings.insert(RigType::Default, "default");
        type_strings.insert(RigType::Mixamo, "mixamo");
        type_strings.insert(RigType::GameEngine, "game_engine");

        let mut rig_weights = FxHashMap::<RigType, FxHashMap<String, FxHashMap<u16, f32>>>::default();
        let mut rig_configs = FxHashMap::<RigType, FxHashMap<String, BoneData>>::default();

        for (rig_type, name) in type_strings.iter() {
            let err_msg = "FAILED TO OPEN WEIGHTS FILE : ".to_string() + name;
            let weights_file = File::open(path.join("rigs/weights.".to_string() + type_strings.get(rig_type).unwrap() + ".json")).expect(&err_msg);
            let weights_reader = BufReader::new(weights_file);
            let err_msg = "FAILED TO READ WEIGHTS JSON : ".to_string() + name;
            let weights: WeightsFile = serde_json::from_reader(weights_reader).expect(&err_msg);
            let mut weights_hashmap = FxHashMap::<String, FxHashMap<u16, f32>>::default();
            for (bone, wts) in weights.weights.iter() {
                let hashmap: FxHashMap<u16, f32> = wts.iter().cloned().collect();
                weights_hashmap.insert(bone.to_string(), hashmap);
            }
            rig_weights.insert(*rig_type, weights_hashmap);

            let err_msg = "FAILED TO OPEN CONFIG FILE : ".to_string() + name;
            let config_file = File::open(path.join("rigs/rig.".to_string() + type_strings.get(rig_type).unwrap() + ".json")).expect(&err_msg);
            let config_reader = BufReader::new(config_file);
            let err_msg = "FAILED TO READ CONFIG JSON : ".to_string() + name;
            if *rig_type == RigType::Mixamo {
                let config: MixamoRigConfig = serde_json::from_reader(config_reader).expect(&err_msg);
                rig_configs.insert(*rig_type, config.bones);
            } else {
                let config: FxHashMap<String, BoneData> = serde_json::from_reader(config_reader).expect(&err_msg);
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
 #[derive(Component)]
 pub struct Bone;

 #[derive(Component)]
 pub struct Rig;

/*---------+
 | Systems |
 +---------*/
 pub(crate) fn bone_debug_draw(
    query: Query<(&GlobalTransform, &ChildOf), With<Bone>>,
    transforms: Query<&GlobalTransform, With<Bone>>,
    mut gizmos: Gizmos,
 ) {
    query.iter().for_each(|(transform, child)| {
        let start = transform.translation();
        if let Ok(end) = transforms.get(child.parent()) {
            gizmos.line(start, end.translation(), RED);
        }
    })
 }
    
/*-----------+
 | Functions |
 +-----------*/

/// Returns a deterministic ordering of bones (root first, children after)
fn get_sorted_bones(config: &FxHashMap<String, BoneData>) -> Vec<String> {
    let mut depths = FxHashMap::<&String, usize>::default();

    for (name, bone) in config.iter() {
        let mut depth = 0;
        let mut parent = &bone.parent;
        while !parent.is_empty() {
            depth += 1;
            parent = &config.get(parent).unwrap().parent;
        }
        depths.insert(name, depth);
    }

    let mut sorted: Vec<(&String, usize)> = depths.into_iter().collect();
    sorted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
    sorted.into_iter().map(|(name, _)| name.clone()).collect()
}

/// Spawns bone entities and sets up the hierarchy
fn build_rig_hierarchy(
    commands: &mut Commands,
    rig_entity: Entity,
    config: &FxHashMap<String, BoneData>,
    sorted_bones: &[String],
) -> (FxHashMap<String, Entity>, Vec<Entity>) {
    let mut bone_entities = FxHashMap::<String, Entity>::default();

    // Spawn all bones
    for name in sorted_bones {
        let entity = commands.spawn((Bone, Name::new(name.clone()))).id();
        bone_entities.insert(name.clone(), entity);
    }

    // Wire up parent-child relationships
    for name in sorted_bones {
        let &child = bone_entities.get(name).unwrap();
        if let Some(parent_name) = config.get(name).map(|b| b.parent.as_str()) {
            if !parent_name.is_empty() {
                if let Some(&parent) = bone_entities.get(parent_name) {
                    commands.entity(parent).add_child(child);
                }
            }
        }
    }

    // Attach root(s) to rig entity
    for name in sorted_bones {
        if let Some(bone) = config.get(name) {
            if bone.parent.is_empty() {
                commands.entity(rig_entity).add_child(bone_entities[name]);
            }
        }
    }

    let joint_entities = sorted_bones
        .iter()
        .map(|n| bone_entities[n])
        .collect::<Vec<_>>();

    (bone_entities, joint_entities)
}

/// Computes global transforms and inverse bind poses, inserts Transform into entities
fn compute_bindposes(
    commands: &mut Commands,
    config: &FxHashMap<String, BoneData>,
    sorted_bones: &[String],
    bone_entities: &FxHashMap<String, Entity>,
    vg: &Res<VertexGroups>,
    helpers: &Vec<Vec3>,
) -> Vec<Mat4> {
    use bevy::prelude::*;

    // Step 1: compute global transforms
    let mut global_transforms = FxHashMap::<&String, Transform>::default();
    for name in sorted_bones {
        let bone = &config[name];
        global_transforms.insert(name, get_bone_transform(bone, vg, helpers));
    }

    // Step 2: compute local transforms relative to parent
    let mut local_transforms = FxHashMap::<&String, Transform>::default();
    for name in sorted_bones {
        let mut mat = global_transforms[name].to_matrix();
        let mut parent_names = Vec::new();

        let mut bone = &config[name];
        while !bone.parent.is_empty() {
            parent_names.push(&bone.parent);
            bone = &config[&bone.parent];
        }

        // Apply inverse of each parent's local transform
        for parent_name in parent_names.iter().rev() {
            let parent_local = local_transforms[parent_name].to_matrix();
            mat = parent_local.inverse() * mat;
        }

        local_transforms.insert(name, Transform::from_matrix(mat));
    }

    // Step 3: insert local transforms and compute inverse bindposes
    let mut inv_bindposes = Vec::with_capacity(sorted_bones.len());
    for name in sorted_bones {
        let entity = bone_entities[name];
        let local = local_transforms[name];
        let global = global_transforms[name];

        commands.entity(entity).insert(local);

        // Inverse bindpose is always from global transform
        inv_bindposes.push(global.to_matrix().inverse());
    }

    inv_bindposes
}


/// Orchestrates rig creation
pub(crate) fn build_rig(
    human: &Entity,
    rig: RigType,
    rigs: &Res<RigData>,
    inv_bindpose_assets: &mut ResMut<Assets<SkinnedMeshInverseBindposes>>,
    commands: &mut Commands,
    vg: &Res<VertexGroups>,
    helpers: &Vec<Vec3>,
    base_transform: &Transform,
) -> (SkinnedMesh, Vec<String>) {
    let config = rigs.configs.get(&rig).unwrap();

    // Spawn Rig entity under human
    let rig_entity = commands.spawn((Rig, base_transform.clone())).id();
    commands.entity(*human).add_child(rig_entity);

    // Step 1: get deterministic bone order
    let sorted_bones = get_sorted_bones(config);

    // Step 2: spawn hierarchy
    let (bone_entities, joint_entities) =
        build_rig_hierarchy(commands, rig_entity, config, &sorted_bones);

    // Step 3: compute bindposes
    let inv_bindposes =
        compute_bindposes(commands, config, &sorted_bones, &bone_entities, vg, helpers);

    let inverse_bindposes = inv_bindpose_assets.add(inv_bindposes);

    (SkinnedMesh {
        inverse_bindposes,
        joints: joint_entities,
    }, sorted_bones)
}


pub(crate) fn set_basemesh_rig_arrays(
    rig: RigType,
    mesh: Mesh,
    rigs: &Res<RigData>,
    mhid_lookup: &Vec<u16>,
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
        for (vert, mhv) in mhid_lookup.iter().enumerate() {

            let Some(&wt) = bone_weights.get(mhv) else { continue };

            // Get the vertex(u16) -> weights(f32) map for this bone
            // get the array at the vertex index to get the [u16;4] array we need to insert into
            let mut indices_vec = indices[vert];

            // find smallest weight which is also < wt
            let Some(vec_index) = indices_vec.iter()
                .enumerate()
                .filter_map(|(index, &value)| if (value as f32) < wt { Some(index) } else { None })
                .min() else { continue };

            // Set the bone index in this vector
            indices_vec[vec_index] = bone_index as u16;

            // insert into indices array 
            indices[vert as usize] = indices_vec;

            // use the same vertex vec index to set the weights also
            let mut weights_vec = weights[vert as usize];
            weights_vec[vec_index] = *bone_weights.get(mhv).expect("Failed to get vertex bone weight");
            weights[vert as usize] = weights_vec;
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
    mhid_lookup: &Vec<u16>,
    meshes: &mut ResMut<Assets<Mesh>>,
    helper_maps: &Vec<HelperMap>,
    sorted_bones: &Vec<String>,
) -> Handle<Mesh> {
    let weights_res = rigs.weights.get(&rig).expect("No weights for rig?");
    let mut new_mesh = mesh.clone();

    // Build hashmaps to store bone info for each obj vertex id
    let mut indices = Vec::<[u16; 4]>::default();
    let mut weights = Vec::<[f32; 4]>::default();

    // loop over obj vertices
    for (vert, mhv) in mhid_lookup.iter().enumerate() {
        // Create vec in the map for bone indices and weights
        let mut indices_vec = Vec::<usize>::new();
        let mut weights_vec = Vec::<f32>::new();

        // Get helper map for this obj_id
        let helper_map = &helper_maps[*mhv as usize];

        // loop over bones and find any matching helper indices
        for (bone_index, bone_name) in sorted_bones.iter().enumerate() {
            let Some(bone_weights) = weights_res.get(bone_name) else { continue };

            if let Some(v) = helper_map.single_vertex {
                // For single vertex mapping just apply data for that vertex
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

        // Deduplicate vertices by summing weights 
        let mut aggregate = FxHashMap::<u16, f32>::default();
        for (&ind, &wt) in indices_vec.iter().zip(weights_vec.iter()) {
            let wtsum = aggregate.entry(ind as u16).or_insert(0.0);
            *wtsum += wt;
        }
        let (mut vtx_indices, mut vtx_weights): (Vec<u16>, Vec<f32>) = aggregate.into_iter().unzip();

        // 4 bone limit for bevy animation. Take top 4 weights
        if vtx_indices.len() > 4 {

            // Sort vec indices based on the weights
            let mut ordering: Vec<usize> = (0..vtx_weights.len()).collect();
            ordering.sort_by(|&i, &j| vtx_weights[j].partial_cmp(&vtx_weights[i]).unwrap());

            // Get vec indices of top 4 weights
            let top_weights: Vec<usize> = ordering.iter().take(4).copied().collect();

            // set back into the original vecs
            let new_vtx_weights: Vec<f32> = top_weights.iter().map(|&i| vtx_weights[i]).collect();
            let new_vtx_indices: Vec<u16> = top_weights.iter().map(|&i| vtx_indices[i]).collect();
            vtx_indices = new_vtx_indices;
            vtx_weights = new_vtx_weights;
        }

        indices[vert] = vtx_indices[..4].try_into().unwrap();

        let mut raw_weights: [f32; 4] = vtx_weights[..4].try_into().unwrap();
        let sum: f32 = vtx_weights.iter().sum();
        for (i, &val) in vtx_weights.iter().enumerate() {
            raw_weights[i] = val / sum;
        };
        weights[vert] = raw_weights;
    }

    new_mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(indices));
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, VertexAttributeValues::Float32x4(weights));
    meshes.add(new_mesh)
}

fn get_bone_transform(
    bone: &BoneData,
    vg: &Res<VertexGroups>,
    mh_vertices: &Vec<Vec3>,
) -> Transform {
    let (v1, v2) = get_bone_vertices(&bone.head, vg);
    let (v3, v4) = get_bone_vertices(&bone.tail, vg);
    let start = (mh_vertices[v1 as usize] + mh_vertices[v2 as usize]) * 0.5;
    let end = (mh_vertices[v3 as usize] + mh_vertices[v4 as usize]) * 0.5;
    let mut transform = Transform::from_translation(start)
        .with_rotation(Quat::from_rotation_arc(Vec3::Y, (end - start).normalize()));
    transform.rotate_local_y(bone.roll);
    transform
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