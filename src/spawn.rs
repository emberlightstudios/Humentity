use bevy::{ecs::intern::Internable, mesh::{morph::MeshMorphWeights, skinning::{SkinnedMesh, SkinnedMeshInverseBindposes}}, prelude::*};
use crate::{HumentityGlobalConfig, assets::CharacterAssetRegistry, basemesh::VertexGroups, prelude::*, rigs::{RigData, RootBonePrevious, get_model_space_skeleton_transforms}};
use ahash::AHashMap;


/*--------------+
 |  Components  |
 +--------------*/
/// Defines the shape of a character.  Place it at the root, with individual parts as children.
#[derive(Component, Clone, Default)]
pub struct CharacterShapeConfig {
    pub prefab_morph_targets: MorphTargets,
    pub prefab: &'static str,
    pub(crate) bone_translations: AHashMap<&'static str, Vec3>,
    pub(crate) bone_delta_rotations: AHashMap<&'static str, Quat>,
}

impl CharacterShapeConfig {
    pub fn new(prefab: &'static str, morphs: MorphTargets) -> Self {
        Self {
            prefab,
            prefab_morph_targets: morphs,
            bone_translations: AHashMap::default(),
            bone_delta_rotations: AHashMap::default(),
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
    pub rig: Entity,        // AnimationPlayer
    pub root_bone: Entity,
}

/*-----------+
 |  Systems  |
 +-----------*/
pub(crate) fn spawn_rig_scene(
    new_humans: Query<(Entity, &CharacterShapeConfig), Added<CharacterShapeConfig>>,
    prefabs: Res<CharacterArchetypePrefabs>,
    mut commands: Commands,
) {
    new_humans.iter().for_each(|(human, config)| {
        // Spawn rig scene
        if let Some(prefab) = prefabs.get(&config.prefab) {
            let cached_scene = prefab.rig.scene.clone()
                .expect("No rig archetype scene found");
            let cached_scene = commands
                .spawn((
                    DynamicSceneRoot::from(cached_scene),
                    Name::new("RigScene"),
                ))
                .id();
            commands.entity(human).insert(FitSkeleton).add_child(cached_scene);
        } else {
            error!("No such prefab named {}", config.prefab);
            commands.entity(human).despawn();
        }
    })
}

pub(crate) fn fit_skeleton_to_shape(
    mut commands: Commands,
    prefabs: Res<CharacterArchetypePrefabs>,
    rigs: Query<(Entity, &SkinnedMesh), Without<Mesh3d>>,
    children: Query<&Children>,
    mut configs: Query<(Entity, &mut CharacterShapeConfig, &Transform, Option<&mut CharacterRagdoll>, Option<&RootMotion>), With<FitSkeleton>>,
    names: Query<&Name>,
    global_transforms: Query<&GlobalTransform>,
    mut local_transforms: Query<&mut Transform, Without<CharacterShapeConfig>>,
    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    basemesh: Res<BaseMesh>,
    morph_targets: Res<MakeHumanMorphs>,
    vg: Res<VertexGroups>,
    rig_data: Res<RigData>,
    global_config: Res<HumentityGlobalConfig>,
) {
    for (root, mut config, model_transform, ragdoll, root_motion) in configs.iter_mut() {
        let prefab = &prefabs[&config.prefab];

        let mut skinned_mesh: Option<&SkinnedMesh> = None;
        let mut rig_entity = Entity::PLACEHOLDER;
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
                            Transform::from_matrix(model_transform.to_matrix().inverse() * transform.to_matrix()).rotation
                        );
                    }
                }
                break;
            }
        }
        if skinned_mesh.is_none() { return }
        let skinned_mesh = skinned_mesh.unwrap();

        // Re-fit skeleton to mesh shape
        let helpers = prefab.get_helpers(&config.prefab_morph_targets, &*basemesh, &*morph_targets);
        let mut global_bone_transforms = get_model_space_skeleton_transforms(
            &prefab.rig.bone_order, &helpers, prefab.rig.rig_type, &bone_rotations, &*vg, &*rig_data);
        let mut local_bone_transforms = AHashMap::default();

        // The skeleton was adjusted so that the bones' rotations align head to tail.
        // The skeleton now fits the mesh's shape but this can induce animation artifacts due to 
        // differences in proportions/bind poses. In order to prevent this let's adjust the skeleton so 
        // that the bones have the same positions, but rotations are adjusted to align exactly with
        // the reference skeleton from the animation glb files.
        let bone_config = &rig_data.configs[&prefab.rig.rig_type];
        for &bone in &prefab.rig.bone_order {
            if let Some(bone_data) = bone_config.get(bone) {
                let parent_transform = match global_bone_transforms.get(bone_data.parent) {
                    Some(xform) => *xform,
                    None => Transform::IDENTITY,
                };
                let old_global = global_bone_transforms[bone];
            
                // Use reference rotation, preserve global position
                let reference_rot = prefab.rig.bone_rotations[bone];
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

        if global_config.translation_animation_tracks {
            // Cache the bone translations for animation post-processing
            config.bone_translations = local_bone_transforms
                .iter()
                .map(|(&n, t)| (n, t.translation))
                .collect();

            // We re-aligned the bone rotations to match the reference skeleton exaclty, but this
            // came at the cost of adding in some translation offsets.  When retargeting translation
            // tracks we need to correct for this.  Here we cache a small rotation which we can
            // apply to re-align translation directions later.
            let mut bone_rotation_deltas: AHashMap<&'static str, Quat> = AHashMap::default();
            for &name in global_bone_transforms.keys() {
                let bone_data = bone_config.get(name).unwrap();
                if bone_data.parent == "" { continue };
                let ref_bone = global_transforms.get(bone_entities[name]).unwrap();
                let ref_parent = global_transforms.get(bone_entities[bone_data.parent]).unwrap();
                let ref_dir: Vec3 = (ref_bone.translation() - ref_parent.translation()).normalize();
                let shape_dir = (global_bone_transforms[name].translation - global_bone_transforms[bone_data.parent].translation).normalize();
                let delta = Quat::from_rotation_arc(ref_dir, shape_dir);
                let parent_rot = global_bone_transforms[bone_data.parent].rotation;
                bone_rotation_deltas.insert(name, parent_rot.inverse() * delta * parent_rot);
            }
            config.bone_delta_rotations = bone_rotation_deltas;
        }

        // Create new skinned_mesh, put it on root
        // Root doesn't have a mesh3d but it makes it easier to clone for added components.
        let mut inv_bindposes = vec![];
        for &bone in prefab.rig.bone_order.iter() {
            inv_bindposes.push(global_bone_transforms[bone].to_matrix().inverse());
        }

        let root_bone = bone_entities[prefab.rig.bone_order[0]];
        commands.entity(root).insert(
            (
                SkinnedMesh {
                    joints: skinned_mesh.joints.clone(),
                    inverse_bindposes: inv_bindpose_assets.add(inv_bindposes),
                },
                RelatedEntities { rig: rig_entity, root_bone },
            )
        ).remove::<FitSkeleton>();

        // Remove skinned mesh from rig_entity
        commands.entity(rig_entity).remove::<SkinnedMesh>();

        // Set up root bone transform tracking
        if let Some(root_motion) = root_motion {
            let root_name = prefab.rig.bone_order[0];
            let transform = local_bone_transforms[root_name];
            let mut translation = transform.translation;
            if !root_motion.y_translate {
                translation.y = 0.;
            }
            commands.entity(root_bone).insert(RootBonePrevious::default());
        }

        // Set up ragdoll if added
        if let Some(mut ragdoll) = ragdoll {
            ragdoll.spawn_ragdoll(&mut commands, &helpers, prefab.rig.rig_type,
                &bone_entities, &global_transforms, &*rig_data, rig_entity);
        }
    }
}

pub(crate) fn setup_human_parts(
    parts: Query<(Entity, &CharacterPart, &ChildOf), Without<Mesh3d>>,
    configs: Query<(&CharacterShapeConfig, &SkinnedMesh)>,
    mut registry: ResMut<CharacterAssetRegistry>,
    mut prefabs: ResMut<CharacterArchetypePrefabs>,
    rig_data: Res<RigData>,
    mut basemesh: ResMut<BaseMesh>,
    mh_morphs: Res<MakeHumanMorphs>,
    paths: Res<HumentityPathsConfig>,
    mut asset_server: ResMut<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    for (entity, part, child_of) in parts.iter() {
        let Ok((config, skinned_mesh)) = configs.get(child_of.parent()) else 
            { continue };

        // Get some releveant data
        let prefab_name = config.prefab;
        let prefab = prefabs.get_mut(&config.prefab).unwrap();
        let morph_weights = prefab.shapes
            .iter()
            .map(|s| *config.prefab_morph_targets.get(&s.name).unwrap_or(&0.))
            .collect::<Vec<_>>();
        let morph_weights = MeshMorphWeights::new(morph_weights).unwrap();
        
        // Spawn meshes
        match part {
            CharacterPart::BaseMesh => {
                if let Some(handle) = basemesh.get_rigged_mesh_handle(
                    prefab_name, prefab, &mut *meshes, &mut *images, &*rig_data, &*mh_morphs)
                {
                    commands.entity(entity).insert((
                        Mesh3d(handle.clone()),
                        skinned_mesh.clone(),
                    ));
                    let mesh = meshes.get(&handle).unwrap();
                    if mesh.has_morph_targets() {
                        commands.entity(entity).insert(morph_weights);
                    }
                }
            }
            CharacterPart::BodyPart(name) | CharacterPart::Equipment(name) | CharacterPart::ProxyMesh(name) => {
                let Some(asset) = registry.get_mut(part) else {
                    error!("No such asset: {} - Cannot load", name);
                    continue;
                };
                if let Some(handle) = asset.get_rigged_mesh_handle(
                    &mut *asset_server, prefab_name, prefab, &*rig_data,
                    &*basemesh, &*mh_morphs, &*paths, &mut *meshes, &mut *images
                ) {
                    commands.entity(entity).insert((
                        Mesh3d(handle.clone()),
                        skinned_mesh.clone(),
                    ));
                    let mesh = meshes.get(&handle).unwrap();
                    if mesh.has_morph_targets() {
                        commands.entity(entity).insert(morph_weights);
                    }
                }
            }
        }
    }
}
