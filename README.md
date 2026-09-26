# Humentity

A Bevy plugin for loading, morphing, rigging, and animating MakeHuman-based 3D humanoid characters at runtime.

![Screenshot](https://i.imghippo.com/files/eVWiu1727317384.png)

## Features

- **Template-based morphing** — hundreds of MakeHuman shape keys are baked into a small set of runtime morph targets, preserving GPU instancing and batching across characters
- **Auto-rigging** — skeletal rigs are built automatically from MakeHuman rig/weight data and fitted to each character's morphed shape
- **Skeleton LOD** — each character has one fixed full skeleton; the active LOD set decides which bone sub-trees are disabled (via `SkeletonLodDisabled`) so their `GlobalTransform`s stop propagating
- **Mesh LOD** — use MakeHuman's lower-poly proxy meshes with Bevy's `VisibilityRange` for distance-based mesh switching
- **Animation retargeting** — import glTF animation clips and retarget them to arbitrary character shapes
- **GPU crowd posing** — pose thousands of characters (10,000 in `gpu_crowd`) with a compute shader: clips bake once, per-instance blend weights drive the mix, and a custom skinning vertex shader does the rest. No per-bone entities, no `AnimationPlayer`, no transform propagation for the crowd
- **GPU morphs** — morph targets keep working through the GPU skinning shader, and per-instance shape weights blend fitted skeletons to match
- **GPU joints readback** — optional copy of the posed joint buffer back to the CPU for hitboxes and gameplay queries
- **Stitched meshes** — split a character into multiple mesh pieces (head, body, clothing) with continuous normals across seam cuts
- **Character scale** — `CharacterScale` scales the skeleton root, bones, skinned mesh, and ragdoll colliders together
- **Ragdoll physics** — optional integration with [avian3d](https://github.com/Jondolf/avian) (experimental — see the note below)
- **Asset loaders** — native Bevy loaders for `.mhclo`, `.obj`, `.target`, `.macro`, rig configs, and other MakeHuman data formats
- **Custom assets** — build your own meshes and morph targets in Blender via MPFB

## Quick start

A single CPU-skinned character:

```rust
use bevy::prelude::*;
use humentity::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, HumentityPlugin))
        .insert_resource(SkeletonLodConfig::new(&[
            BoneMergeConfig::full().merge_default_rig_toes(),
        ]))
        .add_systems(Startup, load_assets)
        .add_systems(
            Update,
            (
                attach_mesh,
                spawn_character.run_if(resource_exists::<HumentityAssetsReady>),
            ),
        )
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
        skeleton_lod: MeshBuildLod::Cpu(0),
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
        if let Some(handle) = cached_meshes.get(&(
            part.mesh.clone(),
            asset.template.clone(),
            MeshBuildLod::Cpu(part.skeleton_lod),
        )) {
            commands
                .entity(entity)
                .insert((Mesh3d(handle.clone()), skm.clone()));
        }
    }
}
```

## How it works

1. **Configure** — Insert a `SkeletonLodConfig` resource with up to `MAX_LODS` (4) `BoneMergeConfig` entries. Each entry defines one LOD level by naming which bone subtrees to remove and merge into their parents (`without_children_of`), or into one surviving child bone so posing still works through it (`merge_into_kept_bone`, e.g. `merge_default_rig_toes` for the default rig).

2. **Load** — Call `load_and_insert_humentity_assets` to load the MakeHuman basemesh, vertex groups, morph targets, rig config, and reference rig as ECS resources. Gate the rest on `HumentityAssetsReady`.

3. **Morph** — Create a `CharacterTemplate` with one or more `CharacterMorphShape` entries. Each shape maps named MakeHuman morphs to float weights. The template system bakes hundreds of underlying MakeHuman shape keys into a compact set of morph targets.

4. **Build** — Trigger a `LoadAssetMeshJob` (single or stitched) via the `MhcloMeshBuilder` resource. The job loads OBJ data and morph targets, then spawns a background thread to build the final mesh with the correct bone weights for the requested LOD. The result is sent back over a channel and cached in `CachedMhcloMeshHandles`. This is non-blocking — the main thread continues while the mesh is built asynchronously. CPU meshes use `MeshBuildLod::Cpu(lod)`; GPU crowd meshes use `MeshBuildLod::Gpu`.

5. **Spawn** — Create a `CharacterShape` entity with `CharacterPart` children (CPU) or `GpuCharacterPart` children (GPU crowd). CPU parts name their skeleton LOD; GPU parts always build on the single `GpuSkeletonLod` skeleton.

6. **Rig** — The plugin spawns one full skeleton per CPU character, fits its bone transforms to the morphed shape, computes the inverse bindposes, and sets up `SkinnedMesh` on each part over the surviving bone subset for its LOD level. `CharacterScale` on the `CharacterShape` scales the skeleton root (bones and skinned mesh follow).

7. **Activate** — Write `SkeletonLodState { active: [...] }` on the character to set the active LOD levels. The reconcile system disables every bone sub-tree removed by all active LODs, so unneeded bones stop propagating transforms.

## GPU characters

Use the CPU path above for a handful of full-fidelity characters. Use the GPU path for crowds: one shared mesh, one shared material, and no skeleton entities at all. Clips are baked once to local bone matrices, a compute shader poses every instance × bone each frame, and the joint buffer feeds the skinning vertex shader. Crowd members are plain entities with a `Transform`, a `Mesh3d`, a material, and a `MeshTag` instance id.

### Setup

Add `HumentityGpuPlugin` next to `HumentityPlugin`, register one full skeleton for the bake, and point `GpuSkeletonLod` at it (it must match the LOD the crowd meshes were built with, or joint indices won't line up):

```rust
use humentity::prelude::*;

app.add_plugins((
    HumentityPlugin,
    HumentityGpuPlugin {
        instances: 10_000,
        sample_rate: 30.0,
        ..default()
    },
));
app.insert_resource(SkeletonLodConfig::new(&[BoneMergeConfig::full()]));
app.insert_resource(GpuSkeletonLod(0));
```

`HumentityGpuPlugin` takes `instances`, `sample_rate` (bake frames per second), `blend_slots` (clips mixed per instance, max 4), `max_clips` (bank size, max 64, slot 0 is always the bindpose fallback), `max_shapes` (bodies blended per instance, max 8), and `readback_joints` (see below). The `gpu_crowd` and `gpu_morphs` examples share this setup through `examples/shared/mod.rs` (`setup_app_gpu`).

### Meshes

Build the crowd mesh with `MeshBuildLod::Gpu`, or convert an already-built CPU mesh with `request_gpu_mesh` / `make_gpu_mesh`. The GPU mesh keeps position, normal, uv, morph targets, and custom `ATTRIBUTE_GPU_JOINT_INDEX` / `ATTRIBUTE_GPU_JOINT_WEIGHT` attributes, but drops Bevy's standard skin attributes so no CPU skin buffer is needed:

```rust
mesh_builder.trigger(LoadAssetMeshJob::Single {
    part: part.clone(),
    template_handle: template.clone(),
    skeleton_lod: MeshBuildLod::Gpu,
});
```

Finished builds land in `CachedMhcloMeshHandles`, keyed by `(MhcloAsset, CharacterTemplate, MeshBuildLod)` — the same proxy built for another template, or for CPU vs GPU, is a separate entry. The cache is the handoff between the background builder and your spawn code: trigger the job, then read the handle from the cache each frame until it appears. `request_gpu_mesh` wraps this for the GPU path: it converts the already-built CPU mesh when one exists and only triggers a background `Gpu` build otherwise.

GPU crowd entities are static until the pose pipeline drives them — spawn them with `Mesh3d`, a crowd material, and `MeshTag(index)` and nothing else:

```rust
commands.spawn((
    Transform::from_xyz(x, 0.0, z),
    Mesh3d(mesh.clone()),
    MeshMaterial3d(material.clone()),
    MeshTag(index as u32),
));
```

### Clips

Clip loads are manual. Queue them on `GpuAnimationBank` once the retargeted asset is loaded, then wait for them to land before wiring blend slots:

```rust
// Queue (e.g. when the RetargetedAnimationAsset finishes loading):
bank.request_load("Idle-loop", GpuClipMode::Loop);

// Spawn-gate:
if bank.is_loaded("Idle-loop") {
    let idle = bank.slot_of("Idle-loop").unwrap();
    anims.set_slot(index, 0, idle as u32);
}
```

`request_load` spawns one background bake task per clip; `request_unload` packs the buffer back up. `GpuClipMode::Loop` loops, `GpuClipMode::OnceHold` fires `GpuOneShotDone` and holds the last frame. Bank slot 0 is the permanent bindpose fallback — empty rows read rest pose, but you must still drive the slot weight to 0.

### Blending per instance

`GpuInstanceAnims` holds the live state: per-slot weights (eased toward targets each frame), clocks, rates, the blend-slot → bank wiring, shape weights, and root scales:

```rust
// Point blend slot 0 at walk, slot 1 at strafe, then crossfade:
anims.set_slot(i, 0, walk_slot);
anims.set_slot(i, 1, strafe_slot);
anims.set_target(i, &[walk_weight, strafe_weight]);
```

`set_rate` changes playback speed, `trigger_one_shot` restarts a `OnceHold` clip.

### Morphs and body shapes

The GPU mesh keeps its morph targets, so per-entity `MeshMorphWeights` work through the GPU vertex shader. Skeletons need the same treatment: fit one `GpuShapeSkeleton` per template shape (`fit_shape_skeleton` / `fit_shape_skeleton_from_helpers`), register it on `GpuCrowdShapes` (slot `i` = `shapes[i]`, slice `i + 1` on the GPU, slice 0 is always reference), and drive matching per-instance weights plus a root-scale correction:

```rust
shapes.register(fit_shape_skeleton_from_helpers("baby", &helpers, &bank.bones, &rig, &vg));
anims.set_shape_weights(index, &[bodybuilder_w, baby_w]);
anims.set_root_scale(index, scale);
```

`gpu_morphs.rs` (baby / bodybuilder / hybrids sharing one mesh) is the reference. Keep the entity morph-weight order and the shape registration order the same or the skeleton won't match the mesh.

### Joints readback (optional)

Off by default. Set `readback_joints: true` and the plugin copies the posed `joints` buffer to the CPU every frame into `GpuJointsReadback` (row-major `instance * num_bones + bone` matrices, 1–2 frames stale, no main-thread stall). Gameplay reads the resource, never the GPU:

```rust
if let Some(m) = readback.joint(instance, bone) { /* hitbox math */ }
```

### Custom crowd materials

`CrowdMaterial` is an `ExtendedMaterial<StandardMaterial, GpuCrowdExtension>`. For your own shading, compose the shared `gpu_skin_wgsl()` snippet with your own `@vertex` entry that calls `gpu_skin_vertex`, and route the joint attributes with `specialize_gpu_vertex_layout` (locations 6/7). The examples do exactly this via `CustomCrowdMaterial` in `examples/shared/mod.rs` + `crowd_vertex.wgsl`.

## Ragdoll physics

> **Status note: ragdolls are the weakest part of this crate right now, and I'm not happy with how they look.** Colliders spawn, joints form, and characters fall over — the plumbing works — but the motion doesn't look good yet. Expect twitch and jitter (the examples run higher-than-default `RagdollDensity` just to calm it down), partial ragdolls fighting their kinematic parents.

With that said, the pieces:

- Bones are never physics bodies. Bones carry only `Parent`/`Children` + `Transform`/`GlobalTransform`. Colliders are separate entities with `RigidBody`, linked to bones via `BoneForCollider` and `ColliderOffset`, and `sync_bones_to_ragdoll` writes dynamic collider positions back to the skeleton in `HumentitySkeletonSystemSet`.
- Flip `CharacterRagdoll` between `None`, `Full`, and `Partial(bones)` to go limp. Kinematic colliders track their bones each `FixedUpdate`; dynamic ones drive them.
- Tune with `RagdollDensity`, `RagdollDamping`, `RagdollMobility` (0 = locked, 1 = full anatomical range), `RagdollJointLimitOverrides` (per-bone swing/twist or hinge limits), and `RagdollCollisionLayers`. Limits resolve through `default_joint_limit` / `resolve_joint_limit`.
- `ragdoll_dof.rs` floats the character and sweeps one joint degree of freedom at a time so you can judge each limit. Use it before touching the joint tables.
- After a ragdoll ends, fire `commands.trigger(ResetToBindPose(character))` to snap bones back to the fitted bind pose before restarting the clip — otherwise stale ragdoll translations survive under the rotation-only animation.
- `CharacterScale` is respected: collider shapes are built scaled and joint anchors are resolved in the scaled bone frame.
- The `physx` feature keeps an alternate backend (`bevy_mod_physx`), but it is most likely broken: it hasn't been maintained and may or may not come back. avian3d is the active backend.

## Examples

| Example | Description |
|---|---|
| `lod.rs` | Distance-based mesh and skeleton LOD with `VisibilityRange` |
| `stress_test.rs` | 576 CPU-animated characters (24x24 grid) |
| `animation.rs` | Retargeted idle animation on a morphed character |
| `morphs_and_templates.rs` | Template system and runtime morph targets |
| `stitched_parts.rs` | Multi-part meshes with continuous normals and per-part morphs |
| `assets.rs` | Loading body parts, clothing, hair, and accessories |
| `character_creator.rs` | Real-time mesh modification UI with sliders |
| `gpu_crowd.rs` | 10,000 GPU-posed characters sharing one mesh/material |
| `gpu_morphs.rs` | GPU posing with morph targets + blended shape skeletons |
| `ragdoll_avian.rs` | Full-body ragdoll with avian3d physics (Space toggles) |
| `ragdoll_avian_partial.rs` | Partial ragdoll (arms only) with kinematic colliders |
| `stress_test_ragdoll_avian.rs` | Grid of ragdoll characters with sleep timers |
| `ragdoll_dof.rs` | One-joint-at-a-time limit tuning rig |
| `ragdoll_physx.rs` | Ragdoll via the alternate `physx` backend (likely broken, unmaintained) |

Run examples with:

```sh
cargo run --example lod
cargo run --example stress_test
cargo run --example animation
cargo run --example gpu_crowd
cargo run --example gpu_morphs
cargo run --example ragdoll_avian --features avian
cargo run --example ragdoll_dof --features avian
```

## Feature flags

| Feature | Description |
|---|---|
| `avian` | Enables ragdoll physics via [avian3d](https://github.com/Jondolf/avian) (active backend) |
| `physx` | Alternate backend via `bevy_mod_physx` (off by default; most likely broken, unmaintained — may or may not come back) |
| `debug` | Enables Bevy's debug rendering |

## Custom assets

Custom meshes, morph targets, and rig data can be authored in Blender using [MPFB](https://github.com/makehumancommunity/makehuman-plugin-for-blender) and exported as `.mhclo`/`.obj` files with `.target` shape keys. The crate's native asset loaders handle these formats automatically.

## Animation and ragdoll bone sync

The `HumentitySkeletonSystemSet` system set runs in `PostUpdate` after Bevy's `AnimationSystems` and before `TransformSystems::Propagate`. The built-in `rescale_root_bone_translation` system (corrects root bone Y translation for different human proportions) and `sync_bones_to_ragdoll` (writes ragdoll collider positions back to the skeleton) both run in this set.

To run your own systems after this pass, order them relative to this set:

```rust
app.add_systems(
    PostUpdate,
    my_system.after(HumentitySkeletonSystemSet),
);
```

Joint (re)spawn has its own set: order tooling that seats collider bodies and flips `CharacterRagdoll` before `HumentityRagdollSystemSet`, since joint rest frames are baked from the collider bodies as they stand when the set runs.
