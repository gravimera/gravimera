use bevy::prelude::*;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};

use crate::object::registry::{ObjectDef, ObjectPartKind};

#[derive(Clone, Debug)]
pub(crate) struct Gen3dStatusLogEntry {
    pub(crate) seq: u32,
    pub(crate) step: String,
    pub(crate) why: String,
    pub(crate) result: String,
    pub(crate) duration_ms: u128,
}

#[derive(Clone, Debug)]
pub(crate) struct Gen3dStatusLogActiveStep {
    pub(crate) seq: u32,
    pub(crate) step: String,
    pub(crate) why: String,
    pub(crate) started_at: std::time::Instant,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Gen3dStatusLog {
    pub(crate) entries: Vec<Gen3dStatusLogEntry>,
    pub(crate) active: Option<Gen3dStatusLogActiveStep>,
    pub(crate) next_seq: u32,
}

impl Gen3dStatusLog {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.active = None;
        self.next_seq = 0;
    }

    pub(crate) fn start_step(&mut self, step: impl Into<String>, why: impl Into<String>) {
        const MAX_ENTRIES: usize = 200;

        if self.active.is_some() {
            self.finish_step("Interrupted.");
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.active = Some(Gen3dStatusLogActiveStep {
            seq,
            step: step.into(),
            why: why.into(),
            started_at: std::time::Instant::now(),
        });
        if self.entries.len() > MAX_ENTRIES {
            let drain = self.entries.len().saturating_sub(MAX_ENTRIES);
            self.entries.drain(0..drain);
        }
    }

    pub(crate) fn finish_step(&mut self, result: impl Into<String>) {
        let Some(active) = self.active.take() else {
            return;
        };
        let now = std::time::Instant::now();
        let duration_ms = now
            .duration_since(active.started_at)
            .as_millis()
            .min(u128::from(u64::MAX));
        self.entries.push(Gen3dStatusLogEntry {
            seq: active.seq,
            step: active.step,
            why: active.why,
            result: result.into(),
            duration_ms,
        });
    }

    pub(crate) fn finish_step_if_active(&mut self, result: impl Into<String>) {
        if self.active.is_none() {
            return;
        }
        self.finish_step(result);
    }

    pub(crate) fn active_elapsed(&self) -> Option<std::time::Duration> {
        self.active
            .as_ref()
            .map(|active| std::time::Instant::now().duration_since(active.started_at))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen3dSpeedMode {
    Level3,
}

impl Default for Gen3dSpeedMode {
    fn default() -> Self {
        Self::Level3
    }
}

impl Gen3dSpeedMode {
    pub(crate) fn wants_component_interaction(self) -> bool {
        true
    }

    pub(crate) fn label(self) -> &'static str {
        "Level 3"
    }

    pub(crate) fn short_label(self) -> &'static str {
        "L3"
    }
}

#[derive(Component)]
pub(crate) struct Gen3dExitButton;

#[derive(Resource, Default)]
pub(crate) struct Gen3dManualTweakState {
    pub(crate) enabled: bool,
    pub(crate) selected_part_id: Option<u128>,
    pub(crate) deform_mode: bool,
    pub(crate) deform_selected_index: Option<usize>,
    pub(crate) undo: Vec<Gen3dManualTweakUndoEntry>,
    pub(crate) redo: Vec<Gen3dManualTweakUndoEntry>,

    pub(crate) color_picker_open: bool,
    pub(crate) color_picker_h: f32,
    pub(crate) color_picker_s: f32,
    pub(crate) color_picker_v: f32,
    pub(crate) color_picker_rgb_text: String,
    pub(crate) color_picker_rgb_focused: bool,
    pub(crate) color_picker_palette_image: Handle<Image>,
    pub(crate) color_picker_value_image: Handle<Image>,
    pub(crate) color_picker_recent_rgba: Vec<[f32; 4]>,
}

#[derive(Clone, Debug)]
pub(crate) struct Gen3dManualTweakUndoEntry {
    pub(crate) label: String,
    pub(crate) undo_args_json: serde_json::Value,
    pub(crate) redo_args_json: serde_json::Value,
}

#[derive(Resource, Default)]
pub(crate) struct Gen3dWorkshop {
    pub(crate) images: Vec<Gen3dImageRef>,
    pub(crate) prompt: String,
    pub(crate) prompt_focused: bool,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) status_log: Gen3dStatusLog,
    pub(crate) image_viewer: Option<usize>,
    pub(crate) speed_mode: Gen3dSpeedMode,
    pub(crate) side_tab: Gen3dSideTab,
    pub(crate) side_panel_open: bool,
    pub(crate) prompt_scrollbar_drag: Option<Gen3dPromptScrollbarDrag>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen3dSideTab {
    Status,
    Prefab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen3dSeedFromPrefabMode {
    EditOverwrite,
    Fork,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Gen3dSeedFromPrefabRequest {
    pub(crate) mode: Gen3dSeedFromPrefabMode,
    pub(crate) prefab_id: u128,
    pub(crate) target_entity: Option<Entity>,
}

#[derive(Resource, Default)]
pub(crate) struct Gen3dPendingSeedFromPrefab {
    pub(crate) request: Option<Gen3dSeedFromPrefabRequest>,
}

impl Default for Gen3dSideTab {
    fn default() -> Self {
        Self::Status
    }
}

#[derive(Resource, Default)]
pub(crate) struct Gen3dPreview {
    pub(crate) target: Option<Handle<Image>>,
    pub(crate) camera: Option<Entity>,
    pub(crate) root: Option<Entity>,
    pub(crate) capture_root: Option<Entity>,
    pub(crate) show_collision: bool,
    pub(crate) collision_dirty: bool,
    pub(crate) ui_applied_session_id: Option<uuid::Uuid>,
    pub(crate) ui_applied_assembly_rev: Option<u32>,
    pub(crate) ui_applied_mark_parts: bool,
    pub(crate) capture_applied_session_id: Option<uuid::Uuid>,
    pub(crate) capture_applied_assembly_rev: Option<u32>,
    pub(crate) draft_focus: Vec3,
    pub(crate) view_pan: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) distance: f32,
    pub(crate) last_cursor: Option<Vec2>,
    pub(crate) animation_channel: String,
    pub(crate) animation_channels: Vec<String>,
    pub(crate) animation_dropdown_open: bool,
    pub(crate) explode_components: bool,
    pub(crate) hovered_component: Option<Gen3dPreviewHoveredComponent>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct Gen3dPreviewHoveredComponent {
    pub(crate) entity: Entity,
    pub(crate) object_id: u128,
    pub(crate) label: String,
}

#[derive(Resource, Default, Clone)]
pub(crate) struct Gen3dDraft {
    pub(crate) defs: Vec<ObjectDef>,
}

impl Gen3dDraft {
    pub(crate) fn root_def(&self) -> Option<&ObjectDef> {
        self.defs
            .iter()
            .find(|def| def.object_id == super::gen3d_draft_object_id())
    }

    pub(crate) fn total_primitive_parts(&self) -> usize {
        self.defs
            .iter()
            .map(|def| {
                def.parts
                    .iter()
                    .filter(|part| matches!(part.kind, ObjectPartKind::Primitive { .. }))
                    .count()
            })
            .sum()
    }

    pub(crate) fn total_non_projectile_primitive_parts(&self) -> usize {
        let projectile_id = super::gen3d_draft_projectile_object_id();
        self.defs
            .iter()
            .filter(|def| def.object_id != projectile_id)
            .map(|def| {
                def.parts
                    .iter()
                    .filter(|part| matches!(part.kind, ObjectPartKind::Primitive { .. }))
                    .count()
            })
            .sum()
    }

    pub(crate) fn component_count(&self) -> usize {
        self.root_def()
            .map(|def| {
                def.parts
                    .iter()
                    .filter(|part| matches!(part.kind, ObjectPartKind::ObjectRef { .. }))
                    .count()
            })
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Gen3dImageRef {
    pub(crate) path: PathBuf,
    pub(crate) ui_image: Handle<Image>,
    pub(crate) aspect_ratio: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Gen3dPromptScrollbarDrag {
    pub(crate) grab_offset: f32,
}

#[derive(Component)]
pub(crate) struct Gen3dWorkshopRoot;

#[derive(Component)]
pub(crate) struct Gen3dImagesList;

#[derive(Component)]
pub(crate) struct Gen3dImagesListItem;

#[derive(Component)]
pub(crate) struct Gen3dThumbnailButton {
    index: usize,
}

impl Gen3dThumbnailButton {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct Gen3dImagesInlinePanel;

#[derive(Component)]
pub(crate) struct Gen3dStatusScrollPanel;

#[derive(Component)]
pub(crate) struct Gen3dStatusScrollbarTrack;

#[derive(Component)]
pub(crate) struct Gen3dStatusScrollbarThumb;

#[derive(Component)]
pub(crate) struct Gen3dClearImagesButton;

#[derive(Component)]
pub(crate) struct Gen3dClearImagesButtonText;

#[derive(Component)]
pub(crate) struct Gen3dPromptBox;

#[derive(Component)]
pub(crate) struct Gen3dPromptScrollPanel;

#[derive(Component)]
pub(crate) struct Gen3dPromptCaret;

#[derive(Component)]
pub(crate) struct Gen3dPromptScrollbarTrack;

#[derive(Component)]
pub(crate) struct Gen3dPromptScrollbarThumb;

#[derive(Component)]
pub(crate) struct Gen3dPromptRichText;

#[derive(Component)]
pub(crate) struct Gen3dPromptHintText;

#[derive(Component)]
pub(crate) struct Gen3dGenerateButton;

#[derive(Component)]
pub(crate) struct Gen3dGenerateButtonText;

#[derive(Component)]
pub(crate) struct Gen3dSaveButton;

#[derive(Component)]
pub(crate) struct Gen3dSaveButtonText;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakButton;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakButtonText;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakSaveButton;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakSaveButtonText;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerRoot;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerPalette;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerPaletteSelector;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerValue;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerValueSelector;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerRgbField;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerRgbFieldText;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerPreviewSwatch;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerApplyButton;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerApplyButtonText;

#[derive(Component)]
pub(crate) struct Gen3dManualTweakColorPickerRecentSwatch {
    index: usize,
}

impl Gen3dManualTweakColorPickerRecentSwatch {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct Gen3dCancelQueueButton;

#[derive(Component)]
pub(crate) struct Gen3dCancelQueueButtonText;

#[derive(Component)]
pub(crate) struct Gen3dStatusText;

#[derive(Component)]
pub(crate) struct Gen3dStatusLogsText;

#[derive(Component)]
pub(crate) struct Gen3dSideTabButton {
    tab: Gen3dSideTab,
}

impl Gen3dSideTabButton {
    pub(crate) fn new(tab: Gen3dSideTab) -> Self {
        Self { tab }
    }

    pub(crate) fn tab(&self) -> Gen3dSideTab {
        self.tab
    }
}

#[derive(Component)]
pub(crate) struct Gen3dSideTabButtonText {
    tab: Gen3dSideTab,
}

impl Gen3dSideTabButtonText {
    pub(crate) fn new(tab: Gen3dSideTab) -> Self {
        Self { tab }
    }

    pub(crate) fn tab(&self) -> Gen3dSideTab {
        self.tab
    }
}

#[derive(Component)]
pub(crate) struct Gen3dStatusPanelRoot;

#[derive(Component)]
pub(crate) struct Gen3dPrefabPanelRoot;

#[derive(Component)]
pub(crate) struct Gen3dSidePanelRoot;

#[derive(Component)]
pub(crate) struct Gen3dSidePanelToggleButton;

#[derive(Component)]
pub(crate) struct Gen3dSidePanelToggleButtonText;

#[derive(Component)]
pub(crate) struct Gen3dPrefabScrollPanel;

#[derive(Component)]
pub(crate) struct Gen3dPrefabScrollbarTrack;

#[derive(Component)]
pub(crate) struct Gen3dPrefabScrollbarThumb;

#[derive(Component)]
pub(crate) struct Gen3dPrefabDetailsText;

#[derive(Component)]
pub(crate) struct Gen3dPreviewCamera;

#[derive(Component)]
pub(crate) struct Gen3dPreviewSceneRoot;

#[derive(Component)]
pub(crate) struct Gen3dPreviewPanel;

#[derive(Component)]
pub(crate) struct Gen3dPreviewPanelImage;

#[derive(Resource)]
pub(crate) struct Gen3dPreviewExportDialogJob {
    pub(crate) receiver: Mutex<Option<mpsc::Receiver<Option<PathBuf>>>>,
}

impl Default for Gen3dPreviewExportDialogJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Component)]
pub(crate) struct Gen3dPreviewOverlayRoot;

#[derive(Component)]
pub(crate) struct Gen3dPreviewStatsText;

#[derive(Component)]
pub(crate) struct Gen3dPreviewHoverFrame;

#[derive(Component)]
pub(crate) struct Gen3dPreviewHoverInfoCard;

#[derive(Component)]
pub(crate) struct Gen3dPreviewHoverInfoText;

#[derive(Component)]
pub(crate) struct Gen3dTweakSelectedFrame;

#[derive(Component)]
pub(crate) struct Gen3dTweakSelectedInfoCard;

#[derive(Component)]
pub(crate) struct Gen3dTweakSelectedInfoText;

#[derive(Component)]
pub(crate) struct Gen3dTweakHelpText;

#[derive(Component)]
pub(crate) struct Gen3dPreviewAnimationDropdownButton;

#[derive(Component)]
pub(crate) struct Gen3dPreviewAnimationDropdownButtonText;

#[derive(Component)]
pub(crate) struct Gen3dPreviewExplodeToggleButton;

#[derive(Component)]
pub(crate) struct Gen3dPreviewExplodeToggleButtonText;

#[derive(Component)]
pub(crate) struct Gen3dPreviewExportButton;

#[derive(Component)]
pub(crate) struct Gen3dPreviewExportButtonText;

#[derive(Component)]
pub(crate) struct Gen3dPreviewAnimationDropdownList;

#[derive(Component)]
pub(crate) struct Gen3dPreviewAnimationOptionButton {
    index: usize,
}

impl Gen3dPreviewAnimationOptionButton {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct Gen3dPreviewAnimationOptionButtonText {
    index: usize,
}

impl Gen3dPreviewAnimationOptionButtonText {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct Gen3dPreviewComponentLabelsRoot;

#[derive(Component)]
pub(crate) struct Gen3dPreviewComponentLabel {
    index: usize,
}

impl Gen3dPreviewComponentLabel {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct Gen3dPreviewComponentLabelText {
    index: usize,
}

impl Gen3dPreviewComponentLabelText {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct Gen3dPreviewLight;

#[derive(Component)]
pub(crate) struct Gen3dPreviewModelRoot;

#[derive(Component)]
pub(crate) struct Gen3dPreviewUiModelRoot;

#[derive(Component)]
pub(crate) struct Gen3dPreviewCollisionRoot;

#[derive(Component)]
pub(crate) struct Gen3dImageViewerRoot;

#[derive(Component)]
pub(crate) struct Gen3dImageViewerImage;

#[derive(Component)]
pub(crate) struct Gen3dImageViewerCaption;

#[derive(Component)]
pub(crate) struct Gen3dThumbnailTooltipRoot;

#[derive(Component)]
pub(crate) struct Gen3dThumbnailTooltipText;

#[derive(Component)]
pub(crate) struct Gen3dReviewOverlayRoot;

#[derive(Component)]
pub(crate) struct Gen3dReviewCaptureCamera;
