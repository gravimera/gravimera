# Sky (atmosphere)

Gravimera uses Bevy’s built-in atmosphere sky for rendered mode.

## Where it’s configured

- Camera setup: `src/water_scene.rs` (`ensure_main_camera_atmosphere`)

## Notes

- The atmosphere is attached to the `MainCamera` and also enables `AtmosphereEnvironmentMapLight`
  so PBR materials (including water) get plausible environment lighting/reflections.

