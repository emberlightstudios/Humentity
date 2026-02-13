use ahash::AHashMap;
use bevy::{ecs::intern::Internable, mesh::{morph::{MeshMorphWeights, MorphTargetImage}, skinning::SkinnedMesh}, prelude::*, tasks::AsyncComputeTaskPool};
use crossbeam_channel::{Sender, Receiver};
use crate::{NAME_INTERNER, assets::{CharacterAssetData, CharacterAssetRegistry, CharacterPart, StitchedPart, StitchedParts, parse_character_asset},
    morphs::{MakeHumanMorphs, MorphTargets}, prefab::{CharacterArchetypePrefab, CharacterArchetypePrefabs, PrefabOverride}, prelude::BaseMesh,
    rigs::{BoneTranslationData, RigData, SkeletonCaches}};
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

/// The state of a mesh load process for character parts
#[derive(PartialEq, Eq, Debug, Default, Clone)]
pub enum AssetLoadState {
    #[default]
    None,
    LoadingData,
    LoadingObj,
    BuildingMesh,
    Finished
}

/// A resource for communicating with background threads for mesh loading
#[derive(Resource, Deref, Default)]
pub struct AssetLoadingMediators(AHashMap<LoadAssetMeshJob, (LoadingMediator, AssetLoadState)>);

/// The type of a mesh load job, single CharacterPart or multiple parts in a StitchedMesh
#[derive(Eq, PartialEq, Hash, Clone, Debug)]
pub enum LoadAssetMeshJob {
    Single{ part: CharacterPart, prefab_name: &'static str },
    Stitched{ parts: StitchedParts, prefab_name: &'static str },
}

impl AssetLoadingMediators {
    /// Trigger a rebuild of a mesh or group of stitched meshes
    pub fn trigger(&mut self, key: LoadAssetMeshJob) {
        if !self.0.contains_key(&key) {
            self.0.insert(key, (LoadingMediator::default(), AssetLoadState::None));
        }
    }
    
    /// Removes the key when finished
    fn finish(&mut self, key: &LoadAssetMeshJob) {
        self.0.remove(key);
    }
}

/// A wrapper around crossbeam channels for communicating with the background threads
#[derive(Clone)]
pub struct LoadingMediator {
    pub(crate) mesh_building_msg_sender: Sender<MeshConstructedMsg>,
    pub(crate) mesh_building_msg_receiver: Receiver<MeshConstructedMsg>,
    pub(crate) asset_loading_msg_sender: Sender<AssetLoadedMsg>,
    pub(crate) asset_loading_msg_receiver: Receiver<AssetLoadedMsg>,
}

impl Default for LoadingMediator {
    fn default() -> Self {
        let ( asset_loading_msg_sender, asset_loading_msg_receiver ) = crossbeam_channel::unbounded();
        let ( mesh_building_msg_sender, mesh_building_msg_receiver ) = crossbeam_channel::unbounded();
        Self { asset_loading_msg_receiver, asset_loading_msg_sender, mesh_building_msg_receiver, mesh_building_msg_sender }
    }
}

/// A message sent from the bg thread when the mhclo file has finished loading
pub(crate) struct AssetLoadedMsg {
    part: CharacterPart,
    data: CharacterAssetData,
}

/// A message sent from the bg thread when the mesh or meshes are ready
pub(crate) struct MeshConstructedMsg {
    pub(crate) final_meshes: Vec<Mesh>,
    pub(crate) morph_names: Vec<Vec<String>>,
    pub(crate) morph_images: Vec<MorphTargetImage>, 
}

/// Handles parsing .mhclo, loading obj, fixing normals and cachine mesh handles for CharacterPart
pub(crate) fn handle_single_mesh_load_tasks(
    parts: Query<(Entity, &CharacterPart, &ChildOf, Option<&PrefabOverride>), Without<Mesh3d>>,
    character_root: Query<(&CharacterShapeConfig, &SkinnedMesh)>,
    prefabs: Res<CharacterArchetypePrefabs>,
    mut asset_registry: ResMut<CharacterAssetRegistry>,
    meshes: Res<Assets<Mesh>>,
    mut commands: Commands,
    mut mediators: ResMut<AssetLoadingMediators>,
) {
    if parts.count() == 0 { return; }

    for (entity, &part, parent, prefab_override) in parts {
        let Ok((config, skinned_mesh)) =
                    character_root.get(parent.parent())
            // SkinnedMesh component will be added to the character root only
            // after the skeleton is fit. Wait for this before inserting the mesh
            // which requires this component
            else { continue };

        let prefab_name = if let Some(p) = prefab_override { p.0 } else { config.prefab };

        let Some(asset) = asset_registry.get_mut(&part) else {
            error!("No such asset: {:#?} - Cannot load", part);
            commands.entity(entity).despawn();
            continue;
        };

        if let Some(handle) = asset.mesh_handles.get(prefab_name) {
            if let Some(mesh) = meshes.get(handle) && mesh.has_morph_targets(){
                let morph_weights = prefabs[prefab_name]
                    .shapes
                    .iter()
                    .map(|s| *config.prefab_morph_targets.get(s.name).unwrap_or(&0.))
                    .collect::<Vec<_>>();
                let morph_weights = MeshMorphWeights::new(morph_weights).unwrap();
                commands.entity(entity).insert(morph_weights);
            }
            commands.entity(entity).insert((
                Mesh3d(handle.clone()),
                skinned_mesh.clone(),
            ));
        } else {
            mediators.trigger(LoadAssetMeshJob::Single { part, prefab_name });
        }
    }
}

/// Handles parsing .mhclo, loading obj, fixing normals and cachine mesh handles for StitchedPart
pub(crate) fn handle_stitched_mesh_load_tasks(
    parts_lists: Query<(Entity, &StitchedParts, &ChildOf)>,
    character_root: Query<(&CharacterShapeConfig, &SkinnedMesh)>,
    prefabs: Res<CharacterArchetypePrefabs>,
    mut asset_registry: ResMut<CharacterAssetRegistry>,
    mut commands: Commands,
    meshes: ResMut<Assets<Mesh>>,
    mut mediators: ResMut<AssetLoadingMediators>,
) {
    if parts_lists.count() == 0 { return; }

    'outer: for (entity, collection, parent) in parts_lists {
        let Ok((config, skinned_mesh)) =
                    character_root.get(parent.parent())
            // SkinnedMesh component will be added to the character root only
            // after the skeleton is fit. Wait for this before inserting the mesh
            // which requires this component
            else { continue };
        
        let mut done = true;

        for part in collection.iter() {
            let StitchedPart { part, prefab_override } = part;

            let Some(asset) = asset_registry.get_mut(part) else {
                error!("No such asset: {:#?} - Cannot load", part);
                commands.entity(entity).despawn();
                continue 'outer;
            };

            let prefab = if let Some(ov) = prefab_override { ov.0 } else { config.prefab };
            if asset.mesh_handles.get(prefab).is_none() {
                done = false;
            }
        }

        if done {
            let root = parent.parent();

            for part in collection.iter() {
                let StitchedPart { part, prefab_override } = part;
                let prefab_name = if let Some(ov) = prefab_override { ov.0 } else { config.prefab };
                let Some(asset) = asset_registry.get_mut(part) else { continue };
                let Some(handle) = asset.mesh_handles.get(prefab_name) else { continue };
                let prefab = &prefabs[prefab_name];

                let child = commands.spawn_empty().id();
                commands.entity(root).add_child(child);

                if let Some(mesh) = meshes.get(handle) && mesh.has_morph_targets(){
                    let morph_weights = prefab
                        .shapes
                        .iter()
                        .map(|s| *config.prefab_morph_targets.get(s.name).unwrap_or(&0.))
                        .collect::<Vec<_>>();
                    let morph_weights = MeshMorphWeights::new(morph_weights).unwrap();
                    commands.entity(child).insert(morph_weights);
                }
                commands.entity(child).insert((
                    *part,
                    skinned_mesh.clone(),
                    Mesh3d(handle.clone()),
                ));
            }
            commands.entity(entity).despawn();

        } else { // not done yet
            mediators.trigger(LoadAssetMeshJob::Stitched{ parts: collection.clone(), prefab_name: config.prefab });
        }

    }
}

/// This system runs in phases, so it gets triggered multiple times to load a mesh
/// State is tracked by [`AssetLoadState`]
pub(crate) fn mesh_build(
    prefabs: ResMut<CharacterArchetypePrefabs>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut asset_registry: ResMut<CharacterAssetRegistry>,
    morphs: Res<MakeHumanMorphs>,
    basemesh: Res<BaseMesh>,
    rig_data: Res<RigData>,
    asset_server: Res<AssetServer>,
    sk_cache: Res<SkeletonCaches>,
    mut mediators: ResMut<AssetLoadingMediators>,
) {
    for (key, (mediator, load_state)) in mediators.0.iter_mut() {
        match key {
            LoadAssetMeshJob::Single { part, prefab_name } => {
                build_single_mesh_process(
                    mediator, load_state, *part, prefab_name, &prefabs, &mut meshes, &mut asset_registry,
                    &morphs, &basemesh, &rig_data, &asset_server, &sk_cache,
                );

                for msg in mediator.mesh_building_msg_receiver.try_iter() {
                    let mesh = handle_single_mesh_complete(msg, &prefabs[prefab_name], &mut images);
                    let handle = meshes.add(mesh);
                    let Some(asset) = asset_registry.get_mut(part) else { continue };
                    asset.mesh_handles.insert(prefab_name, handle.clone());
                    asset.raw_mesh_handle = None;
                    *load_state = AssetLoadState::Finished;
                }
            }
            LoadAssetMeshJob::Stitched{ parts, prefab_name } => {
                build_stitched_meshes_process(
                    mediator, load_state, parts, prefab_name, &prefabs, &mut meshes, &mut asset_registry,
                    &morphs, &basemesh, &rig_data, &asset_server, &sk_cache, 
                );

                for msg in mediator.mesh_building_msg_receiver.try_iter() {
                    let prefab_names = parts
                        .iter()
                        .map(|p| p.prefab_override.as_ref().map_or(*prefab_name, |ov| ov.0))
                        .collect::<Vec<_>>();
                    
                    let prefabs = prefab_names
                        .iter()
                        .map(|p| &prefabs[p])
                        .collect::<Vec<_>>();

                    let new_meshes = handle_stitched_mesh_complete(msg, &prefabs, &mut images);

                    for (i_mesh, mesh) in new_meshes.into_iter().enumerate() {
                        let handle = meshes.add(mesh);
                        let Some(asset) = asset_registry.get_mut(&parts[i_mesh].part) else { continue };
                        asset.mesh_handles.insert(prefab_names[i_mesh], handle.clone());
                        asset.raw_mesh_handle = None;
                    }
                    *load_state = AssetLoadState::Finished;
                }
            }
        }
    }
}

/// Handles the message for a completed single mesh and builds the final mesh
pub(crate) fn handle_single_mesh_complete(
    msg: MeshConstructedMsg,
    prefab: &CharacterArchetypePrefab,
    images: &mut Assets<Image>,
) -> Mesh {
    let MeshConstructedMsg { final_meshes, morph_names, morph_images } = msg;
    let mut mesh = final_meshes.into_iter().next().unwrap();

    if !prefab.shapes.is_empty() {
        let morph_names = morph_names.into_iter().next().unwrap();
        let morph_image = morph_images.into_iter().next().unwrap();
        let image = images.add(morph_image.0);
        mesh = mesh
            .with_morph_target_names(morph_names)
            .with_morph_targets(image);
    }
    mesh
}

/// Handles the message for a completed stitched mesh and builds the final meshes
pub(crate) fn handle_stitched_mesh_complete(
    msg: MeshConstructedMsg,
    prefabs: &[&CharacterArchetypePrefab],
    images: &mut Assets<Image>,
) -> Vec<Mesh> {
    let MeshConstructedMsg { final_meshes, morph_names, morph_images } = msg;
    let mut meshes = vec![];

    for (i_mesh, ((mut mesh, image), names)) in final_meshes
            .into_iter()
            .zip(morph_images)
            .zip(morph_names)
            .enumerate()
    {
        let prefab = prefabs[i_mesh];
        if !prefab.shapes.is_empty() {
            let morph_names = names;
            let image = images.add(image.0);
            mesh = mesh
                .with_morph_target_names(morph_names)
                .with_morph_targets(image);
            meshes.push(mesh)
        }
    }
    meshes
}

/// Handles the actual steps involved in constructing a single mesh
pub(crate) fn build_single_mesh_process(
    mediator: &LoadingMediator,
    load_state: &mut AssetLoadState,
    part: CharacterPart,
    prefab_name: &'static str,
    prefabs: &CharacterArchetypePrefabs,
    meshes: &mut Assets<Mesh>,
    asset_registry: &mut CharacterAssetRegistry,
    morphs: &MakeHumanMorphs,
    basemesh: &BaseMesh,
    rig_data: &RigData,
    asset_server: &AssetServer,
    sk_cache: &SkeletonCaches,
) {
    let pool = AsyncComputeTaskPool::get();
    let Some(asset) = asset_registry.get_mut(&part) else { return };
    let prefab = prefabs.get(prefab_name).expect("No such prefab");


    // Start loading data if we haven't already
    if *load_state == AssetLoadState::None {
        *load_state = AssetLoadState::LoadingData;
        if asset.data.is_none()  {
            let path = asset.paths.mh_file.clone();
            let sender = mediator.asset_loading_msg_sender.clone();

            pool.spawn(async move {
                let data = parse_character_asset(&path);
                sender.send(AssetLoadedMsg { data, part })
            })
            .detach();
            return;
        }
    }

    // Check if asset data just loaded, set data
    for AssetLoadedMsg { data, .. } in mediator.asset_loading_msg_receiver.try_iter() {
        asset.data = Some(data);
    }

    if *load_state == AssetLoadState::LoadingData &&
        let Some(data) = &mut asset.data
    {
        // Load obj if not loaded
        if asset.raw_mesh_handle.is_none() {
            let handle: Handle<Mesh> = data.obj_file.load_asset(asset_server);
            asset.raw_mesh_handle = Some(handle);
            *load_state = AssetLoadState::LoadingObj
        }
    }

    if *load_state == AssetLoadState::LoadingObj
                && let Some(handle) = &asset.raw_mesh_handle
                && let Some(input_mesh) = meshes.get(handle)
    {
        *load_state = AssetLoadState::BuildingMesh;

        let input_mesh = input_mesh.clone();
        let data = asset.data.as_ref().unwrap().clone();
        let prefab = prefab.clone();
        let mh_morphs = morphs.targets.clone();
        let basemesh = basemesh.clone();
        let rig_weights = rig_data.weights.clone();
        let sk_cache = sk_cache[&prefab.rig.rig_type].clone();

        let sender = mediator.mesh_building_msg_sender.clone();

        pool.spawn(async move {
            let (mesh, morph_names, morph_image) = data.build_final_mesh(
                &input_mesh, &prefab, &mh_morphs, &basemesh, &rig_weights, &sk_cache
            );
            sender.send(MeshConstructedMsg {
                final_meshes: vec![mesh],
                morph_names: vec![morph_names],
                morph_images: vec![morph_image],
            })
        }).detach()
    }
}

/// Handles the actual steps involved in constructing a stitched mesh
fn build_stitched_meshes_process(
    mediator: &LoadingMediator,
    load_state: &mut AssetLoadState,
    parts: &StitchedParts,
    prefab_name: &'static str,
    prefabs: &CharacterArchetypePrefabs,
    meshes: &mut Assets<Mesh>,
    asset_registry: &mut CharacterAssetRegistry,
    morphs: &MakeHumanMorphs,
    basemesh: &BaseMesh,
    rig_data: &RigData,
    asset_server: &AssetServer,
    sk_cache: &SkeletonCaches,
) {
    let pool = AsyncComputeTaskPool::get();
                
    // Start loading data if we haven't already
    if *load_state == AssetLoadState::None {
        *load_state = AssetLoadState::LoadingData;
        for StitchedPart { part, .. } in parts.iter() {
            let part = *part;
            let Some(asset) = asset_registry.get(&part) else { continue };
            if asset.data.is_none() {
                let path = asset.paths.mh_file.clone();
                let sender = mediator.asset_loading_msg_sender.clone();
                pool.spawn(async move {
                    let data = parse_character_asset(&path);
                    sender.send(AssetLoadedMsg { data, part })
                })
                .detach();
            }
        }
    }

    // Check if asset data just loaded, set data
    for AssetLoadedMsg { data, part } in mediator.asset_loading_msg_receiver.try_iter() {
        let Some(asset) = asset_registry.get_mut(&part) else { continue };
        asset.data = Some(data);
    }

    let still_loading = parts
        .iter()
        .map(|p| p.part)
        .filter_map(|p| asset_registry.get(&p))
        .any(|p| p.data.is_none());

    if still_loading { return }

    if *load_state == AssetLoadState::LoadingData {
        *load_state = AssetLoadState::LoadingObj;
        // Load obj if not loaded
        for StitchedPart { part, .. } in parts.iter() {
            let Some(asset) = asset_registry.get_mut(part) else { continue };
            if let Some(data) = &asset.data && asset.raw_mesh_handle.is_none() {
                let handle: Handle<Mesh> = data.obj_file.load_asset(asset_server);
                asset.raw_mesh_handle = Some(handle);
            }
        }
        return;
    }

    if *load_state == AssetLoadState::LoadingObj {
        let assets = parts
            .iter()
            .map(|p| &p.part)
            .map(|p| asset_registry.get(p).unwrap())
            .collect::<Vec<_>>();

        let meshes = assets
            .iter()
            .filter(|a| a.raw_mesh_handle.is_some())
            .filter_map(|a| meshes.get(a.raw_mesh_handle.as_ref().unwrap()))
            .collect::<Vec<_>>();

        if meshes.len() != parts.len() { return }

        *load_state = AssetLoadState::BuildingMesh;

        let mut meshes = meshes
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();

        let asset_data = assets
            .iter()
            .filter_map(|a| a.data.clone())
            .collect::<Vec<_>>();

        let prefab_names = parts
            .iter()
            .map(|p| &p.prefab_override)
            .map(|o| { if let Some(p) = o { p.0 } else { prefab_name }})
            .collect::<Vec<_>>();

        let rig = prefabs[prefab_names[0]].rig.rig_type;

        let prefabs = prefab_names
            .iter()
            .map(|p| &prefabs[p])
            .cloned()
            .collect::<Vec<_>>();

        let mh_morphs = morphs.targets.clone();
        let basemesh = basemesh.clone();
        let rig_weights = rig_data.weights.clone();
        //  I guess we have to assume one rig, even with prefab overrides
        let sk_cache = sk_cache[&rig].clone();

        let sender = mediator.mesh_building_msg_sender.clone();
        pool.spawn(async move {
            let (final_meshes, morph_names, morph_images) = CharacterAssetData::build_final_meshes(
                &asset_data, &mut meshes, &prefabs, &mh_morphs, &basemesh, &rig_weights, &sk_cache
            );
            sender.send(MeshConstructedMsg {
                final_meshes,
                morph_names,
                morph_images,
            })
        }).detach();
    }
}

/// Monitors for finished jobs and removes their mediators
pub(crate) fn mediators_clean_up(mut mediators: ResMut<AssetLoadingMediators>) {
    let n = mediators.len();
    if n > 0 {
        let mut remove = vec![];
        for (key, (_, state)) in mediators.iter() {
            if *state == AssetLoadState::Finished {
                remove.push(key.clone());
            }
        }

        for key in remove {
            mediators.finish(&key);
        }
    }
}
