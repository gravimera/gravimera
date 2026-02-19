use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::constants::*;
use crate::geometry::{safe_abs_scale_y, snap_to_grid};
use crate::object::registry::{MobilityMode, ObjectLibrary};
use crate::scene_store::SceneSaveRequest;
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, MainCamera, ObjectPrefabId,
    BuildState, Player, SelectionState,
};

const DRAG_START_THRESHOLD_PX: f32 = 6.0;
const DRAG_PICK_RADIUS_PX: f32 = 26.0;

#[derive(Clone, Copy, Debug)]
struct PendingDrag {
    entity: Entity,
    prefab_id: u128,
    start_cursor: Vec2,
    offset_xz: Vec2,
    is_unit: bool,
    mobility_mode: Option<MobilityMode>,
}

#[derive(Clone, Copy, Debug)]
struct ActiveDrag {
    entity: Entity,
    prefab_id: u128,
    offset_xz: Vec2,
    is_unit: bool,
    mobility_mode: Option<MobilityMode>,
}

#[derive(Resource, Default, Debug)]
pub(crate) struct WorldDragState {
    pending: Option<PendingDrag>,
    active: Option<ActiveDrag>,
}

impl WorldDragState {
    pub(crate) fn blocks_selection(&self) -> bool {
        self.pending.is_some() || self.active.is_some()
    }
}

pub(crate) fn world_drag_start(
    mut state: ResMut<WorldDragState>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    build: Res<BuildState>,
    model_library: Res<crate::model_library_ui::ModelLibraryUiState>,
    selection: Res<SelectionState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    library: Res<ObjectLibrary>,
    commandables: Query<(Entity, &Transform, &Collider, &ObjectPrefabId), With<Commandable>>,
    players: Query<(), With<Player>>,
    build_objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
) {
    if state.pending.is_some() || state.active.is_some() {
        return;
    }
    if build.placing_active {
        return;
    }
    if model_library.is_drag_active() {
        return;
    }
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if selection.selected.len() != 1 {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_global)) = camera_q.single() else {
        return;
    };

    let entity = *selection.selected.iter().next().unwrap();
    if players.contains(entity) {
        return;
    }

    if let Ok((_e, transform, collider, prefab_id)) = commandables.get(entity) {
        let mobility_mode = library.mobility(prefab_id.0).map(|m| m.mode);
        if cursor_hits_unit(
            cursor,
            camera,
            camera_global,
            &library,
            prefab_id.0,
            transform,
            collider,
        ) {
            if let Some(offset_xz) =
                cursor_offset_xz(window, camera, &camera_global, &library, &build_objects, transform)
            {
                state.pending = Some(PendingDrag {
                    entity,
                    prefab_id: prefab_id.0,
                    start_cursor: cursor,
                    offset_xz,
                    is_unit: true,
                    mobility_mode,
                });
            }
        }
        return;
    }

    if let Ok((transform, _collider, _dimensions, prefab_id)) = build_objects.get(entity) {
        let mobility_mode = library.mobility(prefab_id.0).map(|m| m.mode);
        if cursor_hits_build_object(cursor, camera, &camera_global, transform) {
            if let Some(offset_xz) =
                cursor_offset_xz(window, camera, &camera_global, &library, &build_objects, transform)
            {
                state.pending = Some(PendingDrag {
                    entity,
                    prefab_id: prefab_id.0,
                    start_cursor: cursor,
                    offset_xz,
                    is_unit: false,
                    mobility_mode,
                });
            }
        }
    }
}

pub(crate) fn world_drag_update(
    mut state: ResMut<WorldDragState>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    build: Res<BuildState>,
    model_library: Res<crate::model_library_ui::ModelLibraryUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    library: Res<ObjectLibrary>,
    mut units: Query<
        (&mut Transform, &Collider, &ObjectPrefabId),
        (With<Commandable>, Without<Player>, Without<BuildObject>),
    >,
    mut build_objects: Query<
        (Entity, &mut Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        (With<BuildObject>, Without<Commandable>),
    >,
    mut selection: ResMut<SelectionState>,
    mut scene_saves: bevy::ecs::message::MessageWriter<SceneSaveRequest>,
) {
    if build.placing_active || model_library.is_drag_active() {
        state.pending = None;
        state.active = None;
        selection.drag_start = None;
        selection.drag_end = None;
        return;
    }

    let cursor = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position());

    if mouse_buttons.just_released(MouseButton::Left) {
        if state.active.take().is_some() {
            scene_saves.write(SceneSaveRequest::new("dragged instance"));
        }
        state.pending = None;
        selection.drag_start = None;
        selection.drag_end = None;
        return;
    }

    let Some(cursor) = cursor else {
        return;
    };

    if let Some(pending) = state.pending {
        if cursor.distance(pending.start_cursor) > DRAG_START_THRESHOLD_PX {
            state.active = Some(ActiveDrag {
                entity: pending.entity,
                prefab_id: pending.prefab_id,
                offset_xz: pending.offset_xz,
                is_unit: pending.is_unit,
                mobility_mode: pending.mobility_mode,
            });
            state.pending = None;
        }
    }

    let Some(active) = state.active else {
        return;
    };
    let Ok((camera, camera_global)) = camera_q.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let cursor_pos = window.cursor_position().unwrap_or(cursor);
    let Ok(ray) = camera.viewport_to_world(&camera_global, cursor_pos) else {
        return;
    };

    let origin = ray.origin;
    let direction = ray.direction.as_vec3();
    let denom = direction.y;
    if denom.abs() < 1e-5 {
        return;
    }

    let mut best_t = f32::INFINITY;
    let mut best_hit = None;

    let t_ground = (0.0 - origin.y) / denom;
    if t_ground >= 0.0 {
        best_t = t_ground;
        best_hit = Some((origin + direction * t_ground, 0.0));
    }

    for (entity, transform, collider, dimensions, prefab_id) in build_objects.iter_mut() {
        if !library.interaction(prefab_id.0).supports_standing {
            continue;
        }
        if !active.is_unit && entity == active.entity {
            continue;
        }

        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let top_y = transform.translation.y - origin_y + dimensions.size.y;
        let t = (top_y - origin.y) / denom;
        if t < 0.0 || t >= best_t {
            continue;
        }

        let hit = origin + direction * t;
        let point = Vec2::new(hit.x, hit.z);
        let center = Vec2::new(transform.translation.x, transform.translation.z);
        if point.x < center.x - collider.half_extents.x
            || point.x > center.x + collider.half_extents.x
            || point.y < center.y - collider.half_extents.y
            || point.y > center.y + collider.half_extents.y
        {
            continue;
        }

        best_t = t;
        best_hit = Some((hit, top_y));
    }

    let Some((hit, surface_y)) = best_hit else {
        return;
    };

    let mut desired = Vec2::new(hit.x, hit.z) + active.offset_xz;
    desired.x = snap_to_grid(desired.x, BUILD_GRID_SIZE);
    desired.y = snap_to_grid(desired.y, BUILD_GRID_SIZE);

    if active.is_unit {
        let Ok((mut transform, collider, prefab_id)) = units.get_mut(active.entity) else {
            state.active = None;
            return;
        };
        if prefab_id.0 != active.prefab_id {
            state.active = None;
            return;
        }

        let radius = collider.radius.max(0.01);
        desired = desired.clamp(
            Vec2::splat(-WORLD_HALF_SIZE + radius),
            Vec2::splat(WORLD_HALF_SIZE - radius),
        );
        transform.translation.x = desired.x;
        transform.translation.z = desired.y;

        if active.mobility_mode != Some(MobilityMode::Air) {
            let scale_y = safe_abs_scale_y(transform.scale);
            let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
            transform.translation.y = surface_y + origin_y;
        }
    } else {
        let Ok((_e, mut transform, collider, dimensions, prefab_id)) = build_objects.get_mut(active.entity) else {
            state.active = None;
            return;
        };
        if prefab_id.0 != active.prefab_id {
            state.active = None;
            return;
        }

        desired = desired.clamp(
            Vec2::new(-WORLD_HALF_SIZE + collider.half_extents.x, -WORLD_HALF_SIZE + collider.half_extents.y),
            Vec2::new(WORLD_HALF_SIZE - collider.half_extents.x, WORLD_HALF_SIZE - collider.half_extents.y),
        );
        transform.translation.x = desired.x;
        transform.translation.z = desired.y;

        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let bottom_y = surface_y.max(0.0);
        transform.translation.y = bottom_y + origin_y;

        let _ = dimensions;
    }

    selection.drag_start = None;
    selection.drag_end = None;
}

fn cursor_hits_build_object(
    cursor: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    transform: &Transform,
) -> bool {
    let Some(screen) = camera
        .world_to_viewport(camera_transform, transform.translation)
        .ok()
    else {
        return false;
    };
    screen.distance(cursor) <= DRAG_PICK_RADIUS_PX
}

fn cursor_hits_unit(
    cursor: Vec2,
    camera: &Camera,
    camera_global: &GlobalTransform,
    library: &ObjectLibrary,
    prefab_id: u128,
    transform: &Transform,
    collider: &Collider,
) -> bool {
    let scale_y = safe_abs_scale_y(transform.scale);
    let height = library
        .size(prefab_id)
        .map(|s| s.y * scale_y)
        .unwrap_or(HERO_HEIGHT_WORLD * scale_y);
    let world_pos = transform.translation + Vec3::Y * (height * 0.5);
    let Some(screen) = camera.world_to_viewport(camera_global, world_pos).ok() else {
        return false;
    };

    let camera_right = camera_global.rotation() * Vec3::X;
    let scale = transform
        .scale
        .x
        .abs()
        .max(transform.scale.z.abs())
        .max(1e-3);
    let world_r = (collider.radius * scale).max(0.0);
    if world_r <= 1e-6 {
        return screen.distance(cursor) <= DRAG_PICK_RADIUS_PX;
    }
    let edge_world = world_pos + camera_right * world_r;
    let Some(edge_screen) = camera.world_to_viewport(camera_global, edge_world).ok() else {
        return false;
    };
    let pixel_radius = screen.distance(edge_screen).max(1.0);

    screen.distance(cursor) <= pixel_radius
}

fn cursor_offset_xz(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    build_objects: &Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    transform: &Transform,
) -> Option<Vec2> {
    let pick = crate::cursor_pick::cursor_surface_pick(
        window,
        camera,
        camera_transform,
        library,
        build_objects,
    )?;
    let offset = Vec2::new(transform.translation.x - pick.hit.x, transform.translation.z - pick.hit.z);
    Some(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_drag_update_has_disjoint_transform_queries() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<SceneSaveRequest>();
        app.init_resource::<WorldDragState>();
        app.init_resource::<BuildState>();
        app.init_resource::<crate::model_library_ui::ModelLibraryUiState>();
        app.init_resource::<SelectionState>();
        app.init_resource::<ObjectLibrary>();
        app.insert_resource(ButtonInput::<MouseButton>::default());

        app.add_systems(Update, world_drag_update);
        app.update();
    }
}
