# Sky (atmosphere)

Gravimera uses Bevy’s built-in atmosphere sky for rendered mode.

## Where it’s configured

- Camera setup: `src/water_scene.rs` (`ensure_main_camera_atmosphere`)

## Notes

- The atmosphere is attached to the `MainCamera` and also enables `AtmosphereEnvironmentMapLight`
  so PBR materials (including water) get plausible environment lighting/reflections.
- `DistanceFog` is used alongside the atmosphere to soften the ocean/sky horizon.
- Rendered mode disables Bevy’s `GlobalAmbientLight` and relies on a single `DirectionalLight` plus
  the atmosphere-generated environment map lighting. The environment map intensity is tuned down to
  avoid over-brightening the scene when the sky lighting kicks in.

## Performance

Atmosphere and atmosphere-driven environment lighting use compute shaders each frame.

`src/water_scene.rs` intentionally uses reduced-quality defaults (smaller LUTs, fewer samples, and a smaller
`AtmosphereEnvironmentMapLight.size`) to keep frame times reasonable.
