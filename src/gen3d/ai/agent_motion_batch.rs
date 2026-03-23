use bevy::log::{debug, warn};
use bevy::prelude::*;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::Gen3dToolResultJsonV1;
use crate::threaded_result::{new_shared_result, take_shared_result, SharedResult};

use super::super::state::{Gen3dDraft, Gen3dWorkshop};
use super::agent_utils::sanitize_prefix;
use super::artifacts::{
    append_gen3d_run_log, write_gen3d_assembly_snapshot, write_gen3d_json_artifact,
    write_gen3d_text_artifact,
};
use super::parse;
use super::{
    fail_job, set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiProgress,
    Gen3dAiTextResponse,
};

#[derive(Clone, Debug)]
pub(super) struct ApplyMotionAuthoringSummary {
    pub(super) decision: &'static str,
    pub(super) edges: usize,
}

pub(super) fn apply_motion_authoring_for_channel(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    authored: &super::schema::AiMotionAuthoringJsonV1,
    expected_channel: &str,
) -> Result<ApplyMotionAuthoringSummary, String> {
    use crate::object::registry::{
        PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef, PartAnimationSlot,
        PartAnimationSpec,
    };

    let expected_run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
    let expected_channel = expected_channel.trim().to_ascii_lowercase();
    if expected_channel.is_empty() {
        return Err("Missing expected_channel for motion authoring application.".into());
    }

    let mut issues: Vec<String> = Vec::new();
    if !expected_run_id.trim().is_empty() && authored.applies_to.run_id.trim() != expected_run_id {
        issues.push(format!(
            "applies_to.run_id mismatch (got {}, expected {})",
            authored.applies_to.run_id.trim(),
            expected_run_id.trim()
        ));
    }
    if authored.applies_to.attempt != job.attempt
        || authored.applies_to.plan_hash.trim() != job.plan_hash.trim()
        || authored.applies_to.assembly_rev != job.assembly_rev
    {
        issues.push(format!(
            "applies_to mismatch (got attempt={} plan_hash={} assembly_rev={}, expected attempt={} plan_hash={} assembly_rev={})",
            authored.applies_to.attempt,
            authored.applies_to.plan_hash.trim(),
            authored.applies_to.assembly_rev,
            job.attempt,
            job.plan_hash.trim(),
            job.assembly_rev,
        ));
    }

    match authored.decision {
        super::schema::AiMotionAuthoringDecisionJsonV1::RegenGeometryRequired => {
            if !authored.replace_channels.is_empty() || !authored.edges.is_empty() {
                issues.push(
                    "decision=regen_geometry_required must set replace_channels=[] and edges=[] (do not author clips)."
                        .to_string(),
                );
            }
            if authored.reason.trim().is_empty() {
                issues
                    .push("reason must be non-empty when decision=regen_geometry_required".into());
            }
        }
        super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips => {
            if authored.replace_channels.len() != 1
                || authored.replace_channels[0].as_str() != expected_channel
            {
                issues.push(format!(
                    "replace_channels must be exactly [\"{expected_channel}\"] for single-channel motion authoring (got {:?})",
                    authored.replace_channels
                ));
            }
            if authored.edges.is_empty() {
                issues.push("edges must be non-empty when decision=author_clips".to_string());
            }
            for edge in authored.edges.iter() {
                for slot in edge.slots.iter() {
                    if slot.channel.as_str() != expected_channel {
                        issues.push(format!(
                            "slot.channel must be {expected_channel:?} for component `{}` (got `{}`)",
                            edge.component.trim(),
                            slot.channel.as_str()
                        ));
                    }
                }
            }
        }
        super::schema::AiMotionAuthoringDecisionJsonV1::Unknown => {
            issues.push(
                "AI motion-authoring has invalid `decision` value (expected `author_clips` or `regen_geometry_required`)."
                    .to_string(),
            );
        }
    }

    if !issues.is_empty() {
        issues.sort();
        issues.dedup();
        return Err(format!(
            "motion-authoring validation failed:\n- {}",
            issues.join("\n- ")
        ));
    }

    if matches!(
        authored.decision,
        super::schema::AiMotionAuthoringDecisionJsonV1::RegenGeometryRequired
    ) {
        return Ok(ApplyMotionAuthoringSummary {
            decision: "regen_geometry_required",
            edges: 0,
        });
    }

    let mut name_to_idx: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (idx, c) in job.planned_components.iter().enumerate() {
        name_to_idx.insert(c.name.clone(), idx);
    }

    fn driver_from_ai(
        driver: super::schema::AiAnimationDriverJsonV1,
    ) -> Option<PartAnimationDriver> {
        match driver {
            super::schema::AiAnimationDriverJsonV1::Always => Some(PartAnimationDriver::Always),
            super::schema::AiAnimationDriverJsonV1::MovePhase => {
                Some(PartAnimationDriver::MovePhase)
            }
            super::schema::AiAnimationDriverJsonV1::MoveDistance => {
                Some(PartAnimationDriver::MoveDistance)
            }
            super::schema::AiAnimationDriverJsonV1::AttackTime => {
                Some(PartAnimationDriver::AttackTime)
            }
            super::schema::AiAnimationDriverJsonV1::ActionTime => {
                Some(PartAnimationDriver::ActionTime)
            }
            super::schema::AiAnimationDriverJsonV1::Unknown => None,
        }
    }

    fn transform_from_delta(delta: &super::schema::AiAnimationDeltaTransformJsonV1) -> Transform {
        let translation =
            delta
                .pos
                .unwrap_or([0.0, 0.0, 0.0])
                .map(|v| if v.is_finite() { v } else { 0.0 });
        let translation = Vec3::new(translation[0], translation[1], translation[2]);

        let scale = delta
            .scale
            .unwrap_or([1.0, 1.0, 1.0])
            .map(|v| if v.is_finite() { v } else { 1.0 });
        let scale = Vec3::new(scale[0], scale[1], scale[2]);

        let rotation = match delta.rot_quat_xyzw {
            Some([x, y, z, w]) => {
                let q = Quat::from_xyzw(x, y, z, w);
                if q.is_finite() {
                    q.normalize()
                } else {
                    Quat::IDENTITY
                }
            }
            _ => Quat::IDENTITY,
        };

        Transform {
            translation,
            rotation,
            scale,
        }
    }

    let replace: std::collections::HashSet<&str> = authored
        .replace_channels
        .iter()
        .map(|s| s.as_str())
        .collect();
    let mut issues: Vec<String> = Vec::new();

    for edge in authored.edges.iter() {
        let name = edge.component.trim();
        if name.is_empty() {
            continue;
        }
        let Some(&component_idx) = name_to_idx.get(name) else {
            issues.push(format!("Unknown component: {name}"));
            continue;
        };
        if job.planned_components[component_idx].attach_to.is_none() {
            issues.push(format!(
                "Component {name} is the root (no attach_to); cannot author edge motion"
            ));
            continue;
        }

        let mut replacement_slots: Vec<PartAnimationSlot> = Vec::new();
        let mut channels_seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for slot in edge.slots.iter() {
            let channel = slot.channel.trim();
            if channel.is_empty() {
                continue;
            }
            if channel != "attack_primary" && !channels_seen.insert(channel) {
                issues.push(format!(
                    "Duplicate channel `{channel}` for component `{name}` (only attack_primary may have multiple variants)"
                ));
                continue;
            }

            let Some(driver) = driver_from_ai(slot.driver) else {
                issues.push(format!(
                    "Unknown driver for component `{name}` channel `{channel}`"
                ));
                continue;
            };

            let speed_scale = slot.speed_scale.abs().max(1e-3);
            let time_offset_units = slot.time_offset_units;

            let clip = match &slot.clip {
                super::schema::AiAnimationClipJsonV1::Loop {
                    duration_units,
                    keyframes,
                } => PartAnimationDef::Loop {
                    duration_secs: duration_units.abs().max(1e-3),
                    keyframes: keyframes
                        .iter()
                        .map(|kf| PartAnimationKeyframeDef {
                            time_secs: kf.t_units,
                            delta: transform_from_delta(&kf.delta),
                        })
                        .collect(),
                },
                super::schema::AiAnimationClipJsonV1::Once {
                    duration_units,
                    keyframes,
                } => PartAnimationDef::Once {
                    duration_secs: duration_units.abs().max(1e-3),
                    keyframes: keyframes
                        .iter()
                        .map(|kf| PartAnimationKeyframeDef {
                            time_secs: kf.t_units,
                            delta: transform_from_delta(&kf.delta),
                        })
                        .collect(),
                },
                super::schema::AiAnimationClipJsonV1::PingPong {
                    duration_units,
                    keyframes,
                } => PartAnimationDef::PingPong {
                    duration_secs: duration_units.abs().max(1e-3),
                    keyframes: keyframes
                        .iter()
                        .map(|kf| PartAnimationKeyframeDef {
                            time_secs: kf.t_units,
                            delta: transform_from_delta(&kf.delta),
                        })
                        .collect(),
                },
                super::schema::AiAnimationClipJsonV1::Spin {
                    axis,
                    radians_per_unit,
                    axis_space,
                } => PartAnimationDef::Spin {
                    axis: Vec3::new(axis[0], axis[1], axis[2]),
                    radians_per_unit: *radians_per_unit,
                    axis_space: axis_space.to_space(),
                },
            };

            replacement_slots.push(PartAnimationSlot {
                channel: channel.to_string().into(),
                spec: PartAnimationSpec {
                    driver,
                    speed_scale,
                    time_offset_units,
                    basis: Transform::IDENTITY,
                    clip,
                },
            });
        }

        if let Some(att) = job.planned_components[component_idx].attach_to.as_mut() {
            att.animations
                .retain(|slot| !replace.contains(slot.channel.as_ref()));
            att.animations.extend(replacement_slots);
            super::internal_base_slot::normalize_internal_base_slot(&mut att.animations);
        }
    }

    if !issues.is_empty() {
        issues.sort();
        issues.dedup();
        return Err(format!(
            "motion-authoring validation failed:\n- {}",
            issues.join("\n- ")
        ));
    }

    if let Err(err) = super::convert::sync_attachment_tree_to_defs(&job.planned_components, draft) {
        fail_job(
            workshop,
            job,
            format!("Failed to apply motion-authoring: {err}"),
        );
        return Err(format!("Failed to apply motion-authoring: {err}"));
    }
    write_gen3d_assembly_snapshot(job.pass_dir.as_deref(), &job.planned_components);

    let movable = draft
        .root_def()
        .and_then(|def| def.mobility.as_ref())
        .is_some();
    if movable && expected_channel == "move" {
        let has_move = job.planned_components.iter().any(|c| {
            c.attach_to.as_ref().is_some_and(|att| {
                att.animations
                    .iter()
                    .any(|slot| slot.channel.as_ref() == "move")
            })
        });
        if !has_move {
            return Err(
                "decision=author_clips must produce at least one `move` animation slot for movable drafts."
                    .to_string(),
            );
        }
    }
    if movable && expected_channel == "action" {
        let has_action = job.planned_components.iter().any(|c| {
            c.attach_to.as_ref().is_some_and(|att| {
                att.animations
                    .iter()
                    .any(|slot| slot.channel.as_ref() == "action")
            })
        });
        if !has_action {
            return Err(
                "decision=author_clips must produce at least one `action` animation slot for movable drafts."
                    .to_string(),
            );
        }
    }

    job.motion_authoring = Some(authored.clone());

    Ok(ApplyMotionAuthoringSummary {
        decision: "author_clips",
        edges: authored.edges.len(),
    })
}

pub(super) fn poll_agent_motion_batch(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
) -> Option<Gen3dToolResultJsonV1> {
    const MAX_MOTION_RETRIES: u8 = 1;

    let Some(mut batch) = job.agent.pending_motion_batch.take() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing pending motion batch state.",
        );
        return None;
    };
    let Some(call_id_for_prefix) = job
        .agent
        .pending_tool_call
        .as_ref()
        .map(|c| c.call_id.clone())
    else {
        fail_job(workshop, job, "Internal error: missing pending tool call.");
        return None;
    };

    let total = batch.requested_channels.len();
    if total == 0 {
        let call = job.agent.pending_tool_call.take().unwrap();
        job.agent.pending_llm_tool = None;
        job.agent.pending_motion_batch = None;
        job.motion_queue.clear();
        job.motion_in_flight.clear();
        job.motion_attempts.clear();
        return Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "requested": 0,
                "succeeded": [],
                "failed": [],
            }),
        ));
    }

    // 1) Apply any completed motion results.
    let mut i = 0usize;
    while i < job.motion_in_flight.len() {
        let Some(result) = take_shared_result(&job.motion_in_flight[i].shared_result) else {
            i += 1;
            continue;
        };

        let task = job.motion_in_flight.swap_remove(i);
        let channel = task.channel.clone();

        match result {
            Ok(resp) => {
                debug!(
                    "Gen3D batch: motion finished (channel={}, api={:?})",
                    channel, resp.api
                );
                job.note_api_used(resp.api);
                if let Some(tokens) = resp.total_tokens {
                    job.add_tokens(tokens);
                }
                job.session = resp.session;

                if let Some(dir) = job.pass_dir.as_deref() {
                    write_gen3d_text_artifact(
                        Some(dir),
                        format!("motion_{}_raw.txt", channel.as_str()),
                        resp.text.trim(),
                    );
                }

                let authored = match parse::parse_ai_motion_authoring_from_text(&resp.text) {
                    Ok(v) => v,
                    Err(err) => {
                        if task.attempt < MAX_MOTION_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D batch: motion request failed; retrying (channel={}, attempt {}/{}) err={}",
                                channel,
                                next + 1,
                                MAX_MOTION_RETRIES + 1,
                                super::truncate_for_ui(&err, 600),
                            );
                            job.motion_attempts.insert(channel.clone(), next);
                            job.motion_queue.insert(0, channel);
                            continue;
                        }
                        batch.completed_channels.insert(channel.clone());
                        batch.failed.push(super::Gen3dMotionBatchFailure {
                            channel,
                            error: err,
                        });
                        continue;
                    }
                };

                let summary = match apply_motion_authoring_for_channel(
                    workshop,
                    job,
                    draft,
                    &authored,
                    channel.as_str(),
                ) {
                    Ok(summary) => summary,
                    Err(err) => {
                        if task.attempt < MAX_MOTION_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D batch: motion apply failed; retrying (channel={}, attempt {}/{}) err={}",
                                channel,
                                next + 1,
                                MAX_MOTION_RETRIES + 1,
                                super::truncate_for_ui(&err, 600),
                            );
                            job.motion_attempts.insert(channel.clone(), next);
                            job.motion_queue.insert(0, channel);
                            continue;
                        }
                        batch.completed_channels.insert(channel.clone());
                        batch.failed.push(super::Gen3dMotionBatchFailure {
                            channel,
                            error: err,
                        });
                        continue;
                    }
                };

                if let Some(dir) = job.pass_dir.as_deref() {
                    write_gen3d_json_artifact(
                        Some(dir),
                        format!("motion_{}.json", channel.as_str()),
                        &serde_json::to_value(&authored).unwrap_or(serde_json::Value::Null),
                    );
                    write_gen3d_json_artifact(
                        Some(dir),
                        format!("motion_{}_result.json", channel.as_str()),
                        &serde_json::json!({
                            "ok": true,
                            "channel": channel,
                            "decision": summary.decision,
                            "edges": summary.edges,
                        }),
                    );
                }

                batch.completed_channels.insert(channel);
            }
            Err(err) => {
                if task.attempt < MAX_MOTION_RETRIES {
                    let next = task.attempt + 1;
                    warn!(
                        "Gen3D batch: motion request failed; retrying (channel={}, attempt {}/{}) err={}",
                        channel,
                        next + 1,
                        MAX_MOTION_RETRIES + 1,
                        super::truncate_for_ui(&err, 600),
                    );
                    job.motion_attempts.insert(channel.clone(), next);
                    job.motion_queue.insert(0, channel);
                    continue;
                }
                batch.completed_channels.insert(channel.clone());
                batch.failed.push(super::Gen3dMotionBatchFailure {
                    channel,
                    error: err,
                });
            }
        }
    }

    // 2) Start new motion requests up to the parallel limit.
    let mut parallel = job.max_parallel_components.max(1).min(total);
    if job.session.responses_previous_id.is_some()
        && job.session.responses_continuation_supported.is_none()
    {
        parallel = parallel.min(1);
    }
    while job.motion_in_flight.len() < parallel && !job.motion_queue.is_empty() {
        let channel = job.motion_queue.remove(0);
        if batch.completed_channels.contains(&channel) {
            continue;
        }

        let Some(ai) = job.ai.clone() else {
            fail_job(workshop, job, "Internal error: missing AI config.");
            return None;
        };
        let Some(pass_dir) = job.pass_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
            return None;
        };
        let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();

        let attempt = *job.motion_attempts.get(&channel).unwrap_or(&0);

        let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: "Starting…".into(),
        }));

        let (mut has_idle_slot, mut has_move_slot) = (false, false);
        for comp in job.planned_components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            for slot in att.animations.iter() {
                match slot.channel.as_ref() {
                    "idle" => has_idle_slot = true,
                    "move" => has_move_slot = true,
                    _ => {}
                }
            }
        }

        let system = super::prompts::build_gen3d_motion_authoring_system_instructions();
        let image_object_summary = job
            .user_image_object_summary
            .as_ref()
            .map(|s| s.text.as_str());
        let user_text = super::prompts::build_gen3d_motion_authoring_user_text(
            &job.user_prompt_raw,
            image_object_summary,
            &run_id,
            job.attempt,
            &job.plan_hash,
            job.assembly_rev,
            channel.as_str(),
            job.rig_move_cycle_m,
            has_idle_slot,
            has_move_slot,
            &job.planned_components,
            draft,
        );

        let prefix = if attempt == 0 {
            format!(
                "tool_motion_batch_{call_id}_{}",
                channel.as_str(),
                call_id = call_id_for_prefix.as_str()
            )
        } else {
            format!(
                "tool_motion_batch_{call_id}_{}_retry{}",
                channel.as_str(),
                attempt,
                call_id = call_id_for_prefix.as_str()
            )
        };

        append_gen3d_run_log(
            job.pass_dir.as_deref(),
            format!(
                "motion_batch_start channel={} attempt={} parallel={} total={}",
                channel.as_str(),
                attempt,
                parallel,
                total
            ),
        );

        let reasoning_effort = ai.model_reasoning_effort().to_string();
        spawn_gen3d_ai_text_thread(
            shared.clone(),
            progress.clone(),
            job.cancel_flag.clone(),
            job.session.clone(),
            Some(super::structured_outputs::Gen3dAiJsonSchemaKind::MotionAuthoringV1),
            config.gen3d_require_structured_outputs,
            ai,
            reasoning_effort,
            system,
            user_text,
            Vec::new(),
            pass_dir,
            sanitize_prefix(&prefix),
        );

        job.motion_in_flight.push(super::Gen3dInFlightMotion {
            channel,
            attempt,
            shared_result: shared,
            _progress: progress,
        });
    }

    let done = batch.completed_channels.len();
    let in_flight = job.motion_in_flight.len();
    let pending = job.motion_queue.len();
    workshop.status = format!(
        "Authoring motion channels (batch)… ({done}/{total})\nIn flight: {in_flight} | pending: {pending}\nParallel: {parallel}"
    );
    if let Some(progress) = job.shared_progress.as_ref() {
        set_progress(
            progress,
            format!("Authoring motion channels (batch)… ({done}/{total})"),
        );
    }

    if done == total && in_flight == 0 && pending == 0 {
        let failures_by_channel: std::collections::HashMap<&str, &str> = batch
            .failed
            .iter()
            .map(|f| (f.channel.as_str(), f.error.as_str()))
            .collect();
        let mut succeeded: Vec<String> = Vec::new();
        for ch in batch.requested_channels.iter() {
            if !failures_by_channel.contains_key(ch.as_str()) {
                succeeded.push(ch.clone());
            }
        }

        let failed_json: Vec<serde_json::Value> = batch
            .failed
            .iter()
            .map(|f| {
                serde_json::json!({
                    "channel": f.channel,
                    "error": super::truncate_for_ui(&f.error, 600),
                })
            })
            .collect();

        let call = job.agent.pending_tool_call.take().unwrap();
        job.agent.pending_llm_tool = None;
        job.agent.pending_motion_batch = None;
        job.motion_queue.clear();
        job.motion_in_flight.clear();
        job.motion_attempts.clear();

        if let Some(dir) = job.pass_dir.as_deref() {
            write_gen3d_json_artifact(
                Some(dir),
                "motion_batch_result.json",
                &serde_json::json!({
                    "requested_channels": batch.requested_channels,
                    "succeeded": succeeded,
                    "failed": failed_json,
                }),
            );
        }

        return Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "requested": total,
                "succeeded": succeeded,
                "failed": failed_json,
            }),
        ));
    }

    job.agent.pending_motion_batch = Some(batch);
    None
}
