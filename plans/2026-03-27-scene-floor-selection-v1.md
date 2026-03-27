# Per-Scene Floor Selection Persistence

## Objective

Ensure each scene remembers its selected floor across restarts and scene switches, defaulting to the built-in Default Floor when no selection exists, and keep UI selection consistent with the active floor. This will align floor choice with existing scene/realm persistence patterns. References: `src/realm.rs:13`, `src/realm.rs:43`, `src/scene_store.rs:2231`, `src/genfloor/runtime.rs:13`.

## Implementation Plan

- [ ] 1. Define a small versioned JSON file for per-scene floor selection and place it under the scene directory. Use existing scene path helpers to choose a stable path. References: `src/paths.rs:131`, `src/paths.rs:153`.
- [ ] 2. Add read/write helpers for the floor selection file that validate realm/scene IDs and tolerate missing files. Follow the active-selection persistence pattern for error handling. References: `src/realm.rs:39`, `src/realm.rs:366`.
- [ ] 3. During scene switch, load the stored floor selection and apply it to `ActiveWorldFloor`, and set Floors UI selection accordingly. If missing or invalid, fall back to `FloorDefV1::default_world()` and clear the selection. References: `src/scene_store.rs:2238`, `src/genfloor/runtime.rs:47`, `src/floor_library_ui.rs:1471`.
- [ ] 4. Persist floor selection when the user changes it in the Floors list so the scene restores it on restart. References: `src/floor_library_ui.rs:1443`.
- [ ] 5. Persist floor selection when GenFloor saves and applies a new floor so the scene restores it on restart. References: `src/genfloor/ui.rs:210`.
- [ ] 6. Update GenFloor docs to describe per-scene floor selection storage and default behavior. References: `docs/genfloor/README.md:10`.
- [ ] 7. Run the required smoke test after implementation using the project’s test folder convention. References: `AGENTS.md:1`.

## Verification Criteria

- On app restart, the last-selected floor for the active scene is restored and applied.
- Switching scenes restores each scene’s previously selected floor without cross-contamination.
- If no floor selection is stored for a scene, the Default Floor is applied and shown as selected.
- GenFloor save sets and persists the new floor selection immediately.
- Smoke test completes: `GRAVIMERA_HOME` pointing at `test/run_1` with `cargo run -- --rendered-seconds 2`.

## Potential Risks and Mitigations

1. **Risk: Stored floor ID points to a deleted or missing floor package.**
   Mitigation: On load, validate the package exists; fall back to Default Floor and overwrite the stored selection if missing.
2. **Risk: Scene switching can occur while UI state is mid-interaction, causing mismatched selection indicators.**
   Mitigation: Reset Floors UI selection to the loaded scene selection immediately after the scene switch is applied.

## Alternative Approaches

1. Store the selected floor inside `scene.dat` by bumping its version and adding a new field. Trade-off: couples scene object serialization to floor metadata and requires updating scene encoding/decoding logic.
2. Store floor selection at the realm level instead of per scene. Trade-off: simpler persistence but does not meet the per-scene requirement.
