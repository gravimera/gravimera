# Scene Sources and Build Artifacts (VCS-Friendly Scenes)

_(Spec document; see `docs/gamedesign/22_scene_creation.md` and `docs/gamedesign/29_observability_and_resumability.md` for product goals.)_

This spec defines how a Gravimera scene is represented so it can be:

- managed in a VCS (especially git) as a “process repo”,
- edited in parallel by multiple agents/humans,
- reviewed via diffs,
- merged without binary conflicts,
- and still loaded quickly at runtime via optional binary build artifacts.

The key idea is to separate:

- **authoritative scene sources** (text, canonical, mergeable), from
- **derived build outputs** (binary caches like today’s `scene.dat` / `instances.dat`).

## Non-Negotiable Properties

1) **Textual source of truth**: every scene has an authoritative text representation suitable for diffs, code review, and merges.
2) **Clear separation of concerns**: source files describe intent/structure; build outputs are caches that can be regenerated.
3) **Deterministic compilation**: given the same sources + seed policy + engine version, compilation produces the same build outputs and stable ids.
4) **Stable identifiers**: scene elements referenced by story/brains/portals use stable ids that survive edits and regeneration.
5) **Decomposable edits**: sources can be split so that independent edits touch different files (minimize merge conflicts).
6) **Canonicalization**: tools must be able to rewrite sources into a canonical form so diffs are stable and whitespace/order noise is eliminated.

## Definitions

- **Scene workspace**: the on-disk directory used for authoring a scene (often a git repo).
- **Scene sources**: authoritative, canonical text files describing the scene (procedural layers, pinned instances, markers, portals, metadata).
- **Build outputs**: derived caches produced from sources to speed up loading/simulation (binary instance tables, nav caches, baked render data).
- **Compilation**: deterministic process that turns sources into build outputs.
- **Checkpoint**: a durable “good state” of a scene workspace (often a commit) that can be resumed or branched.

## Source vs Build: Authority Rules

- **Sources are authoritative.** Build outputs must be regenerable from sources.
- Build outputs **must not** be required for a realm package to be editable; they are performance caches.
- Runtime formats like today’s `scene.dat` are considered **build outputs** for process management purposes.

## Scene Directory Layout (Normative)

Within a realm package (see `docs/gamedesign/20_realm_package_manifest.md`) each scene directory uses:

    scenes/<scene_id>/
      scene.json
      src/
        index.json
        meta.json
        markers.json
        portals/
          <portal_id>.json
        layers/
          <layer_id>.json
        pinned_instances/
          <instance_id>.json
        style/
          style_pack_ref.json
      build/                      (optional; derived)
        instances.dat             (optional)
        nav.dat                   (optional)
        build_manifest.json       (optional)
      runs/                       (optional; durable artifacts for long runs)
        <run_id>/
          ...

Notes:

- `src/` is designed to be committed and merged. It is the **source of truth**.
- `build/` is designed to be discarded and regenerated. It should be treated as a cache.
- `runs/` is designed for durable debugging/resume artifacts; hosts may prune it by policy.

## `scene.json` Responsibilities (Manifest)

`scene.json` is the entry point for a scene and must include:

- `format_version`
- `scene_id`, label, and scene metadata (biome/lighting/nav settings)
- pointers to the source index under `src/` (required)
- optional pointers to build outputs (if present)
- an **authority declaration** so loaders know what must exist:
  - `authority = "source"` (recommended): load from `src/` and optionally use `build/` if valid
  - `authority = "build_only"` (optional distribution mode): load only from build outputs (not editable without sources)

This spec does not mandate exact JSON field names, but it mandates these responsibilities.

## `src/index.json` Responsibilities (Source Index)

The source index exists so tools can:

- discover all source files for a scene,
- validate referential integrity across files,
- canonicalize/rewrite sources deterministically.

`src/index.json` must include:

- `format_version`
- pointers/paths to `meta.json`, `markers.json`
- list of portal records (either as paths or ids -> paths)
- list of layer records (layer id -> path)
- pointer to pinned instance directory (or list of pinned instance files)
- pointer to style pack reference(s)

## Portal Files (Directed Travel Edges)

Portals are authored as separate JSON files under:

    src/portals/<portal_id>.json

Portals are **one-way** by default. For bidirectional travel, creators place two portals (A→B and B→A).

### Portal File Schema (v1, minimal / normative)

Minimum required fields (v1):

- `format_version`: integer (currently `1`)
- `portal_id`: string (stable within the scene)
- `destination_scene_id`: string (target scene id within the same realm by default)

Optional fields (v1):

- `from_marker_id`: string (marker id in this scene used as an authoring anchor for the portal gate)

Additional fields are allowed and must be preserved by round-trip tools when possible.

## Layer Files and Ownership

Procedural layers are authored as separate files under `src/layers/` to make parallel editing practical.

Each layer must include:

- `format_version`
- `layer_id` (stable within the scene)
- deterministic seed policy (explicit)
- declarations of which instances it owns (conceptually), to support regeneration and provenance

### Layer File Schema (v1, minimal / normative)

Layer files are JSON objects stored under:

    src/layers/<layer_id>.json

Minimum required fields (v1):

- `format_version`: integer (currently `1`)
- `layer_id`: string (stable within the scene)
- `kind`: string (layer kind)

The minimal generic layer kind supported by the engine is:

#### `kind = "explicit_instances"`

This kind is a “manual placement layer”: it explicitly enumerates concrete instances, but they are
still **owned by the layer** (not pinned), so regeneration semantics remain consistent.

Fields:

- `instances`: array of instance specs
  - `local_id`: string (stable identifier within this layer; required for deterministic ids)
  - `prefab_id`: UUID string
  - `transform`: object
    - `translation`: `{ "x": <f32>, "y": <f32>, "z": <f32> }`
    - `rotation`: `{ "x": <f32>, "y": <f32>, "z": <f32>, "w": <f32> }` (quaternion)
    - `scale`: `{ "x": <f32>, "y": <f32>, "z": <f32> }`
  - `tint_rgba` (optional): `{ "r": <f32>, "g": <f32>, "b": <f32>, "a": <f32> }` (linear RGBA)

Placement notes (non-normative):

- `transform.translation` is applied to the instance root as-is.
- To rest an object on the ground plane (y=0), set:
  - `translation.y = ground_origin_y * abs(scale.y)` when the prefab defines `ground_origin_y` (realm prefab packs).
  - otherwise `translation.y = (prefab_size.y * abs(scale.y)) / 2`.

Additional fields are allowed and must be preserved by round-trip tools when possible.

Additional procedural layer kinds are defined in:

- `docs/gamedesign/33_scene_layer_kinds_v1.md`

### Deterministic Instance Id Derivation (v1, normative)

For `explicit_instances` (and any future procedural layer kinds), the engine must derive a stable
UUID for each compiled instance. The v1 rule is:

- Use UUID v5 with namespace `uuid::Uuid::NAMESPACE_URL`.
- The name/key is the UTF-8 string:

    gravimera/scene_sources/v1/scene/<scene_id>/layer/<layer_id>/instance/<local_id>

This produces deterministic instance ids across machines and runs and enables safe regeneration
without duplicates.

Regeneration uses the rule defined in `docs/gamedesign/22_scene_creation.md`:

- **layers own their outputs unless pinned**

Pinned instances live outside layer ownership (see below).

## Pinned Instances (Hand-Owned Exceptions)

Pinned instances exist to:

- preserve hand-crafted set pieces across regeneration,
- allow “surgical” edits without rewriting a whole layer,
- keep diffs small and intent clear.

Pinned instances are stored as separate files:

    src/pinned_instances/<instance_id>.json

Each pinned instance must include:

- `format_version`
- stable `instance_id`
- `prefab_id` reference and transform
- optional tags/overrides used by story/brains
- optional provenance field (“pinned from layer X at compile signature Y”) for debugging

### Pinned Instance File Schema (v1, normative)

Pinned instances are authored as JSON objects:

- `format_version`: integer (currently `1`)
- `instance_id`: UUID string (stable identifier for story/brains references)
- `prefab_id`: UUID string (builtin or generated prefab id)
- `transform`: object
  - `translation`: `{ "x": <f32>, "y": <f32>, "z": <f32> }`
  - `rotation`: `{ "x": <f32>, "y": <f32>, "z": <f32>, "w": <f32> }` (quaternion)
  - `scale`: `{ "x": <f32>, "y": <f32>, "z": <f32> }`
- `tint_rgba` (optional): `{ "r": <f32>, "g": <f32>, "b": <f32>, "a": <f32> }` (linear RGBA)

Placement notes (non-normative):

- `transform.translation` is applied to the instance root as-is.
- To rest an object on the ground plane (y=0), set:
  - `translation.y = ground_origin_y * abs(scale.y)` when the prefab defines `ground_origin_y` (realm prefab packs).
  - otherwise `translation.y = (prefab_size.y * abs(scale.y)) / 2`.

Additional fields are allowed and must be preserved by round-trip tools when possible.

## Canonicalization Rules (Diff Stability)

To keep diffs stable and merge-friendly, tools must be able to rewrite sources into a canonical form:

- stable ordering for lists (sorted by stable ids)
- stable key ordering for objects (project-defined canonical order)
- normalized numeric formatting (avoid platform-dependent float formatting)
- forbid nondeterministic fields in sources (timestamps, host paths, random ids)

Canonicalization is required for multi-agent workflows: two agents making the same semantic change should produce the same file bytes after formatting.

## Build Outputs (`build/`)

Build outputs are optional derived caches. Typical outputs:

- `build/instances.dat`: packed instance representation for fast load/simulation
- `build/nav.dat`: navigation cache

If build outputs are present, `build/build_manifest.json` must allow staleness detection:

- content hash of the source set (or source index hash)
- engine version + determinism compatibility version
- compilation options used (if any)

Loaders must treat build outputs as invalid if the manifest does not match the current sources and engine compatibility policy.

## Runs (`runs/`) and Long-Running Generation

For long-running scene generation/editing, a scene workspace may persist durable run artifacts under `runs/`:

- input specs (intent/scorecard/seed policy)
- blueprint patches applied over time
- validation reports and evidence pointers
- stable signatures for determinism/regression checks

This enables crash-resume and automated debugging without rerunning the full pipeline.

Run artifacts must not be required to load the scene; they are for process management and developer/agent tooling.
