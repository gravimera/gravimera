use bevy::ecs::system::SystemParam;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseButtonInput, MouseWheel};
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::time::{TimeSystems, TimeUpdateStrategy, Virtual};
use bevy::window::{CursorMoved, PrimaryWindow};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Deserialize;

use crate::assets::SceneAssets;
use crate::config::AppConfig;
use crate::constants::*;
use crate::geometry::{clamp_world_xz, safe_abs_scale_y, snap_to_grid};
use crate::navigation;
use crate::object::registry::ObjectLibrary;
use crate::prefab_descriptors::PrefabDescriptorLibrary;
use crate::scene_store::SceneSaveRequest;
use crate::types::*;

pub(crate) struct AutomationPlugin;

impl Plugin for AutomationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutomationRuntime>();
        app.init_resource::<crate::scene_sources_runtime::SceneSourcesWorkspace>();
        app.add_systems(Startup, automation_startup);
        app.add_systems(
            PreUpdate,
            (
                automation_mask_local_input_messages.after(InputSystems),
                automation_suppress_local_input_state
                    .after(InputSystems)
                    .after(automation_mask_local_input_messages),
            ),
        );
        app.add_systems(Update, automation_process_requests);
        app.add_systems(First, automation_time_control.before(TimeSystems));
        app.add_systems(First, automation_step_tick.after(TimeSystems));
    }
}

#[derive(Clone)]
struct AutomationRequest {
    method: String,
    path: String,
    body: Vec<u8>,
    reply: std::sync::mpsc::Sender<AutomationReply>,
}

#[derive(Clone)]
struct AutomationReply {
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
}

struct AutomationStepJob {
    remaining_frames: u32,
    dt: Duration,
    reply: std::sync::mpsc::Sender<AutomationReply>,
}

#[derive(Resource, Default)]
pub(crate) struct AutomationRuntime {
    enabled: bool,
    disable_local_input: bool,
    pause_on_start: bool,
    token: Option<String>,
    listen_addr: Option<String>,
    inbox: Option<Arc<Mutex<std::sync::mpsc::Receiver<AutomationRequest>>>>,
    stop_flag: Option<Arc<AtomicBool>>,
    server_thread: Arc<Mutex<Option<JoinHandle<()>>>>,
    time_paused: bool,
    active_step: Option<AutomationStepJob>,
    step_queue: VecDeque<AutomationStepJob>,
}

impl Drop for AutomationRuntime {
    fn drop(&mut self) {
        if let Some(flag) = self.stop_flag.take() {
            flag.store(true, Ordering::Relaxed);
        }
        if let Ok(mut guard) = self.server_thread.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}

pub(crate) fn local_input_enabled(config: Res<AppConfig>) -> bool {
    let _ = config;
    // NOTE: Input masking is handled by Automation in `PreUpdate` when
    // `automation.enabled && automation.disable_local_input`.
    // Input-driven systems can keep running; when masking is enabled they observe cleared
    // keyboard/mouse state, while the Automation HTTP API drives gameplay via semantic endpoints
    // (select/move/fire/mode/etc), not raw input injection.
    true
}

fn automation_startup(mut runtime: ResMut<AutomationRuntime>, config: Res<AppConfig>) {
    runtime.enabled = config.automation_enabled;
    runtime.disable_local_input = config.automation_disable_local_input;
    runtime.pause_on_start = config.automation_pause_on_start;
    runtime.token = config.automation_token.clone();
    runtime.time_paused = config.automation_pause_on_start;

    if !config.automation_enabled {
        return;
    }

    let bind = config
        .automation_bind
        .clone()
        .unwrap_or_else(|| "127.0.0.1:8791".to_string());

    let (tx, rx) = std::sync::mpsc::channel::<AutomationRequest>();
    runtime.inbox = Some(Arc::new(Mutex::new(rx)));

    let stop_flag = Arc::new(AtomicBool::new(false));
    runtime.stop_flag = Some(stop_flag.clone());

    let token = config.automation_token.clone();

    let server = match tiny_http::Server::http(&bind) {
        Ok(server) => server,
        Err(err) => {
            error!("Automation API: failed to bind {bind}: {err}");
            runtime.enabled = false;
            runtime.inbox = None;
            runtime.stop_flag = None;
            return;
        }
    };

    let listen = server.server_addr().to_string();
    runtime.listen_addr = Some(format!("http://{listen}"));
    info!("Automation API listening on http://{listen}");

    let thread = std::thread::Builder::new()
        .name("gravimera_automation_http".into())
        .spawn(move || server_loop(server, tx, token, stop_flag))
        .ok();

    if let Some(thread) = thread {
        if let Ok(mut guard) = runtime.server_thread.lock() {
            *guard = Some(thread);
        }
    } else {
        error!("Automation API: failed to spawn server thread.");
        runtime.enabled = false;
        runtime.inbox = None;
        runtime.stop_flag = None;
    }
}

fn automation_mask_local_input_messages(
    config: Res<AppConfig>,
    runtime: Res<AutomationRuntime>,
    mut keyboard: Option<MessageReader<KeyboardInput>>,
    mut mouse_buttons: Option<MessageReader<MouseButtonInput>>,
    mut mouse_wheel: Option<MessageReader<MouseWheel>>,
    mut cursor_moved: Option<MessageReader<CursorMoved>>,
    mut drag_and_drop: Option<MessageReader<bevy::window::FileDragAndDrop>>,
) {
    if !config.automation_enabled || !runtime.enabled || !runtime.disable_local_input {
        return;
    }

    // Drain OS input messages so no other system can observe them.
    if let Some(keyboard) = keyboard.as_mut() {
        for _ in keyboard.read() {}
    }
    if let Some(mouse_buttons) = mouse_buttons.as_mut() {
        for _ in mouse_buttons.read() {}
    }
    if let Some(mouse_wheel) = mouse_wheel.as_mut() {
        for _ in mouse_wheel.read() {}
    }
    if let Some(cursor_moved) = cursor_moved.as_mut() {
        for _ in cursor_moved.read() {}
    }
    if let Some(drag_and_drop) = drag_and_drop.as_mut() {
        for _ in drag_and_drop.read() {}
    }
}

fn automation_suppress_local_input_state(
    config: Res<AppConfig>,
    mut keys: Option<ResMut<ButtonInput<KeyCode>>>,
    mut mouse_buttons: Option<ResMut<ButtonInput<MouseButton>>>,
    runtime: Res<AutomationRuntime>,
) {
    if !config.automation_enabled || !runtime.enabled || !runtime.disable_local_input {
        return;
    }

    if let Some(keys) = keys.as_deref_mut() {
        keys.clear();
        let pressed_now: Vec<KeyCode> = keys.get_pressed().copied().collect();
        for key in pressed_now {
            keys.release(key);
            let _ = keys.clear_just_released(key);
        }
    }
    if let Some(mouse_buttons) = mouse_buttons.as_deref_mut() {
        mouse_buttons.clear();
        let pressed_buttons: Vec<MouseButton> = mouse_buttons.get_pressed().copied().collect();
        for button in pressed_buttons {
            mouse_buttons.release(button);
            let _ = mouse_buttons.clear_just_released(button);
        }
    }
}

fn server_loop(
    server: tiny_http::Server,
    tx: std::sync::mpsc::Sender<AutomationRequest>,
    token: Option<String>,
    stop: Arc<AtomicBool>,
) {
    let timeout = Duration::from_millis(200);
    let request_id = AtomicU64::new(1);

    while !stop.load(Ordering::Relaxed) {
        let request = match server.recv_timeout(timeout) {
            Ok(Some(req)) => req,
            Ok(None) => continue,
            Err(err) => {
                warn!("Automation API: server recv error: {err}");
                continue;
            }
        };

        let id = request_id.fetch_add(1, Ordering::Relaxed);
        handle_http_request(id, request, &tx, token.as_deref());
    }
}

fn handle_http_request(
    _id: u64,
    mut request: tiny_http::Request,
    tx: &std::sync::mpsc::Sender<AutomationRequest>,
    token: Option<&str>,
) {
    let method = request.method().as_str().to_string();
    let path_full = request.url().to_string();
    let path = path_full
        .split_once('?')
        .map(|(p, _q)| p.to_string())
        .unwrap_or(path_full);

    if let Some(token) = token {
        let auth_header = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Authorization"))
            .map(|h| h.value.as_str().to_string());
        let expected = format!("Bearer {token}");
        if auth_header.as_deref() != Some(expected.as_str()) {
            let body = serde_json::json!({
                "ok": false,
                "error": "Unauthorized",
            })
            .to_string();
            respond_json(request, 401, body);
            return;
        }
    }

    let mut body = Vec::new();
    if request.body_length().unwrap_or(0) > 0 {
        if let Err(err) = request.as_reader().read_to_end(&mut body) {
            let body = serde_json::json!({
                "ok": false,
                "error": format!("Failed to read request body: {err}"),
            })
            .to_string();
            respond_json(request, 400, body);
            return;
        }
    } else {
        let _ = request.as_reader().read_to_end(&mut body);
    }

    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<AutomationReply>();
    let msg = AutomationRequest {
        method,
        path,
        body,
        reply: reply_tx,
    };

    if tx.send(msg).is_err() {
        let body = serde_json::json!({
            "ok": false,
            "error": "Automation inbox disconnected",
        })
        .to_string();
        respond_json(request, 500, body);
        return;
    }

    // Some semantic endpoints can block longer than a typical HTTP request, e.g. `/v1/step`
    // (advancing many frames) or `/v1/gen3d/save` (serializing large drafts / scene persistence).
    // Keep this generous so automation drivers can remain simple and robust.
    match reply_rx.recv_timeout(Duration::from_secs(600)) {
        Ok(reply) => {
            let mut response = tiny_http::Response::from_data(reply.body);
            response = response.with_status_code(tiny_http::StatusCode(reply.status));
            if let Ok(header) =
                tiny_http::Header::from_bytes("Content-Type", reply.content_type.as_bytes())
            {
                response = response.with_header(header);
            }
            let _ = request.respond(response);
        }
        Err(_) => {
            let body = serde_json::json!({
                "ok": false,
                "error": "Automation request timed out",
            })
            .to_string();
            respond_json(request, 504, body);
        }
    }
}

fn respond_json(request: tiny_http::Request, status: u16, body_json: String) {
    let mut response = tiny_http::Response::from_string(body_json);
    response = response.with_status_code(tiny_http::StatusCode(status));
    if let Ok(header) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
        response = response.with_header(header);
    }
    let _ = request.respond(response);
}

#[derive(SystemParam)]
struct AutomationWorld<'w, 's> {
    windows: Query<'w, 's, (Entity, &'static Window), With<PrimaryWindow>>,
    players: Query<'w, 's, (), With<Player>>,
    player_q: Query<'w, 's, (&'static Transform, &'static Collider), With<Player>>,
    commandables: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static Collider,
            &'static ObjectPrefabId,
        ),
        With<Commandable>,
    >,
    enemies: Query<'w, 's, (Entity, &'static ObjectId), With<Enemy>>,
    build_objects: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static AabbCollider,
            &'static BuildDimensions,
            &'static ObjectPrefabId,
        ),
        With<BuildObject>,
    >,
    selectable_entities: Query<
        'w,
        's,
        (Entity, &'static ObjectId),
        (
            Or<(With<Commandable>, With<BuildObject>, With<Enemy>)>,
            Without<Bullet>,
        ),
    >,
    state_objects: Query<
        'w,
        's,
        (
            Entity,
            &'static ObjectId,
            &'static ObjectPrefabId,
            &'static Transform,
            Option<&'static Player>,
            Option<&'static Enemy>,
            Option<&'static BuildObject>,
            Option<&'static Commandable>,
        ),
        Or<(With<Commandable>, With<BuildObject>, With<Enemy>)>,
    >,
    world_objects: Query<
        'w,
        's,
        (
            Entity,
            &'static ObjectId,
            &'static Transform,
            &'static ObjectPrefabId,
            Option<&'static ObjectTint>,
        ),
        (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>),
    >,
    children_q: Query<'w, 's, &'static Children>,
    scene_instances: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static ObjectId,
            &'static ObjectPrefabId,
            Option<&'static ObjectTint>,
            Option<&'static ObjectForms>,
            Option<&'static SceneLayerOwner>,
        ),
        (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>),
    >,
}

#[derive(SystemParam)]
struct AutomationGen3d<'w> {
    log_sinks: Option<Res<'w, crate::app::Gen3dLogSinks>>,
    workshop: Option<ResMut<'w, crate::gen3d::Gen3dWorkshop>>,
    job: Option<ResMut<'w, crate::gen3d::Gen3dAiJob>>,
    draft: Option<ResMut<'w, crate::gen3d::Gen3dDraft>>,
    pending_seed: Option<ResMut<'w, crate::gen3d::Gen3dPendingSeedFromPrefab>>,
    asset_server: Option<Res<'w, AssetServer>>,
    assets: Option<Res<'w, SceneAssets>>,
    images: Option<ResMut<'w, Assets<Image>>>,
    meshes: Option<ResMut<'w, Assets<Mesh>>>,
    materials: Option<ResMut<'w, Assets<StandardMaterial>>>,
    material_cache: Option<ResMut<'w, crate::object::visuals::MaterialCache>>,
    mesh_cache: Option<ResMut<'w, crate::object::visuals::PrimitiveMeshCache>>,
    prefab_thumbnail_capture: Option<ResMut<'w, crate::gen3d::Gen3dPrefabThumbnailCaptureRuntime>>,
    scene_saves: MessageWriter<'w, SceneSaveRequest>,
}

struct AutomationContext<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w> {
    commands: &'a mut Commands<'cmd_w, 'cmd_s>,
    config: &'a AppConfig,
    active_realm_id: &'a str,
    active_scene_id: &'a str,
    library: &'a mut ObjectLibrary,
    prefab_descriptors: &'a mut PrefabDescriptorLibrary,
    gen3d: &'a mut AutomationGen3d<'gen3d_w>,
    world: &'a AutomationWorld<'world_w, 'world_s>,
    mode: Option<&'a State<GameMode>>,
    next_mode: Option<&'a mut NextState<GameMode>>,
    build_scene: Option<&'a State<BuildScene>>,
    next_build_scene: Option<&'a mut NextState<BuildScene>>,
    selection: Option<&'a mut SelectionState>,
    fire: Option<&'a mut FireControl>,
    motion_ui: Option<&'a mut crate::motion_ui::MotionAlgorithmUiState>,
    runtime: &'a mut AutomationRuntime,
    scene_workspace: &'a mut crate::scene_sources_runtime::SceneSourcesWorkspace,
    scene_build_runtime: Option<&'a mut crate::scene_build_ai::SceneBuildAiRuntime>,
    exit: &'a mut MessageWriter<'exit_w, AppExit>,
}

#[derive(SystemParam)]
struct AutomationUi<'w> {
    selection: Option<ResMut<'w, SelectionState>>,
    fire: Option<ResMut<'w, FireControl>>,
    motion_ui: Option<ResMut<'w, crate::motion_ui::MotionAlgorithmUiState>>,
}

fn automation_process_requests(
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    config: Res<AppConfig>,
    active: Option<Res<crate::realm::ActiveRealmScene>>,
    mut runtime: ResMut<AutomationRuntime>,
    mut scene_workspace: ResMut<crate::scene_sources_runtime::SceneSourcesWorkspace>,
    mut scene_build_runtime: Option<ResMut<crate::scene_build_ai::SceneBuildAiRuntime>>,
    mode: Option<Res<State<GameMode>>>,
    mut next_mode: Option<ResMut<NextState<GameMode>>>,
    build_scene: Option<Res<State<BuildScene>>>,
    mut next_build_scene: Option<ResMut<NextState<BuildScene>>>,
    mut ui: AutomationUi,
    mut library: ResMut<ObjectLibrary>,
    mut prefab_descriptors: ResMut<PrefabDescriptorLibrary>,
    mut gen3d: AutomationGen3d,
    world: AutomationWorld,
) {
    if !runtime.enabled || !config.automation_enabled {
        return;
    }
    let Some(inbox) = runtime.inbox.clone() else {
        return;
    };

    // IMPORTANT: Process a limited number of HTTP requests per frame.
    //
    // Many endpoints use `Commands` (deferred ECS writes). If we process multiple requests in the
    // same frame, later requests may not observe entities/components created by earlier requests
    // until the end-of-frame `ApplyDeferred` runs. This leads to surprising API behavior (e.g.
    // `/v1/gen3d/save` spawns an entity via `Commands`, but an immediate `/v1/select` in the same
    // frame won't find it).
    //
    // One request per frame keeps automation deterministic and makes the API act more like a
    // transactional boundary: after the reply is sent, deferred writes will be applied before the
    // next request is processed.
    const MAX_REQUESTS_PER_FRAME: usize = 1;

    for _ in 0..MAX_REQUESTS_PER_FRAME {
        let msg = {
            let Ok(guard) = inbox.lock() else {
                return;
            };
            match guard.try_recv() {
                Ok(msg) => msg,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    runtime.enabled = false;
                    break;
                }
            }
        };

        let active_realm_id: &str = match active.as_ref() {
            Some(active) => active.realm_id.as_str(),
            None => crate::paths::default_realm_id(),
        };
        let active_scene_id: &str = match active.as_ref() {
            Some(active) => active.scene_id.as_str(),
            None => crate::paths::default_scene_id(),
        };

        let mut ctx = AutomationContext {
            commands: &mut commands,
            config: &config,
            active_realm_id,
            active_scene_id,
            library: &mut library,
            prefab_descriptors: &mut prefab_descriptors,
            gen3d: &mut gen3d,
            world: &world,
            mode: mode.as_deref(),
            next_mode: next_mode.as_deref_mut(),
            build_scene: build_scene.as_deref(),
            next_build_scene: next_build_scene.as_deref_mut(),
            selection: ui.selection.as_deref_mut(),
            fire: ui.fire.as_deref_mut(),
            motion_ui: ui.motion_ui.as_deref_mut(),
            runtime: &mut runtime,
            scene_workspace: &mut scene_workspace,
            scene_build_runtime: scene_build_runtime.as_deref_mut(),
            exit: &mut exit,
        };

        let reply = handle_request_main_thread(&mut ctx, &msg);
        if let Some(reply) = reply {
            let _ = msg.reply.send(reply);
        } else {
            // Deferred reply endpoint (currently `/v1/step`). Stop here to preserve ordering.
            break;
        }
    }
}

#[derive(Deserialize)]
struct SelectRequest {
    instance_ids: Vec<String>,
}

#[derive(Deserialize)]
struct MoveRequest {
    x: f32,
    z: f32,
    #[serde(default)]
    y: Option<f32>,
}

#[derive(Deserialize)]
struct SpawnRequest {
    prefab_id_uuid: String,
    #[serde(default)]
    x: Option<f32>,
    #[serde(default)]
    y: Option<f32>,
    #[serde(default)]
    z: Option<f32>,
    #[serde(default)]
    yaw: Option<f32>,
}

#[derive(Deserialize)]
struct ModeRequest {
    mode: String,
}

#[derive(Deserialize)]
struct FireRequest {
    active: bool,
    #[serde(default)]
    target: Option<FireTargetRequest>,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum FireTargetRequest {
    #[serde(rename = "point")]
    Point { x: f32, z: f32 },
    #[serde(rename = "enemy")]
    Enemy { instance_id: String },
}

#[derive(Deserialize)]
struct StepRequest {
    frames: u32,
    #[serde(default)]
    dt_secs: Option<f32>,
}

#[derive(Deserialize)]
struct ScreenshotRequest {
    path: String,
    #[serde(default)]
    include_ui: Option<bool>,
}

#[derive(Deserialize)]
struct MetaPanelRequest {
    #[serde(default)]
    instance_id_uuid: Option<String>,
}

#[derive(Deserialize)]
struct MetaGen3dActionRequest {
    #[serde(default)]
    instance_id_uuid: Option<String>,
}

#[derive(Deserialize)]
struct ForceAnimationChannelRequest {
    instance_ids: Vec<String>,
    channel: String,
}

#[derive(Deserialize)]
struct Gen3dPromptRequest {
    prompt: String,
}

#[derive(Deserialize)]
struct Gen3dSeedFromPrefabRequest {
    prefab_id_uuid: String,
}

#[derive(Deserialize)]
struct SceneBuildStartRequest {
    description: String,
}

#[derive(Deserialize)]
struct SceneBuildStopRequest {}

#[derive(Deserialize)]
struct SceneSourcesImportRequest {
    src_dir: String,
}

#[derive(Deserialize)]
struct SceneSourcesExportRequest {
    out_dir: String,
}

#[derive(Deserialize)]
struct SceneSourcesReloadRequest {}

#[derive(Deserialize)]
struct SceneSourcesCompileRequest {}

#[derive(Deserialize)]
struct SceneSourcesRegenerateLayerRequest {
    layer_id: String,
}

#[derive(Deserialize)]
struct SceneSourcesPatchRequest {
    scorecard: crate::scene_validation::ScorecardSpecV1,
    patch: crate::scene_sources_patch::SceneSourcesPatchV1,
}

#[derive(Deserialize)]
struct SceneRunStatusRequest {
    run_id: String,
}

#[derive(Deserialize)]
struct SceneRunApplyPatchRequest {
    run_id: String,
    step: u32,
    scorecard: crate::scene_validation::ScorecardSpecV1,
    patch: crate::scene_sources_patch::SceneSourcesPatchV1,
}

fn handle_scene_sources_routes<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>(
    ctx: &mut AutomationContext<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>,
    msg: &AutomationRequest,
) -> Option<AutomationReply> {
    let commands = &mut *ctx.commands;
    let scene_workspace = &mut *ctx.scene_workspace;
    let library = &mut *ctx.library;
    let scene_instances = &ctx.world.scene_instances;

    match (msg.method.as_str(), msg.path.as_str()) {
        ("POST", "/v1/scene_sources/reload") => {
            let _req: SceneSourcesReloadRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            if let Err(err) =
                crate::scene_sources_runtime::reload_scene_sources_in_workspace(scene_workspace)
            {
                return Some(json_error(409, err));
            }

            Some(AutomationReply {
                status: 200,
                body: serde_json::json!({ "ok": true }).to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/compile") => {
            let _req: SceneSourcesCompileRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let existing = scene_instances
                .iter()
                .map(|(e, t, id, prefab, tint, _forms, owner)| {
                    crate::scene_sources_runtime::SceneWorldInstance {
                        entity: e,
                        instance_id: *id,
                        prefab_id: *prefab,
                        transform: t.clone(),
                        tint: tint.map(|t| t.0),
                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                    }
                });

            let report = match crate::scene_sources_runtime::compile_scene_sources_all_layers(
                commands,
                scene_workspace,
                library,
                existing,
            ) {
                Ok(v) => v,
                Err(err) => return Some(json_error(409, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "spawned": report.spawned,
                "updated": report.updated,
                "despawned": report.despawned,
                "layers_compiled": report.layers_compiled,
                "pinned_upserts": report.pinned_upserts,
            })
            .to_string();

            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/regenerate_layer") => {
            let req: SceneSourcesRegenerateLayerRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let existing = scene_instances
                .iter()
                .map(|(e, t, id, prefab, tint, _forms, owner)| {
                    crate::scene_sources_runtime::SceneWorldInstance {
                        entity: e,
                        instance_id: *id,
                        prefab_id: *prefab,
                        transform: t.clone(),
                        tint: tint.map(|t| t.0),
                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                    }
                });

            let report = match crate::scene_sources_runtime::regenerate_scene_layer(
                commands,
                scene_workspace,
                library,
                existing,
                &req.layer_id,
            ) {
                Ok(v) => v,
                Err(err) => return Some(json_error(409, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "layer_id": req.layer_id,
                "spawned": report.spawned,
                "updated": report.updated,
                "despawned": report.despawned,
            })
            .to_string();

            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("GET", "/v1/scene_sources/signature") => {
            let existing = scene_instances
                .iter()
                .map(|(e, t, id, prefab, tint, _forms, owner)| {
                    crate::scene_sources_runtime::SceneWorldInstance {
                        entity: e,
                        instance_id: *id,
                        prefab_id: *prefab,
                        transform: t.clone(),
                        tint: tint.map(|t| t.0),
                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                    }
                });

            let summary = match crate::scene_sources_runtime::scene_signature_summary(existing) {
                Ok(v) => v,
                Err(err) => return Some(json_error(500, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "overall_sig": summary.overall_sig,
                "pinned_sig": summary.pinned_sig,
                "layer_sigs": summary.layer_sigs,
                "total_instances": summary.total_instances,
                "pinned_instances": summary.pinned_instances,
                "layer_instance_counts": summary.layer_instance_counts,
            })
            .to_string();

            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/validate") => {
            let scorecard: crate::scene_validation::ScorecardSpecV1 =
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                };

            let existing = scene_instances
                .iter()
                .map(|(e, t, id, prefab, tint, _forms, owner)| {
                    crate::scene_sources_runtime::SceneWorldInstance {
                        entity: e,
                        instance_id: *id,
                        prefab_id: *prefab,
                        transform: t.clone(),
                        tint: tint.map(|t| t.0),
                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                    }
                });

            let report = match crate::scene_sources_runtime::validate_scene_sources(
                scene_workspace,
                library,
                existing,
                &scorecard,
            ) {
                Ok(v) => v,
                Err(err) => return Some(json_error(409, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "report": report,
            })
            .to_string();

            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/patch_validate") => {
            let req: SceneSourcesPatchRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let result = match crate::scene_sources_runtime::validate_scene_sources_patch(
                scene_workspace,
                library,
                &req.scorecard,
                &req.patch,
            ) {
                Ok(v) => v,
                Err(err) => return Some(json_error(409, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "patch_summary": result.patch_summary,
                "validation_report": result.validation_report,
            })
            .to_string();

            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/patch_apply") => {
            let req: SceneSourcesPatchRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let existing = scene_instances
                .iter()
                .map(|(e, t, id, prefab, tint, _forms, owner)| {
                    crate::scene_sources_runtime::SceneWorldInstance {
                        entity: e,
                        instance_id: *id,
                        prefab_id: *prefab,
                        transform: t.clone(),
                        tint: tint.map(|t| t.0),
                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                    }
                });

            let result = match crate::scene_sources_runtime::apply_scene_sources_patch(
                commands,
                scene_workspace,
                library,
                existing,
                &req.scorecard,
                &req.patch,
            ) {
                Ok(v) => v,
                Err(err) => return Some(json_error(409, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "applied": result.applied,
                "patch_summary": result.patch_summary,
                "compile_report": result.compile_report,
                "validation_report": result.validation_report,
            })
            .to_string();

            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/run_status") => {
            let req: SceneRunStatusRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let status = match crate::scene_runs::scene_run_status(scene_workspace, &req.run_id) {
                Ok(v) => v,
                Err(err) => return Some(json_error(409, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "status": status,
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/run_apply_patch") => {
            let req: SceneRunApplyPatchRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let existing = scene_instances
                .iter()
                .map(|(e, t, id, prefab, tint, _forms, owner)| {
                    crate::scene_sources_runtime::SceneWorldInstance {
                        entity: e,
                        instance_id: *id,
                        prefab_id: *prefab,
                        transform: t.clone(),
                        tint: tint.map(|t| t.0),
                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                    }
                });

            let response = match crate::scene_runs::scene_run_apply_patch_step(
                commands,
                scene_workspace,
                library,
                existing,
                &req.run_id,
                req.step,
                &req.scorecard,
                &req.patch,
            ) {
                Ok(v) => v,
                Err(err) => return Some(json_error(409, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "run_id": response.run_id,
                "step": response.step,
                "mode": response.mode,
                "result": response.result,
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/import") => {
            let req: SceneSourcesImportRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let src_dir = PathBuf::from(req.src_dir.trim());
            let existing_entities = scene_instances.iter().map(|(e, _, _, _, _, _, _)| e);

            let report = match crate::scene_sources_runtime::import_scene_sources_replace_world(
                commands,
                scene_workspace,
                library,
                &src_dir,
                existing_entities,
            ) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "imported_instances": report.instance_count,
                "src_dir": src_dir.display().to_string(),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_sources/export") => {
            let req: SceneSourcesExportRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let out_dir = PathBuf::from(req.out_dir.trim());
            let objects =
                scene_instances
                    .iter()
                    .filter_map(|(_e, t, id, prefab, tint, forms, owner)| {
                        owner.is_none().then_some((t, id, prefab, tint, forms))
                    });

            let report = match crate::scene_sources_runtime::export_scene_sources_from_world(
                scene_workspace,
                objects,
                &out_dir,
            ) {
                Ok(v) => v,
                Err(err) => return Some(json_error(409, err)),
            };

            let body = serde_json::json!({
                "ok": true,
                "exported_instances": report.instance_count,
                "out_dir": out_dir.display().to_string(),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        _ => Some(json_error(404, "Not found")),
    }
}

fn handle_gen3d_routes<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>(
    ctx: &mut AutomationContext<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>,
    msg: &AutomationRequest,
) -> Option<AutomationReply> {
    let commands = &mut *ctx.commands;
    let config = ctx.config;
    let active_realm_id = ctx.active_realm_id;
    let active_scene_id = ctx.active_scene_id;
    let log_sinks = ctx.gen3d.log_sinks.as_deref();
    let library = &mut *ctx.library;
    let prefab_descriptors = &mut *ctx.prefab_descriptors;
    let mut gen3d_workshop = ctx.gen3d.workshop.as_deref_mut();
    let mut gen3d_job = ctx.gen3d.job.as_deref_mut();
    let mut gen3d_draft = ctx.gen3d.draft.as_deref_mut();
    let asset_server = ctx.gen3d.asset_server.as_deref();
    let assets = ctx.gen3d.assets.as_deref();
    let images = ctx.gen3d.images.as_deref_mut();
    let meshes = ctx.gen3d.meshes.as_deref_mut();
    let materials = ctx.gen3d.materials.as_deref_mut();
    let material_cache = ctx.gen3d.material_cache.as_deref_mut();
    let mesh_cache = ctx.gen3d.mesh_cache.as_deref_mut();
    let prefab_thumbnail_capture = ctx.gen3d.prefab_thumbnail_capture.as_deref_mut();
    let scene_saves = &mut ctx.gen3d.scene_saves;

    let player_q = &ctx.world.player_q;
    let world_objects = &ctx.world.world_objects;
    let children_q = &ctx.world.children_q;
    let mode = ctx.mode;
    let build_scene = ctx.build_scene;

    match (msg.method.as_str(), msg.path.as_str()) {
        ("GET", "/v1/gen3d/status") => {
            let Some(workshop) = gen3d_workshop.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(job) = gen3d_job.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let draft_ready = gen3d_draft
                .as_deref()
                .map(|d| d.root_def().is_some() && d.total_non_projectile_primitive_parts() > 0)
                .unwrap_or(false);

            let body = serde_json::json!({
                "ok": true,
                "running": job.is_running(),
                "build_complete": job.is_build_complete(),
                "can_resume": job.can_resume(),
                "edit_base_prefab_id": job.edit_base_prefab_id().map(|id| uuid::Uuid::from_u128(id).to_string()),
                "save_overwrite_prefab_id": job.save_overwrite_prefab_id().map(|id| uuid::Uuid::from_u128(id).to_string()),
                "draft_ready": draft_ready,
                "run_id": job.run_id().map(|id| id.to_string()),
                "attempt": job.attempt(),
                "pass": job.pass(),
                "status": workshop.status.clone(),
                "error": workshop.error.clone(),
                "run_dir": job.run_dir_path().map(|p| p.display().to_string()),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/prompt") => {
            let Some(workshop) = gen3d_workshop.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let req: Gen3dPromptRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            if let Err(err) = crate::gen3d::validate_gen3d_user_prompt_limits(&req.prompt) {
                return Some(json_error(400, err));
            }
            workshop.prompt = req.prompt;
            Some(AutomationReply {
                status: 200,
                body: serde_json::json!({ "ok": true }).to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/build") => {
            let Some(mode) = mode else {
                return Some(json_error(501, "Gen3D build requires rendered mode."));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(501, "Gen3D build requires rendered mode."));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(workshop) = gen3d_workshop.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            if job.is_running() {
                return Some(json_error(409, "Gen3D build is already running."));
            }

            let sinks = log_sinks.cloned();
            if let Err(err) = crate::gen3d::gen3d_start_build_from_api(
                build_scene,
                ctx.active_realm_id,
                ctx.active_scene_id,
                config,
                sinks,
                workshop,
                job,
                draft,
            ) {
                return Some(json_error(400, err));
            }

            let body = serde_json::json!({
                "ok": true,
                "run_id": job.run_id().map(|id| id.to_string()),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/edit_from_prefab") => {
            let Some(mode) = mode else {
                return Some(json_error(501, "Gen3D edit requires rendered mode."));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(501, "Gen3D edit requires rendered mode."));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(workshop) = gen3d_workshop.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            if job.is_running() {
                return Some(json_error(409, "Gen3D build is already running."));
            }

            let req: Gen3dSeedFromPrefabRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            let uuid = match uuid::Uuid::parse_str(req.prefab_id_uuid.trim()) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid prefab_id_uuid: {err}"))),
            };

            let sinks = log_sinks.cloned();
            if let Err(err) = crate::gen3d::gen3d_start_edit_session_from_prefab_id_from_api(
                build_scene,
                config,
                sinks,
                workshop,
                job,
                draft,
                active_realm_id,
                active_scene_id,
                uuid.as_u128(),
            ) {
                return Some(json_error(400, err));
            }

            let body = serde_json::json!({
                "ok": true,
                "run_id": job.run_id().map(|id| id.to_string()),
                "can_resume": job.can_resume(),
                "edit_base_prefab_id": job.edit_base_prefab_id().map(|id| uuid::Uuid::from_u128(id).to_string()),
                "save_overwrite_prefab_id": job.save_overwrite_prefab_id().map(|id| uuid::Uuid::from_u128(id).to_string()),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/fork_from_prefab") => {
            let Some(mode) = mode else {
                return Some(json_error(501, "Gen3D fork requires rendered mode."));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(501, "Gen3D fork requires rendered mode."));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(workshop) = gen3d_workshop.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            if job.is_running() {
                return Some(json_error(409, "Gen3D build is already running."));
            }

            let req: Gen3dSeedFromPrefabRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            let uuid = match uuid::Uuid::parse_str(req.prefab_id_uuid.trim()) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid prefab_id_uuid: {err}"))),
            };

            let sinks = log_sinks.cloned();
            if let Err(err) = crate::gen3d::gen3d_start_fork_session_from_prefab_id_from_api(
                build_scene,
                config,
                sinks,
                workshop,
                job,
                draft,
                active_realm_id,
                active_scene_id,
                uuid.as_u128(),
            ) {
                return Some(json_error(400, err));
            }

            let body = serde_json::json!({
                "ok": true,
                "run_id": job.run_id().map(|id| id.to_string()),
                "can_resume": job.can_resume(),
                "edit_base_prefab_id": job.edit_base_prefab_id().map(|id| uuid::Uuid::from_u128(id).to_string()),
                "save_overwrite_prefab_id": job.save_overwrite_prefab_id().map(|id| uuid::Uuid::from_u128(id).to_string()),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/resume") => {
            let Some(mode) = mode else {
                return Some(json_error(501, "Gen3D resume requires rendered mode."));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(501, "Gen3D resume requires rendered mode."));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(workshop) = gen3d_workshop.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            if job.is_running() {
                return Some(json_error(409, "Gen3D build is already running."));
            }

            let sinks = log_sinks.cloned();
            if let Err(err) =
                crate::gen3d::gen3d_resume_build_from_api(build_scene, config, sinks, workshop, job)
            {
                return Some(json_error(400, err));
            }

            let body = serde_json::json!({
                "ok": true,
                "run_id": job.run_id().map(|id| id.to_string()),
                "pass": job.pass(),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/stop") => {
            let Some(workshop) = gen3d_workshop.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            crate::gen3d::gen3d_cancel_build_from_api(workshop, job);
            Some(AutomationReply {
                status: 200,
                body: serde_json::json!({ "ok": true }).to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/apply_draft_ops") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D apply_draft_ops requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D apply_draft_ops requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let result = match crate::gen3d::gen3d_apply_draft_ops_from_api(job, draft, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/snapshot") => {
            let Some(mode) = mode else {
                return Some(json_error(501, "Gen3D snapshot requires rendered mode."));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(501, "Gen3D snapshot requires rendered mode."));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_snapshot_from_api(job, draft, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/list_snapshots") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D list_snapshots requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D list_snapshots requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_list_snapshots_from_api(job, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/diff_snapshots") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D diff_snapshots requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D diff_snapshots requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_diff_snapshots_from_api(job, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/restore_snapshot") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D restore_snapshot requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D restore_snapshot requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_restore_snapshot_from_api(job, draft, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/create_workspace") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D create_workspace requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D create_workspace requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_create_workspace_from_api(job, draft, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/delete_workspace") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D delete_workspace requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D delete_workspace requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_delete_workspace_from_api(job, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/set_active_workspace") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D set_active_workspace requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D set_active_workspace requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_set_active_workspace_from_api(job, draft, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/diff_workspaces") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D diff_workspaces requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D diff_workspaces requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_diff_workspaces_from_api(job, draft, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/copy_from_workspace") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D copy_from_workspace requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D copy_from_workspace requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_copy_from_workspace_from_api(job, draft, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/merge_workspace") => {
            let Some(mode) = mode else {
                return Some(json_error(
                    501,
                    "Gen3D merge_workspace requires rendered mode.",
                ));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Gen3D merge_workspace requires rendered mode.",
                ));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            let args: serde_json::Value = if msg.body.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let result = match crate::gen3d::gen3d_merge_workspace_from_api(job, draft, args) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, err)),
            };

            Some(AutomationReply {
                status: 200,
                body: result.to_string().into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/gen3d/save") => {
            let Some(mode) = mode else {
                return Some(json_error(501, "Gen3D save requires rendered mode."));
            };
            let Some(build_scene) = build_scene else {
                return Some(json_error(501, "Gen3D save requires rendered mode."));
            };
            if !matches!(mode.get(), GameMode::Build)
                || !matches!(build_scene.get(), BuildScene::Preview)
            {
                return Some(json_error(409, "Switch to Build Preview scene first."));
            }
            let Some(workshop) = gen3d_workshop.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(job) = gen3d_job.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(draft) = gen3d_draft.as_deref() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };
            let Some(asset_server) = asset_server else {
                return Some(json_error(
                    501,
                    "Gen3D save requires AssetServer (rendered mode).",
                ));
            };
            let Some(assets) = assets else {
                return Some(json_error(
                    501,
                    "Gen3D save requires SceneAssets (rendered mode).",
                ));
            };
            let Some(meshes) = meshes else {
                return Some(json_error(
                    501,
                    "Gen3D save requires meshes (rendered mode).",
                ));
            };
            let Some(materials) = materials else {
                return Some(json_error(
                    501,
                    "Gen3D save requires materials (rendered mode).",
                ));
            };
            let Some(material_cache) = material_cache else {
                return Some(json_error(
                    501,
                    "Gen3D save requires material cache (rendered mode).",
                ));
            };
            let Some(mesh_cache) = mesh_cache else {
                return Some(json_error(
                    501,
                    "Gen3D save requires mesh cache (rendered mode).",
                ));
            };

            let Ok((player_transform, player_collider)) = player_q.single() else {
                return Some(json_error(500, "Cannot save: missing hero entity."));
            };

            let saved = match crate::gen3d::gen3d_save_current_draft_seed_aware_from_api(
                commands,
                asset_server,
                assets,
                meshes,
                materials,
                material_cache,
                mesh_cache,
                active_realm_id,
                active_scene_id,
                library,
                prefab_descriptors,
                workshop,
                job,
                draft,
                false,
                player_transform,
                player_collider,
                world_objects,
                children_q,
                scene_saves,
            ) {
                Ok(saved) => saved,
                Err(err) => return Some(json_error(400, err)),
            };

            if let (Some(images), Some(prefab_thumbnail_capture)) =
                (images, prefab_thumbnail_capture)
            {
                let thumbnail_path =
                    crate::realm_prefab_packages::realm_prefab_package_thumbnail_path(
                        active_realm_id,
                        saved.prefab_id,
                    );
                if let Err(err) = crate::gen3d::gen3d_request_prefab_thumbnail_capture(
                    commands,
                    prefab_thumbnail_capture,
                    images,
                    asset_server,
                    assets,
                    meshes,
                    materials,
                    material_cache,
                    mesh_cache,
                    &*library,
                    saved.prefab_id,
                    thumbnail_path,
                ) {
                    warn!("Gen3D: thumbnail capture skipped: {err}");
                }
            }

            let body = serde_json::json!({
                "ok": true,
                "instance_id_uuid": uuid::Uuid::from_u128(saved.instance_id.0).to_string(),
                "prefab_id_uuid": uuid::Uuid::from_u128(saved.prefab_id).to_string(),
                "mobility": saved.mobility,
                "pos": [saved.position.x, saved.position.y, saved.position.z],
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        _ => Some(json_error(404, "Not found")),
    }
}

fn handle_scene_build_routes<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>(
    ctx: &mut AutomationContext<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>,
    msg: &AutomationRequest,
) -> Option<AutomationReply> {
    let commands = &mut *ctx.commands;
    let config = ctx.config;
    let active_realm_id = ctx.active_realm_id;
    let active_scene_id = ctx.active_scene_id;
    let library = &mut *ctx.library;
    let mut scene_build_runtime = ctx.scene_build_runtime.as_deref_mut();

    let mode = ctx.mode;
    let build_scene = ctx.build_scene;

    match (msg.method.as_str(), msg.path.as_str()) {
        ("GET", "/v1/scene_build/status") => {
            let Some(scene_build) = scene_build_runtime.as_deref() else {
                return Some(json_error(
                    501,
                    "Scene Build is not available in this app mode.",
                ));
            };
            let status = scene_build.automation_status();
            let body = serde_json::json!({ "ok": true, "status": status }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_build/start") => {
            let Some(mode) = mode else {
                return Some(json_error(501, "Scene Build requires rendered mode."));
            };
            if let Some(build_scene) = build_scene {
                if matches!(mode.get(), GameMode::Build)
                    && matches!(build_scene.get(), BuildScene::Preview)
                {
                    return Some(json_error(409, "Switch to the Realm scene first."));
                }
            }
            let Some(scene_build) = scene_build_runtime.as_deref_mut() else {
                return Some(json_error(
                    501,
                    "Scene Build is not available in this app mode.",
                ));
            };
            let req: SceneBuildStartRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            let active = crate::realm::ActiveRealmScene {
                realm_id: active_realm_id.to_string(),
                scene_id: active_scene_id.to_string(),
            };
            let run_id = match crate::scene_build_ai::start_scene_build_from_description(
                scene_build,
                config,
                &active,
                library,
                &req.description,
            ) {
                Ok(run_id) => run_id,
                Err(err) => {
                    let status = if err.to_lowercase().contains("already running") {
                        409
                    } else {
                        400
                    };
                    return Some(json_error(status, err));
                }
            };

            let body = serde_json::json!({ "ok": true, "run_id": run_id }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/scene_build/stop") => {
            let _req: SceneBuildStopRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            let Some(scene_build) = scene_build_runtime.as_deref_mut() else {
                return Some(json_error(
                    501,
                    "Scene Build is not available in this app mode.",
                ));
            };
            let run_id = scene_build.cancel_in_flight(commands, "canceled via API");
            let body =
                serde_json::json!({ "ok": true, "canceled": run_id.is_some(), "run_id": run_id })
                    .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        _ => Some(json_error(404, "Not found")),
    }
}

fn handle_animation_routes<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>(
    ctx: &mut AutomationContext<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>,
    msg: &AutomationRequest,
) -> Option<AutomationReply> {
    let commands = &mut *ctx.commands;
    let selectable_entities = &ctx.world.selectable_entities;

    match (msg.method.as_str(), msg.path.as_str()) {
        ("POST", "/v1/animation/force_channel") => {
            let req: ForceAnimationChannelRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            if req.instance_ids.is_empty() {
                return Some(json_error(400, "instance_ids must be non-empty."));
            }

            let channel = req.channel.trim().to_string();

            let mut id_to_entity: std::collections::HashMap<u128, Entity> =
                std::collections::HashMap::with_capacity(selectable_entities.iter().len());
            for (entity, object_id) in selectable_entities.iter() {
                id_to_entity.insert(object_id.0, entity);
            }

            let mut missing: Vec<String> = Vec::new();
            let mut targets: Vec<Entity> = Vec::with_capacity(req.instance_ids.len());
            for id_str in req.instance_ids {
                match uuid::Uuid::parse_str(id_str.trim()) {
                    Ok(uuid) => {
                        let id = uuid.as_u128();
                        if let Some(entity) = id_to_entity.get(&id).copied() {
                            targets.push(entity);
                        } else {
                            missing.push(uuid.to_string());
                        }
                    }
                    Err(_) => missing.push(id_str),
                }
            }
            if !missing.is_empty() {
                return Some(json_error(
                    404,
                    format!("Instances not found: {}", missing.join(", ")),
                ));
            }

            for entity in targets.iter().copied() {
                let mut cmd = commands.entity(entity);
                if channel.is_empty() {
                    cmd.remove::<ForcedAnimationChannel>();
                } else {
                    cmd.insert(ForcedAnimationChannel {
                        channel: channel.clone(),
                    });
                }
            }

            let body = serde_json::json!({
                "ok": true,
                "updated": targets.len(),
                "channel": channel,
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        _ => Some(json_error(404, "Not found")),
    }
}

fn handle_request_main_thread<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>(
    ctx: &mut AutomationContext<'a, 'cmd_w, 'cmd_s, 'gen3d_w, 'world_w, 'world_s, 'exit_w>,
    msg: &AutomationRequest,
) -> Option<AutomationReply> {
    if msg.path.starts_with("/v1/scene_sources/") {
        return handle_scene_sources_routes(ctx, msg);
    }
    if msg.path.starts_with("/v1/gen3d/") {
        return handle_gen3d_routes(ctx, msg);
    }
    if msg.path.starts_with("/v1/scene_build/") {
        return handle_scene_build_routes(ctx, msg);
    }
    if msg.path.starts_with("/v1/animation/") {
        return handle_animation_routes(ctx, msg);
    }

    let commands = &mut *ctx.commands;
    let library = &mut *ctx.library;
    let asset_server = ctx.gen3d.asset_server.as_deref();
    let assets = ctx.gen3d.assets.as_deref();
    let meshes = ctx.gen3d.meshes.as_deref_mut();
    let materials = ctx.gen3d.materials.as_deref_mut();
    let material_cache = ctx.gen3d.material_cache.as_deref_mut();
    let mesh_cache = ctx.gen3d.mesh_cache.as_deref_mut();

    let windows = &ctx.world.windows;
    let players = &ctx.world.players;
    let player_q = &ctx.world.player_q;
    let commandables = &ctx.world.commandables;
    let enemies = &ctx.world.enemies;
    let build_objects = &ctx.world.build_objects;
    let selectable_entities = &ctx.world.selectable_entities;
    let state_objects = &ctx.world.state_objects;

    let mode = ctx.mode;
    let next_mode = ctx.next_mode.as_deref_mut();
    let build_scene = ctx.build_scene;
    let next_build_scene = ctx.next_build_scene.as_deref_mut();
    let mut selection = ctx.selection.as_deref_mut();
    let mut fire = ctx.fire.as_deref_mut();
    let runtime = &mut *ctx.runtime;
    let exit = &mut *ctx.exit;

    match (msg.method.as_str(), msg.path.as_str()) {
        ("GET", "/v1/health") => {
            let body = serde_json::json!({
                "ok": true,
                "name": "gravimera",
                "version": env!("CARGO_PKG_VERSION"),
                "automation": {
                    "disable_local_input": runtime.disable_local_input,
                    "pause_on_start": runtime.pause_on_start,
                    "paused": runtime.time_paused,
                    "listen_addr": runtime.listen_addr,
                }
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("GET", "/v1/window") => {
            let Some((window_entity, window)) = windows.iter().next() else {
                return Some(json_error(
                    501,
                    "No primary window available (headless mode).",
                ));
            };

            let cursor = window.cursor_position().map(|p| [p.x, p.y]);
            let body = serde_json::json!({
                "ok": true,
                "window_entity": format!("{window_entity:?}"),
                "width": window.width(),
                "height": window.height(),
                "scale_factor": window.scale_factor(),
                "cursor": cursor,
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("GET", "/v1/state") => {
            let mode_str = mode.map(|m| match m.get() {
                GameMode::Build => "build",
                GameMode::Play => "play",
            });
            let build_scene_str = match (mode, build_scene) {
                (Some(mode), Some(scene)) if matches!(mode.get(), GameMode::Build) => {
                    Some(match scene.get() {
                        BuildScene::Realm => "realm",
                        BuildScene::Preview => "preview",
                    })
                }
                _ => None,
            };

            let selected_ids: Vec<String> = selection
                .as_deref()
                .map(|sel| {
                    sel.selected
                        .iter()
                        .filter_map(|entity| state_objects.get(*entity).ok().map(|v| v.1))
                        .map(|id| uuid::Uuid::from_u128(id.0).to_string())
                        .collect()
                })
                .unwrap_or_default();

            let objects: Vec<serde_json::Value> = state_objects
                .iter()
                .map(|(_entity, instance_id, prefab_id, transform, player, enemy, build, unit)| {
                    let forward = transform.rotation * Vec3::Z;
                    let yaw = forward.x.atan2(forward.z);
                    let attack = library.attack(prefab_id.0);
                    let attack_kind = attack.as_ref().map(|attack| match attack.kind {
                        crate::object::registry::UnitAttackKind::Melee => "melee",
                        crate::object::registry::UnitAttackKind::RangedProjectile => {
                            "ranged_projectile"
                        }
                    });
                    serde_json::json!({
                        "instance_id_uuid": uuid::Uuid::from_u128(instance_id.0).to_string(),
                        "prefab_id_uuid": uuid::Uuid::from_u128(prefab_id.0).to_string(),
                        "pos": [transform.translation.x, transform.translation.y, transform.translation.z],
                        "scale": [transform.scale.x, transform.scale.y, transform.scale.z],
                        "yaw": yaw,
                        "is_player": player.is_some(),
                        "is_enemy": enemy.is_some(),
                        "is_build_object": build.is_some(),
                        "is_commandable": unit.is_some(),
                        "has_attack": attack.is_some(),
                        "attack_kind": attack_kind,
                    })
                })
                .collect();

            let body = serde_json::json!({
                "ok": true,
                "mode": mode_str,
                "build_scene": build_scene_str,
                "selected_instance_ids": selected_ids,
                "objects": objects,
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/pause") => {
            if runtime.active_step.is_some() || !runtime.step_queue.is_empty() {
                return Some(json_error(409, "Cannot pause: a step is in progress."));
            }
            runtime.time_paused = true;
            let body = serde_json::json!({ "ok": true, "paused": true }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/resume") => {
            if runtime.active_step.is_some() || !runtime.step_queue.is_empty() {
                return Some(json_error(409, "Cannot resume: a step is in progress."));
            }
            runtime.time_paused = false;
            let body = serde_json::json!({ "ok": true, "paused": false }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/step") => {
            // This endpoint is synchronous (the HTTP request blocks until the step completes),
            // but we clamp the requested work to keep it responsive.
            const MAX_FRAMES: u32 = 1200;
            let req: StepRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            let frames = req.frames.clamp(1, MAX_FRAMES);
            let dt_secs = req.dt_secs.unwrap_or(1.0 / 60.0);
            if !dt_secs.is_finite() || dt_secs <= 0.0 {
                return Some(json_error(
                    400,
                    "`dt_secs` must be a positive finite number.",
                ));
            }
            let dt_secs = dt_secs.clamp(0.001, 0.1);
            let dt = Duration::from_secs_f32(dt_secs);

            let job = AutomationStepJob {
                remaining_frames: frames,
                dt,
                reply: msg.reply.clone(),
            };
            if runtime.active_step.is_none() {
                runtime.active_step = Some(job);
            } else {
                runtime.step_queue.push_back(job);
            }
            runtime.time_paused = true;

            // Defer reply until `automation_step_tick` finishes the last frame.
            None
        }
        ("POST", "/v1/screenshot") => {
            let req: ScreenshotRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            if req.include_ui == Some(false) {
                return Some(json_error(501, "`include_ui=false` is not supported yet."));
            }

            if windows.iter().next().is_none() {
                return Some(json_error(
                    501,
                    "No primary window available (headless mode).",
                ));
            }

            let path = PathBuf::from(req.path);
            if let Some(parent) = path.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    return Some(json_error(500, format!("Failed to create dir: {err}")));
                }
            }

            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()));

            let body =
                serde_json::json!({ "ok": true, "path": path.display().to_string() }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/meta/open") => {
            let Some(motion_ui) = ctx.motion_ui.as_deref_mut() else {
                return Some(json_error(
                    501,
                    "Meta panel is not available in this app mode.",
                ));
            };

            let req: MetaPanelRequest = if msg.body.is_empty() {
                MetaPanelRequest {
                    instance_id_uuid: None,
                }
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let target_entity = if let Some(id_str) = req.instance_id_uuid.as_ref() {
                let Ok(uuid) = uuid::Uuid::parse_str(id_str.trim()) else {
                    return Some(json_error(400, "Invalid `instance_id_uuid`."));
                };
                let instance_id = ObjectId(uuid.as_u128());
                state_objects.iter().find_map(
                    |(entity, object_id, _prefab_id, _t, _p, _e, _b, commandable)| {
                        (object_id.0 == instance_id.0 && commandable.is_some()).then_some(entity)
                    },
                )
            } else {
                selection.as_deref().and_then(|selection| {
                    selection.selected.iter().copied().find(|entity| {
                        state_objects
                            .get(*entity)
                            .ok()
                            .and_then(|row| row.7)
                            .is_some()
                    })
                })
            };

            let Some(target_entity) = target_entity else {
                return Some(json_error(
                    400,
                    "No commandable target found (provide `instance_id_uuid` or select a unit).",
                ));
            };

            motion_ui.open_for(target_entity);

            let body = serde_json::json!({ "ok": true }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/meta/close") => {
            let Some(motion_ui) = ctx.motion_ui.as_deref_mut() else {
                return Some(json_error(
                    501,
                    "Meta panel is not available in this app mode.",
                ));
            };
            motion_ui.close();
            let body = serde_json::json!({ "ok": true }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/meta/gen3d/copy") => {
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Meta Gen3D actions require build scene switching (rendered mode).",
                ));
            };
            if !matches!(build_scene.get(), BuildScene::Realm) {
                return Some(json_error(409, "Switch to Build Realm scene first."));
            }

            let Some(asset_server) = asset_server else {
                return Some(json_error(
                    501,
                    "Meta Gen3D Copy requires AssetServer (rendered mode).",
                ));
            };
            let Some(assets) = assets else {
                return Some(json_error(
                    501,
                    "Meta Gen3D Copy requires SceneAssets (rendered mode).",
                ));
            };
            let Some(meshes) = meshes else {
                return Some(json_error(
                    501,
                    "Meta Gen3D Copy requires meshes (rendered mode).",
                ));
            };
            let Some(materials) = materials else {
                return Some(json_error(
                    501,
                    "Meta Gen3D Copy requires materials (rendered mode).",
                ));
            };
            let Some(material_cache) = material_cache else {
                return Some(json_error(
                    501,
                    "Meta Gen3D Copy requires material cache (rendered mode).",
                ));
            };
            let Some(mesh_cache) = mesh_cache else {
                return Some(json_error(
                    501,
                    "Meta Gen3D Copy requires mesh cache (rendered mode).",
                ));
            };

            let req: MetaGen3dActionRequest = if msg.body.is_empty() {
                MetaGen3dActionRequest {
                    instance_id_uuid: None,
                }
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let target_entity = if let Some(id_str) = req.instance_id_uuid.as_ref() {
                let Ok(uuid) = uuid::Uuid::parse_str(id_str.trim()) else {
                    return Some(json_error(400, "Invalid `instance_id_uuid`."));
                };
                let instance_id = ObjectId(uuid.as_u128());
                state_objects.iter().find_map(
                    |(entity, object_id, _prefab_id, _t, _p, _e, _b, commandable)| {
                        (object_id.0 == instance_id.0 && commandable.is_some()).then_some(entity)
                    },
                )
            } else {
                selection.as_deref().and_then(|selection| {
                    selection.selected.iter().copied().find(|entity| {
                        state_objects
                            .get(*entity)
                            .ok()
                            .and_then(|row| row.7)
                            .is_some()
                    })
                })
            };

            let Some(target_entity) = target_entity else {
                return Some(json_error(
                    400,
                    "No commandable target found (provide `instance_id_uuid` or select a unit).",
                ));
            };

            let Ok((_entity, transform, _instance_id, prefab_id, tint, forms, _owner)) =
                ctx.world.scene_instances.get(target_entity)
            else {
                return Some(json_error(404, "Target instance not found."));
            };

            let is_gen3d_saved = ctx
                .prefab_descriptors
                .get(prefab_id.0)
                .and_then(|d| d.provenance.as_ref())
                .and_then(|p| p.source.as_deref())
                .is_some_and(|v| v.trim() == "gen3d");
            if !is_gen3d_saved {
                return Some(json_error(
                    400,
                    "Copy is supported only for Gen3D-saved prefabs.",
                ));
            }

            let Some(def) = library.get(prefab_id.0) else {
                return Some(json_error(404, "Prefab not found."));
            };
            let size = def.size.abs();
            let collider_half_xz = match def.collider {
                crate::object::registry::ColliderProfile::CircleXZ { radius } => {
                    Vec2::splat(radius.abs())
                }
                crate::object::registry::ColliderProfile::AabbXZ { half_extents } => Vec2::new(
                    half_extents.x.abs().max(0.01),
                    half_extents.y.abs().max(0.01),
                ),
                crate::object::registry::ColliderProfile::None => {
                    Vec2::new((size.x * 0.5).max(0.01), (size.z * 0.5).max(0.01))
                }
            };
            let radius = collider_half_xz.x.max(collider_half_xz.y).max(0.1);

            let snap_step = BUILD_GRID_SIZE.max(0.01);
            let offset_step = BUILD_UNIT_SIZE.max(snap_step);
            let offset = Vec3::new(offset_step, 0.0, offset_step);

            let mut new_transform = *transform;
            new_transform.translation += offset;
            new_transform.translation.x = snap_to_grid(new_transform.translation.x, snap_step);
            new_transform.translation.z = snap_to_grid(new_transform.translation.z, snap_step);
            new_transform.translation.x = clamp_world_xz(new_transform.translation.x, radius);
            new_transform.translation.z = clamp_world_xz(new_transform.translation.z, radius);

            let forms = forms
                .cloned()
                .unwrap_or_else(|| ObjectForms::new_single(prefab_id.0));
            let tint_color = tint.map(|t| t.0);
            let instance_id = ObjectId::new_v4();

            let mut entity_commands = commands.spawn((
                instance_id,
                *prefab_id,
                forms,
                Commandable,
                Collider { radius },
                new_transform,
                Visibility::Inherited,
            ));
            if let Some(tint_color) = tint_color {
                entity_commands.insert(ObjectTint(tint_color));
            }

            crate::object::visuals::spawn_object_visuals(
                &mut entity_commands,
                library,
                asset_server,
                assets,
                meshes,
                materials,
                material_cache,
                mesh_cache,
                prefab_id.0,
                tint_color,
            );

            let body = serde_json::json!({
                "ok": true,
                "instance_id_uuid": uuid::Uuid::from_u128(instance_id.0).to_string(),
                "prefab_id_uuid": uuid::Uuid::from_u128(prefab_id.0).to_string(),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/meta/gen3d/edit") | ("POST", "/v1/meta/gen3d/fork") => {
            let Some(build_scene) = build_scene else {
                return Some(json_error(
                    501,
                    "Meta Gen3D actions require build scene switching (rendered mode).",
                ));
            };
            if !matches!(build_scene.get(), BuildScene::Realm) {
                return Some(json_error(409, "Switch to Build Realm scene first."));
            }
            let Some(next_mode) = next_mode else {
                return Some(json_error(
                    501,
                    "Meta Gen3D actions require game mode switching (rendered mode).",
                ));
            };
            let Some(next_build_scene) = next_build_scene else {
                return Some(json_error(
                    501,
                    "Meta Gen3D actions require build scene switching (rendered mode).",
                ));
            };
            let Some(pending_seed) = ctx.gen3d.pending_seed.as_deref_mut() else {
                return Some(json_error(501, "Gen3D is not available in this app mode."));
            };

            if ctx.gen3d.job.as_deref().is_some_and(|job| job.is_running()) {
                return Some(json_error(
                    409,
                    "Cannot seed while a Gen3D build is running (stop it first).",
                ));
            }

            let req: MetaGen3dActionRequest = if msg.body.is_empty() {
                MetaGen3dActionRequest {
                    instance_id_uuid: None,
                }
            } else {
                match serde_json::from_slice(&msg.body) {
                    Ok(v) => v,
                    Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
                }
            };

            let target_entity = if let Some(id_str) = req.instance_id_uuid.as_ref() {
                let Ok(uuid) = uuid::Uuid::parse_str(id_str.trim()) else {
                    return Some(json_error(400, "Invalid `instance_id_uuid`."));
                };
                let instance_id = ObjectId(uuid.as_u128());
                state_objects.iter().find_map(
                    |(entity, object_id, _prefab_id, _t, _p, _e, _b, commandable)| {
                        (object_id.0 == instance_id.0 && commandable.is_some()).then_some(entity)
                    },
                )
            } else {
                selection.as_deref().and_then(|selection| {
                    selection.selected.iter().copied().find(|entity| {
                        state_objects
                            .get(*entity)
                            .ok()
                            .and_then(|row| row.7)
                            .is_some()
                    })
                })
            };

            let Some(target_entity) = target_entity else {
                return Some(json_error(
                    400,
                    "No commandable target found (provide `instance_id_uuid` or select a unit).",
                ));
            };

            let Ok((_entity, _transform, instance_id, prefab_id, _tint, _forms, _owner)) =
                ctx.world.scene_instances.get(target_entity)
            else {
                return Some(json_error(404, "Target instance not found."));
            };

            let is_gen3d_saved = ctx
                .prefab_descriptors
                .get(prefab_id.0)
                .and_then(|d| d.provenance.as_ref())
                .and_then(|p| p.source.as_deref())
                .is_some_and(|v| v.trim() == "gen3d");
            if !is_gen3d_saved {
                return Some(json_error(
                    400,
                    "Edit/Fork is supported only for Gen3D-saved prefabs.",
                ));
            }

            let mode = if msg.path.as_str().ends_with("/edit") {
                crate::gen3d::Gen3dSeedFromPrefabMode::EditOverwrite
            } else {
                crate::gen3d::Gen3dSeedFromPrefabMode::Fork
            };
            pending_seed.request = Some(crate::gen3d::Gen3dSeedFromPrefabRequest {
                mode,
                prefab_id: prefab_id.0,
                target_entity: Some(target_entity),
            });

            if let Some(motion_ui) = ctx.motion_ui.as_deref_mut() {
                motion_ui.close();
            }
            next_mode.set(GameMode::Build);
            next_build_scene.set(BuildScene::Preview);

            let body = serde_json::json!({
                "ok": true,
                "instance_id_uuid": uuid::Uuid::from_u128(instance_id.0).to_string(),
                "prefab_id_uuid": uuid::Uuid::from_u128(prefab_id.0).to_string(),
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/spawn") => {
            let Some(asset_server) = asset_server else {
                return Some(json_error(
                    501,
                    "Spawning requires AssetServer (rendered mode).",
                ));
            };
            let Some(assets) = assets else {
                return Some(json_error(
                    501,
                    "Spawning requires SceneAssets (rendered mode).",
                ));
            };
            let Some(meshes) = meshes else {
                return Some(json_error(501, "Spawning requires meshes (rendered mode)."));
            };
            let Some(materials) = materials else {
                return Some(json_error(
                    501,
                    "Spawning requires materials (rendered mode).",
                ));
            };
            let Some(material_cache) = material_cache else {
                return Some(json_error(
                    501,
                    "Spawning requires material cache (rendered mode).",
                ));
            };
            let Some(mesh_cache) = mesh_cache else {
                return Some(json_error(
                    501,
                    "Spawning requires mesh cache (rendered mode).",
                ));
            };

            let req: SpawnRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            let Ok(prefab_uuid) = uuid::Uuid::parse_str(req.prefab_id_uuid.trim()) else {
                return Some(json_error(400, "Invalid prefab_id_uuid UUID."));
            };
            let prefab_id = prefab_uuid.as_u128();
            let Some(def) = library.get(prefab_id) else {
                return Some(json_error(404, "Prefab not found."));
            };

            let size = def.size.abs();
            let collider_half_xz = match def.collider {
                crate::object::registry::ColliderProfile::CircleXZ { radius } => {
                    Vec2::splat(radius.abs())
                }
                crate::object::registry::ColliderProfile::AabbXZ { half_extents } => Vec2::new(
                    half_extents.x.abs().max(0.01),
                    half_extents.y.abs().max(0.01),
                ),
                crate::object::registry::ColliderProfile::None => {
                    Vec2::new((size.x * 0.5).max(0.01), (size.z * 0.5).max(0.01))
                }
            };

            let Ok((player_transform, player_collider)) = player_q.single() else {
                return Some(json_error(500, "Cannot spawn: missing hero entity."));
            };

            let mut pos = if let (Some(x), Some(z)) = (req.x, req.z) {
                Vec3::new(x, req.y.unwrap_or(0.0), z)
            } else {
                let forward = player_transform.rotation * Vec3::Z;
                let mut dir = Vec3::new(forward.x, 0.0, forward.z);
                if dir.length_squared() <= 1e-6 {
                    dir = Vec3::Z;
                } else {
                    dir = dir.normalize();
                }
                let radius = collider_half_xz.x.max(collider_half_xz.y).max(0.1);
                let distance = player_collider.radius + radius + BUILD_UNIT_SIZE * 2.0;
                let base = player_transform.translation + dir * distance;
                Vec3::new(base.x, req.y.unwrap_or(0.0), base.z)
            };

            if req.x.is_none() || req.z.is_none() {
                pos.y = req
                    .y
                    .unwrap_or_else(|| library.ground_origin_y_or_default(prefab_id));
            }

            pos.x = pos.x.clamp(
                -WORLD_HALF_SIZE + collider_half_xz.x,
                WORLD_HALF_SIZE - collider_half_xz.x,
            );
            pos.z = pos.z.clamp(
                -WORLD_HALF_SIZE + collider_half_xz.y,
                WORLD_HALF_SIZE - collider_half_xz.y,
            );

            let yaw = req.yaw.unwrap_or(0.0);
            let mut transform = Transform::from_translation(pos);
            if yaw.is_finite() {
                transform.rotation = Quat::from_rotation_y(yaw);
            }

            let instance_id = ObjectId::new_v4();
            let mobility = def.mobility.is_some();

            let mut entity_commands = if mobility {
                let radius = collider_half_xz.x.max(collider_half_xz.y).max(0.1);
                commands.spawn((
                    instance_id,
                    ObjectPrefabId(prefab_id),
                    Commandable,
                    Collider { radius },
                    transform,
                    Visibility::Inherited,
                ))
            } else {
                commands.spawn((
                    instance_id,
                    ObjectPrefabId(prefab_id),
                    BuildObject,
                    BuildDimensions { size },
                    AabbCollider {
                        half_extents: collider_half_xz,
                    },
                    transform,
                    Visibility::Inherited,
                ))
            };

            crate::object::visuals::spawn_object_visuals(
                &mut entity_commands,
                library,
                asset_server,
                assets,
                meshes,
                materials,
                material_cache,
                mesh_cache,
                prefab_id,
                None,
            );

            let body = serde_json::json!({
                "ok": true,
                "instance_id_uuid": uuid::Uuid::from_u128(instance_id.0).to_string(),
                "prefab_id_uuid": uuid::Uuid::from_u128(prefab_id).to_string(),
                "mobility": mobility,
                "pos": [pos.x, pos.y, pos.z],
            })
            .to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/select") => {
            let Some(selection) = selection.as_deref_mut() else {
                return Some(json_error(
                    501,
                    "Selection is not available in this app mode.",
                ));
            };
            let req: SelectRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            let mut selected = std::collections::HashSet::new();
            for id_str in req.instance_ids {
                let Ok(uuid) = uuid::Uuid::parse_str(id_str.trim()) else {
                    continue;
                };
                let id = ObjectId(uuid.as_u128());
                for (entity, object_id) in selectable_entities.iter() {
                    if object_id.0 == id.0 {
                        selected.insert(entity);
                        break;
                    }
                }
            }
            selection.selected = selected;
            selection.drag_start = None;
            selection.drag_end = None;

            let body =
                serde_json::json!({ "ok": true, "selected": selection.selected.len() }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/move") => {
            let Some(selection) = selection.as_deref() else {
                return Some(json_error(
                    501,
                    "Selection is not available in this app mode.",
                ));
            };
            if selection.selected.is_empty() {
                return Some(json_error(400, "No selected units."));
            }

            let req: MoveRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            let goal = Vec2::new(req.x, req.z);
            let goal_ground_y = req.y.unwrap_or(0.0).max(0.0);

            let obstacles = collect_nav_obstacles(build_objects, library);
            let mut any_order = false;

            for entity in selection.selected.iter().copied() {
                let Ok((_entity, transform, collider, prefab_id)) = commandables.get(entity) else {
                    continue;
                };
                let Some(mobility) = library.mobility(prefab_id.0) else {
                    continue;
                };

                let radius = collider.radius.max(0.01);
                let min = Vec2::splat(-WORLD_HALF_SIZE + radius);
                let max = Vec2::splat(WORLD_HALF_SIZE - radius);
                let clamped_goal = goal.clamp(min, max);

                let start = Vec2::new(transform.translation.x, transform.translation.z);
                let scale_y = safe_abs_scale_y(transform.scale);
                let origin_y = if players.contains(entity) {
                    PLAYER_Y
                } else {
                    library.ground_origin_y_or_default(prefab_id.0) * scale_y
                };
                let current_ground_y = (transform.translation.y - origin_y).max(0.0);
                let height = library
                    .size(prefab_id.0)
                    .map(|s| s.y * scale_y)
                    .unwrap_or(HERO_HEIGHT_WORLD * scale_y);

                let mut order = MoveOrder::default();
                match mobility.mode {
                    crate::object::registry::MobilityMode::Air => {
                        order.target = Some(clamped_goal);
                    }
                    crate::object::registry::MobilityMode::Ground => {
                        let Some(path) = navigation::find_path_height_aware(
                            start,
                            current_ground_y,
                            clamped_goal,
                            goal_ground_y,
                            radius,
                            height,
                            WORLD_HALF_SIZE,
                            NAV_GRID_SIZE,
                            &obstacles,
                        ) else {
                            commands.entity(entity).remove::<MoveOrder>();
                            continue;
                        };
                        let path = navigation::smooth_path_height_aware(
                            start,
                            current_ground_y,
                            path,
                            radius,
                            height,
                            NAV_GRID_SIZE,
                            &obstacles,
                        );
                        order.path = path.into();
                        order.target = Some(clamped_goal);
                    }
                }

                commands.entity(entity).insert(order);
                any_order = true;
            }

            if !any_order {
                return Some(json_error(409, "No move orders could be issued."));
            }

            let body = serde_json::json!({ "ok": true }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/fire") => {
            let Some(fire) = fire.as_deref_mut() else {
                return Some(json_error(
                    501,
                    "Fire control is not available in this app mode.",
                ));
            };
            let req: FireRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };

            fire.active = req.active;
            fire.target = None;

            if req.active {
                if let Some(target) = req.target {
                    match target {
                        FireTargetRequest::Point { x, z } => {
                            fire.target = Some(FireTarget::Point(Vec2::new(x, z)));
                        }
                        FireTargetRequest::Enemy { instance_id } => {
                            let Ok(uuid) = uuid::Uuid::parse_str(instance_id.trim()) else {
                                return Some(json_error(400, "Invalid enemy instance_id UUID."));
                            };
                            let id = ObjectId(uuid.as_u128());
                            let mut found = None;
                            for (entity, enemy_id) in enemies.iter() {
                                if enemy_id.0 == id.0 {
                                    found = Some(entity);
                                    break;
                                }
                            }
                            if let Some(enemy_entity) = found {
                                fire.target = Some(FireTarget::Unit(enemy_entity));
                            } else {
                                return Some(json_error(404, "Enemy not found."));
                            }
                        }
                    }
                }
            }

            let body = serde_json::json!({ "ok": true }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/mode") => {
            let Some(next_mode) = next_mode else {
                return Some(json_error(
                    501,
                    "Game mode switching is not available in this app mode.",
                ));
            };
            let req: ModeRequest = match serde_json::from_slice(&msg.body) {
                Ok(v) => v,
                Err(err) => return Some(json_error(400, format!("Invalid JSON: {err}"))),
            };
            let mode_str = req.mode.trim().to_ascii_lowercase();
            match mode_str.as_str() {
                "build" => {
                    next_mode.set(GameMode::Build);
                    if let Some(next_build_scene) = next_build_scene {
                        next_build_scene.set(BuildScene::Realm);
                    }
                }
                "play" => {
                    next_mode.set(GameMode::Play);
                    if let Some(next_build_scene) = next_build_scene {
                        next_build_scene.set(BuildScene::Realm);
                    }
                }
                // Legacy compatibility: treat Gen3D as Build Preview scene.
                "gen3d" | "gen3d_workshop" | "preview" | "build_preview" => {
                    let Some(next_build_scene) = next_build_scene else {
                        return Some(json_error(
                            501,
                            "Build scene switching is not available in this app mode.",
                        ));
                    };
                    next_mode.set(GameMode::Build);
                    next_build_scene.set(BuildScene::Preview);
                }
                _ => {
                    return Some(json_error(
                        400,
                        "Invalid mode (expected build/play; legacy: gen3d).",
                    ));
                }
            }
            let body = serde_json::json!({ "ok": true }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        ("POST", "/v1/shutdown") => {
            exit.write(AppExit::Success);
            let body = serde_json::json!({ "ok": true }).to_string();
            Some(AutomationReply {
                status: 200,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
        _ => {
            let body = serde_json::json!({
                "ok": false,
                "error": "Not found",
            })
            .to_string();
            Some(AutomationReply {
                status: 404,
                body: body.into_bytes(),
                content_type: "application/json",
            })
        }
    }
}

fn json_error(status: u16, message: impl Into<String>) -> AutomationReply {
    let body = serde_json::json!({ "ok": false, "error": message.into() }).to_string();
    AutomationReply {
        status,
        body: body.into_bytes(),
        content_type: "application/json",
    }
}

fn collect_nav_obstacles(
    objects: &Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    library: &ObjectLibrary,
) -> Vec<navigation::NavObstacle> {
    let mut obstacles = Vec::with_capacity(objects.iter().len());
    for (transform, collider, dimensions, prefab_id) in objects.iter() {
        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let bottom_y = transform.translation.y - origin_y;
        let top_y = bottom_y + dimensions.size.y;
        let interaction = library.interaction(prefab_id.0);
        obstacles.push(navigation::NavObstacle {
            movement_block: interaction.movement_block,
            supports_standing: interaction.supports_standing,
            center: Vec2::new(transform.translation.x, transform.translation.z),
            half: collider.half_extents,
            bottom_y,
            top_y,
        });
    }
    obstacles
}

fn automation_time_control(
    config: Res<AppConfig>,
    runtime: Res<AutomationRuntime>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    mut virtual_time: ResMut<Time<Virtual>>,
) {
    if !config.automation_enabled || !runtime.enabled {
        return;
    }
    if let Some(job) = runtime.active_step.as_ref() {
        *strategy = TimeUpdateStrategy::ManualDuration(job.dt);
        virtual_time.unpause();
        return;
    }
    *strategy = TimeUpdateStrategy::Automatic;
    if runtime.time_paused {
        virtual_time.pause();
    } else {
        virtual_time.unpause();
    }
}

fn automation_step_tick(config: Res<AppConfig>, mut runtime: ResMut<AutomationRuntime>) {
    if !config.automation_enabled || !runtime.enabled {
        return;
    }
    let Some(job) = runtime.active_step.as_mut() else {
        return;
    };
    if job.remaining_frames == 0 {
        runtime.active_step = None;
        return;
    }

    job.remaining_frames = job.remaining_frames.saturating_sub(1);
    if job.remaining_frames > 0 {
        return;
    }

    let body = serde_json::json!({ "ok": true }).to_string();
    let _ = job.reply.send(AutomationReply {
        status: 200,
        body: body.into_bytes(),
        content_type: "application/json",
    });

    runtime.active_step = runtime.step_queue.pop_front();
}
