use ahash::AHashMap;
use avian3d::prelude::*;
use bevy::prelude::*;

use crate::rigs::{BoneData, RigData, RigType};

/// Use to find radius and center of sphere
const HEAD_VERTICES: [usize; 2] = [5063, 5389];
/// These are for the right side fo the body only.
/// Augment with 4 more verts with x -> -x
/// Use to find box shape
const TORSO_VERTICES: [usize; 4] = [1553, 3753, 4097, 4049];
const PELVIS_VERTICES: [usize; 4] = [4174, 4243, 4353, 4170];
// Use first 2 to find center and radius of capsule top
// Use last 2 to find center and radius of capsule bottom
const UPPER_LEG_VERTICES: [usize; 4] = [4407, 4268, 4567, 4565];
const LOWER_LEG_VERTICES: [usize; 4] = [4662, 4664, 6385, 6375];
const UPPER_ARM_VERTICES: [usize; 4] = [1630, 1432, 3330, 3323];
const LOWER_ARM_VERTICES: [usize; 4] = [3412, 3877, 3552, 3906];
/// Use cuboids. Get dimensions from  2x, 2y, 2z
const HAND_VERTICES: [usize; 6] = [2776, 3189, 2119, 3909, 3247, 3650];
const FOOT_VERTICES: [usize; 6] = [6251, 6705, 4972, 5845, 6214, 6298];

pub(crate) fn control_ragdoll(
    mut commands: Commands,
    ragdolls: Query<
        (Entity, &CharacterRagdoll),
        (With<CharacterRagdoll>, Changed<CharacterRagdoll>),
    >,
    joints_containers: Query<Entity, With<PhysicsJointsContainer>>,
    children: Query<&Children>,
) {
    for (root, ragdoll) in ragdolls {
        if ragdoll.active {
            for &entity in ragdoll.rigidbodies.values() {
                commands.entity(entity).insert(RigidBody::Dynamic);
                ragdoll.spawn_joints(&mut commands, root);
            }
        } else {
            for &entity in ragdoll.rigidbodies.values() {
                commands.entity(entity).remove::<RigidBody>();
                for child in children.iter_descendants(entity) {
                    if joints_containers.get(child).is_ok() {
                        commands.entity(child).despawn();
                        break;
                    }
                }
            }
        }
    }
}

#[derive(Component, Default)]
#[require(PhysicsJointsContainer)]
pub struct CharacterRagdoll {
    pub active: bool,
    rigidbodies: AHashMap<RagdollBone, Entity>,
}

#[derive(Component, Default)]
pub(crate) struct PhysicsJointsContainer;

#[derive(Hash, Eq, PartialEq)]
enum RagdollBone {
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

impl CharacterRagdoll {
    pub fn new(active: bool) -> Self {
        Self {
            active,
            ..default()
        }
    }

    pub(crate) fn spawn_ragdoll(
        &mut self,
        commands: &mut Commands,
        helpers: &[Vec3],
        rig_type: RigType,
        bone_entities: &AHashMap<&'static str, Entity>,
        global_transforms: &Query<&GlobalTransform>,
        rig_data: &RigData,
        rig_entity: Entity,
    ) {
        let config = &rig_data.configs[&rig_type];
        let rig_transform = Transform::from(*global_transforms.get(rig_entity).unwrap());

        let (collider, transform) = self.get_head_collider(helpers);
        let name = self.get_head_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::Head,
            commands,
        );

        let (collider, transform) = self.get_midsection_collider(helpers, RagdollBone::Chest);
        let name = self.get_torso_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::Chest,
            commands,
        );

        let (collider, transform) = self.get_midsection_collider(helpers, RagdollBone::Pelvis);
        let name = self.get_pelvis_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::Pelvis,
            commands,
        );

        let (collider, transform) = self.get_limb_collider(helpers, RagdollBone::UpperLeftArm);
        let name = self.get_upper_left_arm_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::UpperLeftArm,
            commands,
        );

        let (collider, transform) = self.get_limb_collider(helpers, RagdollBone::UpperRightArm);
        let name = self.get_upper_right_arm_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::UpperRightArm,
            commands,
        );

        let (collider, transform) = self.get_limb_collider(helpers, RagdollBone::LowerLeftArm);
        let name = self.get_lower_left_arm_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::LowerLeftArm,
            commands,
        );

        let (collider, transform) = self.get_limb_collider(helpers, RagdollBone::LowerRightArm);
        let name = self.get_lower_right_arm_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::LowerRightArm,
            commands,
        );

        let (collider, transform) = self.get_limb_collider(helpers, RagdollBone::UpperLeftLeg);
        let name = self.get_upper_left_leg_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::UpperLeftLeg,
            commands,
        );

        let (collider, transform) = self.get_limb_collider(helpers, RagdollBone::UpperRightLeg);
        let name = self.get_upper_right_leg_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::UpperRightLeg,
            commands,
        );

        let (collider, transform) = self.get_limb_collider(helpers, RagdollBone::LowerLeftLeg);
        let name = self.get_lower_left_leg_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::LowerLeftLeg,
            commands,
        );

        let (collider, transform) = self.get_limb_collider(helpers, RagdollBone::LowerRightLeg);
        let name = self.get_lower_right_leg_bone_name(rig_type);
        self.insert_collider(
            collider,
            transform * rig_transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::LowerRightLeg,
            commands,
        );

        let (collider, transform) = self.get_extremity_collider(helpers, RagdollBone::LeftHand);
        let name = self.get_left_hand_name(rig_type);
        let &bone = bone_entities.get(name).unwrap();
        let transform = Transform::from(*global_transforms.get(bone).unwrap()) * transform;
        self.insert_collider(
            collider,
            transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::LeftHand,
            commands,
        );

        let (collider, transform) = self.get_extremity_collider(helpers, RagdollBone::RightHand);
        let name = self.get_right_hand_name(rig_type);
        let &bone = bone_entities.get(name).unwrap();
        let transform = Transform::from(*global_transforms.get(bone).unwrap()) * transform;
        self.insert_collider(
            collider,
            transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::RightHand,
            commands,
        );

        let (collider, transform) = self.get_extremity_collider(helpers, RagdollBone::LeftFoot);
        let name = self.get_left_foot_name(rig_type);
        let &bone = bone_entities.get(name).unwrap();
        let transform = Transform::from(*global_transforms.get(bone).unwrap()) * transform;
        self.insert_collider(
            collider,
            transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::LeftFoot,
            commands,
        );

        let (collider, transform) = self.get_extremity_collider(helpers, RagdollBone::RightFoot);
        let name = self.get_right_foot_name(rig_type);
        let &bone = bone_entities.get(name).unwrap();
        let transform = Transform::from(*global_transforms.get(bone).unwrap()) * transform;
        self.insert_collider(
            collider,
            transform,
            name,
            bone_entities,
            global_transforms,
            rig_entity,
            config,
            RagdollBone::RightFoot,
            commands,
        );
    }

    pub(crate) fn spawn_joints(&self, commands: &mut Commands, entity: Entity) {
        // ---------------- Set Up Physics Joints -------------------//
        let damping = JointDamping {
            linear: 0.5,
            angular: 0.5,
        };

        let head = self.rigidbodies[&RagdollBone::Head];
        let torso = self.rigidbodies[&RagdollBone::Chest];
        let mut joints = vec![];
        joints.push(
            commands
                .spawn((
                    SphericalJoint::new(head, torso),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Neck Joint"),
                ))
                .id(),
        );

        let hips = self.rigidbodies[&RagdollBone::Pelvis];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(hips, torso),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Spine Joint"),
                ))
                .id(),
        );

        let upper_leg = self.rigidbodies[&RagdollBone::UpperLeftLeg];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(hips, upper_leg),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Left Hip Joint"),
                ))
                .id(),
        );

        let lower_leg = self.rigidbodies[&RagdollBone::LowerLeftLeg];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(lower_leg, upper_leg),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Left Knee Joint"),
                ))
                .id(),
        );

        let foot = self.rigidbodies[&RagdollBone::LeftFoot];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(lower_leg, foot),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Left Ankle Joint"),
                ))
                .id(),
        );

        let upper_leg = self.rigidbodies[&RagdollBone::UpperRightLeg];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(hips, upper_leg),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Right Hip Joint"),
                ))
                .id(),
        );

        let lower_leg = self.rigidbodies[&RagdollBone::LowerRightLeg];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(lower_leg, upper_leg),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Right Knee Joint"),
                ))
                .id(),
        );

        let foot = self.rigidbodies[&RagdollBone::RightFoot];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(lower_leg, foot),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Right Ankle Joint"),
                ))
                .id(),
        );

        let upper_arm = self.rigidbodies[&RagdollBone::UpperLeftArm];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(torso, upper_arm),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Left Shoulder Joint"),
                ))
                .id(),
        );

        let lower_arm = self.rigidbodies[&RagdollBone::LowerLeftArm];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(lower_arm, upper_arm),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Left Elbow Joint"),
                ))
                .id(),
        );

        let hand = self.rigidbodies[&RagdollBone::LeftHand];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(lower_arm, hand),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Left Wrist Joint"),
                ))
                .id(),
        );

        let upper_arm = self.rigidbodies[&RagdollBone::UpperRightArm];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(torso, upper_arm),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Right Shoulder Joint"),
                ))
                .id(),
        );

        let lower_arm = self.rigidbodies[&RagdollBone::LowerRightArm];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(lower_arm, upper_arm),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Right Elbow Joint"),
                ))
                .id(),
        );

        let hand = self.rigidbodies[&RagdollBone::RightHand];
        joints.push(
            commands
                .spawn((
                    FixedJoint::new(lower_arm, hand),
                    JointCollisionDisabled,
                    damping,
                    Name::new("Right Wrist Joint"),
                ))
                .id(),
        );

        let container = commands.entity(entity).insert(PhysicsJointsContainer).id();
        commands.entity(container).add_children(&joints);
    }

    fn insert_collider(
        &mut self,
        collider: Collider,
        transform: Transform,
        name: &'static str,
        bone_entities: &AHashMap<&'static str, Entity>,
        global_transforms: &Query<&GlobalTransform>,
        rig_entity: Entity,
        config: &AHashMap<&'static str, BoneData>,
        ragdoll_bone: RagdollBone,
        commands: &mut Commands,
    ) {
        let parent_name = config[name].parent;
        let &parent = bone_entities
            .get(parent_name)
            .map_or(&rig_entity, |parent| parent);
        let parent_transform = global_transforms.get(parent).unwrap();
        let transform =
            Transform::from_matrix(parent_transform.to_matrix().inverse() * transform.to_matrix());
        let collider = commands.spawn((transform, collider)).id();
        let rb = commands
            .spawn((
                Transform::IDENTITY,
                //RigidBody::Dynamic,
                //RigidBodyDisabled,
            ))
            .id();
        commands.entity(parent).add_child(rb);
        commands.entity(rb).add_child(collider);
        self.rigidbodies.insert(ragdoll_bone, rb);
    }

    fn get_head_collider(&self, helpers: &[Vec3]) -> (Collider, Transform) {
        let center = (helpers[HEAD_VERTICES[0]] + helpers[HEAD_VERTICES[1]]) * 0.5;
        let radius = (helpers[HEAD_VERTICES[0]] - center).length();
        (
            Collider::sphere(radius),
            Transform::from_translation(center),
        )
    }

    fn get_midsection_collider(
        &self,
        helpers: &[Vec3],
        joint: RagdollBone,
    ) -> (Collider, Transform) {
        let ref_verts = match joint {
            RagdollBone::Chest => TORSO_VERTICES,
            RagdollBone::Pelvis => PELVIS_VERTICES,
            _ => unimplemented!("wrong joint iniput"),
        };

        let mut verts = [Vec3::ZERO; 8];
        for (i, mhv) in ref_verts.iter().enumerate() {
            verts[i] = helpers[*mhv];
            verts[i + 4] = Vec3::new(-verts[i].x, verts[i].y, verts[i].z)
        }
        let center = verts.iter().sum::<Vec3>() / 8.;

        let xmin = verts
            .iter()
            .map(|v| v.x)
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        let xmax = verts
            .iter()
            .map(|v| v.x)
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        let ymin = verts
            .iter()
            .map(|v| v.y)
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        let ymax = verts
            .iter()
            .map(|v| v.y)
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        let zmin = verts
            .iter()
            .map(|v| v.z)
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        let zmax = verts
            .iter()
            .map(|v| v.z)
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        (
            Collider::cuboid(xmax - xmin, ymax - ymin, zmax - zmin),
            Transform::from_translation(center),
        )
    }

    fn get_limb_collider(&self, helpers: &[Vec3], joint: RagdollBone) -> (Collider, Transform) {
        let ref_verts = match joint {
            RagdollBone::LowerLeftArm | RagdollBone::LowerRightArm => LOWER_ARM_VERTICES,
            RagdollBone::UpperLeftArm | RagdollBone::UpperRightArm => UPPER_ARM_VERTICES,
            RagdollBone::LowerLeftLeg | RagdollBone::LowerRightLeg => LOWER_LEG_VERTICES,
            RagdollBone::UpperLeftLeg | RagdollBone::UpperRightLeg => UPPER_LEG_VERTICES,
            _ => unimplemented!("wrong joint iniput"),
        };
        let mut verts = [Vec3::ZERO; 4];
        for (i, &mhv) in ref_verts.iter().enumerate() {
            verts[i] = helpers[mhv];
            if matches!(joint, RagdollBone::LowerRightArm)
                || matches!(joint, RagdollBone::UpperRightArm)
                || matches!(joint, RagdollBone::LowerRightLeg)
                || matches!(joint, RagdollBone::UpperRightLeg)
            {
                verts[i].x = -verts[i].x;
            }
        }

        let p1 = (verts[0] + verts[1]) * 0.5;
        let p2 = (verts[2] + verts[3]) * 0.5;
        let r = (verts[0] - verts[1]).length() * 0.5;

        (Collider::capsule_endpoints(r, p1, p2), Transform::IDENTITY)
    }

    fn get_extremity_collider(
        &self,
        helpers: &[Vec3],
        joint: RagdollBone,
    ) -> (Collider, Transform) {
        let ref_verts = match joint {
            RagdollBone::LeftHand | RagdollBone::RightHand => HAND_VERTICES,
            RagdollBone::LeftFoot | RagdollBone::RightFoot => FOOT_VERTICES,
            _ => unimplemented!("wrong joint input"),
        };
        let mut verts = [Vec3::ZERO; 6];
        for (i, &mhv) in ref_verts.iter().enumerate() {
            verts[i] = helpers[mhv];
            if matches!(joint, RagdollBone::RightHand) || matches!(joint, RagdollBone::RightFoot) {
                verts[i].x = -verts[i].x;
            }
        }

        let x = (verts[0] - verts[1]).length();
        let y = (verts[2] - verts[3]).length();
        let z = (verts[4] - verts[5]).length();

        (
            Collider::cuboid(x, y, z),
            Transform::from_translation(Vec3::Y * y * 0.4),
        )
    }

    pub(crate) fn get_head_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "head",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_torso_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "spine03",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_pelvis_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "root",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_upper_left_arm_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "upperarm01.L",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_upper_right_arm_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "upperarm01.R",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_lower_left_arm_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "lowerarm01.L",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_lower_right_arm_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "lowerarm01.R",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_upper_left_leg_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "upperleg01.L",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_upper_right_leg_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "upperleg01.R",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_lower_left_leg_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "lowerleg01.L",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_lower_right_leg_bone_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "lowerleg01.R",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_left_hand_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "metacarpal2.L",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_right_hand_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "metacarpal2.R",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_left_foot_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "foot.L",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }

    pub(crate) fn get_right_foot_name(&self, rig_type: RigType) -> &'static str {
        match rig_type {
            RigType::Default => "foot.R",
            _ => unimplemented!("need more bone mappings set up"),
        }
    }
}
