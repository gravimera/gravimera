use bevy::log::{debug, warn};
use bevy::prelude::*;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::tools::{
    TOOL_ID_DIFF_SNAPSHOTS, TOOL_ID_LIST_SNAPSHOTS, TOOL_ID_LLM_GENERATE_MOTIONS,
    TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_RENDER_PREVIEW, TOOL_ID_SMOKE_CHECK, TOOL_ID_SNAPSHOT,
    TOOL_ID_VALIDATE,
};
use crate::gen3d::agent::{
    append_agent_trace_event_v1, AgentTraceEventV1, Gen3dAgentActionJsonV1, Gen3dToolCallJsonV1,
    Gen3dToolResultJsonV1,
};
use crate::types::{ActionClock, AnimationChannelsActive, AttackClock, LocomotionClock};

use super::super::state::{
    Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dReviewCaptureCamera, Gen3dWorkshop,
};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::agent_loop::spawn_agent_step_request;
use super::agent_parsing::{is_transient_ai_error_message, parse_agent_step};
use super::agent_tool_dispatch::execute_tool_call;
use super::agent_utils::{
    compute_agent_state_hash, note_observable_tool_result, step_had_no_progress_try,
    truncate_json_for_log,
};
use super::artifacts::{
    append_gen3d_jsonl_artifact, append_gen3d_run_log, write_gen3d_text_artifact,
};
use super::status_steps;
use super::GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES;
use super::{
    fail_job, gen3d_advance_pass, set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob,
    Gen3dAiPhase, Gen3dAiProgress, Gen3dAiTextResponse, Gen3dPendingFinishRun,
};
use crate::threaded_result::{new_shared_result, take_shared_result};

fn run_complete_enough_for_auto_finish(job: &Gen3dAiJob, draft: &Gen3dDraft) -> bool {
    if draft.total_non_projectile_primitive_parts() == 0 {
        return false;
    }

    if job
        .planned_components
        .iter()
        .any(|c| c.actual_size.is_none())
    {
        return false;
    }

    if !job.agent.pending_regen_component_indices.is_empty() {
        return false;
    }

    if job.agent.last_validate_ok != Some(true) {
        return false;
    }

    if job.agent.last_smoke_ok != Some(true) {
        return false;
    }

    if job.agent.last_motion_ok == Some(false) {
        return false;
    }

    let llm_available = job
        .ai
        .as_ref()
        .map(|ai| !ai.base_url().starts_with("mock://gen3d"))
        .unwrap_or(true);
    let appearance_review_enabled = llm_available && job.review_appearance;
    if appearance_review_enabled {
        if !job.agent.ever_rendered
            || !job.agent.ever_reviewed
            || job.agent.rendered_since_last_review
        {
            return false;
        }
    }

    let movable = draft
        .root_def()
        .and_then(|def| def.mobility.as_ref())
        .is_some();
    if movable {
        let has_move = job.planned_components.iter().any(|c| {
            c.attach_to.as_ref().is_some_and(|att| {
                att.animations
                    .iter()
                    .any(|slot| slot.channel.as_ref() == "move")
            })
        });

        if !has_move {
            return false;
        }
    }

    true
}

fn append_qa_warnings_to_status(status: &mut String, agent: &super::Gen3dAgentState) {
    let count = agent.last_qa_warnings_count.unwrap_or(0);
    if count == 0 {
        return;
    }

    status.push_str("\n\nQA warnings (non-blocking):");
    status.push_str(&format!("\n- count: {count}"));
    if let Some(example) = agent.last_qa_warning_example.as_ref() {
        let example = example.trim();
        if !example.is_empty() {
            status.push_str("\n- example: ");
            status.push_str(example);
        }
    }
}

fn no_progress_guard_stop_fixits(job: &Gen3dAiJob) -> Vec<serde_json::Value> {
    let mut fixits: Vec<serde_json::Value> = Vec::new();

    let mut snaps: Vec<(&String, &super::job::Gen3dAgentSnapshot)> =
        job.agent.snapshots.iter().collect();
    snaps.sort_by(|a, b| {
        a.1.created_at_ms
            .cmp(&b.1.created_at_ms)
            .then_with(|| a.0.cmp(b.0))
    });

    if snaps.len() >= 2 {
        let a = snaps[snaps.len() - 2].0.as_str();
        let b = snaps[snaps.len() - 1].0.as_str();
        fixits.push(serde_json::json!({
            "tool_id": TOOL_ID_DIFF_SNAPSHOTS,
            "args": { "version": 1, "a": a, "b": b },
        }));
        return fixits;
    }

    if snaps.is_empty() {
        fixits.push(serde_json::json!({
            "tool_id": TOOL_ID_SNAPSHOT,
            "args": { "version": 1, "label": "snap_before_debug" },
        }));
    }

    fixits.push(serde_json::json!({
        "tool_id": TOOL_ID_LIST_SNAPSHOTS,
        "args": { "version": 1, "max_items": 20 },
    }));

    fixits
}

fn engine_run_summary_json(job: &Gen3dAiJob, draft: &Gen3dDraft) -> serde_json::Value {
    let llm_available = job
        .ai
        .as_ref()
        .map(|ai| !ai.base_url().starts_with("mock://gen3d"))
        .unwrap_or(true);
    let appearance_review_enabled = llm_available && job.review_appearance;

    serde_json::json!({
        "attempt": job.attempt,
        "pass": job.pass,
        "assembly_rev": job.assembly_rev,
        "plan_hash": job.plan_hash.as_str(),
        "preserve_existing_components_mode": job.preserve_existing_components_mode,
        "planned_components_total": job.planned_components.len(),
        "primitive_parts_total": draft.total_non_projectile_primitive_parts(),
        "qa": {
            "ever_validated": job.agent.ever_validated,
            "ever_smoke_checked": job.agent.ever_smoke_checked,
            "last_validate_ok": job.agent.last_validate_ok,
            "last_smoke_ok": job.agent.last_smoke_ok,
            "last_motion_ok": job.agent.last_motion_ok,
            "warnings_count": job.agent.last_qa_warnings_count,
        },
        "review": {
            "review_appearance_enabled": appearance_review_enabled,
            "ever_rendered": job.agent.ever_rendered,
            "ever_reviewed": job.agent.ever_reviewed,
            "rendered_since_last_review": job.agent.rendered_since_last_review,
        },
    })
}

fn finalize_run_now(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    finish: Gen3dPendingFinishRun,
) {
    workshop.status = finish.workshop_status;
    workshop
        .status_log
        .finish_step_if_active("Finished.".to_string());
    append_gen3d_run_log(job.pass_dir.as_deref(), finish.run_log);
    info!("{}", finish.info_log);
    job.finish_run_metrics();
    job.running = false;
    job.build_complete = true;
    job.phase = Gen3dAiPhase::Idle;
    job.shared_progress = None;
    job.shared_result = None;
    job.pending_finish_run = None;
}

fn descriptor_meta_user_prompt(job: &Gen3dAiJob, workshop: &Gen3dWorkshop) -> String {
    let raw = job.user_prompt_raw().trim();
    if raw.is_empty() {
        workshop.prompt.trim().to_string()
    } else {
        raw.to_string()
    }
}

fn descriptor_meta_animation_channels_ordered(draft: &Gen3dDraft) -> Vec<String> {
    use crate::object::registry::{ObjectPartKind, PART_ANIMATION_INTERNAL_BASE_CHANNEL};
    use std::collections::HashSet;

    let root_id = super::super::gen3d_draft_object_id();
    let by_id: std::collections::HashMap<u128, &crate::object::registry::ObjectDef> =
        draft.defs.iter().map(|d| (d.object_id, d)).collect();

    fn visit(
        by_id: &std::collections::HashMap<u128, &crate::object::registry::ObjectDef>,
        object_id: u128,
        visited: &mut HashSet<u128>,
        channels: &mut HashSet<String>,
    ) {
        if !visited.insert(object_id) {
            return;
        }
        let Some(def) = by_id.get(&object_id) else {
            return;
        };
        for part in def.parts.iter() {
            for slot in part.animations.iter() {
                let ch = slot.channel.as_ref().trim();
                if !ch.is_empty() && ch != PART_ANIMATION_INTERNAL_BASE_CHANNEL {
                    channels.insert(ch.to_string());
                }
            }
            if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                visit(by_id, *child, visited, channels);
            }
        }
    }

    let mut visited: HashSet<u128> = HashSet::new();
    let mut channels: HashSet<String> = HashSet::new();
    visit(&by_id, root_id, &mut visited, &mut channels);

    if draft
        .root_def()
        .and_then(|def| def.attack.as_ref())
        .is_some()
    {
        channels.insert("attack_primary".to_string());
    }

    let mut out: Vec<String> = Vec::new();
    for key in ["idle", "move", "attack_primary"] {
        if channels.remove(key) {
            out.push(key.to_string());
        }
    }
    let mut rest: Vec<String> = channels.into_iter().collect();
    rest.sort();
    out.extend(rest);
    out
}

fn descriptor_meta_motion_summary_json(draft: &Gen3dDraft) -> serde_json::Value {
    use crate::object::registry::{
        ObjectPartKind, PartAnimationDef, PartAnimationDriver, PART_ANIMATION_INTERNAL_BASE_CHANNEL,
    };
    use std::collections::{BTreeMap, BTreeSet, HashSet};

    #[derive(Default)]
    struct Summary {
        slots: u32,
        animated_parts: u32,
        drivers: BTreeSet<String>,
        clip_kinds: BTreeSet<String>,
        loop_duration_min: Option<f32>,
        loop_duration_max: Option<f32>,
        speed_scale_min: Option<f32>,
        speed_scale_max: Option<f32>,
        has_time_offsets: bool,
    }

    fn driver_name(driver: PartAnimationDriver) -> &'static str {
        match driver {
            PartAnimationDriver::Always => "always",
            PartAnimationDriver::MovePhase => "move_phase",
            PartAnimationDriver::MoveDistance => "move_distance",
            PartAnimationDriver::AttackTime => "attack_time",
            PartAnimationDriver::ActionTime => "action_time",
        }
    }

    let root_id = super::super::gen3d_draft_object_id();
    let by_id: std::collections::HashMap<u128, &crate::object::registry::ObjectDef> =
        draft.defs.iter().map(|d| (d.object_id, d)).collect();

    fn visit(
        by_id: &std::collections::HashMap<u128, &crate::object::registry::ObjectDef>,
        object_id: u128,
        visited: &mut HashSet<u128>,
        summaries: &mut BTreeMap<String, Summary>,
    ) {
        if !visited.insert(object_id) {
            return;
        }
        let Some(def) = by_id.get(&object_id) else {
            return;
        };
        for part in def.parts.iter() {
            let mut channels_in_part: BTreeSet<String> = BTreeSet::new();
            for slot in part.animations.iter() {
                let channel = slot.channel.as_ref().trim();
                if channel.is_empty() || channel == PART_ANIMATION_INTERNAL_BASE_CHANNEL {
                    continue;
                }
                channels_in_part.insert(channel.to_string());
                let entry = summaries.entry(channel.to_string()).or_default();
                entry.slots = entry.slots.saturating_add(1);
                entry
                    .drivers
                    .insert(driver_name(slot.spec.driver).to_string());
                entry.speed_scale_min = Some(
                    entry
                        .speed_scale_min
                        .map_or(slot.spec.speed_scale, |v| v.min(slot.spec.speed_scale)),
                );
                entry.speed_scale_max = Some(
                    entry
                        .speed_scale_max
                        .map_or(slot.spec.speed_scale, |v| v.max(slot.spec.speed_scale)),
                );
                if slot.spec.time_offset_units.abs() > 1e-6 {
                    entry.has_time_offsets = true;
                }
                match &slot.spec.clip {
                    PartAnimationDef::Loop { duration_secs, .. }
                    | PartAnimationDef::Once { duration_secs, .. }
                    | PartAnimationDef::PingPong { duration_secs, .. } => {
                        entry.clip_kinds.insert(
                            match &slot.spec.clip {
                                PartAnimationDef::Loop { .. } => "loop",
                                PartAnimationDef::Once { .. } => "once",
                                PartAnimationDef::PingPong { .. } => "ping_pong",
                                PartAnimationDef::Spin { .. } => {
                                    unreachable!("spin handled below")
                                }
                            }
                            .to_string(),
                        );
                        if duration_secs.is_finite() && *duration_secs > 0.0 {
                            entry.loop_duration_min = Some(
                                entry
                                    .loop_duration_min
                                    .map_or(*duration_secs, |v| v.min(*duration_secs)),
                            );
                            entry.loop_duration_max = Some(
                                entry
                                    .loop_duration_max
                                    .map_or(*duration_secs, |v| v.max(*duration_secs)),
                            );
                        }
                    }
                    PartAnimationDef::Spin { .. } => {
                        entry.clip_kinds.insert("spin".to_string());
                    }
                }
            }

            for channel in channels_in_part {
                if let Some(entry) = summaries.get_mut(&channel) {
                    entry.animated_parts = entry.animated_parts.saturating_add(1);
                }
            }

            if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                visit(by_id, *child, visited, summaries);
            }
        }
    }

    let mut visited: HashSet<u128> = HashSet::new();
    let mut summaries: BTreeMap<String, Summary> = BTreeMap::new();
    visit(&by_id, root_id, &mut visited, &mut summaries);

    let mut channels: Vec<serde_json::Value> = Vec::new();
    for (channel, summary) in summaries {
        let drivers: Vec<String> = summary.drivers.into_iter().collect();
        let clip_kinds: Vec<String> = summary.clip_kinds.into_iter().collect();
        channels.push(serde_json::json!({
            "channel": channel,
            "slots": summary.slots,
            "animated_parts": summary.animated_parts,
            "drivers": drivers,
            "clip_kinds": clip_kinds,
            "loop_duration_secs_min": summary.loop_duration_min,
            "loop_duration_secs_max": summary.loop_duration_max,
            "speed_scale_min": summary.speed_scale_min,
            "speed_scale_max": summary.speed_scale_max,
            "has_time_offsets": summary.has_time_offsets,
        }));
    }

    serde_json::json!({
        "version": 1,
        "channels": channels,
    })
}

fn maybe_start_descriptor_meta_request(
    _config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    finish: &Gen3dPendingFinishRun,
) -> bool {
    if job.ai.is_none() {
        return false;
    }
    if job.pass_dir.is_none() {
        return false;
    }
    if draft.total_non_projectile_primitive_parts() == 0 {
        return false;
    }
    if job.descriptor_meta_for_save().is_some() {
        return false;
    }
    if job.shared_result.is_some() {
        warn!("Gen3D: skipping descriptor-meta; unexpected in-flight shared_result.");
        return false;
    }

    if let Some(in_flight) = job.descriptor_meta_in_flight.as_ref() {
        let stale = job.run_id != Some(in_flight.run_id)
            || job.plan_hash.trim() != in_flight.plan_hash.trim();
        if !stale {
            let in_flight = job.descriptor_meta_in_flight.take().unwrap();
            job.shared_result = Some(in_flight.shared_result);
            let progress = job
                .shared_progress
                .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                .clone();
            set_progress(&progress, "Waiting for prefab metadata…");
            append_gen3d_run_log(job.pass_dir.as_deref(), "descriptor_meta_adopt_in_flight");
            job.pending_finish_run = Some(finish.clone());
            job.phase = Gen3dAiPhase::AgentWaitingDescriptorMeta;
            return true;
        }
    }

    let Some(root_def) = draft.root_def() else {
        return false;
    };

    let user_prompt = descriptor_meta_user_prompt(job, workshop);

    let roles = vec![if root_def.mobility.is_some() {
        "unit".to_string()
    } else {
        "building".to_string()
    }];

    let size_m = root_def.size;
    let ground_origin_y_m = root_def
        .ground_origin_y
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or_else(|| {
            if size_m.y.is_finite() {
                size_m.y.abs() * 0.5
            } else {
                0.0
            }
        });

    let mobility_str = root_def.mobility.map(|m| match m.mode {
        crate::object::registry::MobilityMode::Ground => "ground".to_string(),
        crate::object::registry::MobilityMode::Air => "air".to_string(),
    });
    let attack_kind_str = root_def.attack.as_ref().map(|a| match a.kind {
        crate::object::registry::UnitAttackKind::Melee => "melee".to_string(),
        crate::object::registry::UnitAttackKind::RangedProjectile => {
            "ranged_projectile".to_string()
        }
    });

    let mut anchors: Vec<String> = root_def
        .anchors
        .iter()
        .map(|a| a.name.as_ref().to_string())
        .collect();
    anchors.sort();
    anchors.dedup();

    let animation_channels = descriptor_meta_animation_channels_ordered(draft);
    let motion_summary_json = descriptor_meta_motion_summary_json(draft);

    let plan_extracted_text = job
        .pass_dir_path()
        .and_then(|dir| std::fs::read(dir.join("plan_extracted.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| serde_json::to_string_pretty(&value).ok());

    let system = super::prompts::build_gen3d_descriptor_meta_system_instructions();
    let user_text = super::prompts::build_gen3d_descriptor_meta_user_text(
        root_def.label.as_ref(),
        &user_prompt,
        &roles,
        size_m,
        ground_origin_y_m,
        mobility_str.as_deref(),
        attack_kind_str.as_deref(),
        &anchors,
        &animation_channels,
        plan_extracted_text.as_deref(),
        Some(&motion_summary_json),
    );

    let Some(ai) = job.ai.clone() else {
        return false;
    };
    let Some(pass_dir) = job.pass_dir.clone() else {
        return false;
    };

    let shared = new_shared_result::<Gen3dAiTextResponse, String>();
    job.shared_result = Some(shared.clone());
    let progress = job
        .shared_progress
        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
        .clone();

    workshop.status = finish.workshop_status.clone();
    set_progress(&progress, "Generating prefab metadata…");
    append_gen3d_run_log(Some(&pass_dir), "descriptor_meta_start");

    let reasoning_effort = ai.model_reasoning_effort().to_string();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(super::structured_outputs::Gen3dAiJsonSchemaKind::DescriptorMetaV1),
        job.require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        Vec::new(),
        pass_dir,
        "descriptor_meta".into(),
    );

    job.pending_finish_run = Some(finish.clone());
    job.phase = Gen3dAiPhase::AgentWaitingDescriptorMeta;
    true
}

pub(super) fn start_finish_run_sequence(
    config: &AppConfig,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    finish: Gen3dPendingFinishRun,
) {
    if maybe_start_descriptor_meta_request(config, workshop, job, draft, &finish) {
        return;
    }

    if maybe_start_pass_snapshot_capture(
        config,
        commands,
        images,
        workshop,
        job,
        draft,
        super::Gen3dAgentAfterPassSnapshot::FinishRun {
            workshop_status: finish.workshop_status.clone(),
            run_log: finish.run_log.clone(),
            info_log: finish.info_log.clone(),
        },
    ) {
        workshop.status = finish.workshop_status;
        return;
    }

    finalize_run_now(workshop, job, finish);
}

pub(super) fn poll_agent_descriptor_meta(
    config: &AppConfig,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
) {
    let Some(shared) = job.shared_result.as_ref() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing descriptor-meta shared_result.",
        );
        return;
    };
    let Some(result) = take_shared_result(shared) else {
        return;
    };
    job.shared_result = None;

    match result {
        Ok(resp) => {
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                job.add_tokens(tokens);
            }
            match super::parse::parse_ai_descriptor_meta_from_text(&resp.text) {
                Ok(meta) => {
                    job.descriptor_meta_cache = Some(super::Gen3dDescriptorMetaCache {
                        plan_hash: job.plan_hash.clone(),
                        meta,
                    });
                }
                Err(err) => {
                    warn!("Gen3D: descriptor-meta parse failed: {err}");
                }
            }
        }
        Err(err) => {
            warn!("Gen3D: descriptor-meta request failed: {err}");
        }
    }

    let Some(finish) = job.pending_finish_run.take() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing pending finish after descriptor-meta.",
        );
        return;
    };

    // After metadata, optionally capture pass screenshots, then finish.
    if maybe_start_pass_snapshot_capture(
        config,
        commands,
        images,
        workshop,
        job,
        draft,
        super::Gen3dAgentAfterPassSnapshot::FinishRun {
            workshop_status: finish.workshop_status.clone(),
            run_log: finish.run_log.clone(),
            info_log: finish.info_log.clone(),
        },
    ) {
        workshop.status = finish.workshop_status;
        return;
    }

    finalize_run_now(workshop, job, finish);
}

pub(super) fn poll_agent_step(
    config: &AppConfig,
    commands: &mut Commands,
    review_cameras: &Query<Entity, With<Gen3dReviewCaptureCamera>>,
    workshop: &mut Gen3dWorkshop,
    feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
) {
    let Some(shared) = job.shared_result.as_ref() else {
        return;
    };
    let result = shared.lock().ok().and_then(|mut g| g.take());
    let Some(result) = result else {
        return;
    };
    job.shared_result = None;
    job.metrics.note_agent_step_response_received();

    match result {
        Ok(resp) => {
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                job.add_tokens(tokens);
            }
            job.agent.step_request_retry_attempt = 0;

            let text = resp.text;
            if let Some(pass_dir) = job.pass_dir.as_deref() {
                write_gen3d_text_artifact(Some(pass_dir), "agent_step_raw.txt", text.trim());
            }

            match parse_agent_step(&text) {
                Ok(step) => {
                    workshop.error = None;
                    if !step.status_summary.trim().is_empty() {
                        workshop.status = step.status_summary.trim().to_string();
                    }

                    let mut actions_summary = Vec::new();
                    for action in step.actions.iter() {
                        match action {
                            Gen3dAgentActionJsonV1::ToolCall { tool_id, .. } => {
                                actions_summary.push(format!("tool_call:{tool_id}"));
                            }
                            Gen3dAgentActionJsonV1::Done { .. } => {
                                actions_summary.push("done".to_string());
                            }
                        }
                    }
                    let actions_summary = actions_summary.join(", ");

                    append_agent_trace_event_v1(
                        job.run_dir.as_deref(),
                        &AgentTraceEventV1::Info {
                            message: format!(
                                "Gen3D agent step parsed: status_summary={:?} actions=[{}]",
                                step.status_summary.trim(),
                                actions_summary
                            ),
                        },
                    );
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        format!(
                            "agent_step_parsed status_summary={:?} actions=[{}]",
                            step.status_summary.trim(),
                            actions_summary
                        ),
                    );
                    debug!(
                        "Gen3D agent step parsed: status_summary={:?} actions=[{}]",
                        step.status_summary.trim(),
                        actions_summary
                    );

                    job.agent.step_actions = step.actions;
                    job.agent.step_action_idx = 0;
                    job.agent.step_tool_results.clear();
                    job.agent.step_had_observable_output = false;
                    job.phase = Gen3dAiPhase::AgentExecutingActions;
                }
                Err(err) => {
                    job.agent.step_repair_attempt = job.agent.step_repair_attempt.saturating_add(1);
                    let attempt = job.agent.step_repair_attempt;
                    if attempt <= 2 {
                        workshop.status =
                            format!("Agent output invalid (attempt {attempt}/2). Retrying…");
                        workshop.error = Some(err.clone());
                        append_gen3d_run_log(
                            job.pass_dir.as_deref(),
                            format!(
                                "agent_step_parse_failed attempt={attempt} err={}",
                                err.trim()
                            ),
                        );
                        warn!(
                            "Gen3D agent step parse error (attempt {attempt}/2): {}",
                            err.trim()
                        );
                        if let Some(pass_dir) = job.pass_dir.clone() {
                            let _ = spawn_agent_step_request(config, workshop, job, pass_dir);
                            job.phase = Gen3dAiPhase::AgentWaitingStep;
                            return;
                        }
                    }
                    fail_job(
                        workshop,
                        job,
                        format!("Gen3D agent step parse error: {err}"),
                    );
                }
            }
        }
        Err(err) => {
            if is_transient_ai_error_message(&err) {
                job.agent.step_request_retry_attempt =
                    job.agent.step_request_retry_attempt.saturating_add(1);
                let attempt = job.agent.step_request_retry_attempt;
                if attempt <= GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES {
                    workshop.status = format!(
                        "AI request failed (attempt {attempt}/{GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES}); retrying…"
                    );
                    workshop.error = Some(err.clone());
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        format!(
                            "agent_step_request_failed transient attempt={attempt} err={}",
                            super::truncate_for_ui(&err, 600)
                        ),
                    );
                    warn!(
                        "Gen3D agent step request transient failure; retrying (attempt {attempt}/{GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES}) err={}",
                        super::truncate_for_ui(&err, 240)
                    );
                    if let Some(pass_dir) = job.pass_dir.clone() {
                        let _ = spawn_agent_step_request(config, workshop, job, pass_dir);
                        job.phase = Gen3dAiPhase::AgentWaitingStep;
                        return;
                    }
                }

                if draft.total_non_projectile_primitive_parts() > 0 {
                    super::finish_job_best_effort(
                        commands,
                        review_cameras,
                        workshop,
                        job,
                        format!(
                            "AI transient failure after {attempt} retry attempt(s). Last error: {}",
                            super::truncate_for_ui(&err, 600)
                        ),
                    );
                    return;
                }
            }

            fail_job(workshop, job, err);
        }
    }

    let _ = feedback_history;
}

pub(super) fn execute_agent_actions(
    config: &AppConfig,
    time: &Time,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    preview: &mut Gen3dPreview,
    preview_model: &mut Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
            &mut ActionClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    let max_actions_per_tick = 4usize;
    let mut executed = 0usize;

    while executed < max_actions_per_tick {
        if job.agent.step_action_idx >= job.agent.step_actions.len() {
            // No-progress guard: if the agent keeps asking for steps but nothing changes and we
            // don't change the draft/assembly, stop best-effort.
            let state_hash = compute_agent_state_hash(job, draft);
            let changed = job
                .agent
                .last_state_hash
                .as_deref()
                .map(|h| h != state_hash.as_str())
                .unwrap_or(true);
            if changed {
                job.agent.no_progress_tries = 0;
                job.agent.no_progress_inspection_steps = 0;
                job.agent.last_state_hash = Some(state_hash.clone());
            } else {
                if step_had_no_progress_try(&job.agent.step_tool_results) {
                    job.agent.no_progress_tries = job.agent.no_progress_tries.saturating_add(1);
                } else {
                    job.agent.no_progress_inspection_steps =
                        job.agent.no_progress_inspection_steps.saturating_add(1);
                }
            }
            job.agent.step_had_observable_output = false;

            let tries_max = config.gen3d_no_progress_tries_max;
            let inspection_max = config.gen3d_inspection_steps_max;
            let tries_triggered = tries_max > 0 && job.agent.no_progress_tries >= tries_max;
            let inspection_triggered =
                inspection_max > 0 && job.agent.no_progress_inspection_steps >= inspection_max;
            if tries_triggered || inspection_triggered {
                if !run_complete_enough_for_auto_finish(job, draft) {
                    // Prefer continuing so the agent can run the required QA sequence.
                    // If it refuses, budgets will stop the run anyway.
                    job.agent.no_progress_tries = 0;
                    job.agent.no_progress_inspection_steps = 0;
                    job.agent.last_state_hash = Some(state_hash);
                } else {
                    let fixits = no_progress_guard_stop_fixits(job);
                    job.append_info_event_best_effort(
                        super::info_store::InfoEventKindV1::BudgetStop,
                        None,
                        None,
                        format!(
                            "Budget stop: no-progress guard triggered (tries: {}/{}; inspection steps: {}/{}).",
                            job.agent.no_progress_tries,
                            tries_max,
                            job.agent.no_progress_inspection_steps,
                            inspection_max
                        ),
                        serde_json::json!({
                            "reason": "no_progress_guard_triggered",
                            "stop_reason": "no_progress",
                            "tries": job.agent.no_progress_tries,
                            "tries_max": tries_max,
                            "inspection_steps": job.agent.no_progress_inspection_steps,
                            "inspection_steps_max": inspection_max,
                            "last_state_hash": job.agent.last_state_hash.as_deref(),
                            "fixits": fixits,
                        }),
                    );

                    workshop.error = None;
                    let mut status = format!(
                        "Build finished (best effort).\nReason: No-progress guard triggered (tries: {}/{}; inspection steps: {}/{}).",
                        job.agent.no_progress_tries,
                        tries_max,
                        job.agent.no_progress_inspection_steps,
                        inspection_max
                    );
                    append_qa_warnings_to_status(&mut status, &job.agent);
                    start_finish_run_sequence(
                        config,
                        commands,
                        images,
                        workshop,
                        job,
                        draft,
                        Gen3dPendingFinishRun {
                            workshop_status: status,
                            run_log: format!(
                                "no_progress_guard_stop tries={} inspection_steps={}",
                                job.agent.no_progress_tries,
                                job.agent.no_progress_inspection_steps
                            ),
                            info_log: format!(
                                "Gen3D agent: best-effort stop (no-progress guard; tries={} inspection_steps={}).",
                                job.agent.no_progress_tries,
                                job.agent.no_progress_inspection_steps
                            ),
                        },
                    );
                    return;
                }
            }

            // Step complete: request next step.
            if maybe_start_pass_snapshot_capture(
                config,
                commands,
                images,
                workshop,
                job,
                draft,
                super::Gen3dAgentAfterPassSnapshot::AdvancePassAndRequestStep,
            ) {
                return;
            }
            if let Err(err) = gen3d_advance_pass(job) {
                fail_job(workshop, job, err);
                return;
            }
            let Some(pass_dir) = job.pass_dir.clone() else {
                fail_job(workshop, job, "Internal error: missing pass dir");
                return;
            };
            append_gen3d_run_log(Some(&pass_dir), "agent_step_complete; requesting next step");
            debug!("Gen3D agent: step complete; requesting next step");
            job.phase = Gen3dAiPhase::AgentWaitingStep;
            job.agent.step_repair_attempt = 0;
            let _ = spawn_agent_step_request(config, workshop, job, pass_dir);
            return;
        }

        let action = job.agent.step_actions[job.agent.step_action_idx].clone();
        match action {
            Gen3dAgentActionJsonV1::Done { reason } => {
                // Guardrail: only stop if we have a usable draft (at least one non-projectile primitive part).
                // If the agent says "done" too early (empty draft), continue so it can generate primitives.
                if draft.total_non_projectile_primitive_parts() == 0 {
                    workshop.error = Some(
                        "Agent requested done before generating any primitives; continuing."
                            .to_string(),
                    );
                    workshop.status = "Continuing Gen3D build… (agent ended early)".into();
                    job.agent.step_action_idx = job.agent.step_actions.len();
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        "agent_done_ignored (no primitives yet); continuing",
                    );
                    warn!("Gen3D agent requested done before primitives existed; continuing");
                    continue;
                }

                // If this is an edit-overwrite run, don't allow the agent to finish immediately
                // while QA has explicit errors. Overwrite saves are destructive: finishing early
                // would auto-save an invalid prefab into the realm/scene. We give the agent a
                // small bounded number of chances to apply QA fixits, then respect `done` as a
                // best-effort stop.
                if job.overwrite_save_blocked_by_qa_errors() {
                    const MAX_IGNORES: u8 = 2;
                    let ignore_idx = job.agent.done_ignored_due_to_qa_errors.saturating_add(1);
                    if ignore_idx <= MAX_IGNORES {
                        job.agent.done_ignored_due_to_qa_errors = ignore_idx;
                        workshop.error = Some(format!(
                            "Agent requested done but latest QA reported errors; continuing (ignore {ignore_idx}/{MAX_IGNORES}). Run `qa_v1`, apply fixits, then retry."
                        ));
                        workshop.status =
                            "Continuing Gen3D build… (QA errors block overwrite save)".to_string();
                        append_gen3d_run_log(
                            job.pass_dir.as_deref(),
                            format!(
                                "agent_done_ignored (qa errors; overwrite save) validate_ok={:?} smoke_ok={:?} motion_ok={:?}",
                                job.agent.last_validate_ok,
                                job.agent.last_smoke_ok,
                                job.agent.last_motion_ok
                            ),
                        );
                        warn!(
                            "Gen3D agent requested done but QA reported errors; continuing (overwrite save) validate_ok={:?} smoke_ok={:?} motion_ok={:?}",
                            job.agent.last_validate_ok,
                            job.agent.last_smoke_ok,
                            job.agent.last_motion_ok
                        );
                        job.agent.step_action_idx = job.agent.step_actions.len();
                        continue;
                    }
                }

                // Stop means stop: respect `done` even if QA/review are incomplete.
                // However, keep "unfinished" state visible in the UI status message.
                let llm_available = job
                    .ai
                    .as_ref()
                    .map(|ai| !ai.base_url().starts_with("mock://gen3d"))
                    .unwrap_or(true);
                let appearance_review_enabled = llm_available && job.review_appearance;

                let mut unfinished: Vec<String> = Vec::new();
                if appearance_review_enabled && !job.agent.ever_rendered {
                    unfinished.push(format!("No `{TOOL_ID_RENDER_PREVIEW}` has been run."));
                }
                if appearance_review_enabled && job.agent.rendered_since_last_review {
                    unfinished.push("Latest preview renders have not been reviewed.".into());
                }
                if llm_available && !job.agent.ever_reviewed {
                    unfinished.push(format!("No `{TOOL_ID_LLM_REVIEW_DELTA}` has been run."));
                }
                if !job.agent.ever_validated || !job.agent.ever_smoke_checked {
                    let mut missing: Vec<&str> = Vec::new();
                    if !job.agent.ever_validated {
                        missing.push(TOOL_ID_VALIDATE);
                    }
                    if !job.agent.ever_smoke_checked {
                        missing.push(TOOL_ID_SMOKE_CHECK);
                    }
                    unfinished.push(format!(
                        "QA not run: {} (recommended: `qa_v1`).",
                        missing.join(", ")
                    ));
                } else if job.agent.last_smoke_ok == Some(false) {
                    unfinished.push("Latest smoke check reported ok=false.".into());
                }
                if job.agent.last_motion_ok == Some(false) {
                    unfinished.push("Latest motion_validation reported ok=false.".into());
                }

                let movable = draft
                    .root_def()
                    .and_then(|def| def.mobility.as_ref())
                    .is_some();
                if movable {
                    let has_move = job.planned_components.iter().any(|c| {
                        c.attach_to.as_ref().is_some_and(|att| {
                            att.animations
                                .iter()
                                .any(|slot| slot.channel.as_ref() == "move")
                        })
                    });
                    let has_action = job.planned_components.iter().any(|c| {
                        c.attach_to.as_ref().is_some_and(|att| {
                            att.animations
                                .iter()
                                .any(|slot| slot.channel.as_ref() == "action")
                        })
                    });

                    if !has_move || !has_action {
                        let mut missing: Vec<&str> = Vec::new();
                        if !has_move {
                            missing.push("move");
                        }
                        if !has_action {
                            missing.push("action");
                        }
                        unfinished.push(format!(
                            "Movable unit missing authored channel(s): {} (suggestion: `{TOOL_ID_LLM_GENERATE_MOTIONS}` with channels={:?}, then `{TOOL_ID_SMOKE_CHECK}`).",
                            missing.join(", "),
                            missing,
                        ));
                    }
                }

                let mut status = if reason.trim().is_empty() {
                    "Build finished.".to_string()
                } else {
                    "Build finished.".to_string()
                };

                let summary_json = engine_run_summary_json(job, draft);
                status.push_str("\n\nSummary:");
                status.push_str(&format!(
                    "\n- assembly_rev: {}",
                    summary_json
                        .get("assembly_rev")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                ));
                status.push_str(&format!(
                    "\n- planned_components_total: {}",
                    summary_json
                        .get("planned_components_total")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                ));
                status.push_str(&format!(
                    "\n- primitive_parts_total: {}",
                    summary_json
                        .get("primitive_parts_total")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                ));
                status.push_str(&format!(
                    "\n- validate_ok: {}",
                    summary_json
                        .get("qa")
                        .and_then(|v| v.get("last_validate_ok"))
                        .and_then(|v| v.as_bool())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ));
                status.push_str(&format!(
                    "\n- smoke_ok: {}",
                    summary_json
                        .get("qa")
                        .and_then(|v| v.get("last_smoke_ok"))
                        .and_then(|v| v.as_bool())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ));
                status.push_str(&format!(
                    "\n- motion_ok: {}",
                    summary_json
                        .get("qa")
                        .and_then(|v| v.get("last_motion_ok"))
                        .and_then(|v| v.as_bool())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ));

                let agent_note = reason.trim();
                let agent_note_capped = super::truncate_for_ui(agent_note, 800);
                if !agent_note.is_empty() {
                    status.push_str("\n\nAgent note (unverified): ");
                    status.push_str(&super::truncate_for_ui(agent_note, 360));
                }
                if !unfinished.is_empty() {
                    status.push_str("\n\nUnfinished checks (best effort):");
                    for item in unfinished.iter() {
                        status.push_str("\n- ");
                        status.push_str(item.trim());
                    }
                }
                append_qa_warnings_to_status(&mut status, &job.agent);

                job.append_info_event_best_effort(
                    super::info_store::InfoEventKindV1::EngineLog,
                    None,
                    None,
                    "Agent done.".into(),
                    serde_json::json!({
                        "kind": "agent_done",
                        "agent_note": agent_note_capped,
                        "agent_note_truncated": agent_note_capped != agent_note,
                        "summary": summary_json,
                        "unfinished_checks": unfinished,
                    }),
                );

                workshop.error = None;
                start_finish_run_sequence(
                    config,
                    commands,
                    images,
                    workshop,
                    job,
                    draft,
                    Gen3dPendingFinishRun {
                        workshop_status: status,
                        run_log: format!(
                            "agent_done assembly_rev={} note={:?}",
                            job.assembly_rev,
                            super::truncate_for_ui(agent_note, 240)
                        ),
                        info_log: format!(
                            "Gen3D agent: done. assembly_rev={} validate_ok={:?} smoke_ok={:?} motion_ok={:?} note={:?}",
                            job.assembly_rev,
                            job.agent.last_validate_ok,
                            job.agent.last_smoke_ok,
                            job.agent.last_motion_ok,
                            super::truncate_for_ui(agent_note, 240)
                        ),
                    },
                );
                return;
            }
            Gen3dAgentActionJsonV1::ToolCall {
                call_id,
                tool_id,
                args,
            } => {
                let call = Gen3dToolCallJsonV1 {
                    call_id,
                    tool_id,
                    args,
                };
                job.metrics
                    .note_tool_call_started(call.call_id.as_str(), call.tool_id.as_str());
                status_steps::log_tool_call_started(workshop, &call);
                append_agent_trace_event_v1(
                    job.run_dir.as_deref(),
                    &AgentTraceEventV1::ToolCall {
                        call_id: call.call_id.clone(),
                        tool_id: call.tool_id.clone(),
                        args: call.args.clone(),
                    },
                );
                append_gen3d_jsonl_artifact(
                    job.pass_dir.as_deref(),
                    "tool_calls.jsonl",
                    &serde_json::json!({
                        "call_id": call.call_id.clone(),
                        "tool_id": call.tool_id.clone(),
                        "args": call.args.clone(),
                    }),
                );
                let call_id_for_log = call.call_id.clone();
                let tool_id_for_log = call.tool_id.clone();
                append_gen3d_run_log(
                    job.pass_dir.as_deref(),
                    format!(
                        "tool_call_start call_id={} tool_id={} args={}",
                        call.call_id,
                        call.tool_id,
                        truncate_json_for_log(&call.args, 600)
                    ),
                );
                debug!(
                    "Gen3D tool call start: call_id={} tool_id={} args={}",
                    call.call_id,
                    call.tool_id,
                    truncate_json_for_log(&call.args, 600)
                );
                job.append_info_event_best_effort(
                    super::info_store::InfoEventKindV1::ToolCallStart,
                    Some(call.tool_id.clone()),
                    Some(call.call_id.clone()),
                    format!("Tool call start: {}", call.tool_id),
                    serde_json::json!({ "args": call.args.clone() }),
                );

                match execute_tool_call(
                    config,
                    time,
                    commands,
                    images,
                    workshop,
                    feedback_history,
                    job,
                    draft,
                    preview,
                    preview_model,
                    call,
                ) {
                    ToolCallOutcome::Immediate(result) => {
                        job.metrics.note_tool_result(&result);
                        status_steps::log_tool_call_finished(workshop, job, &*draft, &result);
                        append_gen3d_run_log(
                            job.pass_dir.as_deref(),
                            format!(
                                "tool_call_result call_id={} tool_id={} ok={} {}",
                                result.call_id,
                                result.tool_id,
                                result.ok,
                                if result.ok {
                                    format!(
                                        "result={}",
                                        result
                                            .result
                                            .as_ref()
                                            .map(|v| truncate_json_for_log(v, 900))
                                            .unwrap_or_else(|| "<none>".into())
                                    )
                                } else {
                                    format!("error={}", result.error.as_deref().unwrap_or("<none>"))
                                }
                            ),
                        );
                        if result.ok {
                            debug!(
                                "Gen3D tool call ok: call_id={} tool_id={} result={}",
                                result.call_id,
                                result.tool_id,
                                result
                                    .result
                                    .as_ref()
                                    .map(|v| truncate_json_for_log(v, 900))
                                    .unwrap_or_else(|| "<none>".into())
                            );
                        } else {
                            let err = result.error.as_deref().unwrap_or("<none>");
                            if err.starts_with("Refusing force:true regeneration") {
                                debug!(
                                    "Gen3D tool call refused: call_id={} tool_id={} error={}",
                                    result.call_id, result.tool_id, err
                                );
                            } else {
                                warn!(
                                    "Gen3D tool call failed: call_id={} tool_id={} error={}",
                                    result.call_id, result.tool_id, err
                                );
                            }
                        }
                        append_agent_trace_event_v1(
                            job.run_dir.as_deref(),
                            &AgentTraceEventV1::ToolResult {
                                call_id: result.call_id.clone(),
                                tool_id: result.tool_id.clone(),
                                ok: result.ok,
                                result: result.result.clone(),
                                error: result.error.clone(),
                            },
                        );
                        append_gen3d_jsonl_artifact(
                            job.pass_dir.as_deref(),
                            "tool_results.jsonl",
                            &serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                        );
                        let message = if result.ok {
                            format!("Tool call ok: {}", result.tool_id)
                        } else {
                            let err = result.error.as_deref().unwrap_or("").trim();
                            let first_line = err.split('\n').next().unwrap_or("");
                            if first_line.is_empty() {
                                format!("Tool call error: {}", result.tool_id)
                            } else {
                                format!(
                                    "Tool call error: {}: {}",
                                    result.tool_id,
                                    super::truncate_for_ui(first_line, 240)
                                )
                            }
                        };
                        job.append_info_event_best_effort(
                            super::info_store::InfoEventKindV1::ToolCallResult,
                            Some(result.tool_id.clone()),
                            Some(result.call_id.clone()),
                            message,
                            serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                        );
                        note_observable_tool_result(job, &result);
                        job.agent.step_tool_results.push(result);
                        if job
                            .agent
                            .step_tool_results
                            .last()
                            .map(|r| !r.ok)
                            .unwrap_or(false)
                        {
                            // End the step early on tool failures so the agent can adapt.
                            // Continuing to execute the remaining tool calls tends to cascade
                            // errors because later actions usually depend on earlier outputs.
                            job.agent.step_action_idx = job.agent.step_actions.len();
                            return;
                        }
                        job.agent.step_action_idx += 1;
                        executed += 1;
                        continue;
                    }
                    ToolCallOutcome::StartedAsync => {
                        // Tool execution will resume once async work completes.
                        append_gen3d_run_log(
                            job.pass_dir.as_deref(),
                            format!(
                                "tool_call_async_started call_id={} tool_id={}",
                                call_id_for_log, tool_id_for_log
                            ),
                        );
                        debug!(
                            "Gen3D tool call started async: call_id={} tool_id={}",
                            call_id_for_log, tool_id_for_log
                        );
                        job.agent.step_action_idx += 1;
                        return;
                    }
                }
            }
        }
    }
}

pub(super) fn maybe_start_pass_snapshot_capture(
    config: &AppConfig,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    after: super::Gen3dAgentAfterPassSnapshot,
) -> bool {
    if !config.gen3d_save_pass_screenshots {
        return false;
    }
    if draft.total_non_projectile_primitive_parts() == 0 {
        return false;
    }
    if job.agent.pending_pass_snapshot.is_some() {
        return false;
    }
    let Some(pass_dir) = job.pass_dir.clone() else {
        return false;
    };

    let views = [
        super::Gen3dReviewView::Front,
        super::Gen3dReviewView::LeftBack,
        super::Gen3dReviewView::RightBack,
        super::Gen3dReviewView::Top,
        super::Gen3dReviewView::Bottom,
    ];
    match super::start_gen3d_review_capture(
        commands,
        images,
        &pass_dir,
        draft,
        false,
        "pass",
        &views,
        super::super::GEN3D_PREVIEW_WIDTH_PX,
        super::super::GEN3D_PREVIEW_HEIGHT_PX,
    ) {
        Ok(state) => {
            job.agent.pending_pass_snapshot = Some(state);
            job.agent.pending_after_pass_snapshot = Some(after);
            job.phase = Gen3dAiPhase::AgentCapturingPassSnapshot;
            if let Some(progress) = job.shared_progress.as_ref() {
                set_progress(progress, "Saving pass screenshots… (0/5)");
            }
            true
        }
        Err(err) => {
            warn!(
                "Gen3D: failed to start pass snapshot capture in {}: {err}",
                pass_dir.display()
            );
            workshop.error = Some(format!("Gen3D: pass screenshot capture failed: {err}"));
            false
        }
    }
}

pub(super) fn poll_agent_pass_snapshot_capture(
    config: &AppConfig,
    commands: &mut Commands,
    _images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    _feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
) {
    let Some(state) = job.agent.pending_pass_snapshot.as_ref() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing pending pass snapshot",
        );
        return;
    };
    let (done, expected) = state
        .progress
        .lock()
        .map(|g| (g.completed, g.expected))
        .unwrap_or((0, 1));
    if done < expected {
        if let Some(progress) = job.shared_progress.as_ref() {
            set_progress(
                progress,
                format!("Saving pass screenshots… ({done}/{expected})"),
            );
        }
        return;
    }

    let Some(state) = job.agent.pending_pass_snapshot.take() else {
        return;
    };
    for cam in state.cameras.iter().copied() {
        commands.entity(cam).try_despawn();
    }
    let paths = state.image_paths.clone();
    for path in &paths {
        if std::fs::metadata(path).is_err() {
            warn!(
                "Gen3D: pass snapshot missing output file: {}",
                path.display()
            );
        }
    }

    let Some(after) = job.agent.pending_after_pass_snapshot.take() else {
        warn!("Gen3D: missing after-pass-snapshot continuation; resuming build.");
        job.phase = Gen3dAiPhase::AgentExecutingActions;
        return;
    };

    match after {
        super::Gen3dAgentAfterPassSnapshot::AdvancePassAndRequestStep => {
            if let Err(err) = gen3d_advance_pass(job) {
                fail_job(workshop, job, err);
                return;
            }
            let Some(pass_dir) = job.pass_dir.clone() else {
                fail_job(workshop, job, "Internal error: missing pass dir");
                return;
            };
            append_gen3d_run_log(Some(&pass_dir), "agent_step_complete; requesting next step");
            debug!("Gen3D agent: step complete; requesting next step");
            job.phase = Gen3dAiPhase::AgentWaitingStep;
            job.agent.step_repair_attempt = 0;
            let _ = spawn_agent_step_request(config, workshop, job, pass_dir);
        }
        super::Gen3dAgentAfterPassSnapshot::FinishRun {
            workshop_status,
            run_log,
            info_log,
        } => {
            workshop.status = workshop_status;
            append_gen3d_run_log(job.pass_dir.as_deref(), run_log);
            info!("{info_log}");
            job.finish_run_metrics();
            job.running = false;
            job.build_complete = true;
            job.phase = Gen3dAiPhase::Idle;
            job.shared_progress = None;
            job.shared_result = None;
        }
    }
}

pub(super) enum ToolCallOutcome {
    Immediate(Gen3dToolResultJsonV1),
    StartedAsync,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_qa_warnings_to_status_noop_without_warnings() {
        let mut status = "Build finished.".to_string();
        let agent = super::super::Gen3dAgentState::default();
        append_qa_warnings_to_status(&mut status, &agent);
        assert_eq!(status, "Build finished.");

        let mut agent = super::super::Gen3dAgentState::default();
        agent.last_qa_warnings_count = Some(0);
        let mut status = "Build finished.".to_string();
        append_qa_warnings_to_status(&mut status, &agent);
        assert_eq!(status, "Build finished.");
    }

    #[test]
    fn append_qa_warnings_to_status_includes_count_and_example() {
        let mut agent = super::super::Gen3dAgentState::default();
        agent.last_qa_warnings_count = Some(2);
        agent.last_qa_warning_example = Some(
            "motion_validation jaw_lower attack_self_intersection: Attack animation increases self-intersection relative to idle pose."
                .to_string(),
        );

        let mut status = "Build finished.".to_string();
        append_qa_warnings_to_status(&mut status, &agent);

        assert!(status.contains("QA warnings (non-blocking):"));
        assert!(status.contains("count: 2"));
        assert!(status.contains("example: motion_validation jaw_lower attack_self_intersection"));
    }

    #[test]
    fn overwrite_save_blocked_by_qa_errors_requires_overwrite_mode() {
        let mut job = super::super::Gen3dAiJob::default();
        job.agent.last_smoke_ok = Some(false);
        assert!(!job.overwrite_save_blocked_by_qa_errors());

        job.set_save_overwrite_prefab_id(Some(123));
        assert!(job.overwrite_save_blocked_by_qa_errors());
    }

    #[test]
    fn overwrite_save_blocked_by_qa_errors_triggers_only_on_explicit_errors() {
        let mut job = super::super::Gen3dAiJob::default();
        job.set_save_overwrite_prefab_id(Some(123));
        assert!(!job.overwrite_save_blocked_by_qa_errors());

        job.agent.last_validate_ok = Some(true);
        job.agent.last_smoke_ok = Some(true);
        job.agent.last_motion_ok = Some(true);
        assert!(!job.overwrite_save_blocked_by_qa_errors());

        job.agent.last_validate_ok = Some(false);
        assert!(job.overwrite_save_blocked_by_qa_errors());
    }
}
