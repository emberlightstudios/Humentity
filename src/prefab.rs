use bevy::{animation::{AnimationTarget, AnimationTargetId}, ecs::intern::Internable, mesh::{skinning::{SkinnedMesh, SkinnedMeshInverseBindposes}}, prelude::*};
use crate::{animation::get_skeleton_rotations, basemesh::VertexGroups, mesh_ops::{MeshProcessingState}, morphs::adjust_helpers_to_morphs, prelude::*, rigs::{get_bone_order, RigData}};
use ahash::{AHashMap};

/// In order to dynamically reshape humans at runtime, we can define a CharacterArchetype which is a mesh 
/// cached from a given set of MorphTargets.  Archetypes are added as new distinct shapekeys to the base 
/// mesh, and the rest of the makehuman shapekeys are removed.  Use this for distinct faces or body types.
/// You can also blend between them, since they are just shapekeys.
pub struct CharacterShapeArchetype {
    pub name: &'static str,
    pub morphs: MorphTargets,
}

impl CharacterShapeArchetype {
    pub fn new(name: &'static str, morphs: MorphTargets) -> Self {
        Self { name, morphs }
    }
}

/// Encapsulates all the animation properties and cached data associated with an archetype/prefab.
#[derive(Default)]
pub struct CharacterAnimationArchetype {
    pub animations: AHashMap<&'static str, Handle<AnimationClip>>,
    pub animation_glbs: Vec<&'static str>,
    pub rig_type: RigType,
    pub(crate) scene: Option<Handle<DynamicScene>>,
    pub(crate) bone_order: Vec<&'static str>,
    pub(crate) bone_rotations: AHashMap<&'static str, Quat>,
}

impl CharacterAnimationArchetype {
    pub fn new(rig_type: RigType, animation_glbs: impl IntoIterator<Item = &'static str>) -> Self {
        let mut instance = Self::default();
        instance.rig_type = rig_type;
        instance.animation_glbs = animation_glbs
            .into_iter()
            .collect::<Vec<_>>();
        instance
    }
}

/// A collection of base shapes and animation properties.  The shapes will be baked into a
/// new Mesh as morph targets.
#[derive(Default)]
pub struct CharacterArchetypePrefab {
    pub shapes: Vec<CharacterShapeArchetype>,
    pub rig: CharacterAnimationArchetype,
}

impl CharacterArchetypePrefab {
    pub fn new(shapes: impl IntoIterator<Item = CharacterShapeArchetype>, rig: CharacterAnimationArchetype) -> Self {
        Self { shapes: shapes.into_iter().collect(), rig }
    }

    pub(crate) fn get_helpers(&self, morph_values: &MorphTargets, basemesh: &BaseMesh, morph_targets: &MakeHumanMorphs) -> Vec<Vec3> {
        let mut mh_morphs = MorphTargets::default();
        for shape in self.shapes.iter() {
            let Some(weight) = morph_values.get(&shape.name) else { continue };
            for (&k, v) in shape.morphs.iter() {
                let entry = mh_morphs.entry(k).or_insert(0.);
                *entry += *v * weight;
            }
        }
        adjust_helpers_to_morphs(&mh_morphs, morph_targets, basemesh)
    }
}

/*--------+
 | Events |
 +--------*/
 /// Use this to modify prefab shapes
 /// The systems below will update mesh handles
 #[derive(Event, Clone)]
 pub struct ModifyPrefabShape {
    pub morphs: MorphTargets,
    pub prefab_name: &'static str,
    pub shape_name: &'static str,
    pub parts: Vec<CharacterPart>,
 }

/*-----------+
 | Resources +|
 +-----------*/
#[derive(Resource, Deref, DerefMut, Default)]
pub struct CharacterArchetypePrefabs(AHashMap<&'static str, CharacterArchetypePrefab>);

impl CharacterArchetypePrefabs {
    pub fn new(prefabs: impl IntoIterator<Item = (&'static str, CharacterArchetypePrefab)>) -> Self {
        Self(prefabs.into_iter().collect::<AHashMap<&'static str, CharacterArchetypePrefab>>())
    }

    pub fn basemesh() -> Self {
        let mut prefabs = AHashMap::default();
        prefabs.insert("", CharacterArchetypePrefab::default());
        Self(prefabs)
    }
}

/// Tracks modified shapes with pending mesh updates
#[derive(Resource, Deref, DerefMut, Default)]
pub(crate) struct ArchetypeShapeUpdate(Vec::<ModifyPrefabShape>);

/*---------+
 | Systems |
 +---------*/
/// Monitor pending shape changes in asset meshes
pub(crate) fn update_asset_shapes(
    mut shape_updates: ResMut<ArchetypeShapeUpdate>,
    mut assets: ResMut<CharacterAssetRegistry>,
    mut basemesh: ResMut<BaseMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut asset_server: ResMut<AssetServer>,
    mh_morphs: Res<MakeHumanMorphs>,
    rig_data: Res<RigData>,
    prefabs: Res<CharacterArchetypePrefabs>,
    paths: Res<HumentityPathsConfig>,
    mut commands: Commands,
) {
    let mut done = vec![];

    for (i, shape_mod) in shape_updates.iter().enumerate() {
        let prefab_name = shape_mod.prefab_name;
        let prefab = &prefabs[prefab_name];
        let mut finished = true;

        for part in shape_mod.parts.iter() {
            match part {
                CharacterPart::BaseMesh => {
                    if basemesh.get_rigged_mesh_handle(prefab_name, prefab, &mut *meshes,
                            &mut *images, &*rig_data, &*mh_morphs).is_none()
                    {
                        finished = false;
                    } else {}
                },
                CharacterPart::BodyPart(_) |
                CharacterPart::Equipment(_) |
                CharacterPart::ProxyMesh(_) => {
                    let asset = assets.assets.get_mut(part).unwrap();
                    if asset.get_rigged_mesh_handle(&mut *asset_server, prefab_name, &prefab, &*rig_data,
                            &*basemesh, &*mh_morphs, &*paths, &mut *meshes, &mut *images).is_none()
                    {
                        finished = false;
                    }
                }
            }
        }

        if finished { done.push(i) }
    }

    for i in done.iter().rev() {
        shape_updates.remove(*i);
    }
    if shape_updates.is_empty() {
        commands.remove_resource::<ArchetypeShapeUpdate>();
    }
}

/// Handle events to modify prefab shapes
pub(crate) fn on_prefab_shape_modified(
    trigger: On<ModifyPrefabShape>,
    mut prefabs: ResMut<CharacterArchetypePrefabs>,
    mut assets: ResMut<CharacterAssetRegistry>,
    mut basemesh: ResMut<BaseMesh>,
    shape_updates: Option<ResMut<ArchetypeShapeUpdate>>,
    mut commands: Commands,
) {
    let ModifyPrefabShape{ morphs, prefab_name, shape_name, parts } = trigger.event();
    let prefab_name = *prefab_name;
    let shape_name = *shape_name;

    let Some(prefab) = prefabs.get_mut(prefab_name) 
        else {
            error!("No such prefab to modify: {}", prefab_name);
            return;
        };
    let Some(shape_index) = prefab.shapes.iter().position(|s| s.name == shape_name)
        else {
            error!("No such shape to modify {} on prefab {}", shape_name, prefab_name);
            return;
        };

    let shape = prefab.shapes.get_mut(shape_index).unwrap();
    shape.morphs = morphs.clone();

    if let Some(mut shape_updates) = shape_updates {
        if let Some(index) = shape_updates
            .iter()
            .position(|v| v.prefab_name == prefab_name && v.shape_name == shape_name)
        {
            shape_updates[index] = trigger.event().clone();
        } else {
            shape_updates.push(trigger.event().clone());
        }
    } else {
        let mut shape_updates = ArchetypeShapeUpdate::default();
        shape_updates.push(trigger.event().clone());
        commands.insert_resource(shape_updates);
    }

    for part in parts {
        match part {
            CharacterPart::BaseMesh => {
                basemesh.prefab_state.insert(prefab_name, MeshProcessingState::Unprocessed);
            },
            CharacterPart::BodyPart(name) |
            CharacterPart::Equipment(name) |
            CharacterPart::ProxyMesh(name) => {
                let Some(asset) = assets.assets.get_mut(part)
                    else {
                        error!("No registered asset called {}", name);
                        return;
                    };
                if let Some(ref mut data) = &mut asset.data {
                    data.prefab_load_state.insert(name, MeshProcessingState::Unprocessed);
                }
                // If the asset isn't loaded then it shouldn't have been passed in the message.
                // I'm not going to force loading now.
            }
        }
    }

}

pub(crate) fn create_human_prefab_rig_scenes(world: &mut World) {
    // Only run if prefab rig scenes are None
    let prefabs = world.get_resource::<CharacterArchetypePrefabs>()
        .expect("No human prefabs resource found");
    for (_, prefab) in prefabs.iter() {
        if prefab.rig.scene.is_some() { return }
    }

    let prefab_data = prefabs
        .iter()
        .map(|(&n, p)| (n, p.rig.rig_type))
        .collect::<Vec<_>>();
    
    for (name, rig_type) in prefab_data {
        let bone_rotations = get_skeleton_rotations(world, rig_type)
            .expect("Failed to get skeleton rotations from glb file");

        let bone_order = get_bone_order(world, rig_type);
        let base_mesh = world.get_resource::<BaseMesh>().unwrap();
        let helpers = &base_mesh.vertices.clone();
        let scene = build_human_rig_scene(
            &helpers, rig_type, &bone_rotations, &bone_order, world
        );
        
        let mut prefabs = world.resource_mut::<CharacterArchetypePrefabs>();
        let prefab = prefabs.get_mut(&name).unwrap();
        prefab.rig.scene = Some(scene);
        prefab.rig.bone_order = bone_order.clone();
        prefab.rig.bone_rotations = bone_rotations;
    }
    let mut state = world.resource_mut::<NextState<HumentityLoadState>>();
    state.set(HumentityLoadState::AnimationProcessing);
}


/// Spawns bone entities and sets up the hierarchy
pub(crate) fn build_human_rig_scene(
    helpers: &Vec<Vec3>,
    rig: RigType,
    bone_rotations: &AHashMap<&'static str, Quat>,
    bone_order: &Vec<&'static str>,
    world: &mut World
) -> Handle<DynamicScene> {
    let mh_config = &world.resource::<RigData>().configs[&rig];

    // Set up some convenient data structures for tracking joints/bones and entities
    let mut bone_entities = AHashMap::<&'static str, Entity>::default();

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
    for &name in bone_order.iter() {
        let mut path = Vec::<Name>::new();
        path.push(Name::from(name));
        let mut bone = &mh_config[&name];

        while !bone.parent.is_empty() {
            let parent = NAME_INTERNER.intern(&bone.parent).leak();
            path.push(Name::new(parent));
            bone = &mh_config[&parent];
        }
        path.push(Name::new("Human.rig"));

        let entity = scene_world.spawn((
            Name::new(name),
            AnimationTarget {
                id: AnimationTargetId::from_names(path.iter().rev()),
                player: rig_entity,
            },
        )).id();

        let parent = mh_config[name].parent;
        if parent != "" {
            let parent = bone_entities[parent];
            scene_world.entity_mut(entity).insert(ParentBone(parent));
        }

        bone_entities.insert(name, entity);
    }

    // Wire up parent-child relationships
    for &name in bone_order.iter() {
        let &child = bone_entities.get(&name).unwrap();
        if let Some(parent_name) = mh_config.get(&name).map(|b| b.parent.to_string()) {
            if !parent_name.is_empty() {
                if let Some(&parent) = bone_entities.get(NAME_INTERNER.intern(&parent_name).leak()) {
                    scene_world.entity_mut(child).insert(ChildOf(parent));
                }
            }
        }
    }

    // Attach root(s) to rig entity
    for &name in bone_order.iter() {
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
    for &name in bone_order.iter() {
        let entity = bone_entities[&name];
        let local = local_transforms[&name];
        let global = global_transforms[&name];

        scene_world.entity_mut(entity).insert(local);

        // Inverse bindpose is always from global transform
        inverse_bindposes.push(global.to_matrix().inverse());
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
    ds.add(DynamicScene::from_world(&scene_world))
}