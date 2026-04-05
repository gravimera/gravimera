use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use std::collections::HashMap;
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
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::PrimitiveTopology;

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

    let base_positions: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return Err("Mesh is missing POSITION attribute (Float32x3).".to_string()),
    };

    let indices = indices_as_u32(mesh.indices(), base_positions.len())?;

    let subdivision_iters = ffd_subdivision_iterations(grid);
    let (mut positions, indices) = if subdivision_iters > 0 {
        subdivide_triangle_list(
            base_positions.as_slice(),
            indices.as_slice(),
            subdivision_iters,
        )?
    } else {
        (base_positions, indices)
    };

    let Some((min, max)) = aabb_from_positions(positions.as_slice()) else {
        return Ok(());
    };
    let size = max - min;

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

    let normals = compute_normals_triangle_list_u32(positions.as_slice(), indices.as_slice())?;

    if subdivision_iters > 0 {
        // Rebuild the mesh so vertex attributes stay consistent after subdivision.
        let mut rebuilt = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        rebuilt.insert_indices(Indices::U32(indices));
        rebuilt.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        rebuilt.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        *mesh = rebuilt;
        return Ok(());
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    Ok(())
}

fn ffd_subdivision_iterations(grid: [u8; 3]) -> usize {
    let max_axis = grid.iter().copied().max().unwrap_or(2) as usize;
    let desired = max_axis.saturating_sub(2);
    desired.min(2)
}

fn indices_as_u32(indices: Option<&Indices>, vertex_count: usize) -> Result<Vec<u32>, String> {
    let Some(indices) = indices else {
        return Ok((0..vertex_count as u32).collect());
    };

    let mut out = Vec::with_capacity(indices.len());
    for idx in indices.iter() {
        let idx: u32 = idx
            .try_into()
            .map_err(|_| "Mesh index could not be converted to u32.".to_string())?;
        out.push(idx);
    }
    Ok(out)
}

fn subdivide_triangle_list(
    positions: &[[f32; 3]],
    indices: &[u32],
    iterations: usize,
) -> Result<(Vec<[f32; 3]>, Vec<u32>), String> {
    if iterations == 0 {
        return Ok((positions.to_vec(), indices.to_vec()));
    }
    if !indices.len().is_multiple_of(3) {
        return Err("Mesh indices are not a multiple of 3.".to_string());
    }

    let mut pos: Vec<Vec3> = positions
        .iter()
        .map(|p| Vec3::new(p[0], p[1], p[2]))
        .collect();
    let mut idx: Vec<u32> = indices.to_vec();

    for _ in 0..iterations {
        let mut next_pos = pos.clone();
        let mut midpoint_cache: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next_idx: Vec<u32> = Vec::with_capacity(idx.len().saturating_mul(4));

        let mut midpoint = |a: u32, b: u32| -> Result<u32, String> {
            let key = if a < b { (a, b) } else { (b, a) };
            if let Some(existing) = midpoint_cache.get(&key) {
                return Ok(*existing);
            }

            let ia = a as usize;
            let ib = b as usize;
            if ia >= pos.len() || ib >= pos.len() {
                return Err("Mesh indices are out of bounds.".to_string());
            }
            let m = (pos[ia] + pos[ib]) * 0.5;
            let out = next_pos.len() as u32;
            next_pos.push(m);
            midpoint_cache.insert(key, out);
            Ok(out)
        };

        for tri in idx.chunks(3) {
            let a = tri[0];
            let b = tri[1];
            let c = tri[2];

            let ab = midpoint(a, b)?;
            let bc = midpoint(b, c)?;
            let ca = midpoint(c, a)?;

            next_idx.extend_from_slice(&[a, ab, ca]);
            next_idx.extend_from_slice(&[ab, b, bc]);
            next_idx.extend_from_slice(&[ca, bc, c]);
            next_idx.extend_from_slice(&[ab, bc, ca]);
        }

        pos = next_pos;
        idx = next_idx;
    }

    let out_positions: Vec<[f32; 3]> = pos.iter().map(|p| [p.x, p.y, p.z]).collect();
    Ok((out_positions, idx))
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

fn compute_normals_triangle_list_u32(
    positions: &[[f32; 3]],
    indices: &[u32],
) -> Result<Vec<[f32; 3]>, String> {
    let vertex_count = positions.len();
    if vertex_count == 0 {
        return Ok(Vec::new());
    }
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
