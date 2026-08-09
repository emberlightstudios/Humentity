use crate::{
    basemesh::VertexGroups,
    helpers::HelperVertexPositions,
    prelude::*,
    rigs::{
        BoneTranslationData, RigBundleRes, RigData, SkeletonRootBone,
        get_model_space_skeleton_transforms,
    },
    skeleton_lod::{MAX_LODS, SkeletonLodConfig, all_children_of},
};
use ahash::{AHashMap, AHashSet};
use bevy::{
    animation::AnimatedBy,
    ecs::intern::Internable,
    mesh::morph::MeshMorphWeights,
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};

/// Describes a character's single fixed skeleton.
///
/// Placed on the `CharacterShape` entity once the skeleton has been fitted.
/// All bones exist as entities in `skeleton_entity`'s scene; skeleton LOD is
/// expressed purely by enabling/disabling bone sub-trees within it (via
/// `SkeletonLodDisabled`).
#[derive(Component, Debug)]
pub struct CharacterSkeleton {
    /// The one full skeleton scene entity, a child of the `CharacterShape`.
    pub skeleton_entity: Entity,
    /// Bone name → bone entity in the fixed skeleton (full reference rig).
    pub bone_map: AHashMap<&'static str, Entity>,
    /// Full inverse bindposes, in reference bone order.
    pub model_space_inv_bindposes: Vec<Mat4>,
}

/// Placed on `CharacterShape` once its single skeleton has been fitted.
#[derive(Component, Debug)]
pub struct SkeletonsReady;

/// Tracks which skeleton LOD levels are currently active (in use by a shown mesh).
///
/// Placed on the `CharacterShape`. `active[k] == true` means LOD `k` is active.
/// The reconcile system (`sync_skeleton_lod_subtrees`) disables a bone sub-tree
/// iff its root is present in **every** active LOD's cumulative remove list.
#[derive(Component, Debug, Clone, Copy)]
pub struct SkeletonLodState {
    pub active: [bool; MAX_LODS],
}

/// Internal marker on a `CharacterShape` that has spawned a skeleton but not yet
/// fitted it to morphs.
#[derive(Component, Debug)]
pub(crate) struct FitSkeleton;

/// Custom disabling component for skeleton LOD bone sub-trees.
/// Registered as a disabling component so Bevy's default query filters exclude it.
/// Used instead of [`Disabled`] to avoid conflicting with other systems.
#[derive(Component, Clone, Debug, Default)]
pub struct SkeletonLodDisabled;

/// Spawns the single full skeleton scene as a child of each `CharacterShape`,
/// and marks the character as needing a fit.
pub(crate) fn spawn_rig_skeleton(
    characters: Query<
        Entity,
        (
            Without<SkeletonsReady>,
            Without<FitSkeleton>,
            With<CharacterShape>,
            With<HelperVertexPositions>,
        ),
    >,
    rig_bundle: Res<RigBundleRes>,
    mut commands: Commands,
) {
    for entity in &characters {
        let Some(scene) = rig_bundle.scene.clone() else {
            continue;
        };

        let skeleton_entity = commands
            .spawn((
                DynamicWorldRoot::from(scene),
                Transform::from_rotation(crate::MODEL_ROTATION_FIX),
                FitSkeleton,
                Name::new("Skeleton"),
            ))
            .id();

        commands.entity(entity).add_child(skeleton_entity);
        commands.entity(entity).insert((
            CharacterSkeleton {
                skeleton_entity,
                bone_map: AHashMap::default(),
                model_space_inv_bindposes: Vec::new(),
            },
            FitSkeleton,
        ));
    }
}

/// Fits the single skeleton's bone transforms to the character's morph shape.
/// Builds the full `bone_map` and `model_space_inv_bindposes` for `CharacterSkeleton`,
/// then removes `FitSkeleton`.
#[allow(clippy::type_complexity)]
pub(crate) fn fit_skeleton_to_shape(
    mut commands: Commands,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
    skinned_meshes: Query<
        &SkinnedMesh,
        (Without<Mesh3d>, With<ChildOf>, Allow<SkeletonLodDisabled>),
    >,
    children: Query<&Children, Allow<SkeletonLodDisabled>>,
    joint_names: Query<&Name, (With<SkeletalBone>, Allow<SkeletonLodDisabled>)>,
    mut characters: Query<
        (
            Entity,
            &CharacterShape,
            Option<&AnimationPlayer>,
            Option<&HelperVertexPositions>,
            &mut CharacterSkeleton,
        ),
        (With<FitSkeleton>, Allow<SkeletonLodDisabled>),
    >,
    mut local_transforms: Query<
        &mut Transform,
        (Without<CharacterShape>, Allow<SkeletonLodDisabled>),
    >,
    rig_data: Res<RigData>,
    vg: Res<VertexGroups>,
) {
    for (entity, character_shape, animation_player, computed_helpers, skeleton) in
        characters.iter_mut()
    {
        let Some(h) = computed_helpers else {
            continue;
        };
        let helpers = &h.0;
        // Wait for the background helper computation before fitting; FitSkeleton is
        // only removed on a successful fit, so this retries until helpers are ready.
        if helpers.is_empty() {
            continue;
        }
        let Some(mut shape) = shape_assets.get_mut(&character_shape.0) else {
            continue;
        };

        let Some(rig_spec) = rig_data.0.as_ref() else {
            continue;
        };

        // Find the rig scene entity (holds SkinnedMesh) inside the single skeleton.
        let mut skm: Option<SkinnedMesh> = None;
        for child in children.iter_descendants(skeleton.skeleton_entity) {
            if let Ok(s) = skinned_meshes.get(child) {
                skm = Some(s.clone());
                break;
            }
        }
        let Some(skm) = skm else {
            continue;
        };

        // Build the full bone_map from SkinnedMesh.joints + joint names.
        // With a single full skeleton, skm.joints is in reference rig order.
        let mut bone_entities = AHashMap::default();
        for &joint in &skm.joints {
            if let Ok(name) = joint_names.get(joint) {
                bone_entities.insert(NAME_INTERNER.intern(name.as_str()).leak(), joint);
            }
        }

        if bone_entities.is_empty() {
            continue;
        }

        if animation_player.is_some() {
            for &bone_entity in bone_entities.values() {
                commands
                    .entity(bone_entity)
                    .insert(AnimatedBy(entity));
            }
        }

        // Re-fit skeleton to mesh shape.
        let mut model_space_bindposes = get_model_space_skeleton_transforms(
            &rig_spec.reference_rig.bone_names,
            helpers,
            rig_spec,
            &vg,
        );

        let bone_config = &rig_spec.config;

        // Pass 1: Replace all model-space rotations with reference rig rotations.
        // This must happen before any local computation so parent lookups are correct.
        for &bone in &rig_spec.reference_rig.bone_names {
            if bone_config.bones.contains_key(bone) {
                let old_global = model_space_bindposes[bone];
                let reference_rot = rig_spec.reference_rig.model_space_bindpose[bone].rotation;
                model_space_bindposes.insert(
                    bone,
                    Transform {
                        translation: old_global.translation,
                        rotation: reference_rot,
                        scale: old_global.scale,
                    },
                );
            }
        }

        // Pass 2: Compute local transforms from model-space using the reference rig parent chain.
        let mut local_bone_transforms = AHashMap::default();
        for &bone in &rig_spec.reference_rig.bone_names {
            if let Some(bone_data) = bone_config.bones.get(bone) {
                let parent_name = if bone_data.parent.is_empty() {
                    ""
                } else {
                    NAME_INTERNER.intern(&bone_data.parent).leak()
                };
                let parent_transform = if parent_name.is_empty() {
                    Transform::IDENTITY
                } else {
                    model_space_bindposes
                        .get(parent_name)
                        .copied()
                        .unwrap_or(Transform::IDENTITY)
                };

                let child_matrix = model_space_bindposes[bone].to_matrix();
                let parent_matrix = parent_transform.to_matrix();
                let new_local = Transform::from_matrix(parent_matrix.inverse() * child_matrix);

                local_bone_transforms.insert(bone, new_local);

                if let Some(&joint) = bone_entities.get(bone)
                    && let Ok(mut local_transform) = local_transforms.get_mut(joint)
                {
                    *local_transform = new_local;
                }
            }
        }

        // Cache per-bone translation data for animation rescaling (computed once per asset).
        if matches!(shape.bone_translations, BoneTranslationData::None) {
            let translations = model_space_bindposes
                .iter()
                .map(|(&bone, xform)| (bone, xform.translation))
                .collect();
            shape.bone_translations = BoneTranslationData::Full(translations);
            shape.bone_delta_rotations = AHashMap::<&'static str, Quat>::default();
        }

        // Compute full inverse bindposes in reference bone order.
        let model_space_inv_bindposes: Vec<Mat4> = rig_spec
            .reference_rig
            .bone_names
            .iter()
            .map(|&bone| model_space_bindposes[bone].to_matrix().inverse())
            .collect();

        // Rotate skeleton to face -Z (model verts face +Z).
        commands
            .entity(skeleton.skeleton_entity)
            .insert(Transform::from_rotation(crate::MODEL_ROTATION_FIX));

        // Compute root bone scale factor for retargeted animation Y correction.
        let root_bone_name = rig_spec.reference_rig.bone_names[0];
        let reference_root_y = rig_spec.reference_rig.model_space_bindpose[root_bone_name]
            .translation
            .y;
        let fitted_root_y = model_space_bindposes[root_bone_name].translation.y;
        let root_scale = if reference_root_y.abs() > 1e-6 {
            fitted_root_y / reference_root_y
        } else {
            1.0
        };

        // Capture the root entity before `bone_entities` is moved into the
        // deferred insert below.
        let root_bone_entity = bone_entities[root_bone_name];

        commands.entity(entity).insert(CharacterSkeleton {
            skeleton_entity: skeleton.skeleton_entity,
            bone_map: bone_entities,
            model_space_inv_bindposes,
        });
        commands
            .entity(skeleton.skeleton_entity)
            .insert(SkeletonRootBone {
                entity: root_bone_entity,
                root_scale,
                bind_pose_y: fitted_root_y,
            });
        // `FitSkeleton` lives on the CharacterShape (the query filter), so remove
        // it there. This is what allows `check_skeletons_ready` to fire.
        commands.entity(entity).remove::<FitSkeleton>();
    }
}

/// Once the single skeleton is fitted (no `FitSkeleton`), insert `SkeletonsReady`.
pub(crate) fn check_skeletons_ready(
    mut commands: Commands,
    characters: Query<(Entity, Option<&FitSkeleton>), (With<CharacterSkeleton>, Without<SkeletonsReady>)>,
) {
    for (entity, fit) in &characters {
        if fit.is_none() {
            commands.entity(entity).insert(SkeletonsReady);
        }
    }
}

/// Reconciles the enabled/disabled bone sub-trees against the active LOD set.
///
/// A bone sub-tree is disabled iff its **anchor** root is present in *every*
/// active LOD's cumulative remove list (the intersection of active LOD lists).
/// This is the safe rule during crossfades: only sub-trees removed by *all*
/// active LODs are disabled; everything else stays enabled.
///
/// Only the removed *descendants* of each anchor are disabled — the anchor bone
/// itself always survives (it is the merge target that the removed bones' weights
/// are folded into). For example LOD 1 (`without_children_of ["foot.*", "head"]`)
/// disables the toe and face bones but keeps `foot.*` and `head` enabled so the
/// skinned mesh does not collapse to the origin.
///
/// Only acts when a character's `SkeletonLodState` changes (including fresh
/// inserts after a respawn), via `Changed<SkeletonLodState>`.
#[allow(clippy::type_complexity)]
pub(crate) fn sync_skeleton_lod_subtrees(
    characters: Query<
        (Entity, &SkeletonLodState, &CharacterSkeleton),
        Changed<SkeletonLodState>,
    >,
    lod_config: Option<Res<SkeletonLodConfig>>,
    rig_data: Res<RigData>,
    mut commands: Commands,
    bones: Query<
        (
            &Transform,
            &GlobalTransform,
            Has<SkeletonLodDisabled>,
        ),
        Allow<SkeletonLodDisabled>,
    >,
) {
    let Some(config) = lod_config else {
        return;
    };
    let config_count = config.1;
    let Some(rig_spec) = rig_data.0.as_ref() else {
        return;
    };
    let bone_parents = &rig_spec.reference_rig.bone_parents;

    for (_entity, state, skeleton) in &characters {
        let active = state.active;

        if !active.iter().any(|&a| a) {
            continue;
        }

        // Union (per the intersection rule over anchors) of every removed bone
        // name: descendants of each anchor that is present in all active LODs.
        let mut disabled: AHashSet<&'static str> = AHashSet::default();
        for k in 0..config_count {
            if !active[k] {
                continue;
            }
            for anchor in &config.0[k].without_children_of {
                let present_in_all = (0..config_count)
                    .filter(|&m| active[m])
                    .all(|m| config.0[m].without_children_of.iter().any(|r| r == anchor));
                if !present_in_all {
                    continue;
                }
                for descendant in all_children_of(bone_parents, anchor) {
                    disabled.insert(descendant);
                }
            }
        }

        // Apply: disable removed bones, re-enable everything else.
        for (&bone_name, &bone_entity) in skeleton.bone_map.iter() {
            let should_disable = disabled.contains(bone_name);
            let Ok((transform, global_transform, is_disabled)) = bones.get(bone_entity) else {
                continue;
            };
            if should_disable {
                if !is_disabled {
                    commands.entity(bone_entity).insert(SkeletonLodDisabled);
                }
            } else if is_disabled {
                // Re-enable: force `Changed<Transform>` (so transform propagation
                // recomputes the tree) AND `Changed<GlobalTransform>` (so the skin
                // extraction re-samples the joint). Removing the disabling
                // component alone marks neither. Re-inserting marks changed
                // regardless of value equality, which matters because propagation's
                // `set_if_neq` won't fire when the frozen global transform is already
                // correct, leaving a stale `IDENTITY` joint matrix in the render
                // staging buffer and detaching the mesh.
                commands
                    .entity(bone_entity)
                    .insert((transform.clone(), global_transform.clone()))
                    .remove::<SkeletonLodDisabled>();
            }
        }
    }
}

/// Sets up `SkinnedMesh` on each `CharacterPart` from the single skeleton:
/// the joints are the surviving bones for the part's LOD level, and the inverse
/// bindposes are the corresponding subset of the full skeleton's.
#[allow(clippy::type_complexity)]
pub(crate) fn setup_part_skinning(
    parts: Query<(Entity, &ChildOf, &CharacterPart), Without<SkinnedMesh>>,
    characters: Query<(&CharacterSkeleton, &SkeletonsReady)>,
    rig_bundle: Res<RigBundleRes>,
    rig_data: Res<RigData>,
    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut commands: Commands,
) {
    for (part_entity, parent, part) in &parts {
        let parent_entity = parent.parent();
        let Ok((skeleton, _)) = characters.get(parent_entity) else {
            continue;
        };
        let Some(rig_spec) = rig_data.0.as_ref() else {
            continue;
        };
        let Some(lod_data) = rig_bundle.bundle.as_ref().map(|b| &b.lod_data) else {
            continue;
        };

        let idx = part.skeleton_lod.min(lod_data.len().saturating_sub(1));
        let bone_names = &lod_data[idx].bone_names;

        // Map surviving bone names to entities and their inverse-bindpose subset.
        let mut joints = Vec::with_capacity(bone_names.len());
        let mut inv_bindposes = Vec::with_capacity(bone_names.len());
        let mut valid = true;
        for &bone_name in bone_names {
            let Some(&joint) = skeleton.bone_map.get(bone_name) else {
                valid = false;
                break;
            };
            let Some(ref_idx) = rig_spec.bone_index(bone_name) else {
                valid = false;
                break;
            };
            joints.push(joint);
            inv_bindposes.push(skeleton.model_space_inv_bindposes[ref_idx]);
        }

        if !valid || joints.is_empty() {
            continue;
        }

        let inv_bindpose_handle = inv_bindpose_assets.add(inv_bindposes);
        commands.entity(part_entity).insert(SkinnedMesh {
            joints,
            inverse_bindposes: inv_bindpose_handle,
        });
    }
}

/// Observer that strips a character's skeleton and mesh state when its `HelperVertexPositions`
/// are removed (e.g. the character went off-screen and gave up its per-vertex data).
///
/// Physics/ragdoll cleanup is handled separately by the avian observer on the same
/// trigger. Users can register their own `On<Remove, HelperVertexPositions>` observers to clean up
/// additional per-character data that humentity can't know about generically, such as
/// material handles.
pub(crate) fn on_character_helpers_removed(
    trigger: On<Remove, HelperVertexPositions>,
    characters: Query<Option<&CharacterSkeleton>, With<CharacterShape>>,
    parts: Query<(Entity, &ChildOf, &CharacterPart)>,
    mut commands: Commands,
) {
    let entity = trigger.entity;
    if characters.get(entity).is_err() {
        return;
    }

    if let Ok(Some(skeleton)) = characters.get(entity) {
        commands.entity(skeleton.skeleton_entity).despawn();
    }
    commands
        .entity(entity)
        .remove::<CharacterSkeleton>()
        .remove::<SkeletonLodState>()
        .remove::<SkeletonsReady>();

    // Strip mesh handles from the character's parts so they don't dangle against
    // the despawned skeleton joints.
    for (part_entity, parent, _part) in &parts {
        if parent.parent() == entity {
            commands
                .entity(part_entity)
                .remove::<Mesh3d>()
                .remove::<SkinnedMesh>()
                .remove::<MeshMorphWeights>();
        }
    }
}
