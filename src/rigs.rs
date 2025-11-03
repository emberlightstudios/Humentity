use bevy::{
    animation::AnimationTarget, color::palettes::css::RED, ecs::intern::Internable, mesh::VertexAttributeValues, prelude::*
};
use serde::Deserialize;
use serde_json;
use std::{fs::File, io::BufReader};
use ahash::AHashMap;

use crate::{assets::HelperMap, basemesh::VertexGroups, mesh_ops::get_vertex_positions, prelude::*};

/*---------+
 |  Types  |
 +---------*/
#[derive(Eq, PartialEq, Hash, Copy, Clone, Default)]
pub enum RigType {
    #[default]
    Default,
    Mixamo,
    GameEngine,
}


/*---------+
 |  JSON   |
 +---------*/
#[derive(Deserialize, Debug)]
pub struct BoneTransform {
    cube_name: Option<String>,
    strategy: String,
    vertex_indices: Option<Vec<u16>>,
    vertex_index: Option<u16>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct BoneJson {
    //inherit_scale: String,
    //roll: f32,
    pub(crate) parent: String,
    head: BoneTransform,
    tail: BoneTransform,
}

#[derive(Deserialize, Debug)]
struct WeightsFile {
    weights: AHashMap<String, Vec<(u16, f32)>>
}

// Contains an extra layer for some reason.  Usual config is in the bones key
#[derive(Deserialize, Debug)]
struct MixamoConfig {
    bones: AHashMap<String, BoneJson>
}

/*-----------+
 | Resources |
 +-----------*/
/// Raw rig data from makehuman json files
#[derive(Resource)]
pub(crate) struct RigData {
    pub(crate) weights: AHashMap<RigType, AHashMap<&'static str, AHashMap<u16, f32>>>,
    pub(crate) configs: AHashMap<RigType, AHashMap<&'static str, BoneData>>,
}

pub(crate) struct BoneData {
    pub(crate) parent: &'static str,
    head: BoneTransform,
    tail: BoneTransform,
}

impl From<BoneJson> for BoneData {
    fn from(value: BoneJson) -> Self {
        Self {
            head: value.head,
            tail: value.tail,
            parent: NAME_INTERNER.intern(&value.parent).leak(),
        }
    }
}

impl FromWorld for RigData {
    fn from_world(world: &mut World) -> Self {
        let config = world.get_resource::<HumentityPathsConfig>().unwrap();
        let path = config.core_assets_path.clone();
        let mut type_strings = AHashMap::<RigType, &str>::default();
        type_strings.insert(RigType::Default, "default");
        type_strings.insert(RigType::Mixamo, "mixamo");
        type_strings.insert(RigType::GameEngine, "game_engine");

        let mut rig_weights = AHashMap::<RigType, AHashMap<&'static str, AHashMap<u16, f32>>>::default();
        let mut rig_configs = AHashMap::<RigType, AHashMap<&'static str, BoneData>>::default();

        for (rig_type, name) in type_strings.iter() {
            let err_msg = "FAILED TO OPEN WEIGHTS FILE : ".to_string() + name;
            let weights_file = File::open(path.join("rigs/weights.".to_string() + type_strings.get(rig_type).unwrap() + ".json")).expect(&err_msg);
            let weights_reader = BufReader::new(weights_file);
            let err_msg = "FAILED TO READ WEIGHTS JSON : ".to_string() + name;
            let weights: WeightsFile = serde_json::from_reader(weights_reader).expect(&err_msg);
            let mut weights_hashmap = AHashMap::<&'static str, AHashMap<u16, f32>>::default();
            for (bone, wts) in weights.weights.iter() {
                let hashmap: AHashMap<u16, f32> = wts.iter().cloned().collect();
                weights_hashmap.insert(NAME_INTERNER.intern(bone).leak(), hashmap);
            }
            rig_weights.insert(*rig_type, weights_hashmap);

            let err_msg = "FAILED TO OPEN CONFIG FILE : ".to_string() + name;
            let config_file = File::open(path.join("rigs/rig.".to_string() + type_strings.get(rig_type).unwrap() + ".json")).expect(&err_msg);
            let config_reader = BufReader::new(config_file);
            let err_msg = "FAILED TO READ CONFIG JSON : ".to_string() + name;
            if *rig_type == RigType::Mixamo { // Mixamo json structure is slightly different
                let config: MixamoConfig = serde_json::from_reader(config_reader).expect(&err_msg);
                rig_configs.insert(
                    *rig_type,
                    config.bones
                        .into_iter()
                        .map(|(k, x)| (NAME_INTERNER.intern(&k).leak(), x.into()))
                        .collect::<AHashMap<&'static str, BoneData>>()
                );
            } else {
                let config: AHashMap<String, BoneJson> = serde_json::from_reader(config_reader).expect(&err_msg);
                rig_configs.insert(
                    *rig_type,
                    config
                        .into_iter()
                        .map(|(k, x)| (NAME_INTERNER.intern(&k).leak(), x.into()))
                        .collect::<AHashMap<&'static str, BoneData>>()
                );
            }
        }
        RigData {
            weights: rig_weights,
            configs: rig_configs,
        }
    }
}

/*---------+
 | Systems |
 +---------*/
 pub(crate) fn bone_debug_draw(
    query: Query<(&GlobalTransform, &ChildOf), With<AnimationTarget>>,
    transforms: Query<&GlobalTransform, With<AnimationTarget>>,
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
pub(crate) fn get_bone_order(world: &mut World, rig: RigType) -> Vec<&'static str> {
    let mh_config = world.get_resource::<RigData>()
        .expect("Humentitiy not loaded");
    let mh_config = &mh_config.configs[&rig];
    let mut depths = AHashMap::<&'static str, usize>::default();
    for (name, bone) in mh_config.iter() {
        let mut depth = 0;
        let mut parent = &bone.parent;
        while !parent.is_empty() {
            depth += 1;
            parent = &mh_config.get(NAME_INTERNER.intern(&parent).leak()).unwrap().parent;
        }
        depths.insert(name, depth);
    }

    let mut sorted_bones: Vec<(&'static str, usize)> = depths.into_iter().collect();
    sorted_bones.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    sorted_bones.into_iter().map(|(name, _)| name).collect::<Vec<&'static str>>()
}

pub(crate) fn set_basemesh_rig_arrays(
    mut mesh: Mesh,
    basemesh: &BaseMesh,
    meshes: &mut Assets<Mesh>,
    bone_order: &Vec<&'static str>,
    rig_type: RigType,
    rig_data: &RigData,
) -> Handle<Mesh> {
    // Build bone index and weight arrays
    let weights_res = rig_data.weights.get(&rig_type).expect("No weights for rig?");
    let vertices = get_vertex_positions(&mesh);
    let mut indices = vec![[0; 4]; vertices.len()];
    let mut weights = vec![[0.0; 4]; vertices.len()];

    for (bone_index, bone_name) in bone_order.iter().enumerate() {
        let Some(bone_weights) = weights_res.get(bone_name) else { continue };

        // loop over vertex, bone weight pairs from config
        for (vert, mhv) in basemesh.mhid_lookup.iter().enumerate() {

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

    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(indices));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, VertexAttributeValues::Float32x4(weights));

    meshes.add(mesh)
}

pub(crate) fn set_asset_rig_arrays(
    mut mesh: Mesh,
    meshes: &mut Assets<Mesh>,
    rig_data: &RigData,
    mhid_lookup: &Vec<u16>,
    helper_map: &Vec<HelperMap>,
    rig: &CharacterAnimationArchetype,
) -> Handle<Mesh> {
    let weights_res = rig_data.weights.get(&rig.rig_type).expect("No weights for rig?");

    // Build hashmaps to store bone info for each obj vertex id
    let mut indices: Vec<[u16; 4]> = vec![[0; 4]; mhid_lookup.len()];
    let mut weights: Vec<[f32; 4]> = vec![[0.; 4]; mhid_lookup.len()];

    // loop over obj vertices
    for (vert, mhv) in mhid_lookup.iter().enumerate() {
        // Create vec in the map for bone indices and weights
        let mut indices_vec = Vec::<usize>::new();
        let mut weights_vec = Vec::<f32>::new();

        // Get helper map for this obj_id
        let helper_map = &helper_map[*mhv as usize];

        // loop over bones and find any matching helper indices
        for (bone_index, bone_name) in rig.bone_order.iter().enumerate() {
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
        let mut aggregate = AHashMap::<u16, f32>::default();
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

        for i in vtx_indices.len()..4 {
            vtx_indices.push(vtx_indices[i % vtx_indices.len()]);
        }
        for i in vtx_weights.len()..4 {
            vtx_weights.push(vtx_weights[i % vtx_weights.len()]);
        }

        indices[vert] = vtx_indices[..4].try_into().unwrap();
        let mut raw_weights: [f32; 4];
        raw_weights = vtx_weights[..4].try_into().unwrap();

        let sum: f32 = vtx_weights.iter().sum();
        for (i, &val) in vtx_weights.iter().enumerate() {
            raw_weights[i] = val / sum;
        };
        weights[vert] = raw_weights;
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(indices));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, VertexAttributeValues::Float32x4(weights));
    meshes.add(mesh)
}

pub(crate) fn get_model_space_skeleton_transforms(
    bone_order: &Vec<&'static str>,
    helpers: &Vec<Vec3>,
    rig_type: RigType,
    bone_rotations: &AHashMap<&'static str, Quat>,
    vg: &VertexGroups,
    rig_data: &RigData
) -> AHashMap<&'static str, Transform> {
    let mh_config = &rig_data.configs[&rig_type];
    // Compute global transforms
    let mut global_transforms = AHashMap::<&'static str, Transform>::default();
    for &name in bone_order.iter() {
        let bone = &mh_config[name];
        let base_rot = bone_rotations[name];
        global_transforms.insert(name, get_bone_transform(bone, base_rot, vg, helpers));
    }
    global_transforms
}

pub(crate) fn get_local_skeleton_transforms(
    bone_order: &Vec<&'static str>, rig_type: RigType, rig_data: &RigData, global_transforms: &AHashMap<&'static str, Transform>
) -> AHashMap<&'static str, Transform> {
    // Compute local transforms relative to parent
    let mh_config = &rig_data.configs[&rig_type];
    let mut local_transforms = AHashMap::<&'static str, Transform>::default();
    for &name in bone_order.iter() {
        let mut mat = global_transforms[name].to_matrix();
        let mut parent_names = Vec::<&'static str>::new();

        let mut bone = &mh_config[name];
        while !bone.parent.is_empty() {
            parent_names.push(&bone.parent);
            bone = &mh_config[NAME_INTERNER.intern(&bone.parent).leak()];
        }

        // Apply inverse of each parent's local transform
        for &parent_name in parent_names.iter().rev() {
            let parent_local = local_transforms[&parent_name].to_matrix();
            mat = parent_local.inverse() * mat;
        }

        local_transforms.insert(name, Transform::from_matrix(mat));
    }
    local_transforms
}

pub(crate) fn get_bone_transform(
    bone: &BoneData,
    base_rot: Quat,
    vg: &VertexGroups,
    helpers: &Vec<Vec3>,
) -> Transform {
    let start = get_bone_position(&bone.head, vg, helpers);
    let end = get_bone_position(&bone.tail, vg, helpers);

    let orientation = (end - start).normalize();
    let correction = Quat::from_rotation_arc(base_rot * Vec3::Y, orientation);

    Transform::from_translation(start).with_rotation(correction * base_rot)
}

fn get_bone_position(
    bone: &BoneTransform,
    vg: &VertexGroups,
    helpers: &Vec<Vec3>,
) -> Vec3 {
    let v1: u16;
    let v2: u16;
    if bone.strategy == "MEAN" {
        v1 = bone.vertex_indices.as_ref().unwrap()[0];
        v2 = bone.vertex_indices.as_ref().unwrap()[1];
        (helpers[v2 as usize] + helpers[v1 as usize]) / 2.
    } else if bone.strategy == "CUBE" {
        let joint = bone.cube_name.as_ref().unwrap();
        v1 = vg.0.get(joint).unwrap()[0][0] as u16;
        v2 = vg.0.get(joint).unwrap()[0][1] as u16;
        let mut pos = Vec3::ZERO;
        for v in v1..v2+1 {
            pos += helpers[v as usize];
        }
        pos / (v2 - v1 + 1) as f32
    } else if bone.strategy == "VERTEX" {
        helpers[bone.vertex_index.unwrap() as usize]
    } else { unimplemented!("Unrecognized bone strategy {}", bone.strategy) }
}