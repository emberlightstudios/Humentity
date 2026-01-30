use ahash::AHashMap;
use bevy::prelude::*;
use serde::Deserialize;
use std::{fs::File, io::BufReader, sync::Arc};

use crate::prelude::*;
use crate::mesh_ops::parse_obj_vertices;

/// This is the last vertex from the body. Everything after is helper geometry
#[allow(dead_code)]
pub(crate) const BODY_VERTICES: u16 = 13380u16;
pub(crate) const BODY_SCALE: f32 = 0.1;

#[derive(Resource, Deserialize, Debug)]
pub(crate) struct VertexGroups(pub(crate) AHashMap<String, Vec<[usize; 2]>>);

#[derive(Resource, Clone)]
pub struct BaseMesh(pub Arc<Vec<Vec3>>);

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

        let file = File::open(path.join("basemesh_vertex_groups.json"))
            .expect("FAILED TO LOAD VERTEX GROUOPS");
        let reader = BufReader::new(file);
        let vg: VertexGroups = serde_json::from_reader(reader).unwrap();

        world.insert_resource(vg);

        BaseMesh(Arc::new(mh_vertices))
    }
}
