# Automatic AI Agent Development Loop (Testing, Evaluation, Regression)

This document defines a development and testing loop for Gravimera’s multi-agent “world builder” system.

The goal is to make agent development feel like software development:

- fast inner-loop iteration,
- deterministic repro,
- measurable quality gates,
- regression prevention,
- automation in CI.

This loop applies to:

- agents that **play** (act),
- agents that **author** (create scenes/worlds),
- resident agents that **operate** (live world maintenance),
- embedded brains/story assets that run inside the simulation.

## Non-Negotiable Requirements

1) **Deterministic test mode exists**: the server can run in step/paused mode with fixed `dt` (admin-only by policy).
2) **Artifacts are reproducible**: a test run produces a stable bundle (seed, request_id, blueprint, logs, metrics, snapshots).
3) **Validations are measurable**: tests gate on numeric metrics and schema checks, not subjective judgments.
4) **Every failure is inspectable**: any gate failure includes enough context to reproduce locally.

For the detailed design of evaluation diagnostics and automatic repair, see:

- `docs/gamedesign/25_evaluation_and_auto_repair.md`

## The Three Loops

### Loop A: Inner Loop (seconds to minutes)

Used by an agent developer on every change.

1) Start a headless Gravimera server in deterministic stepping mode with a fresh temp `GRAVIMERA_HOME`.
2) Apply a small blueprint fixture (or load a small realm fixture).
3) Run the agent for a short budget (e.g. 300 ticks).
4) Collect:
   - validation report (budgets, walkability, repetition),
   - event log segment,
   - a small set of snapshots (state + optional screenshots).
5) Assert invariants (“must pass” gates). Fail fast.

Target runtime: < 1 minute.

### Loop B: Integration Loop (minutes to hours)

Used in CI per PR and nightly builds.

- Run a suite of scenarios:
  - multiple realm/scene fixtures,
  - multiple seeds per fixture,
  - multiple agent role combinations (architect + object + population + story + supervisor).
- Gate on:
  - correctness (schema/invariants),
  - determinism (same seed => same signature),
  - performance envelopes (tick time, event rates),
  - budget conformance.

### Loop C: Release Loop (hours)

Used before a version bump or hosted deployment.

- Run all integration scenarios plus:
  - migration fixtures (old realm formats -> current),
  - long soak tests (resident agents running for simulated days),
  - security/policy tests (capability enforcement, budget clamps).

## Test Layers (What to Test)

### 1) Engine Unit Tests (Rust)

These are traditional tests (`cargo test`) and should cover:

- deterministic id generation rules,
- blueprint compilation primitives (regions/splines/scatter constraints),
- story trigger ordering and budget enforcement,
- behavior graph execution semantics (budgets, deterministic RNG),
- persistence encode/decode + migration transforms.

### 2) API Contract Tests (HTTP)

These are “black box” tests that:

- start the server in headless mode,
- call endpoints,
- validate response schemas and error codes.

Key properties:

- error envelopes are consistent,
- capabilities are enforced (missing capability produces the right code),
- idempotency works for request ids,
- snapshot includes `tick` and `event_id` consistency.

### 3) Scenario Tests (End-to-End)

Scenario tests are the primary evaluation method for agent development.

A scenario defines:

- initial realm/scene fixture (or blueprint to create it),
- the agent(s) to run and their roles/capabilities,
- run budget (ticks),
- gates (metrics/invariants).

Scenarios should be small, deterministic, and fast. A few large scenarios can exist for soak/perf, but most should be tiny.

### 4) Determinism / Replay Tests

To prevent subtle nondeterminism regressions:

- Run scenario twice with same seed/request_id.
- Compare a stable **signature**:
  - hash of event kinds + key payload fields,
  - hash of a canonicalized snapshot,
  - budget counters and counts.

Do not compare huge raw logs. Compare compact hashes plus keep the raw logs as artifacts only on failure.

### 5) Performance Envelope Tests

Agents can create enormous worlds. We need guardrails:

- measure average tick time and p95/p99 tick time under a fixed object/NPC count,
- measure event throughput,
- measure brain step budgets usage,
- ensure budget clamps trigger before runaway CPU/memory.

Performance tests should fail with actionable messages (“brain budget exceeded by X%”) rather than only timing out.

## Fixtures and Artifacts

### Fixtures (Inputs)

Fixtures are stored under `tests/` (or a dedicated `tests/agent/` subtree) and include:

- realm fixtures (small, versioned),
- blueprint fixtures (procedural layers + minimal prefab packs),
- story fixtures (quests/dialogue),
- brain fixtures (behavior graphs).

Fixtures must be:

- schema validated,
- version pinned,
- small enough to run quickly.

### Artifacts (Outputs)

Every test run writes a run directory:

- `run.json` (seed, request_id, engine version, agent version)
- `events.jsonl` (bounded window) or `events.sig` (hash)
- `metrics.json` (budgets, walkability, repetition)
- `snapshots/` (canonical state json; optional screenshots)
- `failure_report.md` (only when failing)

Artifacts must be reproducible and sufficient to rerun the failing test locally.

## Quality Gates (Measurable, Generic)

Avoid “town heuristics”. Gates must be domain-agnostic and parameterized per scenario.

Examples of generic gates:

- **Budget gates**:
  - instance counts, prefab counts, portals, active brains
- **Connectivity gates**:
  - reachability between required markers (graph connectivity)
- **Stability gates**:
  - no story/brain errors,
  - bounded event rates,
  - no action failures above threshold
- **Diversity / repetition gates** (optional but generic):
  - “max identical prefab instance ratio” (parameterized)
  - “unique prefab count within expected bounds”

Even “aesthetic quality” can be handled as:

- optional screenshot critique by an external vision model (not a hard gate by default),
- plus hard gates based on budgets and coherence constraints (style pack usage, palette conformance).

## Automated Multi-Agent Cycle (How It Runs)

The recommended fully automatic cycle for a world-building change:

1) Manager agent emits a WorldSpec and a BlueprintPatch.
2) Supervisor agent runs `blueprints:validate` and reads the ValidationReport.
3) If validation fails, Supervisor requests a patch (structured).
4) If validation passes, Manager applies the blueprint.
5) Run a deterministic sim window (ticks) and collect events/metrics.
6) Supervisor checks gates and either:
   - approves and records the artifact signature, or
   - requests refinement (patch + smaller delta).

This is a “closed loop” where iteration happens automatically until gates pass or a budget limit is hit.

## Tooling (What We Should Build in the Repo)

To make this practical, the repo should include a small “agent lab” runner:

- starts a headless server with `GRAVIMERA_HOME` in a temp dir,
- provisions tokens/capabilities for agent roles,
- runs agents as subprocesses/containers,
- captures artifacts and produces a summary report.

This runner is intentionally generic: it does not encode content knowledge, only test orchestration.

## Failure Triage (Developer Experience)

When a test fails, the system should answer:

1) Which gate failed?
2) Which artifact caused it (blueprint patch id / story asset / brain graph)?
3) How to reproduce locally with one command?
4) What was the last known good signature?

This is what makes agent development sustainable as the system grows.
