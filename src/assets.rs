use::bevy::prelude::*;
use::std::{
    io::{ BufRead, BufReader, },
    fs::File,
    path::PathBuf,
    collections::{ HashMap, HashSet },
};
use walkdir::WalkDir;
use crate::{
    generate_vertex_map,
    get_vertex_positions,
    parse_obj_vertices,
    HumentityGlobalConfig,
    HumentityState,
};

/*---------+
 |  Types  |
 +---------*/
 #[allow(dead_code)]
pub(crate) struct HumanMeshAsset {
   name: String,
   obj_file: PathBuf,
   tags: Vec<String>,
   z_depth: i8,
   scale_data: [ScaleData; 3],
   pub(crate) delete_verts: HashSet<u16>,
   pub(crate) helper_maps: Vec<HelperMap>,
   pub(crate) mesh_handle: Handle<Mesh>,
   pub(crate) vertex_map: HashMap<u16, Vec<u16>>,
}

impl HumanMeshAsset {
    pub(crate) fn get_scale(&self, helpers: &Vec<Vec3>) -> Vec3 {
        Vec3::new(
            (helpers[self.scale_data[0].v1 as usize] - helpers[self.scale_data[0].v2 as usize]).x / self.scale_data[0].scale,
            (helpers[self.scale_data[1].v1 as usize] - helpers[self.scale_data[1].v2 as usize]).y / self.scale_data[1].scale,
            (helpers[self.scale_data[2].v1 as usize] - helpers[self.scale_data[2].v2 as usize]).z / self.scale_data[2].scale
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
   v1: u16,
   v2: u16,
   scale: f32,
}

#[derive(Eq, PartialEq)]
enum FileSection {
   Header,
   Vertices,
   DeleteVertices,
}

#[allow(dead_code)]
#[derive(Eq, PartialEq, Hash, Copy, Clone, Debug)]
pub enum BodyPartSlot {
   Eyes,
   Tongue,
   Teeth,
   Eyelashes,
   Eyebrows,
   Hair,
}

#[allow(dead_code)]
#[derive(Eq, PartialEq, Hash, Copy, Clone, Debug)]
pub enum EquipmentSlot {
   Torso,
}

/*-------------+
 |  Resources  |
 +-------------*/

#[allow(dead_code)]
#[derive(Resource)]
pub(crate) struct HumanAssetRegistry {
    pub(crate) body_parts: HashMap<String, HumanMeshAsset>,
    pub(crate) equipment: HashMap<String, HumanMeshAsset>,
}

impl FromWorld for HumanAssetRegistry {
    fn from_world(world: &mut World) -> Self{
        let mut body_parts = HashMap::<String, HumanMeshAsset>::new();
        let mut equipment = HashMap::<String, HumanMeshAsset>::new();

        let config = world.get_resource_mut::<HumentityGlobalConfig>().expect("No global Humentity config loaded");
        let body_part_paths = config.body_part_paths.clone();
        let equipment_paths = config.equipment_paths.clone();

        for dir in body_part_paths {
            for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                //let stem = path.file_stem().unwrap().to_str().unwrap();
                //if stem.eq_ignore_ascii_case("eyes") { slot = BodyPartSlot::Eyes; }
                if !path.is_file() { continue; }
                let Some(extension) = path.extension().and_then(|e| e.to_str()) else { continue };
                if extension == "mhclo" {
                    let bp = parse_human_asset(path.to_path_buf(), world);
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

        HumanAssetRegistry {
            body_parts: body_parts,
            equipment: equipment,
        }
    }
}

/*-----------+
 |  Systems  |
 +-----------*/
 pub(crate) fn generate_asset_vertex_maps(
    mut registry: ResMut<HumanAssetRegistry>,
    mut next: ResMut<NextState<HumentityState>>,
    meshes: Res<Assets<Mesh>>,
 ) {
    for (_name, asset) in registry.body_parts.iter_mut() {
        let mh_verts = parse_obj_vertices(&asset.obj_file);
        let mesh = meshes.get(&asset.mesh_handle).unwrap();
        let verts = get_vertex_positions(&mesh);
        let vertex_map = generate_vertex_map(&mh_verts, &verts);
        asset.vertex_map = vertex_map;
    }
    for (_name, asset) in registry.equipment.iter_mut() {
        let mh_verts = parse_obj_vertices(&asset.obj_file);
        let mesh = meshes.get(&asset.mesh_handle).unwrap();
        let verts = get_vertex_positions(&mesh);
        let vertex_map = generate_vertex_map(&mh_verts, &verts);
        asset.vertex_map = vertex_map;
    }
    next.set(HumentityState::Ready);
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
                x_scale.v1 = line_vec[1].parse().unwrap();
                x_scale.v2 = line_vec[2].parse().unwrap();
                x_scale.scale = line_vec[3].parse().unwrap();
            } else if *line_vec.first().unwrap() == "y_scale" {
                y_scale.v1 = line_vec[1].parse().unwrap();
                y_scale.v2 = line_vec[2].parse().unwrap();
                y_scale.scale = line_vec[3].parse().unwrap();
            } else if *line_vec.first().unwrap() == "z_scale" {
                z_scale.v1 = line_vec[1].parse().unwrap();
                z_scale.v2 = line_vec[2].parse().unwrap();
                z_scale.scale = line_vec[3].parse().unwrap();
            } else if *line_vec.first().unwrap() == "z_depth" {
                z_depth = line_vec[1].parse().unwrap();
            } else if *line_vec.first().unwrap() == "tag" {
                tags.push(line_vec.last().unwrap().to_string());
            } else if *line_vec.first().unwrap() == "name" {
                name = line_vec.last().unwrap().to_string();
            }
        } else if section == FileSection::Vertices {
            if line_vec.len() == 9 {
                let helper_verts = [
                    line_vec[0].parse().unwrap(),
                    line_vec[1].parse().unwrap(),
                    line_vec[2].parse().unwrap(),
                ];
                let helper_weights = [
                    line_vec[3].parse().unwrap(),
                    line_vec[4].parse().unwrap(),
                    line_vec[5].parse().unwrap(),
                ];
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
            } else { panic!("Unparseable vertex line") }
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
    }
 }