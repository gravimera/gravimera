use bevy::prelude::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::collections::{HashMap, HashSet};

use crate::constants::*;
use crate::geometry::circle_intersects_aabb_xz;
use crate::object::registry::MovementBlockRule;

#[derive(Clone, Copy)]
pub(crate) struct NavObstacle {
    pub(crate) movement_block: Option<MovementBlockRule>,
    pub(crate) supports_standing: bool,
    pub(crate) center: Vec2,
    pub(crate) half: Vec2,
    pub(crate) bottom_y: f32,
    pub(crate) top_y: f32,
}

fn quantize_units(value: f32, unit: f32) -> i32 {
    if unit <= 1e-6 {
        return 0;
    }
    (value / unit).round() as i32
}

fn units_to_world(units: i32, unit: f32) -> f32 {
    units as f32 * unit
}

fn blocked_at(
    pos: Vec2,
    radius: f32,
    ground_y: f32,
    character_height: f32,
    obstacles: &[NavObstacle],
) -> bool {
    obstacles.iter().any(|ob| {
        let Some(rule) = ob.movement_block else {
            return false;
        };
        match rule {
            MovementBlockRule::Always => circle_intersects_aabb_xz(pos, radius, ob.center, ob.half),
            MovementBlockRule::UpperBodyFraction(fraction) => {
                let plane_y = ground_y + character_height * fraction;
                ob.top_y > plane_y
                    && ob.bottom_y < plane_y
                    && circle_intersects_aabb_xz(pos, radius, ob.center, ob.half)
            }
        }
    })
}

fn next_ground_y(
    pos: Vec2,
    radius: f32,
    ground_y: f32,
    character_height: f32,
    obstacles: &[NavObstacle],
) -> f32 {
    let mut best = 0.0f32;
    for ob in obstacles {
        if !ob.supports_standing {
            continue;
        }
        let Some(rule) = ob.movement_block else {
            continue;
        };

        let plane_y = match rule {
            MovementBlockRule::Always => ground_y,
            MovementBlockRule::UpperBodyFraction(fraction) => {
                ground_y + character_height * fraction
            }
        };
        if ob.top_y > plane_y {
            continue;
        }
        if circle_intersects_aabb_xz(pos, radius, ob.center, ob.half) {
            best = best.max(ob.top_y);
        }
    }
    best
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct NodeKey {
    cell: IVec2,
    ground_units: i32,
}

#[derive(Clone, Copy)]
struct Open3dNode {
    f: f32,
    g: f32,
    key: NodeKey,
}

impl PartialEq for Open3dNode {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for Open3dNode {}

impl PartialOrd for Open3dNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Open3dNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f
            .total_cmp(&self.f)
            .then_with(|| other.g.total_cmp(&self.g))
            .then_with(|| self.key.cell.x.cmp(&other.key.cell.x))
            .then_with(|| self.key.cell.y.cmp(&other.key.cell.y))
            .then_with(|| self.key.ground_units.cmp(&other.key.ground_units))
    }
}

fn heuristic_world(pos: Vec2, goal: Vec2) -> f32 {
    (goal - pos).length()
}

pub(crate) fn find_path_height_aware(
    start_world: Vec2,
    start_ground_y: f32,
    goal_world: Vec2,
    goal_ground_y: f32,
    radius: f32,
    character_height: f32,
    world_half_size: f32,
    grid: f32,
    obstacles: &[NavObstacle],
    is_walkable: &impl Fn(Vec2) -> bool,
) -> Option<Vec<Vec2>> {
    if grid <= 0.001 {
        return None;
    }

    let min = Vec2::splat(-world_half_size + radius);
    let max = Vec2::splat(world_half_size - radius);
    let start_world = start_world.clamp(min, max);
    let goal_world = goal_world.clamp(min, max);

    let start_ground_units = quantize_units(start_ground_y.max(0.0), NAV_HEIGHT_QUANT_SIZE).max(0);
    let goal_ground_units = quantize_units(goal_ground_y.max(0.0), NAV_HEIGHT_QUANT_SIZE).max(0);

    if !is_walkable(start_world) || !is_walkable(goal_world) {
        return None;
    }

    if blocked_at(
        goal_world,
        radius,
        units_to_world(goal_ground_units, NAV_HEIGHT_QUANT_SIZE),
        character_height,
        obstacles,
    ) {
        return None;
    }

    let min_cell = IVec2::new((min.x / grid).ceil() as i32, (min.y / grid).ceil() as i32);
    let max_cell = IVec2::new((max.x / grid).floor() as i32, (max.y / grid).floor() as i32);
    if min_cell.x > max_cell.x || min_cell.y > max_cell.y {
        return None;
    }

    let start_cell = to_cell(start_world, grid);
    let start_cell = IVec2::new(
        start_cell.x.clamp(min_cell.x, max_cell.x),
        start_cell.y.clamp(min_cell.y, max_cell.y),
    );

    let start_key = NodeKey {
        cell: start_cell,
        ground_units: start_ground_units,
    };

    let mut open = BinaryHeap::new();
    open.push(Open3dNode {
        f: heuristic_world(cell_center(start_cell, grid), goal_world),
        g: 0.0,
        key: start_key,
    });

    let mut g_score: HashMap<NodeKey, f32> = HashMap::new();
    g_score.insert(start_key, 0.0);
    let mut came_from: HashMap<NodeKey, NodeKey> = HashMap::new();
    let mut closed: HashSet<NodeKey> = HashSet::new();

    let neighbors: [(i32, i32, f32); 8] = [
        (-1, 0, 1.0),
        (1, 0, 1.0),
        (0, -1, 1.0),
        (0, 1, 1.0),
        (-1, -1, 1.4142135),
        (-1, 1, 1.4142135),
        (1, -1, 1.4142135),
        (1, 1, 1.4142135),
    ];

    let goal_threshold = grid * 0.75;
    while let Some(node) = open.pop() {
        if closed.contains(&node.key) {
            continue;
        }
        closed.insert(node.key);

        let node_world = cell_center(node.key.cell, grid);
        if node.key.ground_units == goal_ground_units
            && (node_world - goal_world).length() <= goal_threshold
        {
            let mut keys: Vec<NodeKey> = Vec::new();
            let mut current = node.key;
            keys.push(current);
            while let Some(prev) = came_from.get(&current).copied() {
                current = prev;
                keys.push(current);
            }
            keys.reverse();

            let mut path: Vec<Vec2> = keys.iter().map(|key| cell_center(key.cell, grid)).collect();

            if path.len() >= 2 && (path[0] - start_world).length() <= grid * 0.75 {
                path.remove(0);
            }
            if let Some(last) = path.last().copied() {
                if (last - goal_world).length() > 1e-3 {
                    path.push(goal_world);
                }
            } else {
                path.push(goal_world);
            }

            return Some(path);
        }

        let current_ground_y = units_to_world(node.key.ground_units, NAV_HEIGHT_QUANT_SIZE);
        for (dx, dy, step_units) in neighbors {
            let next_cell = IVec2::new(node.key.cell.x + dx, node.key.cell.y + dy);
            if !in_bounds(next_cell, min_cell, max_cell) {
                continue;
            }

            if dx != 0 && dy != 0 {
                let a_cell = IVec2::new(node.key.cell.x + dx, node.key.cell.y);
                let b_cell = IVec2::new(node.key.cell.x, node.key.cell.y + dy);
                if in_bounds(a_cell, min_cell, max_cell) {
                    let a_pos = cell_center(a_cell, grid);
                    if !is_walkable(a_pos)
                        || blocked_at(a_pos, radius, current_ground_y, character_height, obstacles)
                    {
                        continue;
                    }
                }
                if in_bounds(b_cell, min_cell, max_cell) {
                    let b_pos = cell_center(b_cell, grid);
                    if !is_walkable(b_pos)
                        || blocked_at(b_pos, radius, current_ground_y, character_height, obstacles)
                    {
                        continue;
                    }
                }
            }

            let next_world = cell_center(next_cell, grid);
            if !is_walkable(next_world) {
                continue;
            }
            if blocked_at(
                next_world,
                radius,
                current_ground_y,
                character_height,
                obstacles,
            ) {
                continue;
            }

            let next_ground = next_ground_y(
                next_world,
                radius,
                current_ground_y,
                character_height,
                obstacles,
            );
            let next_units = quantize_units(next_ground.max(0.0), NAV_HEIGHT_QUANT_SIZE).max(0);
            let next_key = NodeKey {
                cell: next_cell,
                ground_units: next_units,
            };

            if closed.contains(&next_key) {
                continue;
            }

            let tentative_g = node.g + step_units * grid;
            let best = g_score.get(&next_key).copied().unwrap_or(f32::INFINITY);
            if tentative_g >= best {
                continue;
            }

            came_from.insert(next_key, node.key);
            g_score.insert(next_key, tentative_g);
            open.push(Open3dNode {
                g: tentative_g,
                f: tentative_g + heuristic_world(next_world, goal_world),
                key: next_key,
            });
        }
    }

    None
}

fn walk_segment_height_aware(
    from: Vec2,
    mut ground_y: f32,
    to: Vec2,
    radius: f32,
    character_height: f32,
    grid: f32,
    obstacles: &[NavObstacle],
    is_walkable: &impl Fn(Vec2) -> bool,
) -> Option<f32> {
    let delta = to - from;
    let dist = delta.length();
    if dist <= 1e-6 {
        return Some(ground_y);
    }

    // Conservative sampling: smaller than grid to catch narrow blockers.
    let step = (grid * 0.25).max(radius * 0.5).clamp(0.05, grid.max(0.05));
    let steps = ((dist / step).ceil() as u32).max(1).min(512);
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let pos = from + delta * t;
        if !is_walkable(pos) {
            return None;
        }
        if blocked_at(pos, radius, ground_y, character_height, obstacles) {
            return None;
        }
        ground_y = next_ground_y(pos, radius, ground_y, character_height, obstacles);
    }

    Some(ground_y)
}

/// Simplify a grid-based path to reduce zig-zagging (visual facing jitter) by "string pulling":
/// repeatedly keep the farthest waypoint that remains traversable from the current anchor.
///
/// This is conservative: it uses the same height-aware blocker rules as pathfinding by sampling
/// points along each candidate segment.
pub(crate) fn smooth_path_height_aware(
    start_world: Vec2,
    start_ground_y: f32,
    mut path: Vec<Vec2>,
    radius: f32,
    character_height: f32,
    grid: f32,
    obstacles: &[NavObstacle],
    is_walkable: &impl Fn(Vec2) -> bool,
) -> Vec<Vec2> {
    if path.len() <= 2 {
        return path;
    }

    // Drop consecutive near-duplicates to avoid degeneracy.
    let eps2 = (grid * 0.05).max(0.01).powi(2);
    let mut deduped: Vec<Vec2> = Vec::with_capacity(path.len());
    for p in path.drain(..) {
        if deduped
            .last()
            .map(|last| (p - *last).length_squared() <= eps2)
            .unwrap_or(false)
        {
            continue;
        }
        deduped.push(p);
    }

    if deduped.len() <= 2 {
        return deduped;
    }

    let mut out: Vec<Vec2> = Vec::with_capacity(deduped.len());
    let mut anchor = start_world;
    let mut ground_y = start_ground_y.max(0.0);
    let mut idx: usize = 0;

    while idx < deduped.len() {
        let mut best = idx;
        let mut best_ground_y: Option<f32> = None;

        // Find the farthest reachable waypoint from the current anchor.
        for j in (idx..deduped.len()).rev() {
            if let Some(end_ground_y) = walk_segment_height_aware(
                anchor,
                ground_y,
                deduped[j],
                radius,
                character_height,
                grid,
                obstacles,
                is_walkable,
            ) {
                best = j;
                best_ground_y = Some(end_ground_y);
                break;
            }
        }

        // Fallback: keep the next waypoint even if smoothing couldn't validate (should be rare).
        let end_ground_y = best_ground_y.unwrap_or(ground_y);
        let next = deduped[best];
        out.push(next);
        anchor = next;
        ground_y = end_ground_y;
        idx = best + 1;
    }

    out
}

fn cell_center(cell: IVec2, grid: f32) -> Vec2 {
    Vec2::new(cell.x as f32 * grid, cell.y as f32 * grid)
}

fn to_cell(pos: Vec2, grid: f32) -> IVec2 {
    IVec2::new((pos.x / grid).round() as i32, (pos.y / grid).round() as i32)
}

fn in_bounds(cell: IVec2, min: IVec2, max: IVec2) -> bool {
    cell.x >= min.x && cell.x <= max.x && cell.y >= min.y && cell.y <= max.y
}
