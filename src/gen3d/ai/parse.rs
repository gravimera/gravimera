use bevy::log::debug;

use super::super::GEN3D_MAX_PARTS;
use super::artifacts::write_gen3d_json_artifact;
use super::schema::{
    AiDescriptorMetaJsonV1, AiDraftJsonV1, AiMotionAuthoringJsonV1, AiPlanJsonV1,
    AiPromptIntentJsonV1, AiReviewDeltaJsonV1,
};

fn normalize_ai_nullable_string_field_allowing_array(
    json_value: &mut serde_json::Value,
    field_name: &str,
    context: &str,
) {
    let Some(obj) = json_value.as_object_mut() else {
        return;
    };

    let Some(value) = obj.get_mut(field_name) else {
        return;
    };

    let trimmed_context = context.trim();
    let trimmed_field_name = field_name.trim();

    match value {
        serde_json::Value::Array(arr) => {
            let mut parts: Vec<String> = Vec::new();
            for item in arr.iter() {
                let Some(raw) = item.as_str() else {
                    continue;
                };
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    continue;
                }
                parts.push(trimmed.to_string());
            }

            if parts.is_empty() {
                *value = serde_json::Value::Null;
                debug!(
                    "Gen3D: normalized `{context}` `{field}` array to null (empty after trim)",
                    context = trimmed_context,
                    field = trimmed_field_name,
                );
                return;
            }

            *value = serde_json::Value::String(parts.join("\n"));
            debug!(
                "Gen3D: normalized `{context}` `{field}` array into string",
                context = trimmed_context,
                field = trimmed_field_name,
            );
        }
        serde_json::Value::String(s) => {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                *value = serde_json::Value::Null;
                debug!(
                    "Gen3D: normalized `{context}` `{field}` string to null (empty after trim)",
                    context = trimmed_context,
                    field = trimmed_field_name,
                );
                return;
            }
            if trimmed != *s {
                *s = trimmed;
                debug!(
                    "Gen3D: trimmed `{context}` `{field}` string",
                    context = trimmed_context,
                    field = trimmed_field_name,
                );
            }
        }
        serde_json::Value::Null => {}
        _ => {
            *value = serde_json::Value::Null;
            debug!(
                "Gen3D: normalized `{context}` `{field}` to null (unexpected type)",
                context = trimmed_context,
                field = trimmed_field_name,
            );
        }
    }
}

fn normalize_ai_plan_component_joint_fields(json_value: &mut serde_json::Value) {
    let Some(components) = json_value
        .get_mut("components")
        .and_then(|v| v.as_array_mut())
    else {
        return;
    };

    for component in components {
        let Some(component_obj) = component.as_object_mut() else {
            continue;
        };

        // Models sometimes emit an invalid top-level `joint` field on components:
        //
        //   { ..., "attach_to": { ... }, "joint": { ... } }
        //
        // The schema only allows joints as attachment metadata:
        //
        //   { ..., "attach_to": { ..., "joint": { ... } } }
        //
        // Normalize this common mistake so we don't trigger an expensive LLM schema-repair step.
        let Some(joint_value) = component_obj.remove("joint") else {
            continue;
        };

        let Some(attach_to) = component_obj.get_mut("attach_to") else {
            // Keep the invalid field so strict schema parsing can surface the issue.
            component_obj.insert("joint".to_string(), joint_value);
            continue;
        };
        let Some(attach_to_obj) = attach_to.as_object_mut() else {
            component_obj.insert("joint".to_string(), joint_value);
            continue;
        };

        // If the attachment already has a joint, drop the invalid top-level copy.
        if attach_to_obj.get("joint").is_some() {
            continue;
        }

        attach_to_obj.insert("joint".to_string(), joint_value);
        debug!(
            "Gen3D: normalized component-level `joint` into `attach_to.joint` (component={})",
            component_obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>")
        );
    }
}

fn normalize_ai_plan_top_level_fields(json_value: &mut serde_json::Value) {
    let Some(obj) = json_value.as_object_mut() else {
        return;
    };

    // Models sometimes include extra top-level metadata fields (ex: `name`, `purpose`) even when
    // instructed to return a strict plan object. Drop unknown top-level fields so we don't trigger
    // an expensive LLM schema-repair step on otherwise-valid plans.
    const ALLOWED: [&str; 10] = [
        "version",
        "rig",
        "mobility",
        "attack",
        "aim",
        "collider",
        "assembly_notes",
        "root_component",
        "reuse_groups",
        "components",
    ];

    let mut dropped: Vec<String> = Vec::new();
    let keys: Vec<String> = obj.keys().cloned().collect();
    for key in keys {
        if !ALLOWED.contains(&key.as_str()) {
            obj.remove(&key);
            dropped.push(key);
        }
    }

    if !dropped.is_empty() {
        dropped.sort();
        debug!("Gen3D: dropped unknown top-level plan fields: {dropped:?}");
    }
}

fn normalize_snake_case_token(raw: &str) -> String {
    let mut normalized = raw.trim().to_ascii_lowercase();
    normalized = normalized.replace(' ', "_");
    normalized = normalized.replace('-', "_");
    while normalized.contains("__") {
        normalized = normalized.replace("__", "_");
    }
    normalized
}

fn normalize_descriptor_tag(raw: &str) -> String {
    let normalized = normalize_snake_case_token(raw);
    let mut out = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

pub(super) fn parse_ai_draft_from_text(text: &str) -> Result<AiDraftJsonV1, String> {
    debug!(
        "Gen3D: extracted component output text (chars={})",
        text.chars().count()
    );
    let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
    debug!(
        "Gen3D: parsing component JSON (chars={})",
        json_text.trim().chars().count()
    );
    if cfg!(debug_assertions) {
        debug!(
            "Gen3D: component output preview (start): {}",
            super::truncate_for_ui(json_text.trim(), 800)
        );
    }

    let json_text = json_text.trim();
    let json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;

    // Record the raw model output for debugging.
    write_gen3d_json_artifact(None, "gen3d_component_raw.json".to_string(), &json_value);

    if json_value.get("version").is_none() {
        return Err("AI draft JSON missing required `version` (expected 2).".into());
    }

    let draft: AiDraftJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;

    if draft.version != 2 {
        return Err(format!(
            "Unsupported AI draft version {} (expected 2)",
            draft.version
        ));
    }
    if draft.parts.is_empty() {
        return Err("AI draft has no parts.".into());
    }
    if draft.parts.len() > GEN3D_MAX_PARTS {
        return Err(format!(
            "AI returned too many parts: {} (max {GEN3D_MAX_PARTS})",
            draft.parts.len()
        ));
    }
    let missing_colors = draft.parts.iter().filter(|p| p.color.is_none()).count();
    if missing_colors > 0 {
        return Err(format!(
            "AI draft missing required `color` on {missing_colors}/{} parts.",
            draft.parts.len()
        ));
    }

    debug!(
        "Gen3D: AI draft parsed (version=2, parts={}, anchors={}, collider={})",
        draft.parts.len(),
        draft.anchors.len(),
        if draft.collider.is_some() {
            "custom"
        } else {
            "default"
        }
    );

    Ok(draft)
}

pub(super) fn parse_ai_plan_from_text(text: &str) -> Result<AiPlanJsonV1, String> {
    debug!(
        "Gen3D: extracted plan output text (chars={})",
        text.chars().count()
    );
    let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
    debug!(
        "Gen3D: parsing plan JSON (chars={})",
        json_text.trim().chars().count()
    );
    if cfg!(debug_assertions) {
        debug!(
            "Gen3D: plan output preview (start): {}",
            super::truncate_for_ui(json_text.trim(), 800)
        );
    }

    let json_text = json_text.trim();
    let mut json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if json_value.get("version").is_none() {
        return Err("AI plan JSON missing required `version` (expected 8).".into());
    }

    normalize_ai_plan_component_joint_fields(&mut json_value);
    normalize_ai_plan_top_level_fields(&mut json_value);

    let plan: AiPlanJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if plan.version != 8 {
        return Err(format!(
            "Unsupported AI plan version {} (expected 8)",
            plan.version
        ));
    }
    Ok(plan)
}

pub(super) fn parse_ai_review_delta_from_text(text: &str) -> Result<AiReviewDeltaJsonV1, String> {
    debug!(
        "Gen3D: extracted review-delta output text (chars={})",
        text.chars().count()
    );
    let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
    debug!(
        "Gen3D: parsing review-delta JSON (chars={})",
        json_text.trim().chars().count()
    );
    if cfg!(debug_assertions) {
        debug!(
            "Gen3D: review-delta output preview (start): {}",
            super::truncate_for_ui(json_text.trim(), 800)
        );
    }

    let json_text = json_text.trim();
    let mut json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if json_value.get("version").is_none() {
        return Err("AI review-delta JSON missing required `version` (expected 1).".into());
    }

    normalize_ai_nullable_string_field_allowing_array(&mut json_value, "summary", "review-delta");
    normalize_ai_nullable_string_field_allowing_array(
        &mut json_value,
        "notes_text",
        "review-delta",
    );
    normalize_ai_nullable_string_field_allowing_array(&mut json_value, "notes", "review-delta");

    let mut delta: AiReviewDeltaJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if delta.version != 1 {
        return Err(format!(
            "Unsupported AI review-delta version {} (expected 1)",
            delta.version
        ));
    }

    delta.applies_to.run_id = delta.applies_to.run_id.trim().to_string();
    delta.applies_to.plan_hash = delta.applies_to.plan_hash.trim().to_string();

    if delta.actions.len() > 64 {
        debug!(
            "Gen3D: truncating review-delta actions from {} to 64",
            delta.actions.len()
        );
        delta.actions.truncate(64);
    }

    if let Some(summary) = delta.summary.as_ref().map(|v| v.trim().to_string()) {
        delta.summary = (!summary.is_empty()).then_some(summary);
    }
    if let Some(notes) = delta.notes_text.as_ref().map(|v| v.trim().to_string()) {
        delta.notes_text = (!notes.is_empty()).then_some(notes);
    }

    Ok(delta)
}

pub(super) fn parse_ai_prompt_intent_from_text(text: &str) -> Result<AiPromptIntentJsonV1, String> {
    debug!(
        "Gen3D: extracted prompt-intent output text (chars={})",
        text.chars().count()
    );
    let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
    debug!(
        "Gen3D: parsing prompt-intent JSON (chars={})",
        json_text.trim().chars().count()
    );
    if cfg!(debug_assertions) {
        debug!(
            "Gen3D: prompt-intent output preview (start): {}",
            super::truncate_for_ui(json_text.trim(), 800)
        );
    }

    let json_text = json_text.trim();
    let json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if json_value.get("version").is_none() {
        return Err("AI prompt-intent JSON missing required `version` (expected 1).".into());
    }

    let intent: AiPromptIntentJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if intent.version != 1 {
        return Err(format!(
            "Unsupported AI prompt-intent version {} (expected 1)",
            intent.version
        ));
    }

    Ok(intent)
}

pub(super) fn parse_ai_descriptor_meta_from_text(
    text: &str,
) -> Result<AiDescriptorMetaJsonV1, String> {
    debug!(
        "Gen3D: extracted descriptor-meta output text (chars={})",
        text.chars().count()
    );
    let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
    debug!(
        "Gen3D: parsing descriptor-meta JSON (chars={})",
        json_text.trim().chars().count()
    );
    if cfg!(debug_assertions) {
        debug!(
            "Gen3D: descriptor-meta output preview (start): {}",
            super::truncate_for_ui(json_text.trim(), 800)
        );
    }

    let json_text = json_text.trim();
    let json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if json_value.get("version").is_none() {
        return Err("AI descriptor-meta JSON missing required `version` (expected 1).".into());
    }

    let mut meta: AiDescriptorMetaJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if meta.version != 1 {
        return Err(format!(
            "Unsupported AI descriptor-meta version {} (expected 1)",
            meta.version
        ));
    }

    meta.name = meta.name.trim().to_string();
    meta.name = meta
        .name
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");

    meta.short = meta.short.trim().to_string();
    meta.short = meta.short.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut tags: Vec<String> = meta
        .tags
        .iter()
        .map(|tag| normalize_descriptor_tag(tag))
        .filter(|tag| !tag.trim().is_empty())
        .take(64)
        .collect();
    tags.sort();
    tags.dedup();
    meta.tags = tags;

    Ok(meta)
}

pub(super) fn parse_ai_motion_authoring_from_text(
    text: &str,
) -> Result<AiMotionAuthoringJsonV1, String> {
    debug!(
        "Gen3D: extracted motion-authoring output text (chars={})",
        text.chars().count()
    );
    let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
    debug!(
        "Gen3D: parsing motion-authoring JSON (chars={})",
        json_text.trim().chars().count()
    );
    if cfg!(debug_assertions) {
        debug!(
            "Gen3D: motion-authoring output preview (start): {}",
            super::truncate_for_ui(json_text.trim(), 800)
        );
    }

    let json_text = json_text.trim();
    let mut json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if json_value.get("version").is_none() {
        return Err("AI motion-authoring JSON missing required `version` (expected 1).".into());
    }

    normalize_ai_nullable_string_field_allowing_array(
        &mut json_value,
        "notes_text",
        "motion-authoring",
    );
    normalize_ai_nullable_string_field_allowing_array(&mut json_value, "notes", "motion-authoring");

    let mut authored: AiMotionAuthoringJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if authored.version != 1 {
        return Err(format!(
            "Unsupported AI motion-authoring version {} (expected 1)",
            authored.version
        ));
    }

    if matches!(
        authored.decision,
        super::schema::AiMotionAuthoringDecisionJsonV1::Unknown
    ) {
        return Err("AI motion-authoring has unknown `decision` value.".into());
    }

    authored.reason = authored.reason.trim().to_string();

    if let Some(notes) = authored.notes_text.as_ref().map(|v| v.trim().to_string()) {
        authored.notes_text = (!notes.is_empty()).then_some(notes);
    }

    // Normalize replace_channels.
    let mut channels: Vec<String> = Vec::new();
    for raw in authored.replace_channels.into_iter() {
        let ch = raw.trim().to_ascii_lowercase();
        if ch.is_empty() {
            continue;
        }
        channels.push(ch);
    }
    channels.sort();
    channels.dedup();
    if channels.len() > 8 {
        channels.truncate(8);
    }
    authored.replace_channels = channels;

    // Sanitize edges/slots.
    const MAX_EDGES: usize = 64;
    const MAX_SLOTS_PER_EDGE: usize = 32;
    const MAX_KEYFRAMES: usize = 48;

    if authored.edges.len() > MAX_EDGES {
        debug!(
            "Gen3D: truncating motion-authoring edges from {} to {MAX_EDGES}",
            authored.edges.len()
        );
        authored.edges.truncate(MAX_EDGES);
    }

    for edge in authored.edges.iter_mut() {
        edge.component = edge.component.trim().to_string();
        if edge.slots.len() > MAX_SLOTS_PER_EDGE {
            edge.slots.truncate(MAX_SLOTS_PER_EDGE);
        }

        for slot in edge.slots.iter_mut() {
            slot.channel = slot.channel.trim().to_ascii_lowercase();

            if matches!(slot.driver, super::schema::AiAnimationDriverJsonV1::Unknown) {
                return Err(format!(
                    "AI motion-authoring slot has unknown driver for component `{}` channel `{}`",
                    edge.component, slot.channel
                ));
            }
            if !slot.speed_scale.is_finite() {
                return Err(format!(
                    "AI motion-authoring slot speed_scale is non-finite for component `{}` channel `{}`",
                    edge.component, slot.channel
                ));
            }
            if !slot.time_offset_units.is_finite() {
                slot.time_offset_units = 0.0;
            }

            match &mut slot.clip {
                super::schema::AiAnimationClipJsonV1::Loop {
                    duration_units,
                    keyframes,
                }
                | super::schema::AiAnimationClipJsonV1::Once {
                    duration_units,
                    keyframes,
                }
                | super::schema::AiAnimationClipJsonV1::PingPong {
                    duration_units,
                    keyframes,
                } => {
                    if !duration_units.is_finite() || *duration_units <= 1e-6 {
                        return Err(format!(
                            "AI motion-authoring clip has invalid duration_units for component `{}` channel `{}`",
                            edge.component, slot.channel
                        ));
                    }
                    if keyframes.len() > MAX_KEYFRAMES {
                        keyframes.truncate(MAX_KEYFRAMES);
                    }
                    if keyframes.is_empty() {
                        return Err(format!(
                            "AI motion-authoring clip has 0 keyframes for component `{}` channel `{}`",
                            edge.component, slot.channel
                        ));
                    }

                    for keyframe in keyframes.iter_mut() {
                        if !keyframe.t_units.is_finite() {
                            keyframe.t_units = 0.0;
                        }
                        keyframe.t_units = keyframe.t_units.clamp(0.0, *duration_units);

                        if let Some(pos) = keyframe.delta.pos.as_ref() {
                            if pos.iter().any(|v| !v.is_finite()) {
                                keyframe.delta.pos = None;
                            }
                        }
                        if let Some(scale) = keyframe.delta.scale.as_ref() {
                            if scale.iter().any(|v| !v.is_finite()) {
                                keyframe.delta.scale = None;
                            }
                        }
                        if let Some(quat) = keyframe.delta.rot_quat_xyzw.as_ref() {
                            if quat.iter().any(|v| !v.is_finite()) {
                                keyframe.delta.rot_quat_xyzw = None;
                            }
                        }
                    }

                    keyframes.sort_by(|a, b| {
                        a.t_units
                            .partial_cmp(&b.t_units)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                super::schema::AiAnimationClipJsonV1::Spin {
                    axis,
                    radians_per_unit,
                    ..
                } => {
                    if axis.iter().any(|v| !v.is_finite()) {
                        return Err(format!(
                            "AI motion-authoring spin axis is non-finite for component `{}` channel `{}`",
                            edge.component, slot.channel
                        ));
                    }
                    if !radians_per_unit.is_finite() {
                        *radians_per_unit = 0.0;
                    }
                }
            }
        }
    }

    Ok(authored)
}

pub(super) fn extract_json_object(text: &str) -> Option<String> {
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let mut last: Option<(usize, usize)> = None;

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth = depth.saturating_add(1);
            }
            '}' => {
                if depth > 0 {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if let Some(s) = start.take() {
                            last = Some((s, idx + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    last.map(|(s, e)| text[s..e].to_string())
}

pub(super) fn extract_json_objects(text: &str, max_objects: usize) -> Vec<String> {
    let max_objects = max_objects.max(1);
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let mut out: Vec<String> = Vec::new();

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth = depth.saturating_add(1);
            }
            '}' => {
                if depth > 0 {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if let Some(s) = start.take() {
                            out.push(text[s..idx + 1].to_string());
                            if out.len() >= max_objects {
                                break;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ai_plan_normalizes_component_level_joint() {
        let text = r#"
        {
          "version": 8,
          "mobility": { "kind": "static" },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "a", "pos": [0.0,0.0,0.0], "forward": [0.0,0.0,1.0], "up": [0.0,1.0,0.0] }
              ]
            },
            {
              "name": "child",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "b", "pos": [0.0,0.0,0.0], "forward": [0.0,0.0,1.0], "up": [0.0,1.0,0.0] }
              ],
              "attach_to": { "parent": "root", "parent_anchor": "a", "child_anchor": "b" },
              "joint": { "kind": "hinge", "axis_join": [1.0, 0.0, 0.0], "limits_degrees": [-10.0, 10.0] }
            }
          ]
        }
        "#;

        let plan = parse_ai_plan_from_text(text).expect("plan should parse");
        let child = plan
            .components
            .iter()
            .find(|c| c.name == "child")
            .expect("child component should exist");
        let attach_to = child
            .attach_to
            .as_ref()
            .expect("child should have attach_to");
        assert!(
            attach_to.joint.is_some(),
            "component-level joint should be migrated to attach_to.joint"
        );
    }

    #[test]
    fn normalize_ai_plan_component_joint_fields_does_not_drop_root_joint() {
        let mut json_value: serde_json::Value = serde_json::from_str(
            r#"
            {
              "version": 8,
              "mobility": { "kind": "static" },
              "components": [
                {
                  "name": "root",
                  "size": [1.0, 1.0, 1.0],
                  "anchors": [
                    { "name": "a", "pos": [0.0,0.0,0.0], "forward": [0.0,0.0,1.0], "up": [0.0,1.0,0.0] }
                  ],
                  "joint": { "kind": "hinge" }
                }
              ]
            }
            "#,
        )
        .expect("JSON should parse");

        normalize_ai_plan_component_joint_fields(&mut json_value);

        assert!(
            json_value
                .get("components")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.get("joint"))
                .is_some(),
            "root component joint should not be dropped (strict schema parsing should surface it)"
        );
    }

    #[test]
    fn parse_ai_plan_drops_unknown_top_level_fields() {
        let text = r#"
        {
          "version": 8,
          "name": "warcar",
          "purpose": "fast vehicle",
          "mobility": { "kind": "ground", "max_speed": 6.0 },
          "attack": null,
          "aim": null,
          "collider": { "kind": "aabb_xz", "half_extents": [1.0, 2.0] },
          "assembly_notes": "",
          "root_component": "root",
          "reuse_groups": [],
          "components": [
            {
              "name": "root",
              "purpose": "",
              "modeling_notes": "",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "a", "pos": [0.0,0.0,0.0], "forward": [0.0,0.0,1.0], "up": [0.0,1.0,0.0] }
              ],
              "contacts": [],
              "attach_to": null
            }
          ]
        }
        "#;

        let plan = parse_ai_plan_from_text(text).expect("plan should parse");
        assert_eq!(plan.version, 8);
        assert!(matches!(
            plan.mobility,
            super::super::schema::AiMobilityJson::Ground { .. }
        ));
        assert_eq!(plan.components.len(), 1);
    }

    #[test]
    fn extracts_last_json_object_when_multiple_present() {
        let text = r#"{"a":1}{"a":2}"#;
        let extracted = extract_json_object(text).expect("should extract JSON object");
        let v: serde_json::Value = serde_json::from_str(&extracted).expect("extracted JSON parses");
        assert_eq!(v.get("a").and_then(|v| v.as_i64()), Some(2));
    }

    #[test]
    fn extracts_multiple_json_objects_in_order() {
        let text = r#"{"a":1} noise {"a":2}{"a":3}"#;
        let extracted = extract_json_objects(text, 16);
        assert_eq!(extracted.len(), 3);
        let vals: Vec<i64> = extracted
            .iter()
            .map(|s| serde_json::from_str::<serde_json::Value>(s).unwrap())
            .filter_map(|v| v.get("a").and_then(|v| v.as_i64()))
            .collect();
        assert_eq!(vals, vec![1, 2, 3]);
    }

    #[test]
    fn parses_descriptor_meta_and_normalizes_tags() {
        let text = r#"ok {"version":1,"name":"  The Very Cute Rabbit  ","short":"  A cute rabbit.\n","tags":["Voxel Art","rabbit","Rabbit","cute!!",""]}"#;
        let meta = parse_ai_descriptor_meta_from_text(text).expect("meta should parse");
        assert_eq!(meta.version, 1);
        assert_eq!(meta.name.as_str(), "The Very Cute");
        assert_eq!(meta.short.as_str(), "A cute rabbit.");
        assert_eq!(
            meta.tags,
            vec![
                "cute".to_string(),
                "rabbit".to_string(),
                "voxel_art".to_string()
            ]
        );
    }

    #[test]
    fn parses_motion_authoring_and_sanitizes() {
        let text = r#"ok {
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":2},
          "decision": "author_clips",
          "reason": "  test  ",
          "replace_channels": [" MOVE ", "idle", ""],
          "edges": [
            {
              "component":" leg_l ",
              "slots":[
                {
                  "channel":"MOVE",
                  "driver":"move_phase",
                  "speed_scale": 1.0,
                  "time_offset_units": 0.0,
                  "clip": {
                    "kind":"loop",
                    "duration_units": 1.0,
                    "keyframes": [
                      {"t_units": 1.2, "delta": {"pos": [0,0,0], "rot_quat_xyzw": [0,0,0,1], "scale": null}},
                      {"t_units": -0.2, "delta": {"pos": null, "rot_quat_xyzw": null, "scale": null}}
                    ]
                  }
                }
              ]
            }
          ],
          "notes_text": "  "
        }"#;

        let authored =
            parse_ai_motion_authoring_from_text(text).expect("motion authoring should parse");
        assert_eq!(authored.version, 1);
        assert_eq!(authored.reason.as_str(), "test");
        assert_eq!(
            authored.replace_channels,
            vec!["idle".to_string(), "move".to_string()]
        );
        assert!(authored.notes_text.is_none());
        assert_eq!(authored.edges.len(), 1);
        assert_eq!(authored.edges[0].component.as_str(), "leg_l");
        assert_eq!(authored.edges[0].slots.len(), 1);
        assert_eq!(authored.edges[0].slots[0].channel.as_str(), "move");
        match &authored.edges[0].slots[0].clip {
            super::super::schema::AiAnimationClipJsonV1::Loop { keyframes, .. } => {
                assert_eq!(keyframes.len(), 2);
                assert!(
                    keyframes[0].t_units <= keyframes[1].t_units,
                    "expected keyframes sorted"
                );
                assert!(
                    (0.0..=1.0).contains(&keyframes[0].t_units)
                        && (0.0..=1.0).contains(&keyframes[1].t_units)
                );
            }
            other => panic!("unexpected clip: {other:?}"),
        }
    }

    #[test]
    fn parses_motion_authoring_notes_array_as_string() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":2},
          "decision": "author_clips",
          "reason": "test",
          "replace_channels": ["move"],
          "edges": [],
          "notes": ["  line 1  ", "", "line 2", 3]
        }"#;

        let authored =
            parse_ai_motion_authoring_from_text(text).expect("motion authoring should parse");
        assert_eq!(authored.notes_text, Some("line 1\nline 2".to_string()));
    }

    #[test]
    fn parses_review_delta_summary_and_notes_arrays_as_strings() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "summary": ["  line 1  ", "", "line 2", 3],
          "notes_text": ["  note 1  ", "", "note 2", null],
          "actions": [
            {"kind":"accept"}
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("review-delta should parse");
        assert_eq!(delta.summary, Some("line 1\nline 2".to_string()));
        assert_eq!(delta.notes_text, Some("note 1\nnote 2".to_string()));
    }

    #[test]
    fn rejects_review_delta_attack_kind_synonyms() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {"kind":"tweak_attack","attack":{"kind":"cannon","cooldown_secs":0.25},"reason":"test"}
          ]
        }"#;
        assert!(
            parse_ai_review_delta_from_text(text).is_err(),
            "expected non-canonical attack.kind to error"
        );
    }

    #[test]
    fn rejects_review_delta_tweak_animation_action() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_animation",
              "component_id":"deadbeef",
              "channel":"move",
              "spec": { "driver":"always", "clip": { "kind":"spin", "axis":[1,0,0], "radians_per_unit": 2.0 } }
            }
          ]
        }"#;
        assert!(
            parse_ai_review_delta_from_text(text).is_err(),
            "expected tweak_animation to be rejected"
        );
    }

    #[test]
    fn parses_review_delta_tweak_contact_stance_null_as_some_none() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {"kind":"tweak_contact","component_id":"deadbeef","contact_name":"ground","stance":null,"reason":"test"}
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("review-delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            super::super::schema::AiReviewDeltaActionJsonV1::TweakContact { stance, .. } => {
                assert!(matches!(stance, Some(None)));
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn rejects_plan_with_wrong_version() {
        let text = r#"{"version":6,"components":[{"name":"root","size":[1,1,1]}]}"#;
        assert!(parse_ai_plan_from_text(text).is_err());
    }
}
