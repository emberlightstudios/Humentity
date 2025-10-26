use bevy::{prelude::*, scene::SceneInstanceReady};

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.add_systems(Startup, setup);
    app.add_systems(Update, update);
    app.run();
}

#[derive(Resource)]
struct MyHandle(Handle<DynamicScene>);

fn setup(
    world: &mut World,
) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let mut meshes = world.resource_mut::<Assets<Mesh>>();
    let mesh = meshes.add(Cuboid::default().mesh());
    let mut ds = world.resource_mut::<Assets<DynamicScene>>();
    let scene = get_scene(registry, mesh);
    let handle = ds.add(scene);
    world.insert_resource(MyHandle(handle));
}

fn get_scene(registry: AppTypeRegistry, mesh_handle: Handle<Mesh>) -> DynamicScene {
    let mut w = World::new();
    w.insert_resource(registry);
    w.spawn((
        Name::new("Test"),
        Mesh3d(mesh_handle)
    ));
    DynamicScene::from_world(&w)
}

fn update(
    handle: Option<Res<MyHandle>>,
    names: Query<&Name>,
    meshes: Query<&Mesh3d>,
    mut commands: Commands
) {
    info!("{}", meshes.count());
    if let Some(handle) = handle {
        commands.spawn(DynamicSceneRoot::from(handle.0.clone()))
            .observe(test_obs);
    }
}

fn test_obs(trigger: On<SceneInstanceReady>) {
    info!("SCENE READY!");
}