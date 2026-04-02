use bevy::camera::visibility::RenderLayers;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;

use crate::constants::{FLOOR_GROUND_SINK_M, WORLD_HALF_SIZE};
use crate::genfloor::defs::{
    FloorAnimationMode, FloorColoringMode, FloorDefV1, FloorMeshKind, FloorReliefMode, FloorWaveV1,
};

#[derive(Component)]
pub(crate) struct WorldFloor;

#[derive(Component)]
pub(crate) struct GenfloorPreviewFloor;

#[derive(Resource, Clone)]
pub(crate) struct ActiveWorldFloor {
    pub(crate) floor_id: Option<u128>,
    pub(crate) def: FloorDefV1,
    pub(crate) dirty: bool,
}

impl Default for ActiveWorldFloor {
    fn default() -> Self {
        Self {
            floor_id: None,
            def: FloorDefV1::default_world(),
            dirty: true,
        }
    }
}

#[derive(Clone)]
pub(crate) struct FloorGrid {
    pub(crate) size_x: f32,
    pub(crate) size_z: f32,
    pub(crate) subdiv_x: u32,
    pub(crate) subdiv_z: u32,
    pub(crate) base_positions: Vec<Vec3>,
}

#[derive(Component, Clone)]
pub(crate) struct FloorCpuWave {
    pub(crate) mesh: Handle<Mesh>,
    pub(crate) grid: FloorGrid,
    pub(crate) waves: Vec<FloorWaveV1>,
    pub(crate) normal_strength: f32,
}

#[derive(Clone, Copy)]
pub(crate) enum FloorFootprint {
    Circle { radius: f32 },
    Aabb { half: Vec2 },
}

#[derive(Clone, Copy)]
pub(crate) struct FloorSample {
    pub(crate) height: f32,
    pub(crate) is_water: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct FloorFootprintSample {
    pub(crate) max_height: f32,
    #[allow(dead_code)]
    pub(crate) min_height: f32,
    pub(crate) is_water: bool,
}

pub(crate) fn sample_floor_point(active: &ActiveWorldFloor, x: f32, z: f32) -> FloorSample {
    let height = relief_height(&active.def, x, z);
    FloorSample {
        height,
        is_water: height < 0.0,
    }
}

pub(crate) fn sample_floor_footprint(
    active: &ActiveWorldFloor,
    center: Vec2,
    footprint: FloorFootprint,
) -> FloorFootprintSample {
    let def = &active.def;
    let size_x = def.mesh.size_m[0].max(0.01);
    let size_z = def.mesh.size_m[1].max(0.01);
    let subdiv_x = def.mesh.subdiv[0].max(1);
    let subdiv_z = def.mesh.subdiv[1].max(1);
    let mut step_x = size_x / subdiv_x as f32;
    let mut step_z = size_z / subdiv_z as f32;
    if !step_x.is_finite() || step_x <= 0.0 {
        step_x = 0.1;
    }
    if !step_z.is_finite() || step_z <= 0.0 {
        step_z = 0.1;
    }

    let (min_x, max_x, min_z, max_z) = match footprint {
        FloorFootprint::Circle { radius } => {
            let r = radius.max(0.0);
            (center.x - r, center.x + r, center.y - r, center.y + r)
        }
        FloorFootprint::Aabb { half } => (
            center.x - half.x.abs(),
            center.x + half.x.abs(),
            center.y - half.y.abs(),
            center.y + half.y.abs(),
        ),
    };

    let mut max_height = f32::NEG_INFINITY;
    let mut min_height = f32::INFINITY;
    let mut x = min_x;
    while x <= max_x + 1e-4 {
        let mut z = min_z;
        while z <= max_z + 1e-4 {
            if let FloorFootprint::Circle { radius } = footprint {
                let r2 = radius.max(0.0) * radius.max(0.0);
                if (Vec2::new(x, z) - center).length_squared() > r2 {
                    z += step_z;
                    continue;
                }
            }

            let height = relief_height(def, x, z);
            max_height = max_height.max(height);
            min_height = min_height.min(height);
            z += step_z;
        }
        x += step_x;
    }

    if !max_height.is_finite() || !min_height.is_finite() {
        let height = relief_height(def, center.x, center.y);
        max_height = height;
        min_height = height;
    }

    FloorFootprintSample {
        max_height,
        min_height,
        is_water: min_height < 0.0,
    }
}

pub(crate) fn apply_floor_sink(height: f32) -> f32 {
    if !height.is_finite() {
        return 0.0;
    }
    if height <= 0.0 {
        // Keep default flat terrain at y=0 without any sink.
        return height;
    }
    (height - FLOOR_GROUND_SINK_M).max(0.0)
}

pub(crate) fn floor_half_size(active: &ActiveWorldFloor) -> Vec2 {
    let size = Vec2::new(active.def.mesh.size_m[0], active.def.mesh.size_m[1]);
    let mut half = Vec2::new(size.x.abs() * 0.5, size.y.abs() * 0.5);
    if !half.x.is_finite() || half.x <= 0.0 {
        half.x = WORLD_HALF_SIZE;
    }
    if !half.y.is_finite() || half.y <= 0.0 {
        half.y = WORLD_HALF_SIZE;
    }
    half
}

pub(crate) fn floor_half_size_min(active: &ActiveWorldFloor) -> f32 {
    let half = floor_half_size(active);
    half.x.min(half.y)
}

pub(crate) fn set_active_world_floor(
    active: &mut ActiveWorldFloor,
    floor_id: Option<u128>,
    mut def: FloorDefV1,
) {
    def.canonicalize_in_place();
    active.floor_id = floor_id;
    active.def = def;
    active.dirty = true;
}

pub(crate) fn apply_active_world_floor(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut active: ResMut<ActiveWorldFloor>,
    floors: Query<Entity, With<WorldFloor>>,
) {
    if floors.is_empty() {
        return;
    }

    if !active.dirty {
        return;
    }

    let def = active.def.clone();
    let (mesh, grid) = build_floor_mesh(&def);
    let mesh_handle = meshes.add(mesh);
    let material = material_from_def(&def, &mut materials);

    // World floors can be spawned/despawned via deferred commands (e.g. preview scenes).
    // Applying commands to entities that were despawned earlier in the same frame will
    // otherwise panic when `apply_deferred` runs, so we gate on existence at apply time.
    for entity in &floors {
        let mesh_handle = mesh_handle.clone();
        let material = material.clone();
        match def.animation.mode {
            FloorAnimationMode::Cpu | FloorAnimationMode::Gpu => {
                let grid = grid.clone();
                let waves = def.animation.waves.clone();
                let normal_strength = def.animation.normal_strength;
                commands.queue(move |world: &mut World| {
                    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
                        return;
                    };

                    entity_mut.insert((Mesh3d(mesh_handle.clone()), MeshMaterial3d(material)));
                    entity_mut.remove::<FloorCpuWave>();
                    entity_mut.insert(FloorCpuWave {
                        mesh: mesh_handle,
                        grid,
                        waves,
                        normal_strength,
                    });
                });
            }
            FloorAnimationMode::None => {
                commands.queue(move |world: &mut World| {
                    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
                        return;
                    };

                    entity_mut.insert((Mesh3d(mesh_handle), MeshMaterial3d(material)));
                    entity_mut.remove::<FloorCpuWave>();
                });
            }
        }
    }

    active.dirty = false;
}

pub(crate) fn genfloor_update_cpu_waves(
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    floors: Query<&FloorCpuWave, With<WorldFloor>>,
) {
    if floors.is_empty() {
        return;
    }

    let t = time.elapsed_secs();
    for wave in &floors {
        let Some(mesh) = meshes.get_mut(&wave.mesh) else {
            continue;
        };
        let mut positions: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
            _ => continue,
        };
        let mut normals: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
            _ => vec![[0.0, 1.0, 0.0]; positions.len()],
        };

        let nx = wave.grid.subdiv_x as usize + 1;
        let nz = wave.grid.subdiv_z as usize + 1;
        if positions.len() != nx * nz {
            continue;
        }

        let mut heights = vec![0.0f32; positions.len()];
        for (idx, base) in wave.grid.base_positions.iter().enumerate() {
            let height = base.y + eval_waves(base.x, base.z, t, &wave.waves);
            heights[idx] = height;
            positions[idx] = [base.x, height, base.z];
        }

        let dx = if wave.grid.subdiv_x > 0 {
            wave.grid.size_x / wave.grid.subdiv_x as f32
        } else {
            1.0
        };
        let dz = if wave.grid.subdiv_z > 0 {
            wave.grid.size_z / wave.grid.subdiv_z as f32
        } else {
            1.0
        };
        let strength = wave.normal_strength.max(0.01);

        for z in 0..nz {
            for x in 0..nx {
                let idx = z * nx + x;
                let h_l = heights[z * nx + x.saturating_sub(1)];
                let h_r = heights[z * nx + (x + 1).min(nx - 1)];
                let h_d = heights[z.saturating_sub(1) * nx + x];
                let h_u = heights[(z + 1).min(nz - 1) * nx + x];
                let n = Vec3::new(
                    (h_l - h_r) * strength,
                    2.0 * dx.max(dz),
                    (h_d - h_u) * strength,
                )
                .normalize_or_zero();
                normals[idx] = [n.x, n.y, n.z];
            }
        }

        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    }
}

pub(crate) fn genfloor_ensure_preview_floor(
    mut commands: Commands,
    mut preview: ResMut<crate::gen3d::Gen3dPreview>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut active: ResMut<ActiveWorldFloor>,
    floors: Query<Entity, With<GenfloorPreviewFloor>>,
) {
    if !floors.is_empty() {
        return;
    }
    let Some(root) = preview.root else {
        return;
    };
    let def = active.def.clone();
    let size_x = def.mesh.size_m[0].max(0.5);
    let size_z = def.mesh.size_m[1].max(0.5);
    let thickness = def.mesh.thickness_m.max(0.05);
    let half_extents = Vec3::new(size_x, thickness, size_z) * 0.5;

    let aspect = 16.0 / 9.0;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;

    let yaw = std::f32::consts::FRAC_PI_6;
    let pitch = -0.45;
    let base_distance =
        crate::orbit_capture::required_distance_for_view(half_extents, yaw, pitch, fov_y, aspect, near);
    let distance = (base_distance * 1.1).clamp(near + 0.2, 500.0);

    preview.draft_focus = Vec3::ZERO;
    preview.view_pan = Vec3::ZERO;
    preview.yaw = yaw;
    preview.pitch = pitch;
    preview.distance = distance;

    let mesh_handle = meshes.add(build_floor_mesh_only(&def));
    let material = build_floor_material(&def, &mut materials);
    commands.entity(root).with_children(|child| {
        child.spawn((
            WorldFloor,
            GenfloorPreviewFloor,
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            RenderLayers::layer(crate::gen3d::GEN3D_PREVIEW_UI_LAYER),
            Transform::IDENTITY,
            Visibility::Inherited,
        ));
    });
    active.dirty = true;
}

fn eval_waves(x: f32, z: f32, t: f32, waves: &[FloorWaveV1]) -> f32 {
    let mut height = 0.0;
    for wave in waves {
        let mut dir = Vec2::new(wave.direction[0], wave.direction[1]);
        if dir.length_squared() < 1e-6 {
            dir = Vec2::X;
        }
        dir = dir.normalize();
        let k = std::f32::consts::TAU / wave.wavelength.max(0.01);
        let phase = k * (dir.x * x + dir.y * z) + wave.speed * t + wave.phase;
        height += wave.amplitude * phase.sin();
    }
    height
}

fn material_from_def(
    def: &FloorDefV1,
    materials: &mut Assets<StandardMaterial>,
) -> Handle<StandardMaterial> {
    let mut rgba = def.material.base_color_rgba;
    if !matches!(def.coloring.mode, FloorColoringMode::Solid) {
        rgba[0] = 1.0;
        rgba[1] = 1.0;
        rgba[2] = 1.0;
    }
    let base_color = Color::srgba(
        rgba[0].clamp(0.0, 1.0),
        rgba[1].clamp(0.0, 1.0),
        rgba[2].clamp(0.0, 1.0),
        rgba[3].clamp(0.0, 1.0),
    );
    let alpha_mode = if rgba[3] < 0.999 {
        AlphaMode::Blend
    } else {
        AlphaMode::Opaque
    };

    materials.add(StandardMaterial {
        base_color,
        metallic: def.material.metallic,
        perceptual_roughness: def.material.roughness,
        unlit: def.material.unlit,
        alpha_mode,
        ..default()
    })
}

fn build_floor_mesh(def: &FloorDefV1) -> (Mesh, FloorGrid) {
    match def.mesh.kind {
        FloorMeshKind::Grid => build_grid_mesh(def),
    }
}

pub(crate) fn build_floor_mesh_only(def: &FloorDefV1) -> Mesh {
    let (mesh, _grid) = build_floor_mesh(def);
    mesh
}

pub(crate) fn build_floor_material(
    def: &FloorDefV1,
    materials: &mut Assets<StandardMaterial>,
) -> Handle<StandardMaterial> {
    material_from_def(def, materials)
}

fn build_grid_mesh(def: &FloorDefV1) -> (Mesh, FloorGrid) {
    let subdiv_x = def.mesh.subdiv[0].max(1);
    let subdiv_z = def.mesh.subdiv[1].max(1);
    let size_x = def.mesh.size_m[0].max(0.5);
    let size_z = def.mesh.size_m[1].max(0.5);
    let uv_tile_x = def.mesh.uv_tiling[0].max(0.01);
    let uv_tile_z = def.mesh.uv_tiling[1].max(0.01);
    let use_vertex_colors = !matches!(def.coloring.mode, FloorColoringMode::Solid);

    let nx = subdiv_x as usize + 1;
    let nz = subdiv_z as usize + 1;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(nx * nz);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(nx * nz);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(nx * nz);
    let mut base_positions: Vec<Vec3> = Vec::with_capacity(nx * nz);
    let mut heights: Vec<f32> = Vec::with_capacity(nx * nz);
    let mut colors: Vec<[f32; 4]> = if use_vertex_colors {
        Vec::with_capacity(nx * nz)
    } else {
        Vec::new()
    };

    for z in 0..=subdiv_z {
        let tz = z as f32 / subdiv_z as f32;
        let z_pos = (tz - 0.5) * size_z;
        for x in 0..=subdiv_x {
            let tx = x as f32 / subdiv_x as f32;
            let x_pos = (tx - 0.5) * size_x;
            let base_y = relief_height(def, x_pos, z_pos);
            positions.push([x_pos, base_y, z_pos]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([tx * uv_tile_x, tz * uv_tile_z]);
            base_positions.push(Vec3::new(x_pos, base_y, z_pos));
            heights.push(base_y);
            if use_vertex_colors {
                colors.push(color_for_position(def, x_pos, z_pos));
            }
        }
    }

    if heights.iter().any(|h| h.abs() > 1e-6) {
        normals = compute_normals_from_heights(&heights, nx, nz, size_x, size_z, 1.0);
    }

    let mut indices: Vec<u32> = Vec::with_capacity((subdiv_x * subdiv_z * 6) as usize);
    for z in 0..subdiv_z {
        for x in 0..subdiv_x {
            let i0 = z * (subdiv_x + 1) + x;
            let i1 = i0 + 1;
            let i2 = i0 + (subdiv_x + 1);
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }

    let mut mesh = Mesh::new(
        bevy::render::render_resource::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if use_vertex_colors {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));

    (
        mesh,
        FloorGrid {
            size_x,
            size_z,
            subdiv_x,
            subdiv_z,
            base_positions,
        },
    )
}

fn relief_height(def: &FloorDefV1, x: f32, z: f32) -> f32 {
    if matches!(def.relief.mode, FloorReliefMode::None) || def.relief.amplitude <= 0.0 {
        return 0.0;
    }
    let noise = &def.relief.noise;
    let n = fbm_noise(
        noise.seed,
        x * noise.frequency,
        z * noise.frequency,
        noise.octaves,
        noise.lacunarity,
        noise.gain,
    );
    (n * 2.0 - 1.0) * def.relief.amplitude
}

fn color_for_position(def: &FloorDefV1, x: f32, z: f32) -> [f32; 4] {
    let palette = &def.coloring.palette;
    if palette.is_empty() {
        return def.material.base_color_rgba;
    }

    let scale_x = def.coloring.scale[0].max(0.05);
    let scale_z = def.coloring.scale[1].max(0.05);
    let angle = def.coloring.angle_deg.to_radians();
    let (sin_a, cos_a) = angle.sin_cos();
    let u = (x / scale_x) * cos_a + (z / scale_z) * sin_a;
    let v = (x / scale_x) * -sin_a + (z / scale_z) * cos_a;

    match def.coloring.mode {
        FloorColoringMode::Solid => palette[0],
        FloorColoringMode::Checker => {
            let ix = u.floor() as i32;
            let iz = v.floor() as i32;
            let idx = (ix + iz).rem_euclid(palette.len() as i32) as usize;
            palette[idx]
        }
        FloorColoringMode::Stripes => {
            let idx = (u.floor() as i32).rem_euclid(palette.len() as i32) as usize;
            palette[idx]
        }
        FloorColoringMode::Gradient => {
            let t = fract01(u);
            sample_palette(palette, t)
        }
        FloorColoringMode::Noise => {
            let noise = &def.coloring.noise;
            let n = fbm_noise(
                noise.seed,
                u * noise.frequency,
                v * noise.frequency,
                noise.octaves,
                noise.lacunarity,
                noise.gain,
            );
            sample_palette(palette, n)
        }
    }
}

fn sample_palette(palette: &[[f32; 4]], t: f32) -> [f32; 4] {
    if palette.is_empty() {
        return [1.0, 1.0, 1.0, 1.0];
    }
    if palette.len() == 1 {
        return palette[0];
    }
    let t = t.clamp(0.0, 1.0) * (palette.len() - 1) as f32;
    let i0 = t.floor() as usize;
    let i1 = (i0 + 1).min(palette.len() - 1);
    let f = t - i0 as f32;
    let a = palette[i0];
    let b = palette[i1];
    [
        lerp(a[0], b[0], f),
        lerp(a[1], b[1], f),
        lerp(a[2], b[2], f),
        lerp(a[3], b[3], f),
    ]
}

fn compute_normals_from_heights(
    heights: &[f32],
    nx: usize,
    nz: usize,
    size_x: f32,
    size_z: f32,
    strength: f32,
) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0, 1.0, 0.0]; heights.len()];
    if heights.is_empty() || nx == 0 || nz == 0 {
        return normals;
    }

    let dx = if nx > 1 {
        size_x / (nx - 1) as f32
    } else {
        1.0
    };
    let dz = if nz > 1 {
        size_z / (nz - 1) as f32
    } else {
        1.0
    };
    let strength = strength.max(0.01);

    for z in 0..nz {
        for x in 0..nx {
            let idx = z * nx + x;
            let h_l = heights[z * nx + x.saturating_sub(1)];
            let h_r = heights[z * nx + (x + 1).min(nx - 1)];
            let h_d = heights[z.saturating_sub(1) * nx + x];
            let h_u = heights[(z + 1).min(nz - 1) * nx + x];
            let n = Vec3::new(
                (h_l - h_r) * strength,
                2.0 * dx.max(dz),
                (h_d - h_u) * strength,
            )
            .normalize_or_zero();
            normals[idx] = [n.x, n.y, n.z];
        }
    }
    normals
}

fn fbm_noise(seed: u32, x: f32, y: f32, octaves: u32, lacunarity: f32, gain: f32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut norm = 0.0;
    let octaves = octaves.max(1);
    for i in 0..octaves {
        let value = value_noise(seed.wrapping_add(i), x * freq, y * freq);
        sum += value * amp;
        norm += amp;
        amp *= gain;
        freq *= lacunarity;
    }
    if norm > 0.0 {
        sum / norm
    } else {
        0.0
    }
}

fn value_noise(seed: u32, x: f32, y: f32) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;

    let sx = smoothstep(x - x0 as f32);
    let sy = smoothstep(y - y0 as f32);

    let n00 = hash2(seed, x0, y0);
    let n10 = hash2(seed, x1, y0);
    let n01 = hash2(seed, x0, y1);
    let n11 = hash2(seed, x1, y1);

    let ix0 = lerp(n00, n10, sx);
    let ix1 = lerp(n01, n11, sx);
    lerp(ix0, ix1, sy)
}

fn hash2(seed: u32, x: i32, y: i32) -> f32 {
    let mut h = seed;
    h ^= (x as u32).wrapping_mul(0x9E3779B9);
    h = h.rotate_left(16);
    h ^= (y as u32).wrapping_mul(0x85EBCA6B);
    h = h.wrapping_mul(0xC2B2AE35);
    h ^= h >> 16;
    (h as f32) / (u32::MAX as f32)
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn fract01(v: f32) -> f32 {
    let f = v - v.floor();
    if f < 0.0 {
        f + 1.0
    } else {
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_grid_mesh_counts_vertices_and_indices() {
        let mut def = FloorDefV1::default_world();
        def.mesh.subdiv = [2, 3];
        def.coloring.mode = FloorColoringMode::Solid;
        def.coloring.palette.clear();
        def.relief.mode = FloorReliefMode::None;
        def.relief.amplitude = 0.0;
        def.animation.mode = FloorAnimationMode::None;
        def.animation.waves.clear();
        def.canonicalize_in_place();

        let (mesh, _grid) = build_grid_mesh(&def);

        let positions_len = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(values)) => values.len(),
            other => panic!("unexpected positions attribute: {other:?}"),
        };
        let indices_len = match mesh.indices() {
            Some(bevy::mesh::Indices::U32(values)) => values.len(),
            other => panic!("unexpected indices: {other:?}"),
        };

        let nx = def.mesh.subdiv[0] as usize + 1;
        let nz = def.mesh.subdiv[1] as usize + 1;
        assert_eq!(positions_len, nx * nz);
        assert_eq!(
            indices_len,
            def.mesh.subdiv[0] as usize * def.mesh.subdiv[1] as usize * 6
        );
        assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
    }

    #[test]
    fn build_grid_mesh_emits_vertex_colors_for_non_solid_modes() {
        let mut def = FloorDefV1::default_world();
        def.mesh.subdiv = [2, 2];
        def.coloring.mode = FloorColoringMode::Checker;
        def.coloring.palette.clear();
        def.animation.mode = FloorAnimationMode::None;
        def.animation.waves.clear();
        def.canonicalize_in_place();

        let (mesh, _grid) = build_grid_mesh(&def);

        let positions_len = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(values)) => values.len(),
            other => panic!("unexpected positions attribute: {other:?}"),
        };
        let colors_len = match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(values)) => values.len(),
            other => panic!("unexpected color attribute: {other:?}"),
        };
        assert_eq!(positions_len, colors_len);
    }

    #[test]
    fn apply_active_world_floor_keeps_dirty_when_no_floors_exist() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<ActiveWorldFloor>();

        app.add_systems(Update, apply_active_world_floor);

        app.update();

        let active = app.world().resource::<ActiveWorldFloor>();
        assert!(
            active.dirty,
            "Expected dirty to remain true with no floors."
        );
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 0);
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 0);
    }

    #[test]
    fn apply_active_world_floor_does_not_panic_if_floor_despawned_before_apply() {
        fn despawn_floors_first(mut commands: Commands, floors: Query<Entity, With<WorldFloor>>) {
            for entity in &floors {
                commands.entity(entity).despawn();
            }
        }

        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<StandardMaterial>>();
        app.init_resource::<ActiveWorldFloor>();
        app.world_mut().spawn((WorldFloor,));

        app.add_systems(Update, despawn_floors_first);
        app.add_systems(Update, apply_active_world_floor.after(despawn_floors_first));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app.update();
        }));
        assert!(result.is_ok(), "Expected no panic, got: {result:?}");
    }
}
