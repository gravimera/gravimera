# Gen3D: Preserve-mode edit policies + plan-diff validation (prevent accidental rig rewires)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D supports seeded edit sessions (“Edit” / “Fork” from an existing Gen3D-saved prefab). In these sessions, the engine defaults to “preserve existing components” behavior so small edits (for example, “add a hat”) do not regenerate the whole object.

Today, preserve-mode replanning (`llm_generate_plan_v1` with `constraints.preserve_existing_components=true`) can still silently “rewire” the rig: a returned plan is allowed to change how existing components attach to each other (which parent anchor they use, which child anchor they use, and so on). Because preserve mode intentionally keeps the existing geometry and anchor frames for already-generated components, this kind of rewire frequently collapses a previously-good model into a pile of overlapping parts (the plan author “guessed” new join frames without seeing the existing numeric frames).

After this change:

1) Preserve-mode replanning becomes a deterministic, policy-enforced “patch” operation: the agent must explicitly choose what kinds of changes it intends to make, and the engine must reject plans whose diffs exceed that policy.

2) The agent can still do “any modification” (add/delete/replace/change motion/rewire), but larger or riskier modifications must be expressed explicitly:
   - either by selecting a broader preserve-mode edit policy, or
   - by turning off preserve mode (full replan / rebuild), or
   - by using deterministic patch tools (`apply_draft_ops_v1`) for attachment offsets and animation slot edits.

User-visible outcome: seeded edits like “add a hat” stop breaking existing models, while intentionally large edits still remain possible and machine-observable.

## Progress

- [x] (2026-03-10 09:03Z) Create this ExecPlan and capture the current failure mode + desired contract.
- [ ] Define the preserve-mode “edit policy” contract (names, defaults, tool args, and docs).
- [ ] Implement plan-diff computation and deterministic validation for `llm_generate_plan_v1` in preserve mode.
- [ ] Update preserve-mode plan prompt to describe the policy and reduce invalid plans.
- [ ] Add regression tests for the validator (no LLM/network dependency).
- [ ] Update documentation: `docs/gen3d/edit_preserve_existing_components.md` and tool schema docs.
- [ ] Run `cargo test` and the required rendered smoke test.
- [ ] Commit.

## Surprises & Discoveries

- Observation: The current preserve-mode merge logic already prevents some plan damage (it preserves geometry parts and preserves anchor frames for existing anchor names), but it does not preserve the attachment tree.
  Evidence: In `src/gen3d/ai/agent_tool_poll.rs`, preserve-mode merge copies old `anchors` and primitive/model `parts`, but then calls `convert::sync_attachment_tree_to_defs(&job.planned_components, draft)` using the *new plan’s* `attach_to` edges. If the plan swaps `parent_anchor`/`child_anchor` for existing components, the world transforms are recomputed and the assembled object can collapse.

- Observation: Preserve-mode prompt text includes “Step 1: Split the object into multiple components”, which makes sense for a fresh plan but is misleading for a seeded edit.
  Evidence: `build_gen3d_plan_user_text_preserve_existing_components()` appends the generic plan instructions from `build_gen3d_plan_user_text_with_hints()`, which starts with that “split” framing and encourages re-authoring the entire attachment tree.

## Decision Log

- Decision: Introduce a versioned, explicit preserve-mode “edit policy” that is selected by the agent via `llm_generate_plan_v1.constraints`, and enforced by a deterministic plan-diff validator.
  Rationale: This is the most generic, non-heuristic way to prevent accidental rewires while still allowing the agent to request broader modifications intentionally.
  Date/Author: 2026-03-10 / flow + agent

- Decision: Default seeded edits (preserve mode) to the safest patch policy that prevents rewiring existing components.
  Rationale: “Add a hat” should not be able to scramble the existing object. Broader edits should require an explicit tool arg so the behavior is deliberate and debuggable.
  Date/Author: 2026-03-10 / flow + agent

## Outcomes & Retrospective

- (not started) This section will be updated once implementation milestones land.

## Context and Orientation

This work is scoped to Gen3D planning and preserve-mode replanning. Relevant code and how it currently works (paths are from repo root):

- `src/gen3d/ai/agent_tool_dispatch.rs`: starts `llm_generate_plan_v1` by building prompt text. In preserve mode it calls `build_gen3d_plan_user_text_preserve_existing_components(...)`.
- `src/gen3d/ai/prompts.rs`: builds the plan prompt user text. Preserve-mode prompt currently includes a compact “existing component snapshot” (names + anchor names only), not numeric frames.
- `src/gen3d/ai/agent_tool_poll.rs`: receives the LLM plan response, parses it, converts it to `planned_components`, and applies it.
  - Preserve-mode guardrails today: (a) must include all existing component names, (b) must keep the same root component name.
  - Preserve-mode merge today: preserves existing component generation status and preserves existing anchors by name; preserves primitive/model geometry parts; but applies the new attachment tree.
- `src/gen3d/ai/job.rs`: defines `Gen3dPlannedComponent` and `Gen3dPlannedAttachment` (the in-memory “plan” that `sync_attachment_tree_to_defs` uses).
- `docs/gen3d/edit_preserve_existing_components.md`: describes preserve-mode behavior; it will need to be updated to match the new “edit policy” contract.

Definitions used in this ExecPlan (plain language):

- Seeded edit session: a Gen3D run seeded from an existing Gen3D-saved prefab (Edit/Fork). In these runs the engine sets `preserve_existing_components_mode=true` by default.
- Preserve mode (preserve existing components): a mode where the engine tries to keep existing component geometry and avoids regenerating already-generated components.
- Attachment tree / rig: the directed tree of `attach_to` edges describing which component is attached to which parent, which anchors are used, and what transform offset/joint/animations apply.
- Rewire: changing an existing component’s `attach_to.parent`, `attach_to.parent_anchor`, or `attach_to.child_anchor` (and therefore changing the topology or interface of the attachment tree).
- Plan-diff validation: a deterministic comparison between the previous plan and the proposed new plan, producing a list of differences and rejecting the plan if it exceeds the selected policy.

## Plan of Work

### Milestone 1 — Define a preserve-mode edit policy contract (tool args + defaults)

Add a new preserve-mode “edit policy” concept for `llm_generate_plan_v1` that the agent must select explicitly via tool args and that the engine must enforce deterministically.

In `src/gen3d/agent/tools.rs`, extend the `llm_generate_plan_v1` args schema to include a new field under `constraints`, for example:

    constraints?: {
      preserve_existing_components?: bool,
      preserve_edit_policy?: "additive" | "allow_offsets" | "allow_rewire",
      rewire_components?: string[]
    }

Contract (this is the important part; the exact field names can change, but the semantics must not):

- When `preserve_existing_components` is false (or preserve mode is not active), the policy fields are ignored (fresh plan semantics).
- When `preserve_existing_components` is true and the draft already contains generated geometry (the preserve-merge path is active), the engine applies plan-diff validation:
  - `additive` (default for seeded edits): The new plan may add components and add anchors, but must not change any existing component’s attachment interface (`attach_to.parent`, `parent_anchor`, `child_anchor`). It also must not change any existing anchor frames for already-existing anchor names. (New anchors are allowed.)
  - `allow_offsets`: Same as `additive`, but allow changing `attach_to.offset` (and only the offset) for existing components whose attachment interface is unchanged. This policy is for explicit “reposition existing parts without rewiring” edits.
  - `allow_rewire`: Allow rewiring of a caller-provided explicit allow-list: only components named in `constraints.rewire_components` may change attachment interface. If `rewire_components` is missing/empty, the tool must reject the plan (no silent “rewire everything”).

This policy is intentionally about “what diffs are allowed” and is not heuristic: it is a strict, deterministic rule set that produces machine-readable errors.

### Milestone 2 — Implement deterministic plan-diff computation and validation

In `src/gen3d/ai/agent_tool_poll.rs` in the `GeneratePlan` tool-result handling, extend the existing preserve-mode guardrails to validate policy-specific diffs before applying the plan.

Implementation sketch:

1) Define a small internal diff model (in code) that is easy to test and easy to return as a tool error payload. For example:

    enum PlanDiffViolationKind { Rewire, OffsetChanged, AnchorFrameChanged, RootChanged, MissingComponent, ... }
    struct PlanDiffViolation { component: Option<String>, field: String, kind: PlanDiffViolationKind, old: String, new: String }

2) Compute `old_by_name` and `new_by_name` maps for components. Reuse the existing “all names present” and “root unchanged” checks, and then add:

  - For each existing component name:
    - If old has `attach_to` and new has `attach_to`:
      - Check interface fields (`parent`, `parent_anchor`, `child_anchor`).
      - If interface differs: record a `Rewire` violation.
      - If interface is same but policy is `additive`:
        - If `offset` differs: record an `OffsetChanged` violation (because offsets must be edited via `apply_draft_ops_v1` or a broader policy).
      - If interface is same and policy is `allow_offsets`:
        - Allow `offset` diffs, but still record violations for diffs in `joint` or `animations` (these must be edited via dedicated tools).
    - If old has `attach_to` and new is missing it (or vice versa): record a `Rewire` violation (attachment removal/addition to an existing component is a topology change).

  - For existing anchors (anchor names present in the old component):
    - If the new plan provides an anchor of the same name with a different frame, record an `AnchorFrameChanged` violation. (These changes are not applied today anyway because preserve-mode merge preserves old anchor frames; rejecting them makes the behavior explicit and non-surprising.)

3) Validate against policy:

- For `additive`: any `Rewire`, `OffsetChanged`, or `AnchorFrameChanged` on an existing component is an error.
- For `allow_offsets`: allow offset diffs when interface unchanged; still reject rewires and anchor-frame changes; reject joint/animation diffs.
- For `allow_rewire`: allow `Rewire` only when `component.name` is in `constraints.rewire_components`; still reject anchor-frame changes for existing anchor names (rewires should use existing anchors or new anchors; existing anchor frames are preserved).

4) If any violations exist, return a tool error with a structured payload:

    {
      "ok": false,
      "error": "Preserve-mode plan violates edit policy",
      "policy": "...",
      "violations": [ ... ],
      "hint": "Either (a) re-run llm_generate_plan_v1 with a broader preserve_edit_policy, (b) disable preserve mode for a full rebuild, or (c) use apply_draft_ops_v1 for offsets/animations."
    }

This error must be deterministic and should include enough detail for the agent to decide its next tool call without guesswork.

### Milestone 3 — Update preserve-mode plan prompt to match the new policy

In `src/gen3d/ai/prompts.rs`, adjust `build_gen3d_plan_user_text_preserve_existing_components(...)` so it no longer encourages rewriting the entire rig.

Concrete changes:

- Add an explicit “Selected preserve edit policy” paragraph near the top of the prompt (the engine knows the policy from tool args). Spell out what is allowed and what will be rejected.
- Remove or heavily rewrite the inherited “Step 1: Split the object into multiple components” section for preserve mode. Preserve-mode replanning must instead read as: “You are patching an existing plan; keep existing attachment interfaces stable unless explicitly allowed.”
- Expand the “existing component snapshot” to include (bounded):
  - each component’s current `attach_to` interface (parent + anchors),
  - the existing anchor name list (already done),
  - but still keep numeric frames out of the default prompt for token budget reasons.

If `preserve_edit_policy=allow_rewire` is selected, include additional bounded numeric context (because rewiring without frames is error-prone):

- Include the numeric frames (`pos`, `forward`, `up`) for the specific anchors that are allowed targets for rewiring (bounded to the anchor names that appear in `rewire_components`’ current `attach_to` and the parent components’ anchor frames).
- Keep a strict max (for example, cap to N components and M anchors) and error if the request exceeds caps; do not silently truncate in a way that changes meaning.

### Milestone 4 — Tests (no LLM/network dependency)

Add deterministic tests that cover the validator behavior. These tests must not call external models and must not depend on local cache folders.

Suggested approach:

- Add a focused unit test module near the validator implementation (for example in `src/gen3d/ai/agent_tool_poll.rs` or a small new `src/gen3d/ai/plan_diff.rs` helper module).
- Construct an `old_components` list representing a simple torso→neck→head chain with known attachment interfaces and anchors.
- Construct a `new_components` list that:
  - keeps names the same,
  - adds a new component (`santa_hat`) and/or new anchor (`hat_mount`) (should be allowed),
  - but also changes an existing component’s `attach_to.parent_anchor` to a new anchor name (a rewire) (must be rejected under `additive` and `allow_offsets`).

Assertions:

- Under `additive`: validator rejects with at least one `Rewire` violation.
- Under `allow_offsets`: same.
- Under `allow_rewire` with `rewire_components=["neck"]`: accept when only neck is rewired; reject if head is rewired without being allow-listed.

Add one test that ensures “anchor frame changed for an existing anchor name” is rejected under all preserve policies.

Any test asset files (if needed later for an integration test) must live under `test/` (not sprinkled in the repo root).

### Milestone 5 — Documentation updates

Update `docs/gen3d/edit_preserve_existing_components.md` to describe:

- the new `preserve_edit_policy` concept,
- defaults for seeded edits,
- what kinds of diffs are rejected, and the intended tool to use for each kind of edit:
  - add/modify primitives: `apply_draft_ops_v1`
  - tweak offsets: `apply_draft_ops_v1` (or `allow_offsets` policy if you intentionally want offsets in-plan)
  - edit motion clips: `llm_generate_motion_authoring_v1` or `apply_draft_ops_v1` upsert/remove animation slots
  - rewire: `allow_rewire` with explicit allow-list, or full rebuild

Also update any tool schema references (for example in `docs/gen3d/next_actions.md` if it enumerates tool args or contracts).

### Milestone 6 — Validation and shipping

From repo root, run:

    cargo test

Then run the required rendered smoke test (UI, not headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Acceptance criteria (human-verifiable):

1) In a seeded edit session, calling `llm_generate_plan_v1` with `preserve_existing_components=true` and default policy no longer allows accidental rewires of existing components. Instead, the tool returns a clear, structured error listing the disallowed diffs.

2) With explicit `preserve_edit_policy=allow_rewire` and `rewire_components=[...]`, a plan that rewires only the allow-listed components is accepted and applied, while rewiring other components is still rejected.

3) “Add a hat” style edits do not scramble an existing model: adding a new component (hat) attached to an existing head anchor (or a new `hat_mount` anchor) results in a stable, coherent assembled object.

4) All tests pass (`cargo test`) and the rendered smoke test starts and renders without crashing.

## Concrete Steps

Implementation should proceed in small commits that keep the project working:

1) Extend tool schema and plumb the new constraint field through `agent_tool_dispatch.rs` into the LLM prompt builder and into `agent_tool_poll.rs` validation logic.
2) Implement the plan-diff validator and wire it into the preserve-mode plan apply path.
3) Update preserve-mode prompt text to reflect the policy and reduce invalid plans.
4) Add tests and run `cargo test`.
5) Run the rendered smoke test.
6) Commit with a clear message (for example: “gen3d: add preserve-mode edit policies + plan-diff validator”).

## Idempotence and Recovery

- The validator is deterministic and should be safe to iterate on: failing validation must leave the draft unchanged (no partial plan merges, no attachment tree sync).
- Keep validation failures observable: return the structured violation list and write the rejected plan text to the pass dir artifacts (so it can be inspected in UI/logs).
- If a policy choice prevents a desired edit, the recovery path is explicit:
  - use a broader policy for preserve-mode replans, or
  - disable preserve mode and do a full replan/rebuild, or
  - use `apply_draft_ops_v1` for offset/animation edits.

## Artifacts and Notes

The implementation should ensure these artifacts remain helpful during debugging:

- When a plan is rejected by policy, record:
  - the parsed new plan hash (if available),
  - the policy + allow-list used,
  - the violation list,
  - and the raw plan response text (already captured for other tool calls).

This makes “why was my plan rejected?” answerable from artifacts without re-running the model.

## Interfaces and Dependencies

Do not add new external dependencies. Keep this within existing Gen3D modules.

At the end of implementation, these public/tool-facing interfaces must exist and be stable:

- `llm_generate_plan_v1` accepts the new preserve-mode policy fields under `constraints` (schema and tool detail output updated).
- `get_tool_detail_v1` for `llm_generate_plan_v1` reflects the new args schema.
- Preserve-mode plan validation produces structured, machine-readable errors on policy violations.

