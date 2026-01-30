use crate::{
    mesh_ops::{
        fix_normals, generate_mhid_lookup, generate_vertex_map, get_uv_coords, get_vertex_normals, get_vertex_positions, get_vertex_tangents, parse_obj_vertices
    },
    morphs::{MHMorphs, adjust_helpers_to_morphs},
    paths_config::{HumentityAssetPath, HumentityAssetSourceId},
    prelude::*,
    rigs::{RigWeights, SkeletonCache, set_asset_rig_arrays}, spawn_mesh::{AssetLoadedMsg, MeshConstructedMsg},
};
use ::bevy::{
    asset::RenderAssetUsages,
    prelude::*,
};
use ::std::{
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};
use ahash::{AHashMap, AHashSet};
use bevy::{
    ecs::intern::Internable,
    mesh::morph::{MorphAttributes, MorphTargetImage},
};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::{path::Path, sync::Arc};
use walkdir::WalkDir;
use crossbeam_channel::{Sender, Receiver};

const ALBEDO_SUBFOLDERS: [&str; 3] = ["albedo", "diffuse", "base_color"];
const NORMAL_SUBFOLDERS: [&str; 1] = ["normal"];
const OCCLUSION_SUBFOLDERS: [&str; 2] = ["occlusion", "ambient_occlusion"];
const ROUGHNESS_METALLIC_SUBFOLDERS: [&str; 3] = ["roughness", "roughness_metallic", "metallic"];

/// The types of asset types which can be added to humans.
/// Does not include base mesh which is special
#[derive(Component, Clone, Eq, PartialEq, Hash, Debug)]
#[require(Visibility)]
pub enum CharacterPart {
    BodyMesh(&'static str),
    BodyPart(&'static str),
    Equipment(&'static str),
}

impl CharacterPart {
    /// Load a texture by name/type for this part and return the handle
    pub fn get_texture_handle(
        &self,
        texture_name: impl AsRef<str>,
        texture_type: CharacterAssetTextureType,
        asset_server: &AssetServer,
        asset_registry: &CharacterAssetRegistry,
    ) -> Handle<Image> {
        let Some(asset) = asset_registry.get(self) else {
            error!("No such asset registered: {:#?}", self);
            return Handle::default();
        };
        asset.get_texture_handle(texture_name.as_ref(), texture_type, asset_server)
    }

    /// Clear out the CharacterAssetData struct storing mesh and texture handles
    pub fn unload_data(&self, registry: &mut CharacterAssetRegistry, prefab_name: &'static str) {
        if let Some(asset) = registry.get_mut(self) {
            asset.unload(prefab_name);
        }
    }
}

impl Serialize for CharacterPart {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = match self {
            CharacterPart::BodyMesh(name) => format!("BodyMesh:{}", name),
            CharacterPart::BodyPart(name) => format!("BodyPart:{}", name),
            CharacterPart::Equipment(name) => format!("Equipment:{}", name),
        };
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for CharacterPart {
    fn deserialize<D>(deserializer: D) -> Result<CharacterPart, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        let mut parts = s.splitn(2, ':');
        let variant = parts.next().unwrap();
        let value = parts
            .next()
            .ok_or_else(|| de::Error::custom("expected variant:data"))?;

        let interned = NAME_INTERNER.intern(value).leak();

        match variant {
            "BodyMesh" => Ok(CharacterPart::BodyMesh(interned)),
            "BodyPart" => Ok(CharacterPart::BodyPart(interned)),
            "Equipment" => Ok(CharacterPart::Equipment(interned)),
            other => Err(de::Error::custom(format!("unknown variant `{}`", other))),
        }
    }
}

/// The texture types which can be loaded for materials which go on [`CharacterAsset`] meshes
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub enum CharacterAssetTextureType {
    Albedo,
    Normal,
    AmbientOcclusion,
    RoughnessMetallic,
}

#[derive(PartialEq, Eq, Debug, Default)]
pub(crate) enum AssetLoadState {
    #[default]
    None,
    LoadingData,
    BuildingMesh,
}

/// Represents a part of a human, either a body part, equipment, or a proxy mesh.
/// This is a wrapper around a mesh which is morphable by the makehuman morph targets.
/// Does not represent the base mesh which is special
pub struct CharacterAsset {
    pub(crate) part: CharacterPart,
    pub(crate) loading: AssetLoadState,
    pub paths: CharacterMeshAssetFilePaths,
    pub data: Option<CharacterAssetData>,
    pub raw_mesh_handle: Option<Handle<Mesh>>,
    pub mesh_handles: AHashMap<&'static str, Handle<Mesh>>,
    pub(crate) mesh_building_msg_sender: Sender<MeshConstructedMsg>,
    pub(crate) mesh_building_msg_receiver: Receiver<MeshConstructedMsg>,
    pub(crate) asset_loading_msg_sender: Sender<AssetLoadedMsg>,
    pub(crate) asset_loading_msg_receiver: Receiver<AssetLoadedMsg>,
}

impl CharacterAsset {
    pub fn new(part: CharacterPart, paths: CharacterMeshAssetFilePaths) -> Self {
        let ( asset_loading_msg_sender, asset_loading_msg_receiver ) = crossbeam_channel::unbounded();
        let ( mesh_building_msg_sender, mesh_building_msg_receiver ) = crossbeam_channel::unbounded();
        Self {
            part, paths, data: None,
            loading: AssetLoadState::None, raw_mesh_handle: None, mesh_handles: AHashMap::default(),
            mesh_building_msg_sender, mesh_building_msg_receiver,
            asset_loading_msg_sender, asset_loading_msg_receiver,
         }
    }

    pub const fn get_name(&self) -> &'static str {
        match self.part {
            CharacterPart::BodyMesh(name)
            | CharacterPart::Equipment(name)
            | CharacterPart::BodyPart(name) => name,
        }
    }

    /// Get a texture by name and texture type
    pub fn get_texture_handle(
        &self,
        texture_name: impl AsRef<str>,
        texture_type: CharacterAssetTextureType,
        asset_server: &AssetServer,
    ) -> Handle<Image> {
        let paths = match texture_type {
            CharacterAssetTextureType::Albedo => &self.paths.albedo_maps,
            CharacterAssetTextureType::Normal => &self.paths.normal_maps,
            CharacterAssetTextureType::AmbientOcclusion => &self.paths.ao_maps,
            CharacterAssetTextureType::RoughnessMetallic => &self.paths.ao_maps,
        };
        if let Some(path) = paths.get(texture_name.as_ref()) {
            path.load_asset(asset_server)
        } else {
            error!(
                "No such texture {} for {:#?}",
                texture_name.as_ref(),
                self.part
            );
            Handle::<Image>::default()
        }
    }

    /// Remove cached mesh handle. 
    pub fn unload(&mut self, prefab_name: &'static str) {
        self.mesh_handles.remove(prefab_name);
        if self.mesh_handles.is_empty() {
            self.data = None;
            self.raw_mesh_handle = None;
            self.mesh_handles.clear();
            self.loading = AssetLoadState::None;
        }
    }
}

/// File paths for assets to be loaded for assets
pub struct CharacterMeshAssetFilePaths {
    pub(crate) mh_file: HumentityAssetPath,
    pub albedo_maps: AHashMap<&'static str, HumentityAssetPath>,
    pub normal_maps: AHashMap<&'static str, HumentityAssetPath>,
    pub ao_maps: AHashMap<&'static str, HumentityAssetPath>,
    pub roughness_metallic_maps: AHashMap<&'static str, HumentityAssetPath>,
}

/// The cached data for the asset, includes handles to relevant assets and
/// other misc. data relevant to the makehuman system read from mh files.
#[allow(dead_code)]
#[derive(Clone)]
pub struct CharacterAssetData {
    pub(crate) helper_map: Vec<HelperMap>,
    pub(crate) delete_verts: AHashSet<u16>,
    pub(crate) obj_file: HumentityAssetPath,
    tags: Vec<&'static str>,
    z_depth: i8, // Currently unused
    scale_data: [ScaleData; 3],
}

impl CharacterAssetData {
    pub(crate) fn build_final_mesh(
        &self,
        input_mesh: &Mesh,
        prefab: &CharacterArchetypePrefab,
        mh_morphs: &Arc<MHMorphs>,
        basemesh: &BaseMesh,
        rig_weights: &Arc<RigWeights>,
        sk_cache: &Arc<SkeletonCache>,
    ) -> (Mesh, Vec<String>, MorphTargetImage) {

        // Load raw mesh and build vertex lookup between mh indices and bevy indices (vert duplicates in bevy)
        let mh_vertices = parse_obj_vertices(self.obj_file.full_path());
        let verts = get_vertex_positions(&input_mesh);
        let vertex_map = generate_vertex_map(&mh_vertices, &verts);
        let mhid_lookup = generate_mhid_lookup(vertex_map);

        // Recaculate mesh from helpers, fixes scale, redoes normals and tangents
        let input_mesh = self.shape_mesh_from_helpers(&input_mesh, &basemesh.0, &mhid_lookup);

        // Build shaped meshes
        let mut meshes = vec![];
        for shape in prefab.shapes.iter() {
            let helpers = adjust_helpers_to_morphs(&shape.morphs, &mh_morphs, &basemesh);
            let mesh = self.shape_mesh_from_helpers(&input_mesh, &helpers, &mhid_lookup);
            meshes.push(mesh);
            //// How to handle heights
            //shape.height =
            //    f32::max(shape.height, helpers.iter().map(|v| v.y).reduce(f32::max)?);
        }

        // Build morphs from shapes
        let base_positions = get_vertex_positions(&input_mesh);
        let base_normals = get_vertex_normals(&input_mesh);
        let base_tangents = get_vertex_tangents(&input_mesh)
            .expect("Failed to get tangents");
        let mut morph_names = vec![];
        let mut morphs = vec![];

        for (is, shape_mesh) in meshes.iter().enumerate() {
            let mut morph = Vec::<MorphAttributes>::new();
            let shape_positions = get_vertex_positions(shape_mesh);
            let shape_normals = get_vertex_normals(shape_mesh);
            let shape_tangents = get_vertex_tangents(shape_mesh)
                .expect("Shape meshes should always have tangents");

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

            morph_names.push(prefab.shapes[is].name.clone());
            morphs.push(morph.into_iter());
        }
        let image = MorphTargetImage::new(
            morphs.into_iter(),
            base_positions.len(),
            RenderAssetUsages::default(),
        )
        .expect("failed to create morph target image");

        // Rig the mesh
        let mut mesh = set_asset_rig_arrays(input_mesh, rig_weights,
            &mhid_lookup, &self.helper_map, &prefab.rig, &sk_cache);
        fix_normals(&mut mesh, &mhid_lookup);
        (mesh, morph_names, image)
    }

    pub(crate) fn get_offset_scale(&self, helpers: &[Vec3]) -> Vec3 {
        Vec3::new(
            (helpers[self.scale_data[0].max as usize].x
                - helpers[self.scale_data[0].min as usize].x)
                / self.scale_data[0].scale,
            (helpers[self.scale_data[1].max as usize].y
                - helpers[self.scale_data[1].min as usize].y)
                / self.scale_data[1].scale,
            (helpers[self.scale_data[2].max as usize].z
                - helpers[self.scale_data[2].min as usize].z)
                / self.scale_data[2].scale,
        )
    }

    /// Adjust an asset mesh to match morphed helpers
    pub fn shape_mesh_from_helpers(
        &self,
        mesh: &Mesh,
        helpers: &[Vec3],
        mhid_lookup: &[u16]
    ) -> Mesh {
        // Note that helpers should already be morphed before input so we don't have to apply weights
        let mut vertices = get_vertex_positions(mesh);
        for (vert, mh_asset_vertex) in mhid_lookup.iter().enumerate() {
            let helper_map = &self.helper_map[*mh_asset_vertex as usize];
            if let Some(mh_helper_vertex) = helper_map.single_vertex {
                vertices[vert] = helpers[mh_helper_vertex as usize];
            } else {
                // Triangulation
                let triangle = helper_map.triangle.as_ref().unwrap();
                let mut position = Vec3::ZERO;
                for i in 0..3 {
                    let mh_vert = triangle.helper_verts[i];
                    let wt = triangle.helper_weights[i];
                    if let Some(mh_helper_position) = helpers.get(mh_vert as usize) {
                        position += mh_helper_position * wt;
                    }
                }
                let offset = self.get_offset_scale(helpers) * triangle.helper_offset;
                vertices[vert] = position + offset;
            }
        }
        Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, get_uv_coords(&mesh))
        .with_inserted_indices(mesh.indices().unwrap().clone())
        .with_computed_area_weighted_normals()
        .with_generated_tangents()
        .unwrap()
    }
}

// Each vertex is mapped to either a single helper vertex
// or triangulated by 3 of them
#[derive(Default, Debug, Clone)]
pub(crate) struct HelperMap {
    pub(crate) single_vertex: Option<u16>,
    pub(crate) triangle: Option<Triangle>,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Triangle {
    pub(crate) helper_verts: [u16; 3],
    pub(crate) helper_weights: [f32; 3],
    pub(crate) helper_offset: Vec3,
}

#[derive(Default, Clone)]
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

#[derive(Resource, Deref, DerefMut)]
#[allow(dead_code)]
pub struct CharacterAssetRegistry(AHashMap<CharacterPart, CharacterAsset>);

impl FromWorld for CharacterAssetRegistry {
    fn from_world(world: &mut World) -> Self {
        let mut assets = AHashMap::<CharacterPart, CharacterAsset>::default();

        let config = world
            .get_resource_mut::<HumentityPathsConfig>()
            .expect("No global Humentity config loaded");

        // Body Parts
        for dir in &config.body_part_paths {
            let prefix = &dir.source_id.root_path;

            for entry in WalkDir::new(prefix.join(&dir.path))
                .into_iter()
                .filter_map(Result::ok)
            {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if extension == "mhclo" {
                    let folder = path.parent().expect("No parent folder?").to_path_buf();

                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .expect("Failed to parse file name");
                    info!("Importing body part : {name}");
                    let name = NAME_INTERNER.intern(name).leak();
                    let albedo_maps =
                        get_texture_paths(&folder, CharacterAssetTextureType::Albedo, &dir.source_id);
                    let normal_maps =
                        get_texture_paths(&folder, CharacterAssetTextureType::Normal, &dir.source_id);
                    let ao_maps = 
                        get_texture_paths(&folder, CharacterAssetTextureType::AmbientOcclusion, &dir.source_id);
                    let roughness_metallic_maps = 
                        get_texture_paths(&folder, CharacterAssetTextureType::AmbientOcclusion, &dir.source_id);
                    let mh_file = path.to_path_buf();

                    let mh_file = HumentityAssetPath {
                        path: mh_file,
                        source_id: dir.source_id.clone(),
                    };
                    let paths = CharacterMeshAssetFilePaths {
                        albedo_maps,
                        normal_maps,
                        ao_maps,
                        roughness_metallic_maps,
                        mh_file,
                    };
                    let part = CharacterPart::BodyPart(name);
                    let asset = CharacterAsset::new(part.clone(), paths);
                    assets.insert(part.clone(), asset);
                }
            }
        }

        // Equipment
        for dir in &config.equipment_paths {
            let prefix = &dir.source_id.root_path;

            for entry in WalkDir::new(prefix.join(&dir.path))
                .into_iter()
                .filter_map(Result::ok)
            {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if extension == "mhclo" {
                    let folder = path.parent().expect("No parent folder?").to_path_buf();

                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .expect("Failed to parse file name");
                    info!("Importing equipment : {name}");
                    let name = NAME_INTERNER.intern(name).leak();
                    let albedo_maps =
                        get_texture_paths(&folder, CharacterAssetTextureType::Albedo, &dir.source_id);
                    let normal_maps =
                        get_texture_paths(&folder, CharacterAssetTextureType::Normal, &dir.source_id);
                    let ao_maps =
                        get_texture_paths(&folder, CharacterAssetTextureType::AmbientOcclusion, &dir.source_id);
                    let roughness_metallic_maps = 
                        get_texture_paths(&folder, CharacterAssetTextureType::AmbientOcclusion, &dir.source_id);
                    let mh_file = path.to_path_buf();
                    let mh_file = HumentityAssetPath {
                        path: mh_file,
                        source_id: dir.source_id.clone(),
                    };
                    let paths = CharacterMeshAssetFilePaths {
                        albedo_maps,
                        normal_maps,
                        ao_maps,
                        roughness_metallic_maps,
                        mh_file,
                    };
                    let part = CharacterPart::Equipment(name);
                    let asset = CharacterAsset::new(part.clone(), paths);
                    assets.insert(part, asset);
                }
            }
        }

        let mut skin_albedo_maps = AHashMap::default();
        let mut skin_normal_maps = AHashMap::default();
        let mut skin_ao_maps = AHashMap::default();
        let mut skin_roughness_metallic_maps = AHashMap::default();

        for dir in &config.skin_texture_paths {
            let prefix = &dir.source_id.root_path;
            let path = prefix.join(&dir.path);
            skin_albedo_maps.extend(get_texture_paths(
                &path,
                CharacterAssetTextureType::Albedo,
                &dir.source_id,
            ));
            skin_normal_maps.extend(get_texture_paths(
                &path,
                CharacterAssetTextureType::Normal,
                &dir.source_id,
            ));
            skin_ao_maps.extend(get_texture_paths(
                &path,
                CharacterAssetTextureType::AmbientOcclusion,
                &dir.source_id,
            ));
            skin_roughness_metallic_maps.extend(get_texture_paths(
                &path,
                CharacterAssetTextureType::RoughnessMetallic,
                &dir.source_id,
            ));
        }

        // Proxy Meshes
        for dir in &config.proxymesh_paths {
            let prefix = &dir.source_id.root_path;
            for entry in WalkDir::new(prefix.join(&dir.path))
                .into_iter()
                .filter_map(Result::ok)
            {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if extension == "proxy" {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .expect("Failed to parse file name");
                    let name = NAME_INTERNER.intern(name).leak();
                    info!("Importing proxy mesh : {name}");
                    let mh_file = path.to_path_buf();
                    let mh_file = HumentityAssetPath {
                        path: mh_file,
                        source_id: dir.source_id.clone(),
                    };
                    let paths = CharacterMeshAssetFilePaths {
                        mh_file,
                        albedo_maps: skin_albedo_maps.clone(),
                        normal_maps: skin_normal_maps.clone(),
                        ao_maps: skin_ao_maps.clone(),
                        roughness_metallic_maps: skin_roughness_metallic_maps.clone(),
                    };
                    let part = CharacterPart::BodyMesh(name);
                    let asset = CharacterAsset::new(part.clone(), paths);
                    assets.insert(part, asset);
                }
            }
        }

        CharacterAssetRegistry(assets)
    }
}

fn get_texture_paths(
    path: &Path,
    texture_type: CharacterAssetTextureType,
    source_id: &HumentityAssetSourceId,
) -> AHashMap<&'static str, HumentityAssetPath> {
    let subfolders = match texture_type {
        CharacterAssetTextureType::Albedo => &ALBEDO_SUBFOLDERS.to_vec(),
        CharacterAssetTextureType::Normal => &NORMAL_SUBFOLDERS.to_vec(),
        CharacterAssetTextureType::AmbientOcclusion => &OCCLUSION_SUBFOLDERS.to_vec(),
        CharacterAssetTextureType::RoughnessMetallic => &ROUGHNESS_METALLIC_SUBFOLDERS.to_vec(),
    };
    let mut textures = AHashMap::default();

    for &folder in subfolders {
        let subfolder = path.join(folder);
        if !subfolder.exists() { continue; }

        for entry in std::fs::read_dir(subfolder).expect("Failed to read folder") {
            let Ok(entry) = entry else { continue };
            let path = entry.path();

            if path.is_file() {
                if let Some(stem) = path.file_stem() {
                    let stem = NAME_INTERNER.intern(stem.to_str().unwrap()).leak();
                    let prefix = &source_id.root_path;
                    let Ok(path) = path.strip_prefix(prefix) else {
                        continue;
                    };
                    let path = path.to_path_buf();
                    let asset_path = HumentityAssetPath {
                        path,
                        source_id: source_id.clone(),
                    };
                    textures.insert(stem, asset_path);
                }
            }
        }
    }
    textures
}

pub(crate) fn parse_character_asset(
    mh_path: &HumentityAssetPath,
) -> CharacterAssetData {
    let mut tags = Vec::<String>::new();
    let mut z_depth = 0_i8;
    let mut delete_verts = AHashSet::<u16>::default();
    let mut helper_map = Vec::<HelperMap>::new();
    let mut x_scale = ScaleData::default();
    let mut y_scale = ScaleData::default();
    let mut z_scale = ScaleData::default();
    let mut obj_file = PathBuf::default();
    let mut section = FileSection::Header;
    //let mut name = "";

    let source_id = &mh_path.source_id;
    let mh_path_buf = &mh_path.path;
    let err_msg = format!(
        "Couldn't open target file {}",
        mh_path_buf.to_string_lossy()
    );
    let file = File::open(mh_path_buf).expect(&err_msg);

    for line_result in BufReader::new(file).lines() {
        let Ok(line) = line_result else { break };
        if line.starts_with("#") {
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with("verts 0") {
            section = FileSection::Vertices;
            continue;
        }
        if line.starts_with("delete_verts") {
            section = FileSection::DeleteVertices;
            continue;
        }

        let line_vec: Vec<&str> = line.split_whitespace().collect();

        if section == FileSection::Header {
            if *line_vec.first().unwrap() == "obj_file" {
                let &filename = line_vec.last().unwrap();
                obj_file = mh_path.path.clone();
                obj_file.set_file_name(filename);
                obj_file = obj_file
                    .strip_prefix(&mh_path.source_id.root_path)
                    .expect("Invalid path prefix")
                    .to_path_buf();
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
            if line_vec[0] == "material" {
                continue;
            }
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
                helper_map.push(HelperMap {
                    triangle: Some(Triangle {
                        helper_verts,
                        helper_weights,
                        helper_offset,
                    }),
                    single_vertex: None,
                });
            } else if line_vec.len() == 1 {
                helper_map.push(HelperMap {
                    triangle: None,
                    single_vertex: Some(line.trim().parse().unwrap()),
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
                    let Some(s) = start else {
                        panic!("Failed to parse delete verts")
                    };
                    for i in s..=v.parse().unwrap() {
                        delete_verts.insert(i);
                    }
                    start = None;
                    grouping = false;
                } else if v != "-" {
                    if let Some(s) = start {
                        delete_verts.insert(s);
                    }
                    start = Some(v.parse().unwrap());
                } else {
                    grouping = true;
                }
            }

            // If there's a final start without a pairing, push it
            if let Some(s) = start {
                delete_verts.insert(s);
            }
        }
    }

    // Ignore.  Use file_stem instead
    //let name = name;
    let tags = tags
        .iter()
        .map(|n| NAME_INTERNER.intern(n).leak())
        .collect::<Vec<_>>();

    let obj_file = HumentityAssetPath {
        path: obj_file,
        source_id: source_id.clone(),
    };

    CharacterAssetData {
        obj_file,
        tags,
        z_depth,
        delete_verts,
        scale_data: [x_scale, y_scale, z_scale],
        helper_map,
    }
}

// MHCLO assets can define a list of delete verts which can be occluded (by clothes for example)
// This can stop the body mesh from popping through clothes, which can be an issue.
// If we were to bake out everything to a new mesh this would be a good optimization, but that
// breaks instancing so it would be a downgrade. If we could add a per material instance GPU buffer of
// indices to cull this would be better and wouldn't break instancing.
//#[allow(dead_code)]
//pub(crate) fn delete_mesh_verts(
//    meshes: &mut ResMut<Assets<Mesh>>,
//    base_mesh: &Res<crate::basemesh::BaseMesh>,
//    delete_verts: AHashSet<u16>,
//) -> Mesh {
//    let mesh = meshes.get(&base_mesh.mesh_handle).unwrap().clone();
//
//    let vertices = get_vertex_positions(&mesh);
//    let normals = get_vertex_normals(&mesh);
//    let uv = get_uv_coords(&mesh);
//    let indices = mesh.indices().expect("FAILED TO GET MESH FACES");
//
//    // Set up new storage for the new mesh
//    let verts = vertices.len() - delete_verts.len(); // Roughly
//    let mut new_vertices = Vec::<Vec3>::with_capacity(verts);
//    let mut new_normals = Vec::<Vec3>::with_capacity(verts);
//    let mut new_uv = Vec::<Vec2>::with_capacity(verts);
//    let mut new_indices = Vec::<u16>::with_capacity(verts);
//
//    // need to map new vertex indices to original before deleting verts
//    let mut indices_map = AHashMap::<u16, u16>::default();
//
//    for (vtx, &mh_vert) in base_mesh.mhid_lookup.iter().enumerate() {
//        if !delete_verts.contains(&mh_vert) {
//            indices_map.insert(vtx as u16, new_vertices.len() as u16);
//            new_vertices.push(vertices[vtx]);
//            new_normals.push(normals[vtx]);
//            new_uv.push(uv[vtx]);
//        }
//    }
//
//    let indices_vec: Vec<u16> = indices.iter().map(|x| x as u16).collect();
//    // Find new face indices
//    for face in indices_vec.chunks(3) {
//        // Check if all vertices still exist in new mesh verts
//        if !face.iter().all(|&i| indices_map.contains_key(&{ i })) {
//            continue;
//        }
//        // Map face to new vertex indices
//        new_indices.extend_from_slice(face);
//    }
//    new_indices = new_indices
//        .iter()
//        .map(|x| *indices_map.get(x).unwrap())
//        .collect();
//
//    let mut new_mesh = Mesh::new(
//        PrimitiveTopology::TriangleList,
//        RenderAssetUsages::RENDER_WORLD,
//    )
//    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, new_vertices)
//    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, new_normals)
//    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, new_uv)
//    .with_inserted_indices(Indices::U16(new_indices));
//    new_mesh.compute_area_weighted_normals();
//    new_mesh.generate_tangents().ok();
//    new_mesh
//}
//