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

## Tooling roadmap (agent-facing)

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

