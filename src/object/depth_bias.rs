use bevy::prelude::*;

use crate::object::registry::{
    MeshKey, ObjectPartDef, ObjectPartKind, PrimitiveParams, PrimitiveVisualDef,
};

const NORMAL_DOT_THRESHOLD: f32 = 0.9995;
const PLANE_DISTANCE_EPS: f32 = 1e-4;
const OVERLAP_EPS: f32 = 1e-5;
const MAX_DEPTH_BIAS: i32 = 8;
const MAX_RENDER_PRIORITY_ABS: i32 = 3;
const RENDER_PRIORITY_DEPTH_BIAS_STEP: i32 = 2;
const MAX_TOTAL_DEPTH_BIAS_ABS: i32 = 32;

pub(crate) fn depth_bias_delta_from_render_priority(render_priority: Option<i32>) -> i32 {
    let Some(render_priority) = render_priority else {
        return 0;
    };
    render_priority
        .clamp(-MAX_RENDER_PRIORITY_ABS, MAX_RENDER_PRIORITY_ABS)
        .saturating_mul(RENDER_PRIORITY_DEPTH_BIAS_STEP)
}

pub(crate) fn clamp_depth_bias(depth_bias: i32) -> i32 {
    depth_bias.clamp(-MAX_TOTAL_DEPTH_BIAS_ABS, MAX_TOTAL_DEPTH_BIAS_ABS)
}

#[derive(Clone, Copy, Debug)]
struct PlanarFace {
    part_index: usize,
    normal: Vec3,
    center: Vec3,
    axis_u: Vec3,
    axis_v: Vec3,
    half_u: f32,
    half_v: f32,
    area: f32,
    thickness_half: f32,
}

#[derive(Default)]
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        let mut out = Self::default();
        out.parent = (0..n).collect();
        out.rank = vec![0; n];
        out
    }

    fn find(&mut self, x: usize) -> usize {
        let p = self.parent[x];
        if p == x {
            return x;
        }
        let root = self.find(p);
        self.parent[x] = root;
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        let rank_a = self.rank[ra];
        let rank_b = self.rank[rb];
        if rank_a < rank_b {
            self.parent[ra] = rb;
        } else if rank_a > rank_b {
            self.parent[rb] = ra;
        } else {
            self.parent[rb] = ra;
            self.rank[ra] = rank_a.saturating_add(1);
        }
    }
}

pub(crate) fn compute_primitive_part_depth_biases(parts: &[ObjectPartDef]) -> Vec<i32> {
    let mut out = vec![0i32; parts.len()];

    let mut faces: Vec<PlanarFace> = Vec::new();
    for (part_index, part) in parts.iter().enumerate() {
        let ObjectPartKind::Primitive { primitive } = &part.kind else {
            continue;
        };
        let (mesh, params) = primitive_mesh_and_params(primitive);
        faces.extend(planar_faces_for_primitive(
            part_index,
            mesh,
            params,
            part.transform,
        ));
    }

    if faces.len() < 2 {
        return out;
    }

    let mut uf = UnionFind::new(faces.len());
    for i in 0..faces.len() {
        for j in (i + 1)..faces.len() {
            let a = faces[i];
            let b = faces[j];
            if a.part_index == b.part_index {
                continue;
            }
            let dot = a.normal.dot(b.normal);
            if !dot.is_finite() || dot < NORMAL_DOT_THRESHOLD {
                continue;
            }
            let plane_delta = a.normal.dot(b.center - a.center).abs();
            if !plane_delta.is_finite() || plane_delta > PLANE_DISTANCE_EPS {
                continue;
            }
            if !planar_faces_overlap(&a, &b) {
                continue;
            }
            uf.union(i, j);
        }
    }

    let mut groups: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for idx in 0..faces.len() {
        let root = uf.find(idx);
        groups.entry(root).or_default().push(idx);
    }

    for face_indices in groups.values() {
        if face_indices.len() < 2 {
            continue;
        }

        let mut unique_parts: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for &fi in face_indices {
            unique_parts.insert(faces[fi].part_index);
        }
        if unique_parts.len() < 2 {
            continue;
        }

        let mut group_faces: Vec<PlanarFace> = face_indices.iter().map(|&i| faces[i]).collect();
        group_faces.sort_by(|a, b| {
            b.area
                .total_cmp(&a.area)
                .then_with(|| b.thickness_half.total_cmp(&a.thickness_half))
                .then_with(|| a.part_index.cmp(&b.part_index))
        });

        let k = group_faces.len().max(1);
        let denom = (k as i32 - 1).max(1);
        for (pos, face) in group_faces.iter().enumerate() {
            let pos = pos as i32;
            let desired = if k as i32 <= MAX_DEPTH_BIAS + 1 {
                pos
            } else {
                (pos * MAX_DEPTH_BIAS) / denom
            };
            if desired <= 0 {
                continue;
            }
            if let Some(slot) = out.get_mut(face.part_index) {
                *slot = (*slot).max(desired);
            }
        }
    }

    out
}

fn primitive_mesh_and_params(
    primitive: &PrimitiveVisualDef,
) -> (MeshKey, Option<&PrimitiveParams>) {
    match primitive {
        PrimitiveVisualDef::Mesh { mesh, .. } => (*mesh, None),
        PrimitiveVisualDef::Primitive { mesh, params, .. } => (*mesh, params.as_ref()),
    }
}

fn planar_faces_for_primitive(
    part_index: usize,
    mesh: MeshKey,
    params: Option<&PrimitiveParams>,
    transform: Transform,
) -> Vec<PlanarFace> {
    let translation = transform.translation;
    let rotation = transform.rotation;
    let scale = transform.scale;
    if !translation.is_finite() || !rotation.is_finite() || !scale.is_finite() {
        return Vec::new();
    }

    let half = scale.abs() * 0.5;
    if half.max_element() <= 1e-6 {
        return Vec::new();
    }

    let right = rotation * Vec3::X;
    let up = rotation * Vec3::Y;
    let forward = rotation * Vec3::Z;
    if !right.is_finite() || !up.is_finite() || !forward.is_finite() {
        return Vec::new();
    }

    let mut out = Vec::new();
    match mesh {
        MeshKey::UnitCube => {
            out.extend(planar_faces_box(
                part_index,
                translation,
                right,
                up,
                forward,
                half,
            ));
        }
        MeshKey::UnitCylinder => {
            out.extend(planar_faces_caps(
                part_index,
                translation,
                right,
                up,
                forward,
                half,
                CapShape::Ellipse,
                CapCount::Two,
            ));
        }
        MeshKey::UnitConicalFrustum => {
            // Approximate with the maximal radius for ordering (good enough for depth ordering).
            let _ = params;
            out.extend(planar_faces_caps(
                part_index,
                translation,
                right,
                up,
                forward,
                half,
                CapShape::Ellipse,
                CapCount::Two,
            ));
        }
        MeshKey::UnitCone => {
            out.extend(planar_faces_caps(
                part_index,
                translation,
                right,
                up,
                forward,
                half,
                CapShape::Ellipse,
                CapCount::BaseOnly,
            ));
        }
        // These shapes do not have stable planar faces that are likely to be authored coplanar.
        MeshKey::UnitSphere | MeshKey::UnitTorus | MeshKey::UnitCapsule => {}
        // Fallback: treat unknown meshes as a box.
        _ => {
            out.extend(planar_faces_box(
                part_index,
                translation,
                right,
                up,
                forward,
                half,
            ));
        }
    }

    out
}

fn planar_faces_overlap(a: &PlanarFace, b: &PlanarFace) -> bool {
    let rel = b.center - a.center;
    let b_center = Vec2::new(rel.dot(a.axis_u), rel.dot(a.axis_v));

    let mut b_u = Vec2::new(b.axis_u.dot(a.axis_u), b.axis_u.dot(a.axis_v));
    if b_u.length_squared() <= 1e-8 {
        return false;
    }
    b_u = b_u.normalize();

    let mut b_v = Vec2::new(b.axis_v.dot(a.axis_u), b.axis_v.dot(a.axis_v));
    b_v -= b_u * b_v.dot(b_u);
    if b_v.length_squared() <= 1e-8 {
        return false;
    }
    b_v = b_v.normalize();

    obb2_intersects(
        Vec2::ZERO,
        Vec2::new(1.0, 0.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(a.half_u, a.half_v),
        b_center,
        b_u,
        b_v,
        Vec2::new(b.half_u, b.half_v),
    )
}

fn obb2_intersects(
    a_center: Vec2,
    a_u: Vec2,
    a_v: Vec2,
    a_half: Vec2,
    b_center: Vec2,
    b_u: Vec2,
    b_v: Vec2,
    b_half: Vec2,
) -> bool {
    let delta = b_center - a_center;
    let axes = [a_u, a_v, b_u, b_v];
    for axis in axes {
        let axis = if axis.length_squared() > 1e-8 {
            axis.normalize()
        } else {
            continue;
        };
        let dist = delta.dot(axis).abs();
        let r_a = a_half.x * axis.dot(a_u).abs() + a_half.y * axis.dot(a_v).abs();
        let r_b = b_half.x * axis.dot(b_u).abs() + b_half.y * axis.dot(b_v).abs();
        if dist > r_a + r_b + OVERLAP_EPS {
            return false;
        }
    }
    true
}

#[derive(Clone, Copy, Debug)]
enum CapCount {
    Two,
    BaseOnly,
}

#[derive(Clone, Copy, Debug)]
enum CapShape {
    Ellipse,
}

fn planar_faces_box(
    part_index: usize,
    translation: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    half: Vec3,
) -> Vec<PlanarFace> {
    let mut out = Vec::with_capacity(6);
    // +X / -X faces
    out.push(face_rect(
        part_index,
        right,
        translation + right * half.x,
        up,
        forward,
        half.y,
        half.z,
        half.x,
    ));
    out.push(face_rect(
        part_index,
        -right,
        translation - right * half.x,
        up,
        forward,
        half.y,
        half.z,
        half.x,
    ));
    // +Y / -Y faces
    out.push(face_rect(
        part_index,
        up,
        translation + up * half.y,
        right,
        forward,
        half.x,
        half.z,
        half.y,
    ));
    out.push(face_rect(
        part_index,
        -up,
        translation - up * half.y,
        right,
        forward,
        half.x,
        half.z,
        half.y,
    ));
    // +Z / -Z faces
    out.push(face_rect(
        part_index,
        forward,
        translation + forward * half.z,
        right,
        up,
        half.x,
        half.y,
        half.z,
    ));
    out.push(face_rect(
        part_index,
        -forward,
        translation - forward * half.z,
        right,
        up,
        half.x,
        half.y,
        half.z,
    ));
    out
}

fn planar_faces_caps(
    part_index: usize,
    translation: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    half: Vec3,
    shape: CapShape,
    count: CapCount,
) -> Vec<PlanarFace> {
    let mut out = Vec::with_capacity(2);
    match count {
        CapCount::Two => {
            // Top cap
            out.push(face_cap(
                part_index,
                up,
                translation + up * half.y,
                right,
                forward,
                half.x,
                half.z,
                half.y,
                shape,
            ));
            // Bottom cap
            out.push(face_cap(
                part_index,
                -up,
                translation - up * half.y,
                right,
                forward,
                half.x,
                half.z,
                half.y,
                shape,
            ));
        }
        CapCount::BaseOnly => {
            out.push(face_cap(
                part_index,
                -up,
                translation - up * half.y,
                right,
                forward,
                half.x,
                half.z,
                half.y,
                shape,
            ));
        }
    }
    out
}

fn face_rect(
    part_index: usize,
    normal: Vec3,
    center: Vec3,
    axis_u: Vec3,
    axis_v: Vec3,
    half_u: f32,
    half_v: f32,
    thickness_half: f32,
) -> PlanarFace {
    let area = 4.0 * half_u.abs().max(0.0) * half_v.abs().max(0.0);
    PlanarFace {
        part_index,
        normal: normalize_or_zero(normal),
        center,
        axis_u: normalize_or_zero(axis_u),
        axis_v: normalize_or_zero(axis_v),
        half_u: half_u.abs(),
        half_v: half_v.abs(),
        area,
        thickness_half: thickness_half.abs(),
    }
}

fn face_cap(
    part_index: usize,
    normal: Vec3,
    center: Vec3,
    axis_u: Vec3,
    axis_v: Vec3,
    half_u: f32,
    half_v: f32,
    thickness_half: f32,
    shape: CapShape,
) -> PlanarFace {
    let half_u = half_u.abs();
    let half_v = half_v.abs();
    let area = match shape {
        CapShape::Ellipse => std::f32::consts::PI * half_u.max(0.0) * half_v.max(0.0),
    };
    PlanarFace {
        part_index,
        normal: normalize_or_zero(normal),
        center,
        axis_u: normalize_or_zero(axis_u),
        axis_v: normalize_or_zero(axis_v),
        half_u,
        half_v,
        area,
        thickness_half: thickness_half.abs(),
    }
}

fn normalize_or_zero(v: Vec3) -> Vec3 {
    if !v.is_finite() || v.length_squared() <= 1e-8 {
        return Vec3::ZERO;
    }
    v.normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{ObjectPartDef, PrimitiveVisualDef};

    #[test]
    fn assigns_depth_bias_for_coplanar_cuboid_roof_layers() {
        // Matches the cached cabin example: multiple cuboids share the same top-face plane.
        let roof = ObjectPartDef::primitive(
            PrimitiveVisualDef::Primitive {
                mesh: MeshKey::UnitCube,
                params: None,
                color: Color::WHITE,
                unlit: false,
            },
            Transform::from_translation(Vec3::new(0.0, 0.225, 0.0))
                .with_scale(Vec3::new(0.85, 0.10, 0.85)),
        );
        let band = ObjectPartDef::primitive(
            PrimitiveVisualDef::Primitive {
                mesh: MeshKey::UnitCube,
                params: None,
                color: Color::WHITE,
                unlit: false,
            },
            Transform::from_translation(Vec3::new(0.0, 0.26, 0.27))
                .with_scale(Vec3::new(0.75, 0.03, 0.28)),
        );
        let hatch = ObjectPartDef::primitive(
            PrimitiveVisualDef::Primitive {
                mesh: MeshKey::UnitCube,
                params: None,
                color: Color::WHITE,
                unlit: false,
            },
            Transform::from_translation(Vec3::new(0.0, 0.265, 0.0))
                .with_scale(Vec3::new(0.35, 0.02, 0.35)),
        );

        let parts = vec![roof, band, hatch];
        let biases = compute_primitive_part_depth_biases(&parts);
        assert_eq!(biases.len(), 3);
        assert_eq!(biases[0], 0, "roof should be the base layer");
        assert!(biases[1] > biases[0], "band should render in front of roof");
        assert!(
            biases[2] > biases[1],
            "hatch should render in front of band"
        );
    }

    #[test]
    fn assigns_depth_bias_for_concentric_cylinder_caps() {
        // Matches the wheel pattern: concentric capped cylinders share the same cap planes.
        let outer = ObjectPartDef::primitive(
            PrimitiveVisualDef::Primitive {
                mesh: MeshKey::UnitCylinder,
                params: None,
                color: Color::WHITE,
                unlit: false,
            },
            Transform::from_translation(Vec3::ZERO).with_scale(Vec3::new(0.6, 0.26, 0.6)),
        );
        let inner = ObjectPartDef::primitive(
            PrimitiveVisualDef::Primitive {
                mesh: MeshKey::UnitCylinder,
                params: None,
                color: Color::WHITE,
                unlit: false,
            },
            Transform::from_translation(Vec3::ZERO).with_scale(Vec3::new(0.56, 0.26, 0.56)),
        );

        let parts = vec![outer, inner];
        let biases = compute_primitive_part_depth_biases(&parts);
        assert_eq!(biases.len(), 2);
        assert_eq!(biases[0], 0, "outer cylinder should be the back layer");
        assert!(
            biases[1] > biases[0],
            "inner cylinder should render in front"
        );
    }
}
