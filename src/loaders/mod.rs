//! Asset loaders for the MakeHuman data files.
//!
//! An `AssetLoader::extensions` claim is global: whoever claims `"toml"` or
//! `"json"` receives every user file with that suffix. So the self-describing
//! MakeHuman suffixes (`.mhclo`, `.proxy`, `.macro`, `.target`) are claimed,
//! and the loaders for generic data formats (`.toml`: template, profile and
//! shape; `.obj`: `ObjVertsAsset`) claim nothing. Those are reached through a
//! typed load — `asset_server.load::<CharacterTemplate>(path)` — which
//! `AssetLoaders::find` resolves by asset type before it consults the path.
//!
//! The one untyped entry point is `.target`: `MakeHumanMorphs` enumerates them
//! with `AssetServer::load_folder`, which has no asset type to resolve by.

mod composite_target_manifest;
mod macro_json;
mod mhclo;
mod obj_verts;
mod profile;
mod reference_rig;
mod rig_config;
mod rig_weights;
mod rotation_only_animation;
mod shape;
mod target;
mod template;

pub use composite_target_manifest::{
    CategoryMorphsAsset, CompositeTarget, CompositeTargetsAsset, OppositesAsset,
    TargetManifestAssetLoader,
};
pub use macro_json::{MacroBounds, MacroDataAsset, MacroDataAssetLoader};
pub use mhclo::{MhcloAsset, MhcloAssetLoader, MhcloVertexMap};
pub use obj_verts::{
    ObjVertsAsset, ObjVertsAssetLoader, ObjVertsSettings, VertexGroupsAsset,
    VertexGroupsAssetLoader,
};
pub use profile::{CharacterProfile, CharacterProfileAssetLoader, ProfileError};
pub use reference_rig::ReferenceRigAsset;
pub use rig_config::{BoneJsonConfig, BoneTransformSpec, RigConfigAsset, RigConfigAssetLoader};
pub use rig_weights::{RigWeightsAsset, RigWeightsAssetLoader};
pub use rotation_only_animation::{
    RotationOnlyAnimationAsset, RotationOnlyAnimationAssetLoader, RotationOnlyAnimationSettings,
    ShapeBakeRequest, ShapeBakedAnimationAsset, ShapeBakedAnimationAssetLoader,
    ShapeBakedAnimationSettings,
};
pub use target::{TargetAsset, TargetAssetLoader, TargetDelta};

pub use shape::{CharacterShape, CharacterShapeAssetLoader};
pub use template::{CharacterTemplate, CharacterTemplateAssetLoader};
