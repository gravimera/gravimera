use serde::Serialize;

pub(crate) const TOOL_ID_LIST: &str = "list_tools_v1";

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
                one_line_summary: "Returns a structured summary of components/attachments/anchors.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_QUERY_COMPONENT_PARTS,
                title: "Query component parts",
                one_line_summary: "Lists part ids and transforms for a component (bounded).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_VALIDATE,
                title: "Validate draft",
                one_line_summary: "Runs deterministic structural validations and returns issues.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SMOKE_CHECK,
                title: "Smoke check",
                one_line_summary: "Runs lightweight behavioral checks (bounded, deterministic).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_QA,
                title: "QA",
                one_line_summary: "Runs validate_v1 + smoke_check_v1 and returns a combined summary.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LIST_RUN_ARTIFACTS,
                title: "List run artifacts",
                one_line_summary: "Lists bounded artifacts for this run (scoped; no arbitrary paths).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_READ_ARTIFACT,
                title: "Read artifact",
                one_line_summary: "Reads a bounded slice of a run artifact (scoped; no traversal).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SEARCH_ARTIFACTS,
                title: "Search artifacts",
                one_line_summary: "Searches run artifacts for a substring (scoped; bounded).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_APPLY_DRAFT_OPS,
                title: "Apply draft ops",
                one_line_summary: "Applies deterministic edit operations to the current draft.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SNAPSHOT,
                title: "Snapshot",
                one_line_summary: "Captures a named snapshot of the current Gen3D session state.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LIST_SNAPSHOTS,
                title: "List snapshots",
                one_line_summary: "Lists available in-session snapshots.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DIFF_SNAPSHOTS,
                title: "Diff snapshots",
                one_line_summary: "Returns a structured diff between two snapshots.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_RESTORE_SNAPSHOT,
                title: "Restore snapshot",
                one_line_summary: "Restores the current session state from a snapshot.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_COMPONENT,
                title: "Copy component",
                one_line_summary: "Copies one component into other planned components (no regen).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MIRROR_COMPONENT,
                title: "Mirror component",
                one_line_summary: "Mirrors a component into a new one (L/R symmetry; no regen).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_COMPONENT_SUBTREE,
                title: "Copy component subtree",
                one_line_summary: "Copies a component + descendants into other planned subtrees.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MIRROR_COMPONENT_SUBTREE,
                title: "Mirror component subtree",
                one_line_summary: "Mirrors a component subtree (L/R symmetric limb chains).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DETACH_COMPONENT,
                title: "Detach component copy",
                one_line_summary:
                    "Materializes a linked component copy into real geometry so it can diverge.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_PLAN,
                title: "LLM: generate plan",
                one_line_summary: "Calls the model to produce a component plan, then applies it.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENT,
                title: "LLM: generate component",
                one_line_summary: "Calls the model to generate one component draft, then applies it.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENTS,
                title: "LLM: generate components (batch)",
                one_line_summary: "Generates multiple components in parallel; applies deterministically.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_MOTION_AUTHORING,
                title: "LLM: generate motion authoring",
                one_line_summary: "Bakes explicit animation clips onto attachment edges.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_REVIEW_DELTA,
                title: "LLM: review delta",
                one_line_summary:
                    "Calls the model to propose deterministic tweak ops or targeted regen indices.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_RENDER_PREVIEW,
                title: "Render preview",
                one_line_summary: "Renders deterministic preview images to the run cache.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_CREATE_WORKSPACE,
                title: "Create workspace",
                one_line_summary: "Creates a new workspace cloned from an existing workspace.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DELETE_WORKSPACE,
                title: "Delete workspace",
                one_line_summary: "Deletes a workspace (not the active workspace).",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SET_ACTIVE_WORKSPACE,
                title: "Set active workspace",
                one_line_summary: "Switches which workspace is shown in the preview scene.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DIFF_WORKSPACES,
                title: "Diff workspaces",
                one_line_summary: "Returns a structured diff between two workspaces.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_FROM_WORKSPACE,
                title: "Copy from workspace",
                one_line_summary:
                    "Cherry-picks component(s) or subtrees from another workspace into the active one.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MERGE_WORKSPACE,
                title: "Merge workspaces",
                one_line_summary: "Performs a deterministic 3-way merge into a new workspace.",
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SUBMIT_TOOLING_FEEDBACK,
                title: "Submit tooling feedback",
                one_line_summary: "Records tool feedback (missing tools/bugs/enhancements).",
            },
        ];
        out.sort_by(|a, b| a.tool_id.cmp(b.tool_id));
        out
    }
}
