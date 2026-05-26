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
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::{
    basemesh::VertexGroups,
    loaders::{MhcloVertexMap, ReferenceRigAsset, RigConfigAsset, RigWeightsAsset},
    prelude::*,
};

#[derive(Eq, PartialEq, Hash, Copy, Clone, Default, Serialize, Deserialize, Debug)]
pub enum RigType {
    #[default]
    Default,
    Mixamo,
    GameEngine,
}

#[derive(Clone, Default, Debug)]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub(crate) struct RootBonePrevious {
    pub(crate) translation: Vec3,
    pub(crate) yaw: f32,
    pub(crate) prev_weights: Vec<f32>,
}

#[derive(Clone)]
pub struct RigSpec {
    pub(crate) weights: Arc<RigWeightsAsset>,
    pub(crate) config: Arc<RigConfigAsset>,
    pub(crate) reference_rig: Arc<ReferenceRigAsset>,
    pub(crate) scene: Option<Handle<DynamicScene>>,
}

#[derive(Resource)]
pub struct RigData {
    rigs: AHashMap<RigType, RigSpec>,
    #[allow(dead_code)]
    config_handle: Handle<RigConfigAsset>,
    #[allow(dead_code)]
    weights_handle: Handle<RigWeightsAsset>,
    #[allow(dead_code)]
    ref_rig_handle: Handle<ReferenceRigAsset>,
}

impl RigData {
    pub fn new(
        asset_server: &AssetServer,
        config_path: &'static str,
        weights_path: &'static str,
        ref_rig_path: &'static str,
    ) -> Self {
        Self {
            rigs: default(),
            config_handle: asset_server.load::<RigConfigAsset>(config_path),
            weights_handle: asset_server.load::<RigWeightsAsset>(weights_path),
            ref_rig_handle: asset_server.load::<ReferenceRigAsset>(ref_rig_path),
        }
    }
}

impl Deref for RigData {
    type Target = AHashMap<RigType, RigSpec>;
    fn deref(&self) -> &Self::Target {
        &self.rigs
    }
}

impl DerefMut for RigData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.rigs
    }
}

impl RigSpec {
    pub fn bone_index(&self, bone_name: &str) -> Option<usize> {
        self.reference_rig
            .bone_name_to_index
            .get(bone_name)
            .copied()
    }
}

impl std::ops::Index<&RigType> for RigData {
    type Output = RigSpec;
    fn index(&self, index: &RigType) -> &Self::Output {
        self.rigs.index(index)
    }
}

/// Syncs rig assets when all 3 assets are available. Scene building deferred.
pub(crate) fn sync_and_build_rig_data(
    mut config_events: MessageReader<AssetEvent<RigConfigAsset>>,
    mut weights_events: MessageReader<AssetEvent<RigWeightsAsset>>,
    mut reference_rig_events: MessageReader<AssetEvent<ReferenceRigAsset>>,
    mut rig_data: ResMut<RigData>,
    config_assets: Res<Assets<RigConfigAsset>>,
    weights_assets: Res<Assets<RigWeightsAsset>>,
    reference_rig_assets: Res<Assets<ReferenceRigAsset>>,
) {
    if config_events.is_empty() && weights_events.is_empty() && reference_rig_events.is_empty() {
        return;
    }

    config_events.read();
    weights_events.read();
    reference_rig_events.read();

    // Collect available rigs from each asset type
    let wts: Vec<_> = weights_assets.iter().map(|(_, wt)| wt.rig).collect();
    let ref_rigs: Vec<_> = reference_rig_assets.iter().map(|(_, r)| r.rig).collect();

    // Find rigs where all 3 assets are available
    let to_load: Vec<_> = config_assets
        .iter()
        .map(|(_, cfg)| cfg.rig)
        .filter(|rig| wts.contains(rig))
        .filter(|rig| ref_rigs.contains(rig))
        .filter(|rig| !rig_data.contains_key(rig))
        .collect();

    // Insert rigs - scene will be built lazily
    for rig in to_load {
        let (_, config) = config_assets
            .iter()
            .find(|(_, a)| a.rig == rig)
            .unwrap_or_else(|| panic!("Config asset for rig {rig:?} not found"));
        let (_, weights) = weights_assets
            .iter()
            .find(|(_, a)| a.rig == rig)
            .unwrap_or_else(|| panic!("Weights asset for rig {rig:?} not found"));
        let (_, reference_rig) = reference_rig_assets
            .iter()
            .find(|(_, a)| a.rig == rig)
            .unwrap_or_else(|| panic!("Reference rig asset for rig {rig:?} not found"));

        rig_data.insert(
            rig,
            RigSpec {
                weights: Arc::new(weights.clone()),
                config: Arc::new(config.clone()),
                reference_rig: Arc::new(reference_rig.clone()),
                scene: None,
            },
        );
    }
}

use bevy::ecs::system::SystemState;

/// Builds skeleton scenes for rigs that have all assets but no scene yet.
pub(crate) fn build_rig_scenes(world: &mut World) {
    let rigs_to_build: Vec<_> = {
        let mut state: SystemState<ResMut<RigData>> = SystemState::new(world);
        let rig_data = state.get_mut(world);
        rig_data
            .iter()
            .filter(|(_, spec)| spec.scene.is_none())
            .map(|(rig_type, spec)| (*rig_type, Arc::clone(&spec.reference_rig)))
            .collect()
    };

    for (rig_type, reference_rig) in rigs_to_build {
        let scene = build_skeleton_scene(&reference_rig, world);

        let mut state: SystemState<ResMut<RigData>> = SystemState::new(world);
        let mut rig_data = state.get_mut(world);
        rig_data.get_mut(&rig_type).unwrap().scene = Some(scene);
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

pub(crate) fn set_asset_rig_arrays(
    mesh: &mut Mesh,
    mhid_lookup: &[u16],
    helper_map: &[MhcloVertexMap],
    rig_spec: &RigSpec,
) {
    let bone_names = &rig_spec.reference_rig.bone_names;
    let rig_weights = &rig_spec.weights.weights;
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
                        && helper_wt > 0.0 {
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
                            && helper_wt > 0.0 {
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

/// Builds skeleton scene from reference rig.
pub(crate) fn build_skeleton_scene(
    reference_rig: &Arc<ReferenceRigAsset>,
    world: &mut World,
) -> Handle<DynamicScene> {
    let bone_order = &reference_rig.bone_names;
    let ref_bone_parents = &reference_rig.bone_parents;
    let ref_local_bindpose = &reference_rig.local_bindpose;

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
            // Not really a "bone" per se but useful when finding local bone transforms
            SkeletalBone,
        ))
        .id();

    // Spawn all bone entities
    for (i, &name) in bone_order.iter().enumerate() {
        let mut path = Vec::<Name>::new();
        path.push(Name::from(name));

        let mut current_parent = ref_bone_parents.get(name).cloned().unwrap_or_default();
        while !current_parent.is_empty() {
            path.push(Name::new(current_parent.clone()));
            let next = ref_bone_parents
                .get(&NAME_INTERNER.intern(&current_parent).leak())
                .cloned()
                .unwrap_or_default();
            current_parent = next;
        }

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

    // Wire up parent-child relationships.
    for &name in bone_order.iter() {
        let &child = bone_entities.get(&name).unwrap();
        let parent_name = ref_bone_parents.get(name).cloned().unwrap_or_default();
        if !parent_name.is_empty()
            && let Some(&parent) = bone_entities.get(NAME_INTERNER.intern(&parent_name).leak()) {
                scene_world.entity_mut(parent).add_child(child);
            }
    }

    // Attach root(s) to rig entity
    for &name in bone_order.iter() {
        let is_root = ref_bone_parents
            .get(name)
            .map(|p| p == &"Human.rig".to_string())
            .unwrap_or(false);
        if is_root {
            scene_world.entity_mut(rig_entity).add_child(bone_entities[&name]);
        }
    }

    let (global_transforms, local_transforms) = {
        let local = ref_local_bindpose.clone();
        let model_space = reference_rig.model_space_bindpose.clone();

        (model_space, local)
    };

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
    vg: &VertexGroups,
    rig_data: &RigData,
) -> AHashMap<&'static str, Transform> {
    let mh_config = &rig_data
        .get(&rig_type)
        .unwrap_or_else(|| panic!("No rig data loaded for {rig_type:?}"))
        .config;
    // Compute global transforms from basemesh + roll in config (no GLB reference)
    let mut global_transforms = AHashMap::<&'static str, Transform>::default();
    for &name in bone_order.iter() {
        let bone = &mh_config.bones[name];
        global_transforms.insert(name, get_bone_transform(bone, vg, helpers));
    }
    global_transforms
}

#[allow(dead_code)]
pub(crate) fn get_local_skeleton_transforms(
    bone_order: &Vec<&'static str>,
    rig_type: RigType,
    rig_data: &RigData,
    global_transforms: &AHashMap<&'static str, Transform>,
) -> AHashMap<&'static str, Transform> {
    // Compute local transforms relative to parent
    let mh_config = &rig_data
        .get(&rig_type)
        .unwrap_or_else(|| panic!("No rig data loaded for {rig_type:?}"))
        .config;
    let mut local_transforms = AHashMap::<&'static str, Transform>::default();
    for &name in bone_order.iter() {
        let mut mat = global_transforms[name].to_matrix();
        let mut parent_names = Vec::<&'static str>::new();

        let mut bone = &mh_config.bones[name];
        while !bone.parent.is_empty() {
            parent_names.push(&bone.parent);
            bone = &mh_config.bones[NAME_INTERNER.intern(&bone.parent).leak()];
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

#[allow(dead_code)]
pub(crate) fn get_bone_order(rig: RigType, world: &World) -> Vec<&'static str> {
    let rig_data = world.resource::<RigData>();
    let mh_config = &rig_data
        .get(&rig)
        .unwrap_or_else(|| panic!("No rig data loaded for {rig:?}"))
        .config;
    let mut depths = AHashMap::<&'static str, usize>::default();
    for (name, bone) in mh_config.bones.iter() {
        let mut depth = 0;
        let mut parent = &bone.parent;
        while !parent.is_empty() {
            depth += 1;
            parent = &mh_config
                .bones
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

pub(crate) fn get_bone_transform(
    bone: &BoneJsonConfig,
    vg: &VertexGroups,
    helpers: &[Vec3],
) -> Transform {
    let start = get_bone_position(&bone.head, vg, helpers);
    let end = get_bone_position(&bone.tail, vg, helpers);

    let orientation = (end - start).normalize();
    // Align bone axis (Y in local) with head->tail, then apply roll around that axis (Blender convention)
    let r_align = Quat::from_rotation_arc(Vec3::Y, orientation);
    let base_rot = r_align * Quat::from_rotation_y(bone.roll);

    Transform::from_translation(start).with_rotation(base_rot)
}

fn get_bone_position(bone: &BoneTransformSpec, vg: &VertexGroups, helpers: &[Vec3]) -> Vec3 {
    let v1: u16;
    let v2: u16;
    if bone.strategy == "MEAN" {
        v1 = bone.vertex_indices.as_ref().unwrap()[0];
        v2 = bone.vertex_indices.as_ref().unwrap()[1];
        (helpers[v2 as usize] + helpers[v1 as usize]) / 2.
    } else if bone.strategy == "CUBE" {
        let joint = bone.cube_name.as_ref().unwrap();
        v1 = vg.get(joint).unwrap()[0][0] as u16;
        v2 = vg.get(joint).unwrap()[0][1] as u16;
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
