use serde::Serialize;

pub(crate) const TOOL_ID_LIST: &str = "list_tools_v1";
pub(crate) const TOOL_ID_DESCRIBE: &str = "describe_tool_v1";

pub(crate) const TOOL_ID_GET_USER_INPUTS: &str = "get_user_inputs_v1";
pub(crate) const TOOL_ID_GET_STATE_SUMMARY: &str = "get_state_summary_v1";
pub(crate) const TOOL_ID_GET_SCENE_GRAPH_SUMMARY: &str = "get_scene_graph_summary_v1";
pub(crate) const TOOL_ID_VALIDATE: &str = "validate_v1";
pub(crate) const TOOL_ID_SMOKE_CHECK: &str = "smoke_check_v1";
pub(crate) const TOOL_ID_COPY_COMPONENT: &str = "copy_component_v1";
pub(crate) const TOOL_ID_COPY_COMPONENT_SUBTREE: &str = "copy_component_subtree_v1";
pub(crate) const TOOL_ID_DETACH_COMPONENT: &str = "detach_component_v1";

pub(crate) const TOOL_ID_LLM_GENERATE_PLAN: &str = "llm_generate_plan_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_COMPONENT: &str = "llm_generate_component_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_COMPONENTS: &str = "llm_generate_components_v1";
pub(crate) const TOOL_ID_LLM_REVIEW_DELTA: &str = "llm_review_delta_v1";

pub(crate) const TOOL_ID_RENDER_PREVIEW: &str = "render_preview_v1";
pub(crate) const TOOL_ID_CREATE_WORKSPACE: &str = "create_workspace_v1";
pub(crate) const TOOL_ID_DELETE_WORKSPACE: &str = "delete_workspace_v1";
pub(crate) const TOOL_ID_SET_ACTIVE_WORKSPACE: &str = "set_active_workspace_v1";
pub(crate) const TOOL_ID_SUBMIT_TOOLING_FEEDBACK: &str = "submit_tooling_feedback_v1";

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gen3dToolDescriptorV1 {
    pub(crate) tool_id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) one_line_summary: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gen3dToolDescriptionV1 {
    pub(crate) tool_id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) one_line_summary: &'static str,
    pub(crate) description: &'static str,
    pub(crate) args_example: serde_json::Value,
    pub(crate) result_example: serde_json::Value,
}

#[derive(Default)]
pub(crate) struct Gen3dToolRegistryV1;

impl Gen3dToolRegistryV1 {
    pub(crate) fn list(&self) -> Vec<Gen3dToolDescriptorV1> {
        let mut out = vec![
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LIST,
                title: "List tools",
                one_line_summary: "Lists available Gen3D agent tools.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DESCRIBE,
                title: "Describe tool",
                one_line_summary: "Returns the schema/description for a tool_id.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_USER_INPUTS,
                title: "Get user inputs",
                one_line_summary: "Returns the prompt text and cached input images for this run.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_STATE_SUMMARY,
                title: "Get state summary",
                one_line_summary: "Returns a compact summary of current plan/draft/budget state.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_SCENE_GRAPH_SUMMARY,
                title: "Get scene graph summary",
                one_line_summary: "Returns a structured summary of the current Gen3D draft scene graph.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_VALIDATE,
                title: "Validate draft",
                one_line_summary: "Runs structural validations and returns issues.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SMOKE_CHECK,
                title: "Smoke check",
                one_line_summary: "Runs lightweight behavioral checks based on prompt and current draft.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_COMPONENT,
                title: "Copy component",
                one_line_summary: "Copies a generated component into other planned components (mirrored/repeated parts; saves LLM calls).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_COMPONENT_SUBTREE,
                title: "Copy component subtree",
                one_line_summary:
                    "Copies a generated component subtree into other planned subtrees (symmetric limb chains; saves LLM calls).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DETACH_COMPONENT,
                title: "Detach component copy",
                one_line_summary: "Materializes a linked component copy into real geometry so it can diverge.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_PLAN,
                title: "LLM: generate plan",
                one_line_summary: "Calls the model to produce a component plan, then applies it to the draft.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENT,
                title: "LLM: generate component",
                one_line_summary: "Calls the model to generate one component draft, then applies it to the draft.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENTS,
                title: "LLM: generate components (batch)",
                one_line_summary:
                    "Generates multiple components in parallel and applies them deterministically to the draft.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_REVIEW_DELTA,
                title: "LLM: review delta",
                one_line_summary: "Calls the model to review the current draft and return tweak/regenerate actions.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_RENDER_PREVIEW,
                title: "Render preview",
                one_line_summary: "Renders the current draft from specified angles and writes PNGs to the cache.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_CREATE_WORKSPACE,
                title: "Create workspace",
                one_line_summary: "Creates a new preview workspace cloned from an existing workspace.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DELETE_WORKSPACE,
                title: "Delete workspace",
                one_line_summary: "Deletes a preview workspace.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SET_ACTIVE_WORKSPACE,
                title: "Set active workspace",
                one_line_summary: "Switches which workspace is shown in the preview panel.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SUBMIT_TOOLING_FEEDBACK,
                title: "Submit tooling feedback",
                one_line_summary: "Records tool feedback (missing tools/bugs/enhancements) for developers.",
            },
        ];
        out.sort_by(|a, b| a.tool_id.cmp(b.tool_id));
        out
    }

    pub(crate) fn describe(&self, tool_id: &str) -> Option<Gen3dToolDescriptionV1> {
        match tool_id {
            TOOL_ID_LIST => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_LIST,
                title: "List tools",
                one_line_summary: "Lists available Gen3D agent tools.",
                description: "Returns a list of tool descriptors with tool_id and summaries.",
                args_example: serde_json::json!({}),
                result_example: serde_json::json!({
                    "tools": [
                        {"tool_id":"list_tools_v1","title":"List tools","one_line_summary":"..."}
                    ]
                }),
            }),
            TOOL_ID_DESCRIBE => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_DESCRIBE,
                title: "Describe tool",
                one_line_summary: "Returns the schema/description for a tool_id.",
                description: "Pass a tool_id and get a detailed description plus example JSON.",
                args_example: serde_json::json!({ "tool_id": "render_preview_v1" }),
                result_example: serde_json::json!({
                    "tool_id":"render_preview_v1",
                    "title":"Render preview",
                    "one_line_summary":"...",
                    "description":"...",
                    "args_example":{},
                    "result_example":{}
                }),
            }),
            TOOL_ID_GET_USER_INPUTS => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_GET_USER_INPUTS,
                title: "Get user inputs",
                one_line_summary: "Returns the prompt text and cached input images for this run.",
                description:
                    "Use this to fetch the player's prompt and the cached input images (0–6).",
                args_example: serde_json::json!({}),
                result_example: serde_json::json!({
                    "prompt": "A goblin with a spear",
                    "images": [".../attempt_0/inputs/images/img_01.png"],
                }),
            }),
            TOOL_ID_GET_STATE_SUMMARY => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_GET_STATE_SUMMARY,
                title: "Get state summary",
                one_line_summary: "Returns a compact summary of current plan/draft/budget state.",
                description:
                    "Use this to quickly understand where the build is (plan, components generated, budgets).",
                args_example: serde_json::json!({}),
                result_example: serde_json::json!({
                    "run_id": "uuid",
                    "attempt": 0,
                    "pass": 3,
                    "plan_hash": "sha256:...",
                    "components_total": 8,
                    "components_generated": 3,
                    "draft_defs": 5,
                    "tokens_run": 12345,
                }),
            }),
            TOOL_ID_GET_SCENE_GRAPH_SUMMARY => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_GET_SCENE_GRAPH_SUMMARY,
                title: "Get scene graph summary",
                one_line_summary: "Returns a structured summary of the current Gen3D draft scene graph.",
                description: "Returns the same structured info written to `scene_graph_summary.json` during builds.",
                args_example: serde_json::json!({}),
                result_example: serde_json::json!({
                    "version": 1,
                    "root": {"size":[1,1,1]},
                    "components": [{"name":"body","generated":true}],
                }),
            }),
            TOOL_ID_VALIDATE => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_VALIDATE,
                title: "Validate draft",
                one_line_summary: "Runs structural validations and returns issues.",
                description: "Runs engine-side structural checks (anchors, attachments, non-finite transforms, required fields).",
                args_example: serde_json::json!({}),
                result_example: serde_json::json!({
                    "ok": true,
                    "issues": [],
                }),
            }),
            TOOL_ID_SMOKE_CHECK => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_SMOKE_CHECK,
                title: "Smoke check",
                one_line_summary: "Runs lightweight behavioral checks based on prompt and current draft.",
                description: "Checks high-level semantics like: if prompt implies combat, ensure mobility+attack exist.",
                args_example: serde_json::json!({}),
                result_example: serde_json::json!({
                    "ok": true,
                    "issues": [],
                }),
            }),
            TOOL_ID_COPY_COMPONENT => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_COPY_COMPONENT,
                title: "Copy component",
                one_line_summary: "Copies an already-generated component into another planned component (detached or linked).",
                description:
                    "Use this when multiple components should share the same geometry (wheels/legs/pairs).\n\
                     This avoids another `llm_generate_component_v1` call and keeps symmetric pieces consistent.\n\
                     Note: The target component may have a different parent attachment offset and different attachment animations than the source. Copying only affects the target's own geometry (and optionally its anchors), and preserves existing child attachment refs.\n\
                     Modes:\n\
                     - detached: duplicates the primitive parts into the target component.\n\
                     - linked: target becomes a lightweight wrapper that references the source component (SOURCE must be a leaf component; otherwise children would be duplicated).\n\
                     Optional `anchors` controls whether detached copies overwrite anchors:\n\
                     - preserve_target: (default) keep the target anchors unchanged (recommended for symmetric parts and stable join frames).\n\
                     - copy_source: copy source anchors into the target (can change join frames).\n\
                     Note: when anchors=preserve_target, the engine aligns copied geometry to the target's mount anchor so the part follows the target's mount orientation.\n\
                     Optional `transform` applies a local-space delta (pos/rot/scale) to the copied geometry (and to anchors only when anchors=copy_source).",
                args_example: serde_json::json!({
                    "source_component": "wheel_front_left",
                    "targets": ["wheel_front_right","wheel_back_left","wheel_back_right"],
                    "mode": "detached",
                    "anchors": "preserve_target",
                    "transform": { "pos": [0.0, 0.0, 0.0] }
                }),
                result_example: serde_json::json!({
                    "ok": true,
                    "copies": [
                        {"source":"wheel_front_left","target":"wheel_front_right","mode":"detached"}
                    ],
                }),
            }),
            TOOL_ID_COPY_COMPONENT_SUBTREE => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_COPY_COMPONENT_SUBTREE,
                title: "Copy component subtree",
                one_line_summary:
                    "Copies a generated component subtree (root + descendants) into another planned subtree.",
                description:
                    "Use this for symmetric limb chains (legs/arms) where a root component has attached descendants.\n\
                     It structurally matches the source and target subtrees by attachment edge keys (parent_anchor, child_anchor) and copies geometry for each matched pair.\n\
                     If the TARGET subtree is missing descendants, this tool expands the target subtree by cloning the missing branches from the source subtree into new planned components, then copies geometry.\n\
                     By default it preserves TARGET anchors so each subtree keeps its mount interface and join frames stable.\n\
                     Use anchors=copy_source only when you want to overwrite the TARGET anchors to match the SOURCE exactly.\n\
                     Note: when anchors=preserve_target, the engine aligns copied geometry to each target component's mount anchor so limb chains follow the target mounts (useful for radial limbs).\n\
                     This is a convenience tool that avoids many repeated `copy_component_v1` calls.\n\
                     Args:\n\
                     - source_root: source component (name or index)\n\
                     - targets: list of target root components (names or indices)\n\
                     - mode: detached (only supported today)\n\
                     - anchors: preserve_target (default) or copy_source\n\
                     - transform: optional delta applied to copied geometry.",
                args_example: serde_json::json!({
                    "source_root": "leg_front_left",
                    "targets": ["leg_front_right","leg_back_left","leg_back_right"],
                    "mode": "detached",
                    "anchors": "preserve_target"
                }),
                result_example: serde_json::json!({
                    "ok": true,
                    "copies": [
                        {"source":"leg_front_left","target":"leg_front_right","mode":"detached"},
                        {"source":"foot_front_left","target":"foot_front_right","mode":"detached"}
                    ],
                }),
            }),
            TOOL_ID_DETACH_COMPONENT => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_DETACH_COMPONENT,
                title: "Detach component copy",
                one_line_summary: "Materializes a linked component copy into real geometry so it can diverge.",
                description:
                    "If a component was created as a linked copy (via `copy_component_v1` with mode=linked), it shares geometry with its source.\n\
                     Use this tool to materialize the current linked geometry into real primitives inside the target component so it can be regenerated or tweaked independently.",
                args_example: serde_json::json!({
                    "component": "wheel_front_right"
                }),
                result_example: serde_json::json!({
                    "ok": true,
                    "component": "wheel_front_right",
                    "mode": "detached"
                }),
            }),
            TOOL_ID_LLM_GENERATE_PLAN => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_LLM_GENERATE_PLAN,
                title: "LLM: generate plan",
                one_line_summary: "Calls the model to produce a component plan, then applies it to the draft.",
                description: "Use this to (re)generate a component plan. The engine parses and applies the plan into a draft root with component stubs.",
                args_example: serde_json::json!({}),
                result_example: serde_json::json!({
                    "ok": true,
                    "components_total": 8,
                    "plan_hash": "sha256:...",
                }),
            }),
            TOOL_ID_LLM_GENERATE_COMPONENT => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENT,
                title: "LLM: generate component",
                one_line_summary: "Calls the model to generate one component draft, then applies it to the draft.",
                description:
                    "Generates a single component from the current plan and applies it into the draft.\n\
                     Provide either `component_index` (preferred) or a name hint.\n\
                     Name hints are matched loosely (snake_case / case-insensitive) against the current plan component names.\n\
                     Accepted name fields: `component_name`, `component_id`, `component`, `name`.",
                args_example: serde_json::json!({ "component_index": 0 }),
                result_example: serde_json::json!({
                    "ok": true,
                    "component_name": "left_leg",
                    "parts": 12,
                }),
            }),
            TOOL_ID_LLM_GENERATE_COMPONENTS => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENTS,
                title: "LLM: generate components (batch)",
                one_line_summary:
                    "Generates multiple components in parallel and applies them deterministically to the draft.",
                description:
                    "Generates multiple components from the current plan in parallel, bounded by config `gen3d.max_parallel_components`, and applies the results deterministically to the shared draft.\n\
                     If you do not provide any indices/names, it generates all missing components by default.\n\
                     Args:\n\
                     - `component_indices`: 0-based indices (optional)\n\
                     - `component_names`: name hints (optional)\n\
                     - `missing_only`: bool (optional; default true when no explicit indices/names)\n\
                     - `force`: bool (optional; if true, regenerate even if already generated).",
                args_example: serde_json::json!({ "missing_only": true }),
                result_example: serde_json::json!({
                    "requested": 6,
                    "succeeded": 6,
                    "failed": [],
                }),
            }),
            TOOL_ID_LLM_REVIEW_DELTA => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_LLM_REVIEW_DELTA,
                title: "LLM: review delta",
                one_line_summary: "Calls the model to review the current draft and return tweak/regenerate actions.",
                description: "Generates and applies a review delta (tweak transforms/anchors/attachments, request regen/replan).",
                args_example: serde_json::json!({}),
                result_example: serde_json::json!({
                    "ok": true,
                    "accepted": false,
                    "had_actions": true,
                }),
            }),
            TOOL_ID_RENDER_PREVIEW => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_RENDER_PREVIEW,
                title: "Render preview",
                one_line_summary: "Renders the current draft from specified angles and writes PNGs to the cache.",
                description:
                    "Renders the current draft to PNGs using the Gen3D preview scene. Use for agent self-review.\n\
                     Views are selected by `views` (preferred) or `angles` (alias).\n\
                     Resolution can be specified as `resolution` (square) or `width`+`height`.\n\
                     For convenience, you can also pass `image_size` (or `image_size_px`) as the maximum dimension; it scales the default 16:9 capture size.",
                args_example: serde_json::json!({
                    "views": ["front", "front_left", "left_back", "right_back", "top", "bottom"],
                    "image_size": 768,
                    "overlay": "axes_grid",
                    "background": "neutral_studio",
                    "prefix": "review",
                }),
                result_example: serde_json::json!({
                    "images": [".../review_front.png", ".../review_top.png"]
                }),
            }),
            TOOL_ID_CREATE_WORKSPACE => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_CREATE_WORKSPACE,
                title: "Create workspace",
                one_line_summary: "Creates a new preview workspace cloned from an existing workspace.",
                description:
                    "Creates a new preview workspace cloned from an existing workspace.\n\
                     Use this to try alternative assembly/edits without overwriting the active workspace.\n\
                     Tip: if you want to `set_active_workspace_v1` in the SAME step, pass a stable `workspace_id` (or `name`) so you can refer to it immediately.",
                args_example: serde_json::json!({ "workspace_id": "alt", "from": "main" }),
                result_example: serde_json::json!({ "workspace_id": "alt" }),
            }),
            TOOL_ID_DELETE_WORKSPACE => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_DELETE_WORKSPACE,
                title: "Delete workspace",
                one_line_summary: "Deletes a preview workspace.",
                description: "Deletes a non-active workspace. Deleting the active workspace is an error.",
                args_example: serde_json::json!({ "workspace_id": "alt" }),
                result_example: serde_json::json!({ "ok": true }),
            }),
            TOOL_ID_SET_ACTIVE_WORKSPACE => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_SET_ACTIVE_WORKSPACE,
                title: "Set active workspace",
                one_line_summary: "Switches which workspace is shown in the preview panel.",
                description: "Switches the preview panel to show a specific workspace draft.",
                args_example: serde_json::json!({ "workspace_id": "main" }),
                result_example: serde_json::json!({ "ok": true }),
            }),
            TOOL_ID_SUBMIT_TOOLING_FEEDBACK => Some(Gen3dToolDescriptionV1 {
                tool_id: TOOL_ID_SUBMIT_TOOLING_FEEDBACK,
                title: "Submit tooling feedback",
                one_line_summary: "Records tool feedback (missing tools/bugs/enhancements) for developers.",
                description:
                    "Use this to record structured feedback about missing tools, tool bugs, or desired enhancements. Entries persist across restarts and appear in the Tool Feedback UI tab.",
                args_example: serde_json::json!({
                    "version": 1,
                    "priority": "medium",
                    "title": "Need anchor overlay tool",
                    "summary": "I need a tool to render anchors and attachment frames to debug placement.",
                    "details": {
                        "missing_tools": ["render_preview_v1.overlay = anchors"],
                        "example": "A chair leg appears rotated; an anchor overlay would help.",
                    }
                }),
                result_example: serde_json::json!({ "ok": true, "entry_ids": ["uuid"] }),
            }),
            _ => None,
        }
    }
}
