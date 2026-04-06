use bevy::asset::AssetId;
use bevy::camera::primitives::MeshAabb;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::{Ime, PrimaryWindow};

use crate::object::visuals::VisualPartId;
use crate::types::BuildScene;

use super::ai::Gen3dAiJob;
use super::preview;
use super::state::{
    Gen3dDraft, Gen3dManualTweakButton, Gen3dManualTweakButtonText, Gen3dManualTweakState,
    Gen3dManualTweakColorPickerApplyButton, Gen3dManualTweakColorPickerCancelButton,
    Gen3dManualTweakColorPickerPalette,
    Gen3dManualTweakColorPickerPaletteSelector, Gen3dManualTweakColorPickerPreviewSwatch,
    Gen3dManualTweakColorPickerRecentSwatch, Gen3dManualTweakColorPickerRgbField,
    Gen3dManualTweakColorPickerRgbFieldText, Gen3dManualTweakColorPickerRoot,
    Gen3dManualTweakColorPickerValue, Gen3dManualTweakColorPickerValueSelector,
    Gen3dPreviewAnimationDropdownButton, Gen3dPreviewAnimationDropdownList, Gen3dPreviewCamera,
    Gen3dPreviewExplodeToggleButton, Gen3dPreviewExportButton, Gen3dPreviewPanel,
    Gen3dSidePanelRoot, Gen3dSidePanelToggleButton, Gen3dTweakSelectedFrame,
    Gen3dTweakSelectedInfoCard, Gen3dTweakSelectedInfoText, Gen3dWorkshop,
};
use super::task_queue::{Gen3dTaskQueue, Gen3dTaskState};

const MANUAL_TWEAK_UNDO_LIMIT: usize = 64;
const MANUAL_TWEAK_MOVE_STEP_M: f32 = 0.05;
const MANUAL_TWEAK_MOVE_STEP_FAST_M: f32 = 0.20;
const MANUAL_TWEAK_MOVE_STEP_PRECISE_M: f32 = 0.01;
const MANUAL_TWEAK_ROT_STEP_DEG: f32 = 5.0;
const MANUAL_TWEAK_ROT_STEP_FAST_DEG: f32 = 45.0;
const MANUAL_TWEAK_ROT_STEP_PRECISE_DEG: f32 = 1.0;
const MANUAL_TWEAK_SCALE_STEP: f32 = 1.05;
const MANUAL_TWEAK_SCALE_STEP_FAST: f32 = 1.20;
const MANUAL_TWEAK_SCALE_STEP_PRECISE: f32 = 1.01;
const MANUAL_TWEAK_SCALE_MIN: f32 = 0.01;
const MANUAL_TWEAK_SCALE_MAX: f32 = 50.0;

const MANUAL_TWEAK_COLOR_PICKER_RECENT_LIMIT: usize = 12;
const MANUAL_TWEAK_COLOR_PICKER_PALETTE_TEX_SIZE_PX: u32 = 256;
const MANUAL_TWEAK_COLOR_PICKER_VALUE_TEX_WIDTH_PX: u32 = 16;
const MANUAL_TWEAK_COLOR_PICKER_VALUE_TEX_HEIGHT_PX: u32 = 256;
const MANUAL_TWEAK_COLOR_PICKER_UI_PALETTE_SIZE_PX: f32 = 180.0;
const MANUAL_TWEAK_COLOR_PICKER_UI_VALUE_HEIGHT_PX: f32 = 180.0;

fn active_session_is_queued(task_queue: &Gen3dTaskQueue) -> bool {
    task_queue
        .metas
        .get(&task_queue.active_session_id)
        .is_some_and(|meta| meta.task_state == Gen3dTaskState::Waiting)
}

pub(crate) fn gen3d_manual_tweak_button(
    build_scene: Res<State<BuildScene>>,
    task_queue: Res<Gen3dTaskQueue>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    job: Res<Gen3dAiJob>,
    draft: Res<Gen3dDraft>,
    mut buttons: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Visibility,
            &mut Node,
        ),
        With<Gen3dManualTweakButton>,
    >,
    mut texts: Query<&mut Text, With<Gen3dManualTweakButtonText>>,
    mut last_interaction: Local<Option<Interaction>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let queued = active_session_is_queued(&task_queue);
    let has_draft = draft.root_def().is_some() && draft.total_non_projectile_primitive_parts() > 0;
    let available = has_draft && !job.is_running() && !queued;

    if !available && tweak.enabled {
        tweak.enabled = false;
        tweak.selected_part_id = None;
        tweak.deform_mode = false;
        tweak.deform_selected_index = None;
        tweak.color_picker_open = false;
        tweak.color_picker_rgb_focused = false;
    }

    let label = if tweak.enabled {
        "Exit Tweak"
    } else {
        "Manual Tweak"
    };
    for mut text in &mut texts {
        **text = label.into();
    }

    for (interaction, mut bg, mut border, mut vis, mut node) in &mut buttons {
        if !available {
            node.display = Display::None;
            *vis = Visibility::Hidden;
            *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55));
            *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70));
            *last_interaction = None;
            continue;
        }

        node.display = Display::Flex;
        *vis = Visibility::Inherited;

        match *interaction {
            Interaction::Pressed => {
                *bg = if tweak.enabled {
                    BackgroundColor(Color::srgba(0.20, 0.12, 0.26, 0.96))
                } else {
                    BackgroundColor(Color::srgba(0.12, 0.14, 0.26, 0.96))
                };
                *border = if tweak.enabled {
                    BorderColor::all(Color::srgb(0.90, 0.50, 1.00))
                } else {
                    BorderColor::all(Color::srgb(0.55, 0.65, 1.00))
                };

                if !matches!(*last_interaction, Some(Interaction::Pressed)) {
                    tweak.enabled = !tweak.enabled;
                    tweak.selected_part_id = None;
                    tweak.deform_mode = false;
                    tweak.deform_selected_index = None;
                    tweak.color_picker_open = false;
                    tweak.color_picker_rgb_focused = false;
                    workshop.error = None;
                    workshop.status = if tweak.enabled {
                        "Manual tweak enabled. Click a part in the preview to select it.".into()
                    } else {
                        "Manual tweak exited.".into()
                    };
                }
            }
            Interaction::Hovered => {
                *bg = if tweak.enabled {
                    BackgroundColor(Color::srgba(0.18, 0.11, 0.22, 0.90))
                } else {
                    BackgroundColor(Color::srgba(0.10, 0.12, 0.22, 0.90))
                };
                *border = if tweak.enabled {
                    BorderColor::all(Color::srgb(0.85, 0.45, 0.95))
                } else {
                    BorderColor::all(Color::srgb(0.50, 0.60, 0.95))
                };
            }
            Interaction::None => {
                *bg = if tweak.enabled {
                    BackgroundColor(Color::srgba(0.16, 0.09, 0.20, 0.80))
                } else {
                    BackgroundColor(Color::srgba(0.08, 0.10, 0.18, 0.80))
                };
                *border = if tweak.enabled {
                    BorderColor::all(Color::srgb(0.75, 0.40, 0.85))
                } else {
                    BorderColor::all(Color::srgb(0.40, 0.50, 0.85))
                };
            }
        }

        *last_interaction = Some(*interaction);
    }
}

#[derive(SystemParam)]
pub(crate) struct ManualTweakPickUi<'w, 's> {
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    panel_interactions: Query<'w, 's, &'static Interaction, With<Gen3dPreviewPanel>>,
    side_panel_root: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dSidePanelRoot>,
    >,
    side_panel_toggle: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dSidePanelToggleButton>,
    >,
    anim_dropdown_button: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dPreviewAnimationDropdownButton>,
    >,
    explode_toggle_button: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dPreviewExplodeToggleButton>,
    >,
    export_button: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dPreviewExportButton>,
    >,
    anim_dropdown_list: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dPreviewAnimationDropdownList>,
    >,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<Gen3dPreviewCamera>>,
    panels:
        Query<'w, 's, (&'static ComputedNode, &'static UiGlobalTransform), With<Gen3dPreviewPanel>>,
}

#[derive(Default)]
pub(crate) struct ManualTweakClickState {
    pressed_cursor_physical: Option<Vec2>,
    pressed_in_preview: bool,
}

#[derive(Default)]
pub(crate) struct MeshAabbCache {
    aabbs: std::collections::HashMap<AssetId<Mesh>, (Vec3, Vec3)>,
}

fn compute_mesh_local_aabb(mesh: &Mesh) -> Option<(Vec3, Vec3)> {
    let aabb = mesh.compute_aabb()?;
    let min: Vec3 = aabb.min().into();
    let max: Vec3 = aabb.max().into();
    if !min.is_finite() || !max.is_finite() {
        return None;
    }
    Some((min, max))
}

fn mesh_local_aabb_cached(
    cache: &mut MeshAabbCache,
    meshes: &Assets<Mesh>,
    handle: &Handle<Mesh>,
) -> Option<(Vec3, Vec3)> {
    let id = handle.id();
    if let Some(value) = cache.aabbs.get(&id) {
        return Some(*value);
    }
    let mesh = meshes.get(handle)?;
    let aabb = compute_mesh_local_aabb(mesh)?;
    cache.aabbs.insert(id, aabb);
    Some(aabb)
}

fn ray_intersects_aabb(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;

    for axis in 0..3 {
        let o = origin[axis];
        let d = direction[axis];
        let min = min[axis];
        let max = max[axis];

        if d.abs() < 1e-6 {
            if o < min || o > max {
                return None;
            }
            continue;
        }

        let inv_d = 1.0 / d;
        let mut t0 = (min - o) * inv_d;
        let mut t1 = (max - o) * inv_d;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        t_min = t_min.max(t0);
        t_max = t_max.min(t1);
        if t_max < t_min {
            return None;
        }
    }

    if t_max < 0.0 {
        None
    } else {
        Some(t_min.max(0.0))
    }
}

fn node_visible(vis: Option<&Visibility>) -> bool {
    vis.map(|v| !matches!(*v, Visibility::Hidden))
        .unwrap_or(true)
}

fn preview_cursor_unblocked(
    window: &Window,
    panel_interactions: &Query<&Interaction, With<Gen3dPreviewPanel>>,
    side_panel_root: &Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dSidePanelRoot>,
    >,
    side_panel_toggle: &Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dSidePanelToggleButton>,
    >,
    anim_dropdown_button: &Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dPreviewAnimationDropdownButton>,
    >,
    explode_toggle_button: &Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dPreviewExplodeToggleButton>,
    >,
    export_button: &Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dPreviewExportButton>,
    >,
    anim_dropdown_list: &Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dPreviewAnimationDropdownList>,
    >,
) -> bool {
    let mut hovered = panel_interactions
        .iter()
        .any(|i| matches!(*i, Interaction::Hovered | Interaction::Pressed));
    if !hovered {
        return false;
    }
    let Some(cursor) = window.physical_cursor_position() else {
        return false;
    };

    if let Ok((node, transform, vis)) = side_panel_root.single() {
        if node_visible(vis) && node.contains_point(*transform, cursor) {
            hovered = false;
        }
    }
    if hovered {
        if let Ok((node, transform, vis)) = side_panel_toggle.single() {
            if node_visible(vis) && node.contains_point(*transform, cursor) {
                hovered = false;
            }
        }
    }
    if hovered {
        if let Ok((node, transform, vis)) = anim_dropdown_button.single() {
            if node_visible(vis) && node.contains_point(*transform, cursor) {
                hovered = false;
            }
        }
    }
    if hovered {
        if let Ok((node, transform, vis)) = explode_toggle_button.single() {
            if node_visible(vis) && node.contains_point(*transform, cursor) {
                hovered = false;
            }
        }
    }
    if hovered {
        if let Ok((node, transform, vis)) = export_button.single() {
            if node_visible(vis) && node.contains_point(*transform, cursor) {
                hovered = false;
            }
        }
    }
    if hovered {
        if let Ok((node, transform, vis)) = anim_dropdown_list.single() {
            if node_visible(vis) && node.contains_point(*transform, cursor) {
                hovered = false;
            }
        }
    }

    hovered
}

pub(crate) fn gen3d_manual_tweak_pick_part(
    build_scene: Res<State<BuildScene>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    ui: ManualTweakPickUi,
    color_picker_root: Query<(&ComputedNode, &UiGlobalTransform, &Visibility), With<Gen3dManualTweakColorPickerRoot>>,
    parts: Query<(&GlobalTransform, &VisualPartId, &Mesh3d)>,
    meshes: Res<Assets<Mesh>>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut click_state: Local<ManualTweakClickState>,
    mut mesh_cache: Local<MeshAabbCache>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        click_state.pressed_cursor_physical = None;
        click_state.pressed_in_preview = false;
        return;
    }
    if !tweak.enabled {
        click_state.pressed_cursor_physical = None;
        click_state.pressed_in_preview = false;
        return;
    }

    let Ok(window) = ui.windows.single() else {
        return;
    };
    let hovered = preview_cursor_unblocked(
        window,
        &ui.panel_interactions,
        &ui.side_panel_root,
        &ui.side_panel_toggle,
        &ui.anim_dropdown_button,
        &ui.explode_toggle_button,
        &ui.export_button,
        &ui.anim_dropdown_list,
    );

    if mouse_buttons.just_pressed(MouseButton::Left) {
        let mut in_color_picker = false;
        if tweak.color_picker_open {
            if let Ok((node, transform, vis)) = color_picker_root.single() {
                if let Some(cursor) = window.physical_cursor_position() {
                    if !matches!(*vis, Visibility::Hidden)
                        && node.contains_point(*transform, cursor)
                    {
                        in_color_picker = true;
                    }
                }
            }
        }
        click_state.pressed_cursor_physical = window.physical_cursor_position();
        click_state.pressed_in_preview = hovered && !in_color_picker;
    }

    if !mouse_buttons.just_released(MouseButton::Left) {
        return;
    }

    let pressed_in_preview = click_state.pressed_in_preview;
    let start = click_state.pressed_cursor_physical;
    click_state.pressed_cursor_physical = None;
    click_state.pressed_in_preview = false;

    if !pressed_in_preview {
        return;
    }
    if !hovered {
        return;
    }

    let Some(end) = window.physical_cursor_position() else {
        return;
    };
    let Some(start) = start else {
        return;
    };
    if (end - start).length_squared() > 16.0 {
        // Treat as drag (camera orbit), not a click selection.
        return;
    }

    let Some((panel_node, panel_transform)) = ui.panels.single().ok() else {
        return;
    };
    let Some(layout) = preview::preview_image_layout(panel_node, *panel_transform) else {
        return;
    };
    let Some(cursor_target) = preview::preview_cursor_to_target(end, layout.image_bounds_physical)
    else {
        return;
    };

    let Ok((camera, camera_transform)) = ui.cameras.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_target) else {
        return;
    };

    let mut best: Option<(u128, f32)> = None;
    for (global_transform, part_id, mesh3d) in parts.iter() {
        let Some((min, max)) = mesh_local_aabb_cached(&mut mesh_cache, &meshes, &mesh3d.0) else {
            continue;
        };

        let inverse = global_transform.to_matrix().inverse();
        let origin_local = inverse.transform_point3(ray.origin);
        let direction_local = inverse.transform_vector3(ray.direction.into());

        let Some(t) = ray_intersects_aabb(origin_local, direction_local, min, max) else {
            continue;
        };

        match best {
            None => best = Some((part_id.0, t)),
            Some((_, best_t)) if t < best_t => best = Some((part_id.0, t)),
            _ => {}
        }
    }

    let picked = best.map(|(id, _)| id);
    if picked != tweak.selected_part_id {
        tweak.selected_part_id = picked;
        tweak.deform_selected_index = None;
        workshop.error = None;
        workshop.status = if let Some(id) = picked {
            format!("Selected part {}.", uuid::Uuid::from_u128(id))
        } else {
            "Selection cleared.".to_string()
        };
    }
}

fn tweak_mod_shift(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)
}

fn tweak_mod_precision(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight)
}

fn tweak_mod_cmd(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight)
}

fn clamp_scale(v: Vec3) -> Vec3 {
    let clamp_component = |value: f32| {
        if !value.is_finite() {
            return 1.0;
        }
        let sign = if value == 0.0 { 1.0 } else { value.signum() };
        let mag = value
            .abs()
            .clamp(MANUAL_TWEAK_SCALE_MIN, MANUAL_TWEAK_SCALE_MAX);
        sign * mag
    };
    Vec3::new(
        clamp_component(v.x),
        clamp_component(v.y),
        clamp_component(v.z),
    )
}

fn mesh_key_to_draft_ops_name(mesh: crate::object::registry::MeshKey) -> Option<&'static str> {
    use crate::object::registry::MeshKey;
    Some(match mesh {
        MeshKey::UnitCube => "cube",
        MeshKey::UnitCylinder => "cylinder",
        MeshKey::UnitCone => "cone",
        MeshKey::UnitSphere => "sphere",
        MeshKey::UnitCapsule => "capsule",
        MeshKey::UnitConicalFrustum => "conical_frustum",
        MeshKey::UnitTorus => "torus",
        _ => return None,
    })
}

fn primitive_params_to_draft_ops_json(
    params: &crate::object::registry::PrimitiveParams,
) -> serde_json::Value {
    use crate::object::registry::PrimitiveParams;
    match params {
        PrimitiveParams::Capsule {
            radius,
            half_length,
        } => serde_json::json!({
            "kind": "capsule",
            "radius": *radius,
            "half_length": *half_length,
        }),
        PrimitiveParams::ConicalFrustum {
            radius_top,
            radius_bottom,
            height,
        } => serde_json::json!({
            "kind": "conical_frustum",
            "top_radius": *radius_top,
            "bottom_radius": *radius_bottom,
            "height": *height,
        }),
        PrimitiveParams::Torus {
            minor_radius,
            major_radius,
        } => serde_json::json!({
            "kind": "torus",
            "minor_radius": *minor_radius,
            "major_radius": *major_radius,
        }),
    }
}

fn primitive_deform_to_draft_ops_json(
    deform: &crate::object::registry::PrimitiveDeformDef,
) -> serde_json::Value {
    use crate::object::registry::PrimitiveDeformDef;

    match deform {
        PrimitiveDeformDef::FfdV1(ffd) => serde_json::json!({
            "kind": "ffd_v1",
            "grid": ffd.grid,
            "offsets": ffd.offsets.iter().map(|v| [v.x, v.y, v.z]).collect::<Vec<_>>(),
        }),
    }
}

fn color_to_rgba(color: Color) -> [f32; 4] {
    let c = color.to_srgba();
    [c.red, c.green, c.blue, c.alpha]
}

fn rgb_u8_to_color(r: u8, g: u8, b: u8) -> Color {
    Color::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

fn format_rgb_text(color: Color) -> String {
    let c = color.to_srgba();
    let r = (c.red.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (c.green.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (c.blue.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn srgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let r = r.clamp(0.0, 1.0);
    let g = g.clamp(0.0, 1.0);
    let b = b.clamp(0.0, 1.0);

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let v = max;
    if delta <= 1e-6 || max <= 1e-6 {
        return (0.0, 0.0, v);
    }

    let s = if max <= 1e-6 { 0.0 } else { delta / max };

    let mut h = if (max - r).abs() <= 1e-6 {
        (g - b) / delta
    } else if (max - g).abs() <= 1e-6 {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };
    h /= 6.0;
    if h < 0.0 {
        h += 1.0;
    }
    (h.clamp(0.0, 1.0), s.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

fn hsv_to_srgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(1.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    if s <= 1e-6 {
        return (v, v, v);
    }

    let hf = h * 6.0;
    let i = hf.floor() as i32;
    let f = hf - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));

    match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

fn parse_rgb_text(text: &str) -> Option<(u8, u8, u8)> {
    let raw = text.trim();
    if raw.is_empty() {
        return None;
    }

    let hex = raw.strip_prefix('#').unwrap_or(raw).trim();
    let hex = hex.strip_prefix("0x").unwrap_or(hex).trim();
    if hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                return Some((r, g, b));
            }
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
                return Some(((r << 4) | r, (g << 4) | g, (b << 4) | b));
            }
            _ => {}
        }
    }
    None
}

fn color_picker_current_color(tweak: &Gen3dManualTweakState) -> Color {
    let (r, g, b) = hsv_to_srgb(tweak.color_picker_h, tweak.color_picker_s, tweak.color_picker_v);
    Color::srgb(r, g, b)
}

fn color_picker_set_from_color(tweak: &mut Gen3dManualTweakState, color: Color) {
    let rgba = color_to_rgba(color);
    let (h, s, v) = srgb_to_hsv(rgba[0], rgba[1], rgba[2]);
    tweak.color_picker_h = h;
    tweak.color_picker_s = s;
    tweak.color_picker_v = v;
    tweak.color_picker_rgb_text = format_rgb_text(color);
}

fn build_set_transform_json(transform: Transform) -> serde_json::Value {
    let q = transform.rotation.normalize();
    serde_json::json!({
        "pos": [transform.translation.x, transform.translation.y, transform.translation.z],
        "scale": [transform.scale.x, transform.scale.y, transform.scale.z],
        "rot_quat_xyzw": [q.x, q.y, q.z, q.w],
    })
}

fn build_set_primitive_json(
    primitive: &crate::object::registry::PrimitiveVisualDef,
    color: Color,
) -> Result<serde_json::Value, String> {
    use crate::object::registry::PrimitiveVisualDef;

    match primitive {
        PrimitiveVisualDef::Primitive {
            mesh,
            params,
            unlit,
            deform,
            ..
        } => {
            let mesh_name = mesh_key_to_draft_ops_name(*mesh).ok_or_else(|| {
                format!(
                    "Unsupported primitive mesh for manual recolor: {mesh:?} (not editable via DraftOps)"
                )
            })?;
            let mut value = serde_json::json!({
                "mesh": mesh_name,
                "color_rgba": color_to_rgba(color),
                "unlit": *unlit,
            });
            if let Some(params) = params.as_ref() {
                value
                    .as_object_mut()
                    .expect("json object")
                    .insert("params".into(), primitive_params_to_draft_ops_json(params));
            }
            if let Some(deform) = deform.as_ref() {
                value
                    .as_object_mut()
                    .expect("json object")
                    .insert("deform".into(), primitive_deform_to_draft_ops_json(deform));
            }
            Ok(value)
        }
        PrimitiveVisualDef::Mesh { mesh, .. } => Err(format!(
            "Manual recolor does not support mesh/material primitives yet ({mesh:?})."
        )),
    }
}

fn build_update_primitive_part_args(
    component: &str,
    part_id: u128,
    set_transform: Option<Transform>,
    set_primitive: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut op = serde_json::json!({
        "kind": "update_primitive_part",
        "component": component,
        "part_id_uuid": uuid::Uuid::from_u128(part_id).to_string(),
    });
    if let Some(transform) = set_transform {
        op.as_object_mut()
            .expect("json object")
            .insert("set_transform".into(), build_set_transform_json(transform));
    }
    if let Some(primitive) = set_primitive {
        op.as_object_mut()
            .expect("json object")
            .insert("set_primitive".into(), primitive);
    }

    serde_json::json!({
        "version": 1,
        "atomic": true,
        "ops": [op],
    })
}

fn patch_apply_draft_ops_args(
    mut args: serde_json::Value,
    if_assembly_rev: u32,
) -> serde_json::Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    obj.insert("version".into(), serde_json::json!(1));
    obj.insert("atomic".into(), serde_json::json!(true));
    obj.insert("if_assembly_rev".into(), serde_json::json!(if_assembly_rev));
    args
}

fn push_undo_entry(
    tweak: &mut Gen3dManualTweakState,
    entry: super::state::Gen3dManualTweakUndoEntry,
) {
    if tweak.undo.len() >= MANUAL_TWEAK_UNDO_LIMIT {
        let drain = tweak.undo.len().saturating_sub(MANUAL_TWEAK_UNDO_LIMIT - 1);
        tweak.undo.drain(0..drain);
    }
    tweak.undo.push(entry);
}

pub(crate) fn find_selected_primitive_part(
    draft: &Gen3dDraft,
    part_id: u128,
) -> Option<(
    String,
    Transform,
    crate::object::registry::PrimitiveVisualDef,
)> {
    for def in &draft.defs {
        let component = def.label.trim().to_string();
        for part in &def.parts {
            if part.part_id != Some(part_id) {
                continue;
            }
            let crate::object::registry::ObjectPartKind::Primitive { primitive } = &part.kind
            else {
                continue;
            };
            return Some((component, part.transform, primitive.clone()));
        }
    }
    None
}

pub(crate) fn gen3d_ffd_pick_control_point_index_for_primitive(
    primitive: &crate::object::registry::PrimitiveVisualDef,
    ray_origin_local: Vec3,
    ray_dir_local: Vec3,
) -> Option<usize> {
    let Some((base_min, base_max)) = primitive_base_aabb_for_ffd(primitive) else {
        return None;
    };
    let Some((grid, offsets)) = primitive_ffd_grid_and_offsets(primitive) else {
        return None;
    };

    let base_size = (base_max - base_min).abs().max(Vec3::splat(0.01));
    let radius_local = (base_size.length() * 0.04).clamp(0.015, 0.10);
    pick_control_point_index(
        ray_origin_local,
        ray_dir_local,
        base_min,
        base_max,
        grid,
        offsets.as_slice(),
        radius_local,
    )
}

pub(crate) fn gen3d_manual_tweak_hotkeys(
    build_scene: Res<State<BuildScene>>,
    keys: Res<ButtonInput<KeyCode>>,
    task_queue: Res<Gen3dTaskQueue>,
    ui: ManualTweakPickUi,
    mut job: ResMut<Gen3dAiJob>,
    mut draft: ResMut<Gen3dDraft>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut tweak: ResMut<Gen3dManualTweakState>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    if !tweak.enabled {
        return;
    }
    if workshop.image_viewer.is_some() {
        return;
    }
    if workshop.prompt_focused {
        return;
    }
    if job.is_running() || active_session_is_queued(&task_queue) {
        return;
    }

    let Ok(window) = ui.windows.single() else {
        return;
    };
    let hovered = preview_cursor_unblocked(
        window,
        &ui.panel_interactions,
        &ui.side_panel_root,
        &ui.side_panel_toggle,
        &ui.anim_dropdown_button,
        &ui.explode_toggle_button,
        &ui.export_button,
        &ui.anim_dropdown_list,
    );
    if !hovered {
        return;
    }

    let modifier_cmd = tweak_mod_cmd(&keys);
    let modifier_shift = tweak_mod_shift(&keys);
    let modifier_precision = tweak_mod_precision(&keys);

    if modifier_cmd {
        let redo_requested = (keys.just_pressed(KeyCode::KeyZ) && modifier_shift)
            || keys.just_pressed(KeyCode::KeyY);
        let undo_requested = keys.just_pressed(KeyCode::KeyZ) && !modifier_shift;
        if undo_requested {
            let Some(entry) = tweak.undo.pop() else {
                return;
            };
            let args = patch_apply_draft_ops_args(entry.undo_args_json.clone(), job.assembly_rev());
            match super::gen3d_apply_draft_ops_from_api(&mut job, &mut draft, args) {
                Ok(_) => {
                    workshop.error = None;
                    workshop.status = format!("Undo: {}", entry.label);
                    tweak.redo.push(entry);
                }
                Err(err) => {
                    workshop.error = Some(err);
                    tweak.undo.push(entry);
                }
            }
            return;
        }
        if redo_requested {
            let Some(entry) = tweak.redo.pop() else {
                return;
            };
            let args = patch_apply_draft_ops_args(entry.redo_args_json.clone(), job.assembly_rev());
            match super::gen3d_apply_draft_ops_from_api(&mut job, &mut draft, args) {
                Ok(_) => {
                    workshop.error = None;
                    workshop.status = format!("Redo: {}", entry.label);
                    push_undo_entry(&mut tweak, entry);
                }
                Err(err) => {
                    workshop.error = Some(err);
                    tweak.redo.push(entry);
                }
            }
            return;
        }

    }

    if tweak.color_picker_open {
        return;
    }

    if keys.just_pressed(KeyCode::KeyV) {
        tweak.deform_mode = !tweak.deform_mode;
        tweak.deform_selected_index = None;
        workshop.error = None;
        workshop.status = if tweak.deform_mode {
            "Sculpt (FFD) enabled. Drag a control point in the preview (Shift=big, Ctrl=precision)."
                .into()
        } else {
            "Sculpt (FFD) exited.".into()
        };
        return;
    }

    if keys.just_pressed(KeyCode::KeyC) {
        let color = if let Some(recent) = tweak.color_picker_recent_rgba.first().copied() {
            Color::srgba(recent[0], recent[1], recent[2], recent[3])
        } else if let Some(part_id) = tweak.selected_part_id {
            find_selected_primitive_part(&draft, part_id)
                .map(|(_component, _before, primitive)| match &primitive {
                    crate::object::registry::PrimitiveVisualDef::Primitive { color, .. } => *color,
                    _ => Color::srgb(0.85, 0.87, 0.90),
                })
                .unwrap_or(Color::srgb(1.0, 1.0, 1.0))
        } else {
            Color::srgb(1.0, 1.0, 1.0)
        };

        color_picker_set_from_color(&mut tweak, color);
        tweak.color_picker_rgb_focused = false;
        tweak.color_picker_open = true;
        workshop.error = None;
        workshop.status = if tweak.selected_part_id.is_some() {
            "Color picker opened.".into()
        } else {
            "Color picker opened. Select a part to apply.".into()
        };
        return;
    }

    let Some(part_id) = tweak.selected_part_id else {
        return;
    };

    let Some((component, before_transform, _primitive)) =
        find_selected_primitive_part(&draft, part_id)
    else {
        tweak.selected_part_id = None;
        workshop.error = Some("Selected part no longer exists in the draft.".into());
        return;
    };

    let mut requested_move = Vec3::ZERO;
    if keys.just_pressed(KeyCode::ArrowLeft) {
        requested_move.x -= 1.0;
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        requested_move.x += 1.0;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        requested_move.z += 1.0;
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        requested_move.z -= 1.0;
    }
    if keys.just_pressed(KeyCode::PageUp) {
        requested_move.y += 1.0;
    }
    if keys.just_pressed(KeyCode::PageDown) {
        requested_move.y -= 1.0;
    }

    let mut requested_rot_deg: f32 = 0.0;
    if keys.just_pressed(KeyCode::Comma) {
        requested_rot_deg -= 1.0;
    }
    if keys.just_pressed(KeyCode::Period) {
        requested_rot_deg += 1.0;
    }

    let mut requested_scale: f32 = 1.0;
    let scale_step = if modifier_precision {
        MANUAL_TWEAK_SCALE_STEP_PRECISE
    } else if modifier_shift {
        MANUAL_TWEAK_SCALE_STEP_FAST
    } else {
        MANUAL_TWEAK_SCALE_STEP
    };
    if keys.just_pressed(KeyCode::Minus) {
        requested_scale *= 1.0 / scale_step;
    }
    if keys.just_pressed(KeyCode::Equal) {
        requested_scale *= scale_step;
    }

    if !requested_move.is_finite() || !requested_rot_deg.is_finite() || !requested_scale.is_finite()
    {
        return;
    }

    let mut set_transform = None;
    if requested_move.length_squared() > 1e-6
        || requested_rot_deg.abs() > 1e-6
        || (requested_scale - 1.0).abs() > 1e-6
    {
        let step = if modifier_precision {
            MANUAL_TWEAK_MOVE_STEP_PRECISE_M
        } else if modifier_shift {
            MANUAL_TWEAK_MOVE_STEP_FAST_M
        } else {
            MANUAL_TWEAK_MOVE_STEP_M
        };
        let delta = requested_move * step;

        let rot_step_deg = if modifier_precision {
            MANUAL_TWEAK_ROT_STEP_PRECISE_DEG
        } else if modifier_shift {
            MANUAL_TWEAK_ROT_STEP_FAST_DEG
        } else {
            MANUAL_TWEAK_ROT_STEP_DEG
        };
        let delta_rot = Quat::from_rotation_y((requested_rot_deg * rot_step_deg).to_radians());

        let mut next = before_transform;
        next.translation += delta;
        next.rotation = (next.rotation * delta_rot).normalize();
        next.scale = clamp_scale(next.scale * requested_scale);

        if next.translation.is_finite() && next.rotation.is_finite() && next.scale.is_finite() {
            set_transform = Some(next);
        }
    }

    if set_transform.is_none() {
        return;
    }

    let before_args = build_update_primitive_part_args(
        component.as_str(),
        part_id,
        set_transform.map(|_| before_transform),
        None,
    );

    let after_transform = set_transform.unwrap_or(before_transform);
    let after_args = build_update_primitive_part_args(
        component.as_str(),
        part_id,
        Some(after_transform),
        None,
    );

    let label = if requested_rot_deg.abs() > 1e-6 {
        "Rotate".to_string()
    } else if (requested_scale - 1.0).abs() > 1e-6 {
        "Scale".to_string()
    } else {
        "Move".to_string()
    };

    let apply_args = patch_apply_draft_ops_args(after_args.clone(), job.assembly_rev());
    match super::gen3d_apply_draft_ops_from_api(&mut job, &mut draft, apply_args) {
        Ok(_) => {
            workshop.error = None;
            workshop.status = format!("Tweak: {label}");
            push_undo_entry(
                &mut tweak,
                super::state::Gen3dManualTweakUndoEntry {
                    label,
                    undo_args_json: before_args,
                    redo_args_json: after_args,
                },
            );
            tweak.redo.clear();
        }
        Err(err) => {
            workshop.error = Some(err);
        }
    }
}

pub(crate) fn gen3d_manual_tweak_update_selected_overlay(
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Gen3dPreviewCamera>>,
    panels: Query<(&ComputedNode, &UiGlobalTransform), With<Gen3dPreviewPanel>>,
    parts: Query<(&GlobalTransform, &VisualPartId, &Mesh3d)>,
    meshes: Res<Assets<Mesh>>,
    draft: Res<Gen3dDraft>,
    tweak: Res<Gen3dManualTweakState>,
    mut overlay_nodes: Query<
        (
            Option<&Gen3dTweakSelectedFrame>,
            Option<&Gen3dTweakSelectedInfoCard>,
            &mut Node,
            &mut Visibility,
        ),
        Or<(
            With<Gen3dTweakSelectedFrame>,
            With<Gen3dTweakSelectedInfoCard>,
        )>,
    >,
    mut overlay_texts: Query<(&Gen3dTweakSelectedInfoText, &mut Text)>,
    mut mesh_cache: Local<MeshAabbCache>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let hide_all =
        |overlay_nodes: &mut Query<
            (
                Option<&Gen3dTweakSelectedFrame>,
                Option<&Gen3dTweakSelectedInfoCard>,
                &mut Node,
                &mut Visibility,
            ),
            Or<(
                With<Gen3dTweakSelectedFrame>,
                With<Gen3dTweakSelectedInfoCard>,
            )>,
        >,
         overlay_texts: &mut Query<(&Gen3dTweakSelectedInfoText, &mut Text)>| {
            for (_frame, _card, mut node, mut vis) in overlay_nodes.iter_mut() {
                node.display = Display::None;
                *vis = Visibility::Hidden;
            }
            for (_marker, mut text) in overlay_texts.iter_mut() {
                **text = "".into();
            }
        };

    if !tweak.enabled {
        hide_all(&mut overlay_nodes, &mut overlay_texts);
        return;
    }

    let context = windows
        .single()
        .ok()
        .zip(cameras.single().ok())
        .zip(panels.single().ok())
        .and_then(
            |((window, (camera, camera_transform)), (panel_node, panel_transform))| {
                preview::preview_image_layout(panel_node, *panel_transform)
                    .map(|layout| (window, camera, camera_transform, layout))
            },
        );
    let Some((_window, camera, camera_transform, layout)) = context else {
        hide_all(&mut overlay_nodes, &mut overlay_texts);
        return;
    };

    let mut frame_rect_panel: Option<Rect> = None;

    let selection = tweak.selected_part_id;
    let info_text = if let Some(part_id) = selection {
        let part = parts.iter().find(|(_, id, _)| id.0 == part_id).and_then(
            |(global_transform, _id, mesh3d)| {
                mesh_local_aabb_cached(&mut mesh_cache, &meshes, &mesh3d.0)
                    .map(|aabb| (global_transform.to_matrix(), aabb.0, aabb.1))
            },
        );
        if let Some((world_from_local, min_local, max_local)) = part {
            let corners = [
                Vec3::new(min_local.x, min_local.y, min_local.z),
                Vec3::new(min_local.x, min_local.y, max_local.z),
                Vec3::new(min_local.x, max_local.y, min_local.z),
                Vec3::new(min_local.x, max_local.y, max_local.z),
                Vec3::new(max_local.x, min_local.y, min_local.z),
                Vec3::new(max_local.x, min_local.y, max_local.z),
                Vec3::new(max_local.x, max_local.y, min_local.z),
                Vec3::new(max_local.x, max_local.y, max_local.z),
            ];

            let mut min = Vec2::splat(f32::INFINITY);
            let mut max = Vec2::splat(f32::NEG_INFINITY);
            let mut any = false;
            for corner in corners {
                let world = world_from_local.transform_point3(corner);
                let Ok(viewport) = camera.world_to_viewport(camera_transform, world) else {
                    continue;
                };
                min = min.min(viewport);
                max = max.max(viewport);
                any = true;
            }

            if any && min.is_finite() && max.is_finite() {
                let frame_min =
                    preview::preview_target_to_panel_logical(min, layout).max(Vec2::ZERO);
                let frame_max = preview::preview_target_to_panel_logical(max, layout)
                    .min(layout.panel_size_logical);
                frame_rect_panel = Some(Rect {
                    min: frame_min,
                    max: frame_max,
                });
            }
        }

        let mut component_label = None;
        let mut primitive_label = None;
        let mut transform = None;
        if let Some((def, part)) = draft.defs.iter().find_map(|def| {
            def.parts.iter().find_map(|part| {
                if part.part_id == Some(part_id) {
                    Some((def, part))
                } else {
                    None
                }
            })
        }) {
            component_label = Some(def.label.to_string());
            transform = Some(part.transform);
            if let crate::object::registry::ObjectPartKind::Primitive { primitive } = &part.kind {
                primitive_label = match primitive {
                    crate::object::registry::PrimitiveVisualDef::Primitive { mesh, .. } => {
                        Some(format!("{mesh:?}"))
                    }
                    crate::object::registry::PrimitiveVisualDef::Mesh { mesh, .. } => {
                        Some(format!("{mesh:?}"))
                    }
                };
            }
        }
        let display_component = component_label.as_deref().filter(|v| !v.trim().is_empty());

        let mut info_text = format!(
            "Manual Tweak\nSelected: {}\nPart: {}\n\nMove: arrows | PgUp/PgDn (Shift=big, Ctrl=precision)\nRotate: ,/. (Shift=45°, Ctrl=precision)\nScale: -/= (Shift=big, Ctrl=precision)\nRecolor: C (open picker)\nUndo/Redo: Ctrl/Cmd+Z/Y",
            display_component.unwrap_or("unknown"),
            primitive_label.unwrap_or_else(|| "primitive".to_string()),
        );
        info_text.push_str("\nSculpt (FFD): V (toggle), LMB drag handle (Shift=big, Ctrl=precision)");
        if tweak.deform_mode {
            info_text.push_str("\nSculpt: ON (LMB drag empty space to orbit)");
        }
        if let Some(index) = tweak.deform_selected_index {
            info_text.push_str(&format!("\nControl point: {index}"));
        }
        if let Some(transform) = transform {
            info_text = format!(
                "{info_text}\n\npos: {:.2} {:.2} {:.2}\nscale: {:.2} {:.2} {:.2}",
                transform.translation.x,
                transform.translation.y,
                transform.translation.z,
                transform.scale.x,
                transform.scale.y,
                transform.scale.z,
            );
        }
        info_text
    } else {
        "Manual Tweak\nClick a part in the preview to select it.\n\nMove: arrows | PgUp/PgDn (Shift=big, Ctrl=precision)\nRotate: ,/. (Shift=45°, Ctrl=precision)\nScale: -/= (Shift=big, Ctrl=precision)\nRecolor: C (open picker)\nUndo/Redo: Ctrl/Cmd+Z/Y\n\nEsc: exit tweak".to_string()
    };

    for (frame_marker, card_marker, mut node, mut vis) in &mut overlay_nodes {
        if frame_marker.is_some() {
            let Some(rect) = frame_rect_panel else {
                node.display = Display::None;
                *vis = Visibility::Hidden;
                continue;
            };
            let frame_min = rect.min.max(Vec2::ZERO);
            let frame_max = rect.max.min(layout.panel_size_logical);
            let frame_size = (frame_max - frame_min).max(Vec2::splat(2.0));
            node.left = Val::Px(frame_min.x);
            node.top = Val::Px(frame_min.y);
            node.width = Val::Px(frame_size.x);
            node.height = Val::Px(frame_size.y);
            node.display = Display::Flex;
            *vis = Visibility::Visible;
            continue;
        }

        if card_marker.is_some() {
            if let Some(rect) = frame_rect_panel {
                let card_left =
                    (rect.max.x + 10.0).clamp(8.0, (layout.panel_size_logical.x - 240.0).max(8.0));
                let card_top = rect
                    .min
                    .y
                    .clamp(8.0, (layout.panel_size_logical.y - 140.0).max(8.0));
                node.left = Val::Px(card_left);
                node.top = Val::Px(card_top);
            } else {
                node.left = Val::Px(8.0);
                node.top = Val::Px(8.0);
            }
            node.display = Display::Flex;
            *vis = Visibility::Visible;
        }
    }

    for (_marker, mut text) in &mut overlay_texts {
        **text = info_text.clone().into();
    }
}

const MANUAL_TWEAK_FFD_DEFAULT_GRID: [u8; 3] = [3, 3, 3];

#[derive(Component, Copy, Clone, Debug)]
pub(crate) struct Gen3dManualTweakFfdHandle {
    part_id: u128,
    index: usize,
}

#[derive(Default)]
pub(crate) struct ManualTweakFfdHandleAssets {
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
    material_selected: Option<Handle<StandardMaterial>>,
}

#[derive(Default)]
pub(crate) struct ManualTweakFfdDragState {
    active: bool,
    component: String,
    part_id: u128,
    cp_index: usize,
    grid: [u8; 3],
    base_min: Vec3,
    base_max: Vec3,
    base_offsets: Vec<Vec3>,
    start_hit_world: Vec3,
    plane_normal_world: Vec3,
    primitive_base: Option<crate::object::registry::PrimitiveVisualDef>,
    before_args_json: serde_json::Value,
    last_apply_time_secs: f32,
}

impl ManualTweakFfdDragState {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

fn primitive_base_size_for_ffd(
    mesh: crate::object::registry::MeshKey,
    params: Option<&crate::object::registry::PrimitiveParams>,
) -> Vec3 {
    use crate::object::registry::{MeshKey, PrimitiveParams};

    match mesh {
        MeshKey::UnitCube => Vec3::ONE,
        MeshKey::UnitCylinder => Vec3::ONE,
        MeshKey::UnitCone => Vec3::ONE,
        MeshKey::UnitSphere => Vec3::ONE,
        MeshKey::UnitCapsule => match params {
            Some(PrimitiveParams::Capsule {
                half_length,
                radius,
            }) => Vec3::new(radius * 2.0, (half_length + radius) * 2.0, radius * 2.0),
            _ => Vec3::ONE,
        },
        MeshKey::UnitConicalFrustum => match params {
            Some(PrimitiveParams::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            }) => {
                let r = radius_top.max(*radius_bottom);
                Vec3::new(r * 2.0, *height, r * 2.0)
            }
            _ => Vec3::ONE,
        },
        MeshKey::UnitTorus => match params {
            Some(PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            }) => {
                let r = major_radius + minor_radius;
                Vec3::new(r * 2.0, minor_radius * 2.0, r * 2.0)
            }
            _ => Vec3::ONE,
        },
        _ => Vec3::ONE,
    }
}

fn primitive_base_aabb_for_ffd(
    primitive: &crate::object::registry::PrimitiveVisualDef,
) -> Option<(Vec3, Vec3)> {
    use crate::object::registry::PrimitiveVisualDef;

    let PrimitiveVisualDef::Primitive { mesh, params, .. } = primitive else {
        return None;
    };
    let size = primitive_base_size_for_ffd(*mesh, params.as_ref())
        .abs()
        .max(Vec3::splat(0.01));
    let half = size * 0.5;
    Some((-half, half))
}

fn ffd_point_count(grid: [u8; 3]) -> Option<usize> {
    let nx = grid[0] as usize;
    let ny = grid[1] as usize;
    let nz = grid[2] as usize;
    if nx < 2 || ny < 2 || nz < 2 {
        return None;
    }
    Some(nx.saturating_mul(ny).saturating_mul(nz))
}

fn primitive_ffd_grid_and_offsets(
    primitive: &crate::object::registry::PrimitiveVisualDef,
) -> Option<([u8; 3], Vec<Vec3>)> {
    use crate::object::registry::{PrimitiveDeformDef, PrimitiveVisualDef};

    let PrimitiveVisualDef::Primitive { deform, .. } = primitive else {
        return None;
    };

    match deform {
        None => {
            let grid = MANUAL_TWEAK_FFD_DEFAULT_GRID;
            let count = ffd_point_count(grid)?;
            Some((grid, vec![Vec3::ZERO; count]))
        }
        Some(PrimitiveDeformDef::FfdV1(ffd)) => Some((ffd.grid, ffd.offsets.clone())),
    }
}

fn primitive_with_ffd_offsets(
    primitive: &crate::object::registry::PrimitiveVisualDef,
    grid: [u8; 3],
    offsets: Vec<Vec3>,
) -> Option<crate::object::registry::PrimitiveVisualDef> {
    use crate::object::registry::{PrimitiveDeformDef, PrimitiveFfdDeformV1, PrimitiveVisualDef};

    let PrimitiveVisualDef::Primitive {
        mesh,
        params,
        color,
        unlit,
        ..
    } = primitive
    else {
        return None;
    };
    Some(PrimitiveVisualDef::Primitive {
        mesh: *mesh,
        params: params.clone(),
        color: *color,
        unlit: *unlit,
        deform: Some(PrimitiveDeformDef::FfdV1(PrimitiveFfdDeformV1 {
            grid,
            offsets,
        })),
    })
}

fn ffd_control_point_local(
    base_min: Vec3,
    base_max: Vec3,
    grid: [u8; 3],
    offsets: &[Vec3],
    index: usize,
) -> Option<Vec3> {
    let nx = grid[0] as usize;
    let ny = grid[1] as usize;
    let nz = grid[2] as usize;
    if nx < 2 || ny < 2 || nz < 2 {
        return None;
    }
    let expected = nx.saturating_mul(ny).saturating_mul(nz);
    if index >= expected || offsets.len() != expected {
        return None;
    }

    let x = index % nx;
    let yz = index / nx;
    let y = yz % ny;
    let z = yz / ny;

    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let tx = x as f32 / (nx - 1) as f32;
    let ty = y as f32 / (ny - 1) as f32;
    let tz = z as f32 / (nz - 1) as f32;

    let base = Vec3::new(
        lerp(base_min.x, base_max.x, tx),
        lerp(base_min.y, base_max.y, ty),
        lerp(base_min.z, base_max.z, tz),
    );
    Some(base + offsets[index])
}

fn ray_plane_intersection(origin: Vec3, dir: Vec3, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = dir.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (point - origin).dot(normal) / denom;
    if !t.is_finite() {
        return None;
    }
    Some(origin + dir * t)
}

fn pick_control_point_index(
    ray_origin_local: Vec3,
    ray_dir_local: Vec3,
    base_min: Vec3,
    base_max: Vec3,
    grid: [u8; 3],
    offsets: &[Vec3],
    radius_local: f32,
) -> Option<usize> {
    let dir = ray_dir_local.normalize_or_zero();
    if dir.length_squared() < 1e-8 {
        return None;
    }

    let count = ffd_point_count(grid)?;
    if offsets.len() != count {
        return None;
    }

    let mut best: Option<(usize, f32)> = None;
    let radius2 = radius_local.max(0.001).powi(2);
    for index in 0..count {
        let Some(p) = ffd_control_point_local(base_min, base_max, grid, offsets, index) else {
            continue;
        };
        let v = p - ray_origin_local;
        let t = v.dot(dir);
        if t < 0.0 {
            continue;
        }
        let closest = ray_origin_local + dir * t;
        let dist2 = (p - closest).length_squared();
        if dist2 > radius2 {
            continue;
        }
        match best {
            None => best = Some((index, t)),
            Some((_, best_t)) if t < best_t => best = Some((index, t)),
            _ => {}
        }
    }
    best.map(|(index, _)| index)
}

pub(crate) fn gen3d_manual_tweak_ffd_drag(
    build_scene: Res<State<BuildScene>>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    task_queue: Res<Gen3dTaskQueue>,
    ui: ManualTweakPickUi,
    parts: Query<(&GlobalTransform, &VisualPartId)>,
    mut job: ResMut<Gen3dAiJob>,
    mut draft: ResMut<Gen3dDraft>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut drag: Local<ManualTweakFfdDragState>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        drag.reset();
        return;
    }
    if !tweak.enabled || !tweak.deform_mode {
        drag.reset();
        return;
    }
    if tweak.color_picker_open {
        drag.reset();
        return;
    }
    if workshop.image_viewer.is_some() || workshop.prompt_focused {
        drag.reset();
        return;
    }
    if job.is_running() || active_session_is_queued(&task_queue) {
        drag.reset();
        return;
    }

    let Ok(window) = ui.windows.single() else {
        return;
    };
    let hovered = preview_cursor_unblocked(
        window,
        &ui.panel_interactions,
        &ui.side_panel_root,
        &ui.side_panel_toggle,
        &ui.anim_dropdown_button,
        &ui.explode_toggle_button,
        &ui.export_button,
        &ui.anim_dropdown_list,
    );
    if !hovered && !drag.active {
        return;
    }

    let Some(cursor_physical) = window.physical_cursor_position() else {
        return;
    };

    let Some(part_id) = tweak.selected_part_id else {
        drag.reset();
        return;
    };

    let Some((component, _before_transform, primitive)) =
        find_selected_primitive_part(&draft, part_id)
    else {
        drag.reset();
        return;
    };

    let Some((base_min, base_max)) = primitive_base_aabb_for_ffd(&primitive) else {
        return;
    };
    let Some((grid, offsets)) = primitive_ffd_grid_and_offsets(&primitive) else {
        return;
    };

    let part_transform = parts
        .iter()
        .find(|(_t, id)| id.0 == part_id)
        .map(|(t, _id)| t.to_matrix());
    let Some(part_from_local) = part_transform else {
        return;
    };
    let inv_local_from_part = part_from_local.inverse();

    let Some((panel_node, panel_transform)) = ui.panels.single().ok() else {
        return;
    };
    let Some(layout) = preview::preview_image_layout(panel_node, *panel_transform) else {
        return;
    };
    let Some(cursor_target) =
        preview::preview_cursor_to_target(cursor_physical, layout.image_bounds_physical)
    else {
        return;
    };
    let Ok((camera, camera_transform)) = ui.cameras.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_target) else {
        return;
    };

    let ray_origin_world = ray.origin;
    let ray_dir_world: Vec3 = ray.direction.into();

    let ray_origin_local = inv_local_from_part.transform_point3(ray.origin);
    let ray_dir_local = inv_local_from_part.transform_vector3(ray.direction.into());

    let base_size = (base_max - base_min).abs().max(Vec3::splat(0.01));
    let radius_local = (base_size.length() * 0.04).clamp(0.015, 0.10);

    if mouse_buttons.just_pressed(MouseButton::Left) && !drag.active {
        let picked = pick_control_point_index(
            ray_origin_local,
            ray_dir_local,
            base_min,
            base_max,
            grid,
            offsets.as_slice(),
            radius_local,
        );
        if let Some(cp_index) = picked {
            let Some(cp_local) =
                ffd_control_point_local(base_min, base_max, grid, offsets.as_slice(), cp_index)
            else {
                return;
            };

            let cp_world = part_from_local.transform_point3(cp_local);

            let plane_normal_world: Vec3 = camera_transform.forward().into();
            let plane_normal_world = plane_normal_world.normalize_or_zero();
            let plane_normal_world = if plane_normal_world.length_squared() > 1e-8 {
                plane_normal_world
            } else {
                Vec3::Z
            };

            let primitive_base = primitive.clone();
            let base_color = match &primitive_base {
                crate::object::registry::PrimitiveVisualDef::Primitive { color, .. } => *color,
                _ => Color::srgb(0.85, 0.87, 0.90),
            };
            let before_prim_json = match build_set_primitive_json(&primitive_base, base_color) {
                Ok(value) => value,
                Err(err) => {
                    workshop.error = Some(err);
                    return;
                }
            };
            let before_args_json = build_update_primitive_part_args(
                component.as_str(),
                part_id,
                None,
                Some(before_prim_json),
            );

            drag.active = true;
            drag.component = component;
            drag.part_id = part_id;
            drag.cp_index = cp_index;
            drag.grid = grid;
            drag.base_min = base_min;
            drag.base_max = base_max;
            drag.base_offsets = offsets;
            drag.start_hit_world = cp_world;
            drag.plane_normal_world = plane_normal_world;
            drag.primitive_base = Some(primitive_base);
            drag.before_args_json = before_args_json;
            drag.last_apply_time_secs = time.elapsed_secs();

            tweak.deform_selected_index = Some(cp_index);
            workshop.error = None;
            workshop.status = format!("Sculpt: selected control point {cp_index}.");
        }
    }

    if !drag.active {
        return;
    }

    if mouse_buttons.pressed(MouseButton::Left) {
        let now = time.elapsed_secs();
        let apply_due = (now - drag.last_apply_time_secs) >= 0.02;
        if apply_due {
            let current_hit_world = ray_plane_intersection(
                ray_origin_world,
                ray_dir_world,
                drag.start_hit_world,
                drag.plane_normal_world,
            )
            .unwrap_or(drag.start_hit_world);

            let mut delta_world = current_hit_world - drag.start_hit_world;
            if tweak_mod_precision(&keys) {
                delta_world *= 0.25;
            } else if tweak_mod_shift(&keys) {
                delta_world *= 4.0;
            }
            let delta = inv_local_from_part.transform_vector3(delta_world);
            if !delta.is_finite() {
                return;
            }

            let mut offsets = drag.base_offsets.clone();
            if let Some(existing) = offsets.get_mut(drag.cp_index) {
                *existing += delta;
            }

            let Some(primitive_base) = drag.primitive_base.as_ref() else {
                return;
            };
            let Some(after_primitive) =
                primitive_with_ffd_offsets(primitive_base, drag.grid, offsets)
            else {
                return;
            };
            let base_color = match &after_primitive {
                crate::object::registry::PrimitiveVisualDef::Primitive { color, .. } => *color,
                _ => Color::srgb(0.85, 0.87, 0.90),
            };
            let after_prim_json = match build_set_primitive_json(&after_primitive, base_color) {
                Ok(value) => value,
                Err(err) => {
                    workshop.error = Some(err);
                    return;
                }
            };
            let after_args = build_update_primitive_part_args(
                drag.component.as_str(),
                drag.part_id,
                None,
                Some(after_prim_json),
            );

            let apply_args = patch_apply_draft_ops_args(after_args, job.assembly_rev());
            match super::gen3d_apply_draft_ops_from_api(&mut job, &mut draft, apply_args) {
                Ok(_) => {
                    workshop.error = None;
                    drag.last_apply_time_secs = now;
                }
                Err(err) => {
                    workshop.error = Some(err);
                }
            }
        }
    }

    if mouse_buttons.just_released(MouseButton::Left) {
        let current_hit_world = ray_plane_intersection(
            ray_origin_world,
            ray_dir_world,
            drag.start_hit_world,
            drag.plane_normal_world,
        )
        .unwrap_or(drag.start_hit_world);

        let mut delta_world = current_hit_world - drag.start_hit_world;
        if tweak_mod_precision(&keys) {
            delta_world *= 0.25;
        } else if tweak_mod_shift(&keys) {
            delta_world *= 4.0;
        }
        let delta = inv_local_from_part.transform_vector3(delta_world);

        let mut offsets = drag.base_offsets.clone();
        if let Some(existing) = offsets.get_mut(drag.cp_index) {
            *existing += delta;
        }

        let changed = offsets != drag.base_offsets;
        if changed {
            let Some(primitive_base) = drag.primitive_base.as_ref() else {
                drag.reset();
                return;
            };
            let Some(after_primitive) =
                primitive_with_ffd_offsets(primitive_base, drag.grid, offsets)
            else {
                drag.reset();
                return;
            };
            let base_color = match &after_primitive {
                crate::object::registry::PrimitiveVisualDef::Primitive { color, .. } => *color,
                _ => Color::srgb(0.85, 0.87, 0.90),
            };
            let after_prim_json = match build_set_primitive_json(&after_primitive, base_color) {
                Ok(value) => value,
                Err(err) => {
                    workshop.error = Some(err);
                    drag.reset();
                    return;
                }
            };
            let after_args = build_update_primitive_part_args(
                drag.component.as_str(),
                drag.part_id,
                None,
                Some(after_prim_json),
            );

            let apply_args = patch_apply_draft_ops_args(after_args.clone(), job.assembly_rev());
            match super::gen3d_apply_draft_ops_from_api(&mut job, &mut draft, apply_args) {
                Ok(_) => {
                    workshop.error = None;
                    workshop.status = "Tweak: Deform (FFD)".into();
                    push_undo_entry(
                        &mut tweak,
                        super::state::Gen3dManualTweakUndoEntry {
                            label: "Deform (FFD)".into(),
                            undo_args_json: drag.before_args_json.clone(),
                            redo_args_json: after_args,
                        },
                    );
                    tweak.redo.clear();
                }
                Err(err) => {
                    workshop.error = Some(err);
                }
            }
        }

        drag.reset();
    }
}

pub(crate) fn gen3d_manual_tweak_update_ffd_handles(
    mut commands: Commands,
    build_scene: Res<State<BuildScene>>,
    tweak: Res<Gen3dManualTweakState>,
    draft: Res<Gen3dDraft>,
    parts: Query<(Entity, &VisualPartId)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut handle_assets: Local<ManualTweakFfdHandleAssets>,
    mut handles: Query<
        (
            Entity,
            &Gen3dManualTweakFfdHandle,
            &mut Transform,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        With<Gen3dManualTweakFfdHandle>,
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let show = tweak.enabled && tweak.deform_mode && tweak.selected_part_id.is_some();
    if !show {
        for (entity, _handle, _t, _mat) in handles.iter_mut() {
            commands.entity(entity).try_despawn();
        }
        return;
    }

    let Some(part_id) = tweak.selected_part_id else {
        return;
    };
    let Some((part_entity, _)) = parts.iter().find(|(_, id)| id.0 == part_id) else {
        return;
    };
    let Some((_component, _transform, primitive)) = find_selected_primitive_part(&draft, part_id)
    else {
        return;
    };

    let Some((base_min, base_max)) = primitive_base_aabb_for_ffd(&primitive) else {
        return;
    };
    let Some((grid, offsets)) = primitive_ffd_grid_and_offsets(&primitive) else {
        return;
    };
    let Some(count) = ffd_point_count(grid) else {
        return;
    };

    if handle_assets.mesh.is_none() {
        handle_assets.mesh = Some(meshes.add(Sphere::new(0.030)));
    }
    if handle_assets.material.is_none() {
        handle_assets.material = Some(materials.add(StandardMaterial {
            base_color: Color::srgb(0.25, 0.85, 0.95),
            emissive: LinearRgba::rgb(0.10, 0.20, 0.24),
            metallic: 0.0,
            perceptual_roughness: 0.2,
            ..default()
        }));
    }
    if handle_assets.material_selected.is_none() {
        handle_assets.material_selected = Some(materials.add(StandardMaterial {
            base_color: Color::srgb(1.00, 0.85, 0.20),
            emissive: LinearRgba::rgb(0.20, 0.14, 0.02),
            metallic: 0.0,
            perceptual_roughness: 0.15,
            ..default()
        }));
    }

    let mesh = handle_assets.mesh.as_ref().expect("mesh handle");
    let material = handle_assets.material.as_ref().expect("material handle");
    let material_selected = handle_assets
        .material_selected
        .as_ref()
        .expect("selected material handle");

    for (entity, handle, _t, _mat) in handles.iter_mut() {
        if handle.part_id != part_id {
            commands.entity(entity).try_despawn();
        }
    }

    let mut existing: std::collections::HashMap<usize, Entity> = std::collections::HashMap::new();
    for (entity, handle, _t, _mat) in handles.iter() {
        if handle.part_id == part_id {
            existing.insert(handle.index, entity);
        }
    }

    for index in 0..count {
        let Some(pos) =
            ffd_control_point_local(base_min, base_max, grid, offsets.as_slice(), index)
        else {
            continue;
        };
        let selected = tweak.deform_selected_index == Some(index);
        let mat = if selected {
            material_selected.clone()
        } else {
            material.clone()
        };

        if let Some(entity) = existing.get(&index).copied() {
            if let Ok((_e, _handle, mut t, mut m)) = handles.get_mut(entity) {
                t.translation = pos;
                *m = MeshMaterial3d(mat);
            }
            continue;
        }

        let entity = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(pos),
                Visibility::Inherited,
                bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_UI_LAYER),
                Gen3dManualTweakFfdHandle { part_id, index },
            ))
            .id();
        commands.entity(part_entity).add_child(entity);
    }
}

#[derive(Default)]
pub(crate) struct ManualTweakColorPickerDragState {
    palette: bool,
    value: bool,
}

fn manual_tweak_push_recent_color(tweak: &mut Gen3dManualTweakState, color: Color) {
    let rgba = color_to_rgba(color);
    let same_rgb = |a: &[f32; 4], b: &[f32; 4]| {
        (a[0] - b[0]).abs() <= 1.0 / 255.0
            && (a[1] - b[1]).abs() <= 1.0 / 255.0
            && (a[2] - b[2]).abs() <= 1.0 / 255.0
            && (a[3] - b[3]).abs() <= 1.0 / 255.0
    };

    tweak.color_picker_recent_rgba.retain(|v| !same_rgb(v, &rgba));
    tweak.color_picker_recent_rgba.insert(0, rgba);
    if tweak.color_picker_recent_rgba.len() > MANUAL_TWEAK_COLOR_PICKER_RECENT_LIMIT {
        tweak.color_picker_recent_rgba
            .truncate(MANUAL_TWEAK_COLOR_PICKER_RECENT_LIMIT);
    }
}

pub(crate) fn gen3d_manual_tweak_color_picker_rgb_field_focus(
    build_scene: Res<State<BuildScene>>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut fields: Query<&Interaction, (Changed<Interaction>, With<Gen3dManualTweakColorPickerRgbField>)>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    if !tweak.enabled || !tweak.color_picker_open {
        return;
    }

    for interaction in &mut fields {
        if *interaction == Interaction::Pressed {
            tweak.color_picker_rgb_focused = true;
        }
    }
}

pub(crate) fn gen3d_manual_tweak_color_picker_rgb_defocus_on_click_outside(
    build_scene: Res<State<BuildScene>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    fields: Query<(&ComputedNode, &UiGlobalTransform), With<Gen3dManualTweakColorPickerRgbField>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    if !tweak.color_picker_open || !tweak.color_picker_rgb_focused {
        return;
    }
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.physical_cursor_position() else {
        return;
    };
    let Ok((node, transform)) = fields.single() else {
        return;
    };
    if node.contains_point(*transform, cursor) {
        return;
    }

    tweak.color_picker_rgb_focused = false;
}

pub(crate) fn gen3d_manual_tweak_color_picker_rgb_text_input(
    build_scene: Res<State<BuildScene>>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: bevy::ecs::message::MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut ime_events: bevy::ecs::message::MessageReader<Ime>,
) {
    let accept_input =
        matches!(build_scene.get(), BuildScene::Preview) && tweak.color_picker_rgb_focused;
    let accept_char = |ch: char| ch == '#' || ch == 'x' || ch == 'X' || ch.is_ascii_hexdigit();

    for event in ime_events.read() {
        let Ime::Commit { value, .. } = event else {
            continue;
        };
        if !accept_input || value.is_empty() {
            continue;
        }
        for ch in value.chars() {
            if ch.is_control() || ch == '\n' {
                continue;
            }
            if accept_char(ch) {
                if tweak.color_picker_rgb_text.len() < 32 {
                    tweak.color_picker_rgb_text.push(ch);
                }
            }
        }
    }

    let mut changed = false;
    for event in keyboard.read() {
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }
        if !accept_input {
            continue;
        }

        match event.key_code {
            KeyCode::Backspace => {
                changed |= tweak.color_picker_rgb_text.pop().is_some();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                tweak.color_picker_rgb_focused = false;
            }
            KeyCode::Escape => {
                tweak.color_picker_rgb_focused = false;
            }
            KeyCode::KeyV => {
                let modifier = keys.pressed(KeyCode::ControlLeft)
                    || keys.pressed(KeyCode::ControlRight)
                    || keys.pressed(KeyCode::SuperLeft)
                    || keys.pressed(KeyCode::SuperRight);
                if modifier {
                    if let Some(text) = crate::clipboard::read_text() {
                        for ch in text.chars() {
                            if ch.is_control() || ch == '\n' {
                                continue;
                            }
                            if accept_char(ch) {
                                if tweak.color_picker_rgb_text.len() >= 32 {
                                    break;
                                }
                                tweak.color_picker_rgb_text.push(ch);
                                changed = true;
                            }
                        }
                    }
                    continue;
                }

                if let Some(text) = &event.text {
                    for ch in text.chars() {
                        if ch.is_control() || ch == '\n' {
                            continue;
                        }
                        if accept_char(ch) {
                            if tweak.color_picker_rgb_text.len() < 32 {
                                tweak.color_picker_rgb_text.push(ch);
                                changed = true;
                            }
                        }
                    }
                }
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                for ch in text.chars() {
                    if ch.is_control() || ch == '\n' {
                        continue;
                    }
                    if accept_char(ch) {
                        if tweak.color_picker_rgb_text.len() < 32 {
                            tweak.color_picker_rgb_text.push(ch);
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    if changed {
        if let Some((r, g, b)) = parse_rgb_text(&tweak.color_picker_rgb_text) {
            let color = rgb_u8_to_color(r, g, b);
            let rgba = color_to_rgba(color);
            let (h, s, v) = srgb_to_hsv(rgba[0], rgba[1], rgba[2]);
            tweak.color_picker_h = h;
            tweak.color_picker_s = s;
            tweak.color_picker_v = v;
        }
    }
}

pub(crate) fn gen3d_manual_tweak_color_picker_drag(
    build_scene: Res<State<BuildScene>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    palettes: Query<(&ComputedNode, &UiGlobalTransform), With<Gen3dManualTweakColorPickerPalette>>,
    values: Query<(&ComputedNode, &UiGlobalTransform), With<Gen3dManualTweakColorPickerValue>>,
    mut drag: Local<ManualTweakColorPickerDragState>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        drag.palette = false;
        drag.value = false;
        return;
    }
    if !tweak.enabled || !tweak.color_picker_open {
        drag.palette = false;
        drag.value = false;
        return;
    }

    if !mouse_buttons.pressed(MouseButton::Left) {
        drag.palette = false;
        drag.value = false;
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.physical_cursor_position() else {
        return;
    };

    let mouse_just_pressed = mouse_buttons.just_pressed(MouseButton::Left);
    if mouse_just_pressed {
        drag.palette = false;
        drag.value = false;

        if let Ok((node, transform)) = palettes.single() {
            if node.contains_point(*transform, cursor) {
                drag.palette = true;
            }
        }
        if !drag.palette {
            if let Ok((node, transform)) = values.single() {
                if node.contains_point(*transform, cursor) {
                    drag.value = true;
                }
            }
        }
    }

    if drag.palette {
        let Ok((node, transform)) = palettes.single() else {
            return;
        };
        let Some(local) = transform
            .try_inverse()
            .map(|t| t.transform_point2(cursor))
        else {
            return;
        };
        let scale = node.inverse_scale_factor();
        let w = node.size.x.max(1.0) * scale;
        let h = node.size.y.max(1.0) * scale;
        let x = (local.x + node.size.x * 0.5) * scale;
        let y = (local.y + node.size.y * 0.5) * scale;
        let u = (x / w).clamp(0.0, 1.0);
        let v = (y / h).clamp(0.0, 1.0);
        tweak.color_picker_h = u;
        tweak.color_picker_s = (1.0 - v).clamp(0.0, 1.0);
    } else if drag.value {
        let Ok((node, transform)) = values.single() else {
            return;
        };
        let Some(local) = transform
            .try_inverse()
            .map(|t| t.transform_point2(cursor))
        else {
            return;
        };
        let scale = node.inverse_scale_factor();
        let h = node.size.y.max(1.0) * scale;
        let y = (local.y + node.size.y * 0.5) * scale;
        let t = (y / h).clamp(0.0, 1.0);
        tweak.color_picker_v = (1.0 - t).clamp(0.0, 1.0);
    }
}

pub(crate) fn gen3d_manual_tweak_color_picker_recent_swatches(
    build_scene: Res<State<BuildScene>>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut buttons: Query<
        (&Interaction, &Gen3dManualTweakColorPickerRecentSwatch),
        (Changed<Interaction>, With<Button>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    if !tweak.enabled || !tweak.color_picker_open {
        return;
    }

    for (interaction, swatch) in &mut buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let index = swatch.index();
        let Some(rgba) = tweak.color_picker_recent_rgba.get(index).copied() else {
            continue;
        };
        let color = Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3]);
        color_picker_set_from_color(&mut tweak, color);
        tweak.color_picker_rgb_focused = false;
    }
}

pub(crate) fn gen3d_manual_tweak_color_picker_apply_button(
    build_scene: Res<State<BuildScene>>,
    task_queue: Res<Gen3dTaskQueue>,
    mut job: ResMut<Gen3dAiJob>,
    mut draft: ResMut<Gen3dDraft>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut last_interaction: Local<Option<Interaction>>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor, &mut BorderColor), With<Gen3dManualTweakColorPickerApplyButton>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        *last_interaction = None;
        return;
    }
    if !tweak.enabled || !tweak.color_picker_open {
        *last_interaction = None;
        return;
    }
    if job.is_running() || active_session_is_queued(&task_queue) {
        *last_interaction = None;
        return;
    }

    let Ok((interaction, mut bg, mut border)) = buttons.single_mut() else {
        *last_interaction = None;
        return;
    };

    let enabled = tweak.selected_part_id.is_some();
    if !enabled {
        *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55));
        *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70));
        *last_interaction = Some(*interaction);
        return;
    }

    match *interaction {
        Interaction::None => {
            *bg = BackgroundColor(Color::srgba(0.08, 0.14, 0.10, 0.85));
            *border = BorderColor::all(Color::srgb(0.25, 0.80, 0.45));
        }
        Interaction::Hovered => {
            *bg = BackgroundColor(Color::srgba(0.10, 0.18, 0.12, 0.92));
            *border = BorderColor::all(Color::srgb(0.30, 0.90, 0.52));
        }
        Interaction::Pressed => {
            *bg = BackgroundColor(Color::srgba(0.12, 0.22, 0.14, 0.96));
            *border = BorderColor::all(Color::srgb(0.35, 1.00, 0.60));

            let was_pressed = matches!(*last_interaction, Some(Interaction::Pressed));
            if was_pressed {
                return;
            }

            let Some(part_id) = tweak.selected_part_id else {
                workshop.status = "Select a part before applying color.".into();
                *last_interaction = Some(*interaction);
                return;
            };
            let Some((component, _before_transform, primitive)) =
                find_selected_primitive_part(&draft, part_id)
            else {
                tweak.selected_part_id = None;
                workshop.error = Some("Selected part no longer exists in the draft.".into());
                workshop.status = "Recolor failed.".into();
                *last_interaction = Some(*interaction);
                return;
            };

            let color = color_picker_current_color(&tweak);

            let before_color = match &primitive {
                crate::object::registry::PrimitiveVisualDef::Primitive { color, .. } => *color,
                _ => Color::srgb(0.85, 0.87, 0.90),
            };
            let before_prim_json = match build_set_primitive_json(&primitive, before_color) {
                Ok(value) => value,
                Err(err) => {
                    workshop.error = Some(err);
                    workshop.status = "Recolor failed.".into();
                    *last_interaction = Some(*interaction);
                    return;
                }
            };
            let after_prim_json = match build_set_primitive_json(&primitive, color) {
                Ok(value) => value,
                Err(err) => {
                    workshop.error = Some(err);
                    workshop.status = "Recolor failed.".into();
                    *last_interaction = Some(*interaction);
                    return;
                }
            };

            let before_args = build_update_primitive_part_args(
                component.as_str(),
                part_id,
                None,
                Some(before_prim_json),
            );
            let after_args = build_update_primitive_part_args(
                component.as_str(),
                part_id,
                None,
                Some(after_prim_json),
            );

            let apply_args = patch_apply_draft_ops_args(after_args.clone(), job.assembly_rev());
            match super::gen3d_apply_draft_ops_from_api(&mut job, &mut draft, apply_args) {
                Ok(_) => {
                    workshop.error = None;
                    workshop.status = "Tweak: Recolor".into();
                    push_undo_entry(
                        &mut tweak,
                        super::state::Gen3dManualTweakUndoEntry {
                            label: "Recolor".to_string(),
                            undo_args_json: before_args,
                            redo_args_json: after_args,
                        },
                    );
                    tweak.redo.clear();
                    manual_tweak_push_recent_color(&mut tweak, color);
                    tweak.color_picker_open = false;
                    tweak.color_picker_rgb_focused = false;
                }
                Err(err) => {
                    workshop.error = Some(err);
                    workshop.status = "Recolor failed.".into();
                }
            }
        }
    }

    *last_interaction = Some(*interaction);
}

pub(crate) fn gen3d_manual_tweak_color_picker_cancel_button(
    build_scene: Res<State<BuildScene>>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut last_interaction: Local<Option<Interaction>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<Gen3dManualTweakColorPickerCancelButton>,
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        *last_interaction = None;
        return;
    }
    if !tweak.enabled || !tweak.color_picker_open {
        *last_interaction = None;
        return;
    }

    let Ok((interaction, mut bg, mut border)) = buttons.single_mut() else {
        *last_interaction = None;
        return;
    };

    match *interaction {
        Interaction::None => {
            *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.80));
            *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70));
        }
        Interaction::Hovered => {
            *bg = BackgroundColor(Color::srgba(0.16, 0.10, 0.10, 0.88));
            *border = BorderColor::all(Color::srgb(0.95, 0.45, 0.45));
        }
        Interaction::Pressed => {
            *bg = BackgroundColor(Color::srgba(0.22, 0.12, 0.12, 0.96));
            *border = BorderColor::all(Color::srgb(1.00, 0.55, 0.55));

            let was_pressed = matches!(*last_interaction, Some(Interaction::Pressed));
            if was_pressed {
                return;
            }

            tweak.color_picker_open = false;
            tweak.color_picker_rgb_focused = false;
        }
    }

    *last_interaction = Some(*interaction);
}

pub(crate) fn gen3d_manual_tweak_color_picker_update_ui(
    build_scene: Res<State<BuildScene>>,
    time: Res<Time>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut nodes: ParamSet<(
        Query<(&mut Node, &mut Visibility), With<Gen3dManualTweakColorPickerRoot>>,
        Query<&mut Node, With<Gen3dManualTweakColorPickerPaletteSelector>>,
        Query<&mut Node, With<Gen3dManualTweakColorPickerValueSelector>>,
        Query<
            (
                &Gen3dManualTweakColorPickerRecentSwatch,
                &mut BackgroundColor,
                &mut BorderColor,
                &mut Node,
            ),
            (
                With<Button>,
                With<Gen3dManualTweakColorPickerRecentSwatch>,
                Without<Gen3dManualTweakColorPickerRgbField>,
                Without<Gen3dManualTweakColorPickerPreviewSwatch>,
            ),
        >,
    )>,
    mut rgb_fields: Query<
        (&mut BackgroundColor, &mut BorderColor),
        (
            With<Gen3dManualTweakColorPickerRgbField>,
            Without<Gen3dManualTweakColorPickerRecentSwatch>,
            Without<Gen3dManualTweakColorPickerPreviewSwatch>,
        ),
    >,
    mut rgb_texts: Query<&mut Text, With<Gen3dManualTweakColorPickerRgbFieldText>>,
    mut preview_swatches: Query<
        &mut BackgroundColor,
        (
            With<Gen3dManualTweakColorPickerPreviewSwatch>,
            Without<Gen3dManualTweakColorPickerRgbField>,
            Without<Gen3dManualTweakColorPickerRecentSwatch>,
        ),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        if let Ok((mut node, mut vis)) = nodes.p0().single_mut() {
            node.display = Display::None;
            *vis = Visibility::Hidden;
        }
        return;
    }

    let open = tweak.enabled && tweak.color_picker_open;
    if let Ok((mut node, mut vis)) = nodes.p0().single_mut() {
        node.display = if open { Display::Flex } else { Display::None };
        *vis = if open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if !open {
        return;
    }

    let color = color_picker_current_color(&tweak);
    for mut bg in &mut preview_swatches {
        *bg = BackgroundColor(color);
    }

    if !tweak.color_picker_rgb_focused {
        tweak.color_picker_rgb_text = format_rgb_text(color);
    }
    let mut rgb_display = tweak.color_picker_rgb_text.clone();
    if tweak.color_picker_rgb_focused {
        let blink_on = ((time.elapsed_secs() * 2.0).floor() as i32) & 1 == 0;
        if blink_on {
            rgb_display.push('▏');
        }
    }
    for mut text in &mut rgb_texts {
        **text = rgb_display.clone().into();
    }
    for (mut bg, mut border) in &mut rgb_fields {
        let focused = tweak.color_picker_rgb_focused;
        let alpha = if focused { 0.88 } else { 0.80 };
        *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, alpha));
        *border = BorderColor::all(if focused {
            Color::srgb(0.30, 0.55, 0.95)
        } else {
            Color::srgba(0.25, 0.25, 0.30, 0.75)
        });
    }

    let selector_r = 6.0;
    let max = (MANUAL_TWEAK_COLOR_PICKER_UI_PALETTE_SIZE_PX - selector_r * 2.0).max(1.0);
    let h = tweak.color_picker_h.clamp(0.0, 1.0);
    let s = tweak.color_picker_s.clamp(0.0, 1.0);
    let x = selector_r + h * max;
    let y = selector_r + (1.0 - s) * max;
    for mut node in &mut nodes.p1() {
        node.left = Val::Px((x - selector_r).round());
        node.top = Val::Px((y - selector_r).round());
    }

    let v = tweak.color_picker_v.clamp(0.0, 1.0);
    let selector_h = 4.0;
    let h_max = (MANUAL_TWEAK_COLOR_PICKER_UI_VALUE_HEIGHT_PX - selector_h).max(1.0);
    let top = ((1.0 - v) * h_max).round();
    for mut node in &mut nodes.p2() {
        node.top = Val::Px(top);
    }

    for (swatch, mut bg, mut border, mut node) in &mut nodes.p3() {
        let index = swatch.index();
        let Some(rgba) = tweak.color_picker_recent_rgba.get(index).copied() else {
            node.display = Display::None;
            continue;
        };
        node.display = Display::Flex;
        *bg = BackgroundColor(Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3]));
        *border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75));
    }
}

pub(crate) fn gen3d_manual_tweak_color_picker_update_images(
    build_scene: Res<State<BuildScene>>,
    tweak: Res<Gen3dManualTweakState>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<Option<(f32, f32, f32)>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        *last = None;
        return;
    }
    if !tweak.enabled || !tweak.color_picker_open {
        *last = None;
        return;
    }

    let h = tweak.color_picker_h;
    let s = tweak.color_picker_s;
    let v = tweak.color_picker_v;
    let needs_update = match *last {
        Some((ph, ps, pv)) => (ph - h).abs() > 1e-4 || (ps - s).abs() > 1e-4 || (pv - v).abs() > 1e-4,
        None => true,
    };
    if !needs_update {
        return;
    }
    *last = Some((h, s, v));

    if let Some(image) = images.get_mut(&tweak.color_picker_palette_image) {
        let size = MANUAL_TWEAK_COLOR_PICKER_PALETTE_TEX_SIZE_PX as usize;
        if let Some(data) = image.data.as_mut() {
            if data.len() == size * size * 4 {
                for y in 0..size {
                    let s_y = 1.0 - (y as f32 / (size - 1).max(1) as f32);
                    for x in 0..size {
                        let h_x = x as f32 / (size - 1).max(1) as f32;
                        let (r, g, b) = hsv_to_srgb(h_x, s_y, v);
                        let idx = (y * size + x) * 4;
                        data[idx] = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
                        data[idx + 1] = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
                        data[idx + 2] = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
                        data[idx + 3] = 255;
                    }
                }
            }
        }
    }

    if let Some(image) = images.get_mut(&tweak.color_picker_value_image) {
        let w = MANUAL_TWEAK_COLOR_PICKER_VALUE_TEX_WIDTH_PX as usize;
        let h_px = MANUAL_TWEAK_COLOR_PICKER_VALUE_TEX_HEIGHT_PX as usize;
        if let Some(data) = image.data.as_mut() {
            if data.len() == w * h_px * 4 {
                for y in 0..h_px {
                    let v_y = 1.0 - (y as f32 / (h_px - 1).max(1) as f32);
                    let (r, g, b) = hsv_to_srgb(h, s, v_y);
                    let ru8 = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
                    let gu8 = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
                    let bu8 = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
                    for x in 0..w {
                        let idx = (y * w + x) * 4;
                        data[idx] = ru8;
                        data[idx + 1] = gu8;
                        data[idx + 2] = bu8;
                        data[idx + 3] = 255;
                    }
                }
            }
        }
    }
}
