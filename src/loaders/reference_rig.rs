use ahash::{AHashMap, AHashSet};
use bevy::{ecs::intern::Internable, prelude::*};

use crate::{NAME_INTERNER, loaders::RigConfigAsset, rigs::get_bone_transform};

/// Reference hierarchy and bind poses derived from MPFB rig JSON and helper vertices.
#[derive(Clone)]
pub struct ReferenceRigAsset {
    pub bone_names: Vec<&'static str>,
    pub bone_parents: AHashMap<&'static str, String>,
    pub local_bindpose: AHashMap<&'static str, Transform>,
    pub model_space_bindpose: AHashMap<&'static str, Transform>,
    pub bone_name_to_index: AHashMap<&'static str, usize>,
    pub rig_name: String,
}

impl ReferenceRigAsset {
    pub(crate) fn from_rig_config(
        rig_config: &RigConfigAsset,
        helpers: &[Vec3],
        vertex_groups: &AHashMap<String, Vec<[usize; 2]>>,
    ) -> Self {
        let mut bone_parents = AHashMap::default();
        let mut child_bone_names_by_parent = AHashMap::<&'static str, Vec<&'static str>>::default();
        let mut root_bone_names = Vec::new();

        for (&bone_name, bone_config) in &rig_config.bones {
            let parent_name = bone_config.parent.as_str();
            if parent_name.is_empty() || parent_name == "Human.rig" {
                root_bone_names.push(bone_name);
                bone_parents.insert(bone_name, "Human.rig".to_string());
            } else {
                let parent_bone_name = NAME_INTERNER.intern(parent_name).leak();
                assert!(
                    rig_config.bones.contains_key(parent_bone_name),
                    "rig config bone '{bone_name}' names missing parent '{parent_name}'"
                );
                child_bone_names_by_parent
                    .entry(parent_bone_name)
                    .or_default()
                    .push(bone_name);
                bone_parents.insert(bone_name, parent_name.to_string());
            }
        }

        assert_eq!(
            root_bone_names.len(),
            1,
            "rig config '{}' must have exactly one root bone (parent empty or 'Human.rig'), found {}",
            rig_config.rig_name,
            root_bone_names.len(),
        );
        root_bone_names.sort_unstable();
        for child_bone_names in child_bone_names_by_parent.values_mut() {
            child_bone_names.sort_unstable();
        }

        let mut bone_names = Vec::with_capacity(rig_config.bones.len());
        let mut visited_bone_names = AHashSet::default();
        for &root_bone_name in &root_bone_names {
            append_bone_subtree(
                root_bone_name,
                &child_bone_names_by_parent,
                &mut visited_bone_names,
                &mut bone_names,
            );
        }
        assert_eq!(
            bone_names.len(),
            rig_config.bones.len(),
            "rig config '{}' has bones disconnected from its root or a parent cycle",
            rig_config.rig_name,
        );

        let mut model_space_bindpose = AHashMap::default();
        for &bone_name in &bone_names {
            let bone_config = &rig_config.bones[bone_name];
            model_space_bindpose.insert(
                bone_name,
                get_bone_transform(bone_name, bone_config, vertex_groups, helpers),
            );
        }

        let mut local_bindpose = AHashMap::default();
        for &bone_name in &bone_names {
            let parent_name = bone_parents[bone_name].as_str();
            let parent_model_transform = if parent_name == "Human.rig" {
                Transform::IDENTITY
            } else {
                let parent_bone_name = NAME_INTERNER.intern(parent_name).leak();
                *model_space_bindpose
                    .get(parent_bone_name)
                    .unwrap_or_else(|| {
                        panic!(
                            "rig config bone '{bone_name}' parent '{parent_name}' has no model-space bind pose"
                        )
                    })
            };
            let child_model_transform = model_space_bindpose[bone_name];
            let child_local_transform = Transform::from_matrix(
                parent_model_transform.to_matrix().inverse() * child_model_transform.to_matrix(),
            );
            local_bindpose.insert(bone_name, child_local_transform);
        }

        let bone_name_to_index = bone_names
            .iter()
            .enumerate()
            .map(|(bone_index, &bone_name)| (bone_name, bone_index))
            .collect();

        Self {
            bone_names,
            bone_parents,
            local_bindpose,
            model_space_bindpose,
            bone_name_to_index,
            rig_name: rig_config.rig_name.clone(),
        }
    }
}

fn append_bone_subtree(
    parent_bone_name: &'static str,
    child_bone_names_by_parent: &AHashMap<&'static str, Vec<&'static str>>,
    visited_bone_names: &mut AHashSet<&'static str>,
    bone_names: &mut Vec<&'static str>,
) {
    assert!(
        visited_bone_names.insert(parent_bone_name),
        "rig config contains a parent cycle at bone '{parent_bone_name}'"
    );
    bone_names.push(parent_bone_name);

    if let Some(child_bone_names) = child_bone_names_by_parent.get(parent_bone_name) {
        for &child_bone_name in child_bone_names {
            append_bone_subtree(
                child_bone_name,
                child_bone_names_by_parent,
                visited_bone_names,
                bone_names,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loaders::{BoneJsonConfig, BoneTransformSpec};

    fn bone_transform_spec(vertex_index: u16) -> BoneTransformSpec {
        BoneTransformSpec {
            cube_name: None,
            strategy: "VERTEX".to_string(),
            vertex_indices: None,
            vertex_index: Some(vertex_index),
        }
    }

    fn bone_json_config(
        parent_bone_name: &str,
        head_vertex_index: u16,
        tail_vertex_index: u16,
        roll_radians: f32,
    ) -> BoneJsonConfig {
        BoneJsonConfig {
            parent: parent_bone_name.to_string(),
            head: bone_transform_spec(head_vertex_index),
            tail: bone_transform_spec(tail_vertex_index),
            roll: roll_radians,
        }
    }

    #[test]
    fn reference_bindposes_are_built_from_json_hierarchy_and_rolls() {
        let rig_config = RigConfigAsset {
            bones: [
                ("root", bone_json_config("", 0, 1, 0.0)),
                ("child", bone_json_config("root", 1, 2, 0.5)),
            ]
            .into_iter()
            .collect(),
            rig_name: "fixture".to_string(),
        };
        let helpers = [Vec3::ZERO, Vec3::Y, Vec3::Y * 2.0];
        let vertex_groups = AHashMap::default();

        let reference_rig =
            ReferenceRigAsset::from_rig_config(&rig_config, &helpers, &vertex_groups);

        assert_eq!(reference_rig.bone_names, ["root", "child"]);
        assert_eq!(reference_rig.bone_parents["root"], "Human.rig");
        assert_eq!(reference_rig.bone_name_to_index["root"], 0);
        assert_eq!(reference_rig.bone_name_to_index["child"], 1);
        assert!((reference_rig.local_bindpose["child"].translation - Vec3::Y).length() < 1e-6);
        assert!(
            reference_rig.local_bindpose["child"]
                .rotation
                .angle_between(Quat::from_rotation_y(0.5))
                < 1e-6
        );

    }

    #[test]
    fn default_rig_rolls_match_blender_bind_pose_samples() {
        let rig_config_bones: AHashMap<String, BoneJsonConfig> =
            serde_json::from_str(include_str!("../../assets/rigs/rig.default.json"))
                .expect("default rig JSON must parse");
        let rig_config = RigConfigAsset {
            bones: rig_config_bones
                .into_iter()
                .map(|(bone_name, bone_config)| {
                    (NAME_INTERNER.intern(&bone_name).leak(), bone_config)
                })
                .collect(),
            rig_name: "default".to_string(),
        };
        let helpers: Vec<Vec3> = include_str!("../../assets/base.obj")
            .lines()
            .filter_map(|vertex_line| vertex_line.strip_prefix("v "))
            .map(|vertex_line| {
                let mut position_components = vertex_line.split_whitespace().map(|component| {
                    component
                        .parse::<f32>()
                        .expect("base OBJ position components must be numeric")
                });
                Vec3::new(
                    position_components
                        .next()
                        .expect("base OBJ vertex must contain an x coordinate"),
                    position_components
                        .next()
                        .expect("base OBJ vertex must contain a y coordinate"),
                    position_components
                        .next()
                        .expect("base OBJ vertex must contain a z coordinate"),
                )
            })
            .collect();
        let vertex_groups: AHashMap<String, Vec<[usize; 2]>> = serde_json::from_str(
            include_str!("../../assets/basemesh_vertex_groups.json"),
        )
        .expect("default vertex groups must parse");
        let reference_rig =
            ReferenceRigAsset::from_rig_config(&rig_config, &helpers, &vertex_groups);
        // Local XYZW samples copied from the Blender default-rig bind-pose export.
        let blender_bind_pose_rotations = [
            (
                "pelvis.L",
                Quat::from_xyzw(-0.024314038, -0.16432232, -0.7264046, 0.6668908),
            ),
            (
                "upperleg01.L",
                Quat::from_xyzw(0.49055156, 0.62101144, 0.40624815, 0.45680025),
            ),
            (
                "upperleg02.L",
                Quat::from_xyzw(0.099648714, 0.02408164, -0.08383419, 0.9911922),
            ),
            (
                "spine02",
                Quat::from_xyzw(-0.16489373, 0.0, 0.0, 0.9863113),
            ),
            (
                "wrist.L",
                Quat::from_xyzw(-0.107333586, -0.6034186, -0.32568642, 0.71992636),
            ),
            (
                "toe1-1.L",
                Quat::from_xyzw(-0.021465117, 0.94842494, -0.22187611, 0.22538957),
            ),
        ];

        for (bone_name, blender_rotation) in blender_bind_pose_rotations {
            let rebuilt_rotation = reference_rig.local_bindpose[bone_name].rotation;
            assert!(
                rebuilt_rotation.angle_between(blender_rotation) < 0.0005,
                "bone '{bone_name}' local bind rotation differs from Blender: rebuilt={:?}, Blender={:?}",
                rebuilt_rotation.to_array(),
                blender_rotation.to_array(),
            );
        }
    }
}
