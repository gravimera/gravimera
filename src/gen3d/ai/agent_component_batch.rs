use bevy::log::{debug, warn};
use bevy::prelude::*;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::Gen3dToolResultJsonV1;
use crate::threaded_result::{new_shared_result, take_shared_result, SharedResult};

use super::super::state::{Gen3dDraft, Gen3dWorkshop};
use super::agent_utils::sanitize_prefix;
use super::parse;
use super::{
    fail_job, set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiProgress,
    Gen3dAiTextResponse,
};

pub(super) fn poll_agent_component_batch(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    speed: super::super::state::Gen3dSpeedMode,
) -> Option<Gen3dToolResultJsonV1> {
    const MAX_COMPONENT_RETRIES: u8 = 1;

    let Some(mut batch) = job.agent.pending_component_batch.take() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing pending component batch state.",
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

    let total = batch.requested_indices.len();
    if total == 0 {
        let skipped_due_to_preserve_existing_components_json: Vec<serde_json::Value> = batch
            .skipped_due_to_preserve_existing_components
            .iter()
            .copied()
            .filter(|idx| *idx < job.planned_components.len())
            .map(|idx| {
                serde_json::json!({
                    "index": idx,
                    "name": job.planned_components[idx].name.as_str(),
                })
            })
            .collect();
        let call = job.agent.pending_tool_call.take().unwrap();
        job.agent.pending_llm_tool = None;
        job.agent.pending_component_batch = None;
        job.component_queue.clear();
        job.component_in_flight.clear();
        job.component_last_errors.clear();
        return Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "requested": 0,
                "succeeded": 0,
                "failed": [],
                "skipped_due_to_preserve_existing_components": skipped_due_to_preserve_existing_components_json,
                "skipped_due_to_regen_budget": batch.skipped_due_to_regen_budget,
            }),
        ));
    }

    // 1) Apply any completed component results.
    let mut i = 0usize;
    while i < job.component_in_flight.len() {
        let Some(result) = take_shared_result(&job.component_in_flight[i].shared_result) else {
            i += 1;
            continue;
        };

        let task = job.component_in_flight.swap_remove(i);
        let idx = task.idx;
        let component_name = job
            .planned_components
            .get(idx)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("component_{idx}"));

        match result {
            Ok(resp) => {
                debug!(
                    "Gen3D batch: component finished (idx={}, name={}, api={:?}, sent_images={})",
                    idx, component_name, resp.api, task.sent_images
                );
                job.note_api_used(resp.api);
                job.add_token_usage_from_response(
                    resp.input_tokens,
                    resp.output_tokens,
                    resp.total_tokens,
                );
                if let Some(flag) = resp.session.responses_supported {
                    job.session.responses_supported = Some(flag);
                }
                if let Some(flag) = resp.session.responses_stream_required {
                    job.session.responses_stream_required = Some(flag);
                }
                if let Some(flag) = resp.session.responses_continuation_supported {
                    job.session.responses_continuation_supported = Some(flag);
                }
                if let Some(flag) = resp.session.responses_background_supported {
                    job.session.responses_background_supported = Some(flag);
                }
                if let Some(flag) = resp.session.responses_structured_outputs_supported {
                    job.session.responses_structured_outputs_supported = Some(flag);
                }
                if let Some(flag) = resp.session.chat_stream_required {
                    job.session.chat_stream_required = Some(flag);
                }
                if let Some(flag) = resp.session.chat_structured_outputs_supported {
                    job.session.chat_structured_outputs_supported = Some(flag);
                }

                let ai = match parse::parse_ai_draft_from_text(&resp.text) {
                    Ok(ai) => ai,
                    Err(err) => {
                        if task.attempt < MAX_COMPONENT_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D batch: component parse failed; retrying (idx={}, name={}, attempt {}/{}) err={}",
                                idx,
                                component_name,
                                next + 1,
                                MAX_COMPONENT_RETRIES + 1,
                                super::truncate_for_ui(&err, 600),
                            );
                            if idx >= job.component_attempts.len() {
                                job.component_attempts
                                    .resize(job.planned_components.len(), 0);
                            }
                            job.component_attempts[idx] = next;
                            if idx >= job.component_last_errors.len() {
                                job.component_last_errors
                                    .resize(job.planned_components.len(), None);
                            }
                            job.component_last_errors[idx] = Some(err.clone());
                            job.component_queue.insert(0, idx);
                            continue;
                        }
                        batch.completed_indices.insert(idx);
                        batch.failed.push(super::Gen3dComponentBatchFailure {
                            index: idx,
                            name: component_name,
                            error: err,
                        });
                        continue;
                    }
                };

                let converted = match job
                    .planned_components
                    .get(idx)
                    .ok_or_else(|| {
                        format!("Internal error: missing planned component for idx={idx}")
                    })
                    .and_then(|planned| {
                        super::convert::ai_to_component_def(planned, ai, job.step_dir.as_deref())
                    }) {
                    Ok(def) => def,
                    Err(err) => {
                        if task.attempt < MAX_COMPONENT_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D batch: component convert failed; retrying (idx={}, name={}, attempt {}/{}) err={}",
                                idx,
                                component_name,
                                next + 1,
                                MAX_COMPONENT_RETRIES + 1,
                                super::truncate_for_ui(&err, 600),
                            );
                            if idx >= job.component_attempts.len() {
                                job.component_attempts
                                    .resize(job.planned_components.len(), 0);
                            }
                            job.component_attempts[idx] = next;
                            if idx >= job.component_last_errors.len() {
                                job.component_last_errors
                                    .resize(job.planned_components.len(), None);
                            }
                            job.component_last_errors[idx] = Some(err.clone());
                            job.component_queue.insert(0, idx);
                            continue;
                        }
                        batch.completed_indices.insert(idx);
                        batch.failed.push(super::Gen3dComponentBatchFailure {
                            index: idx,
                            name: component_name,
                            error: err,
                        });
                        continue;
                    }
                };
                let component_def = converted.def;
                let converted = super::convert::ConvertedComponentDef {
                    def: component_def,
                    articulation_nodes: converted.articulation_nodes,
                };
                if let Err(err) = super::component_regen::apply_regenerated_component(
                    workshop, job, draft, idx, converted,
                ) {
                    if task.attempt < MAX_COMPONENT_RETRIES {
                        let next = task.attempt + 1;
                        warn!(
                            "Gen3D batch: component integration failed; retrying (idx={}, name={}, attempt {}/{}) err={}",
                            idx,
                            component_name,
                            next + 1,
                            MAX_COMPONENT_RETRIES + 1,
                            super::truncate_for_ui(&err, 600),
                        );
                        if idx >= job.component_attempts.len() {
                            job.component_attempts
                                .resize(job.planned_components.len(), 0);
                        }
                        job.component_attempts[idx] = next;
                        if idx >= job.component_last_errors.len() {
                            job.component_last_errors
                                .resize(job.planned_components.len(), None);
                        }
                        job.component_last_errors[idx] = Some(err.clone());
                        job.component_queue.insert(0, idx);
                        continue;
                    }
                    batch.completed_indices.insert(idx);
                    batch.failed.push(super::Gen3dComponentBatchFailure {
                        index: idx,
                        name: component_name,
                        error: err,
                    });
                    continue;
                }

                batch.completed_indices.insert(idx);
                if idx < job.component_last_errors.len() {
                    job.component_last_errors[idx] = None;
                }
            }
            Err(err) => {
                if task.attempt < MAX_COMPONENT_RETRIES {
                    let next = task.attempt + 1;
                    warn!(
                        "Gen3D batch: component request failed; retrying (idx={}, name={}, attempt {}/{}, sent_images={}) err={}",
                        idx,
                        component_name,
                        next + 1,
                        MAX_COMPONENT_RETRIES + 1,
                        task.sent_images,
                        super::truncate_for_ui(&err, 600),
                    );
                    if idx >= job.component_attempts.len() {
                        job.component_attempts
                            .resize(job.planned_components.len(), 0);
                    }
                    job.component_attempts[idx] = next;
                    if idx >= job.component_last_errors.len() {
                        job.component_last_errors
                            .resize(job.planned_components.len(), None);
                    }
                    job.component_last_errors[idx] = Some(err.clone());
                    job.component_queue.insert(0, idx);
                    continue;
                }
                batch.completed_indices.insert(idx);
                batch.failed.push(super::Gen3dComponentBatchFailure {
                    index: idx,
                    name: component_name,
                    error: err,
                });
            }
        }
    }

    // 2) Start new component requests up to the parallel limit.
    let mut parallel = job.max_parallel_components.max(1).min(total);
    if job.session.responses_previous_id.is_some()
        && job.session.responses_continuation_supported.is_none()
    {
        parallel = parallel.min(1);
    }
    while job.component_in_flight.len() < parallel && !job.component_queue.is_empty() {
        let idx = job.component_queue.remove(0);
        if idx >= job.planned_components.len() {
            continue;
        }
        if batch.completed_indices.contains(&idx) {
            continue;
        }

        let Some(ai) = job.ai.clone() else {
            fail_job(workshop, job, "Internal error: missing AI config.");
            return None;
        };
        let Some(step_dir) = job.step_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D step dir.");
            return None;
        };

        let attempt = *job.component_attempts.get(idx).unwrap_or(&0);
        let image_paths = job.user_images_component.clone();
        let sent_images = !image_paths.is_empty();

        let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: "Starting…".into(),
        }));

        let system = super::prompts::build_gen3d_component_system_instructions();
        let image_object_summary = job
            .user_image_object_summary
            .as_ref()
            .map(|s| s.text.as_str());
        let user_text = super::prompts::build_gen3d_component_user_text(
            &job.user_prompt_raw,
            image_object_summary,
            speed,
            &job.assembly_notes,
            &job.planned_components,
            idx,
        );
        let mut user_text = user_text;
        if attempt > 0 {
            if let Some(err) = job
                .component_last_errors
                .get(idx)
                .and_then(|v| v.as_deref())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                user_text.push_str("\nPrevious attempt error:\n");
                user_text.push_str(&super::truncate_for_ui(err, 1200));
                user_text.push('\n');
            }
        }

        let component_name = job.planned_components[idx].name.as_str();
        let prefix = if attempt == 0 {
            format!(
                "tool_component_batch_{call_id}_c{:02}_{}",
                idx + 1,
                component_name,
                call_id = call_id_for_prefix.as_str()
            )
        } else {
            format!(
                "tool_component_batch_{call_id}_c{:02}_{}_retry{}",
                idx + 1,
                component_name,
                attempt,
                call_id = call_id_for_prefix.as_str()
            )
        };

        let reasoning_effort = ai.model_reasoning_effort().to_string();
        spawn_gen3d_ai_text_thread(
            shared.clone(),
            progress.clone(),
            job.cancel_flag.clone(),
            job.ai_request_timeout(),
            job.session.clone(),
            Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ComponentDraftV1),
            config.gen3d_require_structured_outputs,
            ai,
            reasoning_effort,
            system,
            user_text,
            image_paths,
            step_dir,
            sanitize_prefix(&prefix),
        );

        job.component_in_flight.push(super::Gen3dInFlightComponent {
            idx,
            attempt,
            sent_images,
            shared_result: shared,
            _progress: progress,
        });
    }

    let done = batch.completed_indices.len();
    let in_flight = job.component_in_flight.len();
    let pending = job.component_queue.len();
    workshop.status = format!(
        "Generating components (batch)… ({done}/{total})\nIn flight: {in_flight} | pending: {pending}\nParallel: {parallel}"
    );
    if let Some(progress) = job.shared_progress.as_ref() {
        set_progress(
            progress,
            format!("Generating components (batch)… ({done}/{total})"),
        );
    }

    if done == total && in_flight == 0 && pending == 0 {
        let succeeded = total.saturating_sub(batch.failed.len());
        let failed_json: Vec<serde_json::Value> = batch
            .failed
            .iter()
            .map(|f| {
                serde_json::json!({
                    "index": f.index,
                    "name": f.name,
                    "error": super::truncate_for_ui(&f.error, 600),
                })
            })
            .collect();

        let skipped_due_to_reuse_groups_json: Vec<serde_json::Value> = batch
            .skipped_due_to_reuse_groups
            .iter()
            .copied()
            .filter(|idx| *idx < job.planned_components.len())
            .map(|idx| {
                serde_json::json!({
                    "index": idx,
                    "name": job.planned_components[idx].name.as_str(),
                })
            })
            .collect();
        let skipped_due_to_preserve_existing_components_json: Vec<serde_json::Value> = batch
            .skipped_due_to_preserve_existing_components
            .iter()
            .copied()
            .filter(|idx| *idx < job.planned_components.len())
            .map(|idx| {
                serde_json::json!({
                    "index": idx,
                    "name": job.planned_components[idx].name.as_str(),
                })
            })
            .collect();

        let call = job.agent.pending_tool_call.take().unwrap();
        job.agent.pending_llm_tool = None;
        job.agent.pending_component_batch = None;
        job.component_queue.clear();
        job.component_in_flight.clear();
        job.component_last_errors.clear();

        return Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "requested": total,
                "succeeded": succeeded,
                "failed": failed_json,
                "reuse_groups_total": job.reuse_groups.len(),
                "optimized_by_reuse_groups": batch.optimized_by_reuse_groups,
                "skipped_due_to_reuse_groups": skipped_due_to_reuse_groups_json,
                "skipped_due_to_preserve_existing_components": skipped_due_to_preserve_existing_components_json,
                "skipped_due_to_regen_budget": batch.skipped_due_to_regen_budget,
            }),
        ));
    }

    job.agent.pending_component_batch = Some(batch);
    None
}
