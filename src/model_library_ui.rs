use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::Ime;
use bevy::window::PrimaryWindow;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Mutex};

use crate::assets::SceneAssets;
use crate::config::AppConfig;
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
    ObjectId, ObjectPrefabId, UiFonts, UiToastCommand, UiToastKind,
};
use crate::ui::{set_ime_position_for_rich_text, ImeAnchorXPolicy};

const PANEL_Z_INDEX: i32 = 930;
const PANEL_WIDTH_PX: f32 = 260.0;
const PANEL_WIDTH_MANAGE_PX: f32 = 320.0;
const DRAG_START_THRESHOLD_PX: f32 = 6.0;
const PREFAB_PREVIEW_Z_INDEX: i32 = 1200;
const PREFAB_PREVIEW_MODAL_Z_INDEX: i32 = PREFAB_PREVIEW_Z_INDEX + 20;
const PREFAB_PREVIEW_LAYER: usize = 28;
const PREFAB_PREVIEW_WIDTH_PX: u32 = 640;
const PREFAB_PREVIEW_HEIGHT_PX: u32 = 360;

#[derive(SystemParam)]
pub(crate) struct ModelLibraryEnv<'w> {
    config: Res<'w, AppConfig>,
    build_scene: Res<'w, State<crate::types::BuildScene>>,
    active: Res<'w, crate::realm::ActiveRealmScene>,
}

#[derive(SystemParam)]
pub(crate) struct ModelLibraryGen3dSessionOpener<'w> {
    next_mode: ResMut<'w, NextState<GameMode>>,
    next_build_scene: ResMut<'w, NextState<crate::types::BuildScene>>,
    task_queue: ResMut<'w, crate::gen3d::Gen3dTaskQueue>,
    gen3d_workshop: ResMut<'w, crate::gen3d::Gen3dWorkshop>,
    gen3d_job: ResMut<'w, crate::gen3d::Gen3dAiJob>,
    gen3d_draft: ResMut<'w, crate::gen3d::Gen3dDraft>,
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
    multi_select_mode: bool,
    multi_selected_prefabs: HashSet<u128>,
    export_dialog_pending_ids: Vec<u128>,
    export_dialog_pending_realm: Option<String>,
    import_dialog_pending_realm: Option<String>,
    selected_prefab_id: Option<u128>,
    pending_preview: Option<u128>,
    preview: Option<ModelLibraryPrefabPreview>,
    delete_modal_prefab_id: Option<u128>,
    delete_modal_root: Option<Entity>,
    manage_delete_modal_root: Option<Entity>,
    manage_delete_modal_pending_realm: Option<String>,
    manage_delete_modal_pending_ids: Vec<u128>,
    last_rebuilt_scene: Option<(String, String)>,
}

#[derive(Resource)]
pub(crate) struct ModelLibraryExportJob {
    receiver: Mutex<Option<mpsc::Receiver<Result<usize, String>>>>,
}

impl Default for ModelLibraryExportJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct ModelLibraryImportJob {
    receiver:
        Mutex<Option<mpsc::Receiver<Result<crate::prefab_zip::PrefabZipImportReport, String>>>>,
}

impl Default for ModelLibraryImportJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct ModelLibraryExportDialogJob {
    receiver: Mutex<Option<mpsc::Receiver<Option<std::path::PathBuf>>>>,
}

impl Default for ModelLibraryExportDialogJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct ModelLibraryImportDialogJob {
    receiver: Mutex<Option<mpsc::Receiver<Option<std::path::PathBuf>>>>,
}

impl Default for ModelLibraryImportDialogJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

impl Default for ModelLibraryUiState {
    fn default() -> Self {
        Self {
            models_dirty: true,
            open: false,
            search_query: String::new(),
            search_focused: false,
            drag: None,
            spawn_seq: 0,
            scrollbar_drag: None,
            preview_scrollbar_drag: None,
            thumbnail_cache: HashMap::new(),
            listed_prefabs: Vec::new(),
            multi_select_mode: false,
            multi_selected_prefabs: HashSet::new(),
            export_dialog_pending_ids: Vec::new(),
            export_dialog_pending_realm: None,
            import_dialog_pending_realm: None,
            selected_prefab_id: None,
            pending_preview: None,
            preview: None,
            delete_modal_prefab_id: None,
            delete_modal_root: None,
            manage_delete_modal_root: None,
            manage_delete_modal_pending_realm: None,
            manage_delete_modal_pending_ids: Vec::new(),
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
            self.multi_select_mode = false;
            self.multi_selected_prefabs.clear();
            self.export_dialog_pending_ids.clear();
            self.export_dialog_pending_realm = None;
            self.import_dialog_pending_realm = None;
            self.manage_delete_modal_pending_realm = None;
            self.manage_delete_modal_pending_ids.clear();
        }
    }

    pub(crate) fn select_prefab(&mut self, prefab_id: u128) {
        self.selected_prefab_id = Some(prefab_id);
        self.search_focused = false;
    }

    pub(crate) fn request_preview(&mut self, prefab_id: u128) {
        self.select_prefab(prefab_id);
        self.pending_preview = Some(prefab_id);
    }

    pub(crate) fn selected_prefab_id(&self) -> Option<u128> {
        self.selected_prefab_id
    }

    pub(crate) fn is_drag_active(&self) -> bool {
        self.drag.is_some()
    }

    pub(crate) fn is_search_focused(&self) -> bool {
        self.search_focused
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
pub(crate) struct ModelLibraryGen3dPlaceholderList;

#[derive(Component)]
pub(crate) struct ModelLibraryPrefabList;

#[derive(Component)]
pub(crate) struct ModelLibraryListItem;

#[derive(Component, Debug, Clone)]
pub(crate) struct ModelLibraryPrefabLabelText {
    prefab_id: u128,
}

#[derive(Component)]
pub(crate) struct ModelLibrarySelectionMark {
    pub(crate) model_id: u128,
}

#[derive(Component)]
pub(crate) struct ModelLibraryScrollbarTrack;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollbarThumb;

#[derive(Component)]
pub(crate) struct ModelLibraryItemButton {
    pub(crate) model_id: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelLibraryGen3dIndicatorKind {
    Working,
    Waiting,
}

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dThumbnailIndicator {
    pub(crate) prefab_id: u128,
    kind: ModelLibraryGen3dIndicatorKind,
}

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dPlaceholderItem {
    pub(crate) session_id: crate::gen3d::Gen3dSessionId,
}

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dPlaceholderIndicator {
    pub(crate) session_id: crate::gen3d::Gen3dSessionId,
    kind: ModelLibraryGen3dIndicatorKind,
}

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dButton;

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dButtonText;

#[derive(Component)]
pub(crate) struct ModelLibraryImportButton;

#[derive(Component)]
pub(crate) struct ModelLibraryImportButtonText;

#[derive(Component)]
pub(crate) struct ModelLibraryExportButton;

#[derive(Component)]
pub(crate) struct ModelLibraryExportButtonText;

#[derive(Component)]
pub(crate) struct ModelLibraryManageToggleButton;

#[derive(Component)]
pub(crate) struct ModelLibraryManageToggleButtonText;

#[derive(Component)]
pub(crate) struct ModelLibraryNormalActionsRow;

#[derive(Component)]
pub(crate) struct ModelLibraryManageActionsRow;

#[derive(Component)]
pub(crate) struct ModelLibraryManageDeleteButton;

#[derive(Component)]
pub(crate) struct ModelLibraryManageDeleteButtonText;

#[derive(Component)]
pub(crate) struct ModelLibraryManageSelectAllButton;

#[derive(Component)]
pub(crate) struct ModelLibraryManageSelectNoneButton;

#[derive(Component)]
pub(crate) struct ModelLibraryMultiSelectIndicator {
    pub(crate) model_id: u128,
}

#[derive(Component)]
pub(crate) struct ModelLibraryMultiSelectIndicatorDot {
    pub(crate) model_id: u128,
}

#[derive(Component)]
pub(crate) struct ModelLibrarySearchField;

#[derive(Component)]
pub(crate) struct ModelLibrarySearchFieldText;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewOverlayRoot;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewDeleteButton;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewCloseButton;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewModifyButton;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewDuplicateButton;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewDeleteModalRoot;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewDeleteConfirmButton;

#[derive(Component)]
pub(crate) struct ModelLibraryPreviewDeleteCancelButton;

#[derive(Component)]
pub(crate) struct ModelLibraryManageDeleteModalRoot;

#[derive(Component)]
pub(crate) struct ModelLibraryManageDeleteConfirmButton;

#[derive(Component)]
pub(crate) struct ModelLibraryManageDeleteCancelButton;

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
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::all(Val::Px(999.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.35)),
                    BorderColor::all(Color::srgba(0.25, 0.95, 0.85, 0.90)),
                    ModelLibraryManageToggleButton,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Manage"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgba(0.25, 0.95, 0.85, 0.95)),
                        ModelLibraryManageToggleButtonText,
                    ));
                });
            });

            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                ModelLibraryNormalActionsRow,
            ))
            .with_children(|buttons| {
                buttons
                    .spawn((
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                        BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                        ModelLibraryImportButton,
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("Import"),
                            TextFont {
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            ModelLibraryImportButtonText,
                        ));
                    });

                buttons
                    .spawn((
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
                            Text::new("Generate"),
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
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::FlexStart,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                ModelLibraryManageActionsRow,
            ))
            .with_children(|row| {
                row.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|buttons| {
                    buttons
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            ModelLibraryExportButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Export"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                ModelLibraryExportButtonText,
                            ));
                        });

                    buttons
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            ModelLibraryManageDeleteButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Delete"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                ModelLibraryManageDeleteButtonText,
                            ));
                        });
                });

                row.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|buttons| {
                    buttons
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            ModelLibraryManageSelectAllButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("All"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            ));
                        });

                    buttons
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            ModelLibraryManageSelectNoneButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("None"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            ));
                        });
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
                    scroll
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            ModelLibraryList,
                        ))
                        .with_children(|list| {
                            list.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(6.0),
                                    ..default()
                                },
                                BackgroundColor(Color::NONE),
                                ModelLibraryGen3dPlaceholderList,
                            ));
                            list.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(6.0),
                                    ..default()
                                },
                                BackgroundColor(Color::NONE),
                                ModelLibraryPrefabList,
                            ));
                        });
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
        state.scrollbar_drag = None;
        state.preview_scrollbar_drag = None;
        state.multi_select_mode = false;
        state.multi_selected_prefabs.clear();
        close_model_library_manage_delete_modal(&mut commands, &mut state);
        close_model_library_preview(&mut commands, &mut state);
    }
}

pub(crate) fn model_library_update_manage_mode_ui(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    state: Res<ModelLibraryUiState>,
    mut manage_text: Query<(&mut Text, &mut TextColor), With<ModelLibraryManageToggleButtonText>>,
    mut rows: Query<(
        &mut Node,
        Option<&ModelLibraryNormalActionsRow>,
        Option<&ModelLibraryManageActionsRow>,
    )>,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);

    let manage_mode = visible && state.multi_select_mode;
    let normal_mode = visible && !state.multi_select_mode;

    for (mut node, is_normal, is_manage) in &mut rows {
        if is_normal.is_some() {
            node.display = if normal_mode {
                Display::Flex
            } else {
                Display::None
            };
        }
        if is_manage.is_some() {
            node.display = if manage_mode {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    for (mut text, mut color) in &mut manage_text {
        let next = if state.multi_select_mode {
            "Done"
        } else {
            "Manage"
        };
        if text.0 != next {
            text.0 = next.to_string();
        }

        let next_color = if state.multi_select_mode {
            Color::srgba(0.02, 0.02, 0.03, 0.95)
        } else {
            Color::srgba(0.25, 0.95, 0.85, 0.95)
        };
        if color.0 != next_color {
            color.0 = next_color;
        }
    }
}

pub(crate) fn model_library_update_panel_width(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    state: Res<ModelLibraryUiState>,
    mut roots: Query<&mut Node, With<ModelLibraryRoot>>,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    let width = if visible && state.multi_select_mode {
        PANEL_WIDTH_MANAGE_PX
    } else {
        PANEL_WIDTH_PX
    };

    for mut node in &mut roots {
        node.width = Val::Px(width);
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

pub(crate) fn model_library_search_ime_position(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    state: Res<ModelLibraryUiState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    fields: Query<(&ComputedNode, &UiGlobalTransform), With<ModelLibrarySearchField>>,
    text_root: Query<Entity, With<ModelLibrarySearchFieldText>>,
    children: Query<&Children>,
    nodes: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&Text>,
        Option<&TextSpan>,
        Option<&ImageNode>,
        Option<&Visibility>,
    )>,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    if !visible || !state.search_focused {
        return;
    }
    let Ok((node, transform)) = fields.single() else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let rich_root = text_root.iter().next();
    let anchor_x = if state.search_query.trim().is_empty() {
        ImeAnchorXPolicy::ContentLeft
    } else {
        ImeAnchorXPolicy::LineEnd
    };
    set_ime_position_for_rich_text(
        &mut window,
        node,
        *transform,
        rich_root,
        anchor_x,
        &children,
        &nodes,
    );
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelLibraryEditState {
    Editing,
    Queued,
}

fn model_library_collect_edit_states(
    task_queue: &crate::gen3d::Gen3dTaskQueue,
) -> HashMap<u128, ModelLibraryEditState> {
    let queued_sessions: HashSet<crate::gen3d::Gen3dSessionId> =
        task_queue.queue.iter().copied().collect();
    let mut edit_states: HashMap<u128, ModelLibraryEditState> = HashMap::new();
    for meta in task_queue.metas.values() {
        let prefab_id = match meta.kind {
            crate::gen3d::Gen3dSessionKind::EditOverwrite { prefab_id }
            | crate::gen3d::Gen3dSessionKind::Fork { prefab_id } => prefab_id,
            crate::gen3d::Gen3dSessionKind::NewBuild => continue,
        };

        let state = match meta.task_state {
            crate::gen3d::Gen3dTaskState::Running => Some(ModelLibraryEditState::Editing),
            crate::gen3d::Gen3dTaskState::Waiting => Some(ModelLibraryEditState::Queued),
            crate::gen3d::Gen3dTaskState::Done
            | crate::gen3d::Gen3dTaskState::Failed
            | crate::gen3d::Gen3dTaskState::Canceled
            | crate::gen3d::Gen3dTaskState::Idle => None,
        }
        .or_else(|| {
            queued_sessions
                .contains(&meta.id)
                .then_some(ModelLibraryEditState::Queued)
        });

        let Some(state) = state else {
            continue;
        };

        edit_states
            .entry(prefab_id)
            .and_modify(|prev| {
                if matches!(
                    (*prev, state),
                    (
                        ModelLibraryEditState::Queued,
                        ModelLibraryEditState::Editing
                    )
                ) {
                    *prev = ModelLibraryEditState::Editing;
                }
            })
            .or_insert(state);
    }

    edit_states
}

fn model_library_label_prefix(state: Option<ModelLibraryEditState>) -> (&'static str, Color) {
    match state {
        Some(ModelLibraryEditState::Editing) => {
            ("Editing…: ", Color::srgba(0.30, 0.97, 0.45, 0.95))
        }
        Some(ModelLibraryEditState::Queued) => ("Queued…: ", Color::srgba(0.95, 0.85, 0.25, 0.95)),
        None => ("", Color::srgb(0.92, 0.92, 0.96)),
    }
}

pub(crate) fn model_library_rebuild_list_ui(
    mut commands: Commands,
    active: Res<crate::realm::ActiveRealmScene>,
    mut images: ResMut<Assets<Image>>,
    mut descriptors: ResMut<PrefabDescriptorLibrary>,
    task_queue: Res<crate::gen3d::Gen3dTaskQueue>,
    mut state: ResMut<ModelLibraryUiState>,
    lists: Query<Entity, With<ModelLibraryPrefabList>>,
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
                Text::new("No realm prefabs yet.\nUse Generate to create one."),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.80, 0.86)),
                ModelLibraryListItem,
            ));
        });
        state.listed_prefabs.clear();
        state.multi_selected_prefabs.clear();
        state.models_dirty = false;
        return;
    }

    let edit_states = model_library_collect_edit_states(&task_queue);

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
    if state.multi_select_mode && !state.multi_selected_prefabs.is_empty() {
        let listed: HashSet<u128> = state.listed_prefabs.iter().copied().collect();
        state
            .multi_selected_prefabs
            .retain(|prefab_id| listed.contains(prefab_id));
    }

    commands.entity(list_entity).with_children(|list| {
        for row in rows {
            let (prefix_text, prefix_color) =
                model_library_label_prefix(edit_states.get(&row.prefab_id).copied());

            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
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
                        model_id: row.prefab_id,
                    },
                ));

                b.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(10.0),
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        min_width: Val::Px(0.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|left| {
                    left.spawn((
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

                        thumb
                            .spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(3.0),
                                    top: Val::Px(3.0),
                                    width: Val::Px(14.0),
                                    height: Val::Px(14.0),
                                    border: UiRect::all(Val::Px(2.0)),
                                    border_radius: BorderRadius::all(Val::Px(999.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                                BorderColor::all(Color::srgba(0.30, 0.97, 0.45, 0.95)),
                                UiTransform::default(),
                                Visibility::Hidden,
                                ModelLibraryGen3dThumbnailIndicator {
                                    prefab_id: row.prefab_id,
                                    kind: ModelLibraryGen3dIndicatorKind::Working,
                                },
                            ))
                            .with_children(|spinner| {
                                spinner.spawn((
                                    Text::new("↻"),
                                    TextFont {
                                        font_size: 12.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgba(0.30, 0.97, 0.45, 0.95)),
                                ));
                            });

                        thumb
                            .spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(3.0),
                                    top: Val::Px(3.0),
                                    width: Val::Px(14.0),
                                    height: Val::Px(14.0),
                                    border: UiRect::all(Val::Px(2.0)),
                                    border_radius: BorderRadius::all(Val::Px(999.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                                BorderColor::all(Color::srgba(0.95, 0.85, 0.25, 0.95)),
                                UiTransform::default(),
                                Visibility::Hidden,
                                ModelLibraryGen3dThumbnailIndicator {
                                    prefab_id: row.prefab_id,
                                    kind: ModelLibraryGen3dIndicatorKind::Waiting,
                                },
                            ))
                            .with_children(|spinner| {
                                spinner.spawn((
                                    Text::new("↻"),
                                    TextFont {
                                        font_size: 12.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgba(0.95, 0.85, 0.25, 0.95)),
                                ));
                            });
                    });

                    left.spawn((
                        Text::new(prefix_text),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(prefix_color),
                        ModelLibraryPrefabLabelText {
                            prefab_id: row.prefab_id,
                        },
                    ))
                    .with_children(|label| {
                        label.spawn((
                            TextSpan::new(row.display_name),
                            TextFont {
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        ));
                    });
                });

                b.spawn((
                    Node {
                        width: Val::Px(18.0),
                        height: Val::Px(18.0),
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::all(Val::Px(999.0)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    ModelLibraryMultiSelectIndicator {
                        model_id: row.prefab_id,
                    },
                ))
                .with_children(|radio| {
                    radio.spawn((
                        Node {
                            width: Val::Px(8.0),
                            height: Val::Px(8.0),
                            border_radius: BorderRadius::all(Val::Px(999.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.25, 0.95, 0.85, 0.90)),
                        Visibility::Hidden,
                        ModelLibraryMultiSelectIndicatorDot {
                            model_id: row.prefab_id,
                        },
                    ));
                });
            });
        }
    });

    state.models_dirty = false;
}

pub(crate) fn model_library_sync_gen3d_placeholders(
    mut commands: Commands,
    state: Res<ModelLibraryUiState>,
    task_queue: Res<crate::gen3d::Gen3dTaskQueue>,
    gen3d_workshop: Res<crate::gen3d::Gen3dWorkshop>,
    gen3d_job: Res<crate::gen3d::Gen3dAiJob>,
    lists: Query<Entity, With<ModelLibraryGen3dPlaceholderList>>,
    existing: Query<(Entity, &ModelLibraryGen3dPlaceholderItem)>,
    mut last_sig: Local<Vec<(crate::gen3d::Gen3dSessionId, crate::gen3d::Gen3dTaskState)>>,
) {
    let Ok(list_entity) = lists.single() else {
        return;
    };

    if !state.is_open() {
        if !last_sig.is_empty() {
            for (entity, _) in &existing {
                commands.entity(entity).try_despawn();
            }
            last_sig.clear();
        }
        return;
    }

    fn short_id(id: crate::gen3d::Gen3dSessionId) -> String {
        let s = id.to_string();
        s.chars().take(8).collect()
    }

    let mut placeholders: Vec<(
        crate::gen3d::Gen3dSessionId,
        crate::gen3d::Gen3dTaskState,
        u128,
    )> = Vec::new();
    for meta in task_queue.metas.values() {
        if !matches!(meta.kind, crate::gen3d::Gen3dSessionKind::NewBuild) {
            continue;
        }
        if !matches!(
            meta.task_state,
            crate::gen3d::Gen3dTaskState::Waiting | crate::gen3d::Gen3dTaskState::Running
        ) {
            continue;
        }

        let saved_prefab_id = if meta.id == task_queue.active_session_id {
            gen3d_job.last_saved_prefab_id()
        } else {
            task_queue
                .inactive_states
                .get(&meta.id)
                .and_then(|state| state.job.last_saved_prefab_id())
        };
        if saved_prefab_id.is_some() {
            continue;
        }

        placeholders.push((meta.id, meta.task_state, meta.created_at_ms));
    }

    placeholders.sort_by(|a, b| b.2.cmp(&a.2));
    let sig: Vec<(crate::gen3d::Gen3dSessionId, crate::gen3d::Gen3dTaskState)> = placeholders
        .iter()
        .map(|(id, state, _)| (*id, *state))
        .collect();
    if *last_sig == sig {
        return;
    }
    *last_sig = sig;

    for (entity, _) in &existing {
        commands.entity(entity).try_despawn();
    }

    commands.entity(list_entity).with_children(|list| {
        for (session_id, task_state, _created_at_ms) in placeholders {
            let prompt = if session_id == task_queue.active_session_id {
                gen3d_workshop.prompt.trim()
            } else {
                task_queue
                    .inactive_states
                    .get(&session_id)
                    .map(|state| state.workshop.prompt.trim())
                    .unwrap_or("")
            };
            let (prefix_text, prefix_color) = match task_state {
                crate::gen3d::Gen3dTaskState::Running => {
                    ("Generating…: ", Color::srgba(0.30, 0.97, 0.45, 0.95))
                }
                crate::gen3d::Gen3dTaskState::Waiting => {
                    ("Queued…: ", Color::srgba(0.95, 0.85, 0.25, 0.95))
                }
                _ => ("Generating…: ", Color::srgba(0.30, 0.97, 0.45, 0.95)),
            };
            let short = short_id(session_id);
            let rest = if prompt.is_empty() {
                format!("(new prefab) [{short}]")
            } else {
                let snippet: String = prompt.chars().take(42).collect();
                if prompt.chars().count() > 42 {
                    format!("{snippet}… [{short}]")
                } else {
                    format!("{snippet} [{short}]")
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
                ModelLibraryGen3dPlaceholderItem { session_id },
            ))
            .with_children(|b| {
                b.spawn((
                    Node {
                        width: Val::Px(42.0),
                        height: Val::Px(42.0),
                        border: UiRect::all(Val::Px(1.0)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.75)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ))
                .with_children(|thumb| {
                    thumb.spawn((
                        Text::new("…"),
                        TextFont {
                            font_size: 20.0,
                            ..default()
                        },
                        TextColor(Color::srgba(0.92, 0.92, 0.96, 0.85)),
                    ));

                    thumb
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                right: Val::Px(3.0),
                                top: Val::Px(3.0),
                                width: Val::Px(14.0),
                                height: Val::Px(14.0),
                                border: UiRect::all(Val::Px(2.0)),
                                border_radius: BorderRadius::all(Val::Px(999.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                            BorderColor::all(Color::srgba(0.30, 0.97, 0.45, 0.95)),
                            UiTransform::default(),
                            Visibility::Hidden,
                            ModelLibraryGen3dPlaceholderIndicator {
                                session_id,
                                kind: ModelLibraryGen3dIndicatorKind::Working,
                            },
                        ))
                        .with_children(|spinner| {
                            spinner.spawn((
                                Text::new("↻"),
                                TextFont {
                                    font_size: 12.0,
                                    ..default()
                                },
                                TextColor(Color::srgba(0.30, 0.97, 0.45, 0.95)),
                            ));
                        });

                    thumb
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                right: Val::Px(3.0),
                                top: Val::Px(3.0),
                                width: Val::Px(14.0),
                                height: Val::Px(14.0),
                                border: UiRect::all(Val::Px(2.0)),
                                border_radius: BorderRadius::all(Val::Px(999.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                            BorderColor::all(Color::srgba(0.95, 0.85, 0.25, 0.95)),
                            UiTransform::default(),
                            Visibility::Hidden,
                            ModelLibraryGen3dPlaceholderIndicator {
                                session_id,
                                kind: ModelLibraryGen3dIndicatorKind::Waiting,
                            },
                        ))
                        .with_children(|spinner| {
                            spinner.spawn((
                                Text::new("↻"),
                                TextFont {
                                    font_size: 12.0,
                                    ..default()
                                },
                                TextColor(Color::srgba(0.95, 0.85, 0.25, 0.95)),
                            ));
                        });
                });

                b.spawn((
                    Text::new(prefix_text),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(prefix_color),
                ))
                .with_children(|label| {
                    label.spawn((
                        TextSpan::new(rest),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    ));
                });
            });
        }
    });
}

pub(crate) fn model_library_gen3d_placeholder_item_interactions(
    mode: Res<State<GameMode>>,
    state: Res<ModelLibraryUiState>,
    mut gen3d: ModelLibraryGen3dSessionOpener,
    mut buttons: Query<
        (
            &Interaction,
            &ModelLibraryGen3dPlaceholderItem,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Changed<Interaction>,
    >,
) {
    for (interaction, item, mut bg, mut border) in &mut buttons {
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

                if state.multi_select_mode {
                    continue;
                }
                if !matches!(mode.get(), GameMode::Build) {
                    continue;
                }

                if let Err(err) = gen3d.task_queue.swap_active_session(
                    item.session_id,
                    &mut gen3d.gen3d_workshop,
                    &mut gen3d.gen3d_job,
                    &mut gen3d.gen3d_draft,
                ) {
                    gen3d.gen3d_workshop.error = Some(err);
                }
                gen3d.next_mode.set(GameMode::Build);
                gen3d
                    .next_build_scene
                    .set(crate::types::BuildScene::Preview);
            }
        }
    }
}

pub(crate) fn model_library_update_gen3d_thumbnail_indicators(
    time: Res<Time>,
    task_queue: Res<crate::gen3d::Gen3dTaskQueue>,
    mut indicators: Query<(
        &ModelLibraryGen3dThumbnailIndicator,
        &mut Visibility,
        &mut UiTransform,
    )>,
) {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PrefabState {
        Waiting,
        Working,
    }

    let mut prefab_states: HashMap<u128, PrefabState> = HashMap::new();
    let queued_sessions: HashSet<crate::gen3d::Gen3dSessionId> =
        task_queue.queue.iter().copied().collect();
    for meta in task_queue.metas.values() {
        let prefab_id = match meta.kind {
            crate::gen3d::Gen3dSessionKind::EditOverwrite { prefab_id }
            | crate::gen3d::Gen3dSessionKind::Fork { prefab_id } => prefab_id,
            crate::gen3d::Gen3dSessionKind::NewBuild => continue,
        };

        let mut state = match meta.task_state {
            crate::gen3d::Gen3dTaskState::Running => Some(PrefabState::Working),
            crate::gen3d::Gen3dTaskState::Waiting => Some(PrefabState::Waiting),
            crate::gen3d::Gen3dTaskState::Done
            | crate::gen3d::Gen3dTaskState::Failed
            | crate::gen3d::Gen3dTaskState::Canceled
            | crate::gen3d::Gen3dTaskState::Idle => None,
        };
        if state.is_none() && queued_sessions.contains(&meta.id) {
            state = Some(PrefabState::Waiting);
        }

        let Some(state) = state else {
            continue;
        };

        prefab_states
            .entry(prefab_id)
            .and_modify(|prev| {
                if matches!((*prev, state), (PrefabState::Waiting, PrefabState::Working)) {
                    *prev = PrefabState::Working;
                }
            })
            .or_insert(state);
    }

    let t = time.elapsed_secs();
    for (indicator, mut vis, mut ui_transform) in &mut indicators {
        let state = prefab_states.get(&indicator.prefab_id).copied();
        let show = match (state, indicator.kind) {
            (Some(PrefabState::Working), ModelLibraryGen3dIndicatorKind::Working) => true,
            (Some(PrefabState::Waiting), ModelLibraryGen3dIndicatorKind::Waiting) => true,
            _ => false,
        };
        *vis = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !show {
            continue;
        }

        let offset = ((indicator.prefab_id % 97) as f32) * 0.23;
        ui_transform.rotation = match indicator.kind {
            ModelLibraryGen3dIndicatorKind::Working => Rot2::radians(t * 7.0 + offset),
            ModelLibraryGen3dIndicatorKind::Waiting => Rot2::radians(0.0),
        };
    }
}

pub(crate) fn model_library_update_gen3d_placeholder_indicators(
    time: Res<Time>,
    task_queue: Res<crate::gen3d::Gen3dTaskQueue>,
    mut indicators: Query<(
        &ModelLibraryGen3dPlaceholderIndicator,
        &mut Visibility,
        &mut UiTransform,
    )>,
) {
    let t = time.elapsed_secs();
    for (indicator, mut vis, mut ui_transform) in &mut indicators {
        let state = task_queue
            .metas
            .get(&indicator.session_id)
            .map(|meta| meta.task_state);
        let show = match (state, indicator.kind) {
            (
                Some(crate::gen3d::Gen3dTaskState::Running),
                ModelLibraryGen3dIndicatorKind::Working,
            ) => true,
            (
                Some(crate::gen3d::Gen3dTaskState::Waiting),
                ModelLibraryGen3dIndicatorKind::Waiting,
            ) => true,
            _ => false,
        };
        *vis = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !show {
            continue;
        }

        let offset = ((indicator.session_id.as_u128() % 97) as f32) * 0.23;
        ui_transform.rotation = match indicator.kind {
            ModelLibraryGen3dIndicatorKind::Working => Rot2::radians(t * 7.0 + offset),
            ModelLibraryGen3dIndicatorKind::Waiting => Rot2::radians(0.0),
        };
    }
}

pub(crate) fn model_library_update_prefab_label_text(
    task_queue: Res<crate::gen3d::Gen3dTaskQueue>,
    mut labels: Query<(&ModelLibraryPrefabLabelText, &mut Text, &mut TextColor)>,
) {
    if labels.is_empty() {
        return;
    }

    let edit_states = model_library_collect_edit_states(&task_queue);
    for (label, mut text, mut color) in &mut labels {
        let state = edit_states.get(&label.prefab_id).copied();
        let (prefix, prefix_color) = model_library_label_prefix(state);
        if text.0 != prefix {
            text.0 = prefix.to_string();
        }
        if color.0 != prefix_color {
            color.0 = prefix_color;
        }
    }
}

fn close_model_library_delete_modal(commands: &mut Commands, state: &mut ModelLibraryUiState) {
    if let Some(root) = state.delete_modal_root.take() {
        commands.entity(root).try_despawn();
    }
    state.delete_modal_prefab_id = None;
}

fn open_model_library_delete_modal(
    commands: &mut Commands,
    state: &mut ModelLibraryUiState,
    prefab_id: u128,
) {
    if state.delete_modal_prefab_id.is_some() {
        return;
    }

    let root = commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ZIndex(PREFAB_PREVIEW_MODAL_Z_INDEX),
            ModelLibraryPreviewDeleteModalRoot,
            ModelLibraryPreviewOverlayRoot,
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: Val::Px(420.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(10.0),
                        padding: UiRect::all(Val::Px(14.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.96)),
                    BorderColor::all(Color::srgba(0.35, 0.35, 0.40, 0.85)),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("Delete prefab?"),
                        TextFont {
                            font_size: 16.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.95, 0.95, 0.97)),
                    ));

                    panel.spawn((
                        Text::new(
                            "This deletes prefab files from disk. Existing scene instances remain.",
                        ),
                        TextFont {
                            font_size: 13.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.86, 0.86, 0.90)),
                    ));

                    panel
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_direction: FlexDirection::Row,
                                justify_content: JustifyContent::FlexEnd,
                                column_gap: Val::Px(8.0),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                        ))
                        .with_children(|row| {
                            row.spawn((
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.25, 0.05, 0.05, 0.92)),
                                BorderColor::all(Color::srgba(0.80, 0.25, 0.25, 0.90)),
                                ModelLibraryPreviewDeleteConfirmButton,
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new("Confirm Delete"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.98, 0.90, 0.90)),
                                ));
                            });

                            row.spawn((
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.85)),
                                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
                                ModelLibraryPreviewDeleteCancelButton,
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new("Cancel"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                ));
                            });
                        });
                });
        })
        .id();

    state.delete_modal_prefab_id = Some(prefab_id);
    state.delete_modal_root = Some(root);
}

fn close_model_library_manage_delete_modal(
    commands: &mut Commands,
    state: &mut ModelLibraryUiState,
) {
    if let Some(root) = state.manage_delete_modal_root.take() {
        commands.entity(root).try_despawn();
    }
    state.manage_delete_modal_pending_realm = None;
    state.manage_delete_modal_pending_ids.clear();
}

fn open_model_library_manage_delete_modal(
    commands: &mut Commands,
    state: &mut ModelLibraryUiState,
    realm_id: String,
    prefab_ids: Vec<u128>,
) {
    if state.manage_delete_modal_root.is_some() {
        return;
    }

    let count = prefab_ids.len();
    let title = if count == 1 {
        "Delete selected prefab?".to_string()
    } else {
        format!("Delete {count} selected prefabs?")
    };

    let root = commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ZIndex(PREFAB_PREVIEW_MODAL_Z_INDEX),
            ModelLibraryManageDeleteModalRoot,
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: Val::Px(460.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(10.0),
                        padding: UiRect::all(Val::Px(14.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.96)),
                    BorderColor::all(Color::srgba(0.35, 0.35, 0.40, 0.85)),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new(title),
                        TextFont {
                            font_size: 16.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.95, 0.95, 0.97)),
                    ));

                    panel.spawn((
                        Text::new(
                            "This deletes prefab files from disk. Existing scene instances remain.",
                        ),
                        TextFont {
                            font_size: 13.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.86, 0.86, 0.90)),
                    ));

                    panel
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_direction: FlexDirection::Row,
                                justify_content: JustifyContent::FlexEnd,
                                column_gap: Val::Px(8.0),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                        ))
                        .with_children(|row| {
                            row.spawn((
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.25, 0.05, 0.05, 0.92)),
                                BorderColor::all(Color::srgba(0.80, 0.25, 0.25, 0.90)),
                                ModelLibraryManageDeleteConfirmButton,
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new("Confirm Delete"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.98, 0.90, 0.90)),
                                ));
                            });

                            row.spawn((
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.85)),
                                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
                                ModelLibraryManageDeleteCancelButton,
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new("Cancel"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                ));
                            });
                        });
                });
        })
        .id();

    state.manage_delete_modal_root = Some(root);
    state.manage_delete_modal_pending_realm = Some(realm_id);
    state.manage_delete_modal_pending_ids = prefab_ids;
}

fn close_model_library_preview(commands: &mut Commands, state: &mut ModelLibraryUiState) {
    close_model_library_delete_modal(commands, state);
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
    if !matches!(env.build_scene.get(), crate::types::BuildScene::Realm) {
        state.pending_preview = None;
        return;
    }
    if !state.is_open() {
        return;
    }
    if state.multi_select_mode {
        state.pending_preview = None;
        return;
    }

    let Some(prefab_id) = state.pending_preview.take() else {
        return;
    };
    state.select_prefab(prefab_id);

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
    fn sum_revision_tokens_for_key(
        revisions: &[crate::prefab_descriptors::PrefabDescriptorRevisionV1],
        key: &str,
    ) -> Option<u64> {
        let mut any = false;
        let mut total: u64 = 0;
        for rev in revisions {
            if let Some(tokens) = rev.extra.get(key).and_then(|v| v.as_u64()) {
                any = true;
                total = total.saturating_add(tokens);
            }
        }
        any.then_some(total)
    }

    fn find_generated_duration_ms(
        revisions: &[crate::prefab_descriptors::PrefabDescriptorRevisionV1],
    ) -> Option<u128> {
        revisions
            .iter()
            .find(|rev| rev.summary.trim() == "generated")
            .and_then(|rev| rev.extra.get("duration_ms"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u128)
    }

    let created_duration_ms = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.created_duration_ms)
        .or_else(|| find_generated_duration_ms(revisions));

    let total_input_tokens = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.total_input_tokens)
        .or_else(|| sum_revision_tokens_for_key(revisions, "tokens_input"));
    let total_output_tokens = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.total_output_tokens)
        .or_else(|| sum_revision_tokens_for_key(revisions, "tokens_output"));
    let total_unsplit_tokens = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.total_unsplit_tokens)
        .or_else(|| sum_revision_tokens_for_key(revisions, "tokens_unsplit"));
    let total_tokens = match (
        total_input_tokens,
        total_output_tokens,
        total_unsplit_tokens,
    ) {
        (Some(i), Some(o), Some(u)) => Some(i.saturating_add(o).saturating_add(u)),
        (Some(i), Some(o), None) => Some(i.saturating_add(o)),
        (Some(i), None, Some(u)) => Some(i.saturating_add(u)),
        (None, Some(o), Some(u)) => Some(o.saturating_add(u)),
        (Some(i), None, None) => Some(i),
        (None, Some(o), None) => Some(o),
        (None, None, Some(u)) => Some(u),
        (None, None, None) => None,
    }
    .or_else(|| {
        desc.and_then(|d| d.provenance.as_ref())
            .and_then(|p| p.extra.get("total_tokens"))
            .and_then(|v| v.as_u64())
    })
    .or_else(|| sum_revision_tokens_for_key(revisions, "tokens_total"));

    fn format_duration_ms(ms: u128) -> String {
        let ms = ms.min(u128::from(u64::MAX)) as u64;
        let d = std::time::Duration::from_millis(ms);
        let secs = d.as_secs();
        if secs < 60 {
            format!("{:.1}s", d.as_secs_f32())
        } else if secs < 60 * 60 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            let hours = secs / 3600;
            let mins = (secs % 3600) / 60;
            format!("{hours}h {mins}m")
        }
    }

    let mut meta = String::new();
    meta.push_str(&format!("Name: {name}\n"));
    meta.push_str(&format!("ID: {uuid}\n"));
    if let Some(modified_at_ms) = modified_at_ms {
        meta.push_str(&format!("Last modified: {modified_at_ms}\n"));
    }
    if let Some(created_at_ms) = created_at_ms {
        meta.push_str(&format!("Created: {created_at_ms}\n"));
    }
    if let Some(created_duration_ms) = created_duration_ms {
        meta.push_str(&format!(
            "Create duration: {} ({created_duration_ms}ms)\n",
            format_duration_ms(created_duration_ms)
        ));
    }
    if let (Some(input), Some(output)) = (total_input_tokens, total_output_tokens) {
        meta.push_str(&format!(
            "Tokens (total): in {input} | out {output} | sum {}\n",
            input.saturating_add(output)
        ));
    } else if let Some(total_tokens) = total_tokens {
        meta.push_str(&format!("Tokens (total): {total_tokens}\n"));
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
                height: Val::Px(680.0),
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
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|buttons| {
                    buttons
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            ModelLibraryPreviewDeleteButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Delete"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.94, 0.94, 0.96)),
                            ));
                        });

                    buttons
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            ModelLibraryPreviewModifyButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Edit"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            ));
                        });

                    buttons
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            ModelLibraryPreviewDuplicateButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Duplicate"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            ));
                        });

                    buttons
                        .spawn((
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

pub(crate) fn model_library_preview_delete_button_interactions(
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ModelLibraryPreviewDeleteButton>)>,
) {
    if state.delete_modal_prefab_id.is_some() {
        return;
    }
    let Some(prefab_id) = state.preview.as_ref().map(|p| p.prefab_id) else {
        return;
    };
    for interaction in &mut buttons {
        if *interaction == Interaction::Pressed {
            open_model_library_delete_modal(&mut commands, &mut state, prefab_id);
            break;
        }
    }
}

pub(crate) fn model_library_preview_delete_modal_interactions(
    mut commands: Commands,
    env: ModelLibraryEnv,
    mut state: ResMut<ModelLibraryUiState>,
    mut confirm: Query<
        &Interaction,
        (
            Changed<Interaction>,
            With<ModelLibraryPreviewDeleteConfirmButton>,
        ),
    >,
    mut cancel: Query<
        &Interaction,
        (
            Changed<Interaction>,
            With<ModelLibraryPreviewDeleteCancelButton>,
        ),
    >,
) {
    if state.delete_modal_prefab_id.is_none() {
        return;
    }

    for interaction in &mut cancel {
        if *interaction == Interaction::Pressed {
            close_model_library_delete_modal(&mut commands, &mut state);
            return;
        }
    }

    let Some(prefab_id) = state.delete_modal_prefab_id else {
        return;
    };
    for interaction in &mut confirm {
        if *interaction != Interaction::Pressed {
            continue;
        }

        if env.config.automation_enabled && env.config.automation_monitor_mode {
            close_model_library_delete_modal(&mut commands, &mut state);
            break;
        }

        match crate::realm_prefab_packages::delete_realm_prefab_package(
            &env.active.realm_id,
            prefab_id,
        ) {
            Ok(_) => {
                state.mark_models_dirty();
                state.selected_prefab_id = None;
                state.pending_preview = None;
                close_model_library_preview(&mut commands, &mut state);
            }
            Err(err) => {
                warn!("{err}");
                close_model_library_delete_modal(&mut commands, &mut state);
            }
        }
        break;
    }
}

pub(crate) fn model_library_preview_delete_modal_close_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
) {
    if state.delete_modal_prefab_id.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        close_model_library_delete_modal(&mut commands, &mut state);
    }
}

pub(crate) fn model_library_preview_close_button_interactions(
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ModelLibraryPreviewCloseButton>)>,
) {
    if state.delete_modal_prefab_id.is_some() {
        return;
    }
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

pub(crate) fn model_library_preview_modify_button_interactions(
    mut commands: Commands,
    env: ModelLibraryEnv,
    log_sinks: Option<Res<crate::app::Gen3dLogSinks>>,
    mut task_queue: ResMut<crate::gen3d::Gen3dTaskQueue>,
    mut gen3d_workshop: ResMut<crate::gen3d::Gen3dWorkshop>,
    mut gen3d_job: ResMut<crate::gen3d::Gen3dAiJob>,
    mut gen3d_draft: ResMut<crate::gen3d::Gen3dDraft>,
    mut next_mode: ResMut<NextState<GameMode>>,
    mut next_build_scene: ResMut<NextState<crate::types::BuildScene>>,
    mut state: ResMut<ModelLibraryUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ModelLibraryPreviewModifyButton>)>,
) {
    if state.delete_modal_prefab_id.is_some() {
        return;
    }
    let Some(prefab_id) = state.preview.as_ref().map(|p| p.prefab_id) else {
        return;
    };
    for interaction in &mut buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let existing_session_id = task_queue
            .metas
            .values()
            .find(|meta| match meta.kind {
                crate::gen3d::Gen3dSessionKind::EditOverwrite { prefab_id: id } => id == prefab_id,
                _ => false,
            })
            .map(|meta| meta.id)
            .or_else(|| task_queue.find_session_for_prefab(prefab_id))
            .unwrap_or_else(|| {
                task_queue.create_session(
                    crate::gen3d::Gen3dSessionKind::EditOverwrite { prefab_id },
                    crate::gen3d::Gen3dSessionState::default(),
                )
            });

        if let Err(err) = task_queue.swap_active_session(
            existing_session_id,
            &mut gen3d_workshop,
            &mut gen3d_job,
            &mut gen3d_draft,
        ) {
            gen3d_workshop.error = Some(err);
        } else if gen3d_job.edit_base_prefab_id() != Some(prefab_id) {
            let sinks = log_sinks.as_deref().cloned();
            if let Err(err) = crate::gen3d::gen3d_start_edit_session_from_prefab_id_from_api(
                env.build_scene.as_ref(),
                &env.config,
                sinks,
                &mut gen3d_workshop,
                &mut gen3d_job,
                &mut gen3d_draft,
                &env.active.realm_id,
                &env.active.scene_id,
                prefab_id,
            ) {
                gen3d_workshop.error = Some(err);
            }
        }

        next_mode.set(GameMode::Build);
        next_build_scene.set(crate::types::BuildScene::Preview);
        close_model_library_preview(&mut commands, &mut state);
        break;
    }
}

pub(crate) fn model_library_preview_duplicate_button_interactions(
    mut commands: Commands,
    env: ModelLibraryEnv,
    mut library: ResMut<ObjectLibrary>,
    mut descriptors: ResMut<PrefabDescriptorLibrary>,
    mut state: ResMut<ModelLibraryUiState>,
    mut buttons: Query<
        &Interaction,
        (
            Changed<Interaction>,
            With<ModelLibraryPreviewDuplicateButton>,
        ),
    >,
) {
    if state.delete_modal_prefab_id.is_some() {
        return;
    }
    let Some(prefab_id) = state.preview.as_ref().map(|p| p.prefab_id) else {
        return;
    };

    for interaction in &mut buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let duplicated = match duplicate_realm_prefab_package(
            env.active.realm_id.as_str(),
            prefab_id,
            &mut library,
            &mut descriptors,
        ) {
            Ok(id) => id,
            Err(err) => {
                warn!("{err}");
                return;
            }
        };

        state.mark_models_dirty();
        state.request_preview(duplicated);
        close_model_library_preview(&mut commands, &mut state);
        break;
    }
}

pub(crate) fn model_library_preview_close_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
) {
    if state.delete_modal_prefab_id.is_some() {
        return;
    }
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
    if state.delete_modal_prefab_id.is_some() {
        return;
    }
    if state.multi_select_mode {
        return;
    }
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
        state.request_preview(next_prefab);
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
    if state.delete_modal_prefab_id.is_some() {
        if let Some(preview) = state.preview.as_mut() {
            preview.last_cursor = None;
        }
        for _ in mouse_wheel.read() {}
        return;
    }
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
    if state.delete_modal_prefab_id.is_some() {
        for _ in mouse_wheel.read() {}
        return;
    }
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
    if state.delete_modal_prefab_id.is_some() {
        state.preview_scrollbar_drag = None;
        return;
    }
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

pub(crate) fn model_library_manage_toggle_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ModelLibraryManageToggleButton>),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    if !visible {
        return;
    }

    for (interaction, mut bg, mut border) in &mut buttons {
        let done = state.multi_select_mode;
        match *interaction {
            Interaction::None => {
                if done {
                    *bg = BackgroundColor(Color::srgba(0.25, 0.95, 0.85, 0.78));
                    *border = BorderColor::all(Color::srgba(0.25, 0.95, 0.85, 0.95));
                } else {
                    *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.35));
                    *border = BorderColor::all(Color::srgba(0.25, 0.95, 0.85, 0.90));
                }
            }
            Interaction::Hovered => {
                if done {
                    *bg = BackgroundColor(Color::srgba(0.30, 0.97, 0.87, 0.86));
                    *border = BorderColor::all(Color::srgba(0.30, 0.97, 0.87, 0.98));
                } else {
                    *bg = BackgroundColor(Color::srgba(0.04, 0.08, 0.08, 0.55));
                    *border = BorderColor::all(Color::srgba(0.30, 0.97, 0.87, 0.95));
                }
            }
            Interaction::Pressed => {
                if done {
                    *bg = BackgroundColor(Color::srgba(0.20, 0.85, 0.78, 0.90));
                    *border = BorderColor::all(Color::srgba(0.30, 0.97, 0.87, 0.98));
                } else {
                    *bg = BackgroundColor(Color::srgba(0.06, 0.12, 0.12, 0.75));
                    *border = BorderColor::all(Color::srgba(0.30, 0.97, 0.87, 0.95));
                }

                state.search_focused = false;
                state.drag = None;
                state.pending_preview = None;

                close_model_library_manage_delete_modal(&mut commands, &mut state);

                let next_manage_mode = !state.multi_select_mode;
                state.multi_select_mode = next_manage_mode;
                if next_manage_mode {
                    state.multi_selected_prefabs.clear();
                    if let Some(selected) = state.selected_prefab_id {
                        state.multi_selected_prefabs.insert(selected);
                    }
                    close_model_library_preview(&mut commands, &mut state);
                } else {
                    state.multi_selected_prefabs.clear();
                }
            }
        }
    }
}

pub(crate) fn model_library_manage_select_all_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mut state: ResMut<ModelLibraryUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (
            Changed<Interaction>,
            With<ModelLibraryManageSelectAllButton>,
        ),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm)
        && state.multi_select_mode;
    if !visible {
        return;
    }

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

                state.multi_selected_prefabs = state.listed_prefabs.iter().copied().collect();
                if state.selected_prefab_id.is_none() {
                    state.selected_prefab_id = state.listed_prefabs.first().copied();
                }
            }
        }
    }
}

pub(crate) fn model_library_manage_select_none_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mut state: ResMut<ModelLibraryUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (
            Changed<Interaction>,
            With<ModelLibraryManageSelectNoneButton>,
        ),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm)
        && state.multi_select_mode;
    if !visible {
        return;
    }

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

                state.multi_selected_prefabs.clear();
            }
        }
    }
}

pub(crate) fn model_library_gen3d_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mut next_build_scene: ResMut<NextState<crate::types::BuildScene>>,
    mut task_queue: ResMut<crate::gen3d::Gen3dTaskQueue>,
    mut gen3d_workshop: ResMut<crate::gen3d::Gen3dWorkshop>,
    mut gen3d_job: ResMut<crate::gen3d::Gen3dAiJob>,
    mut gen3d_draft: ResMut<crate::gen3d::Gen3dDraft>,
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

                let session_id = task_queue.create_session(
                    crate::gen3d::Gen3dSessionKind::NewBuild,
                    crate::gen3d::Gen3dSessionState::default(),
                );
                if let Err(err) = task_queue.swap_active_session(
                    session_id,
                    &mut gen3d_workshop,
                    &mut gen3d_job,
                    &mut gen3d_draft,
                ) {
                    gen3d_workshop.error = Some(err);
                    continue;
                }

                if !gen3d_job.is_running() && gen3d_workshop.status.trim().is_empty() {
                    gen3d_workshop.status =
                        "Drop 0–3 images (optional) and/or type a prompt, then click Build."
                            .to_string();
                    gen3d_workshop.speed_mode = crate::gen3d::Gen3dSpeedMode::Level3;
                }

                if !matches!(build_scene.get(), crate::types::BuildScene::Preview) {
                    next_build_scene.set(crate::types::BuildScene::Preview);
                }
            }
        }
    }
}

pub(crate) fn model_library_import_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    env: ModelLibraryEnv,
    mut state: ResMut<ModelLibraryUiState>,
    import_job: Res<ModelLibraryImportJob>,
    import_dialog: Res<ModelLibraryImportDialogJob>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ModelLibraryImportButton>),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    if !visible {
        return;
    }

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

                if let Ok(guard) = import_job.receiver.lock() {
                    if guard.is_some() {
                        toasts.write(UiToastCommand::Show {
                            text: "Import already running.".to_string(),
                            kind: UiToastKind::Warn,
                            ttl_secs: 3.0,
                        });
                        continue;
                    }
                }
                if let Ok(guard) = import_dialog.receiver.lock() {
                    if guard.is_some() {
                        toasts.write(UiToastCommand::Show {
                            text: "Import dialog already open.".to_string(),
                            kind: UiToastKind::Warn,
                            ttl_secs: 3.0,
                        });
                        continue;
                    }
                }

                let (tx, rx) = mpsc::channel();
                if let Ok(mut guard) = import_dialog.receiver.lock() {
                    *guard = Some(rx);
                }
                state.import_dialog_pending_realm = Some(env.active.realm_id.clone());
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .add_filter("Prefab Zip", &["zip"])
                        .pick_file();
                    let _ = tx.send(path);
                });
            }
        }
    }
}

pub(crate) fn model_library_export_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    env: ModelLibraryEnv,
    mut state: ResMut<ModelLibraryUiState>,
    export_job: Res<ModelLibraryExportJob>,
    export_dialog: Res<ModelLibraryExportDialogJob>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ModelLibraryExportButton>),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm)
        && state.multi_select_mode;
    if !visible {
        return;
    }

    fn ensure_zip_extension(path: std::path::PathBuf) -> std::path::PathBuf {
        match path.extension().and_then(|v| v.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("zip") => path,
            _ => path.with_extension("zip"),
        }
    }

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

                if let Ok(guard) = export_job.receiver.lock() {
                    if guard.is_some() {
                        toasts.write(UiToastCommand::Show {
                            text: "Export already running.".to_string(),
                            kind: UiToastKind::Warn,
                            ttl_secs: 3.0,
                        });
                        continue;
                    }
                }
                if let Ok(guard) = export_dialog.receiver.lock() {
                    if guard.is_some() {
                        toasts.write(UiToastCommand::Show {
                            text: "Export dialog already open.".to_string(),
                            kind: UiToastKind::Warn,
                            ttl_secs: 3.0,
                        });
                        continue;
                    }
                }

                if state.multi_selected_prefabs.is_empty() {
                    toasts.write(UiToastCommand::Show {
                        text: "Select prefabs to export first.".to_string(),
                        kind: UiToastKind::Warn,
                        ttl_secs: 4.0,
                    });
                    continue;
                }

                let mut ids: Vec<u128> = state.multi_selected_prefabs.iter().copied().collect();
                ids.sort();
                ids.dedup();
                state.export_dialog_pending_ids = ids;
                state.export_dialog_pending_realm = Some(env.active.realm_id.clone());

                let (tx, rx) = mpsc::channel();
                if let Ok(mut guard) = export_dialog.receiver.lock() {
                    *guard = Some(rx);
                }
                toasts.write(UiToastCommand::Show {
                    text: "Select export location…".to_string(),
                    kind: UiToastKind::Info,
                    ttl_secs: 3.0,
                });
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .add_filter("Prefab Zip", &["zip"])
                        .set_file_name("prefabs.zip")
                        .save_file()
                        .map(ensure_zip_extension);
                    let _ = tx.send(path);
                });
            }
        }
    }
}

pub(crate) fn model_library_manage_delete_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    env: ModelLibraryEnv,
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ModelLibraryManageDeleteButton>),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm)
        && state.multi_select_mode;
    if !visible {
        return;
    }

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

                if state.manage_delete_modal_root.is_some() {
                    continue;
                }
                if state.multi_selected_prefabs.is_empty() {
                    toasts.write(UiToastCommand::Show {
                        text: "Select prefabs to delete first.".to_string(),
                        kind: UiToastKind::Warn,
                        ttl_secs: 4.0,
                    });
                    continue;
                }

                let mut ids: Vec<u128> = state.multi_selected_prefabs.iter().copied().collect();
                ids.sort();
                ids.dedup();
                open_model_library_manage_delete_modal(
                    &mut commands,
                    &mut state,
                    env.active.realm_id.clone(),
                    ids,
                );
            }
        }
    }
}

pub(crate) fn model_library_manage_delete_modal_interactions(
    mut commands: Commands,
    env: ModelLibraryEnv,
    mut state: ResMut<ModelLibraryUiState>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut confirm: Query<
        &Interaction,
        (
            Changed<Interaction>,
            With<ModelLibraryManageDeleteConfirmButton>,
        ),
    >,
    mut cancel: Query<
        &Interaction,
        (
            Changed<Interaction>,
            With<ModelLibraryManageDeleteCancelButton>,
        ),
    >,
) {
    if state.manage_delete_modal_root.is_none() {
        return;
    }

    for interaction in &mut cancel {
        if *interaction == Interaction::Pressed {
            close_model_library_manage_delete_modal(&mut commands, &mut state);
            return;
        }
    }

    for interaction in &mut confirm {
        if *interaction != Interaction::Pressed {
            continue;
        }

        if env.config.automation_enabled && env.config.automation_monitor_mode {
            close_model_library_manage_delete_modal(&mut commands, &mut state);
            break;
        }

        let realm_id = state
            .manage_delete_modal_pending_realm
            .clone()
            .unwrap_or_else(|| env.active.realm_id.clone());
        let ids = state.manage_delete_modal_pending_ids.clone();

        let mut deleted = 0usize;
        let mut failed = 0usize;
        for prefab_id in &ids {
            match crate::realm_prefab_packages::delete_realm_prefab_package(&realm_id, *prefab_id) {
                Ok(_) => deleted += 1,
                Err(err) => {
                    failed += 1;
                    warn!("{err}");
                }
            }
        }

        if deleted > 0 {
            state.mark_models_dirty();
        }
        state.multi_selected_prefabs.clear();
        state.selected_prefab_id = None;
        state.pending_preview = None;
        close_model_library_preview(&mut commands, &mut state);

        close_model_library_manage_delete_modal(&mut commands, &mut state);

        if failed == 0 {
            toasts.write(UiToastCommand::Show {
                text: format!("Deleted {} prefab(s).", deleted),
                kind: UiToastKind::Info,
                ttl_secs: 4.0,
            });
        } else if deleted > 0 {
            toasts.write(UiToastCommand::Show {
                text: format!("Deleted {}, failed {}.", deleted, failed),
                kind: UiToastKind::Warn,
                ttl_secs: 5.0,
            });
        } else {
            toasts.write(UiToastCommand::Show {
                text: "Delete failed.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 5.0,
            });
        }
        break;
    }
}

pub(crate) fn model_library_manage_delete_modal_close_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut state: ResMut<ModelLibraryUiState>,
) {
    if state.manage_delete_modal_root.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        close_model_library_manage_delete_modal(&mut commands, &mut state);
    }
}

pub(crate) fn model_library_export_job_poll(
    export_job: Res<ModelLibraryExportJob>,
    mut toasts: MessageWriter<UiToastCommand>,
) {
    let Ok(mut guard) = export_job.receiver.lock() else {
        return;
    };
    let Some(receiver) = guard.as_ref() else {
        return;
    };

    match receiver.try_recv() {
        Ok(result) => {
            *guard = None;
            match result {
                Ok(count) => {
                    toasts.write(UiToastCommand::Show {
                        text: format!("Exported {} prefab(s).", count),
                        kind: UiToastKind::Info,
                        ttl_secs: 4.0,
                    });
                }
                Err(err) => {
                    toasts.write(UiToastCommand::Show {
                        text: err,
                        kind: UiToastKind::Error,
                        ttl_secs: 5.0,
                    });
                }
            }
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => {
            *guard = None;
            toasts.write(UiToastCommand::Show {
                text: "Export failed: worker disconnected.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 5.0,
            });
        }
    }
}

pub(crate) fn model_library_export_dialog_poll(
    mut state: ResMut<ModelLibraryUiState>,
    export_dialog: Res<ModelLibraryExportDialogJob>,
    export_job: Res<ModelLibraryExportJob>,
    mut toasts: MessageWriter<UiToastCommand>,
) {
    let Ok(mut guard) = export_dialog.receiver.lock() else {
        return;
    };
    let Some(receiver) = guard.as_ref() else {
        return;
    };

    let path = match receiver.try_recv() {
        Ok(path) => {
            *guard = None;
            path
        }
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            *guard = None;
            state.export_dialog_pending_ids.clear();
            state.export_dialog_pending_realm = None;
            toasts.write(UiToastCommand::Show {
                text: "Export canceled: dialog failed.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 4.0,
            });
            return;
        }
    };

    let Some(path) = path else {
        state.export_dialog_pending_ids.clear();
        state.export_dialog_pending_realm = None;
        return;
    };
    let Some(realm_id) = state.export_dialog_pending_realm.take() else {
        return;
    };
    if state.export_dialog_pending_ids.is_empty() {
        return;
    }

    let (tx, rx) = mpsc::channel();
    if let Ok(mut job_guard) = export_job.receiver.lock() {
        if job_guard.is_some() {
            toasts.write(UiToastCommand::Show {
                text: "Export already running.".to_string(),
                kind: UiToastKind::Warn,
                ttl_secs: 3.0,
            });
            return;
        }
        *job_guard = Some(rx);
    }

    let mut ids = state.export_dialog_pending_ids.clone();
    state.export_dialog_pending_ids.clear();
    ids.sort();
    ids.dedup();
    toasts.write(UiToastCommand::Show {
        text: "Exporting prefabs…".to_string(),
        kind: UiToastKind::Info,
        ttl_secs: 3.0,
    });
    std::thread::spawn(move || {
        let result = crate::prefab_zip::export_prefab_packages_to_zip(&realm_id, &ids, &path);
        let _ = tx.send(result);
    });
}

pub(crate) fn model_library_import_dialog_poll(
    mut state: ResMut<ModelLibraryUiState>,
    import_dialog: Res<ModelLibraryImportDialogJob>,
    import_job: Res<ModelLibraryImportJob>,
    mut toasts: MessageWriter<UiToastCommand>,
) {
    let Ok(mut guard) = import_dialog.receiver.lock() else {
        return;
    };
    let Some(receiver) = guard.as_ref() else {
        return;
    };

    let path = match receiver.try_recv() {
        Ok(path) => {
            *guard = None;
            path
        }
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            *guard = None;
            state.import_dialog_pending_realm = None;
            toasts.write(UiToastCommand::Show {
                text: "Import canceled: dialog failed.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 4.0,
            });
            return;
        }
    };

    let Some(path) = path else {
        state.import_dialog_pending_realm = None;
        return;
    };
    let Some(realm_id) = state.import_dialog_pending_realm.take() else {
        return;
    };

    let (tx, rx) = mpsc::channel();
    if let Ok(mut job_guard) = import_job.receiver.lock() {
        if job_guard.is_some() {
            toasts.write(UiToastCommand::Show {
                text: "Import already running.".to_string(),
                kind: UiToastKind::Warn,
                ttl_secs: 3.0,
            });
            return;
        }
        *job_guard = Some(rx);
    }

    toasts.write(UiToastCommand::Show {
        text: "Importing prefabs…".to_string(),
        kind: UiToastKind::Info,
        ttl_secs: 3.0,
    });
    std::thread::spawn(move || {
        let result = crate::prefab_zip::import_prefab_packages_from_zip(&realm_id, &path);
        let _ = tx.send(result);
    });
}

pub(crate) fn model_library_import_job_poll(
    mut state: ResMut<ModelLibraryUiState>,
    import_job: Res<ModelLibraryImportJob>,
    mut toasts: MessageWriter<UiToastCommand>,
) {
    let Ok(mut guard) = import_job.receiver.lock() else {
        return;
    };
    let Some(receiver) = guard.as_ref() else {
        return;
    };

    match receiver.try_recv() {
        Ok(result) => {
            *guard = None;
            match result {
                Ok(report) => {
                    state.mark_models_dirty();
                    let summary = format!(
                        "Imported {}, skipped {}, invalid {}.",
                        report.imported, report.skipped, report.invalid
                    );
                    let kind = if report.invalid > 0 {
                        UiToastKind::Warn
                    } else {
                        UiToastKind::Info
                    };
                    toasts.write(UiToastCommand::Show {
                        text: summary,
                        kind,
                        ttl_secs: 4.0,
                    });
                }
                Err(err) => {
                    toasts.write(UiToastCommand::Show {
                        text: err,
                        kind: UiToastKind::Error,
                        ttl_secs: 5.0,
                    });
                }
            }
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => {
            *guard = None;
            toasts.write(UiToastCommand::Show {
                text: "Import failed: worker disconnected.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 5.0,
            });
        }
    }
}

pub(crate) fn model_library_item_button_interactions(
    keys: Res<ButtonInput<KeyCode>>,
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

        if state.multi_select_mode {
            state.drag = None;
            state.pending_preview = None;

            let shift_pressed =
                keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
            if shift_pressed {
                let anchor = state.selected_prefab_id;
                let clicked = button.model_id;
                if let Some(anchor) = anchor {
                    let anchor_index = state.listed_prefabs.iter().position(|id| *id == anchor);
                    let clicked_index = state.listed_prefabs.iter().position(|id| *id == clicked);
                    if let (Some(a), Some(b)) = (anchor_index, clicked_index) {
                        let (start, end) = if a <= b { (a, b) } else { (b, a) };
                        for idx in start..=end {
                            let prefab_id = state.listed_prefabs[idx];
                            state.multi_selected_prefabs.insert(prefab_id);
                        }
                    } else {
                        state.multi_selected_prefabs.insert(clicked);
                    }
                } else {
                    state.multi_selected_prefabs.insert(clicked);
                }
            } else if state.multi_selected_prefabs.contains(&button.model_id) {
                state.multi_selected_prefabs.remove(&button.model_id);
            } else {
                state.multi_selected_prefabs.insert(button.model_id);
            }
            state.selected_prefab_id = Some(button.model_id);
            continue;
        }

        if state.drag.is_some() {
            continue;
        }

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

pub(crate) fn model_library_update_list_item_styles(
    state: Res<ModelLibraryUiState>,
    mut last_selected: Local<Option<u128>>,
    mut last_multi_mode: Local<bool>,
    mut last_multi: Local<Vec<u128>>,
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
    mut radios: Query<
        (
            Ref<ModelLibraryMultiSelectIndicator>,
            &mut Node,
            &mut BorderColor,
        ),
        Without<ModelLibraryListItem>,
    >,
    mut dots: Query<
        (Ref<ModelLibraryMultiSelectIndicatorDot>, &mut Visibility),
        Without<ModelLibrarySelectionMark>,
    >,
) {
    let selected_id = state.selected_prefab_id();
    let mut multi_ids: Vec<u128> = if state.multi_select_mode {
        state.multi_selected_prefabs.iter().copied().collect()
    } else {
        Vec::new()
    };
    multi_ids.sort();

    let multi_mode_changed = *last_multi_mode != state.multi_select_mode;
    if multi_mode_changed {
        *last_multi_mode = state.multi_select_mode;
    }

    let multi_changed = *last_multi != multi_ids;
    if multi_changed {
        *last_multi = multi_ids;
    }

    let selection_changed = if state.multi_select_mode {
        multi_mode_changed || multi_changed
    } else {
        let changed = *last_selected != selected_id;
        if changed {
            *last_selected = selected_id;
        }
        changed || multi_mode_changed
    };

    for (interaction, button, mut bg, mut border) in &mut buttons {
        if !selection_changed && !interaction.is_changed() && !interaction.is_added() {
            continue;
        }

        let is_selected = if state.multi_select_mode {
            state.multi_selected_prefabs.contains(&button.model_id)
        } else {
            selected_id == Some(button.model_id)
        };

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
        let is_selected = if state.multi_select_mode {
            state.multi_selected_prefabs.contains(&mark.model_id)
        } else {
            selected_id == Some(mark.model_id)
        };
        *vis = if is_selected {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    for (radio, mut node, mut border) in &mut radios {
        if !selection_changed && !radio.is_added() {
            continue;
        }

        node.display = if state.multi_select_mode {
            Display::Flex
        } else {
            Display::None
        };

        let is_selected =
            state.multi_select_mode && state.multi_selected_prefabs.contains(&radio.model_id);
        *border = BorderColor::all(if is_selected {
            Color::srgba(0.25, 0.95, 0.85, 0.85)
        } else {
            Color::srgba(0.25, 0.25, 0.30, 0.65)
        });
    }

    for (dot, mut vis) in &mut dots {
        if !selection_changed && !dot.is_added() {
            continue;
        }

        let is_selected =
            state.multi_select_mode && state.multi_selected_prefabs.contains(&dot.model_id);
        *vis = if is_selected {
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

    let Some(selected_id) = state.selected_prefab_id() else {
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
        .find(|(button, _node, _transform)| button.model_id == selected_id)
        .map(|(_button, node, transform)| (node, transform))
    else {
        return;
    };

    let panel_scale = panel_node.inverse_scale_factor();
    let viewport_h = panel_node.size.y.max(0.0) * panel_scale;
    let content_h = panel_node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h < 1.0 {
        // UI layout not ready yet (content size often reports 0 for one frame after a rebuild).
        // Retry next frame; do not lock-in `last_scrolled`.
        return;
    }
    let max_scroll = (content_h - viewport_h).max(0.0);

    let (panel_top_y, panel_bottom_y) =
        ui_rect_global_y_bounds(panel_node.content_box(), *panel_transform);
    let (item_top_y, item_bottom_y) =
        ui_rect_global_y_bounds(item_node.border_box(), *item_transform);

    let margin_physical = 6.0;
    if max_scroll <= 0.5 {
        // Content fits in the viewport. If the selected item isn't visible, it likely means the UI
        // transforms aren't stable yet. Retry next frame.
        let in_view = item_top_y >= panel_top_y + margin_physical
            && item_bottom_y <= panel_bottom_y - margin_physical;
        if !in_view {
            return;
        }
        scroll.y = 0.0;
        *last_scrolled = Some(selected_id);
        return;
    }
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
    mut gen3d: ModelLibraryGen3dSessionOpener,
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
    if state.multi_select_mode {
        state.drag = None;
        return;
    }

    let Some(mut drag) = state.drag else {
        return;
    };

    if !mouse_buttons.pressed(MouseButton::Left) {
        // Mouse was released; treat as either click-to-preview or drag-spawn.
        let prefab_id = drag.model_id;
        if !drag.is_dragging {
            if let Some(session_id) = gen3d.task_queue.find_session_for_prefab(prefab_id) {
                let open_session = gen3d.task_queue.metas.get(&session_id).is_some_and(|meta| {
                    matches!(
                        meta.task_state,
                        crate::gen3d::Gen3dTaskState::Waiting
                            | crate::gen3d::Gen3dTaskState::Running
                    )
                });
                if open_session {
                    if let Err(err) = gen3d.task_queue.swap_active_session(
                        session_id,
                        &mut gen3d.gen3d_workshop,
                        &mut gen3d.gen3d_job,
                        &mut gen3d.gen3d_draft,
                    ) {
                        gen3d.gen3d_workshop.error = Some(err);
                    }
                    gen3d.next_mode.set(GameMode::Build);
                    gen3d
                        .next_build_scene
                        .set(crate::types::BuildScene::Preview);
                    state.drag = None;
                    return;
                }
            }
        }
        if let Err(err) = ensure_realm_prefab_loaded(&env.active, prefab_id, &mut library) {
            warn!("{err}");
            state.drag = None;
            return;
        }
        if drag.is_dragging && drag.preview_translation.is_some() {
            if env.config.automation_enabled && env.config.automation_monitor_mode {
                // Monitor mode is local read-only: don’t allow spawning from the panel.
                state.request_preview(prefab_id);
            } else {
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
            }
        } else {
            state.request_preview(prefab_id);
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

fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    if !from.exists() {
        return Ok(());
    }
    if !from.is_dir() {
        return Err(format!("Expected directory: {}", from.display()));
    }

    std::fs::create_dir_all(to)
        .map_err(|err| format!("Failed to create {}: {err}", to.display()))?;

    let entries = std::fs::read_dir(from)
        .map_err(|err| format!("Failed to list {}: {err}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
        let src_path = entry.path();
        let dst_path = to.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|err| {
                format!(
                    "Failed to copy {} -> {}: {err}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }

    Ok(())
}

pub(crate) fn duplicate_realm_prefab_package(
    realm_id: &str,
    src_prefab_id: u128,
    library: &mut ObjectLibrary,
    descriptors: &mut PrefabDescriptorLibrary,
) -> Result<u128, String> {
    let src_prefabs_dir =
        crate::realm_prefab_packages::realm_prefab_package_prefabs_dir(realm_id, src_prefab_id);
    if !src_prefabs_dir.exists() {
        return Err(format!(
            "Prefab package not found in this realm: {}",
            uuid::Uuid::from_u128(src_prefab_id)
        ));
    }

    // Ensure all defs in this package are loaded (the root may already be present without internal
    // defs, which would make duplication fail with "Missing prefab def ... referenced by ...").
    crate::realm_prefabs::load_prefabs_into_library_from_dir(&src_prefabs_dir, library)?;

    fn collect_def_ids_recursive(root: &std::path::Path) -> Result<Vec<u128>, String> {
        let mut out: Vec<u128> = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(next) = stack.pop() {
            let entries = std::fs::read_dir(&next)
                .map_err(|err| format!("Failed to list {}: {err}", next.display()))?;
            for entry in entries {
                let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
                    continue;
                };
                if !file_name.ends_with(".json") || file_name.ends_with(".desc.json") {
                    continue;
                }
                let Some(stem) = file_name.strip_suffix(".json") else {
                    continue;
                };
                let Ok(uuid) = uuid::Uuid::parse_str(stem.trim()) else {
                    continue;
                };
                out.push(uuid.as_u128());
            }
        }
        Ok(out)
    }

    let mut def_ids = collect_def_ids_recursive(&src_prefabs_dir)?;

    if def_ids.is_empty() {
        return Err(format!(
            "Prefab package has no defs: {}",
            src_prefabs_dir.display()
        ));
    }

    def_ids.sort();
    def_ids.dedup();

    let mut defs: Vec<crate::object::registry::ObjectDef> = Vec::with_capacity(def_ids.len());
    for id in &def_ids {
        let Some(def) = library.get(*id) else {
            return Err(format!(
                "Missing prefab def {} referenced by {}",
                uuid::Uuid::from_u128(*id),
                src_prefabs_dir.display()
            ));
        };
        defs.push(def.clone());
    }

    let new_root_id = loop {
        let id = uuid::Uuid::new_v4().as_u128();
        let dir = crate::realm_prefab_packages::realm_prefab_package_dir(realm_id, id);
        if !dir.exists() {
            break id;
        }
    };

    let mut id_map: std::collections::HashMap<u128, u128> = std::collections::HashMap::new();
    for def in &defs {
        let new_id = if def.object_id == src_prefab_id {
            new_root_id
        } else {
            uuid::Uuid::new_v4().as_u128()
        };
        id_map.insert(def.object_id, new_id);
    }
    if !id_map.contains_key(&src_prefab_id) {
        return Err(format!(
            "Internal error: prefab package is missing its root def {}.",
            uuid::Uuid::from_u128(src_prefab_id)
        ));
    }

    let mut out_defs: Vec<crate::object::registry::ObjectDef> = Vec::with_capacity(defs.len());
    for def in &defs {
        let Some(new_id) = id_map.get(&def.object_id).copied() else {
            continue;
        };

        let mut new_def = def.clone();
        new_def.object_id = new_id;

        for part in &mut new_def.parts {
            if let crate::object::registry::ObjectPartKind::ObjectRef { object_id } = &mut part.kind
            {
                if let Some(mapped) = id_map.get(object_id) {
                    *object_id = *mapped;
                }
            }
        }

        if let Some(attack) = new_def.attack.as_mut() {
            if matches!(
                attack.kind,
                crate::object::registry::UnitAttackKind::RangedProjectile
            ) {
                if let Some(ranged) = attack.ranged.as_mut() {
                    if let Some(mapped) = id_map.get(&ranged.projectile_prefab) {
                        ranged.projectile_prefab = *mapped;
                    }
                    if let Some(mapped) = id_map.get(&ranged.muzzle.object_id) {
                        ranged.muzzle.object_id = *mapped;
                    }
                }
            }
        }

        if let Some(aim) = new_def.aim.as_mut() {
            for component_id in aim.components.iter_mut() {
                if let Some(mapped) = id_map.get(component_id) {
                    *component_id = *mapped;
                }
            }
        }

        out_defs.push(new_def);
    }

    crate::realm_prefab_packages::save_realm_prefab_package_defs(realm_id, new_root_id, &out_defs)?;

    let src_materials_dir =
        crate::realm_prefab_packages::realm_prefab_package_materials_dir(realm_id, src_prefab_id);
    let dst_materials_dir =
        crate::realm_prefab_packages::realm_prefab_package_materials_dir(realm_id, new_root_id);
    copy_dir_recursive(&src_materials_dir, &dst_materials_dir)?;

    let src_gen3d_source_dir = crate::realm_prefab_packages::realm_prefab_package_gen3d_source_dir(
        realm_id,
        src_prefab_id,
    );
    let dst_gen3d_source_dir =
        crate::realm_prefab_packages::realm_prefab_package_gen3d_source_dir(realm_id, new_root_id);
    copy_dir_recursive(&src_gen3d_source_dir, &dst_gen3d_source_dir)?;

    let src_thumb =
        crate::realm_prefab_packages::realm_prefab_package_thumbnail_path(realm_id, src_prefab_id);
    let dst_thumb =
        crate::realm_prefab_packages::realm_prefab_package_thumbnail_path(realm_id, new_root_id);
    if src_thumb.exists() {
        std::fs::copy(&src_thumb, &dst_thumb).map_err(|err| {
            format!(
                "Failed to copy thumbnail {} -> {}: {err}",
                src_thumb.display(),
                dst_thumb.display()
            )
        })?;
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let new_uuid = uuid::Uuid::from_u128(new_root_id).to_string();

    let mut new_descriptor = descriptors.get(src_prefab_id).cloned().unwrap_or_else(|| {
        crate::prefab_descriptors::PrefabDescriptorFileV1 {
            format_version: crate::prefab_descriptors::PREFAB_DESCRIPTOR_FORMAT_VERSION,
            prefab_id: new_uuid.clone(),
            label: None,
            text: None,
            tags: Vec::new(),
            roles: Vec::new(),
            interfaces: None,
            provenance: None,
            extra: std::collections::BTreeMap::new(),
        }
    });
    new_descriptor.prefab_id = new_uuid.clone();
    if let Some(prov) = new_descriptor.provenance.as_mut() {
        prov.modified_at_ms = Some(now_ms);
        if prov.created_at_ms.is_none() {
            prov.created_at_ms = Some(now_ms);
        }
        if let Some(gen3d) = prov.gen3d.as_mut() {
            gen3d.run_id = None;
        }
    }
    let desc_path =
        crate::realm_prefab_packages::realm_prefab_package_prefabs_dir(realm_id, new_root_id)
            .join(format!("{new_uuid}.desc.json"));
    crate::prefab_descriptors::save_prefab_descriptor_file(&desc_path, &new_descriptor)?;
    descriptors.upsert(new_root_id, new_descriptor);

    let src_edit_bundle = crate::realm_prefab_packages::realm_prefab_package_gen3d_edit_bundle_path(
        realm_id,
        src_prefab_id,
    );
    if src_edit_bundle.exists() {
        let bytes =
            std::fs::read(&src_edit_bundle).map_err(|err| format!("Failed to read: {err}"))?;
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|err| format!("Invalid JSON: {err}"))?;
        if let Some(field) = value.get_mut("root_prefab_id_uuid") {
            *field = serde_json::Value::String(new_uuid.clone());
        }
        let dst_edit_bundle =
            crate::realm_prefab_packages::realm_prefab_package_gen3d_edit_bundle_path(
                realm_id,
                new_root_id,
            );
        let payload =
            serde_json::to_vec_pretty(&value).map_err(|err| format!("Failed to encode: {err}"))?;
        std::fs::write(&dst_edit_bundle, payload)
            .map_err(|err| format!("Failed to write {}: {err}", dst_edit_bundle.display()))?;
    }

    for def in &out_defs {
        library.upsert(def.clone());
    }

    Ok(new_root_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn duplicate_realm_prefab_package_loads_missing_internal_defs() {
        let _guard = ENV_MUTEX.lock().expect("lock env mutex");

        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_duplicate_prefab_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        std::env::set_var("GRAVIMERA_HOME", &temp_root);

        let realm_id = "default";
        let root_id = uuid::Uuid::new_v4().as_u128();
        let internal_id = uuid::Uuid::new_v4().as_u128();

        let internal_def = crate::object::registry::ObjectDef {
            object_id: internal_id,
            label: "Internal".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: crate::object::registry::ColliderProfile::None,
            interaction: crate::object::registry::ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![crate::object::registry::ObjectPartDef::primitive(
                crate::object::registry::PrimitiveVisualDef::Primitive {
                    mesh: crate::object::registry::MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.6, 0.6, 0.7),
                    unlit: false,
                },
                Transform::default(),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let root_def = crate::object::registry::ObjectDef {
            object_id: root_id,
            label: "Root".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: crate::object::registry::ColliderProfile::None,
            interaction: crate::object::registry::ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![crate::object::registry::ObjectPartDef::object_ref(
                internal_id,
                Transform::default(),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        crate::realm_prefab_packages::save_realm_prefab_package_defs(
            realm_id,
            root_id,
            &[root_def, internal_def],
        )
        .expect("save realm prefab package");

        // Simulate a partially loaded library: root def is present, internal defs are not.
        let root_uuid = uuid::Uuid::from_u128(root_id).to_string();
        let src_prefabs_dir =
            crate::realm_prefab_packages::realm_prefab_package_prefabs_dir(realm_id, root_id);
        let root_json = src_prefabs_dir.join(format!("{root_uuid}.json"));

        let partial_dir = temp_root.join("partial_root_only");
        std::fs::create_dir_all(&partial_dir).expect("create partial dir");
        std::fs::copy(&root_json, partial_dir.join(format!("{root_uuid}.json")))
            .expect("copy root def json");

        let mut library = ObjectLibrary::default();
        crate::realm_prefabs::load_prefabs_into_library_from_dir(&partial_dir, &mut library)
            .expect("load root def only");
        assert!(library.get(root_id).is_some(), "root def should be loaded");
        assert!(
            library.get(internal_id).is_none(),
            "internal def should be missing"
        );

        let mut descriptors = PrefabDescriptorLibrary::default();
        let new_root_id =
            duplicate_realm_prefab_package(realm_id, root_id, &mut library, &mut descriptors)
                .expect("duplicate prefab package");
        assert_ne!(
            new_root_id, root_id,
            "duplicate should allocate a new root id"
        );

        let dst_prefabs_dir =
            crate::realm_prefab_packages::realm_prefab_package_prefabs_dir(realm_id, new_root_id);
        let dst_root_uuid = uuid::Uuid::from_u128(new_root_id).to_string();
        assert!(
            dst_prefabs_dir
                .join(format!("{dst_root_uuid}.json"))
                .exists(),
            "duplicate package should contain root def json"
        );

        std::env::remove_var("GRAVIMERA_HOME");
        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
