use crate::{
    basemesh::{BaseMesh, VertexGroups},
    prelude::*,
    rigs::{get_model_space_skeleton_transforms, BoneTranslationData, RigData, RootBonePrevious},
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
pub struct FitSkeleton;

/// For storing refs to commonly needed entities so that you don't have to iter_descendants to find them.
#[derive(Component)]
pub struct SkeletonEntities {
    #[allow(dead_code)]
    pub rig: Entity,
    pub root_bone: Entity,
}

pub(crate) fn spawn_rig_scene(
    new_humans: Query<(Entity, &CharacterShape), (Without<SkinnedMesh>, Without<FitSkeleton>)>,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    rig_data: Res<RigData>,
    mut commands: Commands,
) {
    for (human, config) in new_humans {
        let Some(asset) = shape_assets.get(&config.0) else {
            continue;
        };
        let Some(template) = templates.get(&asset.template) else {
            continue;
        };
        let rig_type = template.rig;
        let Some(rig_spec) = rig_data.get(&rig_type) else {
            continue;
        };
        let Some(scene) = rig_spec.scene.clone() else {
            continue;
        };
        let cached_scene = commands
            .spawn((DynamicWorldRoot::from(scene), Name::new("RigScene")))
            .id();
        commands
            .entity(human)
            .insert(FitSkeleton)
            .add_child(cached_scene);
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn fit_skeleton_to_shape(
    mut commands: Commands,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    skinned_meshes: Query<&SkinnedMesh, (Without<Mesh3d>, With<ChildOf>)>,
    children: Query<&Children>,
    configs: Query<(Entity, &CharacterShape, Option<&RootMotion>), With<FitSkeleton>>,
    names: Query<&Name, With<SkeletalBone>>,
    mut local_transforms: Query<&mut Transform, Without<CharacterShape>>,
    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    basemesh: Res<BaseMesh>,
    morph_targets: Res<MakeHumanMorphs>,
    rig_data: Res<RigData>,
    vg: Res<VertexGroups>,
) {
    for (character_entity, config, root_motion) in configs.iter() {
        let Some(mut asset) = shape_assets.get_mut(&config.0) else {
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

        let rig_type = template.rig;
        let Some(rig_spec) = rig_data.get(&rig_type) else {
            continue;
        };

        let mut rig_entity: Option<Entity> = None;
        let mut skinned_mesh: Option<SkinnedMesh> = None;
        for child in children.iter_descendants(character_entity) {
            if let Ok(rig) = skinned_meshes.get(child) {
                rig_entity = Some(child);
                skinned_mesh = Some(rig.clone());
            }
        }

        let (Some(rig_entity), Some(skm)) = (rig_entity, skinned_mesh) else {
            continue;
        };

        // Collect bone entities by iterating descendants of the rig_entity
        let mut bone_entities = AHashMap::default();
        for descendant in children.iter_descendants(rig_entity) {
            if descendant == rig_entity {
                continue;
            }
            if let Ok(name) = names.get(descendant) {
                let name = NAME_INTERNER.intern(name.as_str()).leak();
                bone_entities.insert(name, descendant);
            }
        }

        if bone_entities.is_empty() {
            continue;
        }

        // Re-fit skeleton to mesh shape.
        let mut model_space_bindposes = get_model_space_skeleton_transforms(
            &rig_spec.reference_rig.bone_names,
            &helpers,
            rig_type,
            &vg,
            &rig_data,
        );
        let mut local_bone_transforms = AHashMap::default();

        let bone_config = &rig_spec.config;
        for &bone in &rig_spec.reference_rig.bone_names {
            if let Some(bone_data) = bone_config.bones.get(bone) {
                let parent = NAME_INTERNER.intern(&bone_data.parent).leak();
                let parent_transform = match model_space_bindposes.get(parent) {
                    Some(xform) => *xform,
                    None => Transform::IDENTITY,
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

                let joint = bone_entities[bone];
                let mut local_transform = local_transforms.get_mut(joint).unwrap();
                *local_transform = new_local;
            }
        }

        // Cache per-bone translation data for animation rescaling (computed once per asset).
        if matches!(asset.bone_translations, BoneTranslationData::None) {
            let translations = model_space_bindposes
                .iter()
                .map(|(&bone, xform)| (bone, xform.translation))
                .collect();
            asset.bone_translations = BoneTranslationData::Full(translations);
            asset.bone_delta_rotations = AHashMap::<&'static str, Quat>::default();
        }

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
        for &bone in rig_spec.reference_rig.bone_names.iter() {
            inv_bindposes.push(model_space_inv_bindposes[bone].to_matrix());
        }

        let root_bone = bone_entities[rig_spec.reference_rig.bone_names[0]];
        let related = SkeletonEntities {
            rig: rig_entity,
            root_bone,
        };

        commands
            .entity(character_entity)
            .insert((
                SkinnedMesh {
                    joints: skm.joints.clone(),
                    inverse_bindposes: inv_bindpose_assets.add(inv_bindposes),
                },
                related,
            ))
            .remove::<FitSkeleton>();

        commands
            .entity(rig_entity)
            .remove::<SkinnedMesh>()
            .insert(Transform::from_rotation(crate::MODEL_ROTATION_FIX));

        if let Some(_root_motion) = root_motion {
            commands
                .entity(root_bone)
                .insert(RootBonePrevious::default());
        }
    }
}
