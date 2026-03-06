# Gen3D: Decisions + Next Actions

This document records Gen3D process decisions and forward-looking next actions.
It may describe behavior that is **not implemented yet**.

Last updated: 2026-03-07

## Non‑negotiable constraints

1) **No heuristic engine algorithms.**
   - A user can ask Gen3D to generate **any object** and **any animation**.
   - Engine behavior must be deterministic + schema/tool driven, not “guess intent”.
   - If required inputs are missing/ambiguous, prefer: **error → regenerate/replan**, not silent fallbacks.

2) **Agent-driven orchestration (Codex-style).**
   - The LLM “agent” drives the iteration loop by choosing tools.
   - The engine provides tools that are:
     - versioned (`*_v1`),
     - strictly validated,
     - deterministic in how they apply results,
     - observable via structured state + artifacts.

## Current implementation snapshot (orientation)

- Build is a tool-driven agent loop (`gen3d_agent_step_v1` → tool calls/results → next step).
- “QA” today is split across tools:
  - `validate_v1` (structural consistency checks),
  - `smoke_check_v1` (behavioral checks + motion validation summary),
  - `render_preview_v1` + `llm_review_delta_v1` (optional appearance loop).
- Today the engine may **ignore** an agent `done` request in some cases (e.g. QA not run, review pending, motion validation failed, etc.). (We intend to change this; see next section.)

## Decision (TODO): respect `done` by default

Decision: when the agent outputs `{"kind":"done"}`, and the draft is non-empty, the engine should
stop the run **even if** there are outstanding issues.

The engine must still make the situation explicit and machine-observable:

- The agent should have easy access to: “QA hasn’t been run”, “motion validation failed”, “review pending”, etc.
- The UI status should clearly reflect “finished early / best effort” and surface the latest QA summary.
- Stopping must not perform hidden “auto-fixes” (no heuristics); it may only run explicit tools.

Rationale:
- The agent may stop intentionally due to time/token/regen budgets or strategic tradeoffs.
- “Stop means stop” aligns with Codex-style autonomy, while the engine remains the deterministic executor/inspector.

Implementation sketch (later):
- Keep the “empty draft” guard (ignore `done` if there are no primitives).
- Otherwise, accept `done` unconditionally and end the run.
- Optional (recommended): auto-run a single composed QA tool (`qa_v1`, below) at stop time and attach its results to the final status/artifacts (but do not block stop).

## Decision (TODO): remove runtime motion mapping from the Gen3D loop

Decision: the Gen3D agent loop should NOT rely on engine-injected runtime motion algorithms for
“units look animated when moving”.

- Keep the runtime motion algorithms in the codebase (they may still be used by other workflows
  or reintroduced later).
- Remove “motion mapping” as a required Gen3D step:
  - the agent should not be required to produce `motion_roles_v1` / `motion_rig_v1`,
  - the run should be allowed to finish with zero authored animation clips.
- If the player wants animations, treat that as an explicit *edit* operation (agent generates/edits
  animation clips as authored data, not as a mapping to a pre-existing runtime algorithm).

Rationale:
- “Any animation” is fundamentally open-ended; mapping into a small set of predefined runtime rigs
  can become a hidden constraint.
- Keeping Gen3D’s default loop focused on geometry + structure reduces assumptions.

Non-goals:
- Do not delete runtime algorithms; do not forbid them in the product forever.

Implementation sketch (later):
- Remove/soften motion-related `done` guardrails; instead only *surface* motion/animation status.
- Update the agent prompt to treat animation authoring as optional and explicitly user-driven.

## Decision (TODO): Gen3D sessions are resumable + editable after stop/save

Decision: “Stop” ends the current background loop, but the *session* (draft + context + artifacts)
remains available so the agent (and user) can continue later.

User-visible outcomes:
- A “Continue” action should resume the agent loop on the existing draft.
- A saved prefab should be editable: from the Meta panel, a player can open a Gen3D edit session
  seeded from that prefab and issue prompts like:
  - “regenerate animation for move/attack”
  - “regenerate component X”
  - “regenerate the whole model but keep silhouette”

Scope constraint:
- Editing is supported **only for Gen3D-saved prefabs** (prefabs whose descriptor includes
  `provenance.source="gen3d"` and `provenance.gen3d`).

Meta panel UX (planned):
- **Copy**: duplicate/spawn a new instance that references the same prefab id (no Gen3D).
- **Edit**: open Gen3D seeded from this prefab and **overwrite the same prefab id** on save.
  - This affects all instances that reference that prefab id.
- **Fork**: open Gen3D seeded from this prefab and **save to a new prefab id**, then rebind only
  the selected instance to the new prefab id.

Key constraints (no heuristics):
- “Editing” must be explicit, schema/tool driven:
  - exact component IDs/names/indices, explicit anchors/edges, explicit animation channels.
- Any conversion from an existing prefab → Gen3D editable draft must be deterministic and not rely
  on intent-guessing decomposition.

Implementation sketch (later):
- Add “edit session” entry points:
  - `start_edit_session_from_prefab_v1` (seed draft from existing prefab defs + descriptor),
  - `resume_session_v1` (continue the agent loop after stop).
- Persist minimal provenance in prefab descriptors so edit sessions can recover stable component
  names and structure without depending on a local cache folder.
- Implement the Meta panel buttons and wire them to edit-session entry points.

## Tooling roadmap (agent-facing)

## Codex-like JSON editing toolbox (ideas)

Goal: make Gen3D feel like “Codex editing a JSON codebase”: the agent can inspect state/history,
apply deterministic patches, branch/compare alternatives, and resume work with full context.

## Context window / token budget policy (design)

Goal: enable many “Continue” iterations without blowing up LLM context windows.

Principles:

- **Prefer stateless requests**: each agent step/tool call should be solvable from a compact
  `state_summary` + on-demand artifact reads, not by growing hidden conversation state.
- **History is pull-based**: the engine writes detailed artifacts; the agent fetches only what it
  needs via bounded read/search tools instead of embedding long histories in every prompt.
- **Diffs over dumps**: prefer small “what changed” summaries and transaction logs over resending
  full plan/draft JSON blobs repeatedly.
- **Strict caps everywhere**: cap recent tool results included inline; cap images per request and
  prefer low-res renders during iteration; cap artifact read sizes.

Implementation implications (later):

- Avoid relying on provider conversation continuation (`previous_response_id` / chat history) as the
  primary memory mechanism; keep memory explicit and inspectable via artifacts.
- Make `get_state_summary_v1` the canonical compact “working set” input for the agent.
- Add `list_run_artifacts_v1` / `read_artifact_v1` / `search_artifacts_v1` (bounded, run-dir scoped).

### Phase 1: observability (read the “repo”)

- Artifact index + bounded reads:
  - `list_run_artifacts_v1`: enumerate artifacts for the current run/pass (JSON/JSONL/TXT/PNG) with sizes + timestamps.
  - `read_artifact_v1`: bounded reads (`max_bytes`, `tail_lines`, `json_pointer`, `jsonl_range`), scoped to the current run dir only.
  - `search_artifacts_v1`: bounded substring search across run artifacts (scoped) to find errors/issue kinds quickly.
- Structured queries over the current draft (avoid forcing the agent to parse huge JSON blobs):
  - `query_components_v1` (filter by name/id/generated/missing),
  - `query_anchors_v1` (filter by component + anchor name),
  - `query_attachments_v1` (edges, joints, offsets, channels),
  - `query_animation_slots_v1` (by channel/driver/clip kind).

### Phase 2: deterministic patching (the “apply_patch” equivalent)

- Apply explicit, validated edit operations and return a structured diff:
  - `apply_draft_ops_v1`: primitive/anchor/attachment edits on the current draft.
  - `apply_plan_ops_v1`: planned-component edits when you want plan+draft to stay aligned.
- Operations should be explicit and unambiguous (no selection heuristics):
  - set/tweak anchor transform,
  - set/tweak attachment offset (with explicit rotation frame),
  - add/remove primitive part by stable id or index,
  - replace a component’s geometry from a provided JSON draft (strict schema),
  - add/remove animation slot on a specific attachment edge + channel.
- Return:
  - `{ ok, applied_ops, rejected_ops, diff_summary, changed_component_ids, new_assembly_rev }`.

### Phase 3: snapshots + time travel (commit log)

- First-class snapshots for safe iteration:
  - `snapshot_v1` → `{ snapshot_id }` (captures draft + planned_components + key job state)
  - `diff_snapshots_v1` (structured diff)
  - `restore_snapshot_v1`
  - `undo_last_ops_v1` (only if ops are guaranteed reversible)
- Prefer replayability:
  - store applied ops as JSONL (“transaction log”) and ensure deterministic replay.

### Phase 4: branching + merging (workspaces as git branches)

Build on existing `create_workspace_v1` / `set_active_workspace_v1` / `delete_workspace_v1`:

- `diff_workspaces_v1`: stable, structured diff across drafts/plans/attachments/animations.
- `merge_workspace_v1`: no “auto resolve”; return explicit conflicts and require the agent to choose.
- `copy_from_workspace_v1`: explicit cherry-picks (component or subtree) from another workspace.

### Phase 5: instrumentation + debugging renders (not heuristics)

- Debug visual tools to make correctness observable without engine “auto-fixes”:
  - `render_debug_overlay_v1` (anchors/joint axes/contact points/frame glyphs)
  - render presets for consistent backgrounds/overlays so diffs are meaningful

### Phase 6: uniform async tasks + monitoring

- Generic async API for long tasks (beyond today’s ad-hoc parallel component waves):
  - `start_task_v1` / `poll_task_v1` / `cancel_task_v1`
- Tasks must be deterministic in application:
  - parallelize *generation/rendering*, but apply results in a declared order and record every apply as an op.

### 1) `qa_v1` (composed tool)

Goal: one call that returns a combined, compact QA verdict.

- Runs `validate_v1` + `smoke_check_v1`.
- Returns a stable summary JSON:
  - `{ ok, validate: {...}, smoke: {...}, errors: [...], warnings: [...] }`
- Writes artifacts to the run dir the same way the underlying tools do.

### 2) Artifact inspection tools (Codex-like “read the files”)

Goal: let the agent learn from its own history without re-prompting the user.

Candidates:
- `list_run_artifacts_v1`: enumerate recent artifacts (JSON/JSONL/TXT/PNGs) for the current run/pass.
- `read_artifact_v1`: read bounded slices (e.g. `max_bytes`, `tail_lines`, `json_pointer`) from JSON/JSONL/TXT artifacts.
  - Must be sandboxed to the current run dir (no arbitrary FS reads).

### 3) Generic async tasks + monitoring (beyond today’s ad-hoc async)

Goal: unify long-running work behind a consistent “start/poll/cancel” surface so the agent can
parallelize safely and monitor progress.

Candidates:
- `start_task_v1` → `{ task_id }`
- `poll_task_v1` → `{ status, progress, partial_outputs? }`
- `cancel_task_v1`

Targets for taskification:
- batch component generation waves,
- render capture (static + motion sheets),
- appearance review,
- export pipelines (e.g. GLB),
- future heavy validations.

### 4) Workspace branching + diffs

Goal: give the agent a safe way to try alternatives (like “git branches”) and compare results.

Existing tools: `create_workspace_v1`, `set_active_workspace_v1`, `delete_workspace_v1`.

Candidate additions:
- `diff_workspaces_v1`: structured diff (components/anchors/attachments/animations changed).
- `copy_from_workspace_v1`: selectively copy component(s) or subtrees from another workspace.

## Next actions checklist (docs + design)

- [ ] Document the “respect `done` by default” policy (including UI messaging and artifacts).
- [ ] Design `qa_v1` output schema + artifact names and update the agent prompt to prefer it.
- [ ] Design artifact inspection tools with strict scoping and bounded reads.
- [ ] Design a generic async task API and migrate existing ad-hoc async flows toward it.
- [ ] Specify “no runtime motion mapping” changes to the agent prompt + QA gating.
- [ ] Specify resumable sessions + “Edit prefab” workflow + save semantics (fork vs overwrite).
- [ ] Add Meta panel buttons: Copy / Edit / Fork (Gen3D-prefab-gated).
