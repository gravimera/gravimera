# Gen3D Workshop (MVP)

Gen3D is an in-game “workshop” mode that drafts a 3D object from **0–6 photos and/or a text prompt** using an OpenAI vision-capable model. The draft is built from a small set of **atom primitives** (cuboid/sphere/cylinder/cone) and assembled as a **combined object** via data-driven composition.

This file describes the **current implementation** in this repo.

The AI is instructed to prioritize **basic structure and proportions** over small decorative details, and to keep components explainable.

---

## User Workflow

1. Enter Gen3D: click the **Gen3D** button (top-left).
2. Drag & drop **0–6** images into the game window (supported: `.png`, `.jpg/.jpeg`, `.webp`).
   - Tip: click a thumbnail to open the original image viewer (`↑/↓` navigate, `Esc` to close) while keeping the 3D preview visible for comparison.
3. Optional: type notes/style in the prompt box (supports paste via `Cmd/Ctrl+V`, and scrolling for long text; **Clear Prompt** wipes it).
   - On WSL, paste/copy prefers the Windows clipboard via interop (`powershell.exe` / `clip.exe`). If interop is disabled, install `wl-clipboard` or `xclip`/`xsel`.
   - The game always provides a default style: “Concise Voxel/Pixel Art style (not necessarily cuboid-only).”
   - If your notes include a different style, the AI should prefer your notes over the default.
   - You must provide at least one: a reference photo or a text prompt.
4. Click **Build** (each click starts a fresh run and overwrites the current draft).
   - While building, click **Stop** to cancel.
5. Review in the preview panel:
   - Orbit/zoom (LMB drag / mouse wheel).
   - Select preview animation state via the **Anim** dropdown (Idle / Move / Attack).
   - Toggle collider overlay via **Collision: On/Off**.
   - Open the **Status / Tool Feedback** overlay via the top-right `≡` button (collapsed by default).
   - The preview shows run time and AI token counters (run + total) at top-left.
6. If needed, change notes/images and click **Build** again.
7. Once a usable draft exists (root + at least one primitive part), click **Save** to place the generated model next to the hero.
   - You can **Save multiple times**, even while building.
   - After leaving Gen3D, you can edit it in Build mode selection (delete/duplicate/move/rotate/scale).

If generation fails, the status panel shows a short error summary; detailed step-by-step logs are printed to the terminal at debug level.

---

## Agent Loop (Implemented)

Gen3D Build is a Codex-style, tool-driven agent loop.

- The game calls the AI for a strict JSON `gen3d_agent_step_v1` object.
  - `status_summary` is shown to the player (Status tab).
  - `actions` are executed by the engine.
- Actions are either:
  - `tool_call`: versioned, engine-validated tools (`*_v1`)
  - `done`: stop the run (best-effort draft stays in the preview)

In practice, the agent usually gets good results by calling:

- `llm_generate_plan_v1` (plan components + anchors + tree attachments)
- `llm_generate_component_v1` (generate one component’s primitives + anchors)
- `render_preview_v1` + `validate_v1` / `smoke_check_v1` (self-review inputs)
- `llm_review_delta_v1` (apply machine-appliable tweaks / request replan / request regen)

The loop continues until the AI returns `done`, the user clicks **Stop**, or a budget/no-progress guard stops best-effort.

---

## How It Maps To The Object System

Gen3D uses the prefab-based object system:

- Each component becomes an `ObjectDef` prefab containing only **primitive** parts.
- The root `gen3d_draft` `ObjectDef` is a combined object with `ObjectRef` parts that reference the component prefabs via **anchor-based attachments** (tree-style).
- Clicking **Save** clones the current draft into fresh UUID-based prefab ids (so multiple saved models can coexist) and spawns a Build instance referencing the new root prefab id.

“Atom vs Combined” is expressed purely through composition:

- Atom object: prefab with a single primitive/model part.
- Combined object: prefab with multiple parts and/or nested `ObjectRef`s.

---

## Cache / Debugging Artifacts

Each run creates a folder under `~/.gravimera/cache/gen3d/` by default (override via `[gen3d].cache_dir` / `gen3d_cache_dir` in `config.toml`):

`~/.gravimera/cache/gen3d/<run_uuid>/`

The game stores request/response + tool artifacts there (useful for debugging):

- `gen3d_run.log` (per-run log with request attempts / HTTP statuses / fallbacks)
- `agent_trace.jsonl` (per-run structured trace: LLM requests/responses + tool calls/results)
- `tool_feedback.jsonl` (per-run; also appended to the global `~/.gravimera/cache/gen3d/tool_feedback_history.jsonl`)
- `save_*.json` (each Save click writes a small metadata artifact)
- `inputs/user_prompt.txt` (raw user prompt as typed)
- `inputs/images/*` (copies of the input reference photos)
- `inputs_manifest.json` (maps original paths → cached copies)
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

## `config.toml` (OpenAI + Gen3D + Logging)

Start by copying `config.example.toml` to `~/.gravimera/config.toml` (the real `config.toml` is gitignored so secrets don’t get committed).

Config lookup:

- The game loads `~/.gravimera/config.toml` by default (fallback: `config.toml` next to the running binary, then `./config.toml`).
- Override config path:
  - `cargo run -- --config ./some_config.toml`
  - or `GRAVIMERA_CONFIG=./some_config.toml cargo run`

Optional logging:

- Add `log_path = "./gravimera.log"` (top-level).
  - Relative paths are resolved relative to the config file directory.

Gen3D budgets / guard:

- `[gen3d].max_seconds` / `[gen3d].max_tokens` cap a Build run (set to `0` to disable a budget).
- `[gen3d].no_progress_max_steps` stops best-effort if the agent produces no progress for N steps (set to `0` to disable).

---

## AI JSON Schemas (Strict)

### Plan JSON (version 7)

```json
{
  "version": 7,
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

`collider` is optional and supports:

- `{ "kind": "none" }`
- `{ "kind": "circle_xz", "radius": number }`
- `{ "kind": "aabb_xz", "half_extents": [hx, hz] }`

Notes:

- This plan has **no absolute placement**. Assembly uses a tree of `attach_to` links.
- Anchor names must be stable and unique per component.
- Do **not** output an anchor named `"origin"`; the engine provides an implicit identity anchor `"origin"`.
- `attach_to.offset` is a tweak transform in the **parent anchor frame** (after alignment).
- The engine does **not** apply heuristic placement tweaks (no automatic overlap/surface nudges). If you need inset/outset/overlap at a join, encode it explicitly in `attach_to.offset.pos`.
- Define attachment anchors as a JOIN FRAME that matches on both sides:
  - `parent_anchor.forward` (+Z) points from the parent toward the child (attachment direction).
  - `child_anchor.forward` (+Z) points in the SAME direction as `parent_anchor.forward` (do not make it opposite, or the child will flip 180°).
  - `parent_anchor.up` (+Y) and `child_anchor.up` (+Y) should generally match to avoid unintended roll.
  Then `attach_to.offset.pos[2]` becomes a reliable in/out control along the attachment direction.
- Component-level animation lives on attachments via `attach_to.animations` (preferred). `attach_to.animation` is a legacy field.
- Attachment animations (`attach_to.animations`) are keyed by channel (`ambient`, `idle`, `move`, `attack_primary`) and each animation spec contains:
  - `driver`: `always` (seconds), `attack_time` (seconds), `move_phase` (meters traveled while moving), `move_distance` (meters traveled; can be signed for spins).
  - `speed_scale` (optional): multiplies the driver time.
  - `time_offset_units` (optional): additive offset in the clip time domain (same units as loop `duration_secs` / keyframe `time_secs`). Use this to phase-stagger repeated limbs without duplicating keyframes.
  - `clip`: either `loop` (keyframed deltas) or `spin` (procedural rotation).
    - For `loop` keyframes, `delta.pos` is in the **parent-anchor join frame** (the same frame as `attach_to.offset.pos`).
    - If you include `delta.forward` / `delta.up`, author them as **direction vectors in the parent component frame** (same coordinates as anchors). The engine converts them into the join frame.
      - Rest pose should match the parent anchor frame: `delta.forward = parent_anchor.forward` and `delta.up = parent_anchor.up`.
      - If you don't need rotation, omit `delta.forward` / `delta.up` entirely.
    - For `spin`, the `axis` is authored in the **child component's local axes** (+X right, +Y up, +Z forward). The engine converts it into the attachment join frame.

Rig / motion contract (optional; used for locomotion validation and AI repair):

- `rig.move_cycle_m` (optional): meters per `move` cycle when using the `move_phase` driver.
- `components[].attach_to.joint` (optional): joint constraint for this attachment edge, expressed in the **parent-anchor join frame**
  (the same frame as `attach_to.offset` and attachment animation deltas).
  - `hinge` joints should include `axis_join` and (optionally) `limits_degrees`.
- `components[].contacts[]` (optional): named ground contacts for this component.
  - Each contact references a component anchor by name.
  - Optional `stance` schedule `{ "phase_01": 0..1, "duty_factor_01": 0..1 }` is used by motion validation to detect obvious slip/lift.

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
      "color": [0.8, 0.8, 0.8, 1.0],
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
- The draft must include **all anchors required by the plan** (extra anchors are ignored).
- Convention: component origin is the component's center, so the component center should be at local `[0,0,0]`.
- Robustness: the engine recenters each generated component to its primitive bounds center (and shifts anchors by the same amount).

---

## Limits (MVP)

- Images: 0–6
- Components (plan): current build mode ≤ 24 (hard cap ≤ 64)
- Total primitives (across all components): hard cap ≤ 1024
- Primitives: cuboid / sphere / cylinder / cone (plus optional `params` for `capsule`, `conical_frustum`, `torus`)

---

## Future Work (Not Implemented Yet)

- Add more primitive parameters and richer atom libraries.
- Add “generate only this component” and version/rollback UI.
