use bevy::prelude::*;
use std::path::PathBuf;

use crate::object::registry::{ObjectDef, ObjectPartKind};

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
pub(crate) struct Gen3dToggleButton;

#[derive(Component)]
pub(crate) struct Gen3dToggleButtonText;

#[derive(Resource, Default)]
pub(crate) struct Gen3dWorkshop {
    pub(crate) images: Vec<Gen3dImageRef>,
    pub(crate) prompt: String,
    pub(crate) prompt_focused: bool,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) image_viewer: Option<usize>,
    pub(crate) speed_mode: Gen3dSpeedMode,
    pub(crate) side_tab: Gen3dSideTab,
    pub(crate) side_panel_open: bool,
    pub(crate) tool_feedback_unread: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen3dSideTab {
    Status,
    ToolFeedback,
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
    pub(crate) show_collision: bool,
    pub(crate) collision_dirty: bool,
    pub(crate) focus: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) distance: f32,
    pub(crate) last_cursor: Option<Vec2>,
    pub(crate) animation_channel: String,
    pub(crate) animation_channels: Vec<String>,
    pub(crate) animation_dropdown_open: bool,
}

#[derive(Resource, Default)]
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
pub(crate) struct Gen3dImagesTipText;

#[derive(Component)]
pub(crate) struct Gen3dImagesScrollPanel;

#[derive(Component)]
pub(crate) struct Gen3dImagesScrollbarTrack;

#[derive(Component)]
pub(crate) struct Gen3dImagesScrollbarThumb;

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
pub(crate) struct Gen3dPromptScrollbarTrack;

#[derive(Component)]
pub(crate) struct Gen3dPromptScrollbarThumb;

#[derive(Component)]
pub(crate) struct Gen3dPromptText;

#[derive(Component)]
pub(crate) struct Gen3dGenerateButton;

#[derive(Component)]
pub(crate) struct Gen3dGenerateButtonText;

#[derive(Component)]
pub(crate) struct Gen3dContinueButton;

#[derive(Component)]
pub(crate) struct Gen3dContinueButtonText;

#[derive(Component)]
pub(crate) struct Gen3dSaveButton;

#[derive(Component)]
pub(crate) struct Gen3dSaveButtonText;

#[derive(Component)]
pub(crate) struct Gen3dStatusText;

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
pub(crate) struct Gen3dToolFeedbackPanelRoot;

#[derive(Component)]
pub(crate) struct Gen3dSidePanelRoot;

#[derive(Component)]
pub(crate) struct Gen3dSidePanelToggleButton;

#[derive(Component)]
pub(crate) struct Gen3dSidePanelToggleButtonText;

#[derive(Component)]
pub(crate) struct Gen3dToolFeedbackScrollPanel;

#[derive(Component)]
pub(crate) struct Gen3dToolFeedbackScrollbarTrack;

#[derive(Component)]
pub(crate) struct Gen3dToolFeedbackScrollbarThumb;

#[derive(Component)]
pub(crate) struct Gen3dToolFeedbackText;

#[derive(Component)]
pub(crate) struct Gen3dCopyFeedbackCodexButton;

#[derive(Component)]
pub(crate) struct Gen3dCopyFeedbackCodexButtonText;

#[derive(Component)]
pub(crate) struct Gen3dCopyFeedbackJsonButton;

#[derive(Component)]
pub(crate) struct Gen3dCopyFeedbackJsonButtonText;

#[derive(Component)]
pub(crate) struct Gen3dClearPromptButton;

#[derive(Component)]
pub(crate) struct Gen3dClearPromptButtonText;

#[derive(Component)]
pub(crate) struct Gen3dPreviewCamera;

#[derive(Component)]
pub(crate) struct Gen3dPreviewSceneRoot;

#[derive(Component)]
pub(crate) struct Gen3dPreviewPanel;

#[derive(Component)]
pub(crate) struct Gen3dPreviewStatsText;

#[derive(Component)]
pub(crate) struct Gen3dCollisionToggleButton;

#[derive(Component)]
pub(crate) struct Gen3dCollisionToggleText;

#[derive(Component)]
pub(crate) struct Gen3dPreviewAnimationDropdownButton;

#[derive(Component)]
pub(crate) struct Gen3dPreviewAnimationDropdownButtonText;

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
pub(crate) struct Gen3dPreviewLight;

#[derive(Component)]
pub(crate) struct Gen3dPreviewModelRoot;

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
