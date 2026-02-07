use bevy::log::debug;
use bevy::prelude::{EulerRot, Quat, Vec3};

use super::super::GEN3D_MAX_PARTS;
use super::artifacts::write_gen3d_json_artifact;
use super::schema::{AiDraftJsonV1, AiPlanFillJsonV1, AiPlanJsonV1, AiReviewDeltaJsonV1};

fn normalize_attack_kind(kind: &str) -> Option<&'static str> {
    let mut normalized = kind.trim().to_ascii_lowercase();
    normalized = normalized.replace(' ', "_");
    normalized = normalized.replace('-', "_");
    while normalized.contains("__") {
        normalized = normalized.replace("__", "_");
    }

    match normalized.as_str() {
        "none" | "no" | "no_attack" | "noattack" => Some("none"),
        "melee" | "melee_attack" | "close" | "close_range" | "close_range_melee" => Some("melee"),
        "ranged_projectile" | "ranged" | "ranged_attack" | "rangedprojectile" | "projectile"
        | "shoot" | "gun" | "cannon" => Some("ranged_projectile"),
        other => {
            if other.contains("no_") || other.contains("_none") {
                return Some("none");
            }
            if other.contains("melee")
                || other.contains("slash")
                || other.contains("bite")
                || other.contains("claw")
                || other.contains("punch")
                || other.contains("stab")
            {
                return Some("melee");
            }
            if other.contains("projectile")
                || other.contains("ranged")
                || other.contains("shoot")
                || other.contains("bullet")
                || other.contains("gun")
                || other.contains("cannon")
                || other.contains("rocket")
            {
                return Some("ranged_projectile");
            }
            None
        }
    }
}

fn normalize_attack_kinds_in_json(value: &mut serde_json::Value) -> bool {
    let mut changed = false;
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == "attack" {
                    if let serde_json::Value::Object(attack) = v {
                        if let Some(kind_value) = attack.get_mut("kind") {
                            if let Some(kind_str) = kind_value.as_str() {
                                if let Some(norm) = normalize_attack_kind(kind_str) {
                                    if kind_str != norm {
                                        *kind_value = serde_json::Value::String(norm.to_string());
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
                if normalize_attack_kinds_in_json(v) {
                    changed = true;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                if normalize_attack_kinds_in_json(item) {
                    changed = true;
                }
            }
        }
        _ => {}
    }
    changed
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

    let mut v1: AiDraftJsonV1 = match serde_json::from_value(json_value.clone()) {
        Ok(v1) => v1,
        Err(err) => {
            if json_value.get("version").is_none() {
                debug!("Gen3D: AI JSON missing `version`; assuming version 2.");
            }
            if let Some(converted) = try_convert_nonconforming_ai_draft_schema(&json_value) {
                debug!("Gen3D: attempting to convert nonconforming AI draft schema.");
                serde_json::from_value(converted)
                    .map_err(|err2| format!("AI JSON schema error: {err2}"))?
            } else {
                return Err(format!("AI JSON schema error: {err}"));
            }
        }
    };
    if v1.version == 0 {
        v1.version = 2;
    }
    if v1.version != 2 {
        return Err(format!(
            "Unsupported AI draft version {} (expected 2)",
            v1.version
        ));
    }
    if v1.parts.is_empty() {
        return Err("AI draft has no parts.".into());
    }
    if v1.parts.len() > GEN3D_MAX_PARTS {
        return Err(format!(
            "AI returned too many parts: {} (max {GEN3D_MAX_PARTS})",
            v1.parts.len()
        ));
    }
    let missing_colors = v1.parts.iter().filter(|p| p.color.is_none()).count();
    if missing_colors > 0 {
        return Err(format!(
            "AI draft missing required `color` on {missing_colors}/{} parts.",
            v1.parts.len()
        ));
    }
    debug!(
        "Gen3D: AI draft parsed (version=2, parts={}, anchors={}, collider={})",
        v1.parts.len(),
        v1.anchors.len(),
        if v1.collider.is_some() {
            "custom"
        } else {
            "default"
        }
    );
    Ok(v1)
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
    if normalize_attack_kinds_in_json(&mut json_value) {
        debug!("Gen3D: normalized attack.kind in plan JSON.");
    }

    let mut plan: AiPlanJsonV1 = match serde_json::from_value(json_value.clone()) {
        Ok(plan) => plan,
        Err(err) => {
            if json_value.get("version").is_none() {
                debug!("Gen3D: AI plan JSON missing `version`; assuming version 6.");
            }
            return Err(format!("AI JSON schema error: {err}"));
        }
    };
    if plan.version == 0 {
        // Plan version is treated as an informational hint. Older runs may omit it.
        // Avoid hard-failing here so we can stay compatible with evolving prompts/models.
        plan.version = 6;
        debug!("Gen3D: AI plan JSON missing `version`; assuming version 6.");
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

    let mut fill: AiPlanFillJsonV1 = serde_json::from_value(json_value.clone())
        .map_err(|err| format!("AI JSON schema error: {err}"))?;
    if fill.version == 0 {
        fill.version = 1;
    }
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
    let mut json_value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if normalize_attack_kinds_in_json(&mut json_value) {
        debug!("Gen3D: normalized attack.kind in review-delta JSON.");
    }
    let mut delta: AiReviewDeltaJsonV1 =
        serde_json::from_value(json_value).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if delta.version == 0 {
        delta.version = 1;
    }
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

fn try_convert_nonconforming_ai_draft_schema(
    json: &serde_json::Value,
) -> Option<serde_json::Value> {
    fn normalize_color_json(value: &serde_json::Value) -> Option<serde_json::Value> {
        let arr = value.as_array()?;
        if arr.len() == 4 {
            return Some(value.clone());
        }
        if arr.len() != 3 {
            return None;
        }
        let r = arr.get(0)?.as_f64()? as f32;
        let g = arr.get(1)?.as_f64()? as f32;
        let b = arr.get(2)?.as_f64()? as f32;
        Some(serde_json::json!([r, g, b, 1.0]))
    }

    fn vec3_from_json(value: &serde_json::Value) -> Option<Vec3> {
        let arr = value.as_array()?;
        if arr.len() != 3 {
            return None;
        }
        let x = arr.get(0)?.as_f64()? as f32;
        let y = arr.get(1)?.as_f64()? as f32;
        let z = arr.get(2)?.as_f64()? as f32;
        Some(Vec3::new(x, y, z))
    }

    fn apply_color_value(
        out_part: &mut serde_json::Value,
        value: &serde_json::Value,
        normalize_color_json: fn(&serde_json::Value) -> Option<serde_json::Value>,
    ) {
        if let Some(hex) = value.as_str() {
            if let Some(rgba) = parse_hex_color(hex) {
                out_part["color"] = serde_json::json!(rgba);
                return;
            }
            if let Some(rgba) = parse_named_color(hex) {
                out_part["color"] = serde_json::json!(rgba);
                return;
            }
            // Non-color strings (like "wood") should not be copied into the schema.
            return;
        }
        if let Some(v) = normalize_color_json(value) {
            out_part["color"] = v;
        }
    }

    // Support older outputs: { parts: [{type/shape, size/scale, rot_deg}] } etc.
    let parts = json
        .get("parts")
        .or_else(|| json.get("primitives"))
        .or_else(|| json.get("atoms"))
        .and_then(|v| v.as_array())?;
    let mut new_parts = Vec::new();
    for part in parts {
        if part.get("op").and_then(|v| v.as_str()).is_some_and(|op| {
            matches!(
                op.trim().to_ascii_lowercase().as_str(),
                "subtract" | "difference" | "cut"
            )
        }) {
            continue;
        }

        let primitive = if let Some(p) = part.get("primitive") {
            p.as_str().map(|s| s.to_string())
        } else if let Some(p) = part.get("prim").and_then(|v| v.as_str()) {
            Some(p.to_string())
        } else if let Some(t) = part.get("type").and_then(|v| v.as_str()) {
            Some(t.to_string())
        } else if let Some(s) = part.get("shape").and_then(|v| v.as_str()) {
            Some(s.to_string())
        } else if let Some(k) = part.get("kind").and_then(|v| v.as_str()) {
            Some(k.to_string())
        } else {
            None
        }?;

        let prim_lower = primitive.to_ascii_lowercase();
        let pos = part
            .get("pos")
            .or_else(|| part.get("position"))
            .or_else(|| part.get("translation"))?
            .clone();
        let scale = if part.get("scale").is_some() {
            part.get("scale")?.clone()
        } else if part.get("size").is_some() {
            part.get("size")?.clone()
        } else if part.get("dimensions").is_some() {
            part.get("dimensions")?.clone()
        } else {
            continue;
        };

        let scale_vec = vec3_from_json(&scale);

        // Allow "cylinder_16" etc; ignore non-primitive entries like decals.
        let mut out_part = if prim_lower.contains("cylinder") {
            serde_json::json!({"primitive":"cylinder","pos":pos,"scale":scale})
        } else if prim_lower.contains("cone") {
            serde_json::json!({"primitive":"cone","pos":pos,"scale":scale})
        } else if prim_lower.contains("sphere") {
            serde_json::json!({"primitive":"sphere","pos":pos,"scale":scale})
        } else if prim_lower.contains("cuboid")
            || prim_lower.contains("cube")
            || prim_lower == "box"
            || prim_lower.contains("box")
        {
            serde_json::json!({"primitive":"cuboid","pos":pos,"scale":scale})
        } else if prim_lower.contains("torus") {
            let mut out = serde_json::json!({"primitive":"cylinder","pos":pos,"scale":scale});
            if let Some(params) = part.get("params") {
                let kind = params
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if kind == "torus" {
                    out["params"] = params.clone();
                }
            }
            if out.get("params").is_none() {
                if let Some(scale_vec) = scale_vec {
                    let outer = scale_vec.x.abs().max(scale_vec.z.abs()).max(0.01);
                    let thickness = scale_vec.y.abs().max(0.01);
                    let minor_radius = (thickness * 0.5).max(0.01);
                    let major_radius = (outer * 0.5 - minor_radius).max(0.01);
                    out["params"] = serde_json::json!({
                      "kind":"torus",
                      "minor_radius": minor_radius,
                      "major_radius": major_radius,
                    });
                    out["scale"] = serde_json::json!([1.0, 1.0, 1.0]);
                }
            }
            out
        } else if prim_lower.contains("capsule") {
            let mut out = serde_json::json!({"primitive":"cylinder","pos":pos,"scale":scale});
            if let Some(params) = part.get("params") {
                let kind = params
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if kind == "capsule" {
                    out["params"] = params.clone();
                }
            }
            if out.get("params").is_none() {
                if let Some(scale_vec) = scale_vec {
                    let radius = (scale_vec.x.abs().min(scale_vec.z.abs()) * 0.5).max(0.01);
                    let full_height = scale_vec.y.abs().max(0.01);
                    let half_length = (full_height * 0.5 - radius).max(0.01);
                    out["params"] = serde_json::json!({
                      "kind":"capsule",
                      "radius": radius,
                      "half_length": half_length,
                    });
                    out["scale"] = serde_json::json!([1.0, 1.0, 1.0]);
                }
            }
            out
        } else if prim_lower.contains("frustum") {
            let mut out = serde_json::json!({"primitive":"cylinder","pos":pos,"scale":scale});
            if let Some(params) = part.get("params") {
                let kind = params
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if kind == "conical_frustum" {
                    out["params"] = params.clone();
                }
            }
            if out.get("params").is_none() {
                if let Some(scale_vec) = scale_vec {
                    let radius = (scale_vec.x.abs().min(scale_vec.z.abs()) * 0.5).max(0.01);
                    let height = scale_vec.y.abs().max(0.01);
                    out["params"] = serde_json::json!({
                      "kind":"conical_frustum",
                      "top_radius": radius,
                      "bottom_radius": radius,
                      "height": height,
                    });
                    out["scale"] = serde_json::json!([1.0, 1.0, 1.0]);
                }
            }
            out
        } else {
            continue;
        };

        // Back-compat for older/nonconforming schemas: convert Euler angles into forward/up vectors.
        let mut rot = Quat::IDENTITY;
        if let Some(rot_val) = part.get("rot_degrees").or_else(|| part.get("rot_deg")) {
            if let Some(arr) = rot_val.as_array() {
                if arr.len() == 3 {
                    let yaw = arr[0].as_f64().unwrap_or(0.0) as f32;
                    let pitch = arr[1].as_f64().unwrap_or(0.0) as f32;
                    let roll = arr[2].as_f64().unwrap_or(0.0) as f32;
                    rot = Quat::from_euler(
                        EulerRot::YXZ,
                        yaw.to_radians(),
                        pitch.to_radians(),
                        roll.to_radians(),
                    );
                }
            }
        } else if let Some(yaw_val) = part.get("yaw_degrees").or_else(|| part.get("yaw_deg")) {
            if let Some(yaw) = yaw_val.as_f64() {
                rot = Quat::from_rotation_y((yaw as f32).to_radians());
            }
        }
        if rot.is_finite() && rot.angle_between(Quat::IDENTITY) > 1e-6 {
            let forward = rot * Vec3::Z;
            let up = rot * Vec3::Y;
            if forward.is_finite() && up.is_finite() {
                out_part["forward"] = serde_json::json!([forward.x, forward.y, forward.z]);
                out_part["up"] = serde_json::json!([up.x, up.y, up.z]);
            }
        }

        if let Some(material) = part.get("material") {
            if let Some(material_str) = material.as_str() {
                if let Some(rgba) = parse_named_color(material_str) {
                    out_part["color"] = serde_json::json!(rgba);
                }
            } else {
                if let Some(color) = material.get("color") {
                    apply_color_value(&mut out_part, color, normalize_color_json);
                }
                if let Some(base_color) = material.get("base_color") {
                    apply_color_value(&mut out_part, base_color, normalize_color_json);
                }
                if let Some(name) = material
                    .get("name")
                    .or_else(|| material.get("kind"))
                    .or_else(|| material.get("type"))
                    .and_then(|v| v.as_str())
                {
                    if let Some(rgba) = parse_named_color(name) {
                        out_part["color"] = serde_json::json!(rgba);
                    }
                }
            }
        }

        if let Some(color) = part.get("color") {
            apply_color_value(&mut out_part, color, normalize_color_json);
        }

        new_parts.push(out_part);
    }

    if new_parts.is_empty() {
        return None;
    }

    Some(serde_json::json!({
      "version": 2,
      "collider": json.get("collider").cloned().unwrap_or(serde_json::Value::Null),
      "anchors": [],
      "parts": new_parts,
    }))
}

pub(super) fn parse_named_color(s: &str) -> Option<[f32; 4]> {
    let raw = s.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with('#') {
        return parse_hex_color(raw);
    }

    let lower = raw.to_ascii_lowercase();
    let lower = lower.as_str();

    // Basic named colors.
    let solid = |r: f32, g: f32, b: f32| Some([r, g, b, 1.0]);
    match lower {
        "white" => return solid(0.95, 0.95, 0.95),
        "black" => return solid(0.05, 0.05, 0.05),
        "gray" | "grey" => return solid(0.60, 0.60, 0.62),
        "red" => return solid(0.85, 0.16, 0.14),
        "green" => return solid(0.18, 0.70, 0.30),
        "blue" => return solid(0.20, 0.45, 0.85),
        "yellow" => return solid(0.92, 0.82, 0.20),
        "orange" => return solid(0.92, 0.55, 0.18),
        "brown" => return solid(0.55, 0.36, 0.20),
        _ => {}
    }

    // Common "foo_white"/"painted_red" style values.
    if lower.contains("white") {
        return solid(0.95, 0.95, 0.95);
    }
    if lower.contains("black") {
        return solid(0.05, 0.05, 0.05);
    }
    if lower.contains("gray") || lower.contains("grey") {
        return solid(0.60, 0.60, 0.62);
    }
    if lower.contains("red") {
        return solid(0.85, 0.16, 0.14);
    }
    if lower.contains("green") {
        return solid(0.18, 0.70, 0.30);
    }
    if lower.contains("blue") {
        return solid(0.20, 0.45, 0.85);
    }
    if lower.contains("yellow") {
        return solid(0.92, 0.82, 0.20);
    }
    if lower.contains("orange") {
        return solid(0.92, 0.55, 0.18);
    }
    if lower.contains("brown") {
        return solid(0.55, 0.36, 0.20);
    }

    // Heuristic material names commonly produced by LLMs.
    if lower.contains("wood") {
        if lower.contains("dark") {
            return solid(0.34, 0.23, 0.12);
        }
        if lower.contains("light") {
            return solid(0.72, 0.52, 0.32);
        }
        return solid(0.58, 0.40, 0.22);
    }
    if lower.contains("bark") {
        return solid(0.40, 0.28, 0.16);
    }
    if lower.contains("leaf") || lower.contains("leaves") || lower.contains("foliage") {
        if lower.contains("light") {
            return solid(0.28, 0.72, 0.32);
        }
        if lower.contains("dark") {
            return solid(0.14, 0.45, 0.20);
        }
        return solid(0.18, 0.58, 0.26);
    }
    if lower.contains("metal") || lower.contains("steel") || lower.contains("iron") {
        if lower.contains("dark") {
            return solid(0.38, 0.40, 0.44);
        }
        return solid(0.62, 0.64, 0.68);
    }
    if lower.contains("stone") || lower.contains("rock") || lower.contains("concrete") {
        if lower.contains("dark") {
            return solid(0.40, 0.40, 0.42);
        }
        return solid(0.58, 0.58, 0.60);
    }
    if lower.contains("plastic") {
        if lower.contains("dark") {
            return solid(0.22, 0.22, 0.24);
        }
        return solid(0.82, 0.82, 0.84);
    }
    if lower.contains("rubber") {
        return solid(0.16, 0.16, 0.18);
    }
    if lower.contains("gold") {
        return solid(0.92, 0.78, 0.22);
    }
    if lower.contains("silver") {
        return solid(0.82, 0.84, 0.88);
    }
    if lower.contains("copper") {
        return solid(0.72, 0.42, 0.24);
    }
    if lower.contains("rust") {
        return solid(0.70, 0.30, 0.16);
    }

    None
}

pub(super) fn parse_hex_color(s: &str) -> Option<[f32; 4]> {
    let s = s.trim();
    let s = s.strip_prefix('#')?;
    let (r, g, b, a) = match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            (r, g, b, 255u8)
        }
        8 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            let a = u8::from_str_radix(&s[6..8], 16).ok()?;
            (r, g, b, a)
        }
        _ => return None,
    };
    Some([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
}

#[cfg(test)]
mod tests {
    use super::super::convert;
    use super::super::schema::{AiAttackJson, AiPrimitiveJson, AiReviewDeltaActionJsonV1};
    use super::*;
    use crate::object::registry::{MeshKey, PrimitiveParams};

    #[test]
    fn extracts_last_json_object_when_multiple_present() {
        let text = r#"{"a":1}{"a":2}"#;
        let extracted = extract_json_object(text).expect("should extract JSON object");
        let v: serde_json::Value = serde_json::from_str(&extracted).expect("extracted JSON parses");
        assert_eq!(v.get("a").and_then(|v| v.as_i64()), Some(2));
    }

    #[test]
    fn normalizes_review_delta_attack_kind_synonyms() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {"kind":"tweak_attack","attack":{"kind":"cannon","cooldown_secs":0.25},"reason":"test"}
          ]
        }"#;
        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakAttack { attack, .. } => match attack {
                AiAttackJson::RangedProjectile { cooldown_secs, .. } => {
                    assert_eq!(*cooldown_secs, Some(0.25));
                }
                other => panic!("expected ranged_projectile, got {other:?}"),
            },
            other => panic!("expected tweak_attack, got {other:?}"),
        }
    }

    #[test]
    fn parses_nonconforming_component_schema_with_shape_and_hex_color() {
        let text = r##"{
          "name":"base_pedestal",
          "parts":[
            {
              "pos":[0.0,0.18,0.0],
              "rot_deg":[0.0,0.0,0.0],
              "size":[1.24,0.36,1.24],
              "shape":"cylinder_16",
              "color":"#D62A2A"
            }
          ],
          "assembly":{"pos":[0.0,0.0,0.0]}
        }"##;

        let draft = parse_ai_draft_from_text(text).expect("draft should parse");
        assert_eq!(draft.version, 2);
        assert_eq!(draft.parts.len(), 1);

        let part = &draft.parts[0];
        assert!(matches!(part.primitive, AiPrimitiveJson::Cylinder));
        assert_eq!(part.pos, [0.0, 0.18, 0.0]);
        assert_eq!(part.scale, [1.24, 0.36, 1.24]);
        assert!(part.forward.is_none());
        assert!(part.up.is_none());
        assert!(part.color.is_some());
    }

    #[test]
    fn parses_nonconforming_component_schema_with_type_and_material_base_color() {
        let text = r##"{
          "name":"base_red_cylinder",
          "parts":[
            {
              "type":"cylinder",
              "pos":[0.0,0.11,0.0],
              "rot_deg":[0.0,0.0,0.0],
              "size":[0.64,0.22,0.64],
              "material": { "base_color": [0.83, 0.12, 0.14] }
            },
            {
              "type":"decal_wrap",
              "pos":[0.0,0.11,0.0],
              "size":[0.64,0.22,0.64]
            }
          ],
          "bounds": { "size": [0.64,0.22,0.64] }
        }"##;

        let draft = parse_ai_draft_from_text(text).expect("draft should parse");
        assert_eq!(draft.version, 2);
        // `decal_wrap` is ignored; at least one primitive part remains.
        assert_eq!(draft.parts.len(), 1);

        let part = &draft.parts[0];
        assert!(matches!(part.primitive, AiPrimitiveJson::Cylinder));
        assert_eq!(part.pos, [0.0, 0.11, 0.0]);
        assert_eq!(part.scale, [0.64, 0.22, 0.64]);
        assert!(part.forward.is_none());
        assert!(part.up.is_none());
        assert!(part.color.is_some());
    }

    #[test]
    fn parses_nonconforming_component_schema_with_primitives_and_material_color() {
        let text = r##"{
          "component": "base_pedestal",
          "assembly_transform": { "pos": [0.0, 0.75, 0.0], "rot_deg": [0.0, 0.0, 0.0] },
          "primitives": [
            {
              "name": "pedestal_body",
              "type": "cylinder",
              "pos": [0.0, 0.0, 0.0],
              "rot_deg": [0.0, 0.0, 0.0],
              "size": [3.2, 1.5, 3.2],
              "material": { "color": "#C61B1B" }
            },
            {
              "name": "front_medallion_ring",
              "type": "torus",
              "pos": [0.0, 0.0, 1.69],
              "rot_deg": [90.0, 0.0, 0.0],
              "size": [0.22, 0.05, 0.22],
              "material": { "color": "#D4B15A" }
            }
          ]
        }"##;

        let draft = parse_ai_draft_from_text(text).expect("draft should parse");
        assert_eq!(draft.version, 2);
        assert_eq!(draft.parts.len(), 2);

        let cylinder = &draft.parts[0];
        assert!(matches!(cylinder.primitive, AiPrimitiveJson::Cylinder));
        assert_eq!(cylinder.pos, [0.0, 0.0, 0.0]);
        assert_eq!(cylinder.scale, [3.2, 1.5, 3.2]);
        assert!(cylinder.color.is_some());

        let torus = &draft.parts[1];
        // Torus is represented as a primitive + params.
        assert!(matches!(torus.primitive, AiPrimitiveJson::Cylinder));
        assert_eq!(torus.pos, [0.0, 0.0, 1.69]);
        assert_eq!(torus.scale, [1.0, 1.0, 1.0]);
        let params = torus.params.as_ref().expect("torus params should exist");
        let parsed = convert::primitive_params_from_ai(params, MeshKey::UnitCylinder)
            .expect("params parse should succeed")
            .expect("params should not be None");
        assert!(matches!(parsed, PrimitiveParams::Torus { .. }));
    }

    #[test]
    fn preserves_torus_params_when_provided_in_component_schema() {
        // The schema recommends `primitive` among the core set plus optional `params`.
        // Some models still emit `"primitive":"torus"`; we should preserve the explicit params.
        let text = r##"{
          "version": 2,
          "parts": [
            {
              "primitive": "torus",
              "params": { "kind": "torus", "minor_radius": 0.020, "major_radius": 0.040 },
              "color": [0.08, 0.06, 0.06, 1.0],
              "pos": [0.0, 0.0, 0.0],
              "rot_degrees": [0.0, 90.0, 0.0],
              "scale": [1.0, 1.0, 1.0]
            }
          ]
        }"##;

        let draft = parse_ai_draft_from_text(text).expect("draft should parse");
        assert_eq!(draft.version, 2);
        assert_eq!(draft.parts.len(), 1);

        let part = &draft.parts[0];
        assert!(matches!(part.primitive, AiPrimitiveJson::Cylinder));
        assert_eq!(part.scale, [1.0, 1.0, 1.0]);
        let params = part.params.as_ref().expect("params should be preserved");
        let parsed = convert::primitive_params_from_ai(params, MeshKey::UnitCylinder)
            .expect("params parse should succeed")
            .expect("params should not be None");
        match parsed {
            PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            } => {
                assert!((minor_radius - 0.020).abs() < 1e-6);
                assert!((major_radius - 0.040).abs() < 1e-6);
            }
            other => panic!("expected torus params, got {other:?}"),
        }
    }
}
