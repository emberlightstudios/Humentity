use std::sync::{Arc, RwLock};

use ahash::AHashMap;
use bevy::{prelude::*, tasks::AsyncComputeTaskPool};
use crossbeam_channel::{Receiver, Sender};
use crate::{assets::{StitchedPart, StitchedParts, build_final_mesh_mhclo, build_final_meshes_mhclo}, basemesh::BaseMesh, loaders::{CharacterShapeAsset, MhcloAsset, ObjVertsAsset, TargetAsset}, morphs::MakeHumanMorphs, template::CharacterTemplate, rigs::{RigData, RigSpec}};

/// Component that references a [`CharacterShapeAsset`].  Place it on the root entity of a character.
#[derive(Component, Clone, Debug)]
#[require(Visibility)]
pub struct CharacterShape(pub Handle<CharacterShapeAsset>);

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
pub struct CachedMhcloMeshHandles(AHashMap<(Handle<MhcloAsset>, Handle<CharacterTemplate>), Handle<Mesh>>);

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
    Single{ part: Handle<MhcloAsset>, template_handle: Handle<CharacterTemplate> },
    Stitched{ parts: StitchedParts, template_handle: Handle<CharacterTemplate> },
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
}

/// This system runs in phases, so it gets triggered multiple times to load a mesh
/// State is tracked by [`AssetLoadState`]
pub(crate) fn mesh_build(
    templates: Res<Assets<CharacterTemplate>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mesh_verts: Res<Assets<ObjVertsAsset>>,
    mhclo_assets: Res<Assets<MhcloAsset>>,
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
            LoadAssetMeshJob::Single { part, template_handle } => {
                let Some(template) = templates.get(template_handle) else {
                    continue;
                };

                build_single_mesh_process(
                    mediator, load_state, part, template_handle, template, &mut meshes, &mesh_verts, &mhclo_assets,
                    &mut morphs, &basemesh.vertices, &rig_data, &asset_server, &mut cached_raw_meshes,
                );

                for msg in mediator.mesh_building_msg_receiver.try_iter() {
                    let mesh = handle_single_mesh_complete(msg, template);
                    let mesh_handle = meshes.add(mesh);
                    cached_meshes.insert((part.clone(), template_handle.clone()), mesh_handle.clone());
                    *load_state = AssetLoadState::Finished;
                }
            }
            LoadAssetMeshJob::Stitched{ parts, template_handle } => {
                let Some(template) = templates.get(template_handle) else {
                    continue;
                };

                build_stitched_meshes_process(
                    mediator, load_state, parts, template_handle, template, &templates, &mut meshes, &mesh_verts,
                    &mhclo_assets, &mut morphs, &basemesh.vertices, &rig_data, &asset_server,
                    &mut cached_raw_meshes,
                );

                for msg in mediator.mesh_building_msg_receiver.try_iter() {
                    let resolved_templates: Vec<&CharacterTemplate> = parts
                        .iter()
                        .map(|p| {
                            let h = p.template_override.as_ref().map_or(template_handle, |ov| &ov.0);
                            templates.get(h).unwrap_or(template)
                        })
                        .collect();

                    let new_meshes = handle_stitched_mesh_complete(msg, &resolved_templates);

                    for (i_mesh, mesh) in new_meshes.into_iter().enumerate() {
                        let handle = parts[i_mesh].part.clone();
                        let mesh_handle = meshes.add(mesh);
                        cached_meshes.insert((handle, template_handle.clone()), mesh_handle.clone());
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
) -> Mesh {
    let MeshConstructedMsg { final_meshes, morph_names } = msg;
    let mut mesh = final_meshes.into_iter().next().unwrap();

    if !template.shapes.is_empty() {
        let morph_names = morph_names.into_iter().next().unwrap();
        mesh = mesh.with_morph_target_names(morph_names);
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
) -> Mesh {
    let (mesh, morph_names) = build_final_mesh_mhclo(
        mhclo, input_mesh, mesh_verts, template, mh_morphs, basemesh, rig_spec,
    );
    let msg = MeshConstructedMsg {
        final_meshes: vec![mesh],
        morph_names: vec![morph_names],
    };
    handle_single_mesh_complete(msg, template)
}

/// Handles the message for a completed stitched mesh and builds the final meshes
pub(crate) fn handle_stitched_mesh_complete(
    msg: MeshConstructedMsg,
    templates: &[&CharacterTemplate],
) -> Vec<Mesh> {
    let MeshConstructedMsg { final_meshes, morph_names } = msg;
    let mut meshes = vec![];

    for (i_mesh, (mesh, names)) in final_meshes
            .into_iter()
            .zip(morph_names)
            .enumerate()
    {
        let template = templates[i_mesh];
        let mesh = if !template.shapes.is_empty() {
            mesh.with_morph_target_names(names)
        } else {
            mesh
        };
        meshes.push(mesh);
    }
    meshes
}

/// Handles the actual steps involved in constructing a single mesh
pub(crate) fn build_single_mesh_process(
    mediator: &LoadingMediator,
    load_state: &mut AssetLoadState,
    part: &Handle<MhcloAsset>,
    _template_handle: &Handle<CharacterTemplate>,
    template: &CharacterTemplate,
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

    let Some(mhclo) = mhclo_assets.get(part) else {
        return;
    };

    // Start loading mhclo if not already
    if *load_state == AssetLoadState::None {
        *load_state = AssetLoadState::LoadedObj;
        if !cached_raw_meshes.contains_key(part) {
            let mesh = asset_server.load::<Mesh>(mhclo.obj_file.clone());
            let verts = asset_server.load::<ObjVertsAsset>(mhclo.obj_file.clone());
            cached_raw_meshes.insert(part.clone(), RawMeshCache { mesh, verts });
            return;
        }
    }

    if *load_state == AssetLoadState::LoadedObj
            && let Some(cache) = cached_raw_meshes.get(part)
            && let Some(input_mesh) = meshes.get(&cache.mesh)
            && let Some(mesh_verts) = mesh_verts.get(&cache.verts)
    {
        if !morphs.is_ready(asset_server) {
            return;
        }

        *load_state = AssetLoadState::BuildSubmitted;
        cached_raw_meshes.remove(part);

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
            let (mesh, morph_names) = build_final_mesh_mhclo(
                &mhclo, &input_mesh, &mesh_verts, &template, mh_morphs, basemesh, &rig_spec
            );
            sender.send(MeshConstructedMsg {
                final_meshes: vec![mesh],
                morph_names: vec![morph_names],
            })
        }).detach()
    }
}

/// Handles the actual steps involved in constructing a stitched mesh
fn build_stitched_meshes_process(
    mediator: &LoadingMediator,
    load_state: &mut AssetLoadState,
    parts: &StitchedParts,
    template_handle: &Handle<CharacterTemplate>,
    parent_template: &CharacterTemplate,
    templates: &Assets<CharacterTemplate>,
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
            if !cached_raw_meshes.contains_key(part) {
                let mhclo = mhclo_assets.get(part).expect("mhclo already checked");
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

        let loaded_mesh_count = loaded_meshes.len();

        if loaded_mesh_count != parts.len() {
            return;
        }

        if !morphs.is_ready(asset_server) {
            return;
        }

        *load_state = AssetLoadState::BuildSubmitted;

        let mut input_meshes: Vec<Mesh> = loaded_meshes.into_iter().cloned().collect();
        let mesh_verts: Vec<ObjVertsAsset> = raw_handles
            .iter()
            .filter_map(|h| h.and_then(|cache| mesh_verts.get(&cache.verts).cloned()))
            .collect();
        let mhclos: Vec<_> = parts
            .iter()
            .map(|p| mhclo_assets.get(&p.part).unwrap().clone())
            .collect();

        let resolved_templates: Vec<&CharacterTemplate> = parts
            .iter()
            .map(|p| {
                let h = p.template_override.as_ref().map_or(template_handle, |ov| &ov.0);
                templates.get(h).unwrap_or(parent_template)
            })
            .collect();

        let rig = resolved_templates[0].rig;
        let templates_for_build: Vec<_> = resolved_templates
            .into_iter()
            .cloned()
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
            let (final_meshes, morph_names) = build_final_meshes_mhclo(
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
