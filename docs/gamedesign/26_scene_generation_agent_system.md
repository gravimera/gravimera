# Scene Generation Agent System (Detailed)

This document “zooms in” on scene generation/editing as a multi-agent system. It defines:

- the agent roles and their boundaries,
- the artifacts they exchange,
- the deterministic validate/apply/simulate loop,
- how to diagnose and repair failures automatically,
- how a learned evaluator (“critic”) can improve over time without baking domain heuristics into the engine.

For product goals around logging, durable artifacts, and crash-resume, see:

- `docs/gamedesign/29_observability_and_resumability.md`

This system must remain **generic**: it should work for any scene type (ancient town, spaceship interior, alien forest, art museum, puzzle dungeon) because it depends on *explicit specs and constraints*, not hidden content rules.

## Key Principle: The Engine Is a Compiler, Agents Are Authors

Gravimera should not contain a “town generator” or “spaceship generator”.

Instead:

- the engine provides generic authoring primitives (regions/splines/scatter/constraints), deterministic compilation, and validators;
- agents produce **specs** and **blueprints** that use those primitives.

The scene generation agent system is therefore a “build toolchain”:

- specs and templates are like source code,
- blueprints are like build outputs,
- validators are like compilers/lints,
- the simulation is the runtime test.

## Objects of Work: Tasks, Runs, and Patches

### SceneGenTask

A SceneGenTask is the unit of work presented to the agent team:

- intent: freeform human description (optional)
- constraints: WorldSpec + ScorecardSpec (required)
- inputs: references (images, existing realm) (optional)
- target: create a new scene, or edit an existing scene

### SceneGenRun

A SceneGenRun is one attempt to satisfy a SceneGenTask:

- stable identifiers: `run_id`, `request_id`, seed(s)
- produced artifacts: ScenePlan, Blueprint, Prefab jobs, Story/Brain assets
- outputs: ValidationReport, event signatures, snapshots/screenshots, metrics

Runs must be reproducible. A developer should be able to rerun a failing run from its artifacts alone.

### BlueprintPatch

All edits are expressed as BlueprintPatches:

- small, mechanical changes (reduce density, widen a path, add an avoid zone)
- validated before apply
- auditable and rollback-friendly

This makes agent iteration safe and testable.

## Required Engine Surfaces (So Agents Can Work)

For the agent system to be effective, the engine must expose:

1) **Validate / apply** for blueprints  
   See `docs/spec/19_blueprint_spec.md` and `docs/spec/16_agent_api_contract.md`.

2) **Scene validation** with structured diagnostics  
   - budgets and estimates
   - reachability/connectivity between declared markers
   - repetition/density metrics (only if scorecard asks)
   - report includes counterexamples and provenance

3) **Provenance tagging** on compiled instances  
   So a supervisor can blame failures on a specific layer/rule.

4) **Deterministic stepping** (where policy allows)  
   So evaluation runs are reproducible.

5) **Observability**  
   - snapshots include `tick` and `event_id`
   - event stream is stable and ordered
   - optional fixed camera screenshot capture (for external critics)

These are not “town features”; they are general-purpose tooling to build any scene.

## Artifacts (Structured Contracts)

The scene generator agents should communicate using a small set of versioned artifacts.

### WorldSpec

Contains the “why” and hard constraints:

- seed policy and determinism requirements
- modules enabled (story/brains always for living worlds)
- budgets (instances, brains, portals, events/sec)
- required markers and connectivity requirements
- capability policy (what this run is allowed to do)

WorldSpec must be explicit because evaluation and repair depend on it.

### SceneIntentSpec

Contains the “what” at a high level:

- target scene id (or create new)
- desired scene regions and semantic tags (user-defined)
- required landmarks / anchors as markers (not “market”, but “landmark_1”)
- optional references (image set) and style pack constraints

SceneIntentSpec can be domain-free: tags and region names are defined by the task, not by the engine.

### StylePackSpec

Defines coherence constraints:

- palette constraints and allowed prefab packs
- proportions and motif constraints
- NPC look constraints

Style is where “similar buildings and decorations” comes from, but it is still generic: it constrains assets, not layouts.

### ScenePlan (Procedural Layers)

A ScenePlan is a declarative description of scene structure using generic primitives:

- regions (polygons), paths (splines), markers, avoid zones
- placement constraints and param knobs
- compilation seeds (scene seed + layer seeds)

ScenePlan does not contain “town rules”. It contains explicit structure and constraints.

### Blueprint / BlueprintPatch

Blueprints are the applied mutations:

- register prefabs/packs (from Object agent)
- define/modify procedural layers (from Architect/Dressing)
- spawn/edit instances (from any agent)
- attach brains and story content (Population/Story)

BlueprintPatch is a minimal delta on top of an existing state.

### ScorecardSpec and ValidationReport

These govern evaluation and repair:

- ScorecardSpec defines what must pass and what to optimize.
- ValidationReport returns:
  - metrics
  - violations with counterexamples
  - provenance blame
  - optional FixIts (suggested repairs)

See `docs/spec/27_scorecards_and_validation_reports.md`.

## Agent Roles and Interfaces (Scene-Focused)

The multi-agent builder in `docs/gamedesign/23_multi_agent_world_builder.md` is general. For scene generation, the critical specialization is **artifact responsibility**:

### Manager / Orchestrator

- owns the run directory and artifacts
- assigns subtasks and enforces schema versions
- enforces deterministic seeds and request_id usage
- merges patches and resolves local references

### Architect (Macro + Meso)

- produces ScenePlan layers for:
  - terrain base constraints
  - region partitioning
  - path graphs (splines)
  - parcel/subdivision parameters (generic)

The architect never “spams instances” directly; it defines structure that compiles into instances.

### Dressing (Micro)

- produces procedural scatter layers with explicit constraints:
  - density, min distance, avoid zones
  - kit constraints (which prefab sets allowed)
  - region/path filters

### Object / Gen3D

- produces a constrained prefab catalog compatible with StylePackSpec
- returns stable prefab ids and asset pack ids

### Population

- creates identities, binds them to markers, attaches schedules/brains
- ensures all brains are bounded (budgets) and debuggable

### Story

- authors quests/dialogue assets and trigger bindings to stable identities/markers
- avoids trigger loops via explicit guards/cooldowns and budgets

### Supervisor / QA

- runs validate/simulate
- produces ValidationReport + FixIts
- requests repair patches (structured), not freeform opinions

### Critic (Learned Evaluator, Optional but Powerful)

- consumes artifacts + screenshots + metrics and outputs:
  - quality score / confidence
  - issue labels with evidence
  - ranked candidate repair operators

The critic evolves over time (offline training) without changing the engine.

See `docs/gamedesign/28_evolving_evaluators.md`.

## The Scene Generation Pipeline (Deterministic)

1) **Spec** (Manager):
   - finalize WorldSpec + SceneIntentSpec + ScorecardSpec
   - select seed(s) and request_id strategy

2) **Style + Asset kit** (Style + Object agents):
   - choose or build a StylePackSpec
   - ensure required prefab kits exist (jobs)

3) **Structure plan** (Architect):
   - emit ScenePlan (regions/paths/markers) with explicit constraints

4) **Dressing** (Dressing):
   - add procedural micro-detail layers with budgets

5) **Population + Story**:
   - spawn identity ids, attach schedule brains
   - install quests/dialogue; bind triggers to markers/identities

6) **Validate** (Supervisor):
   - run `blueprints:validate` and scene validators
   - produce ValidationReport + FixIts

7) **Apply** (Manager):
   - apply blueprint
   - record mapping outputs and audit events

8) **Simulate + Evaluate** (Supervisor + Critic):
   - step N ticks deterministically (or run real-time window)
   - compute scorecard metrics and stable signatures
   - optionally capture fixed screenshots for external critic

9) **Repair loop**:
   - if hard gates fail: apply FixIts (or critic-suggested) patches and repeat
   - stop when hard gates pass and soft score is acceptable

This is a generic build/test loop. “Town” is just one possible SceneIntentSpec + StylePackSpec + templates.

## Where Automatic Improvement Comes From (Without Engine Heuristics)

There are two levers for improvement:

1) **Better specs and templates** (content-side)
2) **Better evaluation and repair policies** (agent-side)

The engine only needs to:

- provide deterministic compilation,
- provide structured diagnostics,
- allow safe patching.

All quality evolution can happen in:

- the critic model (learned evaluator),
- the supervisor’s repair policy,
- the template library.

## Failure Modes and How the System Self-Debugs

When a scene is “bad”, the system should answer:

- which gate failed (budget, connectivity, determinism, brain errors),
- where it came from (layer/rule provenance),
- what minimal patch could fix it (FixIts),
- what to rerun to confirm.

If a failure cannot be repaired without changing intent (ScorecardSpec), the supervisor must return a structured “spec insufficient/conflicting” report.
