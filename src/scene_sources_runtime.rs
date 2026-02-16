use bevy::prelude::*;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::constants::DEFAULT_OBJECT_SIZE_M;
use crate::object::registry::{ColliderProfile, ObjectLibrary};
use crate::scene_sources::{SceneSourcesIndexPaths, SceneSourcesV1, SCENE_SOURCES_FORMAT_VERSION};
use crate::scene_sources_patch::{
    apply_patch_to_sources, SceneSourcesPatchSummaryV1, SceneSourcesPatchV1,
};
use crate::scene_validation::{
    HardGateSpecV1, ProvenanceSummaryV1, ScorecardSpecV1, ValidationReportV1,
    ValidationViolationV1, ViolationEvidenceV1, ViolationSeverityV1,
};
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, ObjectId, ObjectPrefabId,
    ObjectTint, SceneLayerOwner,
};

#[derive(Resource, Default)]
pub(crate) struct SceneSourcesWorkspace {
    pub(crate) loaded_from_dir: Option<PathBuf>,
    pub(crate) sources: Option<SceneSourcesV1>,
}

#[derive(Debug)]
pub(crate) struct SceneSourcesExportReport {
    pub(crate) instance_count: usize,
}

#[derive(Debug)]
pub(crate) struct SceneSourcesImportReport {
    pub(crate) instance_count: usize,
}

pub(crate) fn import_scene_sources_replace_world(
    commands: &mut Commands,
    workspace: &mut SceneSourcesWorkspace,
    library: &ObjectLibrary,
    src_dir: &Path,
    existing_scene_entities: impl Iterator<Item = Entity>,
) -> Result<SceneSourcesImportReport, String> {
    let sources = SceneSourcesV1::load_from_dir(src_dir).map_err(|err| err.to_string())?;

    // Replace the current scene contents (but keep Player, Ground, etc. which are not in the
    // `existing_scene_entities` iterator).
    for entity in existing_scene_entities {
        commands.entity(entity).try_despawn();
    }

    let pinned_instances = parse_pinned_instances(&sources)?;
    for instance in pinned_instances.values() {
        spawn_scene_instance_minimal(commands, library, instance)?;
    }

    workspace.loaded_from_dir = Some(src_dir.to_path_buf());
    workspace.sources = Some(sources);

    Ok(SceneSourcesImportReport {
        instance_count: pinned_instances.len(),
    })
}

pub(crate) fn export_scene_sources_from_world<'a>(
    workspace: &SceneSourcesWorkspace,
    objects: impl Iterator<
        Item = (
            &'a Transform,
            &'a ObjectId,
            &'a ObjectPrefabId,
            Option<&'a ObjectTint>,
        ),
    >,
    out_dir: &Path,
) -> Result<SceneSourcesExportReport, String> {
    let Some(base_sources) = workspace.sources.as_ref() else {
        return Err("No scene sources have been imported in this session.".to_string());
    };

    let mut sources = base_sources.clone();

    let index_paths = SceneSourcesIndexPaths::from_index_json_value(&sources.index_json)
        .map_err(|err| format!("Invalid scene sources index.json: {err}"))?;
    let pinned_dir = index_paths.pinned_instances_dir;

    let existing_docs_by_instance_id =
        map_existing_pinned_docs_by_instance_id(&sources, &pinned_dir);

    // Replace pinned instances with the current world state.
    sources
        .extra_json_files
        .retain(|path, _| !is_under_dir(path, &pinned_dir));

    let mut count = 0usize;
    for (transform, instance_id, prefab_id, tint) in objects {
        let doc = build_pinned_instance_doc(
            existing_docs_by_instance_id.get(&instance_id.0),
            instance_id,
            prefab_id,
            transform,
            tint,
        )?;
        let rel_path = pinned_dir.join(format!("{}.json", uuid::Uuid::from_u128(instance_id.0)));
        sources.extra_json_files.insert(rel_path, doc);
        count += 1;
    }

    // Make sure all sources written are at the currently supported format version.
    sources.index_json["format_version"] = Value::from(SCENE_SOURCES_FORMAT_VERSION);
    sources.meta_json["format_version"] = Value::from(SCENE_SOURCES_FORMAT_VERSION);
    sources.markers_json["format_version"] = Value::from(SCENE_SOURCES_FORMAT_VERSION);
    sources.style_pack_ref_json["format_version"] = Value::from(SCENE_SOURCES_FORMAT_VERSION);

    sources
        .write_to_dir(out_dir)
        .map_err(|err| err.to_string())?;

    Ok(SceneSourcesExportReport {
        instance_count: count,
    })
}

#[derive(Clone, Debug)]
struct PinnedInstance {
    instance_id: ObjectId,
    prefab_id: ObjectPrefabId,
    transform: Transform,
    tint: Option<Color>,
    source_rel_path: Option<PathBuf>,
}

fn parse_pinned_instances(
    sources: &SceneSourcesV1,
) -> Result<BTreeMap<u128, PinnedInstance>, String> {
    let index_paths = SceneSourcesIndexPaths::from_index_json_value(&sources.index_json)
        .map_err(|err| format!("Invalid scene sources index.json: {err}"))?;
    let pinned_dir = index_paths.pinned_instances_dir;

    let mut out = BTreeMap::new();
    for (rel_path, doc) in &sources.extra_json_files {
        if !is_under_dir(rel_path, &pinned_dir) {
            continue;
        }
        let instance = parse_pinned_instance_doc(rel_path, doc)?;
        let instance_id = instance.instance_id.0;
        if out.insert(instance_id, instance).is_some() {
            return Err(format!(
                "Duplicate pinned instance_id in sources: {}",
                uuid::Uuid::from_u128(instance_id)
            ));
        }
    }
    Ok(out)
}

fn parse_pinned_instance_doc(path: &Path, doc: &Value) -> Result<PinnedInstance, String> {
    let Value::Object(map) = doc else {
        return Err(format!(
            "{}: pinned instance must be a JSON object",
            path.display()
        ));
    };

    let format_version = map
        .get("format_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("{}: missing format_version", path.display()))?;
    if format_version != SCENE_SOURCES_FORMAT_VERSION as u64 {
        return Err(format!(
            "{}: unsupported format_version {} (expected {})",
            path.display(),
            format_version,
            SCENE_SOURCES_FORMAT_VERSION
        ));
    }

    let instance_uuid = map
        .get("instance_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{}: missing instance_id", path.display()))?;
    let instance_uuid = uuid::Uuid::parse_str(instance_uuid.trim())
        .map_err(|err| format!("{}: invalid instance_id UUID: {err}", path.display()))?;

    let prefab_uuid = map
        .get("prefab_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{}: missing prefab_id", path.display()))?;
    let prefab_uuid = uuid::Uuid::parse_str(prefab_uuid.trim())
        .map_err(|err| format!("{}: invalid prefab_id UUID: {err}", path.display()))?;

    let transform_val = map
        .get("transform")
        .ok_or_else(|| format!("{}: missing transform", path.display()))?;
    let translation = transform_val
        .get("translation")
        .map(parse_vec3)
        .transpose()?
        .unwrap_or(Vec3::ZERO);
    let rotation = transform_val
        .get("rotation")
        .map(parse_quat)
        .transpose()?
        .unwrap_or(Quat::IDENTITY);
    let scale = transform_val
        .get("scale")
        .map(parse_vec3)
        .transpose()?
        .unwrap_or(Vec3::ONE);

    let tint = map.get("tint_rgba").map(parse_color).transpose()?;

    let transform = sanitize_transform(
        Transform::from_translation(translation)
            .with_rotation(rotation)
            .with_scale(scale),
    )?;

    Ok(PinnedInstance {
        instance_id: ObjectId(instance_uuid.as_u128()),
        prefab_id: ObjectPrefabId(prefab_uuid.as_u128()),
        transform,
        tint,
        source_rel_path: Some(path.to_path_buf()),
    })
}

fn sanitize_transform(mut transform: Transform) -> Result<Transform, String> {
    if !transform.translation.is_finite() {
        return Err("transform.translation must be finite".to_string());
    }

    let mut q = transform.rotation;
    if !q.is_finite() {
        q = Quat::IDENTITY;
    }
    let len2 = q.length_squared();
    if !len2.is_finite() || len2 <= 1e-8 {
        q = Quat::IDENTITY;
    } else {
        q = q.normalize();
    }
    transform.rotation = q;

    let mut scale = transform.scale;
    for v in [&mut scale.x, &mut scale.y, &mut scale.z] {
        if !v.is_finite() || v.abs() < 1e-4 {
            *v = 1.0;
        }
    }
    transform.scale = scale;

    Ok(transform)
}

fn spawn_scene_instance_minimal(
    commands: &mut Commands,
    library: &ObjectLibrary,
    instance: &PinnedInstance,
) -> Result<Entity, String> {
    if library.mobility(instance.prefab_id.0).is_some() {
        Ok(spawn_unit_minimal(
            commands,
            library,
            instance.prefab_id.0,
            instance.transform,
            instance.instance_id,
            instance.tint,
        ))
    } else {
        Ok(spawn_build_object_minimal(
            commands,
            library,
            instance.prefab_id.0,
            instance.transform,
            instance.instance_id,
            instance.tint,
        ))
    }
}

fn spawn_build_object_minimal(
    commands: &mut Commands,
    library: &ObjectLibrary,
    prefab_id: u128,
    mut transform: Transform,
    instance_id: ObjectId,
    tint: Option<Color>,
) -> Entity {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));

    transform.scale = sanitize_scale(transform.scale);

    let (yaw, _pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);
    let c = yaw.cos().abs();
    let s = yaw.sin().abs();

    let scale = transform.scale;
    let (collider_half_xz, size) = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => {
            let r = radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01);
            (
                Vec2::splat(r),
                Vec3::new(r * 2.0, base_size.y * scale.y.abs(), r * 2.0),
            )
        }
        Some(ColliderProfile::AabbXZ { half_extents }) => {
            let half = Vec2::new(
                half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
                half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
            );
            let rotated = Vec2::new(c * half.x + s * half.y, s * half.x + c * half.y);
            (
                rotated,
                Vec3::new(
                    rotated.x * 2.0,
                    base_size.y * scale.y.abs(),
                    rotated.y * 2.0,
                ),
            )
        }
        _ => {
            let half = Vec2::new(
                (base_size.x * 0.5).abs().max(0.01) * scale.x.abs().max(0.01),
                (base_size.z * 0.5).abs().max(0.01) * scale.z.abs().max(0.01),
            );
            let rotated = Vec2::new(c * half.x + s * half.y, s * half.x + c * half.y);
            (
                rotated,
                Vec3::new(
                    rotated.x * 2.0,
                    base_size.y * scale.y.abs(),
                    rotated.y * 2.0,
                ),
            )
        }
    };

    let mut entity_commands = commands.spawn((
        instance_id,
        ObjectPrefabId(prefab_id),
        BuildObject,
        BuildDimensions { size },
        AabbCollider {
            half_extents: collider_half_xz,
        },
        transform,
    ));
    if let Some(tint) = tint {
        entity_commands.insert(ObjectTint(tint));
    }
    entity_commands.id()
}

fn spawn_unit_minimal(
    commands: &mut Commands,
    library: &ObjectLibrary,
    prefab_id: u128,
    mut transform: Transform,
    instance_id: ObjectId,
    tint: Option<Color>,
) -> Entity {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));

    transform.scale = sanitize_scale(transform.scale);

    let scale = transform.scale;
    let radius = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => {
            radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01)
        }
        Some(ColliderProfile::AabbXZ { half_extents }) => {
            let half = Vec2::new(
                half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
                half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
            );
            half.x.max(half.y)
        }
        _ => {
            let size = Vec2::new(
                (base_size.x * scale.x.abs()).abs().max(0.01),
                (base_size.z * scale.z.abs()).abs().max(0.01),
            );
            (size.x.max(size.y) * 0.5).max(0.01)
        }
    };

    let mut entity_commands = commands.spawn((
        instance_id,
        ObjectPrefabId(prefab_id),
        Commandable,
        Collider { radius },
        transform,
    ));
    if let Some(tint) = tint {
        entity_commands.insert(ObjectTint(tint));
    }
    entity_commands.id()
}

fn sanitize_scale(mut scale: Vec3) -> Vec3 {
    for v in [&mut scale.x, &mut scale.y, &mut scale.z] {
        if !v.is_finite() || v.abs() < 1e-4 {
            *v = 1.0;
        }
    }
    scale
}

fn build_pinned_instance_doc(
    existing: Option<&Value>,
    instance_id: &ObjectId,
    prefab_id: &ObjectPrefabId,
    transform: &Transform,
    tint: Option<&ObjectTint>,
) -> Result<Value, String> {
    let mut doc = existing
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    if !doc.is_object() {
        doc = Value::Object(Default::default());
    }
    let map = doc.as_object_mut().expect("doc is object");

    map.insert(
        "format_version".to_string(),
        Value::from(SCENE_SOURCES_FORMAT_VERSION),
    );
    map.insert(
        "instance_id".to_string(),
        Value::from(uuid::Uuid::from_u128(instance_id.0).to_string()),
    );
    map.insert(
        "prefab_id".to_string(),
        Value::from(uuid::Uuid::from_u128(prefab_id.0).to_string()),
    );

    let translation = vec3_json(transform.translation);
    let rotation = quat_json(transform.rotation);
    let scale = vec3_json(transform.scale);
    map.insert(
        "transform".to_string(),
        serde_json::json!({
            "translation": translation,
            "rotation": rotation,
            "scale": scale,
        }),
    );

    if let Some(tint) = tint {
        map.insert("tint_rgba".to_string(), color_json(tint.0));
    } else {
        map.remove("tint_rgba");
    }

    Ok(doc)
}

fn vec3_json(v: Vec3) -> Value {
    serde_json::json!({ "x": v.x, "y": v.y, "z": v.z })
}

fn quat_json(q: Quat) -> Value {
    serde_json::json!({ "x": q.x, "y": q.y, "z": q.z, "w": q.w })
}

fn color_json(c: Color) -> Value {
    // Use a stable representation: linear RGBA floats.
    let linear = c.to_linear();
    serde_json::json!({
        "r": linear.red,
        "g": linear.green,
        "b": linear.blue,
        "a": linear.alpha,
    })
}

fn parse_vec3(value: &Value) -> Result<Vec3, String> {
    match value {
        Value::Object(map) => {
            let x = map
                .get("x")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "vec3.x must be a number".to_string())?;
            let y = map
                .get("y")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "vec3.y must be a number".to_string())?;
            let z = map
                .get("z")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "vec3.z must be a number".to_string())?;
            Ok(Vec3::new(x as f32, y as f32, z as f32))
        }
        Value::Array(items) => {
            if items.len() != 3 {
                return Err("vec3 array must have 3 elements".to_string());
            }
            let x = items[0]
                .as_f64()
                .ok_or_else(|| "vec3[0] must be a number".to_string())?;
            let y = items[1]
                .as_f64()
                .ok_or_else(|| "vec3[1] must be a number".to_string())?;
            let z = items[2]
                .as_f64()
                .ok_or_else(|| "vec3[2] must be a number".to_string())?;
            Ok(Vec3::new(x as f32, y as f32, z as f32))
        }
        _ => Err("expected vec3 object or array".to_string()),
    }
}

fn parse_quat(value: &Value) -> Result<Quat, String> {
    match value {
        Value::Object(map) => {
            let x = map
                .get("x")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "quat.x must be a number".to_string())?;
            let y = map
                .get("y")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "quat.y must be a number".to_string())?;
            let z = map
                .get("z")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "quat.z must be a number".to_string())?;
            let w = map
                .get("w")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "quat.w must be a number".to_string())?;
            Ok(Quat::from_xyzw(x as f32, y as f32, z as f32, w as f32))
        }
        Value::Array(items) => {
            if items.len() != 4 {
                return Err("quat array must have 4 elements".to_string());
            }
            let x = items[0]
                .as_f64()
                .ok_or_else(|| "quat[0] must be a number".to_string())?;
            let y = items[1]
                .as_f64()
                .ok_or_else(|| "quat[1] must be a number".to_string())?;
            let z = items[2]
                .as_f64()
                .ok_or_else(|| "quat[2] must be a number".to_string())?;
            let w = items[3]
                .as_f64()
                .ok_or_else(|| "quat[3] must be a number".to_string())?;
            Ok(Quat::from_xyzw(x as f32, y as f32, z as f32, w as f32))
        }
        _ => Err("expected quat object or array".to_string()),
    }
}

fn parse_color(value: &Value) -> Result<Color, String> {
    match value {
        Value::Object(map) => {
            let r = map
                .get("r")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "color.r must be a number".to_string())?;
            let g = map
                .get("g")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "color.g must be a number".to_string())?;
            let b = map
                .get("b")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "color.b must be a number".to_string())?;
            let a = map
                .get("a")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "color.a must be a number".to_string())?;
            Ok(Color::linear_rgba(r as f32, g as f32, b as f32, a as f32))
        }
        Value::Array(items) => {
            if items.len() != 4 {
                return Err("color array must have 4 elements".to_string());
            }
            let r = items[0]
                .as_f64()
                .ok_or_else(|| "color[0] must be a number".to_string())?;
            let g = items[1]
                .as_f64()
                .ok_or_else(|| "color[1] must be a number".to_string())?;
            let b = items[2]
                .as_f64()
                .ok_or_else(|| "color[2] must be a number".to_string())?;
            let a = items[3]
                .as_f64()
                .ok_or_else(|| "color[3] must be a number".to_string())?;
            Ok(Color::linear_rgba(r as f32, g as f32, b as f32, a as f32))
        }
        _ => Err("expected color object or array".to_string()),
    }
}

fn map_existing_pinned_docs_by_instance_id(
    sources: &SceneSourcesV1,
    pinned_dir: &Path,
) -> HashMap<u128, Value> {
    let mut map = HashMap::new();
    for (path, doc) in &sources.extra_json_files {
        if !is_under_dir(path, pinned_dir) {
            continue;
        }
        let Some(id_str) = doc.get("instance_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Ok(id) = uuid::Uuid::parse_str(id_str.trim()) else {
            continue;
        };
        map.insert(id.as_u128(), doc.clone());
    }
    map
}

fn is_under_dir(path: &Path, dir: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(dir) else {
        return false;
    };
    // Only match files directly under the directory or in subdirectories.
    !rel.as_os_str().is_empty()
}

#[derive(Clone, Debug)]
pub(crate) struct SceneWorldInstance {
    pub(crate) entity: Entity,
    pub(crate) instance_id: ObjectId,
    pub(crate) prefab_id: ObjectPrefabId,
    pub(crate) transform: Transform,
    pub(crate) tint: Option<Color>,
    pub(crate) owner_layer_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SceneCompileReport {
    pub(crate) spawned: usize,
    pub(crate) updated: usize,
    pub(crate) despawned: usize,
    pub(crate) layers_compiled: usize,
    pub(crate) pinned_upserts: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct SceneSignatureSummary {
    pub(crate) overall_sig: String,
    pub(crate) pinned_sig: String,
    pub(crate) layer_sigs: BTreeMap<String, String>,
    pub(crate) total_instances: usize,
    pub(crate) pinned_instances: usize,
    pub(crate) layer_instance_counts: BTreeMap<String, usize>,
}

pub(crate) fn reload_scene_sources_in_workspace(
    workspace: &mut SceneSourcesWorkspace,
) -> Result<(), String> {
    let Some(src_dir) = workspace.loaded_from_dir.as_ref() else {
        return Err("No scene sources directory has been imported in this session.".to_string());
    };

    let sources = SceneSourcesV1::load_from_dir(src_dir).map_err(|err| err.to_string())?;
    workspace.sources = Some(sources);
    Ok(())
}

#[derive(Clone, Debug)]
enum SceneLayer {
    ExplicitInstances(ExplicitInstancesLayer),
    GridInstances(GridInstancesLayer),
    PolylineInstances(PolylineInstancesLayer),
}

#[derive(Clone, Debug)]
struct ExplicitInstancesLayer {
    layer_id: String,
    instances: Vec<ExplicitInstanceSpec>,
    source_rel_path: PathBuf,
}

#[derive(Clone, Debug)]
struct GridInstancesLayer {
    layer_id: String,
    prefab_id: ObjectPrefabId,
    origin: Vec3,
    count_x: u32,
    count_z: u32,
    step_x: f32,
    step_z: f32,
    rotation: Quat,
    scale: Vec3,
    tint: Option<Color>,
    source_rel_path: PathBuf,
}

#[derive(Clone, Debug)]
struct PolylineInstancesLayer {
    layer_id: String,
    prefab_id: ObjectPrefabId,
    points: Vec<Vec3>,
    spacing: f32,
    start_offset: f32,
    rotation: Quat,
    scale: Vec3,
    tint: Option<Color>,
    source_rel_path: PathBuf,
}

#[derive(Clone, Debug)]
struct ExplicitInstanceSpec {
    local_id: String,
    prefab_id: ObjectPrefabId,
    transform: Transform,
    tint: Option<Color>,
}

fn parse_layers(sources: &SceneSourcesV1) -> Result<BTreeMap<String, SceneLayer>, String> {
    let index_paths = SceneSourcesIndexPaths::from_index_json_value(&sources.index_json)
        .map_err(|err| format!("Invalid scene sources index.json: {err}"))?;
    let layers_dir = index_paths.layers_dir;

    let mut out: BTreeMap<String, SceneLayer> = BTreeMap::new();
    for (rel_path, doc) in &sources.extra_json_files {
        if !is_under_dir(rel_path, &layers_dir) {
            continue;
        }
        let layer = parse_layer_doc(rel_path, doc)?;
        let (layer_id, layer) = match layer {
            SceneLayer::ExplicitInstances(layer) => {
                (layer.layer_id.clone(), SceneLayer::ExplicitInstances(layer))
            }
            SceneLayer::GridInstances(layer) => {
                (layer.layer_id.clone(), SceneLayer::GridInstances(layer))
            }
            SceneLayer::PolylineInstances(layer) => {
                (layer.layer_id.clone(), SceneLayer::PolylineInstances(layer))
            }
        };
        if out.insert(layer_id.clone(), layer).is_some() {
            return Err(format!("Duplicate layer_id in sources: {layer_id}"));
        }
    }
    Ok(out)
}

fn parse_layer_doc(path: &Path, doc: &Value) -> Result<SceneLayer, String> {
    let Value::Object(map) = doc else {
        return Err(format!("{}: layer must be a JSON object", path.display()));
    };

    let format_version = map
        .get("format_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("{}: missing format_version", path.display()))?;
    if format_version != SCENE_SOURCES_FORMAT_VERSION as u64 {
        return Err(format!(
            "{}: unsupported format_version {} (expected {})",
            path.display(),
            format_version,
            SCENE_SOURCES_FORMAT_VERSION
        ));
    }

    let layer_id = map
        .get("layer_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{}: missing layer_id", path.display()))?
        .to_string();

    let kind = map
        .get("kind")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{}: missing kind", path.display()))?;

    match kind {
        "explicit_instances" => {
            let instances_val = map
                .get("instances")
                .ok_or_else(|| format!("{}: missing instances", path.display()))?;
            let Value::Array(instances) = instances_val else {
                return Err(format!("{}: instances must be an array", path.display()));
            };
            let mut parsed = Vec::with_capacity(instances.len());
            let mut seen_local_ids = HashSet::new();
            for (idx, item) in instances.iter().enumerate() {
                let spec = parse_explicit_instance_spec(path, idx, item)?;
                if !seen_local_ids.insert(spec.local_id.clone()) {
                    return Err(format!(
                        "{}: duplicate local_id in instances: {}",
                        path.display(),
                        spec.local_id
                    ));
                }
                parsed.push(spec);
            }
            parsed.sort_by(|a, b| a.local_id.cmp(&b.local_id));
            Ok(SceneLayer::ExplicitInstances(ExplicitInstancesLayer {
                layer_id,
                instances: parsed,
                source_rel_path: path.to_path_buf(),
            }))
        }
        "grid_instances" => {
            let prefab_uuid = map
                .get("prefab_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("{}: missing prefab_id", path.display()))?;
            let prefab_uuid = uuid::Uuid::parse_str(prefab_uuid.trim())
                .map_err(|err| format!("{}: invalid prefab_id UUID: {err}", path.display()))?;

            let origin_val = map
                .get("origin")
                .ok_or_else(|| format!("{}: missing origin", path.display()))?;
            let origin = parse_vec3(origin_val)?;
            if !origin.is_finite() {
                return Err(format!("{}: origin must be finite", path.display()));
            }

            let count_val = map
                .get("count")
                .ok_or_else(|| format!("{}: missing count", path.display()))?;
            let Value::Object(count_map) = count_val else {
                return Err(format!("{}: count must be an object", path.display()));
            };
            let count_x = count_map
                .get("x")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| format!("{}: count.x must be an integer", path.display()))?;
            let count_z = count_map
                .get("z")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| format!("{}: count.z must be an integer", path.display()))?;
            let count_x = u32::try_from(count_x)
                .map_err(|_| format!("{}: count.x out of range", path.display()))?;
            let count_z = u32::try_from(count_z)
                .map_err(|_| format!("{}: count.z out of range", path.display()))?;

            let step_val = map
                .get("step")
                .ok_or_else(|| format!("{}: missing step", path.display()))?;
            let Value::Object(step_map) = step_val else {
                return Err(format!("{}: step must be an object", path.display()));
            };
            let step_x = step_map
                .get("x")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| format!("{}: step.x must be a number", path.display()))?
                as f32;
            let step_z = step_map
                .get("z")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| format!("{}: step.z must be a number", path.display()))?
                as f32;
            if !step_x.is_finite() || step_x == 0.0 {
                return Err(format!(
                    "{}: step.x must be finite and non-zero",
                    path.display()
                ));
            }
            if !step_z.is_finite() || step_z == 0.0 {
                return Err(format!(
                    "{}: step.z must be finite and non-zero",
                    path.display()
                ));
            }

            let rotation = map
                .get("rotation")
                .map(parse_quat)
                .transpose()?
                .unwrap_or(Quat::IDENTITY);
            let scale = map
                .get("scale")
                .map(parse_vec3)
                .transpose()?
                .unwrap_or(Vec3::ONE);
            let tint = map.get("tint_rgba").map(parse_color).transpose()?;

            let template = sanitize_transform(
                Transform::from_translation(Vec3::ZERO)
                    .with_rotation(rotation)
                    .with_scale(scale),
            )?;

            Ok(SceneLayer::GridInstances(GridInstancesLayer {
                layer_id,
                prefab_id: ObjectPrefabId(prefab_uuid.as_u128()),
                origin,
                count_x,
                count_z,
                step_x,
                step_z,
                rotation: template.rotation,
                scale: template.scale,
                tint,
                source_rel_path: path.to_path_buf(),
            }))
        }
        "polyline_instances" => {
            let prefab_uuid = map
                .get("prefab_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("{}: missing prefab_id", path.display()))?;
            let prefab_uuid = uuid::Uuid::parse_str(prefab_uuid.trim())
                .map_err(|err| format!("{}: invalid prefab_id UUID: {err}", path.display()))?;

            let points_val = map
                .get("points")
                .ok_or_else(|| format!("{}: missing points", path.display()))?;
            let Value::Array(points_items) = points_val else {
                return Err(format!("{}: points must be an array", path.display()));
            };
            if points_items.len() < 2 {
                return Err(format!(
                    "{}: points must contain at least 2 points",
                    path.display()
                ));
            }
            let mut points = Vec::with_capacity(points_items.len());
            for (idx, item) in points_items.iter().enumerate() {
                let p = parse_vec3(item).map_err(|err| {
                    format!("{}: points[{}] invalid vec3: {err}", path.display(), idx)
                })?;
                if !p.is_finite() {
                    return Err(format!(
                        "{}: points[{}] must be finite",
                        path.display(),
                        idx
                    ));
                }
                points.push(p);
            }
            for idx in 0..(points.len() - 1) {
                let a = points[idx];
                let b = points[idx + 1];
                if (b - a).length_squared() == 0.0 {
                    return Err(format!(
                        "{}: points[{}] and points[{}] must not be identical (zero-length segment)",
                        path.display(),
                        idx,
                        idx + 1
                    ));
                }
            }

            let spacing = map
                .get("spacing")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| format!("{}: spacing must be a number", path.display()))?
                as f32;
            if !spacing.is_finite() || spacing <= 0.0 {
                return Err(format!(
                    "{}: spacing must be finite and > 0",
                    path.display()
                ));
            }

            let start_offset = match map.get("start_offset") {
                None => 0.0f32,
                Some(v) => v
                    .as_f64()
                    .ok_or_else(|| format!("{}: start_offset must be a number", path.display()))?
                    as f32,
            };
            if !start_offset.is_finite() || start_offset < 0.0 {
                return Err(format!(
                    "{}: start_offset must be finite and >= 0",
                    path.display()
                ));
            }

            let rotation = map
                .get("rotation")
                .map(parse_quat)
                .transpose()?
                .unwrap_or(Quat::IDENTITY);
            let scale = map
                .get("scale")
                .map(parse_vec3)
                .transpose()?
                .unwrap_or(Vec3::ONE);
            let tint = map.get("tint_rgba").map(parse_color).transpose()?;

            let template = sanitize_transform(
                Transform::from_translation(Vec3::ZERO)
                    .with_rotation(rotation)
                    .with_scale(scale),
            )?;

            Ok(SceneLayer::PolylineInstances(PolylineInstancesLayer {
                layer_id,
                prefab_id: ObjectPrefabId(prefab_uuid.as_u128()),
                points,
                spacing,
                start_offset,
                rotation: template.rotation,
                scale: template.scale,
                tint,
                source_rel_path: path.to_path_buf(),
            }))
        }
        other => Err(format!(
            "{}: unsupported layer kind: {}",
            path.display(),
            other
        )),
    }
}

fn parse_explicit_instance_spec(
    path: &Path,
    idx: usize,
    doc: &Value,
) -> Result<ExplicitInstanceSpec, String> {
    let Value::Object(map) = doc else {
        return Err(format!(
            "{}: instances[{}] must be an object",
            path.display(),
            idx
        ));
    };

    let local_id = map
        .get("local_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{}: instances[{}] missing local_id", path.display(), idx))?
        .to_string();

    let prefab_uuid = map
        .get("prefab_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{}: instances[{}] missing prefab_id", path.display(), idx))?;
    let prefab_uuid = uuid::Uuid::parse_str(prefab_uuid.trim()).map_err(|err| {
        format!(
            "{}: instances[{}] invalid prefab_id UUID: {err}",
            path.display(),
            idx
        )
    })?;

    let transform_val = map
        .get("transform")
        .ok_or_else(|| format!("{}: instances[{}] missing transform", path.display(), idx))?;
    let translation = transform_val
        .get("translation")
        .map(parse_vec3)
        .transpose()?
        .unwrap_or(Vec3::ZERO);
    let rotation = transform_val
        .get("rotation")
        .map(parse_quat)
        .transpose()?
        .unwrap_or(Quat::IDENTITY);
    let scale = transform_val
        .get("scale")
        .map(parse_vec3)
        .transpose()?
        .unwrap_or(Vec3::ONE);

    let tint = map.get("tint_rgba").map(parse_color).transpose()?;

    let transform = sanitize_transform(
        Transform::from_translation(translation)
            .with_rotation(rotation)
            .with_scale(scale),
    )?;

    Ok(ExplicitInstanceSpec {
        local_id,
        prefab_id: ObjectPrefabId(prefab_uuid.as_u128()),
        transform,
        tint,
    })
}

fn derived_layer_instance_id(scene_id: &str, layer_id: &str, local_id: &str) -> ObjectId {
    let key =
        format!("gravimera/scene_sources/v1/scene/{scene_id}/layer/{layer_id}/instance/{local_id}");
    ObjectId(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes()).as_u128())
}

#[derive(Clone, Debug)]
struct PortalSpec {
    portal_id: String,
    destination_scene_id: String,
    from_marker_id: Option<String>,
    source_rel_path: PathBuf,
}

fn parse_portals(sources: &SceneSourcesV1) -> Result<Vec<PortalSpec>, String> {
    let index_paths = SceneSourcesIndexPaths::from_index_json_value(&sources.index_json)
        .map_err(|err| format!("Invalid scene sources index.json: {err}"))?;
    let portals_dir = index_paths.portals_dir;

    let mut out = Vec::new();
    for (rel_path, doc) in &sources.extra_json_files {
        if !is_under_dir(rel_path, &portals_dir) {
            continue;
        }
        out.push(parse_portal_doc(rel_path, doc)?);
    }

    out.sort_by(|a, b| a.portal_id.cmp(&b.portal_id));
    Ok(out)
}

fn parse_portal_doc(path: &Path, doc: &Value) -> Result<PortalSpec, String> {
    let Value::Object(map) = doc else {
        return Err(format!("{}: portal must be a JSON object", path.display()));
    };

    let format_version = map
        .get("format_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("{}: missing format_version", path.display()))?;
    if format_version != SCENE_SOURCES_FORMAT_VERSION as u64 {
        return Err(format!(
            "{}: unsupported format_version {} (expected {})",
            path.display(),
            format_version,
            SCENE_SOURCES_FORMAT_VERSION
        ));
    }

    let portal_id = map
        .get("portal_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{}: missing portal_id", path.display()))?
        .to_string();

    let destination_scene_id = map
        .get("destination_scene_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{}: missing destination_scene_id", path.display()))?
        .to_string();

    Ok(PortalSpec {
        portal_id,
        destination_scene_id,
        from_marker_id: map
            .get("from_marker_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        source_rel_path: path.to_path_buf(),
    })
}

fn discover_known_scene_ids(src_dir: &Path) -> Result<HashSet<String>, String> {
    let src_name = src_dir.file_name().and_then(|s| s.to_str());
    if src_name != Some("src") {
        return Ok(HashSet::new());
    }

    let scene_dir = src_dir
        .parent()
        .ok_or_else(|| format!("invalid src dir (no parent): {}", src_dir.display()))?;
    let scenes_dir = scene_dir
        .parent()
        .ok_or_else(|| format!("invalid scene dir (no parent): {}", scene_dir.display()))?;

    let mut out = HashSet::new();
    let entries = std::fs::read_dir(scenes_dir)
        .map_err(|err| format!("read_dir failed ({}): {err}", scenes_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read_dir entry failed: {err}"))?;
        let ty = entry
            .file_type()
            .map_err(|err| format!("stat scene dir entry failed: {err}"))?;
        if !ty.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(scene_id) = name.to_str() else {
            continue;
        };

        let candidate_src = entry.path().join("src/index.json");
        if candidate_src.exists() {
            out.insert(scene_id.to_string());
        }
    }

    Ok(out)
}

fn marker_ids_from_sources(sources: &SceneSourcesV1) -> HashSet<String> {
    let Some(markers_obj) = sources
        .markers_json
        .get("markers")
        .and_then(|v| v.as_object())
    else {
        return HashSet::new();
    };

    markers_obj.keys().cloned().collect()
}

pub(crate) fn validate_scene_sources(
    workspace: &SceneSourcesWorkspace,
    library: &ObjectLibrary,
    existing_instances: impl Iterator<Item = SceneWorldInstance>,
    scorecard: &ScorecardSpecV1,
) -> Result<ValidationReportV1, String> {
    let Some(sources) = workspace.sources.as_ref() else {
        return Err("No scene sources have been imported in this session.".to_string());
    };

    validate_scene_sources_impl(
        sources,
        workspace.loaded_from_dir.as_deref(),
        library,
        existing_instances,
        scorecard,
    )
}

fn validate_scene_sources_impl(
    sources: &SceneSourcesV1,
    loaded_from_dir: Option<&Path>,
    library: &ObjectLibrary,
    existing_instances: impl Iterator<Item = SceneWorldInstance>,
    scorecard: &ScorecardSpecV1,
) -> Result<ValidationReportV1, String> {
    if scorecard.format_version != crate::scene_validation::SCORECARD_FORMAT_VERSION {
        return Err(format!(
            "Unsupported scorecard format_version {} (expected {})",
            scorecard.format_version,
            crate::scene_validation::SCORECARD_FORMAT_VERSION
        ));
    }

    let scene_id = sources
        .meta_json
        .get("scene_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let mut report = ValidationReportV1::new(scene_id.clone());

    let pinned_instances = parse_pinned_instances(sources)?;
    let layers = parse_layers(sources)?;
    let portals = parse_portals(sources)?;
    let marker_ids = marker_ids_from_sources(sources);

    let predicted_pinned = pinned_instances.len();
    let mut predicted_by_layer: BTreeMap<String, usize> = BTreeMap::new();
    for (layer_id, layer) in &layers {
        let count = match layer {
            SceneLayer::ExplicitInstances(layer) => layer.instances.len(),
            SceneLayer::GridInstances(layer) => grid_instances_predicted_count(layer)?,
            SceneLayer::PolylineInstances(layer) => polyline_instances_predicted_count(layer)?,
        };
        predicted_by_layer.insert(layer_id.clone(), count);
    }
    let predicted_layers_total: usize = predicted_by_layer.values().sum();
    let predicted_total_instances = predicted_pinned + predicted_layers_total;

    report.metrics.insert(
        "predicted_total_instances".to_string(),
        serde_json::Value::from(predicted_total_instances as u64),
    );
    report.metrics.insert(
        "predicted_pinned_instances".to_string(),
        serde_json::Value::from(predicted_pinned as u64),
    );
    report.metrics.insert(
        "predicted_layer_instances".to_string(),
        serde_json::Value::from(predicted_layers_total as u64),
    );
    report.metrics.insert(
        "predicted_total_portals".to_string(),
        serde_json::Value::from(portals.len() as u64),
    );

    let mut provenance = ProvenanceSummaryV1::default();
    provenance.pinned_instances = predicted_pinned;
    provenance.instances_by_layer = predicted_by_layer.clone();
    report.provenance_summary = Some(provenance);

    // Referential integrity: prefab ids must exist.
    for pinned in pinned_instances.values() {
        if library.get(pinned.prefab_id.0).is_some() {
            continue;
        }
        report.push_violation(ValidationViolationV1 {
            code: "unknown_prefab_id".to_string(),
            message: format!(
                "Pinned instance references unknown prefab_id {}",
                uuid::Uuid::from_u128(pinned.prefab_id.0)
            ),
            severity: ViolationSeverityV1::Error,
            evidence: Some(ViolationEvidenceV1 {
                source_path: pinned
                    .source_rel_path
                    .as_ref()
                    .map(|p| p.display().to_string()),
                instance_id: Some(uuid::Uuid::from_u128(pinned.instance_id.0).to_string()),
                prefab_id: Some(uuid::Uuid::from_u128(pinned.prefab_id.0).to_string()),
                ..Default::default()
            }),
        });
    }
    for (layer_id, layer) in &layers {
        match layer {
            SceneLayer::ExplicitInstances(layer) => {
                for inst in &layer.instances {
                    if library.get(inst.prefab_id.0).is_some() {
                        continue;
                    }
                    report.push_violation(ValidationViolationV1 {
                        code: "unknown_prefab_id".to_string(),
                        message: format!(
                            "Layer {layer_id} instance {} references unknown prefab_id {}",
                            inst.local_id,
                            uuid::Uuid::from_u128(inst.prefab_id.0)
                        ),
                        severity: ViolationSeverityV1::Error,
                        evidence: Some(ViolationEvidenceV1 {
                            source_path: Some(layer.source_rel_path.display().to_string()),
                            layer_id: Some(layer_id.clone()),
                            local_id: Some(inst.local_id.clone()),
                            prefab_id: Some(uuid::Uuid::from_u128(inst.prefab_id.0).to_string()),
                            ..Default::default()
                        }),
                    });
                }
            }
            SceneLayer::GridInstances(layer) => {
                if library.get(layer.prefab_id.0).is_some() {
                    continue;
                }
                report.push_violation(ValidationViolationV1 {
                    code: "unknown_prefab_id".to_string(),
                    message: format!(
                        "Layer {layer_id} references unknown prefab_id {}",
                        uuid::Uuid::from_u128(layer.prefab_id.0)
                    ),
                    severity: ViolationSeverityV1::Error,
                    evidence: Some(ViolationEvidenceV1 {
                        source_path: Some(layer.source_rel_path.display().to_string()),
                        layer_id: Some(layer_id.clone()),
                        prefab_id: Some(uuid::Uuid::from_u128(layer.prefab_id.0).to_string()),
                        ..Default::default()
                    }),
                });
            }
            SceneLayer::PolylineInstances(layer) => {
                if library.get(layer.prefab_id.0).is_some() {
                    continue;
                }
                report.push_violation(ValidationViolationV1 {
                    code: "unknown_prefab_id".to_string(),
                    message: format!(
                        "Layer {layer_id} references unknown prefab_id {}",
                        uuid::Uuid::from_u128(layer.prefab_id.0)
                    ),
                    severity: ViolationSeverityV1::Error,
                    evidence: Some(ViolationEvidenceV1 {
                        source_path: Some(layer.source_rel_path.display().to_string()),
                        layer_id: Some(layer_id.clone()),
                        prefab_id: Some(uuid::Uuid::from_u128(layer.prefab_id.0).to_string()),
                        ..Default::default()
                    }),
                });
            }
        }
    }

    // Deterministic id derivation conflicts with pinned ids.
    if let Some(scene_id) = scene_id.as_deref() {
        let pinned_ids: HashSet<u128> = pinned_instances.keys().copied().collect();
        for (layer_id, layer) in &layers {
            match layer {
                SceneLayer::ExplicitInstances(layer) => {
                    for inst in &layer.instances {
                        let derived_id =
                            derived_layer_instance_id(scene_id, layer_id, inst.local_id.as_str());
                        if !pinned_ids.contains(&derived_id.0) {
                            continue;
                        }
                        report.push_violation(ValidationViolationV1 {
                            code: "instance_id_conflict_pinned_vs_layer".to_string(),
                            message: format!(
                                "Layer output instance_id conflicts with pinned instance_id {}",
                                uuid::Uuid::from_u128(derived_id.0)
                            ),
                            severity: ViolationSeverityV1::Error,
                            evidence: Some(ViolationEvidenceV1 {
                                source_path: Some(layer.source_rel_path.display().to_string()),
                                layer_id: Some(layer_id.clone()),
                                local_id: Some(inst.local_id.clone()),
                                instance_id: Some(uuid::Uuid::from_u128(derived_id.0).to_string()),
                                ..Default::default()
                            }),
                        });
                    }
                }
                SceneLayer::GridInstances(layer) => {
                    for ix in 0..layer.count_x {
                        for iz in 0..layer.count_z {
                            let local_id = grid_instances_local_id(ix, iz);
                            let derived_id =
                                derived_layer_instance_id(scene_id, layer_id, &local_id);
                            if !pinned_ids.contains(&derived_id.0) {
                                continue;
                            }
                            report.push_violation(ValidationViolationV1 {
                                code: "instance_id_conflict_pinned_vs_layer".to_string(),
                                message: format!(
                                    "Layer output instance_id conflicts with pinned instance_id {}",
                                    uuid::Uuid::from_u128(derived_id.0)
                                ),
                                severity: ViolationSeverityV1::Error,
                                evidence: Some(ViolationEvidenceV1 {
                                    source_path: Some(layer.source_rel_path.display().to_string()),
                                    layer_id: Some(layer_id.clone()),
                                    local_id: Some(local_id),
                                    instance_id: Some(
                                        uuid::Uuid::from_u128(derived_id.0).to_string(),
                                    ),
                                    ..Default::default()
                                }),
                            });
                        }
                    }
                }
                SceneLayer::PolylineInstances(layer) => {
                    let count = polyline_instances_predicted_count(layer)?;
                    for k in 0..count {
                        let local_id = polyline_instances_local_id(k);
                        let derived_id = derived_layer_instance_id(scene_id, layer_id, &local_id);
                        if !pinned_ids.contains(&derived_id.0) {
                            continue;
                        }
                        report.push_violation(ValidationViolationV1 {
                            code: "instance_id_conflict_pinned_vs_layer".to_string(),
                            message: format!(
                                "Layer output instance_id conflicts with pinned instance_id {}",
                                uuid::Uuid::from_u128(derived_id.0)
                            ),
                            severity: ViolationSeverityV1::Error,
                            evidence: Some(ViolationEvidenceV1 {
                                source_path: Some(layer.source_rel_path.display().to_string()),
                                layer_id: Some(layer_id.clone()),
                                local_id: Some(local_id),
                                instance_id: Some(uuid::Uuid::from_u128(derived_id.0).to_string()),
                                ..Default::default()
                            }),
                        });
                    }
                }
            }
        }
    } else {
        report.push_violation(ValidationViolationV1 {
            code: "meta_missing_scene_id".to_string(),
            message: "meta.json missing scene_id; deterministic layer ids cannot be derived"
                .to_string(),
            severity: ViolationSeverityV1::Error,
            evidence: Some(ViolationEvidenceV1 {
                source_path: Some("meta.json".to_string()),
                ..Default::default()
            }),
        });
    }

    // Portal validity: destination scenes and marker refs (if possible).
    let known_scene_ids = match loaded_from_dir {
        Some(src_dir) => discover_known_scene_ids(src_dir)?,
        None => HashSet::new(),
    };
    report.metrics.insert(
        "known_scene_ids_count".to_string(),
        serde_json::Value::from(known_scene_ids.len() as u64),
    );

    for portal in &portals {
        if let Some(from_marker_id) = portal.from_marker_id.as_deref() {
            if !marker_ids.contains(from_marker_id) {
                report.push_violation(ValidationViolationV1 {
                    code: "portal_unknown_from_marker".to_string(),
                    message: format!(
                        "Portal {} references unknown from_marker_id {}",
                        portal.portal_id, from_marker_id
                    ),
                    severity: ViolationSeverityV1::Error,
                    evidence: Some(ViolationEvidenceV1 {
                        source_path: Some(portal.source_rel_path.display().to_string()),
                        portal_id: Some(portal.portal_id.clone()),
                        marker_id: Some(from_marker_id.to_string()),
                        ..Default::default()
                    }),
                });
            }
        }

        // If the workspace is under a realm-style `scenes/` layout, validate the destination.
        if !known_scene_ids.is_empty() && !known_scene_ids.contains(&portal.destination_scene_id) {
            report.push_violation(ValidationViolationV1 {
                code: "unknown_portal_destination_scene".to_string(),
                message: format!(
                    "Portal {} references unknown destination_scene_id {}",
                    portal.portal_id, portal.destination_scene_id
                ),
                severity: ViolationSeverityV1::Error,
                evidence: Some(ViolationEvidenceV1 {
                    source_path: Some(portal.source_rel_path.display().to_string()),
                    portal_id: Some(portal.portal_id.clone()),
                    destination_scene_id: Some(portal.destination_scene_id.clone()),
                    ..Default::default()
                }),
            });
        }
    }

    // Scorecard hard gates.
    for gate in &scorecard.hard_gates {
        match gate {
            HardGateSpecV1::Schema { .. } => {
                // Core schema checks are handled by parsing + the invariants above.
            }
            HardGateSpecV1::Budget {
                max_instances,
                max_portals,
            } => {
                if let Some(max_instances) = *max_instances {
                    if predicted_total_instances > max_instances {
                        report.push_violation(ValidationViolationV1 {
                            code: "budget_max_instances_exceeded".to_string(),
                            message: format!(
                                "Budget exceeded: predicted_total_instances {predicted_total_instances} > max_instances {max_instances}"
                            ),
                            severity: ViolationSeverityV1::Error,
                            evidence: Some(ViolationEvidenceV1 {
                                measured: Some(serde_json::Value::from(
                                    predicted_total_instances as u64,
                                )),
                                limit: Some(serde_json::Value::from(max_instances as u64)),
                                ..Default::default()
                            }),
                        });
                    }
                }
                if let Some(max_portals) = *max_portals {
                    if portals.len() > max_portals {
                        report.push_violation(ValidationViolationV1 {
                            code: "budget_max_portals_exceeded".to_string(),
                            message: format!(
                                "Budget exceeded: predicted_total_portals {} > max_portals {max_portals}",
                                portals.len()
                            ),
                            severity: ViolationSeverityV1::Error,
                            evidence: Some(ViolationEvidenceV1 {
                                measured: Some(serde_json::Value::from(portals.len() as u64)),
                                limit: Some(serde_json::Value::from(max_portals as u64)),
                                ..Default::default()
                            }),
                        });
                    }
                }
            }
            HardGateSpecV1::Portals {
                require_known_destinations,
            } => {
                if require_known_destinations.unwrap_or(false) && known_scene_ids.is_empty() {
                    report.push_violation(ValidationViolationV1 {
                        code: "portal_destination_validation_unavailable".to_string(),
                        message: "Portal destination validation is unavailable: cannot discover known scene ids from workspace path".to_string(),
                        severity: ViolationSeverityV1::Warning,
                        evidence: None,
                    });
                }
            }
            HardGateSpecV1::Determinism { .. } => {
                // In Milestone 04 we surface deterministic signatures as metrics; the quality gate
                // (golden signatures) is introduced in Milestone 06.
            }
        }
    }

    // Optional: include world signature summary for debugging.
    let existing: Vec<SceneWorldInstance> = existing_instances.collect();
    if !existing.is_empty() {
        match scene_signature_summary(existing.into_iter()) {
            Ok(sig) => {
                report.metrics.insert(
                    "world_overall_sig".to_string(),
                    serde_json::Value::from(sig.overall_sig),
                );
                report.metrics.insert(
                    "world_total_instances".to_string(),
                    serde_json::Value::from(sig.total_instances as u64),
                );
            }
            Err(err) => report.push_violation(ValidationViolationV1 {
                code: "world_signature_error".to_string(),
                message: format!("Failed to compute world signature: {err}"),
                severity: ViolationSeverityV1::Error,
                evidence: None,
            }),
        }
    }

    Ok(report)
}

#[derive(Debug, Serialize)]
pub(crate) struct SceneSourcesPatchValidateReport {
    pub(crate) patch_summary: SceneSourcesPatchSummaryV1,
    pub(crate) validation_report: ValidationReportV1,
}

#[derive(Debug, Serialize)]
pub(crate) struct SceneSourcesPatchApplyReport {
    pub(crate) applied: bool,
    pub(crate) patch_summary: SceneSourcesPatchSummaryV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) compile_report: Option<SceneCompileReport>,
    pub(crate) validation_report: ValidationReportV1,
}

pub(crate) fn validate_scene_sources_patch(
    workspace: &SceneSourcesWorkspace,
    library: &ObjectLibrary,
    scorecard: &ScorecardSpecV1,
    patch: &SceneSourcesPatchV1,
) -> Result<SceneSourcesPatchValidateReport, String> {
    let Some(src_dir) = workspace.loaded_from_dir.as_deref() else {
        return Err("No scene sources directory has been imported in this session.".to_string());
    };

    let sources = SceneSourcesV1::load_from_dir(src_dir).map_err(|err| err.to_string())?;
    let mut patched = sources.clone();
    let patch_summary = apply_patch_to_sources(&mut patched, patch)?;
    let validation_report = validate_scene_sources_impl(
        &patched,
        Some(src_dir),
        library,
        std::iter::empty(),
        scorecard,
    )?;

    Ok(SceneSourcesPatchValidateReport {
        patch_summary,
        validation_report,
    })
}

pub(crate) fn apply_scene_sources_patch(
    commands: &mut Commands,
    workspace: &mut SceneSourcesWorkspace,
    library: &ObjectLibrary,
    existing_instances: impl Iterator<Item = SceneWorldInstance>,
    scorecard: &ScorecardSpecV1,
    patch: &SceneSourcesPatchV1,
) -> Result<SceneSourcesPatchApplyReport, String> {
    let Some(src_dir) = workspace.loaded_from_dir.as_deref() else {
        return Err("No scene sources directory has been imported in this session.".to_string());
    };

    let sources = SceneSourcesV1::load_from_dir(src_dir).map_err(|err| err.to_string())?;
    let mut patched = sources.clone();
    let patch_summary = apply_patch_to_sources(&mut patched, patch)?;
    let validation_report = validate_scene_sources_impl(
        &patched,
        Some(src_dir),
        library,
        std::iter::empty(),
        scorecard,
    )?;

    if !validation_report.hard_gates_passed {
        return Ok(SceneSourcesPatchApplyReport {
            applied: false,
            patch_summary,
            compile_report: None,
            validation_report,
        });
    }

    patched
        .write_to_dir(src_dir)
        .map_err(|err| err.to_string())?;
    workspace.sources = Some(patched);

    let compile_report =
        compile_scene_sources_all_layers(commands, workspace, library, existing_instances)?;

    Ok(SceneSourcesPatchApplyReport {
        applied: true,
        patch_summary,
        compile_report: Some(compile_report),
        validation_report,
    })
}

pub(crate) fn compile_scene_sources_all_layers(
    commands: &mut Commands,
    workspace: &SceneSourcesWorkspace,
    library: &ObjectLibrary,
    existing_instances: impl Iterator<Item = SceneWorldInstance>,
) -> Result<SceneCompileReport, String> {
    let Some(sources) = workspace.sources.as_ref() else {
        return Err("No scene sources have been imported in this session.".to_string());
    };

    let scene_id = sources
        .meta_json
        .get("scene_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "meta.json missing scene_id".to_string())?;

    let pinned_instances = parse_pinned_instances(sources)?;
    let layers = parse_layers(sources)?;

    let existing: Vec<SceneWorldInstance> = existing_instances.collect();
    let existing_by_id = map_existing_instances_by_id(&existing)?;

    let pinned_ids: HashSet<u128> = pinned_instances.keys().copied().collect();

    let mut report = SceneCompileReport {
        spawned: 0,
        updated: 0,
        despawned: 0,
        layers_compiled: layers.len(),
        pinned_upserts: pinned_instances.len(),
    };

    // First apply pinned instances (unowned).
    for (id_u128, pinned) in &pinned_instances {
        let existing = existing_by_id.get(id_u128);
        if let Some(existing) = existing {
            if existing.owner_layer_id.is_some() {
                return Err(format!(
                    "Pinned instance {} conflicts with a layer-owned entity",
                    uuid::Uuid::from_u128(*id_u128)
                ));
            }
            upsert_scene_instance_entity(
                commands,
                library,
                existing.entity,
                pinned.prefab_id.0,
                pinned.transform,
                pinned.tint,
                None,
            );
            report.updated += 1;
        } else {
            spawn_scene_instance_entity(
                commands,
                library,
                pinned.instance_id,
                pinned.prefab_id.0,
                pinned.transform,
                pinned.tint,
                None,
            );
            report.spawned += 1;
        }
    }

    // Then compile each layer deterministically.
    for (layer_id, layer) in &layers {
        let desired = desired_instances_for_layer(scene_id, layer_id, layer)?;
        let desired_ids: HashSet<u128> = desired.keys().copied().collect();

        // Upsert desired outputs.
        for (id_u128, spec) in &desired {
            if pinned_ids.contains(id_u128) {
                return Err(format!(
                    "Layer output id conflicts with pinned instance id: {}",
                    uuid::Uuid::from_u128(*id_u128)
                ));
            }

            if let Some(existing) = existing_by_id.get(id_u128) {
                match existing.owner_layer_id.as_deref() {
                    Some(owner) if owner == layer_id => {
                        upsert_scene_instance_entity(
                            commands,
                            library,
                            existing.entity,
                            spec.prefab_id.0,
                            spec.transform,
                            spec.tint,
                            Some(layer_id.as_str()),
                        );
                        report.updated += 1;
                    }
                    Some(owner) => {
                        return Err(format!(
                            "Layer output id {} conflicts with entity owned by different layer {owner}",
                            uuid::Uuid::from_u128(*id_u128)
                        ));
                    }
                    None => {
                        return Err(format!(
                            "Layer output id {} conflicts with unowned entity",
                            uuid::Uuid::from_u128(*id_u128)
                        ));
                    }
                }
            } else {
                spawn_scene_instance_entity(
                    commands,
                    library,
                    spec.instance_id,
                    spec.prefab_id.0,
                    spec.transform,
                    spec.tint,
                    Some(layer_id.as_str()),
                );
                report.spawned += 1;
            }
        }

        // Despawn stale outputs owned by this layer.
        for existing in &existing {
            if existing.owner_layer_id.as_deref() != Some(layer_id.as_str()) {
                continue;
            }
            if desired_ids.contains(&existing.instance_id.0) {
                continue;
            }
            commands.entity(existing.entity).try_despawn();
            report.despawned += 1;
        }
    }

    // Remove entities owned by removed layers.
    let layer_id_set: HashSet<&str> = layers.keys().map(|s| s.as_str()).collect();
    for existing in &existing {
        let Some(owner) = existing.owner_layer_id.as_deref() else {
            continue;
        };
        if layer_id_set.contains(owner) {
            continue;
        }
        commands.entity(existing.entity).try_despawn();
        report.despawned += 1;
    }

    // Remove extraneous unowned entities not present in pinned sources.
    for existing in &existing {
        if existing.owner_layer_id.is_some() {
            continue;
        }
        if pinned_ids.contains(&existing.instance_id.0) {
            continue;
        }
        commands.entity(existing.entity).try_despawn();
        report.despawned += 1;
    }

    Ok(report)
}

pub(crate) fn regenerate_scene_layer(
    commands: &mut Commands,
    workspace: &SceneSourcesWorkspace,
    library: &ObjectLibrary,
    existing_instances: impl Iterator<Item = SceneWorldInstance>,
    layer_id: &str,
) -> Result<SceneCompileReport, String> {
    let Some(sources) = workspace.sources.as_ref() else {
        return Err("No scene sources have been imported in this session.".to_string());
    };
    let scene_id = sources
        .meta_json
        .get("scene_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "meta.json missing scene_id".to_string())?;

    let layers = parse_layers(sources)?;
    let layer_id = layer_id.trim();
    if layer_id.is_empty() {
        return Err("layer_id must be a non-empty string".to_string());
    }
    let Some(layer) = layers.get(layer_id) else {
        return Err(format!("Layer not found: {layer_id}"));
    };

    let desired = desired_instances_for_layer(scene_id, layer_id, layer)?;
    let desired_ids: HashSet<u128> = desired.keys().copied().collect();

    let existing: Vec<SceneWorldInstance> = existing_instances.collect();
    let existing_by_id = map_existing_instances_by_id(&existing)?;

    let pinned_instances = parse_pinned_instances(sources)?;
    let pinned_ids: HashSet<u128> = pinned_instances.keys().copied().collect();

    let mut report = SceneCompileReport {
        spawned: 0,
        updated: 0,
        despawned: 0,
        layers_compiled: 1,
        pinned_upserts: 0,
    };

    for (id_u128, spec) in &desired {
        if pinned_ids.contains(id_u128) {
            return Err(format!(
                "Layer output id conflicts with pinned instance id: {}",
                uuid::Uuid::from_u128(*id_u128)
            ));
        }

        if let Some(existing) = existing_by_id.get(id_u128) {
            match existing.owner_layer_id.as_deref() {
                Some(owner) if owner == layer_id => {
                    upsert_scene_instance_entity(
                        commands,
                        library,
                        existing.entity,
                        spec.prefab_id.0,
                        spec.transform,
                        spec.tint,
                        Some(layer_id),
                    );
                    report.updated += 1;
                }
                Some(owner) => {
                    return Err(format!(
                        "Layer output id {} conflicts with entity owned by different layer {owner}",
                        uuid::Uuid::from_u128(*id_u128)
                    ));
                }
                None => {
                    return Err(format!(
                        "Layer output id {} conflicts with unowned entity",
                        uuid::Uuid::from_u128(*id_u128)
                    ));
                }
            }
        } else {
            spawn_scene_instance_entity(
                commands,
                library,
                spec.instance_id,
                spec.prefab_id.0,
                spec.transform,
                spec.tint,
                Some(layer_id),
            );
            report.spawned += 1;
        }
    }

    // Despawn stale outputs owned by this layer.
    for existing in &existing {
        if existing.owner_layer_id.as_deref() != Some(layer_id) {
            continue;
        }
        if desired_ids.contains(&existing.instance_id.0) {
            continue;
        }
        commands.entity(existing.entity).try_despawn();
        report.despawned += 1;
    }

    Ok(report)
}

fn desired_instances_for_layer(
    scene_id: &str,
    layer_id: &str,
    layer: &SceneLayer,
) -> Result<BTreeMap<u128, PinnedInstance>, String> {
    match layer {
        SceneLayer::ExplicitInstances(layer) => {
            let mut out = BTreeMap::new();
            for inst in &layer.instances {
                let instance_id = derived_layer_instance_id(scene_id, layer_id, &inst.local_id);
                let key = instance_id.0;
                if out
                    .insert(
                        key,
                        PinnedInstance {
                            instance_id,
                            prefab_id: inst.prefab_id,
                            transform: inst.transform,
                            tint: inst.tint,
                            source_rel_path: None,
                        },
                    )
                    .is_some()
                {
                    return Err(format!(
                        "Layer {layer_id} generated duplicate instance id: {}",
                        uuid::Uuid::from_u128(key)
                    ));
                }
            }
            Ok(out)
        }
        SceneLayer::GridInstances(layer) => {
            let mut out = BTreeMap::new();
            for ix in 0..layer.count_x {
                for iz in 0..layer.count_z {
                    let local_id = grid_instances_local_id(ix, iz);
                    let instance_id = derived_layer_instance_id(scene_id, layer_id, &local_id);

                    let translation = Vec3::new(
                        layer.origin.x + layer.step_x * (ix as f32),
                        layer.origin.y,
                        layer.origin.z + layer.step_z * (iz as f32),
                    );
                    if !translation.is_finite() {
                        return Err(format!(
                            "Layer {layer_id} generated non-finite translation for {local_id}"
                        ));
                    }

                    let transform = Transform::from_translation(translation)
                        .with_rotation(layer.rotation)
                        .with_scale(layer.scale);

                    let key = instance_id.0;
                    if out
                        .insert(
                            key,
                            PinnedInstance {
                                instance_id,
                                prefab_id: layer.prefab_id,
                                transform,
                                tint: layer.tint,
                                source_rel_path: None,
                            },
                        )
                        .is_some()
                    {
                        return Err(format!(
                            "Layer {layer_id} generated duplicate instance id: {}",
                            uuid::Uuid::from_u128(key)
                        ));
                    }
                }
            }
            Ok(out)
        }
        SceneLayer::PolylineInstances(layer) => {
            let mut out = BTreeMap::new();
            let count = polyline_instances_predicted_count(layer)?;
            if count == 0 {
                return Ok(out);
            }

            let mut segments = Vec::with_capacity(layer.points.len().saturating_sub(1));
            let mut cum_end = 0.0f32;
            for idx in 0..(layer.points.len() - 1) {
                let a = layer.points[idx];
                let b = layer.points[idx + 1];
                let delta = b - a;
                let len = delta.length();
                if !len.is_finite() || len <= 0.0 {
                    return Err(format!(
                        "Layer {layer_id} contains a non-finite or zero-length segment at points[{idx}]"
                    ));
                }
                cum_end += len;
                if !cum_end.is_finite() {
                    return Err(format!(
                        "Layer {layer_id} polyline length overflow (non-finite total length)"
                    ));
                }
                segments.push((a, delta, len, cum_end));
            }

            let total_length = cum_end;
            let mut seg_idx: usize = 0;
            let mut seg_start_dist: f32 = 0.0;
            let mut seg_end_dist: f32 = segments.first().map(|s| s.3).unwrap_or(0.0);

            for k in 0..count {
                let local_id = polyline_instances_local_id(k);
                let instance_id = derived_layer_instance_id(scene_id, layer_id, &local_id);

                let mut d = layer.start_offset + (k as f32) * layer.spacing;
                if !d.is_finite() {
                    return Err(format!(
                        "Layer {layer_id} generated non-finite distance for {local_id}"
                    ));
                }
                if d > total_length {
                    d = total_length;
                }

                while seg_idx + 1 < segments.len() && d > seg_end_dist {
                    seg_start_dist = seg_end_dist;
                    seg_idx += 1;
                    seg_end_dist = segments[seg_idx].3;
                }

                let (seg_a, seg_delta, seg_len, _seg_cum_end) = segments[seg_idx];
                let local_d = d - seg_start_dist;
                let t = local_d / seg_len;
                let translation = seg_a + seg_delta * t;
                if !translation.is_finite() {
                    return Err(format!(
                        "Layer {layer_id} generated non-finite translation for {local_id}"
                    ));
                }

                let transform = Transform::from_translation(translation)
                    .with_rotation(layer.rotation)
                    .with_scale(layer.scale);

                let key = instance_id.0;
                if out
                    .insert(
                        key,
                        PinnedInstance {
                            instance_id,
                            prefab_id: layer.prefab_id,
                            transform,
                            tint: layer.tint,
                            source_rel_path: None,
                        },
                    )
                    .is_some()
                {
                    return Err(format!(
                        "Layer {layer_id} generated duplicate instance id: {}",
                        uuid::Uuid::from_u128(key)
                    ));
                }
            }

            Ok(out)
        }
    }
}

fn grid_instances_local_id(ix: u32, iz: u32) -> String {
    format!("x{ix}_z{iz}")
}

fn grid_instances_predicted_count(layer: &GridInstancesLayer) -> Result<usize, String> {
    let count = (layer.count_x as u64)
        .checked_mul(layer.count_z as u64)
        .ok_or_else(|| "grid_instances predicted count overflow".to_string())?;
    usize::try_from(count).map_err(|_| "grid_instances predicted count too large".to_string())
}

fn polyline_instances_local_id(idx: usize) -> String {
    format!("i{idx}")
}

fn polyline_instances_predicted_count(layer: &PolylineInstancesLayer) -> Result<usize, String> {
    if layer.points.len() < 2 {
        return Ok(0);
    }

    let mut total_length = 0.0f64;
    for idx in 0..(layer.points.len() - 1) {
        let a = layer.points[idx];
        let b = layer.points[idx + 1];
        let len = (b - a).length() as f64;
        if !len.is_finite() {
            return Err("polyline_instances contains a non-finite segment length".to_string());
        }
        total_length += len;
        if !total_length.is_finite() {
            return Err("polyline_instances total_length overflow (non-finite)".to_string());
        }
    }

    let start_offset = layer.start_offset as f64;
    if start_offset > total_length {
        return Ok(0);
    }

    let remaining = total_length - start_offset;
    if !remaining.is_finite() {
        return Err("polyline_instances remaining length must be finite".to_string());
    }

    let spacing = layer.spacing as f64;
    if !spacing.is_finite() || spacing <= 0.0 {
        return Err("polyline_instances spacing must be finite and > 0".to_string());
    }

    let ratio = remaining / spacing;
    if !ratio.is_finite() || ratio < 0.0 {
        return Err("polyline_instances count computation overflow".to_string());
    }

    let count_floor = ratio.floor();
    let max_floor = (usize::MAX as f64) - 1.0;
    if count_floor > max_floor {
        return Err("polyline_instances predicted count too large".to_string());
    }

    Ok((count_floor as usize).saturating_add(1))
}

fn map_existing_instances_by_id(
    existing: &[SceneWorldInstance],
) -> Result<HashMap<u128, SceneWorldInstance>, String> {
    let mut map = HashMap::with_capacity(existing.len());
    for inst in existing.iter().cloned() {
        if map.insert(inst.instance_id.0, inst).is_some() {
            return Err(
                "World contains duplicate ObjectId components for scene instances".to_string(),
            );
        }
    }
    Ok(map)
}

fn spawn_scene_instance_entity(
    commands: &mut Commands,
    library: &ObjectLibrary,
    instance_id: ObjectId,
    prefab_id: u128,
    transform: Transform,
    tint: Option<Color>,
    owner_layer_id: Option<&str>,
) -> Entity {
    let entity = if library.mobility(prefab_id).is_some() {
        spawn_unit_minimal(commands, library, prefab_id, transform, instance_id, tint)
    } else {
        spawn_build_object_minimal(commands, library, prefab_id, transform, instance_id, tint)
    };
    if let Some(owner_layer_id) = owner_layer_id {
        commands.entity(entity).insert(SceneLayerOwner {
            layer_id: owner_layer_id.to_string(),
        });
    }
    entity
}

fn upsert_scene_instance_entity(
    commands: &mut Commands,
    library: &ObjectLibrary,
    entity: Entity,
    prefab_id: u128,
    transform: Transform,
    tint: Option<Color>,
    owner_layer_id: Option<&str>,
) {
    if library.mobility(prefab_id).is_some() {
        let (radius, transform) = compute_unit_radius_and_transform(library, prefab_id, transform);
        let mut ec = commands.entity(entity);
        ec.insert(ObjectPrefabId(prefab_id));
        ec.insert(Commandable);
        ec.insert(Collider { radius });
        ec.insert(transform);
        ec.remove::<BuildObject>();
        ec.remove::<BuildDimensions>();
        ec.remove::<AabbCollider>();
    } else {
        let (collider_half_xz, size, transform) =
            compute_build_object_collider_and_transform(library, prefab_id, transform);
        let mut ec = commands.entity(entity);
        ec.insert(ObjectPrefabId(prefab_id));
        ec.insert(BuildObject);
        ec.insert(BuildDimensions { size });
        ec.insert(AabbCollider {
            half_extents: collider_half_xz,
        });
        ec.insert(transform);
        ec.remove::<Commandable>();
        ec.remove::<Collider>();
    }

    let mut ec = commands.entity(entity);
    if let Some(tint) = tint {
        ec.insert(ObjectTint(tint));
    } else {
        ec.remove::<ObjectTint>();
    }

    if let Some(owner_layer_id) = owner_layer_id {
        ec.insert(SceneLayerOwner {
            layer_id: owner_layer_id.to_string(),
        });
    } else {
        ec.remove::<SceneLayerOwner>();
    }
}

fn compute_unit_radius_and_transform(
    library: &ObjectLibrary,
    prefab_id: u128,
    transform: Transform,
) -> (f32, Transform) {
    let mut transform = transform;
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));
    transform.scale = sanitize_scale(transform.scale);

    let scale = transform.scale;
    let radius = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => {
            radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01)
        }
        Some(ColliderProfile::AabbXZ { half_extents }) => {
            let half = Vec2::new(
                half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
                half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
            );
            half.x.max(half.y)
        }
        _ => {
            let size = Vec2::new(
                (base_size.x * scale.x.abs()).abs().max(0.01),
                (base_size.z * scale.z.abs()).abs().max(0.01),
            );
            (size.x.max(size.y) * 0.5).max(0.01)
        }
    };
    (radius, transform)
}

fn compute_build_object_collider_and_transform(
    library: &ObjectLibrary,
    prefab_id: u128,
    transform: Transform,
) -> (Vec2, Vec3, Transform) {
    let mut transform = transform;
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));

    transform.scale = sanitize_scale(transform.scale);

    let (yaw, _pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);
    let c = yaw.cos().abs();
    let s = yaw.sin().abs();

    let scale = transform.scale;
    let (collider_half_xz, size) = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => {
            let r = radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01);
            (
                Vec2::splat(r),
                Vec3::new(r * 2.0, base_size.y * scale.y.abs(), r * 2.0),
            )
        }
        Some(ColliderProfile::AabbXZ { half_extents }) => {
            let half = Vec2::new(
                half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
                half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
            );
            let rotated = Vec2::new(c * half.x + s * half.y, s * half.x + c * half.y);
            (
                rotated,
                Vec3::new(
                    rotated.x * 2.0,
                    base_size.y * scale.y.abs(),
                    rotated.y * 2.0,
                ),
            )
        }
        _ => {
            let half = Vec2::new(
                (base_size.x * 0.5).abs().max(0.01) * scale.x.abs().max(0.01),
                (base_size.z * 0.5).abs().max(0.01) * scale.z.abs().max(0.01),
            );
            let rotated = Vec2::new(c * half.x + s * half.y, s * half.x + c * half.y);
            (
                rotated,
                Vec3::new(
                    rotated.x * 2.0,
                    base_size.y * scale.y.abs(),
                    rotated.y * 2.0,
                ),
            )
        }
    };

    (collider_half_xz, size, transform)
}

pub(crate) fn scene_signature_summary(
    existing_instances: impl Iterator<Item = SceneWorldInstance>,
) -> Result<SceneSignatureSummary, String> {
    let instances: Vec<SceneWorldInstance> = existing_instances.collect();
    let total_instances = instances.len();

    let mut pinned: Vec<&SceneWorldInstance> = Vec::new();
    let mut by_layer: BTreeMap<String, Vec<&SceneWorldInstance>> = BTreeMap::new();
    for inst in &instances {
        if let Some(layer_id) = inst.owner_layer_id.as_deref() {
            by_layer.entry(layer_id.to_string()).or_default().push(inst);
        } else {
            pinned.push(inst);
        }
    }

    let pinned_sig = signature_for_instances(&pinned)?;
    let mut layer_sigs = BTreeMap::new();
    let mut layer_instance_counts = BTreeMap::new();
    for (layer_id, items) in &by_layer {
        layer_instance_counts.insert(layer_id.clone(), items.len());
        layer_sigs.insert(layer_id.clone(), signature_for_instances(items)?);
    }

    let all_refs: Vec<&SceneWorldInstance> = instances.iter().collect();
    let overall_sig = signature_for_instances(&all_refs)?;

    Ok(SceneSignatureSummary {
        overall_sig,
        pinned_sig,
        layer_sigs,
        total_instances,
        pinned_instances: pinned.len(),
        layer_instance_counts,
    })
}

pub(crate) fn scene_signature_summary_from_sources(
    sources: &SceneSourcesV1,
) -> Result<SceneSignatureSummary, String> {
    let scene_id = sources
        .meta_json
        .get("scene_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "meta.json missing scene_id".to_string())?;

    let pinned_instances = parse_pinned_instances(sources)?;
    let layers = parse_layers(sources)?;

    let pinned_ids: HashSet<u128> = pinned_instances.keys().copied().collect();
    let mut seen_ids: HashSet<u128> = HashSet::new();

    let mut instances: Vec<SceneWorldInstance> = Vec::new();
    let mut next_entity_raw: u32 = 1;
    for pinned in pinned_instances.values() {
        if !seen_ids.insert(pinned.instance_id.0) {
            return Err("sources contain duplicate instance_id across outputs".to_string());
        }
        instances.push(SceneWorldInstance {
            entity: Entity::from_bits(next_entity_raw as u64),
            instance_id: pinned.instance_id,
            prefab_id: pinned.prefab_id,
            transform: pinned.transform,
            tint: pinned.tint,
            owner_layer_id: None,
        });
        next_entity_raw = next_entity_raw.saturating_add(1);
    }

    for (layer_id, layer) in &layers {
        let desired = desired_instances_for_layer(scene_id, layer_id, layer)?;
        for spec in desired.values() {
            if pinned_ids.contains(&spec.instance_id.0) {
                return Err(format!(
                    "Layer output id conflicts with pinned instance id: {}",
                    uuid::Uuid::from_u128(spec.instance_id.0)
                ));
            }
            if !seen_ids.insert(spec.instance_id.0) {
                return Err("sources contain duplicate instance_id across outputs".to_string());
            }
            instances.push(SceneWorldInstance {
                entity: Entity::from_bits(next_entity_raw as u64),
                instance_id: spec.instance_id,
                prefab_id: spec.prefab_id,
                transform: spec.transform,
                tint: spec.tint,
                owner_layer_id: Some(layer_id.clone()),
            });
            next_entity_raw = next_entity_raw.saturating_add(1);
        }
    }

    scene_signature_summary(instances.into_iter())
}

fn signature_for_instances(instances: &[&SceneWorldInstance]) -> Result<String, String> {
    let mut items = instances.to_vec();
    items.sort_by(|a, b| a.instance_id.0.cmp(&b.instance_id.0));

    let mut hasher = Sha256::new();
    for inst in items {
        hasher.update(inst.instance_id.0.to_be_bytes());
        hasher.update(inst.prefab_id.0.to_be_bytes());
        update_hasher_with_vec3(&mut hasher, inst.transform.translation)?;
        update_hasher_with_quat(&mut hasher, inst.transform.rotation)?;
        update_hasher_with_vec3(&mut hasher, inst.transform.scale)?;

        if let Some(tint) = inst.tint {
            hasher.update([1u8]);
            let linear = tint.to_linear();
            update_hasher_with_f32(&mut hasher, linear.red)?;
            update_hasher_with_f32(&mut hasher, linear.green)?;
            update_hasher_with_f32(&mut hasher, linear.blue)?;
            update_hasher_with_f32(&mut hasher, linear.alpha)?;
        } else {
            hasher.update([0u8]);
        }
    }

    Ok(hex_string(&hasher.finalize()))
}

fn update_hasher_with_vec3(hasher: &mut Sha256, v: Vec3) -> Result<(), String> {
    update_hasher_with_f32(hasher, v.x)?;
    update_hasher_with_f32(hasher, v.y)?;
    update_hasher_with_f32(hasher, v.z)?;
    Ok(())
}

fn update_hasher_with_quat(hasher: &mut Sha256, q: Quat) -> Result<(), String> {
    update_hasher_with_f32(hasher, q.x)?;
    update_hasher_with_f32(hasher, q.y)?;
    update_hasher_with_f32(hasher, q.z)?;
    update_hasher_with_f32(hasher, q.w)?;
    Ok(())
}

fn update_hasher_with_f32(hasher: &mut Sha256, v: f32) -> Result<(), String> {
    if !v.is_finite() {
        return Err("signature contains non-finite float".to_string());
    }
    hasher.update(v.to_bits().to_be_bytes());
    Ok(())
}

fn hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod golden_scene_signatures {
    use super::*;
    use std::path::PathBuf;

    const GOLDEN_FORMAT_VERSION: u32 = 1;

    fn golden_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/scene_generation/golden_signatures.json")
    }

    fn fixture_src_dir(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/scene_generation/fixtures")
            .join(name)
            .join("src")
    }

    fn compute_golden() -> serde_json::Value {
        let fixtures = [
            ("minimal", fixture_src_dir("minimal")),
            ("layers_regen", fixture_src_dir("layers_regen")),
            (
                "procedural_layers_v1",
                fixture_src_dir("procedural_layers_v1"),
            ),
        ];

        let mut out = serde_json::Map::new();
        for (name, src_dir) in fixtures {
            let sources = SceneSourcesV1::load_from_dir(&src_dir)
                .unwrap_or_else(|err| panic!("load fixture {name} failed: {err}"));
            let sig = scene_signature_summary_from_sources(&sources)
                .unwrap_or_else(|err| panic!("signature for fixture {name} failed: {err}"));
            let sig_val = serde_json::to_value(sig).expect("sig to_value");
            out.insert(name.to_string(), sig_val);
        }

        serde_json::json!({
            "format_version": GOLDEN_FORMAT_VERSION,
            "fixtures": out,
        })
    }

    fn write_json_atomic(path: &Path, value: &serde_json::Value) {
        let Some(parent) = path.parent() else {
            panic!("no parent for {}", path.display());
        };
        std::fs::create_dir_all(parent).expect("create golden dir");
        let text = serde_json::to_string_pretty(value).expect("serialize golden json");
        let bytes = format!("{text}\n").into_bytes();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).expect("write tmp");
        std::fs::rename(&tmp, path).expect("rename golden");
    }

    #[test]
    fn golden_signatures_match_or_bless() {
        let computed = compute_golden();
        let path = golden_path();

        if std::env::var("GRAVIMERA_BLESS_SCENE_SIGNATURES")
            .ok()
            .is_some_and(|v| v == "1")
        {
            write_json_atomic(&path, &computed);
            eprintln!("Blessed scene signatures at {}", path.display());
            return;
        }

        let bytes = std::fs::read(&path).unwrap_or_else(|err| {
            panic!("read golden signatures failed ({}): {err}", path.display())
        });
        let expected: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            panic!("parse golden signatures failed ({}): {err}", path.display())
        });

        assert_eq!(expected, computed, "Golden scene signatures changed. If this is intended, re-run with GRAVIMERA_BLESS_SCENE_SIGNATURES=1.");
    }
}
