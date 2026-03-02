# Workspaces

In Build mode, the top-left workspace dropdown selects which saved world is active.

- Object Preview uses `scene.dat`.
- Scene Build uses `scene.build.dat` in the same build directory.
- Switching workspaces saves the current workspace and loads the other.

If `scene.scene_dat_path` is set in `config.toml`, Object Preview uses that path and Scene Build uses `scene.build.dat` in the same directory as the override.
