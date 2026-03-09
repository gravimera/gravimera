use bevy::prelude::*;

use super::parse::{extract_json_object, extract_json_objects};
use super::Gen3dPlannedComponent;
use crate::gen3d::agent::Gen3dAgentStepJsonV1;

pub(super) fn normalize_identifier_for_match(value: &str) -> String {
    let mut out = String::new();
    let mut prev_sep = true;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            out.push('_');
            prev_sep = true;
        }
        if out.len() >= 64 {
            break;
        }
    }

    while out.starts_with('_') {
        out.remove(0);
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

pub(super) fn resolve_component_index_by_name_hint(
    components: &[Gen3dPlannedComponent],
    hint: &str,
) -> Option<usize> {
    let hint_norm = normalize_identifier_for_match(hint);
    if hint_norm.is_empty() {
        return None;
    }

    for (idx, c) in components.iter().enumerate() {
        if normalize_identifier_for_match(c.name.as_str()) == hint_norm {
            return Some(idx);
        }
    }

    let hint_tokens: Vec<&str> = hint_norm.split('_').filter(|s| !s.is_empty()).collect();
    if hint_tokens.is_empty() {
        return None;
    }

    let mut best: Option<(usize, f32, usize)> = None;
    for (idx, c) in components.iter().enumerate() {
        let cand_norm = normalize_identifier_for_match(c.name.as_str());
        if cand_norm.is_empty() {
            continue;
        }
        let cand_tokens: Vec<&str> = cand_norm.split('_').filter(|s| !s.is_empty()).collect();
        if cand_tokens.is_empty() {
            continue;
        }

        let mut intersection = 0usize;
        for t in &hint_tokens {
            if cand_tokens.contains(t) {
                intersection += 1;
            }
        }
        let union = hint_tokens.len() + cand_tokens.len() - intersection;
        let mut score = if union == 0 {
            0.0
        } else {
            intersection as f32 / union as f32
        };

        if hint_norm.contains(&cand_norm) || cand_norm.contains(&hint_norm) {
            score += 0.25;
        }
        if hint_tokens.first() == cand_tokens.first() {
            score += 0.12;
        }

        let len_bonus = cand_norm.len().min(hint_norm.len());
        if best
            .as_ref()
            .map(|(_, s, l)| score > *s || (score == *s && len_bonus > *l))
            .unwrap_or(true)
        {
            best = Some((idx, score, len_bonus));
        }
    }

    let (idx, score, _) = best?;
    (score >= 0.34).then_some(idx)
}

pub(super) fn parse_vec3(value: &serde_json::Value) -> Option<Vec3> {
    let arr = value.as_array()?;
    if arr.len() != 3 {
        return None;
    }
    let x = arr.get(0)?.as_f64()? as f32;
    let y = arr.get(1)?.as_f64()? as f32;
    let z = arr.get(2)?.as_f64()? as f32;
    let v = Vec3::new(x, y, z);
    v.is_finite().then_some(v)
}

pub(super) fn parse_quat_xyzw(value: &serde_json::Value) -> Option<Quat> {
    let arr = value.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    let x = arr.get(0)?.as_f64()? as f32;
    let y = arr.get(1)?.as_f64()? as f32;
    let z = arr.get(2)?.as_f64()? as f32;
    let w = arr.get(3)?.as_f64()? as f32;
    let q = Quat::from_xyzw(x, y, z, w).normalize();
    q.is_finite().then_some(q)
}

pub(super) fn parse_delta_transform(value: Option<&serde_json::Value>) -> Transform {
    let mut out = Transform::IDENTITY;
    let Some(value) = value else {
        return out;
    };
    if let Some(pos) = value
        .get("pos")
        .and_then(parse_vec3)
        .or_else(|| value.get("position").and_then(parse_vec3))
        .or_else(|| value.get("translation").and_then(parse_vec3))
    {
        out.translation = pos;
    }
    if let Some(scale) = value
        .get("scale")
        .and_then(parse_vec3)
        .or_else(|| value.get("size").and_then(parse_vec3))
    {
        out.scale = scale;
    }

    // Rotation: accept rot_quat_xyzw / quat_xyzw, or basis forward+up.
    let mut rotation: Option<Quat> = value
        .get("rot_quat_xyzw")
        .and_then(parse_quat_xyzw)
        .or_else(|| value.get("quat_xyzw").and_then(parse_quat_xyzw));
    if rotation.is_none() {
        if let Some(rot) = value.get("rot").and_then(|v| v.as_object()) {
            rotation = rot
                .get("quat_xyzw")
                .and_then(parse_quat_xyzw)
                .or_else(|| rot.get("rot_quat_xyzw").and_then(parse_quat_xyzw));
            if rotation.is_none() {
                if let Some(fwd) = rot.get("forward").and_then(parse_vec3) {
                    let up = rot.get("up").and_then(parse_vec3);
                    rotation = Some(super::convert::plan_rotation_from_forward_up_lossy(fwd, up));
                }
            }
        }
    }
    if rotation.is_none() {
        if let Some(fwd) = value.get("forward").and_then(parse_vec3) {
            let up = value.get("up").and_then(parse_vec3);
            rotation = Some(super::convert::plan_rotation_from_forward_up_lossy(fwd, up));
        }
    }
    if let Some(q) = rotation {
        out.rotation = q;
    }
    out
}

pub(super) fn parse_agent_step(text: &str) -> Result<Gen3dAgentStepJsonV1, String> {
    let candidates = extract_json_objects(text, 16);

    fn is_done_action(action: &crate::gen3d::agent::Gen3dAgentActionJsonV1) -> bool {
        matches!(
            action,
            crate::gen3d::agent::Gen3dAgentActionJsonV1::Done { .. }
        )
    }

    fn has_done(step: &Gen3dAgentStepJsonV1) -> bool {
        step.actions.iter().any(is_done_action)
    }

    fn action_eq(
        a: &crate::gen3d::agent::Gen3dAgentActionJsonV1,
        b: &crate::gen3d::agent::Gen3dAgentActionJsonV1,
    ) -> bool {
        use crate::gen3d::agent::Gen3dAgentActionJsonV1;
        match (a, b) {
            (
                Gen3dAgentActionJsonV1::ToolCall {
                    call_id: a_call_id,
                    tool_id: a_tool_id,
                    args: a_args,
                },
                Gen3dAgentActionJsonV1::ToolCall {
                    call_id: b_call_id,
                    tool_id: b_tool_id,
                    args: b_args,
                },
            ) => a_call_id == b_call_id && a_tool_id == b_tool_id && a_args == b_args,
            (Gen3dAgentActionJsonV1::Done { .. }, Gen3dAgentActionJsonV1::Done { .. }) => true,
            _ => false,
        }
    }

    fn is_done_superset_of_non_done(
        done_step: &Gen3dAgentStepJsonV1,
        non_done_step: &Gen3dAgentStepJsonV1,
    ) -> bool {
        if done_step.actions.len() != non_done_step.actions.len().saturating_add(1) {
            return false;
        }
        if !matches!(done_step.actions.last(), Some(a) if is_done_action(a)) {
            return false;
        }
        if done_step.actions[..done_step.actions.len().saturating_sub(1)]
            .iter()
            .any(is_done_action)
        {
            return false;
        }

        for (a, b) in done_step
            .actions
            .iter()
            .take(non_done_step.actions.len())
            .zip(non_done_step.actions.iter())
        {
            if !action_eq(a, b) {
                return false;
            }
        }
        true
    }

    // When the model outputs multiple JSON objects, it is often "simulating" multiple steps
    // (and sometimes hallucinating tool results). Prefer the *last* parsed step that does NOT
    // include a `done` action so we don't accidentally terminate the run early.
    let mut parsed_steps: Vec<Gen3dAgentStepJsonV1> = Vec::new();

    for json_text in candidates.iter() {
        let json_text = json_text.trim();
        let Ok(mut step) = serde_json::from_str::<Gen3dAgentStepJsonV1>(json_text) else {
            continue;
        };

        if step.version == 0 {
            step.version = 1;
        }
        if step.version != 1 {
            continue;
        }
        if step.actions.len() > 32 {
            step.actions.truncate(32);
        }

        parsed_steps.push(step);
    }

    let mut step = if parsed_steps.is_empty() {
        let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
        let json_text = json_text.trim();
        serde_json::from_str::<Gen3dAgentStepJsonV1>(json_text)
            .map_err(|err| format!("Failed to parse JSON: {err}"))?
    } else {
        let last_step = parsed_steps.last().cloned();
        let last_non_done_step = parsed_steps
            .iter()
            .rev()
            .find(|s| !s.actions.is_empty() && !has_done(s))
            .cloned();

        if let Some(non_done_step) = last_non_done_step.as_ref() {
            if let Some(done_superset) = parsed_steps
                .iter()
                .rev()
                .find(|s| has_done(s) && is_done_superset_of_non_done(s, non_done_step))
                .cloned()
            {
                done_superset
            } else {
                last_non_done_step.or(last_step).unwrap()
            }
        } else {
            last_step.unwrap()
        }
    };

    if step.version == 0 {
        step.version = 1;
    }
    if step.version != 1 {
        return Err(format!(
            "Unsupported gen3d_agent_step version {} (expected 1)",
            step.version
        ));
    }
    if step.actions.len() > 32 {
        step.actions.truncate(32);
    }
    Ok(step)
}

pub(super) fn is_transient_ai_error_message(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("http 429")
        || lower.contains("http 408")
        || lower.contains("http 409")
        || lower.contains("http 425")
        || lower.contains("http 502")
        || lower.contains("http 503")
        || lower.contains("http 504")
        || lower.contains("http 5")
        || lower.contains("status=5")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("failed to connect")
        || lower.contains("econnreset")
        || lower.contains("econnrefused")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen3d::agent::Gen3dAgentActionJsonV1;

    #[test]
    fn agent_step_prefers_non_done_step_when_multiple_present() {
        let text = r#"{"version":1,"status_summary":"first","actions":[{"kind":"tool_call","call_id":"call_1","tool_id":"get_tool_detail_v1","args":{"tool_id":"qa_v1"}}]}{"version":1,"status_summary":"second","actions":[{"kind":"done","reason":"stop"}]}"#;
        let step = parse_agent_step(text).expect("parse");
        assert_eq!(step.status_summary, "first");
        assert_eq!(step.actions.len(), 1);
        assert!(matches!(
            step.actions[0],
            Gen3dAgentActionJsonV1::ToolCall { .. }
        ));
    }

    #[test]
    fn agent_step_uses_last_non_done_step_if_multiple_tool_steps_present() {
        let text = r#"{"version":1,"status_summary":"first","actions":[{"kind":"tool_call","call_id":"call_1","tool_id":"get_tool_detail_v1","args":{"tool_id":"qa_v1"}}]}{"version":1,"status_summary":"second","actions":[{"kind":"tool_call","call_id":"call_2","tool_id":"qa_v1","args":{}}]}"#;
        let step = parse_agent_step(text).expect("parse");
        assert_eq!(step.status_summary, "second");
        assert_eq!(step.actions.len(), 1);
        assert!(matches!(
            step.actions[0],
            Gen3dAgentActionJsonV1::ToolCall { .. }
        ));
    }

    #[test]
    fn agent_step_prefers_done_step_when_it_is_a_strict_superset() {
        let text = r#"{"version":1,"status_summary":"with_done","actions":[{"kind":"tool_call","call_id":"call_1","tool_id":"get_tool_detail_v1","args":{"tool_id":"qa_v1"}},{"kind":"tool_call","call_id":"call_2","tool_id":"qa_v1","args":{}},{"kind":"done","reason":"stop"}]}{"version":1,"status_summary":"without_done","actions":[{"kind":"tool_call","call_id":"call_1","tool_id":"get_tool_detail_v1","args":{"tool_id":"qa_v1"}},{"kind":"tool_call","call_id":"call_2","tool_id":"qa_v1","args":{}}]}"#;
        let step = parse_agent_step(text).expect("parse");
        assert_eq!(step.status_summary, "with_done");
        assert_eq!(step.actions.len(), 3);
        assert!(matches!(
            step.actions[2],
            Gen3dAgentActionJsonV1::Done { .. }
        ));
    }
}
