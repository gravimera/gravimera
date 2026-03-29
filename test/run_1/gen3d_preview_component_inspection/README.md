## Gen3D preview component inspection real test

This rendered HTTP regression uses the default `~/.gravimera` home to load an existing
Gen3D-saved prefab with nested component depth.

It verifies that:

- `POST /v1/gen3d/edit_from_prefab` loads a nested Gen3D prefab into the preview
- `GET /v1/gen3d/preview/components` exposes nested component frames
- `POST /v1/gen3d/preview/probe` can resolve more than one component instead of collapsing to a
  single top-level torso-like hit
- `POST /v1/gen3d/preview/explode` causes visible component separation in the preview

Run with:

```bash
python3 test/run_1/gen3d_preview_component_inspection/run.py
```
