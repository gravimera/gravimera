# Build a Post-Apocalyptic Sci-Fi Wasteland Town Showcase Scene (Automation API)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Create a new, compact showcase scene in the user’s default Gravimera home (`~/.gravimera`) that looks like a colorful **post-apocalyptic wasteland town with science-fiction elements**. The scene should feel immediately attractive from the first screenshot, but also readable as a place where people and machines actually live day to day.

The scene is intentionally different from the existing utopian chrome city. It should use a default-sized terrain, center the composition on **two crossing streets**, and populate that layout with newly generated roads, shops, vehicles, robots, drones, animals, homes, utility structures, and civilian life. The result must persist under `~/.gravimera/realm/default/scenes/<scene_id>/` so the user can load it later from the in-game scene list.

## Progress

- [x] (2026-04-05 19:22 CST) Reviewed the existing automation endpoints, the current showcase builder, and the prior chrome-scene ExecPlan to confirm the new scene can reuse the same durable scene-run pipeline.
- [x] (2026-04-05 19:22 CST) Confirmed the new direction with the user: a fresh profile using only newly created objects, compact default-sized terrain, two crossing streets, colorful post-apocalyptic sci-fi town, everyday-life feel.
- [x] (2026-04-05 19:22 CST) Add this new ExecPlan file and keep it updated during implementation.
- [x] (2026-04-05 19:35 CST) Refactor `tools/showcase_scene_builder.py` into a profile-driven builder so the chrome scene remains reproducible and the new wasteland town can be generated without copying the resumable runtime code.
- [x] (2026-04-05 19:35 CST) Add the wasteland asset program and layout logic: new terrain prompt, new prefab prompts, compact crossroads layout, colorful tint pass, and frequent incremental scene patches plus screenshots.
- [x] (2026-04-05 19:35 CST) Run the required rendered smoke test in an isolated home; the rendered game started and exited cleanly after the two-second limit.
- [ ] (2026-04-05 19:44 CST) Run the real rendered UI build against `~/.gravimera`, capture screenshots under `test/run_1/...`, review the result, and iterate if needed. Completed so far: started the rendered build in `test/run_1/showcase_scene_wasteland_run_01`, created scene `showcase_scene_wasteland_20260405_v1`, generated terrain `5e10e11e-62cb-4004-b353-b5d1ca145910`, and saved the first prefab `road_cracked_tile` as `c26d6753-9cea-4823-b85a-fe8c53f7bab2`. Remaining: let the queue continue until enough infrastructure exists for the first layout patch and screenshots, then continue through the rest of the asset set.

## Surprises & Discoveries

- Observation: the existing showcase builder is already durable and close to reusable. It persists both repo-local run artifacts and engine-side scene run steps, so a new scene profile can inherit the same interruption recovery semantics.
  Evidence: `tools/showcase_scene_builder.py` already writes `manifest.json`, uses `scene_sources/run_status`, and re-applies layout during generation.

- Observation: the current default GenFloor behavior tends toward a roughly `60m x 60m` terrain, which matches the new “small town” requirement better than the prior large-city scene.
  Evidence: `docs/genfloor/README.md` notes the default terrain size, and `src/genfloor/ai.rs` biases generated terrain toward `size_m` around `[60, 60]` unless the user requests otherwise.

- Observation: the prior run left a rendered `gravimera` process alive, which can confuse the next automated UI session if not stopped first.
  Evidence: `pgrep -fl "showcase_scene_builder|cargo run --release|gravimera"` returned `target/release/gravimera`.

- Observation: the builder could be generalized without touching the Automation HTTP API.
  Evidence: the new `wasteland_town` profile reuses the same scene-creation, GenFloor, Gen3D, scene-sources patch, camera, screenshot, and save endpoints that already supported the chrome scene.

- Observation: the required rendered smoke test still succeeds after the builder refactor, so the code changes did not introduce a startup regression in the game binary.
  Evidence: `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2` exited with code `0` on 2026-04-05 19:35 CST.

- Observation: the first road prefab generation is expensive but healthy; a malformed component was retried automatically and the prefab still completed successfully.
  Evidence: the live run log in `test/run_1/showcase_scene_wasteland_run_01/gravimera_stdout.log` shows `patch_asphalt_b` failing an axis-consistency conversion check, then succeeding on retry, followed by the saved prefab id `c26d6753-9cea-4823-b85a-fe8c53f7bab2`.

- Observation: the initial wasteland layout overused road-adjacent tiles, which made the town read as a raised tiled grid instead of two streets with lots.
  Evidence: the first live run loaded at least 175 objects and the visible result showed continuous shoulder slabs across much of the default-sized scene. After tightening the layout rules and resuming the same run, the live API reported `12` road tiles and `10` shoulder pads with `109` total objects before later asset waves refill the scene.

## Decision Log

- Decision: implement the wasteland town as a **new builder profile**, not a one-off fork.
  Rationale: the core runtime logic (starting the rendered game, durable scene patches, resumable Gen3D queue, screenshots, save) is already correct and should stay shared. The theme-specific behavior belongs in data and layout functions.
  Date/Author: 2026-04-05 / Codex.

- Decision: keep the scene compact and explicitly target the default terrain scale instead of reusing the large-city minimum terrain requirement from the chrome profile.
  Rationale: the user asked for a small town centered on two crossing streets. A tighter footprint improves readability, screenshot quality, and “everyday life” density.
  Date/Author: 2026-04-05 / Codex.

- Decision: satisfy “only use new created objects” by generating a fully fresh prefab set for this scene, then duplicate instances of those newly generated prefabs to achieve density.
  Rationale: the user wants the scene’s visible content to be original to this run/style, but duplication is still necessary for a believable town population.
  Date/Author: 2026-04-05 / Codex.

- Decision: prefer colorful survival accents over bleak desaturation.
  Rationale: the user explicitly wants the scene attractive at first glance. In a wasteland setting that means rust, dust, and salvaged materials as the base, then strong teal, orange, yellow, magenta, and lime accents on awnings, signs, lights, vehicles, and market fronts.
  Date/Author: 2026-04-05 / Codex.

- Decision: keep the chrome scene as the default builder profile and add `--profile wasteland_town` for the new variant.
  Rationale: this preserves the existing workflow and existing plan examples while still making the new scene explicit and reproducible.
  Date/Author: 2026-04-05 / Codex.

- Decision: in the wasteland profile, treat shoulder tiles as occasional pads and forecourts, not continuous street bands.
  Rationale: the generated shoulder prefab is visually thicker than a road marking and works better as a deliberate lot surface than as a full-scene strip. Reducing road rows to a single crossing line also makes the town layout more legible.
  Date/Author: 2026-04-05 / Codex.

## Outcomes & Retrospective

- The builder work is now in place and the real run is underway. The scene and terrain already exist under the user’s default Gravimera home, and the first prefab has been generated successfully. The remaining outcome is to let the queue continue until the town itself is visibly assembled, reviewed through screenshots, and, if necessary, iterated for better first-glance color and street-life density.

## Context and Orientation

The existing scene-generation path already works and should be preserved. The key builder is `tools/showcase_scene_builder.py`. That script starts the game in rendered mode with Automation enabled, discovers the random local API port from the log, creates or resumes a scene, queues Gen3D jobs, applies deterministic scene-source layers as durable run steps, captures screenshots, switches to Play mode briefly so built-in unit brains attach, saves the scene, and shuts the game down.

The important API reference is `docs/automation_http_api.md`. The builder relies on:

- `POST /v1/realm_scene/create` and `GET /v1/realm_scene/active` for creating and switching scenes.
- `POST /v1/genfloor/*` endpoints to generate terrain.
- `POST /v1/scene/terrain/select` to apply the generated terrain to the active scene.
- `POST /v1/gen3d/tasks/enqueue` and `GET /v1/gen3d/tasks/<task_id>` to generate prefab packages.
- `POST /v1/prefabs/reload_realm` and `GET /v1/prefabs` so durable scene patches only reference prefab ids that are actually loaded.
- `POST /v1/scene_sources/run_apply_patch` and `POST /v1/scene_sources/run_status` to build the scene incrementally with durable, resumable steps.
- `GET/POST /v1/camera` and `POST /v1/screenshot` to review the scene during generation.

The prior chrome showcase plan is stored at `plans/2026-04-05-showcase-scene-v1.md`. This new plan does not replace it. The chrome city remains one supported profile. The new work adds a second profile for a distinct visual target.

In this repository, a “profile” means a coherent bundle of scene-specific parameters:

- scene id prefix and label text,
- terrain prompt and target size,
- ordered asset-generation prompts,
- layout algorithm,
- tint palette,
- camera shot list.

That separation matters because the runtime mechanics are shared, but the artistic and spatial decisions are not.

## Plan of Work

First, refactor `tools/showcase_scene_builder.py` so it can choose a scene profile instead of hard-coding chrome-city assumptions. Keep the existing runtime helpers intact: HTTP client, process launch, scene-run patch application, screenshot capture, manifest persistence, and prefab reload recovery. The main refactor should introduce profile-specific functions or data structures for:

- scene naming and metadata,
- GenFloor prompt and minimum terrain expectation,
- asset prompt list in generation order,
- layout-layer assembly,
- curated screenshots.

Second, add a wasteland-town profile. The terrain prompt should describe a flat, compact settlement ground suitable for a dusty post-apocalyptic sci-fi town. The builder should not force a very large terrain; it should instead derive a modest layout extent from the actual selected terrain and target a strong central crossroads.

Third, implement the wasteland layout function. The layout should organize the scene into a readable small town around two crossing streets:

- central crossroads with cracked road tiles and faded crossing markings,
- one edge biased toward shops and market stalls,
- one edge biased toward garage and repair structures,
- one edge with stacked homes and small shops,
- one edge with utility structures such as clinic, recycler, water tower, drone pad, or animal pen,
- scattered props and vehicles to break repetition,
- walking units close to street-level activity,
- hovering drones and flying vehicles placed above the roads and rooftops.

The object count target should be at least one hundred placed instances. Achieve this by generating a sufficiently broad new prefab set, then duplicating those new prefabs in a deliberate arrangement instead of waiting for an unrealistic number of unique generations. The builder must continue applying layout patches while generation is still in progress so the UI visibly evolves over time.

Fourth, preserve the “first glance” requirement with a stronger color pass than the chrome scene. That means some building fronts, awnings, scrap signs, vehicle panels, and market assets should receive deterministic tints. The scene must still read as wasteland, so the tint system should complement rust, dust, concrete, and salvaged metal rather than replacing them.

Fifth, keep the interruption story intact. The run should still be resumable through `manifest.json` and durable engine-side scene run steps. Before the real run, stop any previous rendered `gravimera` process so the builder owns a clean UI session.

## Concrete Steps

Work from the repository root:

1. Update the builder and add the new ExecPlan:

       cd /Users/flow/workspace/github/gravimera

   Edit:

   - `tools/showcase_scene_builder.py`
   - `plans/2026-04-05-showcase-scene-wasteland-v1.md`

2. Run the required rendered smoke test in an isolated home:

       tmpdir=$(mktemp -d)
       GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

   Expected result: the game starts in rendered mode and exits cleanly after roughly two seconds without falling back to headless mode or crashing.

3. Commit the builder and plan changes:

       git add tools/showcase_scene_builder.py plans/2026-04-05-showcase-scene-wasteland-v1.md
       git commit -m "tools: add wasteland showcase builder profile"

4. Before the real build, stop any leftover rendered process:

       pgrep -fl "showcase_scene_builder|cargo run --release|gravimera"
       pkill -f "showcase_scene_builder|cargo run --release|gravimera"

   The second command is safe here because the build will immediately start a fresh controlled rendered session.

5. Start the real UI build against the default Gravimera home:

       python3 tools/showcase_scene_builder.py \
         --profile wasteland_town \
         --run-dir test/run_1/showcase_scene_wasteland_run_01

   Expected result: the tool launches `cargo run --release`, creates or resumes a versioned wasteland scene id, generates terrain and new prefab packages, applies layout in visible increments, captures screenshots in `test/run_1/showcase_scene_wasteland_run_01/shots/`, saves the scene, and shuts down cleanly.

6. If generation stalls because of network or AI-service issues, re-run the same command with the same `--run-dir`. The manifest and scene-run status should let the builder continue rather than restart from zero.

## Validation and Acceptance

The work is accepted when all of the following are true:

- Running the smoke-test command starts the rendered game and exits without crash.
- A fresh scene exists under `~/.gravimera/realm/default/scenes/showcase_scene_wasteland_*/`.
- The scene terrain is default-scale or near-default-scale and the town reads as a compact layout centered on two crossing streets.
- The visible content uses only newly generated terrain and prefab assets for this scene profile, with no fallback to previously generated chrome-scene prefabs.
- The final scene contains at least one hundred placed objects total across roads, buildings, props, vehicles, animals, drones, robots, and civilians.
- Screenshots under `test/run_1/showcase_scene_wasteland_run_01/shots/` show both incremental progress and final curated viewpoints.
- Switching briefly into Play mode does not crash and allows the engine’s existing brain-attachment behavior to run for the generated units.
- The saved scene remains available under the user’s default `~/.gravimera` path after the builder exits.

## Idempotence and Recovery

The profile refactor must remain safe to re-run. Reusing the same `--run-dir` should preserve:

- the manifest’s chosen `scene_id`,
- the persistent `scene_run_id`,
- already generated prefab ids,
- already captured screenshot count.

The durable scene-run path under `~/.gravimera/realm/default/scenes/<scene_id>/runs/<run_id>/steps/` must remain the source of truth for applied scene-source patches. If the UI process dies mid-run, restart the builder with the same `--run-dir`; it should query `run_status`, continue from `next_step`, and reuse successful prefabs instead of emitting conflicting patches.

Stopping old `gravimera` processes before the real build is also part of recovery. That avoids multiple rendered automation sessions competing for GPU resources, UI focus, or stale state.

## Artifacts and Notes

Important artifact locations for this run:

- Repo-local run directory: `test/run_1/showcase_scene_wasteland_run_01/`
- Progress screenshots: `test/run_1/showcase_scene_wasteland_run_01/shots/`
- User-visible saved scene: `~/.gravimera/realm/default/scenes/showcase_scene_wasteland_<date>_vN/`

The builder should continue writing a JSON manifest in the run directory. That manifest is the resume journal for the external tool; the engine-side scene-run steps are the durable journal for scene-source mutations.

## Interfaces and Dependencies

`tools/showcase_scene_builder.py` must continue to provide these existing shared behaviors:

- start the rendered game with `cargo run --release`,
- discover the Automation base URL from the log,
- wait for `/v1/health`,
- create or switch scenes,
- select terrain,
- enqueue and poll Gen3D tasks,
- reload realm prefabs,
- apply scene-source patch steps with a stable `run_id`,
- control the camera and capture screenshots,
- save the scene and shut the game down.

At the end of this change, the builder should also expose a stable CLI profile selector, for example:

    python3 tools/showcase_scene_builder.py --profile wasteland_town --run-dir test/run_1/showcase_scene_wasteland_run_01

That interface is important because future scene styles should be additive rather than ad hoc forks.

Revision note: created this new ExecPlan on 2026-04-05 to capture the pivot from the already implemented chrome-city showcase to a second, compact wasteland-town showcase profile requested by the user. The plan is separate so both scene variants remain reproducible.
Revision note: updated on 2026-04-05 after implementing the new builder profile and running the required rendered smoke test, so the plan reflects the current code and the remaining step is the real scene-generation run.
Revision note: updated again on 2026-04-05 after starting the real rendered build, so the plan records the live scene id, terrain id, first generated prefab, and the fact that the queue is still running.
Revision note: updated again on 2026-04-05 after tightening the wasteland street layout, rerunning the rendered smoke test, and resuming the live build so the plan captures the layout correction and its API-visible effect.
