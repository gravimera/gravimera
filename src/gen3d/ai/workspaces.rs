use bevy::prelude::*;
use serde::Deserialize;
use uuid::Uuid;

use crate::object::registry::{
    builtin_object_id, MeshKey, ObjectDef, ObjectPartDef, ObjectPartKind, PartAnimationDef,
    PrimitiveParams, PrimitiveVisualDef,
};

use super::super::state::Gen3dDraft;
use super::job::{Gen3dAgentWorkspace, Gen3dPlannedComponent};
use super::Gen3dAiJob;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffWorkspacesArgsJsonV1 {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    a: Option<String>,
    #[serde(default)]
    b: Option<String>,
    #[serde(default)]
    max_components: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyFromWorkspaceArgsJsonV1 {
    #[serde(default)]
    version: u32,
    from: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    components: Vec<String>,
    #[serde(default)]
    include_attachment: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeWorkspacesArgsJsonV1 {
    #[serde(default)]
    version: u32,
    base: String,
    a: String,
    b: String,
    #[serde(default)]
    output_workspace_id: Option<String>,
    #[serde(default)]
    output_name: Option<String>,
    #[serde(default)]
    max_components: Option<u32>,
}

fn component_object_id_for_name(name: &str) -> u128 {
    builtin_object_id(&format!("gravimera/gen3d/component/{}", name))
}

fn capture_active_workspace(job: &Gen3dAiJob, draft: &Gen3dDraft) -> Gen3dAgentWorkspace {
    Gen3dAgentWorkspace {
        name: job.active_workspace_id().to_string(),
        defs: draft.defs.clone(),
        planned_components: job.planned_components.clone(),
        plan_hash: job.plan_hash.clone(),
        assembly_rev: job.assembly_rev,
        assembly_notes: job.assembly_notes.clone(),
        plan_collider: job.plan_collider.clone(),
        rig_move_cycle_m: job.rig_move_cycle_m,
        motion_authoring: job.motion_authoring.clone(),
        reuse_groups: job.reuse_groups.clone(),
        reuse_group_warnings: job.reuse_group_warnings.clone(),
    }
}

fn get_workspace_clone(
    job: &Gen3dAiJob,
    draft: &Gen3dDraft,
    workspace_id: &str,
) -> Result<Gen3dAgentWorkspace, String> {
    if workspace_id.trim() == job.active_workspace_id().trim() {
        return Ok(capture_active_workspace(job, draft));
    }
    job.agent
        .workspaces
        .get(workspace_id)
        .cloned()
        .ok_or_else(|| format!("Unknown workspace `{workspace_id}`"))
}

#[derive(Clone, Debug, PartialEq)]
struct PrimitivePartFp {
    part_id: Option<u128>,
    render_priority: Option<i32>,
    mesh: Option<MeshKey>,
    params: Option<PrimitiveParams>,
    color: Option<[f32; 4]>,
    unlit: Option<bool>,
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

fn primitive_part_fp(part: &ObjectPartDef) -> Option<PrimitivePartFp> {
    let ObjectPartKind::Primitive { primitive } = &part.kind else {
        return None;
    };
    let (mesh, params, color, unlit) = match primitive {
        PrimitiveVisualDef::Primitive {
            mesh,
            params,
            color,
            unlit,
        } => {
            let c = color.to_srgba();
            (
                Some(*mesh),
                params.clone(),
                Some([c.red, c.green, c.blue, c.alpha]),
                Some(*unlit),
            )
        }
        PrimitiveVisualDef::Mesh { .. } => (None, None, None, None),
    };
    Some(PrimitivePartFp {
        part_id: part.part_id,
        render_priority: part.render_priority,
        mesh,
        params,
        color,
        unlit,
        translation: part.transform.translation,
        rotation: part.transform.rotation,
        scale: part.transform.scale,
    })
}

fn geometry_fp(def: &ObjectDef) -> Vec<PrimitivePartFp> {
    let mut parts: Vec<PrimitivePartFp> = def.parts.iter().filter_map(primitive_part_fp).collect();
    parts.sort_by_key(|p| p.part_id.unwrap_or(0));
    parts
}

fn anchors_fp(def: &ObjectDef) -> Vec<(String, [f32; 10])> {
    let mut out: Vec<(String, [f32; 10])> = def
        .anchors
        .iter()
        .map(|a| {
            let t = a.transform;
            (
                a.name.as_ref().to_string(),
                [
                    t.translation.x,
                    t.translation.y,
                    t.translation.z,
                    t.rotation.x,
                    t.rotation.y,
                    t.rotation.z,
                    t.rotation.w,
                    t.scale.x,
                    t.scale.y,
                    t.scale.z,
                ],
            )
        })
        .collect();
    out.sort_by(|(a, _), (b, _)| a.cmp(b));
    out
}

fn attachment_fp(comp: &Gen3dPlannedComponent) -> serde_json::Value {
    let Some(att) = comp.attach_to.as_ref() else {
        return serde_json::Value::Null;
    };
    let t = att.offset;
    let animations: Vec<serde_json::Value> = att
        .animations
        .iter()
        .map(|slot| {
            let clip_kind = match slot.spec.clip {
                PartAnimationDef::Loop { .. } => "loop",
                PartAnimationDef::Once { .. } => "once",
                PartAnimationDef::PingPong { .. } => "ping_pong",
                PartAnimationDef::Spin { .. } => "spin",
            };
            serde_json::json!({
                "channel": slot.channel.as_ref(),
                "driver": format!("{:?}", slot.spec.driver),
                "speed_scale": slot.spec.speed_scale,
                "time_offset_units": slot.spec.time_offset_units,
                "clip_kind": clip_kind,
            })
        })
        .collect();
    serde_json::json!({
        "parent": att.parent.as_str(),
        "parent_anchor": att.parent_anchor.as_str(),
        "child_anchor": att.child_anchor.as_str(),
        "offset": {
            "pos": [t.translation.x, t.translation.y, t.translation.z],
            "rot_quat_xyzw": [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
            "scale": [t.scale.x, t.scale.y, t.scale.z],
        },
        "joint": att.joint.as_ref().map(|j| {
            serde_json::json!({
                "kind": format!("{:?}", j.kind),
                "axis_join": j.axis_join,
                "limits_degrees": j.limits_degrees,
                "swing_limits_degrees": j.swing_limits_degrees,
                "twist_limits_degrees": j.twist_limits_degrees,
            })
        }),
        "animations": animations,
    })
}

fn attachment_core_fp(comp: &Gen3dPlannedComponent) -> serde_json::Value {
    let Some(att) = comp.attach_to.as_ref() else {
        return serde_json::Value::Null;
    };
    let t = att.offset;
    serde_json::json!({
        "parent": att.parent.as_str(),
        "parent_anchor": att.parent_anchor.as_str(),
        "child_anchor": att.child_anchor.as_str(),
        "offset": {
            "pos": [t.translation.x, t.translation.y, t.translation.z],
            "rot_quat_xyzw": [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
            "scale": [t.scale.x, t.scale.y, t.scale.z],
        },
        "joint": att.joint.as_ref().map(|j| {
            serde_json::json!({
                "kind": format!("{:?}", j.kind),
                "axis_join": j.axis_join,
                "limits_degrees": j.limits_degrees,
                "swing_limits_degrees": j.swing_limits_degrees,
                "twist_limits_degrees": j.twist_limits_degrees,
            })
        }),
    })
}

fn attachment_anims_fp(comp: &Gen3dPlannedComponent) -> serde_json::Value {
    let Some(att) = comp.attach_to.as_ref() else {
        return serde_json::Value::Null;
    };
    let mut animations: Vec<serde_json::Value> = att
        .animations
        .iter()
        .map(|slot| {
            let clip_kind = match slot.spec.clip {
                PartAnimationDef::Loop { .. } => "loop",
                PartAnimationDef::Once { .. } => "once",
                PartAnimationDef::PingPong { .. } => "ping_pong",
                PartAnimationDef::Spin { .. } => "spin",
            };
            serde_json::json!({
                "channel": slot.channel.as_ref(),
                "driver": format!("{:?}", slot.spec.driver),
                "speed_scale": slot.spec.speed_scale,
                "time_offset_units": slot.spec.time_offset_units,
                "clip_kind": clip_kind,
            })
        })
        .collect();
    animations.sort_by(|a, b| {
        a.get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("channel").and_then(|v| v.as_str()).unwrap_or(""))
    });
    serde_json::Value::Array(animations)
}

fn find_root_component_index(components: &[Gen3dPlannedComponent]) -> Option<usize> {
    components.iter().position(|c| c.attach_to.is_none())
}

pub(super) fn create_workspace_v1(
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let from = args_json
        .get("from")
        .or_else(|| args_json.get("base"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| job.active_workspace_id())
        .to_string();
    let name = args_json
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let include_components: Vec<String> = args_json
        .get("include_components")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut source = get_workspace_clone(job, draft, &from)?;
    let source_defs = std::mem::take(&mut source.defs);
    let new_defs = if include_components.is_empty() {
        source_defs
    } else {
        super::agent_utils::build_component_subset_workspace_defs(&source_defs, &include_components)
            .map_err(|err| format!("create_workspace_v1: {err}"))?
    };

    let mut workspace_id = args_json
        .get("workspace_id")
        .or_else(|| args_json.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        // Common agent behavior: provide only `name` and then try to `set_active_workspace`
        // using the same string. Treat `name` as the workspace_id in that case.
        .or_else(|| (!name.is_empty()).then_some(name.clone()))
        // Default: create a predictable workspace id so the agent can refer to it within the
        // same step without having to depend on tool return values.
        .unwrap_or_else(|| "preview".to_string());

    if workspace_id == job.active_workspace_id() || job.agent.workspaces.contains_key(&workspace_id)
    {
        workspace_id = format!("ws{}", job.agent.next_workspace_seq);
    }
    job.agent.next_workspace_seq = job.agent.next_workspace_seq.saturating_add(1);

    if workspace_id == job.active_workspace_id() {
        return Err("create_workspace_v1: workspace_id must not be the active workspace".into());
    }
    if job.agent.workspaces.contains_key(&workspace_id) {
        return Err(format!(
            "create_workspace_v1: workspace_id already exists: `{workspace_id}`"
        ));
    }

    source.name = if name.is_empty() {
        workspace_id.clone()
    } else {
        name
    };
    source.defs = new_defs;

    job.agent.workspaces.insert(workspace_id.clone(), source);

    Ok(serde_json::json!({ "workspace_id": workspace_id }))
}

pub(super) fn delete_workspace_v1(
    job: &mut Gen3dAiJob,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let workspace_id = args_json
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if workspace_id.is_empty() {
        return Err("delete_workspace_v1: missing workspace_id".into());
    }
    if workspace_id == job.active_workspace_id() {
        return Err("delete_workspace_v1: cannot delete the active workspace".into());
    }
    let removed = job.agent.workspaces.remove(&workspace_id).is_some();
    Ok(serde_json::json!({ "ok": removed }))
}

pub(super) fn set_active_workspace_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let workspace_id = args_json
        .get("workspace_id")
        .or_else(|| args_json.get("name"))
        .or_else(|| args_json.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if workspace_id.is_empty() {
        return Err("set_active_workspace_v1: missing workspace_id".into());
    }
    if workspace_id == job.active_workspace_id() {
        return Ok(serde_json::json!({ "ok": true }));
    }

    // Save current active workspace back into the map.
    let prev = job.active_workspace_id().to_string();
    if prev != "main" || !draft.defs.is_empty() || !job.planned_components.is_empty() {
        job.agent
            .workspaces
            .insert(prev.clone(), capture_active_workspace(job, draft));
    }

    let next = if workspace_id == "main" {
        job.agent
            .workspaces
            .get("main")
            .cloned()
            .unwrap_or_else(|| Gen3dAgentWorkspace {
                name: "main".into(),
                defs: Vec::new(),
                planned_components: Vec::new(),
                plan_hash: String::new(),
                assembly_rev: 0,
                assembly_notes: String::new(),
                plan_collider: None,
                rig_move_cycle_m: None,
                motion_authoring: None,
                reuse_groups: Vec::new(),
                reuse_group_warnings: Vec::new(),
            })
    } else if let Some(ws) = job.agent.workspaces.get(&workspace_id) {
        ws.clone()
    } else {
        return Err(format!(
            "set_active_workspace_v1: unknown workspace `{workspace_id}`"
        ));
    };

    draft.defs = next.defs;
    job.planned_components = next.planned_components;
    job.plan_hash = next.plan_hash;
    job.assembly_rev = next.assembly_rev;
    job.assembly_notes = next.assembly_notes;
    job.plan_collider = next.plan_collider;
    job.rig_move_cycle_m = next.rig_move_cycle_m;
    job.motion_authoring = next.motion_authoring;
    job.reuse_groups = next.reuse_groups;
    job.reuse_group_warnings = next.reuse_group_warnings;

    let component_count = job.planned_components.len();
    job.regen_per_component.resize(component_count, 0);
    job.component_attempts.resize(component_count, 0);
    job.component_last_errors.resize(component_count, None);
    job.agent.active_workspace_id = workspace_id;

    Ok(serde_json::json!({ "ok": true }))
}

pub(super) fn diff_workspaces_v1(
    job: &Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: DiffWorkspacesArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid diff_workspaces_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported diff_workspaces_v1 version {} (expected 1)",
            args.version
        ));
    }

    let active_id = job.active_workspace_id().to_string();
    let a_id = args
        .a
        .unwrap_or_else(|| active_id.clone())
        .trim()
        .to_string();
    let b_id = args
        .b
        .unwrap_or_else(|| "main".to_string())
        .trim()
        .to_string();
    let a_id = if a_id.is_empty() {
        active_id.clone()
    } else {
        a_id
    };
    let b_id = if b_id.is_empty() {
        "main".to_string()
    } else {
        b_id
    };

    let a_ws = get_workspace_clone(job, draft, &a_id)?;
    let b_ws = get_workspace_clone(job, draft, &b_id)?;

    let max_components = args.max_components.unwrap_or(128).max(1) as usize;

    let mut a_defs: std::collections::HashMap<u128, &ObjectDef> = std::collections::HashMap::new();
    for def in a_ws.defs.iter() {
        a_defs.insert(def.object_id, def);
    }
    let mut b_defs: std::collections::HashMap<u128, &ObjectDef> = std::collections::HashMap::new();
    for def in b_ws.defs.iter() {
        b_defs.insert(def.object_id, def);
    }

    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for c in a_ws.planned_components.iter() {
        names.insert(c.name.clone());
    }
    for c in b_ws.planned_components.iter() {
        names.insert(c.name.clone());
    }

    let mut components_out: Vec<serde_json::Value> = Vec::new();
    let mut truncated = false;
    let mut changed = 0u32;
    let mut geometry_changed = 0u32;
    let mut anchors_changed = 0u32;
    let mut attachments_changed = 0u32;
    let mut animations_changed = 0u32;

    for name in names.into_iter() {
        if components_out.len() >= max_components {
            truncated = true;
            break;
        }

        let a_comp = a_ws.planned_components.iter().find(|c| c.name == name);
        let b_comp = b_ws.planned_components.iter().find(|c| c.name == name);

        let object_id = component_object_id_for_name(&name);
        let a_def = a_defs.get(&object_id).copied();
        let b_def = b_defs.get(&object_id).copied();

        let a_geom = a_def.map(geometry_fp).unwrap_or_default();
        let b_geom = b_def.map(geometry_fp).unwrap_or_default();
        let a_anchors = a_def.map(anchors_fp).unwrap_or_default();
        let b_anchors = b_def.map(anchors_fp).unwrap_or_default();

        let attach_a = a_comp.map(attachment_fp).unwrap_or(serde_json::Value::Null);
        let attach_b = b_comp.map(attachment_fp).unwrap_or(serde_json::Value::Null);

        let this_geometry_changed = a_geom != b_geom;
        let this_anchors_changed = a_anchors != b_anchors;
        let this_attachment_changed = attach_a != attach_b;
        let this_anim_changed = if this_attachment_changed {
            attach_a.get("animations") != attach_b.get("animations")
        } else {
            false
        };

        let only_in_a = a_comp.is_some() && b_comp.is_none();
        let only_in_b = b_comp.is_some() && a_comp.is_none();

        let any_change = only_in_a
            || only_in_b
            || this_geometry_changed
            || this_anchors_changed
            || this_attachment_changed;
        if !any_change {
            continue;
        }

        changed = changed.saturating_add(1);
        if this_geometry_changed {
            geometry_changed = geometry_changed.saturating_add(1);
        }
        if this_anchors_changed {
            anchors_changed = anchors_changed.saturating_add(1);
        }
        if this_attachment_changed {
            attachments_changed = attachments_changed.saturating_add(1);
        }
        if this_anim_changed {
            animations_changed = animations_changed.saturating_add(1);
        }

        components_out.push(serde_json::json!({
            "name": name,
            "component_id_uuid": Uuid::from_u128(object_id).to_string(),
            "only_in_a": only_in_a,
            "only_in_b": only_in_b,
            "geometry_changed": this_geometry_changed,
            "anchors_changed": this_anchors_changed,
            "attachment_changed": this_attachment_changed,
            "animations_changed": this_anim_changed,
        }));
    }

    Ok(serde_json::json!({
        "ok": true,
        "version": 1,
        "a": { "workspace_id": a_id, "plan_hash": a_ws.plan_hash, "assembly_rev": a_ws.assembly_rev, "components": a_ws.planned_components.len(), "defs": a_ws.defs.len() },
        "b": { "workspace_id": b_id, "plan_hash": b_ws.plan_hash, "assembly_rev": b_ws.assembly_rev, "components": b_ws.planned_components.len(), "defs": b_ws.defs.len() },
        "diff_summary": {
            "components_changed": changed,
            "geometry_changed": geometry_changed,
            "anchors_changed": anchors_changed,
            "attachments_changed": attachments_changed,
            "animations_changed": animations_changed,
        },
        "components": components_out,
        "truncated": truncated,
    }))
}

pub(super) fn copy_from_workspace_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: CopyFromWorkspaceArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid copy_from_workspace_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported copy_from_workspace_v1 version {} (expected 1)",
            args.version
        ));
    }

    let from_id = args.from.trim().to_string();
    if from_id.is_empty() {
        return Err("copy_from_workspace_v1 requires non-empty args.from".into());
    }
    let mode = args
        .mode
        .as_deref()
        .unwrap_or("component")
        .trim()
        .to_lowercase();
    if mode != "component" && mode != "subtree" {
        return Err(format!(
            "copy_from_workspace_v1: unsupported mode `{}` (expected `component` or `subtree`)",
            mode
        ));
    }
    let include_attachment = args.include_attachment.unwrap_or(true);

    if args.components.is_empty() {
        return Err("copy_from_workspace_v1 requires a non-empty args.components array".into());
    }
    let mut roots: Vec<String> = args
        .components
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Err("copy_from_workspace_v1 requires a non-empty args.components array".into());
    }

    let source_ws = get_workspace_clone(job, draft, &from_id)?;

    let mut src_by_name: std::collections::HashMap<&str, &Gen3dPlannedComponent> =
        std::collections::HashMap::new();
    for comp in source_ws.planned_components.iter() {
        src_by_name.insert(comp.name.as_str(), comp);
    }
    for root in roots.iter() {
        if !src_by_name.contains_key(root.as_str()) {
            return Err(format!(
                "Unknown component `{}` in source workspace `{}`",
                root, from_id
            ));
        }
    }

    let components_to_copy: std::collections::BTreeSet<String> = if mode == "subtree" {
        let mut children: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for comp in source_ws.planned_components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            children
                .entry(att.parent.clone())
                .or_default()
                .push(comp.name.clone());
        }
        for list in children.values_mut() {
            list.sort();
            list.dedup();
        }

        let mut out: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut stack: Vec<String> = roots.clone();
        while let Some(name) = stack.pop() {
            if !out.insert(name.clone()) {
                continue;
            }
            if let Some(kids) = children.get(&name) {
                for kid in kids.iter() {
                    stack.push(kid.clone());
                }
            }
        }
        out
    } else {
        roots.into_iter().collect()
    };

    let mut src_defs_by_id: std::collections::HashMap<u128, &ObjectDef> =
        std::collections::HashMap::new();
    for def in source_ws.defs.iter() {
        src_defs_by_id.insert(def.object_id, def);
    }

    let mut dst_comp_idx: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (idx, comp) in job.planned_components.iter().enumerate() {
        dst_comp_idx.insert(comp.name.clone(), idx);
    }

    let mut copied: Vec<String> = Vec::new();
    let assembly_rev_before = job.assembly_rev;

    for name in components_to_copy.iter() {
        let object_id = component_object_id_for_name(name);
        let Some(src_def) = src_defs_by_id.get(&object_id).copied() else {
            return Err(format!(
                "Source workspace `{}` missing object def for component `{}`",
                from_id, name
            ));
        };

        let Some(dst_def) = draft.defs.iter_mut().find(|d| d.object_id == object_id) else {
            return Err(format!(
                "Active workspace missing object def for component `{}`",
                name
            ));
        };
        *dst_def = src_def.clone();

        let Some(dst_idx) = dst_comp_idx.get(name).copied() else {
            return Err(format!(
                "Active workspace missing planned component entry for `{}`",
                name
            ));
        };
        let Some(src_comp) = src_by_name.get(name.as_str()).copied() else {
            return Err(format!(
                "Source workspace `{}` missing planned component entry for `{}`",
                from_id, name
            ));
        };

        let dst_comp = job
            .planned_components
            .get_mut(dst_idx)
            .ok_or_else(|| "Internal error: component index out of range".to_string())?;
        dst_comp.anchors = src_def.anchors.clone();
        if include_attachment {
            dst_comp.attach_to = src_comp.attach_to.clone();
        }

        copied.push(name.clone());
    }

    copied.sort();
    copied.dedup();

    if !copied.is_empty() {
        let Some(root_idx) = find_root_component_index(&job.planned_components) else {
            return Err("Internal error: no root component (missing attach_to=None).".into());
        };
        super::convert::resolve_planned_component_transforms(
            &mut job.planned_components,
            root_idx,
        )?;
        super::convert::sync_attachment_tree_to_defs(&job.planned_components, draft)?;
        super::convert::update_root_def_from_planned_components(
            &job.planned_components,
            &job.plan_collider,
            draft,
        );
        if let Some(dir) = job.step_dir_path() {
            super::artifacts::write_gen3d_assembly_snapshot(Some(dir), &job.planned_components);
        }
        job.assembly_rev = job.assembly_rev.saturating_add(1);
    }

    Ok(serde_json::json!({
        "ok": true,
        "version": 1,
        "from": from_id,
        "into": job.active_workspace_id(),
        "include_attachment": include_attachment,
        "assembly_rev_before": assembly_rev_before,
        "assembly_rev_after": job.assembly_rev,
        "copied_components": copied,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AspectDecision {
    Base,
    A,
    B,
    Conflict,
}

fn decide_aspect<T: PartialEq>(base: &T, a: &T, b: &T) -> AspectDecision {
    if a == b {
        if a == base {
            AspectDecision::Base
        } else {
            AspectDecision::A
        }
    } else if a == base {
        AspectDecision::B
    } else if b == base {
        AspectDecision::A
    } else {
        AspectDecision::Conflict
    }
}

pub(super) fn merge_workspace_v1(
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: MergeWorkspacesArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid merge_workspace_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported merge_workspace_v1 version {} (expected 1)",
            args.version
        ));
    }

    let base_id = args.base.trim().to_string();
    let a_id = args.a.trim().to_string();
    let b_id = args.b.trim().to_string();
    if base_id.is_empty() || a_id.is_empty() || b_id.is_empty() {
        return Err("merge_workspace_v1 requires non-empty args.base, args.a, and args.b".into());
    }

    let base_ws = get_workspace_clone(job, draft, &base_id)?;
    let a_ws = get_workspace_clone(job, draft, &a_id)?;
    let b_ws = get_workspace_clone(job, draft, &b_id)?;

    if base_ws.plan_hash.trim() != a_ws.plan_hash.trim()
        || base_ws.plan_hash.trim() != b_ws.plan_hash.trim()
    {
        return Err("merge_workspace_v1 requires base/a/b to have the same plan_hash".into());
    }

    let set_names = |ws: &Gen3dAgentWorkspace| -> std::collections::BTreeSet<String> {
        ws.planned_components
            .iter()
            .map(|c| c.name.clone())
            .collect()
    };
    let base_names = set_names(&base_ws);
    let a_names = set_names(&a_ws);
    let b_names = set_names(&b_ws);
    if base_names != a_names || base_names != b_names {
        return Err(
            "merge_workspace_v1 requires base/a/b to have the same planned component set".into(),
        );
    }

    let max_components = args.max_components.unwrap_or(128).max(1) as usize;

    let mut base_defs_by_id: std::collections::HashMap<u128, &ObjectDef> =
        std::collections::HashMap::new();
    for def in base_ws.defs.iter() {
        base_defs_by_id.insert(def.object_id, def);
    }
    let mut a_defs_by_id: std::collections::HashMap<u128, &ObjectDef> =
        std::collections::HashMap::new();
    for def in a_ws.defs.iter() {
        a_defs_by_id.insert(def.object_id, def);
    }
    let mut b_defs_by_id: std::collections::HashMap<u128, &ObjectDef> =
        std::collections::HashMap::new();
    for def in b_ws.defs.iter() {
        b_defs_by_id.insert(def.object_id, def);
    }

    let mut base_comp_by_name: std::collections::HashMap<&str, &Gen3dPlannedComponent> =
        std::collections::HashMap::new();
    for comp in base_ws.planned_components.iter() {
        base_comp_by_name.insert(comp.name.as_str(), comp);
    }
    let mut a_comp_by_name: std::collections::HashMap<&str, &Gen3dPlannedComponent> =
        std::collections::HashMap::new();
    for comp in a_ws.planned_components.iter() {
        a_comp_by_name.insert(comp.name.as_str(), comp);
    }
    let mut b_comp_by_name: std::collections::HashMap<&str, &Gen3dPlannedComponent> =
        std::collections::HashMap::new();
    for comp in b_ws.planned_components.iter() {
        b_comp_by_name.insert(comp.name.as_str(), comp);
    }

    let mut merged = base_ws.clone();

    let mut conflicts: Vec<serde_json::Value> = Vec::new();
    let mut applied_geometry = 0u32;
    let mut applied_anchors = 0u32;
    let mut applied_attachment = 0u32;
    let mut applied_animations = 0u32;

    let mut truncated = false;
    let mut processed = 0usize;
    for name in base_names.into_iter() {
        if processed >= max_components {
            truncated = true;
            break;
        }
        processed += 1;

        let object_id = component_object_id_for_name(&name);
        let base_def = base_defs_by_id.get(&object_id).copied().ok_or_else(|| {
            format!(
                "Base workspace `{}` missing def for component `{}`",
                base_id, name
            )
        })?;
        let a_def = a_defs_by_id
            .get(&object_id)
            .copied()
            .ok_or_else(|| format!("Workspace `{}` missing def for component `{}`", a_id, name))?;
        let b_def = b_defs_by_id
            .get(&object_id)
            .copied()
            .ok_or_else(|| format!("Workspace `{}` missing def for component `{}`", b_id, name))?;

        let base_comp = base_comp_by_name
            .get(name.as_str())
            .copied()
            .ok_or_else(|| "Internal error: component map desync".to_string())?;
        let a_comp = a_comp_by_name
            .get(name.as_str())
            .copied()
            .ok_or_else(|| "Internal error: component map desync".to_string())?;
        let b_comp = b_comp_by_name
            .get(name.as_str())
            .copied()
            .ok_or_else(|| "Internal error: component map desync".to_string())?;

        let base_geom = geometry_fp(base_def);
        let a_geom = geometry_fp(a_def);
        let b_geom = geometry_fp(b_def);
        let base_anchors = anchors_fp(base_def);
        let a_anchors = anchors_fp(a_def);
        let b_anchors = anchors_fp(b_def);

        let base_attach = attachment_core_fp(base_comp);
        let a_attach = attachment_core_fp(a_comp);
        let b_attach = attachment_core_fp(b_comp);
        let base_anim = attachment_anims_fp(base_comp);
        let a_anim = attachment_anims_fp(a_comp);
        let b_anim = attachment_anims_fp(b_comp);

        let geometry_decision = decide_aspect(&base_geom, &a_geom, &b_geom);
        let anchors_decision = decide_aspect(&base_anchors, &a_anchors, &b_anchors);
        let attach_decision = decide_aspect(&base_attach, &a_attach, &b_attach);
        let anim_decision = decide_aspect(&base_anim, &a_anim, &b_anim);

        let mut any_conflict = false;
        let mut conflict_obj = serde_json::Map::new();
        conflict_obj.insert("component".into(), serde_json::Value::String(name.clone()));

        let geometry_conflict = geometry_decision == AspectDecision::Conflict;
        let anchors_conflict = anchors_decision == AspectDecision::Conflict;
        let attach_conflict = attach_decision == AspectDecision::Conflict;
        let anim_conflict = anim_decision == AspectDecision::Conflict;
        if geometry_conflict || anchors_conflict || attach_conflict || anim_conflict {
            any_conflict = true;
            conflict_obj.insert(
                "geometry".into(),
                serde_json::Value::Bool(geometry_conflict),
            );
            conflict_obj.insert("anchors".into(), serde_json::Value::Bool(anchors_conflict));
            conflict_obj.insert(
                "attachment".into(),
                serde_json::Value::Bool(attach_conflict),
            );
            conflict_obj.insert("animations".into(), serde_json::Value::Bool(anim_conflict));
        }

        let mut merged_def = base_def.clone();
        match geometry_decision {
            AspectDecision::A => {
                if a_geom != base_geom {
                    merged_def = a_def.clone();
                    applied_geometry = applied_geometry.saturating_add(1);
                }
            }
            AspectDecision::B => {
                if b_geom != base_geom {
                    merged_def = b_def.clone();
                    applied_geometry = applied_geometry.saturating_add(1);
                }
            }
            _ => {}
        }
        match anchors_decision {
            AspectDecision::A => {
                if a_anchors != base_anchors {
                    merged_def.anchors = a_def.anchors.clone();
                    applied_anchors = applied_anchors.saturating_add(1);
                }
            }
            AspectDecision::B => {
                if b_anchors != base_anchors {
                    merged_def.anchors = b_def.anchors.clone();
                    applied_anchors = applied_anchors.saturating_add(1);
                }
            }
            _ => {}
        }

        if let Some(def_out) = merged.defs.iter_mut().find(|d| d.object_id == object_id) {
            *def_out = merged_def.clone();
        }

        if let Some(comp_out) = merged
            .planned_components
            .iter_mut()
            .find(|c| c.name == name)
        {
            comp_out.anchors = merged_def.anchors.clone();

            match attach_decision {
                AspectDecision::A => {
                    if a_attach != base_attach {
                        comp_out.attach_to = a_comp.attach_to.clone();
                        applied_attachment = applied_attachment.saturating_add(1);
                    }
                }
                AspectDecision::B => {
                    if b_attach != base_attach {
                        comp_out.attach_to = b_comp.attach_to.clone();
                        applied_attachment = applied_attachment.saturating_add(1);
                    }
                }
                _ => {}
            }

            match anim_decision {
                AspectDecision::A => {
                    if a_anim != base_anim {
                        if let (Some(dst), Some(src_att)) =
                            (comp_out.attach_to.as_mut(), a_comp.attach_to.as_ref())
                        {
                            dst.animations = src_att.animations.clone();
                        }
                        applied_animations = applied_animations.saturating_add(1);
                    }
                }
                AspectDecision::B => {
                    if b_anim != base_anim {
                        if let (Some(dst), Some(src_att)) =
                            (comp_out.attach_to.as_mut(), b_comp.attach_to.as_ref())
                        {
                            dst.animations = src_att.animations.clone();
                        }
                        applied_animations = applied_animations.saturating_add(1);
                    }
                }
                _ => {}
            }
        }

        if any_conflict {
            conflicts.push(serde_json::Value::Object(conflict_obj));
        }
    }

    if applied_geometry + applied_anchors + applied_attachment + applied_animations > 0 {
        let Some(root_idx) = find_root_component_index(&merged.planned_components) else {
            return Err("Internal error: no root component (missing attach_to=None).".into());
        };
        let mut merged_draft = Gen3dDraft { defs: merged.defs };
        super::convert::resolve_planned_component_transforms(
            &mut merged.planned_components,
            root_idx,
        )?;
        super::convert::sync_attachment_tree_to_defs(
            &merged.planned_components,
            &mut merged_draft,
        )?;
        super::convert::update_root_def_from_planned_components(
            &merged.planned_components,
            &merged.plan_collider,
            &mut merged_draft,
        );
        merged.defs = merged_draft.defs;
        merged.assembly_rev = merged.assembly_rev.saturating_add(1);
    }

    let mut workspace_id = args
        .output_workspace_id
        .take()
        .unwrap_or_else(|| "merge".to_string())
        .trim()
        .to_string();
    if workspace_id.is_empty() {
        workspace_id = "merge".to_string();
    }
    if workspace_id.trim() == job.active_workspace_id().trim()
        || job.agent.workspaces.contains_key(&workspace_id)
    {
        workspace_id = format!("ws{}", job.agent.next_workspace_seq);
    }
    job.agent.next_workspace_seq = job.agent.next_workspace_seq.saturating_add(1);

    if workspace_id.trim() == job.active_workspace_id().trim() {
        return Err(
            "merge_workspace_v1 output_workspace_id must not be the active workspace".into(),
        );
    }
    if job.agent.workspaces.contains_key(&workspace_id) {
        return Err(format!(
            "merge_workspace_v1 workspace_id already exists: `{workspace_id}`"
        ));
    }

    let name = args
        .output_name
        .unwrap_or_else(|| workspace_id.clone())
        .trim()
        .to_string();
    merged.name = if name.is_empty() {
        workspace_id.clone()
    } else {
        name
    };

    job.agent.workspaces.insert(workspace_id.clone(), merged);

    Ok(serde_json::json!({
        "ok": true,
        "version": 1,
        "workspace_id": workspace_id,
        "truncated": truncated,
        "conflicts": conflicts,
        "applied": {
            "geometry": applied_geometry,
            "anchors": applied_anchors,
            "attachment": applied_attachment,
            "animations": applied_animations,
        },
    }))
}

pub(crate) fn gen3d_create_workspace_from_api(
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    create_workspace_v1(job, draft, args_json)
}

pub(crate) fn gen3d_delete_workspace_from_api(
    job: &mut Gen3dAiJob,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    delete_workspace_v1(job, args_json)
}

pub(crate) fn gen3d_set_active_workspace_from_api(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    set_active_workspace_v1(job, draft, args_json)
}

pub(crate) fn gen3d_diff_workspaces_from_api(
    job: &Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    diff_workspaces_v1(job, draft, args_json)
}

pub(crate) fn gen3d_copy_from_workspace_from_api(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    copy_from_workspace_v1(job, draft, args_json)
}

pub(crate) fn gen3d_merge_workspace_from_api(
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    merge_workspace_v1(job, draft, args_json)
}
