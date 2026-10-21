//! Stroke snapping to CLIP STUDIO special rulers, as pure geometry.
//!
//! [`candidates`] collects, for a stroke starting at a point, one line per ruler direction that
//! applies to the layer being painted (parallel rulers, and the vanishing points of perspective
//! rulers). A [`StrokeSnapper`] then follows the pointer: until it has moved the lock distance the
//! stroke stays on its start; then it locks to the candidate closest to the drag direction and
//! projects every later point onto that line, as PAINT does. Special rulers snap without a
//! distance threshold. See `docs/csp-special-rulers.md`.

use prism_doc::special_ruler::{RulerKind, RulerRange, SpecialRulers, VanishTarget, angle_dir};
use prism_doc::{Document, LayerId};
use serde::Serialize;

/// A line a stroke may follow: through `origin` along unit `dir`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SnapLine {
    pub origin: [f64; 2],
    pub dir: [f64; 2],
    /// The layer holding the ruler, the ruler's index, and the vanishing point's index.
    pub layer: LayerId,
    pub ruler: usize,
    pub point: Option<usize>,
}

/// Whether the rulers on the layer at `ruler_path` apply to the layer at `target_path`.
/// Same folder = the target shares the ruler layer's parent, or sits inside the ruler layer
/// when that is a group.
fn applies(range: RulerRange, ruler_path: &[usize], target_path: &[usize]) -> bool {
    match range {
        RulerRange::AllLayers => true,
        RulerRange::EditingLayer => ruler_path == target_path,
        RulerRange::SameFolder => {
            let parent = |p: &[usize]| p.get(..p.len().saturating_sub(1)).map(<[usize]>::to_vec);
            parent(ruler_path) == parent(target_path) || target_path.starts_with(ruler_path)
        }
    }
}

/// Every shown ruler set that applies to `target`, with its layer, bottom to top.
pub fn rulers_for(doc: &Document, target: LayerId) -> Vec<(LayerId, &SpecialRulers)> {
    let walk = doc.walk();
    let Some(target_path) = walk.iter().find(|(_, _, l)| l.id == target).map(|(p, _, _)| p.clone()) else {
        return Vec::new();
    };
    walk.iter()
        .filter_map(|(path, _, l)| {
            let r = l.special_rulers.as_ref()?;
            (r.shown && !r.is_empty() && applies(r.range, path, &target_path)).then_some((l.id, r))
        })
        .collect()
}

fn unit(v: [f64; 2]) -> Option<[f64; 2]> {
    let len = v[0].hypot(v[1]);
    (len.is_finite() && len > 1e-9).then(|| [v[0] / len, v[1] / len])
}

/// The snapping lines through `start` for a stroke on `target`.
pub fn candidates(doc: &Document, target: LayerId, start: [f64; 2]) -> Vec<SnapLine> {
    if !start.iter().all(|v| v.is_finite()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (layer, set) in rulers_for(doc, target) {
        for (ruler, r) in set.rulers.iter().enumerate().filter(|(_, r)| r.snap) {
            match &r.kind {
                RulerKind::Parallel { angle_deg, .. } => {
                    if let Some(dir) = unit(angle_dir(*angle_deg)) {
                        out.push(SnapLine { origin: start, dir, layer, ruler, point: None });
                    }
                }
                RulerKind::Perspective(p) => {
                    for (i, vp) in p.points.iter().enumerate().filter(|(_, vp)| vp.snap) {
                        let dir = match vp.target {
                            VanishTarget::Parallel { angle_deg } => unit(angle_dir(angle_deg)),
                            VanishTarget::Point(q) => unit([q[0] - start[0], q[1] - start[1]]),
                        };
                        if let Some(dir) = dir {
                            out.push(SnapLine { origin: start, dir, layer, ruler, point: Some(i) });
                        }
                    }
                }
            }
        }
    }
    out
}

/// `p` projected onto `line`.
pub fn project(line: &SnapLine, p: [f64; 2]) -> [f64; 2] {
    let t = (p[0] - line.origin[0]) * line.dir[0] + (p[1] - line.origin[1]) * line.dir[1];
    [line.origin[0] + line.dir[0] * t, line.origin[1] + line.dir[1] * t]
}

/// Snaps one stroke: hold the start until the pointer has moved `lock` pixels, then follow the
/// candidate line closest to the drag direction.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeSnapper {
    lines: Vec<SnapLine>,
    start: [f64; 2],
    lock: f64,
    locked: Option<usize>,
}

impl StrokeSnapper {
    /// `lock` is in document pixels (callers divide a screen distance by the zoom).
    pub fn new(lines: Vec<SnapLine>, start: [f64; 2], lock: f64) -> Self {
        let lock = if lock.is_finite() { lock.max(0.0) } else { 0.0 };
        Self { lines, start, lock, locked: None }
    }

    /// No ruler applies: every point passes through.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The line the stroke follows, once chosen.
    pub fn locked(&self) -> Option<&SnapLine> {
        self.locked.and_then(|i| self.lines.get(i))
    }

    /// The snapped position of pointer `p`.
    pub fn push(&mut self, p: [f64; 2]) -> [f64; 2] {
        if self.lines.is_empty() || !p.iter().all(|v| v.is_finite()) {
            return p;
        }
        if let Some(line) = self.locked() {
            return project(line, p);
        }
        let d = [p[0] - self.start[0], p[1] - self.start[1]];
        let Some(drag) = unit(d).filter(|_| d[0].hypot(d[1]) >= self.lock) else {
            return self.start;
        };
        let best = self
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| (i, (l.dir[0] * drag[0] + l.dir[1] * drag[1]).abs()))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        self.locked = best;
        self.locked().map_or(p, |line| project(line, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_doc::special_ruler::{PerspectiveRuler, SpecialRuler, VanishingPoint};
    use prism_doc::{ColorMode, Layer, SampleType, Size};

    fn iso_rulers() -> SpecialRulers {
        let point = |a: f64| VanishingPoint { target: VanishTarget::Parallel { angle_deg: a }, handle: [-8.4e6, -4.9e6], ..Default::default() };
        SpecialRulers {
            rulers: vec![SpecialRuler {
                snap: true,
                kind: RulerKind::Perspective(PerspectiveRuler { points: vec![point(30.6163181116636), point(149.39613684716832), point(270.0)], eye_level: None }),
            }],
            ..Default::default()
        }
    }

    fn doc_with(rulers: SpecialRulers) -> (Document, LayerId) {
        let mut d = Document::new("t", Size::new(100, 100), ColorMode::Rgb, SampleType::U8);
        let mut l = Layer::raster("Isometric ruler", d.pixel_format());
        l.special_rulers = Some(rulers);
        let id = l.id;
        d.layers.push(l);
        (d, id)
    }

    fn snap(doc: &Document, layer: LayerId, start: [f64; 2], pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut s = StrokeSnapper::new(candidates(doc, layer, start), start, 3.0);
        pts.iter().map(|p| s.push(*p)).collect()
    }

    fn on_line(p: [f64; 2], start: [f64; 2], angle_deg: f64) -> bool {
        let d = angle_dir(angle_deg);
        ((p[0] - start[0]) * d[1] - (p[1] - start[1]) * d[0]).abs() < 1e-9
    }

    #[test]
    fn isometric_drags_lock_to_the_nearest_of_three_directions() {
        let (doc, id) = doc_with(iso_rulers());
        let start = [50.0, 50.0];
        // Drag directions in pixel space (y down) and the ruler angle each should pick.
        for (drag_deg, want) in [(28.0, 30.6163181116636), (-150.0, 30.6163181116636), (160.0, 149.39613684716832), (85.0, 270.0), (-80.0, 270.0)] {
            let d = angle_dir(drag_deg);
            let pts: Vec<[f64; 2]> = (1..20).map(|i| [start[0] + d[0] * i as f64 * 2.0, start[1] + d[1] * i as f64 * 2.0 + if i > 4 { (i % 3) as f64 } else { 0.0 }]).collect();
            let out = snap(&doc, id, start, &pts);
            assert!(out.iter().all(|p| on_line(*p, start, want)), "drag {drag_deg}° → {want}°: {out:?}");
        }
    }

    #[test]
    fn the_stroke_holds_its_start_until_the_lock_distance() {
        let (doc, id) = doc_with(iso_rulers());
        let out = snap(&doc, id, [10.0, 10.0], &[[11.0, 10.5], [12.0, 10.0], [20.0, 16.0]]);
        assert_eq!(out[0], [10.0, 10.0]);
        assert_eq!(out[1], [10.0, 10.0]);
        assert!(on_line(out[2], [10.0, 10.0], 30.6163181116636) || on_line(out[2], [10.0, 10.0], 149.39613684716832));
    }

    #[test]
    fn point_vanishing_points_converge_and_disabled_ones_are_skipped() {
        let vp = |x: f64, snap: bool| VanishingPoint { snap, target: VanishTarget::Point([x, 0.0]), handle: [x, 0.0], guides: Vec::new() };
        let rulers = SpecialRulers {
            rulers: vec![SpecialRuler { snap: true, kind: RulerKind::Perspective(PerspectiveRuler { points: vec![vp(-1000.0, true), vp(1000.0, false)], eye_level: None }) }],
            ..Default::default()
        };
        let (doc, id) = doc_with(rulers);
        let lines = candidates(&doc, id, [0.0, 100.0]);
        assert_eq!(lines.len(), 1);
        let out = snap(&doc, id, [0.0, 100.0], &[[30.0, 120.0]]);
        // Towards (-1000, 0) through the start: the projection stays on that line.
        let d = [-1000.0, -100.0];
        assert!(((out[0][0]) * d[1] - (out[0][1] - 100.0) * d[0]).abs() < 1e-6, "{out:?}");
        // A start on the vanishing point itself has no direction.
        assert!(candidates(&doc, id, [-1000.0, 0.0]).is_empty());
    }

    #[test]
    fn hidden_or_unsnapped_rulers_and_bad_points_pass_through() {
        let mut r = iso_rulers();
        r.shown = false;
        let (doc, id) = doc_with(r);
        assert_eq!(snap(&doc, id, [0.0, 0.0], &[[40.0, 7.0]]), vec![[40.0, 7.0]]);
        let mut r = iso_rulers();
        r.rulers[0].snap = false;
        let (doc, id) = doc_with(r);
        assert!(candidates(&doc, id, [0.0, 0.0]).is_empty());
        let (doc, id) = doc_with(iso_rulers());
        let mut s = StrokeSnapper::new(candidates(&doc, id, [0.0, 0.0]), [0.0, 0.0], f64::NAN);
        let nan = s.push([f64::NAN, 1.0]);
        assert!(nan[0].is_nan());
        assert!(candidates(&doc, id, [f64::INFINITY, 0.0]).is_empty());
    }

    #[test]
    fn show_range_limits_which_layers_snap() {
        let mut d = Document::new("t", Size::new(10, 10), ColorMode::Rgb, SampleType::U8);
        let fmt = d.pixel_format();
        let sibling = Layer::raster("sibling", fmt);
        let mut ruled = Layer::raster("ruled", fmt);
        let mut r = iso_rulers();
        r.range = RulerRange::SameFolder;
        ruled.special_rulers = Some(r);
        let outside = Layer::raster("outside", fmt);
        let (sid, rid, oid) = (sibling.id, ruled.id, outside.id);
        d.layers.push(outside);
        d.layers.push(Layer::group("folder", vec![sibling, ruled]));
        assert_eq!(rulers_for(&d, sid).len(), 1);
        assert_eq!(rulers_for(&d, rid).len(), 1);
        assert!(rulers_for(&d, oid).is_empty());
        let ruled = d.layer_mut(rid).unwrap();
        ruled.special_rulers.as_mut().unwrap().range = RulerRange::EditingLayer;
        assert!(rulers_for(&d, sid).is_empty());
        assert_eq!(rulers_for(&d, rid).len(), 1);
        d.layer_mut(rid).unwrap().special_rulers.as_mut().unwrap().range = RulerRange::AllLayers;
        assert_eq!(rulers_for(&d, oid).len(), 1);
    }
}
