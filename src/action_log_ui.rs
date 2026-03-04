use bevy::prelude::*;

use crate::action_log::{ActionLogEntry, ActionLogSource, ActionLogState};
use crate::types::{BuildScene, GameMode};

const BUTTON_Z_INDEX: i32 = 965;
const PANEL_Z_INDEX: i32 = 935;
const PANEL_WIDTH_PX: f32 = 420.0;
const PANEL_MAX_HEIGHT_PX: f32 = 320.0;
const MAX_LINES: usize = 18;

#[derive(Component)]
pub(crate) struct ActionLogToggleButton;

#[derive(Component)]
pub(crate) struct ActionLogToggleButtonText;

#[derive(Component)]
pub(crate) struct ActionLogPanelRoot;

#[derive(Component)]
pub(crate) struct ActionLogPanelText;

pub(crate) fn setup_action_log_ui(mut commands: Commands) {
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(110.0),
                width: Val::Px(92.0),
                height: Val::Px(34.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.60)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
            Outline {
                width: Val::Px(1.0),
                color: Color::srgba(0.25, 0.25, 0.30, 0.65),
                offset: Val::Px(0.0),
            },
            ZIndex(BUTTON_Z_INDEX),
            Visibility::Hidden,
            ActionLogToggleButton,
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("Log: On"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                ActionLogToggleButtonText,
            ));
        });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(44.0),
                left: Val::Px(10.0),
                width: Val::Px(PANEL_WIDTH_PX),
                max_height: Val::Px(PANEL_MAX_HEIGHT_PX),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.85)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
            Outline {
                width: Val::Px(1.0),
                color: Color::srgba(0.25, 0.25, 0.30, 0.75),
                offset: Val::Px(0.0),
            },
            ZIndex(PANEL_Z_INDEX),
            Visibility::Hidden,
            ActionLogPanelRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("Action Log"),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.95, 0.97)),
            ));

            root.spawn((
                Text::new(""),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::srgb(0.86, 0.86, 0.90)),
                TextShadow {
                    offset: Vec2::splat(2.0),
                    color: Color::linear_rgba(0.0, 0.0, 0.0, 0.85),
                },
                ActionLogPanelText,
            ));
        });
}

pub(crate) fn handle_action_log_toggle_button(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<ActionLogToggleButton>),
    >,
    mut action_log: ResMut<ActionLogState>,
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
) {
    if !matches!(mode.get(), GameMode::Play) || !matches!(build_scene.get(), BuildScene::Realm) {
        return;
    }

    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.60));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.75));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.85));
                action_log.enabled = !action_log.enabled;
            }
        }
    }
}

pub(crate) fn update_action_log_ui(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    action_log: Res<ActionLogState>,
    mut toggle_vis: Query<
        &mut Visibility,
        (With<ActionLogToggleButton>, Without<ActionLogPanelRoot>),
    >,
    mut panel_vis: Query<
        &mut Visibility,
        (With<ActionLogPanelRoot>, Without<ActionLogToggleButton>),
    >,
    mut toggle_texts: Query<
        &mut Text,
        (With<ActionLogToggleButtonText>, Without<ActionLogPanelText>),
    >,
    mut panel_texts: Query<
        &mut Text,
        (With<ActionLogPanelText>, Without<ActionLogToggleButtonText>),
    >,
    mut last_version: Local<u64>,
    mut last_enabled: Local<bool>,
    mut last_visible: Local<bool>,
) {
    let can_show =
        matches!(mode.get(), GameMode::Play) && matches!(build_scene.get(), BuildScene::Realm);
    let visible = can_show && action_log.enabled;

    for mut vis in &mut toggle_vis {
        *vis = if can_show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut vis in &mut panel_vis {
        *vis = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    let label = if action_log.enabled {
        "Log: On"
    } else {
        "Log: Off"
    };
    for mut text in &mut toggle_texts {
        **text = label.into();
    }

    if !visible {
        *last_version = action_log.version();
        *last_enabled = action_log.enabled;
        *last_visible = visible;
        return;
    }

    let version = action_log.version();
    if *last_visible && version == *last_version && action_log.enabled == *last_enabled {
        return;
    }
    *last_version = version;
    *last_enabled = action_log.enabled;
    *last_visible = visible;

    let body = format_action_log_body(action_log.entries(), MAX_LINES);
    for mut text in &mut panel_texts {
        **text = body.clone();
    }
}

fn format_action_log_body(
    entries: &std::collections::VecDeque<ActionLogEntry>,
    max_lines: usize,
) -> String {
    if entries.is_empty() || max_lines == 0 {
        return "(no actions yet)".to_string();
    }

    let mut lines: Vec<&ActionLogEntry> = entries.iter().rev().take(max_lines).collect();
    lines.reverse();

    let mut out = String::new();
    for (idx, entry) in lines.iter().enumerate() {
        let source = match entry.source {
            ActionLogSource::Brain => "B",
            ActionLogSource::Player => "P",
        };
        let time_s = entry.at_secs.max(0.0);
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "[{time_s:>6.1}] {source} {}",
            entry.message.trim()
        ));
    }
    out
}
