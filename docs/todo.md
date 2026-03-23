See implementation plan: `docs/execplans/todo_gen3d_motion_authoring_split_channels_and_webp.md`.

- [x] Gen3D motion authoring
  - [x] Split motion authoring per-channel (one tool call == one channel) and batch in parallel for multiple channels.
  - [x] Remove the default `ambient` channel.
  - [x] Allow unlimited motion channels; number keys `1`..`0` force the prefab’s ordered top-10 channels.
- [x] Fix warning: `WARN bevy_image::image: feature "webp" is not enabled` (2026-03-23T11:09:39.834536Z)
