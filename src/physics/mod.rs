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

pub(crate) const COLLIDERS: [ColliderBone; 15] = [
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

pub(crate) fn get_collider_parent(bone: ColliderBone) -> Option<ColliderBone> {
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

#[cfg(feature = "avian")]
pub mod avian;
#[cfg(feature = "physx")]
pub mod physx;
