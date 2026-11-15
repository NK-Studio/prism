//! Ray–solid intersection for the mannequin's capsules and ellipsoids (closed forms, no meshes).
use super::mannequin::Shape;
use super::math::{V3, add, dot, normalize_or, scale, sub};

/// Nearest hit in front of the ray origin (`d` a unit vector): distance and outward normal.
pub fn hit(shape: &Shape, o: V3, d: V3) -> Option<(f64, V3)> {
    match *shape {
        Shape::Capsule { a, b, r } => capsule(o, d, a, b, r),
        Shape::Ellipsoid { c, axes, radii } => ellipsoid(o, d, c, axes, radii),
    }
}

fn sphere(o: V3, d: V3, c: V3, r: f64) -> Option<f64> {
    let oc = sub(o, c);
    let b = dot(oc, d);
    let h = b * b - (dot(oc, oc) - r * r);
    if h < 0.0 {
        return None;
    }
    let t = -b - h.sqrt();
    (t > 0.0).then_some(t)
}

fn capsule(o: V3, d: V3, pa: V3, pb: V3, r: f64) -> Option<(f64, V3)> {
    let ba = sub(pb, pa);
    let baba = dot(ba, ba);
    let t = if baba < 1e-18 {
        sphere(o, d, pa, r)?
    } else {
        let oa = sub(o, pa);
        let (bard, baoa, rdoa, oaoa) = (dot(ba, d), dot(ba, oa), dot(d, oa), dot(oa, oa));
        let a = baba - bard * bard;
        let b = baba * rdoa - baoa * bard;
        let c = baba * oaoa - baoa * baoa - r * r * baba;
        let h = b * b - a * c;
        let side = if a > 1e-12 && h >= 0.0 {
            let t = (-b - h.sqrt()) / a;
            let y = baoa + t * bard;
            (y > 0.0 && y < baba && t > 0.0).then_some(t)
        } else {
            None
        };
        match side {
            Some(t) => t,
            None => {
                // The caps: the nearer of the two end spheres.
                match (sphere(o, d, pa, r), sphere(o, d, pb, r)) {
                    (Some(x), Some(y)) => x.min(y),
                    (Some(x), None) | (None, Some(x)) => x,
                    (None, None) => return None,
                }
            }
        }
    };
    let p = add(o, scale(d, t));
    let k = if baba < 1e-18 { 0.0 } else { (dot(sub(p, pa), ba) / baba).clamp(0.0, 1.0) };
    let axis = add(pa, scale(ba, k));
    Some((t, normalize_or(sub(p, axis), scale(d, -1.0))))
}

fn ellipsoid(o: V3, d: V3, c: V3, axes: [V3; 3], radii: V3) -> Option<(f64, V3)> {
    let oc = sub(o, c);
    let lo = [dot(oc, axes[0]) / radii[0], dot(oc, axes[1]) / radii[1], dot(oc, axes[2]) / radii[2]];
    let ld = [dot(d, axes[0]) / radii[0], dot(d, axes[1]) / radii[1], dot(d, axes[2]) / radii[2]];
    let a = dot(ld, ld);
    let b = dot(lo, ld);
    let cc = dot(lo, lo) - 1.0;
    let h = b * b - a * cc;
    if a < 1e-18 || h < 0.0 {
        return None;
    }
    let t = (-b - h.sqrt()) / a;
    if t <= 0.0 {
        return None;
    }
    let lp = [lo[0] + t * ld[0], lo[1] + t * ld[1], lo[2] + t * ld[2]];
    let n = add(add(scale(axes[0], lp[0] / radii[0]), scale(axes[1], lp[1] / radii[1])), scale(axes[2], lp[2] / radii[2]));
    Some((t, normalize_or(n, scale(d, -1.0))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_hit_capsules_and_ellipsoids_where_expected() {
        let d = [0.0, 0.0, -1.0];
        let cap = Shape::Capsule { a: [-5.0, 0.0, 0.0], b: [5.0, 0.0, 0.0], r: 1.0 };
        let (t, n) = hit(&cap, [0.0, 0.0, 10.0], d).unwrap();
        assert!((t - 9.0).abs() < 1e-9 && (n[2] - 1.0).abs() < 1e-9, "side");
        let (t, _) = hit(&cap, [5.5, 0.0, 10.0], d).unwrap();
        assert!((t - (10.0 - (1.0f64 - 0.25).sqrt())).abs() < 1e-9, "cap");
        assert!(hit(&cap, [7.0, 0.0, 10.0], d).is_none(), "beyond the cap");
        assert!(hit(&cap, [0.0, 2.0, 10.0], d).is_none());
        assert!(hit(&cap, [0.0, 0.0, -10.0], d).is_none(), "behind the origin");
        // A capsule seen end on still shows its cap.
        let end_on = Shape::Capsule { a: [0.0, 0.0, -5.0], b: [0.0, 0.0, 5.0], r: 1.0 };
        assert!((hit(&end_on, [0.0, 0.0, 10.0], d).unwrap().0 - 4.0).abs() < 1e-9);
        let e = Shape::Ellipsoid { c: [0.0; 3], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], radii: [4.0, 2.0, 3.0] };
        assert!((hit(&e, [0.0, 0.0, 10.0], d).unwrap().0 - 7.0).abs() < 1e-9);
        assert!(hit(&e, [3.9, 0.0, 10.0], d).is_some() && hit(&e, [0.0, 2.1, 10.0], d).is_none());
    }
}
