use bevy::{ecs::intern::Internable, mesh::{morph::MeshMorphWeights, skinning::{SkinnedMesh, SkinnedMeshInverseBindposes}}, prelude::*};
use crate::{prelude::*, assets::CharacterAssetRegistry, basemesh::VertexGroups, rigs::{get_model_space_skeleton_transforms, RigData}};
use ahash::AHashMap;


/*--------------+
 |  Components  |
 +--------------*/
#[derive(Component, Clone, Default)]
pub struct CharacterShapeConfig {
    pub prefab_morph_targets: MorphTargets,
    pub prefab: &'static str,
}

impl CharacterShapeConfig {
    pub fn new(prefab: &'static str, morphs: MorphTargets) -> Self {
        Self { prefab, prefab_morph_targets: morphs}
    }
}

#[derive(Component)]
pub struct FitSkeleton;

//#[derive(Component, Clone, Default)]
//pub struct HumanAssetConfig {
//    pub equipment: Vec<&'static str>,
//    pub body_parts: Vec<&'static str>,
//    pub eye_color: Color,
//    pub eyebrow_color: Color,
//    pub hair_color: Color,
//}

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
    mut configs: Query<(Entity, &CharacterShapeConfig, &Transform, Option<&mut CharacterRagdoll>), With<FitSkeleton>>,
    names: Query<&Name>,
    global_transforms: Query<&GlobalTransform>,
    mut local_transforms: Query<&mut Transform, Without<CharacterShapeConfig>>,
    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    basemesh: Res<BaseMesh>,
    morph_targets: Res<MakeHumanMorphs>,
    vg: Res<VertexGroups>,
    rig_data: Res<RigData>,
) {
    for (root, config, model_transform, ragdoll) in configs.iter_mut() {
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
        //info!("{:#?}", prefab.shapes[0].morphs);
        //info!("{:#?}", config.prefab_morph_targets);
        let helpers = prefab.get_helpers(&config.prefab_morph_targets, &*basemesh, &*morph_targets);
        let mut global_bone_transforms = get_model_space_skeleton_transforms(
            &prefab.rig.bone_order, &helpers, prefab.rig.rig_type, &bone_rotations, &*vg, &*rig_data);
        let mut local_bone_transforms = AHashMap::default();

        // The skeleton was adjusted so that the bones' rotations align head to tail.
        // The skeleton now fits the mesh's shape but this can induce animation artifacts due to 
        // differences in proportions/bind poses. In order to prevent this let's adjust the skeleton so 
        // that the bones have the same positions, but rotations are adjusted to align with the reference
        // skeleton from the animation glb files.
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

        let mut inv_bindposes = vec![];
        for &bone in prefab.rig.bone_order.iter() {
            inv_bindposes.push(global_bone_transforms[bone].to_matrix().inverse());
        }

        // Create new skinned_mesh, put it on root
        commands.entity(root).insert(SkinnedMesh {
            joints: skinned_mesh.joints.clone(),
            inverse_bindposes: inv_bindpose_assets.add(inv_bindposes),
        }).remove::<FitSkeleton>();
        // Remove skinned mesh from rig_entity
        commands.entity(rig_entity).remove::<SkinnedMesh>();

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
    prefabs: Res<CharacterArchetypePrefabs>,
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
        let prefab = &prefabs[&config.prefab];
        let morph_weights = prefab.shapes
            .iter()
            .map(|s| *config.prefab_morph_targets.get(&s.name).unwrap_or(&0.))
            .collect::<Vec<_>>();
        let morph_weights = MeshMorphWeights::new(morph_weights).unwrap();
        
        // Spawn meshes
        match part {
            CharacterPart::BaseMesh => {
                if let Some(handle) = basemesh.get_rigged_mesh_handle(
                    prefab_name, &*prefab, &mut *meshes, &mut *images, &*rig_data, &*mh_morphs)
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


//pub(crate) fn on_human_assets_changed(
//    mut new_humans: Query<(Entity, &HumanMeshConfig, Option<&HumanAssetConfig>), Added<HumanMeshConfig>>,
//    mut commands: Commands,
//    mut meshes: ResMut<Assets<Mesh>>,
//    mut materials: ResMut<Assets<StandardMaterial>>,
//    mut clips: ResMut<Assets<AnimationClip>>,
//    mut inv_bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
//    global_config: Res<HumentityGlobalConfig>,
//    registry: Res<HumanAssetRegistry>,
//    base_mesh: Res<crate::basemesh::BaseMesh>,
//    targets: Res<MorphTargets>,
//    asset_server: Res<AssetServer>,
//    rigs: Res<rigs::RigData>,
//    vg: Res<VertexGroups>,
//    asset_textures: Res<HumanAssetTextures>,
//) {
//    if new_humans.count() == 0 { return }
//    let transparent_slots = &global_config.transparent_slots;
//
//    new_humans.iter_mut().for_each(|(human, config, mut anim_config)| {
//        // Body Material
//        let albedo = asset_server.load("humentity://skin_textures/albedo/".to_string() + &config.skin_albedo);
//        let material = materials.add(StandardMaterial {
//            base_color_texture: Some(albedo),
//            perceptual_roughness: 1.,
//            ..default()
//        });
//
//        // Get morphed helper mesh
//        let helpers = morphs::adjust_helpers_to_morphs(
//            &config.morph_targets, &targets, &base_mesh
//        );
//        let mut delete_verts = AHashSet::<u16>::default();
//
//        // Setup animation related data
//        let mut skinned_mesh: SkinnedMesh = SkinnedMesh::default();
//        let mut sorted_bones: Vec<String> = vec![];
//        let mut inv_bindposes: Vec<Mat4> = vec![];
//        let mut local_bone_transforms: AHashMap<String, Transform> = AHashMap::default();
//        if let Some(anim_config) = anim_config.as_mut() {
//            (skinned_mesh, sorted_bones, inv_bindposes, local_bone_transforms) = rigs::build_rig(
//                &human, anim_config.rig, &rigs, &mut inv_bindpose_assets, &mut commands, &vg, &helpers
//            );
//        }
//
//        // Body Parts
//        for bp in config.body_parts.iter() {
//
//            // Set up mesh
//            let asset = registry.body_parts.get(bp)
//                .expect(&format!("FAILED TO FIND BODY PART {}", bp));
//            //delete_verts.extend(&asset.delete_verts);
//            let mesh = morphs::bake_asset_morphs(
//                &config.morph_targets, &targets, &mut meshes, &helpers, &asset,
//            );
//            let mesh_handle: Handle<Mesh> = if let Some(anim_config) = anim_config.as_mut() {
//                rigs::set_asset_rig_arrays(
//                    anim_config.rig, mesh, &rigs, &asset.mhid_lookup, &mut meshes, &asset.helper_maps, &sorted_bones,
//                )
//            } else {
//                meshes.add(mesh)
//            };
//
//            // Set up material
//            let mut material = StandardMaterial::default();
//            if let Some(albedos) = asset_textures.albedo_maps.get(&asset.name) {
//                if albedos.len() > 0 { material.base_color_texture = Some(albedos[0].clone()); }
//            }
//            if let Some(normal) = asset_textures.normal_map.get(&asset.name) {
//                material.normal_map_texture = Some(normal.clone());
//            }
//            if let Some(ao) = asset_textures.ao_map.get(&asset.name) {
//                material.occlusion_texture = Some(ao.clone());
//            }
//            for slot in asset.slots.iter() {
//                if transparent_slots.contains(slot) {
//                    material.alpha_mode = AlphaMode::Blend;
//                    material.reflectance = 0.25;
//                    if slot.contains("Eyebrow") { material.base_color = config.eyebrow_color; }
//                    else if slot.contains("Eye") && !slot.contains("Eyelash") { material.base_color = config.eye_color; }
//                    else if slot.contains("Hair") { material.base_color = config.hair_color; }
//                }
//            }
//            let material = materials.add(material);
//
//            // Spawn entity
//            let bp = commands.spawn((
//                Mesh3d(mesh_handle),
//                MeshMaterial3d(material),
//                Transform::IDENTITY,
//            )).id();
//            if let Some(_) = anim_config {
//                commands.entity(bp).insert(skinned_mesh.clone());
//            }
//            commands.entity(human).add_child(bp);
//        }
//
//        // Equipment
//        for eq in config.equipment.iter() {
//            // Set up mesh
//            let asset = registry.equipment.get(eq)
//                .expect(&format!("FAILED TO FIND EQUIPMENT {}", eq));
//            delete_verts.extend(&asset.delete_verts);
//            let mesh = morphs::bake_asset_morphs(
//                &config.morph_targets, &targets, &mut meshes, &helpers, &asset,
//            );
//            let mesh_handle: Handle<Mesh> = if let Some(anim_config) = anim_config.as_mut() {
//                rigs::set_asset_rig_arrays(
//                    anim_config.rig, mesh, &rigs, &asset.mhid_lookup, &mut meshes, &asset.helper_maps, &sorted_bones,
//                )
//            } else {
//                meshes.add(mesh)
//            };
//
//            // Set up material
//            let mut material = StandardMaterial::default();
//            if let Some(albedos) = asset_textures.albedo_maps.get(&asset.name) {
//                if albedos.len() > 0 { material.base_color_texture = Some(albedos[0].clone()); }
//            }
//            if let Some(normal) = asset_textures.normal_map.get(&asset.name) {
//                material.normal_map_texture = Some(normal.clone());
//            }
//            if let Some(ao) = asset_textures.ao_map.get(&asset.name) {
//                material.occlusion_texture = Some(ao.clone());
//            }
//
//            // Spawn entity
//            let asset = commands.spawn((
//                skinned_mesh.clone(),
//                Mesh3d(mesh_handle),
//                MeshMaterial3d(materials.add(material)),
//                Transform::IDENTITY,
//            )).id();
//            if let Some(_) = anim_config {
//                commands.entity(asset).insert(skinned_mesh.clone());
//            }
//            commands.entity(human).add_child(asset);
//        }
//
//        // Body Mesh
//        let mesh = assets::delete_mesh_verts(&mut meshes, &base_mesh, delete_verts);
//        let mesh = morphs::bake_body_morphs(&mesh, &base_mesh.mhid_lookup, &helpers);
//        let mesh_handle: Handle<Mesh> = if let Some(anim_config) = anim_config.as_mut() {
//            rigs::set_basemesh_rig_arrays(anim_config.rig, mesh, &rigs, &base_mesh.mhid_lookup, &mut meshes, &sorted_bones)
//        } else {
//            meshes.add(mesh)
//        };
//        
//        // Spawn avatar entity
//        let avatar = commands.spawn((
//            Name::new("Avatar"),
//            Mesh3d(mesh_handle),
//            MeshMaterial3d(material),
//            Transform::IDENTITY,
//        )).id();
//        if let Some(_) = anim_config {
//            commands.entity(avatar).insert(skinned_mesh.clone());
//        }
//        commands.entity(human).add_child(avatar);
//
//        if let Some(anim_config) = anim_config.as_mut() {
//            let glbs = anim_config.animation_glbs.clone();
//            for path in glbs.iter() {
//                let clips = get_animation_clips(path, &local_bone_transforms)
//                    .expect("Failed to build animation clips")
//                    .iter()
//                    .map(|(k, v)| (k.clone(), clips.add(v.clone())))
//                    .collect::<AHashMap<String, Handle<AnimationClip>>>();
//                anim_config.clip_handles.extend(clips);
//
//            }
//        }
//    })
//
//}
//
//