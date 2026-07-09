use std::ops::Deref;
use std::sync::Arc;

use ahash::AHashMap;
use bevy::asset::AssetPath;
use bevy::prelude::*;

use crate::loaders::{ObjVertsAsset, VertexGroupsAsset};
use crate::loaders::ObjVertsSettings;

#[derive(Resource)]
pub struct BaseMesh {
    pub vertices: Arc<Vec<Vec3>>,
    #[allow(dead_code)]
    handle: Handle<ObjVertsAsset>,
}

impl BaseMesh {
    pub fn new(asset_server: &AssetServer, path: impl Into<AssetPath<'static>>) -> Self {
        let handle: Handle<ObjVertsAsset> = asset_server.load_builder()
            .with_settings(|settings: &mut ObjVertsSettings| {
                settings.is_basemesh_helpers = true;
            })
            .load(path);
        Self {
            vertices: default(),
            handle,
        }
    }
}

impl Deref for BaseMesh {
    type Target = Arc<Vec<Vec3>>;
    fn deref(&self) -> &Self::Target {
        &self.vertices
    }
}

#[derive(Resource)]
pub struct VertexGroups {
    data: Arc<AHashMap<String, Vec<[usize; 2]>>>,
    #[allow(dead_code)]
    handle: Handle<VertexGroupsAsset>,
}

impl VertexGroups {
    pub fn new(asset_server: &AssetServer, path: impl Into<AssetPath<'static>>) -> Self {
        let handle = asset_server.load::<VertexGroupsAsset>(path);
        Self {
            data: default(),
            handle,
        }
    }
}

impl Deref for VertexGroups {
    type Target = AHashMap<String, Vec<[usize; 2]>>;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

pub(crate) fn extract_basemesh_asset(
    basemesh_assets: Res<Assets<ObjVertsAsset>>,
    mut basemesh_events: MessageReader<AssetEvent<ObjVertsAsset>>,
    mut meshes: ResMut<BaseMesh>,
) {
    for ev in basemesh_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev
            && let Some(asset) = basemesh_assets.get(*id)
                && asset.is_basemesh_helpers {
                    meshes.vertices = Arc::new(asset.vertices.clone());
                    return;
                }
    }
}

pub(crate) fn extract_vertex_groups_asset(
    vg_assets: Res<Assets<VertexGroupsAsset>>,
    mut vg_events: MessageReader<AssetEvent<VertexGroupsAsset>>,
    mut vg: ResMut<VertexGroups>,
) {
    for ev in vg_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev
            && let Some(asset) = vg_assets.get(*id) {
                vg.data = Arc::new(asset.0.clone());
            }
    }
}
