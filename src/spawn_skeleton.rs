use ahash::AHashSet;

use crate::{
    basemesh::{BaseMesh, VertexGroups},
    prelude::*,
    rigs::{
        get_model_space_skeleton_transforms, BoneTranslationData, RigBundleRes, RigData,
        RootBonePrevious,
    },
    skeleton_lod::SkeletonLodConfig,
};
use ahash::AHashMap;
use bevy::{
    animation::AnimatedBy,
    ecs::intern::Internable,
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};

/// Placed on each rig scene entity to identify it as a character skeleton at a given LOD level.
#[derive(Component, Debug)]
pub struct CharacterSkeleton {
    pub lod: usize,
}

/// Placed on `CharacterShape` once all `CharacterSkeleton` children have been fitted.
#[derive(Component, Debug)]
pub struct SkeletonsReady;

/// Optional filter on `CharacterShape` to restrict which LOD levels get spawned.
/// `None` means spawn all LOD levels defined in `SkeletonLodConfig`.
#[derive(Component, Debug, Default)]
pub struct SkeletonLodFilter(pub Option<AHashSet<usize>>);

/// Maps LOD index → skeleton entity. Placed on `CharacterShape` by `spawn_rig_skeletons`.
#[derive(Component, Debug, Default)]
pub struct SkeletonLodMap(pub AHashMap<usize, Entity>);

/// Internal marker on a skeleton entity that has been spawned but not yet fitted to morphs.
#[derive(Component, Debug)]
pub(crate) struct FitSkeleton;

/// Custom disabling component for skeleton LOD entities.
/// Registered as a disabling component so Bevy's default query filters exclude it.
/// Used instead of [`Disabled`] to avoid conflicting with other systems.
#[derive(Component, Clone, Debug, Default)]
pub struct SkeletonLodDisabled;

/// Event to enable a skeleton LOD variant on a character.
/// Removes `SkeletonLodDisabled` from the skeleton entity and all its descendants.
#[derive(Event, Debug, Clone)]
pub struct EnableSkeletonLod {
    pub character: Entity,
    pub lod: usize,
}

/// Event to disable a skeleton LOD variant on a character.
/// Inserts `SkeletonLodDisabled` on the skeleton entity and all its descendants.
#[derive(Event, Debug, Clone)]
pub struct DisableSkeletonLod {
    pub character: Entity,
    pub lod: usize,
}

/// Local-space bind pose transforms for each joint in a skeleton.
/// Indexed to match `SkinnedMesh.joints`. Stored on each `CharacterSkeleton` entity during fitting.
#[derive(Component, Debug, Clone)]
pub struct SkeletonLocalBindPose(pub Vec<Transform>);

/// Event to reset a skeleton's bones to their fitted bind pose.
/// Useful when re-enabling a skeleton that was mid-animation when disabled.
#[derive(Event, Debug, Clone)]
pub struct ResetSkeletonToBindPose {
    pub character: Entity,
    pub lod: usize,
}

/// Spawns one skeleton scene per unique LOD level as a child of each `CharacterShape`.
/// LOD levels are determined from `SkeletonLodConfig`, not from `CharacterPart` children.
pub(crate) fn spawn_rig_skeletons(
    characters: Query<Entity, (Without<SkeletonsReady>, Without<FitSkeleton>)>,
    has_skeleton_children: Query<&SkeletonLodMap>,
    rig_bundle: Res<RigBundleRes>,
    lod_config: Option<Res<SkeletonLodConfig>>,
    filter_query: Query<Option<&SkeletonLodFilter>>,
    mut commands: Commands,
) {
    for entity in &characters {
        // Skip if we already spawned skeletons (SkeletonLodMap present) but haven't fitted yet
        if has_skeleton_children.get(entity).is_ok() {
            continue;
        }
        let Some(bundle) = rig_bundle.0.as_ref() else {
            continue;
        };

        // Determine LOD levels from SkeletonLodConfig
        let num_variants = lod_config.as_ref().map(|c| c.0.len()).unwrap_or(1);

        // Apply filter if present
        let lod_filter = filter_query.get(entity).ok().flatten();

        let mut lod_map = AHashMap::default();

        for lod_idx in 0..num_variants {
            if let Some(SkeletonLodFilter(Some(allowed))) = lod_filter {
                if !allowed.contains(&lod_idx) {
                    continue;
                }
            }

            let variant_idx = lod_idx.min(bundle.lod_variants.len() - 1);
            let variant = &bundle.lod_variants[variant_idx];
            let scene = variant.scene.clone();

            let skeleton_entity = commands
                .spawn((
                    DynamicWorldRoot::from(scene),
                    CharacterSkeleton { lod: lod_idx },
                    FitSkeleton,
                    Name::new(format!("Skeleton LOD {lod_idx}")),
                ))
                .id();

            commands.entity(entity).add_child(skeleton_entity);
            lod_map.insert(lod_idx, skeleton_entity);
        }

        commands.entity(entity).insert(SkeletonLodMap(lod_map));
    }
}

/// Fits each skeleton's bone transforms to the character's morph shape.
/// Operates on `CharacterSkeleton + FitSkeleton` entities, not on `CharacterPart`.
#[allow(clippy::type_complexity)]
pub(crate) fn fit_skeleton_to_shape(
    mut commands: Commands,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    skinned_meshes: Query<
        &SkinnedMesh,
        (Without<Mesh3d>, With<ChildOf>, Allow<SkeletonLodDisabled>),
    >,
    children: Query<&Children, Allow<SkeletonLodDisabled>>,
    skeletons: Query<
        (Entity, &ChildOf, &CharacterSkeleton, Option<&RootMotion>),
        (With<FitSkeleton>, Allow<SkeletonLodDisabled>),
    >,
    characters: Query<(&CharacterShape, Option<&AnimationPlayer>)>,
    mut local_transforms: Query<
        &mut Transform,
        (Without<CharacterShape>, Allow<SkeletonLodDisabled>),
    >,
    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    basemesh: Res<BaseMesh>,
    morph_targets: Res<MakeHumanMorphs>,
    rig_data: Res<RigData>,
    vg: Res<VertexGroups>,
    rig_bundle: Res<RigBundleRes>,
) {
    for (skeleton_entity, parent, skeleton, root_motion) in skeletons.iter() {
        let parent_entity = parent.parent();
        let Ok((character_shape, animation_player)) = characters.get(parent_entity) else {
            continue;
        };
        let Some(mut asset) = shape_assets.get_mut(&character_shape.0) else {
            continue;
        };
        let Some(template) = templates.get(&asset.template) else {
            continue;
        };
        let Ok(helpers) = template.get_helpers(
            &asset.template_morph_targets,
            &basemesh.vertices,
            &morph_targets,
        ) else {
            continue;
        };

        let Some(rig_spec) = rig_data.0.as_ref() else {
            continue;
        };

        // Determine which bone names and parent mapping to use for this LOD variant
        let (variant_bone_names, variant_bone_to_parent): (
            Vec<&'static str>,
            AHashMap<&'static str, &'static str>,
        ) = rig_bundle
            .0
            .as_ref()
            .and_then(|bundle| {
                let idx = skeleton.lod.min(bundle.lod_variants.len() - 1);
                let variant = &bundle.lod_variants[idx];
                Some((variant.bone_names.clone(), variant.bone_to_parent.clone()))
            })
            .unwrap_or_else(|| {
                let bone_to_parent: AHashMap<&'static str, &'static str> = rig_spec
                    .reference_rig
                    .bone_parents
                    .iter()
                    .map(|(&name, parent)| {
                        let parent_leaked = if parent.is_empty() {
                            ""
                        } else {
                            NAME_INTERNER.intern(parent).leak()
                        };
                        (name, parent_leaked)
                    })
                    .collect();
                (rig_spec.reference_rig.bone_names.clone(), bone_to_parent)
            });

        // Find the rig scene child (DynamicWorld) that was spawned for this skeleton
        let mut rig_entity: Option<Entity> = None;
        let mut skinned_mesh: Option<SkinnedMesh> = None;
        for child in children.iter_descendants(skeleton_entity) {
            if let Ok(rig) = skinned_meshes.get(child) {
                rig_entity = Some(child);
                skinned_mesh = Some(rig.clone());
            }
        }

        let (Some(rig_entity), Some(skm)) = (rig_entity, skinned_mesh) else {
            continue;
        };

        // Build bone_entities from SkinnedMesh.joints (already entity-mapped during
        // DynamicWorld spawning) and the variant's bone_names order.
        // skm.joints is ordered by variant bone_names, not the full reference rig.
        let mut bone_entities = AHashMap::default();
        for (i, &bone_name) in variant_bone_names.iter().enumerate() {
            if let Some(&joint_entity) = skm.joints.get(i) {
                bone_entities.insert(bone_name, joint_entity);
            }
        }

        // Filter to only the variant bone names we actually need
        let variant_set: AHashSet<&str> = variant_bone_names.iter().copied().collect();
        bone_entities.retain(|k, _| variant_set.contains(*k));

        if bone_entities.is_empty() {
            continue;
        }

        if animation_player.is_some() {
            for &bone_entity in bone_entities.values() {
                commands
                    .entity(bone_entity)
                    .insert(AnimatedBy(parent_entity));
            }
        }

        // Re-fit skeleton to mesh shape
        let mut model_space_bindposes = get_model_space_skeleton_transforms(
            &rig_spec.reference_rig.bone_names,
            &helpers,
            rig_spec,
            &vg,
        );
        let mut local_bone_transforms = AHashMap::default();

        let bone_config = &rig_spec.config;
        for &bone in &rig_spec.reference_rig.bone_names {
            if let Some(bone_data) = bone_config.bones.get(bone) {
                let parent_name = variant_bone_to_parent
                    .get(bone)
                    .copied()
                    .unwrap_or_else(|| NAME_INTERNER.intern(&bone_data.parent).leak());
                let parent_transform = if parent_name.is_empty() {
                    Transform::IDENTITY
                } else {
                    match model_space_bindposes.get(parent_name) {
                        Some(xform) => *xform,
                        None => Transform::IDENTITY,
                    }
                };
                let old_global = model_space_bindposes[bone];

                let reference_rot = rig_spec.reference_rig.model_space_bindpose[bone].rotation;
                let new_global = Transform {
                    translation: old_global.translation,
                    rotation: reference_rot,
                    scale: old_global.scale,
                };

                let parent_matrix = parent_transform.to_matrix();
                let child_matrix = new_global.to_matrix();
                let new_local = Transform::from_matrix(parent_matrix.inverse() * child_matrix);

                local_bone_transforms.insert(bone, new_local);
                model_space_bindposes.insert(bone, new_global);

                if let Some(&joint) = bone_entities.get(bone) {
                    let mut local_transform = local_transforms.get_mut(joint).unwrap();
                    *local_transform = new_local;
                }
            }
        }

        let local_bind_pose: Vec<Transform> = variant_bone_names
            .iter()
            .filter_map(|bone| local_bone_transforms.get(bone).copied())
            .collect();

        // Cache per-bone translation data for animation rescaling (computed once per asset)
        if matches!(asset.bone_translations, BoneTranslationData::None) {
            let translations = model_space_bindposes
                .iter()
                .map(|(&bone, xform)| (bone, xform.translation))
                .collect();
            asset.bone_translations = BoneTranslationData::Full(translations);
            asset.bone_delta_rotations = AHashMap::<&'static str, Quat>::default();
        }

        // Compute inverse bindposes from fitted transforms for this LOD variant
        let model_space_inv_bindposes = model_space_bindposes
            .iter()
            .map(|(&bone, transform)| {
                (
                    bone,
                    Transform::from_matrix(transform.to_matrix().inverse()),
                )
            })
            .collect::<AHashMap<_, _>>();

        let mut inv_bindposes = vec![];
        for &bone in variant_bone_names.iter() {
            inv_bindposes.push(model_space_inv_bindposes[bone].to_matrix());
        }

        // Update the rig entity's SkinnedMesh with correct inverse bindposes
        let new_inv_handle = inv_bindpose_assets.add(inv_bindposes);
        commands.entity(rig_entity).insert(SkinnedMesh {
            joints: skm.joints.clone(),
            inverse_bindposes: new_inv_handle,
        });

        // Rotate skeleton to face -Z (model verts face +Z)
        commands
            .entity(skeleton_entity)
            .insert(Transform::from_rotation(crate::MODEL_ROTATION_FIX));

        // Handle root motion
        if let Some(_root_motion) = root_motion {
            let root_bone = bone_entities[variant_bone_names[0]];
            commands
                .entity(root_bone)
                .insert(RootBonePrevious::default());
        }

        // Remove FitSkeleton — this skeleton is now fitted, start disabled by default
        commands
            .entity(skeleton_entity)
            .insert(SkeletonLocalBindPose(local_bind_pose))
            .remove::<FitSkeleton>()
            .insert_recursive::<Children>(SkeletonLodDisabled);
    }
}

/// Once all `CharacterSkeleton` children of a `CharacterShape` have been fitted
/// (no longer have `FitSkeleton`), insert `SkeletonsReady`.
pub(crate) fn check_skeletons_ready(
    mut commands: Commands,
    characters: Query<(Entity, &SkeletonLodMap), Without<SkeletonsReady>>,
    fitted: Query<&CharacterSkeleton, (Without<FitSkeleton>, Allow<SkeletonLodDisabled>)>,
) {
    for (entity, lod_map) in &characters {
        let all_fitted = lod_map.0.values().all(|e| fitted.get(*e).is_ok());
        if all_fitted && !lod_map.0.is_empty() {
            commands.entity(entity).insert(SkeletonsReady);
        }
    }
}

/// Observer that enables a skeleton LOD variant by removing `SkeletonLodDisabled`.
/// Automatically triggers a bind pose reset so bones start clean.
pub(crate) fn on_enable_skeleton_lod(
    trigger: On<EnableSkeletonLod>,
    lod_map_query: Query<&SkeletonLodMap>,
    mut commands: Commands,
) {
    let event = trigger.event();
    let Ok(lod_map) = lod_map_query.get(event.character) else {
        return;
    };
    let Some(&skeleton_entity) = lod_map.0.get(&event.lod) else {
        return;
    };
    commands
        .entity(skeleton_entity)
        .remove_recursive::<Children, SkeletonLodDisabled>();
    commands.trigger(ResetSkeletonToBindPose {
        character: event.character,
        lod: event.lod,
    });
}

/// Observer that disables a skeleton LOD variant by inserting `SkeletonLodDisabled`.
pub(crate) fn on_disable_skeleton_lod(
    trigger: On<DisableSkeletonLod>,
    lod_map_query: Query<&SkeletonLodMap>,
    mut commands: Commands,
) {
    let event = trigger.event();
    let Ok(lod_map) = lod_map_query.get(event.character) else {
        return;
    };
    let Some(&skeleton_entity) = lod_map.0.get(&event.lod) else {
        return;
    };
    commands
        .entity(skeleton_entity)
        .insert_recursive::<Children>(SkeletonLodDisabled);
}

/// Observer that resets a skeleton's bones to their fitted bind pose.
/// Called automatically on enable, or can be triggered standalone.
#[allow(clippy::type_complexity)]
pub(crate) fn on_reset_skeleton_to_bind_pose(
    trigger: On<ResetSkeletonToBindPose>,
    lod_map_query: Query<&SkeletonLodMap>,
    bind_pose_query: Query<&SkeletonLocalBindPose, Allow<SkeletonLodDisabled>>,
    skinned_meshes: Query<
        &SkinnedMesh,
        (Without<Mesh3d>, With<ChildOf>, Allow<SkeletonLodDisabled>),
    >,
    children_query: Query<&Children, Allow<SkeletonLodDisabled>>,
    mut bone_transforms: Query<
        &mut Transform,
        (Without<CharacterShape>, Allow<SkeletonLodDisabled>),
    >,
) {
    let event = trigger.event();
    let Ok(lod_map) = lod_map_query.get(event.character) else {
        return;
    };
    let Some(&skeleton_entity) = lod_map.0.get(&event.lod) else {
        return;
    };
    let Ok(bind_pose) = bind_pose_query.get(skeleton_entity) else {
        return;
    };

    // Find the SkinnedMesh joints inside the skeleton's descendants
    for child in children_query.iter_descendants(skeleton_entity) {
        if let Ok(skm) = skinned_meshes.get(child) {
            for (joint_entity, bind_transform) in skm.joints.iter().zip(bind_pose.0.iter()) {
                if let Ok(mut transform) = bone_transforms.get_mut(*joint_entity) {
                    *transform = *bind_transform;
                }
            }
            return;
        }
    }
}

/// Sets up `SkinnedMesh` on each `CharacterPart` by cloning from the matching
/// `CharacterSkeleton`'s rig entity.
pub(crate) fn setup_part_skinning(
    parts: Query<(Entity, &ChildOf, &CharacterPart), Without<SkinnedMesh>>,
    characters: Query<(&SkeletonLodMap, &SkeletonsReady)>,
    skinned_meshes: Query<
        &SkinnedMesh,
        (Without<Mesh3d>, With<ChildOf>, Allow<SkeletonLodDisabled>),
    >,
    children: Query<&Children, Allow<SkeletonLodDisabled>>,
    mut commands: Commands,
) {
    for (part_entity, parent, part) in &parts {
        let parent_entity = parent.parent();
        let Ok((lod_map, _)) = characters.get(parent_entity) else {
            continue;
        };

        // Find the skeleton entity for this part's LOD level
        let Some(&skeleton_entity) = lod_map.0.get(&part.lod) else {
            continue;
        };

        // Find the rig entity inside the skeleton's scene that has SkinnedMesh
        let mut part_skm: Option<SkinnedMesh> = None;
        for child in children.iter_descendants(skeleton_entity) {
            if let Ok(skm) = skinned_meshes.get(child) {
                part_skm = Some(skm.clone());
                break;
            }
        }

        if let Some(skm) = part_skm {
            commands.entity(part_entity).insert(skm);
        }
    }
}
