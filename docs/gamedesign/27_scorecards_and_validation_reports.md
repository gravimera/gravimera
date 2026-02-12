# Scorecards and Validation Reports (Contracts for Evaluation)

This document defines the **structured evaluation contract** that enables automatic diagnosis and auto-repair.

The core design is:

- ScorecardSpec tells the system what “good” means for this task.
- Validators produce a ValidationReport with:
  - metrics
  - violations (with counterexamples)
  - provenance blame
  - suggested FixIts (repair patches)

This is generic: it encodes no “town rules”. All content-specific evaluation must be explicitly specified by the scorecard (or reference-based external critics).

## ScorecardSpec

ScorecardSpec is a versioned artifact associated with a SceneGenRun.

### Required Fields

- `format_version` (u32)
- `scope`:
  - `realm_id`
  - `scene_id` (optional; if absent, realm-level)
  - `region_filter` (optional; evaluate only inside region tags or polygons)
- `hard_gates`: list of gates that must pass
- `soft_metrics`: list of metrics to optimize (optional)
- `weights`: weights for soft metrics (optional)

### Gate Types (Generic)

Hard gates should be drawn from generic families:

- **SchemaGate**: “all artifacts validate; no missing refs”
- **BudgetGate**: instance/prefab/portal/brain/event-rate caps
- **DeterminismGate**: repeated run signature must match
- **ConnectivityGate**: reachability between required markers/regions
- **StabilityGate**: no brain/story errors; bounded action failure rates

Soft metrics (optional) can include:

- density distribution targets (parameterized)
- repetition/duplicate ratio targets (parameterized)
- style conformance (only if StylePackSpec provided)

### Example (Illustrative)

    {
      "format_version": 1,
      "scope": { "scene_id": "hub" },
      "hard_gates": [
        { "kind": "budget", "max_instances": 40000, "max_active_brains": 5000 },
        { "kind": "connectivity", "required_marker_pairs": [["spawn","portal_out"]] },
        { "kind": "stability", "max_brain_errors": 0, "max_story_errors": 0 }
      ],
      "soft_metrics": [
        { "kind": "repetition_ratio", "target": 0.15 }
      ],
      "weights": { "repetition_ratio": 1.0 }
    }

## ValidationReport

ValidationReport is the output of `scenes:validate` and/or `blueprints:validate` plus runtime checks (after simulation).

### Required Fields

- `format_version` (u32)
- `tick` and `event_id` (for consistency with snapshots)
- `metrics`: map from metric name -> numeric or structured values
- `violations`: list of violations
- `provenance_summary`: optional aggregation by layer/rule
- `fixits`: optional list of suggested repairs

## Violations (Structured Diagnostics)

A violation must include:

- `code`: stable machine code (e.g. `budget_exceeded`, `marker_unreachable`, `brain_error`)
- `message`: human-readable summary
- `severity`: `error` (hard gate fail) or `warning`
- `scope`: scene/region/object references
- `counterexample`: minimal data that proves the failure
- `blame`: provenance information (layer_id/rule_id) if applicable

### Counterexample Examples

Connectivity violation:

- start marker id
- end marker id
- evidence:
  - a blocked corridor polygon, or
  - nav query parameters + “no path” proof

Budget violation:

- which budget exceeded
- measured count
- top contributing provenance groups (layer_id counts)

Brain failure:

- brain id / unit identity id
- node path and last error reason

## FixIts (Repair Suggestions)

FixIts are machine-actionable repair suggestions that the Manager can apply as BlueprintPatches.

### FixIt Fields

- `fixit_id` (string)
- `reason`: which violation(s) it targets
- `operator`: a repair operator name (from a controlled catalog)
- `parameters`: operator parameters (layer ids, factors, polygons, etc)
- `expected_effect`: what metric should change and in which direction
- `risk`: optional; whether it may break other gates

Example FixIt:

- operator: `ReduceDensity`
- parameters: `{ "layer_id": "street_props", "factor": 0.7 }`
- expected_effect: instances decrease, budget satisfied

Repair operators should be generic mechanical edits (no “make it more ancient”).

## Provenance Summary (Blame)

If provenance tags are available, reports should include:

- counts of instances per layer_id/rule_id
- counts of violations attributed per layer_id/rule_id

This enables automatic fault localization and helps the repair policy choose the smallest patch.

## Stable Signatures for Regression and Determinism

To compare runs, produce compact signatures:

- `events_sig`: hash of canonicalized event kinds + key payload fields
- `snapshot_sig`: hash of canonicalized state summary (counts, ids, provenance aggregates)
- `metrics_sig`: hash of scorecard metrics

The report should include these signatures so CI can detect regressions without storing huge logs.

