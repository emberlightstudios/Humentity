//! Realtime mesh modification example
//!
//! The process is slow.  Unfortunately bevy has a hard limit on the number of morphs a mesh may have,
//! so we have to rebuild the mesh in real time and it is slow.  Not sure if it can be made faster.
//!

mod shared;

use ahash::AHashMap;
use bevy::feathers::FeathersPlugins;
use bevy::feathers::controls::FeathersButton;
use bevy::feathers::controls::FeathersSlider;
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::ThemedText;
use bevy::feathers::theme::UiTheme;
use bevy::prelude::*;
use bevy::scene::CommandsSceneExt;
use bevy::tasks::AsyncComputeTaskPool;
use bevy::ui_widgets::Activate;
use bevy::ui_widgets::Slider;
use bevy::ui_widgets::SliderPrecision;
use bevy::ui_widgets::SliderStep;
use bevy::ui_widgets::ValueChange;
use bevy::ui_widgets::slider_self_update;
use humentity::prelude::*;
use shared::setup_app;
use std::sync::Arc;

#[derive(Component, Clone, Default, Deref)]
struct ButtonCategory(&'static str);

impl ButtonCategory {
    const fn new(s: &'static str) -> Self {
        Self(s)
    }
}

#[derive(Component, Clone, Default)]
struct SliderMetadata(&'static str, &'static str);

impl SliderMetadata {
    const fn new(a: &'static str, b: &'static str) -> Self {
        Self(a, b)
    }
}

#[derive(Component)]
struct RootNode;

#[derive(Resource)]
struct UiInitialized;

#[derive(Resource, DerefMut, Deref)]
struct SliderValues(ahash::AHashMap<&'static str, MorphTargets>);

impl Default for SliderValues {
    fn default() -> Self {
        let mut instance = Self(Default::default());
        instance.insert_value("macro", "age", 0.5);
        instance.insert_value("macro", "gender", 1.);
        instance.insert_value("macro", "caucasian", 1.);
        instance.insert_value("macro", "height", 0.5);
        instance.insert_value("macro", "cupsize", 0.5);
        instance.insert_value("macro", "firmness", 0.5);
        instance.insert_value("macro", "weight", 0.5);
        instance.insert_value("macro", "muscle", 0.5);
        instance.insert_value("macro", "proportions", 0.5);
        instance
    }
}

impl SliderValues {
    fn insert_value(&mut self, category: &'static str, name: &'static str, value: f32) {
        let category = self.entry(category).or_default();
        category.insert(name, value);
    }
}

/// Ordered category priority — categories not in this list appear at the end.
const CATEGORY_ORDER: &[&str] = &[
    "macro",
    "head",
    "forehead",
    "eyes",
    "eyebrows",
    "nose",
    "mouth",
    "cheek",
    "chin",
    "ears",
    "neck",
    "torso",
    "breast",
    "stomach",
    "pelvis",
    "buttocks",
    "arms",
    "hands",
    "legs",
    "feet",
    "asymmetry",
    "expressions",
];

/// Categories to hide from the UI.
const CATEGORY_BLOCKLIST: &[&str] = &["genitals", "measure", "unsorted"];

struct BuildResult {
    mesh: Mesh,
}

/// Cached asset handles and derived data for direct mesh building
#[derive(Resource)]
struct CreatorAssets {
    mhclo: Handle<MhcloAsset>,
    input_mesh: Handle<Mesh>,
    mesh_verts: Handle<ObjVertsAsset>,
    entity: Entity,
    vertex_map: Arc<AHashMap<u16, Vec<u16>>>,
    mhid_lookup: Arc<Vec<u16>>,
    needs_rebuild: bool,
    rx: Option<crossbeam_channel::Receiver<BuildResult>>,
    /// Unchanging data cached as Arc after first build — avoids deep cloning on rebuilds
    cached_input_mesh: Option<Arc<Mesh>>,
    cached_mhclo: Option<Arc<MhcloAsset>>,
}

fn main() {
    let mut app = setup_app();

    app        .add_plugins(FeathersPlugins)
        .insert_resource(UiTheme(create_dark_theme()))
        .add_systems(Update, setup_and_add_human.run_if(resource_exists::<HumentityAssetsReady>))
        .add_systems(
            Update,
            update_character_mesh.run_if(resource_exists::<CreatorAssets>),
        )
        .add_systems(
            Update,
            init_ui
                .run_if(resource_exists::<MakeHumanMorphs>)
                .run_if(not(resource_exists::<UiInitialized>))
                .run_if(resource_exists::<CreatorAssets>),
        )
        .insert_resource(SliderValues::default())
        .run();
}

fn setup_and_add_human(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    let mhclo_handle = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");
    let entity = commands
        .spawn((
            Name::new("Character"),
            Transform::from_translation(Vec3::new(-1., 0., 0.))
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
            InheritedVisibility::default(),
        ))
        .id();

    commands.insert_resource(CreatorAssets {
        mhclo: mhclo_handle,
        input_mesh: Handle::default(),
        mesh_verts: Handle::default(),
        entity,
        vertex_map: Arc::new(AHashMap::default()),
        mhid_lookup: Arc::new(vec![]),
        needs_rebuild: false,
        rx: None,
        cached_input_mesh: None,
        cached_mhclo: None,
    });
}

fn update_character_mesh(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mesh_verts_assets: Res<Assets<ObjVertsAsset>>,
    mhclo_assets: Res<Assets<MhcloAsset>>,
    mh_morphs: Res<MakeHumanMorphs>,
    basemesh: Res<BaseMesh>,
    sliders: Option<Res<SliderValues>>,
    mut creator: ResMut<CreatorAssets>,
) {
    // Phase 1 -- load OBJ assets once when MhcloAsset becomes available
    if creator.input_mesh == Handle::default() {
        let Some(mhclo) = mhclo_assets.get(&creator.mhclo) else {
            return;
        };
        creator.input_mesh = asset_server.load::<Mesh>(mhclo.obj_file.clone());
        creator.mesh_verts = asset_server.load::<ObjVertsAsset>(mhclo.obj_file.clone());
        return;
    }

    // Wait for OBJ assets to finish loading
    let Some(input_mesh) = meshes.get(&creator.input_mesh) else {
        return;
    };
    let Some(mesh_verts) = mesh_verts_assets.get(&creator.mesh_verts) else {
        return;
    };
    let Some(mhclo) = mhclo_assets.get(&creator.mhclo) else {
        return;
    };

    // Phase 1.5 — one-time precomputation of unchanging data (vertex_map, mhid_lookup, cached mesh/mhclo)
    if creator.vertex_map.is_empty() {
        let vmap = generate_vertex_map(&mesh_verts.vertices, &get_vertex_positions(input_mesh));
        let mhid = generate_mhid_lookup(&vmap);
        creator.vertex_map = Arc::new(vmap);
        creator.mhid_lookup = Arc::new(mhid);
        creator.cached_input_mesh = Some(Arc::new(input_mesh.clone()));
        creator.cached_mhclo = Some(Arc::new(mhclo.clone()));
        creator.needs_rebuild = true;
    }

    // Phase 2 -- collect result from a completed async build
    if let Some(rx) = &creator.rx {
        if let Ok(result) = rx.try_recv() {
            commands
                .entity(creator.entity)
                .insert(Mesh3d(meshes.add(result.mesh)));
            creator.rx = None;
        }
        return;
    }

    // Phase 3 -- start an async build when flagged
    if creator.rx.is_some() {
        return;
    }
    if !creator.needs_rebuild && !creator.vertex_map.is_empty() {
        return;
    }
    creator.needs_rebuild = false;

    // Resolve morph weights on the main thread (fast — hashmap lookups only)
    if !mh_morphs.is_ready(&asset_server) {
        return;
    }
    let Some(sliders) = sliders else { return };
    let mut morphs = MorphTargets::default();
    for (&category, targets) in sliders.iter() {
        for (&name, &value) in targets.iter() {
            if category == "macro" || value != 0. {
                morphs.insert(name, value);
            }
        }
    }
    let resolved = mh_morphs.compute_target_weights(&morphs).unwrap();

    let (tx, rx) = crossbeam_channel::unbounded();
    creator.rx = Some(rx);

    let mh_morphs_targets = mh_morphs.targets.clone();
    let basemesh_vec = basemesh.vertices.clone();
    let vertex_map = creator.vertex_map.clone();
    let mhid_lookup = creator.mhid_lookup.clone();
    let input_mesh_arc = creator.cached_input_mesh.as_ref().unwrap().clone();
    let mhclo_arc = creator.cached_mhclo.as_ref().unwrap().clone();

    AsyncComputeTaskPool::get()
        .spawn(async move {
            let helpers =
                adjust_helpers_to_morphs(&resolved, &mh_morphs_targets, &basemesh_vec).unwrap();
            let mesh = shape_mesh_from_helpers_mhclo(
                &input_mesh_arc,
                &mhclo_arc,
                &helpers,
                &mhid_lookup,
                &vertex_map,
            );
            let _ = tx.send(BuildResult { mesh });
        })
        .detach();
}

fn init_ui(
    mut commands: Commands,
    mh_morphs: Res<MakeHumanMorphs>,
    mut sliders: ResMut<SliderValues>,
) {
    let root = commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                display: Display::Flex,
                align_items: AlignItems::Start,
                justify_content: JustifyContent::Start,
                flex_direction: FlexDirection::Column,
                overflow: Overflow {
                    x: OverflowAxis::Visible,
                    y: OverflowAxis::Scroll,
                },
                ..default()
            },
            RootNode,
        ))
        .id();
    let top_bar = commands
        .spawn(Node {
            width: percent(100),
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            ..default()
        })
        .id();
    commands.entity(root).add_child(top_bar);
    let morphs = mh_morphs.get_morph_names();

    // Order categories: known ones first, then any extras from the morph map
    let known: std::collections::HashSet<&str> = CATEGORY_ORDER.iter().copied().collect();
    for &category in CATEGORY_ORDER {
        let Some(morph_names) = morphs.get(category) else {
            continue;
        };
        let sliders = sliders.entry(category).or_default();
        for &morph in morph_names.iter() {
            if !sliders.contains_key(morph) {
                sliders.insert(morph, 0.);
            }
        }

        let btn = commands
            .spawn_scene(bsn! {
                ButtonCategory::new(category)
                @FeathersButton {
                    @caption: bsn! { Text(category) ThemedText }
                }
                on(category_selected)
            })
            .id();
        commands.entity(top_bar).add_child(btn);
    }
    for &category in morphs.keys() {
        if CATEGORY_BLOCKLIST.contains(&category) {
            continue;
        }
        if known.contains(category) {
            continue;
        }
        let morph_names = &morphs[category];
        let sliders = sliders.entry(category).or_default();
        for &morph in morph_names.iter() {
            if !sliders.contains_key(morph) {
                sliders.insert(morph, 0.);
            }
        }

        let btn = commands
            .spawn_scene(bsn! {
                ButtonCategory::new(category)
                @FeathersButton {
                    @caption: bsn! { Text(category) ThemedText }
                }
                on(category_selected)
            })
            .id();
        commands.entity(top_bar).add_child(btn);
    }

    commands.insert_resource(UiInitialized);
}

fn category_selected(
    trigger: On<Activate>,
    categories: Query<&ButtonCategory>,
    sliders: Query<&ChildOf, With<Slider>>,
    slider_values: Res<SliderValues>,
    mut commands: Commands,
    mh_morphs: Res<MakeHumanMorphs>,
    root: Query<Entity, With<RootNode>>,
) {
    let btn = trigger.entity;
    let category = **categories.get(btn).unwrap();
    let min_values = mh_morphs.get_min_values();

    for childof in sliders.iter() {
        commands.entity(childof.parent()).despawn();
    }

    let root = root.single().unwrap();
    let morphs = slider_values.get(category).unwrap();
    for (&name, &morph) in morphs.iter() {
        let slider = commands
            .spawn_scene(bsn! {
                Node {
                    display: Display::Grid,
                    grid_auto_flow: GridAutoFlow::Column,
                    grid_template_columns: vec![RepeatedGridTrack::flex(2, 1.)],
                    width: percent(45),
                }
                Children [
                    (
                        SliderMetadata::new(category, name)
                        @FeathersSlider {
                            @min: min_values[name],
                            @max: 1.0,
                            @value: morph,
                        }
                        SliderStep(0.1)
                        SliderPrecision(2)
                        on(on_slider_value_changed)
                        on(slider_self_update)
                    ),
                    (Text::new(name) ThemedText),
                ]
            })
            .id();
        commands.entity(root).add_child(slider);
    }
}

fn on_slider_value_changed(
    trigger: On<ValueChange<f32>>,
    mut sliders: ResMut<SliderValues>,
    mut creator: ResMut<CreatorAssets>,
    slider_metadata: Query<&SliderMetadata>,
) {
    let metadata = slider_metadata.get(trigger.source).unwrap();
    sliders.insert_value(metadata.0, metadata.1, trigger.value);
    creator.needs_rebuild = true;
}
