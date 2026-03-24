use bevy::prelude::*;

use super::Gen3dPlannedComponent;

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
    let hint = hint.trim();
    if hint.is_empty() {
        return None;
    }

    // Safety: only exact match OR unique normalized match. No fuzzy scoring.
    if let Some(idx) = components.iter().position(|c| c.name == hint) {
        return Some(idx);
    }

    let hint_norm = normalize_identifier_for_match(hint);
    if hint_norm.is_empty() {
        return None;
    }

    let mut found: Option<usize> = None;
    for (idx, c) in components.iter().enumerate() {
        if normalize_identifier_for_match(c.name.as_str()) == hint_norm {
            if found.is_some() {
                // Ambiguous normalized match: refuse to guess.
                return None;
            }
            found = Some(idx);
        }
    }
    found
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
