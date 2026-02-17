use ahash::AHashMap;
use bevy::{
    animation::{AnimatedBy, AnimationTargetId},
    color::palettes::css::RED,
    ecs::intern::Internable,
    mesh::{
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
        VertexAttributeValues,
    },
    prelude::*,
};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::BufReader, sync::Arc};

use crate::{
    assets::HelperMap, basemesh::VertexGroups, prelude::*,
};

#[derive(Eq, PartialEq, Hash, Copy, Clone, Default, Serialize, Deserialize, Debug)]
pub enum RigType {
    #[default]
    Default,
    Mixamo,
    GameEngine,
}

#[derive(Clone, Default, Debug)]
pub(crate) enum BoneTranslationData {
    #[default]
    None,
    Root(Vec3),
    Full(AHashMap<&'static str, Vec3>),
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct SkeletalBone;

/// Adds root motion to XZ-components on translation.  I would add Y but the default rig has a
/// root bone at the hips.  If animation translation tracks are not enabled this will have no effect.
/// This is still experimental and will probably remain broken until official support arrives in Bevy.
#[derive(Component, Default)]
pub struct RootMotion {
    /// I wouldn't use this unless your root bone is at the ground
    pub y_translate: bool,
    pub yaw: bool,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub(crate) struct RootBone;

/// Caches previous transform data for root bone, used in root motion
#[derive(Component, Default)]
pub(crate) struct RootBonePrevious {
    pub(crate) translation: Vec3,
    pub(crate) yaw: f32,
    pub(crate) prev_weights: Vec<f32>,
}

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
    weights: AHashMap<String, Vec<(u16, f32)>>,
}

// Contains an extra layer for some reason.  Usual config is in the bones key
#[derive(Deserialize, Debug)]
struct MixamoConfig {
    bones: AHashMap<String, BoneJson>,
}

pub(crate) type RigWeights = AHashMap<RigType, AHashMap<&'static str, AHashMap<u16, f32>>>;
pub(crate) type RigConfigs = AHashMap<RigType, AHashMap<&'static str, BoneData>>;

/// Raw rig data from makehuman json files
#[derive(Resource)]
pub(crate) struct RigData {
    pub(crate) weights: Arc<RigWeights>,
    pub(crate) configs: Arc<RigConfigs>,
}

pub(crate) struct BoneData {
    pub(crate) parent: &'static str,
    head: BoneTransform,
    tail: BoneTransform,
}

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct SkeletonCaches(AHashMap<RigType, Arc<SkeletonCache>>);

pub(crate) struct SkeletonCache {
    pub(crate) bone_order: Vec<&'static str>,
    /// Model Space
    pub(crate) bone_model_space_rots: AHashMap<&'static str, Quat>,
    /// Bone Local Space
    pub(crate) bone_local_translations: AHashMap<&'static str, Vec3>,
    /// A scene for the skeleton
    pub(crate) scene: Handle<DynamicScene>,
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

        let mut rig_weights =
            AHashMap::<RigType, AHashMap<&'static str, AHashMap<u16, f32>>>::default();
        let mut rig_configs = AHashMap::<RigType, AHashMap<&'static str, BoneData>>::default();

        for (rig_type, name) in type_strings.iter() {
            let err_msg = "FAILED TO OPEN WEIGHTS FILE : ".to_string() + name;
            let weights_file =
                File::open(path.join(
                    "rigs/weights.".to_string() + type_strings.get(rig_type).unwrap() + ".json",
                ))
                .expect(&err_msg);
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
            let config_file = File::open(
                path.join("rigs/rig.".to_string() + type_strings.get(rig_type).unwrap() + ".json"),
            )
            .expect(&err_msg);
            let config_reader = BufReader::new(config_file);
            let err_msg = "FAILED TO READ CONFIG JSON : ".to_string() + name;
            if *rig_type == RigType::Mixamo {
                // Mixamo json structure is slightly different
                let config: MixamoConfig = serde_json::from_reader(config_reader).expect(&err_msg);
                rig_configs.insert(
                    *rig_type,
                    config
                        .bones
                        .into_iter()
                        .map(|(k, x)| (NAME_INTERNER.intern(&k).leak(), x.into()))
                        .collect::<AHashMap<&'static str, BoneData>>(),
                );
            } else {
                let config: AHashMap<String, BoneJson> =
                    serde_json::from_reader(config_reader).expect(&err_msg);
                rig_configs.insert(
                    *rig_type,
                    config
                        .into_iter()
                        .map(|(k, x)| (NAME_INTERNER.intern(&k).leak(), x.into()))
                        .collect::<AHashMap<&'static str, BoneData>>(),
                );
            }
        }
        RigData {
            weights: Arc::new(rig_weights),
            configs: Arc::new(rig_configs),
        }
    }
}

pub(crate) fn bone_debug_draw(
    query: Query<(&GlobalTransform, &ChildOf), With<AnimationTargetId>>,
    transforms: Query<&GlobalTransform, With<AnimationTargetId>>,
    mut gizmos: Gizmos,
) {
    query.iter().for_each(|(transform, child_of)| {
        let start = transform.translation();
        if let Ok(end) = transforms.get(child_of.parent()) {
            gizmos.line(start, end.translation(), RED);
        }
    })
}

pub(crate) fn get_bone_order(world: &mut World, rig: RigType) -> Vec<&'static str> {
    let mh_config = world
        .get_resource::<RigData>()
        .expect("Humentitiy not loaded");
    let mh_config = &mh_config.configs[&rig];
    let mut depths = AHashMap::<&'static str, usize>::default();
    for (name, bone) in mh_config.iter() {
        let mut depth = 0;
        let mut parent = &bone.parent;
        while !parent.is_empty() {
            depth += 1;
            parent = &mh_config
                .get(NAME_INTERNER.intern(parent).leak())
                .unwrap()
                .parent;
        }
        depths.insert(name, depth);
    }

    let mut sorted_bones: Vec<(&'static str, usize)> = depths.into_iter().collect();
    sorted_bones.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
    sorted_bones
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<&'static str>>()
}

pub(crate) fn set_asset_rig_arrays(
    mesh: &mut Mesh,
    rig_weights: &Arc<RigWeights>,
    mhid_lookup: &[u16],
    helper_map: &[HelperMap],
    rig: &CharacterAnimationArchetype,
    cache: &SkeletonCache,
) {
    let weights_res = rig_weights
        .get(&rig.rig_type)
        .expect("No weights for rig?");

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
        for (bone_index, bone_name) in cache.bone_order.iter().enumerate() {
            let Some(bone_weights) = weights_res.get(bone_name) else {
                continue;
            };

            if let Some(v) = helper_map.single_vertex {
                // For single vertex mapping just apply data for that vertex
                let Some(helper_wt) = bone_weights.get(&v) else {
                    continue;
                };
                if *helper_wt <= 0.0 {
                    continue;
                };
                indices_vec.push(bone_index);
                weights_vec.push(*helper_wt);
            } else {
                // Triangle.  Have to weight the base vertices
                let triangle = helper_map.triangle.as_ref().unwrap();
                for (i, mh_id) in triangle.helper_verts.iter().enumerate() {
                    let Some(helper_wt) = bone_weights.get(mh_id) else {
                        continue;
                    };
                    if *helper_wt <= 0.0 {
                        continue;
                    };

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
        let (mut vtx_indices, mut vtx_weights): (Vec<u16>, Vec<f32>) =
            aggregate.into_iter().unzip();

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
        }
        weights[vert] = raw_weights;
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

/// Spawns bone entities and sets up the hierarchy
pub(crate) fn build_human_rig_scene(
    helpers: &[Vec3],
    rig: RigType,
    bone_rotations: &AHashMap<&'static str, Quat>,
    bone_order: &Vec<&'static str>,
    world: &mut World,
) -> Handle<DynamicScene> {
    let mh_config = &world.resource::<RigData>().configs[&rig];

    // Set up some convenient data structures for tracking joints/bones and entities
    let mut bone_entities = AHashMap::<&'static str, Entity>::default();

    // Start scene world with rig entity
    let registry = world.resource::<AppTypeRegistry>();
    let mut scene_world = World::new();
    scene_world.insert_resource(registry.clone());

    let rig_entity = scene_world
        .spawn((
            AnimationPlayer::default(),
            Name::new("Human.rig"),
            Transform::IDENTITY,
        ))
        .id();

    // Spawn all bone entities
    for (i, &name) in bone_order.iter().enumerate() {
        let mut path = Vec::<Name>::new();
        path.push(Name::from(name));
        let mut bone = &mh_config[&name];

        while !bone.parent.is_empty() {
            let parent = NAME_INTERNER.intern(bone.parent).leak();
            path.push(Name::new(parent));
            bone = &mh_config[&parent];
        }
        path.push(Name::new("Human.rig"));

        let entity = scene_world
            .spawn((
                Name::new(name),
                AnimationTargetId::from_names(path.iter().rev()),
                AnimatedBy(rig_entity),
                SkeletalBone,
            ))
            .id();

        if i == 0 {
            scene_world.entity_mut(entity).insert(RootBone);
        }
        bone_entities.insert(name, entity);
    }

    // Wire up parent-child relationships
    for &name in bone_order.iter() {
        let &child = bone_entities.get(&name).unwrap();
        if let Some(parent_name) = mh_config.get(&name).map(|b| b.parent.to_string()) 
            && !parent_name.is_empty() && let Some(&parent) = bone_entities.get(NAME_INTERNER.intern(&parent_name).leak())
        {
            scene_world.entity_mut(child).insert(ChildOf(parent));
        }
    }

    // Attach root(s) to rig entity
    for &name in bone_order.iter() {
        if let Some(bone) = mh_config.get(&name) && bone.parent.is_empty() {
            scene_world
                .entity_mut(bone_entities[&name])
                .insert(ChildOf(rig_entity));
        }
    }

    let vg = world.resource::<VertexGroups>();
    let rig_data = world.resource::<RigData>();
    let global_transforms =
        get_model_space_skeleton_transforms(bone_order, helpers, rig, bone_rotations, vg, rig_data);
    let local_transforms =
        get_local_skeleton_transforms(bone_order, rig, rig_data, &global_transforms);

    // compute inverse bindposes
    let mut inverse_bindposes = Vec::with_capacity(bone_order.len());
    for &name in bone_order.iter() {
        let entity = bone_entities[&name];
        let local = local_transforms[&name];
        let global = global_transforms[&name];

        scene_world.entity_mut(entity).insert(local);

        // Inverse bindpose is always from global transform
        inverse_bindposes.push(global.to_matrix().inverse());
    }

    // Setup SkinnedMesh component and AnimationPlayer
    let mut inverse_bindpose_assets = world.resource_mut::<Assets<SkinnedMeshInverseBindposes>>();
    let inverse_bindposes = inverse_bindpose_assets.add(inverse_bindposes);
    // only need to return bindposes.  entities will have to be mapped manually after spawning scene
    let joint_entities = bone_order
        .iter()
        .map(|n| bone_entities[n])
        .collect::<Vec<_>>();

    let skinned_mesh = SkinnedMesh {
        inverse_bindposes,
        joints: joint_entities,
    };
    scene_world.entity_mut(rig_entity).insert(skinned_mesh);

    let mut ds = world.resource_mut::<Assets<DynamicScene>>();
    ds.add(DynamicScene::from_world(&scene_world))
}

pub(crate) fn get_model_space_skeleton_transforms(
    bone_order: &Vec<&'static str>,
    helpers: &[Vec3],
    rig_type: RigType,
    bone_rotations: &AHashMap<&'static str, Quat>,
    vg: &VertexGroups,
    rig_data: &RigData,
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
    bone_order: &Vec<&'static str>,
    rig_type: RigType,
    rig_data: &RigData,
    global_transforms: &AHashMap<&'static str, Transform>,
) -> AHashMap<&'static str, Transform> {
    // Compute local transforms relative to parent
    let mh_config = &rig_data.configs[&rig_type];
    let mut local_transforms = AHashMap::<&'static str, Transform>::default();
    for &name in bone_order.iter() {
        let mut mat = global_transforms[name].to_matrix();
        let mut parent_names = Vec::<&'static str>::new();

        let mut bone = &mh_config[name];
        while !bone.parent.is_empty() {
            parent_names.push(bone.parent);
            bone = &mh_config[NAME_INTERNER.intern(bone.parent).leak()];
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
    helpers: &[Vec3],
) -> Transform {
    let start = get_bone_position(&bone.head, vg, helpers);
    let end = get_bone_position(&bone.tail, vg, helpers);

    let orientation = (end - start).normalize();
    let correction = Quat::from_rotation_arc(base_rot * Vec3::Y, orientation);

    Transform::from_translation(start).with_rotation(correction * base_rot)
}

fn get_bone_position(bone: &BoneTransform, vg: &VertexGroups, helpers: &[Vec3]) -> Vec3 {
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
        for v in v1..v2 + 1 {
            pos += helpers[v as usize];
        }
        pos / (v2 - v1 + 1) as f32
    } else if bone.strategy == "VERTEX" {
        helpers[bone.vertex_index.unwrap() as usize]
    } else {
        unimplemented!("Unrecognized bone strategy {}", bone.strategy)
    }
}
