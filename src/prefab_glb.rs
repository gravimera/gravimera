use bevy::prelude::*;
use gltf_json as json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::object::registry::{
    AttachmentDef, MeshKey, ObjectDef, ObjectLibrary, ObjectPartDef, ObjectPartKind,
    PartAnimationDef, PartAnimationDriver, PartAnimationSlot, PartAnimationSpec,
    PartAnimationSpinAxisSpace, PrimitiveParams, PrimitiveVisualDef,
};

const DEFAULT_EXPORT_FPS: u32 = 30;
const DEFAULT_LOOP_DURATION_SECS: f32 = 2.0;
const DEFAULT_ACTION_DURATION_SECS: f32 = 1.0;
const MAX_ANIM_DURATION_SECS: f32 = 10.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PrefabGltfGlbExportOptions {
    pub(crate) fps: u32,
    pub(crate) move_units_per_sec: f32,
}

impl Default for PrefabGltfGlbExportOptions {
    fn default() -> Self {
        Self {
            fps: DEFAULT_EXPORT_FPS,
            move_units_per_sec: 1.0,
        }
    }
}

pub(crate) struct PrefabGltfGlbExportReport {
    pub(crate) exported: usize,
    pub(crate) out_paths: Vec<PathBuf>,
}

fn sanitize_label_for_filename(label: &str) -> String {
    let mut out = String::new();
    let mut prev_sep = false;

    for ch in label.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            Some(ch)
        } else if ch.is_whitespace() || ch == '.' || ch == '/' || ch == '\\' {
            Some('_')
        } else {
            Some('_')
        };

        let Some(mapped) = mapped else { continue };
        if mapped == '_' {
            if prev_sep {
                continue;
            }
            prev_sep = true;
            out.push('_');
        } else {
            prev_sep = false;
            out.push(mapped);
        }
    }

    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        return "Prefab".to_string();
    }
    trimmed.chars().take(48).collect()
}

pub(crate) fn export_prefabs_to_gltf_glb_dir(
    prefab_ids: &[u128],
    out_dir: &Path,
    library: &ObjectLibrary,
    options: PrefabGltfGlbExportOptions,
) -> Result<PrefabGltfGlbExportReport, String> {
    if prefab_ids.is_empty() {
        return Err("No prefab ids provided.".to_string());
    }

    std::fs::create_dir_all(out_dir)
        .map_err(|err| format!("Failed to create {}: {err}", out_dir.display()))?;

    let mut exported = 0usize;
    let mut out_paths: Vec<PathBuf> = Vec::new();
    for prefab_id in prefab_ids {
        let uuid = uuid::Uuid::from_u128(*prefab_id).to_string();
        let label = library
            .get(*prefab_id)
            .map(|def| def.label.as_ref().to_string())
            .ok_or_else(|| format!("Missing prefab def {uuid} (not loaded in ObjectLibrary)."))?;
        let label = sanitize_label_for_filename(&label);
        let base = format!("{label}_{uuid}");
        let glb_path = out_dir.join(format!("{base}.glb"));
        let gltf_path = out_dir.join(format!("{base}.gltf"));
        let bin_path = out_dir.join(format!("{base}.bin"));
        export_prefab_to_gltf_glb_paths(
            *prefab_id, &glb_path, &gltf_path, &bin_path, library, options,
        )?;
        exported += 1;
        out_paths.push(glb_path);
        out_paths.push(gltf_path);
        out_paths.push(bin_path);
    }

    Ok(PrefabGltfGlbExportReport {
        exported,
        out_paths,
    })
}

#[derive(Clone, Copy, Debug)]
struct ExportRootState<'a> {
    forced_channel: Option<&'a str>,
    attack_active: bool,
    action_active: bool,
    move_active: bool,
}

impl ExportRootState<'_> {
    fn idle_active(&self) -> bool {
        !self.attack_active && !self.action_active && !self.move_active
    }
}

fn root_state_for_channel(channel: &str) -> ExportRootState<'_> {
    let channel = channel.trim();
    match channel {
        "attack" => ExportRootState {
            forced_channel: None,
            attack_active: true,
            action_active: false,
            move_active: false,
        },
        "action" => ExportRootState {
            forced_channel: None,
            attack_active: false,
            action_active: true,
            move_active: false,
        },
        "move" => ExportRootState {
            forced_channel: None,
            attack_active: false,
            action_active: false,
            move_active: true,
        },
        "idle" => ExportRootState {
            forced_channel: None,
            attack_active: false,
            action_active: false,
            move_active: false,
        },
        "ambient" => ExportRootState {
            forced_channel: Some("ambient"),
            attack_active: false,
            action_active: false,
            move_active: false,
        },
        other => ExportRootState {
            forced_channel: Some(other),
            attack_active: false,
            action_active: false,
            move_active: false,
        },
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct PrimitiveParamsKey {
    a_milli: i32,
    b_milli: i32,
    c_milli: i32,
    kind: u8,
}

fn quantize_milli(v: f32) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    (v * 1000.0).round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

impl PrimitiveParamsKey {
    fn from_params(params: &PrimitiveParams) -> Self {
        match *params {
            PrimitiveParams::Capsule {
                radius,
                half_length,
            } => Self {
                kind: 1,
                a_milli: quantize_milli(radius),
                b_milli: quantize_milli(half_length),
                c_milli: 0,
            },
            PrimitiveParams::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            } => Self {
                kind: 2,
                a_milli: quantize_milli(radius_top),
                b_milli: quantize_milli(radius_bottom),
                c_milli: quantize_milli(height),
            },
            PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            } => Self {
                kind: 3,
                a_milli: quantize_milli(minor_radius),
                b_milli: quantize_milli(major_radius),
                c_milli: 0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct MeshGeometryKey {
    mesh: MeshKey,
    params: Option<PrimitiveParamsKey>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct MeshInstanceKey {
    geometry: MeshGeometryKey,
    material: MaterialKeyHash,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum AlphaModeKey {
    Opaque,
    Blend,
}

#[derive(Clone, Copy, Debug)]
struct MaterialSpec {
    base_color_linear: [f32; 4],
    metallic: f32,
    roughness: f32,
    emissive_linear: [f32; 3],
    unlit: bool,
    alpha_mode: AlphaModeKey,
    alpha_cutoff: Option<f32>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct MaterialKeyHash {
    base_r: u32,
    base_g: u32,
    base_b: u32,
    base_a: u32,
    emissive_r: u32,
    emissive_g: u32,
    emissive_b: u32,
    metallic: u32,
    roughness: u32,
    unlit: bool,
    alpha_mode: u8,
    alpha_cutoff: u32,
}

impl MaterialKeyHash {
    fn from_spec(spec: &MaterialSpec) -> Self {
        fn bits(v: f32) -> u32 {
            if v.is_finite() {
                v.to_bits()
            } else {
                0.0f32.to_bits()
            }
        }

        let alpha_mode = match spec.alpha_mode {
            AlphaModeKey::Opaque => 0,
            AlphaModeKey::Blend => 2,
        };
        let alpha_cutoff = bits(spec.alpha_cutoff.unwrap_or(0.0));

        Self {
            base_r: bits(spec.base_color_linear[0]),
            base_g: bits(spec.base_color_linear[1]),
            base_b: bits(spec.base_color_linear[2]),
            base_a: bits(spec.base_color_linear[3]),
            emissive_r: bits(spec.emissive_linear[0]),
            emissive_g: bits(spec.emissive_linear[1]),
            emissive_b: bits(spec.emissive_linear[2]),
            metallic: bits(spec.metallic),
            roughness: bits(spec.roughness),
            unlit: spec.unlit,
            alpha_mode,
            alpha_cutoff,
        }
    }
}

#[derive(Clone, Debug)]
struct ExportNodeAnim {
    parent_object_id: u128,
    child_object_id: Option<u128>,
    attachment: Option<AttachmentDef>,
    base_transform: Transform,
    fallback_basis: Transform,
    slots: Vec<PartAnimationSlot>,
}

#[derive(Clone, Debug)]
struct ExportNodeMeta {
    gltf_node: usize,
    anim: Option<ExportNodeAnim>,
}

fn export_prefab_to_gltf_glb_paths(
    prefab_id: u128,
    glb_path: &Path,
    gltf_path: &Path,
    bin_path: &Path,
    library: &ObjectLibrary,
    options: PrefabGltfGlbExportOptions,
) -> Result<(), String> {
    let Some(root_def) = library.get(prefab_id) else {
        return Err(format!(
            "Missing prefab def {} (not loaded in ObjectLibrary).",
            uuid::Uuid::from_u128(prefab_id)
        ));
    };

    let fps = options.fps.max(1).min(240);
    let move_units_per_sec = if options.move_units_per_sec.is_finite() {
        options.move_units_per_sec.max(0.0)
    } else {
        1.0
    };
    let options = PrefabGltfGlbExportOptions {
        fps,
        move_units_per_sec,
    };

    let mut builder = GltfGlbBuilder::new();

    // Root node represents the prefab.
    let root_node = builder.push_node(json::Node {
        name: Some(root_def.label.to_string()),
        ..Default::default()
    });

    let mut nodes: Vec<ExportNodeMeta> = Vec::new();
    let mut stack: Vec<u128> = Vec::new();
    build_object_nodes(
        library,
        prefab_id,
        root_node,
        &mut stack,
        &mut builder,
        &mut nodes,
    )?;

    // Base pose: for nodes with animation slots, set base transform to the idle pose at t=0 to
    // match runtime behavior (fallback to ambient/fallback_basis when idle slots are missing).
    let idle_state = root_state_for_channel("idle");
    for meta in nodes.iter() {
        let Some(anim) = meta.anim.as_ref() else {
            continue;
        };
        let pose =
            sample_node_transform(library, anim, idle_state, 0.0, options.move_units_per_sec);
        builder.set_node_transform(meta.gltf_node, pose);
    }

    // Export animations (one glTF animation per channel present in the prefab).
    let has_animated_nodes = nodes.iter().any(|meta| meta.anim.is_some());
    if has_animated_nodes {
        let channels = library.animation_channels_ordered(prefab_id);
        for channel in channels {
            let channel = channel.trim().to_string();
            if channel.is_empty() {
                continue;
            }
            export_channel_animation(library, prefab_id, &channel, &nodes, &mut builder, &options)?;
        }
    }

    // Finalize scene.
    builder.root_scene_nodes = vec![root_node];
    let (mut root, bin_bytes) = builder.finish_root()?;

    // Write GLB.
    let json_bytes = json::serialize::to_vec(&root)
        .map_err(|err| format!("Failed to serialize glTF JSON: {err}"))?;
    write_glb(glb_path, &json_bytes, &bin_bytes)?;

    // Write glTF (JSON + .bin).
    let bin_file_name = bin_path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| "Invalid bin output path.".to_string())?;
    if let Some(buffer) = root.buffers.first_mut() {
        buffer.uri = Some(bin_file_name.to_string());
    }
    let gltf_json_bytes = json::serialize::to_vec(&root)
        .map_err(|err| format!("Failed to serialize glTF JSON: {err}"))?;
    write_gltf(gltf_path, &gltf_json_bytes)?;
    write_bin(bin_path, &bin_bytes)?;
    Ok(())
}

fn build_object_nodes(
    library: &ObjectLibrary,
    object_id: u128,
    parent_node: usize,
    stack: &mut Vec<u128>,
    builder: &mut GltfGlbBuilder,
    nodes: &mut Vec<ExportNodeMeta>,
) -> Result<(), String> {
    if stack.len() > 64 {
        return Err(format!(
            "Prefab composition depth exceeded (>{}).",
            stack.len()
        ));
    }
    if stack.contains(&object_id) {
        let mut path: Vec<String> = stack
            .iter()
            .map(|id| uuid::Uuid::from_u128(*id).to_string())
            .collect();
        path.push(uuid::Uuid::from_u128(object_id).to_string());
        return Err(format!(
            "Prefab composition cycle detected: {}.",
            path.join(" -> ")
        ));
    }

    let Some(def) = library.get(object_id) else {
        return Err(format!(
            "Missing object def {} referenced by prefab graph.",
            uuid::Uuid::from_u128(object_id)
        ));
    };

    stack.push(object_id);

    for (part_index, part) in def.parts.iter().enumerate() {
        let mut resolved = part.transform;
        if let Some(attachment) = part.attachment.as_ref() {
            resolved = resolve_attachment_transform(library, def, part, attachment)
                .unwrap_or(part.transform);
        }

        let node_name = format!(
            "{} part#{}",
            def.label.as_ref(),
            part_index.saturating_add(1)
        );

        let gltf_node = builder.push_node(json::Node {
            name: Some(node_name.clone()),
            ..Default::default()
        });
        builder.add_child(parent_node, gltf_node);
        builder.set_node_transform(gltf_node, resolved);

        let mut meta = ExportNodeMeta {
            gltf_node,
            anim: None,
        };

        if !part.animations.is_empty() {
            let child_object_id = match &part.kind {
                ObjectPartKind::ObjectRef { object_id } => Some(*object_id),
                _ => None,
            };
            meta.anim = Some(ExportNodeAnim {
                parent_object_id: def.object_id,
                child_object_id,
                attachment: part.attachment.clone(),
                base_transform: part.transform,
                fallback_basis: part.fallback_basis,
                slots: part.animations.clone(),
            });
        }

        match &part.kind {
            ObjectPartKind::ObjectRef { object_id: child } => {
                build_object_nodes(library, *child, gltf_node, stack, builder, nodes)?;
            }
            ObjectPartKind::Primitive { primitive } => {
                let (mesh_key, params) = primitive_mesh_key(primitive);
                let geometry_key = MeshGeometryKey {
                    mesh: mesh_key,
                    params,
                };
                let material_spec = material_spec_for_primitive_visual(primitive);
                let material_key = MaterialKeyHash::from_spec(&material_spec);
                builder.attach_mesh(gltf_node, geometry_key, &material_key, &material_spec)?;
            }
            ObjectPartKind::Model { scene } => {
                return Err(format!(
                    "Prefab contains Model part which is not supported for glTF/GLB export yet: scene={scene}"
                ));
            }
        }

        nodes.push(meta);
    }

    stack.pop();
    Ok(())
}

fn primitive_mesh_key(primitive: &PrimitiveVisualDef) -> (MeshKey, Option<PrimitiveParamsKey>) {
    match primitive {
        PrimitiveVisualDef::Mesh { mesh, .. } => (*mesh, None),
        PrimitiveVisualDef::Primitive { mesh, params, .. } => {
            let params_key = params.as_ref().and_then(|params| match (*mesh, params) {
                (MeshKey::UnitCapsule, PrimitiveParams::Capsule { .. })
                | (MeshKey::UnitConicalFrustum, PrimitiveParams::ConicalFrustum { .. })
                | (MeshKey::UnitTorus, PrimitiveParams::Torus { .. }) => {
                    Some(PrimitiveParamsKey::from_params(params))
                }
                _ => None,
            });
            (*mesh, params_key)
        }
    }
}

fn material_spec_for_primitive_visual(primitive: &PrimitiveVisualDef) -> MaterialSpec {
    match primitive {
        PrimitiveVisualDef::Mesh { material, .. } => material_spec_for_material_key(*material),
        PrimitiveVisualDef::Primitive { color, unlit, .. } => {
            let linear = color.to_linear();
            let alpha_mode = if linear.alpha < 0.999 {
                AlphaModeKey::Blend
            } else {
                AlphaModeKey::Opaque
            };
            MaterialSpec {
                base_color_linear: [linear.red, linear.green, linear.blue, linear.alpha],
                metallic: 0.0,
                roughness: 1.0,
                emissive_linear: [0.0, 0.0, 0.0],
                unlit: *unlit,
                alpha_mode,
                alpha_cutoff: None,
            }
        }
    }
}

fn material_spec_for_material_key(key: crate::object::registry::MaterialKey) -> MaterialSpec {
    let (base, metallic, roughness) = match key {
        crate::object::registry::MaterialKey::BuildBlock { index } => match index {
            0 => (Color::srgb(0.86, 0.56, 0.36), 0.0, 0.92),
            1 => (Color::srgb(0.80, 0.42, 0.28), 0.0, 0.92),
            _ => (Color::srgb(0.68, 0.30, 0.20), 0.0, 0.92),
        },
        crate::object::registry::MaterialKey::FenceStake => {
            (Color::srgb(0.22, 0.24, 0.27), 1.0, 0.35)
        }
        crate::object::registry::MaterialKey::FenceStick => {
            (Color::srgb(0.40, 0.42, 0.46), 1.0, 0.30)
        }
        crate::object::registry::MaterialKey::TreeTrunk { variant } => match variant {
            0 => (Color::srgb(0.48, 0.30, 0.16), 0.0, 0.9),
            1 => (Color::srgb(0.44, 0.28, 0.15), 0.0, 0.9),
            _ => (Color::srgb(0.52, 0.33, 0.19), 0.0, 0.9),
        },
        crate::object::registry::MaterialKey::TreeMain { variant } => match variant {
            0 => (Color::srgb(0.11, 0.42, 0.18), 0.0, 0.94),
            1 => (Color::srgb(0.10, 0.36, 0.22), 0.0, 0.94),
            _ => (Color::srgb(0.14, 0.44, 0.15), 0.0, 0.94),
        },
        crate::object::registry::MaterialKey::TreeCrown { variant } => match variant {
            0 => (Color::srgb(0.30, 0.70, 0.34), 0.0, 0.92),
            1 => (Color::srgb(0.26, 0.62, 0.40), 0.0, 0.92),
            _ => (Color::srgb(0.34, 0.74, 0.30), 0.0, 0.92),
        },
    };

    let linear = base.to_linear();
    MaterialSpec {
        base_color_linear: [linear.red, linear.green, linear.blue, linear.alpha],
        metallic,
        roughness,
        emissive_linear: [0.0, 0.0, 0.0],
        unlit: false,
        alpha_mode: AlphaModeKey::Opaque,
        alpha_cutoff: None,
    }
}

fn export_channel_animation(
    library: &ObjectLibrary,
    prefab_id: u128,
    channel: &str,
    nodes: &[ExportNodeMeta],
    builder: &mut GltfGlbBuilder,
    options: &PrefabGltfGlbExportOptions,
) -> Result<(), String> {
    let duration = animation_duration_secs(library, prefab_id, channel);
    let fps = options.fps.max(1) as f32;
    let frames = (duration * fps).round().max(1.0) as usize;
    let sample_count = frames.saturating_add(1);

    let mut times: Vec<f32> = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let t = (i as f32) / fps;
        times.push(t.min(duration));
    }

    let root_state = root_state_for_channel(channel);
    builder.add_animation(
        channel,
        &times,
        nodes
            .iter()
            .filter_map(|meta| meta.anim.as_ref().map(|anim| (meta.gltf_node, anim))),
        library,
        root_state,
        options.move_units_per_sec,
    )?;

    Ok(())
}

fn animation_duration_secs(library: &ObjectLibrary, prefab_id: u128, channel: &str) -> f32 {
    let channel = channel.trim();
    if channel.is_empty() {
        return DEFAULT_LOOP_DURATION_SECS;
    }

    let mut duration = match channel {
        "attack" => library
            .channel_attack_duration_secs(prefab_id, "attack")
            .unwrap_or(DEFAULT_ACTION_DURATION_SECS),
        "action" => library
            .channel_action_duration_secs(prefab_id, "action")
            .unwrap_or(DEFAULT_ACTION_DURATION_SECS),
        _ => DEFAULT_LOOP_DURATION_SECS,
    };

    if !duration.is_finite() {
        duration = DEFAULT_LOOP_DURATION_SECS;
    }

    duration.clamp(0.05, MAX_ANIM_DURATION_SECS)
}

fn sample_node_transform(
    library: &ObjectLibrary,
    node: &ExportNodeAnim,
    state: ExportRootState<'_>,
    wall_time_secs: f32,
    move_units_per_sec: f32,
) -> Transform {
    let forced_channel = state.forced_channel;

    fn choose_first_spec<'a>(
        slots: &'a [PartAnimationSlot],
        channel: &str,
    ) -> Option<&'a PartAnimationSpec> {
        let channel = channel.trim();
        if channel.is_empty() {
            return None;
        }
        slots
            .iter()
            .find(|slot| slot.channel.as_ref() == channel)
            .map(|slot| &slot.spec)
    }

    let mut chosen: Option<&PartAnimationSpec> = None;
    if let Some(forced) = forced_channel {
        chosen = choose_first_spec(&node.slots, forced);
    }

    if chosen.is_none() {
        for channel in ["attack", "action", "move", "idle", "ambient"] {
            let active = match channel {
                "attack" => state.attack_active,
                "action" => state.action_active,
                "move" => state.move_active,
                "idle" => state.idle_active(),
                "ambient" => true,
                _ => false,
            };
            if !active {
                continue;
            }
            if let Some(spec) = choose_first_spec(&node.slots, channel) {
                chosen = Some(spec);
                break;
            }
        }
    }

    let (basis, delta, spec, attachment) = if let Some(spec) = chosen {
        let driver_time = match spec.driver {
            PartAnimationDriver::Always
            | PartAnimationDriver::AttackTime
            | PartAnimationDriver::ActionTime => wall_time_secs,
            PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance => {
                wall_time_secs * move_units_per_sec
            }
        };

        let mut t = driver_time * spec.speed_scale.max(0.0);
        if spec.time_offset_units.is_finite() {
            t += spec.time_offset_units;
        }
        (
            sanitize_transform(spec.basis),
            sample_part_animation(&spec.clip, t),
            Some(spec),
            node.attachment.as_ref(),
        )
    } else {
        (
            sanitize_transform(node.fallback_basis),
            Transform::IDENTITY,
            None,
            node.attachment.as_ref(),
        )
    };

    let base = sanitize_transform(node.base_transform);
    let base_with_basis = mul_transform(&base, &basis);
    let animated_base = match (spec, attachment) {
        (Some(spec), Some(_attachment)) => match &spec.clip {
            PartAnimationDef::Spin {
                axis_space: PartAnimationSpinAxisSpace::ChildLocal,
                ..
            } => {
                let child_anchor = node
                    .child_object_id
                    .and_then(|object_id| library.get(object_id))
                    .and_then(|def| {
                        anchor_transform(
                            def,
                            node.attachment.as_ref().unwrap().child_anchor.as_ref(),
                        )
                    })
                    .unwrap_or(Transform::IDENTITY);
                apply_child_local_delta_to_attachment_offset(base_with_basis, child_anchor, delta)
            }
            _ => mul_transform(&base_with_basis, &delta),
        },
        _ => mul_transform(&base_with_basis, &delta),
    };

    if let Some(attachment) = node.attachment.as_ref() {
        let local = library
            .get(node.parent_object_id)
            .and_then(|parent_def| {
                resolve_attachment_transform_with_offset(
                    library,
                    parent_def,
                    node.child_object_id,
                    attachment,
                    &animated_base,
                )
            })
            .unwrap_or(animated_base);
        sanitize_transform(local)
    } else {
        sanitize_transform(animated_base)
    }
}

fn sanitize_transform(t: Transform) -> Transform {
    let mut out = t;
    if !out.translation.is_finite() {
        out.translation = Vec3::ZERO;
    }
    if !out.rotation.is_finite() {
        out.rotation = Quat::IDENTITY;
    } else {
        out.rotation = out.rotation.normalize();
    }
    if !out.scale.is_finite() {
        out.scale = Vec3::ONE;
    }
    out
}

fn mul_transform(a: &Transform, b: &Transform) -> Transform {
    let composed = a.to_matrix() * b.to_matrix();
    crate::geometry::mat4_to_transform_allow_degenerate_scale(composed).unwrap_or(*b)
}

fn anchor_transform(def: &ObjectDef, name: &str) -> Option<Transform> {
    if name == "origin" {
        return Some(Transform::IDENTITY);
    }
    def.anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
}

fn resolve_attachment_transform(
    library: &ObjectLibrary,
    parent_def: &ObjectDef,
    part: &ObjectPartDef,
    attachment: &AttachmentDef,
) -> Option<Transform> {
    let parent_anchor = anchor_transform(parent_def, attachment.parent_anchor.as_ref())?;
    let child_anchor = match &part.kind {
        ObjectPartKind::ObjectRef { object_id } => library
            .get(*object_id)
            .and_then(|def| anchor_transform(def, attachment.child_anchor.as_ref()))
            .unwrap_or(Transform::IDENTITY),
        _ => Transform::IDENTITY,
    };

    let composed =
        parent_anchor.to_matrix() * part.transform.to_matrix() * child_anchor.to_matrix().inverse();
    crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
}

fn resolve_attachment_transform_with_offset(
    library: &ObjectLibrary,
    parent_def: &ObjectDef,
    child_object_id: Option<u128>,
    attachment: &AttachmentDef,
    offset: &Transform,
) -> Option<Transform> {
    let parent_anchor = anchor_transform(parent_def, attachment.parent_anchor.as_ref())?;
    let child_anchor = child_object_id
        .and_then(|object_id| {
            library
                .get(object_id)
                .and_then(|def| anchor_transform(def, attachment.child_anchor.as_ref()))
        })
        .unwrap_or(Transform::IDENTITY);

    let composed =
        parent_anchor.to_matrix() * offset.to_matrix() * child_anchor.to_matrix().inverse();
    crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
}

fn apply_child_local_delta_to_attachment_offset(
    base_offset: Transform,
    child_anchor: Transform,
    delta_child_local: Transform,
) -> Transform {
    let child_anchor_mat = child_anchor.to_matrix();
    let inv_child_anchor = child_anchor_mat.inverse();
    if !inv_child_anchor.is_finite() {
        return mul_transform(&base_offset, &delta_child_local);
    }

    // `offset` is applied as: parent_anchor * offset * inv(child_anchor).
    // If we want a delta in the CHILD's local frame, the desired composition is:
    //   parent_anchor * offset * inv(child_anchor) * delta_child_local
    // Rebase into the offset slot:
    //   offset' = offset * inv(child_anchor) * delta * child_anchor
    let rebased_mat = inv_child_anchor * delta_child_local.to_matrix() * child_anchor_mat;
    let rebased = crate::geometry::mat4_to_transform_allow_degenerate_scale(rebased_mat)
        .unwrap_or(delta_child_local);
    mul_transform(&base_offset, &rebased)
}

fn sample_part_animation(animation: &PartAnimationDef, time_secs: f32) -> Transform {
    match animation {
        PartAnimationDef::Loop {
            duration_secs,
            keyframes,
        } => sample_keyframes_loop(*duration_secs, keyframes, time_secs),
        PartAnimationDef::Once {
            duration_secs,
            keyframes,
        } => sample_keyframes_clamped(*duration_secs, keyframes, time_secs),
        PartAnimationDef::PingPong {
            duration_secs,
            keyframes,
        } => {
            let duration = (*duration_secs).max(1e-6);
            let mut t = if time_secs.is_finite() {
                time_secs
            } else {
                0.0
            };
            let period = duration * 2.0;
            t = t.rem_euclid(period);
            if t > duration {
                t = period - t;
            }
            sample_keyframes_clamped(duration, keyframes, t)
        }
        PartAnimationDef::Spin {
            axis,
            radians_per_unit,
            ..
        } => {
            let axis = if axis.length_squared() > 1e-6 {
                axis.normalize()
            } else {
                Vec3::Y
            };
            let angle = if time_secs.is_finite() && radians_per_unit.is_finite() {
                time_secs * *radians_per_unit
            } else {
                0.0
            };
            Transform {
                translation: Vec3::ZERO,
                rotation: Quat::from_axis_angle(axis, angle),
                scale: Vec3::ONE,
            }
        }
    }
}

fn sample_keyframes_loop(
    duration_secs: f32,
    keyframes: &[crate::object::registry::PartAnimationKeyframeDef],
    time_secs: f32,
) -> Transform {
    let duration = duration_secs.max(1e-6);
    let mut t = if time_secs.is_finite() {
        time_secs
    } else {
        0.0
    };
    t = t.rem_euclid(duration);

    if keyframes.is_empty() {
        return Transform::IDENTITY;
    }
    if keyframes.len() == 1 {
        return keyframes[0].delta;
    }

    let mut prev = &keyframes[0];
    for next in &keyframes[1..] {
        if t < next.time_secs {
            let dt = (next.time_secs - prev.time_secs).max(1e-6);
            let alpha = ((t - prev.time_secs) / dt).clamp(0.0, 1.0);
            return lerp_transform(&prev.delta, &next.delta, alpha);
        }
        prev = next;
    }

    let first = &keyframes[0];
    let last = prev;
    let t0 = last.time_secs;
    let t1 = duration + first.time_secs;
    let dt = (t1 - t0).max(1e-6);
    let alpha = ((t - t0) / dt).clamp(0.0, 1.0);
    lerp_transform(&last.delta, &first.delta, alpha)
}

fn sample_keyframes_clamped(
    duration_secs: f32,
    keyframes: &[crate::object::registry::PartAnimationKeyframeDef],
    time_secs: f32,
) -> Transform {
    let duration = duration_secs.max(1e-6);
    let mut t = if time_secs.is_finite() {
        time_secs
    } else {
        0.0
    };
    t = t.clamp(0.0, duration);

    if keyframes.is_empty() {
        return Transform::IDENTITY;
    }
    if keyframes.len() == 1 {
        return keyframes[0].delta;
    }

    if t <= keyframes[0].time_secs {
        return keyframes[0].delta;
    }

    let mut prev = &keyframes[0];
    for next in &keyframes[1..] {
        if t < next.time_secs {
            let dt = (next.time_secs - prev.time_secs).max(1e-6);
            let alpha = ((t - prev.time_secs) / dt).clamp(0.0, 1.0);
            return lerp_transform(&prev.delta, &next.delta, alpha);
        }
        prev = next;
    }

    prev.delta
}

fn lerp_transform(a: &Transform, b: &Transform, alpha: f32) -> Transform {
    let translation = a.translation.lerp(b.translation, alpha);
    let rotation = a.rotation.slerp(b.rotation, alpha).normalize();
    let scale = a.scale.lerp(b.scale, alpha);
    Transform {
        translation,
        rotation,
        scale,
    }
}

#[derive(Clone, Copy, Debug)]
struct GeometryAccessors {
    positions: json::Index<json::Accessor>,
    normals: Option<json::Index<json::Accessor>>,
    indices: Option<json::Index<json::Accessor>>,
}

struct GltfGlbBuilder {
    root: json::Root,
    bin: Vec<u8>,
    node_children: HashMap<usize, Vec<usize>>,
    node_transforms: HashMap<usize, Transform>,
    root_scene_nodes: Vec<usize>,
    geometry_cache: HashMap<MeshGeometryKey, GeometryAccessors>,
    mesh_cache: HashMap<MeshInstanceKey, json::Index<json::Mesh>>,
    material_cache: HashMap<MaterialKeyHash, json::Index<json::Material>>,
    uses_unlit: bool,
}

impl GltfGlbBuilder {
    fn new() -> Self {
        let mut root = json::Root::default();
        root.asset = json::Asset {
            generator: Some("gravimera".to_string()),
            version: "2.0".to_string(),
            ..Default::default()
        };
        Self {
            root,
            bin: Vec::new(),
            node_children: HashMap::new(),
            node_transforms: HashMap::new(),
            root_scene_nodes: Vec::new(),
            geometry_cache: HashMap::new(),
            mesh_cache: HashMap::new(),
            material_cache: HashMap::new(),
            uses_unlit: false,
        }
    }

    fn push_node(&mut self, mut node: json::Node) -> usize {
        let idx = self.root.nodes.len();
        node.children = None;
        node.mesh = None;
        node.translation = None;
        node.rotation = None;
        node.scale = None;
        self.root.nodes.push(node);
        idx
    }

    fn add_child(&mut self, parent: usize, child: usize) {
        self.node_children.entry(parent).or_default().push(child);
    }

    fn set_node_transform(&mut self, node: usize, transform: Transform) {
        self.node_transforms
            .insert(node, sanitize_transform(transform));
    }

    fn attach_mesh(
        &mut self,
        node: usize,
        geometry_key: MeshGeometryKey,
        material_key: &MaterialKeyHash,
        material_spec: &MaterialSpec,
    ) -> Result<(), String> {
        let mesh_index = self.get_or_create_mesh(geometry_key, material_key, material_spec)?;
        self.root.nodes[node].mesh = Some(mesh_index);
        Ok(())
    }

    fn get_or_create_mesh(
        &mut self,
        geometry_key: MeshGeometryKey,
        material_key: &MaterialKeyHash,
        material_spec: &MaterialSpec,
    ) -> Result<json::Index<json::Mesh>, String> {
        let cache_key = MeshInstanceKey {
            geometry: geometry_key,
            material: *material_key,
        };
        if let Some(existing) = self.mesh_cache.get(&cache_key) {
            return Ok(*existing);
        }

        let geo = self.get_or_create_geometry(geometry_key)?;
        let material_idx = self.get_or_create_material(material_key, material_spec);

        let mut attributes: BTreeMap<
            json::validation::Checked<json::mesh::Semantic>,
            json::Index<json::Accessor>,
        > = BTreeMap::new();
        attributes.insert(
            json::validation::Checked::Valid(json::mesh::Semantic::Positions),
            geo.positions,
        );
        if let Some(normals) = geo.normals {
            attributes.insert(
                json::validation::Checked::Valid(json::mesh::Semantic::Normals),
                normals,
            );
        }

        let primitive = json::mesh::Primitive {
            attributes,
            extensions: None,
            extras: Default::default(),
            indices: geo.indices,
            material: Some(material_idx),
            mode: json::validation::Checked::Valid(json::mesh::Mode::Triangles),
            targets: None,
        };

        let json_mesh = json::Mesh {
            extensions: None,
            extras: Default::default(),
            name: None,
            primitives: vec![primitive],
            weights: None,
        };

        let idx = json::Index::new(self.root.meshes.len() as u32);
        self.root.meshes.push(json_mesh);
        self.mesh_cache.insert(cache_key, idx);
        Ok(idx)
    }

    fn get_or_create_geometry(
        &mut self,
        key: MeshGeometryKey,
    ) -> Result<GeometryAccessors, String> {
        if let Some(existing) = self.geometry_cache.get(&key) {
            return Ok(*existing);
        }

        let mesh = build_bevy_mesh_from_key(key)?;
        if mesh.primitive_topology()
            != bevy::render::render_resource::PrimitiveTopology::TriangleList
        {
            return Err("Only triangle list meshes are supported for glTF/GLB export.".to_string());
        }

        let Some(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|v| v.as_float3())
        else {
            return Err("Mesh is missing POSITION attribute.".to_string());
        };

        let normals = mesh
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .and_then(|v| v.as_float3());

        let indices: Option<Vec<u32>> = mesh.indices().map(|indices| {
            indices
                .iter()
                .map(|idx| idx.try_into().unwrap_or(0u32))
                .collect()
        });

        let pos_accessor = self.push_accessor_vec3_f32(positions, Some("POSITION"))?;
        let normal_accessor = if let Some(normals) = normals {
            Some(self.push_accessor_vec3_f32(normals, Some("NORMAL"))?)
        } else {
            None
        };
        let indices_accessor = if let Some(indices) = indices.as_ref() {
            Some(self.push_accessor_indices_u32(indices)?)
        } else {
            None
        };

        let accessors = GeometryAccessors {
            positions: pos_accessor,
            normals: normal_accessor,
            indices: indices_accessor,
        };
        self.geometry_cache.insert(key, accessors);
        Ok(accessors)
    }

    fn get_or_create_material(
        &mut self,
        key: &MaterialKeyHash,
        spec: &MaterialSpec,
    ) -> json::Index<json::Material> {
        if let Some(existing) = self.material_cache.get(key) {
            return *existing;
        }

        if spec.unlit {
            self.uses_unlit = true;
        }

        let alpha_mode = match spec.alpha_mode {
            AlphaModeKey::Opaque => json::material::AlphaMode::Opaque,
            AlphaModeKey::Blend => json::material::AlphaMode::Blend,
        };

        let pbr = json::material::PbrMetallicRoughness {
            base_color_factor: json::material::PbrBaseColorFactor(spec.base_color_linear),
            metallic_factor: json::material::StrengthFactor(spec.metallic),
            roughness_factor: json::material::StrengthFactor(spec.roughness),
            ..Default::default()
        };

        let mut material = json::Material::default();
        material.pbr_metallic_roughness = pbr;
        material.emissive_factor = json::material::EmissiveFactor(spec.emissive_linear);
        material.alpha_mode = json::validation::Checked::Valid(alpha_mode);
        material.alpha_cutoff = spec.alpha_cutoff.map(json::material::AlphaCutoff);
        material.double_sided = false;

        if spec.unlit {
            material.extensions = Some(json::extensions::material::Material {
                unlit: Some(json::extensions::material::Unlit {}),
                ..Default::default()
            });
        }

        let idx = json::Index::new(self.root.materials.len() as u32);
        self.root.materials.push(material);
        self.material_cache.insert(*key, idx);
        idx
    }

    fn push_aligned(&mut self, bytes: &[u8]) -> (u64, u64) {
        let align = 4usize;
        let padding = (align - (self.bin.len() % align)) % align;
        for _ in 0..padding {
            self.bin.push(0);
        }
        let offset = self.bin.len() as u64;
        self.bin.extend_from_slice(bytes);
        let len = bytes.len() as u64;
        (offset, len)
    }

    fn push_view(
        &mut self,
        offset: u64,
        len: u64,
        target: Option<json::buffer::Target>,
    ) -> json::Index<json::buffer::View> {
        let view = json::buffer::View {
            buffer: json::Index::new(0),
            byte_length: json::validation::USize64(len),
            byte_offset: Some(json::validation::USize64(offset)),
            byte_stride: None,
            name: None,
            target: target.map(json::validation::Checked::Valid),
            extensions: None,
            extras: Default::default(),
        };
        let idx = json::Index::new(self.root.buffer_views.len() as u32);
        self.root.buffer_views.push(view);
        idx
    }

    fn push_accessor_vec3_f32(
        &mut self,
        values: &[[f32; 3]],
        name: Option<&str>,
    ) -> Result<json::Index<json::Accessor>, String> {
        let mut bytes: Vec<u8> = Vec::with_capacity(values.len() * 12);
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in values {
            for i in 0..3 {
                min[i] = min[i].min(v[i]);
                max[i] = max[i].max(v[i]);
            }
            for i in 0..3 {
                bytes.extend_from_slice(&v[i].to_le_bytes());
            }
        }
        let (offset, len) = self.push_aligned(&bytes);
        let view = self.push_view(offset, len, None);

        let accessor = json::Accessor {
            buffer_view: Some(view),
            byte_offset: Some(json::validation::USize64(0)),
            count: json::validation::USize64::from(values.len()),
            component_type: json::validation::Checked::Valid(json::accessor::GenericComponentType(
                json::accessor::ComponentType::F32,
            )),
            extensions: None,
            extras: Default::default(),
            type_: json::validation::Checked::Valid(json::accessor::Type::Vec3),
            min: Some(serde_json::json!([min[0], min[1], min[2]])),
            max: Some(serde_json::json!([max[0], max[1], max[2]])),
            name: name.map(|s| s.to_string()),
            normalized: false,
            sparse: None,
        };
        let idx = json::Index::new(self.root.accessors.len() as u32);
        self.root.accessors.push(accessor);
        Ok(idx)
    }

    fn push_accessor_indices_u32(
        &mut self,
        values: &[u32],
    ) -> Result<json::Index<json::Accessor>, String> {
        let mut bytes: Vec<u8> = Vec::with_capacity(values.len() * 4);
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let (offset, len) = self.push_aligned(&bytes);
        let view = self.push_view(offset, len, None);

        let accessor = json::Accessor {
            buffer_view: Some(view),
            byte_offset: Some(json::validation::USize64(0)),
            count: json::validation::USize64::from(values.len()),
            component_type: json::validation::Checked::Valid(json::accessor::GenericComponentType(
                json::accessor::ComponentType::U32,
            )),
            extensions: None,
            extras: Default::default(),
            type_: json::validation::Checked::Valid(json::accessor::Type::Scalar),
            min: None,
            max: None,
            name: None,
            normalized: false,
            sparse: None,
        };
        let idx = json::Index::new(self.root.accessors.len() as u32);
        self.root.accessors.push(accessor);
        Ok(idx)
    }

    fn add_animation<'a>(
        &mut self,
        name: &str,
        times: &[f32],
        nodes: impl Iterator<Item = (usize, &'a ExportNodeAnim)>,
        library: &ObjectLibrary,
        root_state: ExportRootState<'_>,
        move_units_per_sec: f32,
    ) -> Result<(), String> {
        let time_accessor = self.push_accessor_times(times)?;

        let mut samplers: Vec<json::animation::Sampler> = Vec::new();
        let mut channels: Vec<json::animation::Channel> = Vec::new();

        for (node_index, node_anim) in nodes {
            let mut translations: Vec<[f32; 3]> = Vec::with_capacity(times.len());
            let mut rotations: Vec<[f32; 4]> = Vec::with_capacity(times.len());
            let mut scales: Vec<[f32; 3]> = Vec::with_capacity(times.len());

            for t in times {
                let transform =
                    sample_node_transform(library, node_anim, root_state, *t, move_units_per_sec);
                translations.push([
                    transform.translation.x,
                    transform.translation.y,
                    transform.translation.z,
                ]);
                rotations.push([
                    transform.rotation.x,
                    transform.rotation.y,
                    transform.rotation.z,
                    transform.rotation.w,
                ]);
                scales.push([transform.scale.x, transform.scale.y, transform.scale.z]);
            }

            let trans_accessor = self.push_accessor_vec3_f32(&translations, None)?;
            let rot_accessor = self.push_accessor_vec4_f32(&rotations)?;
            let scale_accessor = self.push_accessor_vec3_f32(&scales, None)?;

            let trans_sampler = json::animation::Sampler {
                extensions: None,
                extras: Default::default(),
                input: time_accessor,
                interpolation: json::validation::Checked::Valid(
                    json::animation::Interpolation::Linear,
                ),
                output: trans_accessor,
            };
            let rot_sampler = json::animation::Sampler {
                extensions: None,
                extras: Default::default(),
                input: time_accessor,
                interpolation: json::validation::Checked::Valid(
                    json::animation::Interpolation::Linear,
                ),
                output: rot_accessor,
            };
            let scale_sampler = json::animation::Sampler {
                extensions: None,
                extras: Default::default(),
                input: time_accessor,
                interpolation: json::validation::Checked::Valid(
                    json::animation::Interpolation::Linear,
                ),
                output: scale_accessor,
            };

            let trans_sampler_idx = json::Index::new(samplers.len() as u32);
            samplers.push(trans_sampler);
            let rot_sampler_idx = json::Index::new(samplers.len() as u32);
            samplers.push(rot_sampler);
            let scale_sampler_idx = json::Index::new(samplers.len() as u32);
            samplers.push(scale_sampler);

            let target_node = json::Index::new(node_index as u32);

            channels.push(json::animation::Channel {
                sampler: trans_sampler_idx,
                target: json::animation::Target {
                    extensions: None,
                    extras: Default::default(),
                    node: target_node,
                    path: json::validation::Checked::Valid(json::animation::Property::Translation),
                },
                extensions: None,
                extras: Default::default(),
            });
            channels.push(json::animation::Channel {
                sampler: rot_sampler_idx,
                target: json::animation::Target {
                    extensions: None,
                    extras: Default::default(),
                    node: target_node,
                    path: json::validation::Checked::Valid(json::animation::Property::Rotation),
                },
                extensions: None,
                extras: Default::default(),
            });
            channels.push(json::animation::Channel {
                sampler: scale_sampler_idx,
                target: json::animation::Target {
                    extensions: None,
                    extras: Default::default(),
                    node: target_node,
                    path: json::validation::Checked::Valid(json::animation::Property::Scale),
                },
                extensions: None,
                extras: Default::default(),
            });
        }

        let anim = json::Animation {
            extensions: None,
            extras: Default::default(),
            channels,
            name: Some(name.to_string()),
            samplers,
        };
        self.root.animations.push(anim);
        Ok(())
    }

    fn push_accessor_times(
        &mut self,
        times: &[f32],
    ) -> Result<json::Index<json::Accessor>, String> {
        let mut bytes: Vec<u8> = Vec::with_capacity(times.len() * 4);
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for t in times {
            let t = if t.is_finite() { *t } else { 0.0 };
            min = min.min(t);
            max = max.max(t);
            bytes.extend_from_slice(&t.to_le_bytes());
        }
        let (offset, len) = self.push_aligned(&bytes);
        let view = self.push_view(offset, len, None);

        let accessor = json::Accessor {
            buffer_view: Some(view),
            byte_offset: Some(json::validation::USize64(0)),
            count: json::validation::USize64::from(times.len()),
            component_type: json::validation::Checked::Valid(json::accessor::GenericComponentType(
                json::accessor::ComponentType::F32,
            )),
            extensions: None,
            extras: Default::default(),
            type_: json::validation::Checked::Valid(json::accessor::Type::Scalar),
            min: Some(serde_json::json!([min])),
            max: Some(serde_json::json!([max])),
            name: None,
            normalized: false,
            sparse: None,
        };
        let idx = json::Index::new(self.root.accessors.len() as u32);
        self.root.accessors.push(accessor);
        Ok(idx)
    }

    fn push_accessor_vec4_f32(
        &mut self,
        values: &[[f32; 4]],
    ) -> Result<json::Index<json::Accessor>, String> {
        let mut bytes: Vec<u8> = Vec::with_capacity(values.len() * 16);
        for v in values {
            for i in 0..4 {
                bytes.extend_from_slice(&v[i].to_le_bytes());
            }
        }
        let (offset, len) = self.push_aligned(&bytes);
        let view = self.push_view(offset, len, None);

        let accessor = json::Accessor {
            buffer_view: Some(view),
            byte_offset: Some(json::validation::USize64(0)),
            count: json::validation::USize64::from(values.len()),
            component_type: json::validation::Checked::Valid(json::accessor::GenericComponentType(
                json::accessor::ComponentType::F32,
            )),
            extensions: None,
            extras: Default::default(),
            type_: json::validation::Checked::Valid(json::accessor::Type::Vec4),
            min: None,
            max: None,
            name: None,
            normalized: false,
            sparse: None,
        };
        let idx = json::Index::new(self.root.accessors.len() as u32);
        self.root.accessors.push(accessor);
        Ok(idx)
    }

    fn finish_root(mut self) -> Result<(json::Root, Vec<u8>), String> {
        // Stitch children + transforms into root nodes.
        for (idx, node) in self.root.nodes.iter_mut().enumerate() {
            if let Some(children) = self.node_children.get(&idx) {
                node.children = Some(
                    children
                        .iter()
                        .map(|c| json::Index::new(*c as u32))
                        .collect(),
                );
            }
            if let Some(t) = self.node_transforms.get(&idx).copied() {
                node.translation = Some([t.translation.x, t.translation.y, t.translation.z]);
                node.rotation = Some(json::scene::UnitQuaternion([
                    t.rotation.x,
                    t.rotation.y,
                    t.rotation.z,
                    t.rotation.w,
                ]));
                node.scale = Some([t.scale.x, t.scale.y, t.scale.z]);
            }
        }

        // Single buffer for the BIN chunk.
        self.root.buffers.push(json::Buffer {
            byte_length: json::validation::USize64::from(self.bin.len()),
            uri: None,
            name: None,
            extensions: None,
            extras: Default::default(),
        });

        if self.uses_unlit {
            self.root
                .extensions_used
                .push("KHR_materials_unlit".to_string());
            self.root
                .extensions_required
                .push("KHR_materials_unlit".to_string());
        }

        let scene = json::Scene {
            extensions: None,
            extras: Default::default(),
            name: None,
            nodes: self
                .root_scene_nodes
                .iter()
                .map(|n| json::Index::new(*n as u32))
                .collect(),
        };
        self.root.scenes.push(scene);
        self.root.scene = Some(json::Index::new(0));

        Ok((self.root, self.bin))
    }
}

fn build_bevy_mesh_from_key(key: MeshGeometryKey) -> Result<Mesh, String> {
    let mesh: Mesh = match (key.mesh, key.params) {
        (MeshKey::UnitCube, _) => Cuboid::new(1.0, 1.0, 1.0).into(),
        (MeshKey::UnitCylinder, _) => Cylinder::new(0.5, 1.0).into(),
        (MeshKey::UnitCone, _) => Cone::new(0.5, 1.0).into(),
        (MeshKey::UnitSphere, _) => Sphere::new(0.5).into(),
        (MeshKey::UnitPlane, _) => Plane3d::default().into(),
        (MeshKey::UnitCapsule, Some(params)) if params.kind == 1 => {
            let radius = (params.a_milli as f32) / 1000.0;
            let half_length = (params.b_milli as f32) / 1000.0;
            Capsule3d::new(radius, half_length).into()
        }
        (MeshKey::UnitCapsule, _) => Capsule3d::new(0.25, 0.5).into(),
        (MeshKey::UnitConicalFrustum, Some(params)) if params.kind == 2 => ConicalFrustum {
            radius_top: (params.a_milli as f32) / 1000.0,
            radius_bottom: (params.b_milli as f32) / 1000.0,
            height: (params.c_milli as f32) / 1000.0,
        }
        .into(),
        (MeshKey::UnitConicalFrustum, _) => ConicalFrustum {
            radius_top: 0.25,
            radius_bottom: 0.5,
            height: 1.0,
        }
        .into(),
        (MeshKey::UnitTorus, Some(params)) if params.kind == 3 => Torus::new(
            (params.a_milli as f32) / 1000.0,
            (params.b_milli as f32) / 1000.0,
        )
        .into(),
        (MeshKey::UnitTorus, _) => Torus::new(0.25, 0.5).into(),
        (MeshKey::UnitTriangle, _) => Triangle3d::new(
            Vec3::new(0.0, 0.0, 0.5),
            Vec3::new(-0.5, 0.0, -0.5),
            Vec3::new(0.5, 0.0, -0.5),
        )
        .into(),
        (MeshKey::UnitTetrahedron, _) => Tetrahedron::default().into(),
        (MeshKey::TreeTrunk, _) => Cylinder::new(1.0, 1.0).into(),
        (MeshKey::TreeCone, _) => Cone::new(1.0, 1.0).into(),
    };
    Ok(mesh)
}

fn write_glb(path: &Path, json_bytes: &[u8], bin_bytes: &[u8]) -> Result<(), String> {
    fn pad4(mut v: Vec<u8>, pad: u8) -> Vec<u8> {
        while v.len() % 4 != 0 {
            v.push(pad);
        }
        v
    }

    let json_padded = pad4(json_bytes.to_vec(), b' ');
    let bin_padded = pad4(bin_bytes.to_vec(), 0);

    let total_len = 12u32 + 8u32 + json_padded.len() as u32 + 8u32 + bin_padded.len() as u32;

    let mut out: Vec<u8> = Vec::with_capacity(total_len as usize);
    out.extend_from_slice(&0x46546C67u32.to_le_bytes()); // 'glTF'
    out.extend_from_slice(&2u32.to_le_bytes()); // version
    out.extend_from_slice(&total_len.to_le_bytes());

    out.extend_from_slice(&(json_padded.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // 'JSON'
    out.extend_from_slice(&json_padded);

    out.extend_from_slice(&(bin_padded.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E4942u32.to_le_bytes()); // 'BIN\0'
    out.extend_from_slice(&bin_padded);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, out).map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn write_gltf(path: &Path, json_bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, json_bytes)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn write_bin(path: &Path, bin_bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, bin_bytes)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}
