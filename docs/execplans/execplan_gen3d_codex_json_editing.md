# Gen3D: Codex-like JSON Editing (Resumable Sessions + Prefab Edit/Fork + Agent Tools)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, Gen3D feels like “Codex editing a JSON codebase” instead of a one-shot generator.

Player-visible outcome:

- In the Meta panel for **Gen3D-saved** prefabs, the player has three actions:
  - **Copy**: spawn/duplicate another instance referencing the same prefab id (no Gen3D).
  - **Edit**: open Gen3D seeded from this prefab; Save overwrites the same prefab id (all instances update).
  - **Fork**: open Gen3D seeded from this prefab; Save writes a new prefab id and rebinds only the selected instance.
- Gen3D “Stop” cancels in-flight work but keeps the session context, so the player (or an agent) can **Continue** generation later instead of restarting from scratch.
- The Gen3D agent can inspect state/history (artifacts) and apply deterministic patches via tools, iterating like a coding agent.

Engineering constraints (must hold throughout):

1) **No heuristic engine algorithms.** If information is missing or ambiguous, prefer hard errors and explicit tool/schema inputs rather than “guess intent” fallbacks.
2) **Agent-driven orchestration.** The LLM chooses tools; the engine executes tools deterministically and records observable artifacts.

Context window / token budget policy (must hold throughout):

- Each agent step/tool call should be solvable from a compact, bounded “working set” prompt (state summary + last error + recent tool results).
- Large history must be pulled on demand from artifacts via bounded read/search tools; do not embed long logs/JSONs in every prompt.
- Prefer diffs and transaction logs over resending full plan/draft JSON repeatedly.
- Cap images per request and prefer low-res renders during iteration; send original photos only when needed.
- Avoid treating provider-side conversation continuation (`previous_response_id` / chat history) as the primary memory mechanism; prefer explicit, inspectable memory in run artifacts.

Pre-implementation contracts (must be written down before coding):

- Define a versioned, machine-readable run/session status contract (QA/review/motion/task flags) that is surfaced to both agent and UI.
- Define strict `done` vs QA policy: engine respects `done` by default (except empty draft), while making incomplete QA visible and easy to query.
- Define Stop/Continue semantics precisely (what is cancelled vs persisted; run_id behavior).
- Decide how Edit/Fork seeds data deterministically:
  - preferred: persist a Gen3D “source bundle” alongside saved prefabs (source vs compiled),
  - fallback: deterministic reconstruction from prefab defs + provenance (lossy but non-heuristic).
- Define artifact tools security + bounds (run-dir scoping, max bytes/lines, stable artifact refs).
- Define deterministic async/parallel semantics (declared apply order + transaction log).

## Progress

- [x] (2026-03-07) Record roadmap decisions and tool ideas in `docs/gen3d/next_actions.md`.
- [x] (2026-03-07) Milestone A (Foundation): add `qa_v1` and run-artifact read tools (list/read/search) for the current run dir.
- [x] (2026-03-07) Milestone B (“Stop means stop”): accept agent `done` by default (keep only empty-draft guard) and surface QA/review state as machine-observable status.
- [x] (2026-03-07) Milestone C (Resumable sessions): add “Continue” for a stopped/cancelled session without resetting the draft.
- [x] (2026-03-07) Milestone D (Edit-from-prefab entry points): implement deterministic “seed Gen3D from Gen3D-saved prefab” APIs (Edit and Fork semantics).
- [x] (2026-03-07) Milestone E (Meta panel wiring): add Meta panel buttons Copy/Edit/Fork (gated to Gen3D-saved prefabs) and connect them to the entry points.
- [x] (2026-03-07) Milestone F (Deterministic patch ops): add an engine “apply_patch-like” tool (`apply_draft_ops_v1`) with explicit IDs, stable part IDs, and a structured diff.
- [x] (2026-03-07) Milestone G (Snapshots + branching): add snapshot/diff/restore and workspace diff/merge/copy tools so the agent can branch and compare alternatives safely.

## Surprises & Discoveries

- Observation: The config loader rejected empty API keys even when using `mock://gen3d`, which blocked rendered Gen3D regressions without secrets.
  Evidence: `tools/gen3d_real_test.py` failed with HTTP 400 from `/v1/gen3d/build`: `config.toml: missing openai.token / openai.OPENAI_API_KEY (or env OPENAI_API_KEY)`.

## Decision Log

- Decision: Restrict Edit/Fork to Gen3D-saved prefabs only (descriptor provenance gate).
  Rationale: Avoid heuristic “reverse engineering” of arbitrary prefabs into Gen3D components/attachments.
  Date/Author: 2026-03-07 / flow + agent

- Decision: Meta panel has three buttons with explicit semantics: Copy (instance duplication), Edit (overwrite prefab id), Fork (new prefab id for selected instance).
  Rationale: Mirrors “duplicate vs edit in place vs branch” in code workflows.
  Date/Author: 2026-03-07 / flow + agent

- Decision: Default behavior should respect agent `done` (stop means stop), even if QA/review are incomplete.
  Rationale: Agent autonomy; explicit “best effort / unfinished” is better than the engine silently overriding intent.
  Date/Author: 2026-03-07 / flow + agent

- Decision: Remove runtime motion mapping as a required Gen3D step (keep algorithms, but don’t require mapping for Gen3D completion).
  Rationale: “Any animation” should not be constrained by pre-existing runtime rig taxonomies; keep Gen3D loop focused and assumption-minimal.
  Date/Author: 2026-03-07 / flow + agent

- Decision: Allow empty provider tokens when `base_url` starts with `mock://gen3d` (debug/test builds only).
  Rationale: Enables deterministic, rendered end-to-end Gen3D tests without requiring secrets (the mock backend never calls the network).
  Date/Author: 2026-03-07 / agent

- Decision: Respect `done` unconditionally (except empty draft) and report unfinished QA/review as warnings, not guardrails.
  Rationale: “Stop means stop” is required for agent autonomy and resumable sessions; visibility beats silent override.
  Date/Author: 2026-03-07 / agent

- Decision: Stop is a pause (keeps draft + artifacts), and Resume continues the same `run_id` on a new `pass` (budgets accumulate across resumes).
  Rationale: Enables iterative work without losing context while keeping artifacts append-only and budgets enforceable.
  Date/Author: 2026-03-07 / agent

- Decision: Persist a Gen3D “source bundle” alongside Gen3D-saved prefabs and load it for Edit/Fork seeding; fall back to deterministic reconstruction from saved prefab defs when missing.
  Rationale: “Editing source code” beats heuristic reverse-engineering of “compiled output”, but older saves must remain editable best-effort without guessing.
  Date/Author: 2026-03-07 / agent

## Outcomes & Retrospective

- Milestone A delivered:
  - Added `qa_v1` (composed `validate_v1` + `smoke_check_v1`) with a compact `{ ok, validate, smoke, errors, warnings }` summary and a `qa.json` artifact.
  - Added run-dir-scoped artifact tools: `list_run_artifacts_v1`, `read_artifact_v1`, `search_artifacts_v1` (bounded; rejects traversal; no arbitrary paths).
  - Updated the agent prompt to prefer `qa_v1` and to mention artifact tools.
  - Updated `mock://gen3d` agent flow to call `qa_v1` so QA composition is exercised in offline runs.
  - Ran the required rendered smoke test and a rendered `tools/gen3d_real_test.py` run using `mock://gen3d` (no key).

- Milestone B delivered:
  - Agent `done` now ends the run immediately when a usable draft exists (empty draft remains a hard guardrail).
  - Instead of continuing the run, the UI status message includes best-effort “unfinished checks” warnings (missing QA/review, motion validation failure, missing motion path).
  - Updated the agent prompt to prefer running `qa_v1` before `done`.

- Milestone C delivered:
  - Stop no longer resets the Gen3D session state, so the draft and artifacts remain available for continuation.
  - Added “Continue” in the Gen3D UI and `POST /v1/gen3d/resume` in the Automation HTTP API.
  - Verified Stop → Continue works in a rendered run via `tools/gen3d_real_test.py --stop-resume` using `mock://gen3d`.

- Milestone D delivered:
  - Added Automation HTTP APIs: `POST /v1/gen3d/edit_from_prefab` and `POST /v1/gen3d/fork_from_prefab` (Gen3D-saved prefabs only).
  - Extended Gen3D Save to persist `gen3d_source_v1/` alongside saved prefabs and to record `provenance.gen3d.extra.source_bundle_v1` in the prefab descriptor.
  - Implemented Edit (overwrite) Save semantics by allowing the Save path to keep the same root prefab id; prunes stale prefab-def JSON files on overwrite.
  - Verified Edit overwrite + Fork new-id via a rendered regression: `tools/gen3d_real_test.py --edit-fork-regression` using `mock://gen3d` (no key).

- Milestone E delivered:
  - Added Meta panel Gen3D actions (Gen3D-saved prefabs only): Copy (duplicate instance), Edit (overwrite prefab id), Fork (new prefab id for the selected instance).
  - Edit/Fork from the Meta panel seed a Gen3D session from the selected instance and switch to Build Preview scene.
  - Save is now seed-aware: Edit refreshes all instances of the overwritten prefab id; Fork rebinds only the selected instance to the new prefab id.
  - Added Automation HTTP APIs: `POST /v1/meta/gen3d/copy`, `POST /v1/meta/gen3d/edit`, `POST /v1/meta/gen3d/fork`.
  - Verified via a rendered regression: `tools/gen3d_real_test.py --meta-edit-fork-regression` using `mock://gen3d` (no key).

- Milestone F delivered:
  - Introduced stable `part_id` for Gen3D-generated primitive parts (deterministic UUIDv5; key: `gravimera/gen3d/part/<component>/<part_idx>`).
  - Added `query_component_parts_v1` (bounded) so the agent can inspect component parts and obtain stable `part_id_uuid` targets.
  - Added `apply_draft_ops_v1`, a deterministic “apply_patch-like” tool with strict validation, explicit targets, and a structured diff:
    - anchor transforms (by component + anchor),
    - attachment offsets (by child component),
    - primitive part add/remove/update (by component + `part_id_uuid`),
    - animation slot upsert/remove (by child component + channel).
  - `apply_draft_ops_v1` writes a JSONL transaction log (`draft_ops.jsonl`) and a summary snapshot (`apply_draft_ops_last.json`) under the current pass dir.
  - Added an Automation API hook for rendered tests: `POST /v1/gen3d/apply_draft_ops`.
  - Verified via rendered regressions: `tools/gen3d_real_test.py --apply-draft-ops-regression` using `mock://gen3d` (no key).

- Milestone G delivered:
  - Workspaces now capture the full “editable session state” needed for safe branching (not just `draft.defs`): `planned_components`, `plan_hash`, `assembly_rev`, motion authoring state, reuse groups/warnings.
  - Added snapshot tools:
    - `snapshot_v1`, `list_snapshots_v1`
    - `diff_snapshots_v1` (structured per-component diff)
    - `restore_snapshot_v1` (blocked while running; workspace-scoped restore)
    - snapshot events are logged to `snapshots.jsonl` under the run dir.
  - Added workspace branching tools:
    - `diff_workspaces_v1` (structured diff)
    - `copy_from_workspace_v1` (explicit cherry-picks into the active workspace)
    - `merge_workspace_v1` (deterministic 3-way merge; conflicts returned; no auto-resolve).
  - Added Automation HTTP APIs for rendered tests: `/v1/gen3d/*snapshots*` and `/v1/gen3d/*workspace*`.
  - Verified in rendered runs via `tools/gen3d_real_test.py --snapshots-regression` and `--workspace-regression` using `tests/gen3d_real_test/config.toml` + `mock://gen3d` (no key).

## Context and Orientation

Gen3D code lives in `src/gen3d/`.

Important modules:

- `src/gen3d/ai/agent_loop/mod.rs`: top-level per-frame agent state machine (waiting for step, executing actions, waiting for async tools).
- `src/gen3d/ai/agent_step.rs`: parses an agent step and executes its actions; currently contains “done guardrails”.
- `src/gen3d/ai/agent_tool_dispatch.rs`: deterministic tool dispatch for non-LLM tools and LLM-tool starters.
- `src/gen3d/ai/agent_tool_poll.rs`: polls async LLM tool results and applies them deterministically.
- `src/gen3d/agent/tools.rs`: tool registry: IDs, descriptions, and examples.
- `src/gen3d/save.rs`: saves Gen3D drafts into depot prefabs + descriptors (includes provenance fields).

Prefab descriptors:

- Type: `crate::prefab_descriptors::PrefabDescriptorFileV1` in `src/prefab_descriptors.rs`.
- Gen3D provenance lives under `descriptor.provenance.source=="gen3d"` and `descriptor.provenance.gen3d` (contains prompt/run_id and extra fields).
- Spec: `docs/gamedesign/35_prefab_descriptors_v1.md`.

Meta panel UI:

- Implemented in `src/motion_ui.rs` today (shows algorithms/brains). This is where the Copy/Edit/Fork buttons live.

Terminology:

- **Draft**: the in-progress Gen3D object graph shown in the Gen3D preview.
- **Saved prefab**: the persisted prefab defs + descriptor in the model depot/realm; many instances can reference the same prefab id.
- **Edit (overwrite)**: modify the saved prefab id in place (all instances update).
- **Fork**: create a new prefab id from an existing one (only the selected instance rebinds).

## Plan of Work

This is intentionally implemented in small, shippable milestones. Each milestone should be:

- additive when possible,
- deterministic (no heuristics),
- observable (artifacts + structured status),
- gated by a rendered smoke test,
- committed with a clear message.

### Milestone A: Foundation tools (QA + artifact reads)

Goal: give the agent a single “tests” button and the ability to inspect its own run history like a codebase.

1) Add a composed tool `qa_v1` that runs `validate_v1` + `smoke_check_v1` and returns a combined JSON summary.
2) Add run-dir-scoped artifact tools:
   - `list_run_artifacts_v1`
   - `read_artifact_v1` (bounded)
   - (optional) `search_artifacts_v1` (bounded)

Acceptance:

- The agent can call `qa_v1` and get `{ ok, validate, smoke, errors, warnings }`.
- The agent can list and read the last pass’s artifacts without the model re-generating summaries.
- The tools cannot read arbitrary filesystem paths; they are scoped to the current run dir only.

### Milestone B: “Stop means stop” (`done` respected)

Goal: make `done` unconditional (except empty draft), while keeping “unfinished state” explicit.

1) Change the agent `done` handling in `src/gen3d/ai/agent_step.rs`:
   - keep only the empty-draft guard (no primitives → ignore done),
   - otherwise stop immediately.
2) Ensure the agent can still observe that QA/review are incomplete via `get_state_summary_v1` and/or `qa_v1`.
3) Update the agent system prompt to emphasize: “prefer QA before done”, but do not enforce it engine-side.

Acceptance:

- When the agent outputs `done`, the run stops and the draft remains in preview.
- UI status reflects whether QA/review were incomplete (best-effort messaging).

### Milestone C: Resumable sessions (Continue after Stop)

Goal: a stopped run can be resumed without resetting the draft and without requiring new user images.

1) Separate “cancel in-flight async work” from “reset session state”.
2) Add a new UI action and/or API route to `resume` the current Gen3D session.
3) Ensure session context (draft + planned components + last tool results) remains available after Stop.

Acceptance:

- Player can Stop, then Continue, and the agent resumes from current state without losing the draft.

### Milestone D: Seed edit sessions from Gen3D-saved prefabs (Gen3D-only)

Goal: open Gen3D with a draft derived deterministically from a saved Gen3D prefab.

Preferred approach (recommended): treat prefabs like “compiled output” and store Gen3D “source” so Edit/Fork is exact.

1) Extend Gen3D save to persist a compact “source bundle” alongside any Gen3D-saved prefab, sufficient to reopen the session later without reverse engineering:
   - minimum: draft object graph + stable IDs + attachment edges + anchors/offsets + planned component names
   - optional: applied ops log / snapshots
   - include a version tag (e.g. `gen3d_source_v1`) and a size cap
2) Implement Edit/Fork seeding by loading this source bundle into a new Gen3D session deterministically.

Fallback approach (only if needed): deterministic reconstruction from prefab defs + provenance.

3) If we cannot load a source bundle (older saves), reconstruct a “best effort” editable draft from “saved prefab defs” → “Gen3D editable draft”:
   - extract component names from `ObjectDef.label` (`gen3d_component_<name>`) and/or descriptor provenance extra,
   - reconstruct attachment edges from `ObjectRef` parts with attachments,
   - carry over primitives, anchors, and attachment offsets exactly (no inference),
   - if required info is missing, return a hard error (no heuristics).
2) Add entry points:
   - start edit (overwrite) for a given prefab id,
   - start fork (new prefab id) for a given prefab id.

Acceptance:

- If the selected prefab is Gen3D-saved, Edit/Fork opens a Gen3D session that visually matches the prefab.
- If not Gen3D-saved, Edit/Fork returns a hard error (no heuristics).

### Milestone E: Meta panel Copy/Edit/Fork buttons

Goal: expose Copy/Edit/Fork from the Meta panel, gated to Gen3D-saved prefabs.

1) Add UI buttons in `src/motion_ui.rs`.
2) Gate visibility and errors using `PrefabDescriptorLibrary` provenance.
3) Wire:
   - Copy → duplicate/spawn instance (same prefab id).
   - Edit → open Gen3D edit session (overwrite).
   - Fork → open Gen3D edit session (fork).

Acceptance:

- Buttons appear only for Gen3D-saved prefabs and perform the correct action.

### Milestone F: Deterministic patch ops (`apply_draft_ops_v1`)

Goal: provide a tool equivalent of “apply_patch”, with explicit targets and a structured diff.

1) Introduce stable IDs for primitives/parts where needed (prefer `part_id` rather than indices).
2) Define an ops schema (v1) that can:
   - tweak anchors and attachment offsets,
   - add/remove/update primitive parts by id,
   - add/remove/update animation slots by edge+channel.
3) Apply ops deterministically and record them as a transaction log (JSONL) for replay/undo.

Acceptance:

- The agent can perform small edits without calling an LLM review-delta loop.
- Every op is explicit and produces a diff summary; invalid ops are rejected with structured errors.

### Milestone G: Snapshots + branching

Goal: enable safe exploration like “git branches”.

1) Add snapshot/diff/restore for a Gen3D session.
2) Add workspace diff/copy/merge tooling (merge must return conflicts; no auto-resolve).

Acceptance:

- The agent can branch, compare, and selectively copy changes across workspaces.

## Concrete Steps

General workflow for each milestone:

1) Implement the changes.
2) Run the rendered smoke test:

   - tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

3) Commit with a clear message.

## Validation and Acceptance

Minimum per-milestone validation:

- `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

When adding tools, add focused `cargo test` coverage for parsing/apply logic when feasible.

## Idempotence and Recovery

- Every milestone should be repeatable: if a feature is partially implemented, keep it behind a tool id or UI gate rather than leaving half-wired UI paths.
- Prefer adding new tool IDs rather than changing semantics of existing ones unless the milestone explicitly calls for it.
