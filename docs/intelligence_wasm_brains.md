# Intelligence WASM Brains (Guest ABI v1)

Gravimera’s embedded intelligence service can run standalone brains as **WebAssembly (WASM)** modules.

This document describes the **guest ABI v1** and the on-disk **module store** layout.

## Module store layout

Module store root:

```
<root_dir>/intelligence/wasm_modules/
```

Each module is a folder named after its `module_id`:

```
<root_dir>/intelligence/wasm_modules/<module_id>/
  module.json
  brain.wasm              # if source_kind = "wasm_only"
  brain_user.rs           # if source_kind = "rust_source"
  build/brain.wasm        # compiled output cache (rust_source)
  build/brain_user.sha256 # compile cache key (sha256 of brain_user.rs)
```

### `module.json`

Example:

```json
{
  "module_id": "my.brain.v1",
  "abi_version": 1,
  "source_kind": "wasm_only"
}
```

Fields:

- `module_id` (string): must match the folder name.
- `abi_version` (number): currently only `1` is supported.
- `source_kind` (string):
  - `"wasm_only"`: use `brain.wasm`
  - `"rust_source"`: compile `brain_user.rs` into `build/brain.wasm` when the host loads the module

## Runtime restrictions (v1)

Enforced by the service when loading/executing a module:

- The module must have **no imports** (no WASI).
- The module must export:
  - `memory`
  - `brain_alloc_v1`
  - `brain_tick_v1`
- Exported `memory` must declare a **maximum** size, and it must be `<= 256` pages (16 MiB).
- The `brain.wasm` file size must be `<= 10 MiB`.
- Per tick:
  - output buffer cap: `8192` bytes
  - fuel budget: `2_000_000` (execution traps when fuel is exhausted)

The host simulation remains authoritative: outputs are treated as **requests**, and are additionally filtered by host-granted **capabilities**.

## Guest ABI v1

All integers are **little-endian**.

### Exports

The guest module must export:

- `memory`: linear memory
- `brain_alloc_v1(len: u32) -> u32`
  - Returns a guest pointer to a `len`-byte buffer the host can read/write.
- `brain_tick_v1(obs_ptr: u32, obs_len: u32, out_ptr: u32, out_cap: u32) -> u32`
  - Reads `obs_len` bytes at `obs_ptr`.
  - Writes up to `out_cap` bytes at `out_ptr`.
  - Returns the number of bytes written (must be `> 0` on success).

### Observation encoding (`obs_v1`)

The host encodes a bounded snapshot derived from `TickInput`.

Header (88 bytes):

- `dt_ms: u32`
- `tick_index: u64`
- `rng_seed: u64`
- `self_kind_id: u128` (little-endian, sent as two `u64` words)
- `self_tags_bits: u64`
- `self_pos: [f32; 3]`
- `self_yaw: f32`
- `self_vel: [f32; 3]`
- `self_health: i32` (`-1` = None)
- `self_health_max: i32` (`-1` = None)
- `self_stamina: i32` (`-1` = None)
- `nearby_count: u32`

Then `nearby_count` records, each 88 bytes:

- `entity_id: u128` (little-endian, sent as two `u64` words)
- `kind_id: u128`
- `rel_pos: [f32; 3]`
- `rel_vel: [f32; 3]`
- `health: i32` (`-1` = None)
- `health_max: i32` (`-1` = None)
- `radius: f32` (`-1.0` = None)
- `aabb_half_extents_xz: [f32; 2]` (`[-1.0, -1.0]` = None)
- `tags_bits: u64`
- `pad: u32` (reserved)

`entity_id` and `kind_id` are derived from UUID strings when possible; otherwise they are stable hashes.

### Output encoding (`out_v1`)

The guest writes:

- `command_count: u32`
- then `command_count` fixed-size records of 32 bytes each:
  - `kind: u32`
  - `payload: [u8; 28]` (kind-specific)

Command kinds:

- `1` = `MoveTo`
  - payload:
    - `pos: [f32; 3]`
    - `valid_until_tick: u64` (`u64::MAX` = None)
- `2` = `SetMove`
  - payload:
    - `vec2: [f32; 2]`
    - `valid_until_tick: u64` (`u64::MAX` = None)
- `3` = `AttackTarget`
  - payload:
    - `target_id: u128`
    - `valid_until_tick: u64` (`u64::MAX` = None)
- `5` = `SleepForTicks`
  - payload:
    - `ticks: u32`

The host filters commands by capabilities:

- `MoveTo`/`SetMove` require `brain.move`
- `AttackTarget` requires `brain.combat`
- `Say` (not in v1 encoding above) would require `brain.talk`

## Rust compilation (`rust_source`)

For a `rust_source` module, the service shells out to `rustc` to compile `brain_user.rs` into a WASM module.

It looks for `rustc` in:

1. `GRAVIMERA_RUSTC` (recommended when shipping a bundled toolchain)
2. a bundled toolchain (`toolchain/rust/bin/rustc`, auto-detected relative to the running executable, if present)
3. `PATH`

Compilation is cached under:

- `build/brain.wasm`
- `build/brain_user.sha256` (sha256 of `brain_user.rs`)

Compile flags (current v1):

```bash
rustc --edition=2021 --crate-type=cdylib --target wasm32-unknown-unknown \
  -O -C panic=abort -C lto=thin -C strip=symbols \
  -C link-arg=--max-memory=16777216 \
  -o build/brain.wasm brain_user.rs
```

Your `brain_user.rs` must export `memory`, `brain_alloc_v1`, and `brain_tick_v1` as described above.
