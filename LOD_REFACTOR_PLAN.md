# humentity — Single-Skeleton LOD Refactor Plan

## Context (no prior conversation needed)
We are refactoring skeleton LOD in the **humentity** crate (`C:\Users\matt\Documents\rust\rpg\crates\humentity`). humentity is a **submodule crate with its own git repo**. Changes here are to humentity only — **do not** touch the workspace `characters` crate or other crates this session.

**Current (broken/over-engineered) design:** For each LOD level we construct a *separate skeleton scene* (a reduced-bone `DynamicWorld`), with per-LOD inverse bindposes, per-LOD merged weights, per-LOD animation target ids, and per-LOD bone hierarchy. Each character spawns up to 4 skeleton scenes. `SkeletonLodMap([Option<Entity>;4])` tracks them; whole-skeleton enable/disable toggles `SkeletonLodDisabled` recursively. Physics (`physics/avian.rs`) maintains `SkeletonLodState { active, just_enabled, canonical }` and a `lod_bone_entities` per-collider×per-LOD table, plus Phase A/B "replicate canonical transform to every LOD" logic in `sync_bones_to_ragdoll`.

**Target:** ONE fixed full skeleton per character (all bones exist as entities). Skeleton LOD = enabling/disabling **bone sub-trees** within it. The whole purpose is to **stop GlobalTransform propagation** on unneeded bones (a known bottleneck) by inserting the registered disabling component `SkeletonLodDisabled` on those sub-trees.

## Core design decisions (already agreed)
1. **Keep the merge machinery.** Per-LOD `merged_weights` + surviving `bone_names` are still computed and used to paint each proxy mesh (a low-LOD mesh is fully weighted to surviving/parent bones, so it references fewer joints). `CharacterPart.skeleton_lod`, `LoadAssetMeshJob::Single.skeleton_lod`, and the mesh cache keys all STAY.
2. **Each LOD = cumulative list of subtree-root bone names** (empty = full skeleton). Lists are **cumulative/nested** (`LOD0 ⊂ LOD1 ⊂ LOD2`), built exactly as today via chained `BoneMergeConfig::full().without_children_of(...)` / `.clone().without_children_of(...)`. No semantic change to `BoneMergeConfig`/`SkeletonLodConfig`.
3. **Reconcile rule:** a root `R` is disabled **iff `R` is present in EVERY active LOD's cumulative list** (i.e. `R` ∈ intersection of active LOD lists). This is the safe rule: during a crossfade only sub-trees removed by *all* active LODs are disabled; everything else stays enabled.
   - Steady `{2}` → disable LOD2's full list (toes+face+hands).
   - Crossfade `{0,2}` → only toes (present in both lists); face/hands stay on because the fading LOD0 mesh still uses them.
   - With nested cumulative lists this is monotonic and correct.
4. **Active-LOD tracking:** a component holding `active: [bool; MAX_LODS]` (array, not Vec). Reuse the existing name `SkeletonLodState` but slim it to just the `active` array; move it into `spawn_skeleton.rs` (it is a skeleton-LOD concern; physics no longer needs it). Examples write this array during crossfades.
5. **Ragdoll simplifies:** colliders attach to the single skeleton; drop the multi-LOD `lod_bone_entities`, canonical/`just_enabled` machinery, and Phase A/B replication.
6. **String interning:** bone-name lists are `&'static str` (via the existing `NAME_INTERNER`), so cumulative lists cost only pointers.

## Data structures (replacement)
- Replace `SkeletonLodVariant` (in `skeleton_lod.rs`) with:
  ```rust
  pub struct SkeletonLodData {
      pub bone_names: Vec<&'static str>,   // surviving bones, reference order (for mesh painting + part skinning)
      pub merged_weights: AHashMap<&'static str, AHashMap<u16, f32>>,
  }
  ```
  Drop from the old struct: `scene`, `bone_to_parent`, `local_bindpose`, `model_space_bindpose`, `inverse_bindposes`, `animation_target_ids`, `merge_config`.
- `RigBundle { lod_data: Vec<SkeletonLodData> }` (replace `lod_variants`).
- Replace `SkeletonLodMap([Option<Entity>;4])` with a single-skeleton component:
  ```rust
  pub struct CharacterSkeleton {
      pub skeleton_entity: Entity,
      pub bone_map: AHashMap<&'static str, Entity>,      // name -> bone entity in the fixed skeleton
      pub model_space_inv_bindposes: Vec<Mat4>,           // full, in reference bone order
  }
  ```
- Slim `SkeletonLodState` to `pub struct SkeletonLodState { pub active: [bool; MAX_LODS] }` (move to `spawn_skeleton.rs`).

## File-by-file changes

### `src/skeleton_lod.rs`
- Keep `BoneMergeConfig`, `all_children_of`, `resolve_remove_set`, `merge_weights`, `merge_hierarchy`. `without_children_of` stays **cumulative**.
- `SkeletonLodVariant` → `SkeletonLodData` as above.
- Rename `build_lod_variants` → `build_lod_data`; change return to `Vec<SkeletonLodData>`. Keep the merge logic exactly (resolve remove-set → surviving names via `merge_hierarchy` → `merge_weights`). **Remove** the per-LOD scene building (`build_merged_skeleton_scene`), inverse-bindpose asset creation, and `animation_target_ids` computation.
- `RigBundle { lod_variants }` → `RigBundle { lod_data: Vec<SkeletonLodData> }`.

### `src/rigs.rs`
- `build_rig_scenes`: build the **one** full skeleton scene via the existing `build_skeleton_scene(&reference_rig, world)`. Build `Vec<SkeletonLodData>` via `build_lod_data`. Store both in `RigBundleRes`. `set_asset_rig_arrays` unchanged.

### `src/spawn_skeleton.rs` (largest change)
- `CharacterSkeleton { lod }` → `CharacterSkeleton { skeleton_entity, bone_map, model_space_inv_bindposes }`; remove the `lod` field.
- Remove: `SkeletonLodMap`, `SkeletonLodFilter`, `SkeletonLocalBindPose`, `ResetSkeletonToBindPose`, `EnableSkeletonLod`, `DisableSkeletonLod`, and observers `on_enable_skeleton_lod`, `on_disable_skeleton_lod`, `on_reset_skeleton_to_bind_pose`.
- Add slim `SkeletonLodState { active: [bool; MAX_LODS] }` (moved from physics).
- `spawn_rig_skeletons` → `spawn_rig_skeleton`: spawn ONE full skeleton scene as a child of `CharacterShape` (no LOD loop, no filter), with `CharacterSkeleton` (skeleton_entity set) + `FitSkeleton`. Keep `MODEL_ROTATION_FIX` transform.
- `fit_skeleton_to_shape`: fit the single skeleton once. Build the full `bone_map` (name→entity from `SkinnedMesh.joints` + joint `Name`s). Compute the full `model_space_inv_bindposes` (in reference bone order). Insert `CharacterSkeleton` (all fields), remove `FitSkeleton`. Drop all per-LOD logic (variant bone names, per-LOD inverse bindpose handles, `SkeletonLocalBindPose`, `SkeletonRootBone` per-LOD).
- `check_skeletons_ready`: now simply checks the single skeleton is fitted (no `FitSkeleton`) → insert `SkeletonsReady` on the `CharacterShape`.
- `setup_part_skinning`: for each `CharacterPart`, get its `skeleton_lod`, look up `RigBundleRes.lod_data[idx].bone_names`, map each surviving name → entity via `CharacterSkeleton.bone_map`, build the inverse-bindpose subset by filtering `model_space_inv_bindposes` to those surviving bone indices, create a `SkinnedMesh`, and insert on the part. (Parts no longer pull `SkinnedMesh` from a per-LOD rig entity.)
- **Add** reconcile system `sync_skeleton_lod_subtrees` (Update):
  - Query: `(Entity, &SkeletonLodState, &CharacterSkeleton)`, gated `run_if`/`Changed<SkeletonLodState>` (or check change each frame).
  - Res: `Res<RigBundleRes>` or `Option<Res<SkeletonLodConfig>>`.
  - For each active LOD index `k` (from the config's count), and for each root name in that LOD's list (`SkeletonLodConfig.0[k].without_children_of` — the cumulative roots): `disabled = all active LOD indices m (0..config_count, active[m]) contain root`. If disabled → `insert_recursive::<Children>(SkeletonLodDisabled)` on `bone_map[root]`; else → `remove_recursive::<Children, SkeletonLodDisabled>`.
  - Handle roots present in the map; skip missing.
- `on_character_helpers_removed`: despawn the one `CharacterSkeleton.skeleton_entity`; remove `CharacterSkeleton` + `SkeletonsReady`; strip part meshes (unchanged logic).

### `src/spawn_mesh.rs`
- `build_single_mesh_process` and `build_stitched_meshes_process`: replace reads of `bundle.lod_variants[idx].bone_names/.merged_weights` with `bundle.lod_data[idx].bone_names/.merged_weights`. The `skeleton_lod` field on `LoadAssetMeshJob::Single` stays.

### `src/assets.rs`
- **No changes** — `build_final_mesh_mhclo` / `build_final_meshes_mhclo` still take `lod_bone_names` + `lod_weights`; callers pass the per-LOD data from `RigBundleRes.lod_data`.

### `src/physics/avian.rs`
- `CharacterColliders`: replace `lod_bone_entities: Vec<Vec<Option<Entity>>>` with `bone_entities: AHashMap<ColliderBone, Entity>` (single skeleton). Keep `bones_subset` (still used for partial ragdolls). Keep `collider_entities`, `joint_entities`, `bone_transforms` (already `[Transform; COLLIDERS.len()]`).
- Remove `SkeletonLodState`, `compute_canonical_lod`, `on_enable_skeleton_lod_ragdoll`, `on_disable_skeleton_lod_ragdoll`, `clear_just_enabled_lods`.
- `spawn_colliders`: build `bone_entities` from the single skeleton (use `CharacterSkeleton.bone_map` / the single skeleton's joints). Drop the per-LOD loop. Keep batch spawning, inverse-bindpose map, `ColliderOffset`, `ColliderForCharacter`, etc.
- `sync_colliders`: use `colliders.bone_entities[bone]` directly (no canonical LOD lookup).
- `set_ragdoll_state`: use `bone_entities` for joint anchor computation.
- `sync_bones_to_ragdoll`: collapse Phase A/B to a single direct write to the single skeleton's bones. Remove all LOD iteration and `applied_joint_world` multi-LOD handling; keep the lerp for smoothness and the parent-relative local computation using the single skeleton's hierarchy.
- `on_character_helpers_removed` (avian): remove `SkeletonLodState` reference; keep despawn of colliders/joints via lists.

### `src/lib.rs`
- Update module wiring: `spawn_skeleton` keeps the same systems but renames `spawn_rig_skeletons`→`spawn_rig_skeleton` and adds `sync_skeleton_lod_subtrees`; remove the observers for `EnableSkeletonLod`/`DisableSkeletonLod`/`ResetSkeletonToBindPose`; physics drops the removed observers/systems and `clear_just_enabled_lods`; keep `register_disabling_component::<SkeletonLodDisabled>()`.
- Update the `prelude` exports: remove `SkeletonLodMap`, `SkeletonLodFilter`, `SkeletonLocalBindPose`, `ResetSkeletonToBindPose`, `EnableSkeletonLod`, `DisableSkeletonLod`; add/expose `CharacterSkeleton` (new fields), slim `SkeletonLodState`, `RigBundle`/`SkeletonLodData`.

## humentity examples to update (must keep the crate compiling)
- `examples/shared/mod.rs`: `default_skeleton_lods()` stays cumulative (chained builders) — no change to the config list itself. Update any `SkeletonLodMap`/`EnableSkeletonLod` usage to write `SkeletonLodState { active: [...] }`.
- `examples/lod.rs`: replace `SkeletonLodFilter`, `SkeletonLodMap`, and the `EnableSkeletonLod`/`DisableSkeletonLod` triggers in `sync_skeleton_lod_to_visibility` with updating each character's `SkeletonLodState.active` array based on which proxy `VisibilityRange` is in range (set active[0..=lod] = true for the in-range lod, or set the single active lod). Remove `SkeletonLodFilter` usage on reference characters.
- `examples/stress_test.rs` and `examples/stress_test_ragdoll_avian.rs`: same replacement of `SkeletonLodMap` + enable/disable triggers with `SkeletonLodState.active` writes.
- `examples/morphs_and_templates.rs`: `on_skeletons_ready` currently fires `EnableSkeletonLod` — replace with writing `SkeletonLodState { active: [true, false, false, false] }` (or all false) on `On<Add, SkeletonsReady>`.
- `examples/ragdoll_avian_partial.rs`: uses `Allow<SkeletonLodDisabled>` in queries — keep; no structural change unless the query needs updating for the single skeleton.
- Mesh-building calls in examples (`LoadAssetMeshJob::Single { skeleton_lod }`, `CharacterPart { skeleton_lod }`) stay.

## What to remove (summary)
- `SkeletonLodVariant` (scene/bindpose/animation fields), `build_merged_skeleton_scene`.
- `SkeletonLodMap`, `SkeletonLodFilter`, `SkeletonLocalBindPose`, `ResetSkeletonToBindPose`, `EnableSkeletonLod`, `DisableSkeletonLod` + their observers.
- `SkeletonLodState` (physics version) → slimmed and moved to `spawn_skeleton`.
- `CharacterColliders.lod_bone_entities`, `compute_canonical_lod`, LOD ragdoll observers, `clear_just_enabled_lods`, Phase A/B multi-LOD replication.

## Verification
- Build only the humentity crate: run from `crates/humentity` (it is its own workspace/repo). **Must disable command timeouts** (per AGENTS.md builds always time out). Use e.g. `cargo build --features avian` (or the crate's default features) with the Bash tool timeout raised.
- Fix all compiler errors within humentity + examples.
- Optionally run the `lod` example (`cargo run --example lod`) to sanity-check skeleton fitting and LOD toggling visually; expect it to compile and run.

## Rules to honor (from AGENTS.md)
- **Do not create a `nul` file** on Windows; if one is found, delete it.
- humentity is a **submodule** with its own git repo — use `git` inside `crates/humentity`. Commit/push humentity FIRST, then update the gitlink at the workspace root via the `/commit` command workflow (submodule-first, dependency lock-and-revert, workspace root). Do **not** push unless the user asks.
- Do not change anything outside humentity this session.
- After writing each file, re-read AGENTS.md and verify each applicable rule was followed.
- Only implement what is in this plan (the single feature requested); do not add extra features.
- Prefer existing patterns/libraries; no new deps; keep unopinionated framework style (this is a general-purpose crate).

## Open item to confirm during implementation (non-blocking)
- Exact `MAX_LODS` constant (currently `4`) is preserved; `SkeletonLodState.active` is `[bool; MAX_LODS]`.
