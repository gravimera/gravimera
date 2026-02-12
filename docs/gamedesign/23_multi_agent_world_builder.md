# Multi-Agent World Builder (Generic, No Hidden Heuristics)

This document proposes a multi-agent architecture for Gravimera’s core promise: **AI freedom creation** of living worlds (metaverse-like realms) using the agent API.

The engine stays generic: it provides a deterministic simulation, declarative authoring primitives, and validators. “Town knowledge” (or any other domain knowledge) lives in **agent-authored templates and style packs**, not in hard-coded engine heuristics.

## Why Multiple Agents?

Creating a living scene is not one task. It mixes:

- spatial design (terrain, layout graphs, placement constraints),
- asset creation (prefabs, materials, style coherence),
- population and schedules (NPC roles and behavior graphs),
- story logic (quests/dialogue/triggers),
- safety and budgets,
- quality evaluation and iteration.

A single monolithic agent tends to be slow, inconsistent, and hard to debug. A small team of specialized agents produces better results if they share:

- a common contract (schemas),
- a deterministic authoring workflow (validate/apply),
- measurable acceptance criteria.

## “No Heuristics” Interpreted as a System Rule

The goal is not “no algorithms”. The goal is **no hidden, domain-specific assumptions** baked into the engine like “a town should have a market”.

The design rule is:

1) The engine provides **generic operators** (regions, splines, scatters, constraints, compilation).
2) Any “content-specific” logic is expressed as **data** (blueprints/templates/style packs) authored by humans or agents.
3) All randomness is deterministic and parameterized (seeded), never implicit.

This makes the same system usable for *anything*: ancient town, spaceship interior, alien forest, puzzle dungeon, art gallery, etc.

## Shared Artifacts (How Agents Communicate)

Agents should not hand each other prose. They should pass structured artifacts that can be validated and applied.

Core artifacts (conceptual; carried via API payloads or files):

- **WorldSpec**: the user’s intent + constraints (theme, budgets, modules, references).
- **StylePackSpec**: palettes, building kit, prop kit, NPC look kit (coherence).
- **ScenePlan**: procedural layers + constraints (roads/regions/scatters/markers), plus seeds.
- **PrefabPack**: prefab definitions and asset references (including Gen3D job outputs).
- **StoryPack**: quests/dialogue/triggers/actions assets.
- **BrainPack**: behavior graphs, schedule templates, brain configs.
- **Blueprint / BlueprintPatch**: the concrete mutation set to apply (see `docs/gamedesign/19_blueprint_spec.md`).
- **ValidationReport**: metrics and failures (budgets, walkability, repetition, nav cost).

If every agent produces one of these artifacts, the pipeline becomes reliable and testable.

## Recommended Agent Roles

### 1) Manager / Orchestrator Agent

Responsibilities:

- turn user intent into a WorldSpec with explicit constraints and budgets,
- split work into subtasks and assign them,
- enforce schemas, request ids, deterministic seeds, and idempotency,
- merge BlueprintPatches and resolve reference mappings,
- keep an audit-friendly “decision log” (why changes were made).

This is the “task tracker + integrator”.

### 2) Architect Agent (Spatial / Layout)

Responsibilities:

- propose ScenePlan layers using generic primitives:
  - regions (polygons), paths (splines), markers, avoid zones,
  - placement constraints (min distance, align, snap, slope limits),
  - parcel/space subdivision parameters (generic subdivision, not “town-only”),
- ensure the plan is compilable under budgets,
- output a BlueprintPatch that adds/updates scene procedural layers.

### 3) Object Agent (Prefab / Gen3D)

Responsibilities:

- generate or curate prefabs (buildings, props, NPC bodies/clothes) using Gen3D and/or composition,
- produce a coherent PrefabPack compatible with the StylePackSpec,
- expose anchors/attachments for doors, signs, weapons, etc,
- register prefabs via jobs and return stable prefab ids.

### 4) Style Agent (Coherence)

Responsibilities:

- define StylePackSpec from reference images or text constraints,
- constrain the palette and proportions so generated content matches the place,
- avoid “asset soup” by enforcing a limited kit and controlled variation knobs.

This role is what makes “similar buildings and decorations” achievable.

### 5) Dressing Agent (Micro Detail at Scale)

Responsibilities:

- add micro-detail layers (scatter, spline placement, clusters) via procedural layers,
- tune densities and constraints to avoid clutter/emptiness,
- keep everything deterministic and budget-aware.

### 6) Population Agent (NPCs and Schedules)

Responsibilities:

- create NPC identities, assign looks/roles, bind home/work markers,
- attach schedule brains or behavior graphs,
- ensure NPC logic is bounded and debuggable (state inspection available).

### 7) Story Agent

Responsibilities:

- author quests/dialogue assets and story variables initialization,
- bind triggers to stable identities/markers/portals (not fragile instance ids),
- ensure triggers/actions respect budgets and do not create infinite loops.

See `docs/gamedesign/17_story_system_contract.md`.

### 8) Supervisor / QA Agent

Responsibilities:

- run validators and gate changes:
  - budgets, walkability, nav cost,
  - repetition detection,
  - event-rate estimates,
- request rebuild/refinement as patches rather than freeform feedback,
- optionally critique screenshots externally (aesthetic review) but rely on **measurable** engine metrics for gating.

### 9) Safety / Governance Agent (Hosting)

Responsibilities:

- enforce capability policy and role limitations,
- clamp budgets and rate limits,
- quarantine or reject unsafe blueprints/story/brains,
- produce audit events and reports.

This can be implemented as server policy or as a resident moderator agent depending on hosting model.

## The Deterministic Creation Loop (End-to-End)

A stable loop for a multi-agent builder:

1) **Spec**: Manager writes WorldSpec (seed, budgets, enabled modules, style refs).
2) **Plan**: Architect + Style + Object agents propose artifacts.
3) **Validate**: Supervisor runs validation (dry run) and returns a ValidationReport.
4) **Apply**: Manager submits `blueprints:apply` with `request_id` and receives id mappings.
5) **Observe**: snapshot + event cursor recorded; optional fixed camera screenshots.
6) **Refine**: agents produce BlueprintPatches; repeat.

The only “state” in the agents is the artifact history and the event/snapshot cursors. That makes the system restartable.

## Avoiding Drift and “Style Collapse”

Living worlds evolve. Without structure they become incoherent.

Countermeasures:

- lock scenes to a StylePackSpec and require style review for changes,
- treat all large changes as blueprint patches that can be audited and rolled back,
- enforce budgets (too many unique prefabs ruins coherence and performance),
- use identity ids for story-critical NPCs so replacements don’t break narrative.

## Where This Connects to the Existing Design Docs

- API surfaces: `docs/gamedesign/16_agent_api_contract.md`
- Blueprint validate/apply: `docs/gamedesign/19_blueprint_spec.md`
- Scene-scale creation: `docs/gamedesign/22_scene_creation.md`
- Story system: `docs/gamedesign/17_story_system_contract.md`
- Behavior graphs: `docs/gamedesign/18_behavior_graph_spec.md`
- Safety/governance: `docs/gamedesign/11_safety_and_governance.md`

