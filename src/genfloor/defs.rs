use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::constants::WORLD_HALF_SIZE;

pub(crate) const FLOOR_DEF_FORMAT_VERSION: u32 = 1;
const DEFAULT_FLOOR_SIZE_M: f32 = WORLD_HALF_SIZE * 2.4;
const DEFAULT_SUBDIV: u32 = 64;
const MAX_SUBDIV: u32 = 256;
const MIN_WAVELENGTH: f32 = 0.05;
const MAX_WAVES: usize = 8;
const MAX_COLOR_PALETTE: usize = 6;
const MAX_NOISE_OCTAVES: u32 = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FloorMeshKind {
    Grid,
}

impl Default for FloorMeshKind {
    fn default() -> Self {
        Self::Grid
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FloorMeshV1 {
    pub(crate) kind: FloorMeshKind,
    pub(crate) size_m: [f32; 2],
    pub(crate) subdiv: [u32; 2],
    pub(crate) thickness_m: f32,
    pub(crate) uv_tiling: [f32; 2],
}

impl Default for FloorMeshV1 {
    fn default() -> Self {
        Self {
            kind: FloorMeshKind::Grid,
            size_m: [DEFAULT_FLOOR_SIZE_M, DEFAULT_FLOOR_SIZE_M],
            subdiv: [DEFAULT_SUBDIV, DEFAULT_SUBDIV],
            thickness_m: 0.1,
            uv_tiling: [4.0, 4.0],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FloorMaterialV1 {
    pub(crate) base_color_rgba: [f32; 4],
    pub(crate) metallic: f32,
    pub(crate) roughness: f32,
    pub(crate) unlit: bool,
}

impl Default for FloorMaterialV1 {
    fn default() -> Self {
        Self {
            base_color_rgba: [0.16, 0.17, 0.20, 1.0],
            metallic: 0.0,
            roughness: 0.95,
            unlit: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FloorAnimationMode {
    None,
    Cpu,
    Gpu,
}

impl Default for FloorAnimationMode {
    fn default() -> Self {
        Self::Cpu
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FloorWaveV1 {
    pub(crate) amplitude: f32,
    pub(crate) wavelength: f32,
    pub(crate) direction: [f32; 2],
    pub(crate) speed: f32,
    pub(crate) phase: f32,
}

impl Default for FloorWaveV1 {
    fn default() -> Self {
        Self {
            amplitude: 0.2,
            wavelength: 4.0,
            direction: [1.0, 0.0],
            speed: 1.0,
            phase: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FloorAnimationV1 {
    pub(crate) mode: FloorAnimationMode,
    pub(crate) waves: Vec<FloorWaveV1>,
    pub(crate) normal_strength: f32,
}

impl Default for FloorAnimationV1 {
    fn default() -> Self {
        Self {
            mode: FloorAnimationMode::Cpu,
            waves: vec![FloorWaveV1::default()],
            normal_strength: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FloorReliefMode {
    None,
    Noise,
}

impl Default for FloorReliefMode {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FloorNoiseV1 {
    pub(crate) seed: u32,
    pub(crate) frequency: f32,
    pub(crate) octaves: u32,
    pub(crate) lacunarity: f32,
    pub(crate) gain: f32,
}

impl Default for FloorNoiseV1 {
    fn default() -> Self {
        Self {
            seed: 1,
            frequency: 0.25,
            octaves: 3,
            lacunarity: 2.0,
            gain: 0.5,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FloorReliefV1 {
    pub(crate) mode: FloorReliefMode,
    pub(crate) amplitude: f32,
    #[serde(default)]
    pub(crate) noise: FloorNoiseV1,
}

impl Default for FloorReliefV1 {
    fn default() -> Self {
        Self {
            mode: FloorReliefMode::None,
            amplitude: 0.0,
            noise: FloorNoiseV1::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FloorColoringMode {
    Solid,
    Checker,
    Stripes,
    Gradient,
    Noise,
}

impl Default for FloorColoringMode {
    fn default() -> Self {
        Self::Solid
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FloorColoringV1 {
    pub(crate) mode: FloorColoringMode,
    pub(crate) palette: Vec<[f32; 4]>,
    pub(crate) scale: [f32; 2],
    pub(crate) angle_deg: f32,
    #[serde(default)]
    pub(crate) noise: FloorNoiseV1,
}

impl Default for FloorColoringV1 {
    fn default() -> Self {
        Self {
            mode: FloorColoringMode::Solid,
            palette: Vec::new(),
            scale: [4.0, 4.0],
            angle_deg: 0.0,
            noise: FloorNoiseV1::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FloorDefV1 {
    pub(crate) format_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    pub(crate) mesh: FloorMeshV1,
    pub(crate) material: FloorMaterialV1,
    #[serde(default)]
    pub(crate) coloring: FloorColoringV1,
    #[serde(default)]
    pub(crate) relief: FloorReliefV1,
    pub(crate) animation: FloorAnimationV1,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, serde_json::Value>,
}

impl Default for FloorDefV1 {
    fn default() -> Self {
        Self::default_world()
    }
}

impl FloorDefV1 {
    pub(crate) fn default_world() -> Self {
        let mesh = FloorMeshV1 {
            kind: FloorMeshKind::Grid,
            size_m: [DEFAULT_FLOOR_SIZE_M, DEFAULT_FLOOR_SIZE_M],
            subdiv: [1, 1],
            thickness_m: 0.1,
            uv_tiling: [1.0, 1.0],
        };
        let material = FloorMaterialV1 {
            base_color_rgba: [0.16, 0.17, 0.20, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            unlit: false,
        };
        let coloring = FloorColoringV1::default();
        let relief = FloorReliefV1::default();
        let animation = FloorAnimationV1 {
            mode: FloorAnimationMode::None,
            waves: Vec::new(),
            normal_strength: 1.0,
        };
        Self {
            format_version: FLOOR_DEF_FORMAT_VERSION,
            label: Some("Default Floor".to_string()),
            mesh,
            material,
            coloring,
            relief,
            animation,
            extra: BTreeMap::default(),
        }
    }

    pub(crate) fn canonicalize_in_place(&mut self) {
        self.format_version = FLOOR_DEF_FORMAT_VERSION;

        if let Some(label) = self.label.as_mut() {
            *label = label.trim().to_string();
            if label.is_empty() {
                self.label = None;
            }
        }

        let size_x = if self.mesh.size_m[0].is_finite() {
            self.mesh.size_m[0].abs().max(DEFAULT_FLOOR_SIZE_M)
        } else {
            DEFAULT_FLOOR_SIZE_M
        };
        let size_z = if self.mesh.size_m[1].is_finite() {
            self.mesh.size_m[1].abs().max(DEFAULT_FLOOR_SIZE_M)
        } else {
            DEFAULT_FLOOR_SIZE_M
        };
        self.mesh.size_m = [size_x, size_z];

        let subdiv_x = self.mesh.subdiv[0].clamp(1, MAX_SUBDIV);
        let subdiv_z = self.mesh.subdiv[1].clamp(1, MAX_SUBDIV);
        self.mesh.subdiv = [subdiv_x, subdiv_z];

        if !self.mesh.thickness_m.is_finite() {
            self.mesh.thickness_m = 0.1;
        }
        if !self.mesh.uv_tiling[0].is_finite() || self.mesh.uv_tiling[0].abs() < 0.01 {
            self.mesh.uv_tiling[0] = 1.0;
        }
        if !self.mesh.uv_tiling[1].is_finite() || self.mesh.uv_tiling[1].abs() < 0.01 {
            self.mesh.uv_tiling[1] = 1.0;
        }

        for c in &mut self.material.base_color_rgba {
            if !c.is_finite() {
                *c = 1.0;
            }
            *c = c.clamp(0.0, 1.0);
        }
        if !self.material.metallic.is_finite() {
            self.material.metallic = 0.0;
        }
        if !self.material.roughness.is_finite() {
            self.material.roughness = 0.95;
        }
        self.material.metallic = self.material.metallic.clamp(0.0, 1.0);
        self.material.roughness = self.material.roughness.clamp(0.0, 1.0);

        self.coloring.mode = match self.coloring.mode {
            FloorColoringMode::Solid => FloorColoringMode::Solid,
            FloorColoringMode::Checker => FloorColoringMode::Checker,
            FloorColoringMode::Stripes => FloorColoringMode::Stripes,
            FloorColoringMode::Gradient => FloorColoringMode::Gradient,
            FloorColoringMode::Noise => FloorColoringMode::Noise,
        };
        let scale_x = if self.coloring.scale[0].is_finite() {
            self.coloring.scale[0].abs().max(0.05)
        } else {
            4.0
        };
        let scale_z = if self.coloring.scale[1].is_finite() {
            self.coloring.scale[1].abs().max(0.05)
        } else {
            4.0
        };
        self.coloring.scale = [scale_x, scale_z];
        if !self.coloring.angle_deg.is_finite() {
            self.coloring.angle_deg = 0.0;
        }
        self.coloring.angle_deg = ((self.coloring.angle_deg + 180.0) % 360.0) - 180.0;
        if self.coloring.palette.len() > MAX_COLOR_PALETTE {
            self.coloring.palette.truncate(MAX_COLOR_PALETTE);
        }
        if self.coloring.palette.is_empty()
            && !matches!(self.coloring.mode, FloorColoringMode::Solid)
        {
            self.coloring.palette = vec![
                [0.12, 0.35, 0.75, 1.0],
                [0.88, 0.55, 0.20, 1.0],
                [0.20, 0.70, 0.35, 1.0],
            ];
        }
        for color in &mut self.coloring.palette {
            for c in color.iter_mut() {
                if !c.is_finite() {
                    *c = 1.0;
                }
                *c = c.clamp(0.0, 1.0);
            }
        }
        canonicalize_noise(&mut self.coloring.noise);

        self.relief.mode = match self.relief.mode {
            FloorReliefMode::None => FloorReliefMode::None,
            FloorReliefMode::Noise => FloorReliefMode::Noise,
        };
        if !self.relief.amplitude.is_finite() {
            self.relief.amplitude = 0.0;
        }
        if matches!(self.relief.mode, FloorReliefMode::None) {
            self.relief.amplitude = 0.0;
        } else {
            self.relief.amplitude = self.relief.amplitude.clamp(0.0, 10.0);
        }
        canonicalize_noise(&mut self.relief.noise);

        if !self.animation.normal_strength.is_finite() || self.animation.normal_strength <= 0.0 {
            self.animation.normal_strength = 1.0;
        }

        if self.animation.waves.len() > MAX_WAVES {
            self.animation.waves.truncate(MAX_WAVES);
        }
        if self.animation.waves.is_empty() {
            self.animation.waves.push(FloorWaveV1::default());
        }
        for wave in &mut self.animation.waves {
            if !wave.amplitude.is_finite() {
                wave.amplitude = 0.0;
            }
            if !wave.wavelength.is_finite() {
                wave.wavelength = 1.0;
            }
            wave.amplitude = wave.amplitude.clamp(-10.0, 10.0);
            wave.wavelength = wave.wavelength.abs().max(MIN_WAVELENGTH);
            if !wave.direction[0].is_finite() {
                wave.direction[0] = 1.0;
            }
            if !wave.direction[1].is_finite() {
                wave.direction[1] = 0.0;
            }
            if !wave.speed.is_finite() {
                wave.speed = 0.0;
            }
            if !wave.phase.is_finite() {
                wave.phase = 0.0;
            }
        }
    }
}

fn canonicalize_noise(noise: &mut FloorNoiseV1) {
    if !noise.frequency.is_finite() {
        noise.frequency = 0.25;
    }
    noise.frequency = noise.frequency.abs().max(0.001);
    if !noise.lacunarity.is_finite() {
        noise.lacunarity = 2.0;
    }
    noise.lacunarity = noise.lacunarity.abs().max(1.0);
    if !noise.gain.is_finite() {
        noise.gain = 0.5;
    }
    noise.gain = noise.gain.clamp(0.0, 1.0);
    if noise.octaves == 0 {
        noise.octaves = 1;
    }
    noise.octaves = noise.octaves.min(MAX_NOISE_OCTAVES);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_clamps_subdiv_and_adds_palette_for_patterns() {
        let mut def = FloorDefV1 {
            format_version: 999,
            label: Some("  ".to_string()),
            mesh: FloorMeshV1 {
                kind: FloorMeshKind::Grid,
                size_m: [-10.0, 0.0],
                subdiv: [9999, 0],
                thickness_m: f32::NAN,
                uv_tiling: [0.0, -0.0],
            },
            material: FloorMaterialV1::default(),
            coloring: FloorColoringV1 {
                mode: FloorColoringMode::Checker,
                palette: Vec::new(),
                scale: [0.0, f32::INFINITY],
                angle_deg: f32::NAN,
                noise: FloorNoiseV1 {
                    seed: 1,
                    frequency: 0.0,
                    octaves: 999,
                    lacunarity: f32::NAN,
                    gain: 2.0,
                },
            },
            relief: FloorReliefV1::default(),
            animation: FloorAnimationV1::default(),
            extra: BTreeMap::default(),
        };

        def.canonicalize_in_place();

        assert_eq!(def.format_version, FLOOR_DEF_FORMAT_VERSION);
        assert!(def.label.is_none());
        assert_eq!(def.mesh.subdiv[0], MAX_SUBDIV);
        assert_eq!(def.mesh.subdiv[1], 1);
        assert!(def.mesh.size_m[0] >= DEFAULT_FLOOR_SIZE_M);
        assert!(def.mesh.size_m[1] >= DEFAULT_FLOOR_SIZE_M);
        assert!(!def.coloring.palette.is_empty());
        assert!(def.coloring.noise.octaves <= MAX_NOISE_OCTAVES);
    }
}
