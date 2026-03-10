use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::collections::HashMap;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::geometry::{clamp_world_xz, snap_to_grid};
use crate::object::registry::ObjectLibrary;
use crate::object::registry::{ColliderProfile, MobilityMode};
use crate::object::visuals;
use crate::prefab_descriptors::PrefabDescriptorLibrary;
use crate::scene_store::SceneSaveRequest;
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, GameMode, ObjectId,
    ObjectPrefabId,
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

#[derive(Component)]
pub(crate) struct ModelLibraryRoot;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollPanel;

#[derive(Component)]
pub(crate) struct ModelLibraryList;

#[derive(Component)]
pub(crate) struct ModelLibraryListItem;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollbarTrack;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollbarThumb;

#[derive(Component)]
pub(crate) struct ModelLibraryItemButton {
    pub(crate) model_id: u128,
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

pub(crate) fn setup_model_library_ui(mut commands: Commands) {
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
                field.spawn((
                    Text::new("Search…"),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.80, 0.80, 0.86, 0.75)),
                    ModelLibrarySearchFieldText,
                ));
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
    mut fields: Query<&Interaction, (Changed<Interaction>, With<ModelLibrarySearchField>)>,
) {
    if !state.is_open() {
        return;
    }

    for interaction in &mut fields {
        if *interaction == Interaction::Pressed {
            state.search_focused = true;
        }
    }
}

pub(crate) fn model_library_search_text_input(
    mut state: ResMut<ModelLibraryUiState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: MessageReader<KeyboardInput>,
) {
    if !state.is_open() {
        keyboard.clear();
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
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                state.search_focused = false;
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
    state: Res<ModelLibraryUiState>,
    mut fields: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<ModelLibrarySearchField>,
    >,
    mut texts: Query<(&mut Text, &mut TextColor), With<ModelLibrarySearchFieldText>>,
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
    let (text_value, text_color) = if query.is_empty() {
        ("Search…".to_string(), Color::srgba(0.80, 0.80, 0.86, 0.75))
    } else {
        (query.to_string(), Color::srgba(0.92, 0.92, 0.96, 1.0))
    };

    for (mut text, mut color) in &mut texts {
        if **text != text_value {
            **text = text_value.clone();
        }
        *color = TextColor(text_color);
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
    if model_ids.is_empty() {
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
    struct Row {
        prefab_id: u128,
        display_name: String,
        modified_at_ms: u128,
        score: u32,
        thumbnail: Option<Handle<Image>>,
    }

    let query = state.search_query.trim().to_string();
    let mut rows: Vec<Row> = Vec::new();

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

        rows.push(Row {
            prefab_id,
            display_name,
            modified_at_ms,
            score,
            thumbnail,
        });
    }

    rows.sort_by(|a, b| {
        if !query.is_empty() {
            b.score
                .cmp(&a.score)
                .then_with(|| b.modified_at_ms.cmp(&a.modified_at_ms))
                .then_with(|| a.display_name.cmp(&b.display_name))
                .then_with(|| a.prefab_id.cmp(&b.prefab_id))
        } else {
            b.modified_at_ms
                .cmp(&a.modified_at_ms)
                .then_with(|| a.display_name.cmp(&b.display_name))
                .then_with(|| a.prefab_id.cmp(&b.prefab_id))
        }
    });

    state.listed_prefabs = rows.iter().map(|row| row.prefab_id).collect();

    commands.entity(list_entity).with_children(|list| {
        for row in rows {
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
                ModelLibraryItemButton {
                    model_id: row.prefab_id,
                },
            ))
            .with_children(|b| {
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
                    if let Some(handle) = row.thumbnail.as_ref() {
                        thumb.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            ImageNode::new(handle.clone()).with_mode(NodeImageMode::Stretch),
                        ));
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
) -> Result<(Entity, Handle<Image>), String> {
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
            Vec3::new(10.0, 18.0, 8.0),
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
        ))
        .id();
    commands.entity(scene_root).add_child(camera_id);

    Ok((scene_root, target))
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

    let (scene_root, target) = match spawn_model_library_preview_scene(
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
    meta.push('\n');
    if let Some(long) = long {
        meta.push_str(long);
    } else if let Some(short) = short {
        meta.push_str(short);
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
                    target.clone(),
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
        scene_root,
        target,
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
    mut state: ResMut<ModelLibraryUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut buttons: Query<
        (
            &Interaction,
            &ModelLibraryItemButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Changed<Interaction>,
    >,
) {
    let cursor = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position());
    for (interaction, button, mut bg, mut border) in &mut buttons {
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
                if state.preview.is_some() {
                    state.pending_preview = Some(button.model_id);
                    continue;
                }
                if state.drag.is_none() {
                    if let Some(cursor) = cursor {
                        state.drag = Some(ModelLibraryDrag {
                            model_id: button.model_id,
                            start_cursor: cursor,
                            is_dragging: false,
                            preview_translation: None,
                        });
                    }
                }
            }
        }
    }
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
    if state.preview.is_some() {
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
