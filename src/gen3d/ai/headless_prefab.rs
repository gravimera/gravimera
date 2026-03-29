#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::config::AppConfig;
use crate::gen3d::state::{Gen3dDraft, Gen3dSpeedMode};
use crate::object::registry::{ObjectDef, ObjectPartDef, ObjectPartKind};

use super::ai_service::{generate_text_via_ai_service, Gen3dAiServiceConfig};
use super::convert;
use super::parse;
use super::prompts;
use super::schema::{AiDraftJsonV1, AiPlanJsonV1};
use super::structured_outputs::Gen3dAiJsonSchemaKind;
use super::{Gen3dAiProgress, Gen3dAiSessionState};

#[derive(Clone, Debug)]
pub(crate) struct Gen3dHeadlessPrefabResult {
    pub(crate) root_prefab_id: u128,
    pub(crate) defs: Vec<ObjectDef>,
}

pub(crate) fn gen3d_generate_prefab_defs_headless(
    config: &AppConfig,
    prompt: &str,
    run_dir: &Path,
) -> Result<Gen3dHeadlessPrefabResult, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Gen3D headless: empty prompt".into());
    }

    let llm = match config.gen3d_ai_service {
        crate::config::Gen3dAiService::OpenAi => config
            .openai
            .as_ref()
            .cloned()
            .map(Gen3dAiServiceConfig::OpenAi)
            .ok_or_else(|| {
                "Gen3D headless: OpenAI is not configured (missing [openai] in config.toml)."
                    .to_string()
            })?,
        crate::config::Gen3dAiService::Mimo => config
            .mimo
            .as_ref()
            .cloned()
            .map(Gen3dAiServiceConfig::Mimo)
            .ok_or_else(|| {
                "Gen3D headless: MiMo is not configured (missing [mimo] in config.toml)."
                    .to_string()
            })?,
        crate::config::Gen3dAiService::Gemini => config
            .gemini
            .as_ref()
            .cloned()
            .map(Gen3dAiServiceConfig::Gemini)
            .ok_or_else(|| {
                "Gen3D headless: Gemini is not configured (missing [gemini] in config.toml)."
                    .to_string()
            })?,
        crate::config::Gen3dAiService::Claude => config
            .claude
            .as_ref()
            .cloned()
            .map(Gen3dAiServiceConfig::Claude)
            .ok_or_else(|| {
                "Gen3D headless: Claude is not configured (missing [claude] in config.toml)."
                    .to_string()
            })?,
    };

    std::fs::create_dir_all(run_dir)
        .map_err(|err| format!("Failed to create {}: {err}", run_dir.display()))?;

    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress::default()));
    let mut session = Gen3dAiSessionState::default();

    let speed = Gen3dSpeedMode::default();
    let image_paths: Vec<std::path::PathBuf> = Vec::new();

    let plan = gen3d_plan_via_openai(
        config,
        &progress,
        &mut session,
        &llm,
        prompt,
        speed,
        &image_paths,
        run_dir,
    )?;

    let (planned_components, assembly_notes, mut initial_defs) =
        convert::ai_plan_to_initial_draft_defs(plan)?;

    let mut child_ref_parts: HashMap<u128, Vec<ObjectPartDef>> = HashMap::new();
    for def in initial_defs.iter() {
        if !def.label.starts_with("gen3d_component_") {
            continue;
        }
        let refs: Vec<ObjectPartDef> = def
            .parts
            .iter()
            .filter(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. }))
            .cloned()
            .collect();
        if !refs.is_empty() {
            child_ref_parts.insert(def.object_id, refs);
        }
    }

    let mut draft = Gen3dDraft {
        defs: std::mem::take(&mut initial_defs),
    };

    for (idx, planned) in planned_components.iter().enumerate() {
        let draft_json = gen3d_component_draft_via_openai(
            config,
            &progress,
            &mut session,
            &llm,
            prompt,
            speed,
            &assembly_notes,
            planned_components.as_slice(),
            idx,
            &image_paths,
            run_dir,
        )?;

        let converted = convert::ai_to_component_def(planned, draft_json, Some(run_dir))?;
        let mut def = converted.def;
        if let Some(mut refs) = child_ref_parts.remove(&def.object_id) {
            def.parts.append(&mut refs);
        }

        if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == def.object_id) {
            *existing = def;
        } else {
            draft.defs.push(def);
        }
    }

    let min_ground_contact_y_in_root = planned_components
        .iter()
        .flat_map(|comp| comp.contacts.iter().map(move |contact| (comp, contact)))
        .filter(|(_comp, contact)| contact.kind == super::schema::AiContactKindJson::Ground)
        .filter_map(|(comp, contact)| {
            let anchor_name = contact.anchor.trim();
            let anchor_local = if anchor_name == "origin" {
                Some(Vec3::ZERO)
            } else {
                comp.anchors
                    .iter()
                    .find(|a| a.name.as_ref() == anchor_name)
                    .map(|a| a.transform.translation)
            }?;
            let pos = comp.pos;
            let rot = comp.rot;
            (pos.is_finite() && rot.is_finite()).then_some((pos + rot * anchor_local).y)
        })
        .filter(|y| y.is_finite())
        .reduce(f32::min);

    let (root_prefab_id, defs) =
        super::super::save::draft_to_saved_defs(&draft, false, None, min_ground_contact_y_in_root)?;

    Ok(Gen3dHeadlessPrefabResult {
        root_prefab_id,
        defs,
    })
}

fn gen3d_plan_via_openai(
    config: &AppConfig,
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: &mut Gen3dAiSessionState,
    ai: &Gen3dAiServiceConfig,
    prompt: &str,
    speed: Gen3dSpeedMode,
    image_paths: &[std::path::PathBuf],
    run_dir: &Path,
) -> Result<AiPlanJsonV1, String> {
    let system = prompts::build_gen3d_plan_system_instructions();
    let user = prompts::build_gen3d_plan_user_text(prompt, None, speed);

    let resp = generate_text_via_ai_service(
        progress,
        session.clone(),
        None,
        std::time::Duration::from_secs(config.ai_request_timeout_secs.max(1)),
        Some(Gen3dAiJsonSchemaKind::PlanV1),
        config.gen3d_require_structured_outputs,
        ai,
        ai.model_reasoning_effort(),
        &system,
        &user,
        image_paths,
        Some(run_dir),
        "plan",
    )?;

    *session = resp.session;
    parse::parse_ai_plan_from_text(&resp.text)
}

fn gen3d_component_draft_via_openai(
    config: &AppConfig,
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: &mut Gen3dAiSessionState,
    ai: &Gen3dAiServiceConfig,
    prompt: &str,
    speed: Gen3dSpeedMode,
    assembly_notes: &str,
    planned_components: &[super::Gen3dPlannedComponent],
    component_index: usize,
    image_paths: &[std::path::PathBuf],
    run_dir: &Path,
) -> Result<AiDraftJsonV1, String> {
    let system = prompts::build_gen3d_component_system_instructions();
    let user = prompts::build_gen3d_component_user_text(
        prompt,
        None,
        speed,
        assembly_notes,
        planned_components,
        component_index,
    );

    let component = planned_components
        .get(component_index)
        .map(|c| c.name.as_str())
        .unwrap_or("component");
    let prefix = format!(
        "component_{:02}_{}",
        component_index + 1,
        sanitize_artifact_key(component)
    );

    let resp = generate_text_via_ai_service(
        progress,
        session.clone(),
        None,
        std::time::Duration::from_secs(config.ai_request_timeout_secs.max(1)),
        Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
        config.gen3d_require_structured_outputs,
        ai,
        ai.model_reasoning_effort(),
        &system,
        &user,
        image_paths,
        Some(run_dir),
        &prefix,
    )?;

    *session = resp.session;
    parse::parse_ai_draft_from_text(&resp.text)
}

fn sanitize_artifact_key(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '_' | '-' | ' ') {
            if !out.ends_with('_') {
                out.push('_');
            }
        }
    }
    let out = out.trim_matches('_');
    let out = if out.is_empty() { "x" } else { out };
    out.chars().take(32).collect()
}
