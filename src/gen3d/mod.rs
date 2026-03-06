mod agent;
mod ai;
mod images;
mod preview;
mod save;
mod state;
mod status;
mod tool_feedback;
mod tool_feedback_ui;
mod ui;

use crate::object::registry::builtin_object_id;

const GEN3D_MAX_IMAGES: usize = 6;
// Some Gen3D steps include extra internal preview renders in addition to user reference images.
// Keep a slightly higher cap for a single OpenAI request.
const GEN3D_REVIEW_VIEW_IMAGES: usize = 9; // 6 horizontal + 1 top + 2 motion sheets
const GEN3D_MAX_REQUEST_IMAGES: usize = GEN3D_MAX_IMAGES + GEN3D_REVIEW_VIEW_IMAGES;
const GEN3D_MAX_PARTS: usize = 1024;
const GEN3D_MAX_COMPONENTS: usize = 64;
const GEN3D_MAX_CHAT_HISTORY_MESSAGES: usize = 24;
// Long-running Structured Outputs generations (large schemas + high reasoning effort) can exceed a
// few minutes on some providers. Keep this generous so background /responses polling doesn't time
// out prematurely.
const GEN3D_RESPONSES_POLL_MAX_SECS: u64 = 1_800;
const GEN3D_RESPONSES_POLL_INITIAL_DELAY_MS: u64 = 250;
const GEN3D_RESPONSES_POLL_MAX_DELAY_MS: u64 = 2_000;
const GEN3D_PREVIEW_LAYER: usize = 30;
const GEN3D_REVIEW_LAYER: usize = 31;
const GEN3D_PREVIEW_WIDTH_PX: u32 = 960;
const GEN3D_PREVIEW_HEIGHT_PX: u32 = 540;
const GEN3D_REVIEW_CAPTURE_WIDTH_PX: u32 = GEN3D_PREVIEW_WIDTH_PX * 2;
const GEN3D_REVIEW_CAPTURE_HEIGHT_PX: u32 = GEN3D_PREVIEW_HEIGHT_PX * 2;
const GEN3D_DRAFT_OBJECT_KEY: &str = "gravimera/gen3d/draft";
const GEN3D_DRAFT_PROJECTILE_KEY: &str = "gravimera/gen3d/projectile";
const GEN3D_PREVIEW_DEFAULT_YAW: f32 = 0.0;
// Negative pitch means the camera is above the model looking slightly down (front view).
const GEN3D_PREVIEW_DEFAULT_PITCH: f32 = -0.45;
const GEN3D_PREVIEW_DEFAULT_DISTANCE: f32 = 6.0;
const GEN3D_DEFAULT_STYLE_PROMPT: &str =
    "Concise Voxel/Pixel Art style (not necessarily cuboid-only).";

fn gen3d_draft_object_id() -> u128 {
    builtin_object_id(GEN3D_DRAFT_OBJECT_KEY)
}

fn gen3d_draft_projectile_object_id() -> u128 {
    builtin_object_id(GEN3D_DRAFT_PROJECTILE_KEY)
}

pub(crate) use ai::{
    gen3d_apply_pending_seed_from_prefab, gen3d_cancel_build_from_api,
    gen3d_resume_build_from_api, gen3d_start_build_from_api,
    gen3d_start_edit_session_from_prefab_id_from_api,
    gen3d_start_fork_session_from_prefab_id_from_api,
};
pub(crate) use ai::{gen3d_continue_button, gen3d_generate_button, gen3d_poll_ai_job, Gen3dAiJob};
#[allow(unused_imports)]
pub(crate) use ai::{gen3d_generate_prefab_defs_headless, Gen3dHeadlessPrefabResult};
pub(crate) use images::{
    gen3d_clear_images_button, gen3d_handle_drag_and_drop, gen3d_image_viewer_click_to_close,
    gen3d_image_viewer_keyboard_navigation, gen3d_images_scroll_wheel,
    gen3d_rebuild_images_list_ui, gen3d_thumbnail_button_open_viewer,
    gen3d_thumbnail_button_style_on_interaction, gen3d_thumbnail_button_style_on_selection,
    gen3d_update_image_viewer_ui, gen3d_update_images_scrollbar_ui,
    gen3d_update_images_tip_visibility, gen3d_update_thumbnail_tooltip,
};
pub(crate) use preview::{
    gen3d_apply_draft_to_preview, gen3d_preview_orbit_controls,
    gen3d_preview_tick_selected_animation, gen3d_update_collision_overlay,
};
pub(crate) use save::gen3d_save_button;
pub(crate) use save::gen3d_save_current_draft_seed_aware_from_api;
pub(crate) use state::*;
pub(crate) use status::{gen3d_status_scroll_wheel, gen3d_update_status_scrollbar_ui};
pub(crate) use tool_feedback::{gen3d_load_tool_feedback_history, Gen3dToolFeedbackHistory};
pub(crate) use tool_feedback_ui::{
    gen3d_copy_tool_feedback_buttons, gen3d_side_tab_buttons, gen3d_tool_feedback_scroll_wheel,
    gen3d_update_side_tab_ui, gen3d_update_tool_feedback_scrollbar_ui,
    gen3d_update_tool_feedback_text,
};
pub(crate) use ui::{
    enter_gen3d_mode, exit_gen3d_mode, gen3d_cleanup_preview_scene_when_idle,
    gen3d_clear_prompt_button, gen3d_collision_toggle_button,
    gen3d_preview_animation_dropdown_button, gen3d_preview_animation_dropdown_scroll_wheel,
    gen3d_preview_animation_option_buttons, gen3d_prompt_box_focus, gen3d_prompt_scroll_wheel,
    gen3d_prompt_text_input, gen3d_rebuild_preview_animation_dropdown_options_ui,
    gen3d_side_panel_toggle_button, gen3d_update_preview_animation_dropdown_ui,
    gen3d_update_prompt_scrollbar_ui, gen3d_update_side_panel_ui, gen3d_update_ui_text,
    handle_gen3d_toggle_button, update_gen3d_toggle_button_label,
};
