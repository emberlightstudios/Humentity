use bevy::{ecs::intern::Internable, mesh::morph::{MorphAttributes, MorphTargetImage}};
use::bevy::{
    prelude::*,
    mesh::{
        PrimitiveTopology,
        Indices,
    },
    asset::RenderAssetUsages,
};
use::std::{
    io::{ BufRead, BufReader, },
    fs::File,
    path::PathBuf,
};
use ahash::{AHashMap, AHashSet};
use walkdir::WalkDir;
use crate::{
    mesh_ops::{generate_mhid_lookup, generate_vertex_map, get_uv_coords, get_vertex_normals, get_vertex_positions, get_vertex_tangents, parse_obj_vertices, MeshProcessingState, PrefabLoadState}, morphs::{adjust_helpers_to_morphs, asset_mesh_from_helpers}, prelude::*, rigs::{set_asset_rig_arrays, RigData}
};

/*---------+
 |  Asset  |
 +---------*/
 /// The types of asset types which can be added to humans.
 /// Does not include base mesh which is special
#[derive(Component, Clone, Eq, PartialEq, Hash)]
pub enum HumanPart {
    BaseMesh,
    ProxyMesh(Name),
    BodyPart(Name),
    Equipment(Name),
}

/// The texture types which can be loaded for materials which go on [`HumanAsset`] meshes
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum HumanAssetTextureType {
    Albedo,
    Normal,
    AmbientOcclusion,
    SubsurfaceScattering,
}

/// Represents a part of a human, either a body part, equipment, or a proxy mesh. 
/// This is a wrapper around a mesh which is morphable by the makehuman morph targets.
/// Does not represent the base mesh which is special
pub struct HumanAsset {
    part: HumanPart,
    paths: HumanMeshAssetFilePaths,
    pub(crate) data: Option<HumanAssetData>,
}

impl HumanAsset {
    pub fn get_name(&self) -> Name {
        match &self.part {
            HumanPart::BaseMesh => Name::new("basemesh"),
            HumanPart::ProxyMesh(name) |
            HumanPart::Equipment(name) |
            HumanPart::BodyPart(name) => name.clone()
        }
    }

    /// Check if the data is defined
    pub fn is_loaded(&self) -> bool {
        self.data.is_some()
    }

    /// Checks if asset data is loaded.  If not, parses makehuman file and then loads the mesh.  Returns true if mesh was just loaded
    pub fn load_asset_if_unloaded(&mut self, asset_server: &mut AssetServer) -> bool {
        if self.is_loaded() { return false; }
        self.data = Some(parse_human_asset(&self.paths.mh_file, &self.part, asset_server));
        true
    }

    /// Return the mesh handle of the asset's core mesh.  This is not the obj mesh, which must be resized first.
    pub(crate) fn get_mesh_handle(
        &mut self, asset_server: &mut AssetServer, meshes: &mut Assets<Mesh>, paths: &HumentityPathsConfig
    ) -> Option<Handle<Mesh>> {
        if self.data.is_none() {
            self.load_asset_if_unloaded(asset_server);
        }
        let data = self.data.as_mut().unwrap();
        data.get_mesh_handle(meshes, paths)
    }

    /// Return the mesh handle of the asset which has been augmented with arrays for skinning with the given rig
    pub(crate) fn get_rigged_mesh_handle(
        &mut self, asset_server: &mut AssetServer, 
        prefab_name: &Name,
        prefab: &HumanArchetypePrefab,
        rig_data: &RigData,
        basemesh: &BaseMesh,
        morph_targets: &HumanMorphs,
        paths: &HumentityPathsConfig,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Mesh>> {
        if self.data.is_none() {
            self.load_asset_if_unloaded(asset_server);
            return None
        }
        let data = self.data.as_mut().unwrap();
        data.get_rigged_mesh_handle(prefab_name, prefab, basemesh, morph_targets, rig_data, paths, meshes, images)
    }

    /// Get a texture by name, loading into Assets if necessary
    pub fn get_texture_handle(
        &mut self, name: Name, texture_type: HumanAssetTextureType, asset_server: &mut AssetServer
    ) -> Handle<Image> {
        if self.data.is_none() {
            self.load_asset_if_unloaded(asset_server);
        }
        let handles = match texture_type {
            HumanAssetTextureType::Albedo => &mut self.data.as_mut().unwrap().albedo_map_handles,
            HumanAssetTextureType::Normal => &mut self.data.as_mut().unwrap().normal_map_handles,
            HumanAssetTextureType::AmbientOcclusion => &mut self.data.as_mut().unwrap().ao_map_handles,
            _ => unimplemented!("No such texture type defined for this asset type"),
        };
        if !handles.contains_key(&name) {
            let paths = match texture_type {
                HumanAssetTextureType::Albedo => &mut self.paths.albedo_maps,
                HumanAssetTextureType::Normal => &mut self.paths.normal_maps,
                HumanAssetTextureType::AmbientOcclusion => &mut self.paths.ao_maps,
                _ => unimplemented!("No such texture type defined for this asset type"),
            };
            let part_name = match &self.part {
                HumanPart::BodyPart(n)  |
                HumanPart::Equipment(n) |
                HumanPart::ProxyMesh(n) => n,
                _ => unimplemented!("Base mesh is not a HumanAsset")
            };
            let path = paths.get(&name)
                .expect(&format!("No albedo map {name} found for {}", part_name));
            handles.insert(
                name.clone(),
                asset_server.load(format!("humentity://{}", path.to_str().expect("Unreadable path string")))
            );
        }
        handles[&name].clone()
    }

    /// Remove cached handles.  If there are no other handles in the world then
    /// bevy should unload the assets.
    pub fn unload(&mut self) {
        self.data = None
    }


}

/// File paths for assets to be loaded for assets
#[derive(Default)]
struct HumanMeshAssetFilePaths {
    mh_file: PathBuf,
    albedo_maps: AHashMap<Name, PathBuf>,
    normal_maps: AHashMap<Name, PathBuf>,
    ao_maps: AHashMap<Name, PathBuf>,
}

/// The cached data for the asset, includes handles to relevant assets and
/// other misc. data relevant to the makehuman system read from mh files.
#[derive(Default)]
#[allow(dead_code)]
pub struct HumanAssetData {
    pub bodypart_slots: Vec<BodyPartSlot>,
    pub equipment_slots: Vec<EquipmentSlot>,
    pub(crate) base_mesh_handle: Handle<Mesh>,
    pub(crate) prefab_load_state: PrefabLoadState,
    pub(crate) albedo_map_handles: AHashMap<Name, Handle<Image>>,
    pub(crate) normal_map_handles: AHashMap<Name, Handle<Image>>,
    pub(crate) ao_map_handles: AHashMap<Name, Handle<Image>>,
    pub(crate) helper_map: Vec<HelperMap>,
    pub(crate) mhid_lookup: Vec<u16>,
    pub(crate) delete_verts: AHashSet<u16>,
    obj_file: PathBuf,
    tags: Vec<Name>,
    z_depth: i8,
    scale_data: [ScaleData; 3],
}

impl HumanAssetData {
    pub(crate) fn get_mesh_handle(
        &mut self, meshes: &mut Assets<Mesh>, paths: &HumentityPathsConfig
    ) -> Option<Handle<Mesh>> {
        // Get the makehuman vertex index lookup 
        let Some(mesh) = meshes.get(&self.base_mesh_handle) else { return None };
        let path = paths.core_assets_path.join(&self.obj_file);
        let mh_verts = parse_obj_vertices(path);
        let verts = get_vertex_positions(mesh);
        let vertex_map = generate_vertex_map(&mh_verts, &verts);
        self.mhid_lookup = generate_mhid_lookup(&vertex_map);
        Some(self.base_mesh_handle.clone())
    }

    pub(crate) fn get_rigged_mesh_handle(
        &mut self,
        prefab_name: &Name,
        prefab: &HumanArchetypePrefab,
        basemesh: &BaseMesh,
        morph_targets: &HumanMorphs,
        rig_data: &RigData,
        paths: &HumentityPathsConfig,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Mesh>> {
        if !self.prefab_load_state.contains_key(prefab_name) {
            self.prefab_load_state.insert(prefab_name.clone(), MeshProcessingState::Unprocessed);
        }
        if self.get_mesh_handle(meshes, paths).is_none() { return None }

        match &self.prefab_load_state[prefab_name] {
            MeshProcessingState::Ready(handle) => return Some(handle.clone()),
            MeshProcessingState::Morphed(handle) => {
                let mesh = meshes.get(handle).unwrap().clone();
                let handle = set_asset_rig_arrays(mesh, meshes, rig_data, &self.mhid_lookup, &self.helper_map, &prefab.rig);
                self.prefab_load_state.insert(prefab_name.clone(), MeshProcessingState::Ready(handle.clone()));
                return Some(handle);
            }
            MeshProcessingState::Unprocessed => {
                let mut handles = vec![];
                for shape in prefab.shapes.iter() {
                    let helpers = adjust_helpers_to_morphs(&shape.morphs, morph_targets, basemesh);
                    let handle = asset_mesh_from_helpers(&helpers, &shape.morphs, morph_targets, meshes, self);
                    handles.push(handle);
                }
                self.prefab_load_state.insert(prefab_name.clone(), MeshProcessingState::Shaped(handles));

                let asset_base_mesh = meshes.get(&self.base_mesh_handle).unwrap();
                self.base_mesh_handle = meshes.add(asset_base_mesh.clone()
                    .with_computed_area_weighted_normals()
                    .with_generated_tangents()
                    .unwrap());

                None
            }
            MeshProcessingState::Shaped(shaped_meshes) => {
                let asset_base_mesh = meshes.get(&self.base_mesh_handle).unwrap();
                let base_positions = get_vertex_positions(&asset_base_mesh);
                let base_normals = get_vertex_normals(&asset_base_mesh);
                let Ok(base_tangents) = get_vertex_tangents(&asset_base_mesh) else { return None };
                let mut morph_names = vec![];
                let mut morphs = vec![];

                for (is, shape) in prefab.shapes.iter().enumerate() {
                    let mut morph = Vec::<MorphAttributes>::new();
                    let shape_mesh = meshes.get(&shaped_meshes[is]).unwrap();
                    let shape_positions = get_vertex_positions(&shape_mesh);
                    let shape_normals = get_vertex_normals(&shape_mesh);
                    let shape_tangents = get_vertex_tangents(&shape_mesh)
                        .expect("Shape meshes should always have tangents");

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
                    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, get_vertex_positions(&asset_base_mesh))
                    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, get_uv_coords(&asset_base_mesh))
                    .with_inserted_indices(asset_base_mesh.indices().unwrap().clone())
                    .with_computed_area_weighted_normals()
                    .with_morph_targets(images.add(image.0))
                    .with_morph_target_names(morph_names)
                    .with_generated_tangents().unwrap();

                self.prefab_load_state.insert(prefab_name.clone(), MeshProcessingState::Morphed(meshes.add(mesh)));
                None
            }
        }
    }

    pub(crate) fn get_offset_scale(&self, helpers: &Vec<Vec3>) -> Vec3 {
        Vec3::new(
            (helpers[self.scale_data[0].max as usize].x - helpers[self.scale_data[0].min as usize].x) / self.scale_data[0].scale,
            (helpers[self.scale_data[1].max as usize].y - helpers[self.scale_data[1].min as usize].y) / self.scale_data[1].scale,
            (helpers[self.scale_data[2].max as usize].z - helpers[self.scale_data[2].min as usize].z) / self.scale_data[2].scale,
        )
    }
}

pub enum PartSlots {
    BodyPartSlots(Vec<BodyPartSlot>),
    EquipmentSlots(Vec<EquipmentSlot>),
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum BodyPartSlot {
    LeftEye,
    RightEye,
    LeftEyebrow,
    RightEyebrow,
    LeftEyelash,
    RightEyelash,
    Tongue,
    Teeth,
    Hair,
    FacialHair,
    Custom(Name),
}

impl BodyPartSlot {
    fn match_name(name: impl AsRef<str>) -> BodyPartSlot {
        match name.as_ref() {
            "LeftEye" => BodyPartSlot::LeftEye,
            "RightEye" => BodyPartSlot::RightEye,
            "LeftEyebrow" => BodyPartSlot::LeftEyebrow,
            "RightEyebrow" => BodyPartSlot::RightEyebrow,
            "LeftEyelash" => BodyPartSlot::LeftEyelash,
            "RightEyelash" => BodyPartSlot::RightEyelash,
            "Tongue" => BodyPartSlot::Tongue,
            "Teeth" => BodyPartSlot::Teeth,
            "Hair" => BodyPartSlot::Hair,
            "FacialHair" => BodyPartSlot::FacialHair,
            _ => {
                BodyPartSlot::Custom(Name::new(NAME_INTERNER.intern(name.as_ref()).leak()))
            }
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum EquipmentSlot {
    Head,
    Eyes,
    Ears,
    Mouth,
    Nose,
    Torso,
    Hips,
    RightArm,
    LeftArm,
    LeftHand,
    RightHand,
    LeftLeg,
    RightLeg,
    LeftFoot,
    RightFoot,
    Custom(Name),
}

impl EquipmentSlot {
    fn match_name(name: impl AsRef<str>) -> EquipmentSlot {
        match name.as_ref() {
            "Head" => EquipmentSlot::Head,
            "Eyes" => EquipmentSlot::Eyes,
            "Ears" => EquipmentSlot::Ears,
            "Mouth" => EquipmentSlot::Mouth,
            "Nose" => EquipmentSlot::Nose,
            "Torso" => EquipmentSlot::Torso,
            "Hips" => EquipmentSlot::Hips,
            "LeftArm" => EquipmentSlot::LeftArm,
            "RightArm" => EquipmentSlot::RightArm,
            "LeftHand" => EquipmentSlot::LeftHand,
            "RightHand" => EquipmentSlot::RightHand,
            "LeftFoot" => EquipmentSlot::LeftFoot,
            "RightFoot" => EquipmentSlot::RightFoot,
            "LeftLeg" => EquipmentSlot::LeftLeg,
            "RightLeg" => EquipmentSlot::RightLeg,
            _ => EquipmentSlot::Custom(Name::new(NAME_INTERNER.intern(name.as_ref()).leak()))
        }
    }
}

/*-------------------+
 |  Makehuman Types  |
 +-------------------*/
// Each vertex is mapped to either a single helper vertex
// or triangulated by 3 of them
#[derive(Default, Debug)]
pub(crate) struct HelperMap {
    pub(crate) single_vertex: Option<u16>,
    pub(crate) triangle: Option<Triangle>,
}

#[derive(Default, Debug)]
pub(crate) struct Triangle {
   pub(crate) helper_verts: [u16; 3],
   pub(crate) helper_weights: [f32; 3],
   pub(crate) helper_offset: Vec3,
}

#[derive(Default)]
struct ScaleData {
   min: u16,
   max: u16,
   scale: f32,
}

#[derive(Eq, PartialEq)]
enum FileSection {
   Header,
   Vertices,
   DeleteVertices,
}

/*-------------+
 |  Resources  |
 +-------------*/
#[derive(Default, Resource)]
#[allow(dead_code)]
pub struct HumanBodyTextures {
    albedo_maps: AHashMap<Name, PathBuf>,
    normal_maps: AHashMap<Name, PathBuf>,
    ao_maps: AHashMap<Name, PathBuf>,
    sss_maps: AHashMap<Name, PathBuf>,
}

#[derive(Resource)]
#[allow(dead_code)]
pub struct HumanAssetRegistry {
    pub assets: AHashMap<HumanPart, HumanAsset>,
    //pub bodypart_slots: AHashMap<BodyPartSlot, Vec<Name>>,
    //pub equipment_slots: AHashMap<EquipmentSlot, Vec<Name>>,
}

impl HumanAssetRegistry {
    pub fn get(&self, part: &HumanPart) -> Option<&HumanAsset> {
        self.assets.get(part)
    }
    
    pub fn get_mut(&mut self, part: &HumanPart) -> Option<&mut HumanAsset> {
        self.assets.get_mut(part)
    }
}

impl FromWorld for HumanAssetRegistry {
    fn from_world(world: &mut World) -> Self{
        let mut assets = AHashMap::<HumanPart, HumanAsset>::default();
        //let mut body_parts = AHashMap::<BodyPartSlot, Vec<Name>>::default();
        //let mut equipment = AHashMap::<EquipmentSlot, Vec<Name>>::default();

        let config = world.get_resource_mut::<HumentityPathsConfig>()
            .expect("No global Humentity config loaded");
        let root_path = config.core_assets_path.clone();
        let body_part_paths = config.body_part_paths.clone();
        let equipment_paths = config.equipment_paths.clone();
        let proxymesh_paths = config.proxymesh_paths.clone();

        for dir in body_part_paths {
            for entry in WalkDir::new(root_path.join(dir)).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                if !path.is_file() { continue; }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else { continue };
                if extension == "mhclo" {
                    let folder = path.parent().expect("No parent folder?").to_path_buf();

                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .expect("Failed to parse file name");
                    info!("Importing body part : {name}");
                    let name = Name::new(NAME_INTERNER.intern(name).leak());
                    let albedo_maps = get_textures(&folder, HumanAssetTextureType::Albedo);
                    let normal_maps = get_textures(&folder, HumanAssetTextureType::Normal);
                    let ao_maps = get_textures(&folder, HumanAssetTextureType::AmbientOcclusion);
                    let paths = HumanMeshAssetFilePaths {
                        albedo_maps, normal_maps, ao_maps, mh_file: path.to_path_buf()
                    };
                    let part = HumanPart::BodyPart(name.clone());
                    let asset = HumanAsset {
                        part: part.clone(), data: None, paths
                    };
                    // insert into name hashmap
                    assets.insert(part.clone(), asset);
                }
            }
        }

        for dir in equipment_paths {
            for entry in WalkDir::new(root_path.join(dir)).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                if !path.is_file() { continue; }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else { continue };
                if extension == "mhclo" {
                    let folder = path.parent().expect("No parent folder?").to_path_buf();

                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .expect("Failed to parse file name");
                    info!("Importing equipment : {name}");
                    let name = Name::new(NAME_INTERNER.intern(name).leak());
                    let albedo_maps = get_textures(&folder, HumanAssetTextureType::Albedo);
                    let normal_maps = get_textures(&folder, HumanAssetTextureType::Normal);
                    let ao_maps = get_textures(&folder, HumanAssetTextureType::AmbientOcclusion);
                    let paths = HumanMeshAssetFilePaths {
                        albedo_maps, normal_maps, ao_maps, mh_file: path.to_path_buf()
                    };
                    let part = HumanPart::Equipment(name.clone());
                    let asset = HumanAsset {
                        part: part.clone(), data: None, paths
                    };
                    // insert into name hashmap
                    assets.insert(part, asset);
                }
            }
        }

        for dir in proxymesh_paths {
            for entry in WalkDir::new(root_path.join(dir)).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                if !path.is_file() { continue; }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else { continue };
                if extension == "proxy" {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .expect("Failed to parse file name");
                    let name = Name::new(NAME_INTERNER.intern(name).leak());
                    info!("Importing proxy mesh : {name}");
                    let mut paths = HumanMeshAssetFilePaths::default();
                    paths.mh_file = path.to_path_buf();
                    let part = HumanPart::ProxyMesh(name.clone());
                    let asset = HumanAsset {
                        part: part.clone(), data: None, paths
                    };
                    // insert into name hashmap
                    assets.insert(part, asset);
                }
            }
        }

        // Load body textures
        let path = config.core_assets_path.join("skin_textures");
        let albedo_maps = get_textures(&path, HumanAssetTextureType::Albedo);
        let normal_maps = get_textures(&path, HumanAssetTextureType::Normal);
        let ao_maps = get_textures(&path, HumanAssetTextureType::AmbientOcclusion);
        let sss_maps = get_textures(&path, HumanAssetTextureType::SubsurfaceScattering);

        world.insert_resource(HumanBodyTextures { albedo_maps, normal_maps, ao_maps, sss_maps });

        HumanAssetRegistry {
            assets,
            //body_parts: slot_body_parts,
            //equipment: slot_equipment,
        }
    }
}

/*-------------+
 |  Functions  |
 +-------------*/
fn get_textures(path: &PathBuf, texture_type: HumanAssetTextureType) -> AHashMap<Name, PathBuf> {
    let folder = match texture_type {
        HumanAssetTextureType::Albedo => path.join("albedo"),
        HumanAssetTextureType::Normal => path.join("Normal"),
        HumanAssetTextureType::AmbientOcclusion => path.join("ao"),
        HumanAssetTextureType::SubsurfaceScattering => path.join("sss"),
    };
    let mut textures = AHashMap::default();
    if !folder.exists() { return textures }
    for entry in std::fs::read_dir(folder)
        .expect("Failed to read folder")
    {
        let Ok(entry) = entry else { continue };
        let path = entry.path();

        if path.is_file() {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let stem = NAME_INTERNER.intern(stem).leak();
                textures.insert(Name::new(stem), path);
            }
        }
    }
    textures
}

fn parse_human_asset(mh_path: &PathBuf, part: &HumanPart, asset_server: &AssetServer) -> HumanAssetData {
    let mut tags = Vec::<String>::new();
    let mut z_depth = 0 as i8;
    let mut delete_verts = AHashSet::<u16>::default();
    let mut helper_map = Vec::<HelperMap>::new();
    let mut x_scale = ScaleData::default();
    let mut y_scale = ScaleData::default();
    let mut z_scale = ScaleData::default();
    let mut obj_file = PathBuf::default();
    let mut section = FileSection::Header;
    //let mut name = Name::new("");

    let err_msg = format!("Couldn't open target file {}", mh_path.to_string_lossy());
    let file = File::open(&mh_path).expect(&err_msg);
    for line_result in BufReader::new(file).lines() {

        let Ok(line) = line_result else { break };
        if line.starts_with("#") { continue; }
        if line.trim().is_empty() { continue; }
        if line.starts_with("verts 0") { section = FileSection::Vertices; continue; }
        if line.starts_with("delete_verts") { section = FileSection::DeleteVertices; continue; }

        let line_vec: Vec<&str> = line.trim().split_whitespace().collect();

        if section == FileSection::Header {
            if *line_vec.first().unwrap() == "obj_file" {
                let filename = line_vec.last().unwrap();
                obj_file = mh_path.clone();
                obj_file.set_file_name(filename);
                obj_file = obj_file.strip_prefix(PathBuf::from("./assets")).unwrap().to_path_buf();
            } else if *line_vec.first().unwrap() == "x_scale" {
                x_scale.min = line_vec[1].parse().unwrap();
                x_scale.max = line_vec[2].parse().unwrap();
                x_scale.scale = line_vec[3].parse().unwrap();
            } else if *line_vec.first().unwrap() == "y_scale" {
                y_scale.min = line_vec[1].parse().unwrap();
                y_scale.max = line_vec[2].parse().unwrap();
                y_scale.scale = line_vec[3].parse().unwrap();
            } else if *line_vec.first().unwrap() == "z_scale" {
                z_scale.min = line_vec[1].parse().unwrap();
                z_scale.max = line_vec[2].parse().unwrap();
                z_scale.scale = line_vec[3].parse().unwrap();
            } else if *line_vec.first().unwrap() == "z_depth" {
                z_depth = line_vec[1].parse().unwrap();
            } else if *line_vec.first().unwrap() == "tag" {
                tags.push(line_vec.last().unwrap().to_string());
            } else if *line_vec.first().unwrap() == "name" {
                //name = line_vec.last().unwrap().to_string();
            }
        } else if section == FileSection::Vertices {
            // Some header lines work there way down here on occasion
            if line_vec[0] == "material" { continue; }
            if line_vec.len() == 9 {
                let helper_verts = [
                    line_vec[0].parse().unwrap(),
                    line_vec[1].parse().unwrap(),
                    line_vec[2].parse().unwrap(),
                ];
                let mut helper_weights = [
                    line_vec[3].parse().unwrap(),
                    line_vec[4].parse().unwrap(),
                    line_vec[5].parse().unwrap(),
                ];
                for i in 0..3 {
                    helper_weights[i] /= helper_weights.iter().sum::<f32>();
                }
                let helper_offset = Vec3::new(
                    line_vec[6].parse().unwrap(),
                    line_vec[7].parse().unwrap(),
                    line_vec[8].parse().unwrap(),
                );
                helper_map.push(HelperMap{
                    triangle: Some(Triangle {
                        helper_verts: helper_verts,
                        helper_weights: helper_weights,
                        helper_offset: helper_offset,
                    }),
                    single_vertex: None
                });
            } else if line_vec.len() == 1 {
                helper_map.push(HelperMap{
                    triangle: None,
                    single_vertex: Some(line.trim().parse().unwrap())
                });
            } else {
                println!("{:?}", line);
                panic!("Unparseable vertex line")
            }
        } else if section == FileSection::DeleteVertices {
            // Either vert index "v" or vert range "v1 - v2"
            let mut start: Option<u16> = None;
            let mut grouping = false;
            for &v in line_vec.iter() {
                if grouping {
                    let Some(s) = start else { panic!("Failed to parse delete verts") };
                    for i in s..=v.parse().unwrap() { delete_verts.insert(i); };
                    start = None;
                    grouping = false;
                } else if v != "-" {
                    if let Some(s) = start { delete_verts.insert(s); }
                    start = Some(v.parse().unwrap());
                } else { grouping = true; }
            }

            // If there's a final start without a pairing, push it
            if let Some(s) = start {
                delete_verts.insert(s);
            }
        }
    }

    // Ignore.  Use file_stem instead
    //let name = Name::new(name);
    let tags = tags
        .iter()
        .map(|n| Name::new(NAME_INTERNER.intern(n).leak()))
        .collect::<Vec<_>>();
    let mut bodypart_slots = vec![];
    let mut equipment_slots = vec![];

    match &part {
        HumanPart::BodyPart(_) => {
            for tag in tags.iter() {
                let slot = BodyPartSlot::match_name(tag);
                if !matches!(slot, BodyPartSlot::Custom(_)) {
                    bodypart_slots.push(slot);
                }
            }
        }
        HumanPart::Equipment(_) => {
            for tag in tags.iter() {
                let slot = EquipmentSlot::match_name(tag);
                if !matches!(slot, EquipmentSlot::Custom(_)) {
                    equipment_slots.push(slot);
                }
            }
        }
        _ => { }
    }
    let base_mesh_path = format!("humentity://{}", obj_file.clone().to_str().unwrap());
    let base_mesh_handle = asset_server.load(base_mesh_path);

    HumanAssetData {
        obj_file,
        tags,
        z_depth,
        delete_verts,
        scale_data: [x_scale, y_scale, z_scale],
        base_mesh_handle,
        bodypart_slots,
        equipment_slots,
        helper_map,
        ..default()
    }
}

#[allow(dead_code)]
pub(crate) fn delete_mesh_verts(
    meshes: &mut ResMut<Assets<Mesh>>,
    base_mesh: &Res<crate::basemesh::BaseMesh>,
    delete_verts: AHashSet<u16>,
) -> Mesh {
    let mesh = meshes.get(&base_mesh.mesh_handle).unwrap().clone();

    let vertices = get_vertex_positions(&mesh);
    let normals = get_vertex_normals(&mesh);
    let uv = get_uv_coords(&mesh);
    let indices = mesh.indices().expect("FAILED TO GET MESH FACES");

    // Set up new storage for the new mesh
    let verts = vertices.len() - delete_verts.len();  // Roughly
    let mut new_vertices = Vec::<Vec3>::with_capacity(verts);
    let mut new_normals = Vec::<Vec3>::with_capacity(verts);
    let mut new_uv = Vec::<Vec2>::with_capacity(verts);
    let mut new_indices = Vec::<u16>::with_capacity(verts);
    
    // need to map new vertex indices to original before deleting verts
    let mut indices_map = AHashMap::<u16, u16>::default();

    for (vtx, &mh_vert) in base_mesh.mhid_lookup.iter().enumerate() {
        if !delete_verts.contains(&mh_vert) {
            indices_map.insert(vtx as u16, new_vertices.len() as u16);
            new_vertices.push(vertices[vtx as usize]);
            new_normals.push(normals[vtx as usize]);
            new_uv.push(uv[vtx as usize]);
        }
    }
    
    let indices_vec: Vec<u16> = indices.iter().map(|x| x as u16).collect();
    // Find new face indices
    for face in indices_vec.chunks(3) {
        // Check if all vertices still exist in new mesh verts
        if !face.iter().all(|&i| indices_map.contains_key(&(i as u16))) { continue; }
        // Map face to new vertex indices
        new_indices.extend_from_slice(face);
    }
    new_indices = new_indices.iter().map(|x| *indices_map.get(x).unwrap()).collect();

    let mut new_mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, new_vertices)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, new_normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, new_uv)
        .with_inserted_indices(Indices::U16(new_indices));
    new_mesh.compute_area_weighted_normals();
    new_mesh.generate_tangents().ok();
    new_mesh
}