use std::f32::consts::PI;

use crate::{
    basemesh::VertexGroups,
    prelude::*,
    rigs::{
        get_model_space_skeleton_transforms, BoneTranslationData, RigData, RootBonePrevious,
        SkeletonCaches,
    },
    HumentityGlobalConfig, TranslationTracks,
};
use ahash::AHashMap;
use bevy::{
    ecs::intern::Internable,
    mesh::{
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    prelude::*,
};


/// This component will trigger the re-fitting of the skeleton to the character's morphs.
/// Add it after changing morphs.
#[derive(Component)]
pub struct FitSkeleton;

/// For storing refs to commonly needed entities so that you don't have to iter_descendants to find them.
#[derive(Component)]
pub struct RelatedEntities {
    #[allow(dead_code)]
    pub rig: Entity, // Skeleton Root/AnimationPlayer
    pub root_bone: Entity,
    pub head: Entity,
    pub right_hand: Entity,
    pub left_hand: Entity,
    pub right_foot: Entity,
    pub left_foot: Entity,
    pub right_shoulder: Entity,
    pub left_shoulder: Entity,
}

pub(crate) fn spawn_rig_scene(
    new_humans: Query<(Entity, &CharacterShapeConfig), (Without<SkinnedMesh>, Without<FitSkeleton>)>,
    prefabs: Res<CharacterArchetypePrefabs>,
    skeleton_caches: Res<SkeletonCaches>,
    mut commands: Commands,
) {
    new_humans.iter().for_each(|(human, config)| {
        // Spawn rig scene
        if let Some(prefab) = prefabs.get(&config.prefab) {
            let rig_type = prefab.rig.rig_type;
            let cached_scene = skeleton_caches[&rig_type].scene.clone();
            let cached_scene = commands
                .spawn((DynamicSceneRoot::from(cached_scene), Name::new("RigScene")))
                .id();
            commands
                .entity(human)
                .insert(FitSkeleton)
                .add_child(cached_scene);
        } else {
            error!("No such prefab named {}", config.prefab);
            commands.entity(human).despawn();
        }
    })
}

#[allow(clippy::type_complexity)]
pub(crate) fn fit_skeleton_to_shape(
    mut commands: Commands,
    prefabs: Res<CharacterArchetypePrefabs>,
    rigs: Query<(Entity, &SkinnedMesh), Without<Mesh3d>>,
    children: Query<&Children>,
    mut configs: Query<
        (
            Entity,
            &mut CharacterShapeConfig,
            &Transform,
            Option<&mut CharacterRagdoll>,
            Option<&RootMotion>,
        ),
        With<FitSkeleton>,
    >,
    names: Query<&Name>,
    global_transforms: Query<&GlobalTransform>,
    mut local_transforms: Query<&mut Transform, Without<CharacterShapeConfig>>,
    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    basemesh: Res<BaseMesh>,
    morph_targets: Res<MakeHumanMorphs>,
    skeleton_caches: Res<SkeletonCaches>,
    vg: Res<VertexGroups>,
    rig_data: Res<RigData>,
    global_config: Res<HumentityGlobalConfig>,
) {
    for (character_entity, mut config, model_transform, ragdoll, root_motion) in configs.iter_mut() {
        let prefab = &prefabs[&config.prefab];
        let rig_type = prefab.rig.rig_type;
        let cache = &skeleton_caches[&rig_type];

        // Find the relevant entities below
        let mut skinned_mesh: Option<&SkinnedMesh> = None;
        let mut rig_entity = Entity::PLACEHOLDER;

        // Cache current model space bone rotations and entities
        let mut bone_rotations = AHashMap::default();
        let mut bone_entities = AHashMap::default();

        for child in children.iter_descendants(character_entity) {
            if let Ok((entity, skm)) = rigs.get(child) {
                skinned_mesh = Some(skm);
                rig_entity = entity;
                for child in children.iter_descendants(child) {
                    if let Ok(transform) = global_transforms.get(child) {
                        let name = names.get(child).unwrap();
                        let name = NAME_INTERNER.intern(name.as_str()).leak();
                        bone_entities.insert(name, child);
                        bone_rotations.insert(
                            name,
                            Transform::from_matrix(
                                model_transform.to_matrix().inverse() * transform.to_matrix(),
                            )
                            .rotation,
                        );
                    }
                }
                break;
            }
        }
        if skinned_mesh.is_none() {
            return;
        }
        // This has the refs to the joint entities, but we will have to recompute inverse bindposes
        let skinned_mesh = skinned_mesh.unwrap();

        // Re-fit skeleton to mesh shape.  This is based on fixed vertices in the base mesh.
        // This will move and rotate the bones to align with those verts.
        let helpers = prefab.get_helpers(&config.prefab_morph_targets, &basemesh, &morph_targets);
        let mut global_bone_transforms = get_model_space_skeleton_transforms(
            &cache.bone_order,
            &helpers,
            rig_type,
            &bone_rotations,
            &vg,
            &rig_data,
        );
        let mut local_bone_transforms = AHashMap::default();

        // The skeleton has now been adjusted so that the bones' rotations align head to tail.
        // The skeleton now fits the mesh's shape but this can induce animation artifacts due to
        // differences in proportions/bind poses. Different proportions lead to different rotations
        // in the bind/rest pose, but animation clips only store rotation offsets so the final pose in
        // any given frame will change with character proportions. Therefore different characer shapes
        // can lead to very different poses in the same animation clip.
        //
        // In order to prevent this we adjust the bone bind pose transfoms so that they have the same
        // positions in model space, but we force their rotations to align exactly with the reference 
        // skeleton from which the animation clips were authored in the glb files. In other words, we
        // rotate the bones such that they may not point to their child bone anymore. Instead they will
        // match the reference rig bind pose rotations exactly, so that rotation offsets from the
        // AninationClips look as consistent as possible. This will require changing not just rotations
        // but also translations in general, as changing the rotation on a bone will alter the model
        // space translation of all children in the hierarchy, so we alter the translations to get
        // the bone back into the correct position after rotating it's parents. 

        let bone_config = &rig_data.configs[&prefab.rig.rig_type];
        for &bone in &cache.bone_order {
            if let Some(bone_data) = bone_config.get(bone) {
                let parent_transform = match global_bone_transforms.get(bone_data.parent) {
                    Some(xform) => *xform,
                    None => Transform::IDENTITY,
                };
                let old_global = global_bone_transforms[bone];

                // Use reference rotation, preserve global position
                let reference_rot = cache.bone_model_space_rots[bone];
                let new_global = Transform {
                    translation: old_global.translation,
                    rotation: reference_rot,
                    scale: old_global.scale,
                };

                // Recompute local transform
                let parent_matrix = parent_transform.to_matrix();
                let child_matrix = new_global.to_matrix();
                let new_local = Transform::from_matrix(parent_matrix.inverse() * child_matrix);

                local_bone_transforms.insert(bone, new_local);
                global_bone_transforms.insert(bone, new_global);

                // Update the joint entity transforms
                let joint = bone_entities[bone];
                let mut local_transform = local_transforms.get_mut(joint).unwrap();
                *local_transform = new_local;
            }
        }

        if !matches!(global_config.translation_tracks, TranslationTracks::None) {
            // Cache the bone translations for animation post-processing
            match global_config.translation_tracks {
                TranslationTracks::Root => {
                    let root_bone = cache.bone_order[0];
                    let root_trans = local_bone_transforms[root_bone];
                    config.bone_translations = BoneTranslationData::Root(root_trans.translation);
                }
                TranslationTracks::Full => {
                    let bone_translations = local_bone_transforms
                        .iter()
                        .map(|(&n, t)| (n, t.translation))
                        .collect::<AHashMap<&'static str, Vec3>>();
                    config.bone_translations = BoneTranslationData::Full(bone_translations);
                }
                _ => {}
            }

            // We re-aligned the bone rotations to match the reference skeleton exactly, but this
            // came at the cost of adding in some translation offsets. This introduces another 
            // complexity if we are retargeting translation tracks, since the translations are in 
            // local bone space, which is not rotated.  We need to correct for this.  Here we cache
            // a small rotation which we can apply to re-align translation directions during
            // animation postprocessing for translation tracks.
            if matches!(global_config.translation_tracks, TranslationTracks::Full) {
                let mut bone_rotation_deltas: AHashMap<&'static str, Quat> = AHashMap::default();
                for &name in cache.bone_order.iter() {
                    let bone_data = bone_config.get(name).unwrap();
                    if bone_data.parent.is_empty() {
                        continue;
                    };
                    let ref_bone = global_transforms.get(bone_entities[name]).unwrap();
                    let ref_parent = global_transforms
                        .get(bone_entities[bone_data.parent])
                        .unwrap();
                    let ref_dir: Vec3 =
                        (ref_bone.translation() - ref_parent.translation()).normalize();
                    let shape_dir = (global_bone_transforms[name].translation
                        - global_bone_transforms[bone_data.parent].translation)
                        .normalize();
                    let delta = Quat::from_rotation_arc(ref_dir, shape_dir);
                    let parent_rot = global_bone_transforms[bone_data.parent].rotation;
                    bone_rotation_deltas.insert(name, parent_rot.inverse() * delta * parent_rot);
                }
                config.bone_delta_rotations = bone_rotation_deltas;
            }
        }

        // Create new skinned_mesh, put it on the character root.
        // Root doesn't have a mesh3d but it makes it easier to clone for children with CharacterPart.
        let mut inv_bindposes = vec![];
        for &bone in cache.bone_order.iter() {
            inv_bindposes.push(global_bone_transforms[bone].to_matrix().inverse());
        }

        // Cache commonly used joint entities for easy access, e.g. IK
        let root_bone = bone_entities[cache.bone_order[0]];
        let mut head = Entity::PLACEHOLDER;
        let mut right_hand = Entity::PLACEHOLDER;
        let mut left_hand = Entity::PLACEHOLDER;
        let mut right_foot = Entity::PLACEHOLDER;
        let mut left_foot = Entity::PLACEHOLDER;
        let mut right_shoulder = Entity::PLACEHOLDER;
        let mut left_shoulder = Entity::PLACEHOLDER;
        for &bone in cache.bone_order.iter() {
            if ["head"].contains(&bone) {
                head = bone_entities[bone];
            } else if ["wrist.L"].contains(&bone) {
                left_hand = bone_entities[bone];
            } else if ["wrist.R"].contains(&bone) {
                right_hand = bone_entities[bone];
            } else if ["foot.L"].contains(&bone) {
                left_foot = bone_entities[bone];
            } else if ["foot.R"].contains(&bone) {
                right_foot = bone_entities[bone];
            } else if ["shoulder01.L"].contains(&bone) {
                left_shoulder = bone_entities[bone];
            } else if ["shoulder01.R"].contains(&bone) {
                right_shoulder = bone_entities[bone];
            }
        }
        let related = RelatedEntities {
            rig: rig_entity,
            root_bone,
            head,
            left_foot,
            left_hand,
            right_foot,
            right_hand,
            right_shoulder,
            left_shoulder,
        };

        commands
            .entity(character_entity)
            .insert((
                SkinnedMesh {
                    joints: skinned_mesh.joints.clone(),
                    inverse_bindposes: inv_bindpose_assets.add(inv_bindposes),
                },
                related,
            ))
            .remove::<FitSkeleton>();

        // Remove skinned mesh from rig_entity, and rotate to face the correct forward direction
        commands.entity(rig_entity)
            .remove::<SkinnedMesh>()
            .insert(Transform::from_rotation(Quat::from_rotation_y(PI)));

        // Set up root bone transform tracking
        if let Some(_root_motion) = root_motion {
            commands
                .entity(root_bone)
                .insert(RootBonePrevious::default());
        }

        // Set up ragdoll if added
        if let Some(mut ragdoll) = ragdoll {
            #[cfg(feature = "ragdolls")]
            ragdoll.spawn_ragdoll(
                &mut commands,
                character_entity,
                &helpers,
                prefab.rig.rig_type,
                &bone_entities,
                &global_transforms,
                &rig_data,
                rig_entity,
            );
            #[cfg(not(feature = "ragdolls"))]
            error!("Ragdoll functionality requires the \"ragdolls\" freature");
        }
    }
}
