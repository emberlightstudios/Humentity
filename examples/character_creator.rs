//! Realtime mesh modification example

mod shared;

use std::time::Duration;

use bevy::feathers::controls::button;
use bevy::feathers::controls::slider;
use bevy::feathers::controls::ButtonProps;
use bevy::feathers::controls::SliderProps;
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::ThemedText;
use bevy::feathers::theme::UiTheme;
use bevy::feathers::FeathersPlugins;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use bevy::ui_widgets::observe;
use bevy::ui_widgets::slider_self_update;
use bevy::ui_widgets::Activate;
use bevy::ui_widgets::Slider;
use bevy::ui_widgets::SliderPrecision;
use bevy::ui_widgets::SliderStep;
use bevy::ui_widgets::ValueChange;
use humentity::prelude::*;
use shared::setup_app;

const TEMPLATE: &str = "TemplateName";
const SHAPE_NAME: &str = "DefaultShapeName";

#[derive(Component, Deref)]
struct ButtonCategory(&'static str);

#[derive(Component)]
struct SliderMetadata(&'static str, &'static str);

#[derive(Component)]
struct RootNode;

#[derive(Resource)]
struct UiInitialized;

#[derive(Resource)]
struct DirectBuild {
    mhclo: Handle<MhcloAsset>,
    raw_mesh: Option<Handle<Mesh>>,
    raw_verts: Option<Handle<ObjVertsAsset>>,
    needs_build: bool,
}

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
        let category = self.entry(category).or_insert(MorphTargets::default());
        category.insert(name, value);
    }

    fn categories(&self) -> Vec<&'static str> {
        vec![
            "macro", "head", "forehead", "eyes", "eyebrows", "nose", "mouth",
            "cheek", "chin", "ears", "neck", "torso", "breast", "stomach",
            "pelvis", "buttocks", "arms", "hands", "legs", "feet", "asymmetry",
        ]
    }
}

fn main() {
    let mut app = setup_app();

    app.add_plugins(FeathersPlugins)
        .insert_resource(UiTheme(create_dark_theme()))
        .add_systems(
            Update,
            setup_and_add_human
                .run_if(resource_exists::<MakeHumanMorphs>)
                .run_if(not(resource_exists::<DirectBuild>)),
        )
        .add_systems(
            Update,
            init_ui
                .run_if(resource_exists::<MakeHumanMorphs>)
                .run_if(not(resource_exists::<UiInitialized>)),
        )
        .add_systems(Update, build_direct.run_if(resource_exists::<DirectBuild>))
        .add_systems(Update, rebuild.run_if(on_timer(Duration::from_millis(50))))
        .insert_resource(SliderValues::default())
        .run();
}

fn build_direct(
    mut state: ResMut<DirectBuild>,
    asset_server: Res<AssetServer>,
    mhclo_assets: Res<Assets<MhcloAsset>>,
    obj_verts: Res<Assets<ObjVertsAsset>>,
    templates: Res<CharacterTemplates>,
    morphs: Res<MakeHumanMorphs>,
    basemesh: Res<BaseMesh>,
    rig_data: Res<RigData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    parts: Query<(Entity, &CharacterPart)>,
) {
    // Phase 1: kick off raw mesh loading
    if state.raw_mesh.is_none() {
        if let Some(mhclo) = mhclo_assets.get(&state.mhclo) {
            let mesh = asset_server.load::<Mesh>(mhclo.obj_file.clone());
            let verts = asset_server.load::<ObjVertsAsset>(mhclo.obj_file.clone());
            state.raw_mesh = Some(mesh);
            state.raw_verts = Some(verts);
        }
        return;
    }

    if !state.needs_build {
        return;
    }

    // Phase 2: build when all dependencies are ready
    let Some(mhclo) = mhclo_assets.get(&state.mhclo) else { return };
    let Some(raw_mesh) = state.raw_mesh.as_ref().and_then(|h| meshes.get(h)) else { return };
    let Some(raw_verts) = state.raw_verts.as_ref().and_then(|h| obj_verts.get(h)) else { return };
    let Some(template) = templates.get(TEMPLATE) else { return };
    let Some(rig_spec) = rig_data.get(&template.rig) else { return };

    let mesh = build_single_mesh_direct(
        mhclo,
        raw_mesh,
        raw_verts,
        template,
        morphs.targets.clone(),
        basemesh.0.clone(),
        rig_spec,
        &mut images,
    );

    let mesh_handle = meshes.add(mesh);

    for (entity, part) in parts.iter() {
        if part.0 == state.mhclo {
            commands.entity(entity).insert(Mesh3d(mesh_handle.clone()));
        }
    }

    state.needs_build = false;
}

fn setup_and_add_human(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mh_morphs: Res<MakeHumanMorphs>,
) {
    let mut morph_targets = MorphTargets::default();
    morph_targets.insert("age", 0.5);
    morph_targets.insert("gender", 1.0);
    morph_targets.insert("caucasian", 1.0);

    let resolved = match mh_morphs.compute_target_weights(&morph_targets) {
        Ok(m) => m,
        Err(err) => {
            error!("Error computing morph targets: {err}");
            return;
        }
    };

    let loaded = mh_morphs.targets.read().unwrap();
    for k in resolved.keys() {
        if !loaded.contains_key(k) {
            return;
        }
    }

    commands.insert_resource(CharacterTemplates::new([(
        TEMPLATE,
        CharacterTemplate::new(
            [CharacterMorphShapes::new(SHAPE_NAME, resolved)],
            RigType::Default,
        ),
    )]));

    let mhclo = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    commands.insert_resource(DirectBuild {
        mhclo: mhclo.clone(),
        raw_mesh: None,
        raw_verts: None,
        needs_build: true,
    });

    let mut morph_weights = MorphTargets::default();
    morph_weights.insert(SHAPE_NAME, 1.0);

    commands.spawn((
        Name::new("Character"),
        Transform::from_translation(Vec3::new(-1., 0., 0.)),
        InheritedVisibility::default(),
        CharacterShapeConfig::new(TEMPLATE, morph_weights),
        children![(CharacterPart(mhclo), Name::new("basemesh"))],
    ));
}

fn rebuild(
    mut state: ResMut<DirectBuild>,
    human: Single<(Entity, &RelatedEntities), With<CharacterShapeConfig>>,
    mut commands: Commands,
) {
    let (entity, related) = *human;

    state.needs_build = true;

    commands.entity(related.rig).despawn();
    commands
        .entity(entity)
        .remove::<RelatedEntities>()
        .remove::<SkinnedMesh>();
}

fn on_slider_value_changed(
    trigger: On<ValueChange<f32>>,
    mut sliders: ResMut<SliderValues>,
    slider_metadata: Query<&SliderMetadata>,
    mh_morphs: Res<MakeHumanMorphs>,
    mut templates: ResMut<CharacterTemplates>,
) {
    let metadata = slider_metadata.get(trigger.event().source).unwrap();
    sliders.insert_value(metadata.0, metadata.1, trigger.event().value);

    let mut morphs = MorphTargets::default();
    for (&category, targets) in sliders.iter() {
        for (&name, &value) in targets.iter() {
            if category == "macro" || value != 0. {
                morphs.insert(name, value);
            }
        }
    }

    morphs = match mh_morphs.compute_target_weights(&morphs) {
        Ok(m) => m,
        Err(err) => {
            error!("Error computing morph targets: {err}");
            return;
        }
    };

    let template = templates.get_mut(TEMPLATE).unwrap();
    let shape = template.shapes.get_mut(0).unwrap();
    shape.morphs = morphs;
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

    for &category in sliders.categories().iter() {
        let morphs = &morphs[category];
        let sliders = sliders.entry(category).or_insert(MorphTargets::default());
        for &morph in morphs.iter() {
            if !sliders.contains_key(morph) {
                sliders.insert(morph, 0.);
            }
        }

        let btn = commands
            .spawn((
                ButtonCategory(category),
                button(
                    ButtonProps::default(),
                    (),
                    Spawn((Text::new(category), ThemedText)),
                ),
            ))
            .observe(category_selected)
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
    for (&name, morph) in morphs.iter() {
        let slider = commands
            .spawn((
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
                    ),
                    (Text::new(name), ThemedText)
                ],
            ))
            .id();
        commands.entity(root).add_child(slider);
    }
}
