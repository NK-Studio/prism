//! CLIP STUDIO PAINT special rulers (특수 자) attached to a layer: parallel line rulers and
//! perspective rulers whose vanishing points are points or parallel directions. Pure data; the
//! stroke snapping lives in the engine (`special_ruler_snap`). See `docs/csp-special-rulers.md`.
//!
//! Coordinates are document pixels. Angles are degrees counter-clockwise with y up (PAINT's
//! `ParallelAngle`), so a direction in pixel space (y down) is `(cos a, -sin a)`.

use serde::{Deserialize, Serialize};

/// Rulers one layer may hold.
pub const MAX_RULERS: usize = 64;
/// Vanishing points one perspective ruler may hold.
pub const MAX_VANISHING_POINTS: usize = 16;
/// Guide handles one vanishing point may hold.
pub const MAX_GUIDES: usize = 16;
/// Largest stored coordinate magnitude. PAINT keeps parallel vanishing-point handles millions of
/// pixels away, so this is far beyond any canvas.
pub const MAX_COORD: f64 = 1e9;

/// The special rulers of one layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpecialRulers {
    pub rulers: Vec<SpecialRuler>,
    /// The rulers are shown (and snap). PAINT's layer ruler visibility.
    pub shown: bool,
    pub range: RulerRange,
}

impl Default for SpecialRulers {
    fn default() -> Self {
        Self { rulers: Vec::new(), shown: true, range: RulerRange::AllLayers }
    }
}

/// Which layers a ruler applies to (PAINT: Show in all layers / same folder / editing target).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RulerRange {
    #[default]
    AllLayers,
    SameFolder,
    EditingLayer,
}

impl RulerRange {
    pub fn name(self) -> &'static str {
        match self {
            Self::AllLayers => "all",
            Self::SameFolder => "folder",
            Self::EditingLayer => "editing",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "all" => Some(Self::AllLayers),
            "folder" => Some(Self::SameFolder),
            "editing" => Some(Self::EditingLayer),
            _ => None,
        }
    }
}

/// One ruler. `snap` = strokes follow it (PAINT's per-ruler Snap).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecialRuler {
    pub snap: bool,
    #[serde(flatten)]
    pub kind: RulerKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RulerKind {
    /// Parallel line ruler: every stroke runs at `angle_deg`. `center` is its handle.
    Parallel { angle_deg: f64, center: [f64; 2] },
    Perspective(PerspectiveRuler),
}

impl RulerKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Parallel { .. } => "parallel",
            Self::Perspective(_) => "perspective",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PerspectiveRuler {
    pub points: Vec<VanishingPoint>,
    /// Eye level handle; the eye level is drawn only while a point is not parallel.
    pub eye_level: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VanishingPoint {
    /// Strokes may snap towards this point.
    pub snap: bool,
    pub target: VanishTarget,
    /// Where the point or its handle is drawn.
    pub handle: [f64; 2],
    /// Guide handle points (each guide runs through one towards the target).
    pub guides: Vec<[f64; 2]>,
}

impl Default for VanishingPoint {
    fn default() -> Self {
        Self { snap: true, target: VanishTarget::Point([0.0, 0.0]), handle: [0.0, 0.0], guides: Vec::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum VanishTarget {
    /// Lines converge on this point.
    Point([f64; 2]),
    /// The point was made parallel: lines run at this angle.
    Parallel { angle_deg: f64 },
}

/// `a` in [0, 360).
pub fn normalize_angle(a: f64) -> f64 {
    let r = a.rem_euclid(360.0);
    if r >= 360.0 { 0.0 } else { r }
}

/// The unit pixel-space direction (y down) of `angle_deg` (counter-clockwise, y up).
pub fn angle_dir(angle_deg: f64) -> [f64; 2] {
    let a = angle_deg.to_radians();
    [a.cos(), -a.sin()]
}

fn coord(what: &str, p: [f64; 2]) -> Result<(), String> {
    if p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) { Ok(()) } else { Err(format!("special ruler {what} must be finite and within ±{MAX_COORD}")) }
}

fn angle(a: f64) -> Result<(), String> {
    if a.is_finite() { Ok(()) } else { Err("special ruler angle must be finite".into()) }
}

impl VanishingPoint {
    pub fn validate(&self) -> Result<(), String> {
        match self.target {
            VanishTarget::Point(p) => coord("vanishing point", p)?,
            VanishTarget::Parallel { angle_deg } => angle(angle_deg)?,
        }
        coord("handle", self.handle)?;
        if self.guides.len() > MAX_GUIDES {
            return Err(format!("a vanishing point holds at most {MAX_GUIDES} guides"));
        }
        self.guides.iter().try_for_each(|g| coord("guide", *g))
    }
}

impl SpecialRuler {
    pub fn validate(&self) -> Result<(), String> {
        match &self.kind {
            RulerKind::Parallel { angle_deg, center } => {
                angle(*angle_deg)?;
                coord("centre", *center)
            }
            RulerKind::Perspective(p) => {
                if p.points.is_empty() || p.points.len() > MAX_VANISHING_POINTS {
                    return Err(format!("a perspective ruler needs 1 to {MAX_VANISHING_POINTS} vanishing points"));
                }
                if let Some(e) = p.eye_level {
                    coord("eye level", e)?;
                }
                p.points.iter().try_for_each(VanishingPoint::validate)
            }
        }
    }
}

impl SpecialRulers {
    /// Check limits and coordinates (for anything read from a file or a command).
    pub fn validate(&self) -> Result<(), String> {
        if self.rulers.len() > MAX_RULERS {
            return Err(format!("a layer holds at most {MAX_RULERS} special rulers"));
        }
        self.rulers.iter().try_for_each(SpecialRuler::validate)
    }
    pub fn is_empty(&self) -> bool {
        self.rulers.is_empty()
    }
    /// Move every handle and point by `d` (parallel directions do not change).
    pub fn translate(&mut self, d: [f64; 2]) {
        let mv = |p: &mut [f64; 2]| {
            p[0] += d[0];
            p[1] += d[1];
        };
        for r in &mut self.rulers {
            match &mut r.kind {
                RulerKind::Parallel { center, .. } => mv(center),
                RulerKind::Perspective(p) => {
                    if let Some(e) = &mut p.eye_level {
                        mv(e);
                    }
                    for vp in &mut p.points {
                        if let VanishTarget::Point(q) = &mut vp.target {
                            mv(q);
                        }
                        mv(&mut vp.handle);
                        vp.guides.iter_mut().for_each(mv);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso() -> SpecialRulers {
        let point = |a: f64| VanishingPoint { target: VanishTarget::Parallel { angle_deg: a }, handle: [-8.4e6, -4.9e6], ..Default::default() };
        SpecialRulers {
            rulers: vec![SpecialRuler { snap: true, kind: RulerKind::Perspective(PerspectiveRuler { points: vec![point(30.6), point(149.4), point(270.0)], eye_level: None }) }],
            ..Default::default()
        }
    }

    #[test]
    fn angles_are_counter_clockwise_with_y_up() {
        let d = angle_dir(90.0);
        assert!(d[0].abs() < 1e-12 && (d[1] + 1.0).abs() < 1e-12, "90° points up the screen");
        assert_eq!(normalize_angle(-30.0), 330.0);
        assert_eq!(normalize_angle(720.0), 0.0);
    }

    #[test]
    fn validate_accepts_far_parallel_handles_and_rejects_bad_input() {
        assert!(iso().validate().is_ok());
        let mut r = iso();
        let RulerKind::Perspective(p) = &mut r.rulers[0].kind else { panic!("perspective") };
        p.points[0].handle = [f64::NAN, 0.0];
        assert!(r.validate().is_err());
        let mut r = iso();
        let RulerKind::Perspective(p) = &mut r.rulers[0].kind else { panic!("perspective") };
        p.points.clear();
        assert!(r.validate().is_err());
        let mut r = iso();
        r.rulers = vec![r.rulers[0].clone(); MAX_RULERS + 1];
        assert!(r.validate().is_err());
        let r = SpecialRulers { rulers: vec![SpecialRuler { snap: true, kind: RulerKind::Parallel { angle_deg: f64::INFINITY, center: [0.0, 0.0] } }], ..Default::default() };
        assert!(r.validate().is_err());
    }

    #[test]
    fn translate_moves_points_and_keeps_directions() {
        let mut r = iso();
        r.rulers.push(SpecialRuler { snap: true, kind: RulerKind::Parallel { angle_deg: 10.0, center: [1.0, 2.0] } });
        let RulerKind::Perspective(p) = &mut r.rulers[0].kind else { panic!("perspective") };
        p.points[0].guides = vec![[5.0, 5.0]];
        p.points.push(VanishingPoint { target: VanishTarget::Point([3.0, 4.0]), handle: [3.0, 4.0], ..Default::default() });
        r.translate([10.0, -1.0]);
        let RulerKind::Perspective(p) = &r.rulers[0].kind else { panic!("perspective") };
        assert_eq!(p.points[0].target, VanishTarget::Parallel { angle_deg: 30.6 });
        assert_eq!((p.points[0].guides[0], p.points[3].target), ([15.0, 4.0], VanishTarget::Point([13.0, 3.0])));
        assert_eq!(r.rulers[1].kind, RulerKind::Parallel { angle_deg: 10.0, center: [11.0, 1.0] });
    }

    #[test]
    fn serde_round_trips() {
        let r = iso();
        let back: SpecialRulers = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
        let old: SpecialRulers = serde_json::from_str("{}").unwrap();
        assert_eq!(old, SpecialRulers::default());
    }
}
