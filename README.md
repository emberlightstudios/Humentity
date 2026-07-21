# Humentity

A Bevy plugin for loading, morphing, rigging, and animating MakeHuman-based 3D humanoid characters at runtime.

![Screenshot](https://i.imghippo.com/files/eVWiu1727317384.png)

## Features

- **Template-based morphing** — hundreds of MakeHuman shape keys are baked into a small set of runtime morph targets, preserving GPU instancing and batching across characters
- **Auto-rigging** — skeletal rigs are built automatically from MakeHuman rig/weight data and fitted to each character's morphed shape
- **Skeleton LOD** — reduce bone counts at runtime by merging child bone subtrees into their parents, with automatic weight rebinding and inverse bindpose recomputation
- **Mesh LOD** — use MakeHuman's lower-poly proxy meshes with Bevy's `VisibilityRange` for distance-based mesh switching
- **Animation retargeting** — import glTF animation clips and retarget them to arbitrary character shapes
- **Stitched meshes** — split a character into multiple mesh pieces (head, body, clothing) with continuous normals across seam cuts
- **Skeleton LOD filtering** — use `SkeletonLodFilter` to restrict which LOD levels are spawned per character, saving memory when certain detail levels are unnecessary
- **Ragdoll physics** — optional integration with [avian3d](https://github.com/Jondolf/avian)
- **Asset loaders** — native Bevy loaders for `.mhclo`, `.obj`, `.target`, `.macro`, rig configs, and other MakeHuman data formats
- **Custom assets** — build your own meshes and morph targets in Blender via MPFB

## Quick start

```rust
use bevy::prelude::*;
use humentity::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, HumentityPlugin))
        .insert_resource(SkeletonLodConfig(vec![
            BoneMergeConfig::full().without_children_of(&["foot.L", "foot.R"]),
        ]))
        .add_systems(Startup, load_assets)
        .add_systems(Update, attach_mesh)
        .add_observer(spawn_character)
        .run();
}

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

fn spawn_character(
    _trigger: On<MorphsReady>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut mesh_builder: ResMut<MhcloMeshBuilder>,
    mut template_assets: ResMut<Assets<CharacterTemplate>>,
    mut shape_assets: ResMut<Assets<CharacterShapeAsset>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // 1. Define a template and trigger a mesh build
    let mut morphs = MorphTargets::default();
    morphs.insert("gender", 0.0);

    let template = template_assets.add(CharacterTemplate::new([
        CharacterMorphShape::new("woman", morphs),
    ]));

    let mesh = asset_server.load::<MhcloAsset>("proxymeshes/basemesh/basemesh.proxy");

    mesh_builder.trigger(LoadAssetMeshJob::Single {
        part: mesh.clone(),
        template_handle: template.clone(),
        skeleton_lod: 0,
    });

    // 2. Spawn the character entity using the template
    let mut weights = MorphTargets::default();
    weights.insert("woman", 1.0);

    let mat = materials.add(StandardMaterial::from_color(Color::WHITE));

    commands.spawn((
        CharacterShape(shape_assets.add(CharacterShapeAsset::new(template, weights))),
        InheritedVisibility::default(),
        children![(
            CharacterPart { mesh, skeleton_lod: 0 },
            Name::new("body"),
            MeshMaterial3d(mat),
        )],
    ));
}

/// Once the skeleton is fitted and the mesh build is complete, insert Mesh3d on each part.
fn attach_mesh(
    parts: Query<(Entity, &ChildOf, &CharacterPart, Option<&SkinnedMesh>), Without<Mesh3d>>,
    characters: Query<&CharacterShape>,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    cached_meshes: Res<CachedMhcloMeshHandles>,
    mut commands: Commands,
) {
    for (entity, parent, part, skm) in &parts {
        let Ok(shape) = characters.get(parent.parent()) else {
            continue;
        };
        let Some(asset) = shape_assets.get(&shape.0) else {
            continue;
        };
        let Some(skm) = skm else {
            continue;
        };
        if let Some(handle) =
            cached_meshes.get(&(part.mesh.clone(), asset.template.clone(), part.skeleton_lod))
        {
            commands
                .entity(entity)
                .insert((Mesh3d(handle.clone()), skm.clone()));
        }
    }
}
```

## How it works

1. **Configure** — Insert a `SkeletonLodConfig` resource with a list of `BoneMergeConfig` entries. Each entry defines one LOD level by specifying which bone subtrees to remove and merge into their parents.

2. **Load** — Call `load_and_insert_humentity_assets` to load the MakeHuman basemesh, vertex groups, morph targets, rig config, and reference rig as ECS resources.

3. **Morph** — Create a `CharacterTemplate` with one or more `CharacterMorphShape` entries. Each shape maps named MakeHuman morphs to float weights. The template system bakes hundreds of underlying MakeHuman shape keys into a compact set of morph targets.

4. **Build** — Trigger a `LoadAssetMeshJob` (single or stitched) via the `MhcloMeshBuilder` resource. The job loads OBJ data and morph targets, then spawns a background thread to build the final mesh with the correct bone weights for the requested skeleton LOD level. The result is sent back over a channel and cached in `CachedMhcloMeshHandles`. This is non-blocking — the main thread continues while the mesh is built asynchronously.

5. **Spawn** — Create a `CharacterShape` entity with `CharacterPart` children. Each `CharacterPart` specifies which mesh handle to use and which skeleton LOD level it targets.

6. **Rig** — The plugin automatically spawns skeleton entities for each LOD level, fits bone transforms to the morphed shape, computes inverse bindposes, and sets up `SkinnedMesh` on each part.

7. **Activate** — Trigger `EnableSkeletonLod` to enable a specific LOD level. All other levels start disabled via `SkeletonLodDisabled`.

## Examples

| Example | Description |
|---|---|
| `lod.rs` | Distance-based mesh and skeleton LOD with `VisibilityRange` |
| `stress_test.rs` | Benchmarks 576 animated characters (24x24 grid) |
| `animation.rs` | Retargeted idle animation on a morphed character |
| `morphs_and_templates.rs` | Template system and runtime morph targets |
| `stitched_parts.rs` | Multi-part meshes with continuous normals and per-part morphs |
| `assets.rs` | Loading body parts, clothing, hair, and accessories |
| `character_creator.rs` | Real-time mesh modification UI with sliders |
| `ragdoll_avian.rs` | Full-body ragdoll with avian3d physics |
| `ragdoll_avian_partial.rs` | Partial ragdoll (arms only) with kinematic colliders |

Run examples with:

```sh
cargo run --example lod
cargo run --example stress_test
cargo run --example animation --features avian
```

## Feature flags

| Feature | Description |
|---|---|
| `avian` | Enables ragdoll physics via [avian3d](https://github.com/Jondolf/avian) |
| `debug` | Enables Bevy's debug rendering |

## Custom assets

Custom meshes, morph targets, and rig data can be authored in Blender using [MPFB](https://github.com/makehumancommunity/makehuman-plugin-for-blender) and exported as `.mhclo`/`.obj` files with `.target` shape keys. The crate's native asset loaders handle these formats automatically.

## Animation postprocessing

The `HumentityAnimationPostProcess` system set runs in `PostUpdate` after Bevy's `AnimationSystems` and before `TransformSystems::Propagate`. The built-in `rescale_root_bone_translation` system runs in this set to correct root bone Y translation for different human proportions.

To add your own animation postprocessing, order your systems relative to this set:

```rust
app.add_systems(
    PostUpdate,
    my_system.after(HumentityAnimationPostProcess),
);
```
