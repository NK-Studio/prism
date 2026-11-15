//! Hand setup: blend a hand between three shapes, CLIP STUDIO-style. The shapes sit at the corners
//! of a triangle: fingers straight and together, fingers straight and spread, and one closed hand
//! picked from four (fist, cat paw, tube grip, tight grip). Weights blend their joint angles;
//! locked fingers keep their pose.
//!
//! The shapes are Prism's own joint angles. They are applied through hinge axes found from the
//! hand's rest geometry (the joint offsets), not from any one skeleton's axis convention: a
//! finger curls about `bone × palm normal`, where the palm is the side the thumb's base sits on,
//! and spreads about the palm normal. That fits CLIP STUDIO's skeleton (fingers along local ±X,
//! curling about local −Z on both hands, as its own hand setup poses show) and Prism's standing
//! figure alike. Evidence: docs/sut-analysis.md › Pose materials › Hand setup.
use super::math::{Q, V3, axis_angle, cross, dot, len, normalize_or, qmul, qnorm, sub};
use prism_doc::figure::{Figure, Joint};

/// The digits, thumb first, as they appear in joint names (`lefthandindex1_bb_`).
pub const DIGITS: [&str; 5] = ["thumb", "index", "middle", "ring", "pinky"];

/// Which hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn prefix(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// The closed hand at the triangle's bottom corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Closed {
    /// A fist, the thumb over the middle phalanges.
    #[default]
    Fist,
    /// Fingers bent at the middle joints, knuckles only a little: a cat's paw.
    CatPaw,
    /// Fingers round a cylinder, the thumb closing the ring.
    Tube,
    /// A tight grip, every joint bent hard.
    Grip,
}

impl Closed {
    pub const ALL: [Closed; 4] = [Closed::Fist, Closed::CatPaw, Closed::Tube, Closed::Grip];

    pub fn id(self) -> &'static str {
        match self {
            Closed::Fist => "fist",
            Closed::CatPaw => "catPaw",
            Closed::Tube => "tube",
            Closed::Grip => "grip",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(id))
    }
}

/// One hand shape, in degrees. A finger (index, middle, ring, pinky) is
/// `[spread, knuckle, middle joint, end joint]`, spread positive towards the little finger. The
/// thumb is `[abduct, oppose, twist, middle joint, end joint]`: away from the index in the palm's
/// plane, towards the palm, turned about itself so it bends across the palm, then its two bends.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shape {
    fingers: [[f64; 4]; 4],
    thumb: [f64; 5],
}

const FLAT: Shape = Shape { fingers: [[-2.0, 4.0, 5.0, 3.0], [0.0, 4.0, 5.0, 3.0], [2.0, 5.0, 5.0, 3.0], [4.0, 6.0, 6.0, 4.0]], thumb: [22.0, 8.0, 25.0, 6.0, 8.0] };
const SPREAD: Shape = Shape { fingers: [[-13.0, 2.0, 4.0, 3.0], [0.0, 2.0, 4.0, 3.0], [11.0, 3.0, 4.0, 3.0], [25.0, 4.0, 4.0, 3.0]], thumb: [52.0, 4.0, 15.0, 2.0, 6.0] };
const FIST: Shape = Shape { fingers: [[3.0, 84.0, 98.0, 58.0], [0.0, 87.0, 100.0, 58.0], [-3.0, 89.0, 100.0, 56.0], [-7.0, 91.0, 96.0, 54.0]], thumb: [8.0, 48.0, 72.0, 32.0, 42.0] };
const CAT_PAW: Shape = Shape { fingers: [[1.0, 32.0, 82.0, 52.0], [0.0, 35.0, 86.0, 54.0], [-1.0, 38.0, 86.0, 54.0], [-3.0, 42.0, 84.0, 52.0]], thumb: [14.0, 34.0, 62.0, 22.0, 30.0] };
const TUBE: Shape = Shape { fingers: [[2.0, 48.0, 62.0, 44.0], [0.0, 52.0, 66.0, 44.0], [-2.0, 56.0, 66.0, 44.0], [-5.0, 60.0, 66.0, 42.0]], thumb: [28.0, 42.0, 70.0, 18.0, 24.0] };
const GRIP: Shape = Shape { fingers: [[2.0, 74.0, 102.0, 72.0], [0.0, 86.0, 106.0, 72.0], [-3.0, 92.0, 106.0, 70.0], [-7.0, 96.0, 100.0, 66.0]], thumb: [4.0, 56.0, 82.0, 30.0, 44.0] };

fn closed_shape(c: Closed) -> Shape {
    match c {
        Closed::Fist => FIST,
        Closed::CatPaw => CAT_PAW,
        Closed::Tube => TUBE,
        Closed::Grip => GRIP,
    }
}

/// A hand setup: the blend weights of the flat, spread and closed shapes (normalised; only their
/// ratio matters), which closed shape, and the fingers left alone (thumb first).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandSetup {
    pub weights: [f64; 3],
    pub closed: Closed,
    pub locked: [bool; 5],
}

impl HandSetup {
    /// The blended shape, or an error for weights that are negative, not finite or all zero.
    fn shape(&self) -> Result<Shape, String> {
        if self.weights.iter().any(|w| !w.is_finite() || *w < 0.0) {
            return Err("hand weights must be finite and not negative".into());
        }
        let total: f64 = self.weights.iter().sum();
        if total <= 1e-9 {
            return Err("hand weights must not all be zero".into());
        }
        let w = self.weights.map(|v| v / total);
        let shapes = [FLAT, SPREAD, closed_shape(self.closed)];
        let mut out = Shape { fingers: [[0.0; 4]; 4], thumb: [0.0; 5] };
        for (s, wi) in shapes.iter().zip(w) {
            for (o, f) in out.fingers.iter_mut().zip(s.fingers) {
                for (a, b) in o.iter_mut().zip(f) {
                    *a += b * wi;
                }
            }
            for (a, b) in out.thumb.iter_mut().zip(s.thumb) {
                *a += b * wi;
            }
        }
        Ok(out)
    }
}

/// Exact name of the joint called `name` (ASCII case-insensitive) in `root`.
fn joint_name(root: &Joint, name: &str) -> Option<String> {
    root.walk().into_iter().map(|(_, j)| j).find(|j| j.name.eq_ignore_ascii_case(name)).map(|j| j.name.clone())
}

fn offset_of(root: &Joint, name: &Option<String>) -> Option<V3> {
    name.as_ref().and_then(|n| root.find(n)).map(|j| j.offset)
}

/// The rest frame of one hand, in the hand joint's local coordinates: its long direction, the
/// direction across the knuckles towards the little finger, and the palm normal.
struct Palm {
    across: V3,
    normal: V3,
}

fn palm(root: &Joint, side: Side) -> Option<Palm> {
    let name = |digit: &str, k: u32| joint_name(root, &format!("{}hand{digit}{k}_bb_", side.prefix()));
    let base = |digit: &str| offset_of(root, &name(digit, 1));
    let index = base("index").or_else(|| base("middle"))?;
    let little = base("pinky").or_else(|| base("ring"))?;
    let long = base("middle").or_else(|| base("index")).or_else(|| base("ring"))?;
    let across = normalize_or(sub(little, index), [0.0, 0.0, 0.0]);
    let along = normalize_or(long, [0.0, 0.0, 0.0]);
    let n = cross(across, along);
    if len(across) < 0.5 || len(along) < 0.5 || len(n) < 1e-6 {
        return None;
    }
    let mut normal = normalize_or(n, [0.0, 0.0, 1.0]);
    // The palm is the side the thumb's base sits on.
    if let Some(t) = base("thumb")
        && dot(t, normal) < 0.0
    {
        normal = [-normal[0], -normal[1], -normal[2]];
    }
    Some(Palm { across, normal })
}

/// The rest direction of the bone that starts at joint `name` (its first child's offset).
fn bone(root: &Joint, name: &str) -> Option<V3> {
    let j = root.find(name)?;
    let d = j.children.first()?.offset;
    (len(d) > 1e-9).then(|| normalize_or(d, [1.0, 0.0, 0.0]))
}

/// The hinge a bone along `d` curls about, towards the palm.
fn hinge(d: V3, palm: &Palm) -> Option<V3> {
    let h = cross(d, palm.normal);
    (len(h) > 1e-6).then(|| normalize_or(h, [0.0, 0.0, 1.0]))
}

fn deg(axis: V3, degrees: f64) -> Q {
    axis_angle(axis, degrees.to_radians())
}

fn set(root: &mut Joint, name: &str, q: Q) {
    if let Some(j) = root.find_mut(name) {
        j.rotation = qnorm(q);
    }
}

/// Pose one hand of `figure`. Joints the hand lacks are skipped; a figure without that hand's
/// fingers is an error.
pub fn set_hand(figure: &mut Figure, side: Side, setup: &HandSetup) -> Result<(), String> {
    let shape = setup.shape()?;
    let root = &figure.root;
    let palm = palm(root, side).ok_or_else(|| format!("the figure has no {} hand fingers", side.prefix()))?;
    let mut edits: Vec<(String, Q)> = Vec::new();
    for (di, digit) in DIGITS.iter().enumerate() {
        if setup.locked.get(di).copied().unwrap_or(false) {
            continue;
        }
        let names: Vec<Option<String>> = (1..=3).map(|k| joint_name(root, &format!("{}hand{digit}{k}_bb_", side.prefix()))).collect();
        let Some(Some(first)) = names.first() else { continue };
        let Some(d1) = bone(root, first) else { continue };
        let Some(h1) = hinge(d1, &palm) else { continue };
        if di == 0 {
            let Some(t) = shape.thumb.get(0..5) else { continue };
            // Abduct: about the palm normal, away from the little finger.
            let abduct_axis = if dot(cross(palm.normal, d1), palm.across) < 0.0 { palm.normal } else { [-palm.normal[0], -palm.normal[1], -palm.normal[2]] };
            // Twist: about the thumb, turning its bend towards the little finger.
            let twist_axis = if dot(cross(d1, palm.normal), palm.across) > 0.0 { d1 } else { [-d1[0], -d1[1], -d1[2]] };
            let q = qmul(qmul(deg(abduct_axis, t[0]), deg(h1, t[1])), deg(twist_axis, t[2]));
            edits.push((first.clone(), q));
            for (k, angle) in [(1usize, t[3]), (2, t[4])] {
                if let Some(Some(n)) = names.get(k)
                    && let Some(h) = bone(root, n).and_then(|d| hinge(d, &palm))
                {
                    edits.push((n.clone(), deg(h, angle)));
                }
            }
            continue;
        }
        let Some(f) = shape.fingers.get(di - 1) else { continue };
        // Spread about the palm normal, positive towards the little finger.
        let spread_axis = if dot(cross(palm.normal, d1), palm.across) > 0.0 { palm.normal } else { [-palm.normal[0], -palm.normal[1], -palm.normal[2]] };
        edits.push((first.clone(), qmul(deg(spread_axis, f[0]), deg(h1, f[1]))));
        for (k, angle) in [(1usize, f[2]), (2, f[3])] {
            if let Some(Some(n)) = names.get(k)
                && let Some(h) = bone(root, n).and_then(|d| hinge(d, &palm))
            {
                edits.push((n.clone(), deg(h, angle)));
            }
        }
    }
    for (name, q) in edits {
        set(&mut figure.root, &name, q);
    }
    Ok(())
}

/// Where the fingertips of `side` point, for tests: the unit direction of each digit's last
/// segment in the hand's rest frame after posing.
#[cfg(test)]
pub(crate) fn tip_direction(figure: &Figure, side: Side, digit: &str) -> Option<V3> {
    let posed = super::skeleton::pose(figure);
    let hand = posed.iter().find(|p| p.name.eq_ignore_ascii_case(&format!("{}hand_bb_", side.prefix())))?;
    let a = posed.iter().find(|p| p.name.eq_ignore_ascii_case(&format!("{}hand{digit}3_bb_", side.prefix())))?;
    let b = posed.iter().find(|p| p.name.eq_ignore_ascii_case(&format!("{}hand{digit}4_bb_", side.prefix())))?;
    let inv = super::math::qconj(hand.rot);
    Some(normalize_or(super::math::rotate(inv, sub(b.pos, a.pos)), [0.0, 0.0, 0.0]))
}

#[cfg(test)]
#[path = "hand_tests.rs"]
mod tests;
