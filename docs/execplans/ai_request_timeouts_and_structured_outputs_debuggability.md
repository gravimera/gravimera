# AI request timeouts + Gen3D structured outputs debuggability

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, Gravimera’s AI-backed features (Gen3D and Scene Build AI) will be more reliable and easier to debug:

- A developer can configure a single **AI request timeout** (default **240s / 4 minutes**) in `config.toml` to control how long we wait for an AI backend to start responding.
- Gen3D component prompts will explicitly describe the `collider` contract, preventing repeated parse/retry churn caused by `collider.kind` synonyms like `box`/`cuboid`.
- When structured outputs are invalid and Gen3D retries, each attempt will persist its own request/response artifacts so the “bad” response is inspectable. Errors caused by truncated/incomplete JSON will be reported more clearly.

You can see it working by:

1. Adding `[ai] request_timeout_secs = 240` to your config (or relying on the default).
2. Running a Gen3D build and confirming the run dir includes per-attempt `*_retry_*` raw artifacts when retries occur.
3. (Optional) Intentionally forcing a structured-output failure (using existing mock hooks in debug/test) and confirming the invalid attempt artifacts are preserved.

## Progress

- [x] (2026-03-27) Add `AppConfig.ai_request_timeout_secs` with default 240s; parse `[ai].request_timeout_secs` and update `config.example.toml` + docs.
- [x] (2026-03-27) Thread the timeout value through Gen3D AI request plumbing and apply it as the curl “first-byte” timeout across OpenAI/MiMo/Gemini/Claude.
- [x] (2026-03-27) Apply the timeout to Scene Build AI curl calls.
- [x] (2026-03-27) Fix Gen3D component prompt ↔ schema mismatch by documenting `collider` in component system instructions, and add targeted schema repair hints for collider kind.
- [x] (2026-03-27) Preserve structured-output retry artifacts per attempt and write `*_structured_outputs_invalid.txt` on failures; improve EOF/truncation error messages.
- [x] (2026-03-27) Run formatting + tests as appropriate, then run the required rendered smoke test.
- [x] (2026-03-27) Commit with a clear message.

## Surprises & Discoveries

- `apply_llm_generate_plan_ops_v1` was building JSON tool results for rejection paths but returning `Err(...)`, which broke unit tests expecting a JSON tool result. Fixed by returning the JSON result for deterministic rejection paths (scope/preserve/rejected ops).

## Decision Log

- Decision: Treat the new `ai.request_timeout_secs` as the **“first response byte” timeout** for streaming AI requests (Gen3D) rather than a hard cap on total request duration.
  Rationale: Real Gen3D requests can legitimately take > 4 minutes end-to-end (seen in logs), but the problem to solve is “backend never starts streaming within 120s”.
  Date/Author: 2026-03-27 / codex

## Outcomes & Retrospective

- Added a global `config.toml` knob: `[ai].request_timeout_secs` (default 240s).
- Gen3D now uses this value as the curl “first-byte timeout” for streaming requests across all supported providers.
- Scene Build AI now uses this value as the curl `--max-time`.
- Structured-output retries now preserve per-attempt artifacts and persist `*_structured_outputs_invalid.txt` for failed attempts.

## Context and Orientation

Key files/modules involved:

- `src/config.rs`: `AppConfig` definition + config.toml parsing.
- `config.example.toml`: documented config surface for developers.
- `docs/gen3d/README.md`: Gen3D workflow + config notes.
- `src/gen3d/ai/ai_service.rs`: structured outputs enforcement + retries.
- `src/gen3d/ai/openai.rs`, `mimo.rs`, `gemini.rs`, `claude.rs`: curl request implementations and stream parsing.
- `src/gen3d/ai/prompts.rs`: Gen3D prompt text (system + user).
- `src/gen3d/ai/repair_hints.rs`: schema repair hint strings appended on retry.
- `src/scene_build_ai.rs`: “Scene Build AI” planning calls to OpenAI.

Terminology:

- “First-byte timeout”: maximum wall time to wait for the AI backend to send the first response body bytes. This is distinct from:
  - connect timeout (TCP/TLS establishment)
  - idle timeout (no new bytes after the stream started)
  - hard timeout (absolute max total request duration)

## Plan of Work

1. Extend `AppConfig` with an AI request timeout seconds field and parse it from `config.toml`.
2. Plumb the value into Gen3D’s `spawn_gen3d_ai_text_thread` → `generate_text_via_ai_service` path and into each AI backend implementation so curl first-byte timeouts use it.
3. Apply the same timeout to Scene Build AI curl `--max-time`.
4. Update Gen3D component system instructions to explicitly include `collider` (nullable) and the allowed `collider.kind` enum values.
5. Improve structured outputs retry debuggability:
   - Make retries write distinct artifacts (prefix includes retry attempt).
   - When structured outputs validation fails, write the offending text to `*_structured_outputs_invalid.txt`.
   - If the parse error is an EOF-style JSON error, include “possible truncated stream” in the error message.
6. Update docs and run validation commands.

## Concrete Steps

All commands run from the repo root.

- Implement code changes.
- Run:

    cargo fmt
    cargo test

- Run the required smoke test (rendered; not headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Validation and Acceptance

- Config: `config.example.toml` documents `[ai].request_timeout_secs` and it is parsed into `AppConfig`.
- Gen3D: on structured-output retries, multiple raw artifacts exist (attempt-specific) rather than being overwritten.
- Gen3D: component prompt no longer encourages invalid `collider.kind` values; schema repair hints mention the exact allowed values.
- Scene Build AI: curl calls use the configured timeout.
- The rendered smoke test launches and exits without crash.

## Idempotence and Recovery

- The new config key is optional; defaults apply when absent.
- If a user sets an invalid timeout (<= 0), config parsing should emit a clear error and fall back to the default.

## Artifacts and Notes

- Keep retry attempt artifacts in the run dir using distinct filenames (e.g. `*_retry_2_*`).
