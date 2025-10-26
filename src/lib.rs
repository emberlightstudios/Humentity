mod basemesh;
mod spawning;
mod morphs;
mod rigs;
mod paths_config;
mod assets;
mod mesh_ops;
mod animation;
mod prefab;

use bevy::asset::io::AssetSourceBuilder;
use bevy::prelude::*;
use bevy_obj::ObjPlugin;
use bevy::ecs::intern::Interner;
use prelude::*;

pub static NAME_INTERNER: Interner<str> = Interner::new();

pub mod prelude {
    pub use crate::{
        NAME_INTERNER,
        Humentity, HumentityLoadState,
        rigs::RigType,
        morphs::{HumanMorphs, MorphTargets},
        basemesh::BaseMesh,
        paths_config::HumentityPathsConfig,
        prefab::{HumanArchetypePrefab, HumanArchetypePrefabs, HumanShapeArchetype, HumanAnimationArchetype},
        assets::{HumanAsset, HumanAssetRegistry, HumanPart, HumanBodyTextures},
        animation::HumanAnimationClips,
        spawning::HumanShapeConfig,
    };
}

#[derive(States, Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub enum HumentityLoadState {
    LoadingCoreAssets,
    BuildingPrefabs,
    RetargetingAnimations,
    Ready,
}

/*----------+
 |  Plugin  |
 +----------*/
 /// The plugin struct
pub struct Humentity{
    /// The paths used by the plugin
    pub config: paths_config::HumentityPathsConfig,
    /// Enable this to draw bone gizmos
    pub debug: bool,
}

impl Humentity {
    pub fn new(config: paths_config::HumentityPathsConfig) -> Self {
        Humentity { config, debug: true }
    }
}

impl Plugin for Humentity {
    fn build(&self, app: &mut App) {
        if app.world().is_resource_added::<AssetServer>() {
            panic!("Humentity plugin must be added before AssetServer/DefaultPlugins.")
        }

        app
            .insert_resource(self.config.clone())
            .register_asset_source("humentity", AssetSourceBuilder::platform_default(
                self.config.core_assets_path.to_str()
                    .expect("Failed to get path str"),
                None
            ))
            .add_systems(Update, (
                (
                    basemesh::create_body_mesh
                        .run_if(resource_exists::<basemesh::HelperMeshHandle>),
                ).run_if(in_state(HumentityLoadState::LoadingCoreAssets)),
                (                    
                    (
                        prefab::create_human_prefab_rig_scenes,
                        prefab::create_basemesh_prefab_shapes,
                        prefab::create_basemesh_prefab_morphable_mesh,
                        prefab::rig_prefab_meshes,
                    ).chain().run_if(
                        resource_exists::<HumanArchetypePrefabs>
                        .and(in_state(HumentityLoadState::BuildingPrefabs))
                    ),
                ),
                animation::retarget_animations
                    .run_if(in_state(HumentityLoadState::RetargetingAnimations)),
                (
                    spawning::spawn_rig_scene,
                    spawning::fit_skeleton_to_shape,
                    spawning::setup_human_parts,
                ).chain().run_if(
                    in_state(HumentityLoadState::Ready)
                    .and(resource_exists::<HumanAssetRegistry>)
                )
            ));

        if self.debug {
            app.add_systems(Update, rigs::bone_debug_draw);
        }
    }

    fn finish(&self, app: &mut App) {
        app.insert_state(HumentityLoadState::LoadingCoreAssets);
        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin);
        }
        app
            .init_resource::<basemesh::BaseMesh>()
            .init_resource::<assets::HumanAssetRegistry>()
            .init_resource::<morphs::HumanMorphs>()
            .init_resource::<rigs::RigData>();
    }
}

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Resource)]
pub struct HumentityLoading;
