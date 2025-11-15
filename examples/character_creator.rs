//! Realtime mesh modification example

mod shared;

use bevy::feathers::FeathersPlugins;
use bevy::feathers::controls::ButtonProps;
use bevy::feathers::controls::SliderProps;
use bevy::feathers::controls::button;
use bevy::feathers::controls::slider;
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::ThemedText;
use bevy::feathers::theme::UiTheme;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use bevy::ui_widgets::Slider;
use bevy::ui_widgets::SliderPrecision;
use bevy::ui_widgets::SliderStep;
use bevy::ui_widgets::ValueChange;
use bevy::ui_widgets::observe;
use bevy::ui_widgets::slider_self_update;
use humentity::prelude::*;
use shared::{cam_controls, add_material};

const PREFAB: &'static str = "PrefabName";
const SHAPE_NAME: &'static str = "DefaultShapeName";

fn main() {
    let mut app = App::new();
    app
        .add_plugins((
            // Point to the humentity crate location
            Humentity {
                paths: HumentityPathsConfig::from_crate_path("./"),
                config: HumentityGlobalConfig {
                    translation_tracks: TranslationTracks::None,
                    ..default()
                }
            },
            DefaultPlugins,
            FeathersPlugins,
        ))
        .insert_resource(UiTheme(create_dark_theme()))
        .add_systems(Startup, setup_env)
        .add_systems(
            OnExit(HumentityLoadState::LoadingCoreAssets), 
            setup_prefab
        )
        .add_systems(
            OnEnter(HumentityLoadState::Ready),
            move |mut commands: Commands| { add_human(&mut commands) },
        )
        .add_systems(Update, (cam_controls, add_material, poll_mesh_handle))
        .insert_resource(SliderValues::default())
        .run();
}

// For tracking categories when reacting to button presses
#[derive(Component, Deref)]
struct ButtonCategory(&'static str);

// For tracking which morph a slider corresponds to
#[derive(Component)]
struct SliderMetadata(&'static str, &'static str);

// The root UI node
#[derive(Component)]
struct RootNode;

// Check if the mesh has been updated
fn setup_prefab(mut commands: Commands, mh_morphs: Res<MakeHumanMorphs>) {
    let mut morphs = MorphTargets::default();
    morphs.insert("age", 0.5);
    morphs.insert("gender", 1.0);
    morphs.insert("caucasian", 1.0);
    
    let mut prefabs = CharacterArchetypePrefabs::default();
    prefabs.insert(
        PREFAB,
        CharacterArchetypePrefab::new(
            vec![
                CharacterShapeArchetype::new(SHAPE_NAME, mh_morphs.compute_target_weights(&morphs)),
            ],
            CharacterAnimationArchetype::default(), 
        )
    );

    commands.insert_resource(prefabs);
}

// Global slider state.
#[derive(Resource, DerefMut, Deref)]
struct SliderValues(ahash::AHashMap<&'static str, MorphTargets>);

impl Default for SliderValues {
    fn default() -> Self {
        // We need to set the initial slider values for our UI
        let mut instance = Self(Default::default());

        // These we explicitly defined in our prefab
        instance.insert_value("macro", "age", 0.5);
        instance.insert_value("macro", "gender", 1.);
        instance.insert_value("macro", "caucasian", 1.);

        // Macro shapes have implicit values when you don't set them explicitly.
        // These are the default values.
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
        let category = self.entry(category).or_insert(MorphTargets::default());
        category.insert(name, value);
    }

    fn categories(&self) -> Vec<&'static str> {
        vec![
            "macro", "head", "forehead",  "eyes", "eyebrows", "nose", "mouth", "cheek", "chin", "ears", 
            "neck", "torso", "breast", "stomach", "pelvis", "buttocks", "arms", "hands", "legs", "feet", "asymmetry",
        ]
    }
}

// Check for an updated mesh
fn poll_mesh_handle(
    basemesh: Res<BaseMesh>,
    human: Query<(Entity, &Mesh3d), With<CharacterPart>>,
    mut commands: Commands,
) {
    let Ok((entity, human)) = human.single() else { return };
    // CharacterAsset (which you can get from the CharacterAssetRegistry) also has a prefab_state field.
    if let MeshProcessingState::Ready(handle) = &basemesh.prefab_state[PREFAB] {
        if *handle != **human {
            commands.entity(entity).insert(Mesh3d(handle.clone()));
        }
    }
}

// Trigger a change in a prefab shape
fn on_slider_value_changed(
    trigger: On<ValueChange<f32>>,
    mut commands: Commands,
    mut sliders: ResMut<SliderValues>,
    slider_metadata: Query<&SliderMetadata>,
    mh_morphs: Res<MakeHumanMorphs>,
) {
    // The process for readying the body and asset meshes is not the fastest.  It has to go through
    // several stages to get to the end result.
    //
    // This is just a simple example.  In a real game you might load several assets, eyes, eyebrows, etc.
    // Each will need to be re-processesed.  In addition you might play an idle animation in the 
    // character creation menu.  After reshaping the mesh the skeleton will need to be re-fitted to the 
    // shape of the new mesh.

    let metadata = slider_metadata.get(trigger.event().source).unwrap();
    sliders.insert_value(metadata.0, metadata.1, trigger.event().value);

    // Only the UI uses nested iterators for categories
    // Everything in humentity wanst flat iterators
    let mut morphs = MorphTargets::default();
    for (&category, targets) in sliders.iter() {
        for (&name, &value) in targets.iter() {
            if category == "macro" || value != 0. {
                morphs.insert(name, value);
            }
        }
    }

    // ALWAYS convert macro/composite sliders to makehuman morph targets
    morphs = mh_morphs.compute_target_weights(&morphs);

    commands.trigger(ModifyPrefabShape {
        prefab_name: PREFAB,
        shape_name: SHAPE_NAME,
        parts: vec![CharacterPart::BaseMesh], // Insert other loaded assets as necessary
        morphs 
    });
}

fn setup_env(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // circular base
    let mesh = meshes.add(Circle::new(4.0));
    let material = materials.add(Color::WHITE);

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material.clone()),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
    ));

    // A light:
    commands.spawn((
        PointLight {
            intensity: 15_000_0.0,
            radius: 20.,
            range: 20.,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.0, 1.0, 3.0),
    ));

    // A camera:
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(2.0, 1., 3.5).looking_at(Vec3::Y * 1., Vec3::Y),
    ));

    let ui = commands.register_system(init_ui);
    commands.run_system(ui);
}

// Spawn the human
fn add_human(
    commands: &mut Commands,
) {
    let mut morphs = MorphTargets::default();
    morphs.insert(SHAPE_NAME, 1.0);

    commands.spawn((
        Transform::from_translation(Vec3::new(1., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(PREFAB, morphs.clone()),
        children![(
            CharacterPart::BaseMesh 
        )]
    ));
}

// Set up the category buttons at the top
fn init_ui(
    mut commands: Commands,
    mh_morphs: Res<MakeHumanMorphs>,
    mut sliders: ResMut<SliderValues>,
) {
    let root = commands.spawn(
        (
            Node {
                width: percent(100),
                height: percent(100),
                display: Display::Flex,
                align_items: AlignItems::Start,
                justify_content: JustifyContent::Start,
                flex_direction: FlexDirection::Column,
                overflow: Overflow { x: OverflowAxis::Visible, y: OverflowAxis::Scroll },
                ..default()
            },
            RootNode,
        )
    ).id();
    let top_bar = commands.spawn(Node {
        width: percent(100),
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        ..default()
    }).id();
    commands.entity(root).add_child(top_bar);
    let morphs = mh_morphs.get_morph_names();

    for &category in sliders.categories().iter() {
        let morphs = &morphs[category];
        let sliders = sliders.entry(category).or_insert(MorphTargets::default());
        for &morph in morphs.iter() {
            if !sliders.contains_key(morph) {
                sliders.insert(morph, 0.);
            }
        }

        let btn = commands.spawn(
            (
                ButtonCategory(category),
                button(
                    ButtonProps::default(),
                    (),
                    Spawn((
                        Text::new(category),
                        ThemedText,
                    )),
                )
            )
        )
            .observe(category_selected)
            .id();
        commands.entity(top_bar).add_child(btn);
    }
}

//Show individual sliders when a category is selected
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

    // Remove any existing sliders
    for childof in sliders.iter() {
        commands.entity(childof.parent()).despawn();
    }
    // Spawn new sliders
    let root = root.single().unwrap();
    let morphs = slider_values.get(category).unwrap();
    for (&name, morph) in morphs.iter() {
        let slider = commands.spawn((
            Node {
                display: Display::Grid,
                grid_auto_flow: GridAutoFlow::Column,
                grid_template_columns: RepeatedGridTrack::flex(2, 1.),
                width: percent(45),
                ..default()
            },
            children![
                (
                    SliderMetadata(category, name),
                    slider(
                        SliderProps {
                            min: min_values[name],
                            max: 1.0,
                            value: *morph,
                            ..default()
                        },
                        (SliderStep(0.1), SliderPrecision(2)),
                    ),
                    observe(on_slider_value_changed),
                    observe(slider_self_update)
                ), (
                    Text::new(name), ThemedText,
                )
            ]
        )).id();
        commands.entity(root).add_child(slider);
    }
}