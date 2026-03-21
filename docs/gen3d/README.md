# Gen3D (AI model generation + deterministic editing)

Gen3D is Gravimera’s in-game AI modeling workflow: it generates a component plan, drafts
per-component primitive geometry, authors optional motion clips, and iterates with deterministic
validation/QA.

This doc focuses on the **workflow**, **orchestrators**, and the **tool contracts** the
agent/pipeline uses.

## Setup

1. Create a local config:

```bash
mkdir -p ~/.gravimera
cp config.example.toml ~/.gravimera/config.toml
```

2. Edit `~/.gravimera/config.toml`:

- Set your AI provider in `[openai]` / `[mimo]` / `[gemini]` / `[claude]`
- (Optional) Configure Gen3D behavior under `[gen3d]` (notably `orchestrator`)

## Run artifacts

By default, each Gen3D run writes artifacts under:

- `~/.gravimera/cache/gen3d/<run_id>/`
  - `attempt_<n>/pass_<m>/...` (per-pass artifacts)
  - `info_store_v1/` (KV + events + blobs metadata)

## Concurrent runs (Build UI)

Gen3D supports up to **3 concurrent running builds**. The Prefabs panel surfaces these runs via a
realm-local “in-flight” file:

- `.../<realm>/prefabs/gen3d_in_flight_v1.json` (see `src/realm_prefab_packages.rs`)

Behavior:

- Clicking the **Gen3D** button in the Prefabs panel starts a **new Gen3D session** (empty prompt /
  images). To view an existing run, click its row in the Prefabs list.
- If there are already 3 `running` runs, clicking **Gen3D** shows a toast: `生成中任务已满`.
- When a run finishes, its status updates to `completed` (the entry is not removed). Failures are
  `failed`. The remove button deletes the entry (and cancels the run if it is still running).
- Implementation detail: only the currently loaded session owns the preview/render context; inactive
  runs continue polling AI in the background but defer any render-dependent tool/step until loaded.

DraftOps-specific artifacts:

- `attempt_*/pass_*/draft_ops_suggested_last.json` — latest `llm_generate_draft_ops_v1` suggestion
  payload.
- `attempt_*/pass_*/apply_draft_ops_last.json` — latest `apply_draft_ops_v1` result (`diff_summary`,
  `rejected_ops`, etc).

## Orchestrators

Gen3D supports two orchestrators (config: `[gen3d].orchestrator`):

### 1) Agent-step (default)

The engine asks the model for a strict JSON `gen3d_agent_step_v1` object:

- `status_summary` is shown to the player
- `actions[]` contains tool calls (or `done`)

The engine executes the tool calls and re-prompts the agent with a bounded tool list + recent tool
results until the run ends.

### 2) Deterministic pipeline

When `[gen3d].orchestrator = "pipeline"`, the engine runs a deterministic state machine:

- Create sessions: plan → generate components → QA loop → (optional) renders/review-delta → finish
- Seeded Edit/Fork sessions: preserve-mode plan ops → capture part snapshots → DraftOps suggest+apply
  → QA loop → finish

If the pipeline cannot make progress (schema repair exhausted, repeated DraftOps rejections, etc.),
it falls back to agent-step with an explicit reason.

## DraftOps-first primitive editing (seeded edits)

For edit requests like “make the wings larger” where regeneration is not required, Gen3D prefers
**in-place primitive edits**:

1. Capture editable part snapshots (per-component): `query_component_parts_v1`
2. Ask the model for DraftOps suggestions: `llm_generate_draft_ops_v1`
3. Apply atomically with revision gating:
   - Pipeline: `apply_draft_ops_v1` with `atomic=true` and `if_assembly_rev=<current>`
   - Agent-step/manual: `apply_last_draft_ops_v1` (applies the latest
     `draft_ops_suggested_last.json`) or `apply_draft_ops_from_event_v1` (applies a specific
     suggestion `event_id`)

Key safety rules:

- Always apply DraftOps with `atomic=true` so invalid ops don’t partially accumulate.
- Always apply with `if_assembly_rev` so stale suggestions cannot apply to a changed assembly.

### DraftOps application helpers (agent-step/manual)

Agent-step runs often use `llm_generate_draft_ops_v1` to *suggest* a bounded list of DraftOps.
Because the main agent prompt only includes compact tool-result summaries, the engine provides
deterministic “apply-by-reference” tools:

- `apply_last_draft_ops_v1`: applies the latest `attempt_*/pass_*/draft_ops_suggested_last.json`.
- `apply_draft_ops_from_event_v1`: applies DraftOps from a specific Info Store `tool_call_result`
  `event_id` for `llm_generate_draft_ops_v1`.

Both tools apply via `apply_draft_ops_v1` with `atomic=true` and `if_assembly_rev` gating.

## Debugging

Useful tools:

- KV: `info_kv_list_keys_v1`, `info_kv_get_v1`
- Events: `info_events_list_v1`, `info_events_get_v1`, `info_events_search_v1`
- Draft diffs: inspect `apply_draft_ops_last.json` and `draft_ops_suggested_last.json` under the
  pass folders.

See also: `docs/execplans/gen3d_deterministic_pipeline.md`.
