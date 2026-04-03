use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GenScenePhase {
    Idle,
    Planning,
    AwaitSceneSwitch,
    GeneratingFloor,
    GeneratingModels,
    Applying,
    Done,
    Failed,
    Canceled,
}

impl Default for GenScenePhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Resource, Default)]
pub(crate) struct GenSceneWorkshop {
    pub(crate) open: bool,
    pub(crate) prompt: String,
    pub(crate) prompt_focused: bool,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) running: bool,
    pub(crate) close_locked: bool,
    pub(crate) run_id: Option<String>,
    pub(crate) active_scene_id: Option<String>,
    pub(crate) prompt_scrollbar_drag: Option<GenScenePromptScrollbarDrag>,
    pub(crate) side_panel_open: bool,
    pub(crate) side_tab: GenSceneSideTab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenSceneSideTab {
    Status,
    Prefab,
}

impl Default for GenSceneSideTab {
    fn default() -> Self {
        Self::Status
    }
}

#[derive(Component)]
pub(crate) struct GenSceneSideTabButton {
    tab: GenSceneSideTab,
}

impl GenSceneSideTabButton {
    pub(crate) fn new(tab: GenSceneSideTab) -> Self {
        Self { tab }
    }

    pub(crate) fn tab(&self) -> GenSceneSideTab {
        self.tab
    }
}

#[derive(Component)]
pub(crate) struct GenSceneSideTabButtonText {
    tab: GenSceneSideTab,
}

impl GenSceneSideTabButtonText {
    pub(crate) fn new(tab: GenSceneSideTab) -> Self {
        Self { tab }
    }

    pub(crate) fn tab(&self) -> GenSceneSideTab {
        self.tab
    }
}

#[derive(Resource, Default)]
pub(crate) struct GenSceneJob {
    pub(crate) phase: GenScenePhase,
    pub(crate) running: bool,
    pub(crate) cancel_requested: bool,
    pub(crate) cancel_flag: Option<Arc<AtomicBool>>,
    pub(crate) run_id: Option<String>,
    pub(crate) run_dir: Option<std::path::PathBuf>,
    pub(crate) target_scene_id: Option<String>,
    pub(crate) plan: Option<GenScenePlanV1>,
    pub(crate) plan_shared: Option<crate::threaded_result::SharedResult<GenScenePlanV1, String>>,
    pub(crate) resolved_prefabs: HashMap<String, u128>,
    pub(crate) model_tasks: Vec<GenSceneModelTask>,
    pub(crate) floor_choice: Option<GenSceneFloorChoice>,
    pub(crate) floor_generation_started: bool,
    pub(crate) floor_generation_prev_id: Option<u128>,
    pub(crate) placements: Vec<GenScenePlacementV1>,
    pub(crate) next_run_step: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct GenSceneModelTask {
    pub(crate) asset_key: String,
    pub(crate) prompt: String,
    pub(crate) session_id: crate::gen3d::Gen3dSessionId,
}

#[derive(Clone, Debug)]
pub(crate) enum GenSceneFloorChoice {
    Default,
    Existing(u128),
    GeneratedPrompt(String),
}

#[derive(Resource, Default)]
pub(crate) struct GenScenePreview {
    pub(crate) target: Option<Handle<Image>>,
    pub(crate) camera: Option<Entity>,
    pub(crate) focus: Vec3,
    pub(crate) half_extents: Vec3,
    pub(crate) dirty: bool,
    pub(crate) active: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GenScenePromptScrollbarDrag {
    pub(crate) grab_offset: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct GenScenePlanV1 {
    pub(crate) version: u32,
    pub(crate) terrain: GenSceneTerrainPlanV1,
    pub(crate) assets: Vec<GenSceneAssetPlanV1>,
    pub(crate) placements: Vec<GenScenePlacementV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct GenSceneTerrainPlanV1 {
    #[serde(default)]
    pub(crate) existing_floor_id: Option<String>,
    #[serde(default)]
    pub(crate) genfloor_prompt: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct GenSceneAssetPlanV1 {
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) existing_prefab_id: Option<String>,
    #[serde(default)]
    pub(crate) gen3d_prompt: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct GenScenePlacementV1 {
    pub(crate) asset_key: String,
    pub(crate) x: f32,
    pub(crate) z: f32,
    pub(crate) yaw_deg: f32,
    #[serde(default)]
    pub(crate) scale: Option<f32>,
    #[serde(default)]
    pub(crate) count: Option<u32>,
}
