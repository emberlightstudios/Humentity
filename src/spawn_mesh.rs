use ahash::AHashMap;
use bevy::{ecs::intern::Internable, mesh::{morph::{MeshMorphWeights, MorphTargetImage}, skinning::SkinnedMesh}, prelude::*, tasks::AsyncComputeTaskPool};
use crate::{NAME_INTERNER, assets::{AssetLoadState, CharacterAssetData, CharacterAssetRegistry, CharacterPart, parse_character_asset}, morphs::{MakeHumanMorphs, MorphTargets}, prefab::{CharacterArchetypePrefab, CharacterArchetypePrefabs}, prelude::BaseMesh, rigs::{BoneTranslationData, RigData, SkeletonCaches}};
use serde::{Deserialize, Deserializer, Serialize};


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

#[derive(Message, Deref)]
pub struct CharacterPartMeshSpawned(Entity);

pub(crate) struct AssetLoadedMsg {
    pub(crate) data: CharacterAssetData,
}

pub(crate) struct MeshConstructedMsg {
    pub(crate) final_mesh: Mesh,
    pub(crate) morph_names: Vec<String>,
    pub(crate) morph_image: MorphTargetImage, 
}

#[derive(Event)]
pub struct BuildMesh {
    pub part: CharacterPart,
    pub prefab: &'static str,
}

pub(crate) fn handle_mesh_load_tasks(
    parts: Query<(Entity, &CharacterPart, &ChildOf), Without<Mesh3d>>,
    character_root: Query<(&CharacterShapeConfig, &SkinnedMesh)>,
    prefabs: Res<CharacterArchetypePrefabs>,
    mut asset_registry: ResMut<CharacterAssetRegistry>,
    mut commands: Commands,
) {
    if parts.count() == 0 { return; }

    for (entity, part, parent) in parts {
        let Ok((config, skinned_mesh)) = character_root.get(parent.parent())
            // SkinnedMesh component will be added to the character root only
            // after the skeleton is fit. Wait for this before inserting the mesh
            // which requires this component
            else { continue };

        let Some(asset) = asset_registry.get_mut(part) else {
            error!("No such asset: {:#?} - Cannot load", part);
            commands.entity(entity).despawn();
            continue;
        };

        if let Some(handle) = asset.mesh_handles.get(config.prefab) {
            let prefab = &prefabs[config.prefab];
            add_mesh_component(entity, handle.clone(), config, skinned_mesh, prefab, &mut commands);
        } else {
            commands.trigger(BuildMesh {
                part: part.clone(), prefab: config.prefab
            });
        }
    }
}

fn add_mesh_component(
    entity: Entity,
    mesh_handle: Handle<Mesh>,
    config: &CharacterShapeConfig,
    skinned_mesh: &SkinnedMesh,
    prefab: &CharacterArchetypePrefab,
    commands: &mut Commands,
) {
    commands.entity(entity).insert((
        Mesh3d(mesh_handle),
        skinned_mesh.clone(),
    ));
    if !config.prefab_morph_targets.is_empty() {
        let morph_weights = prefab
            .shapes
            .iter()
            .map(|s| NAME_INTERNER.intern(&s.name).leak())
            .map(|s| *config.prefab_morph_targets.get(s).unwrap_or(&0.))
            .collect::<Vec<_>>();
        let morph_weights = MeshMorphWeights::new(morph_weights).unwrap();
        commands.entity(entity).insert(morph_weights);
    }
}

/// This system runs in phases, so it gets triggered multiple times to load a mesh
pub(crate) fn trigger_mesh_build(
    trigger: On<BuildMesh>,
    mut prefabs: ResMut<CharacterArchetypePrefabs>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut asset_registry: ResMut<CharacterAssetRegistry>,
    morphs: Res<MakeHumanMorphs>,
    basemesh: Res<BaseMesh>,
    rig_data: Res<RigData>,
    asset_server: Res<AssetServer>,
    sk_cache: Res<SkeletonCaches>,
) {
    let prefab_name = trigger.prefab;
    let prefab = prefabs.get_mut(prefab_name)
        .expect("No such prefab");

    let Some(asset) = asset_registry.get_mut(&trigger.part) else {
        error!("No such asset: {:#?} - Cannot load", trigger.part);
        return;
    };

    // Check if mesh construction just finished, add mesh3d component
    for MeshConstructedMsg { final_mesh, morph_names, morph_image }
            in asset.mesh_building_msg_receiver.try_iter()
    {

        let mut mesh = final_mesh;
        if !prefab.shapes.is_empty() {
            let image = images.add(morph_image.0);
            mesh = mesh
                .with_morph_target_names(morph_names)
                .with_morph_targets(image);
        }

        let handle = meshes.add(mesh);
        asset.mesh_handles.insert(prefab_name, handle.clone());
        asset.raw_mesh_handle = None;
        asset.loading = AssetLoadState::None;

        return;
    }

    let pool = AsyncComputeTaskPool::get();

    // Start loading if we haven't already
    if asset.data.is_none() && asset.loading == AssetLoadState::None {
        asset.loading = AssetLoadState::LoadingData;

        let path = asset.paths.mh_file.clone();
        let sender = asset.asset_loading_msg_sender.clone();

        pool.spawn(async move {
            let data = parse_character_asset(&path);
            sender.send(AssetLoadedMsg { data: data })
        })
        .detach();
        return;
    }

    // Check if asset data just loaded, set data
    for AssetLoadedMsg { data } in asset.asset_loading_msg_receiver.try_iter() {
        asset.data = Some(data);
        return;
    }

    if asset.loading != AssetLoadState::BuildingMesh &&
        let Some(data) = &mut asset.data
    {
        // Load obj if not loaded
        if asset.raw_mesh_handle.is_none() {
            let handle: Handle<Mesh> = data.obj_file.load_asset(&asset_server);
            asset.raw_mesh_handle = Some(handle);
        }
        
        // If mesh loaded begin constructing the final mesh
        if let Some(handle) = &asset.raw_mesh_handle &&
                let Some(input_mesh) = meshes.get(handle) {
            asset.loading = AssetLoadState::BuildingMesh;

            let input_mesh = input_mesh.clone();
            let data = asset.data.as_ref().unwrap().clone();
            let prefab = prefab.clone();
            let mh_morphs = morphs.targets.clone();
            let basemesh = basemesh.clone();
            let rig_weights = rig_data.weights.clone();
            let sk_cache = sk_cache[&prefab.rig.rig_type].clone();

            let sender = asset.mesh_building_msg_sender.clone();
            pool.spawn(async move {
                let (final_mesh, morph_names, morph_image) = data.build_final_mesh(
                    &input_mesh, &prefab, &mh_morphs, &basemesh, &rig_weights, &sk_cache
                );
                sender.send(MeshConstructedMsg { final_mesh, morph_names, morph_image })
            }).detach()
        }
    }
}
