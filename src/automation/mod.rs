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
use crate::navigation;
use crate::object::registry::ObjectLibrary;
use crate::scene_store::SceneSaveRequest;
use crate::types::*;

pub(crate) struct AutomationPlugin;

impl Plugin for AutomationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutomationRuntime>();
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
}

#[derive(SystemParam)]
struct AutomationGen3d<'w> {
    log_sinks: Option<Res<'w, crate::app::Gen3dLogSinks>>,
    workshop: Option<ResMut<'w, crate::gen3d::Gen3dWorkshop>>,
    job: Option<ResMut<'w, crate::gen3d::Gen3dAiJob>>,
    draft: Option<ResMut<'w, crate::gen3d::Gen3dDraft>>,
    asset_server: Option<Res<'w, AssetServer>>,
    assets: Option<Res<'w, SceneAssets>>,
    meshes: Option<ResMut<'w, Assets<Mesh>>>,
    materials: Option<ResMut<'w, Assets<StandardMaterial>>>,
    material_cache: Option<ResMut<'w, crate::object::visuals::MaterialCache>>,
    mesh_cache: Option<ResMut<'w, crate::object::visuals::PrimitiveMeshCache>>,
    scene_saves: MessageWriter<'w, SceneSaveRequest>,
}

fn automation_process_requests(
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    config: Res<AppConfig>,
    mut runtime: ResMut<AutomationRuntime>,
    mode: Option<Res<State<GameMode>>>,
    mut next_mode: Option<ResMut<NextState<GameMode>>>,
    mut selection: Option<ResMut<SelectionState>>,
    mut fire: Option<ResMut<FireControl>>,
    mut library: ResMut<ObjectLibrary>,
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

        let reply = handle_request_main_thread(
            &mut commands,
            &config,
            gen3d.log_sinks.as_deref(),
            &mut library,
            gen3d.workshop.as_deref_mut(),
            gen3d.job.as_deref_mut(),
            gen3d.draft.as_deref_mut(),
            gen3d.asset_server.as_deref(),
            gen3d.assets.as_deref(),
            gen3d.meshes.as_deref_mut(),
            gen3d.materials.as_deref_mut(),
            gen3d.material_cache.as_deref_mut(),
            gen3d.mesh_cache.as_deref_mut(),
            &mut gen3d.scene_saves,
            &world.windows,
            &world.players,
            &world.player_q,
            &world.commandables,
            &world.enemies,
            &world.build_objects,
            &world.selectable_entities,
            &world.state_objects,
            mode.as_deref(),
            next_mode.as_deref_mut(),
            selection.as_deref_mut(),
            fire.as_deref_mut(),
            &mut runtime,
            &msg,
            &mut exit,
        );
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
struct Gen3dPromptRequest {
    prompt: String,
}

fn handle_request_main_thread(
    commands: &mut Commands,
    config: &AppConfig,
    log_sinks: Option<&crate::app::Gen3dLogSinks>,
    library: &mut ObjectLibrary,
    mut gen3d_workshop: Option<&mut crate::gen3d::Gen3dWorkshop>,
    mut gen3d_job: Option<&mut crate::gen3d::Gen3dAiJob>,
    mut gen3d_draft: Option<&mut crate::gen3d::Gen3dDraft>,
    asset_server: Option<&AssetServer>,
    assets: Option<&SceneAssets>,
    meshes: Option<&mut Assets<Mesh>>,
    materials: Option<&mut Assets<StandardMaterial>>,
    material_cache: Option<&mut crate::object::visuals::MaterialCache>,
    mesh_cache: Option<&mut crate::object::visuals::PrimitiveMeshCache>,
    scene_saves: &mut MessageWriter<SceneSaveRequest>,
    windows: &Query<(Entity, &Window), With<PrimaryWindow>>,
    players: &Query<(), With<Player>>,
    player_q: &Query<(&Transform, &Collider), With<Player>>,
    commandables: &Query<(Entity, &Transform, &Collider, &ObjectPrefabId), With<Commandable>>,
    enemies: &Query<(Entity, &ObjectId), With<Enemy>>,
    build_objects: &Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    selectable_entities: &Query<
        (Entity, &ObjectId),
        (
            Or<(With<Commandable>, With<BuildObject>, With<Enemy>)>,
            Without<Bullet>,
        ),
    >,
    state_objects: &Query<
        (
            Entity,
            &ObjectId,
            &ObjectPrefabId,
            &Transform,
            Option<&Player>,
            Option<&Enemy>,
            Option<&BuildObject>,
            Option<&Commandable>,
        ),
        Or<(With<Commandable>, With<BuildObject>, With<Enemy>)>,
    >,
    mode: Option<&State<GameMode>>,
    next_mode: Option<&mut NextState<GameMode>>,
    mut selection: Option<&mut SelectionState>,
    mut fire: Option<&mut FireControl>,
    runtime: &mut AutomationRuntime,
    msg: &AutomationRequest,
    exit: &mut MessageWriter<AppExit>,
) -> Option<AutomationReply> {
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
                GameMode::Gen3D => "gen3d",
            });

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
            if !matches!(mode.get(), GameMode::Gen3D) {
                return Some(json_error(409, "Switch to Gen3D mode first."));
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
            if let Err(err) =
                crate::gen3d::gen3d_start_build_from_api(mode, config, sinks, workshop, job, draft)
            {
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
        ("POST", "/v1/gen3d/save") => {
            let Some(mode) = mode else {
                return Some(json_error(501, "Gen3D save requires rendered mode."));
            };
            if !matches!(mode.get(), GameMode::Gen3D) {
                return Some(json_error(409, "Switch to Gen3D mode first."));
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

            let saved = match crate::gen3d::gen3d_save_current_draft_from_api(
                commands,
                asset_server,
                assets,
                meshes,
                materials,
                material_cache,
                mesh_cache,
                library,
                workshop,
                job,
                draft,
                player_transform,
                player_collider,
                scene_saves,
            ) {
                Ok(saved) => saved,
                Err(err) => return Some(json_error(400, err)),
            };

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
                let origin_y = if players.contains(entity) {
                    PLAYER_Y
                } else {
                    library.size(prefab_id.0).map(|s| s.y * 0.5).unwrap_or(0.0)
                };
                let current_ground_y = (transform.translation.y - origin_y).max(0.0);
                let height = library
                    .size(prefab_id.0)
                    .map(|s| s.y)
                    .unwrap_or(HERO_HEIGHT_WORLD);

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
                                fire.target = Some(FireTarget::Enemy(enemy_entity));
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
            let new_mode = match mode_str.as_str() {
                "build" => GameMode::Build,
                "play" => GameMode::Play,
                "gen3d" | "gen3d_workshop" => GameMode::Gen3D,
                _ => return Some(json_error(400, "Invalid mode (expected build/play/gen3d).")),
            };
            next_mode.set(new_mode);
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
        let half_y = dimensions.size.y * 0.5;
        let interaction = library.interaction(prefab_id.0);
        obstacles.push(navigation::NavObstacle {
            movement_block: interaction.movement_block,
            supports_standing: interaction.supports_standing,
            center: Vec2::new(transform.translation.x, transform.translation.z),
            half: collider.half_extents,
            bottom_y: transform.translation.y - half_y,
            top_y: transform.translation.y + half_y,
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
