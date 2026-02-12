# Evolving Evaluators (Learned Critics for Scene Quality)

This document describes how Gravimera’s agent ecosystem can use *learning* to improve scene evaluation and repair over time, while keeping:

- the engine generic (no domain heuristics),
- tests reproducible (no online weight updates mid-run),
- hosting safe (budgets and capabilities enforced).

## What the Critic Does

A learned critic consumes run artifacts and produces:

- `quality_score` (continuous)
- `pass_probability` for soft quality (separate from hard gates)
- `issue_labels` with evidence pointers (screenshots ids, metrics deltas, provenance groups)
- `repair_rankings`: suggested repair operators / patches with expected improvement

The critic never overrides hard gates; hard gates remain static rules.

## How the Critic Learns (Signals)

Critics can evolve using three types of training signals:

### 1) Human Preferences (Strong Signal)

Collect pairwise comparisons:

- given two runs A and B (same task/spec), which is better?
- optionally: why (label tags)

This is efficient and robust: humans are better at relative judgments than absolute scores.

### 2) Proxy Metrics (Weak but Cheap)

Use measurable proxies already in the system:

- walkability/connectivity success rate
- NPC stuck rate and recovery rate
- story progression success rate (“quest completed within T ticks”)
- event storm detection (events/sec threshold)
- style pack conformance (if explicitly specified)

These proxies do not define “beauty” but they define “functional quality”.

### 3) Repair Outcome Data (Self-Supervision)

When the system applies a patch:

- record `before_score`, `after_score`, and which operator was used.

Over time, the critic learns which repairs tend to improve which failure patterns.

## Training Lifecycle (Keep Tests Reproducible)

To avoid nondeterminism:

- training happens offline (outside the running realm)
- each critic model has a `critic_version` id
- evaluation runs record which critic_version was used
- CI uses pinned critic versions per test suite

Promotion policy:

- train candidate critic on new data
- evaluate on a frozen holdout scenario suite
- only promote if it improves metrics without increasing false passes

## Avoiding Reward Hacking

If agents learn to “game the critic”, quality degrades.

Mitigations:

- critics must consume both:
  - **metrics** (hard to fake without breaking gates), and
  - **artifacts** (provenance summaries), and optionally
  - **screenshots** (harder to spoof)
- maintain adversarial tests:
  - detect “empty scene that passes metrics” by requiring minimum content constraints in ScorecardSpec
- keep some gates unweighted and non-negotiable (budgets, determinism, safety)

## Critic Interface (Contract)

The critic should accept a standardized input bundle:

- WorldSpec + SceneIntentSpec + ScorecardSpec
- ValidationReport(s)
- stable signatures (events_sig, snapshot_sig)
- optional screenshots (fixed camera set)

And output:

- score and confidence
- issues with evidence pointers
- ranked repair operators with parameter suggestions

The output must be machine-actionable so a Supervisor can convert it into BlueprintPatches.

## Using the Critic in the Repair Loop

When hard gates fail:

- use deterministic FixIts first (mechanical repairs)

When hard gates pass but quality is “meh”:

- use critic suggestions to apply small improvements
- keep the patch budget small per iteration
- rerun scorecard and keep only improvements (hill-climb with rollback)

This creates an “autotuning” loop that improves scenes gradually without injecting domain heuristics into the engine.

## Where This Fits

- agent dev loop: `docs/gamedesign/24_agent_dev_loop.md`
- evaluation/repair: `docs/gamedesign/25_evaluation_and_auto_repair.md`
- scene gen system: `docs/gamedesign/26_scene_generation_agent_system.md`

