mod animation;
mod assets;
mod basemesh;
mod bone_debug;
mod loaders;
mod mesh_ops;
mod morphs;
mod physics;
mod rigs;
mod skeleton_lod;
mod spawn_mesh;
mod spawn_skeleton;
mod template;

use bevy::asset::AssetPath;
use bevy::ecs::intern::Interner;
use bevy::prelude::*;
use bevy_obj::ObjPlugin;
use prelude::*;

use crate::rigs::BuiltRigs;

pub static NAME_INTERNER: Interner<str> = Interner::new();

pub mod prelude {
    #[cfg(feature = "avian")]
    pub use crate::physics::avian::RagdollCollisionLayers;
    #[cfg(feature = "avian")]
    pub use crate::physics::avian::{
        CharacterColliders, CharacterRagdoll, ColliderForCharacter, ColliderOffset,
        KinematicCollider,
    };
    #[cfg(feature = "physx")]
    pub use crate::physics::physx::{
        ColliderForCharacter, ColliderList, ColliderType, HitboxCollider, HurtboxCollider,
        PhysxCharacterColliders, RagdollCollider, RagdollColliderFilter,
    };
    pub use crate::physics::{
        COLLIDERS, ColliderBone, RagdollDamping, RagdollDensity, RagdollMobility,
    };
    pub use crate::{
        bone_debug::BoneDebugPlugin,
        HumentityPlugin,
        NAME_INTERNER,
        animation::{HumentityAnimationPostProcess, TranslationTracks},
        assets::{StitchedPart, StitchedParts, shape_mesh_from_helpers_mhclo},
        basemesh::{BaseMesh, VertexGroups},
        load_and_insert_humentity_assets,
        loaders::{
            BoneJsonConfig, BoneTransformSpec, CategoryMorphsAsset, CharacterShapeAsset,
            CharacterShapeConfigLoader, CharacterTemplateAssetLoader, CompositeTarget,
            CompositeTargetsAsset, MacroBoundString, MacroBounds, MacroDataAsset,
            MacroDataAssetLoader, MhcloAsset, MhcloAssetLoader, ObjVertsAsset, ObjVertsAssetLoader,
            ObjVertsSettings, OppositesAsset, ReferenceRigAsset, ReferenceRigAssetLoader,
            RetargetedAnimationAsset, RetargetedAnimationAssetLoader, RetargetedAnimationSettings,
            RigConfigAsset, RigConfigAssetLoader, RigWeightsAsset, RigWeightsAssetLoader,
            TargetAsset, TargetAssetLoader, TargetDelta, TargetManifestAssetLoader,
            VertexGroupsAsset, VertexGroupsAssetLoader,
        },
        mesh_ops::{generate_mhid_lookup, generate_vertex_map, get_vertex_positions},
        morphs::{
            MakeHumanMorphs, MorphError, MorphTargets, MorphsReady, adjust_helpers_to_morphs,
        },
        rigs::{RigData, RigSpec, SkeletonRootBone, SkeletalBone},
        skeleton_lod::{
            BoneMergeConfig, RigBundle, SkeletonLodConfig,
            SkeletonLodVariant,
        },
        spawn_mesh::{
            CachedMhcloMeshHandles, CharacterPart, CharacterShape, LoadAssetMeshJob,
            MhcloMeshBuilder, build_single_mesh_direct,
        },
        spawn_skeleton::{
            CharacterSkeleton, DisableSkeletonLod, EnableSkeletonLod, ResetSkeletonToBindPose,
            SkeletonLodDisabled, SkeletonLodFilter, SkeletonLodMap, SkeletonLocalBindPose,
            SkeletonsReady,
        },
        template::{CharacterMorphShape, CharacterTemplate, TemplateOverride},
    };
}

/// Holds strong handles to rig assets to prevent eviction.
#[derive(Resource)]
pub(crate) struct RigAssetHandles {
    #[allow(dead_code)]
    pub config: Handle<loaders::RigConfigAsset>,
    #[allow(dead_code)]
    pub weights: Handle<loaders::RigWeightsAsset>,
    #[allow(dead_code)]
    pub reference_rig: Handle<loaders::ReferenceRigAsset>,
}

/// Loads and inserts all core resources (BaseMesh, VertexGroups, MakeHumanMorphs, RigData)
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
    commands.insert_resource(basemesh::VertexGroups::new(
        asset_server,
        vertex_groups_path,
    ));
    commands.insert_resource(morphs::MakeHumanMorphs::new(
        asset_server,
        target_composites_path,
        target_macros_path,
        targets_folder_path,
    ));

    // Rig assets are loaded by path and detected by the loaders.
    // The sync_and_build_rig_data system will detect them and store in RigData.
    // Store handles in a resource to prevent eviction.
    let config_handle: Handle<loaders::RigConfigAsset> = asset_server.load(rig_config_path);
    let weights_handle: Handle<loaders::RigWeightsAsset> = asset_server.load(rig_weights_path);
    let ref_rig_handle: Handle<loaders::ReferenceRigAsset> = asset_server.load(ref_rig_path);

    commands.insert_resource(RigAssetHandles {
        config: config_handle,
        weights: weights_handle,
        reference_rig: ref_rig_handle,
    });
}

/// Model verts are facing Z instead of NEG_Z, so forward() faces the wrong direction.
pub(crate) const MODEL_ROTATION_FIX: Quat = Quat::from_xyzw(0., 1., 0., 0.);

/// The plugin struct
pub struct HumentityPlugin;

impl Plugin for HumentityPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ObjPlugin>() {
            app.add_plugins(ObjPlugin);
        }

        app.insert_resource(spawn_mesh::MhcloMeshBuilder::default())
            .insert_resource(spawn_mesh::CachedMhcloMeshHandles::default())
            .insert_resource(spawn_mesh::CachedMhcloRawMeshHandles::default())
            .insert_resource(rigs::RigData::new())
            .insert_resource(rigs::RigBundleRes::default());

        app.world_mut()
            .register_disabling_component::<spawn_skeleton::SkeletonLodDisabled>();

        app.register_type::<rigs::SkeletalBone>()
            .register_type::<rigs::RootBone>()
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
                    basemesh::extract_basemesh_asset.run_if(resource_exists::<basemesh::BaseMesh>),
                    basemesh::extract_vertex_groups_asset
                        .run_if(resource_exists::<basemesh::VertexGroups>),
                    morphs::sync_macro_data
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    morphs::sync_composite_data
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    morphs::sync_loaded_morph_targets
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    morphs::check_morphs_ready
                        .after(morphs::sync_macro_data)
                        .after(morphs::sync_composite_data)
                        .after(morphs::sync_loaded_morph_targets)
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    rigs::sync_and_build_rig_data
                        .run_if(resource_exists::<rigs::RigData>)
                        .run_if(not(resource_exists::<BuiltRigs>)),
                    rigs::build_rig_scenes
                        .after(rigs::sync_and_build_rig_data)
                        .run_if(not(resource_exists::<BuiltRigs>)),
                    (
                        (
                            spawn_skeleton::spawn_rig_skeletons,
                            spawn_skeleton::fit_skeleton_to_shape,
                            spawn_skeleton::check_skeletons_ready,
                            spawn_skeleton::setup_part_skinning,
                        )
                            .chain(),
                        (spawn_mesh::mesh_build, spawn_mesh::mediators_clean_up).chain(),
                    )
                        .chain()
                        .after(rigs::build_rig_scenes)
                        .run_if(resource_exists::<basemesh::BaseMesh>)
                        .run_if(resource_exists::<basemesh::VertexGroups>)
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>)
                        .run_if(resource_exists::<rigs::RigData>),
                ),
            )
            .add_observer(spawn_skeleton::on_enable_skeleton_lod)
            .add_observer(spawn_skeleton::on_disable_skeleton_lod)
            .add_observer(spawn_skeleton::on_reset_skeleton_to_bind_pose)
            .add_systems(
                Update,
                template::resolve_template_morphs
                    .before(spawn_mesh::mesh_build)
                    .run_if(resource_exists::<morphs::MakeHumanMorphs>),
            )
            .add_systems(
                PostUpdate,
                animation::rescale_root_bone_translation
                    .in_set(animation::HumentityAnimationPostProcess)
                    .after(bevy::app::AnimationSystems)
                    .before(TransformSystems::Propagate),
            );

        #[cfg(feature = "avian")]
        {
            use bevy::app::AnimationSystems;

            app.register_type::<RagdollDensity>()
                .register_type::<RagdollDamping>()
                .add_systems(
                    Update,
                    (
                        physics::avian::mark_needs_colliders
                            .after(spawn_skeleton::check_skeletons_ready),
                        physics::avian::spawn_colliders
                            .after(physics::avian::mark_needs_colliders)
                            .after(spawn_skeleton::check_skeletons_ready),
                        physics::avian::set_ragdoll_state,
                        physics::avian::update_collision_layers,
                    ),
                )
                .add_systems(FixedUpdate, physics::avian::sync_colliders)
                .add_systems(
                    PostUpdate,
                    physics::avian::sync_bones_to_ragdoll
                        .after(AnimationSystems)
                        .before(TransformSystems::Propagate),
                );
        }

        #[cfg(feature = "physx")]
        {
            use bevy::app::AnimationSystems;
            use bevy_mod_physx::prelude::Physics;

            app.add_systems(
                Startup,
                physics::physx::create_collider_physics_material.run_if(resource_exists::<Physics>),
            )
            .add_systems(
                Update,
                (
                    physics::physx::auto_add_ragdoll_colliders
                        .after(spawn_skeleton::check_skeletons_ready),
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
                ),
            )
            .add_systems(
                PostUpdate,
                physics::physx::sync_skeleton_to_ragdoll.after(AnimationSystems),
            )
            .add_observer(physics::physx::mark_entity_needs_colliders::<HitboxCollider>)
            .add_observer(physics::physx::mark_entity_needs_colliders::<HurtboxCollider>)
            .add_observer(physics::physx::mark_entity_needs_colliders::<RagdollCollider>);
        }
    }
}
