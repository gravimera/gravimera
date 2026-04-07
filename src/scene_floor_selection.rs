use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

const SCENE_TERRAIN_DAT_FORMAT_VERSION: u32 = 2;
const LEGACY_SCENE_FLOOR_SELECTION_FORMAT_VERSION: u32 = 1;

pub(crate) struct SceneTerrainSelection {
    pub(crate) terrain_id: Option<u128>,
    pub(crate) def: crate::genfloor::defs::FloorDefV1,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacySceneFloorSelectionFileV1 {
    format_version: u32,
    floor_id: Option<String>,
}

pub(crate) fn load_scene_floor_selection(
    realm_id: &str,
    scene_id: &str,
) -> Result<SceneTerrainSelection, String> {
    let realm_id = crate::realm::sanitize_id(realm_id)
        .ok_or_else(|| "scene terrain selection: invalid realm id".to_string())?;
    let scene_id = crate::realm::sanitize_id(scene_id)
        .ok_or_else(|| "scene terrain selection: invalid scene id".to_string())?;
    let path = crate::paths::scene_floor_selection_path(&realm_id, &scene_id);
    let legacy_path = crate::paths::legacy_scene_floor_selection_path(&realm_id, &scene_id);
    load_scene_floor_selection_from_paths(&realm_id, &path, &legacy_path)
}

pub(crate) fn save_scene_floor_selection(
    realm_id: &str,
    scene_id: &str,
    floor_id: Option<u128>,
) -> Result<(), String> {
    let realm_id = crate::realm::sanitize_id(realm_id)
        .ok_or_else(|| "scene terrain selection: invalid realm id".to_string())?;
    let scene_id = crate::realm::sanitize_id(scene_id)
        .ok_or_else(|| "scene terrain selection: invalid scene id".to_string())?;
    let path = crate::paths::scene_floor_selection_path(&realm_id, &scene_id);
    let legacy_path = crate::paths::legacy_scene_floor_selection_path(&realm_id, &scene_id);
    save_scene_floor_selection_to_paths(&realm_id, &path, &legacy_path, floor_id)
}

pub(crate) fn migrate_legacy_scene_floor_selection_files() -> Result<(), String> {
    let realms_dir = crate::paths::realms_dir();
    if !realms_dir.exists() {
        return Ok(());
    }

    let mut errors = Vec::new();
    let Ok(realm_entries) = std::fs::read_dir(&realms_dir) else {
        return Ok(());
    };

    for realm_entry in realm_entries.flatten() {
        let realm_path = realm_entry.path();
        if !realm_path.is_dir() {
            continue;
        }
        let Some(realm_name) = realm_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(realm_id) = crate::realm::sanitize_id(realm_name) else {
            continue;
        };

        let scenes_dir = crate::paths::realm_dir(&realm_id).join("scenes");
        let Ok(scene_entries) = std::fs::read_dir(&scenes_dir) else {
            continue;
        };

        for scene_entry in scene_entries.flatten() {
            let scene_path = scene_entry.path();
            if !scene_path.is_dir() {
                continue;
            }
            let Some(scene_name) = scene_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(scene_id) = crate::realm::sanitize_id(scene_name) else {
                continue;
            };

            let path = crate::paths::scene_floor_selection_path(&realm_id, &scene_id);
            let legacy_path = crate::paths::legacy_scene_floor_selection_path(&realm_id, &scene_id);
            if !legacy_path.exists() {
                continue;
            }

            if let Err(err) = load_scene_floor_selection_from_paths(&realm_id, &path, &legacy_path)
            {
                errors.push(err);
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" | "))
    }
}

pub(crate) fn read_scene_floor_selection_from_build_dir(
    build_dir: &Path,
) -> Result<Option<u128>, String> {
    let path = build_dir.join("terrain.grav");
    match std::fs::read(&path) {
        Ok(bytes) => {
            let parsed = crate::proto::gravimera::terrain::v1::SceneTerrainDat::decode(
                bytes.as_slice(),
            )
            .map_err(|err| {
                format!(
                    "scene terrain selection: failed to decode {}: {err}",
                    path.display()
                )
            })?;
            if !matches!(parsed.format_version, 1 | 2) {
                return Ok(None);
            }
            return Ok(parsed.terrain_id.as_ref().map(uuid128_to_u128));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "scene terrain selection: failed to read {}: {err}",
                path.display()
            ))
        }
    }

    let legacy_path = build_dir.join("floor_selection.json");
    let legacy_bytes = match std::fs::read(&legacy_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "scene terrain selection: failed to read {}: {err}",
                legacy_path.display()
            ))
        }
    };

    parse_legacy_scene_floor_selection(&legacy_bytes, &legacy_path)
}

pub(crate) fn remap_scene_floor_selection_in_build_dir(
    build_dir: &Path,
    floor_id_map: &BTreeMap<u128, u128>,
) -> Result<(), String> {
    if floor_id_map.is_empty() {
        return Ok(());
    }

    let Some(floor_id) = read_scene_floor_selection_from_build_dir(build_dir)? else {
        return Ok(());
    };
    let Some(new_id) = floor_id_map.get(&floor_id).copied() else {
        return Ok(());
    };
    if new_id == floor_id {
        return Ok(());
    }

    let path = build_dir.join("terrain.grav");
    let legacy_path = build_dir.join("floor_selection.json");

    match std::fs::read(&path) {
        Ok(bytes) => {
            let mut parsed = crate::proto::gravimera::terrain::v1::SceneTerrainDat::decode(
                bytes.as_slice(),
            )
            .map_err(|err| {
                format!(
                    "scene terrain selection: failed to decode {}: {err}",
                    path.display()
                )
            })?;
            if !matches!(parsed.format_version, 1 | 2) {
                return Ok(());
            }
            parsed.terrain_id = Some(u128_to_uuid128(new_id));
            let bytes = parsed.encode_to_vec();
            write_atomic(&path, &bytes)?;
            remove_if_exists(&legacy_path)?;
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // Fallback: write a v1 selection-only file (no embedded def available here).
            let doc = crate::proto::gravimera::terrain::v1::SceneTerrainDat {
                format_version: 1,
                terrain_id: Some(u128_to_uuid128(new_id)),
                terrain_def: None,
            };
            let bytes = doc.encode_to_vec();
            write_atomic(&path, &bytes)?;
            remove_if_exists(&legacy_path)?;
            Ok(())
        }
        Err(err) => Err(format!(
            "scene terrain selection: failed to read {}: {err}",
            path.display()
        )),
    }
}

fn load_scene_floor_selection_from_paths(
    realm_id: &str,
    path: &Path,
    legacy_path: &Path,
) -> Result<SceneTerrainSelection, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let parsed = crate::proto::gravimera::terrain::v1::SceneTerrainDat::decode(
                bytes.as_slice(),
            )
            .map_err(|err| {
                format!(
                    "scene terrain selection: failed to decode {}: {err}",
                    path.display()
                )
            })?;
            if !matches!(parsed.format_version, 1 | 2) {
                return Err(format!(
                    "scene terrain selection: unsupported terrain.grav format_version {}",
                    parsed.format_version
                ));
            }

            let terrain_id = parsed
                .terrain_id
                .as_ref()
                .map(uuid128_to_u128)
                .filter(|id| *id != crate::floor_library_ui::DEFAULT_FLOOR_ID);
            let def = match parsed.terrain_def.as_ref() {
                Some(def) => terrain_def_from_dat(def),
                None => load_def_for_terrain_id(realm_id, terrain_id)?,
            };

            // Best-effort: upgrade v1 selection-only files to v2 so scenes are self-contained.
            if parsed.format_version != SCENE_TERRAIN_DAT_FORMAT_VERSION || parsed.terrain_def.is_none()
            {
                let _ = save_scene_terrain_dat_to_paths(path, legacy_path, terrain_id, &def);
            }

            if legacy_path.exists() {
                let _ = std::fs::remove_file(legacy_path);
            }
            return Ok(SceneTerrainSelection { terrain_id, def });
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "scene terrain selection: failed to read {}: {err}",
                path.display()
            ))
        }
    }

    let legacy_bytes = match std::fs::read(legacy_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let def = crate::genfloor::defs::FloorDefV1::default_world();
            let _ = save_scene_terrain_dat_to_paths(path, legacy_path, None, &def);
            return Ok(SceneTerrainSelection {
                terrain_id: None,
                def,
            });
        }
        Err(err) => {
            return Err(format!(
                "scene terrain selection: failed to read {}: {err}",
                legacy_path.display()
            ))
        }
    };

    let terrain_id = parse_legacy_scene_floor_selection(&legacy_bytes, legacy_path)?
        .filter(|id| *id != crate::floor_library_ui::DEFAULT_FLOOR_ID);
    let def = load_def_for_terrain_id(realm_id, terrain_id)?;
    save_scene_terrain_dat_to_paths(path, legacy_path, terrain_id, &def)?;
    Ok(SceneTerrainSelection { terrain_id, def })
}

fn save_scene_floor_selection_to_paths(
    realm_id: &str,
    path: &Path,
    legacy_path: &Path,
    floor_id: Option<u128>,
) -> Result<(), String> {
    let terrain_id = floor_id.filter(|id| *id != crate::floor_library_ui::DEFAULT_FLOOR_ID);
    let def = load_def_for_terrain_id(realm_id, terrain_id)?;
    save_scene_terrain_dat_to_paths(path, legacy_path, terrain_id, &def)
}

fn load_def_for_terrain_id(
    realm_id: &str,
    terrain_id: Option<u128>,
) -> Result<crate::genfloor::defs::FloorDefV1, String> {
    match terrain_id {
        None => Ok(crate::genfloor::defs::FloorDefV1::default_world()),
        Some(id) => crate::realm_floor_packages::load_realm_floor_def(realm_id, id),
    }
}

fn save_scene_terrain_dat_to_paths(
    path: &Path,
    legacy_path: &Path,
    terrain_id: Option<u128>,
    def: &crate::genfloor::defs::FloorDefV1,
) -> Result<(), String> {
    let doc = crate::proto::gravimera::terrain::v1::SceneTerrainDat {
        format_version: SCENE_TERRAIN_DAT_FORMAT_VERSION,
        terrain_id: terrain_id.map(u128_to_uuid128),
        terrain_def: Some(terrain_def_to_dat(def)),
    };
    let bytes = doc.encode_to_vec();
    write_atomic(path, &bytes)?;
    remove_if_exists(legacy_path)?;
    Ok(())
}

fn parse_legacy_scene_floor_selection(bytes: &[u8], path: &Path) -> Result<Option<u128>, String> {
    let parsed: LegacySceneFloorSelectionFileV1 = serde_json::from_slice(bytes).map_err(|err| {
        format!(
            "scene terrain selection: invalid legacy JSON in {}: {err}",
            path.display()
        )
    })?;

    if parsed.format_version != LEGACY_SCENE_FLOOR_SELECTION_FORMAT_VERSION {
        return Ok(None);
    }

    let Some(raw_id) = parsed.floor_id else {
        return Ok(None);
    };
    let id = uuid::Uuid::parse_str(raw_id.trim()).map_err(|err| {
        format!(
            "scene terrain selection: invalid legacy floor id in {}: {err}",
            path.display()
        )
    })?;
    Ok(Some(id.as_u128()))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "scene terrain selection: failed to create {}: {err}",
                parent.display()
            )
        })?;
    }

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, bytes).map_err(|err| {
        format!(
            "scene terrain selection: failed to write {}: {err}",
            tmp_path.display()
        )
    })?;

    if let Err(err) = std::fs::rename(&tmp_path, path) {
        if path.exists() {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp_path, path).map_err(|rename_err| {
                format!(
                    "scene terrain selection: failed to replace {} after rename error {err}: {rename_err}",
                    path.display()
                )
            })?;
        } else {
            return Err(format!(
                "scene terrain selection: failed to rename {} to {}: {err}",
                tmp_path.display(),
                path.display()
            ));
        }
    }

    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "scene terrain selection: failed to remove {}: {err}",
            path.display()
        )),
    }
}

fn u128_to_uuid128(value: u128) -> crate::proto::gravimera::common::v1::Uuid128 {
    crate::proto::gravimera::common::v1::Uuid128 {
        hi: (value >> 64) as u64,
        lo: value as u64,
    }
}

fn uuid128_to_u128(value: &crate::proto::gravimera::common::v1::Uuid128) -> u128 {
    ((value.hi as u128) << 64) | (value.lo as u128)
}

fn terrain_def_to_dat(
    def: &crate::genfloor::defs::FloorDefV1,
) -> crate::proto::gravimera::terrain::v1::TerrainDefV1 {
    use crate::proto::gravimera::terrain::v1 as terrain;

    fn noise_to_dat(noise: &crate::genfloor::defs::FloorNoiseV1) -> terrain::TerrainNoiseV1 {
        terrain::TerrainNoiseV1 {
            seed: noise.seed,
            frequency: noise.frequency,
            octaves: noise.octaves,
            lacunarity: noise.lacunarity,
            gain: noise.gain,
        }
    }

    fn mesh_to_dat(mesh: &crate::genfloor::defs::FloorMeshV1) -> terrain::TerrainMeshV1 {
        terrain::TerrainMeshV1 {
            kind: 0,
            size_x_m: mesh.size_m[0],
            size_z_m: mesh.size_m[1],
            subdiv_x: mesh.subdiv[0],
            subdiv_z: mesh.subdiv[1],
            thickness_m: mesh.thickness_m,
            uv_tiling_x: mesh.uv_tiling[0],
            uv_tiling_z: mesh.uv_tiling[1],
        }
    }

    fn material_to_dat(material: &crate::genfloor::defs::FloorMaterialV1) -> terrain::TerrainMaterialV1 {
        terrain::TerrainMaterialV1 {
            base_color_r: material.base_color_rgba[0],
            base_color_g: material.base_color_rgba[1],
            base_color_b: material.base_color_rgba[2],
            base_color_a: material.base_color_rgba[3],
            metallic: material.metallic,
            roughness: material.roughness,
            unlit: material.unlit,
        }
    }

    fn coloring_to_dat(coloring: &crate::genfloor::defs::FloorColoringV1) -> terrain::TerrainColoringV1 {
        let mode = match coloring.mode {
            crate::genfloor::defs::FloorColoringMode::Solid => 0,
            crate::genfloor::defs::FloorColoringMode::Checker => 1,
            crate::genfloor::defs::FloorColoringMode::Stripes => 2,
            crate::genfloor::defs::FloorColoringMode::Gradient => 3,
            crate::genfloor::defs::FloorColoringMode::Noise => 4,
        };
        terrain::TerrainColoringV1 {
            mode,
            palette: coloring
                .palette
                .iter()
                .map(|rgba| terrain::TerrainColorRgba {
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                })
                .collect(),
            scale_x: coloring.scale[0],
            scale_z: coloring.scale[1],
            angle_deg: coloring.angle_deg,
            noise: Some(noise_to_dat(&coloring.noise)),
        }
    }

    fn relief_to_dat(relief: &crate::genfloor::defs::FloorReliefV1) -> terrain::TerrainReliefV1 {
        let mode = match relief.mode {
            crate::genfloor::defs::FloorReliefMode::None => 0,
            crate::genfloor::defs::FloorReliefMode::Noise => 1,
        };
        terrain::TerrainReliefV1 {
            mode,
            amplitude: relief.amplitude,
            noise: Some(noise_to_dat(&relief.noise)),
        }
    }

    fn animation_to_dat(
        animation: &crate::genfloor::defs::FloorAnimationV1,
    ) -> terrain::TerrainAnimationV1 {
        let mode = match animation.mode {
            crate::genfloor::defs::FloorAnimationMode::None => 0,
            crate::genfloor::defs::FloorAnimationMode::Cpu => 1,
            crate::genfloor::defs::FloorAnimationMode::Gpu => 2,
        };
        terrain::TerrainAnimationV1 {
            mode,
            waves: animation
                .waves
                .iter()
                .map(|wave| terrain::TerrainWaveV1 {
                    amplitude: wave.amplitude,
                    wavelength: wave.wavelength,
                    direction_x: wave.direction[0],
                    direction_z: wave.direction[1],
                    speed: wave.speed,
                    phase: wave.phase,
                })
                .collect(),
            normal_strength: animation.normal_strength,
        }
    }

    crate::proto::gravimera::terrain::v1::TerrainDefV1 {
        format_version: def.format_version,
        label: def.label.clone(),
        mesh: Some(mesh_to_dat(&def.mesh)),
        material: Some(material_to_dat(&def.material)),
        coloring: Some(coloring_to_dat(&def.coloring)),
        relief: Some(relief_to_dat(&def.relief)),
        animation: Some(animation_to_dat(&def.animation)),
    }
}

fn terrain_def_from_dat(
    dat: &crate::proto::gravimera::terrain::v1::TerrainDefV1,
) -> crate::genfloor::defs::FloorDefV1 {
    use crate::proto::gravimera::terrain::v1 as terrain;

    fn noise_from_dat(dat: &terrain::TerrainNoiseV1) -> crate::genfloor::defs::FloorNoiseV1 {
        crate::genfloor::defs::FloorNoiseV1 {
            seed: dat.seed,
            frequency: dat.frequency,
            octaves: dat.octaves,
            lacunarity: dat.lacunarity,
            gain: dat.gain,
        }
    }

    fn mesh_from_dat(dat: &terrain::TerrainMeshV1) -> crate::genfloor::defs::FloorMeshV1 {
        crate::genfloor::defs::FloorMeshV1 {
            kind: crate::genfloor::defs::FloorMeshKind::Grid,
            size_m: [dat.size_x_m, dat.size_z_m],
            subdiv: [dat.subdiv_x, dat.subdiv_z],
            thickness_m: dat.thickness_m,
            uv_tiling: [dat.uv_tiling_x, dat.uv_tiling_z],
        }
    }

    fn material_from_dat(dat: &terrain::TerrainMaterialV1) -> crate::genfloor::defs::FloorMaterialV1 {
        crate::genfloor::defs::FloorMaterialV1 {
            base_color_rgba: [
                dat.base_color_r,
                dat.base_color_g,
                dat.base_color_b,
                dat.base_color_a,
            ],
            metallic: dat.metallic,
            roughness: dat.roughness,
            unlit: dat.unlit,
        }
    }

    fn coloring_from_dat(dat: &terrain::TerrainColoringV1) -> crate::genfloor::defs::FloorColoringV1 {
        let mode = match dat.mode {
            1 => crate::genfloor::defs::FloorColoringMode::Checker,
            2 => crate::genfloor::defs::FloorColoringMode::Stripes,
            3 => crate::genfloor::defs::FloorColoringMode::Gradient,
            4 => crate::genfloor::defs::FloorColoringMode::Noise,
            _ => crate::genfloor::defs::FloorColoringMode::Solid,
        };

        crate::genfloor::defs::FloorColoringV1 {
            mode,
            palette: dat
                .palette
                .iter()
                .map(|rgba| [rgba.r, rgba.g, rgba.b, rgba.a])
                .collect(),
            scale: [dat.scale_x, dat.scale_z],
            angle_deg: dat.angle_deg,
            noise: dat
                .noise
                .as_ref()
                .map(noise_from_dat)
                .unwrap_or_default(),
        }
    }

    fn relief_from_dat(dat: &terrain::TerrainReliefV1) -> crate::genfloor::defs::FloorReliefV1 {
        let mode = match dat.mode {
            1 => crate::genfloor::defs::FloorReliefMode::Noise,
            _ => crate::genfloor::defs::FloorReliefMode::None,
        };

        crate::genfloor::defs::FloorReliefV1 {
            mode,
            amplitude: dat.amplitude,
            noise: dat
                .noise
                .as_ref()
                .map(noise_from_dat)
                .unwrap_or_default(),
        }
    }

    fn animation_from_dat(
        dat: &terrain::TerrainAnimationV1,
    ) -> crate::genfloor::defs::FloorAnimationV1 {
        let mode = match dat.mode {
            2 => crate::genfloor::defs::FloorAnimationMode::Gpu,
            1 => crate::genfloor::defs::FloorAnimationMode::Cpu,
            _ => crate::genfloor::defs::FloorAnimationMode::None,
        };
        crate::genfloor::defs::FloorAnimationV1 {
            mode,
            waves: dat
                .waves
                .iter()
                .map(|wave| crate::genfloor::defs::FloorWaveV1 {
                    amplitude: wave.amplitude,
                    wavelength: wave.wavelength,
                    direction: [wave.direction_x, wave.direction_z],
                    speed: wave.speed,
                    phase: wave.phase,
                })
                .collect(),
            normal_strength: dat.normal_strength,
        }
    }

    let mut out = crate::genfloor::defs::FloorDefV1 {
        format_version: dat.format_version,
        label: dat.label.clone(),
        mesh: dat.mesh.as_ref().map(mesh_from_dat).unwrap_or_default(),
        material: dat
            .material
            .as_ref()
            .map(material_from_dat)
            .unwrap_or_default(),
        coloring: dat
            .coloring
            .as_ref()
            .map(coloring_from_dat)
            .unwrap_or_default(),
        relief: dat.relief.as_ref().map(relief_from_dat).unwrap_or_default(),
        animation: dat
            .animation
            .as_ref()
            .map(animation_from_dat)
            .unwrap_or_default(),
        extra: BTreeMap::default(),
    };
    out.canonicalize_in_place();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protobuf_roundtrip_uses_terrain_grav() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_terrain_selection_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let path = temp_root.join("terrain.grav");
        let legacy_path = temp_root.join("floor_selection.json");
        let floor_id = uuid::Uuid::new_v4().as_u128();

        save_scene_terrain_dat_to_paths(
            &path,
            &legacy_path,
            Some(floor_id),
            &crate::genfloor::defs::FloorDefV1::default_world(),
        )
        .expect("save terrain selection");
        let loaded = read_scene_floor_selection_from_build_dir(temp_root.as_path())
            .expect("load selection");

        assert_eq!(loaded, Some(floor_id));
        assert!(path.exists(), "terrain.grav should be written");
        assert!(
            !legacy_path.exists(),
            "legacy floor_selection.json should not exist after save"
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn migrates_legacy_json_into_protobuf() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_terrain_selection_legacy_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");

        let path = temp_root.join("terrain.grav");
        let legacy_path = temp_root.join("floor_selection.json");
        std::fs::write(
            &legacy_path,
            serde_json::to_vec_pretty(&LegacySceneFloorSelectionFileV1 {
                format_version: LEGACY_SCENE_FLOOR_SELECTION_FORMAT_VERSION,
                floor_id: None,
            })
            .expect("encode legacy json"),
        )
        .expect("write legacy json");

        let loaded = load_scene_floor_selection_from_paths("default", &path, &legacy_path)
            .expect("migrate selection");

        assert_eq!(loaded.terrain_id, None);
        assert!(path.exists(), "terrain.grav should exist after migration");
        assert!(
            !legacy_path.exists(),
            "legacy floor_selection.json should be removed after migration"
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
