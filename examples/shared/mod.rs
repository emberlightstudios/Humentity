#![allow(dead_code)]
use bevy::{
    asset::AssetPlugin,
    dev_tools::fps_overlay::FpsOverlayPlugin,
    input::mouse::MouseMotion,
    mesh::{MeshVertexBufferLayoutRef, morph::MeshMorphWeights, skinning::SkinnedMesh},
    pbr::{
        ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
        MaterialPlugin,
    },
    prelude::*,
    render::{render_resource::*, storage::ShaderBuffer},
    shader::ShaderRef,
    window::{PresentMode, Window, WindowPlugin},
};
use bevy_egui::prelude::*;
use bevy_inspector_egui::quick::WorldInspectorPlugin;
use humentity::HumentityPlugin;
use humentity::prelude::*;

/// Default skeleton LOD configurations.
///
/// LOD 0: Merge toes into a single toe bone per foot.
/// LOD 1: Remove face,
/// LOD 2: Remove hands, fingers, feet
pub fn default_skeleton_lods() -> Vec<BoneMergeConfig> {
    // Merge toe bones into a single posable toe per foot
    let lod0 = BoneMergeConfig::full().merge_default_rig_toes();

    let lod1 = lod0.clone().without_children_of(&["head"]);

    let lod2 = lod1.clone().without_children_of(&[
        "lowerarm02.L",
        "lowerarm02.R",
        "lowerleg02.L",
        "lowerleg02.R",
    ]);

    vec![lod0, lod1, lod2]
}

/// The single full skeleton every GPU crowd example poses: no bones merged away.
pub const fn gpu_skeleton() -> BoneMergeConfig {
    BoneMergeConfig::full()
}

/// Index of the single crowd skeleton in [`SkeletonLodConfig`].
pub const GPU_SKELETON_LOD: usize = 0;

/// Camera framing for the shared `setup_env`: `Close` frames a single
/// character at the origin (CPU examples), `Far` frames the wide crowd
/// centered at z = 8 (GPU crowd examples).
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraFraming {
    Close,
    Far,
}

/// Builds the `App` every GPU crowd example shares: humentity assets live at
/// the crate's `assets/` dir, GPU plugin + crowd material are wired,
/// and the single [`gpu_skeleton`] is registered at
/// [`GPU_SKELETON_LOD`].
pub fn setup_app_gpu(instances: usize, sample_rate: f32, framing: CameraFraming) -> App {
    let asset_dir: String = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .to_string_lossy()
        .into_owned();
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_dir,
                ..default()
            })
            // No vsync so FPS reads true frame cost, not the display refresh.
            .set(WindowPlugin {
                primary_window: Some(Window {
                    present_mode: PresentMode::Immediate,
                    ..default()
                }),
                ..default()
            }),
        HumentityPlugin::default(),
        HumentityGpuPlugin {
            instances,
            sample_rate,
            ..default()
        },
        MaterialPlugin::<CustomCrowdMaterial>::default(),
    ));
    app.insert_resource(SkeletonLodConfig::new(&[gpu_skeleton()]));
    app.insert_resource(GpuSkeletonLod(GPU_SKELETON_LOD));
    app.insert_resource(framing);
    app.add_plugins(FpsOverlayPlugin::default());
    app.add_systems(Startup, (load_core_assets, setup_env));
    app.add_systems(Update, cam_controls);
    app
}

/// Core humentity assets every GPU crowd example needs, loaded from the
/// crate's `assets/` dir.
fn load_core_assets(asset_server: Res<AssetServer>, mut commands: Commands) {
    load_and_insert_humentity_assets(
        &mut commands,
        &asset_server,
        "base.obj",
        "basemesh_vertex_groups.json",
        "target.json",
        "macro.macro",
        "targets",
        "rigs/rig.default.json",
        "rigs/weights.default.json",
        "skeletons/default.glb",
    );
}

pub fn setup_app() -> App {
    // I moved target.json and macro.macro to the root of the assets folder because when trying to load
    // the target folders, the asset server tried to load them there also.

    let mut app = App::new();

    let asset_dir: String = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .to_string_lossy()
        .into_owned();
    app.add_plugins((
        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_dir,
                ..default()
            })
            // No vsync so FPS reads true frame cost, not the display refresh.
            .set(WindowPlugin {
                primary_window: Some(Window {
                    present_mode: PresentMode::Immediate,
                    ..default()
                }),
                ..default()
            }),
        HumentityPlugin,
    ))
    .add_plugins((EguiPlugin::default(), WorldInspectorPlugin::new()))
    .add_plugins(FpsOverlayPlugin::default())
    .insert_resource(SkeletonLodConfig::new(&default_skeleton_lods()))
    .insert_resource(CameraFraming::Close)
    .add_systems(Startup, load_assets)
    .add_systems(Startup, setup_env)
    .add_systems(
        Update,
        (
            enable_first_skeleton_on_ready,
            update_mesh_when_ready,
            cam_controls,
            add_material,
            //debug_forward_gizmo,
        ),
    );

    app
}

/// These assets are necessary to get the plugin to work.
fn load_assets(asset_server: Res<AssetServer>, mut commands: Commands) {
    load_and_insert_humentity_assets(
        &mut commands,
        &asset_server,
        "base.obj",
        "basemesh_vertex_groups.json",
        "target.json",
        "macro.macro",
        "targets",
        "rigs/rig.default.json",
        "rigs/weights.default.json",
        "skeletons/default.glb",
    );
}

/// Enables the first skeleton LOD (lowest detail) when a character's skeleton
/// has been fitted. With a single fixed skeleton, this is expressed by writing
/// `SkeletonLodState` with only LOD 0 active.
pub fn enable_first_skeleton_on_ready(
    characters: Query<Entity, Added<SkeletonReady>>,
    mut commands: Commands,
) {
    for entity in &characters {
        let mut active = [false; MAX_LODS];
        active[0] = true;
        commands.entity(entity).insert(SkeletonLodState { active });
    }
}

/// Scans for CharacterParts still waiting for a mesh handle from the background threads
fn update_mesh_when_ready(
    character_parts: Query<
        (
            Entity,
            &ChildOf,
            &CharacterPart,
            Option<&SkinnedMesh>,
            Option<&Transform>,
        ),
        Without<Mesh3d>,
    >,
    parents: Query<&ChildOf>,
    characters: Query<&CharacterShape>,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    template_overrides: Query<&TemplateOverride>,
    cached_meshes: Res<CachedMhcloMeshHandles>,
    meshes: Res<Assets<Mesh>>,
    template_assets: Res<Assets<CharacterTemplate>>,
    mut commands: Commands,
) {
    for (entity, _, part, part_skm, xform) in character_parts.iter() {
        // Parts may sit under intermediate grouping nodes (e.g. a "CPU Meshes"
        // or "GPU Meshes" child), so walk up until the CharacterShape owner.
        // `iter_ancestors` yields the direct parent first, so flat parts work too.
        let character_shape = parents
            .iter_ancestors::<ChildOf>(entity)
            .find_map(|ancestor| characters.get(ancestor).ok());
        let Some(character_shape) = character_shape else {
            continue;
        };
        let Some(asset) = shape_assets.get(&character_shape.0) else {
            continue;
        };
        let template = &asset.template;

        // SkinnedMesh must be placed on the CharacterPart by setup_part_skinning first
        let Some(skm) = part_skm else {
            continue;
        };

        // Stitched parts may carry a TemplateOverride: the builder caches under
        // the effective template, so the lookup must use it too.
        let effective_template = template_overrides.get(entity).ok().map_or_else(
            || template.clone(),
            |override_template| override_template.0.clone(),
        );
        if let Some(mesh_handle) = cached_meshes.get(&(
            part.mesh.clone(),
            effective_template.clone(),
            MeshBuildLod::Cpu(part.skeleton_lod),
        )) {
            commands.entity(entity).insert((
                Mesh3d(mesh_handle.clone()),
                xform.copied().unwrap_or_default(),
                skm.clone(),
            ));
            let mesh = meshes.get(mesh_handle).unwrap();
            if mesh.has_morph_targets() {
                let active_template = effective_template.clone();
                if let Some(template_data) = template_assets.get(&active_template) {
                    let morph_weights = template_data
                        .shapes
                        .iter()
                        .map(|s| *asset.template_morph_targets.get(s.name).unwrap_or(&0.))
                        .collect::<Vec<_>>();
                    commands.entity(entity).insert(MeshMorphWeights::Value {
                        weights: morph_weights,
                    });
                }
            }
        }
    }
}

pub fn cam_controls(
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    kb_input: Res<ButtonInput<KeyCode>>,
    mut _pitch: Local<f32>,
    mut _yaw: Local<f32>,
    mut init: Local<bool>,
) {
    if !*init {
        *init = true;
        *_yaw = std::f32::consts::PI;
    }
    const MS: f32 = 1e-1;
    const LS: f32 = 5e-3;
    let Ok(transform) = cam.single().cloned() else {
        return;
    };
    let Ok(mut cam) = cam.single_mut() else {
        return;
    };
    for _ev in mouse_motion.read() {
        //*_yaw -= ev.delta.x * LS;
        //*_pitch -= ev.delta.y * LS;
    }
    cam.rotation = Quat::from_euler(EulerRot::YXZ, *_yaw, *_pitch, 0.);
    let mut mv = Vec3::ZERO;
    if kb_input.pressed(KeyCode::KeyD) {
        mv.x += MS
    }
    if kb_input.pressed(KeyCode::KeyA) {
        mv.x -= MS
    }
    if kb_input.pressed(KeyCode::KeyS) {
        mv.z += MS
    }
    if kb_input.pressed(KeyCode::KeyW) {
        mv.z -= MS
    }
    if kb_input.pressed(KeyCode::KeyQ) {
        mv.y -= MS
    }
    if kb_input.pressed(KeyCode::KeyE) {
        mv.y += MS
    }
    cam.translation += Transform::from_rotation(transform.rotation) * mv;
}

pub fn add_material(
    humans: Query<Entity, (With<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for human in humans.iter() {
        let mat = materials.add(StandardMaterial::from_color(Color::BLACK));
        commands.entity(human).insert(MeshMaterial3d(mat));
    }
}

fn debug_forward_gizmo(
    characters: Query<&GlobalTransform, With<CharacterShape>>,
    mut gizmos: Gizmos,
) {
    for gt in &characters {
        let origin = gt.translation();
        let forward = gt.forward().as_vec3();
        gizmos.line(origin, origin + forward * 2.0, Color::srgb(0.0, 1.0, 0.0));
    }
}

pub fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    framing: Res<CameraFraming>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(80.0, 80.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.2, 0.2, 0.25))),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 3000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(10.0, 20.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Close frames a single character at the origin (same 4m-back, 1m-up
    // framing as the old gpu_morphs reframe, retargeted to the origin);
    // Far keeps the wide-crowd framing centered at z = 8.
    let camera = match *framing {
        CameraFraming::Close => {
            Transform::from_xyz(0.0, 1.0, -4.0).looking_at(Vec3::new(0.0, 0.7, 0.0), Vec3::Y)
        }
        CameraFraming::Far => {
            Transform::from_xyz(0.0, 18.0, -30.0).looking_at(Vec3::new(0.0, 1.0, 8.0), Vec3::Y)
        }
    };
    commands.spawn((Camera3d::default(), camera));
}

/// Material extension for examples with their own GPU crowd vertex shader.
/// Skinning itself always comes from the shared `humentity::crowd_skin`
/// module.
pub type CustomCrowdMaterial = ExtendedMaterial<StandardMaterial, CustomCrowdExtension>;

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct CustomCrowdExtension {
    #[storage(100, read_only)]
    joints: Handle<ShaderBuffer>,
    #[uniform(101)]
    crowd: GpuCrowdUniform,
}

impl MaterialExtension for CustomCrowdExtension {
    fn vertex_shader() -> ShaderRef {
        CROWD_FORWARD_SHADER.into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        ShaderRef::Default
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_gpu_vertex_layout(descriptor, layout)
    }
}

/// Builds the example crowd material from the baked render handles.
pub fn custom_crowd_material(
    handles: &GpuRenderHandles,
    materials: &mut Assets<CustomCrowdMaterial>,
) -> Handle<CustomCrowdMaterial> {
    materials.add(ExtendedMaterial {
        base: StandardMaterial::from_color(Color::WHITE),
        extension: CustomCrowdExtension {
            joints: handles.joints.clone(),
            crowd: GpuCrowdUniform {
                num_bones: handles.num_bones,
                pad_a: 0,
                pad_b: 0,
                pad_c: 0,
            },
        },
    })
}
