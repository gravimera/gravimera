# Water (ocean background)

Gravimera’s rendered mode includes a simple ocean surface powered by the `bevy_water` crate.

## Where it’s configured

- Plugin wiring: `src/app.rs`
- Settings + camera depth prepass: `src/water_scene.rs`
- Sky (atmosphere): `docs/sky.md`

## Tuning

The water surface is controlled by `bevy_water::WaterSettings` (a Bevy resource).

Common knobs (see `src/water_scene.rs`):

- `height`: base Y of the water surface (keep below the ground plane at `y=0` to avoid flooding)
- `amplitude`: wave height multiplier
- `spawn_tiles`: grid size (tiles are `256x256` world units each)
- `water_quality`: mesh subdivision + shader quality (`Basic`..`Ultra`)

## Depth prepass

`src/water_scene.rs` adds `DepthPrepass` to the `MainCamera` so the water shader can use the depth buffer
for shallow/deep tinting near geometry.
