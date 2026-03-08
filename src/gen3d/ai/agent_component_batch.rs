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
};
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
        let call = job.agent.pending_tool_call.take().unwrap();
        job.agent.pending_llm_tool = None;
        job.agent.pending_component_batch = None;
        job.component_queue.clear();
        job.component_in_flight.clear();
        return Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "requested": 0,
                "succeeded": 0,
                "failed": [],
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
                if let Some(tokens) = resp.total_tokens {
                    job.add_tokens(tokens);
                }
                if let Some(flag) = resp.session.responses_supported {
                    job.session.responses_supported = Some(flag);
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

                let component_def = match job
                    .planned_components
                    .get(idx)
                    .ok_or_else(|| {
                        format!("Internal error: missing planned component for idx={idx}")
                    })
                    .and_then(|planned| {
                        super::convert::ai_to_component_def(planned, ai, job.pass_dir.as_deref())
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

                if let Some(comp) = job.planned_components.get_mut(idx) {
                    comp.actual_size = Some(component_def.size);
                    comp.anchors = component_def.anchors.clone();
                }

                // Replace component def in-place, preserving existing object refs.
                let target_id = component_def.object_id;
                if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == target_id) {
                    let preserved_refs: Vec<crate::object::registry::ObjectPartDef> = existing
                        .parts
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::object::registry::ObjectPartKind::ObjectRef { .. }
                            )
                        })
                        .cloned()
                        .collect();
                    let mut merged = component_def;
                    merged.parts.extend(preserved_refs);
                    *existing = merged;
                } else {
                    draft.defs.push(component_def);
                }

                if let Some(root_idx) = job
                    .planned_components
                    .iter()
                    .position(|c| c.attach_to.is_none())
                {
                    if let Err(err) = super::convert::resolve_planned_component_transforms(
                        &mut job.planned_components,
                        root_idx,
                    ) {
                        warn!("Gen3D batch: failed to resolve transforms: {err}");
                    }
                }
                super::convert::update_root_def_from_planned_components(
                    &job.planned_components,
                    &job.plan_collider,
                    draft,
                );
                write_gen3d_assembly_snapshot(job.pass_dir.as_deref(), &job.planned_components);
                job.assembly_rev = job.assembly_rev.saturating_add(1);

                batch.completed_indices.insert(idx);
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
        let Some(pass_dir) = job.pass_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
            return None;
        };

        let attempt = *job.component_attempts.get(idx).unwrap_or(&0);
        let sent_images = !job.user_images.is_empty();
        let image_paths = if sent_images {
            job.user_images.clone()
        } else {
            Vec::new()
        };

        let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: "Starting…".into(),
        }));

        let system = super::prompts::build_gen3d_component_system_instructions();
        let user_text = super::prompts::build_gen3d_component_user_text(
            &job.user_prompt_raw,
            !job.user_images.is_empty(),
            speed,
            &job.assembly_notes,
            &job.planned_components,
            idx,
        );

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

        let reasoning_effort = super::openai::cap_reasoning_effort(
            ai.model_reasoning_effort(),
            &config.gen3d_reasoning_effort_component,
        );
        spawn_gen3d_ai_text_thread(
            shared.clone(),
            progress.clone(),
            job.cancel_flag.clone(),
            job.session.clone(),
            Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ComponentDraftV1),
            config.gen3d_require_structured_outputs,
            ai,
            reasoning_effort,
            system,
            user_text,
            image_paths,
            pass_dir,
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

        let mut auto_copy = super::reuse_groups::apply_auto_copy(
            &mut job.planned_components,
            draft,
            &job.reuse_groups,
        );
        if auto_copy.component_copies_applied > 0 {
            if let Some(root_idx) = job
                .planned_components
                .iter()
                .position(|c| c.attach_to.is_none())
            {
                if let Err(err) = super::convert::resolve_planned_component_transforms(
                    &mut job.planned_components,
                    root_idx,
                ) {
                    auto_copy.errors.push(format!(
                        "auto_copy: failed to resolve transforms after copy: {err}"
                    ));
                }
            }
            super::convert::update_root_def_from_planned_components(
                &job.planned_components,
                &job.plan_collider,
                draft,
            );
            write_gen3d_assembly_snapshot(job.pass_dir.as_deref(), &job.planned_components);
            job.assembly_rev = job.assembly_rev.saturating_add(1);
        }

        let mut fallback_component_indices: Vec<usize> = auto_copy
            .fallback_component_indices
            .iter()
            .copied()
            .filter(|idx| *idx < job.planned_components.len())
            .filter(|idx| {
                job.planned_components
                    .get(*idx)
                    .is_some_and(|c| c.actual_size.is_none())
            })
            .collect();
        fallback_component_indices.sort_unstable();
        fallback_component_indices.dedup();

        if !fallback_component_indices.is_empty() {
            let mut pending_set: std::collections::HashSet<usize> = job
                .agent
                .pending_regen_component_indices
                .iter()
                .copied()
                .collect();
            for idx in fallback_component_indices.iter().copied() {
                pending_set.insert(idx);
            }
            let mut pending: Vec<usize> = pending_set.into_iter().collect();
            pending.sort_unstable();
            job.agent.pending_regen_component_indices = pending;
            job.agent
                .pending_regen_component_indices_skipped_due_to_budget
                .clear();

            append_gen3d_run_log(
                job.pass_dir.as_deref(),
                format!(
                    "auto_copy_preflight_fallback components={:?}",
                    fallback_component_indices
                ),
            );
        }

        let fallback_components_json: Vec<serde_json::Value> = fallback_component_indices
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

        if let Some(pass_dir) = job.pass_dir.as_deref() {
            if auto_copy.component_copies_applied > 0
                || !auto_copy.errors.is_empty()
                || !auto_copy.preflight_mismatches.is_empty()
                || !fallback_components_json.is_empty()
            {
                let mut outcomes_json: Vec<serde_json::Value> = Vec::new();
                const MAX_OUTCOMES: usize = 16;
                for outcome in auto_copy.outcomes.iter().take(MAX_OUTCOMES) {
                    let mode = match outcome.mode_used {
                        super::copy_component::Gen3dCopyMode::Detached => "detached",
                        super::copy_component::Gen3dCopyMode::Linked => "linked",
                    };
                    outcomes_json.push(serde_json::json!({
                        "source": outcome.source_component_name.as_str(),
                        "target": outcome.target_component_name.as_str(),
                        "mode": mode,
                    }));
                }
                let outcomes_omitted = auto_copy.outcomes.len().saturating_sub(MAX_OUTCOMES);
                write_gen3d_json_artifact(
                    Some(pass_dir),
                    "auto_copy.json",
                    &serde_json::json!({
                        "version": 1,
                        "enabled": auto_copy.enabled,
                        "component_copies_applied": auto_copy.component_copies_applied,
                        "subtree_copies_applied": auto_copy.subtree_copies_applied,
                        "targets_skipped_already_generated": auto_copy.targets_skipped_already_generated,
                        "subtrees_skipped_partially_generated": auto_copy.subtrees_skipped_partially_generated,
                        "preflight_mismatches": &auto_copy.preflight_mismatches,
                        "fallback_component_indices": &fallback_component_indices,
                        "fallback_components": &fallback_components_json,
                        "errors": &auto_copy.errors,
                        "outcomes": outcomes_json,
                        "outcomes_omitted": outcomes_omitted,
                    }),
                );
            }
        }

        let mut outcomes_json: Vec<serde_json::Value> = Vec::new();
        const MAX_OUTCOMES: usize = 16;
        for outcome in auto_copy.outcomes.iter().take(MAX_OUTCOMES) {
            let mode = match outcome.mode_used {
                super::copy_component::Gen3dCopyMode::Detached => "detached",
                super::copy_component::Gen3dCopyMode::Linked => "linked",
            };
            outcomes_json.push(serde_json::json!({
                "source": outcome.source_component_name.as_str(),
                "target": outcome.target_component_name.as_str(),
                "mode": mode,
            }));
        }
        let outcomes_omitted = auto_copy.outcomes.len().saturating_sub(MAX_OUTCOMES);

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

        let call = job.agent.pending_tool_call.take().unwrap();
        job.agent.pending_llm_tool = None;
        job.agent.pending_component_batch = None;
        job.component_queue.clear();
        job.component_in_flight.clear();

        return Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "requested": total,
                "succeeded": succeeded,
                "failed": failed_json,
                "optimized_by_reuse_groups": batch.optimized_by_reuse_groups,
                "skipped_due_to_reuse_groups": skipped_due_to_reuse_groups_json,
                "skipped_due_to_regen_budget": batch.skipped_due_to_regen_budget,
                "auto_copy": {
                    "enabled": auto_copy.enabled,
                    "component_copies_applied": auto_copy.component_copies_applied,
                    "subtree_copies_applied": auto_copy.subtree_copies_applied,
                    "targets_skipped_already_generated": auto_copy.targets_skipped_already_generated,
                    "subtrees_skipped_partially_generated": auto_copy.subtrees_skipped_partially_generated,
                    "preflight_mismatches": auto_copy.preflight_mismatches,
                    "fallback_component_indices": fallback_component_indices,
                    "fallback_components": fallback_components_json,
                    "errors": auto_copy.errors,
                    "outcomes": outcomes_json,
                    "outcomes_omitted": outcomes_omitted,
                },
            }),
        ));
    }

    job.agent.pending_component_batch = Some(batch);
    None
}
