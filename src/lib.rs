mod animation;
mod assets;
mod basemesh;
mod loaders;
mod mesh_ops;
mod morphs;
mod template;
mod rigs;
mod spawn_skeleton;
mod spawn_mesh;
mod physics;

use bevy::asset::AssetPath;
use bevy::ecs::intern::Interner;
use bevy::prelude::*;
use bevy_obj::ObjPlugin;
use prelude::*;

pub static NAME_INTERNER: Interner<str> = Interner::new();

pub mod prelude {
    pub use crate::{
        loaders::{
            ObjVertsAsset,
            ObjVertsAssetLoader,
            ObjVertsSettings,
            VertexGroupsAsset,
            VertexGroupsAssetLoader,
            CategoryMorphsAsset,
            CompositeTarget,
            CompositeTargetsAsset,
            MacroBoundString,
            MacroBounds,
            MacroDataAsset,
            MacroDataAssetLoader,
            MhcloAsset,
            MhcloAssetLoader,
            OppositesAsset,
            RetargetedAnimationAsset,
            RetargetedAnimationAssetLoader,
            RetargetedAnimationSettings,
            BoneJsonConfig,
            BoneTransformSpec,
            RigConfigAsset,
            RigConfigAssetLoader,
            RigWeightsAsset,
            RigWeightsAssetLoader,
            ReferenceRigAsset,
            ReferenceRigAssetLoader,
            CharacterTemplateAssetLoader,
            CharacterShapeAsset,
            CharacterShapeConfigLoader,
            TargetAsset,
            TargetAssetLoader,
            TargetDelta,
            TargetManifestAssetLoader,
        },
        basemesh::{BaseMesh, VertexGroups},
        animation::TranslationTracks,
        assets::{
            StitchedParts, StitchedPart,
            shape_mesh_from_helpers_mhclo,
        },
        morphs::{MakeHumanMorphs, MorphTargets, MorphError, MorphsReady, adjust_helpers_to_morphs},
        template::{
            CharacterTemplate, CharacterMorphShape, TemplateOverride,
        },
        mesh_ops::{
            get_vertex_positions,
            generate_vertex_map,
            generate_mhid_lookup,
        },
        rigs::{
            RigData,
            RigSpec,
            SkeletalBone,
            RigType,
            RootMotion,
        },
        spawn_mesh::{CharacterShape, MhcloMeshBuilder, LoadAssetMeshJob, CachedMhcloMeshHandles, build_single_mesh_direct},
        spawn_skeleton::{FitSkeleton, RelatedEntities},
        HumentityGlobalConfig,
        HumentityPlugin,
        BoneDebugPlugin,
        load_and_insert_humentity_assets,
        NAME_INTERNER,
    };
    pub use crate::physics::ColliderBone;
    #[cfg(feature = "avian")]
    pub use crate::physics::avian::{CharacterColliders, CharacterRagdoll};
    #[cfg(feature = "physx")]
    pub use crate::physics::physx::{PhysxCharacterColliders, ColliderType, HitboxCollider, HurtboxCollider, RagdollCollider, RagdollColliderFilter, ColliderForCharacter, ColliderList};
}

/// Loads and inserts all 4 core resources (BaseMesh, VertexGroups, MakeHumanMorphs, RigData)
/// as separate ECS resources. Each asset path must be explicitly specified — nothing is inferred.
pub fn load_and_insert_humentity_assets(
    commands: &mut Commands,
    asset_server: &AssetServer,
    base_mesh_path: impl Into<AssetPath<'static>>,
    vertex_groups_path: impl Into<AssetPath<'static>>,
    target_composites_path: impl Into<AssetPath<'static>>,
    target_macros_path: impl Into<AssetPath<'static>>,
    targets_folder_path: impl Into<AssetPath<'static>>,
    rig_config_path: impl Into<AssetPath<'static>>,
    rig_weights_path: impl Into<AssetPath<'static>>,
    ref_rig_path: impl Into<AssetPath<'static>>,
) {
    commands.insert_resource(basemesh::BaseMesh::new(asset_server, base_mesh_path));
    commands.insert_resource(basemesh::VertexGroups::new(asset_server, vertex_groups_path));
    commands.insert_resource(morphs::MakeHumanMorphs::new(
        asset_server,
        target_composites_path,
        target_macros_path,
        targets_folder_path,
    ));
    commands.insert_resource(rigs::RigData::new(
        asset_server,
        rig_config_path,
        rig_weights_path,
        ref_rig_path,
    ));
}

/// Model verts are facing Z instead of NEG_Z, so forward() faces the wrong direction.
pub(crate) const MODEL_ROTATION_FIX: Quat = Quat::from_xyzw(0., 1., 0., 0.);

#[derive(Resource, Default, Clone)]
pub struct HumentityGlobalConfig {
    /// Use animation postprocessing to rescale position tracks to mesh size
    /// This has some performance overhead. If disabled then translation tracks will
    /// be removed from all retargeted animations.
    pub translation_tracks: TranslationTracks,
}

/// The SystemSet for animation post-processing. If you need to add your own
/// animatin post-processing you can set it after this.
#[derive(SystemSet, Debug, Hash, Copy, Clone, Eq, PartialEq)]
pub struct HumentityAnimationSystems;

/// The plugin struct
pub struct HumentityPlugin;

impl Plugin for HumentityPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin);
        }

        app
            .insert_resource(spawn_mesh::MhcloMeshBuilder::default())
            .insert_resource(spawn_mesh::CachedMhcloMeshHandles::default())
            .insert_resource(spawn_mesh::CachedMhcloRawMeshHandles::default())

            .init_asset::<ObjVertsAsset>()
            .register_asset_loader(ObjVertsAssetLoader)
            .init_asset::<VertexGroupsAsset>()
            .register_asset_loader(VertexGroupsAssetLoader)
            .init_asset::<MhcloAsset>()
            .register_asset_loader(MhcloAssetLoader)
            .init_asset::<TargetAsset>()
            .register_asset_loader(TargetAssetLoader)
            .init_asset::<MacroDataAsset>()
            .register_asset_loader(MacroDataAssetLoader)
            .init_asset::<CompositeTargetsAsset>()
            .register_asset_loader(TargetManifestAssetLoader)
            .init_asset::<RetargetedAnimationAsset>()
            .register_asset_loader(RetargetedAnimationAssetLoader)
            .init_asset::<RigWeightsAsset>()
            .register_asset_loader(RigWeightsAssetLoader)
            .init_asset::<RigConfigAsset>()
            .register_asset_loader(RigConfigAssetLoader)
            .init_asset::<ReferenceRigAsset>()
            .register_asset_loader(ReferenceRigAssetLoader)
            .init_asset::<template::CharacterTemplate>()
            .register_asset_loader(loaders::CharacterTemplateAssetLoader)
            .init_asset::<loaders::CharacterShapeAsset>()
            .register_asset_loader(loaders::CharacterShapeConfigLoader)

            .add_systems(
                Update,
                (
                    basemesh::extract_basemesh_asset
                        .run_if(resource_exists::<basemesh::BaseMesh>),
                    basemesh::extract_vertex_groups_asset
                        .run_if(resource_exists::<basemesh::VertexGroups>),
                    morphs::populate_morph_resource
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    morphs::sync_loaded_morph_targets
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    morphs::check_morphs_ready
                        .after(morphs::populate_morph_resource)
                        .after(morphs::sync_loaded_morph_targets)
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    template::resolve_template_morphs
                        .after(morphs::check_morphs_ready)
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    rigs::sync_and_build_rig_data
                        .run_if(resource_exists::<rigs::RigData>),
                    (
                        (   
                            spawn_skeleton::spawn_rig_scene,
                            spawn_skeleton::fit_skeleton_to_shape,
                        ).chain(),
                        (
                            spawn_mesh::mesh_build,
                            spawn_mesh::mediators_clean_up,
                        ).chain(),
                    )
                        .chain()
                        .run_if(resource_exists::<basemesh::BaseMesh>)
                        .run_if(resource_exists::<basemesh::VertexGroups>)
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>)
                        .run_if(resource_exists::<rigs::RigData>)
                ),
            )
            .add_systems(Update, rigs::build_rig_scenes);

        #[cfg(feature = "avian")]
        {
            app.register_type::<RagdollDensity>()
                .register_type::<RagdollDamping>()
                .add_systems(
                Update,
                (
                    physics::avian::mark_needs_colliders,
                    physics::avian::spawn_colliders.after(physics::avian::mark_needs_colliders).after(spawn_skeleton::fit_skeleton_to_shape),
                    physics::avian::set_ragdoll_state,
                ),
            )
            .add_systems(
                FixedUpdate,
                (
                    physics::avian::sync_colliders,
                    physics::avian::sync_bones_to_ragdoll,
                ),
            );
        }

        #[cfg(feature = "physx")]
        {
            use bevy::app::AnimationSystems;
            use bevy_mod_physx::prelude::Physics;

            app.add_systems(
                    Startup,
                    physics::physx::create_collider_physics_material
                            .run_if(resource_exists::<Physics>),
                )
                .add_systems(
                    Update,
                    (
                        physics::physx::auto_add_ragdoll_colliders
                            .after(spawn_skeleton::fit_skeleton_to_shape),
                        physics::physx::spawn_kinematic_colliders::<HitboxCollider>
                            .run_if(resource_exists::<physics::physx::ColliderMaterial>)
                            .run_if(resource_exists::<Physics>)
                            .run_if(resource_exists::<RigData>),
                        physics::physx::spawn_kinematic_colliders::<HurtboxCollider>
                            .run_if(resource_exists::<physics::physx::ColliderMaterial>)
                            .run_if(resource_exists::<Physics>)
                            .run_if(resource_exists::<RigData>),
                        physics::physx::spawn_ragdoll_colliders
                            .run_if(resource_exists::<physics::physx::ColliderMaterial>)
                            .run_if(resource_exists::<Physics>)
                            .run_if(resource_exists::<RigData>),
                        physics::physx::sync_colliders::<HitboxCollider>,
                        physics::physx::sync_colliders::<HurtboxCollider>,
                        physics::physx::on_colliders_changed::<HitboxCollider>,
                        physics::physx::on_colliders_changed::<HurtboxCollider>,
                        physics::physx::on_colliders_changed::<RagdollCollider>,
                    )
                )
                .add_systems(
                    PostUpdate,
                    physics::physx::sync_skeleton_to_ragdoll
                        .after(AnimationSystems)
                )
                .add_observer(physics::physx::mark_entity_needs_colliders::<HitboxCollider>)
                .add_observer(physics::physx::mark_entity_needs_colliders::<HurtboxCollider>)
                .add_observer(physics::physx::mark_entity_needs_colliders::<RagdollCollider>);
        }

        /*
        // Root Motion
        if matches!(self.config.translation_tracks, TranslationTracks::Root) {
            app.add_systems(
                PostUpdate,
                animation::rescale_root_bone_translation
                    .after(AnimationSystems)
                    .in_set(HumentityAnimationSystems),
            );
        } else if matches!(self.config.translation_tracks, TranslationTracks::Full) {
            app.add_systems(
                PostUpdate,
                animation::rescale_bone_translations
                    .after(AnimationSystems)
                    .in_set(HumentityAnimationSystems),
            );
        }

        if !matches!(self.config.translation_tracks, TranslationTracks::None) {
            app.add_systems(
                PostUpdate,
                animation::root_motion
                    .after(HumentityAnimationSystems)
            );
        }
         */
    }

}

pub struct BoneDebugPlugin;

impl Plugin for BoneDebugPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, rigs::bone_debug_draw);
    }
}   