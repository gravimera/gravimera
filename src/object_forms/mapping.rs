use bevy::prelude::*;
use std::cmp::Ordering;

use super::{LeafKindKey, LeafMapping, LeafResolved};

const GLOBAL_ASSIGN_MAX_LEAVES: usize = 256;
const CLUSTER_LEAF_TARGET: usize = 32;

const COST_SCALE: f64 = 1_000_000.0;
const DUMMY_COST: i64 = (25.0 * COST_SCALE) as i64;

const KIND_MISMATCH_COST: f64 = 0.75;

const LEAF_W_POS_XZ: f64 = 1.0;
const LEAF_W_POS_Y: f64 = 3.0;
const LEAF_W_RADIAL: f64 = 0.65;
const LEAF_W_LOGV: f64 = 0.35;
const LEAF_W_SIDE: f64 = 0.12;
const LEAF_SIDE_THRESHOLD_01: f32 = 0.08;

const CLUSTER_W_POS_XZ: f64 = 1.0;
const CLUSTER_W_POS_Y: f64 = 2.5;
const CLUSTER_W_EXTENT: f64 = 0.50;
const CLUSTER_W_KIND: f64 = 0.25;

const EPS_EXTENT: f32 = 1e-4;

pub(super) fn build_leaf_mapping_v2_grouped(
    old: &[LeafResolved],
    new: &[LeafResolved],
) -> LeafMapping {
    if old.is_empty() || new.is_empty() {
        return LeafMapping { pairs: Vec::new() };
    }

    let old_features = compute_leaf_features(old);
    let new_features = compute_leaf_features(new);

    let n = old.len().max(new.len());
    if n <= GLOBAL_ASSIGN_MAX_LEAVES {
        return build_leaf_mapping_global(old, new, &old_features, &new_features);
    }

    build_leaf_mapping_clustered(old, new, &old_features, &new_features, CLUSTER_LEAF_TARGET)
}

fn build_leaf_mapping_global(
    old: &[LeafResolved],
    new: &[LeafResolved],
    old_features: &[LeafFeatures],
    new_features: &[LeafFeatures],
) -> LeafMapping {
    let old_indices: Vec<usize> = (0..old.len()).collect();
    let new_indices: Vec<usize> = (0..new.len()).collect();
    LeafMapping {
        pairs: assign_leaf_pairs(
            old,
            new,
            old_features,
            new_features,
            &old_indices,
            &new_indices,
        ),
    }
}

fn build_leaf_mapping_clustered(
    old: &[LeafResolved],
    new: &[LeafResolved],
    old_features: &[LeafFeatures],
    new_features: &[LeafFeatures],
    target_cluster_size: usize,
) -> LeafMapping {
    let old_clusters = build_leaf_clusters(old, old_features, target_cluster_size);
    let new_clusters = build_leaf_clusters(new, new_features, target_cluster_size);
    if old_clusters.is_empty() || new_clusters.is_empty() {
        return LeafMapping { pairs: Vec::new() };
    }

    let assignment = hungarian_min_cost(&build_cluster_cost_matrix(&old_clusters, &new_clusters));

    let mut pairs = Vec::new();
    for (old_cluster_idx, &new_cluster_idx) in assignment.iter().enumerate() {
        if old_cluster_idx >= old_clusters.len() {
            continue;
        }
        if new_cluster_idx >= new_clusters.len() {
            continue;
        }

        let old_indices = &old_clusters[old_cluster_idx].indices;
        let new_indices = &new_clusters[new_cluster_idx].indices;
        pairs.extend(assign_leaf_pairs(
            old,
            new,
            old_features,
            new_features,
            old_indices,
            new_indices,
        ));
    }

    LeafMapping { pairs }
}

fn assign_leaf_pairs(
    old: &[LeafResolved],
    new: &[LeafResolved],
    old_features: &[LeafFeatures],
    new_features: &[LeafFeatures],
    old_indices: &[usize],
    new_indices: &[usize],
) -> Vec<(usize, usize, bool)> {
    if old_indices.is_empty() || new_indices.is_empty() {
        return Vec::new();
    }

    let n = old_indices.len().max(new_indices.len());
    let mut costs = vec![vec![0i64; n]; n];
    for r in 0..n {
        for c in 0..n {
            costs[r][c] = match (r < old_indices.len(), c < new_indices.len()) {
                (true, true) => {
                    let oi = old_indices[r];
                    let ni = new_indices[c];
                    leaf_pair_cost(old, new, old_features, new_features, oi, ni, r, c)
                }
                (true, false) => DUMMY_COST,
                (false, true) => DUMMY_COST,
                (false, false) => 0,
            };
        }
    }

    let assignment = hungarian_min_cost(&costs);
    let mut out = Vec::new();
    for r in 0..old_indices.len() {
        let c = assignment[r];
        if c >= new_indices.len() {
            continue;
        }
        let oi = old_indices[r];
        let ni = new_indices[c];
        out.push((oi, ni, old[oi].key == new[ni].key));
    }
    out
}

fn leaf_pair_cost(
    old: &[LeafResolved],
    new: &[LeafResolved],
    old_features: &[LeafFeatures],
    new_features: &[LeafFeatures],
    old_idx: usize,
    new_idx: usize,
    tie_row: usize,
    tie_col: usize,
) -> i64 {
    let a = &old_features[old_idx];
    let b = &new_features[new_idx];

    let dx = (a.pos01.x - b.pos01.x) as f64;
    let dy = (a.pos01.y - b.pos01.y) as f64;
    let dz = (a.pos01.z - b.pos01.z) as f64;
    let pos_xz = dx * dx + dz * dz;
    let pos_y = dy * dy;

    let dr = (a.r - b.r) as f64;
    let radial = dr * dr;

    let logv = (a.log_v - b.log_v).abs() as f64;

    let mut side_flip = 0.0;
    if a.side_x.abs() > LEAF_SIDE_THRESHOLD_01 && b.side_x.abs() > LEAF_SIDE_THRESHOLD_01 {
        if a.side_x.signum() != b.side_x.signum() {
            side_flip += 1.0;
        }
    }
    if a.side_z.abs() > LEAF_SIDE_THRESHOLD_01 && b.side_z.abs() > LEAF_SIDE_THRESHOLD_01 {
        if a.side_z.signum() != b.side_z.signum() {
            side_flip += 1.0;
        }
    }

    let mut cost = 0.0;
    cost += LEAF_W_POS_XZ * pos_xz;
    cost += LEAF_W_POS_Y * pos_y;
    cost += LEAF_W_RADIAL * radial;
    cost += LEAF_W_LOGV * logv;
    cost += LEAF_W_SIDE * side_flip;

    if old[old_idx].key != new[new_idx].key {
        cost += KIND_MISMATCH_COST;
    }

    let tie = (tie_row as i64) + (tie_col as i64);
    ((cost * COST_SCALE).round() as i64).saturating_add(tie)
}

#[derive(Clone, Copy, Debug)]
struct LeafFeatures {
    pos01: Vec3,
    r: f32,
    log_v: f32,
    side_x: f32,
    side_z: f32,
}

fn compute_leaf_features(leaves: &[LeafResolved]) -> Vec<LeafFeatures> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for leaf in leaves.iter() {
        let p = leaf.transform.translation;
        min = min.min(p);
        max = max.max(p);
    }

    let mut ext = max - min;
    ext.x = ext.x.abs().max(EPS_EXTENT);
    ext.y = ext.y.abs().max(EPS_EXTENT);
    ext.z = ext.z.abs().max(EPS_EXTENT);
    let inv_ext = Vec3::new(1.0 / ext.x, 1.0 / ext.y, 1.0 / ext.z);

    let mut out = Vec::with_capacity(leaves.len());
    for leaf in leaves.iter() {
        let pos = leaf.transform.translation;
        let mut pos01 = (pos - min) * inv_ext;
        if !pos01.is_finite() {
            pos01 = Vec3::splat(0.5);
        }

        let center = pos01 - Vec3::splat(0.5);
        let r = Vec2::new(center.x, center.z).length();

        let s = leaf.transform.scale;
        let v = (s.x.abs() * s.y.abs() * s.z.abs()).max(EPS_EXTENT);
        let log_v = v.ln();

        out.push(LeafFeatures {
            pos01,
            r,
            log_v,
            side_x: center.x,
            side_z: center.z,
        });
    }
    out
}

#[derive(Clone, Debug)]
struct LeafCluster {
    indices: Vec<usize>,
    summary: LeafClusterSummary,
}

#[derive(Clone, Copy, Debug)]
struct LeafClusterSummary {
    centroid01: Vec3,
    extent01: Vec3,
    mesh_hist: [u16; 12],
    model_count: u16,
    leaf_count: u16,
}

fn build_leaf_clusters(
    leaves: &[LeafResolved],
    features: &[LeafFeatures],
    target_cluster_size: usize,
) -> Vec<LeafCluster> {
    if leaves.is_empty() {
        return Vec::new();
    }

    let mut out_groups: Vec<Vec<usize>> = Vec::new();
    let mut root: Vec<usize> = (0..leaves.len()).collect();
    split_indices_median(
        &mut root,
        features,
        target_cluster_size.max(1),
        &mut out_groups,
    );

    let mut out = Vec::with_capacity(out_groups.len());
    for indices in out_groups.into_iter() {
        let summary = summarize_cluster(leaves, features, &indices);
        out.push(LeafCluster { indices, summary });
    }
    out
}

fn split_indices_median(
    indices: &mut Vec<usize>,
    features: &[LeafFeatures],
    target: usize,
    out: &mut Vec<Vec<usize>>,
) {
    if indices.len() <= target {
        out.push(std::mem::take(indices));
        return;
    }

    let axis = choose_split_axis(indices, features);
    indices.sort_by(|a, b| compare_pos01(*a, *b, features, axis));
    let mid = indices.len() / 2;
    let mut right = indices.split_off(mid);
    let mut left = std::mem::take(indices);

    split_indices_median(&mut left, features, target, out);
    split_indices_median(&mut right, features, target, out);
}

fn choose_split_axis(indices: &[usize], features: &[LeafFeatures]) -> usize {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for &idx in indices.iter() {
        let p = features[idx].pos01;
        min = min.min(p);
        max = max.max(p);
    }
    let range = max - min;

    // Pick the largest range; tie-break: prefer Y (support/head), then X, then Z.
    let ranges = [range.x.abs(), range.y.abs(), range.z.abs()];
    let mut best_axis = 1usize;
    let mut best_range = ranges[1];
    let mut best_priority = 2u8; // y

    for axis in 0..3 {
        let r = ranges[axis];
        let priority = match axis {
            1 => 2, // y
            0 => 1, // x
            _ => 0, // z
        };
        let better =
            r > best_range + 1e-6 || ((r - best_range).abs() <= 1e-6 && priority > best_priority);
        if better {
            best_axis = axis;
            best_range = r;
            best_priority = priority;
        }
    }

    best_axis
}

fn compare_pos01(a: usize, b: usize, features: &[LeafFeatures], axis: usize) -> Ordering {
    let pa = features[a].pos01;
    let pb = features[b].pos01;
    let ca = match axis {
        0 => pa.x,
        1 => pa.y,
        _ => pa.z,
    };
    let cb = match axis {
        0 => pb.x,
        1 => pb.y,
        _ => pb.z,
    };

    ca.partial_cmp(&cb)
        .unwrap_or(Ordering::Equal)
        .then_with(|| pa.y.partial_cmp(&pb.y).unwrap_or(Ordering::Equal))
        .then_with(|| pa.x.partial_cmp(&pb.x).unwrap_or(Ordering::Equal))
        .then_with(|| pa.z.partial_cmp(&pb.z).unwrap_or(Ordering::Equal))
        .then_with(|| a.cmp(&b))
}

fn summarize_cluster(
    leaves: &[LeafResolved],
    features: &[LeafFeatures],
    indices: &[usize],
) -> LeafClusterSummary {
    let count = indices.len().max(1) as f32;

    let mut sum = Vec3::ZERO;
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    let mut mesh_hist = [0u16; 12];
    let mut model_count = 0u16;

    for &idx in indices.iter() {
        let p = features[idx].pos01;
        sum += p;
        min = min.min(p);
        max = max.max(p);

        match &leaves[idx].key {
            LeafKindKey::Primitive(mesh_key) => {
                let rank = LeafKindKey::mesh_rank(*mesh_key) as usize;
                if rank < mesh_hist.len() {
                    mesh_hist[rank] = mesh_hist[rank].saturating_add(1);
                }
            }
            LeafKindKey::Model(_) => {
                model_count = model_count.saturating_add(1);
            }
        }
    }

    let centroid01 = sum / count;
    let extent01 = (max - min).abs();

    LeafClusterSummary {
        centroid01,
        extent01,
        mesh_hist,
        model_count,
        leaf_count: indices.len().min(u16::MAX as usize) as u16,
    }
}

fn build_cluster_cost_matrix(old: &[LeafCluster], new: &[LeafCluster]) -> Vec<Vec<i64>> {
    let n = old.len().max(new.len());
    let mut costs = vec![vec![0i64; n]; n];
    for r in 0..n {
        for c in 0..n {
            costs[r][c] = match (r < old.len(), c < new.len()) {
                (true, true) => cluster_pair_cost(&old[r].summary, &new[c].summary, r, c),
                (true, false) => DUMMY_COST,
                (false, true) => DUMMY_COST,
                (false, false) => 0,
            };
        }
    }
    costs
}

fn cluster_pair_cost(
    a: &LeafClusterSummary,
    b: &LeafClusterSummary,
    tie_row: usize,
    tie_col: usize,
) -> i64 {
    let dx = (a.centroid01.x - b.centroid01.x) as f64;
    let dy = (a.centroid01.y - b.centroid01.y) as f64;
    let dz = (a.centroid01.z - b.centroid01.z) as f64;
    let pos_xz = dx * dx + dz * dz;
    let pos_y = dy * dy;

    let ex = (a.extent01.x - b.extent01.x) as f64;
    let ey = (a.extent01.y - b.extent01.y) as f64;
    let ez = (a.extent01.z - b.extent01.z) as f64;
    let extent = ex * ex + ey * ey + ez * ez;

    let mut kind_diff = 0.0;
    for i in 0..a.mesh_hist.len() {
        kind_diff += (a.mesh_hist[i] as f64 - b.mesh_hist[i] as f64).abs();
    }
    kind_diff += (a.model_count as f64 - b.model_count as f64).abs();
    let norm = (a.leaf_count as f64 + b.leaf_count as f64).max(1.0);
    kind_diff /= norm;

    let mut cost = 0.0;
    cost += CLUSTER_W_POS_XZ * pos_xz;
    cost += CLUSTER_W_POS_Y * pos_y;
    cost += CLUSTER_W_EXTENT * extent;
    cost += CLUSTER_W_KIND * kind_diff;

    let tie = (tie_row as i64) + (tie_col as i64);
    ((cost * COST_SCALE).round() as i64).saturating_add(tie)
}

fn hungarian_min_cost(costs: &[Vec<i64>]) -> Vec<usize> {
    let n = costs.len();
    if n == 0 {
        return Vec::new();
    }
    debug_assert!(costs.iter().all(|row| row.len() == n));

    // Classic Hungarian algorithm (minimization), 1-based indexing.
    let mut u = vec![0i64; n + 1];
    let mut v = vec![0i64; n + 1];
    let mut p = vec![0usize; n + 1];
    let mut way = vec![0usize; n + 1];

    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![i64::MAX; n + 1];
        let mut used = vec![false; n + 1];

        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = i64::MAX;
            let mut j1 = 0usize;

            for j in 1..=n {
                if used[j] {
                    continue;
                }
                let cur = costs[i0 - 1][j - 1] - u[i0] - v[j];
                if cur < minv[j] {
                    minv[j] = cur;
                    way[j] = j0;
                }
                if minv[j] < delta {
                    delta = minv[j];
                    j1 = j;
                }
            }

            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }

            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }

        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }

    let mut assignment = vec![0usize; n];
    for j in 1..=n {
        let row = p[j];
        if row == 0 {
            continue;
        }
        assignment[row - 1] = j - 1;
    }
    assignment
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::MeshKey;

    fn leaf(key: MeshKey, translation: Vec3) -> LeafResolved {
        LeafResolved {
            key: LeafKindKey::Primitive(key),
            transform: Transform::from_translation(translation),
            mirrored: false,
            spawn: super::super::LeafSpawnKind::Mesh {
                mesh: Handle::default(),
                material_proto: StandardMaterial::default(),
                base_color: LinearRgba {
                    red: 1.0,
                    green: 1.0,
                    blue: 1.0,
                    alpha: 1.0,
                },
            },
        }
    }

    #[test]
    fn hungarian_finds_expected_min_assignment() {
        let costs = vec![vec![4, 1, 3], vec![2, 0, 5], vec![3, 2, 2]];
        let a = hungarian_min_cost(&costs);
        assert_eq!(a, vec![1, 0, 2]);
    }

    #[test]
    fn global_mapping_prefers_nearby_pairs_and_is_deterministic() {
        let old = vec![
            leaf(MeshKey::UnitCube, Vec3::new(-1.0, 0.0, 0.0)),
            leaf(MeshKey::UnitCube, Vec3::new(1.0, 0.0, 0.0)),
            leaf(MeshKey::UnitCube, Vec3::new(0.0, 1.0, 0.0)),
        ];
        let new = vec![
            leaf(MeshKey::UnitCube, Vec3::new(0.0, 1.0, 0.0)),
            leaf(MeshKey::UnitCube, Vec3::new(-1.0, 0.0, 0.0)),
            leaf(MeshKey::UnitCube, Vec3::new(1.0, 0.0, 0.0)),
        ];

        let old_features = compute_leaf_features(&old);
        let new_features = compute_leaf_features(&new);
        let a = build_leaf_mapping_global(&old, &new, &old_features, &new_features);
        let b = build_leaf_mapping_global(&old, &new, &old_features, &new_features);
        assert_eq!(a.pairs, b.pairs);

        assert_eq!(a.pairs, vec![(0, 1, true), (1, 2, true), (2, 0, true)]);
    }

    #[test]
    fn clustered_mapping_runs_and_is_deterministic() {
        let mut old = Vec::new();
        let mut new = Vec::new();
        for i in 0..96 {
            let x = (i % 8) as f32;
            let y = (i / 8) as f32;
            old.push(leaf(MeshKey::UnitCube, Vec3::new(x, y, 0.0)));
            // Swap x/y in new to ensure the mapping has to work.
            new.push(leaf(MeshKey::UnitCube, Vec3::new(y, x, 0.0)));
        }

        let old_features = compute_leaf_features(&old);
        let new_features = compute_leaf_features(&new);

        let a = build_leaf_mapping_clustered(&old, &new, &old_features, &new_features, 16);
        let b = build_leaf_mapping_clustered(&old, &new, &old_features, &new_features, 16);
        assert_eq!(a.pairs, b.pairs);
        assert_eq!(a.pairs.len(), 96);
    }
}
