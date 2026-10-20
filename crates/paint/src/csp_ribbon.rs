//! Independently implemented CSP Ribbon (`BrushRibbon`): the tip image is laid continuously
//! along the stroke instead of being stamped as spaced dabs. Source evidence and limitations are
//! recorded in docs/sut-analysis.md › Ribbon.
//!
//! The stroke is walked in steps of [`RIBBON_STEP`] pixels. Each step contributes one slice of
//! the tip: the quadrilateral between the previous step's cross-section and this one, mapped to
//! the next rows of the tip along its ribbon axis. The tip's cross axis spans the ribbon's width
//! (the brush diameter) and keeps the tip's aspect along the stroke, so a tip of `len` texels
//! covers `len · diameter / cross` pixels of path before the next tip of a sequence starts.
use std::sync::Arc;

use prism_geom::{Point, Rect};
use serde::{Deserialize, Serialize};

use crate::brush::TipShape;
use crate::tile::{GrayTile, Mips};

/// Slice corners are clamped to ±this many pixels (documents are far smaller).
const FAR: f64 = 1e9;

/// Path distance between ribbon steps, in document pixels (a constant in CSP's ribbon step).
pub const RIBBON_STEP: f64 = 4.0;

/// The ribbon option of an imported CSP brush.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CspRibbon {
    /// Clockwise quarter turns of the tip in the ribbon (0..=3; larger values wrap). CSP snaps
    /// the tip rotation to the nearest quarter turn: at 0 the tip's rows run along the stroke, at
    /// 1 its columns do (the tip appears upright on a stroke drawn to the right).
    pub quarter_turns: u8,
}

impl CspRibbon {
    /// The quarter turns for a CSP tip rotation in clockwise degrees (`BrushRotation`):
    /// [-45°, 45°) → 0, [45°, 135°) → 1, [135°, 225°) → 2, [225°, 315°) → 3.
    pub fn from_rotation(clockwise_degrees: f64) -> Self {
        let a = if clockwise_degrees.is_finite() { clockwise_degrees.rem_euclid(360.0) } else { 0.0 };
        let quarter_turns = match a {
            a if a < 45.0 => 0,
            a if a < 135.0 => 1,
            a if a < 225.0 => 2,
            a if a < 315.0 => 3,
            _ => 0,
        };
        Self { quarter_turns }
    }

    fn turns(self) -> u8 {
        self.quarter_turns % 4
    }
}

/// The tips a ribbon samples, as [`BrushContext`](crate::BrushContext) keeps them: each without a
/// fully transparent last line along the ribbon (the column or row CSP materials leave uncovered
/// when their tiles stop one texel short of the stored size, see docs/sut-analysis.md), which
/// would otherwise show as a seam wherever one copy of the tip meets the next. With the factors
/// that turn the full tip's normalised coordinates into the trimmed tip's.
pub(crate) fn prepare_tips(tip: &TipShape, ribbon: CspRibbon) -> Vec<(Arc<Mips>, [f32; 2])> {
    let along_x = ribbon.turns() % 2 == 1;
    let prepare = |g: &GrayTile| {
        let (w, h) = (g.width as usize, g.height as usize);
        let empty = |i: usize| g.data.get(i).is_none_or(|v| *v == 0);
        let trimmed = if along_x && w > 1 && (0..h).all(|y| empty(y * w + w - 1)) {
            Some(GrayTile::from_fn(g.width - 1, g.height, |x, y| g.get(x, y)))
        } else if !along_x && h > 1 && (0..w).all(|x| empty((h - 1) * w + x)) {
            Some(GrayTile::from_fn(g.width, g.height - 1, |x, y| g.get(x, y)))
        } else {
            None
        };
        match trimmed {
            Some(t) => (Arc::new(Mips::new(&t)), [g.width as f32 / t.width as f32, g.height as f32 / t.height as f32]),
            None => (Arc::new(Mips::new(g)), [1.0, 1.0]),
        }
    };
    match tip {
        TipShape::Sampled(g) if g.is_valid() => vec![prepare(g)],
        TipShape::Sequence(v) => v.iter().take(64).filter(|g| g.is_valid()).map(prepare).collect(),
        _ => Vec::new(),
    }
}

/// A ribbon cross-section at a step: its centre, half-width and unit cross direction (image
/// space, y down; it points to the right of the direction of travel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RibbonEdge {
    pub center: Point,
    pub half: f32,
    pub cross: [f32; 2],
}

/// One ribbon slice: the quadrilateral between two cross-sections and the tip rows it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RibbonSlice {
    pub start: RibbonEdge,
    pub end: RibbonEdge,
    /// Tip texels along the ribbon axis, from `start` to `end`.
    pub u0: f32,
    pub u1: f32,
    /// Which tip of a sequence (index into it, wrapping).
    pub tip: u32,
    pub quarter_turns: u8,
}

/// Ribbon progress through a stroke (owned by the dab builder).
#[derive(Clone, Debug)]
pub(crate) struct RibbonState {
    ribbon: CspRibbon,
    /// (width, height) of each tip, in texels.
    tips: Vec<(u32, u32)>,
    prev: Option<RibbonEdge>,
    /// The slice prepared at `prev`, drawn at the next step.
    next: (f32, f32, u32),
    tip: u32,
    tip_done: bool,
}

impl RibbonState {
    /// `None` without a usable tip (CSP draws ribbons only with tip images).
    pub(crate) fn new(ribbon: CspRibbon, tips: Vec<(u32, u32)>) -> Option<Self> {
        let tips: Vec<_> = tips.into_iter().filter(|&(w, h)| w > 0 && h > 0).collect();
        (!tips.is_empty()).then_some(Self { ribbon, tips, prev: None, next: (0.0, 0.0, 0), tip: 0, tip_done: false })
    }

    /// (along, cross) length of tip `i` in texels.
    fn axes(&self, i: u32) -> (f32, f32) {
        let (w, h) = self.tips.get(i as usize % self.tips.len().max(1)).copied().unwrap_or((1, 1));
        if self.ribbon.turns().is_multiple_of(2) { (h as f32, w as f32) } else { (w as f32, h as f32) }
    }

    /// One step at `center`: the slice from the previous step (none at the first), and the path
    /// distance to the next step. `radius` is the brush radius there, `direction` the visual
    /// (counter-clockwise, y-up) direction of travel in radians, `scale` the CSP thickness as
    /// (cross, along).
    pub(crate) fn step(&mut self, center: Point, radius: f32, direction: f32, scale: [f32; 2]) -> (Option<RibbonSlice>, f64) {
        let radius = if radius.is_finite() { radius.max(0.1) } else { 0.1 };
        let [cross_scale, along_scale] = scale.map(|s| if s.is_finite() && s > 0.0 { s } else { 1.0 });
        let direction = if direction.is_finite() { direction } else { 0.0 };
        let (sn, cs) = direction.sin_cos();
        let edge = RibbonEdge { center, half: radius * cross_scale, cross: [sn, cs] };
        let slice = self.prev.map(|start| {
            let (u0, u1, tip) = self.next;
            RibbonSlice { start, end: edge, u0, u1, tip, quarter_turns: self.ribbon.turns() }
        });
        // The next slice continues the current tip, or starts the next one at its first row.
        let mut u0 = self.next.1;
        if self.tip_done || self.prev.is_none() {
            if self.prev.is_some() {
                self.tip = self.tip.wrapping_add(1) % self.tips.len().max(1) as u32;
            }
            u0 = 0.0;
        }
        let (len, cross) = self.axes(self.tip);
        // Tip texels per path pixel: the cross axis spans the diameter (before the cross
        // thickness), the along axis keeps that scale times the along thickness.
        let per_px = f64::from(cross) / (2.0 * f64::from(radius) * f64::from(along_scale));
        let mut distance = RIBBON_STEP;
        let mut u1 = f64::from(u0) + RIBBON_STEP * per_px;
        self.tip_done = false;
        if !u1.is_finite() || u1 >= f64::from(len) {
            distance = if per_px.is_finite() && per_px > 0.0 { (f64::from(len - u0) / per_px).clamp(0.0, RIBBON_STEP) } else { RIBBON_STEP };
            u1 = f64::from(len);
            self.tip_done = true;
        }
        self.next = (u0, u1 as f32, self.tip);
        self.prev = Some(edge);
        (slice, distance)
    }
}

impl RibbonSlice {
    /// Corners: start left, start right, end right, end left (left = against `cross`).
    pub fn corners(&self) -> [(f64, f64); 4] {
        let at = |e: &RibbonEdge, side: f64| {
            let k = f64::from(e.half) * side;
            (e.center.x + f64::from(e.cross[0]) * k, e.center.y + f64::from(e.cross[1]) * k)
        };
        [at(&self.start, -1.0), at(&self.start, 1.0), at(&self.end, 1.0), at(&self.end, -1.0)]
    }

    /// Pixels the slice can touch (empty for non-finite geometry).
    pub fn rect(&self) -> Rect {
        let c = self.corners();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (x, y) in c {
            if !x.is_finite() || !y.is_finite() {
                return Rect::EMPTY;
            }
            // Far beyond any document; keeps tile arithmetic on the rectangle within `i32`.
            let (x, y) = (x.clamp(-FAR, FAR), y.clamp(-FAR, FAR));
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Rect::new((x0.floor() as i32).saturating_sub(1), (y0.floor() as i32).saturating_sub(1), (x1.ceil() as i32).saturating_add(1), (y1.ceil() as i32).saturating_add(1))
    }

    /// The x range of the slice's outline on the horizontal line `y` (`None` where it misses).
    /// Rasterising only that span keeps a wide slice drawn diagonally from walking its whole
    /// bounding box.
    pub fn row_span(&self, y: f64) -> Option<(f64, f64)> {
        let c = self.corners();
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..4 {
            let (p, q) = (c[i], c[(i + 1) % 4]);
            let (y0, y1) = (p.1.min(q.1), p.1.max(q.1));
            if !(y0..=y1).contains(&y) {
                continue;
            }
            let x = if (q.1 - p.1).abs() < 1e-12 { p.0.min(q.0) } else { p.0 + (q.0 - p.0) * (y - p.1) / (q.1 - p.1) };
            let x_hi = if (q.1 - p.1).abs() < 1e-12 { p.0.max(q.0) } else { x };
            lo = lo.min(x);
            hi = hi.max(x_hi);
        }
        (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
    }

    /// Where a point falls in the slice: `(s, t)` with `s` across (0 left .. 1 right) and `t`
    /// along (0 start .. 1 end), by inverting the bilinear map of the quadrilateral. `None`
    /// outside; `t` is half-open so neighbouring slices share their edge without overlapping.
    pub fn locate(&self, px: f64, py: f64) -> Option<(f64, f64)> {
        let [a, b, c, d] = self.corners();
        let sub = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0, p.1 - q.1);
        let cross = |p: (f64, f64), q: (f64, f64)| p.0 * q.1 - p.1 * q.0;
        let e = sub(b, a);
        let f = sub(d, a);
        let g = (a.0 - b.0 + c.0 - d.0, a.1 - b.1 + c.1 - d.1);
        let h = (px - a.0, py - a.1);
        let k2 = cross(g, f);
        let k1 = cross(e, f) + cross(h, g);
        let k0 = cross(h, e);
        // s for a given t: h = s·(e + g·t) + f·t, solved on the better-conditioned axis.
        let s_for = |t: f64| {
            let (dx, dy) = (e.0 + g.0 * t, e.1 + g.1 * t);
            if dx.abs() >= dy.abs() {
                (dx.abs() > 1e-12).then(|| (h.0 - f.0 * t) / dx)
            } else {
                Some((h.1 - f.1 * t) / dy)
            }
        };
        let inside = |s: f64, t: f64| (-1e-9..=1.0 + 1e-9).contains(&s) && (0.0..1.0).contains(&t);
        let candidates: [Option<f64>; 2] = if (k2 * k0).abs() <= 1e-10 * k1 * k1 {
            // Parallel start and end edges (or nearly): the equation is linear in t.
            [(k1.abs() > 1e-12).then(|| -k0 / k1), None]
        } else {
            let disc = k1 * k1 - 4.0 * k0 * k2;
            if disc < 0.0 {
                return None;
            }
            let w = disc.sqrt();
            [Some((-k1 - w) / (2.0 * k2)), Some((-k1 + w) / (2.0 * k2))]
        };
        candidates.into_iter().flatten().filter(|t| t.is_finite()).find_map(|t| s_for(t).filter(|s| s.is_finite() && inside(*s, t)).map(|s| (s, t)))
    }

    /// The tip position, normalised to 0..1 per image axis (before flips), for slice
    /// coordinates `(s, t)` on a `w × h` tip: rotated by the quarter turns, so a turn never
    /// mirrors the tip.
    pub fn tip_uv(&self, s: f64, t: f64, w: f32, h: f32) -> (f32, f32) {
        let a = f64::from(self.u0) + t * f64::from(self.u1 - self.u0);
        let (w, h) = (f64::from(w.max(1.0)), f64::from(h.max(1.0)));
        let (x, y) = match self.quarter_turns % 4 {
            0 => ((1.0 - s) * w, a),
            1 => (a, s * h),
            2 => (s * w, h - a),
            _ => (w - a, (1.0 - s) * h),
        };
        ((x / w) as f32, (y / h) as f32)
    }

    /// Tip texels per pixel, the smaller of the two axes (for choosing a mip level).
    pub fn texels_per_px(&self, w: f32, h: f32) -> f32 {
        let cross = if self.quarter_turns.is_multiple_of(2) { w } else { h };
        let across = cross / (2.0 * self.start.half.max(self.end.half)).max(1e-3);
        let length = (self.end.center.x - self.start.center.x).hypot(self.end.center.y - self.start.center.y) as f32;
        let along = if length > 1e-3 { (self.u1 - self.u0).abs() / length } else { across };
        let k = across.min(along);
        if k.is_finite() && k > 0.0 { k } else { 1.0 }
    }
}
