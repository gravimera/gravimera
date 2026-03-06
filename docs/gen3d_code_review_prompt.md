# Gen3D Assumptions/Heuristics Audit — Copy/Paste Prompt

Use the prompt below to repeatedly audit Gen3D for engine-side **assumptions**, **silent defaults**, and **heuristics** (especially anything that violates “no heuristics” and anything that limits “any animation” expressiveness).

```text
You are in the `gravimera` repo root.

Task: Audit Gen3D for any engine code that:
1) assembles/attaches components based on hidden assumptions (axis conventions, join frames, rotation frames, missing anchor fallbacks, etc.) that could yield incorrect results, and
2) uses heuristic “guess intent” logic (violates the Gen3D rule: a user could ask for generating ANY object, so NO heuristics), and
3) limits animation expressiveness (a user could ask for generating ANY animation; do not silently clamp/disable/rewrite motion without an explicit contract).

Scope (minimum):
- `src/gen3d/**` (AI plan/draft conversion, reuse/copy, validation, agent loop)
- Any code that affects Gen3D runtime assembly/animation/visuals, especially `src/object/**` (attachments, PartAnimation runtime), and other obvious shared modules if referenced by Gen3D.

What to produce:
- Update or create a single TODO doc at `docs/gen3d/assumptions_heuristics_todo.md`.
- The doc must be a checkbox TODO list grouped into sections:
  - “Heuristic decisions (engine guesses intent)”
  - “Silent defaults / hidden assumptions”
  - “Any animation constraints”
  - “Diagnostics / guardrails”
- For each TODO item, include:
  - a 1–2 sentence plain-English description of the issue (“what the engine does”),
  - why it is an assumption/heuristic/limitation (tie to the rules above),
  - where it lives (file path + function/symbol name),
  - what could go wrong (concrete failure mode),
  - a suggested direction to fix later (no code changes now; just the direction).

Constraints:
- Do NOT implement fixes yet. This is a code review + documentation-only task.
- Prefer deterministic, contract-based reasoning. If the engine currently “chooses” between behaviors (scores, tie-breaks, fallbacks), treat that as a heuristic unless explicitly required by a formal spec.
- Keep `README.md` clean; put details in `docs/`.

Repo requirements (must follow):
- After changing anything, run a rendered (UI) smoke test (isolated data dir; no `--headless`):
    tmpdir=$(mktemp -d)
    GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2
- Commit the doc update with a clear message.

Deliverable:
- Reply with a short summary of what changed and reference `docs/gen3d/assumptions_heuristics_todo.md`.
```
