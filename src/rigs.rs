use ahash::AHashMap;
use bevy::{
    animation::AnimationTargetId,
    ecs::intern::Internable,
    ecs::system::SystemState,
    mesh::{
        VertexAttributeValues,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    prelude::*,

};
use std::sync::Arc;

use crate::{
    basemesh::VertexGroups,
    loaders::{MhcloVertexMap, ReferenceRigAsset, RigConfigAsset, RigWeightsAsset},
    prelude::*,
    skeleton_lod::{RigBundle, SkeletonLodConfig, build_lod_variants},
};

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

#[derive(Component, Reflect)]
#[reflect(Component)]
pub(crate) struct RootBone;

/// Cached data for the root bone of each skeleton.
/// Stores the root bone entity and a scale factor used to adjust root bone Y translation
/// for different human proportions during retargeted animation playback.
#[derive(Component, Debug)]
pub struct SkeletonRootBone {
    pub entity: Entity,
    pub root_scale: f32,
    pub bind_pose_y: f32,
}

/// The single rig bundle, populated by `build_rig_scenes`.
#[derive(Resource, Default)]
pub(crate) struct RigBundleRes(pub Option<RigBundle>);

/// Stores the single loaded rig specification.
#[derive(Resource)]
pub struct RigData(pub Option<RigSpec>);

#[derive(Clone)]
pub struct RigSpec {
    pub(crate) weights: Arc<RigWeightsAsset>,
    pub(crate) config: Arc<RigConfigAsset>,
    pub(crate) reference_rig: Arc<ReferenceRigAsset>,
}

impl Default for RigData {
    fn default() -> Self {
        Self::new()
    }
}

impl RigData {
    pub const fn new() -> Self {
        Self(None)
    }

    pub const fn is_loaded(&self) -> bool {
        self.0.is_some()
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

/// Tracks which rig assets have loaded, for event-driven sync.
#[derive(Default)]
pub(crate) struct RigLoadTracker {
    config: Option<RigConfigAsset>,
    weights: Option<RigWeightsAsset>,
    reference_rig: Option<ReferenceRigAsset>,
}

/// Syncs rig assets reactively as they load. Once all 3 asset types have reported
/// a `LoadedWithDependencies` event, matches them by `rig_name` and inserts into `RigData`.
pub(crate) fn sync_and_build_rig_data(
    mut rig_data: ResMut<RigData>,
    config_assets: Res<Assets<RigConfigAsset>>,
    weights_assets: Res<Assets<RigWeightsAsset>>,
    reference_rig_assets: Res<Assets<ReferenceRigAsset>>,
    mut config_events: MessageReader<AssetEvent<RigConfigAsset>>,
    mut weights_events: MessageReader<AssetEvent<RigWeightsAsset>>,
    mut ref_rig_events: MessageReader<AssetEvent<ReferenceRigAsset>>,
    mut tracker: Local<RigLoadTracker>,
) {
    if rig_data.is_loaded() {
        return;
    }

    for ev in config_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev
            && let Some(asset) = config_assets.get(*id)
        {
            tracker.config = Some(asset.clone());
        }
    }
    for ev in weights_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev
            && let Some(asset) = weights_assets.get(*id)
        {
            tracker.weights = Some(asset.clone());
        }
    }
    for ev in ref_rig_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev
            && let Some(asset) = reference_rig_assets.get(*id)
        {
            tracker.reference_rig = Some(asset.clone());
        }
    }

    let (Some(config), Some(weights), Some(reference_rig)) = (
        tracker.config.as_ref(),
        tracker.weights.as_ref(),
        tracker.reference_rig.as_ref(),
    ) else {
        return;
    };

    if config.rig_name == weights.rig_name && config.rig_name == reference_rig.rig_name {
        rig_data.0 = Some(RigSpec {
            weights: Arc::new(weights.clone()),
            config: Arc::new(config.clone()),
            reference_rig: Arc::new(reference_rig.clone()),
        });
    }
}

/// Tracks which rigs have already been built so we don't rebuild every frame.
#[derive(Resource, Default)]
pub(crate) struct BuiltRigs;

/// Builds skeleton scenes and LOD variants for the loaded rig.
/// Runs once (deduplicated via [`BuiltRigs`]).
pub(crate) fn build_rig_scenes(world: &mut World) {
    // Collect rig data and config
    let (reference_rig, weights, lod_config) = {
        let mut state: SystemState<(
            Res<RigData>,
            Option<Res<SkeletonLodConfig>>,
        )> = SystemState::new(world);
        match state.get(world) {
            Ok((rig_data, lod_config)) => {
                let Some(spec) = &rig_data.0 else {
                    return;
                };
                (
                    Arc::clone(&spec.reference_rig),
                    Arc::clone(&spec.weights),
                    lod_config.map(|c| c.clone()),
                )
            }
            Err(_) => return,
        }
    };

    // Build full skeleton scene (index 0)
    let _scene = build_skeleton_scene(&reference_rig, world);

    let lod_config = lod_config.unwrap_or_default();

    // Use merge configs from SkeletonLodConfig (or full skeleton)
    let merge_configs: Vec<_> = if lod_config.0.is_empty() {
        vec![crate::skeleton_lod::BoneMergeConfig::full()]
    } else {
        lod_config.0
    };

    // Build LOD variants
    let lod_variants = build_lod_variants(&reference_rig, &weights, &merge_configs, world);

    // Store in registry
    world
        .resource_mut::<RigBundleRes>()
        .0 = Some(RigBundle { lod_variants });

    // Mark as built
    world.insert_resource(BuiltRigs);
}

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

/// Builds skeleton scene from reference rig.
pub(crate) fn build_skeleton_scene(
    reference_rig: &Arc<ReferenceRigAsset>,
    world: &mut World,
) -> Handle<DynamicWorld> {
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
            && let Some(&parent) = bone_entities.get(NAME_INTERNER.intern(&parent_name).leak())
        {
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
            scene_world
                .entity_mut(rig_entity)
                .add_child(bone_entities[&name]);
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

    // Setup SkinnedMesh component
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

    let mut ds = world.resource_mut::<Assets<DynamicWorld>>();

    ds.add(DynamicWorld::from_world(&scene_world))
}

pub(crate) fn get_model_space_skeleton_transforms(
    bone_order: &Vec<&'static str>,
    helpers: &[Vec3],
    rig_spec: &RigSpec,
    vg: &VertexGroups,
) -> AHashMap<&'static str, Transform> {
    let mh_config = &rig_spec.config;
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
    rig_spec: &RigSpec,
    global_transforms: &AHashMap<&'static str, Transform>,
) -> AHashMap<&'static str, Transform> {
    // Compute local transforms relative to parent
    let mh_config = &rig_spec.config;
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
pub(crate) fn get_bone_order(rig_data: &RigData) -> Vec<&'static str> {
    let spec = rig_data
        .0
        .as_ref()
        .expect("No rig data loaded");
    let mh_config = &spec.config;
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
