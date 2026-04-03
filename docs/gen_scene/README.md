# GenScene

GenScene rebuilds the Scene Builder as a Gen3D-style panel that generates complete scenes from a
single prompt. It creates a new scene immediately, then chooses existing terrain + prefabs or
triggers GenFloor/Gen3D when new assets are needed.

## Entering GenScene

1. Open **Scenes** (top panel).
2. Click **Generate** (next to **Import**).

The GenScene panel opens in **Scene Preview** and mirrors the Gen3D layout.

## Build Flow

1. Enter a prompt in the prompt bar.
2. Click **Build**.
3. GenScene creates a new scene, switches to it, and begins planning and generation.

While a build is running, the panel cannot be closed. Click **Stop** to cancel; once the build
finishes or is stopped, the panel can close normally.

## Asset Selection and Generation

GenScene uses a structured plan (JSON) from the AI model:

- Prefer existing terrain and prefabs that match the prompt.
- If no suitable terrain is available, it runs GenFloor and applies the result.
- If no suitable prefabs are available, it runs Gen3D and applies the generated models.
- Placements are applied as scene source patches and compiled into the scene.

When GenScene triggers GenFloor or Gen3D, the **Terrain** and **3D Models** panels immediately
show placeholder items using the same queue logic as their dedicated panels.

## Outputs

Each run writes artifacts under the scene run directory, for example:

- `scenes/<scene_id>/runs/gen_scene_<uuid>/plan.json`

These artifacts capture the AI plan and applied patch for auditability.

## Automation API

See `docs/automation_http_api.md` for `/v1/gen_scene/*` endpoints.
