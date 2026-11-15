//! Prism's own procedural mannequin: smooth capsules and ellipsoids hung on the posed skeleton,
//! sized from body-part classes found by joint name (CLIP STUDIO's naming) and the figure's leg
//! length. Original geometry: nothing here comes from CLIP STUDIO's figure models.
use super::math::{V3, add, cross, len, normalize_or, scale, sub};
use super::skeleton::Posed;

/// A solid the tracer intersects.
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// Segment `a`–`b` swept by a sphere of radius `r`.
    Capsule { a: V3, b: V3, r: f64 },
    /// Centre, three orthonormal axes and the radius along each.
    Ellipsoid { c: V3, axes: [V3; 3], radii: V3 },
}

impl Shape {
    /// A sphere enclosing the solid: centre and radius.
    pub fn bounds(&self) -> (V3, f64) {
        match *self {
            Shape::Capsule { a, b, r } => (scale(add(a, b), 0.5), len(sub(b, a)) / 2.0 + r),
            Shape::Ellipsoid { c, radii, .. } => (c, radii[0].max(radii[1]).max(radii[2])),
        }
    }
}

/// Leg length (thigh + shin) of Prism's standing figure, the reference for every radius below.
const REFERENCE_LEG: f64 = 80.0;

fn class(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    let has = |s: &str| n.contains(s);
    if has("thumb") || has("index") || has("middle") || has("ring") || has("pinky") {
        "finger"
    } else if has("forearm") {
        "forearm"
    } else if has("shoulder") {
        "shoulder"
    } else if has("arm") {
        "upperarm"
    } else if has("hand") {
        "hand"
    } else if has("upleg") || has("thigh") {
        "thigh"
    } else if has("toe") {
        "toe"
    } else if has("foot") {
        "foot"
    } else if has("leg") {
        "shin"
    } else if has("head") && !has("end") {
        "head"
    } else if has("neck") {
        "neck"
    } else if has("spine2") || has("chest") {
        "chest"
    } else if has("spine") {
        "spine"
    } else if has("hips") || has("pelvis") {
        "hips"
    } else {
        "other"
    }
}

fn pos_of<'a>(posed: &'a [Posed], suffix: &str) -> Option<&'a Posed> {
    posed.iter().find(|p| p.name.to_ascii_lowercase().ends_with(suffix))
}

/// Body scale: the figure's leg length over Prism's reference figure (0.2..5).
fn body_scale(posed: &[Posed]) -> f64 {
    let leg = |side: &str| -> Option<f64> {
        let (a, b, c) = (pos_of(posed, &format!("{side}upleg_bb_"))?, pos_of(posed, &format!("{side}leg_bb_"))?, pos_of(posed, &format!("{side}foot_bb_"))?);
        Some(len(sub(b.pos, a.pos)) + len(sub(c.pos, b.pos)))
    };
    let l = leg("left").or_else(|| leg("right")).unwrap_or(REFERENCE_LEG);
    (l / REFERENCE_LEG).clamp(0.2, 5.0)
}

fn capsule(out: &mut Vec<Shape>, a: V3, b: V3, r: f64) {
    if r.is_finite() && r > 0.0 && a.iter().chain(b.iter()).all(|v| v.is_finite()) {
        out.push(Shape::Capsule { a, b, r });
    }
}

/// A tapered limb: two capsules, `r0` at `a` to `r1` at `b`.
fn limb(out: &mut Vec<Shape>, a: V3, b: V3, r0: f64, r1: f64) {
    let m = scale(add(a, b), 0.5);
    capsule(out, a, m, r0);
    capsule(out, m, b, (r0 + r1) / 2.0);
    capsule(out, scale(add(m, b), 0.5), b, r1);
}

fn ellipsoid(out: &mut Vec<Shape>, c: V3, up: V3, side: V3, radii: V3) {
    let y = normalize_or(up, [0.0, 1.0, 0.0]);
    let z = normalize_or(cross(side, y), [0.0, 0.0, 1.0]);
    let x = cross(y, z);
    if radii.iter().all(|r| r.is_finite() && *r > 0.0) && c.iter().all(|v| v.is_finite()) {
        out.push(Shape::Ellipsoid { c, axes: [x, y, z], radii });
    }
}

/// The solids of one posed figure.
pub fn build(posed: &[Posed]) -> Vec<Shape> {
    let s = body_scale(posed);
    let mut out = Vec::new();
    let side_of = |l: &str, r: &str| -> V3 {
        match (pos_of(posed, l), pos_of(posed, r)) {
            (Some(a), Some(b)) => sub(a.pos, b.pos),
            _ => [1.0, 0.0, 0.0],
        }
    };
    let hips_side = side_of("leftupleg_bb_", "rightupleg_bb_");
    // Torso direction, hips to chest: CLIP STUDIO's first spine joint sits on the hips, so the
    // hips-to-spine bone has no usable direction.
    let torso_up = match (pos_of(posed, "hips_bb_"), pos_of(posed, "spine2_bb_").or_else(|| pos_of(posed, "neck_bb_"))) {
        (Some(h), Some(c)) => sub(c.pos, h.pos),
        _ => [0.0, 1.0, 0.0],
    };
    let chest_side = side_of("leftshoulder_bb_", "rightshoulder_bb_");
    for (i, p) in posed.iter().enumerate() {
        let Some(parent) = p.parent.and_then(|q| posed.get(q)) else { continue };
        // The tree root (CLIP STUDIO's `root_group_`) only places the figure.
        if parent.parent.is_none() {
            continue;
        }
        let (a, b) = (parent.pos, p.pos);
        let l = len(sub(b, a));
        let pelvis = class(&parent.name) == "hips" && class(&p.name) == "spine";
        if !(l.is_finite() && (l > 1e-6 || pelvis)) {
            continue;
        }
        let into_finger = class(&p.name) == "finger";
        match class(&parent.name) {
            "head" => ellipsoid(&mut out, scale(add(a, b), 0.5), sub(b, a), chest_side, [l * 0.40, l * 0.55, l * 0.47]),
            "neck" => capsule(&mut out, a, b, 4.8 * s),
            "chest" if class(&p.name) == "neck" => {
                // The ribcage, centred a little below the neck.
                let c = add(a, scale(sub(b, a), 0.15));
                ellipsoid(&mut out, c, sub(b, a), chest_side, [15.5 * s, 14.0 * s, 10.5 * s]);
                capsule(&mut out, a, b, 5.5 * s);
            }
            "chest" => capsule(&mut out, a, b, 5.0 * s),
            "spine" => capsule(&mut out, a, b, 11.0 * s),
            "hips" if class(&p.name) == "spine" => {
                let up = normalize_or(torso_up, [0.0, 1.0, 0.0]);
                ellipsoid(&mut out, add(a, scale(up, -3.0 * s)), up, hips_side, [15.0 * s, 11.0 * s, 10.0 * s]);
                capsule(&mut out, a, b, 11.5 * s);
            }
            "hips" if class(&p.name) == "thigh" => capsule(&mut out, a, b, 8.0 * s),
            "hips" => {}
            "shoulder" => capsule(&mut out, a, b, 4.6 * s),
            "upperarm" => limb(&mut out, a, b, 4.8 * s, 3.8 * s),
            "forearm" => limb(&mut out, a, b, 3.8 * s, 2.7 * s),
            "hand" if into_finger => {
                // Palm to each finger base; the thumb's is slimmer.
                let r = if p.name.contains("thumb") { 1.7 } else { 2.2 };
                capsule(&mut out, a, b, r * s);
            }
            "hand" => capsule(&mut out, a, b, 2.4 * s),
            "finger" => {
                let tip = !posed.iter().any(|c| c.parent == Some(i));
                let r = if p.name.contains("thumb") { 1.0 } else { 0.85 };
                capsule(&mut out, a, b, if tip { r * 0.85 * s } else { r * s });
            }
            "thigh" => limb(&mut out, a, b, 8.2 * s, 5.6 * s),
            "shin" => limb(&mut out, a, b, 5.3 * s, 3.6 * s),
            "foot" => capsule(&mut out, a, b, 3.4 * s),
            "toe" => capsule(&mut out, a, b, 2.5 * s),
            _ => capsule(&mut out, a, b, (l * 0.1).clamp(0.4, 8.0)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figure3d::skeleton::pose;

    #[test]
    fn the_standing_figure_gets_a_body() {
        let posed = pose(&prism_doc::figure::standing_figure());
        assert!((body_scale(&posed) - 1.0).abs() < 1e-9);
        let shapes = build(&posed);
        assert!(shapes.len() > 60, "{}", shapes.len());
        let ellipsoids = shapes.iter().filter(|s| matches!(s, Shape::Ellipsoid { .. })).count();
        assert_eq!(ellipsoids, 3, "head, ribcage, pelvis");
        // Everything sits on the figure: within its 2 m bounding box.
        for sh in &shapes {
            let (c, r) = sh.bounds();
            assert!(c[1] > -10.0 && c[1] < 200.0 && r < 40.0, "{sh:?}");
        }
        assert_eq!(class("lefthandindex2_bb_"), "finger");
        assert_eq!(class("leftforearm_bb_"), "forearm");
        assert_eq!(class("leftarm_bb_"), "upperarm");
        assert_eq!(class("rightupleg_bb_"), "thigh");
        assert_eq!(class("rightleg_bb_"), "shin");
        assert_eq!(class("lefttoebase_end_bb_"), "toe");
        assert_eq!(class("head_end_bb_"), "other");
    }
}
