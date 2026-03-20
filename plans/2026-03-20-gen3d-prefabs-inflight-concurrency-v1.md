# Gen3D Prefab Panel In-Flight Jobs and Concurrency

## Objective

Enable starting multiple Gen3D generation jobs (up to three running concurrently) while showing each job immediately in the Prefabs panel as a “generating” entry that survives restarts, can be opened to view progress in the Gen3D panel, and can be manually removed. Canceling a job must remove its Prefabs entry. This must avoid heuristic scheduling, use a generic FIFO queue, and keep README clean by updating detailed docs under the docs folder instead.

Assumptions: the Prefabs panel is built by the model library UI, Gen3D state is currently single-job and stored in global resources, and we can add a small realm-scoped persistence file without conflicting with prefab package directories. These assumptions are based on `src/model_library_ui.rs:700`, `src/gen3d/state.rs:120`, `src/gen3d/ai/job.rs:570`, and `src/realm_prefab_packages.rs:7`.

## Implementation Plan

- [ ] 1. Introduce a `Gen3dJobManager` resource and per-job context that stores `Gen3dAiJob`, `Gen3dWorkshop`, `Gen3dDraft`, a FIFO queue, and an active job id. References: `src/gen3d/ai/job.rs:570`, `src/gen3d/state.rs:120`.
- [ ] 2. Register the manager as a Bevy resource at startup and define how the Gen3D UI reads the active job view from it so current UI paths stay intact. Reference: `src/app.rs:572`.
- [ ] 3. Define an in-flight prefab entry model with run id, label, status, queue position, timestamps, and error so the Prefabs panel can render “generating” rows. References: `src/gen3d/state.rs:120`, `src/model_library_ui.rs:700`.
- [ ] 4. Add a realm-scoped persistence file path for in-flight entries under the realm prefab root with a non-colliding filename. Reference: `src/realm_prefab_packages.rs:7`.
- [ ] 5. Implement load, save, and remove helpers for the in-flight file with atomic writes, and expose them for the job manager and Prefabs panel. Reference: `src/realm_prefab_packages.rs:21`.
- [ ] 6. Update Gen3D build/edit/fork start paths to create an in-flight entry immediately and enqueue a job context, including API entrypoints. References: `src/gen3d/ai/orchestration.rs:962`, `src/automation/mod.rs:1199`.
- [ ] 7. Add FIFO scheduling that starts up to three jobs concurrently, advances queued jobs when slots free, and recomputes queue positions. Reference: `src/gen3d/ai/orchestration.rs:1292`.
- [ ] 8. Wire cancel, failure, and completion flows to remove or replace in-flight entries (including persisted state) and mark the Prefabs list dirty. Reference: `src/gen3d/ai/orchestration.rs:221`.
- [ ] 9. Extend the Prefabs list builder to merge real prefab packages with in-flight entries, render a “generating” badge with queue position, and disable drag-and-drop for in-flight rows. References: `src/model_library_ui.rs:700`, `src/model_library_ui.rs:919`.
- [ ] 10. Add a manual remove control for in-flight Prefabs rows that deletes the persisted entry and removes queued jobs when applicable. Reference: `src/model_library_ui.rs:919`.
- [ ] 11. Update Prefabs item interactions so in-flight rows open the Gen3D panel for the run id, while real prefabs keep preview behavior and guard the preview open path. References: `src/model_library_ui.rs:1168`, `src/gen3d/ui.rs:205`.
- [ ] 12. Recalculate AI limiter capacity based on the concurrent job cap and per-job parallel components, and add a config knob for max concurrent Gen3D jobs with default three. References: `src/app.rs:443`, `config.example.toml:66`.
- [ ] 13. Update `gen_3d.md` (and any needed docs under `docs/`) to describe in-flight Prefabs behavior, manual removal, persistence, and the concurrency cap without touching README. Reference: `gen_3d.md:1`.
- [ ] 14. Add tests for persistence and queue/cancel behavior, using the existing test folder for fixtures per repo rules. Reference: `AGENTS.md:1`.

## Verification Criteria

- Starting four Gen3D builds results in three running jobs and one queued job; all four appear immediately in the Prefabs panel as “generating” entries with correct queue positions.
- Clicking a “generating” Prefabs entry opens the Gen3D panel and shows the correct run’s status log and progress; real prefabs still open the preview panel.
- Canceling a running or queued Gen3D job removes its Prefabs entry immediately and deletes the persisted in-flight record.
- Restarting the game restores in-flight entries in the Prefabs panel from disk and allows manual removal without errors.
- The Gen3D smoke test in `AGENTS.md:1` is run after implementation and the game starts without crash.

## Potential Risks and Mitigations

1. **Shared rendering resources could be contended by multiple Gen3D jobs, causing preview glitches or crashes.** Mitigation: gate preview rendering to the active job only and treat other jobs as background runs without preview capture unless explicitly opened.
2. **Persisted in-flight entries could become stale if a job fails unexpectedly, confusing users.** Mitigation: show a failed status with a manual remove action and clear entries on explicit cancel or completion.
3. **AI request concurrency could exceed provider rate limits when three jobs run component generation in parallel.** Mitigation: scale the global AI limiter by concurrent jobs and document the configuration in the Gen3D config section.

## Alternative Approaches

1. Keep a separate Gen3D runs list in the Gen3D panel. This is simpler but conflicts with the requirement to surface jobs only in the Prefabs panel.
2. Create a placeholder prefab package directory at job start and write temporary data into it. This would integrate naturally with the Prefabs listing but introduces on-disk partial prefabs and cleanup complexity.
3. Restrict to a single active job and a queued list only, which reduces concurrency complexity but does not meet the requested “up to three concurrent” requirement.
