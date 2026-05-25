use std::sync::{Arc, RwLock};

use ahash::AHashMap;
use bevy::{ecs::intern::Internable, mesh::morph::{MeshMorphWeights, MorphTargetImage}, prelude::*, tasks::AsyncComputeTaskPool};
use crossbeam_channel::{Receiver, Sender};
use crate::{NAME_INTERNER, assets::{CharacterPart, StitchedPart, StitchedParts, build_final_mesh_mhclo, build_final_meshes_mhclo}, basemesh::BaseMesh, loaders::{MhcloAsset, ObjVertsAsset, TargetAsset}, morphs::{MakeHumanMorphs, MorphTargets}, template::{CharacterTemplate, CharacterTemplates, CharacterMorphShapes}, rigs::{BoneTranslationData, RigData, RigSpec}};
use serde::{Deserialize, Deserializer, Serialize};


/// Defines the shape of a character.  Place it at the root, with individual parts as children.
#[derive(Component, Clone, Default, Debug, Serialize)]
#[require(Visibility)]
pub struct CharacterShapeConfig {
    pub template_morph_targets: MorphTargets,
    pub template: &'static str,
    #[serde(skip)]
    pub(crate) bone_translations: BoneTranslationData,
    #[serde(skip)]
    pub(crate) bone_delta_rotations: AHashMap<&'static str, Quat>,
}

impl CharacterShapeConfig {
    pub fn get_morph_weights_component(&self, template: &CharacterTemplate) -> MeshMorphWeights {
        let morph_weights = template
            .shapes
            .iter()
            .map(|s| *self.template_morph_targets.get(s.name).unwrap_or(&0.))
            .collect::<Vec<_>>();
        MeshMorphWeights::new(morph_weights).unwrap()
    }
}

impl<'de> Deserialize<'de> for CharacterShapeConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            template_morph_targets: MorphTargets,
            template: String,
        }

        let raw = Raw::deserialize(deserializer)?;
        let template: &'static str = NAME_INTERNER.intern(&raw.template).leak();

        Ok(Self::new(template, raw.template_morph_targets))
    }
}

impl CharacterShapeConfig {
    pub fn new(template: &'static str, morphs: MorphTargets) -> Self {
        Self {
            template,
            template_morph_targets: morphs,
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
    LoadedObj,
    BuildSubmitted,
    Finished
}

#[derive(Resource, Deref, DerefMut, Default)]
pub struct CachedMhcloMeshHandles(AHashMap<(Handle<MhcloAsset>, &'static str), Handle<Mesh>>);

pub(crate) struct RawMeshCache {
    pub(crate) mesh: Handle<Mesh>,
    pub(crate) verts: Handle<ObjVertsAsset>,
}

#[derive(Resource, Deref, DerefMut, Default)]
pub(crate) struct CachedMhcloRawMeshHandles(AHashMap<Handle<MhcloAsset>, RawMeshCache>);


/// A resource for communicating with background threads for mesh loading
#[derive(Resource, Deref, Default)]
pub struct MhcloMeshBuilder(AHashMap<LoadAssetMeshJob, (LoadingMediator, AssetLoadState)>);

/// The type of a mesh load job, single CharacterMesh or multiple parts in a StitchedMesh
#[derive(Eq, PartialEq, Hash, Clone, Debug)]
pub enum LoadAssetMeshJob {
    Single{ part: CharacterPart, template_name: &'static str },
    Stitched{ parts: StitchedParts, template_name: &'static str },
}

impl MhcloMeshBuilder {
    /// Trigger a rebuild of a mesh or group of stitched meshes
    pub fn trigger(&mut self, key: LoadAssetMeshJob) {
        self.0.insert(key, (LoadingMediator::default(), AssetLoadState::None));
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
}

impl Default for LoadingMediator {
    fn default() -> Self {
        let ( mesh_building_msg_sender, mesh_building_msg_receiver ) = crossbeam_channel::unbounded();
        Self { mesh_building_msg_receiver, mesh_building_msg_sender }
    }
}

/// A message sent from the bg thread when the mesh or meshes are ready
pub(crate) struct MeshConstructedMsg {
    pub(crate) final_meshes: Vec<Mesh>,
    pub(crate) morph_names: Vec<Vec<String>>,
    pub(crate) morph_images: Vec<MorphTargetImage>, 
}

/// This system runs in phases, so it gets triggered multiple times to load a mesh
/// State is tracked by [`AssetLoadState`]
pub(crate) fn mesh_build(
    templates: ResMut<CharacterTemplates>,
    mut meshes: ResMut<Assets<Mesh>>,
    mesh_verts: Res<Assets<ObjVertsAsset>>,
    mhclo_assets: Res<Assets<MhcloAsset>>,
    mut images: ResMut<Assets<Image>>,
    mut morphs: ResMut<MakeHumanMorphs>,
    basemesh: Res<BaseMesh>,
    rig_data: Res<RigData>,
    asset_server: Res<AssetServer>,
    mut cached_meshes: ResMut<CachedMhcloMeshHandles>,
    mut cached_raw_meshes: ResMut<CachedMhcloRawMeshHandles>,
    mut mediators: ResMut<MhcloMeshBuilder>,
) {
    for (key, (mediator, load_state)) in mediators.0.iter_mut() {
        match key {
            LoadAssetMeshJob::Single { part, template_name } => {
                build_single_mesh_process(
                    mediator, load_state, part, template_name, &templates, &mut meshes, &mesh_verts, &mhclo_assets,
                    &mut morphs, &basemesh.0, &rig_data, &asset_server, &mut cached_raw_meshes,
                );

                for msg in mediator.mesh_building_msg_receiver.try_iter() {
                    let mesh = handle_single_mesh_complete(msg, &templates[template_name], &mut images);
                    let mesh_handle = meshes.add(mesh);
                    cached_meshes.insert((part.0.clone(), template_name), mesh_handle.clone());
                    *load_state = AssetLoadState::Finished;
                }
            }
            LoadAssetMeshJob::Stitched{ parts, template_name } => {
                build_stitched_meshes_process(
                    mediator, load_state, parts, template_name, &templates, &mut meshes, &mesh_verts,
                    &mhclo_assets, &mut morphs, &basemesh.0, &rig_data, &asset_server,
                    &mut cached_raw_meshes,
                );

                for msg in mediator.mesh_building_msg_receiver.try_iter() {
                    let template_names = parts
                        .iter()
                        .map(|p| p.template_override.as_ref().map_or(*template_name, |ov| ov.0))
                        .collect::<Vec<_>>();
                    
                    let templates = template_names
                        .iter()
                        .map(|p| &templates[p])
                        .collect::<Vec<_>>();

                    let new_meshes = handle_stitched_mesh_complete(msg, &templates, &mut images);

                    for (i_mesh, mesh) in new_meshes.into_iter().enumerate() {
                        let handle = parts[i_mesh].part.clone();
                        let mesh_handle = meshes.add(mesh);
                        cached_meshes.insert((handle, template_name), mesh_handle.clone());
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
    template: &CharacterTemplate,
    images: &mut Assets<Image>,
) -> Mesh {
    let MeshConstructedMsg { final_meshes, morph_names, morph_images } = msg;
    let mut mesh = final_meshes.into_iter().next().unwrap();

    if !template.shapes.is_empty() {
        let morph_names = morph_names.into_iter().next().unwrap();
        let morph_image = morph_images.into_iter().next().unwrap();
        let image = images.add(morph_image.0);
        mesh = mesh
            .with_morph_target_names(morph_names)
            .with_morph_targets(image);
    }
    mesh
}

/// Build a single mesh directly from loaded assets, bypassing the async job system.
/// All dependencies must already be loaded in their respective asset stores.
pub fn build_single_mesh_direct(
    mhclo: &MhcloAsset,
    input_mesh: &Mesh,
    mesh_verts: &ObjVertsAsset,
    template: &CharacterTemplate,
    mh_morphs: Arc<RwLock<AHashMap<&'static str, TargetAsset>>>,
    basemesh: Arc<Vec<Vec3>>,
    rig_spec: &RigSpec,
    images: &mut Assets<Image>,
) -> Mesh {
    let (mesh, morph_names, morph_image) = build_final_mesh_mhclo(
        mhclo, input_mesh, mesh_verts, template, mh_morphs, basemesh, rig_spec,
    );
    let msg = MeshConstructedMsg {
        final_meshes: vec![mesh],
        morph_names: vec![morph_names],
        morph_images: vec![morph_image],
    };
    handle_single_mesh_complete(msg, template, images)
}

/// Handles the message for a completed stitched mesh and builds the final meshes
pub(crate) fn handle_stitched_mesh_complete(
    msg: MeshConstructedMsg,
    templates: &[&CharacterTemplate],
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
        let template = templates[i_mesh];
        if !template.shapes.is_empty() {
            let morph_names = names;
            let image = images.add(image.0);
            mesh = mesh
                .with_morph_target_names(morph_names)
                .with_morph_targets(image);
        }
        meshes.push(mesh);
    }
    meshes
}

/// Handles the actual steps involved in constructing a single mesh
pub(crate) fn build_single_mesh_process(
    mediator: &LoadingMediator,
    load_state: &mut AssetLoadState,
    part: &CharacterPart,
    template_name: &'static str,
    templates: &CharacterTemplates,
    meshes: &mut Assets<Mesh>,
    mesh_verts: &Assets<ObjVertsAsset>,
    mhclo_assets: &Assets<MhcloAsset>,
    morphs: &mut MakeHumanMorphs,
    basemesh: &Arc<Vec<Vec3>>,
    rig_data: &RigData,
    asset_server: &AssetServer,
    cached_raw_meshes: &mut CachedMhcloRawMeshHandles,
) {
    let pool = AsyncComputeTaskPool::get();
    let template = templates.get(template_name).expect("No such template");

    let Some(mhclo) = mhclo_assets.get(&**part) else {
        return;
    };

    // Start loading mhclo if not already
    if *load_state == AssetLoadState::None {
        *load_state = AssetLoadState::LoadedObj;
        if !cached_raw_meshes.contains_key(&**part) {
            let mesh = asset_server.load::<Mesh>(mhclo.obj_file.clone());
            let verts = asset_server.load::<ObjVertsAsset>(mhclo.obj_file.clone());
            cached_raw_meshes.insert(part.0.clone(), RawMeshCache { mesh, verts });
            return;
        }
    }

    if *load_state == AssetLoadState::LoadedObj
            && let Some(cache) = cached_raw_meshes.get(&**part)
            && let Some(input_mesh) = meshes.get(&cache.mesh)
            && let Some(mesh_verts) = mesh_verts.get(&cache.verts)
    {
        *load_state = AssetLoadState::BuildSubmitted;
        cached_raw_meshes.remove(&**part);

        let input_mesh = input_mesh.clone();
        let mesh_verts = mesh_verts.clone();
        let template = template.clone();
        let mh_morphs = morphs.targets.clone();
        let Some(rig_entry) = rig_data.get(&template.rig) else {
            return;
        };
        let rig_spec = rig_entry.clone();

        let sender = mediator.mesh_building_msg_sender.clone();
        let mhclo = mhclo.clone();
        let basemesh = basemesh.clone();

        pool.spawn(async move {
            let (mesh, morph_names, morph_image) = build_final_mesh_mhclo(
                &mhclo, &input_mesh, &mesh_verts, &template, mh_morphs, basemesh, &rig_spec
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
    template_name: &'static str,
    templates: &CharacterTemplates,
    meshes: &mut Assets<Mesh>,
    mesh_verts: &Assets<ObjVertsAsset>,
    mhclo_assets: &Assets<MhcloAsset>,
    morphs: &mut MakeHumanMorphs,
    basemesh: &Arc<Vec<Vec3>>,
    rig_data: &RigData,
    asset_server: &AssetServer,
    cached_raw_meshes: &mut CachedMhcloRawMeshHandles,
) {
    let pool = AsyncComputeTaskPool::get();

    // Resolve all mhclo assets; bail if any part's asset isn't loaded yet
    let mhclos_ready: Vec<_> = parts
        .iter()
        .map(|p| mhclo_assets.get(&p.part))
        .collect();

    if mhclos_ready.iter().any(|o| o.is_none()) {
        return;
    }

    // Phase 1: ensure raw OBJ meshes are loading (same pattern as single-mesh path)
    if *load_state == AssetLoadState::None {
        *load_state = AssetLoadState::LoadedObj;
        for StitchedPart { part, .. } in parts.iter() {
            if !cached_raw_meshes.contains_key(&*part) {
                let mhclo = mhclo_assets.get(&*part).expect("mhclo already checked");
                let mesh = asset_server.load::<Mesh>(mhclo.obj_file.clone());
                let verts = asset_server.load::<ObjVertsAsset>(mhclo.obj_file.clone());
                cached_raw_meshes.insert(part.clone(), RawMeshCache { mesh, verts });
            }
        }
        return;
    }

    // Phase 2: when all raw meshes are in Assets<Mesh>, spawn the build task
    if *load_state == AssetLoadState::LoadedObj {
        let raw_handles: Vec<_> = parts
            .iter()
            .map(|p| cached_raw_meshes.get(&p.part))
            .collect();

        let all_ready = raw_handles.iter().all(|h| h.is_some());
        if !all_ready {
            return;
        }

        let loaded_meshes: Vec<_> = raw_handles
            .iter()
            .filter_map(|h| h.and_then(|cache| meshes.get(&cache.mesh)))
            .collect();

        let mesh_verts: Vec<_> = raw_handles
            .iter()
            .filter_map(|h| h.and_then(|cache| mesh_verts.get(&cache.verts)))
            .collect();

        if loaded_meshes.len() != parts.len() {
            return;
        }

        *load_state = AssetLoadState::BuildSubmitted;

        let mut input_meshes: Vec<Mesh> = loaded_meshes.into_iter().cloned().collect();
        let mesh_verts: Vec<ObjVertsAsset> = mesh_verts.into_iter().cloned().collect();
        let mhclos: Vec<_> = parts
            .iter()
            .map(|p| mhclo_assets.get(&p.part).unwrap().clone())
            .collect();

        let template_names: Vec<&'static str> = parts
            .iter()
            .map(|p| p.template_override.as_ref().map_or(template_name, |ov| ov.0))
            .collect();

        let rig = templates[template_names[0]].rig;
        let templates_for_build: Vec<_> = template_names
            .iter()
            .map(|p| templates[*p].clone())
            .collect();

        let mh_morphs = morphs.targets.clone();
        let basemesh = basemesh.clone();
        let Some(rig_entry) = rig_data.get(&rig) else {
            return;
        };
        let rig_spec = rig_entry.clone();

        parts.iter().for_each(|p| {
            cached_raw_meshes.remove(&p.part);
        });

        let sender = mediator.mesh_building_msg_sender.clone();
        pool.spawn(async move {
            let (final_meshes, morph_names, morph_images) = build_final_meshes_mhclo(
                &mhclos,
                &mut input_meshes,
                &mesh_verts,
                &templates_for_build,
                mh_morphs,
                basemesh,
                &rig_spec,
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
pub(crate) fn mediators_clean_up(mut mediators: ResMut<MhcloMeshBuilder>) {
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
