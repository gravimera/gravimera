use bevy::prelude::*;
use sha2::{Digest, Sha256};

use crate::gen3d::agent::tools::{
    TOOL_ID_APPLY_DRAFT_OPS, TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE,
    TOOL_ID_COPY_FROM_WORKSPACE, TOOL_ID_DETACH_COMPONENT, TOOL_ID_LLM_GENERATE_COMPONENT,
    TOOL_ID_LLM_GENERATE_COMPONENTS, TOOL_ID_LLM_GENERATE_MOTION_AUTHORING,
    TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_MIRROR_COMPONENT,
    TOOL_ID_MIRROR_COMPONENT_SUBTREE, TOOL_ID_RENDER_PREVIEW, TOOL_ID_RESTORE_SNAPSHOT,
    TOOL_ID_SET_DESCRIPTOR_META, TOOL_ID_SMOKE_CHECK, TOOL_ID_VALIDATE,
};
use crate::gen3d::agent::Gen3dToolResultJsonV1;
use crate::object::registry::{PartAnimationDef, PartAnimationDriver};

use super::super::state::Gen3dDraft;
use super::Gen3dAiJob;

pub(super) fn note_observable_tool_result(job: &mut Gen3dAiJob, result: &Gen3dToolResultJsonV1) {
    if !result.ok {
        return;
    }

    if matches!(
        result.tool_id.as_str(),
        TOOL_ID_RENDER_PREVIEW | TOOL_ID_VALIDATE | TOOL_ID_SMOKE_CHECK
    ) {
        job.agent.step_had_observable_output = true;
    }
}

pub(super) fn step_had_no_progress_try(tool_results: &[Gen3dToolResultJsonV1]) -> bool {
    tool_results.iter().any(|result| {
        matches!(
            result.tool_id.as_str(),
            TOOL_ID_APPLY_DRAFT_OPS
                | TOOL_ID_COPY_COMPONENT
                | TOOL_ID_COPY_COMPONENT_SUBTREE
                | TOOL_ID_COPY_FROM_WORKSPACE
                | TOOL_ID_DETACH_COMPONENT
                | TOOL_ID_LLM_GENERATE_COMPONENT
                | TOOL_ID_LLM_GENERATE_COMPONENTS
                | TOOL_ID_LLM_GENERATE_MOTION_AUTHORING
                | TOOL_ID_LLM_GENERATE_PLAN
                | TOOL_ID_LLM_REVIEW_DELTA
                | TOOL_ID_MIRROR_COMPONENT
                | TOOL_ID_MIRROR_COMPONENT_SUBTREE
                | TOOL_ID_RESTORE_SNAPSHOT
                | TOOL_ID_SET_DESCRIPTOR_META
        )
    })
}

fn quantize_f32_1e4(v: f32) -> i32 {
    if !v.is_finite() {
        return i32::MIN;
    }
    (v * 10000.0).round() as i32
}

fn motion_values_digest(job: &Gen3dAiJob) -> String {
    fn driver_tag(driver: PartAnimationDriver) -> u8 {
        match driver {
            PartAnimationDriver::Always => 0,
            PartAnimationDriver::MovePhase => 1,
            PartAnimationDriver::MoveDistance => 2,
            PartAnimationDriver::AttackTime => 3,
        }
    }

    let mut indices: Vec<usize> = (0..job.planned_components.len()).collect();
    indices.sort_by_key(|&i| job.planned_components[i].name.clone());

    let mut hasher = Sha256::new();
    for idx in indices {
        let comp = &job.planned_components[idx];
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        if att.animations.is_empty() {
            continue;
        }

        let mut slot_indices: Vec<usize> = (0..att.animations.len()).collect();
        slot_indices.sort_by_key(|&i| att.animations[i].channel.to_string());

        for slot_idx in slot_indices {
            let slot = &att.animations[slot_idx];
            hasher.update(comp.name.as_bytes());
            hasher.update(b"|");
            hasher.update(att.parent.as_bytes());
            hasher.update(b"|");
            hasher.update(slot.channel.as_bytes());
            hasher.update(b"|");
            hasher.update([driver_tag(slot.spec.driver)]);
            hasher.update(quantize_f32_1e4(slot.spec.speed_scale).to_le_bytes());
            hasher.update(quantize_f32_1e4(slot.spec.time_offset_units).to_le_bytes());

            match &slot.spec.clip {
                PartAnimationDef::Loop {
                    duration_secs,
                    keyframes,
                } => {
                    hasher.update(b"loop");
                    hasher.update(quantize_f32_1e4(*duration_secs).to_le_bytes());
                    hasher.update((keyframes.len() as u32).to_le_bytes());
                    for k in keyframes {
                        hasher.update(quantize_f32_1e4(k.time_secs).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.z).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.z).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.w).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.z).to_le_bytes());
                    }
                }
                PartAnimationDef::Once {
                    duration_secs,
                    keyframes,
                } => {
                    hasher.update(b"once");
                    hasher.update(quantize_f32_1e4(*duration_secs).to_le_bytes());
                    hasher.update((keyframes.len() as u32).to_le_bytes());
                    for k in keyframes {
                        hasher.update(quantize_f32_1e4(k.time_secs).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.z).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.z).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.w).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.z).to_le_bytes());
                    }
                }
                PartAnimationDef::PingPong {
                    duration_secs,
                    keyframes,
                } => {
                    hasher.update(b"ping_pong");
                    hasher.update(quantize_f32_1e4(*duration_secs).to_le_bytes());
                    hasher.update((keyframes.len() as u32).to_le_bytes());
                    for k in keyframes {
                        hasher.update(quantize_f32_1e4(k.time_secs).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.translation.z).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.z).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.rotation.w).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.x).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.y).to_le_bytes());
                        hasher.update(quantize_f32_1e4(k.delta.scale.z).to_le_bytes());
                    }
                }
                PartAnimationDef::Spin {
                    axis,
                    radians_per_unit,
                } => {
                    hasher.update(b"spin");
                    hasher.update(quantize_f32_1e4(axis.x).to_le_bytes());
                    hasher.update(quantize_f32_1e4(axis.y).to_le_bytes());
                    hasher.update(quantize_f32_1e4(axis.z).to_le_bytes());
                    hasher.update(quantize_f32_1e4(*radians_per_unit).to_le_bytes());
                }
            }

            hasher.update(b"\n");
        }
    }

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{:02x}", b));
    }
    format!("sha256:{hex}")
}

pub(super) fn compute_agent_state_hash(job: &Gen3dAiJob, draft: &Gen3dDraft) -> String {
    let mut summary = super::build_gen3d_scene_graph_summary(
        "",
        0,
        0,
        &job.plan_hash,
        // No-progress guard should reflect *actual* assembly state, not revision counters.
        // Some tool results can bump `assembly_rev` even when the assembled draft doesn't change.
        0,
        &job.planned_components,
        draft,
    );
    if let serde_json::Value::Object(map) = &mut summary {
        // The scene graph summary intentionally omits per-keyframe delta values for size reasons.
        // However, the no-progress guard must treat motion value changes as progress so the agent
        // can iterate on animation fixes without being stopped prematurely.
        map.insert(
            "motion_values_digest".into(),
            serde_json::Value::String(motion_values_digest(job)),
        );
    }
    let text = serde_json::to_string(&summary).unwrap_or_else(|_| summary.to_string());
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{:02x}", b));
    }
    format!("sha256:{hex}")
}

pub(super) fn build_component_subset_workspace_defs(
    source_defs: &[crate::object::registry::ObjectDef],
    include_components: &[String],
) -> Result<Vec<crate::object::registry::ObjectDef>, String> {
    use crate::object::registry::{AttachmentDef, ObjectDef, ObjectPartDef, ObjectPartKind};

    let root_id = super::super::gen3d_draft_object_id();
    let mut by_id: std::collections::HashMap<u128, ObjectDef> = std::collections::HashMap::new();
    for def in source_defs.iter().cloned() {
        by_id.insert(def.object_id, def);
    }

    let mut roots: Vec<u128> = Vec::new();
    for name in include_components {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let object_id = crate::object::registry::builtin_object_id(&format!(
            "gravimera/gen3d/component/{name}"
        ));
        if !by_id.contains_key(&object_id) {
            return Err(format!(
                "Unknown component `{name}` (no matching object def in source draft)."
            ));
        }
        roots.push(object_id);
    }
    if roots.is_empty() {
        return Ok(source_defs.to_vec());
    }

    // Collect reachable defs from the requested roots.
    let mut reachable: std::collections::HashSet<u128> = std::collections::HashSet::new();
    let mut stack: Vec<u128> = roots.clone();
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        let Some(def) = by_id.get(&id) else {
            continue;
        };
        for part in def.parts.iter() {
            if let ObjectPartKind::ObjectRef { object_id } = part.kind {
                stack.push(object_id);
            }
        }
    }

    // Lay out the requested roots side-by-side so the agent can compare multiple variants.
    let margin = 0.6f32;
    let mut centers: Vec<f32> = Vec::with_capacity(roots.len());
    let mut cursor_x = 0.0f32;
    for root in &roots {
        let size = by_id
            .get(root)
            .map(|d| d.size)
            .unwrap_or(Vec3::ONE)
            .abs()
            .max(Vec3::splat(0.01));
        let half_x = size.x * 0.5;
        cursor_x += half_x;
        centers.push(cursor_x);
        cursor_x += half_x + margin;
    }

    // Recenter layout around origin.
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for (idx, root) in roots.iter().enumerate() {
        let size = by_id
            .get(root)
            .map(|d| d.size)
            .unwrap_or(Vec3::ONE)
            .abs()
            .max(Vec3::splat(0.01));
        let half_x = size.x * 0.5;
        let center_x = centers.get(idx).copied().unwrap_or(0.0);
        min_x = min_x.min(center_x - half_x);
        max_x = max_x.max(center_x + half_x);
    }
    let shift_x = (min_x + max_x) * 0.5;
    for x in centers.iter_mut() {
        *x -= shift_x;
    }

    let mut root_parts: Vec<ObjectPartDef> = Vec::with_capacity(roots.len());
    for (idx, object_id) in roots.iter().copied().enumerate() {
        let x = centers.get(idx).copied().unwrap_or(0.0);
        let attachment = AttachmentDef {
            parent_anchor: "origin".into(),
            child_anchor: "origin".into(),
        };
        let part = ObjectPartDef::object_ref(
            object_id,
            Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
        )
        .with_attachment(attachment);
        root_parts.push(part);
    }

    // Root def: keep it simple for preview; disable mobility/attack/collider.
    let mut root_def = by_id.remove(&root_id).unwrap_or_else(|| ObjectDef {
        object_id: root_id,
        label: "gen3d_draft".into(),
        size: Vec3::ONE,
        ground_origin_y: None,
        collider: crate::object::registry::ColliderProfile::None,
        interaction: crate::object::registry::ObjectInteraction::none(),
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
    });
    root_def.parts = root_parts;
    root_def.mobility = None;
    root_def.attack = None;
    root_def.collider = crate::object::registry::ColliderProfile::None;

    // Size: approximate from included children.
    let mut max_y = 0.0f32;
    let mut max_z = 0.0f32;
    for id in reachable.iter() {
        if let Some(def) = by_id.get(id) {
            let size = def.size.abs().max(Vec3::splat(0.01));
            max_y = max_y.max(size.y);
            max_z = max_z.max(size.z);
        }
    }
    let width = (max_x - min_x).abs().max(0.1);
    root_def.size = Vec3::new(width, max_y.max(0.1), max_z.max(0.1));

    // Final defs list: reachable components + root.
    let mut out: Vec<ObjectDef> = Vec::new();
    out.reserve(reachable.len() + 1);
    for (id, def) in by_id.into_iter() {
        if id == root_id {
            continue;
        }
        if reachable.contains(&id) {
            out.push(def);
        }
    }
    out.push(root_def);

    Ok(out)
}

pub(super) fn sanitize_prefix(prefix: &str) -> String {
    let mut out = String::new();
    for ch in prefix.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
        if out.len() >= 48 {
            break;
        }
    }
    if out.is_empty() {
        "artifact".into()
    } else {
        out
    }
}

pub(super) fn truncate_json_for_log(value: &serde_json::Value, max_chars: usize) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".into());
    let mut out = String::new();
    for ch in text.chars() {
        if ch == '\n' || ch == '\r' || ch == '\t' {
            out.push(' ');
        } else {
            out.push(ch);
        }
        if out.chars().count() >= max_chars {
            out.push_str("…");
            break;
        }
    }
    out
}
