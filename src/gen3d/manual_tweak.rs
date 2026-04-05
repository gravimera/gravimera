use bevy::asset::AssetId;
use bevy::camera::primitives::MeshAabb;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::object::visuals::VisualPartId;
use crate::types::BuildScene;

use super::ai::Gen3dAiJob;
use super::preview;
use super::state::{
    Gen3dDraft, Gen3dManualTweakButton, Gen3dManualTweakButtonText, Gen3dManualTweakState,
    Gen3dPreviewAnimationDropdownButton, Gen3dPreviewAnimationDropdownList, Gen3dPreviewCamera,
    Gen3dPreviewExplodeToggleButton, Gen3dPreviewExportButton, Gen3dPreviewPanel,
    Gen3dSidePanelRoot, Gen3dSidePanelToggleButton, Gen3dTweakSelectedFrame,
    Gen3dTweakSelectedInfoCard, Gen3dTweakSelectedInfoText, Gen3dWorkshop,
};
use super::task_queue::{Gen3dTaskQueue, Gen3dTaskState};

const MANUAL_TWEAK_UNDO_LIMIT: usize = 64;
const MANUAL_TWEAK_MOVE_STEP_M: f32 = 0.05;
const MANUAL_TWEAK_MOVE_STEP_FAST_M: f32 = 0.20;
const MANUAL_TWEAK_ROT_STEP_DEG: f32 = 5.0;
const MANUAL_TWEAK_ROT_STEP_FAST_DEG: f32 = 45.0;
const MANUAL_TWEAK_SCALE_STEP: f32 = 1.05;
const MANUAL_TWEAK_SCALE_STEP_FAST: f32 = 1.20;
const MANUAL_TWEAK_SCALE_MIN: f32 = 0.01;
const MANUAL_TWEAK_SCALE_MAX: f32 = 50.0;

const MANUAL_TWEAK_COLOR_PALETTE_RGBA: &[[f32; 4]] = &[
    [0.92, 0.18, 0.22, 1.0], // red
    [0.95, 0.55, 0.18, 1.0], // orange
    [0.95, 0.82, 0.24, 1.0], // yellow
    [0.20, 0.76, 0.35, 1.0], // green
    [0.08, 0.62, 0.85, 1.0], // cyan
    [0.18, 0.42, 0.92, 1.0], // blue
    [0.62, 0.28, 0.88, 1.0], // purple
    [0.94, 0.40, 0.68, 1.0], // pink
    [0.85, 0.87, 0.90, 1.0], // light gray
    [0.35, 0.38, 0.42, 1.0], // dark gray
    [0.30, 0.20, 0.12, 1.0], // brown
    [0.75, 0.75, 0.75, 1.0], // neutral
];

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
    panels: Query<
        'w,
        's,
        (&'static ComputedNode, &'static UiGlobalTransform),
        With<Gen3dPreviewPanel>,
    >,
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
    side_panel_root: &Query<(&ComputedNode, &UiGlobalTransform, Option<&Visibility>), With<Gen3dSidePanelRoot>>,
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
        click_state.pressed_cursor_physical = window.physical_cursor_position();
        click_state.pressed_in_preview = hovered;
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
    let Some(cursor_target) = preview::preview_cursor_to_target(end, layout.image_bounds_physical) else {
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
        let mag = value.abs().clamp(MANUAL_TWEAK_SCALE_MIN, MANUAL_TWEAK_SCALE_MAX);
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

fn color_to_rgba(color: Color) -> [f32; 4] {
    let c = color.to_srgba();
    [c.red, c.green, c.blue, c.alpha]
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
        op.as_object_mut().expect("json object").insert(
            "set_transform".into(),
            build_set_transform_json(transform),
        );
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

fn patch_apply_draft_ops_args(mut args: serde_json::Value, if_assembly_rev: u32) -> serde_json::Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    obj.insert("version".into(), serde_json::json!(1));
    obj.insert("atomic".into(), serde_json::json!(true));
    obj.insert("if_assembly_rev".into(), serde_json::json!(if_assembly_rev));
    args
}

fn push_undo_entry(tweak: &mut Gen3dManualTweakState, entry: super::state::Gen3dManualTweakUndoEntry) {
    if tweak.undo.len() >= MANUAL_TWEAK_UNDO_LIMIT {
        let drain = tweak.undo.len().saturating_sub(MANUAL_TWEAK_UNDO_LIMIT - 1);
        tweak.undo.drain(0..drain);
    }
    tweak.undo.push(entry);
}

fn find_selected_primitive_part(
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

        return;
    }

    let Some(part_id) = tweak.selected_part_id else {
        return;
    };

    let Some((component, before_transform, primitive)) = find_selected_primitive_part(&draft, part_id) else {
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
    if keys.just_pressed(KeyCode::Minus) {
        requested_scale *= if modifier_shift {
            1.0 / MANUAL_TWEAK_SCALE_STEP_FAST
        } else {
            1.0 / MANUAL_TWEAK_SCALE_STEP
        };
    }
    if keys.just_pressed(KeyCode::Equal) {
        requested_scale *= if modifier_shift {
            MANUAL_TWEAK_SCALE_STEP_FAST
        } else {
            MANUAL_TWEAK_SCALE_STEP
        };
    }

    let recolor_requested = keys.just_pressed(KeyCode::KeyC);

    if !requested_move.is_finite() || !requested_rot_deg.is_finite() || !requested_scale.is_finite() {
        return;
    }

    let mut set_transform = None;
    if requested_move.length_squared() > 1e-6
        || requested_rot_deg.abs() > 1e-6
        || (requested_scale - 1.0).abs() > 1e-6
    {
        let step = if modifier_shift {
            MANUAL_TWEAK_MOVE_STEP_FAST_M
        } else {
            MANUAL_TWEAK_MOVE_STEP_M
        };
        let delta = requested_move * step;

        let rot_step_deg = if modifier_shift {
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

    let mut set_primitive = None;
    if recolor_requested {
        if MANUAL_TWEAK_COLOR_PALETTE_RGBA.is_empty() {
            return;
        }
        let len = MANUAL_TWEAK_COLOR_PALETTE_RGBA.len();
        if modifier_shift {
            tweak.color_palette_index = tweak.color_palette_index.wrapping_add(len - 1) % len;
        } else {
            tweak.color_palette_index = tweak.color_palette_index.wrapping_add(1) % len;
        }
        let rgba = MANUAL_TWEAK_COLOR_PALETTE_RGBA[tweak.color_palette_index];
        let color = Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3]);
        match build_set_primitive_json(&primitive, color) {
            Ok(value) => set_primitive = Some(value),
            Err(err) => {
                workshop.error = Some(err);
                return;
            }
        }
    }

    if set_transform.is_none() && set_primitive.is_none() {
        return;
    }

    let before_args = build_update_primitive_part_args(
        component.as_str(),
        part_id,
        set_transform.map(|_| before_transform),
        if set_primitive.is_some() {
            let before_color = match &primitive {
                crate::object::registry::PrimitiveVisualDef::Primitive { color, .. } => *color,
                _ => Color::srgb(0.85, 0.87, 0.90),
            };
            build_set_primitive_json(&primitive, before_color).ok()
        } else {
            None
        },
    );

    let after_transform = set_transform.unwrap_or(before_transform);
    let after_args = build_update_primitive_part_args(
        component.as_str(),
        part_id,
        Some(after_transform),
        set_primitive.clone(),
    );

    let label = if recolor_requested && requested_move.length_squared() > 1e-6 {
        "Transform + recolor".to_string()
    } else if recolor_requested {
        "Recolor".to_string()
    } else if requested_rot_deg.abs() > 1e-6 {
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
        Or<(With<Gen3dTweakSelectedFrame>, With<Gen3dTweakSelectedInfoCard>)>,
    >,
    mut overlay_texts: Query<(&Gen3dTweakSelectedInfoText, &mut Text)>,
    mut mesh_cache: Local<MeshAabbCache>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let hide_all = |overlay_nodes: &mut Query<
        (
            Option<&Gen3dTweakSelectedFrame>,
            Option<&Gen3dTweakSelectedInfoCard>,
            &mut Node,
            &mut Visibility,
        ),
        Or<(With<Gen3dTweakSelectedFrame>, With<Gen3dTweakSelectedInfoCard>)>,
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
        .and_then(|((window, (camera, camera_transform)), (panel_node, panel_transform))| {
            preview::preview_image_layout(panel_node, *panel_transform)
                .map(|layout| (window, camera, camera_transform, layout))
        });
    let Some((_window, camera, camera_transform, layout)) = context else {
        hide_all(&mut overlay_nodes, &mut overlay_texts);
        return;
    };

    let mut frame_rect_panel: Option<Rect> = None;

    let selection = tweak.selected_part_id;
    let info_text = if let Some(part_id) = selection {
        let part = parts
            .iter()
            .find(|(_, id, _)| id.0 == part_id)
            .and_then(|(global_transform, _id, mesh3d)| {
                mesh_local_aabb_cached(&mut mesh_cache, &meshes, &mesh3d.0).map(|aabb| {
                    (global_transform.to_matrix(), aabb.0, aabb.1)
                })
            });
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
                let frame_min = preview::preview_target_to_panel_logical(min, layout).max(Vec2::ZERO);
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
            "Manual Tweak\nSelected: {}\nPart: {}\n\nMove: arrows | PgUp/PgDn (Shift=big)\nRotate: ,/. (Shift=45°)\nScale: -/= (Shift=big)\nRecolor: C (Shift=prev)\nUndo/Redo: Ctrl/Cmd+Z/Y",
            display_component.unwrap_or("unknown"),
            primitive_label.unwrap_or_else(|| "primitive".to_string()),
        );
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
        "Manual Tweak\nClick a part in the preview to select it.\n\nMove: arrows | PgUp/PgDn (Shift=big)\nRotate: ,/. (Shift=45°)\nScale: -/= (Shift=big)\nRecolor: C (Shift=prev)\nUndo/Redo: Ctrl/Cmd+Z/Y\n\nEsc: exit tweak".to_string()
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
                let card_left = (rect.max.x + 10.0)
                    .clamp(8.0, (layout.panel_size_logical.x - 240.0).max(8.0));
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
