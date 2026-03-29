use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::object::registry::{
    builtin_object_id, MeshKey, ObjectDef, ObjectPartDef, ObjectPartKind, PartAnimationDef,
    PartAnimationDriver, PartAnimationFamily, PartAnimationKeyframeDef, PartAnimationSlot,
    PartAnimationSpec, PrimitiveParams, PrimitiveVisualDef,
};

use super::super::state::Gen3dDraft;
use super::super::GEN3D_MAX_PARTS;
use super::agent_utils::sanitize_prefix;
use super::artifacts::{append_gen3d_jsonl_artifact, write_gen3d_json_artifact};
use super::convert;
use super::schema::{
    AiAnimationClipJsonV1, AiAnimationDeltaTransformJsonV1, AiAnimationDriverJsonV1, AiJointJson,
    AiJointKindJson,
};
use super::{Gen3dAiJob, Gen3dPlannedArticulationNode, Gen3dPlannedComponent};

const LEGACY_INTERNAL_BASE_CHANNEL: &str = "__base";

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformDeltaJsonV1 {
    #[serde(default)]
    pos: Option<[f32; 3]>,
    #[serde(default)]
    scale: Option<[f32; 3]>,
    #[serde(default)]
    rot_quat_xyzw: Option<[f32; 4]>,
    #[serde(default)]
    forward: Option<[f32; 3]>,
    #[serde(default)]
    up: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum NullableStringFieldJsonV1 {
    String(String),
    Null(()),
}

impl NullableStringFieldJsonV1 {
    fn into_option_trimmed(self) -> Option<String> {
        match self {
            Self::String(value) => {
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_string())
            }
            Self::Null(()) => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrimitiveParamsJsonV1Capsule {
    radius: f32,
    half_length: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrimitiveParamsJsonV1ConicalFrustum {
    top_radius: f32,
    bottom_radius: f32,
    height: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrimitiveParamsJsonV1Torus {
    minor_radius: f32,
    major_radius: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum PrimitiveParamsJsonV1 {
    Capsule(PrimitiveParamsJsonV1Capsule),
    ConicalFrustum(PrimitiveParamsJsonV1ConicalFrustum),
    Torus(PrimitiveParamsJsonV1Torus),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrimitiveSpecJsonV1 {
    mesh: String,
    #[serde(default)]
    params: Option<PrimitiveParamsJsonV1>,
    #[serde(default)]
    color_rgba: Option<[f32; 4]>,
    #[serde(default)]
    unlit: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnimationSlotSpecJsonV1 {
    #[serde(default)]
    family: super::schema::AiAnimationFamilyJsonV1,
    driver: AiAnimationDriverJsonV1,
    speed_scale: f32,
    #[serde(default)]
    time_offset_units: f32,
    clip: AiAnimationClipJsonV1,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DraftOpJsonV1 {
    SetAnchorTransform {
        component: String,
        anchor: String,
        set: TransformDeltaJsonV1,
    },
    SetAttachmentOffset {
        child_component: String,
        set: TransformDeltaJsonV1,
    },
    SetAttachmentJoint {
        child_component: String,
        set_joint: Option<AiJointJson>,
    },
    UpdatePrimitivePart {
        component: String,
        part_id_uuid: String,
        #[serde(default)]
        set_transform: Option<TransformDeltaJsonV1>,
        #[serde(default)]
        set_primitive: Option<PrimitiveSpecJsonV1>,
        #[serde(default)]
        set_render_priority: Option<i32>,
    },
    AddPrimitivePart {
        component: String,
        part_id_uuid: String,
        primitive: PrimitiveSpecJsonV1,
        transform: TransformDeltaJsonV1,
        #[serde(default)]
        render_priority: Option<i32>,
    },
    RemovePrimitivePart {
        component: String,
        part_id_uuid: String,
    },
    UpsertArticulationNode {
        component: String,
        node_id: String,
        parent_node_id: NullableStringFieldJsonV1,
        set_transform: TransformDeltaJsonV1,
        bound_part_id_uuids: Vec<String>,
    },
    RemoveArticulationNode {
        component: String,
        node_id: String,
    },
    RebindArticulationNodeParts {
        component: String,
        node_id: String,
        bound_part_id_uuids: Vec<String>,
    },
    UpsertAnimationSlot {
        child_component: String,
        channel: String,
        slot: AnimationSlotSpecJsonV1,
    },
    ScaleAnimationSlotRotation {
        child_component: String,
        channel: String,
        scale: f32,
    },
    RemoveAnimationSlot {
        child_component: String,
        channel: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyDraftOpsArgsJsonV1 {
    #[serde(default)]
    version: u32,
    #[serde(default = "default_true")]
    atomic: bool,
    #[serde(default)]
    if_assembly_rev: Option<u32>,
    ops: Vec<DraftOpJsonV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryComponentPartsArgsJsonV1 {
    #[serde(default)]
    version: u32,
    #[serde(default, alias = "component_name", alias = "name")]
    component: Option<String>,
    #[serde(default, alias = "component_idx", alias = "index")]
    component_index: Option<usize>,
    #[serde(default)]
    include_non_primitives: bool,
    #[serde(default)]
    max_parts: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct OpRejectionJsonV1 {
    index: usize,
    kind: String,
    error: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct OpAppliedJsonV1 {
    index: usize,
    kind: String,
    diff: serde_json::Value,
}

fn component_object_id_for_name(name: &str) -> u128 {
    builtin_object_id(&format!("gravimera/gen3d/component/{}", name))
}

fn parse_uuid_u128(field: &str, raw: &str) -> Result<u128, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(format!("Missing {field}"));
    }
    let uuid = Uuid::parse_str(raw).map_err(|err| format!("Invalid {field} UUID: {err}"))?;
    Ok(uuid.as_u128())
}

fn parse_vec3(field: &str, v: [f32; 3]) -> Result<Vec3, String> {
    let out = Vec3::new(v[0], v[1], v[2]);
    if !out.is_finite() {
        return Err(format!("{field} must be finite"));
    }
    Ok(out)
}

fn parse_quat(field: &str, v: [f32; 4]) -> Result<Quat, String> {
    let q = Quat::from_xyzw(v[0], v[1], v[2], v[3]).normalize();
    if !q.is_finite() {
        return Err(format!("{field} must be finite"));
    }
    Ok(q)
}

fn quat_from_forward_up_strict(forward: Vec3, up: Vec3) -> Result<Quat, String> {
    const EPS: f32 = 1e-5;
    if !forward.is_finite() || !up.is_finite() {
        return Err("rotation basis must be finite".into());
    }
    if forward.length_squared() < EPS * EPS {
        return Err("rotation.forward is too small".into());
    }
    if up.length_squared() < EPS * EPS {
        return Err("rotation.up is too small".into());
    }

    let f = forward.normalize();
    let u = up.normalize();
    let r = u.cross(f);
    if !r.is_finite() || r.length_squared() < EPS * EPS {
        return Err("rotation basis is degenerate (forward/up are collinear)".into());
    }
    let r = r.normalize();
    let u2 = f.cross(r);
    if !u2.is_finite() || u2.length_squared() < EPS * EPS {
        return Err("rotation basis is degenerate after orthonormalization".into());
    }
    let u2 = u2.normalize();
    let mat = Mat3::from_cols(r, u2, f);
    let q = Quat::from_mat3(&mat).normalize();
    if !q.is_finite() {
        return Err("rotation basis resolved to a non-finite quaternion".into());
    }
    Ok(q)
}

fn apply_transform_delta(
    target: &mut Transform,
    set: &TransformDeltaJsonV1,
    allow_scale: bool,
    context: &str,
) -> Result<serde_json::Value, String> {
    let mut diff = serde_json::Map::new();

    if let Some(pos) = set.pos {
        let before = target.translation;
        let after = parse_vec3(&format!("{context}.pos"), pos)?;
        target.translation = after;
        diff.insert(
            "pos".into(),
            serde_json::json!({
                "before": [before.x, before.y, before.z],
                "after": [after.x, after.y, after.z],
            }),
        );
    }

    if let Some(scale) = set.scale {
        if !allow_scale {
            return Err(format!("{context}.scale is not allowed for this op"));
        }
        let before = target.scale;
        let after = parse_vec3(&format!("{context}.scale"), scale)?;
        target.scale = after;
        diff.insert(
            "scale".into(),
            serde_json::json!({
                "before": [before.x, before.y, before.z],
                "after": [after.x, after.y, after.z],
            }),
        );
    } else if !allow_scale && set.scale.is_some() {
        return Err(format!("{context}.scale is not allowed for this op"));
    }

    let has_quat = set.rot_quat_xyzw.is_some();
    let has_basis = set.forward.is_some() || set.up.is_some();
    if has_quat && has_basis {
        return Err(format!(
            "{context} rotation must use either rot_quat_xyzw or forward+up (not both)"
        ));
    }
    if has_basis {
        let Some(fwd) = set.forward else {
            return Err(format!(
                "{context}.forward is required when using forward+up"
            ));
        };
        let Some(up) = set.up else {
            return Err(format!("{context}.up is required when using forward+up"));
        };
        let before = target.rotation;
        let fwd = parse_vec3(&format!("{context}.forward"), fwd)?;
        let up = parse_vec3(&format!("{context}.up"), up)?;
        let after = quat_from_forward_up_strict(fwd, up)?;
        target.rotation = after;
        let b = before.normalize();
        diff.insert(
            "rot_quat_xyzw".into(),
            serde_json::json!({
                "before": [b.x, b.y, b.z, b.w],
                "after": [after.x, after.y, after.z, after.w],
            }),
        );
    } else if let Some(q) = set.rot_quat_xyzw {
        let before = target.rotation;
        let after = parse_quat(&format!("{context}.rot_quat_xyzw"), q)?;
        target.rotation = after;
        let b = before.normalize();
        diff.insert(
            "rot_quat_xyzw".into(),
            serde_json::json!({
                "before": [b.x, b.y, b.z, b.w],
                "after": [after.x, after.y, after.z, after.w],
            }),
        );
    }

    Ok(serde_json::Value::Object(diff))
}

fn mesh_key_from_str(raw: &str) -> Option<MeshKey> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cuboid" | "cube" | "unit_cube" => Some(MeshKey::UnitCube),
        "cylinder" | "unit_cylinder" => Some(MeshKey::UnitCylinder),
        "cone" | "unit_cone" => Some(MeshKey::UnitCone),
        "sphere" | "unit_sphere" => Some(MeshKey::UnitSphere),
        "capsule" | "unit_capsule" => Some(MeshKey::UnitCapsule),
        "conical_frustum" | "unit_conical_frustum" => Some(MeshKey::UnitConicalFrustum),
        "torus" | "unit_torus" => Some(MeshKey::UnitTorus),
        _ => None,
    }
}

fn primitive_visual_from_spec(spec: &PrimitiveSpecJsonV1) -> Result<PrimitiveVisualDef, String> {
    let mesh = mesh_key_from_str(spec.mesh.as_str())
        .ok_or_else(|| format!("Unknown primitive.mesh `{}`", spec.mesh.trim()))?;
    let params = match spec.params.as_ref() {
        None => None,
        Some(PrimitiveParamsJsonV1::Capsule(p)) => Some(PrimitiveParams::Capsule {
            radius: p.radius.abs().max(0.001),
            half_length: p.half_length.abs().max(0.001),
        }),
        Some(PrimitiveParamsJsonV1::ConicalFrustum(p)) => Some(PrimitiveParams::ConicalFrustum {
            radius_top: p.top_radius.abs().max(0.001),
            radius_bottom: p.bottom_radius.abs().max(0.001),
            height: p.height.abs().max(0.001),
        }),
        Some(PrimitiveParamsJsonV1::Torus(p)) => Some(PrimitiveParams::Torus {
            minor_radius: p.minor_radius.abs().max(0.001),
            major_radius: p.major_radius.abs().max(0.001),
        }),
    };

    match (mesh, params.as_ref()) {
        (MeshKey::UnitCapsule, Some(PrimitiveParams::Capsule { .. }))
        | (MeshKey::UnitConicalFrustum, Some(PrimitiveParams::ConicalFrustum { .. }))
        | (MeshKey::UnitTorus, Some(PrimitiveParams::Torus { .. })) => {}
        (MeshKey::UnitCapsule | MeshKey::UnitConicalFrustum | MeshKey::UnitTorus, None) => {
            return Err(format!(
                "primitive.params is required for mesh `{}`",
                spec.mesh.trim()
            ));
        }
        (
            MeshKey::UnitCube | MeshKey::UnitCylinder | MeshKey::UnitCone | MeshKey::UnitSphere,
            Some(_),
        ) => {
            return Err(format!(
                "primitive.params must be null/absent for mesh `{}`",
                spec.mesh.trim()
            ));
        }
        _ => {
            // Other meshes should not appear in Gen3D patch ops.
        }
    }

    let color = spec.color_rgba.unwrap_or([0.85, 0.87, 0.90, 1.0]);
    for (idx, c) in color.iter().enumerate() {
        if !c.is_finite() {
            return Err(format!("primitive.color_rgba[{idx}] must be finite"));
        }
    }
    let color = Color::srgba(
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
        color[3].clamp(0.0, 1.0),
    );

    Ok(PrimitiveVisualDef::Primitive {
        mesh,
        params,
        color,
        unlit: spec.unlit.unwrap_or(false),
    })
}

fn animation_driver_from_json(
    driver: AiAnimationDriverJsonV1,
) -> Result<PartAnimationDriver, String> {
    Ok(match driver {
        AiAnimationDriverJsonV1::Always => PartAnimationDriver::Always,
        AiAnimationDriverJsonV1::MovePhase => PartAnimationDriver::MovePhase,
        AiAnimationDriverJsonV1::MoveDistance => PartAnimationDriver::MoveDistance,
        AiAnimationDriverJsonV1::AttackTime => PartAnimationDriver::AttackTime,
        AiAnimationDriverJsonV1::ActionTime => PartAnimationDriver::ActionTime,
        AiAnimationDriverJsonV1::Unknown => {
            return Err("animation driver must not be unknown".into())
        }
    })
}

fn transform_from_anim_delta(delta: &AiAnimationDeltaTransformJsonV1) -> Result<Transform, String> {
    let mut out = Transform::IDENTITY;
    if let Some(pos) = delta.pos {
        let pos = parse_vec3("delta.pos", pos)?;
        out.translation = pos;
    }
    if let Some(q) = delta.rot_quat_xyzw {
        let q = parse_quat("delta.rot_quat_xyzw", q)?;
        out.rotation = q;
    }
    if let Some(scale) = delta.scale {
        let scale = parse_vec3("delta.scale", scale)?;
        out.scale = scale;
    }
    Ok(out)
}

fn animation_slot_from_spec(
    channel: &str,
    slot: &AnimationSlotSpecJsonV1,
) -> Result<PartAnimationSlot, String> {
    let channel = channel.trim();
    if channel.is_empty() {
        return Err("channel must be non-empty".into());
    }
    if channel.len() > 64 {
        return Err("channel is too long".into());
    }

    let driver = animation_driver_from_json(slot.driver)?;
    if !slot.speed_scale.is_finite() {
        return Err("slot.speed_scale must be finite".into());
    }
    if !slot.time_offset_units.is_finite() {
        return Err("slot.time_offset_units must be finite".into());
    }

    let clip = match &slot.clip {
        AiAnimationClipJsonV1::Loop {
            duration_units,
            keyframes,
        } => PartAnimationDef::Loop {
            duration_secs: duration_units.abs().max(1e-3),
            keyframes: keyframes
                .iter()
                .map(|kf| {
                    if !kf.t_units.is_finite() {
                        return Err("keyframe.t_units must be finite".into());
                    }
                    Ok(PartAnimationKeyframeDef {
                        time_secs: kf.t_units,
                        delta: transform_from_anim_delta(&kf.delta)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        },
        AiAnimationClipJsonV1::Once {
            duration_units,
            keyframes,
        } => PartAnimationDef::Once {
            duration_secs: duration_units.abs().max(1e-3),
            keyframes: keyframes
                .iter()
                .map(|kf| {
                    if !kf.t_units.is_finite() {
                        return Err("keyframe.t_units must be finite".into());
                    }
                    Ok(PartAnimationKeyframeDef {
                        time_secs: kf.t_units,
                        delta: transform_from_anim_delta(&kf.delta)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        },
        AiAnimationClipJsonV1::PingPong {
            duration_units,
            keyframes,
        } => PartAnimationDef::PingPong {
            duration_secs: duration_units.abs().max(1e-3),
            keyframes: keyframes
                .iter()
                .map(|kf| {
                    if !kf.t_units.is_finite() {
                        return Err("keyframe.t_units must be finite".into());
                    }
                    Ok(PartAnimationKeyframeDef {
                        time_secs: kf.t_units,
                        delta: transform_from_anim_delta(&kf.delta)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        },
        AiAnimationClipJsonV1::Spin {
            axis,
            radians_per_unit,
            axis_space,
        } => {
            if !radians_per_unit.is_finite() {
                return Err("clip.radians_per_unit must be finite".into());
            }
            let axis = parse_vec3("clip.axis", *axis)?;
            PartAnimationDef::Spin {
                axis,
                radians_per_unit: *radians_per_unit,
                axis_space: axis_space.to_space(),
            }
        }
    };

    Ok(PartAnimationSlot {
        channel: channel.to_string().into(),
        family: match slot.family {
            super::schema::AiAnimationFamilyJsonV1::Overlay => PartAnimationFamily::Overlay,
            _ => PartAnimationFamily::Base,
        },
        spec: PartAnimationSpec {
            driver,
            speed_scale: slot.speed_scale,
            time_offset_units: slot.time_offset_units,
            basis: Transform::IDENTITY,
            clip,
        },
    })
}

fn find_component_def_mut<'a>(
    draft: &'a mut Gen3dDraft,
    component: &str,
) -> Result<&'a mut ObjectDef, String> {
    let name = component.trim();
    if name.is_empty() {
        return Err("component must be non-empty".into());
    }
    let object_id = component_object_id_for_name(name);
    draft
        .defs
        .iter_mut()
        .find(|d| d.object_id == object_id)
        .ok_or_else(|| {
            format!(
                "Component `{}` not found in draft.defs (object_id_uuid={})",
                name,
                Uuid::from_u128(object_id)
            )
        })
}

fn find_planned_component_mut<'a>(
    planned: &'a mut [Gen3dPlannedComponent],
    component: &str,
) -> Result<&'a mut Gen3dPlannedComponent, String> {
    let name = component.trim();
    if name.is_empty() {
        return Err("component must be non-empty".into());
    }
    planned
        .iter_mut()
        .find(|c| c.name.as_str() == name)
        .ok_or_else(|| format!("Unknown component `{}`", name))
}

fn find_root_component_index(planned: &[Gen3dPlannedComponent]) -> Option<usize> {
    planned.iter().position(|c| c.attach_to.is_none())
}

#[derive(Clone, Debug, Default)]
struct ApplyWorkState {
    needs_resolve_transforms: bool,
    needs_sync_attachments: bool,
    primitive_parts_added: u32,
    primitive_parts_removed: u32,
    primitive_parts_updated: u32,
    articulation_nodes_upserted: u32,
    articulation_nodes_removed: u32,
    articulation_nodes_rebound: u32,
    anchors_updated: u32,
    attachments_updated: u32,
    animation_slots_upserted: u32,
    animation_slots_scaled: u32,
    animation_slots_removed: u32,
    changed_component_ids: std::collections::BTreeSet<u128>,
}

fn mark_changed_component(state: &mut ApplyWorkState, component_name: &str) {
    let id = component_object_id_for_name(component_name);
    state.changed_component_ids.insert(id);
}

fn primitive_part_ids_for_def(def: &ObjectDef) -> std::collections::BTreeSet<u128> {
    let mut ids = std::collections::BTreeSet::<u128>::new();
    for part in def.parts.iter() {
        if matches!(part.kind, ObjectPartKind::Primitive { .. }) {
            if let Some(part_id) = part.part_id {
                ids.insert(part_id);
            }
        }
    }
    ids
}

fn parse_bound_part_ids(field: &str, raw_ids: &[String]) -> Result<Vec<u128>, String> {
    let mut out: Vec<u128> = Vec::with_capacity(raw_ids.len());
    for (idx, raw) in raw_ids.iter().enumerate() {
        out.push(parse_uuid_u128(&format!("{field}[{idx}]"), raw.as_str())?);
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(format!("{field} must contain at least one part id"));
    }
    Ok(out)
}

fn validate_component_articulation_nodes(
    component_name: &str,
    nodes: &mut [Gen3dPlannedArticulationNode],
    available_part_ids: &std::collections::BTreeSet<u128>,
) -> Result<(), String> {
    let mut node_index_by_id: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut part_owner: std::collections::HashMap<u128, String> = std::collections::HashMap::new();

    for (idx, node) in nodes.iter_mut().enumerate() {
        node.node_id = node.node_id.trim().to_string();
        if node.node_id.is_empty() {
            return Err(format!(
                "Articulation node on component `{component_name}` is missing required `node_id`."
            ));
        }
        node.parent_node_id = match node.parent_node_id.take() {
            Some(value) => {
                let value = value.trim().to_string();
                (!value.is_empty()).then_some(value)
            }
            None => None,
        };

        if !node.transform.translation.is_finite()
            || !node.transform.rotation.is_finite()
            || !node.transform.scale.is_finite()
        {
            return Err(format!(
                "Articulation node `{}` on component `{component_name}` has a non-finite transform.",
                node.node_id
            ));
        }

        if node
            .parent_node_id
            .as_deref()
            .is_some_and(|parent| parent == node.node_id)
        {
            return Err(format!(
                "Articulation node `{}` on component `{component_name}` cannot parent itself.",
                node.node_id
            ));
        }

        node.bound_part_ids.sort_unstable();
        node.bound_part_ids.dedup();
        if node.bound_part_ids.is_empty() {
            return Err(format!(
                "Articulation node `{}` on component `{component_name}` must bind at least one primitive part.",
                node.node_id
            ));
        }

        for &part_id in node.bound_part_ids.iter() {
            if !available_part_ids.contains(&part_id) {
                return Err(format!(
                    "Articulation node `{}` on component `{component_name}` references missing primitive part {}.",
                    node.node_id,
                    Uuid::from_u128(part_id)
                ));
            }
            if let Some(other_node_id) = part_owner.insert(part_id, node.node_id.clone()) {
                return Err(format!(
                    "Articulation node `{}` on component `{component_name}` reuses primitive part {} already bound by articulation node `{}`.",
                    node.node_id,
                    Uuid::from_u128(part_id),
                    other_node_id
                ));
            }
        }

        if node_index_by_id.insert(node.node_id.clone(), idx).is_some() {
            return Err(format!(
                "Component `{component_name}` contains duplicate articulation node id `{}`.",
                node.node_id
            ));
        }
    }

    let mut visiting = vec![false; nodes.len()];
    let mut visited = vec![false; nodes.len()];

    fn dfs(
        idx: usize,
        component_name: &str,
        nodes: &[Gen3dPlannedArticulationNode],
        node_index_by_id: &std::collections::HashMap<String, usize>,
        visiting: &mut [bool],
        visited: &mut [bool],
    ) -> Result<(), String> {
        if visited[idx] {
            return Ok(());
        }
        if visiting[idx] {
            return Err(format!(
                "Component `{component_name}` contains an articulation-node parent cycle involving `{}`.",
                nodes[idx].node_id
            ));
        }
        visiting[idx] = true;
        if let Some(parent_id) = nodes[idx].parent_node_id.as_ref() {
            let Some(&parent_idx) = node_index_by_id.get(parent_id) else {
                return Err(format!(
                    "Articulation node `{}` on component `{component_name}` references missing parent `{}`.",
                    nodes[idx].node_id,
                    parent_id
                ));
            };
            dfs(
                parent_idx,
                component_name,
                nodes,
                node_index_by_id,
                visiting,
                visited,
            )?;
        }
        visiting[idx] = false;
        visited[idx] = true;
        Ok(())
    }

    for idx in 0..nodes.len() {
        dfs(
            idx,
            component_name,
            nodes,
            &node_index_by_id,
            &mut visiting,
            &mut visited,
        )?;
    }

    Ok(())
}

fn apply_one_op(
    op_index: usize,
    op: &DraftOpJsonV1,
    planned: &mut [Gen3dPlannedComponent],
    draft: &mut Gen3dDraft,
    state: &mut ApplyWorkState,
) -> Result<OpAppliedJsonV1, OpRejectionJsonV1> {
    let kind = match op {
        DraftOpJsonV1::SetAnchorTransform { .. } => "set_anchor_transform",
        DraftOpJsonV1::SetAttachmentOffset { .. } => "set_attachment_offset",
        DraftOpJsonV1::SetAttachmentJoint { .. } => "set_attachment_joint",
        DraftOpJsonV1::UpdatePrimitivePart { .. } => "update_primitive_part",
        DraftOpJsonV1::AddPrimitivePart { .. } => "add_primitive_part",
        DraftOpJsonV1::RemovePrimitivePart { .. } => "remove_primitive_part",
        DraftOpJsonV1::UpsertArticulationNode { .. } => "upsert_articulation_node",
        DraftOpJsonV1::RemoveArticulationNode { .. } => "remove_articulation_node",
        DraftOpJsonV1::RebindArticulationNodeParts { .. } => "rebind_articulation_node_parts",
        DraftOpJsonV1::UpsertAnimationSlot { .. } => "upsert_animation_slot",
        DraftOpJsonV1::ScaleAnimationSlotRotation { .. } => "scale_animation_slot_rotation",
        DraftOpJsonV1::RemoveAnimationSlot { .. } => "remove_animation_slot",
    }
    .to_string();

    let reject = |error: String| OpRejectionJsonV1 {
        index: op_index,
        kind: kind.clone(),
        error,
    };

    let diff = match op {
        DraftOpJsonV1::SetAnchorTransform {
            component,
            anchor,
            set,
        } => {
            let component_name = component.trim();
            let anchor_name = anchor.trim();
            if anchor_name.is_empty() {
                return Err(reject("anchor must be non-empty".into()));
            }
            if anchor_name == "origin" {
                return Err(reject("anchor `origin` is not editable".into()));
            }

            let def = find_component_def_mut(draft, component_name).map_err(reject)?;
            let Some(anchor_def) = def
                .anchors
                .iter_mut()
                .find(|a| a.name.as_ref() == anchor_name)
            else {
                return Err(reject(format!(
                    "Anchor `{}` not found on component `{}`",
                    anchor_name, component_name
                )));
            };

            let diff = apply_transform_delta(&mut anchor_def.transform, set, false, "set")
                .map_err(reject)?;

            if !anchor_def.transform.translation.is_finite()
                || !anchor_def.transform.rotation.is_finite()
            {
                return Err(reject("anchor transform became non-finite".into()));
            }

            // Keep planned anchors in sync with the def we just edited.
            if let Ok(planned_comp) = find_planned_component_mut(planned, component_name) {
                planned_comp.anchors = def.anchors.clone();
            }

            state.needs_resolve_transforms = true;
            state.anchors_updated = state.anchors_updated.saturating_add(1);
            mark_changed_component(state, component_name);
            diff
        }
        DraftOpJsonV1::SetAttachmentOffset {
            child_component,
            set,
        } => {
            let child_name = child_component.trim();
            let planned_child = find_planned_component_mut(planned, child_name).map_err(reject)?;
            let Some(att) = planned_child.attach_to.as_mut() else {
                return Err(reject(format!(
                    "Component `{}` has no attach_to (cannot edit attachment offset on root)",
                    child_name
                )));
            };

            let old_offset = att.offset;
            let diff = apply_transform_delta(&mut att.offset, set, true, "set").map_err(reject)?;
            if !att.offset.translation.is_finite()
                || !att.offset.rotation.is_finite()
                || !att.offset.scale.is_finite()
            {
                return Err(reject("attachment offset became non-finite".into()));
            }
            super::attachment_motion_basis::normalize_attachment_motion(
                &mut att.fallback_basis,
                &mut att.animations,
            );
            super::attachment_motion_basis::rebase_bases_for_offset_change(
                old_offset,
                att.offset,
                &mut att.fallback_basis,
                &mut att.animations,
            );

            state.needs_resolve_transforms = true;
            state.needs_sync_attachments = true;
            state.attachments_updated = state.attachments_updated.saturating_add(1);
            mark_changed_component(state, child_name);
            mark_changed_component(state, att.parent.as_str());
            diff
        }
        DraftOpJsonV1::SetAttachmentJoint {
            child_component,
            set_joint,
        } => {
            let child_name = child_component.trim();
            let planned_child = find_planned_component_mut(planned, child_name).map_err(reject)?;
            let Some(att) = planned_child.attach_to.as_mut() else {
                return Err(reject(format!(
                    "Component `{}` has no attach_to (cannot edit joint on root)",
                    child_name
                )));
            };

            if let Some(joint) = set_joint.as_ref() {
                if joint.kind == AiJointKindJson::Hinge {
                    let Some(axis) = joint.axis_join else {
                        return Err(reject(
                            "Hinge joint requires axis_join (set_joint.axis_join)".into(),
                        ));
                    };
                    let v = Vec3::new(axis[0], axis[1], axis[2]);
                    if !v.is_finite() || v.length_squared() <= 1e-6 {
                        return Err(reject(
                            "Hinge joint axis_join must be finite and non-zero".into(),
                        ));
                    }
                }
            }

            let before = att.joint.clone();
            att.joint = set_joint.clone();

            state.attachments_updated = state.attachments_updated.saturating_add(1);
            mark_changed_component(state, child_name);
            mark_changed_component(state, att.parent.as_str());

            serde_json::json!({
                "before": before,
                "after": att.joint,
            })
        }
        DraftOpJsonV1::UpdatePrimitivePart {
            component,
            part_id_uuid,
            set_transform,
            set_primitive,
            set_render_priority,
        } => {
            let component_name = component.trim();
            let part_id = parse_uuid_u128("part_id_uuid", part_id_uuid.as_str()).map_err(reject)?;

            let def = find_component_def_mut(draft, component_name).map_err(reject)?;
            let mut matches: Vec<usize> = Vec::new();
            for (idx, part) in def.parts.iter().enumerate() {
                if matches!(part.kind, ObjectPartKind::Primitive { .. })
                    && part.part_id == Some(part_id)
                {
                    matches.push(idx);
                }
            }
            if matches.is_empty() {
                return Err(reject(format!(
                    "Primitive part not found: component `{}` part_id_uuid={}",
                    component_name,
                    Uuid::from_u128(part_id),
                )));
            }
            if matches.len() > 1 {
                return Err(reject(format!(
                    "Ambiguous primitive part_id_uuid={} on component `{}` ({} matches)",
                    Uuid::from_u128(part_id),
                    component_name,
                    matches.len()
                )));
            }
            let part_idx = matches[0];
            let part = def
                .parts
                .get_mut(part_idx)
                .ok_or_else(|| reject("Internal error: part index out of range".into()))?;

            let mut diff = serde_json::Map::new();

            if let Some(set) = set_transform.as_ref() {
                let d = apply_transform_delta(&mut part.transform, set, true, "set_transform")
                    .map_err(reject)?;
                diff.insert("transform".into(), d);
            }
            if let Some(spec) = set_primitive.as_ref() {
                let before = &part.kind;
                let new_visual = primitive_visual_from_spec(spec).map_err(reject)?;
                match before {
                    ObjectPartKind::Primitive { .. } => {}
                    _ => return Err(reject("Target part is not a primitive".into())),
                }
                part.kind = ObjectPartKind::Primitive {
                    primitive: new_visual,
                };
                diff.insert("primitive".into(), serde_json::json!({"updated": true}));
            }
            if let Some(rp) = set_render_priority {
                let before = part.render_priority;
                part.render_priority = Some(*rp);
                diff.insert(
                    "render_priority".into(),
                    serde_json::json!({"before": before, "after": part.render_priority}),
                );
            }

            if !part.transform.translation.is_finite()
                || !part.transform.rotation.is_finite()
                || !part.transform.scale.is_finite()
            {
                return Err(reject("part transform became non-finite".into()));
            }

            // Keep component size and planned actual_size consistent.
            def.size = convert::size_from_primitive_parts(&def.parts);
            if let Ok(planned_comp) = find_planned_component_mut(planned, component_name) {
                planned_comp.actual_size = Some(def.size);
            }

            state.primitive_parts_updated = state.primitive_parts_updated.saturating_add(1);
            mark_changed_component(state, component_name);
            serde_json::Value::Object(diff)
        }
        DraftOpJsonV1::AddPrimitivePart {
            component,
            part_id_uuid,
            primitive,
            transform,
            render_priority,
        } => {
            let component_name = component.trim();
            let part_id = parse_uuid_u128("part_id_uuid", part_id_uuid.as_str()).map_err(reject)?;

            let def = find_component_def_mut(draft, component_name).map_err(reject)?;
            let existing = def.parts.iter().any(|p| {
                matches!(p.kind, ObjectPartKind::Primitive { .. }) && p.part_id == Some(part_id)
            });
            if existing {
                return Err(reject(format!(
                    "Primitive part_id_uuid already exists on component `{}`: {}",
                    component_name,
                    Uuid::from_u128(part_id)
                )));
            }

            let primitive = primitive_visual_from_spec(primitive).map_err(reject)?;
            let mut t = Transform::IDENTITY;
            let d = apply_transform_delta(&mut t, transform, true, "transform").map_err(reject)?;

            let mut part = ObjectPartDef::primitive(primitive, t).with_part_id(part_id);
            if let Some(rp) = render_priority {
                part.render_priority = Some(*rp);
            }

            let primitive_count = def
                .parts
                .iter()
                .filter(|p| matches!(p.kind, ObjectPartKind::Primitive { .. }))
                .count();
            if primitive_count.saturating_add(1) > GEN3D_MAX_PARTS {
                return Err(reject(format!(
                    "Component `{}` would exceed max primitive parts ({GEN3D_MAX_PARTS})",
                    component_name
                )));
            }

            def.parts.push(part);
            def.size = convert::size_from_primitive_parts(&def.parts);
            if let Ok(planned_comp) = find_planned_component_mut(planned, component_name) {
                planned_comp.actual_size = Some(def.size);
            }

            state.primitive_parts_added = state.primitive_parts_added.saturating_add(1);
            mark_changed_component(state, component_name);
            serde_json::json!({"transform": d})
        }
        DraftOpJsonV1::RemovePrimitivePart {
            component,
            part_id_uuid,
        } => {
            let component_name = component.trim();
            let part_id = parse_uuid_u128("part_id_uuid", part_id_uuid.as_str()).map_err(reject)?;

            let planned_comp =
                find_planned_component_mut(planned, component_name).map_err(reject)?;
            if let Some(node) = planned_comp
                .articulation_nodes
                .iter()
                .find(|node| node.bound_part_ids.contains(&part_id))
            {
                return Err(reject(format!(
                    "Cannot remove primitive part {} from component `{}` because articulation node `{}` still binds it. Rebind or remove that articulation node first.",
                    Uuid::from_u128(part_id),
                    component_name,
                    node.node_id
                )));
            }

            let def = find_component_def_mut(draft, component_name).map_err(reject)?;
            let mut match_index: Option<usize> = None;
            let mut matches = 0usize;
            for (idx, part) in def.parts.iter().enumerate() {
                if matches!(part.kind, ObjectPartKind::Primitive { .. })
                    && part.part_id == Some(part_id)
                {
                    matches += 1;
                    match_index = Some(idx);
                }
            }
            if matches == 0 {
                return Err(reject(format!(
                    "Primitive part not found: component `{}` part_id_uuid={}",
                    component_name,
                    Uuid::from_u128(part_id)
                )));
            }
            if matches > 1 {
                return Err(reject(format!(
                    "Ambiguous primitive part_id_uuid={} on component `{}` ({} matches)",
                    Uuid::from_u128(part_id),
                    component_name,
                    matches
                )));
            }
            let idx = match_index.unwrap_or(0);
            def.parts.remove(idx);
            def.size = convert::size_from_primitive_parts(&def.parts);
            if let Ok(planned_comp) = find_planned_component_mut(planned, component_name) {
                planned_comp.actual_size = Some(def.size);
            }

            state.primitive_parts_removed = state.primitive_parts_removed.saturating_add(1);
            mark_changed_component(state, component_name);
            serde_json::json!({"removed": true})
        }
        DraftOpJsonV1::UpsertArticulationNode {
            component,
            node_id,
            parent_node_id,
            set_transform,
            bound_part_id_uuids,
        } => {
            let component_name = component.trim();
            let node_id = node_id.trim();
            if node_id.is_empty() {
                return Err(reject("node_id must be non-empty".into()));
            }

            let available_part_ids = {
                let def = find_component_def_mut(draft, component_name).map_err(reject)?;
                primitive_part_ids_for_def(def)
            };
            let bound_part_ids =
                parse_bound_part_ids("bound_part_id_uuids", bound_part_id_uuids.as_slice())
                    .map_err(reject)?;
            let parent_node_id = parent_node_id.clone().into_option_trimmed();

            let planned_comp =
                find_planned_component_mut(planned, component_name).map_err(reject)?;
            let existing_idx = planned_comp
                .articulation_nodes
                .iter()
                .position(|node| node.node_id == node_id);

            let mut next_transform = existing_idx
                .and_then(|idx| {
                    planned_comp
                        .articulation_nodes
                        .get(idx)
                        .map(|node| node.transform)
                })
                .unwrap_or(Transform::IDENTITY);
            let transform_diff =
                apply_transform_delta(&mut next_transform, set_transform, false, "set_transform")
                    .map_err(reject)?;

            let mut diff = serde_json::Map::new();
            diff.insert(
                "node_id".into(),
                serde_json::Value::String(node_id.to_string()),
            );
            diff.insert("transform".into(), transform_diff);

            let parent_after = parent_node_id.clone();
            if let Some(idx) = existing_idx {
                let node = planned_comp
                    .articulation_nodes
                    .get_mut(idx)
                    .ok_or_else(|| {
                        reject("Internal error: articulation node index out of range".into())
                    })?;
                let parent_before = node.parent_node_id.clone();
                let bindings_before: Vec<String> = node
                    .bound_part_ids
                    .iter()
                    .map(|id| Uuid::from_u128(*id).to_string())
                    .collect();
                node.parent_node_id = parent_after.clone();
                node.transform = next_transform;
                node.bound_part_ids = bound_part_ids;
                diff.insert("updated".into(), serde_json::Value::Bool(true));
                diff.insert(
                    "parent_node_id".into(),
                    serde_json::json!({
                        "before": parent_before,
                        "after": parent_after,
                    }),
                );
                diff.insert(
                    "bound_part_id_uuids".into(),
                    serde_json::json!({
                        "before": bindings_before,
                        "after": node
                            .bound_part_ids
                            .iter()
                            .map(|id| Uuid::from_u128(*id).to_string())
                            .collect::<Vec<_>>(),
                    }),
                );
            } else {
                planned_comp
                    .articulation_nodes
                    .push(Gen3dPlannedArticulationNode {
                        node_id: node_id.to_string(),
                        parent_node_id: parent_after.clone(),
                        transform: next_transform,
                        bound_part_ids,
                    });
                diff.insert("added".into(), serde_json::Value::Bool(true));
                diff.insert(
                    "parent_node_id".into(),
                    serde_json::json!({"after": parent_after}),
                );
                diff.insert(
                    "bound_part_id_uuids".into(),
                    serde_json::json!({
                        "after": planned_comp
                            .articulation_nodes
                            .last()
                            .map(|node| {
                                node.bound_part_ids
                                    .iter()
                                    .map(|id| Uuid::from_u128(*id).to_string())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default(),
                    }),
                );
            }

            validate_component_articulation_nodes(
                component_name,
                planned_comp.articulation_nodes.as_mut_slice(),
                &available_part_ids,
            )
            .map_err(reject)?;

            state.articulation_nodes_upserted = state.articulation_nodes_upserted.saturating_add(1);
            mark_changed_component(state, component_name);
            serde_json::Value::Object(diff)
        }
        DraftOpJsonV1::RemoveArticulationNode { component, node_id } => {
            let component_name = component.trim();
            let node_id = node_id.trim();
            if node_id.is_empty() {
                return Err(reject("node_id must be non-empty".into()));
            }

            let planned_comp =
                find_planned_component_mut(planned, component_name).map_err(reject)?;
            let Some(idx) = planned_comp
                .articulation_nodes
                .iter()
                .position(|node| node.node_id == node_id)
            else {
                return Err(reject(format!(
                    "Articulation node `{}` not found on component `{}`",
                    node_id, component_name
                )));
            };

            if let Some(child) = planned_comp
                .articulation_nodes
                .iter()
                .find(|node| node.parent_node_id.as_deref() == Some(node_id))
            {
                return Err(reject(format!(
                    "Cannot remove articulation node `{}` on component `{}` because child articulation node `{}` still parents to it.",
                    node_id,
                    component_name,
                    child.node_id
                )));
            }

            let removed = planned_comp.articulation_nodes.remove(idx);
            state.articulation_nodes_removed = state.articulation_nodes_removed.saturating_add(1);
            mark_changed_component(state, component_name);
            serde_json::json!({
                "removed": true,
                "node_id": removed.node_id,
                "bound_part_id_uuids": removed
                    .bound_part_ids
                    .iter()
                    .map(|id| Uuid::from_u128(*id).to_string())
                    .collect::<Vec<_>>(),
            })
        }
        DraftOpJsonV1::RebindArticulationNodeParts {
            component,
            node_id,
            bound_part_id_uuids,
        } => {
            let component_name = component.trim();
            let node_id = node_id.trim();
            if node_id.is_empty() {
                return Err(reject("node_id must be non-empty".into()));
            }

            let available_part_ids = {
                let def = find_component_def_mut(draft, component_name).map_err(reject)?;
                primitive_part_ids_for_def(def)
            };
            let bound_part_ids =
                parse_bound_part_ids("bound_part_id_uuids", bound_part_id_uuids.as_slice())
                    .map_err(reject)?;

            let planned_comp =
                find_planned_component_mut(planned, component_name).map_err(reject)?;
            let Some(idx) = planned_comp
                .articulation_nodes
                .iter()
                .position(|node| node.node_id == node_id)
            else {
                return Err(reject(format!(
                    "Articulation node `{}` not found on component `{}`",
                    node_id, component_name
                )));
            };

            let before = planned_comp.articulation_nodes[idx]
                .bound_part_ids
                .iter()
                .map(|id| Uuid::from_u128(*id).to_string())
                .collect::<Vec<_>>();
            planned_comp.articulation_nodes[idx].bound_part_ids = bound_part_ids;
            validate_component_articulation_nodes(
                component_name,
                planned_comp.articulation_nodes.as_mut_slice(),
                &available_part_ids,
            )
            .map_err(reject)?;

            state.articulation_nodes_rebound = state.articulation_nodes_rebound.saturating_add(1);
            mark_changed_component(state, component_name);
            serde_json::json!({
                "rebound": true,
                "node_id": node_id,
                "bound_part_id_uuids": {
                    "before": before,
                    "after": planned_comp.articulation_nodes[idx]
                        .bound_part_ids
                        .iter()
                        .map(|id| Uuid::from_u128(*id).to_string())
                        .collect::<Vec<_>>(),
                }
            })
        }
        DraftOpJsonV1::UpsertAnimationSlot {
            child_component,
            channel,
            slot,
        } => {
            let child_name = child_component.trim();
            let channel = channel.trim().to_string();
            if channel == LEGACY_INTERNAL_BASE_CHANNEL {
                return Err(reject(format!(
                    "Animation channel `{}` is reserved.",
                    LEGACY_INTERNAL_BASE_CHANNEL
                )));
            }
            let planned_child = find_planned_component_mut(planned, child_name).map_err(reject)?;

            let replacement = animation_slot_from_spec(&channel, slot).map_err(reject)?;

            let mut diff = serde_json::Map::new();

            let affected: usize = if let Some(att) = planned_child.attach_to.as_mut() {
                let indices: Vec<usize> = att
                    .animations
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.channel.as_ref() == channel)
                    .map(|(idx, _)| idx)
                    .collect();
                let affected = if indices.is_empty() {
                    att.animations.push(replacement);
                    diff.insert("added".into(), serde_json::Value::Bool(true));
                    1
                } else {
                    let affected = indices.len();
                    for idx in indices {
                        att.animations[idx] = replacement.clone();
                    }
                    diff.insert("updated".into(), serde_json::Value::Bool(true));
                    diff.insert(
                        "updated_count".into(),
                        serde_json::Value::Number(affected.into()),
                    );
                    affected
                };
                super::attachment_motion_basis::normalize_attachment_motion(
                    &mut att.fallback_basis,
                    &mut att.animations,
                );
                mark_changed_component(state, att.parent.as_str());
                affected
            } else {
                let indices: Vec<usize> = planned_child
                    .root_animations
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.channel.as_ref() == channel)
                    .map(|(idx, _)| idx)
                    .collect();
                if indices.is_empty() {
                    planned_child.root_animations.push(replacement);
                    diff.insert("added".into(), serde_json::Value::Bool(true));
                    diff.insert("root".into(), serde_json::Value::Bool(true));
                    1
                } else {
                    let affected = indices.len();
                    for idx in indices {
                        planned_child.root_animations[idx] = replacement.clone();
                    }
                    diff.insert("updated".into(), serde_json::Value::Bool(true));
                    diff.insert(
                        "updated_count".into(),
                        serde_json::Value::Number(affected.into()),
                    );
                    diff.insert("root".into(), serde_json::Value::Bool(true));
                    affected
                }
            };

            state.needs_sync_attachments = true;
            state.animation_slots_upserted = state
                .animation_slots_upserted
                .saturating_add(affected.max(1) as u32);
            mark_changed_component(state, child_name);
            serde_json::Value::Object(diff)
        }
        DraftOpJsonV1::ScaleAnimationSlotRotation {
            child_component,
            channel,
            scale,
        } => {
            let child_name = child_component.trim();
            let channel = channel.trim();
            if channel.is_empty() {
                return Err(reject("channel must be non-empty".into()));
            }
            if channel == LEGACY_INTERNAL_BASE_CHANNEL {
                return Err(reject(format!(
                    "Animation channel `{}` is reserved.",
                    LEGACY_INTERNAL_BASE_CHANNEL
                )));
            }
            if !scale.is_finite() || *scale <= 0.0 || *scale > 10.0 {
                return Err(reject("scale must be finite and in (0, 10].".into()));
            }

            let planned_child = find_planned_component_mut(planned, child_name).map_err(reject)?;

            fn scale_delta_rotation(delta: &mut Transform, scale: f32) -> Result<(), String> {
                if !delta.rotation.is_finite() {
                    return Err("keyframe rotation is non-finite".into());
                }
                let q = delta.rotation.normalize();
                if !q.is_finite() {
                    return Err("keyframe rotation became non-finite after normalize".into());
                }
                let (axis, angle) = q.to_axis_angle();
                if !axis.is_finite() || !angle.is_finite() {
                    return Err("failed to compute axis-angle for keyframe rotation".into());
                }
                let scaled = Quat::from_axis_angle(axis, angle * scale).normalize();
                if !scaled.is_finite() {
                    return Err("scaled keyframe rotation became non-finite".into());
                }
                delta.rotation = scaled;
                Ok(())
            }

            let mut scaled_keyframes: u32 = 0;
            let mut scaled_slots: u32 = 0;

            if let Some(att) = planned_child.attach_to.as_mut() {
                let indices: Vec<usize> = att
                    .animations
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.channel.as_ref() == channel)
                    .map(|(idx, _)| idx)
                    .collect();
                if indices.is_empty() {
                    return Err(reject(format!(
                        "Animation slot not found for channel `{}` on `{}`",
                        channel, child_name
                    )));
                }

                for idx in indices {
                    let slot = &mut att.animations[idx];
                    match &mut slot.spec.clip {
                        PartAnimationDef::Loop { keyframes, .. }
                        | PartAnimationDef::Once { keyframes, .. }
                        | PartAnimationDef::PingPong { keyframes, .. } => {
                            for k in keyframes.iter_mut() {
                                scale_delta_rotation(&mut k.delta, *scale).map_err(reject)?;
                                scaled_keyframes = scaled_keyframes.saturating_add(1);
                            }
                        }
                        PartAnimationDef::Spin {
                            radians_per_unit, ..
                        } => {
                            let next = *radians_per_unit * *scale;
                            if !next.is_finite() {
                                return Err(reject(
                                    "scaled spin radians_per_unit became non-finite".into(),
                                ));
                            }
                            *radians_per_unit = next;
                        }
                    }
                    scaled_slots = scaled_slots.saturating_add(1);
                }
                mark_changed_component(state, att.parent.as_str());
            } else {
                let indices: Vec<usize> = planned_child
                    .root_animations
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.channel.as_ref() == channel)
                    .map(|(idx, _)| idx)
                    .collect();
                if indices.is_empty() {
                    return Err(reject(format!(
                        "Animation slot not found for channel `{}` on `{}`",
                        channel, child_name
                    )));
                }

                for idx in indices {
                    let slot = &mut planned_child.root_animations[idx];
                    match &mut slot.spec.clip {
                        PartAnimationDef::Loop { keyframes, .. }
                        | PartAnimationDef::Once { keyframes, .. }
                        | PartAnimationDef::PingPong { keyframes, .. } => {
                            for k in keyframes.iter_mut() {
                                scale_delta_rotation(&mut k.delta, *scale).map_err(reject)?;
                                scaled_keyframes = scaled_keyframes.saturating_add(1);
                            }
                        }
                        PartAnimationDef::Spin {
                            radians_per_unit, ..
                        } => {
                            let next = *radians_per_unit * *scale;
                            if !next.is_finite() {
                                return Err(reject(
                                    "scaled spin radians_per_unit became non-finite".into(),
                                ));
                            }
                            *radians_per_unit = next;
                        }
                    }
                    scaled_slots = scaled_slots.saturating_add(1);
                }
            }

            state.needs_sync_attachments = true;
            state.animation_slots_scaled = state
                .animation_slots_scaled
                .saturating_add(scaled_slots.max(1));
            mark_changed_component(state, child_name);
            serde_json::json!({"scaled": true, "channel": channel, "scale": scale, "scaled_slots": scaled_slots, "scaled_keyframes": scaled_keyframes})
        }
        DraftOpJsonV1::RemoveAnimationSlot {
            child_component,
            channel,
        } => {
            let child_name = child_component.trim();
            let channel = channel.trim();
            if channel.is_empty() {
                return Err(reject("channel must be non-empty".into()));
            }
            if channel == LEGACY_INTERNAL_BASE_CHANNEL {
                return Err(reject(format!(
                    "Animation channel `{}` is reserved.",
                    LEGACY_INTERNAL_BASE_CHANNEL
                )));
            }
            let planned_child = find_planned_component_mut(planned, child_name).map_err(reject)?;
            let removed = if let Some(att) = planned_child.attach_to.as_mut() {
                let before = att.animations.len();
                att.animations.retain(|s| s.channel.as_ref() != channel);
                let removed = before.saturating_sub(att.animations.len());
                if removed == 0 {
                    return Err(reject(format!(
                        "Animation slot not found for channel `{}` on `{}`",
                        channel, child_name
                    )));
                }
                super::attachment_motion_basis::normalize_attachment_motion(
                    &mut att.fallback_basis,
                    &mut att.animations,
                );
                mark_changed_component(state, att.parent.as_str());
                removed
            } else {
                let before = planned_child.root_animations.len();
                planned_child
                    .root_animations
                    .retain(|s| s.channel.as_ref() != channel);
                let removed = before.saturating_sub(planned_child.root_animations.len());
                if removed == 0 {
                    return Err(reject(format!(
                        "Animation slot not found for channel `{}` on `{}`",
                        channel, child_name
                    )));
                }
                removed
            };

            state.needs_sync_attachments = true;
            state.animation_slots_removed = state
                .animation_slots_removed
                .saturating_add(removed.max(1) as u32);
            mark_changed_component(state, child_name);
            serde_json::json!({"removed": true, "removed_count": removed})
        }
    };

    Ok(OpAppliedJsonV1 {
        index: op_index,
        kind,
        diff,
    })
}

fn apply_ops_inner(
    args: &ApplyDraftOpsArgsJsonV1,
    planned: &mut [Gen3dPlannedComponent],
    draft: &mut Gen3dDraft,
) -> (Vec<OpAppliedJsonV1>, Vec<OpRejectionJsonV1>, ApplyWorkState) {
    let mut applied: Vec<OpAppliedJsonV1> = Vec::new();
    let mut rejected: Vec<OpRejectionJsonV1> = Vec::new();
    let mut state = ApplyWorkState::default();

    for (idx, op) in args.ops.iter().enumerate() {
        match apply_one_op(idx, op, planned, draft, &mut state) {
            Ok(v) => applied.push(v),
            Err(err) => rejected.push(err),
        }
    }

    (applied, rejected, state)
}

pub(super) fn query_component_parts_v1(
    job: &Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: QueryComponentPartsArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid query_component_parts_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported query_component_parts_v1 version {} (expected 1)",
            args.version
        ));
    }

    let mut component = args.component.unwrap_or_default();
    component = component.trim().to_string();
    if component.is_empty() {
        let Some(idx) = args.component_index else {
            return Err("Missing args.component or args.component_index".into());
        };
        let Some(name) = job.planned_components.get(idx).map(|c| c.name.as_str()) else {
            let components_total = job.planned_components.len();
            let available: Vec<String> = job
                .planned_components
                .iter()
                .take(24)
                .map(|c| c.name.clone())
                .collect();
            return Err(format!(
                "Invalid args.component_index={idx} (components_total={components_total}). Available (first {}): {available:?}",
                available.len()
            ));
        };
        component = name.to_string();
    }

    let object_id = component_object_id_for_name(component.as_str());
    let def = draft
        .defs
        .iter()
        .find(|d| d.object_id == object_id)
        .ok_or_else(|| format!("Component `{}` not found in draft.defs", component))?;

    let max_parts = args.max_parts.unwrap_or(256).max(1) as usize;
    let hard_cap = GEN3D_MAX_PARTS.min(4096);
    let max_parts = max_parts.min(hard_cap);

    let mut out_parts: Vec<serde_json::Value> = Vec::new();
    let mut truncated = false;
    const SAMPLE_RECOLOR_MAX: usize = 8;
    let mut recolor_samples: Vec<serde_json::Value> = Vec::new();
    let mut transform_sample: Option<serde_json::Value> = None;
    let mut recolorable_primitives_total: usize = 0;
    let mut primitives_with_part_id_total: usize = 0;
    for (part_index, part) in def.parts.iter().enumerate() {
        let kind_str = match &part.kind {
            ObjectPartKind::Primitive { .. } => "primitive",
            ObjectPartKind::ObjectRef { .. } => "object_ref",
            ObjectPartKind::Model { .. } => "model",
        };
        if !args.include_non_primitives && kind_str != "primitive" {
            continue;
        }
        if out_parts.len() >= max_parts {
            truncated = true;
            break;
        }

        let part_id_uuid = part.part_id.map(|id| Uuid::from_u128(id).to_string());
        if kind_str == "primitive" && part_id_uuid.is_some() {
            primitives_with_part_id_total = primitives_with_part_id_total.saturating_add(1);
        }
        let t = part.transform;
        let mut json = serde_json::Map::new();
        json.insert("part_index".into(), serde_json::json!(part_index));
        json.insert("kind".into(), serde_json::Value::String(kind_str.into()));
        json.insert(
            "part_id_uuid".into(),
            part_id_uuid
                .as_ref()
                .cloned()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
        json.insert(
            "render_priority".into(),
            part.render_priority
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
        );
        json.insert(
            "transform".into(),
            serde_json::json!({
                "pos": [t.translation.x, t.translation.y, t.translation.z],
                "rot_quat_xyzw": [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
                "scale": [t.scale.x, t.scale.y, t.scale.z],
            }),
        );

        match &part.kind {
            ObjectPartKind::Primitive { primitive } => {
                let prim_json = match primitive {
                    PrimitiveVisualDef::Primitive {
                        mesh,
                        params,
                        color,
                        unlit,
                    } => {
                        let mesh_apply = match mesh {
                            MeshKey::UnitCube => Some("cube"),
                            MeshKey::UnitCylinder => Some("cylinder"),
                            MeshKey::UnitCone => Some("cone"),
                            MeshKey::UnitSphere => Some("sphere"),
                            MeshKey::UnitCapsule => Some("capsule"),
                            MeshKey::UnitConicalFrustum => Some("conical_frustum"),
                            MeshKey::UnitTorus => Some("torus"),
                            _ => None,
                        };
                        let srgba = color.to_srgba();
                        let params_json = match params {
                            None => serde_json::Value::Null,
                            Some(PrimitiveParams::Capsule {
                                half_length,
                                radius,
                            }) => serde_json::json!({
                                "kind": "capsule",
                                "half_length": half_length,
                                "radius": radius,
                            }),
                            Some(PrimitiveParams::ConicalFrustum {
                                radius_top,
                                radius_bottom,
                                height,
                            }) => serde_json::json!({
                                "kind": "conical_frustum",
                                "top_radius": radius_top,
                                "bottom_radius": radius_bottom,
                                "height": height,
                            }),
                            Some(PrimitiveParams::Torus {
                                minor_radius,
                                major_radius,
                            }) => serde_json::json!({
                                "kind": "torus",
                                "minor_radius": minor_radius,
                                "major_radius": major_radius,
                            }),
                        };
                        if let (Some(part_id_uuid), Some(mesh_apply)) =
                            (part_id_uuid.as_ref(), mesh_apply)
                        {
                            recolorable_primitives_total =
                                recolorable_primitives_total.saturating_add(1);
                            if recolor_samples.len() < SAMPLE_RECOLOR_MAX {
                                recolor_samples.push(serde_json::json!({
                                    "kind": "update_primitive_part",
                                    "component": component.as_str(),
                                    "part_id_uuid": part_id_uuid,
                                    "set_primitive": {
                                        "mesh": mesh_apply,
                                        "params": params_json.clone(),
                                        // Example color. Change this (and replicate the op) as needed.
                                        "color_rgba": [0.20, 0.40, 0.80, 1.00],
                                        "unlit": *unlit,
                                    },
                                }));
                            }
                            if transform_sample.is_none() {
                                transform_sample = Some(serde_json::json!({
                                    "kind": "update_primitive_part",
                                    "component": component.as_str(),
                                    "part_id_uuid": part_id_uuid,
                                    "set_transform": {
                                        "pos": [t.translation.x, t.translation.y, t.translation.z],
                                        "rot_quat_xyzw": [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
                                        "scale": [t.scale.x, t.scale.y, t.scale.z],
                                    },
                                }));
                            }
                        }
                        serde_json::json!({
                            "mesh": format!("{mesh:?}"),
                            "mesh_apply": mesh_apply,
                            "params": params_json,
                            "color_rgba": [srgba.red, srgba.green, srgba.blue, srgba.alpha],
                            "unlit": unlit,
                        })
                    }
                    PrimitiveVisualDef::Mesh { mesh, material } => serde_json::json!({
                        "mesh": format!("{mesh:?}"),
                        "material": format!("{material:?}"),
                    }),
                };
                json.insert("primitive".into(), prim_json);
            }
            ObjectPartKind::ObjectRef { object_id } => {
                json.insert(
                    "object_id_uuid".into(),
                    serde_json::Value::String(Uuid::from_u128(*object_id).to_string()),
                );
                if let Some(att) = part.attachment.as_ref() {
                    json.insert(
                        "attachment".into(),
                        serde_json::json!({
                            "parent_anchor": att.parent_anchor.as_ref(),
                            "child_anchor": att.child_anchor.as_ref(),
                        }),
                    );
                }
                if !part.animations.is_empty() {
                    json.insert(
                        "animations".into(),
                        serde_json::Value::Array(
                            part.animations
                                .iter()
                                .map(|slot| {
                                    serde_json::json!({
                                        "channel": slot.channel.as_ref(),
                                        "driver": format!("{:?}", slot.spec.driver),
                                        "speed_scale": slot.spec.speed_scale,
                                        "time_offset_units": slot.spec.time_offset_units,
                                    })
                                })
                                .collect(),
                        ),
                    );
                }
            }
            ObjectPartKind::Model { scene } => {
                json.insert("scene".into(), serde_json::Value::String(scene.to_string()));
            }
        }

        out_parts.push(serde_json::Value::Object(json));
    }

    let component_index = job
        .planned_components
        .iter()
        .position(|c| c.name == component)
        .map(|idx| idx as u32);
    let planned_articulation_nodes = job
        .planned_components
        .iter()
        .find(|c| c.name == component)
        .map(|planned| planned.articulation_nodes.clone())
        .unwrap_or_default();
    let articulation_nodes = planned_articulation_nodes
        .iter()
        .map(|node| {
            serde_json::json!({
                "node_id": node.node_id,
                "parent_node_id": node.parent_node_id,
                "transform": {
                    "pos": [
                        node.transform.translation.x,
                        node.transform.translation.y,
                        node.transform.translation.z,
                    ],
                    "rot_quat_xyzw": [
                        node.transform.rotation.x,
                        node.transform.rotation.y,
                        node.transform.rotation.z,
                        node.transform.rotation.w,
                    ],
                    "scale": [
                        node.transform.scale.x,
                        node.transform.scale.y,
                        node.transform.scale.z,
                    ],
                },
                "bound_part_id_uuids": node
                    .bound_part_ids
                    .iter()
                    .map(|id| Uuid::from_u128(*id).to_string())
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    let articulation_part_id_samples: Vec<String> = out_parts
        .iter()
        .filter_map(|part| {
            part.get("part_id_uuid")
                .and_then(|value| value.as_str())
                .map(|value| value.trim())
                .filter(|value| !value.is_empty() && *value != "null")
                .map(|value| value.to_string())
        })
        .take(3)
        .collect();

    let mut recipes: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let recolor_sample_total = recolor_samples.len();
    if !recolor_samples.is_empty() {
        recipes.insert(
            "recolor_sample".into(),
            serde_json::json!({
                "tool_id": "apply_draft_ops_v1",
                "note": "Recolor a few sample primitives. Update color_rgba and replicate for more parts as needed. For `update_primitive_part.set_primitive`, keep mesh+params unchanged; only change color_rgba/unlit.",
                "args": {
                    "version": 1,
                    "atomic": true,
                    "if_assembly_rev": job.assembly_rev(),
                    "ops": recolor_samples,
                },
            }),
        );
    }
    let transform_sample_total = transform_sample.as_ref().map(|_| 1).unwrap_or(0);
    if let Some(sample) = transform_sample {
        recipes.insert(
            "update_transform_sample".into(),
            serde_json::json!({
                "tool_id": "apply_draft_ops_v1",
                "note": "Update a primitive part transform (absolute set). Edit pos/rot_quat_xyzw/scale as needed.",
                "args": {
                    "version": 1,
                    "atomic": true,
                    "if_assembly_rev": job.assembly_rev(),
                    "ops": [sample],
                },
            }),
        );
    }
    if let Some(node) = planned_articulation_nodes.first() {
        recipes.insert(
            "upsert_articulation_node_sample".into(),
            serde_json::json!({
                "tool_id": "apply_draft_ops_v1",
                "note": "Add or update an internal articulation node without regenerating geometry. Use existing part_id_uuid values from this snapshot; articulation nodes are rig metadata only.",
                "args": {
                    "version": 1,
                    "atomic": true,
                    "if_assembly_rev": job.assembly_rev(),
                    "ops": [{
                        "kind": "upsert_articulation_node",
                        "component": component.as_str(),
                        "node_id": node.node_id,
                        "parent_node_id": node.parent_node_id,
                        "set_transform": {
                            "pos": [
                                node.transform.translation.x,
                                node.transform.translation.y + 0.02,
                                node.transform.translation.z,
                            ],
                            "rot_quat_xyzw": [
                                node.transform.rotation.x,
                                node.transform.rotation.y,
                                node.transform.rotation.z,
                                node.transform.rotation.w,
                            ],
                        },
                        "bound_part_id_uuids": node
                            .bound_part_ids
                            .iter()
                            .map(|id| Uuid::from_u128(*id).to_string())
                            .collect::<Vec<_>>(),
                    }],
                },
            }),
        );
        recipes.insert(
            "rebind_articulation_node_parts_sample".into(),
            serde_json::json!({
                "tool_id": "apply_draft_ops_v1",
                "note": "Replace the part bindings of an existing articulation node. One primitive part may belong to only one articulation node at a time.",
                "args": {
                    "version": 1,
                    "atomic": true,
                    "if_assembly_rev": job.assembly_rev(),
                    "ops": [{
                        "kind": "rebind_articulation_node_parts",
                        "component": component.as_str(),
                        "node_id": node.node_id,
                        "bound_part_id_uuids": node
                            .bound_part_ids
                            .iter()
                            .map(|id| Uuid::from_u128(*id).to_string())
                            .collect::<Vec<_>>(),
                    }],
                },
            }),
        );

        let mut parent_node_ids = std::collections::HashSet::<&str>::new();
        for other in planned_articulation_nodes.iter() {
            if let Some(parent) = other.parent_node_id.as_deref() {
                parent_node_ids.insert(parent);
            }
        }
        if let Some(leaf) = planned_articulation_nodes
            .iter()
            .find(|candidate| !parent_node_ids.contains(candidate.node_id.as_str()))
        {
            recipes.insert(
                "remove_articulation_node_sample".into(),
                serde_json::json!({
                    "tool_id": "apply_draft_ops_v1",
                    "note": "Remove a leaf articulation node. Removal is rejected when another articulation node still parents to it.",
                    "args": {
                        "version": 1,
                        "atomic": true,
                        "if_assembly_rev": job.assembly_rev(),
                        "ops": [{
                            "kind": "remove_articulation_node",
                            "component": component.as_str(),
                            "node_id": leaf.node_id,
                        }],
                    },
                }),
            );
        }
    } else if !articulation_part_id_samples.is_empty() {
        recipes.insert(
            "add_articulation_node_sample".into(),
            serde_json::json!({
                "tool_id": "apply_draft_ops_v1",
                "note": "Create a new internal articulation node bound to existing primitive parts. This keeps the same component geometry and only adds rig metadata.",
                "args": {
                    "version": 1,
                    "atomic": true,
                    "if_assembly_rev": job.assembly_rev(),
                    "ops": [{
                        "kind": "upsert_articulation_node",
                        "component": component.as_str(),
                        "node_id": "articulation_sample",
                        "parent_node_id": serde_json::Value::Null,
                        "set_transform": {
                            "pos": [0.0, 0.0, 0.0],
                            "rot_quat_xyzw": [0.0, 0.0, 0.0, 1.0],
                        },
                        "bound_part_id_uuids": articulation_part_id_samples,
                    }],
                },
            }),
        );
    }

    let result = serde_json::json!({
        "ok": true,
        "version": 1,
        "component": component.as_str(),
        "component_index": component_index,
        "component_id_uuid": Uuid::from_u128(object_id).to_string(),
        "active_workspace": job.active_workspace_id(),
        "assembly_rev": job.assembly_rev(),
        "parts": out_parts,
        "articulation_nodes": articulation_nodes,
        "truncated": truncated,
        "editability": {
            "primitives_with_part_id_total": primitives_with_part_id_total,
            "recolorable_primitives_total": recolorable_primitives_total,
            "articulation_nodes_total": planned_articulation_nodes.len(),
            "recolor_sample_total": recolor_sample_total,
            "update_transform_sample_total": transform_sample_total,
        },
        "hints": [
            "For recolor: use apply_draft_ops_v1 with kind=update_primitive_part and set_primitive (mesh+params required; change only color_rgba/unlit).",
            "Edits require part_id_uuid. If part_id_uuid is null, that part is not directly editable via apply_draft_ops_v1.",
            "Articulation nodes can be edited in place via upsert_articulation_node, remove_articulation_node, and rebind_articulation_node_parts.",
            "A primitive part may belong to only one articulation node at a time; remove_primitive_part is rejected while a node still binds that part.",
            "All transforms in apply_draft_ops_v1 are absolute sets (not additive deltas).",
        ],
        "recipes": recipes,
    });

    if let Some(dir) = job.step_dir_path() {
        let prefix = sanitize_prefix(&format!("component_parts_{}", component));
        write_gen3d_json_artifact(Some(dir), format!("{prefix}.json"), &result);
    }

    Ok(result)
}

pub(super) fn apply_draft_ops_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    call_id: Option<&str>,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: ApplyDraftOpsArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid apply_draft_ops_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported apply_draft_ops_v1 version {} (expected 1)",
            args.version
        ));
    }
    if args.ops.len() > 64 {
        return Err("apply_draft_ops_v1: too many ops (max 64)".into());
    }
    if let Some(expected) = args.if_assembly_rev {
        if expected != job.assembly_rev() {
            return Err(format!(
                "apply_draft_ops_v1: if_assembly_rev mismatch (expected {}, current {})",
                expected,
                job.assembly_rev()
            ));
        }
    }

    let assembly_rev_before = job.assembly_rev();

    // Atomic mode: apply to clones, then commit only if no rejections.
    let (applied, rejected, state, committed) = if args.atomic {
        let mut planned_clone = job.planned_components.clone();
        let mut draft_clone = draft.clone();
        let (applied, rejected, state) =
            apply_ops_inner(&args, &mut planned_clone, &mut draft_clone);
        if !rejected.is_empty() {
            (Vec::new(), rejected, state, false)
        } else {
            job.planned_components = planned_clone;
            *draft = draft_clone;
            (applied, Vec::new(), state, true)
        }
    } else {
        let (applied, rejected, state) = apply_ops_inner(&args, &mut job.planned_components, draft);
        (applied, rejected, state, true)
    };

    if committed && !applied.is_empty() {
        if let Some(root_idx) = find_root_component_index(&job.planned_components) {
            if state.needs_resolve_transforms {
                convert::resolve_planned_component_transforms(
                    &mut job.planned_components,
                    root_idx,
                )?;
            }
        }

        if state.needs_sync_attachments {
            convert::sync_attachment_tree_to_defs(&job.planned_components, draft)?;
        }

        convert::update_root_def_from_planned_components(
            &job.planned_components,
            &job.plan_collider,
            draft,
        );

        if let Some(dir) = job.step_dir_path() {
            super::artifacts::write_gen3d_assembly_snapshot(Some(dir), &job.planned_components);
        }

        job.assembly_rev = job.assembly_rev.saturating_add(1);
    }

    let assembly_rev_after = job.assembly_rev();
    let changed_component_ids: Vec<String> = state
        .changed_component_ids
        .iter()
        .copied()
        .map(|id| Uuid::from_u128(id).to_string())
        .collect();

    let diff_summary = serde_json::json!({
        "anchors_updated": state.anchors_updated,
        "attachments_updated": state.attachments_updated,
        "primitive_parts": {
            "added": state.primitive_parts_added,
            "removed": state.primitive_parts_removed,
            "updated": state.primitive_parts_updated,
        },
        "articulation_nodes": {
            "upserted": state.articulation_nodes_upserted,
            "removed": state.articulation_nodes_removed,
            "rebound": state.articulation_nodes_rebound,
        },
        "animation_slots": {
            "upserted": state.animation_slots_upserted,
            "scaled": state.animation_slots_scaled,
            "removed": state.animation_slots_removed,
        }
    });

    let result = serde_json::json!({
        "ok": committed && rejected.is_empty(),
        "version": 1,
        "atomic": args.atomic,
        "committed": committed,
        "if_assembly_rev": args.if_assembly_rev,
        "assembly_rev_before": assembly_rev_before,
        "new_assembly_rev": assembly_rev_after,
        "applied_ops": applied,
        "rejected_ops": rejected,
        "diff_summary": diff_summary,
        "changed_component_ids": changed_component_ids,
    });

    if let Some(dir) = job.step_dir_path() {
        let log_ref = "draft_ops.jsonl";
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        append_gen3d_jsonl_artifact(
            Some(dir),
            log_ref,
            &serde_json::json!({
                "ts_ms": ts_ms,
                "tool": "apply_draft_ops_v1",
                "call_id": call_id.unwrap_or(""),
                "active_workspace": job.active_workspace_id(),
                "assembly_rev_before": assembly_rev_before,
                "assembly_rev_after": assembly_rev_after,
                "atomic": args.atomic,
                "committed": committed,
                "result": result,
            }),
        );
        write_gen3d_json_artifact(Some(dir), "apply_draft_ops_last.json", &result);
    }

    Ok(result)
}

pub(crate) fn gen3d_apply_draft_ops_from_api(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    apply_draft_ops_v1(job, draft, Some("api"), args_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen3d::gen3d_draft_object_id;
    use crate::object::registry::{AnchorDef, ColliderProfile, ObjectInteraction};

    fn component_part_id(name: &str, idx: usize) -> u128 {
        builtin_object_id(&format!("gravimera/gen3d/part/{name}/{idx}"))
    }

    fn make_component_def_with_parts(name: &str, primitive_count: usize) -> ObjectDef {
        let object_id = component_object_id_for_name(name);
        let mut parts: Vec<ObjectPartDef> = Vec::new();
        for idx in 0..primitive_count.max(1) {
            let mut part = ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: false,
                },
                Transform::from_translation(Vec3::new(idx as f32 * 0.25, 0.0, 0.0)),
            );
            part.part_id = Some(component_part_id(name, idx));
            parts.push(part);
        }

        let size = crate::gen3d::ai::convert::size_from_primitive_parts(&parts);

        ObjectDef {
            object_id,
            label: format!("gen3d_component_{name}").into(),
            size,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![AnchorDef {
                name: "mount".into(),
                transform: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            }],
            parts,
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        }
    }

    fn make_component_def(name: &str) -> ObjectDef {
        make_component_def_with_parts(name, 1)
    }

    fn make_root_def() -> ObjectDef {
        ObjectDef {
            object_id: gen3d_draft_object_id(),
            label: "gen3d_draft".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        }
    }

    fn make_job_with_components(names: &[&str]) -> Gen3dAiJob {
        let mut job = Gen3dAiJob::default();
        job.planned_components = names
            .iter()
            .enumerate()
            .map(|(idx, name)| Gen3dPlannedComponent {
                display_name: format!("{}. {name}", idx + 1),
                name: (*name).to_string(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: vec![crate::object::registry::AnchorDef {
                    name: "mount".into(),
                    transform: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                }],
                contacts: Vec::new(),
                articulation_nodes: Vec::new(),
                root_animations: Vec::new(),
                attach_to: if idx == 0 {
                    None
                } else {
                    Some(super::super::job::Gen3dPlannedAttachment {
                        parent: names[0].to_string(),
                        parent_anchor: "origin".to_string(),
                        child_anchor: "origin".to_string(),
                        offset: Transform::IDENTITY,
                        fallback_basis: Transform::IDENTITY,
                        joint: None,
                        animations: Vec::new(),
                    })
                },
            })
            .collect();
        job
    }

    #[test]
    fn apply_updates_anchor_transform() {
        let mut job = make_job_with_components(&["root", "child"]);
        let mut draft = Gen3dDraft {
            defs: vec![
                make_root_def(),
                make_component_def("root"),
                make_component_def("child"),
            ],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [
                {
                    "kind": "set_anchor_transform",
                    "component": "root",
                    "anchor": "mount",
                    "set": { "pos": [0.0, 2.0, 0.0] }
                }
            ]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(out.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        let root_def = draft
            .defs
            .iter()
            .find(|d| d.object_id == component_object_id_for_name("root"))
            .unwrap();
        let mount = root_def
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == "mount")
            .unwrap();
        assert!((mount.transform.translation.y - 2.0).abs() < 1e-6);
    }

    #[test]
    fn apply_upserts_root_animation_slot() {
        let mut job = make_job_with_components(&["root"]);
        let mut draft = Gen3dDraft {
            defs: vec![make_root_def(), make_component_def("root")],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [
                {
                    "kind": "upsert_animation_slot",
                    "child_component": "root",
                    "channel": "idle",
                    "slot": {
                        "driver": "always",
                        "speed_scale": 1.0,
                        "time_offset_units": 0.0,
                        "clip": {
                            "kind": "loop",
                            "duration_units": 1.0,
                            "keyframes": [
                                { "t_units": 0.0, "delta": {} }
                            ]
                        }
                    }
                }
            ]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(out.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));

        assert_eq!(job.planned_components.len(), 1);
        assert_eq!(job.planned_components[0].root_animations.len(), 1);
        assert_eq!(
            job.planned_components[0].root_animations[0]
                .channel
                .as_ref(),
            "idle"
        );

        let root_def = draft
            .defs
            .iter()
            .find(|d| d.object_id == gen3d_draft_object_id())
            .expect("draft root def");
        let root_ref = root_def
            .parts
            .iter()
            .find(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. }) && p.attachment.is_some())
            .expect("draft root object-ref part");
        assert!(
            root_ref
                .animations
                .iter()
                .any(|s| s.channel.as_ref() == "idle"),
            "expected root object-ref animations to include idle, got {:?}",
            root_ref.animations
        );
    }

    #[test]
    fn apply_sets_attachment_joint() {
        let mut job = make_job_with_components(&["root", "child"]);
        let mut draft = Gen3dDraft {
            defs: vec![
                make_root_def(),
                make_component_def("root"),
                make_component_def("child"),
            ],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [
                {
                    "kind": "set_attachment_joint",
                    "child_component": "child",
                    "set_joint": { "kind": "hinge", "axis_join": [1.0, 0.0, 0.0], "limits_degrees": [-45.0, 45.0] }
                }
            ]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(out.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        let joint = job.planned_components[1]
            .attach_to
            .as_ref()
            .and_then(|a| a.joint.as_ref())
            .expect("expected joint");
        assert_eq!(joint.kind, AiJointKindJson::Hinge);
        assert_eq!(joint.axis_join, Some([1.0, 0.0, 0.0]));
        assert_eq!(joint.limits_degrees, Some([-45.0, 45.0]));
    }

    #[test]
    fn apply_rejects_hinge_without_axis() {
        let mut job = make_job_with_components(&["root", "child"]);
        let mut draft = Gen3dDraft {
            defs: vec![
                make_root_def(),
                make_component_def("root"),
                make_component_def("child"),
            ],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [
                {
                    "kind": "set_attachment_joint",
                    "child_component": "child",
                    "set_joint": { "kind": "hinge" }
                }
            ]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(!out.get("ok").and_then(|v| v.as_bool()).unwrap_or(true));
        let rejected = out
            .get("rejected_ops")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(!rejected.is_empty());
    }

    #[test]
    fn apply_scales_animation_slot_rotation_keyframes() {
        let mut job = make_job_with_components(&["root", "child"]);
        let mut draft = Gen3dDraft {
            defs: vec![
                make_root_def(),
                make_component_def("root"),
                make_component_def("child"),
            ],
        };

        let child = job.planned_components.get_mut(1).unwrap();
        let att = child.attach_to.as_mut().unwrap();
        let slot = PartAnimationSlot {
            channel: "move".into(),
            family: crate::object::registry::PartAnimationFamily::Base,
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::MovePhase,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis: Transform::IDENTITY,
                clip: PartAnimationDef::Loop {
                    duration_secs: 1.0,
                    keyframes: vec![
                        PartAnimationKeyframeDef {
                            time_secs: 0.0,
                            delta: Transform::IDENTITY,
                        },
                        PartAnimationKeyframeDef {
                            time_secs: 0.5,
                            delta: Transform {
                                rotation: Quat::from_axis_angle(
                                    Vec3::X,
                                    std::f32::consts::FRAC_PI_2,
                                ),
                                ..Default::default()
                            },
                        },
                    ],
                },
            },
        };
        att.animations.push(slot);

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [
                {
                    "kind": "scale_animation_slot_rotation",
                    "child_component": "child",
                    "channel": "move",
                    "scale": 0.5
                }
            ]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(out.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));

        let child = job.planned_components.get(1).unwrap();
        let att = child.attach_to.as_ref().unwrap();
        let slot = att
            .animations
            .iter()
            .find(|s| s.channel.as_ref() == "move")
            .unwrap();
        let PartAnimationDef::Loop { keyframes, .. } = &slot.spec.clip else {
            panic!("expected loop clip");
        };
        let q = keyframes
            .iter()
            .find(|k| (k.time_secs - 0.5).abs() < 1e-6)
            .unwrap()
            .delta
            .rotation;
        let (_axis, angle) = q.normalize().to_axis_angle();
        assert!((angle - std::f32::consts::FRAC_PI_4).abs() < 1e-3);
    }

    #[test]
    fn apply_upserts_articulation_node() {
        let mut job = make_job_with_components(&["root"]);
        let mut draft = Gen3dDraft {
            defs: vec![make_root_def(), make_component_def_with_parts("root", 2)],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [{
                "kind": "upsert_articulation_node",
                "component": "root",
                "node_id": "jaw",
                "parent_node_id": null,
                "set_transform": {
                    "pos": [0.0, -0.1, 0.2],
                    "rot_quat_xyzw": [0.0, 0.0, 0.0, 1.0]
                },
                "bound_part_id_uuids": [Uuid::from_u128(component_part_id("root", 0)).to_string()]
            }]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(out.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        assert_eq!(job.planned_components[0].articulation_nodes.len(), 1);
        let node = &job.planned_components[0].articulation_nodes[0];
        assert_eq!(node.node_id, "jaw");
        assert_eq!(node.parent_node_id, None);
        assert_eq!(node.bound_part_ids, vec![component_part_id("root", 0)]);
        assert_eq!(
            out.get("diff_summary")
                .and_then(|v| v.get("articulation_nodes"))
                .and_then(|v| v.get("upserted"))
                .and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[test]
    fn apply_rebinds_articulation_node_parts() {
        let mut job = make_job_with_components(&["root"]);
        job.planned_components[0].articulation_nodes = vec![Gen3dPlannedArticulationNode {
            node_id: "jaw".into(),
            parent_node_id: None,
            transform: Transform::IDENTITY,
            bound_part_ids: vec![component_part_id("root", 0)],
        }];
        let mut draft = Gen3dDraft {
            defs: vec![make_root_def(), make_component_def_with_parts("root", 2)],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [{
                "kind": "rebind_articulation_node_parts",
                "component": "root",
                "node_id": "jaw",
                "bound_part_id_uuids": [Uuid::from_u128(component_part_id("root", 1)).to_string()]
            }]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(out.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        assert_eq!(
            job.planned_components[0].articulation_nodes[0].bound_part_ids,
            vec![component_part_id("root", 1)]
        );
        assert_eq!(
            out.get("diff_summary")
                .and_then(|v| v.get("articulation_nodes"))
                .and_then(|v| v.get("rebound"))
                .and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[test]
    fn apply_rejects_removing_articulation_node_with_children() {
        let mut job = make_job_with_components(&["root"]);
        job.planned_components[0].articulation_nodes = vec![
            Gen3dPlannedArticulationNode {
                node_id: "head_core".into(),
                parent_node_id: None,
                transform: Transform::IDENTITY,
                bound_part_ids: vec![component_part_id("root", 0)],
            },
            Gen3dPlannedArticulationNode {
                node_id: "jaw".into(),
                parent_node_id: Some("head_core".into()),
                transform: Transform::IDENTITY,
                bound_part_ids: vec![component_part_id("root", 1)],
            },
        ];
        let mut draft = Gen3dDraft {
            defs: vec![make_root_def(), make_component_def_with_parts("root", 2)],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [{
                "kind": "remove_articulation_node",
                "component": "root",
                "node_id": "head_core"
            }]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(!out.get("ok").and_then(|v| v.as_bool()).unwrap_or(true));
        assert_eq!(job.planned_components[0].articulation_nodes.len(), 2);
    }

    #[test]
    fn apply_removes_leaf_articulation_node() {
        let mut job = make_job_with_components(&["root"]);
        job.planned_components[0].articulation_nodes = vec![Gen3dPlannedArticulationNode {
            node_id: "jaw".into(),
            parent_node_id: None,
            transform: Transform::IDENTITY,
            bound_part_ids: vec![component_part_id("root", 0)],
        }];
        let mut draft = Gen3dDraft {
            defs: vec![make_root_def(), make_component_def_with_parts("root", 1)],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [{
                "kind": "remove_articulation_node",
                "component": "root",
                "node_id": "jaw"
            }]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(out.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        assert!(job.planned_components[0].articulation_nodes.is_empty());
    }

    #[test]
    fn apply_rejects_removing_bound_primitive_part() {
        let mut job = make_job_with_components(&["root"]);
        job.planned_components[0].articulation_nodes = vec![Gen3dPlannedArticulationNode {
            node_id: "jaw".into(),
            parent_node_id: None,
            transform: Transform::IDENTITY,
            bound_part_ids: vec![component_part_id("root", 0)],
        }];
        let mut draft = Gen3dDraft {
            defs: vec![make_root_def(), make_component_def_with_parts("root", 1)],
        };

        let args = serde_json::json!({
            "version": 1,
            "atomic": true,
            "ops": [{
                "kind": "remove_primitive_part",
                "component": "root",
                "part_id_uuid": Uuid::from_u128(component_part_id("root", 0)).to_string()
            }]
        });
        let out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), args).unwrap();
        assert!(!out.get("ok").and_then(|v| v.as_bool()).unwrap_or(true));
        let root_def = draft
            .defs
            .iter()
            .find(|d| d.object_id == component_object_id_for_name("root"))
            .expect("root def");
        assert_eq!(root_def.parts.len(), 1);
    }
}
