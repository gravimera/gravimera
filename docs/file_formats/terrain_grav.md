# `terrain.grav` (Scene Terrain Selection)

This file stores the active terrain for a scene.

- Path: `<root_dir>/realm/<realm_id>/scenes/<scene_id>/build/terrain.grav`
- Encoding: protobuf (binary)
- Schema: [`proto/gravimera/terrain/v1/terrain.proto`](/Users/flow/workspace/github/gravimera/proto/gravimera/terrain/v1/terrain.proto)

## Schema

`terrain.grav` encodes `gravimera.terrain.v1.SceneTerrainDat`:

- `format_version`:
  - `1`: selection only (terrain id only; no embedded definition)
  - `2`: selection + embedded `TerrainDefV1` (current)
- `terrain_id` (`Uuid128`):
  - Omitted for Default Terrain.
  - Present when a realm terrain package is selected.
- `terrain_def` (`TerrainDefV1`):
  - Embedded terrain definition so a scene build can be rendered without loading the realm terrain package from disk.

`TerrainDefV1` mirrors `terrain_def_v1.json` / `FloorDefV1` (mesh/material/coloring/relief/animation).

## Migration Behavior

The runtime loader is tolerant of old inputs and will upgrade to the current format:

- Legacy `build/floor_selection.json` is migrated to `build/terrain.grav` and removed.
- `format_version = 1` protobuf files (selection-only) are upgraded to `format_version = 2` by embedding the referenced terrain definition (or Default Terrain if no id is selected).

## Code Generation

The `.proto` files under [`proto/`](/Users/flow/workspace/github/gravimera/proto) are treated as the source of truth.

Rust:

- `build.rs` runs `prost-build` at compile time and writes generated Rust into `$OUT_DIR`.
- [`src/proto.rs`](/Users/flow/workspace/github/gravimera/src/proto.rs) `include!()`s the generated modules for use in the runtime.
- The build uses `protoc-bin-vendored`, so a system `protoc` is not required for `cargo build`.

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

Note: `fixed64` fields (like `Uuid128.hi/lo`) are generated as `bigint` in TypeScript because JavaScript numbers cannot represent 64-bit integers safely.
