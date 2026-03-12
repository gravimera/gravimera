# Gen3D Workshop (MVP)

Gen3D is an in-game “workshop” mode that drafts a 3D object from **0–6 photos and/or a text prompt** using an AI vision-capable model (OpenAI-compatible, Gemini, or Claude). The draft is built from a small set of **atom primitives** (cuboid/sphere/cylinder/cone) and assembled as a **combined object** via data-driven composition.

This file describes the **current implementation** in this repo.

The AI is instructed to prioritize **basic structure and proportions** over small decorative details, and to keep components explainable.

---

## User Workflow

1. Enter Gen3D: click the **Gen3D** button (top-left).
2. Drag & drop **0–6** images (supported: `.png`, `.jpg/.jpeg`, `.webp`).
   - You can drop anywhere in the window; the prompt area is the visual target.
   - Thumbnails appear inside the prompt box on the right. Click a thumbnail to open the viewer (`↑/↓` navigate, `Esc` to close) while keeping the 3D preview visible.
3. Optional: type notes/style in the prompt box (supports paste via `Cmd/Ctrl+V`, and scrolling for long text; **Clear Prompt** wipes it).
   - On WSL, paste/copy prefers the Windows clipboard via interop (`powershell.exe` / `clip.exe`). If interop is disabled, install `wl-clipboard` or `xclip`/`xsel`.
   - The game always provides a default style: “Concise Voxel/Pixel Art style (not necessarily cuboid-only).”
   - If your notes include a different style, the AI should prefer your notes over the default.
   - You must provide at least one: a reference photo or a text prompt.
4. Click **Build** (each click starts a fresh run and overwrites the current draft).
   - While building, click **Stop** to cancel.
   - You can switch back to **Realm** while building; Gen3D keeps working in the background. Return to Preview any time to inspect progress.
5. Review in the preview panel:
   - Orbit/zoom (LMB drag / mouse wheel).
   - Select preview motion channel via the **Anim** dropdown (lists available channels; canonical: Idle / Move / Attack).
   - Toggle collider overlay via **Collision: On/Off** (also controls whether saved buildings block unit movement).
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
- `render_preview_v1` (optional; appearance review) + `validate_v1` / `smoke_check_v1` (structural checks)
- `llm_review_delta_v1` (apply machine-appliable tweaks / request replan / request regen)
  - If `preview_images` is omitted and `[gen3d].review_appearance = true`, the engine uses the latest render outputs; if those are missing or stale for the current `assembly_rev`, it auto-captures a minimal set of review renders (and only captures motion sheets when motion validation reports errors) before calling the model.

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

## How It Maps To The Object System

Gen3D uses the prefab-based object system:

- Each component becomes an `ObjectDef` prefab containing only **primitive** parts.
- The root `gen3d_draft` `ObjectDef` is a combined object with `ObjectRef` parts that reference the component prefabs via **anchor-based attachments** (tree-style).
- Clicking **Save** clones the current draft into fresh UUID-based prefab ids (so multiple saved models can coexist) and spawns an instance in the world (unit if the prefab has mobility; otherwise a build object).
- For saved units, the origin + collision circle are based on the root component (torso/body), not far-out attachments (e.g. weapons).

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
- `[gen3d].ai_service = "claude"` uses `[claude]` config (`token` or env `ANTHROPIC_API_KEY` / `CLAUDE_API_KEY`).

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

`collider` is optional and supports:

- `{ "kind": "none" }`
- `{ "kind": "circle_xz", "radius": number }`
- `{ "kind": "aabb_xz", "half_extents": [hx, hz] }`

Notes:

- This plan has **no absolute placement**. Assembly uses a tree of `attach_to` links.
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

- Images: 0–6
- Components (plan): current build mode ≤ 24 (hard cap ≤ 64)
- Total primitives (across all components): hard cap ≤ 1024
- Primitives: cuboid / sphere / cylinder / cone (plus optional `params` for `capsule`, `conical_frustum`, `torus`)

---

## Future Work (Not Implemented Yet)

- Add more primitive parameters and richer atom libraries.
- Add “generate only this component” and version/rollback UI.
