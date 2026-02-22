use bevy::log::debug;

use super::artifacts::write_gen3d_json_artifact;
use super::schema::{
    AiDescriptorMetaJsonV1, AiDraftJsonV1, AiPlanFillJsonV1, AiPlanJsonV1, AiReviewDeltaJsonV1,
};
use super::super::GEN3D_MAX_PARTS;

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
    let json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if json_value.get("version").is_none() {
        return Err("AI plan JSON missing required `version` (expected 7).".into());
    }

    let plan: AiPlanJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if plan.version != 7 {
        return Err(format!(
            "Unsupported AI plan version {} (expected 7)",
            plan.version
        ));
    }
    Ok(plan)
}

pub(super) fn parse_ai_plan_fill_from_text(text: &str) -> Result<AiPlanFillJsonV1, String> {
    debug!(
        "Gen3D: extracted plan-fill output text (chars={})",
        text.chars().count()
    );
    let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
    debug!(
        "Gen3D: parsing plan-fill JSON (chars={})",
        json_text.trim().chars().count()
    );
    if cfg!(debug_assertions) {
        debug!(
            "Gen3D: plan-fill output preview (start): {}",
            super::truncate_for_ui(json_text.trim(), 800)
        );
    }

    let json_text = json_text.trim();
    let json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if json_value.get("version").is_none() {
        return Err("AI plan-fill JSON missing required `version` (expected 1).".into());
    }

    let fill: AiPlanFillJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if fill.version != 1 {
        return Err(format!(
            "Unsupported AI plan-fill version {} (expected 1)",
            fill.version
        ));
    }
    Ok(fill)
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
    let json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if json_value.get("version").is_none() {
        return Err("AI review-delta JSON missing required `version` (expected 1).".into());
    }

    let mut delta: AiReviewDeltaJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("AI JSON schema error: {err}"))?;
    if delta.version != 1 {
        return Err(format!(
            "Unsupported AI review-delta version {} (expected 1)",
            delta.version
        ));
    }
    if delta.actions.len() > 64 {
        debug!(
            "Gen3D: truncating review-delta actions from {} to 64",
            delta.actions.len()
        );
        delta.actions.truncate(64);
    }
    Ok(delta)
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

#[cfg(test)]
mod tests {
    use super::super::schema::{AiAnimationClipJson, AiAnimationDriverJson, AiReviewDeltaActionJsonV1};
    use super::*;

    #[test]
    fn extracts_last_json_object_when_multiple_present() {
        let text = r#"{"a":1}{"a":2}"#;
        let extracted = extract_json_object(text).expect("should extract JSON object");
        let v: serde_json::Value = serde_json::from_str(&extracted).expect("extracted JSON parses");
        assert_eq!(v.get("a").and_then(|v| v.as_i64()), Some(2));
    }

    #[test]
    fn parses_descriptor_meta_and_normalizes_tags() {
        let text = r#"ok {"version":1,"short":"  A cute rabbit.\n","tags":["Voxel Art","rabbit","Rabbit","cute!!",""]}"#;
        let meta = parse_ai_descriptor_meta_from_text(text).expect("meta should parse");
        assert_eq!(meta.version, 1);
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
    fn rejects_review_delta_missing_driver() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_animation",
              "component_id":"deadbeef",
              "channel":"move",
              "spec": {
                "clip": {
                  "kind":"loop",
                  "duration_secs": 1.0,
                  "keyframes": [ { "time_secs": 0.0 } ]
                }
              }
            }
          ]
        }"#;
        assert!(
            parse_ai_review_delta_from_text(text).is_err(),
            "expected missing spec.driver to error"
        );
    }

    #[test]
    fn parses_review_delta_with_explicit_driver_and_clip() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_animation",
              "component_id":"deadbeef",
              "channel":"move",
              "spec": {
                "driver":"move_phase",
                "clip": {
                  "kind":"loop",
                  "duration_secs": 1.0,
                  "keyframes": [ { "time_secs": 0.0 } ]
                }
              }
            }
          ]
        }"#;
        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakAnimation { spec, .. } => {
                assert!(matches!(spec.driver, AiAnimationDriverJson::MovePhase));
                assert!(matches!(spec.clip, AiAnimationClipJson::Loop { .. }));
            }
            other => panic!("expected tweak_animation, got {other:?}"),
        }
    }

    #[test]
    fn rejects_plan_with_wrong_version() {
        let text = r#"{"version":6,"components":[{"name":"root","size":[1,1,1]}]}"#;
        assert!(parse_ai_plan_from_text(text).is_err());
    }
}

