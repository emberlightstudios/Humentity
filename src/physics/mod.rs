// These are used conditionally by the `avian` and `physx` feature-gated submodules.
#![allow(dead_code)]

use bevy::prelude::*;

pub const HEAD_VERTICES: [usize; 2] = [5063, 5389];
pub const TORSO_VERTICES: [usize; 4] = [1553, 3753, 4097, 4049];
pub const PELVIS_VERTICES: [usize; 4] = [4174, 4243, 4353, 4170];
pub const UPPER_RIGHT_LEG_VERTICES: [usize; 4] = [4407, 4268, 4567, 4565];
pub const LOWER_RIGHT_LEG_VERTICES: [usize; 4] = [4662, 4664, 6385, 6375];
pub const UPPER_RIGHT_ARM_VERTICES: [usize; 4] = [1630, 1432, 3330, 3323];
pub const LOWER_RIGHT_ARM_VERTICES: [usize; 4] = [3412, 3877, 3552, 3906];
pub const UPPER_LEFT_ARM_VERTICES: [usize; 4] = [8302, 8120, 9998, 9991];
pub const LOWER_LEFT_ARM_VERTICES: [usize; 4] = [10080, 10542, 10220, 10571];
pub const UPPER_LEFT_LEG_VERTICES: [usize; 4] = [11025, 10898, 11185, 11183];
pub const LOWER_LEFT_LEG_VERTICES: [usize; 4] = [11280, 11282, 12982, 12972];
pub const RIGHT_HAND_VERTICES: [usize; 6] = [2776, 3189, 2119, 3909, 3247, 3650];
pub const RIGHT_FOOT_VERTICES: [usize; 6] = [6251, 6705, 4972, 5845, 6214, 6298];
pub const LEFT_HAND_VERTICES: [usize; 6] = [9444, 9857, 8787, 10574, 9915, 10318];
pub const LEFT_FOOT_VERTICES: [usize; 6] = [12848, 13301, 11590, 12442, 12811, 12895];

#[derive(Component, Hash, Copy, Clone, Eq, PartialEq, Debug)]
pub enum ColliderBone {
    Head,
    Chest,
    Pelvis,
    UpperRightArm,
    UpperLeftArm,
    LowerRightArm,
    LowerLeftArm,
    UpperRightLeg,
    UpperLeftLeg,
    LowerRightLeg,
    LowerLeftLeg,
    LeftHand,
    RightHand,
    LeftFoot,
    RightFoot,
}

pub const COLLIDERS: [ColliderBone; 15] = [
    ColliderBone::Pelvis,
    ColliderBone::Chest,
    ColliderBone::UpperLeftLeg,
    ColliderBone::UpperRightLeg,
    ColliderBone::LowerLeftLeg,
    ColliderBone::LowerRightLeg,
    ColliderBone::LeftFoot,
    ColliderBone::RightFoot,
    ColliderBone::UpperLeftArm,
    ColliderBone::UpperRightArm,
    ColliderBone::LowerLeftArm,
    ColliderBone::LowerRightArm,
    ColliderBone::LeftHand,
    ColliderBone::RightHand,
    ColliderBone::Head,
];

pub(crate) const DEFAULT_RIG_COLLIDER_BONE_NAMES: [&str; 15] = [
    "root",
    "spine03",
    "upperleg01.L",
    "upperleg01.R",
    "lowerleg01.L",
    "lowerleg01.R",
    "foot.L",
    "foot.R",
    "upperarm01.L",
    "upperarm01.R",
    "lowerarm01.L",
    "lowerarm01.R",
    "wrist.L",
    "wrist.R",
    "head",
];

pub(crate) const fn get_collider_parent(bone: ColliderBone) -> Option<ColliderBone> {
    match bone {
        ColliderBone::Head => Some(ColliderBone::Chest),
        ColliderBone::Chest => Some(ColliderBone::Pelvis),
        ColliderBone::Pelvis => None,
        ColliderBone::UpperRightArm => Some(ColliderBone::Chest),
        ColliderBone::UpperLeftArm => Some(ColliderBone::Chest),
        ColliderBone::LowerRightArm => Some(ColliderBone::UpperRightArm),
        ColliderBone::LowerLeftArm => Some(ColliderBone::UpperLeftArm),
        ColliderBone::UpperRightLeg => Some(ColliderBone::Pelvis),
        ColliderBone::UpperLeftLeg => Some(ColliderBone::Pelvis),
        ColliderBone::LowerRightLeg => Some(ColliderBone::UpperRightLeg),
        ColliderBone::LowerLeftLeg => Some(ColliderBone::UpperLeftLeg),
        ColliderBone::LeftHand => Some(ColliderBone::LowerLeftArm),
        ColliderBone::RightHand => Some(ColliderBone::LowerRightArm),
        ColliderBone::LeftFoot => Some(ColliderBone::LowerLeftLeg),
        ColliderBone::RightFoot => Some(ColliderBone::LowerRightLeg),
    }
}

/// Scales the mass of all ragdoll colliders on this character.
///
/// This is the `ColliderDensity` used on each collider bone.
/// Default: `100.0`.
#[derive(Component, Clone, Copy, Debug, Reflect)]
#[reflect(Component, Debug)]
pub struct RagdollDensity(pub f32);

impl Default for RagdollDensity {
    fn default() -> Self {
        Self(100.0)
    }
}

/// Linear and angular damping applied to each joint when the ragdoll activates.
///
/// Both `JointDamping.linear` and `JointDamping.angular` are set to this value.
/// Default: `5.0`.
#[derive(Component, Clone, Copy, Debug, Reflect)]
#[reflect(Component, Debug)]
pub struct RagdollDamping(pub f32);

impl Default for RagdollDamping {
    fn default() -> Self {
        Self(5.0)
    }
}

/// Multiplier for joint compliance on this character's ragdoll.
///
/// The joint compliance values (point, align, swing, twist) are all
/// multiplied by this value:
///   - `1.0` = default compliance (default)
///   - `0.5` = stiffer joints (half the compliance)
///   - `2.0` = softer joints (double the compliance)
///
/// Apply to your character entity.
#[derive(Component, Clone, Copy, Debug, Reflect)]
#[reflect(Component, Debug)]
pub struct RagdollCompliance(pub f32);

impl Default for RagdollCompliance {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Controls the fraction of full joint range the ragdoll can use.
///
/// The joint limits are scaled by this value:
///   - `1.0` = full anatomical range (default)
///   - `0.5` = half the anatomical range
///   - `0.0` = fully locked (no movement)
///
/// Apply to your character entity.
#[derive(Component, Clone, Copy, Debug)]
pub struct RagdollMobility(pub f32);

impl Default for RagdollMobility {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Marker component that puts the ragdoll to sleep and freezes it.
///
/// When added to a character entity, all collider velocities are zeroed
/// and both sync directions are disabled:
///   - `sync_colliders` (bone → collider for kinematic) is skipped
///   - `sync_bones_to_ragdoll` (collider → bone for dynamic) is skipped
///
/// Remove this component to re-enable normal ragdoll syncing.
#[derive(Component, Clone, Copy, Debug)]
pub struct RagdollSleep;

#[cfg(feature = "avian")]
pub mod avian;
#[cfg(feature = "physx")]
pub mod physx;
#[cfg(feature = "rapier")]
pub mod rapier;
