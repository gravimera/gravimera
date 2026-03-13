# `render_preview_v1`

Side-effect Gen3D tool that renders deterministic preview images and registers them as **Info Store blobs** (opaque `blob_id`s).
This tool does **not** mutate the draft.

## Args

Schema (simplified):

```json
{
  "views": ["front", "left_back", "right_back", "top", "bottom"],
  "resolution": 960,
  "prefix": "render",
  "include_motion_sheets": true
}
```

Notes:
- `resolution` renders a square image; alternatively use `width` + `height`.
- `image_size` is accepted for back-compat and treated as “max dimension”.

## Output

```json
{
  "blob_ids": ["..."],
  "static_blob_ids": ["..."],
  "motion_sheet_blob_ids": { "move": "...", "attack": "..." }
}
```

- `blob_ids` contains everything returned by the tool (static + motion sheets).
- `static_blob_ids` excludes the motion sheet blobs.
- `motion_sheet_blob_ids.move` / `.attack` are `null` if not produced.

The engine still writes PNG files into the run cache for humans, but **does not** return filesystem paths to the agent.

## Blob labels

Use `info_blobs_list_v1` + `info_blobs_get_v1` to inspect recent previews by label (no paths). Labels include:

- `workspace:<workspace_id>`
- `kind:render_preview` plus `view:<view>` (`front`, `left_back`, `right_back`, `front_left`, `front_right`, `back`, `top`, `bottom`)
- `kind:motion_sheet` plus `motion:move` / `motion:attack`

## Motion sprite sheets (`include_motion_sheets=true`)

If enabled, the engine captures front-view frames at phases `[0.00, 0.25, 0.50, 0.75]` and composes a 2×2 PNG sheet:

- `move_sheet.png`: samples the `move` channel over the inferred move cycle.
- `attack_sheet.png`: samples `attack_primary` over the inferred attack window.

## “Does this send images to the LLM?”

No. `render_preview_v1` renders locally and returns `blob_id`s.
Images are only sent to an AI model when you call `llm_review_delta_v1` and provide (or auto-use) `preview_blob_ids`.
