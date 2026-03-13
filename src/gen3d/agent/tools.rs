use serde::Serialize;
use serde_json::Value;

pub(crate) const TOOL_ID_GET_TOOL_DETAIL: &str = "get_tool_detail_v1";

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
pub(crate) const TOOL_ID_SUGGEST_MOTION_REPAIRS: &str = "suggest_motion_repairs_v1";
pub(crate) const TOOL_ID_QA: &str = "qa_v1";

pub(crate) const TOOL_ID_INFO_KV_LIST_KEYS: &str = "info_kv_list_keys_v1";
pub(crate) const TOOL_ID_INFO_KV_LIST_HISTORY: &str = "info_kv_list_history_v1";
pub(crate) const TOOL_ID_INFO_KV_GET: &str = "info_kv_get_v1";
pub(crate) const TOOL_ID_INFO_KV_GET_MANY: &str = "info_kv_get_many_v1";
pub(crate) const TOOL_ID_INFO_EVENTS_LIST: &str = "info_events_list_v1";
pub(crate) const TOOL_ID_INFO_EVENTS_GET: &str = "info_events_get_v1";
pub(crate) const TOOL_ID_INFO_EVENTS_SEARCH: &str = "info_events_search_v1";
pub(crate) const TOOL_ID_INFO_BLOBS_LIST: &str = "info_blobs_list_v1";
pub(crate) const TOOL_ID_INFO_BLOBS_GET: &str = "info_blobs_get_v1";
pub(crate) const TOOL_ID_APPLY_DRAFT_OPS: &str = "apply_draft_ops_v1";
pub(crate) const TOOL_ID_RECENTER_ATTACHMENT_MOTION: &str = "recenter_attachment_motion_v1";
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
                tool_id: TOOL_ID_GET_TOOL_DETAIL,
                title: "Get tool detail",
                one_line_summary:
                    "Read-only: tool introspection (args_schema/args_example) for one tool_id.",
                args_schema: "{ tool_id: string }",
                args_example: serde_json::json!({ "tool_id": "apply_draft_ops_v1" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_USER_INPUTS,
                title: "Get user inputs",
                one_line_summary: "Read-only: returns the user prompt + reference-image main-object summary (no raw image paths; user photos are not sent to the LLM).",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_STATE_SUMMARY,
                title: "Get state summary",
                one_line_summary: "Read-only: compact summary of plan/draft/QA/budgets (for decision-making).",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SET_DESCRIPTOR_META,
                title: "Set descriptor meta",
                one_line_summary: "Mutates session: sets prefab descriptor short name (`name`, <=3 words) + `short` + `tags` for the next Save (seeded edits preserve existing meta unless overridden).",
                args_schema: "{ version?: 1, name?: string, short?: string, tags?: string[] }",
                args_example: serde_json::json!({ "name": "Wooden watchtower", "short": "A wooden watchtower with a narrow staircase.", "tags": ["tower", "wood", "defensive"] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_SCENE_GRAPH_SUMMARY,
                title: "Get scene graph summary",
                one_line_summary: "Read-only: component graph incl. attachments/anchors/resolved transforms (writes `scene_graph_summary.json`).",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INSPECT_PLAN,
                title: "Inspect plan",
                one_line_summary:
                    "Read-only: inspect the last rejected llm_generate_plan_v1 output and return semantic errors + preserve-mode constraints (names/root/policy).",
                args_schema: "{ version?: 1 }",
                args_example: serde_json::json!({ "version": 1 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_GET_PLAN_TEMPLATE,
                title: "Get plan template",
                one_line_summary:
                    "Read-only: write a preserve-mode plan template into the Info Store (KV) for safe copy+edit preserve-mode replans.",
                args_schema: "{ version?: 1 }",
                args_example: serde_json::json!({ "version": 1 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_QUERY_COMPONENT_PARTS,
                title: "Query component parts",
                one_line_summary: "Read-only: list component parts (primitive mesh/color + part_id_uuid + transforms); writes into Info Store KV as `ws.<id>.component_parts.<component>` and returns `info_kv` ref.",
                args_schema:
                    "{ version?: 1, component?: string, component_index?: number, include_non_primitives?: bool, max_parts?: number }",
                args_example: serde_json::json!({ "component": "torso", "max_parts": 128 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_VALIDATE,
                title: "Validate draft",
                one_line_summary: "Read-only: deterministic structural validation; returns issues.",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SMOKE_CHECK,
                title: "Smoke check",
                one_line_summary:
                    "Checks behavior/motion (bounded); may apply deterministic motion contact auto-repair.",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MOTION_METRICS,
                title: "Motion metrics",
                one_line_summary:
                    "Read-only: deterministic stride/contact metrics for the current draft motion (no mutation).",
                args_schema: "{ version?: 1, sample_count?: number }",
                args_example: serde_json::json!({ "version": 1, "sample_count": 32 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SUGGEST_MOTION_REPAIRS,
                title: "Suggest motion repairs",
                one_line_summary: "Read-only: suggests deterministic patches for motion_validation errors (no mutation; explicit apply required).",
                args_schema: "{ version?: 1, max_suggestions?: number, safety_margin_degrees?: number }",
                args_example: serde_json::json!({ "version": 1, "max_suggestions": 8, "safety_margin_degrees": 0.2 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_QA,
                title: "QA",
                one_line_summary:
                    "Runs validate_v1 + smoke_check_v1; may auto-repair motion contact; returns combined summary.",
                args_schema: "{}",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_KV_LIST_KEYS,
                title: "Info KV: list keys",
                one_line_summary:
                    "Read-only: list Info Store KV keys with latest metadata (paged; bounded).",
                args_schema:
                    "{ namespace?: string, key_prefix?: string, sort?: \"key_asc\"|\"last_written_desc\", page?: { limit?: number, cursor?: string } }",
                args_example: serde_json::json!({
                    "namespace": "gen3d",
                    "key_prefix": "ws.main.",
                    "sort": "last_written_desc",
                    "page": { "limit": 50 }
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_KV_LIST_HISTORY,
                title: "Info KV: list history",
                one_line_summary:
                    "Read-only: list historical KV revisions for one key (paged; bounded).",
                args_schema:
                    "{ namespace: string, key: string, sort?: \"rev_desc\"|\"rev_asc\", page?: { limit?: number, cursor?: string } }",
                args_example: serde_json::json!({
                    "namespace": "gen3d",
                    "key": "ws.main.scene_graph_summary",
                    "sort": "rev_desc",
                    "page": { "limit": 50 }
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_KV_GET,
                title: "Info KV: get",
                one_line_summary:
                    "Read-only: fetch a KV value by key + selector (latest/kv_rev/as-of); bounded by max_bytes.",
                args_schema:
                    "{ namespace: string, key: string, selector?: { kind: \"latest\"|\"kv_rev\"|\"as_of_assembly_rev\"|\"as_of_pass\", kv_rev?: number, assembly_rev?: number, pass?: number }, json_pointer?: string, max_bytes?: number }",
                args_example: serde_json::json!({
                    "namespace": "gen3d",
                    "key": "ws.main.scene_graph_summary",
                    "selector": { "kind": "latest" },
                    "json_pointer": "/components_total"
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_KV_GET_MANY,
                title: "Info KV: get many",
                one_line_summary:
                    "Read-only: fetch multiple KV values with a shared selector; returns per-key errors; bounded.",
                args_schema:
                    "{ items: { namespace: string, key: string, json_pointer?: string, max_bytes?: number }[], selector?: { kind: \"latest\"|\"kv_rev\"|\"as_of_assembly_rev\"|\"as_of_pass\", kv_rev?: number, assembly_rev?: number, pass?: number }, max_items?: number }",
                args_example: serde_json::json!({
                    "selector": { "kind": "latest" },
                    "items": [
                        { "namespace": "gen3d", "key": "ws.main.state_summary" },
                        { "namespace": "gen3d", "key": "ws.main.qa" },
                        { "namespace": "gen3d", "key": "ws.main.scene_graph_summary", "json_pointer": "/components_total" }
                    ]
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_EVENTS_LIST,
                title: "Info events: list",
                one_line_summary:
                    "Read-only: list recent Info Store events with filters (paged; bounded; returns data_preview).",
                args_schema:
                    "{ filters?: { kind?: \"tool_call_start\"|\"tool_call_result\"|\"engine_log\"|\"budget_stop\"|\"warning\"|\"error\", tool_id?: string, call_id?: string, min_ts_ms?: number, max_ts_ms?: number, attempt?: number, pass?: number }, sort?: \"ts_desc\"|\"ts_asc\", page?: { limit?: number, cursor?: string } }",
                args_example: serde_json::json!({
                    "filters": { "kind": "tool_call_result" },
                    "sort": "ts_desc",
                    "page": { "limit": 100 }
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_EVENTS_GET,
                title: "Info events: get",
                one_line_summary:
                    "Read-only: fetch one Info Store event by event_id (bounded by max_bytes).",
                args_schema: "{ event_id: number, json_pointer?: string, max_bytes?: number }",
                args_example: serde_json::json!({ "event_id": 1 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_EVENTS_SEARCH,
                title: "Info events: search",
                one_line_summary:
                    "Read-only: substring search over Info Store event messages (paged; bounded).",
                args_schema:
                    "{ query: string, filters?: { kind?: \"tool_call_start\"|\"tool_call_result\"|\"engine_log\"|\"budget_stop\"|\"warning\"|\"error\", attempt?: number, pass?: number }, page?: { limit?: number, cursor?: string } }",
                args_example: serde_json::json!({
                    "query": "ERROR",
                    "filters": { "kind": "error" },
                    "page": { "limit": 100 }
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_BLOBS_LIST,
                title: "Info blobs: list",
                one_line_summary:
                    "Read-only: list blobs (opaque ids for images/sheets) with metadata (paged; bounded).",
                args_schema:
                    "{ filters?: { label_prefix?: string, labels_any?: string[], labels_all?: string[], content_type_prefix?: string, attempt?: number, pass?: number }, sort?: \"created_desc\"|\"created_asc\", page?: { limit?: number, cursor?: string } }",
                args_example: serde_json::json!({
                    "filters": { "labels_all": ["kind:render_preview", "workspace:main"] },
                    "sort": "created_desc",
                    "page": { "limit": 50 }
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_INFO_BLOBS_GET,
                title: "Info blobs: get",
                one_line_summary: "Read-only: fetch one blob’s metadata by blob_id (no bytes).",
                args_schema: "{ blob_id: string }",
                args_example: serde_json::json!({ "blob_id": "00000000-0000-0000-0000-000000000000" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_APPLY_DRAFT_OPS,
                title: "Apply draft ops",
                one_line_summary: "Mutates draft: apply deterministic ops (move attachments, edit joints/motion, update/recolor primitives by part_id_uuid).",
                args_schema:
                    "{ version?: 1, atomic?: bool, if_assembly_rev?: number, ops: DraftOp[] }\n\
\n\
PrimitiveSpec = { mesh:string, params?:PrimitiveParams, color_rgba?:[number,number,number,number], unlit?:bool }\n\
PrimitiveParams =\n\
  | { kind:\"capsule\", radius:number, half_length:number }\n\
  | { kind:\"conical_frustum\", top_radius:number, bottom_radius:number, height:number }\n\
  | { kind:\"torus\", minor_radius:number, major_radius:number }\n\
AnimationSlotSpec = { driver:\"always\"|\"move_phase\"|\"move_distance\"|\"attack_time\", speed_scale:number, time_offset_units?:number, clip:AnimationClip }\n\
AnimationClip =\n\
  | { kind:\"loop\", duration_units:number, keyframes: Keyframe[] }\n\
  | { kind:\"once\", duration_units:number, keyframes: Keyframe[] }\n\
  | { kind:\"ping_pong\", duration_units:number, keyframes: Keyframe[] }\n\
  | { kind:\"spin\", axis:[number,number,number], radians_per_unit:number, axis_space:\"join\"|\"child_local\" }\n\
Keyframe = { t_units:number, delta: AnimationDeltaTransform }\n\
AnimationDeltaTransform = { pos?:[number,number,number], rot_quat_xyzw?:[number,number,number,number], scale?:[number,number,number] }\n\
\n\
DraftOp =\n\
  | { kind:\"set_anchor_transform\", component:string, anchor:string, set:TransformDelta }\n\
  | { kind:\"set_attachment_offset\", child_component:string, set:TransformDelta }\n\
  | { kind:\"set_attachment_joint\", child_component:string, set_joint: Joint|null }\n\
  | { kind:\"update_primitive_part\", component:string, part_id_uuid:string, set_transform?:TransformDelta, set_primitive?:PrimitiveSpec, set_render_priority?:number }\n\
  | { kind:\"add_primitive_part\", component:string, part_id_uuid:string, primitive:PrimitiveSpec, transform:TransformDelta, render_priority?:number }\n\
  | { kind:\"remove_primitive_part\", component:string, part_id_uuid:string }\n\
  | { kind:\"upsert_animation_slot\", child_component:string, channel:string, slot:AnimationSlotSpec }\n\
  | { kind:\"scale_animation_slot_rotation\", child_component:string, channel:string, scale:number }\n\
  | { kind:\"remove_animation_slot\", child_component:string, channel:string }\n\
\n\
Joint = { kind:\"fixed\"|\"hinge\"|\"ball\"|\"free\", axis_join?:[number,number,number], limits_degrees?:[number,number], swing_limits_degrees?:[number,number], twist_limits_degrees?:[number,number] }\n\
TransformDelta = { pos?:[number,number,number], rot_quat_xyzw?:[number,number,number,number], scale?:[number,number,number], forward?:[number,number,number], up?:[number,number,number] }",
                args_example: serde_json::json!({
                    "atomic": true,
                    "ops": [
                        { "kind": "set_attachment_offset", "child_component": "hat", "set": { "pos": [0.0, 0.6, 0.0] } },
                        { "kind": "update_primitive_part", "component": "hat", "part_id_uuid": "00000000-0000-0000-0000-000000000000", "set_primitive": { "mesh": "UnitCylinder", "color_rgba": [0.1, 0.3, 0.9, 1.0] } }
                    ]
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_RECENTER_ATTACHMENT_MOTION,
                title: "Recenter attachment motion",
                one_line_summary: "Mutates draft: deterministically recenter attachment delta rotations around neutral (fixes joint_rest_bias_large without changing motion).",
                args_schema:
                    "{ version?: 1, child_components: (string|number)[], channels?: string[], target?: \"warn\"|\"error\", dry_run?: bool }",
                args_example: serde_json::json!({ "child_components": ["leg_l_shin", "leg_r_shin"], "channels": ["idle","move"], "target": "warn" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SNAPSHOT,
                title: "Snapshot",
                one_line_summary: "Side-effect: save a snapshot of current session state (for diff/restore).",
                args_schema: "{ version?: 1, snapshot_id?: string, label?: string }",
                args_example: serde_json::json!({ "label": "pre_leg_fix_rev16" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LIST_SNAPSHOTS,
                title: "List snapshots",
                one_line_summary: "Read-only: list available in-session snapshots (bounded).",
                args_schema: "{ version?: 1, max_items?: number }",
                args_example: serde_json::json!({ "max_items": 50 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DIFF_SNAPSHOTS,
                title: "Diff snapshots",
                one_line_summary: "Read-only: structured diff between two snapshots (bounded).",
                args_schema: "{ version?: 1, a: string, b: string, max_components?: number }",
                args_example: serde_json::json!({ "a": "snap_1", "b": "snap_2" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_RESTORE_SNAPSHOT,
                title: "Restore snapshot",
                one_line_summary: "Mutates session: restore draft/plan state from a snapshot.",
                args_schema: "{ version?: 1, snapshot_id: string }",
                args_example: serde_json::json!({ "snapshot_id": "snap_1" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_COMPONENT,
                title: "Copy component",
                one_line_summary: "Mutates draft: copy one component into targets (linked/detached; no regen).",
                args_schema:
                    "{ source_component: string|number, targets?: (string|number)[], mode?: \"detached\"|\"linked\", anchors?: string, transform?: TransformDelta }",
                args_example: serde_json::json!({ "source_component": "arm_l_upper", "targets": ["arm_r_upper"], "mode": "linked" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MIRROR_COMPONENT,
                title: "Mirror component",
                one_line_summary: "Mutates draft: mirror one component into targets (L/R; no regen).",
                args_schema:
                    "{ source_component: string|number, targets?: (string|number)[], mode?: \"detached\"|\"linked\", anchors?: string, transform?: TransformDelta }",
                args_example: serde_json::json!({ "source_component": "arm_l_upper", "targets": ["arm_r_upper"], "mode": "detached" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_COMPONENT_SUBTREE,
                title: "Copy component subtree",
                one_line_summary: "Mutates draft: copy a component + descendants into target roots (no regen).",
                args_schema:
                    "{ source_root: string|number, targets: (string|number)[], mode?: \"detached\", anchors?: string, transform?: TransformDelta }",
                args_example: serde_json::json!({ "source_root": "leg_l_thigh", "targets": ["leg_r_thigh"] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MIRROR_COMPONENT_SUBTREE,
                title: "Mirror component subtree",
                one_line_summary: "Mutates draft: mirror a component subtree into target roots (L/R; no regen).",
                args_schema:
                    "{ source_root: string|number, targets: (string|number)[], mode?: \"detached\", anchors?: string, transform?: TransformDelta }",
                args_example: serde_json::json!({ "source_root": "leg_l_thigh", "targets": ["leg_r_thigh"] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DETACH_COMPONENT,
                title: "Detach component copy",
                one_line_summary:
                    "Mutates draft: materialize a linked component copy into real geometry (so it can diverge).",
                args_schema: "{ component: string|number }",
                args_example: serde_json::json!({ "component": "arm_r_upper" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_PLAN,
                title: "LLM: generate plan",
                one_line_summary:
                    "LLM+mutates: generate/replace the component plan; preserve-mode diffs are policy-validated (constraints.preserve_edit_policy).",
                args_schema:
                    "{ prompt?: string, style?: string, plan_template_kv?: { namespace: string, key: string, selector?: { kind: \"latest\"|\"kv_rev\"|\"as_of_assembly_rev\"|\"as_of_pass\", kv_rev?: number, assembly_rev?: number, pass?: number } }, constraints?: { preserve_existing_components?: bool, preserve_edit_policy?: \"additive\"|\"allow_offsets\"|\"allow_rewire\", rewire_components?: string[] }, components?: string[] }",
                args_example: serde_json::json!({
                    "constraints": {
                        "preserve_existing_components": true,
                        "preserve_edit_policy": "additive"
                    }
                }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENT,
                title: "LLM: generate component",
                one_line_summary:
                    "LLM+mutates: generate one component geometry (or regen w/ force). In preserve mode, regen is QA-gated (requires qa_v1 errors).",
                args_schema:
                    "{ component_name?: string, component_index?: number, force?: bool }",
                args_example: serde_json::json!({ "component_name": "leg_l_thigh" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_COMPONENTS,
                title: "LLM: generate components (batch)",
                one_line_summary: "LLM+mutates: batch-generate components (missing_only/force). Force regen is QA-gated in preserve mode (qa_v1 errors required).",
                args_schema:
                    "{ component_indices?: number[], component_names?: string[], missing_only?: bool, force?: bool }",
                args_example: serde_json::json!({ "missing_only": true }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_GENERATE_MOTION_AUTHORING,
                title: "LLM: generate motion authoring",
                one_line_summary: "LLM+mutates: author animation clips (idle/move/attack) on attachment edges.",
                args_schema: "{ prompt?: string }",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_LLM_REVIEW_DELTA,
                title: "LLM: review delta",
                one_line_summary:
                    "LLM+mutates: apply deterministic tweak ops; may request component regen indices (some may be budget/QA-gated).",
                args_schema:
                    "{ preview_blob_ids?: string[], blob_ids?: string[], include_original_images?: bool }",
                args_example: serde_json::json!({}),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_RENDER_PREVIEW,
                title: "Render preview",
                one_line_summary: "Side-effect: render deterministic preview images and register them as Info Store blobs (no draft mutation); returns `blob_ids` (no paths).",
                args_schema:
                    "{ views?: string[], image_size?: number, resolution?: number, width?: number, height?: number, overlay?: string, prefix?: string, include_motion_sheets?: bool }",
                args_example: serde_json::json!({ "views": ["front", "left_back", "right_back", "top", "bottom"], "image_size": 768 }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_CREATE_WORKSPACE,
                title: "Create workspace",
                one_line_summary: "Mutates workspaces: create a new workspace cloned from an existing one.",
                args_schema:
                    "{ from?: string, name?: string, workspace_id?: string, include_components?: string[] }",
                args_example: serde_json::json!({ "workspace_id": "ws_fixlegs", "from": "main" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DELETE_WORKSPACE,
                title: "Delete workspace",
                one_line_summary: "Mutates workspaces: delete a workspace (not the active workspace).",
                args_schema: "{ workspace_id: string }",
                args_example: serde_json::json!({ "workspace_id": "ws_fixlegs" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SET_ACTIVE_WORKSPACE,
                title: "Set active workspace",
                one_line_summary: "UI-only: switch which workspace is shown in the preview scene.",
                args_schema: "{ workspace_id: string }",
                args_example: serde_json::json!({ "workspace_id": "main" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_DIFF_WORKSPACES,
                title: "Diff workspaces",
                one_line_summary: "Read-only: structured diff between two workspaces (bounded).",
                args_schema: "{ version?: 1, a?: string, b?: string, max_components?: number }",
                args_example: serde_json::json!({ "a": "main", "b": "ws_fixlegs" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_COPY_FROM_WORKSPACE,
                title: "Copy from workspace",
                one_line_summary:
                    "Mutates draft: cherry-pick components/subtrees from another workspace into the active one.",
                args_schema:
                    "{ version?: 1, from: string, components?: string[], mode?: string, include_attachment?: bool }",
                args_example: serde_json::json!({ "from": "ws_fixlegs", "components": ["leg_l_thigh", "leg_r_thigh"] }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_MERGE_WORKSPACE,
                title: "Merge workspaces",
                one_line_summary: "Mutates workspaces: deterministic 3-way merge into a new workspace.",
                args_schema:
                    "{ version?: 1, base: string, a: string, b: string, output_workspace_id?: string, output_name?: string, max_components?: number }",
                args_example: serde_json::json!({ "base": "main", "a": "ws_a", "b": "ws_b", "output_workspace_id": "ws_merge" }),
            },
            Gen3dToolDescriptorV1 {
                tool_id: TOOL_ID_SUBMIT_TOOLING_FEEDBACK,
                title: "Submit tooling feedback",
                one_line_summary: "Side-effect: record tool feedback (missing tools/bugs/enhancements).",
                args_schema:
                    "{ version?: 1, priority: \"low|medium|high|blocker\", title: string, summary: string, details?: any }",
                args_example: serde_json::json!({ "version": 1, "priority": "low", "title": "Example", "summary": "Tool docs could include args examples." }),
            },
        ];
        out.sort_by(|a, b| a.tool_id.cmp(b.tool_id));
        out
    }
}
