//! GPU morphs: `morphs_and_templates` bodies posed entirely on the GPU.
//!
//! Same baby/bodybuilder template, but crowd-style: one shared GPU mesh,
//! the idle clip baked once on the reference skeleton, per-entity
//! [`MeshMorphWeights`] for the body variation plus a per-instance
//! shape-weight row blending the fitted shape skeletons (same weights the
//! mesh morphs use, so hybrids ride the true blended skeleton).
//! If morphs work through the GPU skinning vertex shader, the five characters
//! look different despite sharing one mesh handle and one material.

mod shared;

use bevy::{
    mesh::{MeshTag, morph::MeshMorphWeights},
    prelude::*,
};
use humentity::prelude::*;
use shared::{CameraFraming, CustomCrowdMaterial, GPU_SKELETON_LOD, custom_crowd_material, setup_app_gpu};

const BABY: &str = "baby";
const BODYBUILDER: &str = "bodybuilder";
const INSTANCES: usize = 5;

fn main() {
    let mut app = setup_app_gpu(INSTANCES, 30.0, CameraFraming::Close);
    app.add_systems(
        Update,
        (
            trigger_morph_build.run_if(resource_added::<HumentityAssetsReady>),
            request_clip_bakes,
            fit_shape_skeletons,
            spawn_crowd,
        ),
    )
    .run();
}

#[derive(Resource)]
struct MorphBuild {
    part: Handle<MhcloAsset>,
    template: Handle<CharacterTemplate>,
    clips: Handle<RetargetedAnimationAsset>,
}

fn trigger_morph_build(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut templates: ResMut<Assets<CharacterTemplate>>,
) {
    let mut baby_targets = MorphTargets::default();
    baby_targets.insert("age", 0.);

    let mut bodybuilder_targets = MorphTargets::default();
    bodybuilder_targets.insert("muscle", 1.);
    bodybuilder_targets.insert("weight", 1.);

    let template = templates.add(CharacterTemplate::new([
        CharacterMorphShape::new(BODYBUILDER, bodybuilder_targets),
        CharacterMorphShape::new(BABY, baby_targets),
    ]));
    let part = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");
    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: part.clone(),
        template_handle: template.clone(),
        skeleton_lod: GPU_SKELETON_LOD,
    });
    let clips = asset_server.load::<RetargetedAnimationAsset>("animation/idle.glb");
    commands.insert_resource(MorphBuild { part, template, clips });
    info!("gpu morph build triggered");
}

/// Manual clip loads: retries every frame until the bank accepts every clip
/// in the retargeted asset.
fn request_clip_bakes(
    bank: Option<ResMut<GpuAnimationBank>>,
    build: Option<Res<MorphBuild>>,
    clips: Res<Assets<RetargetedAnimationAsset>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(mut bank), Some(build)) = (bank, build) else {
        return;
    };
    let Some(asset) = clips.get(&build.clips) else {
        return;
    };
    // Level-driven, not event-driven: a bank that is missing (base bake not
    // done) or full must not lose the load, so keep asking until accepted.
    let mut retry = false;
    for name in asset.clips.keys() {
        if !bank.is_loaded(name)
            && !bank.is_baking(name)
            && !bank.request_load(name.to_string(), GpuClipMode::Loop)
        {
            retry = true;
        }
    }
    *done = !retry;
}

/// Fits one GPU shape skeleton per template shape from morphed helper verts,
/// then registers them on [`GpuCrowdShapes`]. Manual population: helpers come
/// from the template's own morphs at weight 1, exactly the bodies the mesh
/// morph targets produce. Registration order must match the template's shape
/// order: weight slot `i` blends `shapes[i]`, and the entity morph weights
/// follow template order.
fn fit_shape_skeletons(
    build: Option<Res<MorphBuild>>,
    bank: Option<Res<GpuAnimationBank>>,
    templates: Res<Assets<CharacterTemplate>>,
    base_mesh: Res<BaseMesh>,
    morphs: Res<MakeHumanMorphs>,
    rig_data: Res<RigData>,
    vertex_groups: Res<VertexGroups>,
    mut shapes: ResMut<GpuCrowdShapes>,
    asset_server: Res<AssetServer>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(build), Some(bank)) = (build, bank) else {
        return;
    };
    let Some(template) = templates.get(&build.template) else {
        return;
    };
    if !morphs.is_ready(&asset_server) {
        return;
    }
    let Some(rig) = rig_data.0.as_ref() else {
        return;
    };
    if base_mesh.vertices.is_empty() || vertex_groups.is_empty() {
        return;
    }
    for shape in template.shapes.iter() {
        if !morphs.has_targets_for_shapes(std::slice::from_ref(shape)) {
            return;
        }
    }
    for shape in template.shapes.iter() {
        let helpers = template
            .get_helpers(&single_shape_weights(shape.name), &base_mesh.vertices, &morphs)
            .expect("template shape helpers resolve once targets are ready");
        let fitted = fit_shape_skeleton_from_helpers(
            shape.name,
            &helpers,
            &bank.bones,
            rig,
            &vertex_groups,
        );
        shapes.register(fitted);
    }
    *done = true;
    info!("gpu_morphs: fitted {} shape skeleton(s)", shapes.len());
}

/// Weight vector selecting exactly one template shape at weight 1.
fn single_shape_weights(shape: &'static str) -> MorphTargets {
    let mut weights = MorphTargets::default();
    weights.insert(shape, 1.0);
    weights
}

fn spawn_crowd(
    mut commands: Commands,
    build: Option<Res<MorphBuild>>,
    handles: Option<Res<GpuRenderHandles>>,
    bank: Option<Res<GpuAnimationBank>>,
    shapes: Res<GpuCrowdShapes>,
    cached: Res<CachedMhcloMeshHandles>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CustomCrowdMaterial>>,
    mut anims: Option<ResMut<GpuInstanceAnims>>,
    mut spawned: Local<bool>,
) {
    if *spawned {
        return;
    }
    let (Some(build), Some(handles), Some(bank)) = (build, handles, bank) else {
        return;
    };
    // Idle must be resident before the crowd binds blend slot 0 to it. With
    // slot 0 reserved for the bindpose clip, idle lands at slot 1+.
    let Some(idle) = bank.slot_of("Idle-loop") else {
        return;
    };
    let Some(source_handle) =
        cached.get(&(build.part.clone(), build.template.clone(), GPU_SKELETON_LOD))
    else {
        return;
    };
    let Some(source) = meshes.get(source_handle) else {
        return;
    };
    let Some(mesh) = make_gpu_mesh(source) else {
        return;
    };
    if !mesh.has_morph_targets() {
        warn!("gpu_morphs: mesh has no morph targets, weights will do nothing");
    }
    let mesh = meshes.add(mesh);
    let material = custom_crowd_material(&handles, &mut materials);
    if let Some(anims) = anims.as_mut() {
        for index in 0..INSTANCES {
            anims.set_slot(index, 0, idle as u32);
        }
    }
    *spawned = true;
    // Template order is [bodybuilder, baby]; morph weights and GPU shape
    // weights follow the same order, so the skeleton blend always matches
    // the mesh blend (including hybrids).
    let cases = [
        ("Basemesh", [0.0, 0.0], -2.0),
        ("Baby", [0.0, 1.0], -1.0),
        ("Bodybuilder", [1.0, 0.0], 0.0),
        ("Hybrid normalized", [0.5, 0.5], 1.0),
        ("Hybrid unnormalized", [1.0, 1.0], 2.0),
    ];
    if let Some(anims) = anims.as_mut() {
        for (index, (_, weights, _)) in cases.iter().enumerate() {
            anims.set_shape_weights(index, weights);
            // Same blend the shader applies: reference (1.0) plus the
            // weight-scaled deltas, so hybrids get the true hybrid scale.
            let mut scale = 1.0;
            for (slot, w) in weights.iter().enumerate() {
                if let Some(shape) = shapes.shapes.get(slot) {
                    scale += w * (shape.root_scale - 1.0);
                }
            }
            anims.set_root_scale(index, scale);
        }
    }
    for (index, (name, weights, x)) in cases.into_iter().enumerate() {
        commands.spawn((
            Name::new(name),
            Transform::from_translation(Vec3::new(x, 0., 0.)),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            MeshTag(index as u32),
            MeshMorphWeights::Value {
                weights: weights.to_vec(),
            },
        ));
    }
    info!("spawned {INSTANCES} GPU-posed morph characters sharing one mesh and one material");
}
