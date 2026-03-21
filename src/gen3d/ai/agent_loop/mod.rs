use bevy::log::debug;
use bevy::prelude::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
#[cfg(test)]
use crate::gen3d::agent::Gen3dToolResultJsonV1;
use crate::gen3d::agent::{append_agent_trace_event_v1, AgentTraceEventV1, Gen3dToolRegistryV1};

use super::artifacts::{
    append_gen3d_run_log, write_gen3d_json_artifact, write_gen3d_text_artifact,
};
use super::status_steps;
use super::{
    fail_job, set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiPhase, Gen3dAiProgress,
    Gen3dAiTextResponse,
};
#[cfg(test)]
use crate::gen3d::agent::tools::TOOL_ID_QA;
use crate::threaded_result::{new_shared_result, SharedResult};
use crate::types::{AnimationChannelsActive, AttackClock, LocomotionClock};

#[cfg(test)]
use super::agent_parsing::{parse_delta_transform, resolve_component_index_by_name_hint};
use super::agent_prompt::{build_agent_system_instructions, build_agent_user_text, draft_summary};
use super::agent_render_capture::poll_agent_render_capture;
#[cfg(test)]
use super::agent_review_images::{
    motion_sheets_needed_from_smoke_results, parse_review_preview_blob_ids_from_args,
    review_capture_dimensions_for_max_dim, select_review_preview_blob_ids,
};
use super::agent_step::poll_agent_descriptor_meta;
use super::agent_step::{execute_agent_actions, poll_agent_pass_snapshot_capture, poll_agent_step};
use super::agent_tool_poll::poll_agent_tool;
#[cfg(test)]
use super::agent_utils::compute_agent_state_hash;
use crate::threaded_result::take_shared_result;

use super::super::state::{Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dWorkshop};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;

pub(super) fn poll_gen3d_agent(
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
        ),
        With<Gen3dPreviewModelRoot>,
    >,
    render_allowed: bool,
) {
    super::orchestration::poll_gen3d_descriptor_meta_in_flight(job);

    match job.phase {
        Gen3dAiPhase::AgentWaitingUserImageSummary => {
            poll_agent_user_image_summary(config, workshop, job);
        }
        Gen3dAiPhase::AgentWaitingPromptIntent => {
            poll_agent_prompt_intent(config, workshop, job);
        }
        Gen3dAiPhase::AgentWaitingStep => {
            poll_agent_step(config, commands, workshop, feedback_history, job, draft)
        }
        Gen3dAiPhase::AgentExecutingActions => execute_agent_actions(
            config,
            render_allowed,
            time,
            commands,
            images,
            workshop,
            feedback_history,
            job,
            draft,
            preview,
            preview_model,
        ),
        Gen3dAiPhase::AgentWaitingTool => poll_agent_tool(
            config,
            render_allowed,
            commands,
            images,
            workshop,
            feedback_history,
            job,
            draft,
            preview,
        ),
        Gen3dAiPhase::AgentCapturingRender => {
            poll_agent_render_capture(
                config,
                time,
                commands,
                images,
                workshop,
                job,
                draft,
                preview_model,
            );
        }
        Gen3dAiPhase::AgentCapturingPassSnapshot => poll_agent_pass_snapshot_capture(
            config,
            commands,
            images,
            workshop,
            feedback_history,
            job,
        ),
        Gen3dAiPhase::AgentWaitingDescriptorMeta => poll_agent_descriptor_meta(
            config,
            render_allowed,
            commands,
            images,
            workshop,
            job,
            draft,
        ),
        _ => fail_job(
            workshop,
            job,
            "Internal error: agent entered an unexpected phase.",
        ),
    }
}

pub(super) fn spawn_agent_user_image_summary_request(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    pass_dir: PathBuf,
) -> Result<(), String> {
    if job.user_images.is_empty() {
        job.phase = Gen3dAiPhase::AgentWaitingStep;
        return spawn_agent_step_request(config, workshop, job, pass_dir);
    }
    if job.user_image_object_summary.is_some() {
        job.phase = Gen3dAiPhase::AgentWaitingStep;
        return spawn_agent_step_request(config, workshop, job, pass_dir);
    }

    let Some(ai) = job.ai.clone() else {
        return Err("Internal error: missing AI config.".into());
    };

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Summarizing images…".into(),
    }));
    job.shared_progress = Some(progress.clone());

    set_progress(&progress, "Analyzing reference images…");
    status_steps::log_ai_request_started(
        workshop,
        "Analyze reference images",
        &format!(
            "You provided {} image(s); extracting a short object summary for downstream steps.",
            job.user_images.len()
        ),
    );

    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::Info {
            message: format!(
                "Gen3D: summarizing {} reference image(s) into text",
                job.user_images.len()
            ),
        },
    );
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "user_image_summary_request attempt={} pass={} images={}",
            job.attempt,
            job.pass,
            job.user_images.len()
        ),
    );

    let system = super::prompts::build_gen3d_user_image_object_summary_system_instructions();
    let user_text = super::prompts::build_gen3d_user_image_object_summary_user_text(
        &job.user_prompt_raw,
        job.user_images.len(),
    );
    let reasoning_effort = super::openai::cap_reasoning_effort(ai.model_reasoning_effort(), "low");
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        None,
        config.gen3d_require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        job.user_images.clone(),
        pass_dir,
        "user_image_summary".into(),
    );

    Ok(())
}

pub(super) fn spawn_agent_prompt_intent_request(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    pass_dir: PathBuf,
) -> Result<(), String> {
    if job.prompt_intent.is_some() {
        job.phase = Gen3dAiPhase::AgentWaitingStep;
        return spawn_agent_step_request(config, workshop, job, pass_dir);
    }

    let Some(ai) = job.ai.clone() else {
        return Err("Internal error: missing AI config.".into());
    };

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Analyzing prompt…".into(),
    }));
    job.shared_progress = Some(progress.clone());

    set_progress(&progress, "Determining prompt intent…");
    status_steps::log_ai_request_started(
        workshop,
        "Analyze prompt intent",
        "Determining whether the prompt requires gameplay attack capability.",
    );

    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::Info {
            message: "Gen3D: analyzing prompt intent".into(),
        },
    );
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "prompt_intent_request attempt={} pass={}",
            job.attempt, job.pass
        ),
    );

    let system = super::prompts::build_gen3d_prompt_intent_system_instructions();
    let user_text = super::prompts::build_gen3d_prompt_intent_user_text(
        &job.user_prompt_raw,
        job.user_image_object_summary
            .as_ref()
            .map(|s| s.text.as_str()),
    );
    let reasoning_effort = super::openai::cap_reasoning_effort(ai.model_reasoning_effort(), "low");
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(super::structured_outputs::Gen3dAiJsonSchemaKind::PromptIntentV1),
        config.gen3d_require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        Vec::new(),
        pass_dir,
        "prompt_intent".into(),
    );

    Ok(())
}

fn truncate_text_to_max_words_preserving_whitespace(
    text: &str,
    max_words: usize,
) -> (String, bool, usize) {
    let mut out = String::new();
    let mut in_word = false;
    let mut words = 0usize;

    for ch in text.chars() {
        let is_ws = ch.is_whitespace();
        if !is_ws && !in_word {
            if words >= max_words {
                let out = out.trim().to_string();
                let words_out = crate::gen3d::gen3d_count_whitespace_separated_words(&out);
                return (out, true, words_out);
            }
            words += 1;
            in_word = true;
        } else if is_ws {
            in_word = false;
        }
        out.push(ch);
    }

    let out = out.trim().to_string();
    let words_out = crate::gen3d::gen3d_count_whitespace_separated_words(&out);
    (out, false, words_out)
}

fn poll_agent_user_image_summary(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
) {
    if job.user_images.is_empty() {
        job.phase = Gen3dAiPhase::AgentWaitingStep;
        if let Some(pass_dir) = job.pass_dir.clone() {
            if let Err(err) = spawn_agent_step_request(config, workshop, job, pass_dir) {
                fail_job(workshop, job, err);
            }
        } else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        }
        return;
    }

    if job.user_image_object_summary.is_some() {
        job.phase = Gen3dAiPhase::AgentWaitingStep;
        if let Some(pass_dir) = job.pass_dir.clone() {
            if let Err(err) = spawn_agent_step_request(config, workshop, job, pass_dir) {
                fail_job(workshop, job, err);
            }
        } else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        }
        return;
    }

    if job.shared_result.is_none() {
        let Some(pass_dir) = job.pass_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
            return;
        };
        if let Err(err) = spawn_agent_user_image_summary_request(config, workshop, job, pass_dir) {
            fail_job(workshop, job, err);
        }
        return;
    }

    let Some(shared) = job.shared_result.as_ref() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing Gen3D shared_result.",
        );
        return;
    };
    let Some(result) = take_shared_result(shared) else {
        return;
    };
    job.shared_result = None;
    job.shared_progress = None;

    match result {
        Ok(resp) => {
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                job.add_tokens(tokens);
            }

            let normalized = resp.text.replace("\r\n", "\n").replace('\r', "\n");
            let (text, truncated, word_count) = truncate_text_to_max_words_preserving_whitespace(
                normalized.trim(),
                crate::gen3d::GEN3D_IMAGE_OBJECT_SUMMARY_MAX_WORDS,
            );
            if text.trim().is_empty() {
                fail_job(
                    workshop,
                    job,
                    "Reference image summary was empty. Add a text prompt or try again.",
                );
                return;
            }

            job.user_image_object_summary = Some(super::job::Gen3dUserImageObjectSummary {
                text: text.clone(),
                truncated,
                word_count,
            });
            status_steps::log_ai_request_finished(
                workshop,
                &format!(
                    "OK (words: {word_count}{})",
                    if truncated { ", truncated" } else { "" }
                ),
            );

            let Some(run_dir) = job.run_dir.clone() else {
                fail_job(workshop, job, "Internal error: missing Gen3D run dir.");
                return;
            };
            let attempt_dir = run_dir.join(format!("attempt_{}", job.attempt));
            write_gen3d_text_artifact(Some(&attempt_dir), "inputs/image_object_summary.txt", &text);
            write_gen3d_json_artifact(
                Some(&attempt_dir),
                "inputs/image_object_summary.json",
                &serde_json::json!({
                    "version": 1,
                    "images_count": job.user_images.len(),
                    "word_count": word_count,
                    "truncated": truncated,
                }),
            );

            workshop.status = "Reference images summarized.\nAnalyzing prompt…".to_string();
            job.phase = Gen3dAiPhase::AgentWaitingPromptIntent;

            let Some(pass_dir) = job.pass_dir.clone() else {
                fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
                return;
            };
            if let Err(err) = spawn_agent_prompt_intent_request(config, workshop, job, pass_dir) {
                fail_job(workshop, job, err);
            }
        }
        Err(err) => {
            fail_job(
                workshop,
                job,
                format!(
                    "Reference image pre-processing failed: {err}\nTip: try again or use a text prompt without images."
                ),
            );
        }
    }
}

fn poll_agent_prompt_intent(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
) {
    if job.prompt_intent.is_some() {
        job.phase = Gen3dAiPhase::AgentWaitingStep;
        if let Some(pass_dir) = job.pass_dir.clone() {
            if let Err(err) = spawn_agent_step_request(config, workshop, job, pass_dir) {
                fail_job(workshop, job, err);
            }
        } else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        }
        return;
    }

    if job.shared_result.is_none() {
        let Some(pass_dir) = job.pass_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
            return;
        };
        if let Err(err) = spawn_agent_prompt_intent_request(config, workshop, job, pass_dir) {
            fail_job(workshop, job, err);
        }
        return;
    }

    let Some(shared) = job.shared_result.as_ref() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing Gen3D shared_result.",
        );
        return;
    };
    let Some(result) = take_shared_result(shared) else {
        return;
    };
    job.shared_result = None;
    job.shared_progress = None;

    match result {
        Ok(resp) => {
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                job.add_tokens(tokens);
            }

            let parsed = match super::parse::parse_ai_prompt_intent_from_text(&resp.text) {
                Ok(v) => v,
                Err(err) => {
                    fail_job(
                        workshop,
                        job,
                        format!("Prompt intent classification failed: {err}"),
                    );
                    return;
                }
            };
            let requires_attack = parsed.requires_attack;
            job.prompt_intent = Some(parsed.clone());

            status_steps::log_ai_request_finished(
                workshop,
                &format!("OK (requires_attack={requires_attack})"),
            );

            if let Some(run_dir) = job.run_dir.clone() {
                let attempt_dir = run_dir.join(format!("attempt_{}", job.attempt));
                write_gen3d_json_artifact(
                    Some(&attempt_dir),
                    "inputs/prompt_intent.json",
                    &serde_json::to_value(&parsed).unwrap_or_else(
                        |_| serde_json::json!({"version": 1, "requires_attack": requires_attack}),
                    ),
                );
            }

            workshop.status = "Prompt analyzed.\nPlanning…".to_string();
            job.phase = Gen3dAiPhase::AgentWaitingStep;

            let Some(pass_dir) = job.pass_dir.clone() else {
                fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
                return;
            };
            if let Err(err) = spawn_agent_step_request(config, workshop, job, pass_dir) {
                fail_job(workshop, job, err);
            }
        }
        Err(err) => {
            fail_job(
                workshop,
                job,
                format!("Prompt intent classification failed: {err}"),
            );
        }
    }
}

pub(super) fn spawn_agent_step_request(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    pass_dir: PathBuf,
) -> Result<(), String> {
    let Some(ai) = job.ai.clone() else {
        return Err("Internal error: missing AI config.".into());
    };

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Starting agent…".into(),
    }));
    job.shared_progress = Some(progress.clone());
    job.metrics.note_agent_step_request_started();

    set_progress(&progress, "Thinking…");

    let registry = Gen3dToolRegistryV1::default();
    let system = build_agent_system_instructions();
    let state_summary = draft_summary(config, job);
    {
        let attempt = job.attempt;
        let pass = job.pass;
        let assembly_rev = job.assembly_rev;
        let workspace_id = job.active_workspace_id().trim().to_string();
        let key = format!("ws.{workspace_id}.state_summary");
        if let Ok(store) = job.ensure_info_store() {
            if let Err(err) = store.kv_put(
                attempt,
                pass,
                assembly_rev,
                workspace_id.as_str(),
                "gen3d",
                key.as_str(),
                state_summary.clone(),
                "state summary".into(),
                None,
            ) {
                debug!("Gen3D: failed to write state_summary to Info Store: {err}");
            }
        }
    }
    let user_text = build_agent_user_text(
        config,
        job,
        workshop,
        state_summary,
        &job.agent.step_tool_results,
        &registry,
    );

    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::Info {
            message: "Gen3D agent: requesting next step".into(),
        },
    );
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "agent_step_request attempt={} pass={} plan_hash={} assembly_rev={} active_ws={} components_generated={}/{}",
            job.attempt,
            job.pass,
            job.plan_hash(),
            job.assembly_rev(),
            job.active_workspace_id(),
            job.planned_components.iter().filter(|c| c.actual_size.is_some()).count(),
            job.planned_components.len(),
        ),
    );
    debug!(
        "Gen3D agent: requesting step (attempt={}, pass={}, plan_hash={}, assembly_rev={}, active_ws={}, components_generated={}/{})",
        job.attempt,
        job.pass,
        job.plan_hash(),
        job.assembly_rev(),
        job.active_workspace_id(),
        job.planned_components.iter().filter(|c| c.actual_size.is_some()).count(),
        job.planned_components.len(),
    );

    let reasoning_effort = super::openai::cap_reasoning_effort(
        ai.model_reasoning_effort(),
        &config.gen3d_reasoning_effort_agent_step,
    );
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(super::structured_outputs::Gen3dAiJsonSchemaKind::AgentStepV1),
        config.gen3d_require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        Vec::new(),
        pass_dir,
        "agent_step".into(),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, OpenAiConfig};
    use crate::gen3d::ai::ai_service::Gen3dAiServiceConfig;
    use crate::gen3d::state::{Gen3dDraft, Gen3dPreview, Gen3dSpeedMode, Gen3dWorkshop};
    use crate::gen3d::tool_feedback::Gen3dToolFeedbackHistory;
    use uuid::Uuid;

    #[test]
    fn gen3d_tool_transform_parsing_preserves_negative_and_zero_scale() {
        let value = serde_json::json!({
            "pos": [1.0, 2.0, 3.0],
            "scale": [-1.0, 0.0, 2.0],
        });
        let t = parse_delta_transform(Some(&value));
        assert_eq!(t.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.scale, Vec3::new(-1.0, 0.0, 2.0));
    }

    #[test]
    fn select_review_preview_blobs_prefers_five_static_views() {
        let run_dir = std::env::temp_dir().join(format!(
            "gravimera_gen3d_review_blob_select_{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&run_dir).expect("create run dir");
        let mut store =
            super::super::info_store::Gen3dInfoStore::open_or_create(&run_dir).expect("open store");

        let front = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:front".into()],
                "render_front.png".into(),
            )
            .expect("register front")
            .blob_id;
        let left_back = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:left_back".into()],
                "render_left_back.png".into(),
            )
            .expect("register left_back")
            .blob_id;
        let right_back = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:right_back".into()],
                "render_right_back.png".into(),
            )
            .expect("register right_back")
            .blob_id;
        let top = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:top".into()],
                "render_top.png".into(),
            )
            .expect("register top")
            .blob_id;
        let bottom = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:bottom".into()],
                "render_bottom.png".into(),
            )
            .expect("register bottom")
            .blob_id;
        let move_sheet = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:motion_sheet".into(), "motion:move".into()],
                "move_sheet.png".into(),
            )
            .expect("register move_sheet")
            .blob_id;
        let attack_sheet = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:motion_sheet".into(), "motion:attack".into()],
                "attack_sheet.png".into(),
            )
            .expect("register attack_sheet")
            .blob_id;

        let preview_blob_ids = vec![
            front.clone(),
            left_back.clone(),
            right_back.clone(),
            top.clone(),
            bottom.clone(),
            move_sheet,
            attack_sheet,
        ];
        let selected = select_review_preview_blob_ids(&store, &preview_blob_ids, false, false);
        assert_eq!(selected, vec![front, left_back, right_back, top, bottom,]);
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn select_review_preview_blobs_includes_motion_sheets_when_requested() {
        let run_dir = std::env::temp_dir().join(format!(
            "gravimera_gen3d_review_blob_select_{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&run_dir).expect("create run dir");
        let mut store =
            super::super::info_store::Gen3dInfoStore::open_or_create(&run_dir).expect("open store");

        let front = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:front".into()],
                "render_front.png".into(),
            )
            .expect("register front")
            .blob_id;
        let left_back = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:left_back".into()],
                "render_left_back.png".into(),
            )
            .expect("register left_back")
            .blob_id;
        let right_back = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:right_back".into()],
                "render_right_back.png".into(),
            )
            .expect("register right_back")
            .blob_id;
        let top = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:top".into()],
                "render_top.png".into(),
            )
            .expect("register top")
            .blob_id;
        let bottom = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:render_preview".into(), "view:bottom".into()],
                "render_bottom.png".into(),
            )
            .expect("register bottom")
            .blob_id;
        let move_sheet = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:motion_sheet".into(), "motion:move".into()],
                "move_sheet.png".into(),
            )
            .expect("register move_sheet")
            .blob_id;
        let attack_sheet = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:motion_sheet".into(), "motion:attack".into()],
                "attack_sheet.png".into(),
            )
            .expect("register attack_sheet")
            .blob_id;

        let preview_blob_ids = vec![
            front.clone(),
            left_back.clone(),
            right_back.clone(),
            top.clone(),
            bottom.clone(),
            move_sheet.clone(),
            attack_sheet.clone(),
        ];
        let selected = select_review_preview_blob_ids(&store, &preview_blob_ids, true, true);
        assert_eq!(
            selected,
            vec![
                front,
                left_back,
                right_back,
                top,
                bottom,
                move_sheet,
                attack_sheet,
            ]
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn review_capture_dimensions_for_max_dim_matches_expected_scale() {
        assert_eq!(review_capture_dimensions_for_max_dim(960), (960, 540));
        assert_eq!(review_capture_dimensions_for_max_dim(768), (768, 432));
        assert_eq!(review_capture_dimensions_for_max_dim(1920), (1920, 1080));
    }

    #[test]
    fn select_review_preview_blobs_falls_back_when_only_motion_present() {
        let run_dir = std::env::temp_dir().join(format!(
            "gravimera_gen3d_review_blob_select_{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&run_dir).expect("create run dir");
        let mut store =
            super::super::info_store::Gen3dInfoStore::open_or_create(&run_dir).expect("open store");

        let move_sheet = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:motion_sheet".into(), "motion:move".into()],
                "move_sheet.png".into(),
            )
            .expect("register move_sheet")
            .blob_id;
        let attack_sheet = store
            .register_blob_file(
                0,
                0,
                0,
                "image/png",
                1,
                vec!["kind:motion_sheet".into(), "motion:attack".into()],
                "attack_sheet.png".into(),
            )
            .expect("register attack_sheet")
            .blob_id;

        let preview_blob_ids = vec![move_sheet.clone(), attack_sheet.clone()];
        let selected = select_review_preview_blob_ids(&store, &preview_blob_ids, false, false);
        assert_eq!(selected, preview_blob_ids);
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn motion_sheets_needed_from_smoke_results_is_channel_specific() {
        let smoke_results = serde_json::json!({
            "motion_validation": {
                "ok": false,
                "issues": [
                    { "severity": "error", "kind": "chain_axis_mismatch", "channel": "move" },
                    { "severity": "error", "kind": "attack_self_intersection", "channel": "attack_primary" },
                    { "severity": "warn", "kind": "some_warn", "channel": "move" },
                ]
            }
        });
        assert_eq!(
            motion_sheets_needed_from_smoke_results(&smoke_results),
            (true, true)
        );
    }

    #[test]
    fn motion_sheets_needed_from_smoke_results_falls_back_when_issues_missing() {
        let smoke_results = serde_json::json!({
            "motion_validation": {
                "ok": false
            }
        });
        assert_eq!(
            motion_sheets_needed_from_smoke_results(&smoke_results),
            (true, false)
        );
    }

    #[test]
    fn gen3d_no_progress_state_hash_ignores_assembly_rev_churn() {
        let mut job = Gen3dAiJob::default();
        job.plan_hash = "sha256:deadbeef".to_string();

        let draft = Gen3dDraft::default();

        job.assembly_rev = 1;
        let h1 = compute_agent_state_hash(&job, &draft);
        job.assembly_rev = 999;
        let h2 = compute_agent_state_hash(&job, &draft);

        assert_eq!(h1, h2, "state hash should ignore assembly_rev");
    }

    #[test]
    fn gen3d_agent_state_summary_exposes_budget_remaining() {
        let mut config = AppConfig::default();
        config.gen3d_max_seconds = 200;
        config.gen3d_max_tokens = 2000;
        config.gen3d_no_progress_tries_max = 12;
        config.gen3d_inspection_steps_max = 20;
        config.gen3d_max_regen_total = 16;
        config.gen3d_max_regen_per_component = 2;

        let mut job = Gen3dAiJob::default();
        job.running = false;
        job.last_run_elapsed = Some(std::time::Duration::from_secs_f64(100.25));
        job.current_run_tokens = 1234;
        job.regen_total = 15;
        job.regen_per_component = vec![2, 1];
        job.review_delta_rounds_used = 1;
        job.agent.no_progress_tries = 5;
        job.agent.no_progress_inspection_steps = 8;
        job.planned_components = vec![
            super::super::Gen3dPlannedComponent {
                display_name: "A".into(),
                name: "a".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
            super::super::Gen3dPlannedComponent {
                display_name: "B".into(),
                name: "b".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
        ];

        let summary = draft_summary(&config, &job);
        let budgets = summary
            .get("budgets")
            .and_then(|v| v.as_object())
            .expect("expected budgets object in state summary");

        let regen_remaining = budgets
            .get("regen")
            .and_then(|v| v.get("remaining_total"))
            .and_then(|v| v.as_u64())
            .expect("expected regen remaining_total");
        assert_eq!(regen_remaining, 1);

        let review_delta_remaining = budgets
            .get("review_delta")
            .and_then(|v| v.get("rounds_remaining"))
            .and_then(|v| v.as_u64())
            .expect("expected review_delta rounds_remaining");
        assert_eq!(review_delta_remaining, 1);

        let no_progress_tries_remaining = budgets
            .get("no_progress")
            .and_then(|v| v.get("tries_remaining"))
            .and_then(|v| v.as_u64())
            .expect("expected no_progress tries_remaining");
        assert_eq!(no_progress_tries_remaining, 7);

        let inspection_steps_remaining = budgets
            .get("no_progress")
            .and_then(|v| v.get("inspection_steps_remaining"))
            .and_then(|v| v.as_u64())
            .expect("expected no_progress inspection_steps_remaining");
        assert_eq!(inspection_steps_remaining, 12);

        let token_remaining = budgets
            .get("tokens")
            .and_then(|v| v.get("remaining_run_tokens"))
            .and_then(|v| v.as_u64())
            .expect("expected tokens remaining_run_tokens");
        assert_eq!(token_remaining, 766);

        let elapsed = budgets
            .get("time")
            .and_then(|v| v.get("elapsed_seconds"))
            .and_then(|v| v.as_f64())
            .expect("expected time elapsed_seconds");
        assert!(
            (elapsed - 100.3).abs() < 1e-6,
            "unexpected elapsed={elapsed}"
        );

        let remaining = budgets
            .get("time")
            .and_then(|v| v.get("remaining_seconds"))
            .and_then(|v| v.as_f64())
            .expect("expected time remaining_seconds");
        assert!(
            (remaining - 99.8).abs() < 1e-6,
            "unexpected remaining={remaining}"
        );

        let components = summary
            .get("components")
            .and_then(|v| v.as_array())
            .expect("expected components array");
        assert_eq!(
            components[0]
                .get("regen_remaining")
                .and_then(|v| v.as_u64()),
            Some(0)
        );
        assert_eq!(
            components[0]
                .get("regen_budget_blocked")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            components[1]
                .get("regen_remaining")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            components[1]
                .get("regen_budget_blocked")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn gen3d_agent_state_summary_includes_seed_kind_for_seeded_sessions() {
        let config = AppConfig::default();
        let mut job = Gen3dAiJob::default();

        let summary = draft_summary(&config, &job);
        assert!(summary
            .get("seed")
            .expect("expected seed field in state summary")
            .is_null());

        let prefab_uuid = Uuid::from_u128(123);
        let prefab_id_str = prefab_uuid.to_string();
        job.set_edit_base_prefab_id(Some(prefab_uuid.as_u128()));
        job.set_save_overwrite_prefab_id(Some(prefab_uuid.as_u128()));
        let summary = draft_summary(&config, &job);
        assert_eq!(
            summary
                .get("seed")
                .and_then(|v| v.get("kind"))
                .and_then(|v| v.as_str()),
            Some("edit_overwrite")
        );
        assert_eq!(
            summary
                .get("seed")
                .and_then(|v| v.get("prefab_id"))
                .and_then(|v| v.as_str()),
            Some(prefab_id_str.as_str())
        );

        job.set_save_overwrite_prefab_id(None);
        let summary = draft_summary(&config, &job);
        assert_eq!(
            summary
                .get("seed")
                .and_then(|v| v.get("kind"))
                .and_then(|v| v.as_str()),
            Some("fork")
        );
    }

    #[test]
    fn gen3d_motion_authoring_apply_to_current_ignores_run_id_and_attempt_for_restart_safety() {
        let config = AppConfig::default();

        let mut job = Gen3dAiJob::default();
        job.plan_hash = "sha256:abc123".into();
        job.assembly_rev = 7;
        job.attempt = 0;
        job.run_id = Some(Uuid::from_u128(123));

        job.motion_authoring = Some(super::super::schema::AiMotionAuthoringJsonV1 {
            version: 1,
            applies_to: super::super::schema::AiReviewDeltaAppliesToJsonV1 {
                run_id: "some_prior_run".into(),
                attempt: 999,
                plan_hash: job.plan_hash.clone(),
                assembly_rev: job.assembly_rev,
            },
            decision: super::super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips,
            reason: String::new(),
            replace_channels: Vec::new(),
            edges: Vec::new(),
            notes: None,
        });

        assert!(
            job.motion_authoring_for_current_draft().is_some(),
            "expected persisted motion_authoring to apply when plan_hash+assembly_rev match"
        );

        let summary = draft_summary(&config, &job);
        let applies = summary
            .get("motion_authoring")
            .and_then(|v| v.get("applies_to_current"))
            .and_then(|v| v.as_bool())
            .expect("expected motion_authoring.applies_to_current bool");
        assert!(applies);
    }

    #[test]
    fn gen3d_agent_step_prompt_compacts_large_tool_payloads() {
        let config = AppConfig::default();
        let mut job = Gen3dAiJob::default();
        job.user_prompt_raw = "test prompt".into();

        let workshop = Gen3dWorkshop::default();
        let state_summary = serde_json::json!({"run_id":"test"});
        let registry = crate::gen3d::agent::tools::Gen3dToolRegistryV1::default();

        let huge = "x".repeat(50_000);
        let tool_result = Gen3dToolResultJsonV1::err("call_0".into(), TOOL_ID_QA.into(), huge);

        let prompt = build_agent_user_text(
            &config,
            &job,
            &workshop,
            state_summary,
            &[tool_result],
            &registry,
        );

        // This prompt is written to disk each pass; keep it comfortably small so later passes don't balloon.
        assert!(
            prompt.len() < 24_000,
            "agent_step prompt unexpectedly large: {} bytes",
            prompt.len()
        );
        assert!(
            prompt.contains("…(truncated)"),
            "expected truncation marker in compact prompt summary"
        );
    }

    #[test]
    fn gen3d_component_name_hint_resolves_common_aliases() {
        let components = vec![
            super::super::Gen3dPlannedComponent {
                display_name: "Turret Base".into(),
                name: "turret_base".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: None,
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
            super::super::Gen3dPlannedComponent {
                display_name: "Cannon Barrel".into(),
                name: "cannon_barrel".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: None,
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
        ];

        assert_eq!(
            resolve_component_index_by_name_hint(&components, "roof_turret_base"),
            Some(0)
        );
        assert_eq!(
            resolve_component_index_by_name_hint(&components, "cannon_weapon"),
            Some(1)
        );
    }

    #[test]
    fn gen3d_review_preview_blob_args_ignore_tool_placeholders() {
        let args = serde_json::json!({
            "preview_blob_ids": [
                "$CALL_1.blob_ids[0]",
                "$CALL_2.static_blob_ids[0]",
            ]
        });
        let ids = parse_review_preview_blob_ids_from_args(&args);
        assert!(
            ids.is_empty(),
            "expected placeholder-only ids to be ignored"
        );
    }

    #[test]
    fn gen3d_mock_agent_builds_warcar_prompt_end_to_end() {
        let prompt = "A warcar with a cannon as weapon";

        let run_id = Uuid::new_v4();
        let base_dir = std::env::temp_dir().join(format!("gravimera_gen3d_test_{run_id}"));
        let run_dir = base_dir.join(run_id.to_string());
        let pass_dir = run_dir.join("attempt_0").join("pass_0");
        std::fs::create_dir_all(&pass_dir).expect("create temp gen3d pass dir");

        let openai = OpenAiConfig {
            base_url: "mock://gen3d".into(),
            model: "mock".into(),
            model_reasoning_effort: "none".into(),
            api_key: "mock".into(),
        };

        let mut config = AppConfig {
            openai: Some(openai.clone()),
            ..Default::default()
        };
        // Keep budgets unlimited for the test, but bound execution time in the loop below.
        config.gen3d_max_seconds = 0;
        config.gen3d_max_tokens = 0;

        let mut workshop = Gen3dWorkshop::default();
        workshop.prompt = prompt.to_string();
        workshop.speed_mode = Gen3dSpeedMode::Level3;

        let mut job = Gen3dAiJob::default();
        job.running = true;
        job.build_complete = false;
        job.mode = super::super::Gen3dAiMode::Agent;
        job.phase = super::super::Gen3dAiPhase::AgentWaitingStep;
        job.ai = Some(Gen3dAiServiceConfig::OpenAi(openai));
        job.run_id = Some(run_id);
        job.attempt = 0;
        job.pass = 0;
        job.plan_hash.clear();
        job.assembly_rev = 0;
        job.max_parallel_components = 1;
        job.user_prompt_raw = prompt.to_string();
        job.user_images.clear();
        job.run_dir = Some(run_dir);
        job.pass_dir = Some(pass_dir.clone());
        job.agent = super::super::Gen3dAgentState::default();

        let draft = Gen3dDraft::default();
        let preview = Gen3dPreview::default();
        let feedback_history = Gen3dToolFeedbackHistory::default();

        spawn_agent_step_request(&config, &mut workshop, &mut job, pass_dir)
            .expect("spawn mock agent step");

        let mut app = App::new();
        app.insert_resource(config);
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Assets::<Image>::default());
        app.insert_resource(workshop);
        app.insert_resource(feedback_history);
        app.insert_resource(job);
        app.insert_resource(draft);
        app.insert_resource(preview);

        app.add_systems(
            Update,
            |config: Res<AppConfig>,
             time: Res<Time>,
             mut commands: Commands,
             mut images: ResMut<Assets<Image>>,
             mut workshop: ResMut<Gen3dWorkshop>,
             mut feedback_history: ResMut<Gen3dToolFeedbackHistory>,
             mut job: ResMut<Gen3dAiJob>,
             mut draft: ResMut<Gen3dDraft>,
             mut preview: ResMut<Gen3dPreview>,
             mut preview_model: Query<
                (
                    &mut AnimationChannelsActive,
                    &mut LocomotionClock,
                    &mut AttackClock,
                ),
                With<Gen3dPreviewModelRoot>,
            >,
             review_cameras: Query<Entity, With<Gen3dReviewCaptureCamera>>| {
                poll_gen3d_agent(
                    &config,
                    &time,
                    &mut commands,
                    &mut images,
                    &review_cameras,
                    &mut workshop,
                    &mut feedback_history,
                    &mut job,
                    &mut draft,
                    &mut preview,
                    &mut preview_model,
                );
            },
        );

        let started = std::time::Instant::now();
        loop {
            app.update();
            let running = app.world().resource::<Gen3dAiJob>().is_running();
            if !running {
                break;
            }
            if started.elapsed() > std::time::Duration::from_secs(5) {
                panic!("Gen3D mock agent test timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let draft = app.world().resource::<Gen3dDraft>();
        assert!(
            draft.total_non_projectile_primitive_parts() > 0,
            "expected generated primitive parts"
        );
        let root = draft.root_def().expect("expected Gen3D root def");
        assert!(root.mobility.is_some(), "expected mobility on root def");
        assert!(root.attack.is_some(), "expected attack on root def");

        let job = app.world().resource::<Gen3dAiJob>();
        let meta = job
            .descriptor_meta_for_current_draft()
            .expect("expected descriptor-meta cached before run completion");
        assert!(
            !meta.short.trim().is_empty(),
            "expected descriptor-meta short to be non-empty"
        );
        assert!(
            !meta.tags.is_empty(),
            "expected descriptor-meta tags to be non-empty"
        );
    }
}
