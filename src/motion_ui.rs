use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use serde_json::json;

use crate::intelligence::host_plugin::{IntelligenceHostRuntime, StandaloneBrain};
use crate::intelligence::protocol::{DespawnBrainInstanceRequest, PROTOCOL_VERSION};
use crate::intelligence::sidecar_client::SidecarClient;
use crate::motion::{
    motion_rig_v1_for_prefab, AttackPrimaryMotionAlgorithm, IdleMotionAlgorithm,
    MotionAlgorithmController, MoveMotionAlgorithm,
};
use crate::object::registry::ObjectLibrary;
use crate::prefab_descriptors::PrefabDescriptorLibrary;
use crate::threaded_result::{new_shared_result, spawn_worker_thread, take_shared_result, SharedResult};
use crate::types::{Commandable, MoveOrder, ObjectPrefabId, SelectionState};

const PANEL_Z_INDEX: i32 = 940;
const PANEL_WIDTH_PX: f32 = 300.0;
const PANEL_MAX_HEIGHT_PX: f32 = 680.0;
const PANEL_LIST_MIN_HEIGHT_PX: f32 = 260.0;
const DOUBLE_CLICK_MAX_SECS: f32 = 0.35;

#[derive(Debug, Clone, Copy)]
struct MotionAlgorithmUiScrollbarDrag {
    grab_offset: f32,
}

#[derive(Resource, Debug)]
pub(crate) struct MotionAlgorithmUiState {
    pub(crate) open: bool,
    pub(crate) target: Option<Entity>,
    pub(crate) needs_rebuild: bool,
    last_built_target: Option<Entity>,
    last_click_target: Option<Entity>,
    last_click_time_secs: f32,
    scrollbar_drag: Option<MotionAlgorithmUiScrollbarDrag>,
    brain_modules: Vec<String>,
    brain_modules_loading: bool,
    brain_modules_error: Option<String>,
    brain_modules_job: Option<SharedResult<Vec<String>, String>>,
    brain_modules_fetch_requested: bool,
}

impl Default for MotionAlgorithmUiState {
    fn default() -> Self {
        Self {
            open: false,
            target: None,
            needs_rebuild: false,
            last_built_target: None,
            last_click_target: None,
            last_click_time_secs: -1.0,
            scrollbar_drag: None,
            brain_modules: Vec::new(),
            brain_modules_loading: false,
            brain_modules_error: None,
            brain_modules_job: None,
            brain_modules_fetch_requested: false,
        }
    }
}

impl MotionAlgorithmUiState {
    pub(crate) fn record_click_and_check_double(&mut self, entity: Entity, now_secs: f32) -> bool {
        let is_double = self.last_click_target.is_some_and(|prev| prev == entity)
            && now_secs.is_finite()
            && self.last_click_time_secs.is_finite()
            && (now_secs - self.last_click_time_secs) <= DOUBLE_CLICK_MAX_SECS;

        self.last_click_target = Some(entity);
        self.last_click_time_secs = now_secs;
        is_double
    }

    pub(crate) fn open_for(&mut self, entity: Entity) {
        self.open = true;
        self.target = Some(entity);
        self.needs_rebuild = true;
        self.brain_modules_fetch_requested = true;
        self.brain_modules_loading = false;
        self.brain_modules_error = None;
        self.brain_modules_job = None;
        self.brain_modules.clear();
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.target = None;
        self.needs_rebuild = false;
        self.last_built_target = None;
        self.scrollbar_drag = None;
        self.brain_modules_fetch_requested = false;
        self.brain_modules_loading = false;
        self.brain_modules_error = None;
        self.brain_modules_job = None;
        self.brain_modules.clear();
    }
}

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiRoot;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiTitle;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiSubtitle;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiList;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiListItem;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiScrollPanel;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiScrollbarTrack;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiScrollbarThumb;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MotionAlgorithmUiChannel {
    Idle,
    Move,
    AttackPrimary,
}

#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct MotionAlgorithmUiButton {
    pub(crate) channel: MotionAlgorithmUiChannel,
    pub(crate) algorithm_id: &'static str,
}

#[derive(Component, Clone, Debug)]
pub(crate) struct MetaBrainUiButton {
    pub(crate) module_id: Option<String>,
}

pub(crate) fn setup_motion_algorithm_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(44.0),
                right: Val::Px(10.0),
                width: Val::Px(PANEL_WIDTH_PX),
                max_height: Val::Px(PANEL_MAX_HEIGHT_PX),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.88)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
            Outline {
                width: Val::Px(1.0),
                color: Color::srgba(0.25, 0.25, 0.30, 0.75),
                offset: Val::Px(0.0),
            },
            ZIndex(PANEL_Z_INDEX),
            Visibility::Hidden,
            MotionAlgorithmUiRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("Meta"),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.95, 0.97)),
                MotionAlgorithmUiTitle,
            ));

            root.spawn((
                Text::new(""),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.80, 0.86)),
                MotionAlgorithmUiSubtitle,
            ));

            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    min_height: Val::Px(PANEL_LIST_MIN_HEIGHT_PX),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                row.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        min_height: Val::Px(0.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    ScrollPosition::default(),
                    MotionAlgorithmUiScrollPanel,
                ))
                .with_children(|scroll| {
                    scroll.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        MotionAlgorithmUiList,
                    ));
                });

                row.spawn((
                    Node {
                        width: Val::Px(8.0),
                        height: Val::Percent(100.0),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.45)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    Visibility::Hidden,
                    MotionAlgorithmUiScrollbarTrack,
                ))
                .with_children(|track| {
                    track.spawn((
                        Button,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(1.0),
                            right: Val::Px(1.0),
                            top: Val::Px(0.0),
                            height: Val::Px(18.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.20)),
                        MotionAlgorithmUiScrollbarThumb,
                    ));
                });
            });

            root.spawn((
                Text::new("Tip: double-click a unit's selection circle to open Meta panel."),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.65, 0.65, 0.72)),
            ));
        });
}

pub(crate) fn motion_algorithm_ui_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<MotionAlgorithmUiState>,
) {
    if !state.open {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        state.close();
    }
}

pub(crate) fn motion_algorithm_ui_update(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    descriptors: Res<PrefabDescriptorLibrary>,
    runtime: Res<IntelligenceHostRuntime>,
    mut state: ResMut<MotionAlgorithmUiState>,
    roots: Query<(
        Option<&ObjectPrefabId>,
        Option<&MotionAlgorithmController>,
        Option<&StandaloneBrain>,
    )>,
    mut roots_ui: Query<&mut Visibility, With<MotionAlgorithmUiRoot>>,
    mut subtitle: Query<&mut Text, With<MotionAlgorithmUiSubtitle>>,
    list: Query<Entity, With<MotionAlgorithmUiList>>,
    existing_items: Query<Entity, With<MotionAlgorithmUiListItem>>,
    mut scroll_panels: Query<&mut ScrollPosition, With<MotionAlgorithmUiScrollPanel>>,
) {
    let Ok(mut visibility) = roots_ui.single_mut() else {
        return;
    };

    let Some(target) = state.target else {
        state.open = false;
        *visibility = Visibility::Hidden;
        return;
    };
    if !state.open {
        *visibility = Visibility::Hidden;
        return;
    }

    let Ok((prefab_id, controller, brain)) = roots.get(target) else {
        state.close();
        *visibility = Visibility::Hidden;
        return;
    };
    let Some(prefab_id) = prefab_id else {
        state.close();
        *visibility = Visibility::Hidden;
        return;
    };

    *visibility = Visibility::Visible;

    let target_changed = state.last_built_target != state.target;
    if target_changed {
        state.needs_rebuild = true;
        state.scrollbar_drag = None;
        if let Ok(mut scroll_pos) = scroll_panels.single_mut() {
            scroll_pos.y = 0.0;
        }
    }

    let current_idle = controller
        .map(|c| c.idle_algorithm)
        .unwrap_or(IdleMotionAlgorithm::None);
    let current_move = controller
        .map(|c| c.move_algorithm)
        .unwrap_or(MoveMotionAlgorithm::None);
    let current_attack = controller
        .map(|c| c.attack_primary_algorithm)
        .unwrap_or(AttackPrimaryMotionAlgorithm::None);
    let rig = motion_rig_v1_for_prefab(prefab_id.0, &descriptors)
        .ok()
        .flatten();
    let attack_kind = library
        .get(prefab_id.0)
        .and_then(|def| def.attack.as_ref())
        .map(|a| a.kind);

    if let Ok(mut subtitle) = subtitle.single_mut() {
        let descriptor = descriptors.get(prefab_id.0);
        let label = descriptor
            .and_then(|d| d.label.as_deref())
            .or_else(|| library.get(prefab_id.0).map(|d| d.label.as_ref()))
            .unwrap_or("<unknown>");
        let gen3d_run_id = descriptor
            .and_then(|d| d.provenance.as_ref())
            .and_then(|p| p.gen3d.as_ref())
            .and_then(|g| g.run_id.as_deref());
        let gen3d_run_id = gen3d_run_id.or_else(|| {
            descriptor
                .and_then(|d| d.interfaces.as_ref())
                .and_then(|i| i.extra.get("motion_roles_v1"))
                .and_then(|v| v.get("applies_to"))
                .and_then(|v| v.get("run_id"))
                .and_then(|v| v.as_str())
        });
        let rig_kind = rig.as_ref().map(|r| r.kind_str()).unwrap_or("<none>");
        let brain_label = match brain {
            Some(brain) => brain.module_id.as_str(),
            None => "fallback",
        };
        let brain_error = brain
            .and_then(|b| b.last_error.as_deref())
            .filter(|v| !v.trim().is_empty())
            .map(|v| format!(" (error: {v})"))
            .unwrap_or_default();
        let gen3d_line = gen3d_run_id
            .map(|run_id| format!("\nGen3D run: {run_id}"))
            .unwrap_or_default();
        *subtitle = Text::new(format!(
            "Target: {label}{gen3d_line}\nBrain: {brain_label}{brain_error}\nRig: {rig_kind}\nIdle: {}\nMove: {}\nAttack: {}",
            current_idle.id_str(),
            current_move.id_str(),
            current_attack.id_str(),
        ));
    }

    if let Some(job) = state.brain_modules_job.as_ref() {
        if let Some(result) = take_shared_result(job) {
            state.brain_modules_job = None;
            state.brain_modules_loading = false;
            match result {
                Ok(mut modules) => {
                    modules.sort();
                    modules.dedup();
                    state.brain_modules = modules;
                    state.brain_modules_error = None;
                }
                Err(err) => {
                    state.brain_modules.clear();
                    state.brain_modules_error = Some(err);
                }
            }
            state.needs_rebuild = true;
        }
    }

    if state.brain_modules_fetch_requested
        && state.brain_modules_job.is_none()
        && !state.brain_modules_loading
        && runtime.enabled
    {
        state.brain_modules_fetch_requested = false;

        let Some(addr) = runtime.service_addr else {
            state.brain_modules_error = Some("Intelligence service enabled but missing addr.".into());
            state.needs_rebuild = true;
            return;
        };
        let token = runtime.token.clone();

        let shared = new_shared_result::<Vec<String>, String>();
        let thread_name = "gravimera_meta_brain_modules".to_string();
        let _ = spawn_worker_thread(
            thread_name,
            shared.clone(),
            move || {
                let client = SidecarClient::new(addr, token);
                let resp = client.modules().map_err(|err| err.to_string())?;
                if resp.protocol_version != crate::intelligence::protocol::PROTOCOL_VERSION {
                    return Err(format!(
                        "Protocol mismatch: host={} service={}",
                        crate::intelligence::protocol::PROTOCOL_VERSION,
                        resp.protocol_version
                    ));
                }
                Ok(resp.modules.into_iter().map(|m| m.module_id).collect())
            },
            |_| {},
        );

        state.brain_modules_job = Some(shared);
        state.brain_modules_loading = true;
        state.brain_modules_error = None;
        state.needs_rebuild = true;
    }

    if !state.needs_rebuild {
        return;
    }
    state.needs_rebuild = false;
    state.last_built_target = state.target;

    let Ok(list_entity) = list.single() else {
        return;
    };
    for entity in &existing_items {
        commands.entity(entity).try_despawn();
    }

    let idle_algorithms = rig
        .as_ref()
        .map(|r| r.applicable_idle_algorithms())
        .unwrap_or_else(|| vec![IdleMotionAlgorithm::None]);
    let move_algorithms = rig
        .as_ref()
        .map(|r| r.applicable_move_algorithms())
        .unwrap_or_else(|| vec![MoveMotionAlgorithm::None]);
    let attack_algorithms = rig
        .as_ref()
        .map(|r| r.applicable_attack_primary_algorithms(attack_kind))
        .unwrap_or_else(|| vec![AttackPrimaryMotionAlgorithm::None]);

    let brain_remote_enabled = runtime.enabled;
    let brain_modules_loading = state.brain_modules_loading;
    let brain_modules_error = state.brain_modules_error.clone();
    let brain_modules = state.brain_modules.clone();

    commands.entity(list_entity).with_children(|list| {
        let section_font = TextFont {
            font_size: 12.0,
            ..default()
        };
        let section_color = TextColor(Color::srgb(0.75, 0.75, 0.82));
        let button_font = TextFont {
            font_size: 14.0,
            ..default()
        };
        let button_color = TextColor(Color::srgb(0.92, 0.92, 0.96));
        let button_bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75));
        let button_border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));

        list.spawn((
            Text::new("Brain"),
            section_font.clone(),
            section_color,
            MotionAlgorithmUiListItem,
        ));

        list.spawn((
            Button,
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            button_bg,
            button_border,
            MotionAlgorithmUiListItem,
            MetaBrainUiButton { module_id: None },
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("Fallback (default)"),
                button_font.clone(),
                button_color,
            ));
        });

        if !brain_remote_enabled {
            list.spawn((
                Text::new(
                    "Intelligence service disabled (set [intelligence_service] mode = \"embedded\" | \"sidecar\" in config.toml).",
                ),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.70, 0.76)),
                MotionAlgorithmUiListItem,
            ));
        } else if brain_modules_loading {
            list.spawn((
                Text::new("Loading brain modules..."),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.70, 0.76)),
                MotionAlgorithmUiListItem,
            ));
        } else if let Some(err) = brain_modules_error.as_deref() {
            list.spawn((
                Text::new(format!("Failed to load brain modules: {err}")),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.86, 0.70, 0.70)),
                MotionAlgorithmUiListItem,
            ));
        } else if brain_modules.is_empty() {
            list.spawn((
                Text::new("No brain modules reported by the service."),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.70, 0.76)),
                MotionAlgorithmUiListItem,
            ));
        } else {
            for module_id in &brain_modules {
                list.spawn((
                    Button,
                    Node {
                        width: Val::Percent(100.0),
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    button_bg,
                    button_border,
                    MotionAlgorithmUiListItem,
                    MetaBrainUiButton {
                        module_id: Some(module_id.clone()),
                    },
                ))
                .with_children(|b| {
                    b.spawn((Text::new(module_id.clone()), button_font.clone(), button_color));
                });
            }
        }

        list.spawn((
            Text::new("Idle"),
            section_font.clone(),
            section_color,
            MotionAlgorithmUiListItem,
        ));
        for algorithm in idle_algorithms {
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                button_bg,
                button_border,
                MotionAlgorithmUiListItem,
                MotionAlgorithmUiButton {
                    channel: MotionAlgorithmUiChannel::Idle,
                    algorithm_id: algorithm.id_str(),
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(algorithm.label()),
                    button_font.clone(),
                    button_color,
                ));
            });
        }

        list.spawn((
            Text::new("Move"),
            section_font.clone(),
            section_color,
            MotionAlgorithmUiListItem,
        ));
        for algorithm in move_algorithms {
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                button_bg,
                button_border,
                MotionAlgorithmUiListItem,
                MotionAlgorithmUiButton {
                    channel: MotionAlgorithmUiChannel::Move,
                    algorithm_id: algorithm.id_str(),
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(algorithm.label()),
                    button_font.clone(),
                    button_color,
                ));
            });
        }

        list.spawn((
            Text::new("Attack"),
            section_font,
            section_color,
            MotionAlgorithmUiListItem,
        ));
        for algorithm in attack_algorithms {
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                button_bg,
                button_border,
                MotionAlgorithmUiListItem,
                MotionAlgorithmUiButton {
                    channel: MotionAlgorithmUiChannel::AttackPrimary,
                    algorithm_id: algorithm.id_str(),
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(algorithm.label()),
                    button_font.clone(),
                    button_color,
                ));
            });
        }
    });
}

pub(crate) fn motion_algorithm_ui_update_scrollbar_ui(
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<MotionAlgorithmUiScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<MotionAlgorithmUiScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<MotionAlgorithmUiScrollbarThumb>>,
) {
    let Ok((panel, mut scroll_pos)) = panels.single_mut() else {
        return;
    };
    let Ok((track_node, mut track_vis)) = tracks.single_mut() else {
        return;
    };
    let Ok(mut thumb) = thumbs.single_mut() else {
        return;
    };

    let panel_scale = panel.inverse_scale_factor();
    let track_scale = track_node.inverse_scale_factor();
    let viewport_h = panel.size.y.max(0.0) * panel_scale;
    let content_h = panel.content_size.y.max(0.0) * panel_scale;
    let track_h = track_node.size.y.max(1.0) * track_scale;

    if viewport_h < 1.0 || content_h < 1.0 {
        *track_vis = Visibility::Hidden;
        return;
    }

    if content_h <= viewport_h + 0.5 {
        *track_vis = Visibility::Hidden;
        scroll_pos.y = 0.0;
        thumb.top = Val::Px(0.0);
        thumb.height = Val::Px(track_h);
        return;
    }

    *track_vis = Visibility::Inherited;

    let max_scroll = (content_h - viewport_h).max(1.0);
    scroll_pos.y = scroll_pos.y.clamp(0.0, max_scroll);
    let scroll_y = scroll_pos.y;

    let min_thumb_h = 14.0;
    let thumb_h = (viewport_h * viewport_h / content_h).clamp(min_thumb_h, track_h);
    let max_thumb_top = (track_h - thumb_h).max(0.0);
    let thumb_top = (max_thumb_top * (scroll_y / max_scroll)).clamp(0.0, max_thumb_top);

    thumb.top = Val::Px(thumb_top);
    thumb.height = Val::Px(thumb_h);
}

pub(crate) fn motion_algorithm_ui_scroll_wheel(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<MouseWheel>,
    state: Res<MotionAlgorithmUiState>,
    roots: Query<(&ComputedNode, &UiGlobalTransform, &Visibility), With<MotionAlgorithmUiRoot>>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<MotionAlgorithmUiScrollPanel>>,
) {
    if !state.open || state.scrollbar_drag.is_some() {
        for _ in mouse_wheel.read() {}
        return;
    }

    let Ok(window) = windows.single() else {
        for _ in mouse_wheel.read() {}
        return;
    };
    let Some(cursor) = window.physical_cursor_position() else {
        for _ in mouse_wheel.read() {}
        return;
    };

    let Ok((root_node, root_transform, root_vis)) = roots.single() else {
        for _ in mouse_wheel.read() {}
        return;
    };
    if *root_vis == Visibility::Hidden || !root_node.contains_point(*root_transform, cursor) {
        for _ in mouse_wheel.read() {}
        return;
    }

    let Ok((panel_node, mut scroll)) = panels.single_mut() else {
        for _ in mouse_wheel.read() {}
        return;
    };

    let mut delta_lines = 0.0f32;
    for ev in mouse_wheel.read() {
        let lines = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y / 120.0,
        };
        delta_lines += lines;
    }
    if delta_lines.abs() < 1e-4 {
        return;
    }

    // `ScrollPosition` is in logical pixels. Approximate a line step as 24px.
    let delta_px = delta_lines * 24.0;

    let panel_scale = panel_node.inverse_scale_factor();
    let viewport_h = panel_node.size.y.max(0.0) * panel_scale;
    let content_h = panel_node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
        scroll.y = 0.0;
        return;
    }
    let max_scroll = (content_h - viewport_h).max(0.0);
    scroll.y = (scroll.y - delta_px).clamp(0.0, max_scroll);
}

pub(crate) fn motion_algorithm_ui_scrollbar_drag(
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<MotionAlgorithmUiState>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<MotionAlgorithmUiScrollPanel>>,
    tracks: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<MotionAlgorithmUiScrollbarTrack>,
    >,
    thumbs: Query<(&Interaction, &ComputedNode, &Node), With<MotionAlgorithmUiScrollbarThumb>>,
) {
    if !state.open {
        state.scrollbar_drag = None;
        return;
    }

    if !mouse_buttons.pressed(MouseButton::Left) {
        state.scrollbar_drag = None;
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.physical_cursor_position() else {
        return;
    };
    let Ok((panel_node, mut scroll)) = panels.single_mut() else {
        return;
    };
    let Ok((track_node, track_transform, track_vis)) = tracks.single() else {
        return;
    };
    if *track_vis == Visibility::Hidden {
        state.scrollbar_drag = None;
        return;
    }
    let Ok((interaction, thumb_node, thumb_layout)) = thumbs.single() else {
        return;
    };

    if state.scrollbar_drag.is_none() && *interaction == Interaction::Pressed {
        if let Some(local) = track_transform
            .try_inverse()
            .map(|transform| transform.transform_point2(cursor))
        {
            let track_scale = track_node.inverse_scale_factor();
            let thumb_scale = thumb_node.inverse_scale_factor();
            let cursor_in_track = (local.y + track_node.size.y * 0.5) * track_scale;
            let thumb_top = match thumb_layout.top {
                Val::Px(value) => value,
                _ => 0.0,
            };
            let grab_offset =
                (cursor_in_track - thumb_top).clamp(0.0, thumb_node.size.y.max(1.0) * thumb_scale);
            state.scrollbar_drag = Some(MotionAlgorithmUiScrollbarDrag { grab_offset });
        }
    }

    let Some(drag) = state.scrollbar_drag else {
        return;
    };

    let panel_scale = panel_node.inverse_scale_factor();
    let viewport_h = panel_node.size.y.max(0.0) * panel_scale;
    let content_h = panel_node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
        return;
    }

    let track_scale = track_node.inverse_scale_factor();
    let thumb_scale = thumb_node.inverse_scale_factor();
    let track_h = track_node.size.y.max(1.0) * track_scale;
    let thumb_h = thumb_node.size.y.max(1.0) * thumb_scale;
    let max_thumb_top = (track_h - thumb_h).max(0.0);
    if max_thumb_top <= 1e-4 {
        scroll.y = 0.0;
        return;
    }
    let max_scroll = (content_h - viewport_h).max(1.0);

    let Some(local) = track_transform
        .try_inverse()
        .map(|transform| transform.transform_point2(cursor))
    else {
        return;
    };
    let cursor_in_track = ((local.y + track_node.size.y * 0.5) * track_scale).clamp(0.0, track_h);
    let thumb_top = (cursor_in_track - drag.grab_offset).clamp(0.0, max_thumb_top);

    scroll.y = (thumb_top / max_thumb_top * max_scroll).clamp(0.0, max_scroll);
}

pub(crate) fn motion_algorithm_ui_button_styles(
    state: Res<MotionAlgorithmUiState>,
    roots: Query<Option<&MotionAlgorithmController>>,
    mut buttons: Query<
        (
            &Interaction,
            &MotionAlgorithmUiButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<Button>,
    >,
) {
    let Some(target) = state.target else {
        return;
    };
    if !state.open {
        return;
    }

    let selected_idle = roots
        .get(target)
        .ok()
        .flatten()
        .map(|c| c.idle_algorithm)
        .unwrap_or(IdleMotionAlgorithm::None)
        .id_str();
    let selected_move = roots
        .get(target)
        .ok()
        .flatten()
        .map(|c| c.move_algorithm)
        .unwrap_or(MoveMotionAlgorithm::None)
        .id_str();
    let selected_attack = roots
        .get(target)
        .ok()
        .flatten()
        .map(|c| c.attack_primary_algorithm)
        .unwrap_or(AttackPrimaryMotionAlgorithm::None)
        .id_str();

    for (interaction, button, mut bg, mut border) in &mut buttons {
        let selected_id = match button.channel {
            MotionAlgorithmUiChannel::Idle => selected_idle,
            MotionAlgorithmUiChannel::Move => selected_move,
            MotionAlgorithmUiChannel::AttackPrimary => selected_attack,
        };
        let is_selected = button.algorithm_id == selected_id;
        let (base_bg, base_border) = if is_selected {
            (
                Color::srgba(0.10, 0.10, 0.14, 0.88),
                Color::srgba(0.45, 0.45, 0.60, 0.85),
            )
        } else {
            (
                Color::srgba(0.05, 0.05, 0.06, 0.75),
                Color::srgba(0.25, 0.25, 0.30, 0.65),
            )
        };
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(base_bg);
                *border = BorderColor::all(base_border);
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(
                    (base_bg.to_srgba().red + 0.02).min(1.0),
                    (base_bg.to_srgba().green + 0.02).min(1.0),
                    (base_bg.to_srgba().blue + 0.03).min(1.0),
                    base_bg.to_srgba().alpha,
                ));
                *border = BorderColor::all(Color::srgba(
                    (base_border.to_srgba().red + 0.08).min(1.0),
                    (base_border.to_srgba().green + 0.08).min(1.0),
                    (base_border.to_srgba().blue + 0.10).min(1.0),
                    base_border.to_srgba().alpha,
                ));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(
                    (base_bg.to_srgba().red + 0.05).min(1.0),
                    (base_bg.to_srgba().green + 0.05).min(1.0),
                    (base_bg.to_srgba().blue + 0.07).min(1.0),
                    base_bg.to_srgba().alpha,
                ));
                *border = BorderColor::all(Color::srgba(
                    (base_border.to_srgba().red + 0.12).min(1.0),
                    (base_border.to_srgba().green + 0.12).min(1.0),
                    (base_border.to_srgba().blue + 0.14).min(1.0),
                    base_border.to_srgba().alpha,
                ));
            }
        }
    }
}

pub(crate) fn meta_brain_ui_button_styles(
    state: Res<MotionAlgorithmUiState>,
    brains: Query<Option<&StandaloneBrain>>,
    mut buttons: Query<
        (
            &Interaction,
            &MetaBrainUiButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<Button>,
    >,
) {
    let Some(target) = state.target else {
        return;
    };
    if !state.open {
        return;
    }

    let selected_module = brains
        .get(target)
        .ok()
        .flatten()
        .map(|b| b.module_id.as_str())
        .map(|v| v.to_string());

    for (interaction, button, mut bg, mut border) in &mut buttons {
        let is_selected = match (selected_module.as_deref(), button.module_id.as_deref()) {
            (None, None) => true,
            (Some(selected), Some(module_id)) => selected == module_id,
            _ => false,
        };
        let (base_bg, base_border) = if is_selected {
            (
                Color::srgba(0.10, 0.10, 0.14, 0.88),
                Color::srgba(0.45, 0.45, 0.60, 0.85),
            )
        } else {
            (
                Color::srgba(0.05, 0.05, 0.06, 0.75),
                Color::srgba(0.25, 0.25, 0.30, 0.65),
            )
        };
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(base_bg);
                *border = BorderColor::all(base_border);
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(
                    (base_bg.to_srgba().red + 0.02).min(1.0),
                    (base_bg.to_srgba().green + 0.02).min(1.0),
                    (base_bg.to_srgba().blue + 0.03).min(1.0),
                    base_bg.to_srgba().alpha,
                ));
                *border = BorderColor::all(Color::srgba(
                    (base_border.to_srgba().red + 0.08).min(1.0),
                    (base_border.to_srgba().green + 0.08).min(1.0),
                    (base_border.to_srgba().blue + 0.10).min(1.0),
                    base_border.to_srgba().alpha,
                ));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(
                    (base_bg.to_srgba().red + 0.05).min(1.0),
                    (base_bg.to_srgba().green + 0.05).min(1.0),
                    (base_bg.to_srgba().blue + 0.07).min(1.0),
                    base_bg.to_srgba().alpha,
                ));
                *border = BorderColor::all(Color::srgba(
                    (base_border.to_srgba().red + 0.12).min(1.0),
                    (base_border.to_srgba().green + 0.12).min(1.0),
                    (base_border.to_srgba().blue + 0.14).min(1.0),
                    base_border.to_srgba().alpha,
                ));
            }
        }
    }
}

pub(crate) fn motion_algorithm_ui_button_clicks(
    mut commands: Commands,
    mut state: ResMut<MotionAlgorithmUiState>,
    selection: Res<SelectionState>,
    roots: Query<(&ObjectPrefabId, Option<&MotionAlgorithmController>), With<Commandable>>,
    mut buttons: Query<(&Interaction, &MotionAlgorithmUiButton), Changed<Interaction>>,
) {
    if !state.open {
        return;
    }
    let Some(target) = state.target else {
        return;
    };
    let Ok((target_prefab, _)) = roots.get(target) else {
        return;
    };

    for (interaction, button) in &mut buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let channel = button.channel;
        let alg_label = match channel {
            MotionAlgorithmUiChannel::Idle => button.algorithm_id,
            MotionAlgorithmUiChannel::Move => button.algorithm_id,
            MotionAlgorithmUiChannel::AttackPrimary => button.algorithm_id,
        };

        let mut updated = 0usize;
        let mut targets: Vec<Entity> = selection.selected.iter().copied().collect();
        if !selection.selected.contains(&target) {
            targets.push(target);
        }

        for entity in targets {
            let Ok((prefab_id, controller)) = roots.get(entity) else {
                continue;
            };
            if prefab_id.0 != target_prefab.0 {
                continue;
            }

            let mut next = controller.copied().unwrap_or_default();
            match channel {
                MotionAlgorithmUiChannel::Idle => {
                    let Some(parsed) = IdleMotionAlgorithm::parse(button.algorithm_id) else {
                        continue;
                    };
                    next.idle_algorithm = parsed;
                }
                MotionAlgorithmUiChannel::Move => {
                    let Some(parsed) = MoveMotionAlgorithm::parse(button.algorithm_id) else {
                        continue;
                    };
                    next.move_algorithm = parsed;
                }
                MotionAlgorithmUiChannel::AttackPrimary => {
                    let Some(parsed) = AttackPrimaryMotionAlgorithm::parse(button.algorithm_id)
                    else {
                        continue;
                    };
                    next.attack_primary_algorithm = parsed;
                }
            }

            commands.entity(entity).insert(next);
            updated += 1;
        }

        info!(
            "Motion: set {:?}={} for {} unit(s) of prefab {}",
            channel,
            alg_label,
            updated,
            uuid::Uuid::from_u128(target_prefab.0)
        );

        state.needs_rebuild = true;
    }
}

pub(crate) fn meta_brain_ui_button_clicks(
    mut commands: Commands,
    mut state: ResMut<MotionAlgorithmUiState>,
    runtime: Res<IntelligenceHostRuntime>,
    selection: Res<SelectionState>,
    units: Query<(), With<Commandable>>,
    brains: Query<Option<&StandaloneBrain>>,
    mut buttons: Query<(&Interaction, &MetaBrainUiButton), Changed<Interaction>>,
) {
    if !state.open {
        return;
    }
    let Some(target) = state.target else {
        return;
    };

    for (interaction, button) in &mut buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let mut updated = 0usize;
        let mut targets: Vec<Entity> = selection.selected.iter().copied().collect();
        if !selection.selected.contains(&target) {
            targets.push(target);
        }

        for entity in targets {
            if units.get(entity).is_err() {
                continue;
            }

            match button.module_id.as_deref() {
                None => {
                    if runtime.enabled {
                        let mut instance_ids = Vec::new();
                        if let Ok(Some(brain)) = brains.get(entity) {
                            if let Some(instance_id) = brain.brain_instance_id.clone() {
                                instance_ids.push(instance_id);
                            }
                        }
                        if !instance_ids.is_empty() {
                            if let Some(addr) = runtime.service_addr {
                                let client = SidecarClient::new(addr, runtime.token.clone());
                                let _ = client.despawn(DespawnBrainInstanceRequest {
                                    protocol_version: PROTOCOL_VERSION,
                                    brain_instance_ids: instance_ids,
                                });
                            }
                        }
                    }
                    commands.entity(entity).remove::<StandaloneBrain>();
                    commands.entity(entity).remove::<MoveOrder>();
                    updated += 1;
                }
                Some(module_id) => {
                    if !runtime.enabled {
                        continue;
                    }

                    commands.entity(entity).insert(StandaloneBrain {
                        module_id: module_id.to_string(),
                        config: json!({}),
                        capabilities: vec!["brain.move".into(), "brain.combat".into()],
                        brain_instance_id: None,
                        next_tick_due: 0,
                        last_error: None,
                    });
                    updated += 1;
                }
            }
        }

        info!(
            "Meta: set brain={:?} for {} unit(s)",
            button.module_id,
            updated
        );

        state.needs_rebuild = true;
    }
}
