use bevy::log::debug;
use bevy::prelude::{EulerRot, Quat, Vec3};

use super::super::GEN3D_MAX_PARTS;
use super::artifacts::write_gen3d_json_artifact;
use super::schema::{
    AiDescriptorMetaJsonV1, AiDraftJsonV1, AiPlanFillJsonV1, AiPlanJsonV1, AiReviewDeltaJsonV1,
};

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

fn normalize_animation_driver(raw: &str) -> Option<&'static str> {
    let normalized = normalize_snake_case_token(raw);
    match normalized.as_str() {
        "always" | "idle" | "ambient" => Some("always"),
        "move_phase" | "movephase" | "move_cycle" | "movecycle" | "move_cycle_phase"
        | "move_cycle_m" | "move_cycle_meters" => Some("move_phase"),
        "move_distance" | "movedistance" | "distance" => Some("move_distance"),
        "attack_time" | "attacktime" | "attack" | "attack_primary" => Some("attack_time"),
        _ => None,
    }
}

fn normalize_animation_clip_kind(raw: &str) -> Option<&'static str> {
    let normalized = normalize_snake_case_token(raw);
    match normalized.as_str() {
        "loop" | "cycle" => Some("loop"),
        "spin" | "rotate" | "rotation" => Some("spin"),
        _ => None,
    }
}

fn driver_fallback_for_channel(channel: &str) -> &'static str {
    let normalized = normalize_snake_case_token(channel);
    match normalized.as_str() {
        "move" => "move_phase",
        "attack_primary" | "attack" => "attack_time",
        _ => "always",
    }
}

fn max_keyframe_time_secs(value: &serde_json::Value) -> Option<f64> {
    let arr = value.as_array()?;
    let mut max: Option<f64> = None;
    for item in arr {
        let t = item.get("time_secs").and_then(|v| v.as_f64());
        if let Some(t) = t {
            if !t.is_finite() {
                continue;
            }
            max = Some(max.map_or(t, |m| m.max(t)));
        }
    }
    max
}

fn normalize_review_delta_json(value: &mut serde_json::Value) -> bool {
    let mut changed = false;

    fn rename_key(
        map: &mut serde_json::Map<String, serde_json::Value>,
        from: &str,
        to: &str,
    ) -> bool {
        if !map.contains_key(from) {
            return false;
        }
        if map.contains_key(to) {
            map.remove(from);
            return true;
        }
        let Some(value) = map.remove(from) else {
            return false;
        };
        map.insert(to.to_string(), value);
        true
    }

    fn normalize_transform_set_delta_aliases(
        map: &mut serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        // The review prompt includes a compact label `offset.pos(join_frame)=...` for attachment
        // offsets. Some models incorrectly copy that label into JSON keys like `offset_pos_join`.
        // Accept those near-miss keys deterministically by rewriting them to the schema fields.
        let mut changed = false;
        changed |= rename_key(map, "offset_pos_join", "pos");
        changed |= rename_key(map, "offset_pos", "pos");
        changed |= rename_key(map, "offset_scale_join", "scale");
        changed |= rename_key(map, "offset_scale", "scale");
        changed
    }

    fn json_vec3(value: &serde_json::Value) -> Option<Vec3> {
        let arr = value.as_array()?;
        if arr.len() != 3 {
            return None;
        }
        let x = arr[0].as_f64()? as f32;
        let y = arr[1].as_f64()? as f32;
        let z = arr[2].as_f64()? as f32;
        let v = Vec3::new(x, y, z);
        if v.is_finite() {
            Some(v)
        } else {
            None
        }
    }

    fn normalize_rotation_basis_to_quat_xyzw(
        map: &mut serde_json::Map<String, serde_json::Value>,
        forward_key: &str,
        up_key: &str,
    ) -> Option<[f32; 4]> {
        let forward = map.get(forward_key).and_then(json_vec3)?;
        let up = map.get(up_key).and_then(json_vec3);
        let q = super::convert::plan_rotation_from_forward_up(forward, up);
        if q.is_finite() {
            Some([q.x, q.y, q.z, q.w])
        } else {
            None
        }
    }

    fn normalize_transform_set_json(map: &mut serde_json::Map<String, serde_json::Value>) -> bool {
        use serde_json::Value;

        let mut changed = false;

        changed |= normalize_transform_set_delta_aliases(map);

        // Some models incorrectly emit `quat_xyzw`/`forward`/`up` at the top level instead of
        // nesting under `set.rot`.
        if !map.contains_key("rot") {
            if let Some(q) = map.remove("quat_xyzw") {
                map.insert(
                    "rot".to_string(),
                    Value::Object(serde_json::Map::from_iter([("quat_xyzw".to_string(), q)])),
                );
                changed = true;
            } else if let Some(q) = map.remove("rot_quat_xyzw") {
                map.insert(
                    "rot".to_string(),
                    Value::Object(serde_json::Map::from_iter([("quat_xyzw".to_string(), q)])),
                );
                changed = true;
            } else if map.contains_key("forward") {
                let forward = map.remove("forward").unwrap_or(Value::Null);
                let up = map.remove("up").unwrap_or(Value::Null);
                let mut rot_obj = serde_json::Map::new();
                rot_obj.insert("forward".to_string(), forward);
                if !matches!(up, Value::Null) {
                    rot_obj.insert("up".to_string(), up);
                }
                map.insert("rot".to_string(), Value::Object(rot_obj));
                changed = true;
            }
        }

        // Some models incorrectly nest set transforms under `offset` (copied from scene_graph_summary label).
        if let Some(Value::Object(mut offset_obj)) = map.remove("offset") {
            changed = true;
            changed |= normalize_transform_set_delta_aliases(&mut offset_obj);

            if !map.contains_key("pos") {
                if let Some(v) = offset_obj.get("pos").cloned() {
                    map.insert("pos".to_string(), v);
                }
            }
            if !map.contains_key("scale") {
                if let Some(v) = offset_obj.get("scale").cloned() {
                    map.insert("scale".to_string(), v);
                }
            }
            if !map.contains_key("rot") {
                if let Some(q) = offset_obj.get("quat_xyzw").cloned() {
                    map.insert(
                        "rot".to_string(),
                        Value::Object(serde_json::Map::from_iter([("quat_xyzw".to_string(), q)])),
                    );
                } else if let Some(q) = offset_obj.get("rot_quat_xyzw").cloned() {
                    map.insert(
                        "rot".to_string(),
                        Value::Object(serde_json::Map::from_iter([("quat_xyzw".to_string(), q)])),
                    );
                } else if offset_obj.contains_key("forward") {
                    let forward = offset_obj.get("forward").cloned().unwrap_or(Value::Null);
                    let up = offset_obj.get("up").cloned().unwrap_or(Value::Null);
                    let mut rot_obj = serde_json::Map::new();
                    rot_obj.insert("forward".to_string(), forward);
                    if !matches!(up, Value::Null) {
                        rot_obj.insert("up".to_string(), up);
                    }
                    map.insert("rot".to_string(), Value::Object(rot_obj));
                }
            }
        }

        changed
    }

    fn normalize_transform_delta_json(
        map: &mut serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        use serde_json::Value;

        let mut changed = false;

        changed |= normalize_transform_set_delta_aliases(map);
        changed |= rename_key(map, "quat_xyzw", "rot_quat_xyzw");

        // Some models incorrectly use `rot: {quat_xyzw|forward/up}` in deltas; convert to `rot_quat_xyzw`.
        for rot_key in ["rot", "rotation"] {
            let Some(rot_value) = map.remove(rot_key) else {
                continue;
            };
            changed = true;
            let mut rot_obj = match rot_value {
                Value::Object(obj) => obj,
                _ => continue,
            };
            if !map.contains_key("rot_quat_xyzw") {
                if let Some(q) = rot_obj
                    .remove("rot_quat_xyzw")
                    .or_else(|| rot_obj.remove("quat_xyzw"))
                {
                    map.insert("rot_quat_xyzw".to_string(), q);
                } else if let Some(quat_xyzw) =
                    normalize_rotation_basis_to_quat_xyzw(&mut rot_obj, "forward", "up")
                {
                    map.insert("rot_quat_xyzw".to_string(), serde_json::json!(quat_xyzw));
                }
            }
        }

        // Some models incorrectly nest deltas under `offset`.
        if let Some(Value::Object(mut offset_obj)) = map.remove("offset") {
            changed = true;
            changed |= normalize_transform_set_delta_aliases(&mut offset_obj);

            if !map.contains_key("pos") {
                if let Some(v) = offset_obj.get("pos").cloned() {
                    map.insert("pos".to_string(), v);
                }
            }
            if !map.contains_key("scale") {
                if let Some(v) = offset_obj.get("scale").cloned() {
                    map.insert("scale".to_string(), v);
                }
            }
            if !map.contains_key("rot_quat_xyzw") {
                if let Some(q) = offset_obj
                    .get("rot_quat_xyzw")
                    .cloned()
                    .or_else(|| offset_obj.get("quat_xyzw").cloned())
                {
                    map.insert("rot_quat_xyzw".to_string(), q);
                } else if let Some(Value::Object(mut inner_rot)) = offset_obj.get("rot").cloned() {
                    if let Some(q) = inner_rot
                        .remove("rot_quat_xyzw")
                        .or_else(|| inner_rot.remove("quat_xyzw"))
                    {
                        map.insert("rot_quat_xyzw".to_string(), q);
                    } else if let Some(quat_xyzw) =
                        normalize_rotation_basis_to_quat_xyzw(&mut inner_rot, "forward", "up")
                    {
                        map.insert("rot_quat_xyzw".to_string(), serde_json::json!(quat_xyzw));
                    }
                } else if let Some(quat_xyzw) =
                    normalize_rotation_basis_to_quat_xyzw(&mut offset_obj, "forward", "up")
                {
                    map.insert("rot_quat_xyzw".to_string(), serde_json::json!(quat_xyzw));
                }
            }
        }

        // Some models incorrectly emit `forward`/`up` at the top level for delta rotations.
        if !map.contains_key("rot_quat_xyzw") && map.contains_key("forward") {
            if let Some(quat_xyzw) = normalize_rotation_basis_to_quat_xyzw(map, "forward", "up") {
                map.insert("rot_quat_xyzw".to_string(), serde_json::json!(quat_xyzw));
            }
            map.remove("forward");
            map.remove("up");
            changed = true;
        }

        changed
    }

    fn normalize_anchor_delta_json(map: &mut serde_json::Map<String, serde_json::Value>) -> bool {
        let mut changed = false;

        changed |= rename_key(map, "quat_xyzw", "rot_quat_xyzw");
        if !map.contains_key("rot_quat_xyzw") && map.contains_key("forward") {
            if let Some(quat_xyzw) = normalize_rotation_basis_to_quat_xyzw(map, "forward", "up") {
                map.insert("rot_quat_xyzw".to_string(), serde_json::json!(quat_xyzw));
            }
            map.remove("forward");
            map.remove("up");
            changed = true;
        }
        if let Some(offset) = map.remove("offset") {
            // Reject/strip nested shapes like `{delta:{offset:{pos,quat_xyzw}}}`.
            if let serde_json::Value::Object(mut offset_obj) = offset {
                if !map.contains_key("pos") {
                    if let Some(v) = offset_obj.remove("pos") {
                        map.insert("pos".to_string(), v);
                    }
                }
                if !map.contains_key("rot_quat_xyzw") {
                    if let Some(v) = offset_obj
                        .remove("rot_quat_xyzw")
                        .or_else(|| offset_obj.remove("quat_xyzw"))
                    {
                        map.insert("rot_quat_xyzw".to_string(), v);
                    }
                }
            }
            changed = true;
        }

        changed
    }

    fn normalize_animation_spec_clip_shorthand(
        spec_obj: &mut serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        use serde_json::Value;

        let mut changed = false;

        const CLIP_FIELDS: [&str; 6] = [
            "kind",
            "duration_secs",
            "duration",
            "keyframes",
            "axis",
            "radians_per_unit",
        ];

        // Some models emit an animation spec that directly contains clip fields (kind/keyframes/etc)
        // instead of nesting them under `spec.clip`. Convert this deterministically.

        // Also accept an alternate tagged union shape where `spec` or `spec.clip` uses `{ "loop": {...} }`.
        let mut clip_value: Option<Value> = spec_obj.remove("clip");
        if clip_value.is_none() {
            for tagged_kind in ["loop", "spin"] {
                if let Some(tagged) = spec_obj.remove(tagged_kind) {
                    if let Value::Object(inner) = tagged {
                        let mut clip_obj = inner;
                        clip_obj.insert("kind".to_string(), Value::String(tagged_kind.to_string()));
                        clip_value = Some(Value::Object(clip_obj));
                        changed = true;
                        break;
                    } else {
                        // Put it back if it wasn't an object.
                        spec_obj.insert(tagged_kind.to_string(), tagged);
                    }
                }
            }
        }

        // If `clip` exists but isn't an object, interpret it as a kind string when possible.
        if let Some(clip) = clip_value.take() {
            match clip {
                Value::Object(map) => {
                    clip_value = Some(Value::Object(map));
                }
                Value::String(kind) => {
                    let mut map = serde_json::Map::new();
                    if !kind.trim().is_empty() {
                        map.insert("kind".to_string(), Value::String(kind));
                    }
                    clip_value = Some(Value::Object(map));
                    changed = true;
                }
                Value::Null => {
                    changed = true;
                }
                other => {
                    let mut map = serde_json::Map::new();
                    map.insert("kind".to_string(), other);
                    clip_value = Some(Value::Object(map));
                    changed = true;
                }
            }
        }

        // If there's still no clip object, but clip fields exist at spec-level, lift them.
        if clip_value.is_none() && CLIP_FIELDS.iter().any(|k| spec_obj.contains_key(*k)) {
            let mut clip_obj = serde_json::Map::new();
            for key in CLIP_FIELDS {
                if let Some(v) = spec_obj.remove(key) {
                    clip_obj.insert(key.to_string(), v);
                    changed = true;
                }
            }
            clip_value = Some(Value::Object(clip_obj));
        }

        // If we have a clip object, merge any stray spec-level clip fields into it and accept
        // `{ "loop": {...} }` / `{ "spin": {...} }` wrapper objects.
        if let Some(Value::Object(mut clip_obj)) = clip_value.take() {
            for key in CLIP_FIELDS {
                let Some(spec_val) = spec_obj.remove(key) else {
                    continue;
                };
                changed = true;
                let should_overwrite = match key {
                    "keyframes" => {
                        clip_obj
                            .get("keyframes")
                            .and_then(|v| v.as_array())
                            .is_some_and(|arr| arr.is_empty())
                            && spec_val.as_array().is_some_and(|arr| !arr.is_empty())
                    }
                    "duration_secs" => {
                        clip_obj
                            .get("duration_secs")
                            .and_then(|v| v.as_f64())
                            .is_none_or(|v| !(v.is_finite() && v > 0.0))
                            && spec_val.as_f64().is_some_and(|v| v.is_finite() && v > 0.0)
                    }
                    "kind" => {
                        clip_obj
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .is_none_or(|s| s.trim().is_empty())
                            && spec_val.as_str().is_some_and(|s| !s.trim().is_empty())
                    }
                    _ => false,
                };
                if should_overwrite || !clip_obj.contains_key(key) {
                    clip_obj.insert(key.to_string(), spec_val);
                }
            }

            if clip_obj.get("kind").and_then(|v| v.as_str()).is_none()
                && clip_obj.len() == 1
                && (clip_obj.contains_key("loop") || clip_obj.contains_key("spin"))
            {
                let wrapper_key = if clip_obj.contains_key("loop") {
                    "loop"
                } else {
                    "spin"
                };
                if let Some(Value::Object(inner)) = clip_obj.remove(wrapper_key) {
                    let mut flattened = inner;
                    flattened.insert("kind".to_string(), Value::String(wrapper_key.to_string()));
                    clip_obj = flattened;
                    changed = true;
                }
            }

            spec_obj.insert("clip".to_string(), Value::Object(clip_obj));
        } else if let Some(clip_value) = clip_value {
            spec_obj.insert("clip".to_string(), clip_value);
        }

        changed
    }

    let Some(actions) = value
        .as_object_mut()
        .and_then(|root| root.get_mut("actions"))
        .and_then(|v| v.as_array_mut())
    else {
        return false;
    };

    for action in actions {
        let Some(action_obj) = action.as_object_mut() else {
            continue;
        };
        let kind = action_obj
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match kind {
            "tweak_animation" => {}
            "tweak_component_transform" => {
                if let Some(set_obj) = action_obj.get_mut("set").and_then(|v| v.as_object_mut()) {
                    if normalize_transform_set_json(set_obj) {
                        changed = true;
                    }
                }
                if let Some(delta_obj) = action_obj.get_mut("delta").and_then(|v| v.as_object_mut())
                {
                    if normalize_transform_delta_json(delta_obj) {
                        changed = true;
                    }
                }
                continue;
            }
            "tweak_anchor" => {
                if let Some(delta_obj) = action_obj.get_mut("delta").and_then(|v| v.as_object_mut())
                {
                    if normalize_anchor_delta_json(delta_obj) {
                        changed = true;
                    }
                }
                continue;
            }
            _ => continue,
        }

        let channel = action_obj
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let driver_fallback = driver_fallback_for_channel(channel);

        let Some(spec_obj) = action_obj.get_mut("spec").and_then(|v| v.as_object_mut()) else {
            continue;
        };

        if normalize_animation_spec_clip_shorthand(spec_obj) {
            changed = true;
        }

        let next_driver = match spec_obj.get("driver").and_then(|v| v.as_str()) {
            Some(raw) => normalize_animation_driver(raw).unwrap_or(driver_fallback),
            None => driver_fallback,
        };
        if spec_obj
            .get("driver")
            .and_then(|v| v.as_str())
            .is_none_or(|cur| cur != next_driver)
        {
            spec_obj.insert(
                "driver".to_string(),
                serde_json::Value::String(next_driver.to_string()),
            );
            changed = true;
        }

        let Some(clip_obj) = spec_obj.get_mut("clip").and_then(|v| v.as_object_mut()) else {
            continue;
        };

        let inferred_kind =
            if clip_obj.contains_key("axis") && clip_obj.contains_key("radians_per_unit") {
                Some("spin")
            } else if clip_obj.contains_key("keyframes")
                || clip_obj.contains_key("duration_secs")
                || clip_obj.contains_key("duration")
            {
                Some("loop")
            } else {
                None
            };

        let mut clip_kind = clip_obj
            .get("kind")
            .and_then(|v| v.as_str())
            .and_then(normalize_animation_clip_kind)
            .or(inferred_kind);
        if clip_kind.is_none() {
            continue;
        }
        let clip_kind_str = clip_kind.take().unwrap();
        if clip_obj
            .get("kind")
            .and_then(|v| v.as_str())
            .is_none_or(|cur| cur != clip_kind_str)
        {
            clip_obj.insert(
                "kind".to_string(),
                serde_json::Value::String(clip_kind_str.to_string()),
            );
            changed = true;
        }

        if clip_kind_str != "loop" {
            continue;
        }

        let duration_secs_value = clip_obj.get("duration_secs").and_then(|v| v.as_f64());
        let duration_secs_ok = duration_secs_value.is_some_and(|v| v.is_finite() && v > 0.0);

        if !duration_secs_ok {
            if let Some(duration) = clip_obj.get("duration").and_then(|v| v.as_f64()) {
                let duration = if duration.is_finite() && duration > 0.0 {
                    duration
                } else {
                    1.0
                };
                clip_obj.insert(
                    "duration_secs".to_string(),
                    serde_json::Value::Number(
                        serde_json::Number::from_f64(duration)
                            .unwrap_or_else(|| serde_json::Number::from_f64(1.0).unwrap()),
                    ),
                );
                changed = true;
            } else if let Some(keyframes) = clip_obj.get("keyframes") {
                let max_time = max_keyframe_time_secs(keyframes).unwrap_or(0.0);
                let duration = if max_time > 0.0 { max_time } else { 1.0 };
                if let Some(n) = serde_json::Number::from_f64(duration) {
                    clip_obj.insert("duration_secs".to_string(), serde_json::Value::Number(n));
                    changed = true;
                }
            }
        }

        if clip_obj.contains_key("duration") {
            clip_obj.remove("duration");
            changed = true;
        }
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
                debug!("Gen3D: AI plan JSON missing `version`; assuming version 7.");
            }
            return Err(format!("AI JSON schema error: {err}"));
        }
    };
    if plan.version == 0 {
        // Plan version is treated as an informational hint. Older runs may omit it.
        // Avoid hard-failing here so we can stay compatible with evolving prompts/models.
        plan.version = 7;
        debug!("Gen3D: AI plan JSON missing `version`; assuming version 7.");
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
    if normalize_review_delta_json(&mut json_value) {
        debug!("Gen3D: normalized review-delta JSON (driver/duration/etc).");
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

    let mut meta: AiDescriptorMetaJsonV1 = serde_json::from_value(json_value.clone())
        .map_err(|err| format!("AI JSON schema error: {err}"))?;
    if meta.version == 0 {
        meta.version = 1;
    }
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
    use super::super::schema::{
        AiAnimationClipJson, AiAnimationDriverJson, AiAttackJson, AiPrimitiveJson,
        AiReviewDeltaActionJsonV1,
    };
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
    fn normalizes_review_delta_missing_driver_and_duration() {
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
                assert!(
                    matches!(spec.driver, AiAnimationDriverJson::MovePhase),
                    "expected driver move_phase for channel=move"
                );
                match &spec.clip {
                    AiAnimationClipJson::Loop { duration_secs, .. } => {
                        assert!(
                            (*duration_secs - 1.0).abs() < 1e-6,
                            "expected duration_secs=1.0 default"
                        );
                    }
                    other => panic!("expected loop clip, got {other:?}"),
                }
            }
            other => panic!("expected tweak_animation, got {other:?}"),
        }
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
    fn normalizes_review_delta_animation_spec_clip_shorthand_fields() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_animation",
              "component_id":"deadbeef",
              "channel":"idle",
              "spec": {
                "driver": "always",
                "kind":"loop",
                "duration_secs": 2.0,
                "keyframes": [ { "time_secs": 0.0 }, { "time_secs": 2.0 } ]
              }
            }
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakAnimation { spec, .. } => {
                assert!(matches!(spec.driver, AiAnimationDriverJson::Always));
                match &spec.clip {
                    AiAnimationClipJson::Loop {
                        duration_secs,
                        keyframes,
                    } => {
                        assert!((*duration_secs - 2.0).abs() < 1e-6);
                        assert_eq!(keyframes.len(), 2);
                    }
                    other => panic!("expected loop clip, got {other:?}"),
                }
            }
            other => panic!("expected tweak_animation, got {other:?}"),
        }
    }

    #[test]
    fn normalizes_review_delta_animation_clip_wrapped_loop_object() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_animation",
              "component_id":"deadbeef",
              "channel":"idle",
              "spec": {
                "driver": "always",
                "clip": { "loop": { "duration_secs": 1.2, "keyframes": [ { "time_secs": 0.0 } ] } }
              }
            }
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakAnimation { spec, .. } => {
                assert!(matches!(spec.driver, AiAnimationDriverJson::Always));
                match &spec.clip {
                    AiAnimationClipJson::Loop { duration_secs, .. } => {
                        assert!((*duration_secs - 1.2).abs() < 1e-6);
                    }
                    other => panic!("expected loop clip, got {other:?}"),
                }
            }
            other => panic!("expected tweak_animation, got {other:?}"),
        }
    }

    #[test]
    fn normalizes_review_delta_animation_spec_wrapped_loop_object() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_animation",
              "component_id":"deadbeef",
              "channel":"idle",
              "spec": {
                "driver": "always",
                "loop": { "duration_secs": 1.0, "keyframes": [ { "time_secs": 0.0 } ] }
              }
            }
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakAnimation { spec, .. } => match &spec.clip {
                AiAnimationClipJson::Loop { duration_secs, .. } => {
                    assert!((*duration_secs - 1.0).abs() < 1e-6);
                }
                other => panic!("expected loop clip, got {other:?}"),
            },
            other => panic!("expected tweak_animation, got {other:?}"),
        }
    }

    #[test]
    fn normalizes_review_delta_transform_offset_pos_join_alias() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_component_transform",
              "component_id":"deadbeef",
              "delta": { "offset_pos_join": [0.0, 0.0, 0.06] }
            }
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakComponentTransform { delta, .. } => {
                let Some(delta) = delta.as_ref() else {
                    panic!("expected delta to be present");
                };
                assert_eq!(delta.pos, Some([0.0, 0.0, 0.06]));
            }
            other => panic!("expected tweak_component_transform, got {other:?}"),
        }
    }

    #[test]
    fn normalizes_review_delta_transform_delta_nested_offset_pos() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_component_transform",
              "component_id":"deadbeef",
              "delta": { "offset": { "pos": [0.0, 0.0, 0.06] } }
            }
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakComponentTransform { delta, .. } => {
                let Some(delta) = delta.as_ref() else {
                    panic!("expected delta to be present");
                };
                assert_eq!(delta.pos, Some([0.0, 0.0, 0.06]));
            }
            other => panic!("expected tweak_component_transform, got {other:?}"),
        }
    }

    #[test]
    fn normalizes_review_delta_transform_delta_quat_xyzw_alias() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_component_transform",
              "component_id":"deadbeef",
              "delta": { "quat_xyzw": [0.0, 0.0, 0.0, 1.0] }
            }
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakComponentTransform { delta, .. } => {
                let Some(delta) = delta.as_ref() else {
                    panic!("expected delta to be present");
                };
                assert_eq!(delta.rot_quat_xyzw, Some([0.0, 0.0, 0.0, 1.0]));
            }
            other => panic!("expected tweak_component_transform, got {other:?}"),
        }
    }

    #[test]
    fn normalizes_review_delta_anchor_delta_quat_xyzw_alias() {
        let text = r#"{
          "version": 1,
          "applies_to": {"run_id":"run","attempt":0,"plan_hash":"sha256:deadbeef","assembly_rev":0},
          "actions": [
            {
              "kind":"tweak_anchor",
              "component_id":"deadbeef",
              "anchor_name":"mount",
              "delta": { "quat_xyzw": [0.0, 0.0, 0.0, 1.0] }
            }
          ]
        }"#;

        let delta = parse_ai_review_delta_from_text(text).expect("delta should parse");
        assert_eq!(delta.actions.len(), 1);
        match &delta.actions[0] {
            AiReviewDeltaActionJsonV1::TweakAnchor { delta, .. } => {
                let Some(delta) = delta.as_ref() else {
                    panic!("expected delta to be present");
                };
                assert_eq!(delta.rot_quat_xyzw, Some([0.0, 0.0, 0.0, 1.0]));
            }
            other => panic!("expected tweak_anchor, got {other:?}"),
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
