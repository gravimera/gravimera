# Gen3D: Tool Feedback Tab + Persistent History (AI-Submitted)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D is evolving quickly, and we want the AI to be more initiative and self-sufficient. A practical way to iterate is to let the AI explicitly ask for missing tools or improvements in a structured, developer-friendly way, and to surface those requests directly in the in-game UI.

After this change:

- The Gen3D AI can submit “tooling feedback” as part of its auto-review deltas (e.g. “we need a render tool that can capture a specific camera angle with anchors shown”).
- The game records that tooling feedback in the Gen3D cache folder and in a global history file that persists across restarts.
- The Gen3D UI gets a separate **Tool Feedback** side tab with a `*` unread indicator.
- Testers can click **Copy for Codex (last run)** to copy a compact, prompt-ready summary + file paths, so they can paste it into Codex and immediately start implementing the suggested enhancements.

You can see this working by running the game, generating a Gen3D model, waiting for the auto-review step, and then opening the Tool Feedback tab. If the AI produced feedback, it appears there and is also saved to disk.

## Progress

- [x] (2026-02-01 18:15Z) Write this ExecPlan.
- [ ] Implement persisted Tool Feedback history (`gen3d_cache/tool_feedback_history.jsonl`).
- [ ] Add Tool Feedback UI tab (unread `*` cleared on tab click) + copy buttons.
- [ ] Extend `review_delta_v1` with a `tooling_feedback` action and persist any submitted feedback.
- [ ] Validation: `cargo test` and `cargo run -- --headless --headless-seconds 1`.
- [ ] Update `README.md` and commit.

## Surprises & Discoveries

- Observation: (to fill)
  Evidence: (to fill)

## Decision Log

- Decision: Persist tooling feedback to both a per-run file and a global history file.
  Rationale: Per-run files are easy to share with “this exact run”, while global history survives restarts and helps testing over time.
  Date/Author: 2026-02-01 / Codex

- Decision: Make “Copy for Codex” compact and path-based (no huge schemas inline).
  Rationale: Developers using Codex can open the referenced jsonl/log files locally; keeping the clipboard payload short avoids noise.
  Date/Author: 2026-02-01 / Codex

## Outcomes & Retrospective

(To fill at completion.)

## Context and Orientation

Gen3D lives in `src/gen3d/`.

Relevant existing pieces:

- `src/gen3d/ui.rs`: builds the Gen3D workshop UI.
- `src/gen3d/status.rs`: status panel scrolling + scrollbar UI.
- `src/gen3d/state.rs`: `Gen3dWorkshop` resource and UI marker components.
- `src/gen3d/ai/schema.rs`: typed JSON schemas (plan/draft/review delta).
- `src/gen3d/ai/convert.rs`: applies review delta actions to the current plan/draft.
- `src/gen3d/ai/mod.rs`: main Gen3D AI state machine; where review delta is requested and applied.
- `src/gen3d/ai/artifacts.rs`: helpers for writing json/text artifacts to the cache folder.

Terminology:

- **Run**: one click of Build, producing `gen3d_cache/<run_id>/`.
- **Last run**: for copy purposes, the most recent `run_id` that has any tooling feedback entries (or the current run if building).
- **Tool feedback entry**: one AI-submitted record, written as one JSON object per line (`.jsonl`).

## Plan of Work

### 1) Add a persisted Tool Feedback history resource

Create `src/gen3d/tool_feedback.rs` with:

- `Gen3dToolFeedbackHistory` (Bevy `Resource`) containing a `Vec<Gen3dToolFeedbackEntry>`.
- `Gen3dToolFeedbackEntry` fields:
  - `entry_id` (UUID string)
  - `created_at_ms`
  - `run_id`, `attempt`, `pass` (strings/ints)
  - `priority`, `title`, `summary`
  - `raw` (full JSON payload as `serde_json::Value`)
  - `evidence_paths` (strings for run/pass dir, `tool_feedback.jsonl`, `gravimera.log`, etc.)
- `load_tool_feedback_history(config: &AppConfig) -> Gen3dToolFeedbackHistory` that reads:
  - global file: `<gen3d_cache_dir>/tool_feedback_history.jsonl`
  - ignores invalid lines with `warn!` (do not crash)
- `append_tool_feedback_entry(config, run_dir, entry)` that appends to:
  - per-run file: `gen3d_cache/<run_id>/tool_feedback.jsonl`
  - global history file: `gen3d_cache/tool_feedback_history.jsonl`

Wire `Gen3dToolFeedbackHistory` into app startup so it loads across restarts (rendered + headless is fine).

### 2) Add a new Tool Feedback tab in the Gen3D UI (separate side tab)

Update `src/gen3d/state.rs`:

- Add `Gen3dSideTab` enum with `Status` and `ToolFeedback`.
- Add `side_tab: Gen3dSideTab` and `tool_feedback_unread: bool` to `Gen3dWorkshop`.
- Add UI marker components for:
  - tab buttons and their text
  - tool feedback panel root
  - copy buttons
  - tool feedback text area (scrollable)

Update `src/gen3d/ui.rs` to:

- Replace the current right-side “Status” panel with:
  - A small tab bar at the top (`Status` and `Tool Feedback`).
  - Two content panels (Status panel and Tool Feedback panel) toggled via `Visibility`.
- When the user clicks the Tool Feedback tab:
  - set `workshop.side_tab = ToolFeedback`
  - clear `workshop.tool_feedback_unread` (this removes the `*`)
- If new feedback arrives while the user is on Status:
  - set `workshop.tool_feedback_unread = true` (shows `*`)

Add “Copy for Codex (last run)” and “Copy JSON (last run)” buttons at the top of the Tool Feedback panel.

Clipboard write should be implemented similarly to the existing clipboard read (macOS `pbcopy`, Windows PowerShell `Set-Clipboard`, Linux `wl-copy`/`xclip`/`xsel`).

The “Copy for Codex” payload must be compact and path-based:

    # Gravimera Gen3D tooling feedback (last run)
    Repo: /Users/flow/workspace/github/gravimera
    Git: <hash>
    Run: <run_id>

    Summary:
    - [priority] title — summary (attempt_X/pass_Y)

    Files:
    - gen3d_cache/<run_id>/tool_feedback.jsonl
    - gen3d_cache/<run_id>/attempt_*/pass_*/gen3d_run.log
    - gen3d_cache/<run_id>/attempt_*/pass_*/gravimera.log
    - gen3d_cache/<run_id>/attempt_*/pass_*/review_*.png

### 3) Extend review delta schema with tooling feedback action

Update `src/gen3d/ai/schema.rs`:

- Add a new `AiToolingFeedbackJsonV1` struct. Keep it permissive so we can evolve it (avoid `deny_unknown_fields`).
- Add a new `AiReviewDeltaActionJsonV1` variant:
  - `ToolingFeedback { feedback: AiToolingFeedbackJsonV1 }`

Update `src/gen3d/ai/prompts.rs`:

- In `build_gen3d_review_delta_system_instructions`, document the new `tooling_feedback` action:
  - Use it to request missing tools, enhancements, or report tool bugs.
  - It must include at least `priority`, `title`, `summary`.
  - It can include any additional structured details in `details` for developers.

Update `src/gen3d/ai/convert.rs`:

- Extend `AiReviewDeltaApplyResult` with `tooling_feedback: Vec<AiToolingFeedbackJsonV1>`.
- In `apply_ai_review_delta_actions`, collect any tooling feedback actions into the result, but do not mutate the draft/plan for that action.

Update `src/gen3d/ai/mod.rs`:

- After applying the review delta, for each `tooling_feedback` entry:
  - construct a `Gen3dToolFeedbackEntry` with run metadata and evidence paths
  - append it to the global + per-run jsonl files
  - push it into `Gen3dToolFeedbackHistory`
  - if `workshop.side_tab == Status`, set `workshop.tool_feedback_unread = true`

### 4) Docs, tests, and smoke run

- Update `README.md` to mention the new Tool Feedback tab and where the history is stored.
- Run:
  - `cargo test`
  - `cargo run -- --headless --headless-seconds 1`
- Commit changes.

## Concrete Steps

From repo root (`/Users/flow/workspace/github/gravimera`):

1. Implement `src/gen3d/tool_feedback.rs` and wire resources/systems.
   - Run `cargo test`.

2. Update Gen3D UI for tabs + copy buttons.
   - Run `cargo test`.

3. Extend review delta schema and apply pipeline to record feedback.
   - Run `cargo test`.
   - Run `cargo run -- --headless --headless-seconds 1`.

4. Update `README.md` and commit.

## Validation and Acceptance

Acceptance is met when:

- The game loads `gen3d_cache/tool_feedback_history.jsonl` on startup (no crashes if missing/invalid lines).
- During a Gen3D run, if the AI outputs a `tooling_feedback` action in a review delta, the entry:
  - appears in the Tool Feedback tab,
  - is appended to `gen3d_cache/<run_id>/tool_feedback.jsonl`,
  - is appended to `gen3d_cache/tool_feedback_history.jsonl`.
- The Tool Feedback tab shows a `*` when new feedback arrives while on Status, and clears `*` when the tab is opened.
- “Copy for Codex (last run)” copies a compact summary + paths for all entries from the most recent run.
- `cargo test` passes and headless smoke run exits cleanly.

## Idempotence and Recovery

- History files are append-only; invalid lines are skipped (with warnings).
- Copy operations do not mutate game state.
- If the per-run cache folder can’t be written, the entry still attempts to write to the global history file and shows a warning in logs.

## Artifacts and Notes

(To fill during implementation.)

## Interfaces and Dependencies

- Use existing `AppConfig.gen3d_cache_dir` to locate the cache base directory.
- Use existing `uuid` crate for entry ids.
- Use OS clipboard commands (same approach as `pbpaste` in `src/gen3d/ui.rs`).
