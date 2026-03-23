use super::structured_outputs::Gen3dAiJsonSchemaKind;

fn extract_backticked_token(err: &str, marker: &str) -> Option<String> {
    let start = err.find(marker)?.saturating_add(marker.len());
    let rest = err.get(start..)?;
    let end_rel = rest.find('`')?;
    Some(rest[..end_rel].to_string())
}

fn extract_serde_unknown_field(err: &str) -> Option<String> {
    extract_backticked_token(err, "unknown field `")
}

fn extract_serde_missing_field(err: &str) -> Option<String> {
    extract_backticked_token(err, "missing field `")
}

fn trim_wrapping_quotes(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .trim_matches('`')
        .trim_matches('\'')
        .to_string()
}

fn extract_draft_op_unknown_key(err: &str) -> Option<(String, String)> {
    let prefix = "DraftOp kind=";
    let mid = " includes unknown key ";
    let start = err.find(prefix)?.saturating_add(prefix.len());
    let rest = err.get(start..)?;
    let mid_idx = rest.find(mid)?;
    let kind_raw = rest[..mid_idx].trim();
    let rest2 = rest.get(mid_idx.saturating_add(mid.len())..)?;
    let end_idx = rest2
        .find('.')
        .or_else(|| rest2.find(')'))
        .or_else(|| rest2.find('\n'))
        .unwrap_or(rest2.len());
    let key_raw = rest2[..end_idx].trim();
    Some((trim_wrapping_quotes(kind_raw), trim_wrapping_quotes(key_raw)))
}

fn push_once(out: &mut Vec<String>, msg: impl Into<String>) {
    let msg = msg.into();
    if msg.trim().is_empty() {
        return;
    }
    if out.iter().any(|s| s.trim() == msg.trim()) {
        return;
    }
    out.push(msg);
}

pub(super) fn build_schema_repair_hints(
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    err: &str,
) -> Vec<String> {
    let err = err.trim();
    if err.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<String> = Vec::new();

    match expected_schema {
        Some(Gen3dAiJsonSchemaKind::PlanOpsV1) => {
            if let Some(field) = extract_serde_unknown_field(err) {
                match field.as_str() {
                    "attach_to" => {
                        push_once(
                            &mut out,
                            "For PlanOps op `kind=\"set_attach_to\"`, use the key `set_attach_to` (not `attach_to`).",
                        );
                        push_once(
                            &mut out,
                            "`attach_to` appears inside full-plan components; PlanOps patches use `set_attach_to`.",
                        );
                    }
                    "name" => {
                        push_once(
                            &mut out,
                            "In PlanOps patches, use `component` to refer to an existing component; `name` is only for `add_component`.",
                        );
                    }
                    "component" => {
                        push_once(
                            &mut out,
                            "In PlanOps `add_component`, the new component identifier field is `name` (not `component`).",
                        );
                    }
                    _ => {}
                }
            }

            if err.contains("add_component")
                && err.contains("both `name` and `component`")
                && err.contains("different values")
            {
                push_once(
                    &mut out,
                    "For PlanOps `add_component`, output only `name` for the new component id; omit `component`.",
                );
            }
        }
        Some(Gen3dAiJsonSchemaKind::DraftOpsV1) => {
            if let Some((kind, key)) = extract_draft_op_unknown_key(err) {
                if kind == "upsert_animation_slot" {
                    match key.as_str() {
                        "clip" => {
                            push_once(
                                &mut out,
                                "For `upsert_animation_slot`, top-level keys are `kind`, `child_component`, `channel`, `slot`; put the clip under `slot.clip`.",
                            );
                        }
                        "clip_kind" => {
                            push_once(&mut out, "Use `slot.clip.kind` (not `clip_kind`).");
                        }
                        "driver" => {
                            push_once(&mut out, "Use `slot.driver` (driver is nested under `slot`).");
                        }
                        _ => {}
                    }
                }

                if matches!(
                    kind.as_str(),
                    "set_attachment_offset"
                        | "set_attachment_joint"
                        | "upsert_animation_slot"
                        | "scale_animation_slot_rotation"
                        | "remove_animation_slot"
                ) && key == "component"
                {
                    push_once(&mut out, "Use `child_component` (not `component`) for attachment/motion-slot DraftOps.");
                }
            }

            if err.contains("Missing JSON object in DraftOps output") {
                push_once(
                    &mut out,
                    "Return a single JSON object (no markdown), like {\"version\":1,\"ops\":[...]}",
                );
            }

            if err.contains("DraftOps schema mismatch:") {
                if let Some(field) = extract_serde_missing_field(err) {
                    if field == "ops" {
                        push_once(
                            &mut out,
                            "Top-level DraftOps output must include `ops`: {\"version\":1,\"ops\":[...]}",
                        );
                    }
                }
            }
        }
        Some(Gen3dAiJsonSchemaKind::PlanV1) => {
            if err.contains("AI plan JSON missing required `version`")
                || err.contains("Unsupported AI plan version")
            {
                push_once(&mut out, "Set top-level `version` to 8 for Gen3D plans.");
            }
        }
        Some(Gen3dAiJsonSchemaKind::ComponentDraftV1) => {
            if err.contains("AI draft JSON missing required `version`")
                || err.contains("Unsupported AI draft version")
            {
                push_once(&mut out, "Set top-level `version` to 2 for a component draft.");
            }
            if err.contains("AI draft has no parts") {
                push_once(&mut out, "Include at least 1 entry in `parts`.");
            }
            if err.contains("AI draft missing required `color`") {
                push_once(
                    &mut out,
                    "Every part must include `color`: [r,g,b,a] with 0..1 floats.",
                );
            }
        }
        Some(Gen3dAiJsonSchemaKind::MotionAuthoringV1) => {
            if err.contains("AI motion-authoring JSON missing required `version`")
                || err.contains("Unsupported AI motion-authoring version")
            {
                push_once(
                    &mut out,
                    "Set top-level `version` to 1 for motion authoring output.",
                );
            }
        }
        Some(Gen3dAiJsonSchemaKind::ReviewDeltaV1 | Gen3dAiJsonSchemaKind::ReviewDeltaNoRegenV1) => {
            if err.contains("AI review-delta JSON missing required `version`")
                || err.contains("Unsupported AI review-delta version")
            {
                push_once(&mut out, "Set top-level `version` to 1 for review-delta output.");
            }
        }
        _ => {}
    }

    if out.len() > 3 {
        out.truncate(3);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_plan_ops_attach_to_alias() {
        let err = "llm_generate_plan_ops_v1: AI JSON schema error: unknown field `attach_to`, expected `component` or `set_attach_to`";
        let hints = build_schema_repair_hints(Some(Gen3dAiJsonSchemaKind::PlanOpsV1), err);
        assert!(
            hints.iter().any(|h| h.contains("set_attach_to")),
            "expected a set_attach_to hint, got: {hints:?}"
        );
    }

    #[test]
    fn hints_draft_ops_upsert_animation_slot_clip_nesting() {
        let err = "DraftOp kind=\"upsert_animation_slot\" includes unknown key \"clip\". Allowed keys: [\"kind\",\"child_component\",\"channel\",\"slot\"].";
        let hints = build_schema_repair_hints(Some(Gen3dAiJsonSchemaKind::DraftOpsV1), err);
        assert!(
            hints.iter().any(|h| h.contains("slot.clip")),
            "expected a slot.clip hint, got: {hints:?}"
        );
    }

    #[test]
    fn hints_draft_ops_component_vs_child_component() {
        let err = "DraftOp kind=\"set_attachment_offset\" includes unknown key \"component\". Allowed keys: [\"kind\",\"child_component\",\"set\"].";
        let hints = build_schema_repair_hints(Some(Gen3dAiJsonSchemaKind::DraftOpsV1), err);
        assert!(
            hints.iter().any(|h| h.contains("child_component")),
            "expected a child_component hint, got: {hints:?}"
        );
    }

    #[test]
    fn hints_component_missing_color() {
        let err = "AI draft missing required `color` on 1/3 parts.";
        let hints = build_schema_repair_hints(Some(Gen3dAiJsonSchemaKind::ComponentDraftV1), err);
        assert!(
            hints.iter().any(|h| h.contains("Every part") && h.contains("color")),
            "expected a color hint, got: {hints:?}"
        );
    }
}
