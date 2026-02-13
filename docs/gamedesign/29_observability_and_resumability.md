# Observability and Resumability (Logs, Artifacts, Checkpoints)

Scene generation and world authoring can be long-running and multi-step. To make the system **debuggable** and **robust to crashes**, Gravimera must treat “generation” as a first-class *run* with durable artifacts.

This document describes the product goals for logging, persistence, and resuming work. It does not prescribe exact file formats.

## Goals

1) **Debuggability**: when a generated scene is “bad”, a developer (human or agent) can see *what happened* and *why*.
2) **Durability**: long runs can survive crashes and restarts without losing progress.
3) **Reproducibility**: a run can be replayed deterministically from captured artifacts (subject to policy).
4) **Auditability**: in hosted realms, authoring actions can be traced to principals and request ids.
5) **Safety**: logs must avoid leaking secrets and must be capability-aware.

## The Run as a First-Class Object

Any non-trivial authoring process (scene generation, large blueprint application, asset generation jobs, story pack updates) should be represented as a **run** with:

- an identity (run id),
- a timeline of steps,
- durable artifacts (inputs, outputs, validation reports),
- a clear terminal status (success/failure/canceled),
- a resumability state (what can continue after a restart).

This makes “agent worldbuilding” comparable to a build system: you can inspect and resume, not just “try again”.

## Logging Requirements (Structured, Correlated)

### Correlation IDs

Logs must be correlatable across the whole pipeline:

- run id
- request id(s) used for idempotent actions
- principal/agent id (when applicable)
- scene id(s) and realm id
- job id(s) for long-running tasks

### Structured Logs

Logs should be structured enough that an automated agent can parse them:

- step boundaries: started/finished
- key decisions and constraints used (e.g. which style pack, which seed policy)
- validation outcomes (pass/fail + metrics summary)
- error objects (codes + causes + evidence pointers)

This is not a replacement for human-readable logs; it is a requirement for automated triage and repair.

### Evidence Pointers (Not Just Messages)

When something fails, logs must point to concrete evidence:

- which artifact failed validation,
- which gate failed in the scorecard,
- which inputs caused the failure,
- which region/marker/object identity is involved (as references, not guesses).

## Durable Artifacts (Key Steps Persisted)

To support resuming and debugging, the system must persist the **key step artifacts** of a run to durable storage.

In local development this is local disk by default. In hosted environments this is host-managed durable storage.

Artifacts to persist (conceptually):

- input specs (WorldSpec, SceneIntentSpec, ScorecardSpec) and a reference to any external inputs
- generated plans and patches (ScenePlan, Blueprint, BlueprintPatch history)
- validation reports (including metrics and violations)
- stable run signatures for regression/determinism checks
- a bounded event log segment and snapshots sufficient to reproduce failures
- job results for long-running asset generation (prefab packs, imports)

The goal is that “reproduce the failure” is always possible without re-running the entire pipeline from scratch.

## Checkpoints and Resume Semantics

### Checkpoint Definition

A checkpoint is a durable “commit point” in the run timeline after which the system can safely resume.

Checkpoints should exist after:

- spec finalization,
- successful validation,
- successful application of a blueprint batch,
- completion of long-running jobs,
- completion of an evaluation window (simulation + scorecard).

### Resume Rules

On restart after a crash:

- the run resumes from the last completed checkpoint,
- incomplete steps either:
  - retry safely (idempotent), or
  - are marked failed with a clear reason.

The system must avoid “duplicating work” (e.g. spawning duplicates) by relying on idempotent request ids and explicit run step boundaries.

## Human and Agent UX Expectations

Humans and agents should both be able to:

- list runs and their statuses,
- open a run timeline and see step-by-step progress,
- inspect the artifacts and validation reports,
- resume or fork a run (create a new run from the last checkpoint with a patch).

This makes large-scale creation practical and reduces the cost of iteration.

## Privacy and Secret Handling

Because agents may use external providers:

- logs must never include secrets (API keys, bearer tokens).
- sensitive prompts or images should be stored only if policy allows and should be referenced by content hash or redacted summaries where appropriate.

In hosted realms, artifact retention and access must be controlled by capabilities and host policy.

## Relationship to Other Docs

- Scene generation pipeline: `docs/gamedesign/26_scene_generation_agent_system.md`
- Agent dev loop: `docs/gamedesign/24_agent_dev_loop.md`
- Evaluation and repair goals: `docs/gamedesign/25_evaluation_and_auto_repair.md`
- Specs (exact contracts and formats): `docs/spec/README.md`

