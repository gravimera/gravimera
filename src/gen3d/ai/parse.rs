use bevy::log::debug;

use super::super::GEN3D_MAX_PARTS;
use super::artifacts::write_gen3d_json_artifact;
use super::schema::{
    AiDescriptorMetaJsonV1, AiDraftJsonV1, AiMotionAuthoringJsonV1, AiMotionTargetKindJsonV1,
    AiPlanJsonV1, AiPromptIntentJsonV1, AiReviewDeltaJsonV1,
};

fn sanitize_articulation_nodes(draft: &mut AiDraftJsonV1) -> Result<(), String> {
    use std::collections::{HashMap, HashSet};

    let parts_len = draft.parts.len();
    let mut node_index: HashMap<String, usize> = HashMap::new();
    let mut seen_part_indices: HashSet<usize> = HashSet::new();

    for (idx, node) in draft.articulation_nodes.iter_mut().enumerate() {
        node.node_id = node.node_id.trim().to_string();
        if node.node_id.is_empty() {
            return Err(format!(
                "AI draft articulation_nodes[{idx}] is missing required `node_id`."
            ));
        }
        node.parent_node_id = node
            .parent_node_id
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if node.parent_node_id.as_deref() == Some(node.node_id.as_str()) {
            return Err(format!(
                "AI draft articulation node `{}` cannot parent itself.",
                node.node_id
            ));
        }
        if node.bind_part_indices.is_empty() {
            return Err(format!(
                "AI draft articulation node `{}` must bind at least one part index.",
                node.node_id
            ));
        }
        node.bind_part_indices.sort_unstable();
        node.bind_part_indices.dedup();
        for &part_idx in node.bind_part_indices.iter() {
            if part_idx >= parts_len {
                return Err(format!(
                    "AI draft articulation node `{}` references out-of-range bind_part_indices entry {} (parts_total={parts_len}).",
                    node.node_id, part_idx
                ));
            }
            if !seen_part_indices.insert(part_idx) {
                return Err(format!(
                    "AI draft articulation node `{}` reuses part index {} that is already bound by another articulation node.",
                    node.node_id, part_idx
                ));
            }
        }
        if node_index.insert(node.node_id.clone(), idx).is_some() {
            return Err(format!(
                "AI draft articulation_nodes contains duplicate node_id `{}`.",
                node.node_id
            ));
        }
    }

    let mut visiting = vec![false; draft.articulation_nodes.len()];
    let mut visited = vec![false; draft.articulation_nodes.len()];

    fn dfs(
        idx: usize,
        nodes: &[super::schema::AiArticulationNodeJson],
        node_index: &HashMap<String, usize>,
        visiting: &mut [bool],
        visited: &mut [bool],
    ) -> Result<(), String> {
        if visited[idx] {
            return Ok(());
        }
        if visiting[idx] {
            return Err(format!(
                "AI draft articulation_nodes contains a parent cycle involving `{}`.",
                nodes[idx].node_id
            ));
        }
        visiting[idx] = true;
        if let Some(parent) = nodes[idx].parent_node_id.as_ref() {
            let Some(&parent_idx) = node_index.get(parent) else {
                return Err(format!(
                    "AI draft articulation node `{}` references missing parent_node_id `{}`.",
                    nodes[idx].node_id, parent
                ));
            };
            dfs(parent_idx, nodes, node_index, visiting, visited)?;
        }
        visiting[idx] = false;
        visited[idx] = true;
        Ok(())
    }

    for idx in 0..draft.articulation_nodes.len() {
        dfs(
            idx,
            &draft.articulation_nodes,
            &node_index,
            &mut visiting,
            &mut visited,
        )?;
    }

    Ok(())
}

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

fn normalize_json_named_object_map_to_array(
    value: &mut serde_json::Value,
    item_name_key: &str,
    context: &str,
) {
    match value {
        serde_json::Value::Null => {
            *value = serde_json::Value::Array(Vec::new());
            debug!(
                "Gen3D: normalized `{context}` null into empty array",
                context = context.trim(),
            );
        }
        serde_json::Value::Object(_) => {
            let raw = std::mem::replace(value, serde_json::Value::Null);
            let serde_json::Value::Object(map) = raw else {
                unreachable!();
            };

            let all_values_are_objects = map.values().all(|v| v.is_object());
            if !all_values_are_objects {
                *value = serde_json::Value::Array(vec![serde_json::Value::Object(map)]);
                debug!(
                    "Gen3D: wrapped `{context}` object into singleton array",
                    context = context.trim(),
                );
                return;
            }

            let mut entries: Vec<(String, serde_json::Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));

            let mut arr: Vec<serde_json::Value> = Vec::with_capacity(entries.len());
            for (name, mut item) in entries {
                if let serde_json::Value::Object(item_obj) = &mut item {
                    if !item_obj.contains_key(item_name_key) && !name.trim().is_empty() {
                        item_obj.insert(item_name_key.to_string(), serde_json::Value::String(name));
                    }
                }
                arr.push(item);
            }

            *value = serde_json::Value::Array(arr);
            debug!(
                "Gen3D: normalized `{context}` keyed object map into array",
                context = context.trim(),
            );
        }
        _ => {}
    }
}

fn normalize_json_singleton_array_field(value: &mut serde_json::Value, context: &str) {
    match value {
        serde_json::Value::Null => {
            *value = serde_json::Value::Array(Vec::new());
            debug!(
                "Gen3D: normalized `{context}` null into empty array",
                context = context.trim(),
            );
        }
        serde_json::Value::Object(_) => {
            let raw = std::mem::replace(value, serde_json::Value::Null);
            *value = serde_json::Value::Array(vec![raw]);
            debug!(
                "Gen3D: wrapped `{context}` object into singleton array",
                context = context.trim(),
            );
        }
        _ => {}
    }
}

fn normalize_ai_plan_collection_shapes(json_value: &mut serde_json::Value) {
    let Some(obj) = json_value.as_object_mut() else {
        return;
    };

    if let Some(value) = obj.get_mut("components") {
        normalize_json_named_object_map_to_array(value, "name", "plan.components");
    }
    if let Some(value) = obj.get_mut("reuse_groups") {
        normalize_json_singleton_array_field(value, "plan.reuse_groups");
    }

    let Some(components) = obj.get_mut("components").and_then(|v| v.as_array_mut()) else {
        return;
    };

    for component in components.iter_mut() {
        let Some(component_obj) = component.as_object_mut() else {
            continue;
        };
        if let Some(value) = component_obj.get_mut("anchors") {
            normalize_json_named_object_map_to_array(value, "name", "plan.components[].anchors");
        }
        if let Some(value) = component_obj.get_mut("contacts") {
            normalize_json_named_object_map_to_array(value, "name", "plan.components[].contacts");
        }
        if let Some(value) = component_obj.get_mut("articulation_nodes") {
            normalize_json_named_object_map_to_array(
                value,
                "node_id",
                "plan.components[].articulation_nodes",
            );
        }
    }
}

fn normalize_ai_plan_rig_motion_fields(json_value: &mut serde_json::Value) {
    let Some(obj) = json_value.as_object_mut() else {
        return;
    };

    let Some(mut rig_value) = obj.remove("rig") else {
        return;
    };
    let Some(rig_obj) = rig_value.as_object_mut() else {
        obj.insert("rig".to_string(), rig_value);
        return;
    };

    if let Some(named_motions) = rig_obj.remove("named_motions") {
        if !named_motions.is_null() {
            debug!("Gen3D: dropped unsupported `rig.named_motions` from plan JSON");
        }
    }

    if let Some(raw_nodes) = rig_obj.remove("articulation_nodes") {
        let mut nodes = match raw_nodes {
            serde_json::Value::Array(arr) => arr,
            serde_json::Value::Null => Vec::new(),
            serde_json::Value::Object(map) => vec![serde_json::Value::Object(map)],
            other => {
                rig_obj.insert("articulation_nodes".to_string(), other);
                Vec::new()
            }
        };

        if !nodes.is_empty() {
            let Some(components) = obj.get_mut("components").and_then(|v| v.as_array_mut()) else {
                rig_obj.insert(
                    "articulation_nodes".to_string(),
                    serde_json::Value::Array(nodes),
                );
                if !rig_obj.is_empty() {
                    obj.insert("rig".to_string(), rig_value);
                }
                return;
            };

            let name_to_idx: std::collections::HashMap<String, usize> = components
                .iter()
                .enumerate()
                .filter_map(|(idx, component)| {
                    component
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|name| (name.trim().to_string(), idx))
                })
                .filter(|(name, _)| !name.is_empty())
                .collect();

            let mut moved = 0usize;
            let mut leftovers: Vec<serde_json::Value> = Vec::new();
            for mut node in nodes.drain(..) {
                let Some(node_obj) = node.as_object_mut() else {
                    leftovers.push(node);
                    continue;
                };

                let component_name = node_obj
                    .remove("component")
                    .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty());
                node_obj.remove("bind_part_indices");

                let Some(component_name) = component_name else {
                    leftovers.push(node);
                    continue;
                };
                let Some(&component_idx) = name_to_idx.get(component_name.as_str()) else {
                    node_obj.insert(
                        "component".to_string(),
                        serde_json::Value::String(component_name),
                    );
                    leftovers.push(node);
                    continue;
                };
                let Some(component_obj) = components
                    .get_mut(component_idx)
                    .and_then(|value| value.as_object_mut())
                else {
                    leftovers.push(node);
                    continue;
                };
                let Some(target_nodes) = component_obj
                    .entry("articulation_nodes".to_string())
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()))
                    .as_array_mut()
                else {
                    leftovers.push(node);
                    continue;
                };
                target_nodes.push(node);
                moved = moved.saturating_add(1);
            }

            if moved > 0 {
                debug!(
                    "Gen3D: hoisted {moved} articulation node(s) from `rig.articulation_nodes` into `components[].articulation_nodes`"
                );
            }
            if !leftovers.is_empty() {
                rig_obj.insert(
                    "articulation_nodes".to_string(),
                    serde_json::Value::Array(leftovers),
                );
            }
        }
    }

    if !rig_obj.is_empty() {
        obj.insert("rig".to_string(), rig_value);
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
    let mut draft = draft;

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
    sanitize_articulation_nodes(&mut draft)?;
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

    normalize_ai_plan_collection_shapes(&mut json_value);
    normalize_ai_plan_rig_motion_fields(&mut json_value);
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

    let mut intent: AiPromptIntentJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if intent.version != 1 {
        return Err(format!(
            "Unsupported AI prompt-intent version {} (expected 1)",
            intent.version
        ));
    }
    let mut normalized_channels: Vec<String> = Vec::new();
    for channel in intent.explicit_motion_channels.drain(..) {
        let channel = channel.trim();
        if channel.is_empty() {
            continue;
        }
        if normalized_channels
            .iter()
            .any(|existing| existing == channel)
        {
            continue;
        }
        normalized_channels.push(channel.to_string());
    }
    intent.explicit_motion_channels = normalized_channels;

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

    // Sanitize targets/slots.
    const MAX_TARGETS: usize = 64;
    const MAX_SLOTS_PER_TARGET: usize = 32;
    const MAX_KEYFRAMES: usize = 48;

    if authored.targets.len() > MAX_TARGETS {
        debug!(
            "Gen3D: truncating motion-authoring targets from {} to {MAX_TARGETS}",
            authored.targets.len()
        );
        authored.targets.truncate(MAX_TARGETS);
    }

    for target in authored.targets.iter_mut() {
        target.component = target.component.trim().to_string();
        target.node_id = target
            .node_id
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if target.slots.len() > MAX_SLOTS_PER_TARGET {
            target.slots.truncate(MAX_SLOTS_PER_TARGET);
        }

        for slot in target.slots.iter_mut() {
            slot.channel = slot.channel.trim().to_ascii_lowercase();

            if matches!(slot.driver, super::schema::AiAnimationDriverJsonV1::Unknown) {
                return Err(format!(
                    "AI motion-authoring slot has unknown driver for target `{}` channel `{}`",
                    target.component, slot.channel
                ));
            }
            if matches!(slot.family, super::schema::AiAnimationFamilyJsonV1::Unknown) {
                return Err(format!(
                    "AI motion-authoring slot has unknown family for target `{}` channel `{}`",
                    target.component, slot.channel
                ));
            }
            if !slot.speed_scale.is_finite() {
                return Err(format!(
                    "AI motion-authoring slot speed_scale is non-finite for target `{}` channel `{}`",
                    target.component, slot.channel
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
                            "AI motion-authoring clip has invalid duration_units for target `{}` channel `{}`",
                            target.component, slot.channel
                        ));
                    }
                    if keyframes.len() > MAX_KEYFRAMES {
                        keyframes.truncate(MAX_KEYFRAMES);
                    }
                    if keyframes.is_empty() {
                        return Err(format!(
                            "AI motion-authoring clip has 0 keyframes for target `{}` channel `{}`",
                            target.component, slot.channel
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
                            "AI motion-authoring spin axis is non-finite for target `{}` channel `{}`",
                            target.component, slot.channel
                        ));
                    }
                    if !radians_per_unit.is_finite() {
                        *radians_per_unit = 0.0;
                    }
                }
            }
        }

        match target.kind {
            AiMotionTargetKindJsonV1::RootEdge => {
                target.node_id = None;
            }
            AiMotionTargetKindJsonV1::AttachmentEdge => {
                if target.component.is_empty() {
                    return Err(
                        "AI motion-authoring attachment_edge target is missing `component`.".into(),
                    );
                }
                target.node_id = None;
            }
            AiMotionTargetKindJsonV1::ArticulationNode => {
                if target.component.is_empty() {
                    return Err(
                        "AI motion-authoring articulation_node target is missing `component`."
                            .into(),
                    );
                }
                if target.node_id.is_none() {
                    return Err(
                        "AI motion-authoring articulation_node target is missing `node_id`.".into(),
                    );
                }
            }
            AiMotionTargetKindJsonV1::Unknown => {
                return Err("AI motion-authoring has unknown target `kind` value.".into());
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
    fn parse_ai_plan_hoists_rig_articulation_nodes_and_normalizes_anchor_maps() {
        let text = r#"
        {
          "version": 8,
          "mobility": { "kind": "static" },
          "rig": {
            "articulation_nodes": [
              {
                "component": "head",
                "node_id": "jaw",
                "pos": [0.0, -0.1, 0.2],
                "forward": [0.0, 0.0, 1.0],
                "up": [0.0, 1.0, 0.0],
                "bind_part_indices": [2]
              }
            ],
            "named_motions": [
              { "name": "jaw_open", "poses": [] }
            ]
          },
          "reuse_groups": [],
          "components": [
            {
              "name": "head",
              "purpose": "",
              "modeling_notes": "",
              "size": [1.0, 1.0, 1.0],
              "anchors": {
                "look": {
                  "pos": [0.0, 0.1, 0.2],
                  "forward": [0.0, 0.0, 1.0],
                  "up": [0.0, 1.0, 0.0]
                }
              },
              "contacts": [],
              "attach_to": null
            }
          ]
        }
        "#;

        let plan = parse_ai_plan_from_text(text).expect("plan should parse");
        assert!(
            plan.rig.is_none(),
            "unsupported rig motion fields should be dropped"
        );
        assert_eq!(plan.components.len(), 1);
        assert_eq!(plan.components[0].anchors.len(), 1);
        assert_eq!(plan.components[0].anchors[0].name, "look");
        assert_eq!(plan.components[0].articulation_nodes.len(), 1);
        assert_eq!(plan.components[0].articulation_nodes[0].node_id, "jaw");
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
    fn parses_prompt_intent_and_normalizes_named_motion_channels() {
        let text = r#"{
          "version": 1,
          "requires_attack": false,
          "explicit_motion_channels": [" sing ", "", "dance", "sing", "rap "]
        }"#;

        let intent = parse_ai_prompt_intent_from_text(text).expect("prompt intent should parse");
        assert_eq!(intent.version, 1);
        assert!(!intent.requires_attack);
        assert_eq!(
            intent.explicit_motion_channels,
            vec!["sing".to_string(), "dance".to_string(), "rap".to_string()]
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
          "targets": [
            {
              "kind":"attachment_edge",
              "component":" leg_l ",
              "node_id": null,
              "slots":[
                {
                  "channel":"MOVE",
                  "family":"base",
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
        assert_eq!(authored.targets.len(), 1);
        assert_eq!(authored.targets[0].component.as_str(), "leg_l");
        assert_eq!(
            authored.targets[0].kind,
            super::super::schema::AiMotionTargetKindJsonV1::AttachmentEdge
        );
        assert_eq!(authored.targets[0].slots.len(), 1);
        assert_eq!(authored.targets[0].slots[0].channel.as_str(), "move");
        assert_eq!(
            authored.targets[0].slots[0].family,
            super::super::schema::AiAnimationFamilyJsonV1::Base
        );
        match &authored.targets[0].slots[0].clip {
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
          "targets": [],
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
