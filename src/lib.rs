mod animation;
mod assets;
mod basemesh;
mod bone_debug;
pub mod gpu;
pub(crate) mod helpers;
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
    pub use crate::physics::avian::{
        BoneForCollider, CharacterColliders, CharacterRagdoll, ColliderForCharacter,
        ColliderOffset, DisablePhysics, RagdollCollisionLayers, RagdollJointLimit,
        RagdollJointLimitOverrides, default_joint_limit, resolve_joint_limit,
    };
    pub use crate::physics::{
        COLLIDERS, ColliderBone, HumentityRagdollSystemSet, RagdollDamping, RagdollDensity,
        RagdollMobility, get_collider_parent,
    };
    pub use crate::{
        HumentityAssetsReady, HumentityPlugin, NAME_INTERNER,
        animation::{
            BakedBoneCorrection, BoneTranslationCorrection, DynamicRetargeting,
            HumentitySkeletonSystemSet, RootOnlyRetargeting, ShapeBakedCorrections,
            shape_baked_corrections_for_shape,
        },
        assets::{StitchedPart, StitchedParts, TemplateOverride, shape_mesh_from_helpers_mhclo},
        basemesh::{BaseMesh, VertexGroups},
        bone_debug::BoneDebugPlugin,
        gpu::{
            ATTRIBUTE_GPU_JOINT_INDEX, ATTRIBUTE_GPU_JOINT_WEIGHT, BIND_POSE_CLIP, BIND_POSE_SLOT,
            CROWD_SKIN_SHADER, CrowdMaterial, GpuAnimationBank, GpuAnimationReady, GpuBlendWeights,
            GpuClipMode, GpuCrowdConfig, GpuCrowdExtension, GpuCrowdShapes, GpuCrowdUniform,
            GpuInstanceAnims, GpuJointsReadback, GpuOneShotDone, GpuRenderHandles, GpuShapeSkeleton,
            GpuSkeletonLod, HumentityGpuPlugin, POSE_SHADER, POSE_WORKGROUP_X, POSE_WORKGROUP_Y,
            POSE_WORKGROUP_Z, fit_shape_skeleton, fit_shape_skeleton_from_helpers, gpu_skin_wgsl,
            make_gpu_mesh, pose_grid_side, specialize_gpu_vertex_layout,
        },
        helpers::{RefitCharacter, TeardownCharacter},
        load_and_insert_humentity_assets,
        loaders::{
            BoneJsonConfig, BoneTransformSpec, CategoryMorphsAsset, CharacterShapeAsset,
            CharacterShapeConfigLoader, CharacterTemplateAssetLoader, CompositeTarget,
            CompositeTargetsAsset, MacroBoundString, MacroBounds, MacroDataAsset,
            MacroDataAssetLoader, MhcloAsset, MhcloAssetLoader, ObjVertsAsset, ObjVertsAssetLoader,
            ObjVertsSettings, OppositesAsset, ReferenceRigAsset, ReferenceRigAssetLoader,
            RotationOnlyAnimationAsset, RotationOnlyAnimationAssetLoader,
            RotationOnlyAnimationSettings,
            RigConfigAsset, RigConfigAssetLoader, RigWeightsAsset, RigWeightsAssetLoader,
            ShapeBakedAnimationAsset, ShapeBakedAnimationAssetLoader, ShapeBakedAnimationSettings,
            ShapeBakeRequest, TargetAsset, TargetAssetLoader, TargetDelta, TargetManifestAssetLoader,
            VertexGroupsAsset, VertexGroupsAssetLoader, bake_shape_clips_from_bytes,
        },
        mesh_ops::{generate_mhid_lookup, generate_vertex_map, get_vertex_positions},
        morphs::{MakeHumanMorphs, MorphError, MorphTargets, adjust_helpers_to_morphs},
        rigs::{RigData, RigSpec, SkeletalBone, SkeletonRootBone},
        skeleton_lod::{
            BoneMergeConfig, MAX_LODS, MergeIntoKeptBone, RigBundle, SkeletonLodConfig, SkeletonLodData,
        },
        spawn_mesh::{
            CachedMhcloMeshHandles, CharacterPart, CharacterShape, GpuCharacterPart,
            LoadAssetMeshJob, MeshBuildLod, MhcloMeshBuilder, build_single_mesh_direct,
            request_gpu_mesh,
        },
        spawn_skeleton::{
            CharacterScale, CharacterSkeleton, ResetToBindPose, SkeletonLodDisabled, SkeletonLodState,
            SkeletonsReady,
        },
        template::{CharacterMorphShape, CharacterTemplate},
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

/// Marker resource inserted once all core humentity assets (BaseMesh,
/// VertexGroups, MakeHumanMorphs, RigData) have finished loading.
/// Use `resource_exists::<HumentityAssetsReady>` in `run_if` conditions
/// to gate systems that need all assets available.
#[derive(Resource)]
pub struct HumentityAssetsReady;

fn check_humentity_assets_ready(
    basemesh: Res<basemesh::BaseMesh>,
    vg: Res<basemesh::VertexGroups>,
    morphs: Res<morphs::MakeHumanMorphs>,
    rig_data: Res<rigs::RigData>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    if !basemesh.vertices.is_empty()
        && !vg.is_empty()
        && morphs.is_ready(&asset_server)
        && rig_data.is_loaded()
    {
        commands.insert_resource(HumentityAssetsReady);
    }
}

/// Model verts are facing Z instead of NEG_Z, so forward() faces the wrong direction.
pub(crate) const MODEL_ROTATION_FIX: Quat = Quat::from_xyzw(0., 1., 0., 0.);

/// The plugin struct. Retargeting is per character: put
/// [`RootOnlyRetargeting`](crate::animation::RootOnlyRetargeting) or
/// [`DynamicRetargeting`](crate::animation::DynamicRetargeting) on the
/// `CharacterShape` entity; both systems are always registered but each runs
/// only for characters carrying its marker. Shape-baked clips need no marker
/// and no system.
#[derive(Default)]
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
            .init_asset::<RotationOnlyAnimationAsset>()
            .register_asset_loader(RotationOnlyAnimationAssetLoader)
            .init_asset::<ShapeBakedAnimationAsset>()
            .register_asset_loader(ShapeBakedAnimationAssetLoader)
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
                    check_humentity_assets_ready
                        .run_if(not(resource_exists::<HumentityAssetsReady>)),
                    spawn_skeleton::enforce_skeleton_lod_config_frozen.after(rigs::build_rig_scenes),
                    basemesh::extract_basemesh_asset.run_if(resource_exists::<basemesh::BaseMesh>),
                    basemesh::extract_vertex_groups_asset
                        .run_if(resource_exists::<basemesh::VertexGroups>),
                    morphs::sync_macro_data.run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    morphs::sync_composite_data.run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    morphs::sync_loaded_morph_targets
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>),
                    rigs::sync_and_build_rig_data
                        .run_if(resource_exists::<rigs::RigData>)
                        .run_if(not(resource_exists::<BuiltRigs>)),
                    rigs::build_rig_scenes
                        .after(rigs::sync_and_build_rig_data)
                        .run_if(not(resource_exists::<BuiltRigs>)),
                    (
                        (
                            template::bake_template_deltas,
                            spawn_skeleton::spawn_rig_skeleton,
                            spawn_skeleton::fit_skeleton_to_shape,
                            spawn_skeleton::check_skeletons_ready,
                            spawn_skeleton::setup_part_skinning,
                        )
                            .chain()
                            .run_if(resource_exists::<HumentityAssetsReady>),
                        // No mesh may construct until every core asset is proven
                        // loaded. Resource existence alone races async loads and
                        // stranded jobs in BuildSubmitted with no mesh.
                        (spawn_mesh::mesh_build, spawn_mesh::mediators_clean_up)
                            .chain()
                            .run_if(resource_exists::<HumentityAssetsReady>),
                    )
                        .chain()
                        .after(rigs::build_rig_scenes)
                        .run_if(resource_exists::<basemesh::BaseMesh>)
                        .run_if(resource_exists::<basemesh::VertexGroups>)
                        .run_if(resource_exists::<morphs::MakeHumanMorphs>)
                        .run_if(resource_exists::<rigs::RigData>),
                ),
            )
            .add_observer(spawn_skeleton::on_teardown_character)
            .add_observer(spawn_skeleton::on_refit_character)
            .add_observer(spawn_skeleton::on_reset_to_bind_pose)
            .add_systems(
                Update,
                template::resolve_template_morphs
                    .before(spawn_mesh::mesh_build)
                    .run_if(resource_exists::<morphs::MakeHumanMorphs>),
            )
            .add_systems(
                PostUpdate,
                (
                    animation::rescale_root_bone_translation
                        .in_set(animation::HumentitySkeletonSystemSet)
                        .after(bevy::app::AnimationSystems),
                    animation::rescale_dynamic_retargeting
                        .in_set(animation::HumentitySkeletonSystemSet)
                        .after(bevy::app::AnimationSystems),
                    spawn_skeleton::sync_skeleton_lod_subtrees
                        .in_set(animation::HumentitySkeletonSystemSet)
                        .after(bevy::app::AnimationSystems),
                )
                    .before(TransformSystems::Propagate),
            );
        // Both retarget systems are always registered; each filters by its
        // marker (`RootOnlyRetargeting` owns just the root, `DynamicRetargeting`
        // owns root + all bones together), so unmarked characters cost nothing
        // and mixed styles work side by side. Baked clips need no marker.

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
                        physics::avian::set_ragdoll_state.in_set(HumentityRagdollSystemSet),
                        physics::avian::apply_joint_limit_overrides,
                        physics::avian::update_collision_layers,
                    ),
                )
                .add_systems(FixedUpdate, physics::avian::sync_colliders)
                .add_systems(
                    PostUpdate,
                    physics::avian::sync_bones_to_ragdoll
                        .in_set(animation::HumentitySkeletonSystemSet)
                        .after(AnimationSystems)
                        .before(TransformSystems::Propagate),
                )
                .add_observer(physics::avian::on_teardown_character)
                .add_observer(physics::avian::on_refit_character)
                .add_observer(physics::avian::on_disable_physics);
        }
    }
}
