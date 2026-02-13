# Evaluation and Auto-Repair (How Agents Detect “Bad” and Fix It)

This document answers the central automation problem: how a coding agent (like Codex) can detect that AI-generated world content is **bad**, identify **what to improve**, and apply **automatic fixes** without a human manually inspecting the world.

The key idea is to treat world generation like software compilation:

- **Specs** define what “correct” means.
- **Validators** produce structured diagnostics (not just pass/fail).
- **Provenance** makes failures attributable to specific generator steps.
- **Repair operators** generate small, testable patches.

This stays generic: the engine does not assume “town rules”. It enforces constraints and budgets; content-specific intent lives in user/agent provided specs.

For structured contracts and learned evaluator integration, see:

- `docs/gamedesign/27_scorecards_and_validation_reports.md`
- `docs/gamedesign/28_evolving_evaluators.md`

## Hard Truth: “Bad” Requires a Spec

There is no universal “good-looking scene”. Automatic evaluation requires at least one of:

1) **Hard constraints**: budgets, reachability, schema validity, determinism, safety policy.
2) **Soft constraints**: distributions and style conformance defined explicitly in the spec (not implicit heuristics).
3) **References**: images/scene exemplars to match (optional, host policy), evaluated externally (vision model).

So the system must require a **ScorecardSpec** (or equivalent) for any automated gate.

## Inputs: The Minimum Specs Needed

### 1) WorldSpec (Intent + Constraints)

The Manager/Orchestrator agent should always produce a WorldSpec containing:

- seed(s) and request_id policy,
- budgets (instances, brains, portals, events/sec),
- enabled modules,
- required markers and connectivity requirements (“these markers must be mutually reachable”),
- optional style constraints (palette, kit usage),
- acceptance gates (what must pass).

### 2) ScorecardSpec (Gates + Weights)

ScorecardSpec is a machine-readable list of:

- metrics to compute,
- thresholds (hard fail) and targets (optimize),
- weights for multi-objective ranking,
- the scope (realm/scene/region).

The scorecard makes evaluation explicit and portable across realms.

## Engine Outputs: Validators Must Return Diagnostics, Not Opinions

Validators must output:

1) **Metrics**: numbers (counts, rates, distributions).
2) **Violations**: structured failures with counterexamples.
3) **Provenance**: where the problematic content came from (which blueprint layer, which compile step).
4) **FixIts** (optional but strongly recommended): suggested repair operator + parameters.

### Provenance Tagging (Source Maps for Worlds)

Every compiled instance should carry lightweight provenance metadata:

- `source.blueprint_id`
- `source.layer_id`
- `source.rule_id` (which operator produced it)
- `source.local_ref` (if derived from a blueprint local reference)
- `source.request_id`

This enables fault localization: “these 3,200 props came from layer `street_dressing_v2`”.

Without provenance, automatic repair is blind.

## What “Bad” Looks Like (Generic Failure Classes)

These classes are domain-agnostic and apply to any scene type.

### A) Structural/Schema Failures

- invalid ids, missing references, cyclic prefab references, invalid transforms.

### B) Budget/Safety Failures

- too many instances/brains/portals,
- event rate too high,
- brain budget exceeded,
- story trigger firing explosion,
- rate limit violations.

### C) Simulation Correctness Failures

- determinism mismatch across identical runs,
- unstable ordering (nondeterministic events),
- physics/nav divergence (if enabled).

### D) Connectivity/Playability Failures

- required markers are not mutually reachable,
- portals create dead ends (no return path) when return path is required,
- blocked spawn points.

### E) Coherence/Consistency Failures (Spec-Defined)

These are only valid if explicitly stated in the ScorecardSpec:

- palette conformance (“>= 80% materials from style palette”),
- kit usage constraints (“>= 70% buildings from kit A, <= 10% from kit B”),
- distribution constraints (density per region, height range).

### F) Aesthetic Failures (Optional, Reference-Based)

This requires screenshots and an external evaluator (vision model or human):

- “looks wrong” relative to a reference image set.

These should usually be advisory, not hard gates, unless a host explicitly opts in.

## Finding Improvement Points: Fault Localization

Once you can detect “bad”, the next problem is: *what should change*?

The system should support three localization techniques:

### 1) Blame by Provenance

Group violations by `source.layer_id` and `source.rule_id`. Example:

- budget exceeded because 85% of instances came from `scatter_props_alley`.

### 2) Counterexample Mining

Violations should include minimal counterexamples:

- reachability failure includes:
  - start marker, end marker,
  - a blocking obstacle set or a “blocked corridor” bounding region,
  - the nav query parameters.

- repetition failure includes:
  - the repeated prefab ids,
  - the region(s) where repetition exceeds threshold.

### 3) Regression Diffing (Before/After)

When a change causes failure, compare:

- metrics delta,
- canonical snapshot diff (counts per provenance group),
- event signature diff.

This points directly to the change that broke the scorecard.

## Auto-Repair: Patch Operators

Auto-repair works by applying small patches (BlueprintPatches) using a catalog of **repair operators**.

Repair operators are generic transformations such as:

- `ReduceDensity(layer_id, factor)`
- `AddAvoidZone(layer_id, polygon)`
- `WidenPathSpline(layer_id, spline_id, delta)`
- `RetargetScatterToKit(layer_id, allowed_prefabs)`
- `IncreaseVariation(archetype_id, knob, delta)`
- `AddConnectivityBridge(marker_a, marker_b, prefab_id)` (if the spec allows adding a bridge object)
- `AddCooldown(trigger_id, secs)` / `AddGuardVar(trigger_id, var_key)`
- `ClampBrainBudget(brain_id, steps_per_tick)` / `DisableBrainOnFailure(brain_id)`

These are not content heuristics; they are **mechanical edits** with explicit parameters.

## The Closed-Loop Repair Algorithm (How Codex Fixes Automatically)

Given a failing run:

1) **Reproduce**: rerun the exact scenario with the same seed/request_id and capture artifacts.
2) **Parse violations**: read ValidationReport and group by failure class + provenance.
3) **Propose patches**:
   - pick the smallest set of repair operators that could fix the failing constraints,
   - generate 1..K candidate BlueprintPatches (K small; e.g. <= 5).
4) **Validate candidates**:
   - run `blueprints:validate` for each patch,
   - reject any that violate budgets/policy.
5) **Apply best candidate**:
   - apply patch idempotently with a new request_id,
   - run a short deterministic sim window,
   - recompute scorecard.
6) **Stop**:
   - success when all hard gates pass,
   - otherwise iterate up to a bounded number of cycles,
   - if still failing, emit a structured “needs human spec change” report.

This is exactly how an automated code fixer works: diagnose → patch → rerun tests.

## Determinism as a First-Class Gate

To keep auto-repair trustworthy:

- every evaluation cycle should include a determinism check on at least one small scenario,
- nondeterminism is treated as a **hard fail** because it breaks repeatability and makes repair unstable.

## When Automatic Fixing Should Refuse

Auto-repair must refuse (and ask for a spec update) when:

- goals conflict (scorecard impossible within budgets),
- required capabilities are missing (cannot mutate scenes),
- the only available fixes are “content choices” not specified by the WorldSpec (e.g. “make it more ancient” with no style pack or reference).

In these cases, the system should return:

- which constraints conflict,
- what extra spec fields would make the repair possible (e.g. “provide a style pack palette” or “allow adding bridges”).

## How This Fits the Multi-Agent Builder

The Supervisor/QA agent is responsible for:

- running validators,
- producing violations with provenance and counterexamples,
- generating FixIts (repair suggestions).

The Manager agent:

- chooses which FixIts to apply,
- merges patches,
- tracks request ids and artifacts,
- decides when to stop or escalate.

See also:

- `docs/gamedesign/24_agent_dev_loop.md` (automation loops)
- `docs/gamedesign/23_multi_agent_world_builder.md` (agent roles)
