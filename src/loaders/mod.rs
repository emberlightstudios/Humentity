mod obj_verts;
mod mhclo;
mod macro_json;
mod retargeted_animation;
mod rig_config;
mod rig_weights;
mod target;
mod composite_target_manifest;
mod reference_rig;
mod template_asset;
mod shape_config;

pub use obj_verts::{
    ObjVertsAsset,
    ObjVertsAssetLoader,
    ObjVertsSettings,
    VertexGroupsAsset,
    VertexGroupsAssetLoader,
};
pub use mhclo::{MhcloAsset, MhcloAssetLoader, MhcloVertexMap};
pub use macro_json::{MacroBoundString, MacroBounds, MacroDataAsset, MacroDataAssetLoader};
pub use retargeted_animation::{
    RetargetedAnimationAsset,
    RetargetedAnimationAssetLoader,
    RetargetedAnimationSettings,
};
pub use rig_config::{BoneJsonConfig, BoneTransformSpec, RigConfigAsset, RigConfigAssetLoader};
pub use rig_weights::{RigWeightsAsset, RigWeightsAssetLoader};
pub use target::{TargetAsset, TargetAssetLoader, TargetDelta};
pub use composite_target_manifest::{
    CategoryMorphsAsset,
    CompositeTarget,
    CompositeTargetsAsset,
    OppositesAsset,
    TargetManifestAssetLoader,
};
pub use reference_rig::{ReferenceRigAsset, ReferenceRigAssetLoader};

pub use template_asset::CharacterTemplateAssetLoader;
pub use shape_config::{CharacterShapeAsset, CharacterShapeConfigLoader};
