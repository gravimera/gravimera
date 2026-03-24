// Gen3D tool ids (shared by the deterministic pipeline orchestrator and tool runtime).
//
// Note: the legacy agent-step orchestrator and its prompt-facing tool registry were removed.

pub(crate) const TOOL_ID_GET_TOOL_DETAIL: &str = "get_tool_detail_v1";
pub(crate) const TOOL_ID_BASIS_FROM_UP_FORWARD: &str = "basis_from_up_forward_v1";

pub(crate) const TOOL_ID_GET_USER_INPUTS: &str = "get_user_inputs_v2";
pub(crate) const TOOL_ID_GET_STATE_SUMMARY: &str = "get_state_summary_v1";
pub(crate) const TOOL_ID_GET_SCENE_GRAPH_SUMMARY: &str = "get_scene_graph_summary_v1";
pub(crate) const TOOL_ID_INSPECT_PLAN: &str = "inspect_plan_v1";
pub(crate) const TOOL_ID_GET_PLAN_TEMPLATE: &str = "get_plan_template_v1";
pub(crate) const TOOL_ID_SET_DESCRIPTOR_META: &str = "set_descriptor_meta_v1";
pub(crate) const TOOL_ID_QUERY_COMPONENT_PARTS: &str = "query_component_parts_v1";
pub(crate) const TOOL_ID_VALIDATE: &str = "validate_v1";
pub(crate) const TOOL_ID_SMOKE_CHECK: &str = "smoke_check_v1";
pub(crate) const TOOL_ID_MOTION_METRICS: &str = "motion_metrics_v1";
pub(crate) const TOOL_ID_QA: &str = "qa_v1";

pub(crate) const TOOL_ID_INFO_KV_LIST_KEYS: &str = "info_kv_list_keys_v1";
pub(crate) const TOOL_ID_INFO_KV_LIST_HISTORY: &str = "info_kv_list_history_v1";
pub(crate) const TOOL_ID_INFO_KV_GET: &str = "info_kv_get_v1";
pub(crate) const TOOL_ID_INFO_KV_GET_PAGED: &str = "info_kv_get_paged_v1";
pub(crate) const TOOL_ID_INFO_KV_GET_MANY: &str = "info_kv_get_many_v1";
pub(crate) const TOOL_ID_INFO_EVENTS_LIST: &str = "info_events_list_v1";
pub(crate) const TOOL_ID_INFO_EVENTS_GET: &str = "info_events_get_v1";
pub(crate) const TOOL_ID_INFO_EVENTS_SEARCH: &str = "info_events_search_v1";
pub(crate) const TOOL_ID_INFO_BLOBS_LIST: &str = "info_blobs_list_v1";
pub(crate) const TOOL_ID_INFO_BLOBS_GET: &str = "info_blobs_get_v1";
pub(crate) const TOOL_ID_APPLY_DRAFT_OPS: &str = "apply_draft_ops_v1";
pub(crate) const TOOL_ID_APPLY_LAST_DRAFT_OPS: &str = "apply_last_draft_ops_v1";
pub(crate) const TOOL_ID_APPLY_DRAFT_OPS_FROM_EVENT: &str = "apply_draft_ops_from_event_v1";
pub(crate) const TOOL_ID_APPLY_PLAN_OPS: &str = "apply_plan_ops_v1";
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
pub(crate) const TOOL_ID_LLM_GENERATE_PLAN_OPS: &str = "llm_generate_plan_ops_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_DRAFT_OPS: &str = "llm_generate_draft_ops_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_COMPONENT: &str = "llm_generate_component_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_COMPONENTS: &str = "llm_generate_components_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_MOTION: &str = "llm_generate_motion_v1";
pub(crate) const TOOL_ID_LLM_GENERATE_MOTIONS: &str = "llm_generate_motions_v1";
pub(crate) const TOOL_ID_LLM_REVIEW_DELTA: &str = "llm_review_delta_v1";

pub(crate) const TOOL_ID_RENDER_PREVIEW: &str = "render_preview_v1";
pub(crate) const TOOL_ID_CREATE_WORKSPACE: &str = "create_workspace_v1";
pub(crate) const TOOL_ID_DELETE_WORKSPACE: &str = "delete_workspace_v1";
pub(crate) const TOOL_ID_SET_ACTIVE_WORKSPACE: &str = "set_active_workspace_v1";
pub(crate) const TOOL_ID_DIFF_WORKSPACES: &str = "diff_workspaces_v1";
pub(crate) const TOOL_ID_COPY_FROM_WORKSPACE: &str = "copy_from_workspace_v1";
pub(crate) const TOOL_ID_MERGE_WORKSPACE: &str = "merge_workspace_v1";
pub(crate) const TOOL_ID_SUBMIT_TOOLING_FEEDBACK: &str = "submit_tooling_feedback_v1";

