# Scene Runs (v1) — Durable Artifacts, Checkpoints, Resume

_(Spec document; see `docs/gamedesign/29_observability_and_resumability.md` for product goals.)_

This spec defines a minimal, stable on-disk **run directory** layout for long-running scene
generation and iterative authoring.

Run directories are **durable debug artifacts**: they are not required to load a scene.

## Directory Layout (Normative)

Runs live under the scene directory:

    scenes/<scene_id>/runs/<run_id>/

Minimal structure (v1):

    runs/<run_id>/
      run.json
      steps/
        0001/
          scorecard.json
          patch.json
          pre_validation_report.json
          apply_result.json
          post_signature.json
          complete.json

Notes:

- `steps/<n>/complete.json` is the checkpoint marker for step `<n>`.
- All JSON writes must be atomic at the file level (write temp → rename).
- Hosts may prune `runs/` by policy.

## `run.json` (v1)

`run.json` is the stable entry point for a run.

Required fields:

- `format_version` (integer, `1`)
- `run_id` (string)
- `scene_id` (string)

Optional fields:

- `label` (string)
- `principal_id` / `agent_id` (string)

Run artifacts must not contain secrets (API keys, bearer tokens).

## Step Semantics (v1)

Each step directory `steps/<n>/` represents one authoring iteration, typically:

1) Validate a patch against the current scene sources (`pre_validation_report.json`).
2) Apply the patch (mutate `src/`), recompile layers (`apply_result.json`).
3) Record a stable signature of the intended compiled instance set (`post_signature.json`).
4) Write `complete.json` as the checkpoint.

If a crash happens mid-step, resume is defined as:

- Find the highest `<n>` with `complete.json` present.
- Re-run the next incomplete step.

Idempotency is achieved via deterministic ids (e.g. `request_id` + `local_ref`) in patches.

