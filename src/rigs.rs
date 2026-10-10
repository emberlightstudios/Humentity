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
    basemesh::{BaseMesh, VertexGroups},
    loaders::{ReferenceRigAsset, RigConfigAsset, RigWeightsAsset},
    prelude::*,
    skeleton_lod::{
        BoneMergeConfig, RigBundle, SkeletonLodConfig, build_lod_data,
        fit_default_rig_single_toes,
    },
};

/// Converts Blender's Z-up rig frame into the MakeHuman OBJ model frame.
const BLENDER_TO_MAKEHUMAN_MODEL_ROTATION: Quat = Quat::from_xyzw(
    -std::f32::consts::FRAC_1_SQRT_2,
    0.0,
    0.0,
    std::f32::consts::FRAC_1_SQRT_2,
);

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

/// Tracks config and weight assets while they load.
#[derive(Default)]
pub(crate) struct RigLoadTracker {
    config: Option<RigConfigAsset>,
    weights: Option<RigWeightsAsset>,
}

/// Syncs rig assets reactively as they load. The reference bind pose is built
/// after the config, weights, base-mesh helpers, and vertex groups are available.
pub(crate) fn sync_and_build_rig_data(
    mut rig_data: ResMut<RigData>,
    config_assets: Res<Assets<RigConfigAsset>>,
    weights_assets: Res<Assets<RigWeightsAsset>>,
    base_mesh: Option<Res<BaseMesh>>,
    vertex_groups: Option<Res<VertexGroups>>,
    mut config_events: MessageReader<AssetEvent<RigConfigAsset>>,
    mut weights_events: MessageReader<AssetEvent<RigWeightsAsset>>,
    mut tracker: Local<RigLoadTracker>,
) {
    if rig_data.is_loaded() {
        return;
    }

    for asset_event in config_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = asset_event
            && let Some(loaded_rig_config) = config_assets.get(*id)
        {
            tracker.config = Some(loaded_rig_config.clone());
        }
    }
    for asset_event in weights_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = asset_event
            && let Some(loaded_rig_weights) = weights_assets.get(*id)
        {
            tracker.weights = Some(loaded_rig_weights.clone());
        }
    }

    let (Some(config), Some(weights)) = (tracker.config.as_ref(), tracker.weights.as_ref()) else {
        // The config and weight assets load independently; wait for both events.
        return;
    };
    let (Some(base_mesh), Some(vertex_groups)) = (base_mesh, vertex_groups) else {
        // The asset setup inserts helper resources during startup; wait for that setup.
        return;
    };
    if base_mesh.vertices.is_empty() || vertex_groups.is_empty() {
        // Reference bind-pose positions use the same helper-space inputs as character fitting.
        return;
    }

    if config.rig_name == weights.rig_name {
        let reference_rig =
            ReferenceRigAsset::from_rig_config(config, &base_mesh.vertices, &vertex_groups);
        rig_data.0 = Some(RigSpec {
            weights: Arc::new(weights.clone()),
            config: Arc::new(config.clone()),
            reference_rig: Arc::new(reference_rig),
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
        let mut state: SystemState<(Res<RigData>, Option<Res<SkeletonLodConfig>>)> =
            SystemState::new(world);
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
                "reference rig bone '{name}' was never spawned: JSON bone order and spawned entities diverged (check the rig config hierarchy)"
            )
        });
        let parent_name = ref_bone_parents.get(name).cloned().unwrap_or_default();
        if !parent_name.is_empty()
            && let Some(&parent) = bone_entities.get(NAME_INTERNER.intern(&parent_name).leak())
        {
            scene_world.entity_mut(parent).add_child(child);
        }
    }

    // Attach the single root to the rig entity. MPFB exports exactly one bone
    // under `Human.rig`: 0 means the rig name is wrong, 2+ means the armature
    // is not single-root. Either way attaching silently would desync clip
    // TargetIds from skeleton joints.
    let mut rig_root_children = bone_order
        .iter()
        .filter(|bone_name| {
            ref_bone_parents
                .get(*bone_name)
                .is_some_and(|rig_parent_name| rig_parent_name == "Human.rig")
        })
        .peekable();
    let Some(&root_bone_name) = rig_root_children.next() else {
        panic!(
            "reference rig '{}' has no bone parented to 'Human.rig': the MPFB rig JSON must define one root bone (check the parent fields)",
            reference_rig.rig_name,
        );
    };
    if let Some(&second_root_name) = rig_root_children.next() {
        panic!(
            "reference rig '{}' has multiple bones parented to 'Human.rig' ('{root_bone_name}' and '{second_root_name}'): the MPFB rig JSON must define one root bone",
            reference_rig.rig_name,
        );
    }
    let &root_bone = bone_entities.get(&root_bone_name).unwrap_or_else(|| {
        panic!(
            "reference rig root bone '{root_bone_name}' was never spawned: JSON bone order and spawned entities diverged (check the rig config hierarchy)"
        )
    });
    scene_world.entity_mut(rig_entity).add_child(root_bone);

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

/// Model-space bind pose with reference rotations restored from the JSON-built rig.
/// Positions come from each character's helper vertices; rotations use the base
/// rig's MPFB roll values so morphing cannot change the animation rest frame.
///
/// `merge_config` is the merge the fitted skeleton belongs to. When it keeps a
/// single toe per foot ([`BoneMergeConfig::merge_default_rig_toes`]) the kept
/// toe follows the no-toes clip frame; a full (unmerged) config keeps the
/// reference rotation for every bone, toe included.
pub(crate) fn fitted_model_space_bindposes(
    helpers: &[Vec3],
    rig: &RigSpec,
    vertex_groups: &AHashMap<String, Vec<[usize; 2]>>,
    merge_config: &BoneMergeConfig,
) -> AHashMap<&'static str, Transform> {
    let mut model_space_bindposes = AHashMap::default();
    for &bone_name in &rig.reference_rig.bone_names {
        let bone_config = rig.config.bones.get(bone_name).unwrap_or_else(|| {
            panic!("reference rig bone '{bone_name}' has no MPFB rig JSON configuration")
        });
        let head_position = get_bone_position(&bone_config.head, vertex_groups, helpers);
        let reference_rotation = rig.reference_rig.model_space_bindpose[bone_name].rotation;
        model_space_bindposes.insert(
            bone_name,
            Transform::from_translation(head_position).with_rotation(reference_rotation),
        );
    }
    fit_default_rig_single_toes(
        &rig.reference_rig().rig_name,
        merge_config,
        &mut model_space_bindposes,
    );
    model_space_bindposes
}

pub(crate) fn get_bone_transform(
    bone_name: &'static str,
    bone_config: &BoneJsonConfig,
    vertex_groups: &AHashMap<String, Vec<[usize; 2]>>,
    helpers: &[Vec3],
) -> Transform {
    let head_position = get_bone_position(&bone_config.head, vertex_groups, helpers);
    let tail_position = get_bone_position(&bone_config.tail, vertex_groups, helpers);
    bone_transform_from_endpoints(bone_name, head_position, tail_position, bone_config.roll)
}

pub(crate) fn bone_transform_from_endpoints(
    bone_name: &'static str,
    head_position: Vec3,
    tail_position: Vec3,
    roll_radians: f32,
) -> Transform {
    let bone_direction = (tail_position - head_position)
        .try_normalize()
        .unwrap_or_else(|| {
            panic!(
                "rig config bone '{bone_name}' has coincident head and tail positions; Blender bones must have a non-zero length"
            )
        });
    // MPFB roll angles are defined in Blender's Z-up frame. Align the local
    // bone Y axis using the Blender-frame direction, apply roll there, and
    // convert the resulting frame to the Y-up MakeHuman model coordinates.
    let blender_bone_direction =
        BLENDER_TO_MAKEHUMAN_MODEL_ROTATION.inverse() * bone_direction;
    let align_blender_local_y_to_bone =
        Quat::from_rotation_arc(Vec3::Y, blender_bone_direction);
    let blender_bone_rotation =
        align_blender_local_y_to_bone * Quat::from_rotation_y(roll_radians);
    let model_bone_rotation =
        (BLENDER_TO_MAKEHUMAN_MODEL_ROTATION * blender_bone_rotation).normalize();
    Transform::from_translation(head_position).with_rotation(model_bone_rotation)
}

fn get_bone_position(
    bone_position_spec: &BoneTransformSpec,
    vertex_groups: &AHashMap<String, Vec<[usize; 2]>>,
    helpers: &[Vec3],
) -> Vec3 {
    if bone_position_spec.strategy == "MEAN" {
        let helper_vertex_indices = bone_position_spec
            .vertex_indices
            .as_ref()
            .unwrap_or_else(|| {
                panic!(
                    "rig config bone uses MEAN strategy but has no vertex_indices: each MEAN bone needs exactly 2 helper vertex ids (check the rig config asset)"
                )
            });
        let (first_helper_vertex_index, second_helper_vertex_index) =
            (helper_vertex_indices[0], helper_vertex_indices[1]);
        (helpers[second_helper_vertex_index as usize]
            + helpers[first_helper_vertex_index as usize])
            / 2.0
    } else if bone_position_spec.strategy == "CUBE" {
        let vertex_group_name = bone_position_spec.cube_name.as_ref().unwrap_or_else(|| {
            panic!(
                "rig config bone uses CUBE strategy but has no cube_name: each CUBE bone must name a vertex group (check the rig config asset)"
            )
        });
        let vertex_group_ranges = vertex_groups.get(vertex_group_name).unwrap_or_else(|| {
            panic!(
                "rig config CUBE bone names vertex group '{vertex_group_name}' which is missing from the vertex-groups asset"
            )
        });
        let (first_helper_vertex_index, last_helper_vertex_index) = (
            vertex_group_ranges[0][0] as u16,
            vertex_group_ranges[0][1] as u16,
        );
        let mut helper_position_sum = Vec3::ZERO;
        for helper_vertex_index in first_helper_vertex_index..last_helper_vertex_index + 1 {
            helper_position_sum += helpers[helper_vertex_index as usize];
        }
        helper_position_sum / (last_helper_vertex_index - first_helper_vertex_index + 1) as f32
    } else if bone_position_spec.strategy == "VERTEX" {
        let helper_vertex_index = bone_position_spec.vertex_index.unwrap_or_else(|| {
            panic!(
                "rig config bone uses VERTEX strategy but has no vertex_index: each VERTEX bone needs one helper vertex id (check the rig config asset)"
            )
        });
        helpers[helper_vertex_index as usize]
    } else {
        unimplemented!(
            "rig config bone has unrecognized strategy '{}': expected MEAN, CUBE, or VERTEX (check the rig config asset)",
            bone_position_spec.strategy
        )
    }
}

#[cfg(test)]
mod default_rig_toe_average_tests {
    use crate::skeleton_lod::fit_default_rig_single_toes;

    use super::*;

    /// A config that keeps one toe per foot via the default-rig toe merge.
    fn merged_toes_config() -> BoneMergeConfig {
        BoneMergeConfig::full().merge_default_rig_toes()
    }

    fn toe_test_pose() -> AHashMap<&'static str, Transform> {
        [
            ("toe1-1.L", Vec3::new(0.1, 0.2, 0.3)),
            ("toe1-1.R", Vec3::new(-0.1, 0.2, 0.3)),
        ]
        .into_iter()
        .map(|(toe_bone_name, toe_position)| {
            (toe_bone_name, Transform::from_translation(toe_position))
        })
        .collect()
    }

    #[test]
    fn kept_toe_position_never_moves() {
        let mut model_space = toe_test_pose();
        fit_default_rig_single_toes("default", &merged_toes_config(), &mut model_space);
        // The rig config already plants the kept toe on the big-toe joint;
        // centroid averaging used to drag it ~3cm sideways. Position is exact
        // and must survive the fit untouched.
        assert_eq!(
            model_space["toe1-1.L"].translation,
            Vec3::new(0.1, 0.2, 0.3)
        );
        assert_eq!(
            model_space["toe1-1.R"].translation,
            Vec3::new(-0.1, 0.2, 0.3)
        );
    }

    #[test]
    fn kept_toe_rest_roll_matches_no_toes_bind_pose() {
        let mut model_space = toe_test_pose();
        fit_default_rig_single_toes("default", &merged_toes_config(), &mut model_space);
        // Identity rest rotated by the fix must equal the measured no-toes
        // bind-pose twist (xyzw), not identity: the full-toes reference roll
        // would otherwise twist the skinned toe mesh in every pose.
        let kept_toe_rotation = model_space["toe1-1.L"].rotation;
        let expected_roll_fix =
            Quat::from_array([-0.0412228, -0.8170496, -0.0755843, 0.5701033]).normalize();
        assert!(
            kept_toe_rotation.angle_between(expected_roll_fix) < 1e-4,
            "kept toe must carry the no-toes roll fix, got {kept_toe_rotation:?}"
        );
    }

    #[test]
    fn other_rigs_keep_original_toe_pose() {
        let mut model_space = toe_test_pose();
        fit_default_rig_single_toes("mixamo", &merged_toes_config(), &mut model_space);
        assert_eq!(
            model_space["toe1-1.L"],
            Transform::from_translation(Vec3::new(0.1, 0.2, 0.3))
        );
    }

    #[test]
    fn full_config_keeps_reference_toe_pose() {
        let mut model_space = toe_test_pose();
        fit_default_rig_single_toes("default", &BoneMergeConfig::full(), &mut model_space);
        // Unmerged configs pose every toe bone, so no kept-toe frame exists
        // and the reference rotation must survive untouched.
        assert_eq!(
            model_space["toe1-1.L"],
            Transform::from_translation(Vec3::new(0.1, 0.2, 0.3))
        );
    }

    #[test]
    fn missing_toe_bones_skip_without_panic() {
        let mut model_space: AHashMap<&'static str, Transform> = AHashMap::default();
        fit_default_rig_single_toes("default", &merged_toes_config(), &mut model_space);
        assert!(model_space.is_empty());
    }
}

#[cfg(test)]
mod mpfb_bind_pose_tests {
    use super::*;

    #[test]
    fn upperarm_roll_matches_converted_blender_edit_bone_basis() {
        let upperarm_bind_transform = bone_transform_from_endpoints(
            "upperarm01.L",
            Vec3::new(0.167713, -0.014605, 1.342323),
            Vec3::new(0.215755, -0.019590, 1.286708),
            2.3827133,
        );
        let computed_model_local_x_axis = upperarm_bind_transform.rotation * Vec3::X;
        let computed_model_local_z_axis = upperarm_bind_transform.rotation * Vec3::Z;
        // Blender EditBone axes transformed from its Z-up frame into the OBJ model frame.
        let expected_model_local_x_axis = Vec3::new(-0.567044, -0.704562, -0.426678);
        let expected_model_local_z_axis = Vec3::new(0.503079, -0.706408, 0.497894);

        assert!(
            (computed_model_local_x_axis - expected_model_local_x_axis).length() < 2e-4,
            "computed model-local X axis {computed_model_local_x_axis:?} did not match Blender {expected_model_local_x_axis:?}"
        );
        assert!(
            (computed_model_local_z_axis - expected_model_local_z_axis).length() < 2e-4,
            "computed model-local Z axis {computed_model_local_z_axis:?} did not match Blender {expected_model_local_z_axis:?}"
        );
    }
}

#[cfg(test)]
mod full_toe_shape_fit_tests {
    use super::*;
    use crate::loaders::{BoneJsonConfig, BoneTransformSpec};

    fn helper_vertex_spec(vertex_index: u16) -> BoneTransformSpec {
        BoneTransformSpec {
            cube_name: None,
            strategy: "VERTEX".to_string(),
            vertex_indices: None,
            vertex_index: Some(vertex_index),
        }
    }

    fn test_bone_config(
        parent_bone_name: &str,
        head_vertex_index: u16,
        tail_vertex_index: u16,
        roll_radians: f32,
    ) -> BoneJsonConfig {
        BoneJsonConfig {
            parent: parent_bone_name.to_string(),
            head: helper_vertex_spec(head_vertex_index),
            tail: helper_vertex_spec(tail_vertex_index),
            roll: roll_radians,
        }
    }

    #[test]
    fn full_rig_shape_fit_preserves_reference_toe_rotation() {
        let rig_spec = two_bone_toe_rig();
        let reference_toe_rotation =
            rig_spec.reference_rig.model_space_bindpose["toe1-1.L"].rotation;

        let fitted_model_space = fitted_model_space_bindposes(
            &toe_test_helpers(),
            &rig_spec,
            &AHashMap::default(),
            &BoneMergeConfig::full(),
        );

        assert!(
            fitted_model_space["toe1-1.L"]
                .rotation
                .angle_between(reference_toe_rotation)
                < 1e-6,
            "unmerged GPU shape fit must keep the reference toe bind rotation"
        );
    }

    #[test]
    fn merged_toe_shape_fit_uses_no_toes_clip_frame() {
        let rig_spec = two_bone_toe_rig();
        let reference_toe_rotation =
            rig_spec.reference_rig.model_space_bindpose["toe1-1.L"].rotation;
        let merged_toes = BoneMergeConfig::full().merge_default_rig_toes();

        let fitted_model_space = fitted_model_space_bindposes(
            &toe_test_helpers(),
            &rig_spec,
            &AHashMap::default(),
            &merged_toes,
        );

        // The kept toe carries the measured no-toes twist on top of the
        // reference rotation: a real difference from the reference frame, not
        // identity and not the reference pose.
        let merged_toe_rotation = fitted_model_space["toe1-1.L"].rotation;
        assert!(
            merged_toe_rotation.angle_between(reference_toe_rotation) > 1.0,
            "merged kept toe must carry the no-toes twist, got {merged_toe_rotation:?}"
        );
        let expected_toe_rotation =
            (reference_toe_rotation * Quat::from_array([-0.0412228, -0.8170496, -0.0755843, 0.5701033]))
                .normalize();
        assert!(
            merged_toe_rotation.angle_between(expected_toe_rotation) < 1e-6,
            "merged kept toe must be the reference rotation times the twist fix"
        );
        // Only the kept toe changes frame; its parent foot is untouched.
        assert!(
            fitted_model_space["foot.L"]
                .rotation
                .angle_between(rig_spec.reference_rig.model_space_bindpose["foot.L"].rotation)
                < 1e-6,
            "the foot must keep the reference rotation in both modes"
        );
    }

    /// Two-bone default rig (`foot.L` -> `toe1-1.L`) over helper vertices that
    /// stand in for a morphed body slightly longer than the reference.
    fn two_bone_toe_rig() -> RigSpec {
        let rig_config = RigConfigAsset {
            bones: [
                ("foot.L", test_bone_config("", 0, 1, 0.0)),
                ("toe1-1.L", test_bone_config("foot.L", 1, 2, 0.3)),
            ]
            .into_iter()
            .collect(),
            rig_name: "default".to_string(),
        };
        let helpers = toe_test_helpers();
        let vertex_groups = AHashMap::default();
        let reference_rig =
            ReferenceRigAsset::from_rig_config(&rig_config, &helpers, &vertex_groups);
        RigSpec {
            weights: Arc::new(RigWeightsAsset {
                weights: AHashMap::default(),
                rig_name: "default".to_string(),
            }),
            config: Arc::new(rig_config),
            reference_rig: Arc::new(reference_rig),
        }
    }

    fn toe_test_helpers() -> [Vec3; 3] {
        [Vec3::ZERO, Vec3::Y, Vec3::new(0.25, 1.8, 0.1)]
    }
}
