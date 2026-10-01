use crate::{
    animation::BoneTranslationCorrection,
    basemesh::VertexGroups,
    helpers::{RefitCharacter, TeardownCharacter},
    prelude::*,
    rigs::{BuiltRigs, RigBundleRes, RigData, SkeletonRootBone},
    skeleton_lod::{MAX_LODS, MergeIntoKeptBone, SkeletonLodConfig, all_children_of},
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
    /// Per-bone translation corrections, cached at fit time. Consumed per frame
    /// by `rescale_dynamic_retargeting` (characters with `DynamicRetargeting`)
    /// and at import by the shape-bake loader. Always computed (one-time cost
    /// per fit); `RootOnly` and unmarked characters simply never read it.
    pub translation_corrections: Vec<BoneTranslationCorrection>,
}

/// Entity event: snap a character's bones back to the fitted bind pose.
/// Fire after ragdoll ends (`commands.trigger(ResetToBindPose(character))`):
/// ragdoll writes both positions and rotations onto the bones, but animation
/// clips only drive rotations, so stale translations survive and the pose
/// looks close-but-off. This restores the fitted bind-pose translations and
/// rotations; the clip then takes over rotations from a clean base.
#[derive(Event, Debug, Clone, Copy)]
pub struct ResetToBindPose(pub Entity);

/// Placed on `CharacterShape` once its single skeleton has been fitted.
#[derive(Component, Debug)]
pub struct SkeletonsReady;

/// Tracks which skeleton LOD levels are currently active (in use by a shown mesh).
///
/// The reconcile system (`sync_skeleton_lod_subtrees`) disables a bone sub-tree
/// iff its root is present in **every** active LOD's cumulative remove list.
#[derive(Component, Debug, Clone, Copy)]
pub struct SkeletonLodState {
    pub active: [bool; MAX_LODS],
}

/// Uniform visual scale for a character. Place on the `CharacterShape`
/// entity; the skeleton root copies it at spawn (and keeps it across fits),
/// so bones spread out and the skinned mesh follows. Defaults to 1.
#[derive(Component, Debug, Clone, Copy)]
pub struct CharacterScale(pub f32);

impl Default for CharacterScale {
    fn default() -> Self {
        Self(1.0)
    }
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


/// Skeleton root transform: face -Z (model verts face +Z), keep
/// `CharacterScale`, and shift the default rig rearward (see
/// [`crate::rigs::skeleton_rear_offset_meters`]).
fn skeleton_root_transform(rig_name: &str, character_scale: Vec3) -> Transform {
    Transform::from_rotation(crate::MODEL_ROTATION_FIX)
        .with_scale(character_scale)
        .with_translation(Vec3::new(
            0.0,
            0.0,
            crate::rigs::skeleton_rear_offset_meters(rig_name) * character_scale.x,
        ))
}

/// Spawns the single full skeleton scene as a child of each `CharacterShape`,
/// and marks the character as needing a fit.
pub(crate) fn spawn_rig_skeleton(
    characters: Query<(Entity, Option<&CharacterScale>), (Without<SkeletonsReady>, Without<FitSkeleton>, With<CharacterShape>)>,
    rig_bundle: Res<RigBundleRes>,
    rig_data: Res<RigData>,
    mut commands: Commands,
) {
    for (entity, scale) in &characters {
        let Some(scene) = rig_bundle.scene.clone() else {
            continue;
        };

        let scale = scale.map_or(Vec3::ONE, |s| Vec3::splat(s.0));
        let rig_name = rig_data
            .0
            .as_ref()
            .map_or("", |rig_spec| rig_spec.reference_rig().rig_name.as_str());
        let skeleton_entity = commands
            .spawn((
                DynamicWorldRoot::from(scene),
                skeleton_root_transform(rig_name, scale),
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
                translation_corrections: Vec::new(),
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
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    basemesh: Res<BaseMesh>,
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
            &mut CharacterSkeleton,
            Option<&CharacterScale>,
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
    for (entity, character_shape, animation_player, skeleton, scale) in
        characters.iter_mut()
    {
        let Some(shape_asset) = shape_assets.get(&character_shape.0) else {
            continue;
        };
        let Some(template) = templates.get(&shape_asset.template) else {
            continue;
        };
        if basemesh.vertices.is_empty() {
            continue;
        }
        // All shapes must carry baked deltas before blending; the template
        // bake retries until they do, so this waits for it.
        if template.shapes.iter().any(|shape| shape.helper_deltas.is_none()) {
            continue;
        }
        // Blend this character's morphed helpers on demand: base plus each
        // shape's baked delta times the character's weight. Cheap (~13k
        // verts), synchronous, no per-character background task or cache.
        let helpers = template.blend_helpers(&shape_asset.template_morph_targets, &basemesh.vertices);
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

        // Re-fit skeleton to mesh shape (shared with the GPU shape fit).
        let model_space_bindposes = crate::rigs::fitted_model_space_bindposes(&helpers, rig_spec, &vg);

        let bone_config = &rig_spec.config;

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
        // Cache per-bone translation corrections for `DynamicRetargeting`
        // (per frame) and the shape bake (at import). Always computed:
        // one-time cost per fit; others never read it. Every known bone gets
        // an entry via the shared `bone_translation_correction` helper
        // (zero-length reference bones are identity), so bake, dynamic, and
        // GPU agree on short bones.
        let reference_rig = rig_spec.reference_rig();
        let mut translation_corrections = Vec::new();
        for &bone_name in &reference_rig.bone_names {
            let Some(bone_config_entry) = bone_config.bones.get(bone_name) else {
                continue;
            };
            if bone_config_entry.parent.is_empty() {
                continue; // Root bone owned by `rescale_root_bone_translation`.
            }
            let Some(&bone_entity) = bone_entities.get(bone_name) else {
                continue;
            };
            let Some((translation_length_ratio, translation_direction_adjust)) =
                crate::animation::bone_translation_correction(
                    reference_rig,
                    &bone_config_entry.parent,
                    bone_name,
                    &model_space_bindposes,
                )
            else {
                continue;
            };
            translation_corrections.push(BoneTranslationCorrection {
                bone_entity,
                translation_length_ratio,
                translation_direction_adjust,
            });
        }


        // Compute full inverse bindposes in reference bone order.
        let model_space_inv_bindposes: Vec<Mat4> = rig_spec
            .reference_rig
            .bone_names
            .iter()
            .map(|&bone| model_space_bindposes[bone].to_matrix().inverse())
            .collect();
        // Rotate skeleton to face -Z (model verts face +Z) and shift the
        // default rig rearward (see `skeleton_root_transform`), keeping the
        // CharacterScale copied at spawn.
        let scale = scale.map_or(Vec3::ONE, |s| Vec3::splat(s.0));
        commands
            .entity(skeleton.skeleton_entity)
            .insert(skeleton_root_transform(
                &rig_spec.reference_rig().rig_name,
                scale,
            ));

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
            translation_corrections,
        });
        commands
            .entity(skeleton.skeleton_entity)
            .insert(SkeletonRootBone {
                entity: root_bone_entity,
                root_scale,
                bind_pose_y: fitted_root_y,
                reference_bind_pose_y: reference_root_y,
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

/// Panics if `SkeletonLodConfig` is mutated or replaced after the rig build.
///
/// `SkeletonLodConfig` is upload-once: insert it before the rig build and never
/// touch it afterwards. Mesh weights are baked from the build-time config, so a
/// new config would silently disagree with every built mesh.
///
/// First-sight grace: the build and the guard run in the same schedule, so on
/// the first frame `BuiltRigs` is visible the pre-build insert may still read
/// as changed. That frame is graced once; later frames panic on any change.
pub(crate) fn enforce_skeleton_lod_config_frozen(
    lod_config: Option<Res<SkeletonLodConfig>>,
    built_rigs: Option<Res<BuiltRigs>>,
    mut seen_built: Local<bool>,
) {
    let Some(config) = lod_config else {
        return;
    };
    if built_rigs.is_none() {
        return;
    };
    if !*seen_built {
        // First frame the build is visible: the pre-build insert may still
        // read as changed this frame. Grace it once; real post-build
        // mutations still read changed on later frames and panic below.
        *seen_built = true;
        return;
    }
    if config.is_changed() {
        panic!("SkeletonLodConfig changed after the rig build: upload it once before the build and never mutate or replace it (mesh weights are baked from the build-time config)");
    }
}

/// Reconciles the enabled/disabled bone sub-trees against the active LOD set.
///
/// A bone sub-tree is disabled iff its **anchor** root is present in *every*
/// active LOD's cumulative remove list (the intersection of active LOD lists).
/// Only the removed *descendants* of each anchor are disabled — the anchor bone
/// itself always survives (it is the merge target that the removed bones' weights
/// are folded into). For example LOD 0 (`merge_default_rig_toes`) disables every
/// toe bone except `toe1-1.*` but keeps `foot.*` and the kept toe enabled so the
/// skinned mesh does not collapse to the origin. The same holds for a kept-bone
/// merge: the kept bone itself always stays enabled.
///
/// If *no* LOD is active (no mesh in range needs joints), every bone in
/// `bone_map` — anchors included — is disabled, parking the whole skeleton
/// until a mesh comes back into range.
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
        for (level, &level_active) in active.iter().enumerate() {
            if level_active && level >= config_count {
                panic!("SkeletonLodState activates LOD {level} but SkeletonLodConfig only built {config_count} LOD levels: upload the full config before the rig build (mesh weights and bone disables are baked from it)");
            }
        }

        // No LOD active means no mesh in range needs joints: park the whole
        // skeleton by disabling every bone, anchors included. The
        // anchor-intersection loops below yield nothing over zero actives, so
        // this explicit branch owns the all-false case.
        let mut disabled: AHashSet<&'static str> = AHashSet::default();
        if !active.iter().any(|&a| a) {
            disabled.extend(skeleton.bone_map.keys().copied());
        }

        // Union (per the intersection rule) of every removed bone name:
        // descendants of each anchor present in all active LODs, plus the
        // descendants (minus the kept bone) of each kept-merge parent present
        // in all active LODs. A kept merge is void when an anchor on its
        // parent or any ancestor is also effective: the whole sub-tree, kept
        // bone included, folds into that anchor instead. This mirrors
        // `resolve_remove_set`, which the mesh builds use.
        let mut effective_anchors: Vec<&str> = Vec::new();
        let mut effective_kept_merges: Vec<&MergeIntoKeptBone> = Vec::new();
        for k in 0..config_count {
            if !active[k] {
                continue;
            }
            for anchor in &config.0[k].without_children_of {
                let present_in_all = (0..config_count)
                    .filter(|&m| active[m])
                    .all(|m| config.0[m].without_children_of.iter().any(|r| r == anchor));
                if present_in_all
                    && !effective_anchors
                        .iter()
                        .any(|&known_anchor| known_anchor == anchor.as_str())
                {
                    effective_anchors.push(anchor);
                }
            }
            for kept_merge in &config.0[k].merge_into_kept_bone {
                let present_in_all = (0..config_count).filter(|&m| active[m]).all(|m| {
                    config.0[m]
                        .merge_into_kept_bone
                        .iter()
                        .any(|other_merge| other_merge == kept_merge)
                });
                if present_in_all
                    && !effective_kept_merges
                        .iter()
                        .any(|&known_kept_merge| known_kept_merge == kept_merge)
                {
                    effective_kept_merges.push(kept_merge);
                }
            }
        }
        for &anchor in &effective_anchors {
            for descendant_bone in all_children_of(bone_parents, anchor) {
                disabled.insert(descendant_bone);
            }
        }
        for &kept_merge in &effective_kept_merges {
            let parent_bone_name = NAME_INTERNER.intern(&kept_merge.parent_bone_name).leak();
            let swallowed_by_anchor = effective_anchors.iter().any(|&anchor| {
                all_children_of(bone_parents, anchor)
                    .iter()
                    .any(|&descendant_bone| descendant_bone == parent_bone_name)
            });
            if swallowed_by_anchor {
                continue;
            }
            let kept_bone_name = NAME_INTERNER.intern(&kept_merge.kept_bone_name).leak();
            // A kept merge takes precedence over an anchor on its own parent
            // (mirrors `resolve_remove_set`): the kept bone stays enabled.
            disabled.remove(kept_bone_name);
            for descendant_bone in all_children_of(bone_parents, parent_bone_name) {
                if descendant_bone == kept_bone_name {
                    continue;
                }
                disabled.insert(descendant_bone);
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
                    .insert((*transform, *global_transform))
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
    parents: Query<&ChildOf>,
    characters: Query<(&CharacterSkeleton, &SkeletonsReady)>,
    rig_bundle: Res<RigBundleRes>,
    rig_data: Res<RigData>,
    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut commands: Commands,
) {
    for (part_entity, _, part) in &parts {
        // Parts may sit under intermediate grouping nodes (e.g. a "CPU Meshes"
        // or "GPU Meshes" child), so walk up until the skeleton owner.
        // `iter_ancestors` yields the direct parent first, so flat parts work too.
        let skeleton = parents
            .iter_ancestors::<ChildOf>(part_entity)
            .find_map(|ancestor| characters.get(ancestor).ok());
        let Some((skeleton, _)) = skeleton else {
            continue;
        };
        let Some(rig_spec) = rig_data.0.as_ref() else {
            continue;
        };
        let Some(lod_data) = rig_bundle.bundle.as_ref().map(|b| &b.lod_data) else {
            continue;
        };
        let Some(variant) = lod_data.get(part.skeleton_lod) else {
            panic!("CharacterPart asks for skeleton LOD {} but the rig build only produced {} levels: upload the full SkeletonLodConfig before the build (mesh weights and bone disables are baked from it)", part.skeleton_lod, lod_data.len());
        };
        let bone_names = &variant.bone_names;

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

/// Observer that strips a character's skeleton and mesh state on
/// [`TeardownCharacter`] or [`RefitCharacter`] (e.g. retire or re-fit after
/// morph changes).
///
/// Physics/ragdoll cleanup is handled separately by the avian observer on the
/// same trigger. Users can observe the same events to clean up additional
/// per-character data that humentity can't know about generically, such as
/// material handles.
pub(crate) fn on_teardown_character(
    trigger: On<TeardownCharacter>,
    characters: Query<Option<&CharacterSkeleton>, With<CharacterShape>>,
    parts: Query<Entity, With<CharacterPart>>,
    gpu_parts: Query<Entity, With<GpuCharacterPart>>,
    descendants: Query<&Children>,
    mut commands: Commands,
) {
    teardown_character_state(trigger.event().0, &characters, &parts, &gpu_parts, &descendants, &mut commands);
}

/// Observer for [`RefitCharacter`]: same teardown as [`TeardownCharacter`],
/// plus re-inserts `FitSkeleton` so the spawn/fit chain rebuilds the
/// character from current morph weights on the next frames.
pub(crate) fn on_refit_character(
    trigger: On<RefitCharacter>,
    characters: Query<Option<&CharacterSkeleton>, With<CharacterShape>>,
    parts: Query<Entity, With<CharacterPart>>,
    gpu_parts: Query<Entity, With<GpuCharacterPart>>,
    descendants: Query<&Children>,
    mut commands: Commands,
) {
    let entity = trigger.event().0;
    teardown_character_state(entity, &characters, &parts, &gpu_parts, &descendants, &mut commands);
    commands.entity(entity).insert(FitSkeleton);
}

fn teardown_character_state(
    entity: Entity,
    characters: &Query<Option<&CharacterSkeleton>, With<CharacterShape>>,
    parts: &Query<Entity, With<CharacterPart>>,
    gpu_parts: &Query<Entity, With<GpuCharacterPart>>,
    descendants: &Query<&Children>,
    commands: &mut Commands,
) {
    let Ok(skeleton) = characters.get(entity) else {
        return;
    };
    if let Some(skeleton) = skeleton {
        commands.entity(skeleton.skeleton_entity).despawn();
    }
    commands
        .entity(entity)
        .remove::<CharacterSkeleton>()
        .remove::<SkeletonLodState>()
        .remove::<SkeletonsReady>();
    // Strip mesh handles from the character's parts so they don't dangle against
    // the despawned skeleton joints. Descend from the character to its own
    // parts (grouping nodes included) instead of scanning every part in the
    // scene and testing belonging upward.
    for descendant in descendants.iter_descendants(entity) {
        if parts.get(descendant).is_ok() {
            commands
                .entity(descendant)
                .remove::<Mesh3d>()
                .remove::<SkinnedMesh>()
                .remove::<MeshMorphWeights>();
        }
        // GPU parts hold static meshes with no skinning, but still drop the
        // handle so a refit re-attaches a fresh one.
        if gpu_parts.get(descendant).is_ok() {
            commands.entity(descendant).remove::<Mesh3d>();
        }
    }
}

/// Observer for [`ResetToBindPose`]: recompute the fitted bind-pose locals
/// from the morphed helpers and write them back onto every bone. The fit math
/// is the same two passes as `fit_skeleton_to_shape` (model-space from
/// helpers, then parent-relative locals), but skips rebuilding `bone_map` /
/// inverse bindposes — those don't change after the first fit.
pub(crate) fn on_reset_to_bind_pose(
    trigger: On<ResetToBindPose>,
    characters: Query<(&CharacterShape, &CharacterSkeleton)>,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    basemesh: Res<BaseMesh>,
    mut local_transforms: Query<&mut Transform, (Without<CharacterShape>, Allow<SkeletonLodDisabled>)>,
    rig_data: Res<RigData>,
    vg: Res<VertexGroups>,
) {
    let character = trigger.event().0;
    let Ok((character_shape, skeleton)) = characters.get(character) else {
        return;
    };
    let Some(shape_asset) = shape_assets.get(&character_shape.0) else {
        return;
    };
    let Some(template) = templates.get(&shape_asset.template) else {
        return;
    };
    if basemesh.vertices.is_empty()
        || template.shapes.iter().any(|shape| shape.helper_deltas.is_none())
    {
        return;
    };
    let blended_helpers =
        template.blend_helpers(&shape_asset.template_morph_targets, &basemesh.vertices);
    let Some(rig_spec) = rig_data.0.as_ref() else {
        return;
    };
    let model_space_bindposes =
        crate::rigs::fitted_model_space_bindposes(&blended_helpers, rig_spec, &vg);
    let bone_config = &rig_spec.config;
    for &bone in &rig_spec.reference_rig.bone_names {
        let Some(bone_data) = bone_config.bones.get(bone) else {
            continue;
        };
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
        if let Some(&joint) = skeleton.bone_map.get(bone)
            && let Ok(mut local_transform) = local_transforms.get_mut(joint)
        {
            *local_transform = new_local;
        }
    }
}
