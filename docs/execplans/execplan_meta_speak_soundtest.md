# Meta Panel Speak with Soundtest Adapter Isolation

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document follows `PLANS.md` from the repository root and is maintained in accordance with its requirements.

## Purpose / Big Picture

After this change, a player can open the Meta panel and use a new **Speak** section to choose a voice (`dog`, `cow`, `dragon`), type text in `content`, and click `Speak` to hear synthesized speech. The implementation uses a backend adapter trait so the UI is not tightly coupled to `soundtest` internals.

## Progress

- [x] (2026-03-09 15:55 +08:00) Added `soundtest` dependency and introduced `src/meta_speak.rs` with `MetaSpeakAdapter` trait, `SoundtestMetaSpeakAdapter`, and `MetaSpeakRuntime` resource.
- [x] (2026-03-09 16:08 +08:00) Extended `src/motion_ui.rs` with Speak UI state, voice selector buttons, content field, speak button, worker-thread execution, status/error rendering, and keyboard capture logic.
- [x] (2026-03-09 16:12 +08:00) Wired new Speak systems into `src/app_plugins.rs` in both `PreUpdate` (text input) and `Update` (button interactions/styles).
- [x] (2026-03-09 16:15 +08:00) Added tests in `tests/meta_speak_adapter.rs` for voice contract and backend selection logic.
- [x] (2026-03-09 16:18 +08:00) Updated docs: `docs/controls.md`, new `docs/meta_speak.md`, and README docs index link.
- [x] (2026-03-09 15:54 +08:00) Ran validation commands: `cargo test --test meta_speak_adapter` (3 passed) and rendered smoke (`tmpdir=$(mktemp -d); GRAVIMERA_HOME=\"$tmpdir/.gravimera\" cargo run -- --rendered-seconds 2`) passed.
- [x] (2026-03-09 15:58 +08:00) Committed implementation as `0bb8633` with message `Add Meta panel Speak with soundtest adapter isolation`.

## Surprises & Discoveries

- Observation: Cargo initially resolved `ort` to `2.0.0-rc.12`, which failed in this environment via `soundtest` transitive build.
  Evidence: `error[E0609]: no field SessionOptionsAppendExecutionProvider_VitisAI on type &'static OrtApi` during `cargo check`.

## Decision Log

- Decision: Implement **B + adapter isolation** (`MetaSpeakAdapter`) instead of direct UI-to-library calls.
  Rationale: Keep UI stable and backend-swappable while still using in-process `soundtest` integration.
  Date/Author: 2026-03-09 / Codex

- Decision: Pin `ort` to `=2.0.0-rc.11` in `gravimera/Cargo.toml`.
  Rationale: Avoid resolution to `rc.12` that fails to compile with current `soundtest` dependency graph.
  Date/Author: 2026-03-09 / Codex

## Outcomes & Retrospective

The feature implementation is complete in code, tests, docs, and commit history. Validation passed (new contract tests + required rendered smoke).

## Context and Orientation

Relevant files:

- `src/meta_speak.rs`: Adapter contract and `soundtest` implementation.
- `src/motion_ui.rs`: Meta panel UI/state, Speak controls, text input handling.
- `src/app_plugins.rs`: System scheduling for Speak input/click/style updates.
- `src/app.rs`: Resource initialization for `MetaSpeakRuntime`.
- `tests/meta_speak_adapter.rs`: Contract-level tests for voice set and backend selection.
- `docs/meta_speak.md`: Detailed feature docs.
- `docs/controls.md`: User-facing control entry.

## Plan of Work

The implementation sequence was:

1. Introduce an adapter boundary in `src/meta_speak.rs` and default it to `soundtest`.
2. Extend Meta panel state and rendering to include Speak controls.
3. Add input focus and keyboard capture to keep text entry safe.
4. Run speak execution in worker threads and poll outcomes in UI update.
5. Add tests for stable behavior contracts and document usage.

## Concrete Steps

Working directory: repository root.

Commands executed during implementation:

    cargo check

Validation commands:

    cargo test --test meta_speak_adapter
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Validation and Acceptance

Acceptance criteria:

- The Meta panel shows a **Speak** section with voice options `dog/cow/dragon`, `content` field, and `Speak` button.
- Clicking `Speak` with non-empty content triggers background speech and shows status/errors in panel.
- Typing in `content` does not trigger gameplay keyboard actions.
- New tests pass and rendered smoke command starts and exits without crash.

## Idempotence and Recovery

All edits are additive and safe to rerun. If rendered smoke fails due local environment (audio/display/device), keep artifacts/logs and rerun with a fresh `GRAVIMERA_HOME` temp directory.

## Artifacts and Notes

Compile failure observed before ort pin:

    error[E0609]: no field `SessionOptionsAppendExecutionProvider_VitisAI` on type `&'static OrtApi`

After pinning, `cargo check` completed successfully.

## Interfaces and Dependencies

New interface in `src/meta_speak.rs`:

- `MetaSpeakAdapter` with `fn speak(&self, request: MetaSpeakRequest) -> Result<MetaSpeakOutcome, String>`
- `MetaSpeakRuntime` resource holds `Arc<dyn MetaSpeakAdapter>`

Dependency decision:

- `soundtest` is linked via path dependency.
- `ort` is pinned to `=2.0.0-rc.11` to maintain compatibility in current build graph.

---

Revision note (2026-03-09): Added implementation status, discovered ort resolution issue, recorded the compatibility pin decision, and marked completion after commit `0bb8633`.
