# Gen3D Workshop (MVP)

Gen3D is an in-game “workshop” mode that drafts a 3D object from **0–3 reference photos (optional) and/or a text prompt** using an AI vision-capable model (OpenAI-compatible, Gemini, or Claude). When reference photos are provided, Gen3D first runs a single “image → object summary” request and then continues generation using text only (the raw user photos are not attached to downstream LLM requests). The draft is built from a small set of **atom primitives** (cuboid/sphere/cylinder/cone) and assembled as a **combined object** via data-driven composition.

This file describes the **current implementation** in this repo.

The AI is instructed to prioritize **basic structure and proportions** over small decorative details, and to keep components explainable.

---

## User Workflow

1. Enter Gen3D: click the **Gen3D** button (top-left).
2. Drag & drop **0–3** images (supported: `.png`, `.jpg/.jpeg`, `.webp`).
   - You can drop anywhere in the window; the prompt area is the visual target.
   - Thumbnails appear inside the prompt box on the right. Click a thumbnail to open the viewer (`↑/↓` navigate, `Esc` to close) while keeping the 3D preview visible.
   - Limits: at most 3 images total; each image must be smaller than 5 MiB (over-limit images are refused with a tip).
3. Optional: type notes/style in the prompt box (supports Chinese/IME input, emoji, paste via `Cmd/Ctrl+V`, and scrolling for long text; **Clear Prompt** wipes it).
   - On WSL, paste prefers the Windows clipboard via interop (`powershell.exe` / `clip.exe`). If interop is disabled, install `wl-clipboard` or `xclip`/`xsel`.
   - The game always provides a default style: “Concise Voxel/Pixel Art style (not necessarily cuboid-only).”
   - If your notes include a different style, the AI should prefer your notes over the default.
   - Limits: at most 250 whitespace-separated words and at most 2000 characters (extra input is refused with a tip).
   - You must provide at least one: a reference photo or a text prompt.
4. Click **Build** (each click starts a fresh run and overwrites the current draft).
   - While building, click **Stop** to cancel.
   - You can switch back to **Realm** while building; Gen3D keeps working in the background. Return to Preview any time to inspect progress.
   - A **Generating** entry appears in the Prefabs panel immediately. Click it to reopen Gen3D and see progress. Cancel removes the entry; you can also remove it manually from the Prefabs panel. Entries persist across restarts for cleanup.
5. Review in the preview panel:
   - Orbit/zoom (LMB drag / mouse wheel).
   - Select preview motion channel via the **Anim** dropdown (lists available channels; canonical: Idle / Move / Attack).
   - Toggle collider overlay via **Collision: On/Off** (also controls whether saved buildings block unit movement).
   - Open the **Status** overlay via the top-right `≡` button (collapsed by default).
     - **Status** tab: compact counters (components/parts/motion/pass) + scrollable step logs (why/result/duration).
     - **Prefab** tab: current prefab descriptor details (no revision history).
   - The preview shows run time and AI token counters (run + total) at top-left.
6. If needed, change notes/images and click **Build** again.
7. After Build completes, Gen3D **auto-saves** the prefab and refreshes the **Prefab** tab details.
   - You can still click **Save** to save additional copies (or save mid-run while building).
   - After leaving Gen3D, you can edit it in Build mode selection (delete/duplicate/move/rotate/scale).

If generation fails, the Status log shows the step-by-step errors (why/result/duration). Detailed logs and artifacts are also written under the Gen3D cache directory.

---

## Prefabs Panel In-Flight + Mock Concurrency

- Every Build writes an in-flight entry in the Prefabs panel immediately (status: **Generating** / **Queued** / **Failed**).
- Clicking an in-flight entry switches back to the Gen3D panel to show that run’s status.
- Canceling a run (or clicking the `X` remove button) removes the in-flight entry.
- In-flight entries persist across restarts for cleanup if needed.

Mock mode (for testing without token cost):

- Enable with `[gen3d].mock_enabled = true`.
- Each mock run sleeps for `[gen3d].mock_delay_seconds` (default: 60s) and then saves a simple placeholder prefab.
- Up to `[gen3d].max_parallel_jobs` mock runs execute concurrently; extra runs are queued FIFO.
- This path is intended for UI/testing; real Gen3D runs still execute as a single visible job at a time.

---

## Orchestrators (Agent + Pipeline)

Gen3D supports two orchestrators, selected via `[gen3d].orchestrator` in `config.toml`:

- `agent` (default): a Codex-style `agent_step` loop where the model decides which tool to call next.
- `pipeline`: a deterministic engine-driven state machine that runs the same high-level flow every time (plan → components → QA → optional render/review), using LLM-backed tools only as schema-constrained suggestion producers.
  - When the pipeline cannot make progress (tool schema failures beyond repair budget, repeated atomic DraftOps rejections, no-progress guard, etc.), it **falls back to `agent`** with an explicit status line and an Info Store `EngineLog` event.

---

## Agent Loop Orchestrator (Implemented)

Gen3D Build is a Codex-style, tool-driven agent loop.

- If reference images are present, the engine runs a single pre-agent “image → object summary” request and stores the result as `image_object_summary` (hard-capped at 300 words; the summarizer aims ~160–200 unless the object is unusually complex).
  - The agent and all Gen3D LLM-backed tools then receive *only* the user prompt text plus that summary (raw user reference photos are not sent to the LLM).
  - The agent can call `get_user_inputs_v2` to retrieve `{prompt, reference_images_count, image_object_summary}`; image paths are intentionally not exposed.

- The game calls the AI for a strict JSON `gen3d_agent_step_v1` object.
  - `status_summary` is shown to the player (Status tab).
  - `actions` are executed by the engine.
- Actions are either:
  - `tool_call`: versioned, engine-validated tools (`*_v1`)
  - `done`: stop the run (best-effort draft stays in the preview)

Tool contracts and error recovery:

- Each agent step prompt includes a complete tool list with a brief **args signature** (first line of the tool’s args schema) plus a bounded **example args object**.
  - The agent should use these signatures to avoid malformed tool calls.
  - Only tools whose args signature is exactly `{}` should ever be called with empty `{}` args.
- When a tool call fails, the next step’s “Recent tool results” entry includes the error plus a compact hint (expected args signature, required keys, and an example args object) so the agent can correct the call in the next step without spending an extra `get_tool_detail_v1`.
- `get_tool_detail_v1` still exists for deep inspection of complex tool schemas, but basic correct invocation should not require it.

In practice, the agent usually gets good results by calling:

- `llm_generate_plan_v1` (plan components + anchors + tree attachments)
- `llm_generate_component_v1` (generate one component’s primitives + anchors)
- `render_preview_v1` (optional; appearance review) + `validate_v1` / `smoke_check_v1` (structural checks)
- `llm_review_delta_v1` (apply machine-appliable tweaks / request replan / request regen)
  - If `preview_blob_ids` is omitted and `[gen3d].review_appearance = true`, the engine uses the latest *engine-rendered* preview blobs; if those are missing or stale for the current `assembly_rev`, it auto-captures a minimal set of review renders (and only captures motion sheets when motion validation reports errors) before calling the model.
  - User reference photos are never used as preview images.

Plan-level reuse:

- Plans may include `reuse_groups` to reuse already-generated geometry for symmetric/repeated parts.
- Use explicit reuse tools (avoid regenerating repeated parts):
  - `copy_component_v1`: copy an identical component into one or more targets (rotation-only mount alignment).
  - `mirror_component_v1`: mirror a component into one or more targets (mount-local +X reflection; correct for L/R symmetry).
  - `copy_component_subtree_v1`: copy an identical limb chain/subtree.
  - `mirror_component_subtree_v1`: mirror a limb chain/subtree for L/R symmetry.
- Rendering note: mirroring can introduce negative-scale transforms; the renderer detects mirrored transforms and uses mirrored-winding meshes for primitives so backface culling stays correct.
- Reuse tools default to `anchors=preserve_interfaces`: preserve each target component’s mount interface (and external child-attachment anchors), but copy other anchors so internal anchors stay consistent with copied/mirrored geometry.
  - Use `preserve_target` only if you must keep *all* target anchors unchanged.
  - Use `copy_source` to overwrite target anchors to match the source exactly.
  - When copying a subtree, the engine may need internal parent anchors (e.g. `next`) to exist on the target in order to attach expanded descendants. With `preserve_interfaces` / `copy_source`, missing internal parent anchors are hydrated deterministically from the source so subtree expansion can proceed. With `preserve_target`, missing required anchors still fail the copy.

The loop continues until the AI returns `done`, the user clicks **Stop**, or a budget/no-progress guard stops best-effort.

---

## Pipeline Orchestrator (Implemented)

When `[gen3d].orchestrator = "pipeline"`, the engine runs a deterministic pipeline orchestrator.

Create sessions (new builds):

- Ensure (optional) image summary exists.
- `llm_generate_plan_v1` → `llm_generate_components_v1` (missing-only) → `qa_v1` remediation loop.
- If QA indicates motion validation failures, the pipeline calls `llm_generate_motion_authoring_v1` and re-runs QA.
- If QA still fails after bounded attempts, the pipeline calls `llm_review_delta_v1` as deterministic remediation (apply replan/regen/tweak actions via engine code) and loops back to QA.

Seeded Edit/Fork sessions (DraftOps-first):

- Preserve-mode diff replanning: `get_plan_template_v1` → `llm_generate_plan_ops_v1` (applies ops deterministically in-engine).
- Capture editable part snapshots: `query_component_parts_v1` (per-component; snapshots stored in Info Store).
- Primitive editing: `llm_generate_draft_ops_v1` (suggestions only; strict JSON) → `apply_draft_ops_v1` (engine applies ops atomically with `if_assembly_rev` gating).

Debugging signals:

- Status lines include pipeline phases like “Pipeline: planning… / generating components… / QA…”.
- DraftOps application writes `apply_draft_ops_last.json` under the current `attempt_*/pass_*/` folder.

---

## How It Maps To The Object System

Gen3D uses the prefab-based object system:

- Each component becomes an `ObjectDef` prefab containing only **primitive** parts.
- The root `gen3d_draft` `ObjectDef` is a combined object with `ObjectRef` parts that reference the component prefabs via **anchor-based attachments** (tree-style).
- Clicking **Save** clones the current draft into fresh UUID-based prefab ids (so multiple saved models can coexist) and spawns an instance in the world (unit if the prefab has mobility; otherwise a build object).
- For saved units, the origin is recentered to the assembled model’s **rest-pose bounds center**. The saved collider is preserved from the plan (main-body footprint) and is used for selection/click targeting; it is NOT auto-expanded to enclose long tails/wings/protrusions.

“Atom vs Combined” is expressed purely through composition:

- Atom object: prefab with a single primitive/model part.
- Combined object: prefab with multiple parts and/or nested `ObjectRef`s.

---

## Motion Animations (AI-authored)

Gravimera does not provide runtime motion algorithms. Motion is authored as explicit animation
slots baked onto prefab attachment edges.

How it works:

- Gen3D models are saved as prefab defs (`*.json`) plus prefab descriptors (`*.desc.json`).
- The engine writes a derived summary into descriptors:
  - `interfaces.extra.motion_summary` (structured summary of available channels, drivers, clip kinds, etc).
- For movable units (mobility `ground` / `air`), the Gen3D agent should call
  `llm_generate_motion_authoring_v1` to author at least `move` (and usually `idle`; plus
  `attack_primary` if applicable).

In-game UX (Realm):

- Double-click a unit’s **selection circle** to open the **Meta** panel.
- The Meta panel shows a read-only animation summary plus Brain and Gen3D actions.
  (There is no animation/algorithm selection UI.)

## Cache / Debugging Artifacts

Each run creates a folder under `~/.gravimera/cache/gen3d/` by default (override via `[gen3d].cache_dir` / `gen3d_cache_dir` in `config.toml`):

`~/.gravimera/cache/gen3d/<run_uuid>/`

The game stores request/response + tool artifacts there (useful for debugging):

- `gen3d_run.log` (per-run log with request attempts / HTTP statuses / fallbacks)
- `agent_trace.jsonl` (per-run structured trace: LLM requests/responses + tool calls/results)
- `tool_feedback.jsonl` (per-run; also appended to the global `~/.gravimera/cache/gen3d/tool_feedback_history.jsonl`)
- `save_*.json` (each Save click writes a small metadata artifact)
- `attempt_0/inputs/user_prompt.txt` (raw user prompt as typed)
- `attempt_0/inputs/images/*` (copies of the input reference photos; used for UI preview and the one-time image summary step)
- `attempt_0/inputs/image_object_summary.txt` (one-time “image → object summary”; downstream LLM calls use this text instead of user photos)
- `attempt_0/inputs/image_object_summary.json` (summary metadata: word_count/truncated/images_count)
- `attempt_0/inputs_manifest.json` (maps original paths → cached copies)
- `plan_*` (planning)
- `plan_extracted.json` (engine-extracted plan summary, including computed absolute `pos` + `forward`/`up`)
- `assembly_transforms.json` (latest computed absolute transforms, including `pos` + `forward`/`up`)
- `componentXX_*` (component generation)
- `*_system_text.txt` / `*_user_text.txt` (exact system/user text sent to the AI for that request)
- `render_*.png` (agent-triggered preview renders)
- Endpoint-specific raw responses are stored as `*_responses_raw.txt` / `*_chat_raw.json` depending on the API used.

Per-step (under `attempt_0/pass_*/`):

- `tool_calls.jsonl` / `tool_results.jsonl`
- `gravimera.log`

---

## `config.toml` (AI + Gen3D + Logging)

Start by copying `config.example.toml` to `~/.gravimera/config.toml` (the real `config.toml` is gitignored so secrets don’t get committed).

Config lookup:

- The game loads `~/.gravimera/config.toml` by default (fallback: `config.toml` next to the running binary, then `./config.toml`).
- Override config path:
  - `cargo run -- --config ./some_config.toml`
  - or `GRAVIMERA_CONFIG=./some_config.toml cargo run`

Logging:

- By default, Gravimera writes the main log to `~/.gravimera/gravimera.log`.
- Override in `config.toml`:
  - `[log].path = "./gravimera.log"` (relative to the config file directory)
  - `[log].level = "debug"` (default: `"info"`)
- Disable file logging: set `[log].path = ""`.

Gen3D budgets / guard:

- `[gen3d].review_appearance` controls whether the AI reviews visual appearance from preview renders (default: `false` / structural-only).
  - Note: this controls *image-based* review. In seeded Edit/Fork sessions, Gen3D can still apply machine-appliable alignment/attachment tweaks from user notes even when `review_appearance=false` (no renders sent to the model).
- `[gen3d].max_seconds` / `[gen3d].max_tokens` cap a Build run (set to `0` to disable a budget).
- No-progress guard (set either to `0` to disable):
  - `[gen3d].no_progress_tries_max` stops best-effort after N “try” steps (mutating tools) without changing the assembled-state hash.
  - `[gen3d].inspection_steps_max` stops best-effort after N inspection-only steps (read-only / QA / tool lookup) without changing the assembled-state hash.

Gen3D AI provider:

- `[gen3d].ai_service = "openai"` (default) uses `[openai]` config (`token` or env `OPENAI_API_KEY`).
- `[gen3d].ai_service = "gemini"` uses `[gemini]` config (`token` or env `X_GOOG_API_KEY` / `GEMINI_API_KEY`).
  - Gen3D requests Gemini Structured Outputs via `response_json_schema` for schema-constrained calls; use a Gemini model that supports it (Gemini 2.0+).
- `[gen3d].ai_service = "claude"` uses `[claude]` config (`token` or env `ANTHROPIC_API_KEY` / `CLAUDE_API_KEY`).

Gen3D mock testing:

- Set the provider `base_url` to `mock://gen3d` (debug/test builds only) to avoid real token usage.
- Optional: `[gen3d].mock_delay_seconds = 60` adds a simulated delay (per build) so UI/progress can be tested under slow runs.
- Note: the mock backend does not accept reference images; use prompt-only tests.
- An isolated config is available at `test/gen3d_mock/config.toml`; run with `GRAVIMERA_CONFIG=./test/gen3d_mock/config.toml` and optionally set `GRAVIMERA_HOME` to isolate cache/logs.

---

## AI JSON Schemas (Strict)

### Plan JSON (version 8)

```json
{
  "version": 8,
  "mobility": { "kind": "static" },
  "collider": { "kind": "aabb_xz", "half_extents": [2.0, 2.0] },
  "assembly_notes": "Short notes about shared dimensions / alignment / style.",
  "root_component": "seat",
  "components": [
    {
      "name": "seat",
      "purpose": "What this component is for.",
      "modeling_notes": "How to model it with primitives.",
      "size": [1.2, 0.4, 1.2],
      "anchors": [
        { "name": "back_socket", "pos": [0.0, 0.2, -0.6], "forward": [0.0, 0.0, -1.0], "up": [0.0, 1.0, 0.0] }
      ]
    },
    {
      "name": "back",
      "purpose": "The backrest.",
      "modeling_notes": "A thin vertical slab.",
      "size": [1.2, 0.8, 0.2],
      "anchors": [
        { "name": "bottom_socket", "pos": [0.0, -0.4, 0.1], "forward": [0.0, 0.0, -1.0], "up": [0.0, 1.0, 0.0] }
      ],
      "attach_to": {
        "parent": "seat",
        "parent_anchor": "back_socket",
        "child_anchor": "bottom_socket",
        "offset": { "pos": [0.0, 0.0, 0.0] }
      }
    }
  ]
}
```

`collider` is REQUIRED for movable units (mobility `ground` / `air`) and optional for static objects. It supports:

- `{ "kind": "none" }`
- `{ "kind": "circle_xz", "radius": number }`
- `{ "kind": "aabb_xz", "half_extents": [hx, hz] }`

Notes:

- This plan has **no absolute placement**. Assembly uses a tree of `attach_to` links.
- For movable units, `collider` is the unit's selection/click hit area; size it to the MAIN BODY footprint (do not inflate it to cover long appendages).
- Anchor names must be stable and unique per component.
- Do **not** output an anchor named `"origin"`; the engine provides an implicit identity anchor `"origin"`.
- `attach_to.offset` is a tweak transform in the **parent-anchor join frame** (after alignment).
  - If you include `offset.forward`/`offset.up` or `offset.rot_quat_xyzw`, you MUST include `offset.rot_frame` explicitly (`"join"` or `"parent"`).
    - `"join"`: author the rotation directly in the join frame.
    - `"parent"`: author the rotation in the parent component frame (matching anchors); the engine converts it into the join frame.
- The engine does **not** apply heuristic placement tweaks (no automatic overlap/surface nudges). If you need inset/outset/overlap at a join, encode it explicitly in `attach_to.offset.pos`.
- Placement sanity check (ignore rotation): estimate `child_origin ~= parent_anchor.pos + attach_to.offset.pos - child_anchor.pos`. If that would place a component far away from where it should visually sit, adjust anchor positions and/or the component's size along the attachment direction.
- Define attachment anchors as JOIN frames (each expressed in its OWN component-local coordinates):
  - `parent_anchor.forward` (+Z) points from the parent toward the child (attachment direction) in the parent component's local axes.
  - `child_anchor.forward` (+Z) and `child_anchor.up` (+Y) are expressed in the child component's local axes; set them to match the child's modeling axes at that joint so the child can rotate into the parent's join frame.
    - They do NOT need to numerically equal the parent's vectors.
    - Example: if a chain link is modeled along the child's local +Z axis, use `forward=[0,0,1]` and `up=[0,1,0]` for its joint anchors.
  - Do NOT make the join frames 180° opposed (that flips the child). If you need a flip, encode it via `attach_to.offset` rotation.
  - For intermediate chain links with exactly 2 joint anchors (one parent, one child), the vector from the proximal joint anchor to the distal joint anchor should be aligned with the proximal anchor's +Z (forward) axis in component-local space; otherwise motion validation may report `chain_axis_mismatch`.
  Then `attach_to.offset.pos[2]` becomes a reliable in/out control along the attachment direction.
- Animation policy: Gen3D plans do **not** include animation clips (`attach_to.animations` is not part of the plan schema).
  - For movable units, motion is authored via `llm_generate_motion_authoring_v1` and baked onto attachment edges.

Motion metadata (optional):

- `rig.move_cycle_m` (optional): meters per `move` cycle when using the `move_phase` driver.
- `components[].attach_to.joint` (optional): joint constraint for this attachment edge, expressed in the **parent-anchor join frame**
  (the same frame as `attach_to.offset` and attachment animation deltas).
  - `hinge` joints should include `axis_join` and (optionally) `limits_degrees`.
- `components[].contacts[]` (optional): named ground contacts for this component.
  - Each contact references a component anchor by name.
  - For planted `kind: "ground"` contacts (feet/hooves), include `stance` so locomotion can coordinate gait phasing.

### Component Draft JSON (version 2)

```json
{
  "version": 2,
  "anchors": [
    { "name": "bottom_socket", "pos": [0.0, -0.4, 0.1], "forward": [0.0, 0.0, -1.0], "up": [0.0, 1.0, 0.0] }
  ],
	  "parts": [
	    {
	      "primitive": "cuboid",
	      "params": null,
	      "color": [0.8, 0.8, 0.8, 1.0],
	      "render_priority": 0,
	      "pos": [0.0, 0.2, 0.0],
	      "forward": [0.0, 0.0, -1.0],
	      "up": [0.0, 1.0, 0.0],
	      "scale": [1.2, 0.4, 1.2]
	    }
	  ]
	}
```

Notes:

- `scale` is a **size vector** in world units (sx, sy, sz), not a raw Transform scale.
- `color` is **required** for every part (RGBA in 0..1). The engine rejects drafts that omit it.
- `forward` / `up` are direction vectors (no Euler angles). The engine normalizes them and repairs common degeneracies.
- `params` is accepted for a small set of primitive variants (`capsule`, `conical_frustum`, `torus`).
	- IMPORTANT: all `anchors[]` and `parts[]` transforms are **component-local**. The engine assembles components by aligning anchors.
	- The engine does **not** auto-nudge part placement. If you add thin surface details, place them slightly proud of the supporting surface so they remain visible.
	  - To avoid z-fighting (coplanar overlapping faces), do not place multiple primitives with the exact same planar face plane; use a small inset/outset epsilon, and avoid concentric capped cylinders with identical cap planes.
	  - The renderer applies a small per-part depth bias as a best-effort tie-break for coplanar overlaps, but you should not rely on it.
	  - Optional: set `render_priority` (small integer) to hint which parts should be rendered “in front” when faces end up coplanar. Higher values are biased slightly closer to the camera; keep values small (|render_priority| <= 3).
	- The draft must include **all anchors required by the plan** (extra anchors are ignored).
	- Convention: component origin is the component's center, so the component center should be at local `[0,0,0]`.
	- Robustness: the engine recenters each generated component to its primitive bounds center (and shifts anchors by the same amount).

---

## Limits (MVP)

- Images: 0–3 (each must be smaller than 5 MiB)
- Prompt textbox: ≤ 250 whitespace-separated words and ≤ 2000 characters
- Components (plan): current build mode ≤ 24 (hard cap ≤ 64)
- Total primitives (across all components): hard cap ≤ 1024
- Primitives: cuboid / sphere / cylinder / cone (plus optional `params` for `capsule`, `conical_frustum`, `torus`)

---

## Future Work (Not Implemented Yet)

- Add more primitive parameters and richer atom libraries.
- Add “generate only this component” and version/rollback UI.
