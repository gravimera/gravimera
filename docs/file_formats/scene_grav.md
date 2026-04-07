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

## Code Generation

Rust:

- `build.rs` runs `prost-build` at compile time and writes generated Rust into `$OUT_DIR`.
- [`src/proto.rs`](/Users/flow/workspace/github/gravimera/src/proto.rs) `include!()`s the generated modules for use in the runtime.

TypeScript:

- TS decoding code is generated with `protoc-gen-es` into `ts/gravimera_proto/src/gen/`.
- From repo root:

```bash
cd ts/gravimera_proto
npm install
npm run gen
npm run check
```

This requires `protoc` (the protobuf compiler) to be installed and available on your `PATH`.

If you need emitted JavaScript output (for bundlers/runtime), run:

```bash
cd ts/gravimera_proto
npm run build
```
