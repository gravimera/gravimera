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
  - Fix: ignore `$...` placeholder paths in preview-image parsing (`src/gen3d/ai/agent_review_images.rs`) and add an explicit “no placeholders” rule to the agent system prompt (`src/gen3d/ai/agent_prompt.rs`).

- Saving multiple Gen3D models could stack them on the same spawn position next to the hero, making animation inspection hard.
  - Fix: scatter spawn positions deterministically by save sequence (rings around the hero) in `src/gen3d/save.rs`.

Note: Some prompts (e.g. “A horse”) may still exhaust the Gen3D time budget and finish “best effort”. This is not considered an engine failure as long as:

- `draft_ready=true` (primitives exist),
- Save works,
- movement capture works,
- and (if attackable) firing works.

## 2026-02-10

- `/responses` returned SSE output, but Gen3D treated it as non-JSON and failed with “`/responses returned no output text`”, then fell back to `/chat/completions` (often 504).
  - Symptom: Real tests intermittently fail early during plan/component calls even though HTTP status is 200.
  - Fix: parse SSE payloads and extract `response.output_text.delta` / `response.output_text.done` (and accept `"type":"text"` parts) in `src/gen3d/ai/openai.rs`.

## 2026-02-20

- Mirrored component copies could render with incorrect shading / culling (e.g. one side of wheels appears like a dark disc).
  - Cause: mirror alignment is represented by a negative determinant transform (negative scale). This flips triangle winding, which interacts with default back-face culling.
  - Fix: when spawning visuals, detect mirrored transforms (negative scale determinant) and swap to a cached mesh variant with inverted winding for primitive meshes (`src/object/visuals.rs`).

## 2026-02-21

- Staggered-limb move animations could still step in-phase even when `time_offset_units` is non-zero.
  - Cause: AI-authored loop keyframes repeated with a period equal to the configured `time_offset_units`, making the offset a no-op (e.g. `A,B,A,B,A`).
  - Repro: cache run `~/.gravimera/cache/gen3d/214755a0-e96e-499a-8e55-7710ec9ebd17`.
  - Fix: add a motion validation error (`time_offset_no_effect`) so the repair loop (`llm_review_delta_v1`) adjusts keyframes/offset instead of accepting the broken gait.

## 2026-03-10

- Gen3D unit grounding could be wrong when a component contains a large, nearly-invisible “scaffold” primitive.
  - Symptom: the saved unit floats above the ground; review renders show a faint, oversized cuboid volume.
  - Repro: cache run `~/.gravimera/cache/gen3d/88813978-03c2-4416-bf48-1bf0c3bbfb14`.
  - Cause: grounding used bounds-derived `ground_origin_y` and the scaffold primitive extended the root bounds far below the intended foot contacts.
  - Fix:
    - Drop AI-authored primitives with near-zero alpha during component conversion (`src/gen3d/ai/convert.rs`).
    - When saving unit roots, prefer grounding from declared ground contacts (min-Y of contact anchors) and prune near-invisible primitives from the root component before bounds/size calculations (`src/gen3d/save.rs`).
