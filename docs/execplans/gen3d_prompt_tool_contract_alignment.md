# Gen3D: Prompt ↔ tool contract alignment (args + returns)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D is driven by an LLM “agent step” loop that selects tools by emitting strict JSON. The LLM decides which tool to call (and with what arguments) by reading the tool list embedded in the agent prompt. That tool list is generated from the in-engine tool registry (`src/gen3d/agent/tools.rs`) and shows a brief “args signature” plus a short example.

Today, several Gen3D tools have a mismatched contract between:

- what the prompt *claims* a tool accepts/returns (the “brief args signature” + prose hints), and
- what the tool implementation in the engine actually accepts/returns.

These mismatches are high-cost: the LLM will confidently call tools with the advertised shapes, then hit deterministic errors (wasting steps, budgets, and review-delta rounds). They also increase support burden because the error messages often point to alias keys not present in the prompt contract.

After completing this plan, the Gen3D agent prompt and the tool implementations are contract-aligned:

- If the prompt claims `string|number`, the tool accepts both deterministically.
- If the tool requires an argument, the prompt signature makes that requirement obvious.
- If the tool returns fields used by downstream tools, the prompt summary/descriptor reflects those fields accurately (without dumping huge schemas).

How to see it working (observable outcomes):

1. Run any Gen3D job (or just build the prompt text) and confirm the tool list shows accurate required args (for example, `copy_component_v1` clearly requires a target).
2. Execute tool calls that previously failed purely due to type/requiredness mismatches (for example, `copy_component_v1` with `source_component: 0`) and confirm they now succeed (or fail for legitimate semantic reasons, not arg-shape reasons).
3. Run `cargo test` and the rendered smoke test and confirm the game starts without crashing.


## Progress

- [x] (2026-03-24) Drafted this ExecPlan (`docs/execplans/gen3d_prompt_tool_contract_alignment.md`).
- [x] (2026-03-24) Expanded this ExecPlan with concrete target contracts and edit locations for the known mismatches.
- [ ] Audit all Gen3D tools for prompt/impl mismatches (args + returns), beyond the known cases listed below.
- [ ] Decide the canonical contract per tool (canonical keys, accepted aliases, requiredness, types), and document those decisions in `Decision Log`.
- [ ] Implement contract fixes (update tool registry text and/or tool arg parsing gates) in small, reviewable commits.
- [ ] Add unit tests that lock the contracts (fail before fixes; pass after fixes).
- [ ] Run `cargo test` and the rendered smoke test (`cargo run -- --rendered-seconds 2`), and capture short transcripts in this doc.
- [ ] Update any affected docs under `docs/` (keep `README.md` clean).


## Surprises & Discoveries

- Observation: `AGENTS.md` (and several existing docs) reference `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md`, but those files do not exist in this working tree (only `docs/agent_skills/SKILL_agent.md` exists).
  Evidence: `docs/execplans/gen3d_deterministic_pipeline.md` notes the absence; `ls docs/agent_skills` lists only `SKILL_agent.md`.

- Observation: The agent prompt’s “args signature” is derived from only the first line of `Gen3dToolDescriptorV1.args_schema` and is treated as the “truth” by the model.
  Evidence: `src/gen3d/ai/agent_prompt.rs::build_agent_user_text` uses `first_line(tool.args_schema)` and prints `args={...}` per tool.

- Observation: Several tools accept additional alias keys (for robustness) that are not represented in the prompt signature, and some prompt signatures claim broader types than the implementation currently accepts.
  Evidence: `src/gen3d/ai/agent_tool_dispatch.rs` manually checks multiple alternative keys (for example, `source_component_name`, `source_idx`, etc.).


## Decision Log

- Decision: Treat `Gen3dToolRegistryV1` (prompt-facing tool descriptors) as the contract that the LLM should follow, and adjust implementations to match where it is safe and deterministic.
  Rationale: The LLM’s behavior is driven by the prompt. Aligning the implementation to the prompt reduces repeated “mismatch” failures without needing more prompt rules.
  Date/Author: 2026-03-24 / assistant

- Decision: Prefer deterministic, generic parsing improvements (for example, accept numeric indices where the contract says `string|number`) over introducing any intent-based inference or naming heuristics.
  Rationale: Gen3D should remain generic (no object-specific heuristics), and tool arg parsing must be deterministic and predictable.
  Date/Author: 2026-03-24 / assistant

- Decision: When a mismatch can be fixed either by narrowing the prompt contract or widening the tool implementation, prefer widening the implementation if and only if it is unambiguous and does not weaken existing safety gates.
  Rationale: The prompt contract is already “what the model believes”. If widening the implementation is deterministic (for example, accepting `number` where `string|number` is advertised), it reduces failures without changing user intent or agent policy.
  Date/Author: 2026-03-24 / assistant


## Outcomes & Retrospective

- (Not started) This plan is drafted but not yet executed.


## Context and Orientation

Key concepts (plain language):

- A “tool” is an engine function the LLM can invoke by returning a JSON action of kind `tool_call`. The engine executes tools and returns JSON results in the next step.
- The “tool registry” is the prompt-facing catalog of tools. In this repo it is implemented as `Gen3dToolRegistryV1::list()` returning `Gen3dToolDescriptorV1` entries in `src/gen3d/agent/tools.rs`.
- The “agent prompt tool list” is constructed in `src/gen3d/ai/agent_prompt.rs::build_agent_user_text(...)`. It prints each tool’s `one_line_summary`, the first line of `args_schema`, and an `args_example`.
- “Tool dispatch” is the engine-side implementation that matches a tool id and parses its `args` JSON. In this repo it lives in `src/gen3d/ai/agent_tool_dispatch.rs::execute_tool_call(...)`.

Known mismatches found during initial review (this list must be expanded during the audit step):

- `copy_component_v1` / `mirror_component_v1`
  - Prompt signature claims `source_component: string|number`, but the dispatcher reads `source_component` only as a string name; numeric indices must be supplied via separate, undocumented keys (like `source_component_index`).
  - Prompt signature marks `targets?` optional, but the dispatcher requires at least one target (either `targets` or `target_component`/`target_component_index`) and errors if none are provided.
  - Relevant code: `src/gen3d/agent/tools.rs` (descriptor), `src/gen3d/ai/agent_tool_dispatch.rs` (parsing in the `TOOL_ID_COPY_COMPONENT` / `TOOL_ID_MIRROR_COMPONENT` match arm).

- `copy_component_subtree_v1` / `mirror_component_subtree_v1`
  - Prompt signature claims `source_root: string|number`, but the dispatcher reads `source_root` only as a string name; numeric indices must be supplied via separate keys.
  - Relevant code: `src/gen3d/agent/tools.rs`, `src/gen3d/ai/agent_tool_dispatch.rs` (the subtree copy/mirror match arm).

- `detach_component_v1`
  - Prompt signature claims `component: string|number`, but the dispatcher reads `component` only as a string name; numeric indices must be supplied via separate keys.
  - Relevant code: `src/gen3d/agent/tools.rs`, `src/gen3d/ai/agent_tool_dispatch.rs` (the detach match arm).

- `copy_from_workspace_v1`
  - Prompt signature marks `components?` optional, but the implementation requires a non-empty `components` array.
  - Relevant code: `src/gen3d/agent/tools.rs` (descriptor) vs `src/gen3d/ai/workspaces.rs::copy_from_workspace_v1(...)` (validation).

- `query_component_parts_v1`
  - Prompt signature makes both `component?` and `component_index?` optional, but the implementation requires one of them (errors otherwise). The system prompt text already warns not to call it with `{}`, but the brief signature does not communicate the one-of requirement.
  - Relevant code: `src/gen3d/agent/tools.rs` vs `src/gen3d/ai/draft_ops.rs::query_component_parts_v1(...)` and the system prompt in `src/gen3d/ai/agent_prompt.rs`.

How tool signatures are currently used in error feedback:

- When a tool call fails, `src/gen3d/ai/agent_prompt.rs` tries to extract “required keys” from the tool’s first-line args signature. This is done by `required_keys_from_args_sig(...)`, which only understands a simple `{ key: type, key?: type }` shape.
- This mechanism cannot express “one-of” requirements (for example, “component OR component_index”) without changes either to the signature format or to the parser.

Where to change the contract:

- Prompt-facing tool contracts are authored in `src/gen3d/agent/tools.rs` (as `Gen3dToolDescriptorV1` entries). The agent prompt prints only the first line of `args_schema`, so requiredness/type info must fit on that first line if it needs to be “seen” by the model.
- Tool-side parsing gates are implemented in `src/gen3d/ai/agent_tool_dispatch.rs::execute_tool_call(...)` (sometimes by manual key checks, sometimes by `serde(deny_unknown_fields)` structs).


## Target contracts (initial, for known mismatches)

This section defines the intended “end state” for the tools we already know are mismatched. The audit step may add more tools here.

The guiding rule is: if the prompt advertises `string|number` for a canonical key, the tool must accept either a string name or a numeric index in that same key.

### `copy_component_v1`

Prompt-facing contract changes (edit `src/gen3d/agent/tools.rs`):

- Make it explicit that a target is required by making `targets` required in the brief args signature, and keep `source_component: string|number`.
- Prefer enumerating canonical enum values for the most common switches (so the model stops inventing values). Canonical values are:
  - `mode`: `detached|linked`
  - `anchors`: `preserve_interfaces|preserve_target|copy_source`
  - `alignment_frame`: `join|child_anchor`

Tool-side parsing changes (edit `src/gen3d/ai/agent_tool_dispatch.rs` in the `TOOL_ID_COPY_COMPONENT` arm):

- Accept `args.source_component` as either:
  - string name (existing behavior), or
  - number index (new behavior; treat as `source_component_index`).
- Accept `args.targets[]` array items as string or number (already supported) and ensure the error message for invalid items mentions the canonical contract (indices or names).
- Accept the singular `args.target_component` similarly (string or number), but keep `targets` as the prompt’s canonical way to pass targets.

Unit tests (add to `src/gen3d/ai/agent_tool_dispatch.rs` tests module, or factor a helper and test it):

- Verify that a call can resolve `source_component: 0` and `targets: [1]` into concrete indices without error.
- Verify that omitting all target forms produces an error that points to `targets` / `target_component` (not internal alias names).

### `mirror_component_v1`

Prompt-facing contract changes (edit `src/gen3d/agent/tools.rs`):

- Same as `copy_component_v1` for `source_component` and `targets`, but make it explicit that `alignment_frame` is effectively `join`-only for mirror (the engine rejects `child_anchor`).

Tool-side parsing changes (edit `src/gen3d/ai/agent_tool_dispatch.rs` in the `TOOL_ID_MIRROR_COMPONENT` arm):

- Same numeric acceptance rules as `copy_component_v1` for `source_component` and targets.
- Ensure the existing rejection for `alignment_frame=child_anchor` remains in place and its error message matches what the prompt contract claims.

### `copy_component_subtree_v1` / `mirror_component_subtree_v1`

Prompt-facing contract changes (edit `src/gen3d/agent/tools.rs`):

- Keep `source_root: string|number`.
- Keep `targets: (string|number)[]` required (it is already required in the signature).
- Mirror-only: document any `alignment_frame` restriction if the engine rejects `child_anchor` for mirror-subtree as well.

Tool-side parsing changes (edit `src/gen3d/ai/agent_tool_dispatch.rs` in the subtree copy/mirror arm):

- Accept `args.source_root` as either string name or numeric index (new behavior).
- Ensure targets parsing supports both types (already supports both).

### `detach_component_v1`

Prompt-facing contract changes (edit `src/gen3d/agent/tools.rs`):

- Keep `component: string|number` as the canonical key.

Tool-side parsing changes (edit `src/gen3d/ai/agent_tool_dispatch.rs` in the `TOOL_ID_DETACH_COMPONENT` arm):

- Accept `args.component` as either string name or numeric index (new behavior), in addition to the existing `component_index` alias.

### `copy_from_workspace_v1`

Prompt-facing contract changes (edit `src/gen3d/agent/tools.rs`):

- Make `components` required (remove the `?`).
- Make `mode` an explicit enum: `component|subtree` (instead of `string`).
- Keep `include_attachment?: bool`.

Tool-side changes:

- No behavioral change is required if we only fix the prompt contract. The implementation already requires `components` and validates `mode`.

Unit tests:

- Add a prompt-contract test that asserts the tool list line for `copy_from_workspace_v1` contains `components:` (not `components?:`).

### `query_component_parts_v1` (one-of requirement)

This is a “one-of required” tool (`component` OR `component_index`). The brief args signature format currently cannot express that cleanly.

Target outcome:

- The prompt must communicate, in the tool list itself, that at least one of `component` or `component_index` must be provided.
- The tool must continue to hard-error deterministically when neither is provided (no default guessing).

Implementation approaches (choose one during execution and record it in `Decision Log`):

1. Prompt-only clarity: add a second, short “requires:” hint line into the tool list output in `src/gen3d/ai/agent_prompt.rs::build_agent_user_text(...)` for just this tool id (and any other one-of tools found during audit).
2. Signature-format extension: extend the “brief signature” format plus `required_keys_from_args_sig(...)` to support an explicit one-of marker, and update the descriptor accordingly.

Approach (1) is preferred if it keeps the prompt compact and avoids inventing a signature mini-language.


## Plan of Work

First, do a complete, systematic audit of the tool contracts. This is not optional: the initial review found several mismatches quickly, which strongly suggests there are more. The audit outcome should be a short list of concrete mismatch issues, each with:

- the tool id,
- the advertised prompt-facing args signature and example,
- the implementation-side accepted keys/types/requiredness,
- and a proposed resolution.

Then, for each mismatch, choose one of two contract-first resolutions:

1. Update the tool implementation to match the prompt contract (preferred when it is deterministic and low-risk).
2. Update the prompt contract (tool descriptor) to match the implementation (preferred when widening the implementation would introduce ambiguity or break existing safety gates).

The work should proceed tool-by-tool, in small commits, with a new unit test (or expanded existing unit test) added alongside each fix to prevent regressions.


## Concrete Steps

All commands below are run from the repo root (`/Users/flow/workspace/github/gravimera`).

1. Audit tool contracts.

   - Enumerate tools: read `src/gen3d/agent/tools.rs` and list all `tool_id` values plus the first line of `args_schema`.
   - Enumerate implementations: read `src/gen3d/ai/agent_tool_dispatch.rs::execute_tool_call` match arms, plus any helpers in `src/gen3d/ai/*` that parse tool args via `serde(deny_unknown_fields)` structs.
   - For each tool, record: required keys, optional keys, accepted types (string vs number), and any alias keys.

2. Fix the “string|number but only string is accepted” family of mismatches.

   - Implement a single, shared, deterministic parser for “component reference” arguments that can accept either:
     - a string component name, or
     - a numeric component index,
     for canonical keys where the tool descriptor says `string|number`.
   - Apply it to:
     - `copy_component_v1` / `mirror_component_v1` (source + targets / target),
     - `copy_component_subtree_v1` / `mirror_component_subtree_v1` (source_root + targets),
     - `detach_component_v1` (component).
   - Update tool descriptor `args_schema` lines to reflect any additional requiredness constraints discovered during audit (for example, make it explicit that a target is required for copy/mirror).

3. Fix “optional in prompt but required in implementation” mismatches.

   - Update `copy_from_workspace_v1` descriptor so `components` is required (and document accepted `mode` values as `component|subtree`).
   - Consider whether any other tools have required keys that are currently marked optional in the brief signature; fix those similarly.

4. Decide how to handle “one-of” requirements in prompt hints.

   - For `query_component_parts_v1`, decide whether to:
     - keep the signature as-is but extend the prompt tool list to show a short “requires component OR component_index” hint line, or
     - extend the signature format + required-keys parser to represent one-of requirements.
   - The chosen approach must remain self-contained and deterministic and should not require the model to “guess”.

5. Add unit tests.

   - For each mismatch fix, add a unit test that fails on the old behavior and passes on the new behavior.
   - Prefer tests that exercise the tool-dispatch parsing path, not just helper functions, so the contract is locked end-to-end.
   - If adding fixtures or example data files is necessary, place them under `test/` (for example, `test/gen3d_contract/`).

6. Validate.

   - Run `cargo test` and ensure it passes.
   - Run the rendered smoke test (per `AGENTS.md`):

     tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

     Expect: the game starts, renders briefly, and exits without a crash.

7. Commit frequently.

   - After each logically independent tool contract fix (plus its unit test), commit with a message that names the tool id(s) and the contract change, for example:
     - `gen3d: accept numeric component refs in copy_component_v1`
     - `gen3d: fix copy_from_workspace_v1 prompt args (components required)`


## Validation and Acceptance

Acceptance is met when:

- The agent prompt tool list (`src/gen3d/ai/agent_prompt.rs::build_agent_user_text`) advertises tool arg signatures that match actual tool parsing behavior for all audited tools.
- Tool calls that previously failed due to mismatch now succeed deterministically. At minimum, the following shapes must work:
  - `copy_component_v1` with `{"source_component": 0, "targets": [1], "mode": "linked"}` (or an equivalent target form).
  - `detach_component_v1` with `{"component": 1}`.
  - `copy_component_subtree_v1` with `{"source_root": 0, "targets": [2]}`.
  - `copy_from_workspace_v1` rejects missing `components` and the prompt contract reflects that requiredness.
- `cargo test` passes.
- The rendered smoke test runs without crashing.


## Idempotence and Recovery

- All changes should be safe to re-run: if a tool already supports the contract, reapplying the same patch should be a no-op aside from formatting.
- If any tool contract widening introduces ambiguity or breaks a safety gate, revert that commit and adjust the prompt contract instead (document the decision here in `Decision Log`).
- If the smoke test fails after a change, revert to the last known-good commit and bisect within this series of small commits.


## Interfaces and Dependencies

No new external dependencies are required. Prefer:

- `serde` with `#[serde(untagged)]` enums or small parsing helpers for `string|number` values,
- `#[serde(deny_unknown_fields)]` for strict tools, and
- explicit, actionable error strings when rejecting args.

Do not introduce any intent-dependent heuristics (for example, “guess the target from naming patterns”). All parsing must be deterministic and only based on the provided args and current engine state.


## Plan change notes

- (2026-03-24) Initial draft created from a first-pass mismatch review of Gen3D tool descriptors vs `agent_tool_dispatch` parsing. The audit step is intentionally first because more mismatches are likely.
- (2026-03-24) Added a concrete “Target contracts” section that specifies intended end-state behavior for the known mismatched tools, including specific file edit locations and preferred deterministic parsing patterns.
