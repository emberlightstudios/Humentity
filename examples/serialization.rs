//! Testing out serialization functionality to support data driven workflows

mod shared;

use bevy::{ecs::{intern::Internable}, prelude::*};
use humentity::prelude::*;
use serde::{Deserialize, Serialize};
use shared::{setup_env, add_humentity_plugin};

const PREFAB_NAME: &str = "ExampleHumanPrefab";

fn main() {
    let mut app = App::new();
    add_humentity_plugin(&mut app);

    app
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup_env)
        .add_systems(OnEnter(HumentityLoadState::BuildingPrefabs), setup_prefabs)
        .add_systems(OnEnter(HumentityLoadState::Ready), add_humans)
        .run();
}

fn setup_prefabs(mut commands: Commands, morphs: Res<MakeHumanMorphs>) {
    let toml_str = r#"
        [[shapes]]
        name = "baby"

        [shapes.morphs]
        age = 0

        [[shapes]]
        name = "woman"

        [shapes.morphs]
        muscle = 0.5
        weight = 0.5
        gender = 1

        [rig]
        rig_type = "Default"
        animation_glbs = [
          "assets/animation/idle.glb"
        ]
    "#;

    let mut prefab: CharacterArchetypePrefab = toml::from_str(toml_str).unwrap();

    // Always have to decompose composite sliders (e.g. age, muscle) down to individual morphs
    for shape in prefab.shapes.iter_mut() {
        shape.morphs = morphs.compute_target_weights(&shape.morphs);
    }

    commands.insert_resource(CharacterArchetypePrefabs::new([
        (PREFAB_NAME, prefab)
    ]));
}

#[derive(Serialize, Deserialize)]
struct PartDef {
    part: CharacterPart,
    albedo_map: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct CharacterParts {
    parts: Vec<PartDef>,
}

fn add_humans(
    mut commands: Commands,
    skins: Res<CharacterBodyTextures>,
    mut character_asset_registry: ResMut<CharacterAssetRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let albedo = &skins.albedo_maps;
    for key in albedo.keys() {
        info!("{:#}", key);
    }

    let baby: CharacterShapeConfig = toml::from_str(r#"
        prefab = "ExampleHumanPrefab"

        [prefab_morph_targets]
        baby = 1
    "#).unwrap();

    let woman: CharacterShapeConfig = toml::from_str(r#"
        prefab = "ExampleHumanPrefab"

        [prefab_morph_targets]
        woman = 1
    "#).unwrap();

    let baby_parts: CharacterParts = toml::from_str(r#"
        [[parts]]
        part = "BaseMesh"
    "#).unwrap();

    let woman_parts: CharacterParts = toml::from_str(r#"
        [[parts]]
        part = "ProxyMesh:proxy741"
        albedo_map = "young_asian_male"

        [[parts]]
        part = "Equipment:simple_bra"

        [[parts]]
        part ="Equipment:simple_briefs"
    "#).unwrap();

    let mut part_bundle = |part: PartDef| -> (CharacterPart, MeshMaterial3d<StandardMaterial>) {
        if let Some(albedo) = part.albedo_map {

            // Use the provided str interner from the humentity crate to convert to &'static str
            let albedo: &'static str = NAME_INTERNER.intern(&albedo).leak();

            match &part.part {
                CharacterPart::BaseMesh | CharacterPart::ProxyMesh(_) => {
                    let handle: Handle<Image> = skins.albedo_maps[albedo].load_asset(&asset_server);
                    let mat = StandardMaterial {
                        base_color_texture: Some(handle),
                        ..default()
                    };
                    return (
                        part.part.clone(),
                        MeshMaterial3d(materials.add(mat))
                    );
                }
                CharacterPart::BodyPart(_) | CharacterPart::Equipment(_) => {
                    let handle: Handle<Image> = character_asset_registry.get_mut(&part.part)  
                        .unwrap()
                        .get_texture_handle(albedo, CharacterAssetTextureType::Albedo, &asset_server);
                    let mat = StandardMaterial {
                        base_color_texture: Some(handle),
                        ..default()
                    };
                    return (
                        part.part.clone(),
                        MeshMaterial3d(materials.add(mat))
                    );
                }
            }
        } else {
            let mat = StandardMaterial {
                base_color: Color::LinearRgba(LinearRgba::WHITE),
                ..default()
            };
            return (
                part.part.clone(),
                MeshMaterial3d(materials.add(mat))
            )
        }
    };

    let baby_entity = commands.spawn((
        Transform::from_translation(Vec3::new(-1., 0., 0.)),
        InheritedVisibility::default(),
        baby,
    )).id();

    commands.entity(baby_entity).with_children(|e | {
        for part in baby_parts.parts.into_iter() {
            e.spawn(part_bundle(part));
        }
    });

    let woman_entity = commands.spawn((
        Transform::from_translation(Vec3::new(0., 0., 0.)),
        InheritedVisibility::default(),
        woman,
    )).id();

    commands.entity(woman_entity).with_children(|e| {
        for part in woman_parts.parts.into_iter() {
            e.spawn(part_bundle(part));
        }
    });

}
