use::bevy::{
    prelude::*,
    render::{
        mesh::{
            PrimitiveTopology,
            Indices,
        },
        render_asset::RenderAssetUsages,
    },
};
use::std::{
    io::{ BufRead, BufReader, },
    fs::File,
    path::PathBuf,
    collections::{ HashMap, HashSet },
};
use walkdir::WalkDir;
use crate::{
    generate_vertex_map,
    generate_inverse_vertex_map,
    get_vertex_positions,
    parse_obj_vertices,
    get_vertex_normals, 
    get_uv_coords,
    HumentityGlobalConfig,
    HumentityState,
    LoadingPhase,
    LoadingState,
    BaseMesh,
};

/*---------+
 |  Types  |
 +---------*/
 #[allow(dead_code)]
pub struct HumanMeshAsset {
   pub name: String,
   pub(crate) mesh_handle: Handle<Mesh>,
   pub(crate) helper_maps: Vec<HelperMap>,
   pub(crate) vertex_map: HashMap<u16, Vec<u16>>,
   pub(crate) delete_verts: HashSet<u16>,
   pub slots: Vec<String>,
   obj_file: PathBuf,
   tags: Vec<String>,
   z_depth: i8,
   scale_data: [ScaleData; 3],
}

impl HumanMeshAsset {
    pub(crate) fn get_offset_scale(&self, helpers: &Vec<Vec3>) -> Vec3 {
        Vec3::new(
            (helpers[self.scale_data[0].max as usize] - helpers[self.scale_data[0].min as usize]).x / self.scale_data[0].scale,
            (helpers[self.scale_data[1].max as usize] - helpers[self.scale_data[1].min as usize]).y / self.scale_data[1].scale,
            (helpers[self.scale_data[2].max as usize] - helpers[self.scale_data[2].min as usize]).z / self.scale_data[2].scale,
        )
    }
}

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
#[allow(dead_code)]
#[derive(Resource)]
pub struct HumanAssetTextures {
    pub albedo_maps: HashMap<String, Vec<Handle<Image>>>,
    pub normal_map: HashMap<String, Handle<Image>>,
    pub ao_map: HashMap<String, Handle<Image>>,
}

#[allow(dead_code)]
#[derive(Resource)]
pub struct HumanAssetRegistry {
    pub body_parts: HashMap<String, HumanMeshAsset>,
    pub equipment: HashMap<String, HumanMeshAsset>,
    pub slot_body_parts: HashMap<String, Vec<String>>,
    pub slot_equipment: HashMap<String, Vec<String>>,
}

impl FromWorld for HumanAssetRegistry {
    fn from_world(world: &mut World) -> Self{
        let mut body_parts = HashMap::<String, HumanMeshAsset>::new();
        let mut equipment = HashMap::<String, HumanMeshAsset>::new();
        let mut slot_body_parts = HashMap::<String, Vec<String>>::new();
        let mut slot_equipment = HashMap::<String, Vec<String>>::new();

        let config = world.get_resource_mut::<HumentityGlobalConfig>().expect("No global Humentity config loaded");
        let body_part_paths = config.body_part_paths.clone();
        let equipment_paths = config.equipment_paths.clone();
        let body_part_slots = config.body_part_slots.clone();
        let equipment_slots = config.body_part_slots.clone();

        for dir in body_part_paths {
            for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                if !path.is_file() { continue; }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else { continue };
                if extension == "mhclo" {
                    // parse
                    let mut bp = parse_human_asset(path.to_path_buf(), world);
                    // set slots
                    let mut slots = Vec::<String>::new();
                    for tag in &bp.tags {
                        if body_part_slots.contains(tag) { slots.push(tag.to_string()) };
                    }
                    bp.slots = slots.clone();
                    // insert into slots hashmap
                    for slot in slots.iter() {
                        let bp_vec = slot_body_parts.entry(slot.to_string()).or_insert(Vec::<String>::new());
                        bp_vec.push(bp.name.clone());
                    }
                    // insert into name hashmap
                    body_parts.insert(bp.name.clone(), bp);
                }
            }
        }

        for dir in equipment_paths {
            for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                //let stem = path.file_stem().unwrap().to_str().unwrap();
                //if stem.eq_ignore_ascii_case("eyes") { slot = BodyPartSlot::Eyes; }
                if !path.is_file() { continue; }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else { continue };
                if extension == "mhclo" {
                    let eq = parse_human_asset(path.to_path_buf(), world);
                    equipment.insert(eq.name.clone(), eq);
                }
            }
        }

        // Load textures
        // It is assumed:
        // normal maps end with _normal.png
        // ao maps with _ao.png
        // all else are albedo maps
        let mut albedo_textures = HashMap::<String, Vec<Handle<Image>>>::new();
        let mut normal_texture = HashMap::<String, Handle<Image>>::new();
        let mut ao_texture = HashMap::<String, Handle<Image>>::new();
        let Some(asset_server) = world.get_resource::<AssetServer>() else { panic!("Can't load asset server?") };
        for (name, asset) in equipment.iter().chain(body_parts.iter()) {
            let dir = asset.obj_file.parent().unwrap();
            let mut asset_albedos = Vec::<Handle<Image>>::new();
            for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
                let path = entry.path().to_path_buf();
                if path.is_file() {
                    let Some(extension) = path.extension().and_then(|e| e.to_str()) else { continue };
                    if extension == "png" {
                        let image = asset_server.load(path.clone());
                        if let Some(file) = path.file_name().and_then(|s| s.to_str()) {
                            if file.ends_with("_bump.png") { continue; }
                            if !file.starts_with("overlay_") {
                                if file.ends_with("_normal.png") { normal_texture.insert(name.to_string(), image); }
                                else if file.ends_with("_ao.png") { ao_texture.insert(name.to_string(), image); }
                                else { asset_albedos.push(image); }
                            }
                        }
                    }
                }
            }
            albedo_textures.insert(name.to_string(), asset_albedos);
        }
        world.insert_resource(HumanAssetTextures{ 
            albedo_maps: albedo_textures,
            normal_map: normal_texture, 
            ao_map: ao_texture, 
        });

        HumanAssetRegistry {
            body_parts: body_parts,
            equipment: equipment,
            slot_body_parts: slot_body_parts,
            slot_equipment: slot_equipment,
        }
    }
}

/*-----------+
 |  Systems  |
 +-----------*/
 pub(crate) fn generate_asset_vertex_maps(
    mut registry: ResMut<HumanAssetRegistry>,
    mut loading_state: ResMut<LoadingState>,
    meshes: Res<Assets<Mesh>>,
 ) {
    if *loading_state.0.get(&LoadingPhase::GenerateAssetVertexMap).unwrap() { return };
    for (_name, asset) in registry.body_parts.iter_mut() {
        let Some(_mesh) = meshes.get(&asset.mesh_handle) else { return };
    }
    for (_name, asset) in registry.equipment.iter_mut() {
        let Some(_mesh) = meshes.get(&asset.mesh_handle) else { return };
    }

    for (name, asset) in registry.body_parts.iter_mut() {
        //println!("Importing body part: {name}");
        let mh_verts = parse_obj_vertices(&asset.obj_file);
        let mesh = meshes.get(&asset.mesh_handle).unwrap();
        let verts = get_vertex_positions(&mesh);
        let vertex_map = generate_vertex_map(&mh_verts, &verts);
        asset.vertex_map = vertex_map;
    }
    for (name, asset) in registry.equipment.iter_mut() {
        //println!("Importing equipment: {name}");
        let mh_verts = parse_obj_vertices(&asset.obj_file);
        let mesh = meshes.get(&asset.mesh_handle).unwrap();
        let verts = get_vertex_positions(&mesh);
        let vertex_map = generate_vertex_map(&mh_verts, &verts);
        asset.vertex_map = vertex_map;
    }
    loading_state.0.insert(LoadingPhase::GenerateAssetVertexMap, true);
 }

/*------------+
 |  Funtions  |
 +------------*/
 fn parse_human_asset(path: PathBuf, world: &mut World) -> HumanMeshAsset {
    let mut tags = Vec::<String>::new();
    let mut z_depth = 0 as i8;
    let mut delete_verts = HashSet::<u16>::new();
    let mut helper_map = Vec::<HelperMap>::new();
    let mut x_scale = ScaleData::default();
    let mut y_scale = ScaleData::default();
    let mut z_scale = ScaleData::default();
    let mut name: String = "".to_string();
    
    let mut obj_file = PathBuf::default();
    let asset_server = world.get_resource::<AssetServer>().unwrap();
    let mut section = FileSection::Header;


    let err_msg = format!("Couldn't open target file {}", path.to_string_lossy());
    let file = File::open(&path).expect(&err_msg);
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
                obj_file = path.clone();
                obj_file.set_file_name(filename);
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
                name = line_vec.last().unwrap().to_string();
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
                    single_vertex: Some(line.parse().unwrap())
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

    let mesh_handle = asset_server.load(obj_file.clone());
    let vertex_map = HashMap::<u16, Vec<u16>>::new();

    HumanMeshAsset {
        name: name,
        obj_file: obj_file,
        tags: tags,
        z_depth: z_depth,
        helper_maps: helper_map,
        delete_verts: delete_verts,
        scale_data: [x_scale, y_scale, z_scale],
        mesh_handle: mesh_handle,
        vertex_map: vertex_map,
        slots: vec![],
    }
}

pub(crate) fn delete_mesh_verts(
    meshes: &mut ResMut<Assets<Mesh>>,
    base_mesh: &Res<BaseMesh>,
    delete_verts: HashSet<u16>,
) -> Mesh {
    let mesh = meshes.get(&base_mesh.mesh_handle).unwrap().clone();
    let inv_vertex_map = generate_inverse_vertex_map(&base_mesh.vertex_map);

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
    let mut indices_map = HashMap::<u16, u16>::with_capacity(verts);

    for (&vtx, &mh_vert) in inv_vertex_map.iter() {
        if !delete_verts.contains(&mh_vert) {
            indices_map.insert(vtx, new_vertices.len() as u16);
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
    new_mesh.compute_smooth_normals();
    let _ = new_mesh.generate_tangents();
    new_mesh
}