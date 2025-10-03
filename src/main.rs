use bevy::{gltf::GltfMesh, mesh::{MeshVertexAttribute, MeshVertexAttributeId, VertexAttributeValues, VertexFormat}, prelude::*};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, load_mesh)
        .add_systems(Update, read_mesh.run_if(resource_exists::<GltfHandle>))
        .run();
}

#[derive(Resource)]
struct GltfHandle(Handle<Gltf>);

fn load_mesh(
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let handle = asset_server.load("base.glb");
    commands.insert_resource(GltfHandle(handle));
}

fn read_mesh(
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
    handle: Res<GltfHandle>,
) {
    let Some(gltf) = gltfs.get(&handle.0) else { return };
    let x = &gltf.meshes;
    for m in x {
        let gm: &GltfMesh = gltf_meshes.get(m).unwrap();
        let x = &gm.primitives;
        for p in x {
            let x = &p;
            // find mhid vertex buffer?
            let m = meshes.get(&x.mesh).unwrap();
            const ATTRIBUTE_MH_ID: MeshVertexAttribute =
                MeshVertexAttribute::new("MH_ID", Mesh::FIRST_AVAILABLE_CUSTOM_ATTRIBUTE + 1, VertexFormat::Uint32);
            let x = m.attribute(ATTRIBUTE_MH_ID);
            println!("{}", x.is_some());

        }
    }
    println!("")
}