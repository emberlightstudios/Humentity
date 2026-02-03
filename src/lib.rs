mod animation;
mod assets;
mod basemesh;
mod mesh_ops;
mod morphs;
mod paths_config;
mod physics;
mod prefab;
mod rigs;
mod spawn_skeleton;
mod spawn_mesh;

use bevy::app::AnimationSystems;
use bevy::asset::io::AssetSourceBuilder;
use bevy::ecs::intern::Interner;
use bevy::prelude::*;
use bevy_obj::ObjPlugin;
use prelude::*;

pub static NAME_INTERNER: Interner<str> = Interner::new();

pub mod prelude {
    pub use crate::{
        animation::CharacterAnimationClips,
        assets::{
            CharacterAsset, CharacterAssetRegistry, CharacterAssetTextureType, CharacterPart,
        },
        basemesh::BaseMesh,
        mesh_ops::CharacterAssetMeshReady,
        morphs::{MakeHumanMorphs, MorphTargets},
        paths_config::{HumentityAssetPath, HumentityAssetSourceId, HumentityPathsConfig},
        physics::CharacterRagdoll,
        prefab::{
            CharacterAnimationArchetype, CharacterArchetypePrefab, CharacterArchetypePrefabs,
            CharacterShapeArchetype, ModifyPrefabShape,
        },
        rigs::{ParentBone, RigType, RootMotion},
        spawn_mesh::{CharacterPartMeshSpawned, CharacterShapeConfig, BuildMesh},
        spawn_skeleton::{FitSkeleton, RelatedEntities},
        HumentityGlobalConfig,
        HumentityLoadState,
        HumentityPlugin,
        TranslationTracks,
        NAME_INTERNER,
    };
}

/// Which translation tracks should be kept on animation clips
#[derive(Copy, Clone, Default, Debug)]
pub enum TranslationTracks {
    #[default]
    Root,
    Full,
    None,
}

#[derive(Resource, Default, Clone)]
pub struct HumentityGlobalConfig {
    /// Draw red lines showing the skeleton
    pub debug_draw_bones: bool,
    /// Use animation postprocessing to rescale position tracks to mesh size
    /// This has some performance overhead. If disabled then translation tracks will
    /// be removed from all retargeted animations.
    pub translation_tracks: TranslationTracks,
}

#[derive(States, Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub enum HumentityLoadState {
    BuildingPrefabs,
    AnimationProcessing,
    Ready,
}

/// The SystemSet for animation post-processing. If you need to add your own
/// animatin post-processing you can set it after this.
#[derive(SystemSet, Debug, Hash, Copy, Clone, Eq, PartialEq)]
pub struct HumentityAnimationSystems;

/// The plugin struct
pub struct HumentityPlugin {
    /// The paths used by the plugin
    pub paths: paths_config::HumentityPathsConfig,
    pub config: HumentityGlobalConfig,
}

impl HumentityPlugin {
    pub fn new(paths: paths_config::HumentityPathsConfig) -> Self {
        HumentityPlugin {
            paths,
            config: HumentityGlobalConfig::default(),
        }
    }
}

impl Plugin for HumentityPlugin {
    fn build(&self, app: &mut App) {
        if app.world().is_resource_added::<AssetServer>() {
            panic!("Humentity plugin must be added before AssetServer/DefaultPlugins.")
        }

        app.insert_resource(self.paths.clone())
            .insert_resource(self.config.clone())
            .add_message::<CharacterPartMeshSpawned>()
            .register_asset_source(
                "humentity",
                AssetSourceBuilder::platform_default(
                    self.paths
                        .core_assets_path
                        .to_str()
                        .expect("Failed to get path str"),
                    None,
                ),
            )
            .add_message::<CharacterAssetMeshReady>()
            .add_observer(spawn_mesh::trigger_mesh_build)
            .add_systems(
                Update,
                (
                    // PHASE 1 : BUILDING ARCHETYPE PREFABS
                    prefab::create_character_prefab_rig_scenes
                        .run_if(resource_exists::<CharacterArchetypePrefabs>)
                        .run_if(in_state(HumentityLoadState::BuildingPrefabs)),
                    // PHASE 2 : REBUILDING ANIMATION CLIPS
                    animation::rebuild_animations
                        .run_if(in_state(HumentityLoadState::AnimationProcessing)),
                    // PHASE 3 : READY TO BUILD HUMANS
                    (
                        spawn_skeleton::spawn_rig_scene,
                        spawn_skeleton::fit_skeleton_to_shape,
                        spawn_mesh::handle_mesh_load_tasks,
                        physics::control_ragdoll,
                        //prefab::update_asset_shapes.run_if(resource_exists::<ArchetypeShapeUpdate>),
                    )
                        .chain()
                        .run_if(in_state(HumentityLoadState::Ready))
                        .run_if(resource_exists::<CharacterAssetRegistry>),
                ),
            );

        if self.config.debug_draw_bones {
            app.add_systems(Update, rigs::bone_debug_draw);
        }

        if matches!(self.config.translation_tracks, TranslationTracks::Root) {
            app.add_systems(
                PostUpdate,
                animation::rescale_root_bone_translation
                    .after(AnimationSystems)
                    .run_if(in_state(HumentityLoadState::Ready))
                    .in_set(HumentityAnimationSystems),
            );
        } else if matches!(self.config.translation_tracks, TranslationTracks::Full) {
            app.add_systems(
                PostUpdate,
                animation::rescale_bone_translations
                    .after(AnimationSystems)
                    .run_if(in_state(HumentityLoadState::Ready))
                    .in_set(HumentityAnimationSystems),
            );
        }

        if !matches!(self.config.translation_tracks, TranslationTracks::None) {
            app.add_systems(
                PostUpdate,
                animation::root_motion
                    .after(HumentityAnimationSystems)
                    .run_if(in_state(HumentityLoadState::Ready)),
            );
        }
    }

    fn finish(&self, app: &mut App) {
        app.insert_state(HumentityLoadState::BuildingPrefabs);
        // We do this becuase the sequence of plugin load order must be
        // Humentity -> AssetServer -> ObjPlugin
        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin);
        }
        // Most of the core assets are loaded in the FromWorld impl for these resources
        app.init_resource::<basemesh::BaseMesh>()
            .init_resource::<rigs::SkeletonCaches>()
            .init_resource::<assets::CharacterAssetRegistry>()
            .init_resource::<morphs::MakeHumanMorphs>()
            .init_resource::<rigs::RigData>();
    }
}
