# Mechanical Transform Mapping v2 (Grouped Min‑Cost Assignment)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository’s ExecPlan requirements live in `PLANS.md` at the repo root. Maintain this document accordingly.

## Purpose / Big Picture

Today, switching object forms is visualized by an automatic “mechanical transform” animation that maps old-form primitives to new-form primitives, then interpolates transforms/colors for mapped pairs and fades unmatched parts in/out.

For complex objects (200+ leaf primitives), the current mapping is too “list-order driven”, so important structure can mismatch (support parts map to mid-body parts, left/right swap, head maps to torso, etc.). This makes the animation look random rather than mechanical.

After this change, mapping becomes:

- More coherent: support↔support, core↔core (“torso”), head↔head emerges automatically from geometry.
- Scalable: mapping remains fast for 200+ leaves and when transforming multiple selected objects at once.
- Deterministic: no randomness; same inputs produce the same mapping every run.
- Fully automatic: no authoring UI, tags, or per-object tuning.

This plan does not change the player inputs (`Tab` and hold-`C` copy) or persistence. It only improves how the animation pairs parts.

## Progress

- [x] (2026-02-20) Implement geometry feature extraction (`pos01`, radial distance, size proxy).
- [x] (2026-02-20) Implement deterministic grouping via median-split clustering.
- [x] (2026-02-20) Implement min-cost assignment (Hungarian) and use it for both cluster matching and leaf matching.
- [x] (2026-02-20) Integrate mapping v2 into form-switch animation and keep v1 as fallback.
- [x] (2026-02-20) Add unit tests for Hungarian correctness + mapping determinism.
- [x] (2026-02-20) Run smoke test (`cargo run -- --headless --headless-seconds 1`).
- [x] (2026-02-20) Commit.

Deferred (not required for correctness; revisit if perf issues appear):

- [ ] Add baseline timing instrumentation for mapping.
- [ ] Add runtime caching keyed by prefab ids (requires a safe invalidation strategy because `ObjectLibrary` can `upsert` defs).

## Surprises & Discoveries

- Treating “same primitive kind” as a hard first-pass priority can produce visibly wrong transforms when roles change across forms (e.g. wheels→legs): geometry-role coherence must be allowed to override primitive kind.
  - Resolution: mapping uses a single min-cost assignment with a *finite* kind-mismatch penalty, so it prefers same-kind morphs when reasonable but can choose cross-kind pairs when geometry is a much better match.

## Decision Log

- Decision (2026-02-20): Use a single Hungarian assignment with `kind_mismatch_penalty` instead of a strict “same kind first” pass.
  - Rationale: better supports “support/core/head” role coherence when primitive kinds differ between forms.
- Decision (2026-02-20): Use global assignment for up to `~256` leaves; fall back to grouped assignment (clusters) above that.
  - Rationale: improves quality for typical 200+ leaf objects while keeping a scalable path for very large objects.
- Decision (2026-02-20): Keep the original v1 mapping as a fallback/debug option.

## Outcomes & Retrospective

- Delivered a deterministic, geometry-aware mapping that reduces obvious structural mismatches for large (200+) primitive objects.
- Added unit tests and kept the runtime behavior fully automatic (no authoring UI).

## Context and Orientation

Relevant existing behavior lives in:

- `src/object_forms.rs`
  - `begin_form_transform_animation`: clears visuals, flattens both prefabs to leaf parts, builds a mapping, spawns a temporary animation rig.
  - `flatten_leaf_visuals(_inner)`: resolves `ObjectRef` composition and attachments/anchors to produce a flat list of leaf primitives/models with accumulated `Transform`.
  - `resolve_leaf_assets`: turns leaf prototypes into `LeafResolved { key, transform, spawn }` where `key` is a primitive kind key (`MeshKey`) or model path and `spawn` carries mesh/material handles.
  - `build_leaf_mapping`: current mapping algorithm:
    - first pairs exact `LeafKindKey` matches (same primitive/model type) by list order within each key,
    - then pairs remaining leaves by list order (no geometry cost).
  - `LeafAnimSpawnSpec`: builds animation “spawn specs”:
    - same type → interpolate transform and color,
    - different type → shrink/fade old + grow/fade new,
    - unmatched → fade in/out (currently also moves via center for better effect).

Terminology used in this plan:

- “Leaf”: the final visual primitive or model scene after flattening composition (`ObjectRef`) and attachment transforms. Leaves are animated during a form switch.
- “Mapping”: a set of `(old_leaf_index, new_leaf_index)` pairs that decides which old leaf morphs into which new leaf.
- “Cluster / Group”: a small set of leaves that are spatially near each other in object-local space, used to reduce mapping complexity.
- “Min-cost assignment”: an algorithm that finds the globally optimal one-to-one pairing between two sets given a numeric cost per possible pair (a standard algorithm; commonly implemented with the Hungarian method).

Constraints:

- Must be fully automatic: no tags like “head”, “torso”, “wheel”, etc.
- Must be deterministic.
- Must scale to 200+ leaves per prefab and multi-select transforms.
- Must preserve the existing animation rules (same-type morph vs cross-type shrink/grow vs unmatched fade).

## Proposed Algorithm (Design)

The high-level approach is hybrid:

1) If `max(old_leaves, new_leaves) <= GLOBAL_ASSIGN_MAX_LEAVES`, run one global min-cost assignment using geometry cost plus a finite kind-mismatch penalty.
2) Otherwise, build deterministic clusters, match clusters (old clusters ↔ new clusters) with min-cost assignment, then match leaves within each matched cluster pair using the same leaf-level cost function.

This produces the “support/core/head” priority as an emergent property because the cost function heavily weights vertical position and proximity, and clustering keeps mapping local.

### Step A: Compute leaf features in normalized object-local space

For each prefab, after flattening leaves, compute:

- `pos`: leaf translation in object-local space (from existing `LeafResolved.transform.translation`).
- `scale`: leaf scale (`LeafResolved.transform.scale`).

Compute an object-local bounding box over leaf positions:

- `min = componentwise_min(pos)`
- `max = componentwise_max(pos)`
- `extents = (max - min).max(Vec3::splat(eps))` where `eps` prevents divide-by-zero.

Derive a normalized position:

- `pos01 = (pos - min) / extents` so each axis is approximately in `[0, 1]`.
- `center01 = pos01 - Vec3::splat(0.5)` for “distance to center”.

Compute scalar features:

- `h = pos01.y` (height: support ≈ low, head ≈ high)
- `r = length(Vec2(center01.x, center01.z))` (radial distance in XZ: core vs extremities)
- `v = abs(scale.x * scale.y * scale.z)` (proxy for part “size”)
- `sx = sign(center01.x)` and `sz = sign(center01.z)` (left/right and front/back cues; useful to reduce symmetry swaps)

Store this as `LeafFeatures` alongside each leaf index.

### Step B: Deterministic clustering (grouping) by spatial median-split

Goal: produce clusters of size ~`CLUSTER_LEAF_TARGET` (e.g. 24–40) with no per-object tuning and no randomness.

Use a deterministic median-split recursive partition (a simple BVH/k‑d style build):

- Start with the full set of leaf indices.
- If group size ≤ `CLUSTER_LEAF_TARGET`, emit it as a cluster.
- Else:
  - pick the axis with the largest range in `pos01` among leaves in the group (tie-break order `y > x > z` to emphasize support/head separation first).
  - stable-sort indices by that axis (tie-break by the other axes, then by original index).
  - split at median into two subgroups and recurse.

Each emitted cluster stores summary stats:

- `centroid01`: mean `pos01`
- `mean_h`, `mean_r`, `mean_log_v`
- `aabb01_min/max` (cluster-local bounds in normalized space)
- `kind_counts`: counts of each primitive `MeshKey` rank plus a separate `model_count`

This clustering is deterministic, parameter-light (only `CLUSTER_LEAF_TARGET`), and scales `O(n log n)`.

### Step C: Match clusters old↔new with min-cost assignment

Let old have `M` clusters and new have `N` clusters (usually both around `n / CLUSTER_LEAF_TARGET`).

Build a cost matrix `C` of size `max(M, N)` by padding with dummy clusters:

- Real cluster↔cluster cost is a weighted sum:
  - centroid distance: `||centroid_old - centroid_new||²`, with `y` weight higher than `x/z`
  - AABB extent similarity: `||extent_old - extent_new||²`
  - kind histogram distance: `L1(kind_counts_old - kind_counts_new)` normalized by leaf count
  - optional symmetry hint: penalize sign flips for clusters with clearly non-zero `centroid01.x` or `centroid01.z`
- Real cluster↔dummy cost is a large constant (`UNMATCHED_CLUSTER_COST`) so the assignment leaves unmatched clusters only when counts differ a lot.

Run Hungarian (min-cost assignment) on this small matrix. This produces a cluster pairing that keeps mapping local (support groups map to support groups, etc.).

Unmatched clusters on either side become sources of unmatched leaves (fade out / fade in).

### Step D: Match leaves within matched cluster pairs

For each matched `(old_cluster, new_cluster)`:

1) Build a padded square cost matrix for the leaf indices in these clusters and run Hungarian once:
   - `cost = geometry_cost + (key_mismatch ? KIND_MISMATCH_COST : 0)`
   - `geometry_cost` is a weighted sum of normalized position distance (Y weighted higher than X/Z), radial distance, size proxy, and an optional left/right/front/back sign flip penalty.
2) Emit mapping pairs for assigned rows/cols, marking `same_key = (old.key == new.key)` so the animation can decide “morph” vs “shrink/grow”.
3) Remaining unmatched leaves (due to count mismatch) still use the existing fade/shrink/grow behavior:
   - old unmatched → `fade_out` (already shrinks/fades and now also moves to center)
   - new unmatched → `fade_in`

Leaf-level cost (for both passes) should be integerized for determinism:

- Base terms:
  - `pos_cost = w_pos * ||pos01_old - pos01_new||²`
  - `height_cost = w_h * (h_old - h_new)²` (support/head bias)
  - `radial_cost = w_r * (r_old - r_new)²` (core/extremity bias)
  - `size_cost = w_v * |log(v_old) - log(v_new)|`
  - `side_cost = w_side * side_flip(old, new)` where `side_flip` is 1 when signs disagree and magnitudes are above a small threshold, else 0
- Optional: `rot_cost` using quaternion angular distance if rotation tends to matter for your shapes.

Convert final `f32` cost to `i64` via `round(cost * 1_000_000.0)` and add a deterministic tie-breaker term like `(old_idx as i64) * 3 + (new_idx as i64)` scaled by a tiny epsilon to stabilize equal costs.

## Performance Strategy (200+ leaves)

For typical “large” objects in this repo (~200 leaves), a global `O(n³)` Hungarian assignment is workable and produces the highest quality mapping (no cluster-boundary artifacts). For very large objects, we need a scalable path.

Strategy:

- If `max(old_leaves, new_leaves) <= ~256`, run a single global Hungarian assignment.
- Otherwise, use deterministic clustering + Hungarian cluster matching, then Hungarian leaf matching within each matched cluster pair.

With clustering:

- Cluster count ≈ `n / CLUSTER_LEAF_TARGET` (e.g. `200 / 32 ≈ 7`).
- Cluster matching is `O(m³)` with `m ≈ 7` (tiny).
- Leaf matching is `Σ O(k_i³)` with `k_i ≤ CLUSTER_LEAF_TARGET` (e.g. `7 * 32³ ≈ 230k` ops).

Optional: Add a runtime cache later, but `ObjectLibrary` supports `upsert`, so caching needs a safe invalidation strategy (prefab revision tracking or input hashing).

## Plan of Work

1) Baseline and instrumentation
   - In `src/object_forms.rs`, wrap mapping build with a simple timer (e.g. `Instant`) behind a debug flag or `RUST_LOG=debug` and record the time for a known 200+ leaf object transform.
   - Confirm where time is spent today (flatten vs resolve assets vs mapping).

2) Refactor mapping into a focused module (optional but recommended)
   - Move mapping-related structs/functions out of `src/object_forms.rs` into `src/object_forms/mapping.rs` (or `src/object_forms_mapping.rs`) to keep `object_forms.rs` readable.
   - Keep public surface minimal: a single function that returns mapping pairs for two `Vec<LeafResolved>`.

3) Add feature extraction
   - Define `LeafFeatures` and a `compute_leaf_features(leaves: &[LeafResolved]) -> PrefabLeafFeatures` that computes `pos01`, `h`, `r`, `log_v`, and sign hints.
   - Ensure it handles degenerate AABBs (all parts on a plane/line) by using `eps`.

4) Implement deterministic median-split clustering
   - Define `LeafCluster { indices: Vec<usize>, summary: LeafClusterSummary }`.
   - Implement `build_clusters(features: &PrefabLeafFeatures, target: usize) -> Vec<LeafCluster>` using the median split described above.
   - Unit test: clustering is stable given the same inputs.

5) Implement min-cost assignment (Hungarian) for small matrices
   - Implement a self-contained Hungarian solver that accepts:
     - `costs: Vec<Vec<i64>>` (square, padded)
     - returns `assignment: Vec<usize>` mapping each row to a column.
   - Keep it deterministic: avoid hash maps in core loops; stable iteration order.
   - Unit tests: known small matrices return the expected assignment; rectangular case via padding.

6) Implement cluster matching
   - Compute old/new clusters (from cached features).
   - Build padded cost matrix and run Hungarian.
   - Return a list of matched cluster pairs and lists of unmatched old/new clusters.

7) Implement leaf matching inside clusters
   - For each matched cluster pair:
     - run a single Hungarian assignment using `cost = geometry_cost + (key_mismatch ? KIND_MISMATCH_COST : 0)`.
   - Produce final mapping pairs `(old_idx, new_idx, same_key_bool)` where `same_key` is `old.key == new.key`.

8) Integrate into the existing animation pipeline
   - Replace `build_leaf_mapping` in `begin_form_transform_animation` with `build_leaf_mapping_v2_grouped`.
   - Keep v1 mapping as a fallback (e.g. if clustering produces empty clusters, or solver fails) and as a debug comparison.

9) Cache results
   - Optional: add a runtime cache if perf issues appear.
   - Note: `ObjectLibrary` supports `upsert`, so caching requires a safe invalidation strategy.

10) Tests and acceptance scenes
   - Add a focused unit test module for:
     - determinism (same input → same pairs),
     - “head/support bias” using a synthetic set of leaves (top cluster maps to top cluster).
   - If you need fixture scenes or large prefab examples, store them under `test/` per `AGENTS.md`.

11) Documentation
   - Keep `docs/gamedesign/37_object_forms_and_transformations.md` unchanged unless you introduce user-visible changes.
   - If needed, add a short note that mapping is cost-based and grouped for performance, but avoid over-specifying weights.

## Concrete Steps

All commands run from repo root:

    cargo test
    cargo run -- --headless --headless-seconds 1

Manual mapping sanity run:

    cargo run

Then:

- Create or locate an object with 200+ leaf primitives (a dense Gen3D object is fine).
- Select it and press `Tab` to switch forms several times.
- Observe whether:
  - support parts (low) tend to morph to support parts (low),
  - head/top parts tend to morph to head/top parts,
  - left/right swapping is reduced,
  - transform feels “local” rather than teleporting parts across the object.

## Validation and Acceptance

Acceptance criteria:

- Mapping quality:
  - For a complex object with 200+ leaves, transforms look coherent (no obvious head↔leg swaps, fewer symmetry swaps).
  - Unmatched parts still look good (fade + center in/out remains).
- Determinism:
  - Multiple runs produce identical pairings for the same prefab pair.
- Performance:
  - Transforming a 200+ leaf object does not cause noticeable frame hitching.
  - Transforming multiple selected objects remains responsive.
- Safety:
  - `cargo test` passes.
  - Smoke test passes (`cargo run -- --headless --headless-seconds 1`).

## Idempotence and Recovery

- The change is runtime-only: if something goes wrong, keep a debug toggle to fall back to v1 mapping until v2 is stable.
- If the Hungarian implementation is buggy, temporarily fall back to v1 mapping.

## Artifacts and Notes

Implemented constants live in `src/object_forms/mapping.rs`.

Current values (2026-02-20):

- `GLOBAL_ASSIGN_MAX_LEAVES = 256`
- `CLUSTER_LEAF_TARGET = 32`
- `KIND_MISMATCH_COST = 0.75` (finite, so geometry can override kind when mismatch is large)

Do not overfit these constants to a single object; the algorithm must remain generic across arbitrary shapes.

## Interfaces and Dependencies

Target end-state interfaces (names can be adjusted, but keep the structure):

- A leaf feature struct:

    struct LeafFeatures {
        idx: usize,
        key: LeafKindKey,
        pos01: Vec3,
        h: f32,
        r: f32,
        log_v: f32,
        side_x: f32,
        side_z: f32,
    }

- Cluster builder:

    fn build_leaf_clusters(features: &[LeafFeatures], target: usize) -> Vec<LeafCluster>;

- Assignment solver:

    fn hungarian_min_cost(costs: &[Vec<i64>]) -> Vec<usize>;

- Mapping v2 entrypoint:

    fn build_leaf_mapping_v2_grouped(
        old: &[LeafResolved],
        new: &[LeafResolved],
    ) -> LeafMapping;

- Optional cache resource:

    #[derive(Resource, Default)]
    struct FormTransformMappingCache { ... }
