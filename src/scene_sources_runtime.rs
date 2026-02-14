use bevy::prelude::*;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::constants::BUILD_UNIT_SIZE;
use crate::object::registry::{ColliderProfile, ObjectLibrary};
use crate::scene_sources::{
    SceneSourcesIndexPaths, SceneSourcesV1, SCENE_SOURCES_FORMAT_VERSION,
};
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, ObjectId, ObjectPrefabId,
    ObjectTint,
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

    let index_paths =
        SceneSourcesIndexPaths::from_index_json_value(&sources.index_json).map_err(|err| {
            format!("Invalid scene sources index.json: {err}")
        })?;
    let pinned_dir = index_paths.pinned_instances_dir;

    let existing_docs_by_instance_id = map_existing_pinned_docs_by_instance_id(&sources, &pinned_dir);

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
        let rel_path = pinned_dir.join(format!(
            "{}.json",
            uuid::Uuid::from_u128(instance_id.0)
        ));
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
}

fn parse_pinned_instances(sources: &SceneSourcesV1) -> Result<BTreeMap<u128, PinnedInstance>, String> {
    let index_paths =
        SceneSourcesIndexPaths::from_index_json_value(&sources.index_json).map_err(|err| {
            format!("Invalid scene sources index.json: {err}")
        })?;
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
        .unwrap_or_else(|| Vec3::splat(BUILD_UNIT_SIZE));

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
        .unwrap_or_else(|| Vec3::splat(BUILD_UNIT_SIZE));

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
    let mut doc = existing.cloned().unwrap_or_else(|| Value::Object(Default::default()));
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
