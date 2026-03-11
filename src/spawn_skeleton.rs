use crate::{
    HumentityGlobalConfig, TranslationTracks, basemesh::{BaseMesh, VertexGroups}, prelude::*, rigs::{
        BoneTranslationData, RigData, RootBonePrevious, SkeletonCaches, get_model_space_skeleton_transforms
    }
};
use ahash::AHashMap;
use bevy::{
    ecs::intern::Internable,
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};

/// This component will trigger the re-fitting of the skeleton to the character's morphs.
/// Add it after changing morphs.
#[derive(Component)]
pub(crate) struct FitSkeleton;

/// For storing refs to commonly needed entities so that you don't have to iter_descendants to find them.
#[derive(Component)]
pub struct RelatedEntities {
    #[allow(dead_code)]
    pub rig: Entity, // Skeleton Root/AnimationPlayer
    pub root_bone: Entity,
}

pub(crate) fn spawn_rig_scene(
    new_humans: Query<
        (Entity, &CharacterShapeConfig),
        (Without<SkinnedMesh>, Without<FitSkeleton>),
    >,
    prefabs: Res<CharacterArchetypePrefabs>,
    skeleton_caches: Res<SkeletonCaches>,
    mut commands: Commands,
) {
    for (human, config) in new_humans {
        // Spawn rig scene
        if let Some(prefab) = prefabs.get(&config.prefab) {
            let rig_type = prefab.rig;
            let Some(cache) = skeleton_caches.get(&rig_type) else { 
                return
            };
            let cached_scene = cache.scene.clone();
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
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn fit_skeleton_to_shape(
    mut commands: Commands,
    prefabs: Res<CharacterArchetypePrefabs>,
    rigs: Query<(Entity, &SkinnedMesh), (Without<Mesh3d>, With<ChildOf>)>,
    children: Query<&Children>,
    mut configs: Query<
        (
            Entity,
            &mut CharacterShapeConfig,
            Option<&RootMotion>,
        ),
        With<FitSkeleton>,
    >,
    names: Query<&Name, With<SkeletalBone>>,
    global_transforms: Query<&GlobalTransform, With<SkeletalBone>>,
    mut local_transforms: Query<&mut Transform, Without<CharacterShapeConfig>>,
    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    basemesh: Res<BaseMesh>,
    morph_targets: Res<MakeHumanMorphs>,
    skeleton_caches: Res<SkeletonCaches>,
    vg: Res<VertexGroups>,
    rig_data: Res<RigData>,
    //global_config: Res<HumentityGlobalConfig>,
) {
    for (character_entity, mut config, root_motion) in configs.iter_mut() {
        let prefab = &prefabs[&config.prefab];
        let rig_type = prefab.rig;
        let cache = &skeleton_caches[&rig_type];

        // Find the relevant entities below
        let mut skinned_mesh: Option<&SkinnedMesh> = None;
        let mut rig_entity = Entity::PLACEHOLDER;

        let mut bone_entities = AHashMap::default();

        for child in children.iter_descendants(character_entity) {
            if let Ok((entity, skm)) = rigs.get(child) {
                skinned_mesh = Some(skm);
                rig_entity = entity;
                for child in children.iter_descendants(child) {
                    if let Ok(name) = names.get(child) {
                        let name = NAME_INTERNER.intern(name.as_str()).leak();
                        bone_entities.insert(name, child);
                    }
                }
                break;
            }
        }

        // Character not ready yet, skip for now. This system will run again next frame and hopefully
        // the rig will be loaded by then.
        if skinned_mesh.is_none() {
            return;
        }
        // This has the refs to the joint entities, but we will have to recompute inverse bindposes
        let skinned_mesh = skinned_mesh.unwrap();

        // Re-fit skeleton to mesh shape.  This is based on fixed vertices in the base mesh.
        // This will move and rotate the bones to align with those verts (using roll from rig config).
        let helpers = prefab.get_helpers(
            &config.prefab_morph_targets,
            &basemesh.0,
            &morph_targets,
        ).unwrap_or_else(|e| panic!("{}", e) );
        let mut model_space_bindposes = get_model_space_skeleton_transforms(
            &cache.bone_order, &helpers, rig_type, &vg, &rig_data,
        );
        let mut local_bone_transforms = AHashMap::default();

        // The skeleton has now been adjusted so that the bones' rotations align head to tail.
        // The skeleton now fits the mesh's shape but this can induce animation artifacts due to
        // differences in proportions/bind poses. Different proportions lead to different rotations
        // in the bind/rest pose, but animation clips only store rotation offsets so the final pose in
        // any given frame will change with character proportions. Therefore different character shapes
        // can lead to very different poses in the same animation clip.
        //
        // In order to prevent this we adjust the bone bind pose transforms so that they have the same
        // positions in model space, but we force their rotations to align exactly with the reference
        // skeleton from which the animation clips were authored in the glb files. In other words, we
        // rotate the bones such that they may not point to their child bone anymore. Instead they will
        // match the reference rig bind pose rotations exactly, so that rotation offsets from the
        // AninationClips look as consistent as possible. This will require changing not just rotations
        // but also translations in general, as changing the rotation on a bone will alter the model
        // space translation of all children in the hierarchy, so we alter the translations to get
        // the bone back into the correct position after rotating its parents.

        let bone_config = &rig_data[&prefab.rig].config;
        for &bone in &cache.bone_order {
            if let Some(bone_data) = bone_config.bones.get(bone) {
                let parent = NAME_INTERNER.intern(&bone_data.parent).leak();
                let parent_transform = match model_space_bindposes.get(parent) {
                    Some(xform) => *xform,
                    None => Transform::IDENTITY,
                };
                let old_global = model_space_bindposes[bone];

                // Use reference rotation, preserve global position
                let reference_rot = cache.bone_model_space_transforms[bone].rotation;
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
                model_space_bindposes.insert(bone, new_global);

                // Update the joint entity transforms
                let joint = bone_entities[bone];
                let mut local_transform = local_transforms.get_mut(joint).unwrap();
                *local_transform = new_local;
            }
        }

        /*
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
                    let bone_data = bone_config.bones.get(name).unwrap();
                    if bone_data.parent.is_empty() {
                        continue;
                    };
                    let ref_bone = global_transforms.get(bone_entities[name]).unwrap();
                    let parent = NAME_INTERNER.intern(&bone_data.parent).leak();
                    let ref_parent = global_transforms
                        .get(bone_entities[parent])
                        .unwrap();
                    let ref_dir: Vec3 =
                        (ref_bone.translation() - ref_parent.translation()).normalize();
                    let shape_dir = (model_space_bindposes[name].translation
                        - model_space_bindposes[parent].translation)
                        .normalize();
                    let delta = Quat::from_rotation_arc(ref_dir, shape_dir);
                    let parent_rot = model_space_bindposes[parent].rotation;
                    bone_rotation_deltas.insert(name, parent_rot.inverse() * delta * parent_rot);
                }
                config.bone_delta_rotations = bone_rotation_deltas;
            }
        }
         */

        // Create new skinned_mesh, put it on the character root.
        // Root doesn't have a mesh3d but it makes it easier to clone for children with CharacterPart.
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
        for &bone in cache.bone_order.iter() {
            inv_bindposes.push(model_space_inv_bindposes[bone].to_matrix());
        }

        // Cache commonly used joint entities for easy access, e.g. IK
        let root_bone = bone_entities[cache.bone_order[0]];
        let related = RelatedEntities {
            rig: rig_entity,
            root_bone,
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
        commands
            .entity(rig_entity)
            .remove::<SkinnedMesh>()
            .insert(Transform::from_rotation(crate::MODEL_ROTATION_FIX));

        // Set up root bone transform tracking
        if let Some(_root_motion) = root_motion {
            commands
                .entity(root_bone)
                .insert(RootBonePrevious::default());
        }
    }
}
