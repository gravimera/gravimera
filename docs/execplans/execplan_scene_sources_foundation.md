# ExecPlan: Scene Sources Foundation (Text, Canonical, Mergeable)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this milestone, Gravimera has a concrete, git-friendly **scene source format** that is suitable for process management (diffs, code review, branching, and merging). The engine does not need to *use* these sources yet; this milestone is about defining the on-disk layout and providing a Rust implementation that can read, write, and canonicalize it deterministically.

This makes scene generation debuggable and multi-agent friendly because it establishes a textual source of truth. Binary formats like today’s `scene.dat` remain valuable as runtime caches, but they are not workable as “scene repos”.

Verification is via `cargo test`: canonicalization is stable, idempotent, and does not destroy unknown fields.

## Progress

- [x] (2026-02-13) Create the initial ExecPlan.
- [ ] (2026-02-13) Define the `scenes/<scene_id>/src/` file set and the minimum required fields for v1 sources.
- [ ] (2026-02-13) Implement Rust `SceneSources` types + directory read/write helpers.
- [ ] (2026-02-13) Implement canonicalization (stable ordering + formatting) and unit tests for idempotence.
- [ ] (2026-02-13) Add a minimal fixture under `tests/scene_generation/fixtures/minimal/src/` and a test that canonicalizes it without changes.
- [ ] (2026-02-13) Run `cargo test` + headless smoke boot and commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Use split, canonical JSON files as authoritative scene sources under `scenes/<scene_id>/src/`.
  Rationale: Git needs mergeable text sources; splitting reduces conflicts between parallel agents.
  Date/Author: 2026-02-13 / Codex

- Decision: Preserve unknown fields in sources using an explicit “extras” map in Rust structs.
  Rationale: Agents and future engine versions may add fields; canonicalization and read/write must not silently drop data.
  Date/Author: 2026-02-13 / Codex

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

Current persistence is centered on `scene.dat`:

- `src/scene_store.rs` serializes/deserializes `scene.dat` as protobuf via `prost`.

Target spec for scene sources vs build artifacts:

- `docs/gamedesign/30_scene_sources_and_build_artifacts.md` defines the intended split: `src/` authoritative, `build/` caches.

Repository constraints:

- Tests and fixtures must live under `tests/` (see repo `AGENTS.md`).
- Smoke test after changes must ensure the game starts in headless mode without crashing.

## Plan of Work

Define the v1 scene source layout exactly as described in the spec, but keep the initial required content minimal. The goal is to establish the directory structure and canonical JSON rules without blocking on scene generation algorithms.

Implement a new Rust module that provides three core capabilities:

1) Read a `src/` directory into a `SceneSources` in-memory representation.
2) Write a `SceneSources` back to disk using canonical ordering and stable formatting.
3) Canonicalize an existing `src/` directory in place (read → write) without semantic changes.

The canonicalization rules must be strict enough that two agents making the same semantic change produce identical bytes after canonicalization. At minimum this requires stable sorting of lists by stable ids and stable formatting of floats/ints.

## Concrete Steps

Run from the repo root:

1) Unit tests:

   cargo test

2) Headless smoke boot:

   cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

This milestone is accepted when:

- A minimal fixture exists at `tests/scene_generation/fixtures/minimal/src/` following the v1 layout.
- `cargo test` includes unit tests that assert:
  - canonicalization is idempotent (second run produces byte-identical files),
  - unknown fields survive a read→write round-trip,
  - list ordering is stable (sorted by ids, not insertion order).
- The headless smoke boot command exits successfully.

## Idempotence and Recovery

- The read/write/canonicalize operations must be safe to repeat.
- If the format changes, bump a `format_version` field in each source file and include a migration path in a later milestone.

## Interfaces and Dependencies

Use existing dependencies only:

- `serde` / `serde_json` for types and JSON IO.
- `sha2` if a test needs a directory signature helper.

Avoid adding new crates in this milestone; keep the change low-risk and test-first.
