use::bevy::prelude::*;
use::std::{
    io::{ File, BufReader, },
    path::PathBuf,
};

 pub(crate) struct Mhclo {
    path: PathBuf,
    obj_file: PathBuf,
    tags: Vec<String>,
    z_depth: i8,
    helper_verts: Vec<[u16; 3]>,
    helper_weights: Vec<[f32; 3]>,
    helper_offsets: Vec<[f32; 3]>,
    delete_verts: Vec<u16>,
    scale_data: [ScaleData; 3],
 }

 struct ScaleData {
    v1: u16,
    v2: u16,
    scale: f32,
 }

 enum FileSection {
    Header,
    Vertices,
    DeleteVertices,
 }

 pub enum BodyPartSlot {
    Eyes,
    Tongue,
    Teeth,
    Eyelashes,
    Eyebrows,
    Hair,
 }

 pub enum EquipmentSlot {
    Torso,
 }

/*-------------+
 |  Resources  |
 +-------------*/
 #[derive(Resource)]
 pub(crate) struct HumanAssetRegistry {
    body_parts: HashMap<BodyPartSlot, Vec<Mhclo>>,
    equipment: HashMap<EquipmentSlot, Vec<Mhclo>>,
 }

 impl FromWorld for Mhclo {
    fn from_world(world: &mut World) {

    }
 }