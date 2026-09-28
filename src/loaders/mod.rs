mod composite_target_manifest;
mod macro_json;
mod mhclo;
mod obj_verts;
mod reference_rig;
mod rotation_only_animation;
mod rig_config;
mod rig_weights;
mod shape_config;
mod target;
mod template_asset;

pub use composite_target_manifest::{
    CategoryMorphsAsset, CompositeTarget, CompositeTargetsAsset, OppositesAsset,
    TargetManifestAssetLoader,
};
pub use macro_json::{MacroBoundString, MacroBounds, MacroDataAsset, MacroDataAssetLoader};
pub use mhclo::{MhcloAsset, MhcloAssetLoader, MhcloVertexMap};
pub use obj_verts::{
    ObjVertsAsset, ObjVertsAssetLoader, ObjVertsSettings, VertexGroupsAsset,
    VertexGroupsAssetLoader,
};
pub use reference_rig::{ReferenceRigAsset, ReferenceRigAssetLoader};
pub use rotation_only_animation::{
    RotationOnlyAnimationAsset, RotationOnlyAnimationAssetLoader, RotationOnlyAnimationSettings,
    ShapeBakeRequest, ShapeBakedAnimationAsset, ShapeBakedAnimationAssetLoader,
    ShapeBakedAnimationSettings, bake_shape_clips_from_bytes,
};
pub use rig_config::{BoneJsonConfig, BoneTransformSpec, RigConfigAsset, RigConfigAssetLoader};
pub use rig_weights::{RigWeightsAsset, RigWeightsAssetLoader};
pub use target::{TargetAsset, TargetAssetLoader, TargetDelta};

pub use shape_config::{CharacterShapeAsset, CharacterShapeConfigLoader};
pub use template_asset::CharacterTemplateAssetLoader;
