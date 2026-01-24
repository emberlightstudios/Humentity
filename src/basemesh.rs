use ahash::AHashMap;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{
        morph::{MorphAttributes, MorphTargetImage},
        Indices, Mesh, PrimitiveTopology,
    },
    prelude::*,
};
use serde::Deserialize;
use smallvec::SmallVec;
use std::{fs::File, io::BufReader};

use crate::{mesh_ops::{
    MeshProcessingState, PrefabLoadState, fix_normals, generate_mhid_lookup, generate_vertex_map, get_uv_coords, get_vertex_normals, get_vertex_positions, get_vertex_tangents, parse_obj_vertices
}, rigs::SkeletonCache};
use crate::prelude::*;

pub(crate) const BODY_VERTICES: u16 = 13380u16;
pub(crate) const BODY_SCALE: f32 = 0.1;

/*-------------+
|  Resources  |
+-------------*/
#[derive(Resource, Deserialize, Debug)]
pub(crate) struct VertexGroups(pub(crate) AHashMap<String, Vec<[usize; 2]>>);

#[derive(Resource)]
pub struct BaseMesh {
    /// The prefab mesh loading state
    pub prefab_state: PrefabLoadState,
    /// A handle to the raw base mesh
    pub(crate) mesh_handle: Handle<Mesh>,
    /// The (makehuman/obj) positions in the base mesh
    pub(crate) vertices: Vec<Vec3>,
    /// A map from bevy indices to obj indices
    pub(crate) mhid_lookup: Vec<u16>,
}

#[derive(Resource, Debug, Clone)]
pub(crate) struct HelperMeshHandle(Handle<Mesh>);

// Load base mesh with helpers and vertex group data
impl FromWorld for BaseMesh {
    fn from_world(world: &mut World) -> Self {
        let config = world
            .get_resource::<HumentityPathsConfig>()
            .expect("NO CONFIG LOADED");
        let path = config.core_assets_path.clone();
        
        if !path.join("base.obj").exists() {
            panic!("Path {path:#?} not valid. base.obj not found.  Did you provide the correct path to the Humentity crate?")
        }
        // Get mh vertices from base mesh and helper files
        let mh_vertices = parse_obj_vertices(path.join("base.obj"));

        // Load obj into asset server
        let asset_server = world.resource::<AssetServer>();
        let base_handle: Handle<Mesh> = asset_server.load("humentity://base.obj");

        let file = File::open(path.join("basemesh_vertex_groups.json"))
            .expect("FAILED TO LOAD VERTEX GROUOPS");
        let reader = BufReader::new(file);
        let vg: VertexGroups = serde_json::from_reader(reader).unwrap();

        world.insert_resource(vg);
        world.insert_resource(HelperMeshHandle(base_handle.clone()));

        BaseMesh {
            mesh_handle: base_handle,
            vertices: mh_vertices,
            mhid_lookup: vec![],
            prefab_state: PrefabLoadState::default(),
        }
    }
}

impl BaseMesh {
    pub(crate) fn get_rigged_mesh_handle(
        &mut self,
        prefab_name: &'static str,
        prefab: &mut CharacterArchetypePrefab,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
        rig_data: &crate::rigs::RigData,
        mh_morphs: &MakeHumanMorphs,
        cache: &SkeletonCache,
    ) -> Option<Handle<Mesh>> {
        self.prefab_state.entry(prefab_name).or_default();

        match &self.prefab_state[prefab_name] {
            MeshProcessingState::Unprocessed => {
                self.create_prefab_shapes(prefab, prefab_name, meshes, mh_morphs);
                None
            }
            MeshProcessingState::Shaped(_handles) => {
                self.create_prefab_morphable_mesh(prefab, prefab_name, meshes, images);
                None
            }
            MeshProcessingState::Morphed(_) => {
                self.rig_prefab_meshes(prefab, prefab_name, rig_data, meshes, cache);
                None
            }
            MeshProcessingState::Ready(handle) => Some(handle.clone()),
            _ => unimplemented!("Should not be here"),
        }
    }

    pub(crate) fn create_prefab_shapes(
        &mut self,
        prefab: &mut CharacterArchetypePrefab,
        prefab_name: &'static str,
        meshes: &mut Assets<Mesh>,
        morphs: &MakeHumanMorphs,
    ) {
        if self.prefab_state[prefab_name] != MeshProcessingState::Unprocessed {
            return;
        }
        let mut prefab_meshes = vec![];
        let mut heights = SmallVec::<[f32; 8]>::new();
        for shape in prefab.shapes.iter() {
            let helpers = crate::morphs::adjust_helpers_to_morphs(&shape.morphs, morphs, self);
            let mesh = meshes.get(&self.mesh_handle).unwrap().clone();
            let mut positions = get_vertex_positions(&mesh);

            #[allow(clippy::needless_range_loop)]
            for vtx in 0..positions.len() {
                let mhid = self.mhid_lookup[vtx];
                positions[vtx] = helpers[mhid as usize];
            }

            heights.push(f32::max(
                shape.height,
                positions.iter().map(|v| v.y).reduce(f32::max).unwrap(),
            ));

            let mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, get_uv_coords(&mesh))
            .with_inserted_indices(mesh.indices().unwrap().clone())
            .with_computed_area_weighted_normals()
            .with_generated_tangents()
            .unwrap();

            let handle = meshes.add(mesh);
            prefab_meshes.push(handle);
        }
        self.prefab_state
            .insert(prefab_name, MeshProcessingState::Shaped(prefab_meshes));

        for i_shape in 0..heights.len() {
            let shape = &mut prefab.shapes[i_shape];
            shape.height = heights[i_shape];
        }
    }

    pub(crate) fn create_prefab_morphable_mesh(
        &mut self,
        prefab: &CharacterArchetypePrefab,
        prefab_name: &'static str,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
    ) {
        let mesh = meshes.get(&self.mesh_handle).unwrap().clone();

        if !matches!(
            self.prefab_state[prefab_name],
            MeshProcessingState::Shaped(_)
        ) {
            return;
        }
        let MeshProcessingState::Shaped(shaped_meshes) = &self.prefab_state[prefab_name] else {
            unimplemented!("This should not happen")
        };
        let mut morphs = vec![];
        let mut morph_names = vec![];

        let base_positions = crate::mesh_ops::get_vertex_positions(&mesh);
        let base_normals = crate::mesh_ops::get_vertex_normals(&mesh);
        let base_tangents = crate::mesh_ops::get_vertex_tangents(&mesh)
            .expect("Base mesh should always have tangents at this point.");

        for (is, shape) in prefab.shapes.iter().enumerate() {
            let mut morph = Vec::<MorphAttributes>::new();
            let shape_mesh = meshes.get(&shaped_meshes[is]).unwrap();
            let shape_positions = get_vertex_positions(shape_mesh);
            let shape_normals = get_vertex_normals(shape_mesh);
            let shape_tangents = get_vertex_tangents(shape_mesh)
                .expect("Base mesh should always have tangents at this point.");

            for vtx in 0..base_positions.len() {
                if (shape_positions[vtx] - base_positions[vtx]).length_squared() > 1e-6
                    || (shape_normals[vtx] - base_normals[vtx]).length_squared() > 1e-6
                    || (shape_tangents[vtx] - base_tangents[vtx]).length_squared() > 1e-6
                {
                    morph.push(MorphAttributes::from([
                        shape_positions[vtx] - base_positions[vtx],
                        shape_normals[vtx] - base_normals[vtx],
                        shape_tangents[vtx] - base_tangents[vtx],
                    ]));
                }
            }

            morph_names.push(shape.name.clone());
            morphs.push(morph.into_iter());
        }

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, get_vertex_positions(&mesh))
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, get_uv_coords(&mesh))
        .with_inserted_indices(mesh.indices().unwrap().clone())
        .with_computed_area_weighted_normals()
        .with_generated_tangents()
        .unwrap();
        fix_normals(&mut mesh, &self.mhid_lookup);

        if !morphs.is_empty() {
            let image = MorphTargetImage::new(
                morphs.into_iter(),
                base_positions.len(),
                RenderAssetUsages::default(),
            )
            .expect("failed to create morph target image");

            mesh = mesh
                .with_morph_targets(images.add(image.0))
                .with_morph_target_names(morph_names)
        }

        self.prefab_state
            .insert(prefab_name, MeshProcessingState::Morphed(meshes.add(mesh)));
    }

    pub(crate) fn rig_prefab_meshes(
        &mut self,
        prefab: &CharacterArchetypePrefab,
        prefab_name: &'static str,
        rig_data: &crate::rigs::RigData,
        meshes: &mut Assets<Mesh>,
        cache: &SkeletonCache,
    ) {
        if !matches!(
            self.prefab_state[prefab_name],
            MeshProcessingState::Morphed(_)
        ) {
            return;
        }
        let MeshProcessingState::Morphed(mesh_handle) = &self.prefab_state[prefab_name] else {
            unimplemented!("This should not happen")
        };
        let rig_type = prefab.rig.rig_type;

        let mesh = crate::rigs::set_basemesh_rig_arrays(
            meshes.get(mesh_handle).unwrap().clone(),
            self,
            &cache.bone_order,
            rig_type,
            rig_data,
        );
        self.prefab_state
            .insert(prefab_name, MeshProcessingState::Ready(meshes.add(mesh)));
    }
}

// Remove helper vertices to generate body only mesh
pub(crate) fn create_body_mesh(
    mut base_mesh: ResMut<BaseMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    helper_handle: Option<Res<HelperMeshHandle>>,
    mut state: ResMut<NextState<HumentityLoadState>>,
) {
    let Some(helper_handle) = helper_handle else {
        return;
    };
    let Some(mesh) = meshes.get_mut(&helper_handle.0) else {
        return;
    };

    // Get mesh arrays
    let raw_indices = mesh.indices().expect("FAILED TO LOAD MESH INDICES");
    let vtx_data = get_vertex_positions(mesh);
    let uv_data = get_uv_coords(mesh);
    let vertex_map = generate_vertex_map(&base_mesh.vertices, &vtx_data);
    let mhid_lookup = generate_mhid_lookup(&vertex_map);

    // Create mesh without helpers
    let mesh = generate_mesh_without_helpers(&mhid_lookup, vtx_data, uv_data, raw_indices);

    let vtx_data = get_vertex_positions(&mesh);
    let vertex_map = generate_vertex_map(&base_mesh.vertices[..BODY_VERTICES as usize], &vtx_data);

    // Save values in base mesh resource
    base_mesh.mesh_handle = meshes.add(mesh);
    base_mesh.mhid_lookup = generate_mhid_lookup(&vertex_map);

    commands.remove_resource::<HelperMeshHandle>();
    state.set(HumentityLoadState::BuildingPrefabs)
}

fn generate_mesh_without_helpers(
    mhid_lookup: &[u16],
    vtx_data: Vec<Vec3>,
    uv_data: Vec<Vec2>,
    indices_data: &Indices,
) -> Mesh {
    // Final buffers
    let mut vertices = Vec::<Vec3>::new();
    let mut uv = Vec::<Vec2>::new();

    // Mapping old vertex index -> new vertex index
    let mut new_vert_indices = AHashMap::<u16, u16>::default();

    // Build new vertex buffer, and UVs
    for (vertex, &mh_id) in mhid_lookup.iter().enumerate() {
        if mh_id < BODY_VERTICES {
            new_vert_indices.insert(vertex as u16, vertices.len() as u16);
            vertices.push(vtx_data[vertex]);
            uv.push(uv_data[vertex]);
        }
    }

    // Build new index buffer
    let indices_data: &Vec<u16> = match indices_data {
        Indices::U16(indices_data) => indices_data,
        Indices::U32(indices_data) => &indices_data.iter().map(|&i| i as u16).collect(),
    };
    let mut new_indices = Vec::<u16>::new();
    for chunk in indices_data.iter().as_slice().chunks(3) {
        // Only include triangles where all vertices exist in new_vert_indices
        if chunk.iter().all(|x| new_vert_indices.contains_key(x)) {
            new_indices.extend(chunk.iter().map(|x| new_vert_indices[x]));
        }
    }

    // Create the new mesh
    

    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    .with_inserted_indices(Indices::U16(new_indices))
    .with_computed_area_weighted_normals()
    .with_generated_tangents()
    .unwrap()
}
