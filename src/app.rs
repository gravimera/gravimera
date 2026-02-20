use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
#[cfg(target_os = "macos")]
use bevy::render::settings::{Backends, WgpuSettings};
#[cfg(target_os = "macos")]
use bevy::render::RenderPlugin;
use bevy::ui::UiSystems;
use bevy::window::PrimaryWindow;
use bevy::window::WindowResolution;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use std::{
    fs::OpenOptions,
    io,
    path::Path,
    sync::{Arc, Mutex},
};

use crate::build;
use crate::combat;
use crate::common;
use crate::console;
use crate::constants::*;
use crate::effects;
use crate::enemies;
use crate::headless;
use crate::locomotion;
use crate::object::registry::ObjectLibrary;
use crate::physics;
use crate::player;
use crate::rts;
use crate::scene_store;
use crate::setup;
use crate::types::*;

static BEVY_LOG_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct PrimaryWindowIconState {
    done: bool,
    icon: Option<winit::window::Icon>,
    icon_load_failed: bool,
}

fn load_primary_window_icon() -> Result<winit::window::Icon, String> {
    static ICON_BYTES: &[u8] = include_bytes!("../assets/icon_64.png");

    let image = image::load_from_memory(ICON_BYTES)
        .map_err(|err| format!("Failed to decode embedded window icon PNG: {err}"))?
        .into_rgba8();

    let (width, height) = (image.width(), image.height());
    winit::window::Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|err| format!("Failed to create winit Icon from embedded PNG: {err}"))
}

fn try_set_primary_window_icon(
    primary_window: Query<Entity, With<PrimaryWindow>>,
    mut state: Local<PrimaryWindowIconState>,
    _non_send: NonSendMarker,
) {
    if state.done || state.icon_load_failed {
        return;
    }

    if state.icon.is_none() {
        match load_primary_window_icon() {
            Ok(icon) => state.icon = Some(icon),
            Err(err) => {
                warn!("{err}");
                state.icon_load_failed = true;
                return;
            }
        }
    }

    let Ok(window_entity) = primary_window.single() else {
        return;
    };
    let Some(icon) = state.icon.clone() else {
        return;
    };

    let mut set = false;
    bevy::winit::WINIT_WINDOWS.with_borrow(|winit_windows| {
        if let Some(winit_window) = winit_windows.get_window(window_entity) {
            winit_window.set_window_icon(Some(icon));
            set = true;
        }
    });

    if set {
        state.done = true;
    }
}

pub(crate) fn run() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    if raw_args.first().map(|s| s.as_str()) == Some("model-tool") {
        crate::model_tool::run(raw_args.iter().skip(1).cloned().collect());
    }

    let args = CliArgs::parse();
    if let Err(err) = crate::paths::ensure_default_dirs() {
        eprintln!(
            "Warning: failed to create default data directories under {}: {err}",
            crate::paths::gravimera_dir().display()
        );
    }
    let headless_exit_after = args.headless_exit_after_seconds();
    let mut config = crate::config::load_config_with_override(args.config_path.as_deref());
    args.apply_automation_overrides(&mut config);

    if args.headless {
        run_headless(headless_exit_after, config);
        return;
    }

    if let Err(reason) = render_preflight() {
        eprintln!("{reason}");
        run_headless(headless_exit_after, config);
        return;
    }

    if let Err(reason) = run_rendered_catching_panics(config.clone()) {
        eprintln!("{reason}");
        if let Err(err) = spawn_headless_fallback_process(&raw_args) {
            eprintln!("Failed to respawn headless fallback: {err}");
            run_headless(headless_exit_after, config);
            return;
        }
        let _ = std::io::stderr().flush();
        std::process::exit(0);
    }
}

fn spawn_headless_fallback_process(raw_args: &[String]) -> Result<(), String> {
    let exe =
        std::env::current_exe().map_err(|err| format!("Failed to resolve current exe: {err}"))?;
    let mut cmd = std::process::Command::new(exe);

    // Preserve user args (config/automation flags, etc.), but force `--headless`.
    for arg in raw_args.iter() {
        if arg == "--headless" {
            continue;
        }
        cmd.arg(arg);
    }
    cmd.arg("--headless");

    cmd.spawn()
        .map(|_| ())
        .map_err(|err| format!("Failed to spawn headless process: {err}"))
}

#[derive(Debug, Default, Clone)]
struct CliArgs {
    headless: bool,
    headless_seconds: Option<f32>,
    config_path: Option<PathBuf>,
    automation: bool,
    automation_bind: Option<String>,
    automation_token: Option<String>,
    automation_disable_local_input: Option<bool>,
    automation_pause_on_start: Option<bool>,
}

impl CliArgs {
    fn parse() -> Self {
        let mut parsed = Self::default();
        let mut args = std::env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--headless" => parsed.headless = true,
                "--headless-seconds" => {
                    let Some(value) = args.next() else {
                        eprintln!(
                            "`--headless-seconds` expects a number (example: `--headless-seconds 5`)."
                        );
                        std::process::exit(2);
                    };
                    let seconds: f32 = match value.parse() {
                        Ok(v) => v,
                        Err(_) => {
                            eprintln!(
                                "`--headless-seconds` expects a number (example: `--headless-seconds 5`)."
                            );
                            std::process::exit(2);
                        }
                    };
                    parsed.headless_seconds = Some(seconds);
                }
                "--config" => {
                    let Some(value) = args.next() else {
                        eprintln!("`--config` expects a path to a TOML file (example: `--config ./config.toml`).");
                        std::process::exit(2);
                    };
                    parsed.config_path = Some(PathBuf::from(value));
                }
                "--automation" => parsed.automation = true,
                "--automation-bind" => {
                    let Some(value) = args.next() else {
                        eprintln!("`--automation-bind` expects an address like `127.0.0.1:8791`.");
                        std::process::exit(2);
                    };
                    parsed.automation_bind = Some(value);
                }
                "--automation-token" => {
                    let Some(value) = args.next() else {
                        eprintln!("`--automation-token` expects a token string.");
                        std::process::exit(2);
                    };
                    parsed.automation_token = Some(value);
                }
                "--automation-disable-local-input" => {
                    parsed.automation_disable_local_input = Some(true)
                }
                "--automation-enable-local-input" => {
                    parsed.automation_disable_local_input = Some(false)
                }
                "--automation-pause-on-start" => parsed.automation_pause_on_start = Some(true),
                "--automation-no-pause-on-start" => parsed.automation_pause_on_start = Some(false),
                "--help" | "-h" => {
                    println!(
                        "Gravimera (Bevy shooter demo)\n\n\
                         Usage:\n\
                           cargo run\n\
                           cargo run -- --headless\n\
                           cargo run -- --headless --headless-seconds 2\n\n\
                           cargo run -- model-tool help\n\n\
                         Options:\n\
                           --headless                 Run without rendering (no GPU required)\n\
                           --headless-seconds <secs>  Auto-exit after N seconds; use 0 to run forever\n\
                           --config <path>            Load a specific config TOML (overrides default search)\n\
                           --automation               Enable the local Automation HTTP API\n\
                           --automation-bind <addr>   Bind address (example: 127.0.0.1:8791)\n\
                           --automation-token <tok>   Require Authorization: Bearer <tok>\n\
                           --automation-disable-local-input  Ignore keyboard/mouse input\n\
                           --automation-enable-local-input   Allow keyboard/mouse input\n\
                           --automation-pause-on-start       Start with time paused\n\
                           --automation-no-pause-on-start    Start unpaused\n"
                    );
                    std::process::exit(0);
                }
                other => {
                    eprintln!("Unknown argument: {other}\n(use --help for usage)");
                    std::process::exit(2);
                }
            }
        }

        parsed
    }

    fn apply_automation_overrides(&self, config: &mut crate::config::AppConfig) {
        if self.automation {
            config.automation_enabled = true;
        }
        if let Some(bind) = self.automation_bind.as_ref() {
            config.automation_bind = Some(bind.clone());
        }
        if let Some(token) = self.automation_token.as_ref() {
            config.automation_token = Some(token.clone());
        }
        if let Some(value) = self.automation_disable_local_input {
            config.automation_disable_local_input = value;
        }
        if let Some(value) = self.automation_pause_on_start {
            config.automation_pause_on_start = value;
        }
    }

    fn headless_exit_after_seconds(&self) -> Option<f32> {
        match self.headless_seconds {
            Some(secs) if secs <= 0.0 => None,
            Some(secs) => Some(secs),
            None => Some(DEFAULT_HEADLESS_SECONDS),
        }
    }
}

fn render_preflight() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let backends = wgpu::Backends::METAL;
    #[cfg(not(target_os = "macos"))]
    let backends = wgpu::Backends::all();

    let instance_desc = wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    };
    let instance = wgpu::Instance::new(&instance_desc);

    let adapter = bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }));

    let adapter = match adapter {
        Ok(adapter) => adapter,
        Err(_) => {
            #[cfg(target_os = "macos")]
            return Err("No Metal device detected (wgpu found no Metal adapters).\n\
             This commonly happens in macOS VMs / CI runners that don't expose Metal.\n\
             Falling back to a short headless simulation."
                .to_string());
            #[cfg(not(target_os = "macos"))]
            return Err(
                "No GPU detected (wgpu found no adapters). Running headless simulation instead.\n\
             Tip: on a machine with a compatible GPU/driver, just run without `--headless`."
                    .to_string(),
            );
        }
    };

    let device_desc = wgpu::DeviceDescriptor {
        label: None,
        required_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::default(),
    };

    bevy::tasks::block_on(adapter.request_device(&device_desc))
        .map(|_| ())
        .map_err(|err| {
            format!(
                "GPU adapter was detected, but creating a device failed: {err}\n\
                 Falling back to headless mode.\n\
                 Tip: run `cargo run --bin gpu_probe` for adapter info."
            )
        })
}

fn run_rendered_catching_panics(config: crate::config::AppConfig) -> Result<(), String> {
    let wants_backtrace = std::env::var("RUST_BACKTRACE")
        .map(|v| v != "0")
        .unwrap_or(false);

    if wants_backtrace {
        return std::panic::catch_unwind(|| run_rendered(config))
            .map_err(|_| {
                "Rendered mode crashed. A backtrace was printed above.\n\
                 Falling back to headless mode.\n\
                 Tip: if this always happens, try `cargo run -- --headless`."
                    .to_string()
            })
            .and_then(|exit| {
                exit.is_success().then_some(()).ok_or_else(|| {
                    "Rendered mode exited with an error.\n\
                     Falling back to headless mode.\n\
                     Tip: if this always happens, try `cargo run -- --headless`."
                        .to_string()
                })
            });
    }

    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic payload".to_string());

        eprintln!("Gravimera crashed ({location}): {message}");
        eprintln!("Tip: run `RUST_BACKTRACE=1 cargo run` for a full backtrace.");
    }));

    let result = std::panic::catch_unwind(|| run_rendered(config))
        .map_err(|_| {
            "Rendered mode crashed. Falling back to headless mode.\n\
         Tip: run `RUST_BACKTRACE=1 cargo run` for a full backtrace."
                .to_string()
        })
        .and_then(|exit| {
            exit.is_success().then_some(()).ok_or_else(|| {
                "Rendered mode exited with an error. Falling back to headless mode.\n\
         Tip: run `RUST_BACKTRACE=1 cargo run` for a full backtrace."
                    .to_string()
            })
        });

    std::panic::set_hook(previous_hook);
    result
}

fn run_headless(exit_after_seconds: Option<f32>, config: crate::config::AppConfig) {
    // Shared AI request limiter (Scene Build + Gen3D).
    crate::ai_limiter::set_max_permits(config.gen3d_max_parallel_components.max(1) + 1);

    let mut app = App::new();
    app.insert_resource(config);
    app.add_plugins(
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        ))),
    );
    if !BEVY_LOG_INITIALIZED.swap(true, Ordering::Relaxed) {
        let mut log_plugin = bevy::log::LogPlugin::default();
        log_plugin.filter = format!("{},gravimera::gen3d=debug", log_plugin.filter);
        log_plugin.custom_layer = log_file_layer;
        app.add_plugins(log_plugin);
    }
    app.add_plugins(bevy::transform::TransformPlugin);
    app.add_plugins(crate::automation::AutomationPlugin);
    app.add_systems(Startup, log_file_startup_banner);
    app.init_resource::<ObjectLibrary>();
    app.init_resource::<crate::prefab_descriptors::PrefabDescriptorLibrary>();
    app.init_resource::<crate::object::visuals::MaterialCache>();
    app.init_resource::<crate::object::visuals::PrimitiveMeshCache>();
    app.init_resource::<Game>();
    app.init_resource::<Aim>();
    app.init_resource::<PlayerMuzzles>();
    app.init_resource::<KilledEnemiesThisFrame>();
    app.init_resource::<EnemyKillEffects>();
    app.init_resource::<SpawnRatios>();
    app.add_message::<AppExit>();
    app.add_message::<HealthChangeEvent>();
    app.add_message::<scene_store::SceneSaveRequest>();
    app.insert_resource(headless::HeadlessExit {
        timer: exit_after_seconds.map(|secs| Timer::from_seconds(secs, TimerMode::Once)),
    });
    app.add_systems(PreUpdate, effects::clear_killed_enemies);
    app.add_systems(Startup, headless::setup_headless);
    app.add_systems(
        Update,
        (
            common::tick_cooldowns,
            headless::headless_move_player,
            headless::headless_aim_at_nearest_enemy,
            headless::headless_shooting,
            combat::move_bullets,
            combat::despawn_expired_bullets,
        ),
    );
    app.add_systems(
        Update,
        (
            enemies::spawn_enemies_headless,
            enemies::move_enemies,
            enemies::tick_dog_pounce_cooldowns.after(enemies::move_enemies),
            enemies::dog_try_start_pounce.after(enemies::tick_dog_pounce_cooldowns),
            enemies::update_dog_pounces.after(enemies::dog_try_start_pounce),
            physics::separate_player_from_enemies
                .after(headless::headless_move_player)
                .after(enemies::update_dog_pounces),
            enemies::dog_bite_attack
                .after(physics::separate_player_from_enemies)
                .after(enemies::update_dog_pounces),
            enemies::enemy_shooting.after(enemies::move_enemies),
            enemies::gundam_shooting.after(enemies::move_enemies),
            enemies::move_enemy_projectiles,
            effects::animate_energy_ball_visuals.after(enemies::move_enemy_projectiles),
            enemies::enemy_projectile_player_collisions.after(effects::animate_energy_ball_visuals),
            combat::bullet_enemy_collisions,
            headless::headless_exit_after_timer,
        ),
    );
    app.run();
}

fn run_rendered(config: crate::config::AppConfig) -> AppExit {
    // Shared AI request limiter (Scene Build + Gen3D).
    crate::ai_limiter::set_max_permits(config.gen3d_max_parallel_components.max(1) + 1);

    #[cfg(target_os = "linux")]
    fixup_linux_display_env_for_winit();

    let mut app = App::new();
    app.insert_resource(config);
    app.insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.06)));
    app.init_resource::<ObjectLibrary>();
    app.init_resource::<crate::prefab_descriptors::PrefabDescriptorLibrary>();
    app.init_resource::<crate::object::visuals::MaterialCache>();
    app.init_resource::<crate::object::visuals::PrimitiveMeshCache>();
    app.init_resource::<Game>();
    app.init_resource::<Aim>();
    app.init_resource::<FireControl>();
    app.init_resource::<PlayerMuzzles>();
    app.init_resource::<BuildState>();
    app.init_resource::<SelectionState>();
    app.init_resource::<crate::object_forms::FormCopyState>();
    app.init_resource::<MoveCommandState>();
    app.init_resource::<SlowMoveMode>();
    app.init_resource::<BuildPreview>();
    app.init_resource::<CameraZoom>();
    app.init_resource::<CameraYaw>();
    app.init_resource::<CameraPitch>();
    app.init_resource::<CameraFocus>();
    app.init_resource::<KilledEnemiesThisFrame>();
    app.init_resource::<EnemyKillEffects>();
    app.init_resource::<SpawnRatios>();
    app.init_resource::<CommandConsole>();
    app.init_resource::<crate::gen3d::Gen3dWorkshop>();
    app.init_resource::<crate::gen3d::Gen3dPreview>();
    app.init_resource::<crate::gen3d::Gen3dDraft>();
    app.init_resource::<crate::gen3d::Gen3dAiJob>();
    app.init_resource::<crate::gen3d::Gen3dToolFeedbackHistory>();
    app.init_resource::<crate::scene_authoring_ui::SceneAuthoringUiState>();
    app.init_resource::<crate::model_library_ui::ModelLibraryUiState>();
    app.init_resource::<crate::workspace_ui::WorkspaceUiState>();
    app.init_resource::<crate::world_drag::WorldDragState>();
    app.init_resource::<crate::scene_build_ai::SceneBuildAiRuntime>();
    app.init_resource::<crate::realm::ActiveRealmScene>();
    app.init_resource::<crate::realm::PendingRealmSceneSwitch>();
    app.add_plugins(crate::automation::AutomationPlugin);
    app.add_message::<AppExit>();
    app.add_message::<HealthChangeEvent>();
    app.init_resource::<scene_store::SceneAutosaveState>();
    app.add_message::<scene_store::SceneSaveRequest>();

    let window_plugin = WindowPlugin {
        primary_window: Some(Window {
            title: "Gravimera — Bevy shooter demo".into(),
            resolution: WindowResolution::new(1280, 720),
            ..default()
        }),
        ..default()
    };

    let asset_dir = crate::paths::resolve_assets_dir();
    let asset_plugin = AssetPlugin {
        file_path: asset_dir.to_string_lossy().to_string(),
        ..default()
    };

    let mut log_plugin = bevy::log::LogPlugin::default();
    log_plugin.filter = format!("{},gravimera::gen3d=debug", log_plugin.filter);
    log_plugin.custom_layer = log_file_layer;
    BEVY_LOG_INITIALIZED.store(true, Ordering::Relaxed);

    #[cfg(target_os = "macos")]
    app.add_plugins(
        DefaultPlugins
            .set(window_plugin)
            .set(log_plugin)
            .set(asset_plugin)
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    backends: Some(Backends::METAL),
                    ..default()
                }
                .into(),
                ..default()
            }),
    );

    #[cfg(not(target_os = "macos"))]
    app.add_plugins(
        DefaultPlugins
            .set(window_plugin)
            .set(log_plugin)
            .set(asset_plugin),
    );

    app.init_state::<GameMode>();
    app.init_state::<BuildScene>();
    app.add_systems(PreUpdate, effects::clear_killed_enemies);
    app.add_systems(Update, try_set_primary_window_icon);
    app.add_systems(Startup, log_file_startup_banner);
    app.add_systems(Startup, crate::gen3d::gen3d_load_tool_feedback_history);

    app.add_systems(Startup, setup::setup_rendered);
    app.add_systems(
        Startup,
        crate::build::setup_game_mode_toggle_ui.after(setup::setup_rendered),
    );
    app.add_systems(
        Startup,
        crate::workspace_ui::setup_workspace_ui.after(setup::setup_rendered),
    );
    app.add_systems(
        Startup,
        crate::scene_authoring_ui::setup_scene_authoring_ui.after(setup::setup_rendered),
    );
    app.add_systems(
        Startup,
        crate::model_library_ui::setup_model_library_ui.after(setup::setup_rendered),
    );
    app.add_systems(
        Startup,
        crate::realm::realm_startup_init.after(setup::setup_rendered),
    );
    app.add_systems(
        Startup,
        scene_store::load_scene_dat
            .after(crate::realm::realm_startup_init)
            .after(setup::setup_rendered),
    );
    app.add_systems(
        Update,
        crate::scene_instance_visuals::ensure_scene_instance_visuals_spawned,
    );
    app.add_systems(Update, crate::object_forms::ensure_object_forms_component);
    app.add_systems(
        Update,
        (
            crate::object_forms::tick_form_transform_animations,
            crate::object_forms::sync_form_badges
                .after(crate::object_forms::tick_form_transform_animations),
            crate::object_forms::update_form_badges.after(crate::object_forms::sync_form_badges),
        )
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        crate::object::visuals::apply_pending_scene_overrides
            .after(crate::scene_instance_visuals::ensure_scene_instance_visuals_spawned),
    );
    app.add_systems(
        Update,
        (
            locomotion::ensure_animation_channels_active,
            locomotion::ensure_locomotion_clocks,
            locomotion::update_locomotion_clocks
                .after(locomotion::ensure_locomotion_clocks)
                .after(rts::execute_move_orders)
                .after(enemies::move_enemies)
                .after(physics::separate_enemies)
                .after(physics::separate_player_from_enemies),
        )
            .before(crate::object::visuals::update_part_animations),
    );
    app.add_systems(
        Update,
        locomotion::update_animation_channels_active
            .after(locomotion::ensure_animation_channels_active)
            .after(locomotion::update_locomotion_clocks)
            .after(rts::execute_move_orders)
            .after(rts::update_fire_control)
            .after(combat::unit_attack_execute)
            .before(crate::object::visuals::update_part_animations)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(Update, crate::object::visuals::update_part_animations);
    app.add_systems(
        PostUpdate,
        (
            scene_store::scene_autosave_detect_changes,
            scene_store::scene_save_requests.after(scene_store::scene_autosave_detect_changes),
            scene_store::scene_autosave_tick.after(scene_store::scene_save_requests),
            scene_store::apply_pending_realm_scene_switch.after(scene_store::scene_autosave_tick),
        ),
    );
    app.add_systems(
        PreUpdate,
        (
            crate::scene_authoring_ui::scene_ui_text_input,
            crate::scene_authoring_ui::scene_ui_clear_keyboard_state_when_captured
                .after(crate::scene_authoring_ui::scene_ui_text_input),
        )
            .after(bevy::input::InputSystems),
    );

    app.add_systems(
        Update,
        (
            crate::build::update_game_mode_toggle_button_label,
            crate::workspace_ui::workspace_ui_update_visibility,
            crate::workspace_ui::workspace_ui_dropdown_list_visibility
                .after(crate::workspace_ui::workspace_ui_update_visibility),
            crate::workspace_ui::workspace_ui_update_labels
                .after(crate::workspace_ui::workspace_ui_dropdown_list_visibility),
        ),
    );
    app.add_systems(
        Update,
        (
            crate::workspace_ui::workspace_ui_dropdown_button,
            crate::workspace_ui::workspace_ui_dropdown_option_buttons
                .after(crate::workspace_ui::workspace_ui_dropdown_button),
            crate::workspace_ui::workspace_ui_action_button
                .after(crate::workspace_ui::workspace_ui_dropdown_option_buttons),
            crate::build::handle_game_mode_toggle_button,
        )
            .run_if(console::console_closed)
            .run_if(crate::automation::local_input_enabled),
    );

    app.add_systems(
        Update,
        (
            crate::model_library_ui::model_library_update_visibility,
            crate::model_library_ui::model_library_rebuild_list_ui
                .after(crate::model_library_ui::model_library_update_visibility),
        ),
    );
    app.add_systems(
        Update,
        (
            crate::model_library_ui::model_library_item_button_interactions
                .run_if(crate::automation::local_input_enabled),
            crate::model_library_ui::model_library_drag_update
                .after(crate::model_library_ui::model_library_item_button_interactions)
                .run_if(crate::automation::local_input_enabled),
            crate::model_library_ui::model_library_draw_drag_preview_gizmos
                .after(crate::model_library_ui::model_library_drag_update),
        )
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed),
    );

    app.add_systems(
        Update,
        (
            console::toggle_command_console,
            console::command_console_text_input.after(console::toggle_command_console),
            console::update_command_console_ui.after(console::command_console_text_input),
        )
            .run_if(in_state(BuildScene::Realm))
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(crate::automation::local_input_enabled),
    );

    app.add_systems(
        Update,
        (
            crate::scene_authoring_ui::scene_ui_toggle_button,
            crate::scene_authoring_ui::scene_ui_close_button
                .after(crate::scene_authoring_ui::scene_ui_toggle_button),
            crate::scene_authoring_ui::scene_ui_panel_visibility
                .after(crate::scene_authoring_ui::scene_ui_close_button),
            crate::scene_authoring_ui::scene_ui_sync_active_scene
                .after(crate::scene_authoring_ui::scene_ui_panel_visibility),
            crate::scene_authoring_ui::scene_ui_realm_dropdown_button
                .after(crate::scene_authoring_ui::scene_ui_sync_active_scene),
            crate::scene_authoring_ui::scene_ui_rebuild_realm_list
                .after(crate::scene_authoring_ui::scene_ui_realm_dropdown_button),
            crate::scene_authoring_ui::scene_ui_rebuild_scene_tabs
                .after(crate::scene_authoring_ui::scene_ui_rebuild_realm_list),
            crate::scene_authoring_ui::scene_ui_realm_option_buttons
                .after(crate::scene_authoring_ui::scene_ui_rebuild_scene_tabs),
            crate::scene_authoring_ui::scene_ui_scene_tab_buttons
                .after(crate::scene_authoring_ui::scene_ui_realm_option_buttons),
            crate::scene_authoring_ui::scene_ui_update_realm_scene_button_styles
                .after(crate::scene_authoring_ui::scene_ui_scene_tab_buttons),
            crate::scene_authoring_ui::scene_ui_text_field_focus,
            crate::scene_authoring_ui::scene_ui_action_buttons
                .after(crate::scene_authoring_ui::scene_ui_text_field_focus),
            crate::scene_build_ai::scene_build_ai_poll
                .after(crate::scene_authoring_ui::scene_ui_action_buttons),
            crate::scene_authoring_ui::scene_ui_update_texts
                .after(crate::scene_build_ai::scene_build_ai_poll)
                .after(crate::scene_authoring_ui::scene_ui_update_realm_scene_button_styles),
        )
            .run_if(console::console_closed)
            .run_if(in_state(BuildScene::Realm))
            .run_if(crate::automation::local_input_enabled),
    );
    app.add_systems(
        Update,
        (
            crate::gen3d::handle_gen3d_toggle_button,
            crate::gen3d::update_gen3d_toggle_button_label
                .after(crate::gen3d::handle_gen3d_toggle_button),
        )
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(crate::automation::local_input_enabled),
    );
    app.add_systems(
        Update,
        (
            crate::gen3d::gen3d_handle_drag_and_drop.run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_clear_images_button
                .after(crate::gen3d::gen3d_handle_drag_and_drop)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_prompt_box_focus.run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_side_panel_toggle_button
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_side_tab_buttons.run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_clear_prompt_button.run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_preview_animation_dropdown_button
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_preview_animation_option_buttons
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_collision_toggle_button
                .after(crate::gen3d::gen3d_prompt_box_focus)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_generate_button
                .after(crate::gen3d::gen3d_prompt_box_focus)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_save_button
                .after(crate::gen3d::gen3d_poll_ai_job)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_images_scroll_wheel.run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_status_scroll_wheel.run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_tool_feedback_scroll_wheel
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_prompt_scroll_wheel.run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_preview_animation_dropdown_scroll_wheel
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_preview_orbit_controls
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_copy_tool_feedback_buttons
                .run_if(crate::automation::local_input_enabled),
        )
            .run_if(in_state(BuildScene::Preview)),
    );
    app.add_systems(
        Update,
        (
            crate::gen3d::gen3d_poll_ai_job.after(crate::gen3d::gen3d_generate_button),
            crate::gen3d::gen3d_apply_draft_to_preview
                .after(crate::gen3d::gen3d_poll_ai_job)
                .after(crate::gen3d::gen3d_collision_toggle_button),
            crate::gen3d::gen3d_preview_tick_selected_animation
                .after(crate::gen3d::gen3d_apply_draft_to_preview)
                .before(crate::object::visuals::update_part_animations),
            crate::gen3d::gen3d_update_collision_overlay
                .after(crate::gen3d::gen3d_apply_draft_to_preview)
                .after(crate::gen3d::gen3d_collision_toggle_button),
        )
            .run_if(in_state(BuildScene::Preview)),
    );
    app.add_systems(
        Update,
        (
            crate::gen3d::gen3d_rebuild_images_list_ui
                .after(crate::gen3d::gen3d_handle_drag_and_drop)
                .after(crate::gen3d::gen3d_clear_images_button),
            crate::gen3d::gen3d_update_images_tip_visibility
                .after(crate::gen3d::gen3d_handle_drag_and_drop)
                .after(crate::gen3d::gen3d_clear_images_button),
            crate::gen3d::gen3d_thumbnail_button_open_viewer
                .after(crate::gen3d::gen3d_rebuild_images_list_ui)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_thumbnail_button_style_on_interaction
                .after(crate::gen3d::gen3d_rebuild_images_list_ui)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_update_thumbnail_tooltip
                .after(crate::gen3d::gen3d_thumbnail_button_style_on_interaction)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_image_viewer_keyboard_navigation
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_prompt_text_input
                .after(crate::gen3d::gen3d_prompt_box_focus)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_image_viewer_click_to_close
                .after(crate::gen3d::gen3d_thumbnail_button_open_viewer)
                .run_if(crate::automation::local_input_enabled),
            crate::gen3d::gen3d_update_image_viewer_ui
                .after(crate::gen3d::gen3d_image_viewer_keyboard_navigation)
                .after(crate::gen3d::gen3d_image_viewer_click_to_close),
            crate::gen3d::gen3d_thumbnail_button_style_on_selection
                .after(crate::gen3d::gen3d_image_viewer_keyboard_navigation)
                .after(crate::gen3d::gen3d_image_viewer_click_to_close),
            crate::gen3d::gen3d_update_ui_text
                .after(crate::gen3d::gen3d_prompt_text_input)
                .after(crate::gen3d::gen3d_clear_prompt_button)
                .after(crate::gen3d::gen3d_preview_animation_dropdown_button)
                .after(crate::gen3d::gen3d_preview_animation_option_buttons)
                .after(crate::gen3d::gen3d_collision_toggle_button)
                .after(crate::gen3d::gen3d_poll_ai_job),
            crate::gen3d::gen3d_rebuild_preview_animation_dropdown_options_ui
                .after(crate::gen3d::gen3d_apply_draft_to_preview),
            crate::gen3d::gen3d_update_preview_animation_dropdown_ui
                .after(crate::gen3d::gen3d_update_ui_text),
            crate::gen3d::gen3d_update_side_panel_ui
                .after(crate::gen3d::gen3d_update_ui_text)
                .after(crate::gen3d::gen3d_side_panel_toggle_button),
            crate::gen3d::gen3d_update_side_tab_ui.after(crate::gen3d::gen3d_update_ui_text),
            crate::gen3d::gen3d_update_tool_feedback_text
                .after(crate::gen3d::gen3d_update_side_tab_ui),
        )
            .run_if(in_state(BuildScene::Preview)),
    );
    app.add_systems(
        Update,
        player::update_edge_scroll_cursor_indicator
            .after(console::update_command_console_ui)
            .run_if(in_state(BuildScene::Realm))
            .run_if(crate::automation::local_input_enabled),
    );
    app.add_systems(
        Update,
        (common::restart_game, build::toggle_game_mode)
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(in_state(BuildScene::Realm))
            .run_if(crate::automation::local_input_enabled),
    );
    app.add_systems(
        Update,
        (
            crate::world_drag::world_drag_start
                .before(rts::selection_input)
                .run_if(crate::automation::local_input_enabled),
            crate::world_drag::world_drag_update
                .after(crate::world_drag::world_drag_start)
                .before(rts::selection_input)
                .run_if(crate::automation::local_input_enabled),
        )
            .run_if(in_state(GameMode::Build))
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        (
            rts::selection_input.run_if(crate::automation::local_input_enabled),
            crate::object_forms::object_forms_copy_mode_start_cancel.after(rts::selection_input),
            crate::object_forms::object_forms_copy_mode_update_cursor
                .after(crate::object_forms::object_forms_copy_mode_start_cancel)
                .after(player::update_edge_scroll_cursor_indicator),
            crate::object_forms::object_forms_copy_mode_confirm_on_release
                .after(crate::object_forms::object_forms_copy_mode_update_cursor),
            crate::object_forms::object_forms_tab_switch_selected.after(rts::selection_input),
            rts::toggle_slow_move_mode
                .after(rts::selection_input)
                .before(rts::keyboard_move_input)
                .run_if(crate::automation::local_input_enabled),
            rts::update_selection_box_ui.after(rts::selection_input),
            rts::draw_selected_player_gizmos.after(rts::selection_input),
            rts::unit_animation_hotkeys
                .after(rts::selection_input)
                .before(rts::update_fire_control)
                .run_if(crate::automation::local_input_enabled),
            rts::update_fire_control
                .after(rts::selection_input)
                .run_if(crate::automation::local_input_enabled),
            rts::move_command_input
                .after(rts::selection_input)
                .before(build::build_remove_object)
                .run_if(crate::automation::local_input_enabled),
            rts::keyboard_move_input
                .after(rts::move_command_input)
                .before(rts::execute_move_orders)
                .run_if(crate::automation::local_input_enabled),
            rts::execute_move_orders.after(rts::keyboard_move_input),
            rts::update_unit_aim_yaw_delta
                .after(rts::execute_move_orders)
                .after(rts::update_fire_control),
            combat::ensure_attack_cooldowns.after(rts::selection_input),
            combat::tick_attack_cooldowns.after(combat::ensure_attack_cooldowns),
            combat::unit_attack_execute
                .after(rts::update_unit_aim_yaw_delta)
                .after(combat::tick_attack_cooldowns),
            rts::clear_forced_animation_channel_after_one_shot.after(combat::unit_attack_execute),
        )
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        (
            crate::ui::update_window_title,
            crate::ui::update_health_bars
                .after(rts::execute_move_orders)
                .after(combat::update_lasers),
            crate::ui::update_minimap
                .after(player::camera_follow)
                .after(rts::execute_move_orders)
                .after(physics::separate_player_from_enemies)
                .after(physics::separate_enemies)
                .after(enemies::move_enemies),
        ),
    );
    app.add_systems(
        PostUpdate,
        (
            crate::ui::spawn_health_change_popups.in_set(UiSystems::Content),
            crate::ui::update_health_change_popups
                .after(crate::ui::spawn_health_change_popups)
                .in_set(UiSystems::Content),
        ),
    );
    app.add_systems(
        PostUpdate,
        (
            crate::gen3d::gen3d_update_images_scrollbar_ui,
            crate::gen3d::gen3d_update_prompt_scrollbar_ui,
            crate::gen3d::gen3d_update_status_scrollbar_ui,
            crate::gen3d::gen3d_update_tool_feedback_scrollbar_ui,
        )
            .in_set(UiSystems::Content)
            .run_if(in_state(BuildScene::Preview)),
    );
    app.add_systems(
        Update,
        common::tick_cooldowns
            .run_if(console::console_closed)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        physics::separate_commandables
            .after(rts::execute_move_orders)
            .after(physics::separate_enemies)
            .run_if(console::console_closed)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        physics::separate_player_from_enemies
            .after(physics::separate_commandables)
            .after(rts::execute_move_orders)
            .after(physics::separate_enemies)
            .run_if(console::console_closed)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        player::animate_player_model
            .after(physics::separate_player_from_enemies)
            .run_if(console::console_closed)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        player::camera_zoom_input
            .before(player::camera_follow)
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(in_state(BuildScene::Realm))
            .run_if(crate::automation::local_input_enabled),
    );
    app.add_systems(
        Update,
        (
            player::camera_keyboard_rotate
                .after(player::camera_zoom_input)
                .run_if(crate::automation::local_input_enabled),
            player::camera_edge_pan
                .after(player::camera_keyboard_rotate)
                .run_if(crate::automation::local_input_enabled),
            player::camera_keyboard_pan
                .after(player::camera_edge_pan)
                .run_if(crate::automation::local_input_enabled),
            player::camera_follow_selection.after(player::camera_keyboard_pan),
        )
            .before(player::camera_follow)
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        player::camera_follow
            .before(rts::selection_input)
            .run_if(console::console_closed)
            .run_if(in_state(BuildScene::Realm)),
    );
    app.add_systems(
        Update,
        player::aim_player
            .after(player::camera_follow)
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(in_state(BuildScene::Realm))
            .run_if(crate::automation::local_input_enabled),
    );
    app.add_systems(OnEnter(GameMode::Build), build::enter_build_mode);
    app.add_systems(
        OnEnter(GameMode::Play),
        (
            build::enter_play_mode,
            rts::ensure_default_selection_on_enter_play.after(build::enter_play_mode),
        ),
    );
    app.add_systems(OnEnter(BuildScene::Preview), crate::gen3d::enter_gen3d_mode);
    app.add_systems(OnExit(BuildScene::Preview), crate::gen3d::exit_gen3d_mode);
    app.add_systems(
        OnEnter(GameMode::Play),
        scene_store::request_scene_save_on_enter_play.after(build::enter_play_mode),
    );

    app.add_systems(
        Update,
        (
            build::build_cancel_preview_and_clear_selection,
            build::build_select_object.after(build::build_cancel_preview_and_clear_selection),
            build::build_toggle_fence_axis.after(build::build_select_object),
            build::build_update_preview
                .after(player::aim_player)
                .after(build::build_select_object)
                .after(build::build_toggle_fence_axis)
                .after(build::build_cancel_preview_and_clear_selection),
            build::build_draw_preview_gizmos.after(build::build_update_preview),
            build::build_place_object
                .after(player::aim_player)
                .after(build::build_select_object)
                .after(build::build_toggle_fence_axis)
                .after(build::build_cancel_preview_and_clear_selection),
            build::build_remove_object
                .after(player::aim_player)
                .after(build::build_cancel_preview_and_clear_selection),
            build::build_edit_selected_objects
                .after(rts::selection_input)
                .after(build::build_cancel_preview_and_clear_selection),
            rts::build_unit_hotkeys.after(build::build_edit_selected_objects),
            build::build_remove_selected_objects.after(build::build_edit_selected_objects),
            build::build_draw_selection_gizmos.after(build::build_remove_selected_objects),
        )
            .run_if(in_state(GameMode::Build))
            .run_if(in_state(BuildScene::Realm))
            .run_if(console::console_closed)
            .run_if(crate::scene_authoring_ui::scene_ui_closed)
            .run_if(crate::automation::local_input_enabled),
    );

    app.add_systems(
        Update,
        (
            combat::switch_player_weapon.run_if(crate::automation::local_input_enabled),
            combat::player_fire
                .after(rts::update_fire_control)
                .after(combat::switch_player_weapon),
            combat::update_lasers
                .after(rts::update_fire_control)
                .after(combat::player_fire),
            combat::laser_kill_enemies.after(enemies::move_enemies),
        )
            .run_if(in_state(GameMode::Play).or(in_state(GameMode::Build)))
            .run_if(in_state(BuildScene::Realm))
            .run_if(console::console_closed),
    );
    app.add_systems(
        Update,
        player::update_player_gun_visuals
            .after(common::restart_game)
            .after(combat::switch_player_weapon)
            .after(combat::player_fire)
            .run_if(console::console_closed),
    );
    app.add_systems(
        Update,
        (
            combat::move_bullets,
            combat::bullet_object_collisions.after(combat::move_bullets),
            combat::despawn_expired_bullets.after(combat::move_bullets),
        )
            .run_if(in_state(GameMode::Play).or(in_state(GameMode::Build)))
            .run_if(in_state(BuildScene::Realm))
            .run_if(console::console_closed),
    );
    app.add_systems(
        Update,
        (
            enemies::enemy_shooting.after(enemies::move_enemies),
            enemies::gundam_shooting.after(enemies::move_enemies),
            enemies::move_enemy_projectiles
                .after(enemies::enemy_shooting)
                .after(enemies::gundam_shooting),
            effects::animate_energy_ball_visuals.after(enemies::move_enemy_projectiles),
            enemies::enemy_projectile_object_collisions.after(effects::animate_energy_ball_visuals),
            enemies::enemy_projectile_player_collisions
                .after(enemies::enemy_projectile_object_collisions),
        )
            .run_if(in_state(GameMode::Play))
            .run_if(console::console_closed),
    );
    app.add_systems(
        Update,
        (
            enemies::spawn_enemies,
            enemies::move_enemies.after(enemies::spawn_enemies),
            enemies::tick_dog_pounce_cooldowns.after(enemies::move_enemies),
            enemies::dog_try_start_pounce.after(enemies::tick_dog_pounce_cooldowns),
            enemies::update_dog_pounces.after(enemies::dog_try_start_pounce),
            physics::separate_enemies.after(enemies::update_dog_pounces),
            enemies::animate_enemy_models.after(physics::separate_enemies),
        )
            .run_if(in_state(GameMode::Play))
            .run_if(console::console_closed),
    );
    app.add_systems(
        Update,
        enemies::dog_bite_attack
            .after(physics::separate_player_from_enemies)
            .run_if(in_state(GameMode::Play))
            .run_if(console::console_closed),
    );
    app.add_systems(
        Update,
        (
            combat::bullet_enemy_collisions.after(combat::move_bullets),
            effects::spawn_explosions
                .after(combat::bullet_enemy_collisions)
                .after(combat::laser_kill_enemies),
        )
            .run_if(in_state(GameMode::Play))
            .run_if(console::console_closed),
    );
    app.add_systems(
        Update,
        effects::update_explosion_particles.run_if(console::console_closed),
    );
    app.run()
}

#[cfg(target_os = "linux")]
fn fixup_linux_display_env_for_winit() {
    use std::os::unix::fs::FileTypeExt;
    use std::path::{Path, PathBuf};

    const WSL_REEXEC_MARKER: &str = "GRAVIMERA_WSL_REEXEC";

    fn is_socket(path: &Path) -> bool {
        std::fs::metadata(path)
            .map(|meta| meta.file_type().is_socket())
            .unwrap_or(false)
    }

    fn has_x11_display() -> bool {
        std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
    }

    fn sysroot_lib_dir() -> Option<PathBuf> {
        let home = crate::paths::home_dir()?;
        let sysroot = home.join(".local").join("gravimera-sysroot");

        let multiarch = match std::env::consts::ARCH {
            "x86_64" => Some("x86_64-linux-gnu"),
            "aarch64" => Some("aarch64-linux-gnu"),
            _ => None,
        };

        let candidate = match multiarch {
            Some(multiarch) => sysroot.join("usr").join("lib").join(multiarch),
            None => sysroot.join("usr").join("lib"),
        };
        if candidate.is_dir() {
            return Some(candidate);
        }

        let fallback = sysroot.join("usr").join("lib");
        fallback.is_dir().then_some(fallback)
    }

    fn system_has_library(name: &str) -> bool {
        let multiarch = match std::env::consts::ARCH {
            "x86_64" => Some("x86_64-linux-gnu"),
            "aarch64" => Some("aarch64-linux-gnu"),
            _ => None,
        };

        if let Some(multiarch) = multiarch {
            for base in ["/usr/lib", "/lib"] {
                if Path::new(base).join(multiarch).join(name).exists() {
                    return true;
                }
            }
        }

        for base in ["/usr/lib64", "/lib64", "/usr/lib", "/lib"] {
            if Path::new(base).join(name).exists() {
                return true;
            }
        }

        false
    }

    fn is_winit_backend_forced_to_wayland() -> bool {
        std::env::var("WINIT_UNIX_BACKEND")
            .ok()
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("wayland"))
    }

    fn try_reexec_with_x11_sysroot_libs(lib_dir: &Path) {
        if std::env::var_os(WSL_REEXEC_MARKER).is_some() {
            return;
        }

        let new_ld_library_path = {
            const VAR: &str = "LD_LIBRARY_PATH";
            let lib_dir = lib_dir.to_string_lossy();
            let existing = std::env::var_os(VAR).unwrap_or_default();
            let existing_str = existing.to_string_lossy();
            if existing_str
                .split(':')
                .any(|entry| entry.trim() == lib_dir.trim())
            {
                return;
            }

            if existing_str.trim().is_empty() {
                lib_dir.into_owned()
            } else {
                format!("{lib_dir}:{existing_str}")
            }
        };

        let Ok(exe) = std::env::current_exe() else {
            return;
        };

        eprintln!("WSL display fix: re-execing to apply LD_LIBRARY_PATH for X11 backend support.");

        let mut cmd = std::process::Command::new(exe);
        cmd.args(std::env::args_os().skip(1));
        cmd.env(WSL_REEXEC_MARKER, "1");
        cmd.env("LD_LIBRARY_PATH", new_ld_library_path);
        cmd.env("WINIT_UNIX_BACKEND", "x11");
        cmd.env_remove("WAYLAND_DISPLAY");

        #[allow(unused_imports)]
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        eprintln!("WSL display fix: failed to re-exec process: {err}");
    }

    // WSLg occasionally drops Wayland connections for Vulkan apps. Prefer X11 (XWayland) when
    // available to keep startup stable.
    if crate::platform::is_wsl() && has_x11_display() && !is_winit_backend_forced_to_wayland() {
        let required = ["libxkbcommon-x11.so.0", "libxcb-xkb.so.1"];
        let system_has_required = required.iter().all(|name| system_has_library(name));
        let sysroot_lib_dir = sysroot_lib_dir();
        let sysroot_has_required = sysroot_lib_dir
            .as_ref()
            .is_some_and(|dir| required.iter().all(|name| dir.join(name).exists()));

        if system_has_required || sysroot_has_required {
            let mut can_use_x11 = system_has_required;

            if !system_has_required {
                if let Some(dir) = sysroot_lib_dir.as_ref() {
                    // glibc's dynamic loader does not pick up runtime changes to LD_LIBRARY_PATH
                    // for dlopen(), so we must re-exec before switching to X11 if we're relying on
                    // a user sysroot to provide libxkbcommon-x11 / libxcb-xkb.
                    const VAR: &str = "LD_LIBRARY_PATH";
                    let dir_str = dir.to_string_lossy();
                    let existing = std::env::var_os(VAR).unwrap_or_default();
                    let existing_str = existing.to_string_lossy();
                    let already_available = existing_str
                        .split(':')
                        .any(|entry| entry.trim() == dir_str.trim());

                    if already_available {
                        can_use_x11 = true;
                    } else {
                        try_reexec_with_x11_sysroot_libs(dir);
                        // If re-exec succeeds, we never reach this point.
                        can_use_x11 = false;
                    }
                }
            }

            if can_use_x11 {
                // Some environments set both DISPLAY and WAYLAND_DISPLAY. Prefer X11 for WSL.
                std::env::set_var("WINIT_UNIX_BACKEND", "x11");
                std::env::remove_var("WAYLAND_DISPLAY");
                eprintln!("WSL display fix: forcing X11 backend (unset WAYLAND_DISPLAY).");
                return;
            }
        }
    }

    let Some(wayland_display) = std::env::var_os("WAYLAND_DISPLAY") else {
        return;
    };
    if wayland_display.to_string_lossy().trim().is_empty() {
        return;
    }

    let xdg_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
    let has_wayland_socket = xdg_runtime_dir
        .as_deref()
        .and_then(|dir| {
            if dir.is_empty() {
                return None;
            }
            let path = PathBuf::from(dir).join(&wayland_display);
            is_socket(&path).then_some(path)
        })
        .is_some();
    if has_wayland_socket {
        return;
    }

    let wslg_runtime_dir = Path::new("/mnt/wslg/runtime-dir");
    let wslg_socket_path = wslg_runtime_dir.join(&wayland_display);
    if is_socket(&wslg_socket_path) {
        let previous = xdg_runtime_dir.unwrap_or_default();
        std::env::set_var("XDG_RUNTIME_DIR", wslg_runtime_dir);
        eprintln!(
            "WSL display fix: using XDG_RUNTIME_DIR={} (was {})",
            wslg_runtime_dir.display(),
            previous.to_string_lossy()
        );
        return;
    }

    // Wayland was selected but isn't available. If X11 is available, force winit to use X11.
    if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
        std::env::remove_var("WAYLAND_DISPLAY");
        eprintln!(
            "Display fix: Wayland compositor not reachable; falling back to X11 (unset WAYLAND_DISPLAY)."
        );
    }
}

#[derive(Resource, Clone, Debug)]
struct LogFileStatus {
    requested: PathBuf,
    active: Option<PathBuf>,
    error: Option<String>,
}

#[derive(Resource, Clone)]
pub(crate) struct Gen3dLogSinks {
    enabled: Arc<AtomicU8>,
    inner: Arc<Mutex<Gen3dLogSinksInner>>,
}

#[derive(Default)]
struct Gen3dLogSinksInner {
    global: Option<io::LineWriter<std::fs::File>>,
    gen3d: Option<io::LineWriter<std::fs::File>>,
    gen3d_path: Option<PathBuf>,
}

impl Default for Gen3dLogSinks {
    fn default() -> Self {
        Self {
            enabled: Arc::new(AtomicU8::new(0)),
            inner: Arc::new(Mutex::new(Gen3dLogSinksInner::default())),
        }
    }
}

impl Gen3dLogSinks {
    pub(crate) fn start_gen3d_pass_log(&self, path: PathBuf) -> Result<(), String> {
        let writer = open_log_file_writer(&path)
            .map_err(|err| format!("Failed to open Gen3D log file {}: {err}", path.display()))?;

        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(existing) = guard.gen3d.as_mut() {
            let _ = existing.flush();
        }
        guard.gen3d_path = Some(path);
        guard.gen3d = Some(writer);
        drop(guard);

        self.recompute_enabled();
        Ok(())
    }

    pub(crate) fn stop_gen3d_log(&self) {
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(existing) = guard.gen3d.as_mut() {
            let _ = existing.flush();
        }
        guard.gen3d = None;
        guard.gen3d_path = None;
        drop(guard);

        self.recompute_enabled();
    }

    fn set_global_log(&self, writer: io::LineWriter<std::fs::File>) {
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.global = Some(writer);
        drop(guard);

        self.recompute_enabled();
    }

    fn recompute_enabled(&self) {
        let guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut mask = 0u8;
        if guard.global.is_some() {
            mask |= 1;
        }
        if guard.gen3d.is_some() {
            mask |= 2;
        }
        self.enabled.store(mask, Ordering::Relaxed);
    }
}

fn log_file_startup_banner(status: Option<Res<LogFileStatus>>) {
    let Some(status) = status else {
        return;
    };

    match (status.active.as_ref(), status.error.as_ref()) {
        (Some(active), None) => info!("Logging to file: {}", active.display()),
        (Some(active), Some(err)) => warn!(
            "Requested log file `{}` could not be opened ({err}). Logging to fallback file `{}` instead.",
            status.requested.display(),
            active.display()
        ),
        (None, Some(err)) => warn!(
            "Requested log file `{}` could not be opened ({err}). File logging is disabled.",
            status.requested.display()
        ),
        (None, None) => {}
    }
}

#[derive(Clone)]
struct MultiLogWriter {
    sinks: Gen3dLogSinks,
}

impl io::Write for MultiLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.sinks.enabled.load(Ordering::Relaxed) == 0 {
            return Ok(buf.len());
        }

        let mut guard = match self.sinks.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(writer) = guard.global.as_mut() {
            let _ = writer.write_all(buf);
        }
        if let Some(writer) = guard.gen3d.as_mut() {
            let _ = writer.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.sinks.enabled.load(Ordering::Relaxed) == 0 {
            return Ok(());
        }

        let mut guard = match self.sinks.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(writer) = guard.global.as_mut() {
            let _ = writer.flush();
        }
        if let Some(writer) = guard.gen3d.as_mut() {
            let _ = writer.flush();
        }
        Ok(())
    }
}

fn log_file_layer(app: &mut App) -> Option<bevy::log::BoxedLayer> {
    let config = app
        .world()
        .get_resource::<crate::config::AppConfig>()?
        .clone();
    let sinks = Gen3dLogSinks::default();

    if let Some(requested) = config.log_path.as_ref().cloned() {
        match open_log_file_writer(&requested) {
            Ok(writer) => {
                sinks.set_global_log(writer);
                app.world_mut().insert_resource(LogFileStatus {
                    requested: requested.clone(),
                    active: Some(requested),
                    error: None,
                });
            }
            Err(requested_err) => match fallback_log_path(&requested) {
                Some(fallback_path) => match open_log_file_writer(&fallback_path) {
                    Ok(writer) => {
                        sinks.set_global_log(writer);
                        app.world_mut().insert_resource(LogFileStatus {
                            requested,
                            active: Some(fallback_path.clone()),
                            error: Some(requested_err.to_string()),
                        });
                    }
                    Err(fallback_err) => {
                        app.world_mut().insert_resource(LogFileStatus {
                            requested,
                            active: None,
                            error: Some(format!(
                                "requested: {requested_err}; fallback `{}`: {fallback_err}",
                                fallback_path.display()
                            )),
                        });
                    }
                },
                None => {
                    app.world_mut().insert_resource(LogFileStatus {
                        requested,
                        active: None,
                        error: Some(requested_err.to_string()),
                    });
                }
            },
        }
    }

    app.world_mut().insert_resource(sinks.clone());
    Some(build_multi_log_layer(sinks))
}

fn build_multi_log_layer(sinks: Gen3dLogSinks) -> bevy::log::BoxedLayer {
    let make_writer = move || MultiLogWriter {
        sinks: sinks.clone(),
    };
    let layer = bevy::log::tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(make_writer);

    Box::new(layer)
}

fn fallback_log_path(requested: &Path) -> Option<PathBuf> {
    let file_name = requested
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("gravimera.log"));
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    Some(exe_dir.join(file_name))
}

fn open_log_file_writer(path: &Path) -> io::Result<io::LineWriter<std::fs::File>> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(io::LineWriter::new(file))
}
