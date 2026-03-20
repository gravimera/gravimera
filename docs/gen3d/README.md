# Gen3D notes

Entry point: `gen_3d.md`.

## DraftOps application helpers (agent-step/manual)

Agent-step runs often use `llm_generate_draft_ops_v1` to *suggest* a bounded list of DraftOps.
Because the main agent prompt only includes compact tool-result summaries, the engine provides
deterministic “apply-by-reference” tools:

- `apply_last_draft_ops_v1`: applies the latest `attempt_*/pass_*/draft_ops_suggested_last.json`.
- `apply_draft_ops_from_event_v1`: applies DraftOps from a specific Info Store `tool_call_result`
  `event_id` for `llm_generate_draft_ops_v1`.

Both tools apply via `apply_draft_ops_v1` with:

- `atomic=true` (no partial commits when ops are invalid)
- `if_assembly_rev` gating (stale suggestions cannot apply to a changed draft)

Artifacts:

- `attempt_*/pass_*/draft_ops_suggested_last.json`
- `attempt_*/pass_*/apply_draft_ops_last.json`

