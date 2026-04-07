# `scene.grav` (Scene Build Persistence)

This file stores a scene's build output: embedded prefab definitions plus placed build instances.

- Path: `<root_dir>/realm/<realm_id>/scenes/<scene_id>/build/scene.grav`
- Encoding: protobuf (binary)
- Schema: [`proto/gravimera/scene/v1/scene.proto`](/Users/flow/workspace/github/gravimera/proto/gravimera/scene/v1/scene.proto)

## Schema

`scene.grav` encodes `gravimera.scene.v1.SceneDat`:

- `version`: scene file format version (currently `9` in Rust as `SCENE_DAT_VERSION`)
- `units_per_meter`: quantization scale for stored positions (currently `100`, meaning centimeters)
- `defs`: embedded `SceneDatObjectDef` records (transitive closure of referenced prefab defs)
- `instances`: build instances (position/rotation/scale + selected forms)

The schema includes data-driven:

- Object visuals as parts (`object_ref`, `primitive`, or `model`)
- Attachments and anchors
- Animation specs (loop/spin/once/ping-pong; driver + basis)
- Interaction + simple collider profiles
- Optional mobility/projectile/attack/aim profiles

## Runtime Behavior

On rendered startup, Gravimera queues a one-shot save to rewrite `build/scene.grav` in the current
canonical format (v9). This keeps the build output self-contained and stable for external tools
(like the web viewer). If an existing on-disk scene file cannot be decoded (corrupt or unknown
version), the startup rebuild is skipped to avoid overwriting it.

## Code Generation

Rust:

- Generated Rust modules are checked in under [`src/proto_gen/`](/Users/flow/workspace/github/gravimera/src/proto_gen).
- [`src/proto.rs`](/Users/flow/workspace/github/gravimera/src/proto.rs) `include!()`s these modules for use in the runtime.

If you modify `.proto` files, regenerate the checked-in Rust sources and commit the result.

TypeScript:

- Generated TS sources live under `ts/gravimera_proto/src/gen/` and are checked in.
- The compiled JS output under `ts/gravimera_proto/dist/` is also checked in, so consumers do not
  need to run protobuf generation or TypeScript compilation.
- From repo root (no `protoc` required):

```bash
cd ts/gravimera_proto
npm install
npm run build
```

If you modify `.proto` files and need to regenerate the TS sources, run `npm run regen` in
`ts/gravimera_proto` (this requires `protoc` to be installed and available on your `PATH`).
