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
- On rendered startup, the runtime rewrites `build/terrain.grav` in `format_version = 2` embedding the currently active terrain definition so the build output is self-contained for viewers/export.

## Code Generation

The `.proto` files under [`proto/`](/Users/flow/workspace/github/gravimera/proto) are treated as the source of truth.

Rust:

- Generated Rust modules are checked in under [`src/proto_gen/`](/Users/flow/workspace/github/gravimera/src/proto_gen).
- [`src/proto.rs`](/Users/flow/workspace/github/gravimera/src/proto.rs) `include!()`s these modules for use in the runtime.

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

Note: `fixed64` fields (like `Uuid128.hi/lo`) are generated as `bigint` in TypeScript because JavaScript numbers cannot represent 64-bit integers safely.
