use ahash::AHashMap;
use bevy::{
    animation::AnimationTargetId,
    ecs::intern::Internable,
    ecs::system::SystemState,
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,

};
use std::sync::Arc;

use crate::{
    basemesh::VertexGroups,
    loaders::{ReferenceRigAsset, RigConfigAsset, RigWeightsAsset},
    prelude::*,
    skeleton_lod::{
        RigBundle, SkeletonLodConfig, average_default_rig_toe_positions, build_lod_data,
    },
};

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
    /// Reference-rig root Y the clip was authored against. Offsets are
    /// measured from here, then scaled onto the fitted bind pose.
    pub reference_bind_pose_y: f32,
}

/// Rearward shift for the default rig's skeleton root, in reference-rig meters.
/// The default rig's root bone sits at the rear of the pelvis, so without this
/// fitted characters ride slightly forward of their capsule collider. Scales
/// with `CharacterScale` on the CPU; the GPU pose shader applies the same shift
/// from its uniforms (see `pose.wgsl`). Characters face -Z, so rearward is +Z.
/// Other rigs get no shift.
pub(crate) const DEFAULT_RIG_REAR_OFFSET_METERS: f32 = 0.05;

/// Skeleton-root rearward shift for a rig: the default-rig shift, or zero for
/// any other rig. Shared by the CPU skeleton root (`spawn_skeleton`) and the
/// GPU pose uniforms (`gpu::bake`).
pub(crate) fn skeleton_rear_offset_meters(rig_name: &str) -> f32 {
    if rig_name == "default" {
        DEFAULT_RIG_REAR_OFFSET_METERS
    } else {
        0.0
    }
}

/// The single rig bundle, populated by `build_rig_scenes`.
/// Holds the full skeleton scene (used for spawning) plus the per-LOD merge data
/// (used for mesh painting / part skinning).
#[derive(Resource, Default)]
pub(crate) struct RigBundleRes {
    /// Full skeleton scene (all bones), spawned once per character.
    pub scene: Option<Handle<DynamicWorld>>,
    /// Per-LOD merge data for the rig.
    pub bundle: Option<RigBundle>,
}

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

    pub fn reference_rig(&self) -> &ReferenceRigAsset {
        &self.reference_rig
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
    let scene = build_skeleton_scene(&reference_rig, world);

    let lod_config = lod_config.unwrap_or_default();

    // Use merge configs from SkeletonLodConfig (or full skeleton)
    let merge_configs = &lod_config.0[..lod_config.1];

    // Build per-LOD merge data (no separate skeleton scenes are built)
    let lod_data = build_lod_data(&reference_rig, &weights, merge_configs);

    // Store in registry
    let mut bundle_res = world.resource_mut::<RigBundleRes>();
    bundle_res.scene = Some(scene);
    bundle_res.bundle = Some(RigBundle { lod_data });

    // Mark as built
    world.insert_resource(BuiltRigs);
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
        let &child = bone_entities.get(&name).unwrap_or_else(|| {
            panic!(
                "reference rig bone '{name}' was never spawned: bone_order and spawned entities diverged (check the reference rig GLB joint list)"
            )
        });
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
            let &root_bone = bone_entities.get(&name).unwrap_or_else(|| {
                panic!(
                    "reference rig root bone '{name}' was never spawned: bone_order and spawned entities diverged (check the reference rig GLB joint list)"
                )
            });
            scene_world.entity_mut(rig_entity).add_child(root_bone);
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

/// Model-space bindpose with reference rotations restored (CPU fit pass 1).
///
/// Positions come from the morphed helpers; every rotation is replaced with the
/// reference rig's model-space rotation, so retargeted clips (authored for
/// reference rotations) play correctly on any shape. Shared by the CPU fit
/// (`fit_skeleton_to_shape`) and the GPU shape fit (`fit_shape_skeleton`).
pub(crate) fn fitted_model_space_bindposes(
    helpers: &[Vec3],
    rig: &RigSpec,
    vg: &VertexGroups,
) -> AHashMap<&'static str, Transform> {
    let mut model_space =
        get_model_space_skeleton_transforms(&rig.reference_rig.bone_names, helpers, rig, vg);
    let bone_config = &rig.config;
    // Pass 1: replace all model-space rotations with reference rig rotations.
    // This must happen before any local computation so parent lookups are correct.
    for &bone in &rig.reference_rig.bone_names {
        if bone_config.bones.contains_key(bone) {
            let old_global = model_space[bone];
            let reference_rot = rig.reference_rig.model_space_bindpose[bone].rotation;
            model_space.insert(
                bone,
                Transform {
                    translation: old_global.translation,
                    rotation: reference_rot,
                    scale: old_global.scale,
                },
            );
        }
    }
    average_default_rig_toe_positions(
        &rig.reference_rig().rig_name,
        &rig.reference_rig().bone_parents,
        &mut model_space,
    );
    model_space
}

#[cfg(test)]
mod default_rig_toe_average_tests {
    use crate::skeleton_lod::average_default_rig_toe_positions;

    use super::*;

    fn toe_test_pose() -> (
        AHashMap<&'static str, String>,
        AHashMap<&'static str, Transform>,
    ) {
        let bone_parents: AHashMap<&'static str, String> = [
            ("foot.L", "lowerleg02.L".to_string()),
            ("toe1-1.L", "foot.L".to_string()),
            ("toe1-2.L", "toe1-1.L".to_string()),
            ("toe2-1.L", "foot.L".to_string()),
        ]
        .into_iter()
        .collect();
        let model_space: AHashMap<&'static str, Transform> = [
            ("foot.L", Vec3::ZERO),
            ("toe1-1.L", Vec3::ZERO),
            ("toe1-2.L", Vec3::new(0.0, 0.0, 2.0)),
            ("toe2-1.L", Vec3::new(3.0, 0.0, 0.0)),
        ]
        .into_iter()
        .map(|(toe_bone_name, toe_position)| {
            (toe_bone_name, Transform::from_translation(toe_position))
        })
        .collect();
        (bone_parents, model_space)
    }

    #[test]
    fn kept_toe_moves_to_average_toe_position() {
        let (bone_parents, mut model_space) = toe_test_pose();
        average_default_rig_toe_positions("default", &bone_parents, &mut model_space);
        let kept_toe_translation = model_space["toe1-1.L"].translation;
        assert!(
            (kept_toe_translation - Vec3::new(1.0, 0.0, 2.0 / 3.0)).length() < 1e-6,
            "kept toe must sit at the toe centroid, got {kept_toe_translation:?}"
        );
    }

    #[test]
    fn other_rigs_keep_original_toe_positions() {
        let (bone_parents, mut model_space) = toe_test_pose();
        average_default_rig_toe_positions("mixamo", &bone_parents, &mut model_space);
        assert_eq!(model_space["toe1-1.L"].translation, Vec3::ZERO);
    }

    #[test]
    fn missing_toe_bones_skip_without_panic() {
        let (bone_parents, _) = toe_test_pose();
        let mut model_space: AHashMap<&'static str, Transform> = AHashMap::default();
        average_default_rig_toe_positions("default", &bone_parents, &mut model_space);
        assert!(model_space.is_empty());
    }
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
    if bone.strategy == "MEAN" {
        let indices = bone.vertex_indices.as_ref().unwrap_or_else(|| {
            panic!(
                "rig config bone uses MEAN strategy but has no vertex_indices: each MEAN bone needs exactly 2 helper vertex ids (check the rig config asset)"
            )
        });
        let (v1, v2) = (indices[0], indices[1]);
        (helpers[v2 as usize] + helpers[v1 as usize]) / 2.
    } else if bone.strategy == "CUBE" {
        let joint = bone.cube_name.as_ref().unwrap_or_else(|| {
            panic!(
                "rig config bone uses CUBE strategy but has no cube_name: each CUBE bone must name a vertex group (check the rig config asset)"
            )
        });
        let group = vg.get(joint).unwrap_or_else(|| {
            panic!(
                "rig config CUBE bone names vertex group '{joint}' which is missing from the vertex-groups asset"
            )
        });
        let (v1, v2) = (group[0][0] as u16, group[0][1] as u16);
        let mut pos = Vec3::ZERO;
        for v in v1..v2 + 1 {
            pos += helpers[v as usize];
        }
        pos / (v2 - v1 + 1) as f32
    } else if bone.strategy == "VERTEX" {
        let index = bone.vertex_index.unwrap_or_else(|| {
            panic!(
                "rig config bone uses VERTEX strategy but has no vertex_index: each VERTEX bone needs one helper vertex id (check the rig config asset)"
            )
        });
        helpers[index as usize]
    } else {
        unimplemented!(
            "rig config bone has unrecognized strategy '{}': expected MEAN, CUBE, or VERTEX (check the rig config asset)",
            bone.strategy
        )
    }
}
