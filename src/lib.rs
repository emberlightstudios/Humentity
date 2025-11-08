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

use bevy::app::AnimationSystems;
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
        rigs::{RigType, ParentBone},
        morphs::{MakeHumanMorphs, MorphTargets},
        basemesh::BaseMesh,
        paths_config::HumentityPathsConfig,
        prefab::{
            CharacterArchetypePrefab, CharacterArchetypePrefabs, CharacterShapeArchetype,
            CharacterAnimationArchetype, ModifyPrefabShape
        },
        assets::{CharacterAsset, CharacterAssetRegistry, CharacterPart, CharacterBodyTextures},
        animation::CharacterAnimationClips as CharacterAnimationClips,
        spawn::{CharacterShapeConfig, FitSkeleton},
        material::{CharacterMaterialExtension, CharacterMaterialExtensionData},
        physics::CharacterRagdoll,
        mesh_ops::{CharacterAssetMeshReady, MeshProcessingState},
    };
        
}

#[derive(Resource, Default, Clone)]
pub struct HumentityGlobalConfig {
    /// Draw red lines showing the skeleton
    pub debug_draw_bones: bool,
    /// Use animation postprocessing to rescale position tracks to mesh size
    /// This has some performance overhead. If disabled then translation tracks will
    /// be removed from all retargeted animations.
    pub translation_animation_tracks: bool,
}

#[derive(States, Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub enum HumentityLoadState {
    LoadingCoreAssets,
    BuildingPrefabs,
    AnimationProcessing,
    Ready,
}

/// The SystemSet for animation post-processing. If you need to add your own
/// animatin post-processing you can set it after this.
#[derive(SystemSet, Debug, Hash, Copy, Clone, Eq, PartialEq)]
pub struct HumentityAnimationSystems;

/*----------+
 |  Plugin  |
 +----------*/
 /// The plugin struct
pub struct Humentity{
    /// The paths used by the plugin
    pub paths: paths_config::HumentityPathsConfig,
    pub config: HumentityGlobalConfig,
}

impl Humentity {
    pub fn new(paths: paths_config::HumentityPathsConfig) -> Self {
        Humentity { paths, config: HumentityGlobalConfig::default() }
    }
}

impl Plugin for Humentity {
    fn build(&self, app: &mut App) {
        if app.world().is_resource_added::<AssetServer>() {
            panic!("Humentity plugin must be added before AssetServer/DefaultPlugins.")
        }

        app
            .insert_resource(self.paths.clone())
            .insert_resource(self.config.clone())
            .register_asset_source("humentity", AssetSourceBuilder::platform_default(
                self.paths.core_assets_path.to_str()
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

        if self.config.debug_draw_bones {
            app.add_systems(Update, rigs::bone_debug_draw);
        }
        if self.config.translation_animation_tracks {
            app.add_systems(
                PostUpdate,
                animation::rescale_bone_translations
                    .after(AnimationSystems)
                    .run_if(in_state(HumentityLoadState::Ready))
                    .in_set(HumentityAnimationSystems)
            );
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
