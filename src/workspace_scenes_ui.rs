use bevy::ecs::message::MessageWriter;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::Ime;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

use crate::realm::ActiveRealmScene;
use crate::rich_text::set_rich_text_line;
use crate::scene_store::SceneSaveRequest;
use crate::types::{BuildScene, EmojiAtlas, GameMode, UiFonts};
use crate::workspace_ui::{TopPanelTab, TopPanelUiState};

const SCENE_NAME_MAX_CHARS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScenesUiField {
    None,
    AddSceneName,
}

#[derive(Resource, Debug)]
pub(crate) struct ScenesPanelUiState {
    pub(crate) scenes_dirty: bool,
    pub(crate) add_open: bool,
    pub(crate) focused_field: ScenesUiField,
    pub(crate) name: String,
    pub(crate) error: Option<String>,
    last_realm_id: Option<String>,
    last_panel_open: bool,
}

impl Default for ScenesPanelUiState {
    fn default() -> Self {
        Self {
            scenes_dirty: true,
            add_open: false,
            focused_field: ScenesUiField::None,
            name: String::new(),
            error: None,
            last_realm_id: None,
            last_panel_open: false,
        }
    }
}

#[derive(Component)]
pub(crate) struct ScenesAddSceneButton;

#[derive(Component)]
pub(crate) struct ScenesAddSceneButtonText;

#[derive(Component)]
pub(crate) struct ScenesListScrollPanel;

#[derive(Component)]
pub(crate) struct ScenesList;

#[derive(Component)]
pub(crate) struct ScenesListItem;

#[derive(Component)]
pub(crate) struct SceneSelectButton {
    pub(crate) scene_id: String,
}

#[derive(Component)]
pub(crate) struct AddScenePanelRoot;

#[derive(Component)]
pub(crate) struct AddSceneNameField;

#[derive(Component)]
pub(crate) struct AddSceneNameFieldText;

#[derive(Component)]
pub(crate) struct AddSceneAddButton;

#[derive(Component)]
pub(crate) struct AddSceneCancelButton;

#[derive(Component)]
pub(crate) struct AddSceneErrorText;

fn scenes_panel_open(
    mode: &State<GameMode>,
    build_scene: &State<BuildScene>,
    top: &TopPanelUiState,
) -> bool {
    matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
        && top.selected == Some(TopPanelTab::Scenes)
}

pub(crate) fn scenes_panel_sync_active_realm(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
) {
    let open = scenes_panel_open(&mode, &build_scene, &top);
    if !open && state.add_open {
        state.add_open = false;
        state.focused_field = ScenesUiField::None;
        state.error = None;
    }

    let realm_changed = state.last_realm_id.as_deref() != Some(active.realm_id.as_str());
    if realm_changed {
        state.last_realm_id = Some(active.realm_id.clone());
        state.scenes_dirty = true;
    }

    if open && !state.last_panel_open {
        state.scenes_dirty = true;
    }

    state.last_panel_open = open;
}

pub(crate) fn scenes_panel_set_add_panel_visibility(
    state: Res<ScenesPanelUiState>,
    mut panels: Query<&mut Node, With<AddScenePanelRoot>>,
) {
    let Ok(mut node) = panels.single_mut() else {
        return;
    };
    node.display = if state.add_open {
        Display::Flex
    } else {
        Display::None
    };
}

pub(crate) fn scenes_panel_rebuild_list_ui(
    mut commands: Commands,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    lists: Query<Entity, With<ScenesList>>,
    existing_items: Query<Entity, With<ScenesListItem>>,
    mut panels: Query<&mut ScrollPosition, With<ScenesListScrollPanel>>,
) {
    if !state.scenes_dirty {
        return;
    }
    let Ok(list_entity) = lists.single() else {
        return;
    };

    for entity in &existing_items {
        commands.entity(entity).try_despawn();
    }

    let scenes = crate::realm::list_scenes(&active.realm_id);
    if scenes.is_empty() {
        commands.entity(list_entity).with_children(|list| {
            list.spawn((
                Text::new("No scenes yet."),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.80, 0.86)),
                ScenesListItem,
            ));
        });
        state.scenes_dirty = false;
        return;
    }

    commands.entity(list_entity).with_children(|list| {
        for scene_id in scenes {
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ScenesListItem,
                SceneSelectButton {
                    scene_id: scene_id.clone(),
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(scene_id),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                ));
            });
        }
    });

    if let Ok(mut scroll) = panels.single_mut() {
        scroll.y = 0.0;
    }

    state.scenes_dirty = false;
}

pub(crate) fn scenes_panel_add_scene_button_actions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    mut state: ResMut<ScenesPanelUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ScenesAddSceneButton>)>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) {
        return;
    }

    for interaction in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }
        if state.add_open {
            continue;
        }
        state.add_open = true;
        state.focused_field = ScenesUiField::AddSceneName;
        state.name.clear();
        state.error = None;
    }
}

pub(crate) fn scenes_panel_scene_select_button_actions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    mut pending: ResMut<crate::realm::PendingRealmSceneSwitch>,
    mut saves: MessageWriter<SceneSaveRequest>,
    mut buttons: Query<(&Interaction, &SceneSelectButton), Changed<Interaction>>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) {
        return;
    }

    for (interaction, button) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }
        if button.scene_id == active.scene_id {
            continue;
        }

        pending.target = Some(ActiveRealmScene {
            realm_id: active.realm_id.clone(),
            scene_id: button.scene_id.clone(),
        });
        saves.write(SceneSaveRequest::new("switch scene"));
        state.error = None;
        state.add_open = false;
        state.focused_field = ScenesUiField::None;
    }
}

pub(crate) fn scenes_panel_add_panel_buttons(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    mut pending: ResMut<crate::realm::PendingRealmSceneSwitch>,
    mut saves: MessageWriter<SceneSaveRequest>,
    mut buttons: Query<
        (
            &Interaction,
            Option<&AddSceneAddButton>,
            Option<&AddSceneCancelButton>,
        ),
        (
            Changed<Interaction>,
            Or<(With<AddSceneAddButton>, With<AddSceneCancelButton>)>,
        ),
    >,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.add_open {
        return;
    }

    for (interaction, add, cancel) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }

        if cancel.is_some() {
            state.add_open = false;
            state.focused_field = ScenesUiField::None;
            state.name.clear();
            state.error = None;
            continue;
        }

        if add.is_none() {
            continue;
        }

        let validated = match validate_scene_dir_name(&state.name) {
            Ok(v) => v,
            Err(err) => {
                state.error = Some(err);
                continue;
            }
        };

        let scene_dir = crate::paths::scene_dir(&active.realm_id, &validated);
        if scene_dir.exists() {
            state.error = Some("Scene already exists.".to_string());
            continue;
        }

        if let Err(err) = crate::realm::ensure_realm_scene_scaffold(&active.realm_id, &validated) {
            state.error = Some(err);
            continue;
        }

        if let Err(err) =
            crate::scene_store::ensure_default_scene_dat_exists(&active.realm_id, &validated)
        {
            state.error = Some(err);
            continue;
        }

        state.add_open = false;
        state.focused_field = ScenesUiField::None;
        state.name.clear();
        state.error = None;
        state.scenes_dirty = true;

        pending.target = Some(ActiveRealmScene {
            realm_id: active.realm_id.clone(),
            scene_id: validated,
        });
        saves.write(SceneSaveRequest::new("add scene"));
    }
}

pub(crate) fn scenes_panel_name_field_focus(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    mut state: ResMut<ScenesPanelUiState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut fields: Query<&Interaction, (Changed<Interaction>, With<AddSceneNameField>)>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.add_open {
        return;
    }

    for interaction in &mut fields {
        if matches!(*interaction, Interaction::Pressed) {
            state.focused_field = ScenesUiField::AddSceneName;
            if let Ok(mut window) = windows.single_mut() {
                window.ime_enabled = true;
            }
        }
    }
}

pub(crate) fn scenes_panel_text_input(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    scene_ui: Res<crate::scene_authoring_ui::SceneAuthoringUiState>,
    mut state: ResMut<ScenesPanelUiState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: bevy::ecs::message::MessageReader<KeyboardInput>,
    mut ime_events: bevy::ecs::message::MessageReader<Ime>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if scene_ui.is_open() {
        keyboard.clear();
        ime_events.clear();
        return;
    }
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.add_open {
        keyboard.clear();
        ime_events.clear();
        return;
    }
    if state.focused_field != ScenesUiField::AddSceneName {
        return;
    }

    for event in ime_events.read() {
        if let Ime::Commit { value, .. } = event {
            if !value.is_empty() {
                push_text(&mut state.name, value);
            }
        }
    }

    for event in keyboard.read() {
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }
        state.error = None;

        match event.key_code {
            KeyCode::Backspace => {
                state.name.pop();
            }
            KeyCode::Escape => {
                state.focused_field = ScenesUiField::None;
                if let Ok(mut window) = windows.single_mut() {
                    window.ime_enabled = false;
                }
                ime_events.clear();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                state.focused_field = ScenesUiField::None;
                if let Ok(mut window) = windows.single_mut() {
                    window.ime_enabled = false;
                }
                ime_events.clear();
            }
            KeyCode::KeyV => {
                let modifier = keys.pressed(KeyCode::ControlLeft)
                    || keys.pressed(KeyCode::ControlRight)
                    || keys.pressed(KeyCode::SuperLeft)
                    || keys.pressed(KeyCode::SuperRight);
                if modifier {
                    if let Some(text) = crate::clipboard::read_text() {
                        push_text(&mut state.name, &text);
                    }
                    continue;
                }
                if let Some(text) = &event.text {
                    push_text(&mut state.name, text);
                }
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                push_text(&mut state.name, text);
            }
        }
    }
}

fn push_text(target: &mut String, text: &str) {
    let remaining = SCENE_NAME_MAX_CHARS.saturating_sub(target.chars().count());
    if remaining == 0 {
        return;
    }

    let mut inserted = 0usize;
    for ch in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
        if ch.is_control() || ch == '\n' || ch == '\t' {
            continue;
        }
        target.push(ch);
        inserted += 1;
        if inserted >= remaining {
            break;
        }
    }
}

pub(crate) fn scenes_panel_clear_keyboard_state_when_captured(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    scene_ui: Res<crate::scene_authoring_ui::SceneAuthoringUiState>,
    state: Res<ScenesPanelUiState>,
    mut keys: Option<ResMut<ButtonInput<KeyCode>>>,
) {
    if scene_ui.is_open() {
        return;
    }
    if !scenes_panel_open(&mode, &build_scene, &top)
        || !state.add_open
        || state.focused_field == ScenesUiField::None
    {
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
}

pub(crate) fn scenes_panel_scroll_wheel(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<MouseWheel>,
    roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::workspace_ui::ScenesPanelRoot>,
    >,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<ScenesListScrollPanel>>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) {
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

pub(crate) fn scenes_panel_update_texts(
    state: Res<ScenesPanelUiState>,
    mut commands: Commands,
    ui_fonts: Res<UiFonts>,
    emoji_atlas: Res<EmojiAtlas>,
    asset_server: Res<AssetServer>,
    mut texts: Query<
        (
            &mut Text,
            Option<&AddSceneNameFieldText>,
            Option<&AddSceneErrorText>,
        ),
        Or<(With<AddSceneNameFieldText>, With<AddSceneErrorText>)>,
    >,
    name_field: Query<Entity, With<AddSceneNameFieldText>>,
    mut last_name: Local<Option<String>>,
) {
    for (mut text, _name, error) in &mut texts {
        if error.is_some() {
            *text = Text::new(state.error.clone().unwrap_or_default());
        }
    }

    let Some(entity) = name_field.iter().next() else {
        return;
    };
    let needs_update = match last_name.as_ref() {
        Some(prev) => prev != &state.name,
        None => true,
    };
    if needs_update {
        set_rich_text_line(
            &mut commands,
            entity,
            &state.name,
            &ui_fonts,
            &emoji_atlas,
            &asset_server,
            14.0,
            Color::srgb(0.92, 0.92, 0.96),
            None,
        );
        *last_name = Some(state.name.clone());
    }
}

pub(crate) fn scenes_panel_update_styles(
    state: Res<ScenesPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut params: ParamSet<(
        Query<(&Interaction, &mut BackgroundColor, &mut BorderColor), With<ScenesAddSceneButton>>,
        Query<(
            &Interaction,
            &SceneSelectButton,
            &mut BackgroundColor,
            &mut BorderColor,
        )>,
        Query<(&Interaction, &mut BackgroundColor, &mut BorderColor), With<AddSceneNameField>>,
        Query<
            (
                &Interaction,
                Option<&AddSceneAddButton>,
                Option<&AddSceneCancelButton>,
                &mut BackgroundColor,
                &mut BorderColor,
            ),
            Or<(With<AddSceneAddButton>, With<AddSceneCancelButton>)>,
        >,
    )>,
) {
    {
        let mut add_button = params.p0();
        if let Ok((interaction, mut bg, mut border)) = add_button.single_mut() {
            let (mut bg_color, mut border_color) = if state.add_open {
                (
                    Color::srgba(0.07, 0.07, 0.09, 0.84),
                    Color::srgba(0.35, 0.35, 0.42, 0.75),
                )
            } else {
                (
                    Color::srgba(0.05, 0.05, 0.06, 0.75),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            };
            match *interaction {
                Interaction::Pressed => {
                    bg_color = Color::srgba(0.10, 0.10, 0.12, 0.92);
                    border_color = Color::srgba(0.45, 0.45, 0.55, 0.85);
                }
                Interaction::Hovered => {
                    bg_color = Color::srgba(0.07, 0.07, 0.09, 0.84);
                    border_color = Color::srgba(0.35, 0.35, 0.42, 0.75);
                }
                Interaction::None => {}
            }
            *bg = BackgroundColor(bg_color);
            *border = BorderColor::all(border_color);
        }
    }

    {
        let mut scene_buttons = params.p1();
        for (interaction, button, mut bg, mut border) in &mut scene_buttons {
            let selected = button.scene_id == active.scene_id;
            apply_option_style(selected, *interaction, &mut bg, &mut border);
        }
    }

    {
        let mut name_field = params.p2();
        if let Ok((interaction, mut bg, mut border)) = name_field.single_mut() {
            let focused = state.focused_field == ScenesUiField::AddSceneName && state.add_open;
            let (mut bg_color, border_color) = if focused {
                (
                    Color::srgba(0.03, 0.03, 0.04, 0.78),
                    Color::srgba(0.45, 0.45, 0.55, 0.80),
                )
            } else {
                (
                    Color::srgba(0.02, 0.02, 0.03, 0.65),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            };
            match *interaction {
                Interaction::Pressed => bg_color = Color::srgba(0.10, 0.10, 0.12, 0.92),
                Interaction::Hovered => bg_color = Color::srgba(0.03, 0.03, 0.04, 0.70),
                Interaction::None => {}
            }
            *bg = BackgroundColor(bg_color);
            *border = BorderColor::all(border_color);
        }
    }

    {
        let mut add_panel_buttons = params.p3();
        for (interaction, add, cancel, mut bg, mut border) in &mut add_panel_buttons {
            let base = if add.is_some() {
                (
                    Color::srgba(0.06, 0.10, 0.07, 0.78),
                    Color::srgb(0.25, 0.80, 0.45),
                )
            } else if cancel.is_some() {
                (
                    Color::srgba(0.05, 0.05, 0.06, 0.75),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            } else {
                (
                    Color::srgba(0.05, 0.05, 0.06, 0.75),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            };

            let (mut bg_color, mut border_color) = base;
            match *interaction {
                Interaction::Pressed => {
                    bg_color = Color::srgba(0.10, 0.10, 0.12, 0.92);
                    border_color = Color::srgba(0.45, 0.45, 0.55, 0.85);
                }
                Interaction::Hovered => {
                    bg_color = Color::srgba(0.07, 0.07, 0.09, 0.84);
                    border_color = Color::srgba(0.35, 0.35, 0.42, 0.75);
                }
                Interaction::None => {}
            }
            *bg = BackgroundColor(bg_color);
            *border = BorderColor::all(border_color);
        }
    }
}

fn apply_option_style(
    selected: bool,
    interaction: Interaction,
    bg: &mut BackgroundColor,
    border: &mut BorderColor,
) {
    let (mut bg_color, mut border_color) = if selected {
        (
            Color::srgba(0.06, 0.10, 0.07, 0.85),
            Color::srgb(0.25, 0.80, 0.45),
        )
    } else {
        (
            Color::srgba(0.05, 0.05, 0.06, 0.75),
            Color::srgba(0.25, 0.25, 0.30, 0.65),
        )
    };
    match interaction {
        Interaction::Pressed => {
            bg_color = Color::srgba(0.10, 0.18, 0.13, 0.92);
        }
        Interaction::Hovered => {
            bg_color = Color::srgba(0.07, 0.07, 0.09, 0.84);
            if !selected {
                border_color = Color::srgba(0.35, 0.35, 0.42, 0.75);
            }
        }
        Interaction::None => {}
    }
    *bg = BackgroundColor(bg_color);
    *border = BorderColor::all(border_color);
}

fn validate_scene_dir_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Name cannot be empty.".to_string());
    }
    if name == "." || name == ".." {
        return Err("Name cannot be '.' or '..'.".to_string());
    }
    if name.ends_with('.') {
        return Err("Name cannot end with '.'.".to_string());
    }
    if name.chars().any(|c| c == '/' || c == '\\') {
        return Err("Name cannot contain path separators.".to_string());
    }
    if let Some(ch) = name
        .chars()
        .find(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\0'))
    {
        return Err(format!("Name contains invalid character: '{ch}'"));
    }
    if is_windows_reserved_name(name) {
        return Err("Name is reserved on Windows.".to_string());
    }
    Ok(name.to_string())
}

fn is_windows_reserved_name(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    if upper.is_empty() {
        return false;
    }
    let base = upper.split('.').next().unwrap_or("");
    matches!(
        base,
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[allow(dead_code)]
fn scene_build_scene_dat_path(realm_id: &str, scene_id: &str) -> PathBuf {
    let scene_dat = crate::paths::scene_dat_path(realm_id, scene_id);
    let dir = scene_dat
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    dir.join("scene.build.dat")
}
