# Gen3D Real-Test Issues Log

Developer-facing log of issues found while running rendered, end-to-end Gen3D tests
(Build → Save → move / (optional) fire → screenshots), plus the fixes applied.

Each entry includes enough pointers (cache dir, script, commit) to reproduce.

## 2026-02-06

- Transient HTTP errors while polling Gen3D status (e.g. 502/503/504).
  - Symptom: `tools/gen3d_real_test.py` fails during `/v1/gen3d/status` polling even though the game is still running.
  - Fix: retry/backoff in `tools/gen3d_real_test.py`.

- Automation `/v1/gen3d/save` could exceed the server-side request timeout on large drafts.
  - Symptom: HTTP 504 with `{"ok":false,"error":"Automation request timed out"}`.
  - Fix: increase Automation server reply timeout to 600s in `src/automation/mod.rs` and use longer client timeouts for save/screenshot/step in `tools/gen3d_real_test.py`.

- Agent emitted placeholder file paths like `$CALL_1.render_paths[0]` for `llm_review_delta_v1`, causing a failed tool call.
  - Symptom: `llm_review_delta_v1` fails with “Failed to read image $CALL_…: No such file or directory”.
  - Fix: ignore `$...` placeholder paths in preview-image parsing and add an explicit “no placeholders” rule to the agent system prompt (`src/gen3d/ai/agent_loop.rs`).

- Saving multiple Gen3D models could stack them on the same spawn position next to the hero, making animation inspection hard.
  - Fix: scatter spawn positions deterministically by save sequence (rings around the hero) in `src/gen3d/save.rs`.

Note: Some prompts (e.g. “A horse”) may still exhaust the Gen3D time budget and finish “best effort”. This is not considered an engine failure as long as:

- `draft_ready=true` (primitives exist),
- Save works,
- movement capture works,
- and (if attackable) firing works.

