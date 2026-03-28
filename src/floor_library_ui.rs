use bevy::camera::visibility::RenderLayers;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::message::MessageReader;
use bevy::ecs::system::SystemParam;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::{Ime, PrimaryWindow};
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Mutex};

use crate::config::AppConfig;
use crate::genfloor::defs::FloorDefV1;
use crate::genfloor::set_active_world_floor;
use crate::genfloor::ActiveWorldFloor;
use crate::genfloor::WorldFloor;
use crate::orbit_capture;
use crate::realm::ActiveRealmScene;
use crate::rich_text::{set_rich_text_line, spawn_rich_text_line};
use crate::types::{BuildScene, EmojiAtlas, GameMode, UiFonts, UiToastCommand, UiToastKind};
use crate::ui::{set_ime_position_for_rich_text, ImeAnchorXPolicy};

const PANEL_WIDTH_PX: f32 = 320.0;
const PANEL_WIDTH_MANAGE_PX: f32 = 380.0;
const PANEL_Z_INDEX: i32 = 920;
const FLOOR_PREVIEW_Z_INDEX: i32 = 1200;
const FLOOR_PREVIEW_MODAL_Z_INDEX: i32 = FLOOR_PREVIEW_Z_INDEX + 20;
const FLOOR_PREVIEW_LAYER: usize = 29;
const FLOOR_PREVIEW_WIDTH_PX: u32 = 640;
const FLOOR_PREVIEW_HEIGHT_PX: u32 = 360;
pub(crate) const DEFAULT_FLOOR_ID: u128 = 0;

#[derive(Resource, Debug)]
pub(crate) struct FloorLibraryUiState {
    models_dirty: bool,
    open: bool,
    search_query: String,
    search_focused: bool,
    scrollbar_drag: Option<FloorLibraryScrollbarDrag>,
    thumbnail_cache: HashMap<u128, FloorLibraryThumbnailCacheEntry>,
    listed_floors: Vec<u128>,
    multi_select_mode: bool,
    multi_selected_floors: HashSet<u128>,
    multi_select_anchor_floor_id: Option<u128>,
    export_dialog_pending_ids: Vec<u128>,
    export_dialog_pending_realm: Option<String>,
    import_dialog_pending_realm: Option<String>,
    selected_floor_id: Option<u128>,
    pending_preview: Option<u128>,
    preview: Option<FloorLibraryPreview>,
    manage_delete_modal_root: Option<Entity>,
    manage_delete_modal_pending_realm: Option<String>,
    manage_delete_modal_pending_ids: Vec<u128>,
    last_rebuilt_scene: Option<(String, String)>,
}

#[derive(SystemParam)]
pub(crate) struct FloorLibraryEnv<'w> {
    config: Res<'w, AppConfig>,
    active: Res<'w, ActiveRealmScene>,
}

#[derive(Resource)]
pub(crate) struct FloorLibraryExportJob {
    receiver: Mutex<Option<mpsc::Receiver<Result<usize, String>>>>,
}

impl Default for FloorLibraryExportJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct FloorLibraryImportJob {
    receiver: Mutex<Option<mpsc::Receiver<Result<crate::floor_zip::FloorZipImportReport, String>>>>,
}

impl Default for FloorLibraryImportJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct FloorLibraryExportDialogJob {
    receiver: Mutex<Option<mpsc::Receiver<Option<std::path::PathBuf>>>>,
}

impl Default for FloorLibraryExportDialogJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct FloorLibraryImportDialogJob {
    receiver: Mutex<Option<mpsc::Receiver<Option<std::path::PathBuf>>>>,
}

impl Default for FloorLibraryImportDialogJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

impl Default for FloorLibraryUiState {
    fn default() -> Self {
        Self {
            models_dirty: true,
            open: false,
            search_query: String::new(),
            search_focused: false,
            scrollbar_drag: None,
            thumbnail_cache: HashMap::new(),
            listed_floors: Vec::new(),
            multi_select_mode: false,
            multi_selected_floors: HashSet::new(),
            multi_select_anchor_floor_id: None,
            export_dialog_pending_ids: Vec::new(),
            export_dialog_pending_realm: None,
            import_dialog_pending_realm: None,
            selected_floor_id: None,
            pending_preview: None,
            preview: None,
            manage_delete_modal_root: None,
            manage_delete_modal_pending_realm: None,
            manage_delete_modal_pending_ids: Vec::new(),
            last_rebuilt_scene: None,
        }
    }
}

impl FloorLibraryUiState {
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
            self.scrollbar_drag = None;
            self.search_focused = false;
        }
    }

    pub(crate) fn is_search_focused(&self) -> bool {
        self.search_focused
    }

    pub(crate) fn selected_floor_id(&self) -> Option<u128> {
        self.selected_floor_id
    }

    pub(crate) fn request_preview(&mut self, floor_id: u128) {
        self.pending_preview = Some(floor_id);
    }

    pub(crate) fn set_selected_floor_id(&mut self, floor_id: Option<u128>) {
        self.selected_floor_id = floor_id;
    }
}

#[derive(Debug)]
struct FloorLibraryScrollbarDrag {
    grab_offset: f32,
}

#[derive(Debug)]
struct FloorLibraryThumbnailCacheEntry {
    handle: Handle<Image>,
    modified_at_ms: u128,
}

#[derive(Debug)]
struct FloorLibraryPreview {
    floor_id: u128,
    ui_root: Entity,
    scene_root: Entity,
    target: Handle<Image>,
}

#[derive(Debug)]
struct SpawnedFloorLibraryPreviewScene {
    scene_root: Entity,
    target: Handle<Image>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloorLibraryPlaceholderState {
    Generating,
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FloorLibraryEditState {
    Editing,
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FloorLibraryGenfloorIndicatorKind {
    Working,
    Waiting,
}

#[derive(Component)]
pub(crate) struct FloorLibraryRoot;
#[derive(Component)]
pub(crate) struct FloorLibraryScrollPanel;
#[derive(Component)]
pub(crate) struct FloorLibraryList;
#[derive(Component)]
pub(crate) struct FloorLibraryListItem;
#[derive(Component)]
pub(crate) struct FloorLibraryGenfloorPlaceholderList;
#[derive(Component)]
pub(crate) struct FloorLibraryGenfloorPlaceholderItem;
#[derive(Component)]
pub(crate) struct FloorLibraryItemButton {
    floor_id: u128,
}
#[derive(Component)]
pub(crate) struct FloorLibrarySelectionMark {
    floor_id: u128,
}
#[derive(Component)]
pub(crate) struct FloorLibraryGenfloorThumbnailIndicator {
    floor_id: u128,
    kind: FloorLibraryGenfloorIndicatorKind,
}
#[derive(Component)]
pub(crate) struct FloorLibraryScrollbarTrack;
#[derive(Component)]
pub(crate) struct FloorLibraryScrollbarThumb;
#[derive(Component)]
pub(crate) struct FloorLibraryGenerateButton;
#[derive(Component)]
pub(crate) struct FloorLibraryGenerateButtonText;
#[derive(Component)]
pub(crate) struct FloorLibraryManageToggleButton;
#[derive(Component)]
pub(crate) struct FloorLibraryManageToggleButtonText;
#[derive(Component)]
pub(crate) struct FloorLibraryNormalActionsRow;
#[derive(Component)]
pub(crate) struct FloorLibraryManageActionsRow;
#[derive(Component)]
pub(crate) struct FloorLibraryImportButton;
#[derive(Component)]
pub(crate) struct FloorLibraryImportButtonText;
#[derive(Component)]
pub(crate) struct FloorLibraryExportButton;
#[derive(Component)]
pub(crate) struct FloorLibraryExportButtonText;
#[derive(Component)]
pub(crate) struct FloorLibraryManageDeleteButton;
#[derive(Component)]
pub(crate) struct FloorLibraryManageDeleteButtonText;
#[derive(Component)]
pub(crate) struct FloorLibraryManageSelectAllButton;
#[derive(Component)]
pub(crate) struct FloorLibraryManageSelectNoneButton;
#[derive(Component)]
pub(crate) struct FloorLibraryMultiSelectIndicator {
    floor_id: u128,
}
#[derive(Component)]
pub(crate) struct FloorLibraryMultiSelectIndicatorDot {
    floor_id: u128,
}
#[derive(Component)]
pub(crate) struct FloorLibraryManageDeleteModalRoot;
#[derive(Component)]
pub(crate) struct FloorLibraryManageDeleteConfirmButton;
#[derive(Component)]
pub(crate) struct FloorLibraryManageDeleteCancelButton;
#[derive(Component)]
pub(crate) struct FloorLibrarySearchField;
#[derive(Component)]
pub(crate) struct FloorLibrarySearchFieldText;
#[derive(Component)]
pub(crate) struct FloorLibraryPreviewOverlayRoot;
#[derive(Component)]
pub(crate) struct FloorLibraryPreviewCloseButton;
#[derive(Component)]
pub(crate) struct FloorLibraryPreviewSceneRoot;
#[derive(Component)]
pub(crate) struct FloorLibraryPreviewCamera;
#[derive(Component)]
pub(crate) struct FloorLibraryPreviewFloor;

pub(crate) fn setup_floor_library_ui(
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
            FloorLibraryRoot,
            Visibility::Hidden,
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
                    Text::new("Floors"),
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
                    FloorLibraryManageToggleButton,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Manage"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgba(0.25, 0.95, 0.85, 0.95)),
                        FloorLibraryManageToggleButtonText,
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
                FloorLibraryNormalActionsRow,
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
                        FloorLibraryImportButton,
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("Import"),
                            TextFont {
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            FloorLibraryImportButtonText,
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
                        FloorLibraryGenerateButton,
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("Generate"),
                            TextFont {
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            FloorLibraryGenerateButtonText,
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
                FloorLibraryManageActionsRow,
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
                            FloorLibraryExportButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Export"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                FloorLibraryExportButtonText,
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
                            FloorLibraryManageDeleteButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Delete"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                FloorLibraryManageDeleteButtonText,
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
                            FloorLibraryManageSelectAllButton,
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
                            FloorLibraryManageSelectNoneButton,
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
                FloorLibrarySearchField,
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
                        FloorLibrarySearchFieldText,
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
                    FloorLibraryScrollPanel,
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
                                FloorLibraryGenfloorPlaceholderList,
                            ));
                            list.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(6.0),
                                    ..default()
                                },
                                BackgroundColor(Color::NONE),
                                FloorLibraryList,
                            ));
                        });
                });

                row.spawn((
                    Node {
                        width: Val::Px(10.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Stretch,
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|scroll| {
                    scroll
                        .spawn((
                            Node {
                                flex_grow: 1.0,
                                width: Val::Percent(100.0),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            FloorLibraryScrollbarTrack,
                        ))
                        .with_children(|thumb| {
                            thumb.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(40.0),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.35, 0.35, 0.42, 0.85)),
                                BorderColor::all(Color::srgba(0.55, 0.55, 0.65, 0.95)),
                                FloorLibraryScrollbarThumb,
                            ));
                        });
                });
            });
        });
}

pub(crate) fn floor_library_update_visibility(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut commands: Commands,
    mut state: ResMut<FloorLibraryUiState>,
    mut roots: Query<&mut Visibility, With<FloorLibraryRoot>>,
    mut interactions: Query<
        &mut Interaction,
        Or<(
            With<FloorLibraryGenerateButton>,
            With<FloorLibraryManageToggleButton>,
            With<FloorLibraryImportButton>,
            With<FloorLibraryExportButton>,
            With<FloorLibraryManageDeleteButton>,
            With<FloorLibraryManageSelectAllButton>,
            With<FloorLibraryManageSelectNoneButton>,
            With<FloorLibrarySearchField>,
            With<FloorLibraryListItem>,
            With<FloorLibraryPreviewCloseButton>,
            With<FloorLibraryGenfloorPlaceholderItem>,
        )>,
    >,
    mut was_visible: Local<bool>,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm);
    for mut root in &mut roots {
        *root = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    if !visible {
        state.scrollbar_drag = None;
        state.search_focused = false;
        state.multi_select_mode = false;
        state.multi_selected_floors.clear();
        close_floor_library_manage_delete_modal(&mut commands, &mut state);
        close_floor_library_preview(&mut commands, &mut state);

        if *was_visible {
            for mut interaction in &mut interactions {
                *interaction = Interaction::None;
            }
        }
    }

    *was_visible = visible;
}

pub(crate) fn floor_library_update_manage_mode_ui(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    state: Res<FloorLibraryUiState>,
    mut manage_text: Query<(&mut Text, &mut TextColor), With<FloorLibraryManageToggleButtonText>>,
    mut rows: Query<(
        &mut Node,
        Option<&FloorLibraryNormalActionsRow>,
        Option<&FloorLibraryManageActionsRow>,
    )>,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm);

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

pub(crate) fn floor_library_update_panel_width(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    state: Res<FloorLibraryUiState>,
    mut roots: Query<&mut Node, With<FloorLibraryRoot>>,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm);
    let width = if visible && state.multi_select_mode {
        PANEL_WIDTH_MANAGE_PX
    } else {
        PANEL_WIDTH_PX
    };

    for mut node in &mut roots {
        node.width = Val::Px(width);
    }
}

pub(crate) fn floor_library_manage_toggle_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut commands: Commands,
    mut state: ResMut<FloorLibraryUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<FloorLibraryManageToggleButton>),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm);
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
                state.pending_preview = None;

                close_floor_library_manage_delete_modal(&mut commands, &mut state);

                let next_manage_mode = !state.multi_select_mode;
                state.multi_select_mode = next_manage_mode;
                if next_manage_mode {
                    state.multi_selected_floors.clear();
                    if let Some(selected) = state.selected_floor_id {
                        if selected != DEFAULT_FLOOR_ID {
                            state.multi_selected_floors.insert(selected);
                        }
                    }
                    state.multi_select_anchor_floor_id =
                        state.selected_floor_id.filter(|floor_id| *floor_id != DEFAULT_FLOOR_ID);
                    close_floor_library_preview(&mut commands, &mut state);
                } else {
                    state.multi_selected_floors.clear();
                    state.multi_select_anchor_floor_id = None;
                }
            }
        }
    }
}

pub(crate) fn floor_library_generate_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    state: Res<FloorLibraryUiState>,
    mut next_scene: ResMut<NextState<BuildScene>>,
    mut genfloor_job: ResMut<crate::genfloor::GenFloorAiJob>,
    mut genfloor_workshop: ResMut<crate::genfloor::GenFloorWorkshop>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<FloorLibraryGenerateButton>),
    >,
) {
    if state.multi_select_mode
        || !matches!(mode.get(), GameMode::Build)
        || !matches!(build_scene.get(), BuildScene::Realm)
    {
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
                if !genfloor_job.running {
                    genfloor_job.reset_for_new_build();
                    genfloor_workshop.reset_for_new_build();
                }
                next_scene.set(BuildScene::FloorPreview);
            }
        }
    }
}

pub(crate) fn floor_library_manage_select_all_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut state: ResMut<FloorLibraryUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (
            Changed<Interaction>,
            With<FloorLibraryManageSelectAllButton>,
        ),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
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

                state.multi_selected_floors = state
                    .listed_floors
                    .iter()
                    .copied()
                    .filter(|floor_id| *floor_id != DEFAULT_FLOOR_ID)
                    .collect();
                let anchor_valid = state.multi_select_anchor_floor_id.is_some_and(|floor_id| {
                    floor_id != DEFAULT_FLOOR_ID && state.listed_floors.iter().any(|id| *id == floor_id)
                });
                if !anchor_valid {
                    state.multi_select_anchor_floor_id = state
                        .listed_floors
                        .iter()
                        .copied()
                        .find(|floor_id| *floor_id != DEFAULT_FLOOR_ID);
                }
            }
        }
    }
}

pub(crate) fn floor_library_manage_select_none_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut state: ResMut<FloorLibraryUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (
            Changed<Interaction>,
            With<FloorLibraryManageSelectNoneButton>,
        ),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
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

                state.multi_selected_floors.clear();
            }
        }
    }
}

pub(crate) fn floor_library_import_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    env: FloorLibraryEnv,
    mut state: ResMut<FloorLibraryUiState>,
    import_job: Res<FloorLibraryImportJob>,
    import_dialog: Res<FloorLibraryImportDialogJob>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<FloorLibraryImportButton>),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
        && !state.multi_select_mode;
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
                        .add_filter("Floor Zip", &["zip"])
                        .pick_file();
                    let _ = tx.send(path);
                });
            }
        }
    }
}

pub(crate) fn floor_library_export_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    env: FloorLibraryEnv,
    mut state: ResMut<FloorLibraryUiState>,
    export_job: Res<FloorLibraryExportJob>,
    export_dialog: Res<FloorLibraryExportDialogJob>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<FloorLibraryExportButton>),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
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

                if state.multi_selected_floors.is_empty() {
                    toasts.write(UiToastCommand::Show {
                        text: "Select floors to export first.".to_string(),
                        kind: UiToastKind::Warn,
                        ttl_secs: 4.0,
                    });
                    continue;
                }

                let mut ids: Vec<u128> = state
                    .multi_selected_floors
                    .iter()
                    .copied()
                    .filter(|floor_id| *floor_id != DEFAULT_FLOOR_ID)
                    .collect();
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
                        .add_filter("Floor Zip", &["zip"])
                        .set_file_name("floors.zip")
                        .save_file()
                        .map(ensure_zip_extension);
                    let _ = tx.send(path);
                });
            }
        }
    }
}

pub(crate) fn floor_library_manage_delete_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    env: FloorLibraryEnv,
    mut commands: Commands,
    mut state: ResMut<FloorLibraryUiState>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<FloorLibraryManageDeleteButton>),
    >,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
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
                if state.multi_selected_floors.is_empty() {
                    toasts.write(UiToastCommand::Show {
                        text: "Select floors to delete first.".to_string(),
                        kind: UiToastKind::Warn,
                        ttl_secs: 4.0,
                    });
                    continue;
                }

                let mut ids: Vec<u128> = state
                    .multi_selected_floors
                    .iter()
                    .copied()
                    .filter(|floor_id| *floor_id != DEFAULT_FLOOR_ID)
                    .collect();
                ids.sort();
                ids.dedup();
                open_floor_library_manage_delete_modal(
                    &mut commands,
                    &mut state,
                    env.active.realm_id.clone(),
                    ids,
                );
            }
        }
    }
}

pub(crate) fn floor_library_manage_delete_modal_interactions(
    mut commands: Commands,
    env: FloorLibraryEnv,
    mut state: ResMut<FloorLibraryUiState>,
    mut active_floor: ResMut<ActiveWorldFloor>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut confirm: Query<
        &Interaction,
        (
            Changed<Interaction>,
            With<FloorLibraryManageDeleteConfirmButton>,
        ),
    >,
    mut cancel: Query<
        &Interaction,
        (
            Changed<Interaction>,
            With<FloorLibraryManageDeleteCancelButton>,
        ),
    >,
) {
    if state.manage_delete_modal_root.is_none() {
        return;
    }

    for interaction in &mut cancel {
        if *interaction == Interaction::Pressed {
            close_floor_library_manage_delete_modal(&mut commands, &mut state);
            return;
        }
    }

    for interaction in &mut confirm {
        if *interaction != Interaction::Pressed {
            continue;
        }

        if env.config.automation_enabled && env.config.automation_monitor_mode {
            close_floor_library_manage_delete_modal(&mut commands, &mut state);
            break;
        }

        let realm_id = state
            .manage_delete_modal_pending_realm
            .clone()
            .unwrap_or_else(|| env.active.realm_id.clone());
        let ids = state.manage_delete_modal_pending_ids.clone();

        let mut deleted = 0usize;
        let mut failed = 0usize;
        for floor_id in &ids {
            if *floor_id == DEFAULT_FLOOR_ID {
                continue;
            }
            match crate::realm_floor_packages::delete_realm_floor_package(&realm_id, *floor_id) {
                Ok(_) => {
                    deleted += 1;
                    state.thumbnail_cache.remove(floor_id);
                }
                Err(err) => {
                    failed += 1;
                    warn!("{err}");
                }
            }
        }

        let deleted_selected = state.selected_floor_id.is_some_and(|floor_id| {
            floor_id != DEFAULT_FLOOR_ID && ids.iter().any(|deleted_id| *deleted_id == floor_id)
        });
        let deleted_active = active_floor
            .floor_id
            .is_some_and(|floor_id| ids.iter().any(|deleted_id| *deleted_id == floor_id));
        if deleted_active {
            set_active_world_floor(&mut active_floor, None, FloorDefV1::default_world());
            let _ = crate::scene_floor_selection::save_scene_floor_selection(
                &realm_id,
                &env.active.scene_id,
                None,
            );
        }
        if deleted_active || deleted_selected {
            state.selected_floor_id = Some(DEFAULT_FLOOR_ID);
        }

        if deleted > 0 {
            state.mark_models_dirty();
        }
        state.multi_selected_floors.clear();
        state.pending_preview = None;
        close_floor_library_preview(&mut commands, &mut state);

        close_floor_library_manage_delete_modal(&mut commands, &mut state);

        if failed == 0 {
            toasts.write(UiToastCommand::Show {
                text: format!("Deleted {} floor(s).", deleted),
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

pub(crate) fn floor_library_manage_delete_modal_close_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut state: ResMut<FloorLibraryUiState>,
) {
    if state.manage_delete_modal_root.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        close_floor_library_manage_delete_modal(&mut commands, &mut state);
    }
}

pub(crate) fn floor_library_export_job_poll(
    export_job: Res<FloorLibraryExportJob>,
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
                        text: format!("Exported {} floor(s).", count),
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

pub(crate) fn floor_library_export_dialog_poll(
    mut state: ResMut<FloorLibraryUiState>,
    export_dialog: Res<FloorLibraryExportDialogJob>,
    export_job: Res<FloorLibraryExportJob>,
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
    ids.retain(|floor_id| *floor_id != DEFAULT_FLOOR_ID);
    ids.sort();
    ids.dedup();
    toasts.write(UiToastCommand::Show {
        text: "Exporting floors…".to_string(),
        kind: UiToastKind::Info,
        ttl_secs: 3.0,
    });
    std::thread::spawn(move || {
        let result = crate::floor_zip::export_floor_packages_to_zip(&realm_id, &ids, &path);
        let _ = tx.send(result);
    });
}

pub(crate) fn floor_library_import_dialog_poll(
    mut state: ResMut<FloorLibraryUiState>,
    import_dialog: Res<FloorLibraryImportDialogJob>,
    import_job: Res<FloorLibraryImportJob>,
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
        text: "Importing floors…".to_string(),
        kind: UiToastKind::Info,
        ttl_secs: 3.0,
    });
    std::thread::spawn(move || {
        let result = crate::floor_zip::import_floor_packages_from_zip(&realm_id, &path);
        let _ = tx.send(result);
    });
}

pub(crate) fn floor_library_import_job_poll(
    mut state: ResMut<FloorLibraryUiState>,
    import_job: Res<FloorLibraryImportJob>,
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

pub(crate) fn floor_library_search_field_focus(
    mut state: ResMut<FloorLibraryUiState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut fields: Query<&Interaction, (Changed<Interaction>, With<FloorLibrarySearchField>)>,
) {
    if !state.is_open() {
        return;
    }
    for interaction in &mut fields {
        if matches!(*interaction, Interaction::Pressed) {
            state.search_focused = true;
            if let Ok(mut window) = windows.single_mut() {
                window.ime_enabled = true;
            }
        }
    }
}

pub(crate) fn floor_library_search_ime_position(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    state: Res<FloorLibraryUiState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    fields: Query<(&ComputedNode, &UiGlobalTransform), With<FloorLibrarySearchField>>,
    text_root: Query<Entity, With<FloorLibrarySearchFieldText>>,
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
        && matches!(build_scene.get(), BuildScene::Realm);
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

pub(crate) fn floor_library_search_text_input(
    mut state: ResMut<FloorLibraryUiState>,
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
            KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter => {
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
                if let Some(text) = &event.text {
                    changed |= push_text(&mut state.search_query, text);
                }
            }
        }

        if changed {
            state.models_dirty = true;
        }
    }
}

pub(crate) fn floor_library_update_search_field_ui(
    mut commands: Commands,
    state: Res<FloorLibraryUiState>,
    ui_fonts: Res<UiFonts>,
    emoji_atlas: Res<EmojiAtlas>,
    asset_server: Res<AssetServer>,
    mut fields: Query<(&mut BackgroundColor, &mut BorderColor), With<FloorLibrarySearchField>>,
    rich_text: Query<Entity, With<FloorLibrarySearchFieldText>>,
    mut last_text: Local<Option<(String, bool)>>,
) {
    let Ok((mut bg, mut border)) = fields.single_mut() else {
        return;
    };
    if state.search_focused {
        *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.75));
        *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.85));
    } else {
        *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65));
        *border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));
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

fn floor_library_collect_edit_states(
    genfloor_job: &crate::genfloor::GenFloorAiJob,
    genfloor_workshop: &crate::genfloor::GenFloorWorkshop,
) -> HashMap<u128, FloorLibraryEditState> {
    let mut edit_states: HashMap<u128, FloorLibraryEditState> = HashMap::new();
    let Some(floor_id) = genfloor_job.edit_base_floor_id() else {
        return edit_states;
    };

    let queued = genfloor_workshop.status.to_lowercase().contains("queued");
    let state = if genfloor_job.running {
        Some(FloorLibraryEditState::Editing)
    } else if queued {
        Some(FloorLibraryEditState::Queued)
    } else {
        None
    };
    let Some(state) = state else {
        return edit_states;
    };
    edit_states.insert(floor_id, state);
    edit_states
}

fn floor_library_label_prefix(state: Option<FloorLibraryEditState>) -> (&'static str, Color) {
    match state {
        Some(FloorLibraryEditState::Editing) => {
            ("Editing…: ", Color::srgba(0.30, 0.97, 0.45, 0.95))
        }
        Some(FloorLibraryEditState::Queued) => ("Queued…: ", Color::srgba(0.95, 0.85, 0.25, 0.95)),
        None => ("", Color::srgb(0.92, 0.92, 0.96)),
    }
}

pub(crate) fn floor_library_rebuild_list_ui(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    active: Res<ActiveRealmScene>,
    genfloor_job: Res<crate::genfloor::GenFloorAiJob>,
    genfloor_workshop: Res<crate::genfloor::GenFloorWorkshop>,
    mut state: ResMut<FloorLibraryUiState>,
    lists: Query<Entity, With<FloorLibraryList>>,
    existing_items: Query<Entity, With<FloorLibraryListItem>>,
) {
    if !state.is_open() {
        return;
    }

    let active_scene = Some((active.realm_id.clone(), active.scene_id.clone()));
    if state.last_rebuilt_scene.as_ref() != active_scene.as_ref() {
        state.models_dirty = true;
    }

    if !state.models_dirty {
        return;
    }
    state.models_dirty = false;
    state.last_rebuilt_scene = active_scene;

    let Ok(list_entity) = lists.single() else {
        return;
    };
    for item in &existing_items {
        commands.entity(item).try_despawn();
    }

    #[derive(Debug)]
    struct Row {
        floor_id: u128,
        display_name: String,
        modified_at_ms: u128,
        score: u32,
        thumbnail: Option<Handle<Image>>,
    }

    let query = state.search_query.trim().to_string();
    let mut rows: Vec<Row> = Vec::new();
    let floor_ids = match crate::realm_floor_packages::list_realm_floor_packages(&active.realm_id) {
        Ok(ids) => ids,
        Err(err) => {
            warn!("Failed to list floors: {err}");
            return;
        }
    };

    fn relevance_score(query: &str, name: &str, id: &str) -> u32 {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return 0;
        }
        let name_l = name.to_lowercase();
        let id_l = id.to_lowercase();
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
        for token in query.split_whitespace() {
            if token.is_empty() {
                continue;
            }
            if name_l.contains(token) {
                score = score.saturating_add(60);
            }
        }
        score
    }

    fn build_default_floor_row(query: &str) -> Option<Row> {
        let display_name = "Default Floor".to_string();
        let score = relevance_score(query, &display_name, "default");
        if !query.is_empty() && score == 0 {
            return None;
        }
        Some(Row {
            floor_id: DEFAULT_FLOOR_ID,
            display_name,
            modified_at_ms: u128::MAX,
            score,
            thumbnail: None,
        })
    }

    if let Some(default_row) = build_default_floor_row(&query) {
        rows.push(default_row);
    }

    for floor_id in floor_ids {
        let uuid = uuid::Uuid::from_u128(floor_id).to_string();
        let def =
            crate::realm_floor_packages::load_realm_floor_def(&active.realm_id, floor_id).ok();
        let display_name = def
            .as_ref()
            .and_then(|d| d.label.as_ref())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .unwrap_or_else(|| uuid.clone());

        let floor_def_path = crate::realm_floor_packages::realm_floor_package_floor_def_path(
            &active.realm_id,
            floor_id,
        );
        let modified_at_ms = std::fs::metadata(&floor_def_path)
            .and_then(|m| m.modified())
            .map(system_time_ms)
            .unwrap_or(0);

        let score = relevance_score(&query, &display_name, &uuid);
        if !query.is_empty() && score == 0 {
            continue;
        }

        let thumbnail = {
            let path = crate::realm_floor_packages::realm_floor_package_thumbnail_path(
                &active.realm_id,
                floor_id,
            );
            if let Ok(meta) = std::fs::metadata(&path) {
                let modified_at_ms = meta.modified().map(system_time_ms).unwrap_or(0);
                if let Some(entry) = state.thumbnail_cache.get(&floor_id) {
                    if entry.modified_at_ms == modified_at_ms {
                        Some(entry.handle.clone())
                    } else {
                        match load_png_ui_image(&mut images, &path) {
                            Ok(handle) => {
                                state.thumbnail_cache.insert(
                                    floor_id,
                                    FloorLibraryThumbnailCacheEntry {
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
                                floor_id,
                                FloorLibraryThumbnailCacheEntry {
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
            floor_id,
            display_name,
            modified_at_ms,
            score,
            thumbnail,
        });
    }

    rows.sort_by(|a, b| {
        let a_default = a.floor_id == DEFAULT_FLOOR_ID;
        let b_default = b.floor_id == DEFAULT_FLOOR_ID;
        if a_default != b_default {
            return if a_default {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }

        if !query.is_empty() {
            b.score
                .cmp(&a.score)
                .then_with(|| b.modified_at_ms.cmp(&a.modified_at_ms))
                .then_with(|| a.display_name.cmp(&b.display_name))
                .then_with(|| a.floor_id.cmp(&b.floor_id))
        } else {
            b.modified_at_ms
                .cmp(&a.modified_at_ms)
                .then_with(|| a.display_name.cmp(&b.display_name))
                .then_with(|| a.floor_id.cmp(&b.floor_id))
        }
    });

    state.listed_floors = rows.iter().map(|row| row.floor_id).collect();
    if state.multi_select_mode && !state.multi_selected_floors.is_empty() {
        let listed: HashSet<u128> = state.listed_floors.iter().copied().collect();
        state
            .multi_selected_floors
            .retain(|floor_id| listed.contains(floor_id) && *floor_id != DEFAULT_FLOOR_ID);
    }

    let edit_states = floor_library_collect_edit_states(&genfloor_job, &genfloor_workshop);

    commands.entity(list_entity).with_children(|list| {
        for row in rows {
            let edit_state = edit_states.get(&row.floor_id).copied();
            let (prefix_text, prefix_color) = floor_library_label_prefix(edit_state);
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
                FloorLibraryListItem,
                FloorLibraryItemButton {
                    floor_id: row.floor_id,
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
                    FloorLibrarySelectionMark {
                        floor_id: row.floor_id,
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
                                FloorLibraryGenfloorThumbnailIndicator {
                                    floor_id: row.floor_id,
                                    kind: FloorLibraryGenfloorIndicatorKind::Working,
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
                                FloorLibraryGenfloorThumbnailIndicator {
                                    floor_id: row.floor_id,
                                    kind: FloorLibraryGenfloorIndicatorKind::Waiting,
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
                    ))
                    .with_children(|label_root| {
                        label_root.spawn((
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
                    FloorLibraryMultiSelectIndicator {
                        floor_id: row.floor_id,
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
                        FloorLibraryMultiSelectIndicatorDot {
                            floor_id: row.floor_id,
                        },
                    ));
                });
            });
        }
    });
}

pub(crate) fn floor_library_sync_genfloor_placeholders(
    mut commands: Commands,
    state: Res<FloorLibraryUiState>,
    genfloor_job: Res<crate::genfloor::GenFloorAiJob>,
    genfloor_workshop: Res<crate::genfloor::GenFloorWorkshop>,
    lists: Query<Entity, With<FloorLibraryGenfloorPlaceholderList>>,
    existing: Query<Entity, With<FloorLibraryGenfloorPlaceholderItem>>,
    mut last_sig: Local<Vec<(FloorLibraryPlaceholderState, String)>>,
) {
    let Ok(list_entity) = lists.single() else {
        return;
    };

    if !state.is_open() {
        if !existing.is_empty() {
            for entity in &existing {
                commands.entity(entity).try_despawn();
            }
        }
        last_sig.clear();
        return;
    }

    let prompt = genfloor_workshop.prompt.trim();
    let snippet = if prompt.is_empty() {
        "(new floor)".to_string()
    } else {
        let trimmed: String = prompt.chars().take(42).collect();
        if prompt.chars().count() > 42 {
            format!("{trimmed}…")
        } else {
            trimmed
        }
    };

    let mut placeholders: Vec<(FloorLibraryPlaceholderState, String)> = Vec::new();
    let queued = genfloor_workshop.status.to_lowercase().contains("queued");
    let is_new_build = genfloor_job.edit_base_floor_id().is_none();
    if is_new_build && genfloor_job.running {
        placeholders.push((FloorLibraryPlaceholderState::Generating, snippet.clone()));
    } else if is_new_build && queued {
        placeholders.push((FloorLibraryPlaceholderState::Queued, snippet.clone()));
    }

    if placeholders.is_empty() {
        if !existing.is_empty() {
            for entity in &existing {
                commands.entity(entity).try_despawn();
            }
        }
        last_sig.clear();
        return;
    }

    if *last_sig == placeholders && !existing.is_empty() {
        return;
    }
    *last_sig = placeholders.clone();

    if !existing.is_empty() {
        for entity in &existing {
            commands.entity(entity).try_despawn();
        }
    }

    fn placeholder_prefix(state: FloorLibraryPlaceholderState) -> (&'static str, Color) {
        match state {
            FloorLibraryPlaceholderState::Generating => {
                ("Generating…: ", Color::srgba(0.30, 0.97, 0.45, 0.95))
            }
            FloorLibraryPlaceholderState::Queued => {
                ("Queued…: ", Color::srgba(0.95, 0.85, 0.25, 0.95))
            }
        }
    }

    commands.entity(list_entity).with_children(|list| {
        for (state, snippet) in placeholders {
            let (prefix, prefix_color) = placeholder_prefix(state);
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
                FloorLibraryGenfloorPlaceholderItem,
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
                });

                b.spawn((
                    Text::new(prefix),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(prefix_color),
                ))
                .with_children(|label_root| {
                    label_root.spawn((
                        TextSpan::new(snippet.clone()),
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

pub(crate) fn floor_library_genfloor_placeholder_item_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    state: Res<FloorLibraryUiState>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (
            Changed<Interaction>,
            With<FloorLibraryGenfloorPlaceholderItem>,
        ),
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
                if state.multi_select_mode {
                    continue;
                }
                if !matches!(mode.get(), GameMode::Build)
                    || !matches!(build_scene.get(), BuildScene::Realm)
                {
                    continue;
                }
                next_build_scene.set(BuildScene::FloorPreview);
            }
        }
    }
}

pub(crate) fn floor_library_update_genfloor_thumbnail_indicators(
    time: Res<Time>,
    genfloor_job: Res<crate::genfloor::GenFloorAiJob>,
    genfloor_workshop: Res<crate::genfloor::GenFloorWorkshop>,
    mut indicators: Query<(
        &FloorLibraryGenfloorThumbnailIndicator,
        &mut Visibility,
        &mut UiTransform,
    )>,
) {
    let mut active_state: Option<(u128, FloorLibraryGenfloorIndicatorKind)> = None;
    if let Some(floor_id) = genfloor_job.edit_base_floor_id() {
        let queued = genfloor_workshop.status.to_lowercase().contains("queued");
        if genfloor_job.running {
            active_state = Some((floor_id, FloorLibraryGenfloorIndicatorKind::Working));
        } else if queued {
            active_state = Some((floor_id, FloorLibraryGenfloorIndicatorKind::Waiting));
        }
    }

    let t = time.elapsed_secs();
    for (indicator, mut vis, mut transform) in &mut indicators {
        let show = active_state.as_ref().is_some_and(|(floor_id, kind)| {
            *floor_id == indicator.floor_id && *kind == indicator.kind
        });
        *vis = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !show {
            continue;
        }

        let offset = ((indicator.floor_id % 97) as f32) * 0.23;
        transform.rotation = match indicator.kind {
            FloorLibraryGenfloorIndicatorKind::Working => Rot2::radians(t * 7.0 + offset),
            FloorLibraryGenfloorIndicatorKind::Waiting => Rot2::radians(0.0),
        };
    }
}

fn close_floor_library_manage_delete_modal(
    commands: &mut Commands,
    state: &mut FloorLibraryUiState,
) {
    if let Some(root) = state.manage_delete_modal_root.take() {
        commands.entity(root).try_despawn();
    }
    state.manage_delete_modal_pending_realm = None;
    state.manage_delete_modal_pending_ids.clear();
}

fn open_floor_library_manage_delete_modal(
    commands: &mut Commands,
    state: &mut FloorLibraryUiState,
    realm_id: String,
    floor_ids: Vec<u128>,
) {
    if state.manage_delete_modal_root.is_some() {
        return;
    }

    let count = floor_ids.len();
    let title = if count == 1 {
        "Delete selected floor?".to_string()
    } else {
        format!("Delete {count} selected floors?")
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
            ZIndex(FLOOR_PREVIEW_MODAL_Z_INDEX),
            FloorLibraryManageDeleteModalRoot,
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
                            "This deletes floor files from disk. Scenes referencing deleted floors may fall back to Default Floor.",
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
                                FloorLibraryManageDeleteConfirmButton,
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
                                FloorLibraryManageDeleteCancelButton,
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
    state.manage_delete_modal_pending_ids = floor_ids;
}

fn close_floor_library_preview(commands: &mut Commands, state: &mut FloorLibraryUiState) {
    let Some(preview) = state.preview.take() else {
        return;
    };
    let target_id = preview.target.id();
    commands.entity(preview.ui_root).try_despawn();
    commands.entity(preview.scene_root).try_despawn();
    commands.queue(move |world: &mut World| {
        if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
            images.remove(target_id);
        }
    });
}

fn spawn_floor_library_preview_scene(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    def: &FloorDefV1,
) -> Result<SpawnedFloorLibraryPreviewScene, String> {
    let target = orbit_capture::create_render_target(
        images,
        FLOOR_PREVIEW_WIDTH_PX,
        FLOOR_PREVIEW_HEIGHT_PX,
    );

    let aspect = FLOOR_PREVIEW_WIDTH_PX.max(1) as f32 / FLOOR_PREVIEW_HEIGHT_PX.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;

    let size_x = def.mesh.size_m[0].max(0.5);
    let size_z = def.mesh.size_m[1].max(0.5);
    let thickness = def.mesh.thickness_m.max(0.05);
    let half_extents = Vec3::new(size_x, thickness, size_z) * 0.5;
    let focus = Vec3::ZERO;

    let yaw = std::f32::consts::FRAC_PI_6;
    let pitch = -0.45;
    let base_distance =
        orbit_capture::required_distance_for_view(half_extents, yaw, pitch, fov_y, aspect, near);
    let distance = (base_distance * 1.1).clamp(near + 0.2, 500.0);
    let camera_transform = orbit_capture::orbit_transform(yaw, pitch, distance, focus);

    let render_layer = RenderLayers::layer(FLOOR_PREVIEW_LAYER);

    let scene_root = commands
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            FloorLibraryPreviewSceneRoot,
        ))
        .id();

    let floor_id = commands
        .spawn((
            WorldFloor,
            FloorLibraryPreviewFloor,
            render_layer.clone(),
            Transform::IDENTITY,
            Visibility::Inherited,
        ))
        .id();
    commands.entity(scene_root).add_child(floor_id);

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
            FloorLibraryPreviewCamera,
        ))
        .id();
    commands.entity(scene_root).add_child(camera_id);

    Ok(SpawnedFloorLibraryPreviewScene { scene_root, target })
}

pub(crate) fn floor_library_open_preview_panel(
    mut commands: Commands,
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut images: ResMut<Assets<Image>>,
    mut state: ResMut<FloorLibraryUiState>,
    mut active_floor: ResMut<ActiveWorldFloor>,
) {
    if !matches!(mode.get(), GameMode::Build) || !matches!(build_scene.get(), BuildScene::Realm) {
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

    let Some(floor_id) = state.pending_preview.take() else {
        return;
    };

    if state
        .preview
        .as_ref()
        .is_some_and(|preview| preview.floor_id == floor_id)
    {
        return;
    }

    close_floor_library_preview(&mut commands, &mut state);

    let def = if floor_id == DEFAULT_FLOOR_ID {
        FloorDefV1::default_world()
    } else {
        active_floor.def.clone()
    };

    let scene = match spawn_floor_library_preview_scene(&mut commands, &mut images, &def) {
        Ok(scene) => scene,
        Err(err) => {
            warn!("{err}");
            return;
        }
    };

    let uuid = uuid::Uuid::from_u128(floor_id).to_string();
    let name = if floor_id == DEFAULT_FLOOR_ID {
        "Default Floor".to_string()
    } else {
        def.label
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .unwrap_or_else(|| uuid.clone())
    };

    let info = format!(
        "ID: {}\nSize: {:.1} x {:.1} m\nSubdiv: {} x {}",
        if floor_id == DEFAULT_FLOOR_ID {
            "default".to_string()
        } else {
            uuid.clone()
        },
        def.mesh.size_m[0],
        def.mesh.size_m[1],
        def.mesh.subdiv[0],
        def.mesh.subdiv[1]
    );

    let ui_root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(64.0),
                left: Val::Px(300.0),
                width: Val::Px(700.0),
                height: Val::Px(520.0),
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
            ZIndex(FLOOR_PREVIEW_Z_INDEX),
            FloorLibraryPreviewOverlayRoot,
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
                    FloorLibraryPreviewCloseButton,
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
                        width: Val::Px(FLOOR_PREVIEW_WIDTH_PX as f32),
                        height: Val::Px(FLOOR_PREVIEW_HEIGHT_PX as f32),
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
                Text::new(info),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(Color::srgba(0.78, 0.78, 0.84, 0.95)),
            ));
        })
        .id();

    state.preview = Some(FloorLibraryPreview {
        floor_id,
        ui_root,
        scene_root: scene.scene_root,
        target: scene.target,
    });
    active_floor.dirty = true;
}

pub(crate) fn floor_library_preview_close_button_interactions(
    mut commands: Commands,
    mut state: ResMut<FloorLibraryUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<FloorLibraryPreviewCloseButton>)>,
) {
    if state.preview.is_none() {
        return;
    }
    for interaction in &mut buttons {
        if *interaction == Interaction::Pressed {
            close_floor_library_preview(&mut commands, &mut state);
            break;
        }
    }
}

pub(crate) fn floor_library_preview_close_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut state: ResMut<FloorLibraryUiState>,
) {
    if state.preview.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        close_floor_library_preview(&mut commands, &mut state);
    }
}

pub(crate) fn floor_library_item_button_interactions(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<FloorLibraryUiState>,
    active: Res<ActiveRealmScene>,
    mut active_floor: ResMut<ActiveWorldFloor>,
    mut buttons: Query<(&Interaction, &FloorLibraryItemButton), Changed<Interaction>>,
) {
    for (interaction, button) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }

        if state.multi_select_mode {
            if button.floor_id == DEFAULT_FLOOR_ID {
                continue;
            }
            state.pending_preview = None;

            let shift_pressed =
                keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
            if shift_pressed {
                let anchor = state.multi_select_anchor_floor_id;
                let clicked = button.floor_id;
                if let Some(anchor) = anchor {
                    let anchor_index = state.listed_floors.iter().position(|id| *id == anchor);
                    let clicked_index = state.listed_floors.iter().position(|id| *id == clicked);
                    if let (Some(a), Some(b)) = (anchor_index, clicked_index) {
                        let (start, end) = if a <= b { (a, b) } else { (b, a) };
                        for idx in start..=end {
                            let floor_id = state.listed_floors[idx];
                            if floor_id != DEFAULT_FLOOR_ID {
                                state.multi_selected_floors.insert(floor_id);
                            }
                        }
                    } else {
                        state.multi_selected_floors.insert(clicked);
                    }
                } else {
                    state.multi_selected_floors.insert(clicked);
                }
            } else if state.multi_selected_floors.contains(&button.floor_id) {
                state.multi_selected_floors.remove(&button.floor_id);
            } else {
                state.multi_selected_floors.insert(button.floor_id);
            }
            state.multi_select_anchor_floor_id = Some(button.floor_id);
            continue;
        }

        state.selected_floor_id = Some(button.floor_id);
        if button.floor_id == DEFAULT_FLOOR_ID {
            set_active_world_floor(&mut active_floor, None, FloorDefV1::default_world());
            if let Err(err) = crate::scene_floor_selection::save_scene_floor_selection(
                &active.realm_id,
                &active.scene_id,
                None,
            ) {
                warn!("{err}");
            }
            state.request_preview(button.floor_id);
            continue;
        }
        match crate::realm_floor_packages::load_realm_floor_def(&active.realm_id, button.floor_id) {
            Ok(def) => {
                set_active_world_floor(&mut active_floor, Some(button.floor_id), def);
                if let Err(err) = crate::scene_floor_selection::save_scene_floor_selection(
                    &active.realm_id,
                    &active.scene_id,
                    Some(button.floor_id),
                ) {
                    warn!("{err}");
                }
                state.request_preview(button.floor_id);
            }
            Err(err) => {
                warn!("Failed to load floor: {err}");
            }
        }
    }
}

pub(crate) fn floor_library_update_list_item_styles(
    state: Res<FloorLibraryUiState>,
    mut last_selected: Local<Option<u128>>,
    mut last_multi_mode: Local<bool>,
    mut last_multi: Local<Vec<u128>>,
    mut buttons: Query<
        (
            Ref<Interaction>,
            &FloorLibraryItemButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<FloorLibraryListItem>,
    >,
    mut marks: Query<(Ref<FloorLibrarySelectionMark>, &mut Visibility)>,
    mut radios: Query<
        (
            Ref<FloorLibraryMultiSelectIndicator>,
            &mut Node,
            &mut BorderColor,
        ),
        Without<FloorLibraryListItem>,
    >,
    mut dots: Query<
        (Ref<FloorLibraryMultiSelectIndicatorDot>, &mut Visibility),
        Without<FloorLibrarySelectionMark>,
    >,
) {
    let selected_id = state.selected_floor_id();
    let mut multi_ids: Vec<u128> = if state.multi_select_mode {
        state.multi_selected_floors.iter().copied().collect()
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
            state.multi_selected_floors.contains(&button.floor_id)
        } else {
            selected_id == Some(button.floor_id)
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
            state.multi_selected_floors.contains(&mark.floor_id)
        } else {
            Some(mark.floor_id) == selected_id
        };
        *vis = if is_selected {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    for (radio, mut node, mut border) in &mut radios {
        if !selection_changed && !radio.is_added() {
            continue;
        }

        let is_default = radio.floor_id == DEFAULT_FLOOR_ID;
        node.display = if state.multi_select_mode && !is_default {
            Display::Flex
        } else {
            Display::None
        };

        let is_selected =
            state.multi_select_mode && state.multi_selected_floors.contains(&radio.floor_id);
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

        let is_default = dot.floor_id == DEFAULT_FLOOR_ID;
        let is_selected = !is_default
            && state.multi_select_mode
            && state.multi_selected_floors.contains(&dot.floor_id);
        *vis = if is_selected {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

pub(crate) fn floor_library_scroll_wheel(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut mouse_wheel: MessageReader<MouseWheel>,
    state: Res<FloorLibraryUiState>,
    roots: Query<(&ComputedNode, &UiGlobalTransform, &Visibility), With<FloorLibraryRoot>>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<FloorLibraryScrollPanel>>,
) {
    if !state.is_open()
        || !matches!(mode.get(), GameMode::Build)
        || !matches!(build_scene.get(), BuildScene::Realm)
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

pub(crate) fn floor_library_scrollbar_drag(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<FloorLibraryUiState>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<FloorLibraryScrollPanel>>,
    tracks: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<FloorLibraryScrollbarTrack>,
    >,
    thumbs: Query<(&Interaction, &ComputedNode, &Node), With<FloorLibraryScrollbarThumb>>,
) {
    let active = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm);
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
            state.scrollbar_drag = Some(FloorLibraryScrollbarDrag { grab_offset });
        }
    }

    let Some(drag) = state.scrollbar_drag.as_ref() else {
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

pub(crate) fn floor_library_update_scrollbar_ui(
    panels: Query<(&ComputedNode, &ScrollPosition), With<FloorLibraryScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<FloorLibraryScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<FloorLibraryScrollbarThumb>>,
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

fn load_png_ui_image(
    images: &mut Assets<Image>,
    path: &std::path::Path,
) -> Result<Handle<Image>, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
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

fn system_time_ms(time: std::time::SystemTime) -> u128 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

impl FloorLibraryUiState {
    pub(crate) fn is_drag_active(&self) -> bool {
        self.scrollbar_drag.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_button_resets_genfloor_session_when_idle() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.init_state::<GameMode>();
        app.init_state::<BuildScene>();

        app.init_resource::<crate::genfloor::GenFloorWorkshop>();
        app.init_resource::<crate::genfloor::GenFloorAiJob>();

        let mut job = app
            .world_mut()
            .resource_mut::<crate::genfloor::GenFloorAiJob>();
        job.set_edit_base_floor_id(Some(123));
        job.set_last_saved_floor_id(Some(123));

        let mut workshop = app
            .world_mut()
            .resource_mut::<crate::genfloor::GenFloorWorkshop>();
        workshop.prompt = "previous".to_string();
        workshop.status = "previous".to_string();
        workshop.error = Some("previous".to_string());
        workshop.draft = Some(crate::genfloor::defs::FloorDefV1::default_world());

        let button = app
            .world_mut()
            .spawn((
                FloorLibraryGenerateButton,
                Interaction::None,
                BackgroundColor(Color::NONE),
                BorderColor::all(Color::NONE),
            ))
            .id();

        app.add_systems(Update, floor_library_generate_button_interactions);

        app.update();

        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        // `NextState` transitions are applied on the next frame (StateTransition runs before Update).
        app.update();

        assert!(matches!(
            app.world().resource::<State<BuildScene>>().get(),
            BuildScene::FloorPreview
        ));

        let job = app.world().resource::<crate::genfloor::GenFloorAiJob>();
        assert!(job.edit_base_floor_id().is_none());
        assert!(job.last_saved_floor_id.is_none());

        let workshop = app.world().resource::<crate::genfloor::GenFloorWorkshop>();
        assert!(workshop.prompt.is_empty());
        assert!(workshop.status.is_empty());
        assert!(workshop.error.is_none());
        assert!(workshop.draft.is_none());
    }

    #[test]
    fn update_visibility_resets_stuck_interactions_on_hide() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.init_state::<GameMode>();
        app.init_state::<BuildScene>();

        let mut state = FloorLibraryUiState::default();
        state.open = true;
        app.insert_resource(state);

        let button = app
            .world_mut()
            .spawn((FloorLibraryGenerateButton, Interaction::Pressed))
            .id();
        let _ = app
            .world_mut()
            .spawn((FloorLibraryRoot, Visibility::Visible))
            .id();

        app.add_systems(Update, floor_library_update_visibility);

        // Visible pass.
        app.update();

        // Switch scene (state transitions apply end-of-frame).
        app.world_mut()
            .resource_mut::<NextState<BuildScene>>()
            .set(BuildScene::FloorPreview);
        app.update();

        // Now hidden: should reset Interaction::Pressed -> Interaction::None.
        app.update();

        let interaction = app.world().get::<Interaction>(button).copied();
        assert_eq!(interaction, Some(Interaction::None));
    }
}
