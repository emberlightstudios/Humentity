mod basemesh;
mod morphs;
mod rigs;
mod global_config;
mod assets;
mod animation;
mod mesh_ops;
mod human_config;

use bevy::asset::io::AssetSourceBuilder;
use bevy::prelude::*;
use bevy::ecs::schedule::common_conditions::run_once;
use bevy_obj::ObjPlugin;

pub mod prelude {
    pub use crate::{
        Humentity,
        HumentityLoading,
        assets::HumanAssetRegistry,
        morphs::MorphTargets,
        global_config::HumentityGlobalConfig,
        human_config::HumanConfig,
        rigs::RigType,
        animation::AnimationLibrarySet,
        animation::AnimationLibrarySettings,
        basemesh::BaseMesh,
        rigs::RigData,
    };
}

/*----------+
 |  Plugin  |
 +----------*/
pub struct Humentity{
    pub config: global_config::HumentityGlobalConfig,
    pub debug: bool,
}

impl Humentity {
    pub fn new(config: global_config::HumentityGlobalConfig, debug: bool) -> Self {
        Humentity { config, debug }
    }
}

impl Plugin for Humentity {
    fn build(&self, app: &mut App) {
        if app.world().is_resource_added::<AssetServer>() {
            panic!("Humentity plugin must be added before AssetServer/DefaultPlugins.")
        }

        app.insert_resource(self.config.clone());
        app.insert_resource(HumentityLoading);
        app.register_asset_source("humentity", AssetSourceBuilder::platform_default(
            self.config.core_assets_path.to_str()
                .expect("Failed to get path str"),
            None
        ));

        app.add_systems(Update, (
            (
                basemesh::create_body_mesh
                    .run_if(resource_exists::<basemesh::HelperMeshHandle>),
                assets::generate_asset_vertex_maps
                    .run_if(run_once),
                animation::load_animations
                    .run_if(run_once),
            ).run_if(resource_exists::<HumentityLoading>),
            human_config::on_human_added,
        ));

        if self.debug {
            app.add_systems(Update, rigs::bone_debug_draw);
        }
    }

    fn finish(&self, app: &mut App) {
        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin);
        }
        app.init_resource::<basemesh::BaseMesh>();
        app.init_resource::<assets::HumanAssetRegistry>();
        app.init_resource::<morphs::MorphTargets>();
        app.init_resource::<rigs::RigData>();
        app.init_resource::<animation::AnimationLibrarySet>();
    }
}

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource)]
pub struct HumentityLoading;
