# ExecPlan: Fix OpenAI `/responses` SSE Extraction + JSON5-Lenient Structured Parsing

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D uses “structured outputs” (strict JSON schema) for agent steps, plans, component drafts, and review deltas. When the configured OpenAI-compatible backend at `base_url=https://right.codes/codex/v1` returns Server-Sent Events (SSE) even though we request `stream=false`, our current SSE text reconstruction can duplicate the model output (appending both deltas and the final done text). This produces text that looks like:

    { ...valid JSON object... }{ ...the same JSON object again... }

This triggers warnings like:

    Gen3D: backend did not enforce structured outputs; continuing best-effort… err=trailing characters…

and in some cases it hard-fails the build with:

    backend did not enforce structured outputs (multiple JSON objects detected)

Separately, the model sometimes emits “almost JSON” (for example JSON5-style trailing commas, or stray escaping like `\"` outside of strings). In strict mode we currently fail early while validating structured outputs, even when the intended JSON object is recoverable.

After this change:

- Gen3D and SceneBuild AI will extract output text from `/responses` by selecting the final assistant `message` item (not by concatenating every `output_text` chunk across items, and not by duplicating `delta` + `done`).
- Gen3D’s structured-output validation will accept JSON5-ish outputs and apply a minimal, safe repair for a common LLM escaping mistake, then re-serialize to canonical JSON before downstream parsing.

User-visible success is: Gen3D runs no longer spam “structured outputs violation” warnings due to duplicated SSE text, and builds are more robust to minor JSON syntax deviations without silently accepting multiple-step “simulations”.

## Progress

- [x] (2026-03-09) Create this ExecPlan.
- [x] (2026-03-09) Fix OpenAI `/responses` output text extraction to use the final assistant message only (JSON + SSE paths).
- [x] (2026-03-09) Add JSON5-lenient structured-output parsing in Gen3D (canonicalize to strict JSON on success).
- [x] (2026-03-09) Add regression tests covering (1) SSE duplication, (2) multiple message outputs, (3) JSON5/repair parsing.
- [x] (2026-03-09) Run `cargo test` and the rendered smoke start command from `AGENTS.md`.
- [ ] Commit changes with a clear message.

## Surprises & Discoveries

- Observation: `https://right.codes/codex/v1/responses` returns SSE payloads even when the request sets `"stream": false`.
  Evidence: cached Gen3D artifacts like `~/.gravimera/cache/gen3d/<run_id>/attempt_0/pass_<n>/agent_step_responses_raw.txt` are `event:`/`data:` streams.

- Observation: Our SSE reconstructor currently appends both `response.output_text.delta` chunks and the final `response.output_text.done.text`, duplicating the full output when `done.text` contains the entire final message.
  Evidence: in the SSE tail, `response.output_text.done.text` equals the concatenation of all prior deltas for the same `item_id`.

- Observation: JSON5 parsing alone does not fix the common LLM mistake of outputting `\"` outside string literals (this is invalid in both JSON and JSON5).
  Evidence: cached outputs can contain `...,"args":{"query":"speed",\"max_matches\":50}}...` which fails both parsers unless repaired.

## Decision Log

- Decision: Prefer “final assistant message item” extraction for `/responses` output.
  Rationale: OpenAI Responses can contain multiple output items (reasoning, tool-ish blobs, multiple assistant messages). Gen3D structured outputs expect exactly one JSON object, and the safest interpretation is “use the last assistant message text”.
  Date/Author: 2026-03-09 / Codex

- Decision: Add JSON5-lenient parsing as a fallback during structured-output validation, and canonicalize to strict JSON upon success.
  Rationale: Structured outputs are the contract, but some backends/models drift into JSON5-ish syntax. Accepting these with a clear warning is more robust than failing the entire run, while still keeping downstream parsing strict (via re-serialization).
  Date/Author: 2026-03-09 / Codex

- Decision: Add a minimal repair for `\"` appearing outside of string literals (invalid JSON), before giving up.
  Rationale: This is a common LLM “over-escaping” mistake. Removing backslashes that appear outside strings before a quote is unambiguous (JSON does not allow `\` outside strings) and can salvage otherwise-correct structured outputs.
  Date/Author: 2026-03-09 / Codex

## Outcomes & Retrospective

- Updated `/responses` text extraction to always select the final assistant message item, preventing “multiple JSON objects detected” failures caused by concatenation.
- Updated SSE extraction to prefer parsing the final SSE `response` object (via `response.completed`) instead of reconstructing text by concatenating deltas and done events.
- Added JSON5-lenient structured-output parsing + a minimal repair for `\"` outside strings, then canonicalized successful parses back to strict JSON for downstream processing.
- Added unit tests for both `/responses` extraction and lenient parsing, ran `cargo test`, and verified the rendered smoke start works.

## Context and Orientation

Key modules involved:

- `src/openai_shared.rs`: shared helpers for OpenAI-compatible endpoints, including:
  - `extract_openai_responses_output_text(json)`: extracts text from a `/responses` JSON envelope.
  - `extract_openai_responses_sse_output_text(body)`: best-effort extraction when the backend returns SSE instead of a single JSON object.
- `src/gen3d/ai/openai.rs`: Gen3D OpenAI backend implementation for `/responses` and `/chat/completions`.
- `src/scene_build_ai.rs`: SceneBuild AI uses the same extraction helpers.
- `src/gen3d/ai/ai_service.rs`: enforces structured-output expectations and logs “backend did not enforce structured outputs …” warnings/errors.

Terminology:

- “SSE” (Server-Sent Events): a text stream where each event is separated into lines like `event: ...` and `data: {...json...}`.
- “Responses JSON envelope”: a JSON object returned by `/responses` containing `output: [...]` items. We care about the last `output` item where `type="message"` and `role="assistant"`.
- “Structured outputs”: requests where Gen3D supplies a JSON schema (`text.format` or `response_format`) and expects a single JSON object response.

## Plan of Work

1) Fix `/responses` output extraction.

In `src/openai_shared.rs`:

- Update `extract_openai_responses_output_text` to return the text from the LAST assistant message item (instead of concatenating text across all output items).
- Update `extract_openai_responses_sse_output_text` to prefer parsing the final SSE `response` object (from `response.completed` / `{ "response": ... }` events), then apply the same “last assistant message” extraction.
- Keep a best-effort fallback that reconstructs text from SSE deltas, but do not duplicate by appending both deltas and the final done text.

2) Add JSON5-lenient parsing for Gen3D structured outputs.

In `src/gen3d/ai/ai_service.rs`, inside the `require_structured_outputs` validation block:

- Attempt strict `serde_json` parsing first.
- If strict parsing fails, attempt `json5` parsing.
- If still failing, attempt a minimal repair pass that removes backslashes before quotes when those backslashes occur outside string literals; then retry strict and JSON5 parsing.
- If lenient parsing succeeds, re-serialize the `Value` back into canonical strict JSON and replace `resp.text` with that canonical JSON so downstream parsing remains strict.
- Keep existing “best-effort coercion” behavior for multiple JSON objects, but ensure coerced outputs are canonical JSON (not JSON5-ish).

3) Tests.

- Add unit tests for `src/openai_shared.rs` that cover:
  - SSE that contains both deltas and a final done-text should return the text once (no duplication).
  - A `/responses` JSON envelope with multiple assistant messages should return only the last one.
- Add unit tests for `src/gen3d/ai/ai_service.rs` or a small helper module that cover:
  - JSON5 parsing of a trailing-comma object succeeds and canonicalizes.
  - The `\"key\"` outside-string repair enables parsing and canonicalization.

4) Validation and commit.

- Run `cargo test`.
- Run the rendered smoke start (per `AGENTS.md`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

- Commit with a message like: “Fix OpenAI SSE structured outputs + JSON5 lenient parsing”.

## Concrete Steps

All commands run from the repo root:

1) `cargo fmt`
2) `cargo test`
3) Rendered smoke start:

   tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Validation and Acceptance

Acceptance is satisfied when:

- A cached SSE response that previously triggered `structured_outputs_violation … trailing characters` now yields a single JSON object string (no duplication) and no longer triggers “multiple JSON objects detected”.
- Gen3D structured-output validation accepts JSON5-ish outputs (e.g., trailing commas) by canonicalizing to strict JSON, with a warning logged that the backend did not enforce strict JSON.
- `cargo test` passes.
- The app starts successfully with the rendered smoke start command.

## Idempotence and Recovery

- The changes are additive and safe to rerun.
- If JSON5 parsing introduces an unexpected regression, it can be gated to “structured outputs required + schema present” only, and warnings can be used to diagnose backend behavior.
