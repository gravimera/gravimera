use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::Ime;
use bevy::window::PrimaryWindow;
use std::collections::HashMap;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::geometry::{clamp_world_xz, snap_to_grid};
use crate::object::registry::ObjectLibrary;
use crate::object::registry::{ColliderProfile, MobilityMode};
use crate::object::visuals;
use crate::prefab_descriptors::PrefabDescriptorLibrary;
use crate::rich_text::{set_rich_text_line, spawn_rich_text_line};
use crate::scene_store::SceneSaveRequest;
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, EmojiAtlas, GameMode,
    ObjectId, ObjectPrefabId, UiFonts,
};

const PANEL_Z_INDEX: i32 = 930;
const PANEL_WIDTH_PX: f32 = 260.0;
const DRAG_START_THRESHOLD_PX: f32 = 6.0;
const PREFAB_PREVIEW_Z_INDEX: i32 = 1200;
const PREFAB_PREVIEW_LAYER: usize = 28;
const PREFAB_PREVIEW_WIDTH_PX: u32 = 640;
const PREFAB_PREVIEW_HEIGHT_PX: u32 = 360;

#[derive(SystemParam)]
pub(crate) struct ModelLibraryEnv<'w> {
    build_scene: Res<'w, State<crate::types::BuildScene>>,
    active: Res<'w, crate::realm::ActiveRealmScene>,
}

#[derive(SystemParam)]
pub(crate) struct ModelLibraryPreviewInputCapture<'w, 's> {
    state: Res<'w, ModelLibraryUiState>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    roots: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static Visibility,
        ),
        With<ModelLibraryPreviewOverlayRoot>,
    >,
}

impl<'w, 's> ModelLibraryPreviewInputCapture<'w, 's> {
    pub(crate) fn window(&self) -> Option<&Window> {
        self.windows.single().ok()
    }

    pub(crate) fn captures_cursor(&self, window: &Window) -> bool {
        if !self.state.is_preview_open() {
            return false;
        }
        let Some(cursor) = window.physical_cursor_position() else {
            return false;
        };
        model_library_preview_overlay_contains_cursor(cursor, &self.roots)
    }
}

#[derive(Debug, Clone, Copy)]
struct ModelLibraryDrag {
    model_id: u128,
    start_cursor: Vec2,
    is_dragging: bool,
    preview_translation: Option<Vec3>,
}

#[derive(Debug, Clone, Copy)]
struct ModelLibraryScrollbarDrag {
    grab_offset: f32,
}

#[derive(Debug, Clone)]
struct ModelLibraryThumbnailCacheEntry {
    handle: Handle<Image>,
    modified_at_ms: u128,
}

#[derive(Debug)]
struct ModelLibraryPrefabPreview {
    prefab_id: u128,
    ui_root: Entity,
    scene_root: Entity,
    target: Handle<Image>,
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    last_cursor: Option<Vec2>,
}

#[derive(Debug)]
struct SpawnedModelLibraryPreviewScene {
    scene_root: Entity,
    target: Handle<Image>,
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

#[derive(Resource, Debug)]
pub(crate) struct ModelLibraryUiState {
    models_dirty: bool,
    open: bool,
    search_query: String,
    search_focused: bool,
    drag: Option<ModelLibraryDrag>,
    spawn_seq: u32,
    scrollbar_drag: Option<ModelLibraryScrollbarDrag>,
    preview_scrollbar_drag: Option<ModelLibraryScrollbarDrag>,
    thumbnail_cache: HashMap<u128, ModelLibraryThumbnailCacheEntry>,
    listed_prefabs: Vec<u128>,
    pending_preview: Option<u128>,
    preview: Option<ModelLibraryPrefabPreview>,
    last_rebuilt_scene: Option<(String, String)>,
}

impl Default for ModelLibraryUiState {
    fn default() -> Self {
        Self {
            models_dirty: true,
            open: true,
            search_query: String::new(),
            search_focused: false,
            drag: None,
            spawn_seq: 0,
            scrollbar_drag: None,
            preview_scrollbar_drag: None,
            thumbnail_cache: HashMap::new(),
            listed_prefabs: Vec::new(),
            pending_preview: None,
            preview: None,
            last_rebuilt_scene: None,
        }
    }
}

impl ModelLibraryUiState {
    pub(crate) fn mark_models_dirty(&mut self) {
        self.models_dirty = true;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn set_open(&mut self, open: bool) {
        if self.open == open {
            return;
        }
        self.open = open;
        if !open {
            self.drag = None;
            self.scrollbar_drag = None;
            self.preview_scrollbar_drag = None;
            self.search_focused = false;
            self.pending_preview = None;
        }
    }

    pub(crate) fn is_drag_active(&self) -> bool {
        self.drag.is_some()
    }

    pub(crate) fn is_preview_open(&self) -> bool {
        self.preview.is_some()
    }
}

pub(crate) fn model_library_preview_overlay_contains_cursor(
    cursor_physical: Vec2,
    roots: &Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<ModelLibraryPreviewOverlayRoot>,
    >,
) -> bool {
    roots.iter().any(|(node, transform, vis)| {
        *vis != Visibility::Hidden && node.contains_point(*transform, cursor_physical)
    })
}

#[derive(Component)]
pub(crate) struct ModelLibraryRoot;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollPanel;

#[derive(Component)]
pub(crate) struct ModelLibraryList;

#[derive(Component)]
pub(crate) struct ModelLibraryListItem;

#[derive(Component)]
pub(crate) struct ModelLibrarySelectionMark {
    pub(crate) prefab_id: u128,
}

#[derive(Component)]
pub(crate) struct ModelLibraryScrollbarTrack;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollbarThumb;

#[derive(Component)]
pub(crate) struct ModelLibraryItemButton {
    kind: ModelLibraryItemKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelLibraryItemKind {
    Prefab { prefab_id: u128 },
    InFlight { run_id: u128 },
}

impl ModelLibraryItemKind {
    fn prefab_id(self) -> Option<u128> {
        match self {
            Self::Prefab { prefab_id } => Some(prefab_id),
            Self::InFlight { .. } => None,
        }
    }

    fn is_in_flight(self) -> bool {
        matches!(self, Self::InFlight { .. })
    }
}

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dButton;

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dButtonText;

#[derive(Component)]
pub(crate) struct ModelLibrarySearchField;

#[derive(Component)]
pub(crate) struct ModelLibrarySearchFieldText;

#[derive(Component)]
pub(crate) struct ModelLibraryInFlightRemoveButton {
    run_id: u128,
}

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewOverlayRoot;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewCloseButton;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewInfoScrollPanel;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewInfoScrollbarTrack;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewInfoScrollbarThumb;

#[derive(Component)]
struct ModelLibraryPreviewSceneRoot;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewCamera;

pub(crate) fn setup_model_library_ui(
    mut commands: Commands,
    ui_fonts: Res<UiFonts>,
    emoji_atlas: Res<EmojiAtlas>,
    asset_server: Res<AssetServer>,
) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(44.0),
                left: Val::Px(10.0),
                width: Val::Px(PANEL_WIDTH_PX),
                height: Val::Px(680.0),
                max_height: Val::Px(680.0),
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
            ModelLibraryRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new("Prefabs"),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.95, 0.97)),
                ));

                row.spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    ModelLibraryGen3dButton,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Gen3D"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        ModelLibraryGen3dButtonText,
                    ));
                });
            });

            root.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ModelLibrarySearchField,
            ))
            .with_children(|field| {
                spawn_rich_text_line(
                    field,
                    "Search…",
                    &ui_fonts,
                    &emoji_atlas,
                    &asset_server,
                    14.0,
                    Color::srgba(0.80, 0.80, 0.86, 0.75),
                    (
                        Node {
                            width: Val::Percent(100.0),
                            flex_wrap: FlexWrap::Wrap,
                            justify_content: JustifyContent::FlexStart,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(1.0),
                            row_gap: Val::Px(2.0),
                            ..default()
                        },
                        ModelLibrarySearchFieldText,
                    ),
                    None,
                );
            });

            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    min_height: Val::Px(0.0),
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
                    ModelLibraryScrollPanel,
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
                        ModelLibraryList,
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
                    ModelLibraryScrollbarTrack,
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
                        ModelLibraryScrollbarThumb,
                    ));
                });
            });
        });
}

pub(crate) fn model_library_update_visibility(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
    mut roots: Query<&mut Visibility, With<ModelLibraryRoot>>,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    for mut visibility in &mut roots {
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    if !visible {
        state.drag = None;
        state.search_focused = false;
        state.pending_preview = None;
        state.scrollbar_drag = None;
        state.preview_scrollbar_drag = None;
        close_model_library_preview(&mut commands, &mut state);
    }
}

pub(crate) fn model_library_search_field_focus(
    mut state: ResMut<ModelLibraryUiState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut fields: Query<&Interaction, (Changed<Interaction>, With<ModelLibrarySearchField>)>,
) {
    if !state.is_open() {
        return;
    }

    for interaction in &mut fields {
        if *interaction == Interaction::Pressed {
            state.search_focused = true;
            if let Ok(mut window) = windows.single_mut() {
                window.ime_enabled = true;
            }
        }
    }
}

pub(crate) fn model_library_search_text_input(
    mut state: ResMut<ModelLibraryUiState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: MessageReader<KeyboardInput>,
    mut ime_events: MessageReader<Ime>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !state.is_open() {
        keyboard.clear();
        ime_events.clear();
        return;
    }
    if !state.search_focused {
        return;
    }

    fn push_text(target: &mut String, text: &str) -> bool {
        let before = target.len();
        for ch in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
            if ch.is_control() || ch == '\n' {
                continue;
            }
            if target.chars().count() >= 256 {
                break;
            }
            target.push(ch);
        }
        target.len() != before
    }

    for event in ime_events.read() {
        if let Ime::Commit { value, .. } = event {
            if !value.is_empty() && push_text(&mut state.search_query, value) {
                state.models_dirty = true;
            }
        }
    }

    for event in keyboard.read() {
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }

        let mut changed = false;
        match event.key_code {
            KeyCode::Backspace => {
                let before = state.search_query.chars().count();
                if before > 0 {
                    state.search_query.pop();
                    changed = true;
                }
            }
            KeyCode::Escape => {
                state.search_focused = false;
                if let Ok(mut window) = windows.single_mut() {
                    window.ime_enabled = false;
                }
                ime_events.clear();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                state.search_focused = false;
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
                        if push_text(&mut state.search_query, &text) {
                            state.models_dirty = true;
                        }
                    }
                    continue;
                }
                if let Some(text) = &event.text {
                    changed |= push_text(&mut state.search_query, text);
                }
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                changed |= push_text(&mut state.search_query, text);
            }
        }

        if changed {
            state.models_dirty = true;
        }
    }
}

pub(crate) fn model_library_update_search_field_ui(
    mut commands: Commands,
    state: Res<ModelLibraryUiState>,
    ui_fonts: Res<UiFonts>,
    emoji_atlas: Res<EmojiAtlas>,
    asset_server: Res<AssetServer>,
    mut fields: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<ModelLibrarySearchField>,
    >,
    rich_text: Query<Entity, With<ModelLibrarySearchFieldText>>,
    mut last_text: Local<Option<(String, bool)>>,
) {
    if !state.is_open() {
        return;
    }

    for (interaction, mut bg, mut border) in &mut fields {
        let focused = state.search_focused;
        match *interaction {
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.78));
                *border = BorderColor::all(Color::srgb(0.30, 0.55, 0.95));
            }
            Interaction::Hovered => {
                let alpha = if focused { 0.78 } else { 0.70 };
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, alpha));
                *border = BorderColor::all(if focused {
                    Color::srgb(0.30, 0.55, 0.95)
                } else {
                    Color::srgba(0.25, 0.25, 0.30, 0.75)
                });
            }
            Interaction::None => {
                let alpha = if focused { 0.74 } else { 0.65 };
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, alpha));
                *border = BorderColor::all(if focused {
                    Color::srgb(0.30, 0.55, 0.95)
                } else {
                    Color::srgba(0.25, 0.25, 0.30, 0.65)
                });
            }
        }
    }

    let query = state.search_query.trim();
    let (text_value, hint) = if query.is_empty() {
        ("Search…".to_string(), true)
    } else {
        (query.to_string(), false)
    };
    let text_color = if hint {
        Color::srgba(0.80, 0.80, 0.86, 0.75)
    } else {
        Color::srgba(0.92, 0.92, 0.96, 1.0)
    };

    let needs_update = match last_text.as_ref() {
        Some((prev_text, prev_hint)) => prev_text != &text_value || *prev_hint != hint,
        None => true,
    };
    if needs_update {
        if let Ok(entity) = rich_text.single() {
            set_rich_text_line(
                &mut commands,
                entity,
                &text_value,
                &ui_fonts,
                &emoji_atlas,
                &asset_server,
                14.0,
                text_color,
                None,
            );
            *last_text = Some((text_value, hint));
        }
    }
}

pub(crate) fn model_library_rebuild_list_ui(
    mut commands: Commands,
    active: Res<crate::realm::ActiveRealmScene>,
    mut images: ResMut<Assets<Image>>,
    mut descriptors: ResMut<PrefabDescriptorLibrary>,
    mut state: ResMut<ModelLibraryUiState>,
    lists: Query<Entity, With<ModelLibraryList>>,
    existing_items: Query<Entity, With<ModelLibraryListItem>>,
) {
    let active_changed = match state.last_rebuilt_scene.as_ref() {
        Some((realm_id, _scene_id)) => realm_id != &active.realm_id,
        None => true,
    };
    if active_changed {
        state.models_dirty = true;
        state.thumbnail_cache.clear();
    }

    if !state.models_dirty {
        return;
    }
    let Ok(list_entity) = lists.single() else {
        return;
    };
    state.last_rebuilt_scene = Some((active.realm_id.clone(), active.scene_id.clone()));

    for entity in &existing_items {
        commands.entity(entity).try_despawn();
    }

    descriptors.clear();
    let realm_prefabs_dir = crate::realm_prefab_packages::realm_prefabs_root_dir(&active.realm_id);
    if let Err(err) = crate::prefab_descriptors::load_prefab_descriptors_from_dir(
        &realm_prefabs_dir,
        &mut *descriptors,
    ) {
        warn!("{err}");
    }

    let model_ids = crate::realm_prefab_packages::list_realm_prefab_packages(&active.realm_id)
        .unwrap_or_default();
    let in_flight_entries = crate::gen3d::load_gen3d_in_flight_entries(&active.realm_id);
    if model_ids.is_empty() && in_flight_entries.is_empty() {
        commands.entity(list_entity).with_children(|list| {
            list.spawn((
                Text::new("No realm prefabs yet.\nUse Gen3D to generate one."),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.80, 0.86)),
                ModelLibraryListItem,
            ));
        });
        state.listed_prefabs.clear();
        state.models_dirty = false;
        return;
    }

    fn system_time_ms(time: std::time::SystemTime) -> u128 {
        time.duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u128)
            .unwrap_or(0)
    }

    fn load_png_ui_image(
        images: &mut Assets<Image>,
        path: &std::path::Path,
    ) -> Result<Handle<Image>, String> {
        let bytes = std::fs::read(path)
            .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
        let image = Image::from_buffer(
            &bytes,
            ImageType::Extension("png"),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::linear(),
            bevy::asset::RenderAssetUsages::default(),
        )
        .map_err(|err| format!("Failed to decode {}: {err}", path.display()))?;
        Ok(images.add(image))
    }

    fn relevance_score(
        query: &str,
        name: &str,
        tags: &[String],
        summary: Option<&str>,
        id: &str,
    ) -> u32 {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return 0;
        }
        let name_l = name.to_lowercase();
        let id_l = id.to_lowercase();
        let tags_l: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
        let summary_l = summary.map(|s| s.to_lowercase());

        let mut score: u32 = 0;
        if name_l.contains(&query) {
            score = score.saturating_add(120);
        }
        if name_l.starts_with(&query) {
            score = score.saturating_add(80);
        }
        if id_l.contains(&query) {
            score = score.saturating_add(15);
        }

        let tokens: Vec<&str> = query.split_whitespace().collect();
        for token in tokens {
            if token.is_empty() {
                continue;
            }
            if name_l.contains(token) {
                score = score.saturating_add(60);
            }
            if tags_l.iter().any(|t| t == token) {
                score = score.saturating_add(45);
            } else if tags_l.iter().any(|t| t.contains(token)) {
                score = score.saturating_add(20);
            }
            if let Some(summary_l) = summary_l.as_ref() {
                if summary_l.contains(token) {
                    score = score.saturating_add(12);
                }
            }
        }
        score
    }

    #[derive(Debug)]
    enum RowKind {
        Prefab {
            prefab_id: u128,
            modified_at_ms: u128,
            thumbnail: Option<Handle<Image>>,
        },
        InFlight {
            run_id: u128,
            status: crate::gen3d::Gen3dInFlightStatus,
            created_at_ms: u128,
            queue_pos: Option<usize>,
        },
    }

    #[derive(Debug)]
    struct Row {
        kind: RowKind,
        display_name: String,
        score: u32,
    }

    impl Row {
        fn sort_ms(&self) -> u128 {
            match &self.kind {
                RowKind::Prefab { modified_at_ms, .. } => *modified_at_ms,
                RowKind::InFlight { created_at_ms, .. } => *created_at_ms,
            }
        }

        fn prefab_id(&self) -> Option<u128> {
            match &self.kind {
                RowKind::Prefab { prefab_id, .. } => Some(*prefab_id),
                RowKind::InFlight { .. } => None,
            }
        }
    }

    let query = state.search_query.trim().to_string();
    let mut prefab_rows: Vec<Row> = Vec::new();
    let mut inflight_rows: Vec<Row> = Vec::new();

    let mut queued_positions: HashMap<String, usize> = HashMap::new();
    let mut queued_entries: Vec<&crate::gen3d::Gen3dInFlightEntry> = in_flight_entries
        .iter()
        .filter(|entry| matches!(entry.status, crate::gen3d::Gen3dInFlightStatus::Queued))
        .collect();
    queued_entries.sort_by_key(|entry| entry.created_at_ms);
    for (idx, entry) in queued_entries.iter().enumerate() {
        queued_positions.insert(entry.run_id.clone(), idx.saturating_add(1));
    }

    for entry in &in_flight_entries {
        let run_id = match uuid::Uuid::parse_str(entry.run_id.trim()) {
            Ok(id) => id.as_u128(),
            Err(err) => {
                debug!(
                    "Skipping invalid Gen3D in-flight run id {}: {err}",
                    entry.run_id
                );
                continue;
            }
        };
        let display_name = entry.label.trim();
        let display_name = if display_name.is_empty() {
            "Untitled run".to_string()
        } else {
            display_name.to_string()
        };
        let score = relevance_score(
            query.as_str(),
            &display_name,
            &[],
            entry.error.as_deref(),
            entry.run_id.as_str(),
        );
        if !query.is_empty() && score == 0 {
            continue;
        }
        let queue_pos = match entry.status {
            crate::gen3d::Gen3dInFlightStatus::Queued => {
                queued_positions.get(&entry.run_id).copied()
            }
            _ => None,
        };
        inflight_rows.push(Row {
            kind: RowKind::InFlight {
                run_id,
                status: entry.status.clone(),
                created_at_ms: entry.created_at_ms,
                queue_pos,
            },
            display_name,
            score,
        });
    }

    for prefab_id in model_ids {
        let uuid = uuid::Uuid::from_u128(prefab_id).to_string();
        let desc = descriptors.get(prefab_id);

        let display_name = desc
            .and_then(|d| d.label.as_ref())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .unwrap_or_else(|| uuid.clone());

        let modified_at_ms = desc
            .and_then(|d| d.provenance.as_ref())
            .and_then(|p| p.modified_at_ms.or(p.created_at_ms))
            .unwrap_or_else(|| {
                let prefabs_dir = crate::realm_prefab_packages::realm_prefab_package_prefabs_dir(
                    &active.realm_id,
                    prefab_id,
                );
                let path = prefabs_dir.join(format!("{uuid}.desc.json"));
                std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .map(system_time_ms)
                    .unwrap_or(0)
            });

        let summary = desc
            .and_then(|d| d.text.as_ref())
            .and_then(|t| t.short.as_deref())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        let tags: Vec<String> = desc.map(|d| d.tags.clone()).unwrap_or_default();

        let score = relevance_score(query.as_str(), &display_name, &tags, summary, &uuid);
        if !query.is_empty() && score == 0 {
            continue;
        }

        let thumbnail = {
            let path = crate::realm_prefab_packages::realm_prefab_package_thumbnail_path(
                &active.realm_id,
                prefab_id,
            );
            if let Ok(meta) = std::fs::metadata(&path) {
                let modified_at_ms = meta.modified().map(system_time_ms).unwrap_or(0);
                if let Some(entry) = state.thumbnail_cache.get(&prefab_id) {
                    if entry.modified_at_ms == modified_at_ms {
                        Some(entry.handle.clone())
                    } else {
                        match load_png_ui_image(&mut images, &path) {
                            Ok(handle) => {
                                state.thumbnail_cache.insert(
                                    prefab_id,
                                    ModelLibraryThumbnailCacheEntry {
                                        handle: handle.clone(),
                                        modified_at_ms,
                                    },
                                );
                                Some(handle)
                            }
                            Err(err) => {
                                debug!("{err}");
                                None
                            }
                        }
                    }
                } else {
                    match load_png_ui_image(&mut images, &path) {
                        Ok(handle) => {
                            state.thumbnail_cache.insert(
                                prefab_id,
                                ModelLibraryThumbnailCacheEntry {
                                    handle: handle.clone(),
                                    modified_at_ms,
                                },
                            );
                            Some(handle)
                        }
                        Err(err) => {
                            debug!("{err}");
                            None
                        }
                    }
                }
            } else {
                None
            }
        };

        prefab_rows.push(Row {
            kind: RowKind::Prefab {
                prefab_id,
                modified_at_ms,
                thumbnail,
            },
            display_name,
            score,
        });
    }

    if !query.is_empty() {
        inflight_rows.append(&mut prefab_rows);
        inflight_rows.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.sort_ms().cmp(&a.sort_ms()))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
    } else {
        inflight_rows.sort_by(|a, b| {
            let rank = |row: &Row| match &row.kind {
                RowKind::InFlight { status, .. } => match status {
                    crate::gen3d::Gen3dInFlightStatus::Running => 0,
                    crate::gen3d::Gen3dInFlightStatus::Queued => 1,
                    crate::gen3d::Gen3dInFlightStatus::Failed => 2,
                },
                RowKind::Prefab { .. } => 3,
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| b.sort_ms().cmp(&a.sort_ms()))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });

        prefab_rows.sort_by(|a, b| {
            b.sort_ms()
                .cmp(&a.sort_ms())
                .then_with(|| a.display_name.cmp(&b.display_name))
                .then_with(|| a.prefab_id().cmp(&b.prefab_id()))
        });
        inflight_rows.extend(prefab_rows);
    }

    state.listed_prefabs = inflight_rows
        .iter()
        .filter_map(|row| row.prefab_id())
        .collect();

    commands.entity(list_entity).with_children(|list| {
        for row in inflight_rows {
            let item_kind = match &row.kind {
                RowKind::Prefab { prefab_id, .. } => ModelLibraryItemKind::Prefab {
                    prefab_id: *prefab_id,
                },
                RowKind::InFlight { run_id, .. } => {
                    ModelLibraryItemKind::InFlight { run_id: *run_id }
                }
            };
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ModelLibraryListItem,
                ModelLibraryItemButton { kind: item_kind },
            ))
            .with_children(|b| {
                if let RowKind::Prefab { prefab_id, .. } = &row.kind {
                    b.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(1.0),
                            top: Val::Px(1.0),
                            bottom: Val::Px(1.0),
                            width: Val::Px(4.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.25, 0.95, 0.85, 0.85)),
                        Visibility::Hidden,
                        ModelLibrarySelectionMark {
                            prefab_id: *prefab_id,
                        },
                    ));
                }

                b.spawn((
                    Node {
                        width: Val::Px(42.0),
                        height: Val::Px(42.0),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.75)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ))
                .with_children(|thumb| {
                    if let RowKind::Prefab { thumbnail, .. } = &row.kind {
                        if let Some(handle) = thumbnail.as_ref() {
                            thumb.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                ImageNode::new(handle.clone()).with_mode(NodeImageMode::Stretch),
                            ));
                        }
                    }
                });

                b.spawn((
                    Text::new(row.display_name),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                ));

                if let RowKind::InFlight {
                    status,
                    queue_pos,
                    run_id,
                    ..
                } = &row.kind
                {
                    let status = status.clone();
                    let queue_pos = *queue_pos;
                    let run_id = *run_id;
                    b.spawn((
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                    ));

                    let status_text = match status {
                        crate::gen3d::Gen3dInFlightStatus::Running => "Generating".to_string(),
                        crate::gen3d::Gen3dInFlightStatus::Queued => queue_pos
                            .map(|pos| format!("Queued #{pos}"))
                            .unwrap_or_else(|| "Queued".to_string()),
                        crate::gen3d::Gen3dInFlightStatus::Failed => "Failed".to_string(),
                    };
                    let (badge_bg, badge_border, badge_text) = match status {
                        crate::gen3d::Gen3dInFlightStatus::Failed => (
                            Color::srgba(0.35, 0.12, 0.12, 0.90),
                            Color::srgba(0.65, 0.20, 0.20, 0.85),
                            Color::srgb(0.98, 0.82, 0.82),
                        ),
                        _ => (
                            Color::srgba(0.20, 0.30, 0.12, 0.90),
                            Color::srgba(0.35, 0.55, 0.20, 0.85),
                            Color::srgb(0.85, 0.98, 0.80),
                        ),
                    };

                    b.spawn((
                        Node {
                            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(badge_bg),
                        BorderColor::all(badge_border),
                    ))
                    .with_children(|badge| {
                        badge.spawn((
                            Text::new(status_text),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(badge_text),
                        ));
                    });

                    b.spawn((
                        Button,
                        Node {
                            width: Val::Px(22.0),
                            height: Val::Px(22.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92)),
                        BorderColor::all(Color::srgba(0.45, 0.45, 0.55, 0.85)),
                        bevy::ui::FocusPolicy::Block,
                        ModelLibraryInFlightRemoveButton { run_id },
                    ))
                    .with_children(|button| {
                        button.spawn((
                            Text::new("X"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        ));
                    });
                }
            });
        }
    });

    state.models_dirty = false;
}

fn close_model_library_preview(commands: &mut Commands, state: &mut ModelLibraryUiState) {
    let Some(preview) = state.preview.take() else {
        return;
    };
    state.preview_scrollbar_drag = None;
    let target_id = preview.target.id();
    commands.entity(preview.ui_root).try_despawn();
    commands.entity(preview.scene_root).try_despawn();
    commands.queue(move |world: &mut World| {
        if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
            images.remove(target_id);
        }
    });
}

fn spawn_model_library_preview_scene(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
) -> Result<SpawnedModelLibraryPreviewScene, String> {
    if library.get(prefab_id).is_none() {
        return Err(format!(
            "Cannot preview prefab {}: prefab def is not loaded.",
            uuid::Uuid::from_u128(prefab_id)
        ));
    }

    let size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M))
        .abs()
        .max(Vec3::splat(0.01));
    let origin_y = library.ground_origin_y_or_default(prefab_id);
    let center_y = size.y * 0.5 - origin_y;
    let focus = if center_y.is_finite() {
        Vec3::new(0.0, center_y, 0.0)
    } else {
        Vec3::ZERO
    };

    let target = crate::orbit_capture::create_render_target(
        images,
        PREFAB_PREVIEW_WIDTH_PX,
        PREFAB_PREVIEW_HEIGHT_PX,
    );

    let aspect = PREFAB_PREVIEW_WIDTH_PX.max(1) as f32 / PREFAB_PREVIEW_HEIGHT_PX.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;

    let yaw = std::f32::consts::FRAC_PI_6;
    let pitch = -0.45;
    let half_extents = size * 0.5;
    let base_distance = crate::orbit_capture::required_distance_for_view(
        half_extents,
        yaw,
        pitch,
        fov_y,
        aspect,
        near,
    );
    let distance = (base_distance * 1.08).clamp(near + 0.2, 500.0);
    let camera_transform = crate::orbit_capture::orbit_transform(yaw, pitch, distance, focus);

    let render_layer = bevy::camera::visibility::RenderLayers::layer(PREFAB_PREVIEW_LAYER);

    let scene_root = commands
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            ModelLibraryPreviewSceneRoot,
        ))
        .id();

    let model_id = {
        let mut entity = commands.spawn((Transform::IDENTITY, Visibility::Inherited));
        visuals::spawn_object_visuals_with_settings(
            &mut entity,
            library,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            prefab_id,
            None,
            visuals::VisualSpawnSettings {
                mark_parts: false,
                render_layer: Some(PREFAB_PREVIEW_LAYER),
            },
        );
        entity.id()
    };
    commands.entity(scene_root).add_child(model_id);

    let lights = [
        (
            Vec3::new(10.0, 18.0, -8.0),
            16_000.0,
            true,
            Color::srgb(1.0, 0.97, 0.94),
        ),
        (
            Vec3::new(-10.0, 10.0, 6.0),
            6_500.0,
            false,
            Color::srgb(0.90, 0.95, 1.0),
        ),
        (
            Vec3::new(0.0, 12.0, -12.0),
            4_000.0,
            false,
            Color::srgb(1.0, 1.0, 1.0),
        ),
        (
            Vec3::new(0.0, -14.0, 0.0),
            4_500.0,
            false,
            Color::srgb(0.96, 0.97, 1.0),
        ),
    ];
    for (offset, illuminance, shadows_enabled, color) in lights {
        let light_id = commands
            .spawn((
                DirectionalLight {
                    shadows_enabled,
                    illuminance,
                    color,
                    ..default()
                },
                Transform::from_translation(focus + offset).looking_at(focus, Vec3::Y),
                render_layer.clone(),
            ))
            .id();
        commands.entity(scene_root).add_child(light_id);
    }

    let camera_id = commands
        .spawn((
            Camera3d::default(),
            bevy::camera::Projection::Perspective(projection),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::srgb(0.93, 0.94, 0.96)),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            Tonemapping::TonyMcMapface,
            render_layer.clone(),
            camera_transform,
            ModelLibraryPreviewCamera,
        ))
        .id();
    commands.entity(scene_root).add_child(camera_id);

    Ok(SpawnedModelLibraryPreviewScene {
        scene_root,
        target,
        focus,
        yaw,
        pitch,
        distance,
    })
}

pub(crate) fn model_library_open_preview_panel(
    mut commands: Commands,
    env: ModelLibraryEnv,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
    mut images: ResMut<Assets<Image>>,
    mut library: ResMut<ObjectLibrary>,
    descriptors: Res<PrefabDescriptorLibrary>,
    mut state: ResMut<ModelLibraryUiState>,
) {
    if !state.is_open() || !matches!(env.build_scene.get(), crate::types::BuildScene::Realm) {
        state.pending_preview = None;
        return;
    }

    let Some(prefab_id) = state.pending_preview.take() else {
        return;
    };
    state.search_focused = false;

    if state
        .preview
        .as_ref()
        .is_some_and(|p| p.prefab_id == prefab_id)
    {
        return;
    }
    close_model_library_preview(&mut commands, &mut state);

    if let Err(err) = ensure_realm_prefab_loaded(&env.active, prefab_id, &mut library) {
        warn!("{err}");
        return;
    }

    let scene = match spawn_model_library_preview_scene(
        &mut commands,
        &mut images,
        &asset_server,
        &assets,
        &mut meshes,
        &mut materials,
        &mut material_cache,
        &mut mesh_cache,
        &library,
        prefab_id,
    ) {
        Ok(v) => v,
        Err(err) => {
            warn!("{err}");
            return;
        }
    };

    let uuid = uuid::Uuid::from_u128(prefab_id).to_string();
    let desc = descriptors.get(prefab_id);
    let name = desc
        .and_then(|d| d.label.as_ref())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(|| uuid.clone());
    let tags = desc.map(|d| d.tags.clone()).unwrap_or_default();
    let roles = desc.map(|d| d.roles.clone()).unwrap_or_default();
    let short = desc
        .and_then(|d| d.text.as_ref())
        .and_then(|t| t.short.as_deref())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());
    let long = desc
        .and_then(|d| d.text.as_ref())
        .and_then(|t| t.long.as_deref())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());
    let gen3d_prompt = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.gen3d.as_ref())
        .and_then(|g| g.prompt.as_deref())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());
    let gen3d_descriptor_meta = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.gen3d.as_ref())
        .and_then(|g| g.extra.get("descriptor_meta_v1"));
    let revisions = desc
        .and_then(|d| d.provenance.as_ref())
        .map(|p| p.revisions.as_slice())
        .unwrap_or(&[]);

    let modified_at_ms = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.modified_at_ms);
    let created_at_ms = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.created_at_ms);

    let mut meta = String::new();
    meta.push_str(&format!("Name: {name}\n"));
    meta.push_str(&format!("ID: {uuid}\n"));
    if let Some(modified_at_ms) = modified_at_ms {
        meta.push_str(&format!("Modified: {modified_at_ms}\n"));
    }
    if let Some(created_at_ms) = created_at_ms {
        meta.push_str(&format!("Created: {created_at_ms}\n"));
    }
    if !roles.is_empty() {
        meta.push_str(&format!("Roles: {}\n", roles.join(", ")));
    }
    if !tags.is_empty() {
        meta.push_str(&format!("Tags: {}\n", tags.join(", ")));
    }
    if let Some(size) = library.size(prefab_id) {
        meta.push_str(&format!(
            "Size (m): [{:.3}, {:.3}, {:.3}]\n",
            size.x, size.y, size.z
        ));
    }

    meta.push_str("\nDescriptions\n");
    meta.push_str("Short:\n");
    if let Some(short) = short {
        meta.push_str(short);
        meta.push('\n');
    } else {
        meta.push_str("<none>\n");
    }
    meta.push('\n');
    meta.push_str("Long:\n");
    if let Some(long) = long {
        meta.push_str(long);
        meta.push('\n');
    } else {
        meta.push_str("<none>\n");
    }

    if let Some(gen3d_prompt) = gen3d_prompt {
        meta.push('\n');
        meta.push_str("Gen3D prompt:\n");
        meta.push_str(gen3d_prompt);
        meta.push('\n');
    }

    if let Some(meta_json) = gen3d_descriptor_meta {
        let name = meta_json
            .get("name")
            .and_then(|v| v.as_str())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        let short = meta_json
            .get("short")
            .and_then(|v| v.as_str())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        let tags = meta_json
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        meta.push('\n');
        meta.push_str("AI enriched (descriptor_meta_v1):\n");
        if let Some(name) = name {
            meta.push_str(&format!("- name: {name}\n"));
        }
        if let Some(short) = short {
            meta.push_str("- short: ");
            meta.push_str(short);
            meta.push('\n');
        }
        if !tags.is_empty() {
            meta.push_str(&format!("- tags: {}\n", tags.join(", ")));
        }
    }

    if !revisions.is_empty() {
        const MAX_REVISIONS: usize = 32;
        let total = revisions.len();

        meta.push('\n');
        meta.push_str("Revision prompts (newest first):\n");
        for rev in revisions.iter().rev().take(MAX_REVISIONS) {
            meta.push_str(&format!(
                "- rev {} ({}) {}: {}\n",
                rev.rev, rev.created_at_ms, rev.actor, rev.summary
            ));

            if let Some(prompt) = rev.extra.get("prompt").and_then(|v| v.as_str()) {
                let prompt = prompt.trim();
                if !prompt.is_empty() {
                    meta.push_str("  prompt:\n");
                    for line in prompt.lines() {
                        meta.push_str("    ");
                        meta.push_str(line);
                        meta.push('\n');
                    }
                }
            }

            if let Some(desc_meta) = rev.extra.get("descriptor_meta_v1") {
                let name = desc_meta
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty());
                let short = desc_meta
                    .get("short")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty());
                let tags = desc_meta
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                if name.is_some() || short.is_some() || !tags.is_empty() {
                    meta.push_str("  ai:\n");
                    if let Some(name) = name {
                        meta.push_str(&format!("    name: {name}\n"));
                    }
                    if let Some(short) = short {
                        meta.push_str("    short: ");
                        meta.push_str(short);
                        meta.push('\n');
                    }
                    if !tags.is_empty() {
                        meta.push_str(&format!("    tags: {}\n", tags.join(", ")));
                    }
                }
            }
        }
        if total > MAX_REVISIONS {
            meta.push_str(&format!(
                "… ({} older revisions omitted)\n",
                total - MAX_REVISIONS
            ));
        }
    }

    let ui_root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(64.0),
                left: Val::Px(300.0),
                width: Val::Px(720.0),
                height: Val::Px(560.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.94)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.85)),
            Outline {
                width: Val::Px(1.0),
                color: Color::srgba(0.25, 0.25, 0.30, 0.85),
                offset: Val::Px(0.0),
            },
            ZIndex(PREFAB_PREVIEW_Z_INDEX),
            ModelLibraryPreviewOverlayRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(name.clone()),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.95, 0.97)),
                ));

                row.spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    ModelLibraryPreviewCloseButton,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Exit"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    ));
                });
            });

            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|container| {
                crate::gen3d::spawn_gen3d_preview_panel(
                    container,
                    Node {
                        width: Val::Px(PREFAB_PREVIEW_WIDTH_PX as f32),
                        height: Val::Px(PREFAB_PREVIEW_HEIGHT_PX as f32),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    scene.target.clone(),
                    |_preview| {},
                );
            });

            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    min_height: Val::Px(0.0),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.01, 0.01, 0.015, 0.65)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
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
                    ModelLibraryPreviewInfoScrollPanel,
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new(meta),
                        Node {
                            width: Val::Percent(100.0),
                            align_self: AlignSelf::FlexStart,
                            ..default()
                        },
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.90, 0.90, 0.94)),
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
                    ModelLibraryPreviewInfoScrollbarTrack,
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
                        ModelLibraryPreviewInfoScrollbarThumb,
                    ));
                });
            });
        })
        .id();

    state.preview = Some(ModelLibraryPrefabPreview {
        prefab_id,
        ui_root,
        scene_root: scene.scene_root,
        target: scene.target,
        focus: scene.focus,
        yaw: scene.yaw,
        pitch: scene.pitch,
        distance: scene.distance,
        last_cursor: None,
    });
}

pub(crate) fn model_library_preview_close_button_interactions(
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ModelLibraryPreviewCloseButton>)>,
) {
    if state.preview.is_none() {
        return;
    }
    for interaction in &mut buttons {
        if *interaction == Interaction::Pressed {
            close_model_library_preview(&mut commands, &mut state);
            break;
        }
    }
}

pub(crate) fn model_library_preview_close_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
) {
    if state.preview.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        close_model_library_preview(&mut commands, &mut state);
    }
}

pub(crate) fn model_library_preview_keyboard_navigation(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<ModelLibraryUiState>,
) {
    let active = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm)
        && !state.search_focused;
    if !active {
        return;
    }

    let Some(current_prefab) = state.preview.as_ref().map(|p| p.prefab_id) else {
        return;
    };
    if state.listed_prefabs.is_empty() {
        return;
    }

    let direction = if keys.just_pressed(KeyCode::ArrowDown) {
        1_i32
    } else if keys.just_pressed(KeyCode::ArrowUp) {
        -1_i32
    } else {
        return;
    };

    let current_index = state
        .listed_prefabs
        .iter()
        .position(|id| *id == current_prefab)
        .unwrap_or(0) as i32;
    let max_index = state.listed_prefabs.len().saturating_sub(1) as i32;

    let next_index = (current_index + direction).clamp(0, max_index) as usize;
    let next_prefab = state.listed_prefabs[next_index];
    if next_prefab != current_prefab {
        state.pending_preview = Some(next_prefab);
    }
}

pub(crate) fn model_library_preview_orbit_controls(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<MouseWheel>,
    panel: Query<&Interaction, With<crate::gen3d::Gen3dPreviewPanel>>,
    mut state: ResMut<ModelLibraryUiState>,
    mut cameras: Query<&mut Transform, With<ModelLibraryPreviewCamera>>,
) {
    let active = state.is_open()
        && state.preview.is_some()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    if !active {
        for _ in mouse_wheel.read() {}
        return;
    }

    let Ok(window) = windows.single() else {
        for _ in mouse_wheel.read() {}
        return;
    };

    let hovered = panel
        .iter()
        .any(|i| matches!(*i, Interaction::Hovered | Interaction::Pressed));

    let Some(preview) = state.preview.as_mut() else {
        for _ in mouse_wheel.read() {}
        return;
    };

    let cursor = window.cursor_position();

    if hovered {
        let mut scroll = 0.0f32;
        for ev in mouse_wheel.read() {
            let delta = match ev.unit {
                MouseScrollUnit::Line => ev.y,
                MouseScrollUnit::Pixel => ev.y / 120.0,
            };
            scroll += delta;
        }
        if scroll.abs() > 1e-4 {
            preview.distance = (preview.distance - scroll * 0.6).clamp(0.5, 500.0);
        }
    } else {
        for _ in mouse_wheel.read() {}
    }

    let dragging = hovered && mouse_buttons.pressed(MouseButton::Left);
    if dragging {
        if let (Some(prev), Some(cur)) = (preview.last_cursor, cursor) {
            let delta = cur - prev;
            let sensitivity = 0.010;
            preview.yaw = wrap_angle(preview.yaw - delta.x * sensitivity);
            preview.pitch = (preview.pitch + delta.y * sensitivity).clamp(-1.56, 1.56);
        }
    }

    preview.last_cursor = if hovered { cursor } else { None };

    let camera_transform = crate::orbit_capture::orbit_transform(
        preview.yaw,
        preview.pitch,
        preview.distance,
        preview.focus,
    );
    for mut transform in &mut cameras {
        *transform = camera_transform;
    }
}

pub(crate) fn model_library_update_scrollbar_ui(
    panels: Query<(&ComputedNode, &ScrollPosition), With<ModelLibraryScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<ModelLibraryScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<ModelLibraryScrollbarThumb>>,
) {
    let Ok((panel, scroll_pos)) = panels.single() else {
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
        thumb.top = Val::Px(0.0);
        thumb.height = Val::Px(track_h);
        return;
    }

    *track_vis = Visibility::Inherited;

    let max_scroll = (content_h - viewport_h).max(1.0);
    let scroll_y = scroll_pos.y.clamp(0.0, max_scroll);

    let min_thumb_h = 14.0;
    let thumb_h = (viewport_h * viewport_h / content_h).clamp(min_thumb_h, track_h);
    let max_thumb_top = (track_h - thumb_h).max(0.0);
    let thumb_top = (max_thumb_top * (scroll_y / max_scroll)).clamp(0.0, max_thumb_top);

    thumb.top = Val::Px(thumb_top);
    thumb.height = Val::Px(thumb_h);
}

fn wrap_angle(mut v: f32) -> f32 {
    while v > std::f32::consts::PI {
        v -= std::f32::consts::TAU;
    }
    while v < -std::f32::consts::PI {
        v += std::f32::consts::TAU;
    }
    v
}

pub(crate) fn model_library_update_preview_info_scrollbar_ui(
    panels: Query<(&ComputedNode, &ScrollPosition), With<ModelLibraryPreviewInfoScrollPanel>>,
    mut tracks: Query<
        (&ComputedNode, &mut Visibility),
        With<ModelLibraryPreviewInfoScrollbarTrack>,
    >,
    mut thumbs: Query<&mut Node, With<ModelLibraryPreviewInfoScrollbarThumb>>,
) {
    let Ok((panel, scroll_pos)) = panels.single() else {
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
        thumb.top = Val::Px(0.0);
        thumb.height = Val::Px(track_h);
        return;
    }

    *track_vis = Visibility::Inherited;

    let max_scroll = (content_h - viewport_h).max(1.0);
    let scroll_y = scroll_pos.y.clamp(0.0, max_scroll);

    let min_thumb_h = 14.0;
    let thumb_h = (viewport_h * viewport_h / content_h).clamp(min_thumb_h, track_h);
    let max_thumb_top = (track_h - thumb_h).max(0.0);
    let thumb_top = (max_thumb_top * (scroll_y / max_scroll)).clamp(0.0, max_thumb_top);

    thumb.top = Val::Px(thumb_top);
    thumb.height = Val::Px(thumb_h);
}

pub(crate) fn model_library_scroll_wheel(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<MouseWheel>,
    state: Res<ModelLibraryUiState>,
    roots: Query<(&ComputedNode, &UiGlobalTransform, &Visibility), With<ModelLibraryRoot>>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<ModelLibraryScrollPanel>>,
) {
    if !state.is_open()
        || !matches!(mode.get(), GameMode::Build)
        || !matches!(build_scene.get(), crate::types::BuildScene::Realm)
        || state.scrollbar_drag.is_some()
    {
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

pub(crate) fn model_library_preview_info_scroll_wheel(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<MouseWheel>,
    state: Res<ModelLibraryUiState>,
    mut panels: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<ModelLibraryPreviewInfoScrollPanel>,
    >,
) {
    let active = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm)
        && state.preview.is_some()
        && state.preview_scrollbar_drag.is_none();
    if !active {
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

    let Ok((panel_node, panel_transform, mut scroll)) = panels.single_mut() else {
        for _ in mouse_wheel.read() {}
        return;
    };
    if !panel_node.contains_point(*panel_transform, cursor) {
        for _ in mouse_wheel.read() {}
        return;
    }

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

pub(crate) fn model_library_scrollbar_drag(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<ModelLibraryUiState>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<ModelLibraryScrollPanel>>,
    tracks: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<ModelLibraryScrollbarTrack>,
    >,
    thumbs: Query<(&Interaction, &ComputedNode, &Node), With<ModelLibraryScrollbarThumb>>,
) {
    let active = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    if !active {
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
            state.scrollbar_drag = Some(ModelLibraryScrollbarDrag { grab_offset });
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

pub(crate) fn model_library_preview_info_scrollbar_drag(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<ModelLibraryUiState>,
    mut panels: Query<
        (&ComputedNode, &mut ScrollPosition),
        With<ModelLibraryPreviewInfoScrollPanel>,
    >,
    tracks: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<ModelLibraryPreviewInfoScrollbarTrack>,
    >,
    thumbs: Query<
        (&Interaction, &ComputedNode, &Node),
        With<ModelLibraryPreviewInfoScrollbarThumb>,
    >,
) {
    let active = state.is_open()
        && state.preview.is_some()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    if !active {
        state.preview_scrollbar_drag = None;
        return;
    }

    if !mouse_buttons.pressed(MouseButton::Left) {
        state.preview_scrollbar_drag = None;
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
        state.preview_scrollbar_drag = None;
        return;
    }
    let Ok((interaction, thumb_node, thumb_layout)) = thumbs.single() else {
        return;
    };

    if state.preview_scrollbar_drag.is_none() && *interaction == Interaction::Pressed {
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
            state.preview_scrollbar_drag = Some(ModelLibraryScrollbarDrag { grab_offset });
        }
    }

    let Some(drag) = state.preview_scrollbar_drag else {
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

pub(crate) fn model_library_gen3d_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mut next_build_scene: ResMut<NextState<crate::types::BuildScene>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ModelLibraryGen3dButton>),
    >,
) {
    for (interaction, mut bg, mut border) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75));
                *border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.07, 0.07, 0.09, 0.84));
                *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.75));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92));
                *border = BorderColor::all(Color::srgba(0.45, 0.45, 0.55, 0.85));

                if !matches!(mode.get(), GameMode::Build) {
                    continue;
                }

                match build_scene.get() {
                    crate::types::BuildScene::Realm => {
                        next_build_scene.set(crate::types::BuildScene::Preview);
                    }
                    crate::types::BuildScene::Preview => {
                        next_build_scene.set(crate::types::BuildScene::Realm);
                    }
                }
            }
        }
    }
}

pub(crate) fn model_library_item_button_interactions(
    mut commands: Commands,
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mut next_build_scene: ResMut<NextState<crate::types::BuildScene>>,
    config: Res<crate::config::AppConfig>,
    mut mock_jobs: ResMut<crate::gen3d::Gen3dMockJobManager>,
    mut state: ResMut<ModelLibraryUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut buttons: Query<(&Interaction, &ModelLibraryItemButton), Changed<Interaction>>,
) {
    let cursor = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position());
    for (interaction, button) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }

        if button.kind.is_in_flight() {
            if !matches!(mode.get(), GameMode::Build) {
                continue;
            }
            if state.preview.is_some() {
                close_model_library_preview(&mut commands, &mut state);
            }
            state.drag = None;
            state.pending_preview = None;
            if config.gen3d_mock_enabled {
                if let ModelLibraryItemKind::InFlight { run_id } = button.kind {
                    let run_id = uuid::Uuid::from_u128(run_id);
                    let _ = crate::gen3d::gen3d_mock_select_active_run(&mut mock_jobs, run_id);
                }
            }
            if matches!(build_scene.get(), crate::types::BuildScene::Realm) {
                next_build_scene.set(crate::types::BuildScene::Preview);
            }
            continue;
        }

        let Some(prefab_id) = button.kind.prefab_id() else {
            continue;
        };

        if state.drag.is_some() {
            continue;
        }

        if let Some(cursor) = cursor {
            state.drag = Some(ModelLibraryDrag {
                model_id: prefab_id,
                start_cursor: cursor,
                is_dragging: false,
                preview_translation: None,
            });
        }
    }
}

pub(crate) fn model_library_in_flight_remove_button_interactions(
    active: Res<crate::realm::ActiveRealmScene>,
    mut state: ResMut<ModelLibraryUiState>,
    mut workshop: ResMut<crate::gen3d::Gen3dWorkshop>,
    mut job: ResMut<crate::gen3d::Gen3dAiJob>,
    mut mock_jobs: ResMut<crate::gen3d::Gen3dMockJobManager>,
    mut buttons: Query<
        (
            &Interaction,
            &ModelLibraryInFlightRemoveButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Changed<Interaction>,
    >,
) {
    for (interaction, button, mut bg, mut border) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92));
                *border = BorderColor::all(Color::srgba(0.45, 0.45, 0.55, 0.85));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.16, 0.10, 0.10, 0.95));
                *border = BorderColor::all(Color::srgba(0.65, 0.25, 0.25, 0.90));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.18, 0.08, 0.08, 0.98));
                *border = BorderColor::all(Color::srgba(0.80, 0.25, 0.25, 0.95));

                let run_id = uuid::Uuid::from_u128(button.run_id);
                let mut removed = false;
                if let Ok(found) =
                    crate::gen3d::gen3d_mock_cancel_run(&mut mock_jobs, &active.realm_id, run_id)
                {
                    removed = found;
                }
                if job.run_id() == Some(run_id) && (job.is_running() || job.can_resume()) {
                    crate::gen3d::gen3d_cancel_build_from_api(
                        &mut workshop,
                        &mut job,
                        Some(&mut mock_jobs),
                    );
                }
                if !removed {
                    if let Err(err) =
                        crate::gen3d::remove_gen3d_in_flight_entry(&active.realm_id, run_id)
                    {
                        warn!("Failed to remove Gen3D in-flight entry: {err}");
                    }
                }
                state.mark_models_dirty();
            }
        }
    }
}

pub(crate) fn model_library_update_list_item_styles(
    state: Res<ModelLibraryUiState>,
    mut last_selected: Local<Option<u128>>,
    mut buttons: Query<
        (
            Ref<Interaction>,
            &ModelLibraryItemButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<ModelLibraryListItem>,
    >,
    mut marks: Query<(Ref<ModelLibrarySelectionMark>, &mut Visibility)>,
) {
    let selected_id = state.preview.as_ref().map(|p| p.prefab_id);
    let selection_changed = *last_selected != selected_id;
    if selection_changed {
        *last_selected = selected_id;
    }

    for (interaction, button, mut bg, mut border) in &mut buttons {
        if !selection_changed && !interaction.is_changed() && !interaction.is_added() {
            continue;
        }

        let is_selected = button
            .kind
            .prefab_id()
            .is_some_and(|id| selected_id == Some(id));

        let (bg_color, border_color) = match *interaction {
            Interaction::Pressed => (
                Color::srgba(0.10, 0.10, 0.12, 0.92),
                if is_selected {
                    Color::srgba(0.30, 0.97, 0.87, 0.95)
                } else {
                    Color::srgba(0.45, 0.45, 0.55, 0.85)
                },
            ),
            Interaction::Hovered => (
                if is_selected {
                    Color::srgba(0.08, 0.08, 0.10, 0.86)
                } else {
                    Color::srgba(0.07, 0.07, 0.09, 0.84)
                },
                if is_selected {
                    Color::srgba(0.25, 0.95, 0.85, 0.85)
                } else {
                    Color::srgba(0.35, 0.35, 0.42, 0.75)
                },
            ),
            Interaction::None => (
                if is_selected {
                    Color::srgba(0.06, 0.06, 0.08, 0.82)
                } else {
                    Color::srgba(0.05, 0.05, 0.06, 0.75)
                },
                if is_selected {
                    Color::srgba(0.25, 0.95, 0.85, 0.85)
                } else {
                    Color::srgba(0.25, 0.25, 0.30, 0.65)
                },
            ),
        };

        *bg = BackgroundColor(bg_color);
        *border = BorderColor::all(border_color);
    }

    for (mark, mut vis) in &mut marks {
        if !selection_changed && !mark.is_added() {
            continue;
        }
        *vis = if selected_id == Some(mark.prefab_id) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

fn ui_rect_global_y_bounds(rect: bevy::math::Rect, transform: UiGlobalTransform) -> (f32, f32) {
    let corners = [
        rect.min,
        Vec2::new(rect.max.x, rect.min.y),
        Vec2::new(rect.min.x, rect.max.y),
        rect.max,
    ];
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for corner in corners {
        let point = transform.transform_point2(corner);
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }
    (min_y, max_y)
}

pub(crate) fn model_library_scroll_selected_item_into_view(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    state: Res<ModelLibraryUiState>,
    mut last_scrolled: Local<Option<u128>>,
    mut panels: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<ModelLibraryScrollPanel>,
    >,
    items: Query<
        (&ModelLibraryItemButton, &ComputedNode, &UiGlobalTransform),
        With<ModelLibraryListItem>,
    >,
) {
    let active = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm)
        && state.scrollbar_drag.is_none();
    if !active {
        *last_scrolled = None;
        return;
    }

    let Some(selected_id) = state.preview.as_ref().map(|p| p.prefab_id) else {
        *last_scrolled = None;
        return;
    };
    if *last_scrolled == Some(selected_id) {
        return;
    }

    let Ok((panel_node, panel_transform, mut scroll)) = panels.single_mut() else {
        return;
    };

    let Some((item_node, item_transform)) = items
        .iter()
        .find(|(button, _node, _transform)| {
            button.kind.prefab_id().is_some_and(|id| id == selected_id)
        })
        .map(|(_button, node, transform)| (node, transform))
    else {
        return;
    };

    let panel_scale = panel_node.inverse_scale_factor();
    let viewport_h = panel_node.size.y.max(0.0) * panel_scale;
    let content_h = panel_node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
        scroll.y = 0.0;
        *last_scrolled = Some(selected_id);
        return;
    }
    let max_scroll = (content_h - viewport_h).max(0.0);

    let (panel_top_y, panel_bottom_y) =
        ui_rect_global_y_bounds(panel_node.content_box(), *panel_transform);
    let (item_top_y, item_bottom_y) =
        ui_rect_global_y_bounds(item_node.border_box(), *item_transform);

    let margin_physical = 6.0;
    let mut next_scroll = scroll.y;

    if item_top_y < panel_top_y + margin_physical {
        let delta = (panel_top_y + margin_physical - item_top_y).max(0.0) * panel_scale;
        next_scroll = (next_scroll - delta).clamp(0.0, max_scroll);
    } else if item_bottom_y > panel_bottom_y - margin_physical {
        let delta = (item_bottom_y - (panel_bottom_y - margin_physical)).max(0.0) * panel_scale;
        next_scroll = (next_scroll + delta).clamp(0.0, max_scroll);
    }

    scroll.y = next_scroll;
    *last_scrolled = Some(selected_id);
}

pub(crate) fn model_library_drag_update(
    mut commands: Commands,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    env: ModelLibraryEnv,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &Transform), With<crate::types::MainCamera>>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
    mut library: ResMut<ObjectLibrary>,
    objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    mut scene_saves: bevy::ecs::message::MessageWriter<SceneSaveRequest>,
    mut state: ResMut<ModelLibraryUiState>,
) {
    if !state.is_open() || !matches!(env.build_scene.get(), crate::types::BuildScene::Realm) {
        state.drag = None;
        return;
    }

    let Some(mut drag) = state.drag else {
        return;
    };

    if !mouse_buttons.pressed(MouseButton::Left) {
        // Mouse was released; treat as either click-to-preview or drag-spawn.
        let prefab_id = drag.model_id;
        if let Err(err) = ensure_realm_prefab_loaded(&env.active, prefab_id, &mut library) {
            warn!("{err}");
            state.drag = None;
            return;
        }
        if drag.is_dragging && drag.preview_translation.is_some() {
            let spawn_translation = drag.preview_translation.unwrap();
            let spawned = spawn_prefab_instance(
                &mut commands,
                &asset_server,
                &assets,
                &mut meshes,
                &mut materials,
                &mut material_cache,
                &mut mesh_cache,
                &library,
                prefab_id,
                spawn_translation,
            );
            if spawned.is_some() {
                state.spawn_seq = state.spawn_seq.wrapping_add(1);
                scene_saves.write(SceneSaveRequest::new("spawned prefab from realm"));
            }
        } else {
            state.pending_preview = Some(prefab_id);
        }

        state.drag = None;
        return;
    }

    let cursor = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position());
    if let Some(cursor) = cursor {
        if !drag.is_dragging && cursor.distance(drag.start_cursor) > DRAG_START_THRESHOLD_PX {
            drag.is_dragging = true;
            if state.preview.is_some() {
                close_model_library_preview(&mut commands, &mut state);
            }
        }

        if drag.is_dragging {
            if let Err(err) = ensure_realm_prefab_loaded(&env.active, drag.model_id, &mut library) {
                warn!("{err}");
                drag.preview_translation = None;
                state.drag = Some(drag);
                return;
            }

            let Ok((camera, camera_transform)) = camera_q.single() else {
                drag.preview_translation = None;
                state.drag = Some(drag);
                return;
            };
            let camera_global = GlobalTransform::from(*camera_transform);
            if let Ok(window) = windows.single() {
                if let Some(pick) = crate::cursor_pick::cursor_surface_pick(
                    window,
                    camera,
                    &camera_global,
                    &library,
                    &objects,
                ) {
                    drag.preview_translation = Some(spawn_at_pick(
                        drag.model_id,
                        pick.hit,
                        pick.surface_y,
                        &library,
                    ));
                } else {
                    drag.preview_translation = None;
                }
            }
        }
    }

    state.drag = Some(drag);
}

pub(crate) fn model_library_draw_drag_preview_gizmos(
    mut gizmos: Gizmos,
    mode: Res<State<GameMode>>,
    library: Res<ObjectLibrary>,
    state: Res<ModelLibraryUiState>,
) {
    if !matches!(mode.get(), GameMode::Build) {
        return;
    }
    let Some(drag) = state.drag else {
        return;
    };
    if !drag.is_dragging {
        return;
    }
    let Some(translation) = drag.preview_translation else {
        return;
    };

    let (size, half_xz, origin_y) = prefab_bounds(&library, drag.model_id, Vec3::ONE);
    let bottom_y = translation.y - origin_y;
    let top_y = bottom_y + size.y;

    let min = Vec3::new(
        translation.x - half_xz.x,
        bottom_y,
        translation.z - half_xz.y,
    );
    let max = Vec3::new(translation.x + half_xz.x, top_y, translation.z + half_xz.y);

    draw_dashed_box(&mut gizmos, min, max, Color::srgb(0.25, 0.95, 0.85));
}

fn ensure_realm_prefab_loaded(
    active: &crate::realm::ActiveRealmScene,
    prefab_id: u128,
    library: &mut ObjectLibrary,
) -> Result<(), String> {
    if library.get(prefab_id).is_some() {
        return Ok(());
    }

    let loaded = crate::realm_prefab_packages::load_realm_prefab_package_defs_into_library(
        &active.realm_id,
        prefab_id,
        library,
    )?;
    if loaded == 0 {
        return Err(format!(
            "Prefab {} is not loaded and no realm prefab package was found under {}.",
            uuid::Uuid::from_u128(prefab_id),
            active.realm_id
        ));
    }

    Ok(())
}

fn prefab_bounds(library: &ObjectLibrary, prefab_id: u128, scale: Vec3) -> (Vec3, Vec2, f32) {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));
    let scale = Vec3::new(
        if scale.x.is_finite() {
            scale.x.abs().max(1e-4)
        } else {
            1.0
        },
        if scale.y.is_finite() {
            scale.y.abs().max(1e-4)
        } else {
            1.0
        },
        if scale.z.is_finite() {
            scale.z.abs().max(1e-4)
        } else {
            1.0
        },
    );

    let half_unscaled = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => Vec2::splat(radius.max(0.01)),
        Some(ColliderProfile::AabbXZ { half_extents }) => Vec2::new(
            half_extents.x.abs().max(0.01),
            half_extents.y.abs().max(0.01),
        ),
        _ => Vec2::new(
            (base_size.x * 0.5).abs().max(0.01),
            (base_size.z * 0.5).abs().max(0.01),
        ),
    };

    let half_xz = Vec2::new(half_unscaled.x * scale.x, half_unscaled.y * scale.z);
    let size = Vec3::new(
        half_xz.x * 2.0,
        base_size.y.abs() * scale.y,
        half_xz.y * 2.0,
    );
    let origin_y = library.ground_origin_y_or_default(prefab_id) * scale.y;

    (size, half_xz, origin_y)
}

fn spawn_at_pick(prefab_id: u128, hit: Vec3, surface_y: f32, library: &ObjectLibrary) -> Vec3 {
    let (_size, half_xz, origin_y) = prefab_bounds(library, prefab_id, Vec3::ONE);
    let mobility_mode = library.mobility(prefab_id).map(|m| m.mode);

    let mut pos = Vec3::new(hit.x, surface_y + origin_y, hit.z);
    pos.x = snap_to_grid(pos.x, BUILD_GRID_SIZE);
    pos.z = snap_to_grid(pos.z, BUILD_GRID_SIZE);
    pos.y = match mobility_mode {
        Some(MobilityMode::Air) => surface_y + origin_y + BUILD_UNIT_SIZE * 8.0,
        _ => surface_y + origin_y,
    };

    pos.x = clamp_world_xz(pos.x, half_xz.x);
    pos.z = clamp_world_xz(pos.z, half_xz.y);

    pos
}

fn spawn_prefab_instance(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
    translation: Vec3,
) -> Option<Entity> {
    if library.get(prefab_id).is_none() {
        warn!(
            "Cannot spawn prefab {}: prefab def is not loaded.",
            uuid::Uuid::from_u128(prefab_id)
        );
        return None;
    }

    let instance_id = ObjectId::new_v4();
    let transform = Transform::from_translation(translation);
    let mobility = library.mobility(prefab_id).is_some();

    let (size, half_xz, _origin_y) = prefab_bounds(library, prefab_id, transform.scale);
    let object_radius = half_xz.x.max(half_xz.y).max(0.1);

    let mut entity_commands = if mobility {
        commands.spawn((
            instance_id,
            ObjectPrefabId(prefab_id),
            Commandable,
            Collider {
                radius: object_radius,
            },
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
                half_extents: half_xz,
            },
            transform,
            Visibility::Inherited,
        ))
    };

    visuals::spawn_object_visuals(
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

    Some(entity_commands.id())
}

fn draw_dashed_box(gizmos: &mut Gizmos, min: Vec3, max: Vec3, color: Color) {
    let c0 = Vec3::new(min.x, min.y, min.z);
    let c1 = Vec3::new(max.x, min.y, min.z);
    let c2 = Vec3::new(max.x, min.y, max.z);
    let c3 = Vec3::new(min.x, min.y, max.z);
    let c4 = Vec3::new(min.x, max.y, min.z);
    let c5 = Vec3::new(max.x, max.y, min.z);
    let c6 = Vec3::new(max.x, max.y, max.z);
    let c7 = Vec3::new(min.x, max.y, max.z);

    let dash = 0.12;
    let gap = 0.10;
    draw_dashed_line(gizmos, c0, c1, dash, gap, color);
    draw_dashed_line(gizmos, c1, c2, dash, gap, color);
    draw_dashed_line(gizmos, c2, c3, dash, gap, color);
    draw_dashed_line(gizmos, c3, c0, dash, gap, color);

    draw_dashed_line(gizmos, c4, c5, dash, gap, color);
    draw_dashed_line(gizmos, c5, c6, dash, gap, color);
    draw_dashed_line(gizmos, c6, c7, dash, gap, color);
    draw_dashed_line(gizmos, c7, c4, dash, gap, color);

    draw_dashed_line(gizmos, c0, c4, dash, gap, color);
    draw_dashed_line(gizmos, c1, c5, dash, gap, color);
    draw_dashed_line(gizmos, c2, c6, dash, gap, color);
    draw_dashed_line(gizmos, c3, c7, dash, gap, color);
}

fn draw_dashed_line(
    gizmos: &mut Gizmos,
    start: Vec3,
    end: Vec3,
    dash_len: f32,
    gap_len: f32,
    color: Color,
) {
    let delta = end - start;
    let length = delta.length();
    if length <= 1e-4 {
        return;
    }

    let dash_len = dash_len.max(0.005);
    let step = (dash_len + gap_len).max(0.005);
    let dir = delta / length;

    let mut dist = 0.0;
    while dist < length {
        let segment_start = start + dir * dist;
        let segment_end = start + dir * (dist + dash_len).min(length);
        gizmos.line(segment_start, segment_end, color);
        dist += step;
    }
}
