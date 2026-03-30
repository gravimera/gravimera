use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::constants::*;
use crate::genfloor::{floor_half_size, ActiveWorldFloor};
use crate::geometry::ray_plane_intersection_y0;
use crate::types::*;

pub(crate) fn camera_zoom_input(
    windows: Query<&Window, With<PrimaryWindow>>,
    scenes_panel_roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::workspace_ui::ScenesPanelRoot>,
    >,
    model_panel_roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::model_library_ui::ModelLibraryRoot>,
    >,
    floor_panel_roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::floor_library_ui::FloorLibraryRoot>,
    >,
    model_library_preview_roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::model_library_ui::ModelLibraryPreviewOverlayRoot>,
    >,
    meta_panel_roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::motion_ui::MotionAlgorithmUiRoot>,
    >,
    mut mouse_wheel: MessageReader<MouseWheel>,
    mut zoom: ResMut<CameraZoom>,
) {
    let cursor_over_ui_panel = windows
        .single()
        .ok()
        .and_then(|window| window.physical_cursor_position())
        .is_some_and(|cursor| {
            scenes_panel_roots
                .single()
                .ok()
                .is_some_and(|(node, transform, vis)| {
                    *vis != Visibility::Hidden && node.contains_point(*transform, cursor)
                })
                || model_panel_roots
                    .single()
                    .ok()
                    .is_some_and(|(node, transform, vis)| {
                        *vis != Visibility::Hidden && node.contains_point(*transform, cursor)
                    })
                || floor_panel_roots
                    .single()
                    .ok()
                    .is_some_and(|(node, transform, vis)| {
                        *vis != Visibility::Hidden && node.contains_point(*transform, cursor)
                    })
                || meta_panel_roots
                    .single()
                    .ok()
                    .is_some_and(|(node, transform, vis)| {
                        *vis != Visibility::Hidden && node.contains_point(*transform, cursor)
                    })
                || crate::model_library_ui::model_library_preview_overlay_contains_cursor(
                    cursor,
                    &model_library_preview_roots,
                )
        });
    if cursor_over_ui_panel {
        for _ in mouse_wheel.read() {}
        return;
    }

    let mut scroll = 0.0f32;
    for ev in mouse_wheel.read() {
        let delta = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y / 120.0,
        };
        scroll += delta;
    }

    if scroll.abs() <= 0.0001 {
        return;
    }

    zoom.t = (zoom.t + scroll * CAMERA_ZOOM_SENSITIVITY).clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX);
}

fn edge_pan_factor_for_cursor(cursor: Vec2, width: f32, height: f32, margin_px: f32) -> Vec2 {
    if width <= 1.0 || height <= 1.0 {
        return Vec2::ZERO;
    }

    let margin = margin_px.max(1.0);
    let mut factor = Vec2::ZERO;
    if cursor.x < margin {
        let raw = ((margin - cursor.x) / margin).clamp(0.0, 1.0);
        factor.x = raw * raw;
    } else if cursor.x > width - margin {
        let raw = ((cursor.x - (width - margin)) / margin).clamp(0.0, 1.0);
        factor.x = -raw * raw;
    }

    if cursor.y < margin {
        let raw = ((margin - cursor.y) / margin).clamp(0.0, 1.0);
        factor.y = raw * raw;
    } else if cursor.y > height - margin {
        let raw = ((cursor.y - (height - margin)) / margin).clamp(0.0, 1.0);
        factor.y = -raw * raw;
    }

    factor
}

fn edge_pan_factor(window: &Window) -> Option<(Vec2, Vec2)> {
    let cursor = window.cursor_position()?;
    let width = window.width();
    let height = window.height();
    if width <= 1.0 || height <= 1.0 {
        return None;
    }

    let factor = edge_pan_factor_for_cursor(cursor, width, height, CAMERA_EDGE_PAN_MARGIN_PX);
    Some((cursor, factor))
}

fn sorted_selection_entity_bits(selection: &SelectionState) -> Vec<u64> {
    let mut entities: Vec<u64> = selection
        .selected
        .iter()
        .map(|entity| entity.to_bits())
        .collect();
    entities.sort_unstable();
    entities
}

pub(crate) fn camera_keyboard_rotate(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<CommandConsole>,
    model_library: Res<crate::model_library_ui::ModelLibraryUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    model_library_preview_roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::model_library_ui::ModelLibraryPreviewOverlayRoot>,
    >,
    mut camera_yaw: ResMut<CameraYaw>,
    mut camera_pitch: ResMut<CameraPitch>,
) {
    if console.open {
        return;
    }
    if model_library.is_preview_open() {
        if let Ok(window) = windows.single() {
            if let Some(cursor) = window.physical_cursor_position() {
                if crate::model_library_ui::model_library_preview_overlay_contains_cursor(
                    cursor,
                    &model_library_preview_roots,
                ) {
                    return;
                }
            }
        }
    }
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let mut yaw_dir = 0.0f32;
    if keys.pressed(KeyCode::KeyQ) {
        yaw_dir += 1.0;
    }
    if keys.pressed(KeyCode::KeyE) {
        yaw_dir -= 1.0;
    }
    if yaw_dir.abs() > 1e-4 {
        camera_yaw.yaw =
            wrap_angle(camera_yaw.yaw + yaw_dir * CAMERA_KEY_ROTATE_YAW_RADS_PER_SEC * dt);
        camera_yaw.initialized = true;
    }

    let mut pitch_dir = 0.0f32;
    if keys.pressed(KeyCode::KeyZ) {
        pitch_dir -= 1.0;
    }
    if keys.pressed(KeyCode::KeyX) {
        pitch_dir += 1.0;
    }
    if pitch_dir.abs() > 1e-4 {
        camera_pitch.pitch = (camera_pitch.pitch
            + pitch_dir * CAMERA_KEY_ROTATE_PITCH_RADS_PER_SEC * dt)
            .clamp(CAMERA_PITCH_DELTA_MIN_RADS, CAMERA_PITCH_DELTA_MAX_RADS);
    }
}

pub(crate) fn camera_edge_pan(
    time: Res<Time>,
    zoom: Res<CameraZoom>,
    console: Res<CommandConsole>,
    windows: Query<&Window, With<PrimaryWindow>>,
    model_library: Res<crate::model_library_ui::ModelLibraryUiState>,
    model_library_preview_roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::model_library_ui::ModelLibraryPreviewOverlayRoot>,
    >,
    camera_yaw: Res<CameraYaw>,
    active_floor: Res<ActiveWorldFloor>,
    mut focus: ResMut<CameraFocus>,
) {
    if console.open {
        return;
    }

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    if !focus.initialized {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    if model_library.is_preview_open() {
        if let Some(cursor) = window.physical_cursor_position() {
            if crate::model_library_ui::model_library_preview_overlay_contains_cursor(
                cursor,
                &model_library_preview_roots,
            ) {
                return;
            }
        }
    }
    let Some((_cursor, factor)) = edge_pan_factor(window) else {
        return;
    };
    if factor.length_squared() <= 1e-6 {
        return;
    }

    let zoom_t = zoom.t.clamp(0.0, CAMERA_ZOOM_MAX);
    let speed = CAMERA_EDGE_PAN_SPEED_FAR_UNITS_PER_SEC
        + (CAMERA_EDGE_PAN_SPEED_NEAR_UNITS_PER_SEC - CAMERA_EDGE_PAN_SPEED_FAR_UNITS_PER_SEC)
            * zoom_t;

    let yaw = camera_yaw.yaw;
    let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let right = Vec3::Y.cross(forward);

    let pan = right * factor.x + forward * factor.y;
    if pan.length_squared() <= 1e-6 {
        return;
    }

    let delta = pan.normalize() * speed * dt * pan.length().clamp(0.0, 1.0);
    let floor_half = floor_half_size(&active_floor);
    focus.position.x = (focus.position.x + delta.x).clamp(-floor_half.x, floor_half.x);
    focus.position.z = (focus.position.z + delta.z).clamp(-floor_half.y, floor_half.y);
}

pub(crate) fn camera_keyboard_pan(
    time: Res<Time>,
    zoom: Res<CameraZoom>,
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<CommandConsole>,
    model_library: Res<crate::model_library_ui::ModelLibraryUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    model_library_preview_roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::model_library_ui::ModelLibraryPreviewOverlayRoot>,
    >,
    selection: Res<SelectionState>,
    camera_yaw: Res<CameraYaw>,
    active_floor: Res<ActiveWorldFloor>,
    mut focus: ResMut<CameraFocus>,
) {
    if console.open {
        return;
    }
    if model_library.is_preview_open() {
        if let Ok(window) = windows.single() {
            if let Some(cursor) = window.physical_cursor_position() {
                if crate::model_library_ui::model_library_preview_overlay_contains_cursor(
                    cursor,
                    &model_library_preview_roots,
                ) {
                    return;
                }
            }
        }
    }

    if !selection.selected.is_empty() {
        return;
    }

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    if !focus.initialized {
        return;
    }

    let mut dir = Vec3::ZERO;
    let yaw = camera_yaw.yaw;
    let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let right = Vec3::Y.cross(forward);

    if keys.pressed(KeyCode::KeyW) {
        dir += forward;
    }
    if keys.pressed(KeyCode::KeyS) {
        dir -= forward;
    }
    if keys.pressed(KeyCode::KeyD) {
        dir -= right;
    }
    if keys.pressed(KeyCode::KeyA) {
        dir += right;
    }

    if dir.length_squared() <= 1e-6 {
        return;
    }

    let zoom_t = zoom.t.clamp(0.0, CAMERA_ZOOM_MAX);
    let speed = CAMERA_EDGE_PAN_SPEED_FAR_UNITS_PER_SEC
        + (CAMERA_EDGE_PAN_SPEED_NEAR_UNITS_PER_SEC - CAMERA_EDGE_PAN_SPEED_FAR_UNITS_PER_SEC)
            * zoom_t;

    let delta = dir.normalize() * speed * dt;
    let floor_half = floor_half_size(&active_floor);
    focus.position.x = (focus.position.x + delta.x).clamp(-floor_half.x, floor_half.x);
    focus.position.z = (focus.position.z + delta.z).clamp(-floor_half.y, floor_half.y);
}

pub(crate) fn camera_follow_selection(
    console: Res<CommandConsole>,
    selection: Res<SelectionState>,
    active_floor: Res<ActiveWorldFloor>,
    mut focus: ResMut<CameraFocus>,
    mut last_selected_entities: Local<Vec<u64>>,
    transforms: Query<&Transform, Without<MainCamera>>,
) {
    if console.open {
        return;
    }

    if selection.selected.is_empty() {
        last_selected_entities.clear();
        return;
    }

    let selected_entities = sorted_selection_entity_bits(&selection);
    let selection_changed = last_selected_entities.as_slice() != selected_entities.as_slice();

    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    let mut sum_y = 0.0f32;
    let mut count = 0u32;

    for entity in selection.selected.iter().copied() {
        let Ok(transform) = transforms.get(entity) else {
            continue;
        };
        let pos = transform.translation;
        min_x = min_x.min(pos.x);
        min_z = min_z.min(pos.z);
        max_x = max_x.max(pos.x);
        max_z = max_z.max(pos.z);
        sum_y += pos.y;
        count += 1;
    }

    if count == 0 {
        *last_selected_entities = selected_entities;
        return;
    }

    let target = Vec3::new(
        (min_x + max_x) * 0.5,
        sum_y / count as f32,
        (min_z + max_z) * 0.5,
    );

    if !focus.initialized {
        focus.position = target;
        focus.initialized = true;
        *last_selected_entities = selected_entities;
        return;
    }

    if selection_changed {
        // Keep the RTS camera height stable while the same selection walks across uneven terrain.
        focus.position.y = target.y;
    }
    *last_selected_entities = selected_entities;

    let delta = Vec2::new(target.x - focus.position.x, target.z - focus.position.z);
    let deadzone = CAMERA_FOLLOW_SELECTION_DEADZONE_UNITS.max(0.0);
    if delta.length_squared() <= deadzone * deadzone {
        return;
    }

    let dist = delta.length().max(1e-6);
    let correction = delta - (delta / dist) * deadzone;
    let floor_half = floor_half_size(&active_floor);
    focus.position.x = (focus.position.x + correction.x).clamp(-floor_half.x, floor_half.x);
    focus.position.z = (focus.position.z + correction.y).clamp(-floor_half.y, floor_half.y);
}

pub(crate) fn camera_follow(
    zoom: Res<CameraZoom>,
    mut camera_yaw: ResMut<CameraYaw>,
    camera_pitch: Res<CameraPitch>,
    mut focus: ResMut<CameraFocus>,
    mut camera_q: Query<&mut Transform, (With<MainCamera>, Without<Player>)>,
    player_q: Query<&Transform, With<Player>>,
) {
    let mut camera_transform = match camera_q.single_mut() {
        Ok(t) => t,
        Err(_) => return,
    };

    let t = zoom.t.clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX);

    if !camera_yaw.initialized {
        camera_yaw.yaw = 0.0;
        camera_yaw.initialized = true;
    }

    if !focus.initialized {
        focus.position = player_q
            .iter()
            .next()
            .map(|t| t.translation)
            .unwrap_or(Vec3::new(0.0, PLAYER_Y, 0.0));
        focus.initialized = true;
    }

    let forward = Vec3::new(camera_yaw.yaw.sin(), 0.0, camera_yaw.yaw.cos());
    let right = Vec3::Y.cross(forward);

    let far_offset =
        Vec3::Y * CAMERA_OFFSET.y + right * CAMERA_OFFSET.x - forward * CAMERA_OFFSET.z;
    let zoom_scale = if t >= 0.0 {
        1.0 + t * (CAMERA_ZOOM_NEAR_SCALE - 1.0)
    } else {
        let out_t = (-t).clamp(0.0, 1.0);
        let eased = out_t * out_t;
        1.0 + eased * (CAMERA_ZOOM_FAR_SCALE - 1.0)
    };
    let base_offset = far_offset * zoom_scale;
    let pitch_rot = Quat::from_axis_angle(right, camera_pitch.pitch);
    let offset = pitch_rot * base_offset;

    let focus_pos = focus.position;
    camera_transform.translation = focus_pos + offset;

    camera_transform.look_at(focus_pos, Vec3::Y);
}

fn wrap_angle(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

pub(crate) fn aim_player(
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &Transform), With<MainCamera>>,
    mut aim: ResMut<Aim>,
    player_q: Query<&Transform, With<Player>>,
) {
    aim.has_cursor_hit = false;

    let window = match windows.single() {
        Ok(w) => w,
        Err(_) => return,
    };
    let cursor_pos = match window.cursor_position() {
        Some(p) => p,
        None => return,
    };

    let (camera, camera_transform) = match camera_q.single() {
        Ok(v) => v,
        Err(_) => return,
    };

    let camera_global = GlobalTransform::from(*camera_transform);
    let Ok(ray) = camera.viewport_to_world(&camera_global, cursor_pos) else {
        return;
    };
    let Some(hit) = ray_plane_intersection_y0(ray) else {
        return;
    };
    aim.cursor_hit = hit;
    aim.has_cursor_hit = true;

    let Ok(player_transform) = player_q.single() else {
        return;
    };

    let to_hit = hit - player_transform.translation;
    let flat = Vec3::new(to_hit.x, 0.0, to_hit.z);
    if flat.length_squared() < 0.0001 {
        return;
    }

    aim.direction = flat.normalize();
}

fn edge_pan_arrow_glyph(factor: Vec2) -> &'static str {
    if factor.length_squared() <= 1e-6 {
        return "";
    }
    if factor.x.abs() >= factor.y.abs() {
        if factor.x < -1e-4 {
            "→"
        } else if factor.x > 1e-4 {
            "←"
        } else {
            ""
        }
    } else if factor.y < -1e-4 {
        "↓"
    } else if factor.y > 1e-4 {
        "↑"
    } else {
        ""
    }
}

pub(crate) fn update_edge_scroll_cursor_indicator(
    time: Res<Time>,
    console: Res<CommandConsole>,
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut root_q: Query<(&mut Node, &mut Visibility), With<EdgeScrollIndicatorRoot>>,
    mut text_q: Query<(&mut Text, &mut TextFont, &mut TextColor), With<EdgeScrollIndicatorText>>,
) {
    let Ok((window, mut cursor)) = windows.single_mut() else {
        return;
    };

    let Some((cursor_pos, factor)) = edge_pan_factor(window) else {
        cursor.visible = true;
        if let Ok((_node, mut visibility)) = root_q.single_mut() {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    let active = !console.open && factor.length_squared() > 1e-6;
    if !active {
        cursor.visible = true;
        if let Ok((_node, mut visibility)) = root_q.single_mut() {
            *visibility = Visibility::Hidden;
        }
        return;
    }

    cursor.visible = false;

    if let Ok((mut node, mut visibility)) = root_q.single_mut() {
        let size = EDGE_SCROLL_INDICATOR_SIZE_PX.max(1.0);
        let half = size * 0.5;
        let max_left = (window.width() - size).max(0.0);
        let max_top = (window.height() - size).max(0.0);
        let left = (cursor_pos.x - half).clamp(0.0, max_left);
        let top = (cursor_pos.y - half).clamp(0.0, max_top);
        node.left = Val::Px(left);
        node.top = Val::Px(top);
        *visibility = Visibility::Inherited;
    }

    if let Ok((mut text, mut font, mut color)) = text_q.single_mut() {
        **text = edge_pan_arrow_glyph(factor).to_string();

        let wave =
            (time.elapsed_secs() * EDGE_SCROLL_INDICATOR_PULSE_RADS_PER_SEC).sin() * 0.5 + 0.5;
        font.font_size = EDGE_SCROLL_INDICATOR_FONT_SIZE_PX * (0.9 + wave * 0.2);

        let alpha = EDGE_SCROLL_INDICATOR_ALPHA_MIN
            + wave * (EDGE_SCROLL_INDICATOR_ALPHA_MAX - EDGE_SCROLL_INDICATOR_ALPHA_MIN);
        color.0 = Color::srgba(0.35, 1.0, 0.45, alpha);
    }
}

pub(crate) fn animate_player_model(
    time: Res<Time>,
    mut player_q: Query<(&Transform, &mut PlayerAnimator), With<Player>>,
    mut legs_q: Query<(&PlayerLeg, &mut Transform), Without<Player>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let Ok((player_transform, mut animator)) = player_q.single_mut() else {
        return;
    };

    let delta = player_transform.translation - animator.last_translation;
    animator.last_translation = player_transform.translation;

    let speed = Vec2::new(delta.x, delta.z).length() / dt;
    let speed01 = (speed / PLAYER_SPEED).clamp(0.0, 1.0);

    animator.phase =
        (animator.phase + dt * PLAYER_LEG_SWING_RADS_PER_SEC * speed01) % std::f32::consts::TAU;
    let swing = animator.phase.sin() * PLAYER_LEG_SWING_MAX_RADS * speed01;

    for (leg, mut transform) in &mut legs_q {
        transform.rotation = Quat::from_rotation_x(swing * leg.side);
    }
}

pub(crate) fn update_player_gun_visuals(
    game: Res<Game>,
    mut guns: Query<(&PlayerGunVisual, &mut Visibility)>,
) {
    for (gun, mut visibility) in &mut guns {
        *visibility = if gun.weapon == game.weapon {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::App;

    #[test]
    fn edge_pan_factor_left_is_positive_right_is_negative() {
        let width = 100.0;
        let height = 100.0;
        let margin_px = 10.0;

        let left =
            edge_pan_factor_for_cursor(Vec2::new(0.0, height * 0.5), width, height, margin_px);
        assert!(
            left.x > 0.0,
            "expected left edge pan factor x > 0; got {left:?}"
        );

        let right = edge_pan_factor_for_cursor(
            Vec2::new(width - 0.1, height * 0.5),
            width,
            height,
            margin_px,
        );
        assert!(
            right.x < 0.0,
            "expected right edge pan factor x < 0; got {right:?}"
        );
    }

    #[test]
    fn edge_pan_arrow_glyph_matches_signs() {
        assert_eq!(edge_pan_arrow_glyph(Vec2::new(1.0, 0.0)), "←");
        assert_eq!(edge_pan_arrow_glyph(Vec2::new(-1.0, 0.0)), "→");
    }

    #[test]
    fn camera_follow_selection_keeps_height_stable_for_same_selection() {
        let mut app = App::new();
        app.insert_resource(CommandConsole::default());
        app.insert_resource(SelectionState::default());
        app.insert_resource(ActiveWorldFloor::default());
        app.insert_resource(CameraFocus {
            position: Vec3::new(0.0, 1.0, 0.0),
            initialized: true,
        });
        app.add_systems(Update, camera_follow_selection);

        let first = app
            .world_mut()
            .spawn(Transform::from_xyz(2.0, 5.0, 3.0))
            .id();
        app.world_mut()
            .resource_mut::<SelectionState>()
            .selected
            .insert(first);

        app.update();
        assert!(
            (app.world().resource::<CameraFocus>().position.y - 5.0).abs() < 1e-5,
            "initial selection change should realign camera height"
        );

        app.world_mut()
            .entity_mut(first)
            .insert(Transform::from_xyz(2.0, 8.0, 3.0));

        app.update();
        assert!(
            (app.world().resource::<CameraFocus>().position.y - 5.0).abs() < 1e-5,
            "same selection moving over uneven terrain should not tug camera height"
        );

        let second = app
            .world_mut()
            .spawn(Transform::from_xyz(-1.0, 9.0, -1.0))
            .id();
        {
            let mut selection = app.world_mut().resource_mut::<SelectionState>();
            selection.selected.clear();
            selection.selected.insert(second);
        }

        app.update();
        assert!(
            (app.world().resource::<CameraFocus>().position.y - 9.0).abs() < 1e-5,
            "changing selection should still realign camera height"
        );
    }
}
