# ExecPlan: Gen3D API-Level Structured Outputs (Strict JSON Schema)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D relies on LLMs returning strict JSON for multiple steps (plan, component drafts, and review deltas). Today, even when the model’s intent is correct, runs can waste time and tokens because the output is valid JSON but does not match the strict schema (for example, a single wrong key like `offset_pos_join` causes a parser failure and triggers a repair loop).

After this change, Gen3D will request API-level structured outputs (strict JSON Schema) when supported by the provider, so the model is constrained to emit schema-valid JSON in the first place. This is user-visible as fewer “repair tool output” loops and faster, more reliable builds.

The implementation must be provider-safe: if the upstream endpoint does not support structured outputs, Gen3D must detect that deterministically, disable the feature for the current session, and fall back to the existing “free-form JSON + local parsing/repair” behavior.

## Progress

- [x] (2026-02-10) Write and check in this ExecPlan.
- [x] (2026-02-10) Define strict JSON Schemas for Gen3D outputs (plan / plan fill / component draft / review delta) compatible with OpenAI Structured Outputs constraints.
- [x] (2026-02-10) Thread an “expected schema kind” through all Gen3D LLM calls that expect strict JSON.
- [x] (2026-02-10) Request structured outputs on `/responses` (via `text.format`) and `/chat/completions` (via `response_format`) with graceful fallback when unsupported.
- [x] (2026-02-10) Add unit tests to ensure schemas are wired into request JSON and that rejection detection works.
- [x] (2026-02-10) Run `cargo fmt`, `cargo test`, and a headless smoke start (`cargo run -- --headless --headless-seconds 3`).
- [x] (2026-02-10) Commit changes.

## Surprises & Discoveries

- Observation: Some “near-miss” schema errors are caused by prompt labels that resemble JSON keys (e.g. `offset_pos_join=`).
  Evidence: Cached run `~/.gravimera/cache/gen3d/26ae66e4-0c57-40d3-920c-091e3ccfc110/attempt_0/pass_2/` has `llm_review_delta_v1` output using `offset_pos_join` as a key and triggering a tool-schema repair.

- Observation: OpenAI Structured Outputs has additional schema constraints that affect existing Gen3D JSON shapes.
  Evidence: The schema must use `additionalProperties: false` for objects and treat “optional fields” as `anyOf: [<type>, null]` (which affects maps like `animations`).

## Decision Log

- Decision: Implement structured outputs as a best-effort provider capability, remembered per Gen3D session.
  Rationale: Some OpenAI-compatible gateways do not implement structured outputs. Hard-failing would regress reliability; session-level capability detection allows stable fallback.
  Date/Author: 2026-02-10 / Codex

- Decision: Use strict JSON Schema (not just “valid JSON object”).
  Rationale: Many failures are “valid JSON, wrong keys”. Schema-level constraints prevent unknown keys in nested objects (e.g. transform deltas).
  Date/Author: 2026-02-10 / Codex

- Decision: For `/responses`, request structured outputs using `text.format`; for `/chat/completions`, use `response_format`.
  Rationale: These are the provider’s supported request shapes for Structured Outputs; each endpoint has its own JSON field.
  Date/Author: 2026-02-10 / Codex

- Decision: Represent per-attachment `animations` as a fixed set of known channels with nullable values in structured-output schema, and change Rust parsing types to allow `null` per channel.
  Rationale: Structured Outputs forbids free-form maps via `additionalProperties`, so we need a schema that enumerates allowed channel keys. Allowing `null` avoids forcing identity animations for unused channels.
  Date/Author: 2026-02-10 / Codex

## Outcomes & Retrospective

Structured Outputs is now requested for plan/plan-fill/component-draft/review-delta LLM calls. Unsupported providers are detected via error inspection and automatically fall back to legacy free-form JSON parsing for the remainder of the session. Unit tests cover request JSON wiring and the “structured outputs rejected” detector. `cargo test` and a headless smoke start passed.

## Context and Orientation

Gen3D AI calls are executed via `spawn_gen3d_ai_text_thread` (in `src/gen3d/ai/mod.rs`) which delegates to `openai::generate_text_via_openai` (in `src/gen3d/ai/openai.rs`).

Relevant strict JSON payloads:

- Plan: `AiPlanJsonV1` (`src/gen3d/ai/schema.rs`).
- Plan fill: `AiPlanFillJsonV1` (`src/gen3d/ai/schema.rs`).
- Component draft: `AiDraftJsonV1` (`src/gen3d/ai/schema.rs`).
- Review delta: `AiReviewDeltaJsonV1` (`src/gen3d/ai/schema.rs`).

Local parsing is strict (many structs use `deny_unknown_fields`), and the agent loop uses a “tool schema repair” mechanism when parsing fails. Structured outputs aims to eliminate these failures before they reach local parsing.

## Plan of Work

1) Define schema kinds and JSON Schemas.

Create a small module (under `src/gen3d/ai/`) that defines:

- an enum of “expected output schema kinds” (plan / plan fill / component draft / review delta),
- a function that returns `(name, schema_json)` for each kind.

The schemas should be compact and focus on:

- correct field names,
- correct types,
- `additionalProperties: false` on objects where unknown keys would cause local parsing failure.

2) Thread schema kind through all LLM calls.

Update `spawn_gen3d_ai_text_thread` to accept an “expected schema kind” (or “no schema”), and pass it from every call site that expects strict JSON.

3) Add structured output request fields.

In `src/gen3d/ai/openai.rs`:

- For `/responses`, include a `text.format` object of the form:
  - `{"type":"json_schema","name":"...","schema":{...},"strict":true}`
- For `/chat/completions`, include a `response_format` object of the form:
  - `{"type":"json_schema","json_schema":{"name":"...","schema":{...},"strict":true}}`

4) Provider-safe fallback.

If a structured-output request fails with a deterministic “unsupported parameter / unknown field” error, record that in `Gen3dAiSessionState` and retry the request without structured outputs for the remainder of the session.

5) Tests and validation.

Add unit tests that:

- exercise the request-building functions to confirm the schema field is included when requested,
- confirm the “unsupported structured outputs” error detection flips the session flag and triggers a retry path.

Then run `cargo test` and a headless smoke start.

## Concrete Steps

All commands run from the repo root:

1) `cargo fmt`
2) `cargo test`
3) Smoke: `cargo run -- --headless --headless-seconds 3`

## Validation and Acceptance

Acceptance is satisfied when:

- `cargo test` passes.
- Headless smoke start exits without crashing.
- A cached run that previously triggered a tool-schema repair due to wrong keys no longer needs repair when the provider supports structured outputs (or else it deterministically falls back to the old behavior).

## Idempotence and Recovery

All changes are safe to repeat.

If a provider rejects the structured-output request shape, the session capability flag must disable the feature and keep Gen3D functioning via legacy parsing/repair.

## Artifacts and Notes

When doing a real run, record:

- the `run_id` directory under `~/.gravimera/cache/gen3d/`,
- the request JSON artifacts (`*_responses_request.json` / `*_chat_request.json`),
- whether tool-schema repair events occurred in `agent_trace.jsonl`.
