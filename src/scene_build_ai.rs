use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::log::{error, info, warn};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::AppConfig;
use crate::object::registry::ObjectLibrary;
use crate::realm::ActiveRealmScene;
use crate::scene_authoring_ui::SceneAuthoringUiState;
use crate::scene_sources::SceneSourcesV1;
use crate::scene_sources_patch::{
    SceneSourcesPatchOpV1, SceneSourcesPatchV1, SCENE_SOURCES_PATCH_FORMAT_VERSION,
};
use crate::scene_sources_runtime::{SceneSourcesWorkspace, SceneWorldInstance};
use crate::scene_validation::{HardGateSpecV1, ScorecardSpecV1};
use crate::types::{
    BuildObject, Commandable, ObjectId, ObjectPrefabId, ObjectTint, Player, SceneLayerOwner,
};

const CURL_CONNECT_TIMEOUT_SECS: u32 = 15;
const CURL_MAX_TIME_SECS: u32 = 600;
const MAX_STEP_ATTEMPTS: u8 = 3;

const SCENE_BUILD_STEP_SCREENSHOT_WIDTH_PX: u32 = 1920;
const SCENE_BUILD_STEP_SCREENSHOT_HEIGHT_PX: u32 = 1080;
const SCENE_BUILD_STEP_SCREENSHOT_TIMEOUT_SECS: u64 = 45;

#[derive(Clone, Copy, Debug)]
enum SceneBuildStepScreenshotView {
    Front,
    Right,
    Back,
    Left,
    Top,
}

impl SceneBuildStepScreenshotView {
    fn file_stem(self) -> &'static str {
        match self {
            SceneBuildStepScreenshotView::Front => "front",
            SceneBuildStepScreenshotView::Right => "right",
            SceneBuildStepScreenshotView::Back => "back",
            SceneBuildStepScreenshotView::Left => "left",
            SceneBuildStepScreenshotView::Top => "top",
        }
    }

    fn orbit_angles(self) -> (f32, f32) {
        use std::f32::consts::{FRAC_PI_2, PI};

        // Match the interactive camera-ish angle for the four horizontal views.
        let pitch_iso = -0.45;
        let pitch_top = -1.35;

        match self {
            SceneBuildStepScreenshotView::Front => (0.0, pitch_iso),
            SceneBuildStepScreenshotView::Right => (FRAC_PI_2, pitch_iso),
            SceneBuildStepScreenshotView::Back => (PI, pitch_iso),
            SceneBuildStepScreenshotView::Left => (-FRAC_PI_2, pitch_iso),
            SceneBuildStepScreenshotView::Top => (0.0, pitch_top),
        }
    }
}

const SCENE_BUILD_STEP_SCREENSHOT_VIEWS: [SceneBuildStepScreenshotView; 5] = [
    SceneBuildStepScreenshotView::Front,
    SceneBuildStepScreenshotView::Right,
    SceneBuildStepScreenshotView::Back,
    SceneBuildStepScreenshotView::Left,
    SceneBuildStepScreenshotView::Top,
];

#[derive(Clone, Debug)]
struct SceneBuildStepScreenshotProgress {
    expected: usize,
    completed: usize,
}

#[derive(Clone, Debug)]
struct SceneBuildStepScreenshotCapture {
    step_dir: PathBuf,
    cameras: Vec<Entity>,
    progress: Arc<Mutex<SceneBuildStepScreenshotProgress>>,
    image_paths: Vec<PathBuf>,
    started_at: Instant,
    last_reported_completed: usize,
}

#[derive(Component)]
struct SceneBuildStepScreenshotCamera;

fn create_scene_build_render_target(
    images: &mut Assets<Image>,
    width_px: u32,
    height_px: u32,
) -> Handle<Image> {
    let mut image = Image::new_target_texture(
        width_px.max(1),
        height_px.max(1),
        TextureFormat::bevy_default(),
        None,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    images.add(image)
}

fn scene_build_orbit_transform(yaw: f32, pitch: f32, distance: f32, focus: Vec3) -> Transform {
    let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let pos = focus + rot * Vec3::new(0.0, 0.0, distance);
    Transform::from_translation(pos).looking_at(focus, Vec3::Y)
}

fn scene_build_required_distance_for_view(
    half_extents: Vec3,
    yaw: f32,
    pitch: f32,
    fov_y: f32,
    aspect: f32,
    near: f32,
) -> f32 {
    let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let mut view_dir = -rot * Vec3::Z;
    if !view_dir.is_finite() || view_dir.length_squared() <= 1e-6 {
        view_dir = -Vec3::Z;
    } else {
        view_dir = view_dir.normalize();
    }

    let mut right = Vec3::Y.cross(view_dir);
    if !right.is_finite() || right.length_squared() <= 1e-6 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }
    let mut up = view_dir.cross(right);
    if !up.is_finite() || up.length_squared() <= 1e-6 {
        up = Vec3::Y;
    } else {
        up = up.normalize();
    }

    let extent_right = half_extents.x * right.x.abs()
        + half_extents.y * right.y.abs()
        + half_extents.z * right.z.abs();
    let extent_up =
        half_extents.x * up.x.abs() + half_extents.y * up.y.abs() + half_extents.z * up.z.abs();
    let extent_forward = half_extents.x * view_dir.x.abs()
        + half_extents.y * view_dir.y.abs()
        + half_extents.z * view_dir.z.abs();

    let tan_y = (fov_y * 0.5).tan().max(1e-4);
    let tan_x = (tan_y * aspect).max(1e-4);
    let dist_y = extent_up / tan_y;
    let dist_x = extent_right / tan_x;

    // Ensure the near plane won't clip the bounds.
    dist_x.max(dist_y).max(extent_forward + near + 0.05)
}

fn scene_build_focus_and_half_extents(
    library: &ObjectLibrary,
    instances: impl Iterator<Item = (Transform, ObjectPrefabId)>,
) -> (Vec3, Vec3) {
    let mut any = false;
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for (transform, prefab_id) in instances {
        if !transform.translation.is_finite() || !transform.scale.is_finite() {
            continue;
        }

        let base_size = library
            .size(prefab_id.0)
            .unwrap_or_else(|| Vec3::splat(crate::constants::BUILD_UNIT_SIZE));
        let size = base_size.abs() * transform.scale.abs().max(Vec3::splat(0.001));
        let half = (size * 0.5).max(Vec3::splat(0.001));

        let axis_x = (transform.rotation * Vec3::X).abs();
        let axis_y = (transform.rotation * Vec3::Y).abs();
        let axis_z = (transform.rotation * Vec3::Z).abs();
        let world_half = axis_x * half.x + axis_y * half.y + axis_z * half.z;

        let aabb_min = transform.translation - world_half;
        let aabb_max = transform.translation + world_half;

        min = min.min(aabb_min);
        max = max.max(aabb_max);
        any = true;
    }

    if !any {
        return (Vec3::ZERO, Vec3::new(20.0, 6.0, 20.0));
    }

    let focus = (min + max) * 0.5;
    let half_extents = ((max - min) * 0.5).max(Vec3::splat(0.5));
    (focus, half_extents)
}

fn start_scene_build_step_screenshot_capture(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    step_dir: &Path,
    focus: Vec3,
    half_extents: Vec3,
) -> Result<SceneBuildStepScreenshotCapture, String> {
    std::fs::create_dir_all(step_dir)
        .map_err(|err| format!("Failed to create {}: {err}", step_dir.display()))?;

    let width_px = SCENE_BUILD_STEP_SCREENSHOT_WIDTH_PX;
    let height_px = SCENE_BUILD_STEP_SCREENSHOT_HEIGHT_PX;

    let aspect = width_px.max(1) as f32 / height_px.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;

    let base_distance = SCENE_BUILD_STEP_SCREENSHOT_VIEWS
        .iter()
        .map(|view| {
            let (yaw, pitch) = view.orbit_angles();
            scene_build_required_distance_for_view(half_extents, yaw, pitch, fov_y, aspect, near)
        })
        .fold(0.0f32, f32::max);

    // Include a bit of margin so fences/trees on the edges are less likely to crop.
    let distance = (base_distance * 1.15).clamp(near + 0.2, 500.0);

    let progress = Arc::new(Mutex::new(SceneBuildStepScreenshotProgress {
        expected: SCENE_BUILD_STEP_SCREENSHOT_VIEWS.len(),
        completed: 0,
    }));

    let mut cameras = Vec::with_capacity(SCENE_BUILD_STEP_SCREENSHOT_VIEWS.len());
    let mut image_paths = Vec::with_capacity(SCENE_BUILD_STEP_SCREENSHOT_VIEWS.len());

    let mut views_manifest = Vec::new();

    for &view in &SCENE_BUILD_STEP_SCREENSHOT_VIEWS {
        let target = create_scene_build_render_target(images, width_px, height_px);
        let (yaw, pitch) = view.orbit_angles();
        let transform = scene_build_orbit_transform(yaw, pitch, distance, focus);

        let camera = commands
            .spawn((
                Camera3d::default(),
                bevy::camera::Projection::Perspective(projection.clone()),
                Camera {
                    clear_color: ClearColorConfig::Custom(Color::srgb(0.05, 0.05, 0.06)),
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                Tonemapping::TonyMcMapface,
                transform,
                SceneBuildStepScreenshotCamera,
            ))
            .id();
        cameras.push(camera);

        let file_name = format!("view_{}.png", view.file_stem());
        let path = step_dir.join(&file_name);
        image_paths.push(path.clone());
        views_manifest.push(serde_json::json!({
            "name": view.file_stem(),
            "file": file_name,
            "yaw": yaw,
            "pitch": pitch,
            "distance": distance,
        }));

        let progress_clone = progress.clone();
        commands
            .spawn(Screenshot::image(target))
            .observe(move |event: On<ScreenshotCaptured>| {
                let mut saver = save_to_disk(path.clone());
                saver(event);
                if let Ok(mut guard) = progress_clone.lock() {
                    guard.completed = guard.completed.saturating_add(1);
                }
            });
    }

    let manifest = serde_json::json!({
        "format_version": 1,
        "width_px": width_px,
        "height_px": height_px,
        "focus": { "x": focus.x, "y": focus.y, "z": focus.z },
        "half_extents": { "x": half_extents.x, "y": half_extents.y, "z": half_extents.z },
        "views": views_manifest,
    });
    let _ = write_json_artifact(&step_dir.join("screenshots_manifest.json"), &manifest);

    Ok(SceneBuildStepScreenshotCapture {
        step_dir: step_dir.to_path_buf(),
        cameras,
        progress,
        image_paths,
        started_at: Instant::now(),
        last_reported_completed: 0,
    })
}

fn scene_build_step_screenshot_capture_progress(
    capture: &SceneBuildStepScreenshotCapture,
) -> (usize, usize) {
    let Ok(guard) = capture.progress.lock() else {
        return (0, capture.image_paths.len());
    };
    (guard.completed, guard.expected)
}

fn scene_build_step_screenshot_capture_done(capture: &SceneBuildStepScreenshotCapture) -> bool {
    let (completed, expected) = scene_build_step_screenshot_capture_progress(capture);
    completed >= expected.max(1)
}

fn scene_build_step_screenshot_capture_timed_out(
    capture: &SceneBuildStepScreenshotCapture,
) -> bool {
    let timeout = Duration::from_secs(SCENE_BUILD_STEP_SCREENSHOT_TIMEOUT_SECS);
    capture.started_at.elapsed() > timeout
}

fn cleanup_scene_build_step_screenshot_capture(
    commands: &mut Commands,
    capture: &SceneBuildStepScreenshotCapture,
) {
    for entity in &capture.cameras {
        commands.entity(*entity).try_despawn();
    }
}

#[derive(Clone, Debug, Default)]
struct SceneBuildAiProgress {
    message: String,
}

#[derive(Clone, Debug)]
struct SceneBuildAiStatus {
    run_id: String,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
struct SceneBuildAiPlanStep {
    step_id: String,
    title: String,
    goal: String,
}

#[derive(Clone, Debug)]
struct StepRepairContext {
    error: String,
    previous_output: String,
}

#[derive(Clone, Debug)]
enum SceneBuildAiPhase {
    Cleanup,
    PlanRequest,
    StepRequest { step_index: usize, attempt: u8 },
    CaptureInit { step_index: usize, run_step: u32 },
    CaptureWait { step_index: usize },
}

struct SceneBuildAiJob {
    run_id: String,
    target_realm_id: String,
    target_scene_id: String,
    run_dir: PathBuf,
    description: String,

    openai_base_url: String,
    openai_api_key: String,
    openai_model: String,
    openai_reasoning_effort: String,

    phase: SceneBuildAiPhase,
    plan_steps: Vec<SceneBuildAiPlanStep>,
    next_run_step: u32,
    capture: Option<SceneBuildStepScreenshotCapture>,

    progress: Arc<Mutex<SceneBuildAiProgress>>,
    shared_result: Arc<Mutex<Option<Result<String, String>>>>,
}

#[derive(Resource, Default)]
pub(crate) struct SceneBuildAiRuntime {
    in_flight: Option<SceneBuildAiJob>,
    last_status: Option<SceneBuildAiStatus>,
}

impl SceneBuildAiRuntime {
    pub(crate) fn ui_progress_summary(&self) -> String {
        if let Some(job) = &self.in_flight {
            let msg = job
                .progress
                .lock()
                .ok()
                .map(|p| p.message.clone())
                .unwrap_or_default();
            let run_id = brief_run_id(&job.run_id);
            if msg.trim().is_empty() {
                format!("Build running ({run_id}).")
            } else {
                format!("Build {run_id}: {}", msg.trim())
            }
        } else if let Some(status) = &self.last_status {
            let run_id = brief_run_id(&status.run_id);
            if status.message.trim().is_empty() {
                format!("Last build ({run_id}).")
            } else {
                format!("Last build {run_id}: {}", status.message.trim())
            }
        } else {
            "No build running.".to_string()
        }
    }
}

fn brief_run_id(run_id: &str) -> String {
    let run_id = run_id.trim();
    if let Some(uuid) = run_id.strip_prefix("scene_build_") {
        let short = uuid.get(..8).unwrap_or(uuid);
        return format!("scene_build_{short}");
    }

    if run_id.len() <= 16 {
        return run_id.to_string();
    }

    let start = run_id.get(..8).unwrap_or(run_id);
    let end = run_id.get(run_id.len().saturating_sub(4)..).unwrap_or("");
    format!("{start}…{end}")
}

fn append_scene_build_run_log(run_dir: &Path, message: impl AsRef<str>) {
    let path = run_dir.join("scene_build_run.log");
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut line = format!("[{ts_ms}] {}", message.as_ref());
    if !line.ends_with("\n") {
        line.push('\n');
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    use std::io::Write;
    let _ = file.write_all(line.as_bytes());
}

fn set_progress(
    progress: &Arc<Mutex<SceneBuildAiProgress>>,
    run_id: &str,
    run_dir: &Path,
    message: impl Into<String>,
) {
    let mut message = message.into();
    message = message.replace(['\r', '\n'], " ");
    let message = message.trim().to_string();

    if let Ok(mut guard) = progress.lock() {
        guard.message = message.clone();
    }

    if message.is_empty() {
        info!("Scene build {run_id}: progress updated.");
    } else {
        info!("Scene build {run_id}: {message}");
    }

    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let _ = std::fs::write(run_dir.join("progress.txt"), format!("{message}\n"));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("progress.log"))
    {
        use std::io::Write;
        let _ = writeln!(file, "[{ts_ms}] {message}");
    }
    append_scene_build_run_log(run_dir, format!("progress: {message}"));
}

pub(crate) fn start_scene_build_from_description(
    runtime: &mut SceneBuildAiRuntime,
    config: &AppConfig,
    active: &ActiveRealmScene,
    _library: &ObjectLibrary,
    description: &str,
) -> Result<String, String> {
    if runtime.in_flight.is_some() {
        return Err("A build is already running.".to_string());
    }

    let description = description.trim();
    if description.is_empty() {
        return Err("Scene description is empty.".to_string());
    }

    let openai = config
        .openai
        .as_ref()
        .ok_or_else(|| "OpenAI is not configured (missing [openai] in config.toml).".to_string())?;

    let run_id = format!("scene_build_{}", uuid::Uuid::new_v4());
    let scene_dir = crate::paths::scene_dir(&active.realm_id, &active.scene_id);
    let run_dir = scene_dir.join("runs").join(&run_id);
    let llm_root = run_dir.join("llm");
    std::fs::create_dir_all(&llm_root)
        .map_err(|err| format!("Failed to create {}: {err}", llm_root.display()))?;

    info!(
        "Scene build {run_id} started: realm={}/{} run_dir={}",
        active.realm_id,
        active.scene_id,
        run_dir.display()
    );

    runtime.last_status = None;

    let progress: Arc<Mutex<SceneBuildAiProgress>> =
        Arc::new(Mutex::new(SceneBuildAiProgress::default()));
    set_progress(
        &progress,
        &run_id,
        &run_dir,
        format!(
            "Starting build for {}/{} (model={}).",
            active.realm_id, active.scene_id, openai.model
        ),
    );

    runtime.in_flight = Some(SceneBuildAiJob {
        run_id: run_id.clone(),
        target_realm_id: active.realm_id.clone(),
        target_scene_id: active.scene_id.clone(),
        run_dir,
        description: description.to_string(),
        openai_base_url: openai.base_url.clone(),
        openai_api_key: openai.api_key.clone(),
        openai_model: openai.model.clone(),
        openai_reasoning_effort: openai.model_reasoning_effort.clone(),
        phase: SceneBuildAiPhase::Cleanup,
        plan_steps: Vec::new(),
        next_run_step: 1,
        capture: None,
        progress,
        shared_result: Arc::new(Mutex::new(None)),
    });

    Ok(run_id)
}

pub(crate) fn scene_build_ai_poll(
    mut commands: Commands,
    mut runtime: ResMut<SceneBuildAiRuntime>,
    mut ui: ResMut<SceneAuthoringUiState>,
    active: Res<ActiveRealmScene>,
    library: Res<ObjectLibrary>,
    mut workspace: ResMut<SceneSourcesWorkspace>,
    mut images: ResMut<Assets<Image>>,
    scene_instances: Query<
        (
            Entity,
            &Transform,
            &ObjectId,
            &ObjectPrefabId,
            Option<&ObjectTint>,
            Option<&SceneLayerOwner>,
        ),
        (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>),
    >,
) {
    let Some(mut job) = runtime.in_flight.take() else {
        return;
    };

    if active.realm_id != job.target_realm_id || active.scene_id != job.target_scene_id {
        let msg = format!(
            "Build started for {}/{} but active scene is now {}/{}; ignoring (run_id={}).",
            job.target_realm_id, job.target_scene_id, active.realm_id, active.scene_id, job.run_id
        );
        warn!("{msg}");
        set_progress(
            &job.progress,
            &job.run_id,
            &job.run_dir,
            "Ignored (scene changed).",
        );
        ui.set_status("Build ignored (scene changed).".to_string());
        ui.set_error(msg);
        if let Some(capture) = job.capture.as_ref() {
            cleanup_scene_build_step_screenshot_capture(&mut commands, capture);
        }
        runtime.last_status = Some(SceneBuildAiStatus {
            run_id: job.run_id,
            message: "Ignored (scene changed).".to_string(),
        });
        return;
    }

    let src_dir = crate::realm::scene_src_dir(&active);
    if workspace.loaded_from_dir.as_deref() != Some(src_dir.as_path()) {
        workspace.loaded_from_dir = Some(src_dir.clone());
        workspace.sources = None;
    }

    if workspace.sources.is_none() {
        if let Err(err) =
            crate::scene_sources_runtime::reload_scene_sources_in_workspace(&mut workspace)
        {
            finish_with_error(&mut runtime, &mut ui, &job, err);
            return;
        }
    }

    ui.clear_error();

    match job.phase.clone() {
        SceneBuildAiPhase::Cleanup => {
            let scorecard = default_scorecard();
            set_progress(
                &job.progress,
                &job.run_id,
                &job.run_dir,
                "Cleanup: clearing previous ai_ layers…",
            );

            let cleanup_patch = match build_delete_ai_layers_patch(&src_dir, &job.run_id) {
                Ok(patch) => patch,
                Err(err) => {
                    finish_with_error(&mut runtime, &mut ui, &job, err);
                    return;
                }
            };

            let existing = scene_instances
                .iter()
                .map(|(e, t, id, prefab, tint, owner)| SceneWorldInstance {
                    entity: e,
                    instance_id: *id,
                    prefab_id: *prefab,
                    transform: t.clone(),
                    tint: tint.map(|t| t.0),
                    owner_layer_id: owner.map(|o| o.layer_id.clone()),
                });

            let resp = crate::scene_runs::scene_run_apply_patch_step(
                &mut commands,
                &mut workspace,
                &library,
                existing,
                &job.run_id,
                job.next_run_step,
                &scorecard,
                &cleanup_patch,
            );

            match resp {
                Ok(r) => {
                    let (applied, spawned, updated, despawned) = apply_counts(&r.result);
                    if !applied {
                        finish_with_error(
                            &mut runtime,
                            &mut ui,
                            &job,
                            format!("Cleanup rejected by validators (run_id={}).", job.run_id),
                        );
                        return;
                    }
                    ui.set_status(format!(
                        "Cleanup OK (run_id={}): spawned={} updated={} despawned={}.",
                        job.run_id, spawned, updated, despawned
                    ));
                    job.next_run_step = job.next_run_step.saturating_add(1).max(1);

                    // Start plan request (built after cleanup so it can include any new Gen3D prefabs).
                    let llm_dir = job.run_dir.join("llm").join("plan");
                    let _ = std::fs::create_dir_all(&llm_dir);

                    let system_text = build_plan_system_prompt();
                    let user_text = build_plan_user_prompt(&active, &library, &job.description);
                    let _ = write_text_artifact(&llm_dir.join("system.txt"), &system_text);
                    let _ = write_text_artifact(&llm_dir.join("user.txt"), &user_text);

                    set_progress(&job.progress, &job.run_id, &job.run_dir, "Planning steps…");

                    job.shared_result = Arc::new(Mutex::new(None));
                    spawn_scene_build_llm_thread(
                        job.shared_result.clone(),
                        job.progress.clone(),
                        job.run_id.clone(),
                        job.run_dir.clone(),
                        job.openai_base_url.clone(),
                        job.openai_api_key.clone(),
                        job.openai_model.clone(),
                        job.openai_reasoning_effort.clone(),
                        system_text,
                        user_text,
                        llm_dir,
                        "Plan".to_string(),
                    );

                    job.phase = SceneBuildAiPhase::PlanRequest;
                    runtime.in_flight = Some(job);
                }
                Err(err) => {
                    finish_with_error(&mut runtime, &mut ui, &job, err);
                }
            }
        }
        SceneBuildAiPhase::PlanRequest => {
            let result = take_shared_result(&job.shared_result);
            let Some(result) = result else {
                runtime.in_flight = Some(job);
                return;
            };

            match result {
                Err(err) => {
                    finish_with_error(&mut runtime, &mut ui, &job, err);
                }
                Ok(text) => {
                    let llm_dir = job.run_dir.join("llm").join("plan");
                    let _ = write_text_artifact(&llm_dir.join("response.txt"), &text);

                    let steps = match parse_plan_steps(&text) {
                        Ok(steps) => steps,
                        Err(err) => {
                            finish_with_error(&mut runtime, &mut ui, &job, err);
                            return;
                        }
                    };

                    if steps.is_empty() {
                        finish_with_error(
                            &mut runtime,
                            &mut ui,
                            &job,
                            "Plan returned zero steps.".to_string(),
                        );
                        return;
                    }

                    let steps_doc = serde_json::to_value(&steps).unwrap_or(Value::Null);
                    let _ = write_json_artifact(&llm_dir.join("parsed_steps.json"), &steps_doc);

                    job.plan_steps = steps;

                    let total = job.plan_steps.len();
                    let first = &job.plan_steps[0];
                    set_progress(
                        &job.progress,
                        &job.run_id,
                        &job.run_dir,
                        format!(
                            "Plan ready: {total} steps. Starting 1/{total}: {}",
                            first.title
                        ),
                    );

                    start_step_request(&mut job, &active, &library, &src_dir, 0, 0, None);
                    runtime.in_flight = Some(job);
                }
            }
        }
        SceneBuildAiPhase::StepRequest {
            step_index,
            attempt,
        } => {
            let result = take_shared_result(&job.shared_result);
            let Some(result) = result else {
                runtime.in_flight = Some(job);
                return;
            };

            let total = job.plan_steps.len().max(1);
            let Some(step) = job.plan_steps.get(step_index) else {
                finish_with_error(
                    &mut runtime,
                    &mut ui,
                    &job,
                    format!("Internal error: missing plan step {step_index}."),
                );
                return;
            };

            match result {
                Err(err) => {
                    if attempt.saturating_add(1) < MAX_STEP_ATTEMPTS {
                        set_progress(
                            &job.progress,
                            &job.run_id,
                            &job.run_dir,
                            format!(
                                "Step {}/{} failed; retrying ({}/{})…",
                                step_index + 1,
                                total,
                                attempt + 1,
                                MAX_STEP_ATTEMPTS
                            ),
                        );
                        start_step_request(
                            &mut job,
                            &active,
                            &library,
                            &src_dir,
                            step_index,
                            attempt + 1,
                            Some(StepRepairContext {
                                error: err,
                                previous_output: String::new(),
                            }),
                        );
                        runtime.in_flight = Some(job);
                        return;
                    }

                    finish_with_error(&mut runtime, &mut ui, &job, err);
                }
                Ok(text) => {
                    let llm_dir = step_llm_dir(&job.run_dir, step_index, attempt);
                    let _ = write_text_artifact(&llm_dir.join("response.txt"), &text);

                    let (summary, ops) = match parse_step_ops(&text) {
                        Ok(v) => v,
                        Err(err) => {
                            if attempt.saturating_add(1) < MAX_STEP_ATTEMPTS {
                                set_progress(
                                    &job.progress,
                                    &job.run_id,
                                    &job.run_dir,
                                    format!(
                                        "Step {}/{} parse failed; retrying ({}/{})…",
                                        step_index + 1,
                                        total,
                                        attempt + 1,
                                        MAX_STEP_ATTEMPTS
                                    ),
                                );
                                start_step_request(
                                    &mut job,
                                    &active,
                                    &library,
                                    &src_dir,
                                    step_index,
                                    attempt + 1,
                                    Some(StepRepairContext {
                                        error: err,
                                        previous_output: text.clone(),
                                    }),
                                );
                                runtime.in_flight = Some(job);
                                return;
                            }

                            finish_with_error(&mut runtime, &mut ui, &job, err);
                            return;
                        }
                    };

                    let patch = match wrap_ops_as_patch(&job.run_id, step, step_index, &ops) {
                        Ok(patch) => patch,
                        Err(err) => {
                            finish_with_error(&mut runtime, &mut ui, &job, err);
                            return;
                        }
                    };

                    let ops_doc = serde_json::to_value(&ops).unwrap_or(Value::Null);
                    let _ = write_json_artifact(&llm_dir.join("parsed_ops.json"), &ops_doc);
                    let patch_doc = serde_json::to_value(&patch).unwrap_or(Value::Null);
                    let _ = write_json_artifact(&llm_dir.join("patch.json"), &patch_doc);

                    append_scene_build_run_log(
                        &job.run_dir,
                        format!(
                            "step {} attempt {} parsed_ops={} patch_request_id={}",
                            step_index + 1,
                            attempt,
                            ops.len(),
                            patch.request_id
                        ),
                    );

                    set_progress(
                        &job.progress,
                        &job.run_id,
                        &job.run_dir,
                        format!("Applying step {}/{}…", step_index + 1, total),
                    );

                    let scorecard = default_scorecard();
                    let existing = scene_instances
                        .iter()
                        .map(|(e, t, id, prefab, tint, owner)| SceneWorldInstance {
                            entity: e,
                            instance_id: *id,
                            prefab_id: *prefab,
                            transform: t.clone(),
                            tint: tint.map(|t| t.0),
                            owner_layer_id: owner.map(|o| o.layer_id.clone()),
                        });

                    let resp = crate::scene_runs::scene_run_apply_patch_step(
                        &mut commands,
                        &mut workspace,
                        &library,
                        existing,
                        &job.run_id,
                        job.next_run_step,
                        &scorecard,
                        &patch,
                    );

                    match resp {
                        Ok(r) => {
                            let (applied, spawned, updated, despawned) = apply_counts(&r.result);
                            if !applied {
                                let rejection = rejection_summary(&r.result);
                                append_scene_build_run_log(
                                    &job.run_dir,
                                    format!(
                                        "step {} attempt {} rejected: {}",
                                        step_index + 1,
                                        attempt,
                                        truncate_text(rejection.trim(), 300)
                                    ),
                                );

                                ui.set_status(format!(
                                    "Step {}/{} rejected (run_id={}).",
                                    step_index + 1,
                                    total,
                                    job.run_id
                                ));

                                job.next_run_step = job.next_run_step.saturating_add(1).max(1);

                                if attempt.saturating_add(1) < MAX_STEP_ATTEMPTS {
                                    set_progress(
                                        &job.progress,
                                        &job.run_id,
                                        &job.run_dir,
                                        format!(
                                            "Step {}/{} rejected; retrying ({}/{})…",
                                            step_index + 1,
                                            total,
                                            attempt + 1,
                                            MAX_STEP_ATTEMPTS
                                        ),
                                    );
                                    start_step_request(
                                        &mut job,
                                        &active,
                                        &library,
                                        &src_dir,
                                        step_index,
                                        attempt + 1,
                                        Some(StepRepairContext {
                                            error: rejection,
                                            previous_output: text.clone(),
                                        }),
                                    );
                                    runtime.in_flight = Some(job);
                                    return;
                                }

                                finish_with_error(
                                    &mut runtime,
                                    &mut ui,
                                    &job,
                                    format!(
                                        "Step {}/{} rejected by validators: {}",
                                        step_index + 1,
                                        total,
                                        rejection
                                    ),
                                );
                                return;
                            }

                            ui.set_status(format!(
                                "Step {}/{} OK (run_id={}): spawned={} updated={} despawned={}.",
                                step_index + 1,
                                total,
                                job.run_id,
                                spawned,
                                updated,
                                despawned
                            ));

                            let short_summary = truncate_text(summary.trim(), 160);
                            set_progress(
                                &job.progress,
                                &job.run_id,
                                &job.run_dir,
                                format!(
                                    "Step {}/{} done: {}",
                                    step_index + 1,
                                    total,
                                    if short_summary.is_empty() {
                                        "(no summary)".to_string()
                                    } else {
                                        short_summary
                                    }
                                ),
                            );

                            let run_step = job.next_run_step;
                            job.next_run_step = job.next_run_step.saturating_add(1).max(1);
                            job.phase = SceneBuildAiPhase::CaptureInit {
                                step_index,
                                run_step,
                            };
                            runtime.in_flight = Some(job);
                            return;
                        }
                        Err(err) => {
                            append_scene_build_run_log(
                                &job.run_dir,
                                format!(
                                    "step {} attempt {} apply_error: {}",
                                    step_index + 1,
                                    attempt,
                                    truncate_text(err.trim(), 400)
                                ),
                            );

                            if attempt.saturating_add(1) < MAX_STEP_ATTEMPTS {
                                set_progress(
                                    &job.progress,
                                    &job.run_id,
                                    &job.run_dir,
                                    format!(
                                        "Step {}/{} failed; retrying ({}/{})…",
                                        step_index + 1,
                                        total,
                                        attempt + 1,
                                        MAX_STEP_ATTEMPTS
                                    ),
                                );
                                start_step_request(
                                    &mut job,
                                    &active,
                                    &library,
                                    &src_dir,
                                    step_index,
                                    attempt + 1,
                                    Some(StepRepairContext {
                                        error: err,
                                        previous_output: text.clone(),
                                    }),
                                );
                                runtime.in_flight = Some(job);
                                return;
                            }

                            finish_with_error(&mut runtime, &mut ui, &job, err);
                        }
                    }
                }
            }
        }
        SceneBuildAiPhase::CaptureInit {
            step_index,
            run_step,
        } => {
            let total = job.plan_steps.len().max(1);
            let step_dir = job
                .run_dir
                .join("steps")
                .join(format!("{:04}", run_step.max(1)));

            let instances = scene_instances
                .iter()
                .map(|(_, t, _, prefab, _, _)| (t.clone(), *prefab));
            let (focus, half_extents) = scene_build_focus_and_half_extents(&library, instances);

            match start_scene_build_step_screenshot_capture(
                &mut commands,
                &mut images,
                &step_dir,
                focus,
                half_extents,
            ) {
                Ok(capture) => {
                    let (completed, expected) =
                        scene_build_step_screenshot_capture_progress(&capture);
                    set_progress(
                        &job.progress,
                        &job.run_id,
                        &job.run_dir,
                        format!(
                            "Capturing step {}/{} screenshots… ({}/{})",
                            step_index + 1,
                            total,
                            completed,
                            expected
                        ),
                    );
                    append_scene_build_run_log(
                        &job.run_dir,
                        format!(
                            "screenshots: started step {}/{} run_step={} dir={}",
                            step_index + 1,
                            total,
                            run_step,
                            step_dir.display()
                        ),
                    );

                    job.capture = Some(capture);
                    job.phase = SceneBuildAiPhase::CaptureWait { step_index };
                    runtime.in_flight = Some(job);
                }
                Err(err) => {
                    warn!(
                        "Scene build {}: screenshot capture start failed: {}",
                        job.run_id, err
                    );
                    append_scene_build_run_log(
                        &job.run_dir,
                        format!(
                            "screenshots: start failed step {}/{} run_step={} err={}",
                            step_index + 1,
                            total,
                            run_step,
                            truncate_text(err.trim(), 300)
                        ),
                    );

                    // Continue without screenshots.
                    let next = step_index + 1;
                    if next < job.plan_steps.len() {
                        start_step_request(&mut job, &active, &library, &src_dir, next, 0, None);
                        runtime.in_flight = Some(job);
                        return;
                    }

                    runtime.last_status = Some(SceneBuildAiStatus {
                        run_id: job.run_id.clone(),
                        message: format!("Done ({total} steps)."),
                    });
                    set_progress(
                        &job.progress,
                        &job.run_id,
                        &job.run_dir,
                        format!("Build complete ({total} steps)."),
                    );
                    ui.set_status(format!(
                        "Build complete (run_id={}, steps={total}).",
                        job.run_id
                    ));
                }
            }
        }
        SceneBuildAiPhase::CaptureWait { step_index } => {
            let total = job.plan_steps.len().max(1);
            let Some(capture) = job.capture.as_mut() else {
                warn!(
                    "Scene build {}: missing screenshot capture state (step {}/{})",
                    job.run_id,
                    step_index + 1,
                    total
                );

                let next = step_index + 1;
                if next < job.plan_steps.len() {
                    start_step_request(&mut job, &active, &library, &src_dir, next, 0, None);
                    runtime.in_flight = Some(job);
                    return;
                }

                runtime.last_status = Some(SceneBuildAiStatus {
                    run_id: job.run_id.clone(),
                    message: format!("Done ({total} steps)."),
                });
                set_progress(
                    &job.progress,
                    &job.run_id,
                    &job.run_dir,
                    format!("Build complete ({total} steps)."),
                );
                ui.set_status(format!(
                    "Build complete (run_id={}, steps={total}).",
                    job.run_id
                ));
                return;
            };

            let (completed, expected) = scene_build_step_screenshot_capture_progress(capture);
            if completed != capture.last_reported_completed {
                capture.last_reported_completed = completed;
                set_progress(
                    &job.progress,
                    &job.run_id,
                    &job.run_dir,
                    format!(
                        "Capturing step {}/{} screenshots… ({}/{})",
                        step_index + 1,
                        total,
                        completed,
                        expected
                    ),
                );
            }

            let done = scene_build_step_screenshot_capture_done(capture);
            let timed_out = !done && scene_build_step_screenshot_capture_timed_out(capture);
            if done || timed_out {
                if timed_out {
                    warn!(
                        "Scene build {}: screenshot capture timed out (step {}/{}, dir={})",
                        job.run_id,
                        step_index + 1,
                        total,
                        capture.step_dir.display()
                    );
                    append_scene_build_run_log(
                        &job.run_dir,
                        format!(
                            "screenshots: timed out step {}/{} dir={} completed={}/{}",
                            step_index + 1,
                            total,
                            capture.step_dir.display(),
                            completed,
                            expected
                        ),
                    );
                } else {
                    append_scene_build_run_log(
                        &job.run_dir,
                        format!(
                            "screenshots: done step {}/{} dir={} ({}/{})",
                            step_index + 1,
                            total,
                            capture.step_dir.display(),
                            completed,
                            expected
                        ),
                    );
                }

                cleanup_scene_build_step_screenshot_capture(&mut commands, capture);
                job.capture = None;

                let next = step_index + 1;
                if next < job.plan_steps.len() {
                    start_step_request(&mut job, &active, &library, &src_dir, next, 0, None);
                    runtime.in_flight = Some(job);
                    return;
                }

                runtime.last_status = Some(SceneBuildAiStatus {
                    run_id: job.run_id.clone(),
                    message: format!("Done ({total} steps)."),
                });
                set_progress(
                    &job.progress,
                    &job.run_id,
                    &job.run_dir,
                    format!("Build complete ({total} steps)."),
                );
                ui.set_status(format!(
                    "Build complete (run_id={}, steps={total}).",
                    job.run_id
                ));
                return;
            }

            runtime.in_flight = Some(job);
        }
    }
}

fn start_step_request(
    job: &mut SceneBuildAiJob,
    active: &ActiveRealmScene,
    library: &ObjectLibrary,
    src_dir: &Path,
    step_index: usize,
    attempt: u8,
    repair: Option<StepRepairContext>,
) {
    let total = job.plan_steps.len().max(1);
    let Some(step) = job.plan_steps.get(step_index) else {
        return;
    };

    let llm_dir = step_llm_dir(&job.run_dir, step_index, attempt);
    let _ = std::fs::create_dir_all(&llm_dir);

    let system_text = build_step_system_prompt();
    let mut user_text = build_step_user_prompt(
        active,
        library,
        src_dir,
        &job.description,
        step,
        step_index,
        total,
    );

    if let Some(repair) = repair {
        user_text.push_str(
            "

Previous attempt failed with this error:
",
        );
        user_text.push_str(repair.error.trim());
        user_text.push_str(
            "

Your previous output (for reference; may be truncated):
",
        );
        user_text.push_str(&truncate_text(repair.previous_output.trim(), 2400));
        user_text.push_str(
            "

Fix the JSON so it matches the schemas exactly and resolves the error. Return ONLY JSON.
",
        );
    }

    let _ = write_text_artifact(&llm_dir.join("system.txt"), &system_text);
    let _ = write_text_artifact(&llm_dir.join("user.txt"), &user_text);

    let display = if attempt == 0 {
        format!("Step {}/{}: {}", step_index + 1, total, step.title)
    } else {
        format!(
            "Step {}/{} (retry {}/{}): {}",
            step_index + 1,
            total,
            attempt,
            MAX_STEP_ATTEMPTS.saturating_sub(1),
            step.title
        )
    };

    set_progress(&job.progress, &job.run_id, &job.run_dir, display);
    append_scene_build_run_log(
        &job.run_dir,
        format!(
            "step_request step={} attempt={} llm_dir={}",
            step_index + 1,
            attempt,
            llm_dir.display()
        ),
    );

    job.shared_result = Arc::new(Mutex::new(None));
    let label = if attempt == 0 {
        format!("Step {}/{}", step_index + 1, total)
    } else {
        format!("Step {}/{} (retry {attempt})", step_index + 1, total)
    };
    spawn_scene_build_llm_thread(
        job.shared_result.clone(),
        job.progress.clone(),
        job.run_id.clone(),
        job.run_dir.clone(),
        job.openai_base_url.clone(),
        job.openai_api_key.clone(),
        job.openai_model.clone(),
        job.openai_reasoning_effort.clone(),
        system_text,
        user_text,
        llm_dir,
        label,
    );

    job.phase = SceneBuildAiPhase::StepRequest {
        step_index,
        attempt,
    };
}

fn step_llm_dir(run_dir: &Path, step_index: usize, attempt: u8) -> PathBuf {
    if attempt == 0 {
        return run_dir
            .join("llm")
            .join(format!("step_{:04}", step_index + 1));
    }

    run_dir
        .join("llm")
        .join(format!("step_{:04}_retry_{:02}", step_index + 1, attempt))
}

fn rejection_summary(result: &Value) -> String {
    let Some(report) = result.get("validation_report") else {
        return "Rejected by validators (missing validation_report).".to_string();
    };

    let hard_passed = report
        .get("hard_gates_passed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let violations = report
        .get("violations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut parts = Vec::new();
    parts.push(format!("hard_gates_passed={hard_passed}"));
    parts.push(format!("violations={}", violations.len()));

    for v in violations.iter().take(3) {
        let code = v.get("code").and_then(|v| v.as_str()).unwrap_or("unknown");
        let msg = v
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if msg.is_empty() {
            parts.push(code.to_string());
        } else {
            parts.push(format!("{code}: {msg}"));
        }
    }

    parts.join("; ")
}

fn take_shared_result(
    shared: &Arc<Mutex<Option<Result<String, String>>>>,
) -> Option<Result<String, String>> {
    let Ok(mut guard) = shared.lock() else {
        return None;
    };
    guard.take()
}

fn finish_with_error(
    runtime: &mut SceneBuildAiRuntime,
    ui: &mut SceneAuthoringUiState,
    job: &SceneBuildAiJob,
    err: String,
) {
    error!("Scene build {} failed: {}", job.run_id, err);
    ui.set_status(format!("Build failed (run_id={}).", job.run_id));
    ui.set_error(err.clone());
    set_progress(
        &job.progress,
        &job.run_id,
        &job.run_dir,
        format!("Failed: {}", truncate_text(err.trim(), 240)),
    );
    runtime.last_status = Some(SceneBuildAiStatus {
        run_id: job.run_id.clone(),
        message: format!("Failed: {}", truncate_text(err.trim(), 160)),
    });
}

fn default_scorecard() -> ScorecardSpecV1 {
    ScorecardSpecV1 {
        format_version: crate::scene_validation::SCORECARD_FORMAT_VERSION,
        scope: Default::default(),
        hard_gates: vec![
            HardGateSpecV1::Schema {},
            HardGateSpecV1::Budget {
                max_instances: Some(200_000),
                max_portals: Some(10_000),
            },
        ],
        soft_metrics: Vec::new(),
        weights: Default::default(),
    }
}

fn apply_counts(result: &Value) -> (bool, u64, u64, u64) {
    let applied = result
        .get("applied")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let compile = result.get("compile_report");
    let spawned = compile
        .and_then(|c| c.get("spawned"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let updated = compile
        .and_then(|c| c.get("updated"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let despawned = compile
        .and_then(|c| c.get("despawned"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    (applied, spawned, updated, despawned)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::with_capacity(max_chars + 16);
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn sanitize_step_id(raw: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '_' | '-' | ' ' | '/') {
            if !out.ends_with('_') {
                out.push('_');
            }
        }
    }
    let out = out.trim_matches('_').to_string();
    let out = if out.is_empty() {
        fallback.to_string()
    } else {
        out
    };
    out.chars().take(24).collect()
}

fn parse_plan_steps(raw_text: &str) -> Result<Vec<SceneBuildAiPlanStep>, String> {
    let doc = parse_json_object(raw_text)?;
    let steps_val = doc
        .get("steps")
        .ok_or_else(|| "Plan JSON missing `steps`".to_string())?;
    let steps = steps_val
        .as_array()
        .ok_or_else(|| "Plan `steps` must be an array".to_string())?;

    let mut out = Vec::new();
    for (idx, step_val) in steps.iter().enumerate() {
        let obj = step_val
            .as_object()
            .ok_or_else(|| format!("steps[{idx}] must be an object"))?;
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let goal = obj
            .get("goal")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let raw_id = obj
            .get("step_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let fallback = format!("step_{:02}", idx + 1);
        let step_id = sanitize_step_id(raw_id, &fallback);

        let title = if title.is_empty() {
            format!("Step {}", idx + 1)
        } else {
            title
        };

        out.push(SceneBuildAiPlanStep {
            step_id,
            title,
            goal,
        });
    }

    if out.len() > 12 {
        out.truncate(12);
    }

    Ok(out)
}

fn parse_step_ops(raw_text: &str) -> Result<(String, Vec<SceneSourcesPatchOpV1>), String> {
    let doc = parse_json_object(raw_text)?;

    let summary = doc
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let ops_val = doc
        .get("ops")
        .ok_or_else(|| "Step JSON missing `ops`".to_string())?
        .clone();

    let mut ops: Vec<SceneSourcesPatchOpV1> =
        serde_json::from_value(ops_val).map_err(|err| format!("Invalid ops array: {err}"))?;

    for op in ops.iter_mut() {
        match op {
            SceneSourcesPatchOpV1::UpsertLayer { layer_id, doc } => {
                if !layer_id.trim().starts_with("ai_") {
                    return Err(format!(
                        "upsert_layer.layer_id must start with ai_ (got {layer_id})"
                    ));
                }
                let obj = doc
                    .as_object_mut()
                    .ok_or_else(|| format!("Layer doc for {layer_id} must be an object"))?;
                obj.insert(
                    "format_version".to_string(),
                    Value::from(crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION),
                );
                obj.insert("layer_id".to_string(), Value::from(layer_id.clone()));
            }
            SceneSourcesPatchOpV1::DeleteLayer { layer_id } => {
                if !layer_id.trim().starts_with("ai_") {
                    return Err(format!(
                        "delete_layer.layer_id must start with ai_ (got {layer_id})"
                    ));
                }
            }
            other => {
                return Err(format!(
                    "Unsupported patch op kind in scene build: {other:?} (only upsert_layer/delete_layer allowed)"
                ));
            }
        }
    }

    Ok((summary, ops))
}

fn wrap_ops_as_patch(
    run_id: &str,
    step: &SceneBuildAiPlanStep,
    step_index: usize,
    ops: &[SceneSourcesPatchOpV1],
) -> Result<SceneSourcesPatchV1, String> {
    let request_id = format!("{}/llm_step_{:04}_{}", run_id, step_index + 1, step.step_id);

    Ok(SceneSourcesPatchV1 {
        format_version: SCENE_SOURCES_PATCH_FORMAT_VERSION,
        request_id,
        ops: ops.to_vec(),
    })
}

fn build_delete_ai_layers_patch(
    src_dir: &Path,
    run_id: &str,
) -> Result<SceneSourcesPatchV1, String> {
    let sources = SceneSourcesV1::load_from_dir(src_dir).map_err(|err| err.to_string())?;
    let layer_ids = existing_layer_ids_with_prefix(&sources, "ai_")?;

    let mut ops = Vec::new();
    for layer_id in layer_ids {
        ops.push(SceneSourcesPatchOpV1::DeleteLayer { layer_id });
    }

    Ok(SceneSourcesPatchV1 {
        format_version: SCENE_SOURCES_PATCH_FORMAT_VERSION,
        request_id: format!("{run_id}/cleanup"),
        ops,
    })
}

fn existing_layer_ids_with_prefix(
    sources: &SceneSourcesV1,
    prefix: &str,
) -> Result<Vec<String>, String> {
    let index_paths =
        crate::scene_sources::SceneSourcesIndexPaths::from_index_json_value(&sources.index_json)
            .map_err(|err| format!("Invalid scene sources index.json: {err}"))?;
    let layers_dir = index_paths.layers_dir;

    let mut out = Vec::new();
    for (rel_path, doc) in &sources.extra_json_files {
        let Ok(rel) = rel_path.strip_prefix(&layers_dir) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }

        let Some(layer_id) = doc.get("layer_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let layer_id = layer_id.trim();
        if layer_id.starts_with(prefix) {
            out.push(layer_id.to_string());
        }
    }

    out.sort();
    out.dedup();
    Ok(out)
}

fn build_plan_system_prompt() -> String {
    "You are a scene build planner for the game Gravimera.\n\
Return ONLY valid JSON.\n\
\n\
Output schema:\n\
{\n\
  \"steps\": [\n\
    { \"step_id\": \"snake_case\", \"title\": \"...\", \"goal\": \"...\" },\n\
    ...\n\
  ]\n\
}\n\
\n\
Rules:\n\
- Provide 3 to 8 steps.\n\
- step_id must be lowercase snake_case and short (<= 24 chars).\n\
- Steps should be ordered from coarse-to-fine so the scene becomes usable early and gains detail later.\n"
        .to_string()
}

fn build_plan_user_prompt(
    active: &ActiveRealmScene,
    library: &ObjectLibrary,
    description: &str,
) -> String {
    let catalog = build_prefab_catalog(library);
    format!(
        "Target scene: realm_id={}/ scene_id={}\n\n\
Scene description:\n\
{}\n\n\
Prefab catalog (prefab_id | kind | label | size):\n\
{}\n\n\
Now output JSON with `steps`.\n",
        active.realm_id, active.scene_id, description, catalog
    )
}

fn build_step_system_prompt() -> String {
    r#"You are a scene generation assistant for the game Gravimera.
Return ONLY valid JSON.

Output schema:
{
  "summary": "...",
  "ops": [ <scene_sources_patch_op>, ... ]
}

Where each `scene_sources_patch_op` is one of:
- { "kind": "upsert_layer", "layer_id": "ai_*", "doc": <layer_doc> }
- { "kind": "delete_layer", "layer_id": "ai_*" }

Rules:
- Only use the op kinds above.
- Every layer_id MUST start with "ai_".
- For upsert_layer, `doc` MUST be a valid layer JSON document.
- Use only prefab_id values from the provided catalog.
- Coordinate system: XZ is ground plane, Y is up. Keep objects near the origin.
- Placement: instance `transform.translation` is the CENTER of the object. To rest an object on the ground plane (y=0),
  set translation.y to half the object's scaled height: `translation.y = (prefab_size.y * abs(scale.y)) / 2.0`.
  Prefab sizes are included in the prefab catalog.
- Prefer using your step_id as a namespace in layer_id, e.g. ai_<step_id>_main.

Layer doc schemas (v1, minimal):

1) kind = "explicit_instances"
- Required: kind, instances[].local_id, instances[].prefab_id, instances[].transform
- instances[].transform has: translation (x,y,z), rotation quaternion (x,y,z,w), scale (x,y,z)

Example:
{
  "kind": "explicit_instances",
  "instances": [
    {
      "local_id": "ground_00",
      "prefab_id": "00000000-0000-0000-0000-000000000000",
      "transform": {
        "translation": { "x": 0.0, "y": 0.5, "z": 0.0 },
        "rotation": { "x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
      }
    }
  ]
}

2) kind = "grid_instances"
- Required: kind, prefab_id, origin, count{x,z}, step{x,z}

Example:
{
  "kind": "grid_instances",
  "prefab_id": "00000000-0000-0000-0000-000000000000",
  "origin": { "x": 0.0, "y": 0.5, "z": 0.0 },
  "count": { "x": 10, "z": 10 },
  "step": { "x": 1.0, "z": 1.0 },
  "rotation": { "x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0 },
  "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
}

3) kind = "polyline_instances"
- Required: kind, prefab_id, points[], spacing

Example:
{
  "kind": "polyline_instances",
  "prefab_id": "00000000-0000-0000-0000-000000000000",
  "points": [
    { "x": 0.0, "y": 0.5, "z": 0.0 },
    { "x": 10.0, "y": 0.5, "z": 0.0 }
  ],
  "spacing": 1.0,
  "start_offset": 0.0,
  "rotation": { "x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0 },
  "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
}

Do NOT invent alternate field names (for example: "position", "rotation_y", arrays for vectors, etc.)."#
        .to_string()
}

fn build_step_user_prompt(
    active: &ActiveRealmScene,
    library: &ObjectLibrary,
    src_dir: &Path,
    description: &str,
    step: &SceneBuildAiPlanStep,
    step_index: usize,
    total_steps: usize,
) -> String {
    let catalog = build_prefab_catalog(library);

    let existing_layers = SceneSourcesV1::load_from_dir(src_dir)
        .ok()
        .and_then(|sources| existing_layer_ids_with_prefix(&sources, "ai_").ok())
        .unwrap_or_default();
    let mut existing_lines = String::new();
    for id in existing_layers {
        existing_lines.push_str(&format!("- {id}\n"));
    }
    if existing_lines.trim().is_empty() {
        existing_lines = "<none>\n".to_string();
    }

    let step_goal = if step.goal.trim().is_empty() {
        "(no goal)".to_string()
    } else {
        step.goal.trim().to_string()
    };

    format!(
        "Target scene: realm_id={}/ scene_id={}\n\n\
Step {}/{}\n\
step_id: {}\n\
step_title: {}\n\
step_goal: {}\n\n\
Scene description:\n\
{}\n\n\
Existing ai_ layers:\n\
{}\n\
Prefab catalog (prefab_id | kind | label | size):\n\
{}\n\n\
Now output JSON with `summary` + `ops`.\n",
        active.realm_id,
        active.scene_id,
        step_index + 1,
        total_steps,
        step.step_id,
        step.title,
        step_goal,
        description,
        existing_lines,
        catalog
    )
}

fn build_prefab_catalog(library: &ObjectLibrary) -> String {
    let mut prefabs: Vec<(String, String, &'static str, Vec3)> = Vec::new();
    for (id, def) in library.iter() {
        let uuid = uuid::Uuid::from_u128(*id).to_string();
        let label = def.label.to_string();
        let kind = if def.mobility.is_some() {
            "unit"
        } else {
            "building"
        };
        prefabs.push((uuid, label, kind, def.size));
    }
    prefabs.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    let mut catalog = String::new();
    for (uuid, label, kind, size) in prefabs {
        catalog.push_str(&format!(
            "- {uuid} | {kind} | {label} | size=({:.3},{:.3},{:.3})\n",
            size.x, size.y, size.z
        ));
    }
    catalog
}

fn spawn_scene_build_llm_thread(
    shared: Arc<Mutex<Option<Result<String, String>>>>,
    progress: Arc<Mutex<SceneBuildAiProgress>>,
    run_id: String,
    run_dir: PathBuf,
    base_url: String,
    api_key: String,
    model: String,
    reasoning_effort: String,
    system_text: String,
    user_text: String,
    llm_dir: PathBuf,
    label: String,
) {
    let _ = std::thread::Builder::new()
        .name(format!("gravimera_scene_build_ai_{run_id}"))
        .spawn(move || {
            let res = call_openai_chat_json_object(
                &progress,
                &run_id,
                &run_dir,
                &label,
                &base_url,
                &api_key,
                &model,
                &reasoning_effort,
                &system_text,
                &user_text,
                &llm_dir,
            );
            if let Ok(mut guard) = shared.lock() {
                *guard = Some(res);
            }
        });
}

struct TempSecretFile {
    path: PathBuf,
}

impl TempSecretFile {
    fn create(prefix: &str, contents: &str) -> std::io::Result<Self> {
        use std::io::Write;

        let mut path = std::env::temp_dir();
        let pid = std::process::id();
        let nonce = uuid::Uuid::new_v4();
        path.push(format!("gravimera_{prefix}_{pid}_{nonce}.txt"));

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }

        file.write_all(contents.as_bytes())?;
        file.flush()?;

        Ok(Self { path })
    }

    fn curl_header_arg(&self) -> String {
        format!("@{}", self.path.display())
    }
}

impl Drop for TempSecretFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn curl_auth_header_file(api_key: &str) -> Result<TempSecretFile, String> {
    let api_key = api_key.replace(['\n', '\r'], "");
    let headers = format!("Authorization: Bearer {api_key}\n");
    TempSecretFile::create("openai_auth", &headers).map_err(|err| err.to_string())
}

fn split_curl_http_status<'a>(stdout: &'a str, marker: &str) -> (&'a str, Option<u16>) {
    let Some(pos) = stdout.rfind(marker) else {
        return (stdout, None);
    };
    let (body, rest) = stdout.split_at(pos);
    let code_str = rest[marker.len()..].lines().next().unwrap_or("").trim();
    (body, code_str.parse::<u16>().ok())
}

fn call_openai_chat_json_object(
    progress: &Arc<Mutex<SceneBuildAiProgress>>,
    run_id: &str,
    run_dir: &Path,
    label: &str,
    base_url: &str,
    api_key: &str,
    model: &str,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    llm_dir: &Path,
) -> Result<String, String> {
    let url = crate::config::join_base_url(base_url, "chat/completions");

    set_progress(
        progress,
        run_id,
        run_dir,
        format!("{label}: preparing request (model={model})…"),
    );

    let mut body_json = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": [
            { "role": "system", "content": system_instructions },
            { "role": "user", "content": user_text }
        ],
        "response_format": { "type": "json_object" }
    });
    if reasoning_effort.trim() != "none" && !reasoning_effort.trim().is_empty() {
        body_json["reasoning_effort"] = Value::from(reasoning_effort.trim());
    }

    let _ = write_json_artifact(&llm_dir.join("request.json"), &body_json);
    let body = serde_json::to_vec(&body_json).map_err(|err| err.to_string())?;

    let auth_headers = match curl_auth_header_file(api_key) {
        Ok(headers) => headers,
        Err(err) => {
            set_progress(progress, run_id, run_dir, format!("{label}: failed: {err}"));
            return Err(err);
        }
    };

    set_progress(
        progress,
        run_id,
        run_dir,
        format!("{label}: waiting for AI slot…"),
    );
    let _permit = crate::ai_limiter::acquire_permit();

    set_progress(
        progress,
        run_id,
        run_dir,
        format!("{label}: waiting for response…"),
    );
    let started = std::time::Instant::now();

    let mut cmd = std::process::Command::new("curl");
    cmd.arg("-sS")
        .arg("--connect-timeout")
        .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
        .arg("--max-time")
        .arg(CURL_MAX_TIME_SECS.to_string())
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(auth_headers.curl_header_arg())
        .arg("-d")
        .arg("@-")
        .arg(&url)
        .arg("-w")
        .arg("\n__GRAVIMERA_HTTP_STATUS__%{http_code}\n")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("Failed to start curl: {err}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin
            .write_all(&body)
            .map_err(|err| format!("Failed to write request to curl stdin: {err}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|err| format!("Failed to wait for curl: {err}"))?;

    let elapsed = started.elapsed().as_secs_f32();
    set_progress(
        progress,
        run_id,
        run_dir,
        format!("{label}: received response ({elapsed:.1}s). Parsing…"),
    );

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(format!(
            "curl exited with non-zero status: {}",
            truncate_text(stderr.trim(), 1200)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::write(llm_dir.join("api_response_raw.txt"), &stdout);

    const STATUS_MARKER: &str = "\n__GRAVIMERA_HTTP_STATUS__";
    let (body, status_code) = split_curl_http_status(&stdout, STATUS_MARKER);
    let status_code =
        status_code.ok_or_else(|| "Missing HTTP status marker in curl output.".to_string())?;

    if !(200..=299).contains(&status_code) {
        return Err(format!(
            "OpenAI request failed (HTTP {status_code}). Body (truncated): {}",
            truncate_text(body.trim(), 1200)
        ));
    }

    let json: Value = serde_json::from_str(body.trim()).map_err(|err| {
        format!(
            "Failed to parse OpenAI response JSON: {err}. Body (truncated): {}",
            truncate_text(body.trim(), 1200)
        )
    })?;
    let _ = write_json_artifact(&llm_dir.join("api_response.json"), &json);

    let text = json
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "OpenAI response missing choices[0].message.content".to_string())?
        .to_string();

    Ok(text)
}

fn write_text_artifact(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, format!("{text}\n"))
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn write_json_artifact(path: &Path, json: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(json)
        .map_err(|err| format!("json serialize failed: {err}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn parse_json_object(raw_text: &str) -> Result<Value, String> {
    let trimmed = raw_text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Ok(v);
    }
    if let Some(extracted) = extract_json_object(trimmed) {
        if let Ok(v) = serde_json::from_str::<Value>(&extracted) {
            return Ok(v);
        }
    }
    Err("Failed to parse LLM output as JSON.".to_string())
}

fn extract_json_object(text: &str) -> Option<String> {
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let mut last: Option<(usize, usize)> = None;

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth = depth.saturating_add(1);
            }
            '}' => {
                if depth > 0 {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if let Some(s) = start.take() {
                            last = Some((s, idx + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    last.map(|(s, e)| text[s..e].to_string())
}
