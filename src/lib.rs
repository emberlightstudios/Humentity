mod paths_config;
mod basemesh;
mod morphs;
mod rigs;
mod assets;
mod spawn;
mod prefab;
mod animation;
mod mesh_ops;
mod material;
mod physics;

use bevy::asset::io::AssetSourceBuilder;
use bevy::prelude::*;
use bevy_obj::ObjPlugin;
use bevy::ecs::intern::Interner;
use prelude::*;

use crate::prefab::ArchetypeShapeUpdate;

pub static NAME_INTERNER: Interner<str> = Interner::new();

pub mod prelude {
    pub use crate::{
        NAME_INTERNER,
        Humentity, HumentityLoadState,
        rigs::RigType,
        morphs::{MakeHumanMorphs, MorphTargets},
        basemesh::BaseMesh,
        paths_config::HumentityPathsConfig,
        prefab::{
            CharacterArchetypePrefab, CharacterArchetypePrefabs, CharacterShapeArchetype,
            CharacterAnimationArchetype, ModifyPrefabShape
        },
        assets::{CharacterAsset, CharacterAssetRegistry, CharacterPart, CharacterBodyTextures},
        animation::CharacterAnimationClips as CharacterAnimationClips,
        spawn::CharacterShapeConfig,
        material::{CharacterMaterialExtension, CharacterMaterialExtensionData},
        physics::CharacterRagdoll,
        mesh_ops::{CharacterAssetMeshReady, MeshProcessingState},
    };
        
}

#[derive(States, Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub enum HumentityLoadState {
    LoadingCoreAssets,
    BuildingPrefabs,
    AnimationProcessing,
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
        Humentity { config, debug: false }
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
            .add_message::<CharacterAssetMeshReady>()
            .add_observer(prefab::on_prefab_shape_modified)
            .add_systems(Update, (
                // PHASE 1 : LOADING CORE ASSETS
                (
                    basemesh::create_body_mesh
                        .run_if(resource_exists::<basemesh::HelperMeshHandle>),
                ).run_if(in_state(HumentityLoadState::LoadingCoreAssets)),

                // PHASE 2 : BUILDING ARCHETYPE PREFABS
                (                    
                    (
                        prefab::create_human_prefab_rig_scenes,
                    ).chain().run_if(
                        resource_exists::<CharacterArchetypePrefabs>
                        .and(in_state(HumentityLoadState::BuildingPrefabs))
                    ),
                ),

                // PHASE 3 : REBUILDING ANIMATION CLIPS
                animation::rebuild_animations
                    .run_if(in_state(HumentityLoadState::AnimationProcessing)),

                // PHASE 4 : READY TO BUILD HUMANS
                (
                    spawn::spawn_rig_scene,
                    spawn::fit_skeleton_to_shape,
                    spawn::setup_human_parts,
                    physics::control_ragdoll,
                    prefab::update_asset_shapes.run_if(resource_exists::<ArchetypeShapeUpdate>)
                ).chain().run_if(
                    in_state(HumentityLoadState::Ready)
                    .and(resource_exists::<CharacterAssetRegistry>)
                )
            ));

        if self.debug {
            app.add_systems(Update, rigs::bone_debug_draw);
        }
    }

    fn finish(&self, app: &mut App) {
        app.insert_state(HumentityLoadState::LoadingCoreAssets);
        // We do this becuase the sequence of plugin load order must be 
        // Humentity -> AssetServer -> ObjPlugin
        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin);
        }
        // Most of the core assets are loaded in the FromWorld impl for these resources
        app
            .init_resource::<basemesh::BaseMesh>()
            .init_resource::<assets::CharacterAssetRegistry>()
            .init_resource::<morphs::MakeHumanMorphs>()
            .init_resource::<rigs::RigData>();

    }
}
