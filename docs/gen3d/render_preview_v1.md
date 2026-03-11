# `render_preview_v1`

Side-effect Gen3D tool that renders deterministic preview images to the Gen3D run cache directory.
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
  "images": ["..."],
  "static_images": ["..."],
  "motion_sheets": {
    "move": "…/move_sheet.png",
    "attack": "…/attack_sheet.png"
  }
}
```

- `images` contains everything returned by the tool (static + motion sheets).
- `static_images` excludes the motion sheet images.
- `motion_sheets.move` / `motion_sheets.attack` are `null` if not produced.

## Motion sprite sheets (`include_motion_sheets=true`)

If enabled, the engine captures front-view frames at phases `[0.00, 0.25, 0.50, 0.75]` and composes a 2×2 PNG sheet:

- `move_sheet.png`: samples the `move` channel over the inferred move cycle.
- `attack_sheet.png`: samples `attack_primary` over the inferred attack window.

## “Does this send images to the LLM?”

No. `render_preview_v1` renders locally and returns file paths.
Images are only sent to an AI model when you call `llm_review_delta_v1` and provide (or auto-use) preview images.

