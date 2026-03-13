use bevy::prelude::*;

#[derive(Clone, Debug)]
pub(super) struct BasisFromUpForwardResultV1 {
    pub(super) forward: Vec3,
    pub(super) up: Vec3,
    pub(super) right: Vec3,
    pub(super) forward_source: &'static str,
    pub(super) fallback_axis: Option<&'static str>,
    pub(super) notes: Vec<String>,
}

pub(super) fn basis_from_up_forward_v1(
    up_in: Vec3,
    forward_hint: Option<Vec3>,
) -> Result<BasisFromUpForwardResultV1, String> {
    const EPS: f32 = 1e-6;

    fn is_finite_non_zero(v: Vec3) -> bool {
        v.is_finite() && v.length_squared() > EPS
    }

    fn choose_forward_perpendicular_to(up: Vec3) -> (Vec3, &'static str) {
        let a = up.abs();
        let (axis, axis_name) = if a.x <= a.y && a.x <= a.z {
            (Vec3::X, "x")
        } else if a.y <= a.z {
            (Vec3::Y, "y")
        } else {
            (Vec3::Z, "z")
        };

        let f = axis.cross(up);
        if f.length_squared() <= EPS {
            // Defensive fallback: pick a different axis deterministically.
            let (axis2, axis2_name) = if axis_name == "x" {
                (Vec3::Y, "y")
            } else {
                (Vec3::X, "x")
            };
            return (axis2.cross(up).normalize_or_zero(), axis2_name);
        }
        (f.normalize_or_zero(), axis_name)
    }

    if !is_finite_non_zero(up_in) {
        return Err("`up` must be a finite, non-zero vec3".into());
    }
    let up = up_in.normalize();

    let mut notes: Vec<String> = Vec::new();
    let (forward, forward_source, fallback_axis) = match forward_hint {
        Some(hint_in) => {
            if !is_finite_non_zero(hint_in) {
                return Err("`forward_hint` must be a finite, non-zero vec3 (or omit it)".into());
            }
            let hint = hint_in.normalize();
            let proj = hint - up * hint.dot(up);
            if proj.length_squared() <= EPS {
                let (f, axis_name) = choose_forward_perpendicular_to(up);
                notes.push("forward_hint was parallel to up; used deterministic fallback axis".into());
                (f, "fallback", Some(axis_name))
            } else {
                (proj.normalize(), "projected_forward_hint", None)
            }
        }
        None => {
            let (f, axis_name) = choose_forward_perpendicular_to(up);
            notes.push("forward_hint omitted; used deterministic fallback axis".into());
            (f, "fallback", Some(axis_name))
        }
    };

    if !is_finite_non_zero(forward) {
        return Err("computed forward vector is degenerate (try a different `forward_hint`)".into());
    }

    let right_raw = up.cross(forward);
    if !is_finite_non_zero(right_raw) {
        return Err("could not build a non-degenerate basis: `forward_hint` must not be parallel to `up`".into());
    }
    let right = right_raw.normalize();
    let up_out = forward.cross(right).normalize_or_zero();
    if !is_finite_non_zero(up_out) {
        return Err("could not build a non-degenerate `up` output after orthonormalization".into());
    }

    if notes.len() > 4 {
        notes.truncate(4);
    }

    Ok(BasisFromUpForwardResultV1 {
        forward,
        up: up_out,
        right,
        forward_source,
        fallback_axis,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_unit(v: Vec3, label: &str) {
        let l = v.length();
        assert!((l - 1.0).abs() < 1e-3, "{label} not unit length: {l} v={v:?}");
    }

    fn assert_orthogonal(a: Vec3, b: Vec3, label: &str) {
        let d = a.dot(b).abs();
        assert!(d < 1e-3, "{label} not orthogonal: dot={d} a={a:?} b={b:?}");
    }

    #[test]
    fn fallback_basis_is_orthonormal() {
        let basis = basis_from_up_forward_v1(Vec3::Y, None).expect("basis");
        assert_eq!(basis.forward_source, "fallback");
        assert_eq!(basis.fallback_axis, Some("x"));

        assert_unit(basis.forward, "forward");
        assert_unit(basis.up, "up");
        assert_unit(basis.right, "right");
        assert_orthogonal(basis.forward, basis.up, "forward/up");
        assert_orthogonal(basis.forward, basis.right, "forward/right");
        assert_orthogonal(basis.up, basis.right, "up/right");

        assert!((basis.up - Vec3::Y).length() < 1e-3, "up drifted unexpectedly");
    }

    #[test]
    fn projected_forward_hint_preserves_up() {
        let basis = basis_from_up_forward_v1(Vec3::Y, Some(Vec3::new(1.0, 0.0, 1.0)))
            .expect("basis");
        assert_eq!(basis.forward_source, "projected_forward_hint");
        assert_eq!(basis.fallback_axis, None);
        assert!((basis.up - Vec3::Y).length() < 1e-3, "up changed unexpectedly");
        assert_orthogonal(basis.forward, basis.up, "forward/up");
    }

    #[test]
    fn parallel_forward_hint_triggers_fallback() {
        let basis = basis_from_up_forward_v1(Vec3::Y, Some(Vec3::Y * 10.0)).expect("basis");
        assert_eq!(basis.forward_source, "fallback");
        assert_eq!(basis.fallback_axis, Some("x"));
        assert!(
            basis
                .notes
                .iter()
                .any(|n| n.contains("parallel to up")),
            "expected fallback note, got {:?}",
            basis.notes
        );
        assert!((basis.up - Vec3::Y).length() < 1e-3, "up changed unexpectedly");
    }

    #[test]
    fn zero_up_errors() {
        let err = basis_from_up_forward_v1(Vec3::ZERO, None).unwrap_err();
        assert!(err.contains("up"), "unexpected error: {err}");
    }
}

