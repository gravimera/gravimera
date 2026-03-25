// Gen3D tool runtime helpers (pipeline-only).
//
// Note: the legacy agent-step orchestrator was removed. This module still houses shared
// utilities used by the deterministic pipeline: tool-call dispatch outcomes and the
// best-effort finish sequence (descriptor-meta enrichment + pass screenshots).

use bevy::log::{info, warn};
use bevy::prelude::*;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::Gen3dToolResultJsonV1;
use crate::threaded_result::{new_shared_result, take_shared_result};

use super::super::state::{Gen3dDraft, Gen3dWorkshop};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::artifacts::append_gen3d_run_log;
use super::{
    fail_job, set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiPhase,
    Gen3dAiProgress, Gen3dAiTextResponse, Gen3dPendingFinishRun,
};

fn finalize_run_now(workshop: &mut Gen3dWorkshop, job: &mut Gen3dAiJob, finish: Gen3dPendingFinishRun) {
    workshop.status = finish.workshop_status;
    workshop
        .status_log
        .finish_step_if_active("Finished.".to_string());
    append_gen3d_run_log(job.step_dir.as_deref(), finish.run_log);
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
    use crate::object::registry::ObjectPartKind;
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
                if !ch.is_empty() {
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
    use crate::object::registry::{ObjectPartKind, PartAnimationDef, PartAnimationDriver};
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
                if channel.is_empty() {
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
                                PartAnimationDef::Spin { .. } => unreachable!("spin handled below"),
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
    if job.step_dir.is_none() {
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
        let stale =
            job.run_id != Some(in_flight.run_id) || job.plan_hash.trim() != in_flight.plan_hash.trim();
        if !stale {
            let in_flight = job.descriptor_meta_in_flight.take().unwrap();
            job.shared_result = Some(in_flight.shared_result);
            let progress = job
                .shared_progress
                .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                .clone();
            set_progress(&progress, "Waiting for prefab metadata…");
            append_gen3d_run_log(job.step_dir.as_deref(), "descriptor_meta_adopt_in_flight");
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
        crate::object::registry::UnitAttackKind::RangedProjectile => "ranged_projectile".to_string(),
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
        .attempt_dir()
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
    let Some(step_dir) = job.step_dir.clone() else {
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
    append_gen3d_run_log(Some(&step_dir), "descriptor_meta_start");

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
        step_dir,
        "descriptor_meta".into(),
    );

    job.pending_finish_run = Some(finish.clone());
    job.phase = Gen3dAiPhase::AgentWaitingDescriptorMeta;
    true
}

fn maybe_start_pass_snapshot_capture(
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
    let Some(step_dir) = job.step_dir.clone() else {
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
        &step_dir,
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
                step_dir.display()
            );
            workshop.error = Some(format!("Gen3D: pass screenshot capture failed: {err}"));
            false
        }
    }
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

pub(super) fn poll_agent_pass_snapshot_capture(
    _config: &AppConfig,
    commands: &mut Commands,
    _images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    _feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
) {
    let Some(state) = job.agent.pending_pass_snapshot.as_ref() else {
        fail_job(workshop, job, "Internal error: missing pending pass snapshot");
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
        super::Gen3dAgentAfterPassSnapshot::FinishRun {
            workshop_status,
            run_log,
            info_log,
        } => {
            finalize_run_now(
                workshop,
                job,
                Gen3dPendingFinishRun {
                    workshop_status,
                    run_log,
                    info_log,
                },
            );
        }
    }
}

pub(super) enum ToolCallOutcome {
    Immediate(Gen3dToolResultJsonV1),
    StartedAsync,
}
