use bevy::ecs::query::QueryFilter;
use bevy::prelude::*;

use crate::constants::{
    DEFAULT_OBJECT_SIZE_M, HERO_HEIGHT_WORLD, SELECTION_CIRCLE_FLASH_ALPHA_MAX,
    SELECTION_CIRCLE_FLASH_ALPHA_MIN, SELECTION_CIRCLE_PULSE_RADS_PER_SEC,
    SELECTION_RING_RADIUS_MULT,
};
use crate::geometry::safe_abs_scale_y;
use crate::object::registry::ObjectLibrary;
use crate::types::{AabbCollider, BuildDimensions, Collider, ObjectPrefabId};

pub(crate) const CURSOR_CIRCLE_FALLBACK_RADIUS_PX: f32 = 26.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectionCirclePick {
    pub(crate) entity: Entity,
    pub(crate) screen_center: Vec2,
    pub(crate) pixel_radius: f32,
    pub(crate) is_unit: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CursorPickPreference {
    /// When set, candidates matching this category are considered "more relevant" than others.
    pub(crate) prefer_units: Option<bool>,
}

pub(crate) fn pulse_wave_alpha(time: &Time, started_at_secs: f32) -> (f32, f32) {
    let t = (time.elapsed_secs() - started_at_secs).max(0.0);
    let wave = (t * SELECTION_CIRCLE_PULSE_RADS_PER_SEC).sin() * 0.5 + 0.5;
    let alpha = SELECTION_CIRCLE_FLASH_ALPHA_MIN
        + wave * (SELECTION_CIRCLE_FLASH_ALPHA_MAX - SELECTION_CIRCLE_FLASH_ALPHA_MIN);
    (wave, alpha)
}

pub(crate) fn pulse_radius(base_radius: f32, wave: f32) -> f32 {
    (base_radius.max(1.0) * (0.88 + wave * 0.24)).max(1.0)
}

pub(crate) fn circle_intersects_rect(center: Vec2, radius: f32, min: Vec2, max: Vec2) -> bool {
    let radius = radius.max(0.0);
    let closest = Vec2::new(center.x.clamp(min.x, max.x), center.y.clamp(min.y, max.y));
    center.distance_squared(closest) <= radius * radius
}

fn world_circle_to_screen_circle(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    world_center: Vec3,
    world_radius: f32,
) -> Option<(Vec2, f32)> {
    let screen_center = camera
        .world_to_viewport(camera_transform, world_center)
        .ok()?;
    let camera_right = camera_transform.rotation() * Vec3::X;
    let edge_world = world_center + camera_right * world_radius.max(0.0);
    let pixel_radius = camera
        .world_to_viewport(camera_transform, edge_world)
        .ok()
        .map(|edge| screen_center.distance(edge).max(1.0))
        .unwrap_or(CURSOR_CIRCLE_FALLBACK_RADIUS_PX);
    Some((screen_center, pixel_radius))
}

fn unit_world_center(library: &ObjectLibrary, prefab_id: u128, transform: &Transform) -> Vec3 {
    let scale_y = safe_abs_scale_y(transform.scale);
    let height = library
        .size(prefab_id)
        .map(|s| s.y * scale_y)
        .unwrap_or(HERO_HEIGHT_WORLD * scale_y);
    transform.translation + Vec3::Y * (height * 0.5)
}

fn unit_world_radius(
    library: &ObjectLibrary,
    prefab_id: u128,
    transform: &Transform,
    collider: Option<&Collider>,
) -> f32 {
    let from_collider = collider
        .map(|c| c.radius)
        .filter(|r| r.is_finite() && *r > 0.0);
    let radius = from_collider.unwrap_or_else(|| {
        let scale_xz = transform
            .scale
            .x
            .abs()
            .max(transform.scale.z.abs())
            .max(1e-3);
        let size = library
            .size(prefab_id)
            .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));
        let size_x = size.x.abs().max(0.01) * scale_xz;
        let size_z = size.z.abs().max(0.01) * scale_xz;
        (size_x.max(size_z) * 0.5).max(0.01)
    });
    (radius * SELECTION_RING_RADIUS_MULT).max(0.01)
}

fn build_world_center(transform: &Transform, dimensions: &BuildDimensions) -> Vec3 {
    let height = dimensions.size.y.max(0.01);
    transform.translation + Vec3::Y * (height * 0.5)
}

fn build_world_radius(collider: &AabbCollider) -> f32 {
    let radius = collider
        .half_extents
        .x
        .abs()
        .max(collider.half_extents.y.abs())
        .max(0.01);
    (radius * SELECTION_RING_RADIUS_MULT).max(0.01)
}

pub(crate) fn unit_screen_circle(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    prefab_id: u128,
    transform: &Transform,
    collider: Option<&Collider>,
) -> Option<(Vec2, f32)> {
    let world_center = unit_world_center(library, prefab_id, transform);
    let world_radius = unit_world_radius(library, prefab_id, transform, collider);
    world_circle_to_screen_circle(camera, camera_transform, world_center, world_radius)
}

pub(crate) fn build_screen_circle(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    transform: &Transform,
    collider: &AabbCollider,
    dimensions: &BuildDimensions,
) -> Option<(Vec2, f32)> {
    let world_center = build_world_center(transform, dimensions);
    let world_radius = build_world_radius(collider);
    world_circle_to_screen_circle(camera, camera_transform, world_center, world_radius)
}

pub(crate) fn pick_under_cursor<Uf, Bf>(
    cursor: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    units: &Query<(Entity, &Transform, Option<&Collider>, &ObjectPrefabId), Uf>,
    builds: &Query<
        (
            Entity,
            &Transform,
            &AabbCollider,
            &BuildDimensions,
            &ObjectPrefabId,
        ),
        Bf,
    >,
    include_builds: bool,
    preference: CursorPickPreference,
) -> Option<SelectionCirclePick>
where
    Uf: QueryFilter,
    Bf: QueryFilter,
{
    let mut best: Option<(SelectionCirclePick, f32, f32)> = None;

    let mut consider = |pick: SelectionCirclePick, d: f32, norm: f32| {
        if let Some((best_pick, best_d, best_norm)) = best {
            let preferred = preference
                .prefer_units
                .map(|prefer_units| pick.is_unit == prefer_units);
            let best_preferred = preference
                .prefer_units
                .map(|prefer_units| best_pick.is_unit == prefer_units);
            let better = match (preferred, best_preferred) {
                (Some(true), Some(false)) => true,
                (Some(false), Some(true)) => false,
                _ => {
                    (norm < best_norm)
                        || (norm == best_norm
                            && (d < best_d
                                || (d == best_d
                                    && (pick.pixel_radius < best_pick.pixel_radius
                                        || (pick.pixel_radius == best_pick.pixel_radius
                                            && pick.is_unit
                                            && !best_pick.is_unit)))))
                }
            };
            if better {
                best = Some((pick, d, norm));
            } else {
                best = Some((best_pick, best_d, best_norm));
            }
            return;
        }
        best = Some((pick, d, norm));
    };

    for (entity, transform, collider, prefab_id) in units.iter() {
        let world_center = unit_world_center(library, prefab_id.0, transform);
        let world_radius = unit_world_radius(library, prefab_id.0, transform, collider);
        let Some((screen_center, pixel_radius)) =
            world_circle_to_screen_circle(camera, camera_transform, world_center, world_radius)
        else {
            continue;
        };
        let d = screen_center.distance(cursor);
        if d > pixel_radius {
            continue;
        }
        let norm = d / pixel_radius.max(1.0);
        consider(
            SelectionCirclePick {
                entity,
                screen_center,
                pixel_radius,
                is_unit: true,
            },
            d,
            norm,
        );
    }

    if include_builds {
        for (entity, transform, collider, dimensions, _prefab_id) in builds.iter() {
            let world_center = build_world_center(transform, dimensions);
            let world_radius = build_world_radius(collider);
            let Some((screen_center, pixel_radius)) =
                world_circle_to_screen_circle(camera, camera_transform, world_center, world_radius)
            else {
                continue;
            };
            let d = screen_center.distance(cursor);
            if d > pixel_radius {
                continue;
            }
            let norm = d / pixel_radius.max(1.0);
            consider(
                SelectionCirclePick {
                    entity,
                    screen_center,
                    pixel_radius,
                    is_unit: false,
                },
                d,
                norm,
            );
        }
    }

    best.map(|(pick, _, _)| pick)
}

pub(crate) fn collect_in_rect<Uf, Bf>(
    rect_min: Vec2,
    rect_max: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    units: &Query<(Entity, &Transform, Option<&Collider>, &ObjectPrefabId), Uf>,
    builds: &Query<
        (
            Entity,
            &Transform,
            &AabbCollider,
            &BuildDimensions,
            &ObjectPrefabId,
        ),
        Bf,
    >,
    include_builds: bool,
) -> Vec<SelectionCirclePick>
where
    Uf: QueryFilter,
    Bf: QueryFilter,
{
    let mut picked = Vec::new();

    for (entity, transform, collider, prefab_id) in units.iter() {
        let world_center = unit_world_center(library, prefab_id.0, transform);
        let world_radius = unit_world_radius(library, prefab_id.0, transform, collider);
        let Some((screen_center, pixel_radius)) =
            world_circle_to_screen_circle(camera, camera_transform, world_center, world_radius)
        else {
            continue;
        };
        if !circle_intersects_rect(screen_center, pixel_radius, rect_min, rect_max) {
            continue;
        }
        picked.push(SelectionCirclePick {
            entity,
            screen_center,
            pixel_radius,
            is_unit: true,
        });
    }

    if include_builds {
        for (entity, transform, collider, dimensions, _prefab_id) in builds.iter() {
            let world_center = build_world_center(transform, dimensions);
            let world_radius = build_world_radius(collider);
            let Some((screen_center, pixel_radius)) =
                world_circle_to_screen_circle(camera, camera_transform, world_center, world_radius)
            else {
                continue;
            };
            if !circle_intersects_rect(screen_center, pixel_radius, rect_min, rect_max) {
                continue;
            }
            picked.push(SelectionCirclePick {
                entity,
                screen_center,
                pixel_radius,
                is_unit: false,
            });
        }
    }

    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_intersects_rect_hits_inside_center() {
        assert!(circle_intersects_rect(
            Vec2::new(5.0, 5.0),
            1.0,
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 10.0)
        ));
    }

    #[test]
    fn circle_intersects_rect_hits_edge_touch() {
        assert!(circle_intersects_rect(
            Vec2::new(5.0, 5.0),
            5.0,
            Vec2::new(10.0, 0.0),
            Vec2::new(20.0, 10.0)
        ));
    }

    #[test]
    fn circle_intersects_rect_misses_far() {
        assert!(!circle_intersects_rect(
            Vec2::new(0.0, 0.0),
            1.0,
            Vec2::new(10.0, 10.0),
            Vec2::new(20.0, 20.0)
        ));
    }

    #[test]
    fn pulse_radius_in_range() {
        let r0 = pulse_radius(10.0, 0.0);
        let r1 = pulse_radius(10.0, 1.0);
        assert!((r0 - 8.8).abs() < 1e-4);
        assert!((r1 - 11.2).abs() < 1e-4);
    }
}
