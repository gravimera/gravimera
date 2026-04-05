use bevy::prelude::*;
use serde::Deserialize;
use uuid::Uuid;

use crate::object::registry::{
    builtin_object_id, MeshKey, ObjectDef, ObjectPartDef, ObjectPartKind, PartAnimationDef,
    PrimitiveParams, PrimitiveVisualDef,
};

use super::super::state::Gen3dDraft;
use super::artifacts::append_gen3d_jsonl_artifact;
use super::job::Gen3dAgentSnapshot;
use super::{Gen3dAiJob, Gen3dPlannedComponent};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotArgsJsonV1 {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    snapshot_id: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreSnapshotArgsJsonV1 {
    #[serde(default)]
    version: u32,
    snapshot_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListSnapshotsArgsJsonV1 {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    max_items: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffSnapshotsArgsJsonV1 {
    #[serde(default)]
    version: u32,
    a: String,
    b: String,
    #[serde(default)]
    max_components: Option<u32>,
}

fn component_object_id_for_name(name: &str) -> u128 {
    builtin_object_id(&format!("gravimera/gen3d/component/{}", name))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn next_snapshot_id(job: &mut Gen3dAiJob, requested: Option<String>) -> String {
    let requested = requested
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if let Some(id) = requested {
        return id;
    }

    // Deterministic within a session: snap1, snap2, ...
    loop {
        let id = format!("snap{}", job.agent.next_snapshot_seq);
        job.agent.next_snapshot_seq = job.agent.next_snapshot_seq.saturating_add(1);
        if !job.agent.snapshots.contains_key(&id) {
            return id;
        }
    }
}

fn capture_snapshot_state(
    job: &Gen3dAiJob,
    draft: &Gen3dDraft,
    label: String,
) -> Gen3dAgentSnapshot {
    Gen3dAgentSnapshot {
        workspace_id: job.active_workspace_id().to_string(),
        label,
        created_at_ms: now_ms(),
        defs: draft.defs.clone(),
        planned_components: job.planned_components.clone(),
        plan_hash: job.plan_hash.clone(),
        assembly_rev: job.assembly_rev,
        assembly_notes: job.assembly_notes.clone(),
        plan_collider: job.plan_collider.clone(),
        rig_move_cycle_m: job.rig_move_cycle_m,
        motion_authoring: job.motion_authoring.clone(),
        motion_authoring_by_channel: job.motion_authoring_by_channel.clone(),
        reuse_groups: job.reuse_groups.clone(),
        reuse_group_warnings: job.reuse_group_warnings.clone(),
    }
}

fn restore_snapshot_state(job: &mut Gen3dAiJob, draft: &mut Gen3dDraft, snap: &Gen3dAgentSnapshot) {
    draft.defs = snap.defs.clone();
    job.planned_components = snap.planned_components.clone();
    job.plan_hash = snap.plan_hash.clone();
    job.assembly_rev = snap.assembly_rev;
    job.assembly_notes = snap.assembly_notes.clone();
    job.plan_collider = snap.plan_collider.clone();
    job.rig_move_cycle_m = snap.rig_move_cycle_m;
    job.motion_authoring = snap.motion_authoring.clone();
    job.motion_authoring_by_channel = snap.motion_authoring_by_channel.clone();
    job.reuse_groups = snap.reuse_groups.clone();
    job.reuse_group_warnings = snap.reuse_group_warnings.clone();

    let n = job.planned_components.len();
    job.regen_per_component.resize(n, 0);
    job.component_attempts.resize(n, 0);
    job.component_last_errors.resize(n, None);
}

pub(super) fn snapshot_v1(
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: SnapshotArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid snapshot_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported snapshot_v1 version {} (expected 1)",
            args.version
        ));
    }

    let snapshot_id = next_snapshot_id(job, args.snapshot_id.take());
    let label = args
        .label
        .unwrap_or_else(|| snapshot_id.clone())
        .trim()
        .to_string();

    let snap = capture_snapshot_state(job, draft, label.clone());
    job.agent
        .snapshots
        .insert(snapshot_id.clone(), snap.clone());

    if let Some(run_dir) = job.run_dir_path() {
        append_gen3d_jsonl_artifact(
            Some(run_dir),
            "snapshots.jsonl",
            &serde_json::json!({
                "kind": "snapshot_created",
                "snapshot_id": snapshot_id,
                "label": label,
                "created_at_ms": snap.created_at_ms,
                "active_workspace": job.active_workspace_id(),
                "workspace_id": snap.workspace_id,
                "plan_hash": snap.plan_hash,
                "assembly_rev": snap.assembly_rev,
                "defs": snap.defs.len(),
                "components": snap.planned_components.len(),
            }),
        );
    }

    Ok(serde_json::json!({
        "ok": true,
        "version": 1,
        "snapshot_id": snapshot_id,
        "label": label,
        "created_at_ms": snap.created_at_ms,
        "active_workspace": job.active_workspace_id(),
        "workspace_id": snap.workspace_id,
        "plan_hash": snap.plan_hash,
        "assembly_rev": snap.assembly_rev,
        "defs": snap.defs.len(),
        "components": snap.planned_components.len(),
    }))
}

pub(super) fn list_snapshots_v1(
    job: &Gen3dAiJob,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: ListSnapshotsArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid list_snapshots_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported list_snapshots_v1 version {} (expected 1)",
            args.version
        ));
    }

    let max_items = args.max_items.unwrap_or(200).max(1) as usize;
    let mut ids: Vec<&String> = job.agent.snapshots.keys().collect();
    ids.sort();

    let mut out = Vec::new();
    let mut truncated = false;
    for id in ids {
        if out.len() >= max_items {
            truncated = true;
            break;
        }
        let snap = job
            .agent
            .snapshots
            .get(id)
            .ok_or_else(|| "Internal error: snapshot map desync".to_string())?;
        out.push(serde_json::json!({
            "snapshot_id": id.as_str(),
            "workspace_id": snap.workspace_id.as_str(),
            "label": snap.label.as_str(),
            "created_at_ms": snap.created_at_ms,
            "plan_hash": snap.plan_hash.as_str(),
            "assembly_rev": snap.assembly_rev,
            "defs": snap.defs.len(),
            "components": snap.planned_components.len(),
        }));
    }

    Ok(serde_json::json!({
        "ok": true,
        "version": 1,
        "snapshots": out,
        "truncated": truncated,
    }))
}

pub(super) fn restore_snapshot_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if job.is_running() {
        return Err(
            "restore_snapshot_v1 is only allowed when the Gen3D run is not running.".into(),
        );
    }

    let mut args: RestoreSnapshotArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid restore_snapshot_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported restore_snapshot_v1 version {} (expected 1)",
            args.version
        ));
    }

    let snapshot_id = args.snapshot_id.trim();
    if snapshot_id.is_empty() {
        return Err("Missing snapshot_id".into());
    }

    let before = job.assembly_rev;
    let Some(snap) = job.agent.snapshots.get(snapshot_id).cloned() else {
        return Err(format!("Unknown snapshot_id `{snapshot_id}`"));
    };
    if snap.workspace_id.trim() != job.active_workspace_id().trim() {
        return Err(format!(
            "Snapshot `{snapshot_id}` belongs to workspace `{}`; switch active workspace first.",
            snap.workspace_id
        ));
    }

    restore_snapshot_state(job, draft, &snap);

    if let Some(run_dir) = job.run_dir_path() {
        append_gen3d_jsonl_artifact(
            Some(run_dir),
            "snapshots.jsonl",
            &serde_json::json!({
                "kind": "snapshot_restored",
                "snapshot_id": snapshot_id,
                "label": snap.label,
                "created_at_ms": snap.created_at_ms,
                "active_workspace": job.active_workspace_id(),
                "workspace_id": snap.workspace_id,
                "assembly_rev_before": before,
                "assembly_rev_after": job.assembly_rev,
                "plan_hash": job.plan_hash,
            }),
        );
    }

    Ok(serde_json::json!({
        "ok": true,
        "version": 1,
        "snapshot_id": snapshot_id,
        "assembly_rev_before": before,
        "assembly_rev_after": job.assembly_rev,
        "plan_hash": job.plan_hash,
        "defs": draft.defs.len(),
        "components": job.planned_components.len(),
    }))
}

#[derive(Clone, Debug, PartialEq)]
struct PrimitivePartFp {
    part_id: Option<u128>,
    render_priority: Option<i32>,
    mesh: Option<MeshKey>,
    params: Option<PrimitiveParams>,
    color: Option<[f32; 4]>,
    unlit: Option<bool>,
    deform_id: Option<u128>,
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

fn primitive_part_fp(part: &ObjectPartDef) -> Option<PrimitivePartFp> {
    let ObjectPartKind::Primitive { primitive } = &part.kind else {
        return None;
    };
    let (mesh, params, color, unlit, deform_id) = match primitive {
        PrimitiveVisualDef::Primitive {
            mesh,
            params,
            color,
            unlit,
            deform,
        } => {
            let c = color.to_srgba();
            (
                Some(*mesh),
                params.clone(),
                Some([c.red, c.green, c.blue, c.alpha]),
                Some(*unlit),
                deform.as_ref().map(crate::object::deform::deform_cache_id),
            )
        }
        PrimitiveVisualDef::Mesh { .. } => (None, None, None, None, None),
    };
    Some(PrimitivePartFp {
        part_id: part.part_id,
        render_priority: part.render_priority,
        mesh,
        params,
        color,
        unlit,
        deform_id,
        translation: part.transform.translation,
        rotation: part.transform.rotation,
        scale: part.transform.scale,
    })
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

fn geometry_fp(def: &ObjectDef) -> Vec<PrimitivePartFp> {
    let mut parts: Vec<PrimitivePartFp> = def.parts.iter().filter_map(primitive_part_fp).collect();
    parts.sort_by_key(|p| p.part_id.unwrap_or(0));
    parts
}

pub(super) fn diff_snapshots_v1(
    job: &Gen3dAiJob,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: DiffSnapshotsArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid diff_snapshots_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported diff_snapshots_v1 version {} (expected 1)",
            args.version
        ));
    }

    let a_id = args.a.trim();
    let b_id = args.b.trim();
    if a_id.is_empty() || b_id.is_empty() {
        return Err("diff_snapshots_v1 requires non-empty args.a and args.b".into());
    }
    let Some(a) = job.agent.snapshots.get(a_id) else {
        return Err(format!("Unknown snapshot_id `{a_id}`"));
    };
    let Some(b) = job.agent.snapshots.get(b_id) else {
        return Err(format!("Unknown snapshot_id `{b_id}`"));
    };

    let max_components = args.max_components.unwrap_or(128).max(1) as usize;

    let mut a_defs: std::collections::HashMap<u128, &ObjectDef> = std::collections::HashMap::new();
    for def in a.defs.iter() {
        a_defs.insert(def.object_id, def);
    }
    let mut b_defs: std::collections::HashMap<u128, &ObjectDef> = std::collections::HashMap::new();
    for def in b.defs.iter() {
        b_defs.insert(def.object_id, def);
    }

    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for c in a.planned_components.iter() {
        names.insert(c.name.clone());
    }
    for c in b.planned_components.iter() {
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

        let a_comp = a.planned_components.iter().find(|c| c.name == name);
        let b_comp = b.planned_components.iter().find(|c| c.name == name);

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
            // Attachment fingerprint includes an `animations` field.
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
        "a": { "snapshot_id": a_id, "label": a.label, "workspace_id": a.workspace_id, "plan_hash": a.plan_hash, "assembly_rev": a.assembly_rev, "components": a.planned_components.len(), "defs": a.defs.len() },
        "b": { "snapshot_id": b_id, "label": b.label, "workspace_id": b.workspace_id, "plan_hash": b.plan_hash, "assembly_rev": b.assembly_rev, "components": b.planned_components.len(), "defs": b.defs.len() },
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

pub(crate) fn gen3d_snapshot_from_api(
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    snapshot_v1(job, draft, args_json)
}

pub(crate) fn gen3d_list_snapshots_from_api(
    job: &Gen3dAiJob,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    list_snapshots_v1(job, args_json)
}

pub(crate) fn gen3d_diff_snapshots_from_api(
    job: &Gen3dAiJob,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    diff_snapshots_v1(job, args_json)
}

pub(crate) fn gen3d_restore_snapshot_from_api(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    restore_snapshot_v1(job, draft, args_json)
}
