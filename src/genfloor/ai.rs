use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::config;
use crate::genfloor::defs::FloorDefV1;
use crate::genfloor::runtime::{set_active_world_floor, ActiveWorldFloor};
use crate::genfloor::state::{GenFloorAiJob, GenFloorAiResult, GenFloorAiUsage, GenFloorWorkshop};
use crate::threaded_result::{
    new_shared_result, spawn_worker_thread, take_shared_result, SharedResult,
};
use crate::types::BuildScene;

const GENFLOOR_SYSTEM_PROMPT: &str = "You are a terrain generator for a game editor.\n\
Return ONLY a single JSON object. No markdown, no commentary.\n\
The JSON must follow this schema (all fields required unless marked optional):\n\
{\n\
  format_version: 1,\n\
  label: string? ,\n\
  mesh: {\n\
    kind: 'grid',\n\
    size_m: [f32, f32],\n\
    subdiv: [u32, u32],\n\
    thickness_m: f32,\n\
    uv_tiling: [f32, f32]\n\
  },\n\
  material: {\n\
    base_color_rgba: [f32, f32, f32, f32],\n\
    metallic: f32,\n\
    roughness: f32,\n\
    unlit: bool\n\
  },\n\
  coloring: {\n\
    mode: 'solid' | 'checker' | 'stripes' | 'gradient' | 'noise',\n\
    palette: [[f32, f32, f32, f32]],\n\
    scale: [f32, f32],\n\
    angle_deg: f32,\n\
    noise: { seed: u32, frequency: f32, octaves: u32, lacunarity: f32, gain: f32 }\n\
  },\n\
  relief: {\n\
    mode: 'none' | 'noise',\n\
    amplitude: f32,\n\
    noise: { seed: u32, frequency: f32, octaves: u32, lacunarity: f32, gain: f32 }\n\
  },\n\
  animation: {\n\
    mode: 'none' | 'cpu' | 'gpu',\n\
    waves: [\n\
      { amplitude: f32, wavelength: f32, direction: [f32, f32], speed: f32, phase: f32 }\n\
    ],\n\
    normal_strength: f32\n\
  }\n\
}\n\
Rules:\n\
- Use safe, moderate values and keep subdiv <= 256.\n\
- Terrain should fill the scene; keep size_m around [60, 60] unless the user explicitly requests otherwise.\n\
- If the prompt is short, prefer a subtle look.\n\
- If you use coloring.mode != 'solid', include a 2-5 color palette and set material.base_color_rgba to [1,1,1,1].\n\
- If the prompt asks for bumpy/uneven terrain, use relief.mode = 'noise' with a non-zero amplitude.\n\
- The output MUST be valid JSON (not JSON5).\n\
";

pub(crate) fn genfloor_start_ai_job(
    config: &config::AppConfig,
    prompt: &str,
    job: &mut GenFloorAiJob,
    workshop: &mut crate::gen3d::Gen3dWorkshop,
    floor_workshop: &mut GenFloorWorkshop,
) {
    if job.running {
        return;
    }

    let prompt = prompt.trim();
    if prompt.is_empty() {
        workshop.error = Some("Prompt is empty.".to_string());
        return;
    }
    if let Err(err) = crate::gen3d::validate_gen3d_user_prompt_limits(prompt) {
        workshop.error = Some(err);
        return;
    }

    job.running = true;
    job.cancel_requested = false;
    job.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    job.started_at = Some(Instant::now());
    job.last_run_elapsed = None;
    job.run_tokens = 0;
    floor_workshop.draft = None;
    workshop.error = None;
    workshop.status = "Building terrain...".to_string();
    floor_workshop.error = None;
    floor_workshop.status = workshop.status.clone();

    let shared: SharedResult<GenFloorAiResult, String> = new_shared_result();
    job.shared = Some(shared.clone());

    let user_text = prompt.to_string();
    let config = config.clone();
    let cancel_flag = job.cancel_flag.clone();
    let thread_name = format!("gravimera_genfloor_{}", uuid::Uuid::new_v4());

    let _ = spawn_worker_thread(
        thread_name,
        shared,
        move || call_genfloor_ai(&config, &user_text, cancel_flag),
        |_| {},
    );
}

pub(crate) fn genfloor_poll_ai_job(
    mut job: ResMut<GenFloorAiJob>,
    mut workshop: ResMut<crate::gen3d::Gen3dWorkshop>,
    mut floor_workshop: ResMut<GenFloorWorkshop>,
    active: Res<crate::realm::ActiveRealmScene>,
    mut active_floor: ResMut<ActiveWorldFloor>,
    mut floor_library: ResMut<crate::floor_library_ui::FloorLibraryUiState>,
) {
    let Some(shared) = job.shared.as_ref() else {
        return;
    };
    let Some(result) = take_shared_result(shared) else {
        return;
    };

    if job.cancel_requested {
        job.running = false;
        job.cancel_requested = false;
        job.cancel_flag = None;
        job.last_run_elapsed = job.started_at.map(|start| start.elapsed());
        job.started_at = None;
        job.shared = None;
        workshop.status = "Build canceled.".to_string();
        workshop.error = None;
        floor_workshop.status = workshop.status.clone();
        floor_workshop.error = None;
        floor_workshop.draft = None;
        return;
    }

    job.running = false;
    job.cancel_requested = false;
    job.cancel_flag = None;
    job.last_run_elapsed = job.started_at.map(|start| start.elapsed());
    job.started_at = None;
    job.shared = None;

    match result {
        Ok(mut res) => {
            if let Some(usage) = res.usage.take() {
                job.run_tokens = usage.total_tokens;
                job.total_tokens = job.total_tokens.saturating_add(usage.total_tokens);
            }
            res.def.canonicalize_in_place();

            let floor_id = job
                .edit_base_floor_id()
                .unwrap_or_else(|| uuid::Uuid::new_v4().as_u128());
            match crate::realm_floor_packages::save_realm_floor_def(
                &active.realm_id,
                floor_id,
                &res.def,
            ) {
                Ok(_) => {
                    let source_dir =
                        crate::realm_floor_packages::realm_floor_package_genfloor_source_dir(
                            &active.realm_id,
                            floor_id,
                        );
                    let _ = std::fs::write(source_dir.join("prompt.txt"), workshop.prompt.as_str());
                    set_active_world_floor(&mut active_floor, Some(floor_id), res.def.clone());
                    floor_library.mark_models_dirty();
                    floor_library.set_selected_floor_id(Some(floor_id));
                    if let Err(err) = crate::scene_floor_selection::save_scene_floor_selection(
                        &active.realm_id,
                        &active.scene_id,
                        Some(floor_id),
                    ) {
                        workshop.error = Some(format!(
                            "Terrain saved, but failed to persist selection: {err}"
                        ));
                    } else {
                        workshop.error = None;
                    }
                    if job.edit_base_floor_id().is_none() {
                        job.set_edit_base_floor_id(Some(floor_id));
                    }
                    job.set_last_saved_floor_id(Some(floor_id));
                    floor_workshop.draft = Some(res.def);
                    workshop.status = "Build finished. Terrain saved. Click Edit to run again (auto-save overwrites the same terrain).".to_string();
                    floor_workshop.status = workshop.status.clone();
                    floor_workshop.error = workshop.error.clone();
                }
                Err(err) => {
                    floor_workshop.draft = Some(res.def);
                    workshop.error = Some(err);
                    workshop.status = "Build finished, but auto-save failed.".to_string();
                    floor_workshop.status = workshop.status.clone();
                    floor_workshop.error = workshop.error.clone();
                }
            }
        }
        Err(err) => {
            workshop.error = Some(err);
            workshop.status = "Build failed.".to_string();
            floor_workshop.status = workshop.status.clone();
            floor_workshop.error = workshop.error.clone();
            floor_workshop.draft = None;
        }
    }
}

pub(crate) fn genfloor_cancel_ai_job(
    job: &mut GenFloorAiJob,
    workshop: &mut crate::gen3d::Gen3dWorkshop,
) {
    if !job.running {
        return;
    }
    job.cancel_requested = true;
    if let Some(flag) = job.cancel_flag.as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
    workshop.status = "Cancel requested...".to_string();
}

pub(crate) fn genfloor_update_ui_stats(
    build_scene: Res<State<BuildScene>>,
    job: Res<GenFloorAiJob>,
    mut texts: ParamSet<(
        Query<&mut Text, With<crate::gen3d::Gen3dGenerateButtonText>>,
        Query<&mut Text, With<crate::gen3d::Gen3dPreviewStatsText>>,
    )>,
) {
    if !matches!(build_scene.get(), BuildScene::FloorPreview) {
        return;
    }

    let label = if job.running {
        "Stop"
    } else if job.edit_base_floor_id().is_some() {
        "Edit"
    } else {
        "Build"
    };
    for mut text in &mut texts.p0() {
        text.0 = label.to_string();
    }

    let run_time = job
        .run_elapsed()
        .map(|d| {
            let secs = d.as_secs();
            if secs < 60 {
                format!("{:.1}s", d.as_secs_f32())
            } else {
                format!("{}m {}s", secs / 60, secs % 60)
            }
        })
        .unwrap_or_else(|| "—".into());
    let run_tokens = format_compact_count(job.run_tokens);
    let total_tokens = format_compact_count(job.total_tokens);
    let stats =
        format!("Run time: {run_time}\nTokens (run): {run_tokens}\nTokens (total): {total_tokens}");
    for mut text in &mut texts.p1() {
        text.0 = stats.clone();
    }
}

fn format_compact_count(value: u64) -> String {
    const K: f64 = 1_000.0;
    const M: f64 = 1_000_000.0;
    const B: f64 = 1_000_000_000.0;

    let v = value as f64;
    if v >= B {
        format!("{:.2}B", v / B)
    } else if v >= M {
        format!("{:.2}M", v / M)
    } else if v >= K {
        format!("{:.1}K", v / K)
    } else {
        value.to_string()
    }
}

fn call_genfloor_ai(
    config: &config::AppConfig,
    prompt: &str,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<GenFloorAiResult, String> {
    let response =
        crate::gen3d::gen3d_generate_text_simple(config, GENFLOOR_SYSTEM_PROMPT, prompt, cancel)?;

    let mut def: FloorDefV1 = serde_json::from_str(&response.text)
        .or_else(|_| json5::from_str(&response.text))
        .map_err(|err| format!("Terrain JSON parse error: {err}"))?;
    def.canonicalize_in_place();

    let usage = response.total_tokens.map(|total| GenFloorAiUsage {
        total_tokens: total,
    });

    Ok(GenFloorAiResult { def, usage })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_genfloor_ai_returns_valid_floor_def() {
        let mut config = crate::config::AppConfig::default();
        config.gen3d_ai_service = crate::config::Gen3dAiService::OpenAi;
        config.openai = Some(crate::config::OpenAiConfig {
            base_url: "mock://gen3d".to_string(),
            model: "gpt-5.4".to_string(),
            reasoning_effort: "none".to_string(),
            api_key: "".to_string(),
        });

        let result = call_genfloor_ai(&config, "A checkerboard floor (mock)", None)
            .expect("genfloor mock call");
        assert_eq!(
            result.def.format_version,
            crate::genfloor::defs::FLOOR_DEF_FORMAT_VERSION
        );
        assert!(matches!(
            result.def.coloring.mode,
            crate::genfloor::defs::FloorColoringMode::Checker
        ));
        assert!(result.def.mesh.subdiv[0] <= 256);
        assert!(result.def.mesh.subdiv[1] <= 256);
    }
}
