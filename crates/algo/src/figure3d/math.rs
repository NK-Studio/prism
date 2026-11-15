//! Small 3D vector and quaternion helpers (`[f64; 3]`, `[w, x, y, z]`).

pub type V3 = [f64; 3];
pub type Q = [f64; 4];

pub const IDENTITY: Q = [1.0, 0.0, 0.0, 0.0];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
/// `a` scaled to unit length, or `fallback` when it has none.
pub fn normalize_or(a: V3, fallback: V3) -> V3 {
    let l = len(a);
    if l.is_finite() && l > 1e-12 { scale(a, 1.0 / l) } else { fallback }
}

/// Hamilton product `a * b` (apply `b`, then `a`).
pub fn qmul(a: Q, b: Q) -> Q {
    let [aw, ax, ay, az] = a;
    let [bw, bx, by, bz] = b;
    [
        aw * bw - ax * bx - ay * by - az * bz,
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
    ]
}
pub fn qconj(q: Q) -> Q {
    [q[0], -q[1], -q[2], -q[3]]
}
pub fn qnorm(q: Q) -> Q {
    prism_doc::figure::normalize_quat(q)
}
/// `v` rotated by the unit quaternion `q`.
pub fn rotate(q: Q, v: V3) -> V3 {
    let u = [q[1], q[2], q[3]];
    let t = scale(cross(u, v), 2.0);
    add(add(v, scale(t, q[0])), cross(u, t))
}
/// Rotation of `radians` about `axis` (any length; identity for a zero axis).
pub fn axis_angle(axis: V3, radians: f64) -> Q {
    let a = normalize_or(axis, [0.0, 0.0, 0.0]);
    if a == [0.0, 0.0, 0.0] || !radians.is_finite() {
        return IDENTITY;
    }
    let (s, c) = (radians / 2.0).sin_cos();
    [c, a[0] * s, a[1] * s, a[2] * s]
}
/// The shortest rotation taking direction `from` to direction `to`.
pub fn between(from: V3, to: V3) -> Q {
    let (a, b) = (normalize_or(from, [0.0; 3]), normalize_or(to, [0.0; 3]));
    if a == [0.0; 3] || b == [0.0; 3] {
        return IDENTITY;
    }
    let d = dot(a, b).clamp(-1.0, 1.0);
    if d > 1.0 - 1e-12 {
        return IDENTITY;
    }
    if d < -1.0 + 1e-12 {
        // Opposite: turn half way round any axis perpendicular to `a`.
        let p = if a[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        return axis_angle(cross(a, p), std::f64::consts::PI);
    }
    let c = cross(a, b);
    qnorm([1.0 + d, c[0], c[1], c[2]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        len(sub(a, b)) < 1e-9
    }

    #[test]
    fn rotations_compose_and_map_directions() {
        let q = axis_angle([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2);
        assert!(close(rotate(q, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
        let qq = qmul(q, q);
        assert!(close(rotate(qq, [1.0, 0.0, 0.0]), [-1.0, 0.0, 0.0]));
        assert!(close(rotate(qmul(qconj(q), q), [0.3, 0.4, 0.5]), [0.3, 0.4, 0.5]));
        for (f, t) in [([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), ([0.0, 1.0, 0.0], [0.0, -1.0, 0.0]), ([1.0, 2.0, 3.0], [-3.0, 0.5, 1.0])] {
            let r = rotate(between(f, t), normalize_or(f, [0.0; 3]));
            assert!(close(r, normalize_or(t, [0.0; 3])), "{f:?} -> {t:?}: {r:?}");
        }
        assert_eq!(between([0.0; 3], [1.0, 0.0, 0.0]), IDENTITY);
        assert_eq!(axis_angle([0.0; 3], 1.0), IDENTITY);
    }
}
