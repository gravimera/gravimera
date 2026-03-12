# Gen3D: Reference-Image Summarizer + Input Limits (Text-Only Agent Prompts)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D quality and latency degrade when we send many large reference photos to the LLM repeatedly (plan + component generations + reviews). After this change, Gen3D performs a single AI “vision” pre-processing step that summarizes up to three user reference images into a short, structured text description of the main object. The Gen3D agent and all Gen3D LLM tools then operate on text only (user prompt + that summary) and never attach the raw user images to model requests.

In addition, the Gen3D UI and API enforce strict input limits:

- Images: at most 3, and each must be smaller than 5 MiB. Over-limit images are refused with a clear tip.
- Prompt textbox: at most 250 whitespace-separated words and at most 2000 Unicode characters. Over-limit input is refused with a clear tip.

You can see this working by starting Gen3D, dropping images, typing near the limits, clicking Build, and then inspecting the run cache artifacts (especially `attempt_0/inputs/image_object_summary.txt`) and per-request logs showing that LLM calls no longer include user images.

## Progress

- [x] (2026-03-13 05:26Z) Write this ExecPlan.
- [x] (2026-03-13 06:46Z) Add user-input gates: max 3 images (<5 MiB each), max 250 words and max 2000 chars prompt (UI + API/server-side).
- [x] (2026-03-13 06:46Z) Implement pre-agent “reference image summarizer” request and persist `attempt_0/inputs/image_object_summary.txt`.
- [x] (2026-03-13 06:46Z) Update Gen3D prompt builders and LLM tools to use the summary text and to never attach user images to LLM requests.
- [x] (2026-03-13 06:46Z) Update documentation (`gen_3d.md`) to match behavior and limits (keep `README.md` clean).
- [x] (2026-03-13 06:48Z) Validation: `cargo test` and rendered smoke test (`--rendered-seconds 2`); commit.

## Surprises & Discoveries

- Discovery: The review-delta tool “schema repair” path can also attach images, so we must validate preview image paths there too (not just in the primary review-delta call).
  - Outcome: Added a shared `validate_review_images_for_llm` gate that enforces “preview renders only” and blocks user reference photos and non-run-dir paths.

## Decision Log

- Decision: Cap user reference images at 3 and require each to be smaller than 5 MiB.
  Rationale: This prevents large/slow image payloads and keeps the vision pre-processing request bounded.
  Date/Author: 2026-03-13 / Codex

- Decision: Cap user prompt input at 250 “whitespace-separated words” plus a hard 2000-character limit.
  Rationale: Word limit matches UX expectations; character cap closes the “no spaces” bypass and keeps latency predictable without needing tokenizer-specific caps.
  Date/Author: 2026-03-13 / Codex

- Decision: Use a structured bullet summary with a hard cap of 160 words for the reference-image summarizer output.
  Rationale: 250 words tends to include background trivia/speculation; ~120–160 words preserves the modeling-relevant signal while reducing contradictions and prompt bloat.
  Date/Author: 2026-03-13 / Codex

- Decision: Do not add a user “review/edit the summary” step.
  Rationale: The desired UX is fully automatic; quality is protected by strict “no guessing” instructions + bounded output + user prompt wins on conflicts.
  Date/Author: 2026-03-13 / Codex

## Outcomes & Retrospective

- Outcome: Reference photos are summarized once (bounded, structured text), and downstream Gen3D LLM calls run text-only for improved speed and fewer contradictions.
- Outcome: UI + server-side prompt limits and image limits prevent pathological slow requests and keep behavior predictable.
- Outcome: Tool contracts were tightened (`get_user_inputs_v2`, preview-image validation) to prevent accidental re-sending of user photos.

## Context and Orientation

Gen3D is an in-game workshop mode that generates object prefabs from primitives and saves them into scene data. The Gen3D implementation is primarily in `src/gen3d/`.

Relevant files and responsibilities:

- `src/gen3d/mod.rs`: Gen3D constants (max images, UI sizing) and public re-exports.
- `src/gen3d/images.rs`: Drag & drop reference image handling and thumbnail UI.
- `src/gen3d/ui.rs`: Prompt textbox input and Gen3D workshop UI rendering.
- `src/gen3d/ai/orchestration.rs`: Starts a Build run and initializes `Gen3dAiJob` run state and cache layout.
- `src/gen3d/ai/agent_loop/mod.rs`: Polls agent phases and spawns the agent-step LLM request.
- `src/gen3d/ai/agent_tool_dispatch.rs`: Implements agent-facing tools, including LLM-backed tools (`llm_generate_plan_v1`, `llm_generate_component_v1`, `llm_review_delta_v1`).
- `src/gen3d/ai/prompts.rs`: Builds the plan/component/review prompt text (this is where we must inject the image summary).
- `src/gen3d/agent/tools.rs`: Tool registry and tool descriptors shown to the agent in its prompt.
- `gen_3d.md`: Current implementation docs (must be updated from “0–6 photos” to new behavior + limits).

Terminology:

- “Reference images”: the user-provided images dropped into the Gen3D prompt area.
- “Image summary”: the short, structured text output produced by the pre-processing step. It is the only representation of reference images that downstream Gen3D LLM calls see.

## Plan of Work

First, implement hard input gates in the UI and in the build start path so limits are enforced regardless of how the prompt/images are set (mouse drop vs HTTP automation API). This includes:

1) Update `GEN3D_MAX_IMAGES` to 3 and introduce a byte limit constant for image files (5 MiB).
2) Update drag & drop (`src/gen3d/images.rs`) to refuse files that exceed count or size, with a clear `workshop.error` tip.
3) Update prompt input (`src/gen3d/ui.rs`) to refuse insertions that would exceed either 250 whitespace-separated words or 2000 characters. Ensure paste/IME commits are bounded the same way.
4) Add server-side validation in `gen3d_start_build_from_api` (and automation endpoints that set prompt) so limits cannot be bypassed.

Second, implement a new pre-agent phase in the Gen3D agent loop to summarize reference images once per run:

1) Add a new `Gen3dAiPhase` variant for “summarize reference images”.
2) When a run starts with reference images, enter that phase before `AgentWaitingStep`.
3) Spawn a single LLM request with the reference images attached and strict instructions:
   - describe only the clearly visible main object,
   - structured bullets,
   - no guessing (use “Unknowns”),
   - hard cap 160 words.
4) Persist the final summary into `attempt_0/inputs/image_object_summary.txt` (and optionally a small JSON metadata artifact).
5) Store the summary on `Gen3dAiJob` so it can be included in all later prompt builders/tool outputs.

Third, remove user images from all subsequent LLM requests and update prompts to use the summary:

1) Update `src/gen3d/ai/prompts.rs` so `build_gen3d_effective_user_prompt` accepts an optional `image_object_summary` and includes it in a dedicated section. Remove any instruction that assumes the LLM can “look at photos”.
2) Update all LLM-backed tool dispatch code to pass `Vec::new()` for user images (plan/component/batch/review), and instead include the image summary text in `user_text`.
3) Update `get_user_inputs` tool (bump to `get_user_inputs_v2`) to return `{prompt, image_object_summary, reference_images_count}` and not return the image paths (to discourage agents from trying to use them).
4) Add a tool-side validation in `llm_review_delta_v1` to reject user-provided `preview_images` that point at `inputs/images/` (user reference photos), if that argument path exists today.

Finally, update documentation:

- `gen_3d.md` must reflect:
  - 0–3 reference images, each <5 MiB,
  - prompt limits (250 words + 2000 chars),
  - reference images are summarized once; user images are not attached to downstream Gen3D LLM calls,
  - new cache artifact: `attempt_0/inputs/image_object_summary.txt`.

## Concrete Steps

All commands below are run from the repository root.

1) Implement code changes and update docs.

2) Run unit tests:

    cargo test

3) Run the required rendered smoke test (do NOT use `--headless`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4) Manual acceptance checks in Gen3D (in the rendered UI):

   - Drag/drop 4 images: the 4th is refused and an error tip is visible.
   - Drag/drop an image >= 5 MiB: it is refused with an error tip.
   - Paste/type beyond 250 words or 2000 chars: extra input is refused with an error tip.
   - Click Build with images-only: run proceeds and writes `attempt_0/inputs/image_object_summary.txt`.
   - Inspect `attempt_0/pass_*/gravimera.log` and confirm subsequent LLM requests (plan/component tools) show `images=0` after the pre-processing step.

5) Commit with a clear message once validation passes.

## Validation and Acceptance

Acceptance is met when:

- The rendered smoke test starts and exits without crashing.
- UI + server-side enforcement blocks over-limit images and prompt text.
- A Gen3D run with reference images produces `attempt_0/inputs/image_object_summary.txt`.
- After that, Gen3D LLM-backed tools run without attaching user images to requests (only text prompt + image summary are used).
- Documentation is updated to match the new behavior and limits.

## Idempotence and Recovery

- Re-running the smoke test is safe; it uses a fresh temp `GRAVIMERA_HOME`.
- If a Gen3D run fails mid-way, the cache under `~/.gravimera/cache/gen3d_cache/<run_id>/` remains for inspection; re-run Build to create a new run dir.
- If the image summarizer step fails, the run should stop with an actionable error message suggesting to retry or use a text prompt without images.

## Artifacts and Notes

Expected cache artifacts for a run with reference images (under `gen3d_cache/<run_id>/attempt_0/`):

- `inputs/user_prompt.txt`
- `inputs/images/*` (cached copies of the reference images; still used for UI preview, not sent to downstream LLM calls)
- `inputs/image_object_summary.txt` (new; the only image-derived text sent to Gen3D LLM calls)
- `inputs_manifest.json` (may be extended to include `image_object_summary.txt`)
