use bevy::{
    prelude::*,
    render::mesh::{
        skinning::{ SkinnedMesh, SkinnedMeshInverseBindposes},
        VertexAttributeValues,
    },
};
use serde::Deserialize;
use serde_json;
use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
};
use crate::BaseMesh;

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
struct BoneWeights {
    weights: HashMap<String, Vec<(u16, f32)>>
 }

#[derive(Deserialize, Debug)]
struct BoneTransform {
    cube_name: Option<String>,
    default_position: Vec3,
    strategy: String,
    vertex_index: Option<u16>,
}

#[derive(Deserialize, Debug)]
struct BoneData {
    head: BoneTransform,
    inherit_scale: String,
    parent: String,
    roll: f32,
    tail: BoneTransform,
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
    weights: HashMap<RigType, BoneWeights>,
    configs: HashMap<RigType, HashMap<String, BoneData>>,
 }

impl FromWorld for RigData {
    fn from_world(world: &mut World) -> Self {
        let mut type_strings = HashMap::<RigType, &str>::new();
        type_strings.insert(RigType::Default, "default");
        type_strings.insert(RigType::Mixamo, "mixamo");
        type_strings.insert(RigType::GameEngine, "game_engine");

        let mut rig_weights = HashMap::<RigType, BoneWeights>::new();
        let mut rig_configs = HashMap::<RigType, HashMap<String, BoneData>>::new();

        for (rig_type, name) in type_strings.iter() {
            let err_msg = "FAILED TO OPEN WEIGHTS FILE : ".to_string() + name;
            let weights_file = File::open("assets/rigs/weights.".to_string() + type_strings.get(rig_type).unwrap() + ".json").expect(&err_msg);
            let weights_reader = BufReader::new(weights_file);
            let err_msg = "FAILED TO READ WEIGHTS JSON : ".to_string() + name;
            let weights: BoneWeights = serde_json::from_reader(weights_reader).expect(&err_msg);
            rig_weights.insert(*rig_type, weights);

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
 #[derive(Component)]
 struct Bone(String);


/*-----------+
 | Functions |
 +-----------*/
pub(crate) fn apply_rig(
    rig: RigType,
    human: &Entity,
    mesh: &mut Mesh,
    base_mesh: &Res<BaseMesh>,
    rigs: &Res<RigData>,
    inv_bindpose_assets: &mut ResMut<Assets<SkinnedMeshInverseBindposes>>,
    commands: &mut Commands,
) {
    let Some(weights_res) = rigs.weights.get(&rig) else { return };
    let Some(config_res) = rigs.configs.get(&rig) else { return };
    let mut indices = Vec::<Vec4>::new();

    // Spawn bone entities
    let mut bone_entities = HashMap::<String, Entity>::new();
    for (name, _bone) in config_res.iter() {
        bone_entities.insert(name.to_string(), commands.spawn(Bone(name.to_string())).id());
    }

    // Arrange bone tree hierarchy
    for (name, bone) in config_res.iter() {
        let &child = bone_entities.get(name).unwrap();
        let Some(&parent) = bone_entities.get(&bone.parent) else { continue };
        commands.entity(parent).push_children(&[child]);
    }

    // Set transforms
    let mut transforms = HashMap::<Entity, Transform>::new();
    for (name, bone) in config_res.iter() {
        let &entity = bone_entities.get(name).unwrap();
        //let Some(&parent) = bone_entities.get(&bone.parent) else { continue };
        let mut transform = Transform::default();
        transform.translation = bone.head.default_position;
        transforms.insert(entity, transform);
        //transform.rotation = Quat::from_arc(parent_up, new_up)
        bone_entities.insert(name.to_string(), commands.spawn(TransformBundle {
            local: transform,
            ..default()
        }).id());
    }

    // Create ordered array of bones/joints and names
    let mut joints = Vec::<Entity>::with_capacity(bone_entities.len());
    let mut bone_names = Vec::<String>::with_capacity(bone_entities.len());
    for (name, bone_entity) in bone_entities.iter() {
        joints.push(*bone_entity);
        bone_names.push(name.to_string());
    }

    // Create joints and inverse bind poses
    let mut inv_bindposes = Vec::<Mat4>::new();
    for joint in joints.iter() {
        let Some(inv_pose) = transforms.get(joint) else { continue };
        inv_bindposes.push(inv_pose.compute_matrix().inverse());
    }
    let inverse_bindposes = inv_bindpose_assets.add(inv_bindposes);

    // Build index and weight arrays
    let Some(VertexAttributeValues::Float32x3(vertices)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("MESH VERTICES FAILURE") };
    //let mut weights = Vec::<Vec4>::try_with_capacity(joints.len());
    let mut indices: Vec<[u16; 4]> = vec![[0; 4]; vertices.len()];
    let mut weights = vec![Vec4::ZERO; vertices.len()];

    for (bone_name, bone_weights) in weights_res.weights.iter() {
        let Some(bone_index) = bone_names.iter().position(|x| x == bone_name) else { continue };
        for (mh_id, wt) in bone_weights.iter() {
            let Some(vertices) = base_mesh.vertex_map.get(&(*mh_id as u16)) else { continue };
            for vertex in vertices.iter() {
                // get the [u16;4] array we need to insert into (array of 4 bone indices)
                let mut indices_vec = indices[*vertex as usize];
                // find the first zero index or use the first index
                let vec_index = indices_vec.iter().position(|i| *i == 0).unwrap_or(0);
                // Set the bone index in this vector
                indices_vec[vec_index] = bone_index as u16;
                // insert into indices array 
                indices[*vertex as usize] = indices_vec;
                // use the same vec index to set the weights also
                let mut weights_vec = weights[*mh_id as usize];
                weights_vec[vec_index] = *wt;
                weights[*vertex as usize] = weights_vec;
            }
        }
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(indices));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    
    commands.entity(*human).insert(SkinnedMesh {
        inverse_bindposes: inverse_bindposes.clone(),
        joints: joints,
    });
}
