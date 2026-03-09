use serde::Serialize;
use serde_json::Value;

pub(crate) const TOOL_ID_GET_TOOLS_DETAIL: &str = "get_tools_detail_v1";

pub(crate) const TOOL_ID_GET_USER_INPUTS: &str = "get_user_inputs_v1";
pub(crate) const TOOL_ID_GET_STATE_SUMMARY: &str = "get_state_summary_v1";
pub(crate) const TOOL_ID_GET_SCENE_GRAPH_SUMMARY: &str = "get_scene_graph_summary_v1";
pub(crate) const TOOL_ID_QUERY_COMPONENT_PARTS: &str = "query_component_parts_v1";
pub(crate) const TOOL_ID_VALIDATE: &str = "validate_v1";
pub(crate) const TOOL_ID_SMOKE_CHECK: &str = "smoke_check_v1";
pub(crate) const TOOL_ID_QA: &str = "qa_v1";
pub(crate) const TOOL_ID_LIST_RUN_ARTIFACTS: &str = "list_run_artifacts_v1";
pub(crate) const TOOL_ID_READ_ARTIFACT: &str = "read_artifact_v1";
pub(crate) const TOOL_ID_SEARCH_ARTIFACTS: &str = "search_artifacts_v1";
pub(crate) const TOOL_ID_APPLY_DRAFT_OPS: &str = "apply_draft_ops_v1";
pub(crate) const TOOL_ID_SNAPSHOT: &str = "snapshot_v1";
pub(crate) const TOOL_ID_LIST_SNAPSHOTS: &str = "list_snapshots_v1";
pub(crate) const TOOL_ID_DIFF_SNAPSHOTS: &str = "diff_snapshots_v1";
pub(crate) const TOOL_ID_RESTORE_SNAPSHOT: &str = "restore_snapshot_v1";
pub(crate) const TOOL_ID_COPY_COMPONENT: &str = "copy_component_v1";
pub(crate) const TOOL_ID_MIRROR_COMPONENT: &str = "mirror_component_v1";
pub(crate) const TOOL_ID_COPY_COMPONENT_SUBTREE: &str = "copy_component_subtree_v1";
pub(crate) const TOOL_ID_MIRROR_COMPONENT_SUBTREE: &str = "mirror_component_subtree_v1";
pub(crate) const TOOL_ID_DETACH_COMPONENT: &str = "detach_component_v1";

pub(crate) const TOOL_ID_LLM_GENERATE_PLAN: &str = "llm_generate_plan_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_COMPONENT: &str = "llm_generate_component_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_COMPONENTS: &str = "llm_generate_components_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_MOTION_AUTHORING: &str = "llm_generate_motion_authoring_v1";
pub(crate) const TOOL_ID_LLM_REVIEW_DELTA: &str = "llm_review_delta_v1";

pub(crate) const TOOL_ID_RENDER_PREVIEW: &str = "render_preview_v1";
pub(crate) const TOOL_ID_CREATE_WORKSPACE: &str = "create_workspace_v1";
pub(crate) const TOOL_ID_DELETE_WORKSPACE: &str = "delete_workspace_v1";
pub(crate) const TOOL_ID_SET_ACTIVE_WORKSPACE: &str = "set_active_workspace_v1";
pub(crate) const TOOL_ID_DIFF_WORKSPACES: &str = "diff_workspaces_v1";
pub(crate) const TOOL_ID_COPY_FROM_WORKSPACE: &str = "copy_from_workspace_v1";
pub(crate) const TOOL_ID_MERGE_WORKSPACE: &str = "merge_workspace_v1";
pub(crate) const TOOL_ID_SUBMIT_TOOLING_FEEDBACK: &str = "submit_tooling_feedback_v1";

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gen3dToolDescriptorV1 {
    pub(crate) tool_id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) one_line_summary: &'static str,
    pub(crate) args_schema: &'static str,
    pub(crate) args_example: Value,
}

#[derive(Default)]
pub(crate) struct Gen3dToolRegistryV1;

impl Gen3dToolRegistryV1 {
    pub(crate) fn list(&self) -> Vec<Gen3dToolDescriptorV1> {
        let mut out = vec![
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_TOOLS_DETAIL,
                title: "Get tools detail",
                one_line_summary: "Returns args_schema/args_example for selected tools.",
                args_schema: "{ tool_ids: string[] }",
                args_example: serde_json::json!({ "tool_ids": ["apply_draft_ops_v1", "llm_review_delta_v1"] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_USER_INPUTS,
                title: "Get user inputs",
                one_line_summary: "Returns the prompt text and cached input images for this run.",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_STATE_SUMMARY,
                title: "Get state summary",
                one_line_summary: "Returns a compact summary of current plan/draft/budget state.",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_SCENE_GRAPH_SUMMARY,
                title: "Get scene graph summary",
                one_line_summary: "Returns a structured summary of components/attachments/anchors.",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_QUERY_COMPONENT_PARTS,
                title: "Query component parts",
                one_line_summary: "Lists part ids and transforms for a component (bounded).",
                args_schema:
                    "{ version?: 1, component?: string, component_index?: number, include_non_primitives?: bool, max_parts?: number }",
                args_example: serde_json::json!({ "component": "torso", "max_parts": 128 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_VALIDATE,
                title: "Validate draft",
                one_line_summary: "Runs deterministic structural validations and returns issues.",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SMOKE_CHECK,
                title: "Smoke check",
                one_line_summary: "Runs lightweight behavioral checks (bounded, deterministic).",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_QA,
                title: "QA",
                one_line_summary: "Runs validate_v1 + smoke_check_v1 and returns a combined summary.",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LIST_RUN_ARTIFACTS,
                title: "List run artifacts",
                one_line_summary: "Lists bounded artifacts for this run (scoped; no arbitrary paths).",
                args_schema: "{ path_prefix?: string, max_items?: number }",
                args_example: serde_json::json!({ "path_prefix": "attempt_0/", "max_items": 200 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_READ_ARTIFACT,
                title: "Read artifact",
                one_line_summary: "Reads a bounded slice of a run artifact (scoped; no traversal).",
                args_schema:
                    "{ artifact_ref: string, max_bytes?: number, tail_lines?: number, json_pointer?: string }",
                args_example: serde_json::json!({ "artifact_ref": "attempt_0/pass_0/gravimera.log", "tail_lines": 200 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SEARCH_ARTIFACTS,
                title: "Search artifacts",
                one_line_summary: "Searches run artifacts for a substring (scoped; bounded).",
                args_schema:
                    "{ query: string, path_prefix?: string, max_matches?: number, max_bytes_per_file?: number }",
                args_example: serde_json::json!({ "query": "ERROR", "path_prefix": "attempt_0/" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_APPLY_DRAFT_OPS,
                title: "Apply draft ops",
                one_line_summary: "Applies deterministic edit operations to the current draft.",
                args_schema:
                    "{ version?: 1, atomic?: bool, if_assembly_rev?: number, ops: DraftOp[] }",
                args_example: serde_json::json!({ "atomic": true, "ops": [] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SNAPSHOT,
                title: "Snapshot",
                one_line_summary:
                    "Captures a snapshot of the current Gen3D session state (args: {label?}).",
                args_schema: "{ version?: 1, snapshot_id?: string, label?: string }",
                args_example: serde_json::json!({ "label": "pre_leg_fix_rev16" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LIST_SNAPSHOTS,
                title: "List snapshots",
                one_line_summary: "Lists available in-session snapshots.",
                args_schema: "{ version?: 1, max_items?: number }",
                args_example: serde_json::json!({ "max_items": 50 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DIFF_SNAPSHOTS,
                title: "Diff snapshots",
                one_line_summary: "Returns a structured diff between two snapshots.",
                args_schema: "{ version?: 1, a: string, b: string, max_components?: number }",
                args_example: serde_json::json!({ "a": "snap_1", "b": "snap_2" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_RESTORE_SNAPSHOT,
                title: "Restore snapshot",
                one_line_summary: "Restores the current session state from a snapshot.",
                args_schema: "{ version?: 1, snapshot_id: string }",
                args_example: serde_json::json!({ "snapshot_id": "snap_1" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_COMPONENT,
                title: "Copy component",
                one_line_summary: "Copies one component into other planned components (no regen).",
                args_schema:
                    "{ source_component: string|number, targets?: (string|number)[], mode?: \"detached\"|\"linked\", anchors?: string, transform?: TransformDelta }",
                args_example: serde_json::json!({ "source_component": "arm_l_upper", "targets": ["arm_r_upper"], "mode": "linked" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MIRROR_COMPONENT,
                title: "Mirror component",
                one_line_summary: "Mirrors a component into a new one (L/R symmetry; no regen).",
                args_schema:
                    "{ source_component: string|number, targets?: (string|number)[], mode?: \"detached\"|\"linked\", anchors?: string, transform?: TransformDelta }",
                args_example: serde_json::json!({ "source_component": "arm_l_upper", "targets": ["arm_r_upper"], "mode": "detached" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_COMPONENT_SUBTREE,
                title: "Copy component subtree",
                one_line_summary: "Copies a component + descendants into other planned subtrees.",
                args_schema:
                    "{ source_root: string|number, targets: (string|number)[], mode?: \"detached\", anchors?: string, transform?: TransformDelta }",
                args_example: serde_json::json!({ "source_root": "leg_l_thigh", "targets": ["leg_r_thigh"] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MIRROR_COMPONENT_SUBTREE,
                title: "Mirror component subtree",
                one_line_summary: "Mirrors a component subtree (L/R symmetric limb chains).",
                args_schema:
                    "{ source_root: string|number, targets: (string|number)[], mode?: \"detached\", anchors?: string, transform?: TransformDelta }",
                args_example: serde_json::json!({ "source_root": "leg_l_thigh", "targets": ["leg_r_thigh"] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DETACH_COMPONENT,
                title: "Detach component copy",
                one_line_summary:
                    "Materializes a linked component copy into real geometry so it can diverge.",
                args_schema: "{ component: string|number }",
                args_example: serde_json::json!({ "component": "arm_r_upper" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_PLAN,
                title: "LLM: generate plan",
                one_line_summary: "Calls the model to produce a component plan, then applies it.",
                args_schema:
                    "{ prompt?: string, style?: string, constraints?: { preserve_existing_components?: bool }, components?: string[] }",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENT,
                title: "LLM: generate component",
                one_line_summary: "Calls the model to generate one component draft, then applies it.",
                args_schema:
                    "{ component_name?: string, component_index?: number, force?: bool }",
                args_example: serde_json::json!({ "component_name": "leg_l_thigh" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENTS,
                title: "LLM: generate components (batch)",
                one_line_summary: "Generates multiple components in parallel; applies deterministically.",
                args_schema:
                    "{ component_indices?: number[], component_names?: string[], missing_only?: bool, force?: bool }",
                args_example: serde_json::json!({ "missing_only": true }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_MOTION_AUTHORING,
                title: "LLM: generate motion authoring",
                one_line_summary: "Bakes explicit animation clips onto attachment edges.",
                args_schema: "{ prompt?: string }",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_REVIEW_DELTA,
                title: "LLM: review delta",
                one_line_summary:
                    "Calls the model to propose deterministic tweak ops or targeted regen indices.",
                args_schema:
                    "{ preview_images?: string[], include_original_images?: bool }",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_RENDER_PREVIEW,
                title: "Render preview",
                one_line_summary: "Renders deterministic preview images to the run cache.",
                args_schema:
                    "{ views?: string[], image_size?: number, resolution?: number, width?: number, height?: number, overlay?: string, prefix?: string, include_motion_sheets?: bool }",
                args_example: serde_json::json!({ "views": ["front", "left_back", "right_back", "top", "bottom"], "image_size": 768 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_CREATE_WORKSPACE,
                title: "Create workspace",
                one_line_summary: "Creates a new workspace cloned from an existing workspace.",
                args_schema:
                    "{ from?: string, name?: string, workspace_id?: string, include_components?: string[] }",
                args_example: serde_json::json!({ "workspace_id": "ws_fixlegs", "from": "main" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DELETE_WORKSPACE,
                title: "Delete workspace",
                one_line_summary: "Deletes a workspace (not the active workspace).",
                args_schema: "{ workspace_id: string }",
                args_example: serde_json::json!({ "workspace_id": "ws_fixlegs" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SET_ACTIVE_WORKSPACE,
                title: "Set active workspace",
                one_line_summary: "Switches which workspace is shown in the preview scene.",
                args_schema: "{ workspace_id: string }",
                args_example: serde_json::json!({ "workspace_id": "main" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DIFF_WORKSPACES,
                title: "Diff workspaces",
                one_line_summary: "Returns a structured diff between two workspaces.",
                args_schema: "{ version?: 1, a?: string, b?: string, max_components?: number }",
                args_example: serde_json::json!({ "a": "main", "b": "ws_fixlegs" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_FROM_WORKSPACE,
                title: "Copy from workspace",
                one_line_summary:
                    "Cherry-picks component(s) or subtrees from another workspace into the active one.",
                args_schema:
                    "{ version?: 1, from: string, components?: string[], mode?: string, include_attachment?: bool }",
                args_example: serde_json::json!({ "from": "ws_fixlegs", "components": ["leg_l_thigh", "leg_r_thigh"] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MERGE_WORKSPACE,
                title: "Merge workspaces",
                one_line_summary: "Performs a deterministic 3-way merge into a new workspace.",
                args_schema:
                    "{ version?: 1, base: string, a: string, b: string, output_workspace_id?: string, output_name?: string, max_components?: number }",
                args_example: serde_json::json!({ "base": "main", "a": "ws_a", "b": "ws_b", "output_workspace_id": "ws_merge" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SUBMIT_TOOLING_FEEDBACK,
                title: "Submit tooling feedback",
                one_line_summary: "Records tool feedback (missing tools/bugs/enhancements).",
                args_schema:
                    "{ version?: 1, priority: \"low|medium|high|blocker\", title: string, summary: string, details?: any }",
                args_example: serde_json::json!({ "version": 1, "priority": "low", "title": "Example", "summary": "Tool docs could include args examples." }),
            },
        ];
        out.sort_by(|a, b| a.tool_id.cmp(b.tool_id));
        out
    }
}
