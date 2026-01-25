use crate::{
    HumentityGlobalConfig, TranslationTracks, assets::CharacterAssetRegistry, basemesh::VertexGroups, prelude::*, rigs::{BoneTranslationData, RigData, RootBonePrevious, SkeletonCaches, get_model_space_skeleton_transforms}
};
use ahash::AHashMap;
use bevy::{
    ecs::intern::Internable,
    mesh::{
        morph::MeshMorphWeights,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    prelude::*,
};
use serde::{Deserialize, Deserializer, Serialize};

/*--------------+
|  Components  |
+--------------*/
/// Defines the shape of a character.  Place it at the root, with individual parts as children.
#[derive(Component, Clone, Default, Debug, Serialize)]
#[require(Visibility)]
pub struct CharacterShapeConfig {
    pub prefab_morph_targets: MorphTargets,
    pub prefab: &'static str,
    #[serde(skip)]
    pub(crate) bone_translations: BoneTranslationData,
    #[serde(skip)]
    pub(crate) bone_delta_rotations: AHashMap<&'static str, Quat>,
}

impl<'de> Deserialize<'de> for CharacterShapeConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            prefab_morph_targets: MorphTargets,
            prefab: String,
        }

        let raw = Raw::deserialize(deserializer)?;
        let prefab: &'static str = NAME_INTERNER.intern(&raw.prefab).leak();

        Ok(Self::new(prefab, raw.prefab_morph_targets))
    }
}


impl CharacterShapeConfig {
    pub fn new(prefab: &'static str, morphs: MorphTargets) -> Self {
        Self {
            prefab,
            prefab_morph_targets: morphs,
            bone_translations: BoneTranslationData::None,
            bone_delta_rotations: AHashMap::<&'static str, Quat>::default(),
        }
    }
}

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

/*------------+
|  Messages  |
+------------*/
#[derive(Message, Deref)]
pub struct CharacterPartMeshSpawned(Entity);

/*-----------+
|  Systems  |
+-----------*/
pub(crate) fn spawn_rig_scene(
    new_humans: Query<(Entity, &CharacterShapeConfig), Added<CharacterShapeConfig>>,
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
    for (root, mut config, model_transform, ragdoll, root_motion) in configs.iter_mut() {
        let prefab = &prefabs[&config.prefab];
        let rig_type = prefab.rig.rig_type;
        let cache = &skeleton_caches[&rig_type];

        // Find the relevant entities below
        let mut skinned_mesh: Option<&SkinnedMesh> = None;
        let mut rig_entity = Entity::PLACEHOLDER;

        // Cache current bone rotations and entities
        let mut bone_rotations = AHashMap::default();
        let mut bone_entities = AHashMap::default();

        for child in children.iter_descendants(root) {
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

        // The skeleton was adjusted so that the bones' rotations align head to tail.
        // The skeleton now fits the mesh's shape but this can induce animation artifacts due to
        // differences in proportions/bind poses. In order to prevent this we adjust the bone rotations 
        // so that they have the same positions, but rotations are adjusted to align exactly with
        // the reference skeleton from the animation glb files. In other words, the bones may not be 
        // rotated such that they point to their child bone anymore, but they will match the reference rig rotations.
        // This will require changing both rotations and translations. A child bone needs to translate
        // back into it's correct model space position after its parent rotates.
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
            // came at the cost of adding in some translation offsets.  When retargeting translation
            // tracks we need to correct for this.  Here we cache a small rotation which we can
            // apply to re-align translation directions later.
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

        // Create new skinned_mesh, put it on root
        // Root doesn't have a mesh3d but it makes it easier to clone for added components.
        let mut inv_bindposes = vec![];
        for &bone in cache.bone_order.iter() {
            inv_bindposes.push(global_bone_transforms[bone].to_matrix().inverse());
        }

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
        commands
            .entity(root)
            .insert((
                SkinnedMesh {
                    joints: skinned_mesh.joints.clone(),
                    inverse_bindposes: inv_bindpose_assets.add(inv_bindposes),
                },
                RelatedEntities {
                    rig: rig_entity,
                    root_bone,
                    head,
                    left_foot,
                    left_hand,
                    right_foot,
                    right_hand,
                    right_shoulder,
                    left_shoulder,
                },
            ))
            .remove::<FitSkeleton>();

        // Remove skinned mesh from rig_entity
        commands.entity(rig_entity).remove::<SkinnedMesh>();

        // Set up root bone transform tracking
        if let Some(root_motion) = root_motion {
            let root_name = cache.bone_order[0];
            let transform = local_bone_transforms[root_name];
            let mut translation = transform.translation;
            if !root_motion.y_translate {
                translation.y = 0.;
            }
            commands
                .entity(root_bone)
                .insert(RootBonePrevious::default());
        }

        // Set up ragdoll if added
        if let Some(mut ragdoll) = ragdoll {
            ragdoll.spawn_ragdoll(
                &mut commands,
                &helpers,
                prefab.rig.rig_type,
                &bone_entities,
                &global_transforms,
                &rig_data,
                rig_entity,
            );
        }
    }
}

pub(crate) fn setup_human_parts(
    parts: Query<(Entity, &CharacterPart, &ChildOf), Without<Mesh3d>>,
    configs: Query<(Entity, &CharacterShapeConfig, &SkinnedMesh)>,
    mut registry: ResMut<CharacterAssetRegistry>,
    mut prefabs: ResMut<CharacterArchetypePrefabs>,
    rig_data: Res<RigData>,
    skeleton_caches: Res<SkeletonCaches>,
    mut basemesh: ResMut<BaseMesh>,
    mh_morphs: Res<MakeHumanMorphs>,
    paths: Res<HumentityPathsConfig>,
    mut asset_server: ResMut<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    mut writer: MessageWriter<CharacterPartMeshSpawned>,
) {
    for (entity, part, child_of) in parts.iter() {
        let Ok((human, config, skinned_mesh)) = configs.get(child_of.parent()) else {
            continue;
        };

        // Get some releveant data
        let prefab_name = config.prefab;
        let prefab = prefabs.get_mut(&config.prefab).unwrap();
        let rig_type = prefab.rig.rig_type;
        let cache = &skeleton_caches[&rig_type];

        let morph_weights = prefab
            .shapes
            .iter()
            .map(|s| NAME_INTERNER.intern(&s.name).leak())
            .map(|s| *config.prefab_morph_targets.get(s).unwrap_or(&0.))
            .collect::<Vec<_>>();
        let morph_weights = MeshMorphWeights::new(morph_weights).unwrap();

        // Spawn meshes
        match part {
            CharacterPart::BaseMesh => {
                if let Some(handle) = basemesh.get_rigged_mesh_handle(
                    prefab_name,
                    prefab,
                    &mut meshes,
                    &mut images,
                    &rig_data,
                    &mh_morphs,
                    cache,
                ) {
                    commands
                        .entity(entity)
                        .insert((Mesh3d(handle.clone()), skinned_mesh.clone()));
                    let mesh = meshes.get(&handle).unwrap();
                    if mesh.has_morph_targets() {
                        commands.entity(entity).insert(morph_weights);
                    }
                    writer.write(CharacterPartMeshSpawned(human));
                }
            }
            CharacterPart::BodyPart(name)
            | CharacterPart::Equipment(name)
            | CharacterPart::ProxyMesh(name) => {
                let Some(asset) = registry.get_mut(part) else {
                    error!("No such asset: {} - Cannot load", name);
                    continue;
                };
                if let Some(handle) = asset.get_rigged_mesh_handle(
                    &mut asset_server,
                    prefab_name,
                    prefab,
                    &rig_data,
                    &basemesh,
                    &mh_morphs,
                    &paths,
                    &mut meshes,
                    &mut images,
                    cache,
                ) {
                    commands
                        .entity(entity)
                        .insert((Mesh3d(handle.clone()), skinned_mesh.clone()));
                    let mesh = meshes.get(&handle).unwrap();
                    if mesh.has_morph_targets() {
                        commands.entity(entity).insert(morph_weights);
                    }
                    writer.write(CharacterPartMeshSpawned(human));
                }
            }
        }
    }
}
