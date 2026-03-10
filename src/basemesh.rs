use std::sync::Arc;

use ahash::AHashMap;
use bevy::prelude::*;

use crate::{
    loaders::{BaseMeshAsset, VertexGroupsAsset},
};

pub(crate) const _BODY_VERTICES: u16 = 13380u16;
pub(crate) const _BODY_SCALE: f32 = 0.1;

#[derive(Resource)]
pub struct BaseMesh(pub Arc<Vec<Vec3>>);

#[derive(Resource)]
pub struct VertexGroups(pub Arc<AHashMap<String, Vec<[usize; 2]>>>);

pub(crate) fn extract_basemesh_asset(
    basemesh_assets: Res<Assets<BaseMeshAsset>>,
    mut basemesh_events: MessageReader<AssetEvent<BaseMeshAsset>>,
    mut commands: Commands,
) {
    for ev in basemesh_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev {
            if let Some(asset) = basemesh_assets.get(*id) {
                commands.insert_resource(BaseMesh(Arc::new(asset.0.clone())));
            }
        }
    }
}

pub(crate) fn extract_vertex_groups_asset(
    vg_assets: Res<Assets<VertexGroupsAsset>>,
    mut vg_events: MessageReader<AssetEvent<VertexGroupsAsset>>,
    mut commands: Commands,
) {
    for ev in vg_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = ev {
            if let Some(asset) = vg_assets.get(*id) {
                commands.insert_resource(VertexGroups(Arc::new(asset.0.clone())));
            }
        }
    }
}