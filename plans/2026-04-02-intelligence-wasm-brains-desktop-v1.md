# Desktop Rust→WASM brains in the Intelligence Service (obs-only, self-contained compilation)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` in the repository root.

## Purpose / Big Picture

After this change, Gravimera’s intelligence service can run “standalone brains” implemented as Rust code that is compiled to a `.wasm` module and executed in a WASM runtime. A player (desktop build) can generate or author a brain in Rust, compile it locally using a Rust toolchain that is bundled with the game distribution, and then select that brain module in the Meta panel like any other module id.

This plan intentionally starts with an **obs-only** model: a brain receives a bounded “observation” snapshot from the host and returns a bounded set of requested commands. The brain does not call host query functions (no host imports besides WASM’s own `memory`/exports). This keeps the first version deterministic, easy to sandbox, and fast in the per-tick hot path.

How a human verifies it works (desktop):

- Start the game with the embedded intelligence service enabled and observe `GET /v1/modules` now includes on-disk WASM modules in addition to the built-in demo modules.
- Create a new Rust brain module (either by writing the user-code file manually or via the generation endpoint in a later milestone), run `POST /v1/load_module`, and observe a `.wasm` artifact is produced in the module’s cache directory.
- Spawn a brain instance for a unit, tick it, and observe the unit performs the requested actions in Play mode without crashing the game.
- Confirm the required rendered smoke test still passes for the game.

## Progress

- [x] (2026-04-02 18:10 CST) Draft this ExecPlan and align it with the current intelligence service HTTP/JSON protocol and host plugin flow.
- [x] (2026-04-02) Implement an on-disk WASM module registry under `<root_dir>/intelligence/wasm_modules/` and expose it via `GET /v1/modules`.
- [x] (2026-04-02) Implement Rust→WASM compilation for `rust_source` modules via `rustc` (toolchain lookup via `GRAVIMERA_RUSTC`, bundled `toolchain/rust/`, or `PATH`).
- [x] (2026-04-02) Add a Wasmtime-backed WASM runtime path in the embedded intelligence service (per-brain instance), with import bans + fuel + memory caps.
- [x] (2026-04-02) Define and document a minimal obs-only guest ABI v1 + binary encoding, plus a small demo guest module used in tests.
- [x] (2026-04-02) Add unit tests that load a WASM module, spawn an instance, tick it, and verify output decoding + runaway handling (out-of-fuel trap).
- [x] (2026-04-03) Sync built-in demo modules from `assets/intelligence/wasm_modules/` into the module store (replacing existing folders with the same `module_id`), and prefer on-disk modules over hard-coded demos.
- [x] (2026-04-03) Host: submit `tick_many` on a worker thread and apply outputs on the main thread to avoid blocking render frames.
- [x] (2026-04-03) Host: avoid A* pathfinding on the render thread for brain-issued `MoveTo` (apply as straight-line target); keep A* for player-issued move orders.
- [x] (2026-04-03) Host: auto-attach default demo brains to eligible units in Play mode (and add a per-unit fallback marker to prevent re-attaching when the player selects fallback in the Meta panel).
- [x] (2026-04-03) Fix demo `rust_source` module parsing bug (`demo.coward.v1`) and rebuild its cached `build/` artifacts.
- [ ] Add end-to-end validation: compile, load, tick, and observe in-game behavior.
- [x] (2026-04-03) Bundle toolchain/runtime into desktop distribution artifacts and document it (`tools/publish.py`, `docs/publishing.md`).

## Surprises & Discoveries

- Observation: The “embedded” intelligence service is still exercised over HTTP/JSON on a loopback TCP socket (the host uses `IntelligenceServiceClient` even for embedded mode).
  Evidence: `src/intelligence/host_plugin.rs` constructs an `IntelligenceServiceClient` from `EmbeddedIntelligenceService::listen_addr()` and uses `POST /v1/tick_many`.

## Decision Log

- Decision: Start with obs-only WASM brains (no host imports / query API).
  Rationale: It keeps the first milestone small, makes the sandbox boundary obvious (inputs only), and avoids designing a capability system twice (host queries plus host command validation). We can add query imports later after we have a stable execution + limits foundation.
  Date/Author: 2026-04-02 / Codex

- Decision: Desktop-only “self-contained local compilation” is required, but we will compile in a separate OS process (a helper binary) rather than inside the simulation thread.
  Rationale: Rust compilation is CPU-heavy and can stall the game if done in-process. A helper process is easier to timeout/kill and can share the same bundled toolchain. The embedded service can spawn it.
  Date/Author: 2026-04-02 / Codex

- Decision: Brain-issued movement must not trigger heavy pathfinding on the render thread in v1.
  Rationale: Brains can tick frequently (up to 60 Hz target) and would otherwise cause periodic hitches if `MoveTo` recomputes A* paths each tick. In v1, `MoveTo` from brains is applied as a straight-line target. Player-issued move orders continue to use A* pathfinding.
  Date/Author: 2026-04-03 / Codex

- Decision: Use a simple, stable `extern "C"` ABI for the guest module rather than the component model/WIT for v1.
  Rationale: The component model would improve type ergonomics, but it adds tooling complexity and pushes us toward larger toolchains and additional host libraries. A minimal pointer+len ABI is sufficient for obs-only and can be validated strictly.
  Date/Author: 2026-04-02 / Codex

## Outcomes & Retrospective

- (2026-04-03) Gravimera can now run standalone brains implemented as WASM modules under strict limits (Wasmtime + fuel + memory caps, obs-only/no-imports ABI). Desktop packaging can bundle a Rust toolchain so `rust_source` modules compile locally without external installs, and built-in demo brains are shipped as on-disk modules synced from `assets/`.

## Context and Orientation

In this repository, “standalone brains” are currently implemented as Rust code that runs inside the intelligence service process, selected by `module_id` strings such as `demo.orbit.v1`. The service is exposed over a small HTTP/JSON API and is used by the game host via `IntelligenceServiceClient`.

Key files:

- `src/intelligence/protocol.rs`: HTTP/JSON protocol structs, including `TickInput` and `TickOutput`.
- `src/intelligence/service.rs`: tiny_http server implementation plus the current built-in demo brains.
- `src/intelligence/host_plugin.rs`: the host-side integration that connects, loads modules, spawns brain instances, and sends batched `tick_many`.
- `docs/intelligence_service.md`: current usage docs for the embedded mode.
- `docs/gamedesign/38_intelligence_service_spec.md`: product intent and safety/perf constraints.

Terms used in this plan:

- “Observation” (obs): the data the host provides to a brain each tick. In v1 this is derived from `TickInput` but re-encoded into an obs-only guest ABI (see `Interfaces and Dependencies`).
- “Brain module”: a loadable unit of code identified by `module_id`. In this plan, a module can be built-in demo code or an on-disk `.wasm` artifact plus metadata.
- “Brain instance”: per-unit instance state created by `POST /v1/spawn`. For WASM modules, this will correspond to a WASM instance so the brain can keep state across ticks.
- “Bundled toolchain”: a Rust compiler + sysroot shipped inside the desktop distribution so compilation works on a player machine without a separate Rust installation.

## Plan of Work

We will extend the intelligence service to support an additional module backend: “Rust→WASM brains”. The core change is that `src/intelligence/service.rs` will be refactored so “module_id → module implementation” is not hardcoded to demo modules. Instead, the service will:

- load built-in demo modules as it does today,
- scan a module directory under `GRAVIMERA_HOME` for WASM brain modules,
- compile Rust sources to `.wasm` when needed (desktop-only, local) using a bundled toolchain, and
- execute brain ticks by instantiating and calling a WASM module function under strict limits.

This plan is structured as milestones so we can land a usable system early and then expand capabilities safely.

### Milestone 1: On-disk WASM module registry (no compilation yet)

Add a module store rooted at:

    <root_dir>/intelligence/wasm_modules/

Where `<root_dir>` is the runtime root directory (default `~/.gravimera`, overridable via `GRAVIMERA_HOME`).

Each module lives in:

    <root_dir>/intelligence/wasm_modules/<module_id>/

And contains:

- `module.json` with:
  - `module_id` (string),
  - `abi_version` (u32, start at 1),
  - `source_kind` (enum: `wasm_only` or `rust_source`),
  - optional provenance fields (created_at, prompt_hash, etc).
- `brain.wasm` if `source_kind = wasm_only`.
- `brain_user.rs` if `source_kind = rust_source`.
- `build/brain.wasm` (compiled output cache).

Service changes:

- `GET /v1/modules` should return:
  - the existing demo module ids, plus
  - every `<module_id>` found under the module store that has a usable `brain.wasm` (or a compilable source in later milestones).
- `POST /v1/load_module` should accept on-disk module ids and mark them as “loaded”, returning `ok: true` if the module exists and passes basic validation.

Basic validation in this milestone:

- module id is valid (non-empty, ASCII subset, no `..` path traversal),
- module metadata file parses,
- `brain.wasm` exists for `wasm_only`,
- `brain.wasm` exports the expected ABI exports (see `Interfaces and Dependencies`).

### Milestone 2: WASM runtime execution path (obs-only)

Add a WASM runtime to the intelligence service and implement a new `BrainModuleState` variant for WASM brains.

Execution model:

- Load: parse/compile the WASM module once per module id (module-level cache).
- Spawn: instantiate a fresh WASM instance per brain instance so per-unit state persists.
- Tick: encode host `TickInput` into the obs-only binary format, write it into guest memory, call `brain_tick_v1`, decode the resulting command bytes into a `TickOutput`, and return it.

Limits (enforced by the runtime in v1):

- hard cap on guest linear memory (example: 16–64 MiB, configurable),
- per-tick CPU limit using runtime instruction metering / interruption (fuel or epoch),
- hard cap on output commands (the guest ABI will have a fixed max; host still clamps `TickOutput` as today).

Behavioral acceptance:

- A hand-authored example WASM brain module can be selected in the UI and will move a unit in Play mode.
- A runaway guest (infinite loop) returns a tick error instead of hanging the service.

### Milestone 3: Self-contained local Rust→WASM compilation (desktop)

Implement compilation for modules with `source_kind = rust_source`.

Goals:

- Player machine does not need Rust installed.
- No network access is required for compilation.
- Compilation is performed in a child process with a timeout and bounded disk usage.

Toolchain bundling:

- Add a “toolchain bundle” directory to the desktop distribution that contains a relocatable Rust sysroot for the host platform and includes the `wasm32-unknown-unknown` standard library.
- Add a small runtime helper (`ToolchainLocator`) that finds the bundled toolchain directory relative to the running executable (and allows an env override for development/testing).

Compilation helper binary:

- Add `src/bin/gravimera_wasm_brain_compiler.rs` that:
  - reads `brain_user.rs` and a fixed template from disk,
  - generates a crate root (single `lib.rs`) in a temp work directory,
  - invokes the bundled `rustc` to build `cdylib` for `wasm32-unknown-unknown`,
  - validates the `.wasm` ABI exports/imports,
  - writes output to the module’s `build/brain.wasm`.

Service integration:

- `POST /v1/load_module` for a `rust_source` module:
  - recompiles if the source hash changed or output is missing,
  - returns a compile error as `HTTP 400` with an actionable message (first N lines of `rustc` stderr + where the files are stored under `<root_dir>`).

Security constraints in v1 (desktop/trusted, but still avoid footguns):

- The template must not allow adding external dependencies. Compilation must not invoke Cargo.
- The compiler process runs with:
  - a clean temp directory as `cwd`,
  - a controlled `RUSTC`/`SYSROOT` (bundled),
  - a timeout (example: 60–180 seconds).
- Source is scanned and rejected if it uses compile-time file inclusion macros (`include_str!`, `include_bytes!`) or `extern` imports. This is a best-effort v1 guardrail to prevent “read arbitrary files at compile time”.

### Milestone 4: (Optional in v1) Natural-language generation endpoint

Add a new HTTP endpoint to the intelligence service:

    POST /v1/generate_module

That takes:

- a natural-language description (“make a coward that flees and rests…”),
- optional module name hint,
- optional capabilities list (still obs-only; capabilities only gate output commands).

And returns:

- `module_id`,
- the generated `brain_user.rs` saved under the module store,
- compilation outcome (success + wasm hash, or compile error with diagnostics).

Implementation notes:

- Reuse the existing OpenAI-compatible request machinery already in the repo (`src/scene_build_ai.rs` / `src/openai_shared.rs`) but expose it through a small, intelligence-specific wrapper so future “mobile compile on server” can reuse the same prompt/tool contracts.
- Keep the prompt small and stable by constraining what the model must output: only the `State` struct and the `tick()` function body, inserted into our fixed template.

This milestone is optional for v1 if we want to land compilation+execution first and add generation later.

### Milestone 5: Distribution packaging (desktop)

Update desktop packaging so release artifacts include:

- the game binary,
- the bundled Rust toolchain directory needed for compilation.

Update docs:

- `docs/intelligence_service.md`: document the WASM brain module directory, and how to add a module by dropping a folder under `<root_dir>/intelligence/wasm_modules/`.
- Add a new doc `docs/intelligence_wasm_brains.md` describing:
  - guest ABI v1,
  - module store layout,
  - how to debug compile failures (where logs/artifacts live),
  - the security model (trusted on desktop, obs-only, limits).

## Interfaces and Dependencies

### Guest ABI: `brain_abi_v1` (obs-only)

Guest module requirements (exports):

- `memory`: exported linear memory.
- `brain_alloc_v1(len: u32) -> u32`: returns a guest pointer where the host can write `len` bytes.
- `brain_tick_v1(obs_ptr: u32, obs_len: u32, out_ptr: u32, out_cap: u32) -> u32`: reads an observation byte buffer and writes command bytes into `out_ptr[..out_cap]`. Returns the number of bytes written, or `0` on error (error string retrieval is a v2 feature; in v1 we return a generic error).

Guest module restrictions (validated by host):

- no imports (obs-only; no WASI),
- no additional exports beyond the expected ones (or allow extra exports but ignore them; decide and document in `Decision Log` when implementing),
- maximum linear memory pages (host-enforced).

Observation encoding v1 (binary, little-endian):

The observation and output formats must be deterministic, bounded, and cheap to parse.

Canonical v1 spec + examples:

- `docs/intelligence_wasm_brains.md` (obs_v1 + out_v1)

Reference implementation:

- `src/intelligence/wasm_brains.rs` (`encode_obs_v1` + `decode_out_v1`)

### WASM runtime dependency

For desktop v1, prefer a runtime that supports:

- per-instance memory access (read/write),
- CPU metering / timeout interruption,
- memory limits.

The concrete library choice (e.g., Wasmtime vs Wasmi) must be recorded in `Decision Log` during implementation. The plan assumes we can cap per-tick CPU and memory regardless of which runtime we pick.

### Compilation dependency

The compiler helper must use the bundled Rust toolchain. It must not rely on `rustup` being installed on the player machine.

In v1 we compile using `rustc` directly:

    rustc --edition=2021 --crate-type=cdylib --target wasm32-unknown-unknown -O -C panic=abort -C lto=thin -C strip=symbols -o build/brain.wasm brain_user.rs

Exact flags (especially LTO vs compile time) can be tuned, but must be fixed and hashed into the build cache key.

## Concrete Steps

All commands are run from the repository root unless noted.

Local developer workflow (after implementation):

1) Create a module folder (example):

    mkdir -p test/run_1/intelligence/wasm_modules/demo.wasm_move_to.v1
    # Write module.json + brain.wasm (wasm_only) or module.json + brain_user.rs (rust_source)

2) Point `GRAVIMERA_HOME` to the test folder and run the game (with embedded intelligence service enabled):

    tmpdir=$(pwd)/test/run_1/home
    mkdir -p "$tmpdir"
    GRAVIMERA_HOME="$tmpdir" cargo run

Expected outcomes (after implementation):

- `GET /v1/modules` includes `demo.wasm_move_to.v1`.
- Selecting the module id in the Meta panel and switching to Play causes the unit to move according to the WASM brain.

## Validation and Acceptance

Acceptance is met when all are true on a desktop dev machine:

- A WASM module placed under `<root_dir>/intelligence/wasm_modules/<module_id>/brain.wasm` is discoverable via `GET /v1/modules`.
- `POST /v1/load_module` rejects invalid WASM modules with a clear error (missing exports, has imports, etc).
- Spawning and ticking a WASM brain instance returns a valid `TickOutput` and results in visible in-game behavior (movement at minimum).
- A runaway/infinite-loop WASM brain produces an error in `tick_many` without hanging the service.
- The required rendered smoke test passes:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Idempotence and Recovery

- Module scanning and `GET /v1/modules` are idempotent.
- Compilation is cached by a content hash (template version + user source + rustc version + flags). Re-running `load_module` for unchanged source must not recompile.
- If a compilation fails, the service must keep the previous successful `build/brain.wasm` (if present) unless the user explicitly requests a clean rebuild. A failed compile should not brick an existing working module.
- If the runtime rejects a module at load time (ABI mismatch), recovery is: fix `brain_user.rs` (or replace `brain.wasm`) and re-run `load_module`.

## Artifacts and Notes

During implementation, keep debug artifacts under the repo’s existing test folder convention:

- `test/run_1/intelligence/wasm_modules/<module_id>/` for module sources and builds used by tests.
- `test/run_1/logs/` for captured compiler stderr/stdout in automated tests.

When bundling for release, toolchain artifacts must not be checked into git; they are staged into `dist/` during packaging.
