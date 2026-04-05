use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use uuid::Uuid;

use crate::object::registry::{PrimitiveDeformDef, PrimitiveFfdDeformV1};

pub(crate) fn deform_cache_id(deform: &PrimitiveDeformDef) -> u128 {
    let mut bytes: Vec<u8> = b"gravimera/primitive_deform".to_vec();
    bytes.push(0);
    match deform {
        PrimitiveDeformDef::FfdV1(ffd) => {
            bytes.push(1);
            bytes.extend_from_slice(&ffd.grid);
            for offset in &ffd.offsets {
                bytes.extend_from_slice(&offset.x.to_bits().to_le_bytes());
                bytes.extend_from_slice(&offset.y.to_bits().to_le_bytes());
                bytes.extend_from_slice(&offset.z.to_bits().to_le_bytes());
            }
        }
    }

    Uuid::new_v5(&Uuid::NAMESPACE_URL, bytes.as_slice()).as_u128()
}

pub(crate) fn deform_offsets_aabb(deform: &PrimitiveDeformDef) -> Option<(Vec3, Vec3)> {
    match deform {
        PrimitiveDeformDef::FfdV1(ffd) => offsets_aabb(ffd.offsets.as_slice()),
    }
}

pub(crate) fn apply_deform_to_mesh(
    mesh: &mut Mesh,
    deform: &PrimitiveDeformDef,
) -> Result<(), String> {
    match deform {
        PrimitiveDeformDef::FfdV1(ffd) => apply_ffd_v1(mesh, ffd),
    }
}

fn offsets_aabb(offsets: &[Vec3]) -> Option<(Vec3, Vec3)> {
    if offsets.is_empty() {
        return None;
    }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for offset in offsets {
        if !offset.is_finite() {
            return None;
        }
        min = min.min(*offset);
        max = max.max(*offset);
    }
    Some((min, max))
}

fn apply_ffd_v1(mesh: &mut Mesh, ffd: &PrimitiveFfdDeformV1) -> Result<(), String> {
    if mesh.primitive_topology() != bevy::render::render_resource::PrimitiveTopology::TriangleList {
        return Err("FFD deform only supports triangle list meshes.".to_string());
    }

    let grid = ffd.grid;
    if grid.iter().any(|v| *v < 2) {
        return Err(format!("FFD grid must be >= 2 per axis (got {grid:?})."));
    }
    let expected = (grid[0] as usize)
        .saturating_mul(grid[1] as usize)
        .saturating_mul(grid[2] as usize);
    if ffd.offsets.len() != expected {
        return Err(format!(
            "FFD offsets length {} does not match grid={grid:?} (expected {expected}).",
            ffd.offsets.len()
        ));
    }

    let mut positions: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return Err("Mesh is missing POSITION attribute (Float32x3).".to_string()),
    };

    let Some((min, max)) = aabb_from_positions(positions.as_slice()) else {
        return Ok(());
    };
    let size = max - min;

    let indices = mesh.indices().cloned();
    for p in &mut positions {
        let base = Vec3::new(p[0], p[1], p[2]);
        let u = Vec3::new(
            if size.x.abs() < 1e-6 {
                0.5
            } else {
                (base.x - min.x) / size.x
            },
            if size.y.abs() < 1e-6 {
                0.5
            } else {
                (base.y - min.y) / size.y
            },
            if size.z.abs() < 1e-6 {
                0.5
            } else {
                (base.z - min.z) / size.z
            },
        )
        .clamp(Vec3::ZERO, Vec3::ONE);

        let delta = ffd_trilerp_offset(u, grid, ffd.offsets.as_slice());
        let out = base + delta;
        if !out.is_finite() {
            return Err("FFD produced non-finite vertex position.".to_string());
        }
        p[0] = out.x;
        p[1] = out.y;
        p[2] = out.z;
    }

    let normals = compute_normals_triangle_list(positions.as_slice(), indices.as_ref())?;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    Ok(())
}

fn aabb_from_positions(positions: &[[f32; 3]]) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for p in positions {
        let v = Vec3::new(p[0], p[1], p[2]);
        if !v.is_finite() {
            return None;
        }
        min = min.min(v);
        max = max.max(v);
        any = true;
    }
    any.then_some((min, max))
}

fn ffd_trilerp_offset(u: Vec3, grid: [u8; 3], offsets: &[Vec3]) -> Vec3 {
    let nx = grid[0].max(2) as usize;
    let ny = grid[1].max(2) as usize;
    let nz = grid[2].max(2) as usize;

    let fx = (u.x.clamp(0.0, 1.0) * (nx.saturating_sub(1) as f32)).clamp(0.0, (nx - 1) as f32);
    let fy = (u.y.clamp(0.0, 1.0) * (ny.saturating_sub(1) as f32)).clamp(0.0, (ny - 1) as f32);
    let fz = (u.z.clamp(0.0, 1.0) * (nz.saturating_sub(1) as f32)).clamp(0.0, (nz - 1) as f32);

    let ix0 = (fx.floor() as usize).min(nx - 2);
    let iy0 = (fy.floor() as usize).min(ny - 2);
    let iz0 = (fz.floor() as usize).min(nz - 2);

    let tx = fx - ix0 as f32;
    let ty = fy - iy0 as f32;
    let tz = fz - iz0 as f32;

    let idx = |x: usize, y: usize, z: usize| -> usize { x + nx * (y + ny * z) };
    let read =
        |x: usize, y: usize, z: usize| offsets.get(idx(x, y, z)).copied().unwrap_or(Vec3::ZERO);

    let o000 = read(ix0, iy0, iz0);
    let o100 = read(ix0 + 1, iy0, iz0);
    let o010 = read(ix0, iy0 + 1, iz0);
    let o110 = read(ix0 + 1, iy0 + 1, iz0);
    let o001 = read(ix0, iy0, iz0 + 1);
    let o101 = read(ix0 + 1, iy0, iz0 + 1);
    let o011 = read(ix0, iy0 + 1, iz0 + 1);
    let o111 = read(ix0 + 1, iy0 + 1, iz0 + 1);

    let lerp = |a: Vec3, b: Vec3, t: f32| a + (b - a) * t;
    let ox00 = lerp(o000, o100, tx);
    let ox10 = lerp(o010, o110, tx);
    let ox01 = lerp(o001, o101, tx);
    let ox11 = lerp(o011, o111, tx);

    let oxy0 = lerp(ox00, ox10, ty);
    let oxy1 = lerp(ox01, ox11, ty);

    lerp(oxy0, oxy1, tz)
}

fn compute_normals_triangle_list(
    positions: &[[f32; 3]],
    indices: Option<&Indices>,
) -> Result<Vec<[f32; 3]>, String> {
    let vertex_count = positions.len();
    if vertex_count == 0 {
        return Ok(Vec::new());
    }

    let indices: Vec<u32> = match indices {
        Some(indices) => indices
            .iter()
            .map(|idx| idx.try_into().unwrap_or(0u32))
            .collect(),
        None => (0..vertex_count as u32).collect(),
    };
    if !indices.len().is_multiple_of(3) {
        return Err("Mesh indices are not a multiple of 3.".to_string());
    }

    let mut accum = vec![Vec3::ZERO; vertex_count];
    for tri in indices.chunks(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= vertex_count || i1 >= vertex_count || i2 >= vertex_count {
            return Err("Mesh indices are out of bounds.".to_string());
        }
        let p0 = Vec3::new(positions[i0][0], positions[i0][1], positions[i0][2]);
        let p1 = Vec3::new(positions[i1][0], positions[i1][1], positions[i1][2]);
        let p2 = Vec3::new(positions[i2][0], positions[i2][1], positions[i2][2]);
        let n = (p1 - p0).cross(p2 - p0);
        if !n.is_finite() {
            continue;
        }
        accum[i0] += n;
        accum[i1] += n;
        accum[i2] += n;
    }

    let mut out = Vec::with_capacity(vertex_count);
    for n in accum {
        let n = if n.length_squared() > 1e-10 {
            n.normalize()
        } else {
            Vec3::Y
        };
        out.push([n.x, n.y, n.z]);
    }
    Ok(out)
}
