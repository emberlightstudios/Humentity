use bevy::{animation::{AnimationTarget, AnimationTargetId}, asset::RenderAssetUsages, mesh::{morph::{self, MorphAttributes, MorphTargetImage}, skinning::{SkinnedMesh, SkinnedMeshInverseBindposes}, PrimitiveTopology}, prelude::*};
use crate::{animation::get_skeleton_rotations, basemesh::VertexGroups, mesh_ops::{get_uv_coords, get_vertex_normals, get_vertex_positions, get_vertex_tangents, MeshProcessingState}, morphs::{self, adjust_helpers_to_morphs}, prelude::*, rigs::{get_bone_order, set_basemesh_rig_arrays, BoneData, RigData}};
use ahash::{AHashMap};

/// In order to dynamically reshape humans at runtime, we can define a HumanArchetype which is a mesh 
/// cached from a given set of MorphTargets.  Archetypes are added as new distinct shapekeys to the base 
/// mesh, and the rest of the makehuman shapekeys are removed.  Use this for distinct faces or body types.
/// You can also blend between them, since they are just shapekeys.
pub struct HumanShapeArchetype {
    pub name: Name,
    pub morphs: MorphTargets,
}

impl HumanShapeArchetype {
    pub fn new(name: Name, morphs: MorphTargets) -> Self {
        Self { name, morphs }
    }
}

/// Encapsulates all the animation properties and cached data associated with an archetype/prefab.
#[derive(Default)]
pub struct HumanAnimationArchetype {
    pub animations: AHashMap<Name, Handle<AnimationClip>>,
    pub animation_glbs: Vec<Name>,
    pub rig_type: RigType,
    pub(crate) scene: Option<Handle<DynamicScene>>,
    pub(crate) bone_order: Vec<Name>,
    pub(crate) bones: AHashMap<Name, BoneData>,
}

impl HumanAnimationArchetype {
    pub fn new(rig_type: RigType, animation_glbs: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut instance = Self::default();
        instance.rig_type = rig_type;
        instance.animation_glbs = animation_glbs
            .into_iter()
            .map(|g| Name::new(g.as_ref().to_string()))
            .collect::<Vec<_>>();
        instance
    }
}

/// A collection of base shapes and animation properties.  The shapes will be baked into a
/// new Mesh as morph targets.
#[derive(Default)]
pub struct HumanArchetypePrefab {
    pub shapes: Vec<HumanShapeArchetype>,
    pub rig: HumanAnimationArchetype,
}

impl HumanArchetypePrefab {
    pub fn new(shapes: impl IntoIterator<Item = HumanShapeArchetype>, rig: HumanAnimationArchetype) -> Self {
        Self { shapes: shapes.into_iter().collect(), rig }
    }

    pub(crate) fn get_helpers(&self, morph_values: &MorphTargets, basemesh: &BaseMesh, morph_targets: &HumanMorphs) -> Vec<Vec3> {
        let mut mh_morphs = MorphTargets::default();
        for shape in self.shapes.iter() {
            let Some(weight) = morph_values.get(&shape.name) else { continue };
            for (k, v) in shape.morphs.iter() {
                let entry = mh_morphs.entry(k.clone()).or_insert(0.);
                *entry += *v * weight;
            }
        }
        adjust_helpers_to_morphs(&mh_morphs, morph_targets, basemesh)
    }
}

/*-----------+
 | Resources |
 +-----------*/
#[derive(Resource, Default, Deref, DerefMut)]
pub struct HumanArchetypePrefabs(AHashMap<Name, HumanArchetypePrefab>);

impl HumanArchetypePrefabs {
    pub fn new(prefabs: impl IntoIterator<Item = (Name, HumanArchetypePrefab)>) -> Self {
        Self(prefabs.into_iter().collect::<AHashMap<Name, HumanArchetypePrefab>>())
    }
}

/*---------+
 | Systems |
 +---------*/
pub(crate) fn create_basemesh_prefab_shapes(
    prefabs: ResMut<HumanArchetypePrefabs>,
    mut basemesh: ResMut<BaseMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    morphs: Res<HumanMorphs>,
) {
    // Only run if prefabs not added to basemesh
    if !basemesh.prefab_state.is_empty() { return }

    for (name, prefab) in prefabs.iter() {
        let mut prefab_meshes = vec![];
        for shape in prefab.shapes.iter() {
            let helpers = morphs::adjust_helpers_to_morphs(&shape.morphs, &*morphs, &*basemesh);
            let mesh = meshes.get(&basemesh.mesh_handle).unwrap().clone();
            let mut positions = get_vertex_positions(&mesh);
            for vtx in 0..positions.len() {
                let mhid = basemesh.mhid_lookup[vtx];
                positions[vtx] = helpers[mhid as usize];
            }

            let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, get_uv_coords(&mesh))
                .with_inserted_indices(mesh.indices().unwrap().clone())
                .with_computed_area_weighted_normals()
                .with_generated_tangents()
                .unwrap();

            let handle = meshes.add(mesh);
            prefab_meshes.push(handle);
        }
        basemesh.prefab_state.insert(name.clone(), MeshProcessingState::Shaped(prefab_meshes));
    }
}

pub(crate) fn create_basemesh_prefab_morphable_mesh(
    prefabs: ResMut<HumanArchetypePrefabs>,
    mut basemesh: ResMut<BaseMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Only run if shaped meshes have been generated
    for (name, _) in prefabs.iter() {
        match &basemesh.prefab_state[name] {
            MeshProcessingState::Shaped(shape_meshes) => {
                for shape in shape_meshes.iter() {
                    if meshes.get(shape).is_none() { return }
                }
            }
            _ => return
        }
    }
    let basemesh_mesh = meshes.get(&basemesh.mesh_handle).unwrap().clone();

    for (name, prefab) in prefabs.iter() {
        let MeshProcessingState::Shaped(shaped_meshes) = &basemesh.prefab_state[name] 
            else { unimplemented!("This should not happen") };
        let mut morphs = vec![];
        let mut morph_names = vec![];

        let base_positions = get_vertex_positions(&basemesh_mesh);
        let base_normals = get_vertex_normals(&basemesh_mesh);
        let base_tangents = get_vertex_tangents(&basemesh_mesh)
            .expect("Base mesh should always have tangents at this point.");

        for (is, shape) in prefab.shapes.iter().enumerate() {
            let mut morph = Vec::<MorphAttributes>::new();
            let shape_mesh = meshes.get(&shaped_meshes[is]).unwrap();
            let shape_positions = get_vertex_positions(&shape_mesh);
            let shape_normals = get_vertex_normals(&shape_mesh);
            let shape_tangents = get_vertex_tangents(&shape_mesh)
                .expect("Base mesh should always have tangents at this point.");

            for vtx in 0..base_positions.len() {
                if (shape_positions[vtx] - base_positions[vtx]).length_squared() > 1e-6 || 
                   (  shape_normals[vtx] - base_normals[vtx]  ).length_squared() > 1e-6 || 
                   ( shape_tangents[vtx] - base_tangents[vtx] ).length_squared() > 1e-6 {

                    morph.push(MorphAttributes::from([
                        shape_positions[vtx] - base_positions[vtx],
                        shape_normals[vtx] - base_normals[vtx],
                        shape_tangents[vtx] - base_tangents[vtx],
                    ]));
                }
            }

            morph_names.push(String::from(&shape.name));
            morphs.push(morph.into_iter());
        }

        let image = MorphTargetImage::new(
            morphs.into_iter(), base_positions.len(), RenderAssetUsages::default()
        ).expect("failed to create morph target image");

        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, get_vertex_positions(&basemesh_mesh))
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, get_uv_coords(&basemesh_mesh))
            .with_inserted_indices(basemesh_mesh.indices().unwrap().clone())
            .with_computed_area_weighted_normals()
            .with_morph_targets(images.add(image.0))
            .with_morph_target_names(morph_names)
            .with_generated_tangents().unwrap();

        basemesh.prefab_state.insert(name.clone(), MeshProcessingState::Morphed(meshes.add(mesh)));
    }
}

pub fn create_human_prefab_rig_scenes(world: &mut World) {
    // Only run if prefab rig scenes are None
    let prefabs = world.get_resource::<HumanArchetypePrefabs>()
        .expect("No human prefabs resource found");
    for (_, prefab) in prefabs.iter() {
        if prefab.rig.scene.is_some() { return }
    }

    let prefab_data = prefabs
        .iter()
        .map(|(n, p)| (n.clone(), p.rig.rig_type))
        .collect::<Vec<_>>();
    
    for (name, rig_type) in prefab_data {
        let bone_rotations = get_skeleton_rotations(world, rig_type)
            .expect("Failed to get skeleton rotations from glb file");

        let bone_order = get_bone_order(world, rig_type);
        let base_mesh = world.get_resource::<BaseMesh>().unwrap();
        let helpers = &base_mesh.vertices.clone();
        let (scene, bones) = build_human_rig_scene(
            &helpers, rig_type, &bone_rotations, &bone_order, world
        );
        
        let mut prefabs = world.resource_mut::<HumanArchetypePrefabs>();
        let prefab = prefabs.get_mut(&name).unwrap();
        prefab.rig.scene = Some(scene);
        prefab.rig.bones = bones;
        prefab.rig.bone_order = bone_order.clone();
    }
}

pub(crate) fn rig_prefab_meshes(
    mut basemesh: ResMut<BaseMesh>,
    prefabs: Res<HumanArchetypePrefabs>,
    rig_data: Res<RigData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    // Only run if basemesh has had morph targets added
    for (_, state) in basemesh.prefab_state.iter() {
        let MeshProcessingState::Morphed(_) = state else { return };
    }

    for (name, prefab) in prefabs.iter() {
        let MeshProcessingState::Morphed(mesh_handle) = &basemesh.prefab_state[name]
            else { unimplemented!("This should not happen") };
        let handle = set_basemesh_rig_arrays(
            meshes.get(mesh_handle).unwrap().clone(),
            &*basemesh,
            &mut *meshes,
            &prefab.rig.bone_order,
            prefab.rig.rig_type,
            &*rig_data
        );
        basemesh.prefab_state.insert(name.clone(), MeshProcessingState::Ready(handle));
    }

    commands.set_state(HumentityLoadState::RetargetingAnimations);
}

/// Spawns bone entities and sets up the hierarchy
pub(crate) fn build_human_rig_scene(
    helpers: &Vec<Vec3>,
    rig: RigType,
    bone_rotations: &AHashMap<Name, Quat>,
    bone_order: &Vec<Name>,
    world: &mut World
) -> (Handle<DynamicScene>, AHashMap<Name, BoneData>) {
    let mh_config = &world.resource::<RigData>().configs[&rig];

    // Set up some convenient data structures for tracking joints/bones and entities
    let mut bone_entities = AHashMap::<Name, Entity>::default();

    // Start scene world with rig entity
    let registry = world.resource::<AppTypeRegistry>();
    let mut scene_world = World::new();
    scene_world.insert_resource(registry.clone());
    let rig_entity = scene_world.spawn((
        AnimationPlayer::default(),
        Name::new("Human.rig"),
        Transform::IDENTITY,
    )).id();

    // Spawn all bone entities
    for name in bone_order.iter() {
        let mut path = Vec::<Name>::new();
        path.push(name.clone());
        let mut bone = &mh_config[name];

        while !bone.parent.is_empty() {
            path.push(bone.parent.clone());
            bone = &mh_config[&bone.parent];
        }
        path.push(Name::new("Human.rig"));

        let entity = scene_world.spawn((
            name.clone(),
            AnimationTarget {
                id: AnimationTargetId::from_names(path.iter().rev()),
                player: rig_entity,
            },
        )).id();
        bone_entities.insert(name.clone(), entity);
    }

    // Wire up parent-child relationships
    for name in bone_order.iter() {
        let &child = bone_entities.get(&name).unwrap();
        if let Some(parent_name) = mh_config.get(&name).map(|b| b.parent.to_string()) {
            if !parent_name.is_empty() {
                if let Some(&parent) = bone_entities.get(&Name::new(parent_name)) {
                    scene_world.entity_mut(child).insert(ChildOf(parent));
                }
            }
        }
    }

    // Attach root(s) to rig entity
    for name in bone_order.iter() {
        if let Some(bone) = mh_config.get(&name) {
            if bone.parent.is_empty() {
                scene_world.entity_mut(bone_entities[&name]).insert(ChildOf(rig_entity));
            }
        }
    }

    let vg = world.resource::<VertexGroups>();
    let rig_data = world.resource::<RigData>();
    let global_transforms = crate::rigs::get_model_space_skeleton_transforms(
        bone_order, helpers, rig, bone_rotations, &vg, &rig_data);
    let local_transforms = crate::rigs::get_local_skeleton_transforms(
        bone_order, rig, &rig_data, &global_transforms);

    // compute inverse bindposes
    let mut inverse_bindposes = Vec::with_capacity(bone_order.len());
    for name in bone_order.iter() {
        let entity = bone_entities[&name];
        let local = local_transforms[&name];
        let global = global_transforms[&name];

        scene_world.entity_mut(entity).insert(local);

        // Inverse bindpose is always from global transform
        inverse_bindposes.push(global.to_matrix().inverse());
    }

    // Cache bone data in RigConfig struct for retargeting and runtime skinning
    let mut bones = AHashMap::<Name, BoneData>::default();
    for bone in bone_order.iter() {
        bones.insert(bone.clone(), BoneData {
            local_space_transform: local_transforms[&bone],
            model_space_transform: global_transforms[&bone],
            parent: mh_config[&bone].parent.clone(),
        });
    }

    // Setup SkinnedMesh component and AnimationPlayer
    let mut inverse_bindpose_assets = world.resource_mut::<Assets<SkinnedMeshInverseBindposes>>();
    let inverse_bindposes = inverse_bindpose_assets.add(inverse_bindposes);
    // only need to return bindposes.  entities will have to be mapped manually after spawning scene
    let joint_entities = bone_order
        .iter()
        .map(|n| bone_entities[n])
        .collect::<Vec<_>>();

    let skinned_mesh = SkinnedMesh {
        inverse_bindposes,
        joints: joint_entities,
    };
    scene_world.entity_mut(rig_entity).insert(skinned_mesh);

    let mut ds = world.resource_mut::<Assets<DynamicScene>>();
    (
        ds.add(DynamicScene::from_world(&scene_world)),
        bones,
    )
}