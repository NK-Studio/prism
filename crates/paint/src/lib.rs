//! Brush engine: Photoshop-grade brushes (computed and sampled tips, Shape Dynamics, Scattering,
//! Texture, Dual Brush, Color Dynamics, Transfer, Brush Pose, Noise, Wet Edges, Build-up,
//! Smoothing) plus the dab machinery shared by the retouching tools.
//!
//! Strokes are data ([`Stroke`]) so they can be recorded, replayed by automation, and tested
//! deterministically: every jitter is a hash of the brush seed and the dab index ([`rng`]).
//! Dabs composite with "build-up up to stroke opacity" semantics: within one stroke, coverage
//! accumulates with flow towards each dab's opacity ceiling, and the stroke composites once at the
//! stroke opacity (Photoshop's Opacity versus Flow).
//!
//! Module map: [`brush`] (the serde model), [`dynamics`] (points → dabs), [`render`] (dabs →
//! stroke buffer → pixels), [`presets`] (built-ins), [`mixer`] (Mixer Brush), [`replace`] (Color
//! Replacement), [`retouch`] (sequential/accumulating helpers for the retouch tools).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use prism_geom::{Point, Rect};
use prism_raster::Surface;
use serde::{Deserialize, Serialize};

pub mod bg_erase;
pub mod brush;
pub mod dynamics;
pub mod mixer;
pub mod presets;
pub mod procedural;
pub mod render;
pub mod replace;
pub mod retouch;
pub mod rng;
pub mod tile;

pub use brush::{
    BrushPreset, BrushSettings, ColorDynamics, Control, DualBrush, Dynamic, MAX_BRUSH_SIZE, MAX_SCATTER, MaskMode, Pattern, PatternStyle, Pose, Scattering,
    SectionLocks, ShapeDynamics, Smoothing, Texture, TipShape, Transfer,
};
pub use mixer::MixerSettings;
pub use render::{BrushContext, StrokeRenderer, grid_center, grid_square, render_stroke};
pub use tile::GrayTile;

/// One input sample. Missing stylus data defaults to "mouse": full pressure, no tilt/rotation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    /// 0..1, 1 for mouse.
    pub pressure: f32,
    /// Degrees -90..90 (W3C Pointer Events `tiltX`/`tiltY`).
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Barrel rotation in degrees 0..360.
    pub rotation: f32,
    /// Airbrush stylus wheel 0..1.
    pub wheel: f32,
    /// Timestamp in milliseconds (airbrush build-up and smoothing catch-up).
    pub time: f64,
}

impl Default for StrokePoint {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, wheel: 1.0, time: 0.0 }
    }
}

impl StrokePoint {
    pub fn new(x: f64, y: f64, pressure: f32) -> Self {
        Self { x, y, pressure, ..Default::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub brush: BrushSettings,
    pub points: Vec<StrokePoint>,
}

/// A dab placed along the stroke (after dynamics).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub center: Point,
    pub radius: f32,
    /// Flow: per-dab build-up amount.
    pub alpha: f32,
    /// Radians, counter-clockwise.
    pub angle: f32,
    pub roundness: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Opacity ceiling this dab builds up to (Transfer › Opacity).
    pub opacity: f32,
    /// Straight RGBA paint colour (Color Dynamics).
    pub color: [f32; 4],
    /// Texture depth for per-tip texturing.
    pub depth: f32,
    /// Position in the stroke (0 = first).
    pub index: u64,
    /// Brush Projection: direction (radians, counter-clockwise, y up) along which the tip is
    /// foreshortened, and the factor (1 = no projection).
    pub proj_angle: f32,
    pub proj_scale: f32,
    /// Mixer Brush multipliers of Wet and Mix (Transfer › Wetness/Mix Jitter), 1 = as set.
    pub wet: f32,
    pub mix: f32,
}

impl Dab {
    /// A plain round dab.
    pub fn round(center: Point, radius: f32, alpha: f32) -> Self {
        Self {
            center,
            radius,
            alpha,
            angle: 0.0,
            roundness: 1.0,
            flip_x: false,
            flip_y: false,
            opacity: 1.0,
            color: [0.0, 0.0, 0.0, 1.0],
            depth: 1.0,
            index: 0,
            proj_angle: 0.0,
            proj_scale: 1.0,
            wet: 1.0,
            mix: 1.0,
        }
    }
}

/// All primary dabs of a stroke (smoothing, pose, spacing, build-up and dynamics applied).
pub fn dabs(stroke: &Stroke) -> Vec<Dab> {
    let mut g = dynamics::DabGenerator::new(&stroke.brush, 1.0);
    let (mut out, mut dual) = (Vec::new(), Vec::new());
    g.push(&stroke.points, &mut out, &mut dual);
    g.finish(&mut out, &mut dual);
    out
}

/// Span of the computed tip's anti-aliased edge, in pixels: coverage falls from 1 to 0 along a
/// smoothstep this wide. Its 90 % → 10 % width (≈ 0.61 × span) is the ~1.55 px Photoshop's hard
/// round tips show at every diameter from 10 to 500 px (measured with `cargo xtask brush-compare`,
/// docs/brush-compare.md).
pub const EDGE_SPAN: f32 = 2.55;
/// How far inside the nominal radius the edge reaches 50 % coverage (Photoshop: 0.14–0.18 px for
/// 10–50 px dabs and 40 px lines, 0.30–0.37 px for 100–200 px dabs).
pub const EDGE_INSET: f32 = 0.2;
/// How far beyond the nominal radius a computed tip still paints.
pub const EDGE_REACH: f32 = EDGE_SPAN / 2.0 - EDGE_INSET;
/// How far inside the nominal radius the anti-aliased edge starts (closer in, it is fully on).
pub const EDGE_INNER: f32 = EDGE_SPAN / 2.0 + EDGE_INSET;

/// Anti-aliased edge factor at signed distance `sd` (pixels, positive outside) from a computed
/// tip's nominal boundary.
#[inline]
pub fn edge_coverage(sd: f32) -> f32 {
    let x = ((sd + EDGE_INSET) / EDGE_SPAN + 0.5).clamp(0.0, 1.0);
    1.0 - x * x * (3.0 - 2.0 * x)
}

/// Photoshop's hardness falloff for computed tips, as `(hardness, a, s)`: coverage is full out to
/// relative radius `a` (1 = nominal radius), then falls as the Gaussian tail exp(−((q − a) / s)²),
/// which paints past the nominal radius (hardness 0 still lays 10 % there and stops near 1.6
/// radii). Measured with `cargo xtask brush-compare` (`soft-*` and `dab-h*` cases): the shape
/// scales with the tip from 20 to 200 px, and the tail fits every measured hardness to ~0.01.
/// Hardnesses in between interpolate linearly; 1 is the hard tip ([`edge_coverage`]).
const FALLOFF: [(f32, f32, f32); 9] = [
    (0.0, 0.0, 0.654),
    (0.1, 0.152, 0.570),
    (0.25, 0.361, 0.452),
    (0.4, 0.537, 0.343),
    (0.5, 0.651, 0.276),
    (0.6, 0.738, 0.214),
    (0.75, 0.847, 0.136),
    (0.9, 0.942, 0.060),
    (1.0, 1.0, 0.0),
];
/// Narrowest falloff tail, in pixels: a sharper one would alias. Its 90 % → 10 % width (≈ 1.19 ×)
/// matches the hard edge's ~1.55 px.
pub const MIN_TAIL_PX: f32 = 1.3;
/// Where the tail is cut off, in tail widths: exp(−2.5²) ≈ 0.0019, half an 8-bit level. Hardness 0
/// then stops at 1.64 radii, where Photoshop's does (it paints 0.001 at 1.6 and nothing at 1.7).
const TAIL_CUTOFF: f32 = 2.5;
/// √ln 2: the tail reaches 50 % this many tail widths beyond `a`.
const TAIL_HALF: f32 = 0.832_555;

/// A computed tip's radial profile for one dab size: hard (the anti-aliased edge at the nominal
/// radius) or soft (full out to `a`, then a Gaussian tail of width `s`, both relative to the radius).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Falloff {
    hard: bool,
    a: f32,
    s: f32,
}

impl Falloff {
    /// The profile of a tip with this hardness whose radius is `radius` pixels (for an ellipse, the
    /// minor radius: the tail is never sharper than [`MIN_TAIL_PX`] there).
    pub fn new(hardness: f32, radius: f32) -> Self {
        let h = if hardness.is_finite() { hardness.clamp(0.0, 1.0) } else { 1.0 };
        if h >= 1.0 {
            return Self { hard: true, a: 1.0, s: 0.0 };
        }
        let i = FALLOFF.iter().rposition(|e| e.0 <= h).unwrap_or(0);
        let (Some(&(h0, a0, s0)), Some(&(h1, a1, s1))) = (FALLOFF.get(i), FALLOFF.get(i + 1)) else {
            return Self { hard: true, a: 1.0, s: 0.0 };
        };
        let t = (h - h0) / (h1 - h0);
        let (a, s) = (a0 + (a1 - a0) * t, s0 + (s1 - s0) * t);
        // Widen a tail sharper than the minimum about its 50 % point.
        let s_min = MIN_TAIL_PX / radius.max(0.5);
        if s < s_min {
            let half = a + TAIL_HALF * s;
            return Self { hard: false, a: half - TAIL_HALF * s_min, s: s_min };
        }
        Self { hard: false, a, s }
    }

    pub fn is_hard(&self) -> bool {
        self.hard
    }

    /// Relative radius out to which coverage is full (for a hard tip: the nominal radius, before
    /// the anti-aliased edge).
    pub fn full_to(&self) -> f32 {
        self.a
    }

    /// Relative radius beyond which a soft tip paints nothing (1 for a hard tip, whose edge reach
    /// is in pixels: [`EDGE_REACH`]).
    pub fn reach(&self) -> f32 {
        if self.hard { 1.0 } else { self.a + TAIL_CUTOFF * self.s }
    }

    /// Soft coverage at relative radius `q` (a hard tip is full: its edge is [`edge_coverage`]).
    #[inline]
    pub fn at(&self, q: f32) -> f32 {
        if self.hard || q <= self.a {
            return 1.0;
        }
        let t = (q - self.a) / self.s;
        if t >= TAIL_CUTOFF { 0.0 } else { (-t * t).exp() }
    }
}

/// Coverage of a computed tip at relative radius `q` (0 = centre, 1 = nominal edge) and signed
/// distance `sd` in pixels from the nominal edge: a hard tip's anti-aliased edge, a fixed width in
/// pixels at every size, or a soft tip's falloff, which scales with the tip.
#[inline]
pub fn tip_coverage(q: f32, sd: f32, falloff: &Falloff) -> f32 {
    if falloff.hard { if sd >= EDGE_REACH { 0.0 } else { edge_coverage(sd) } } else { falloff.at(q) }
}

/// Coverage of a round dab at distance `d` from its centre (anti-aliased edge, hardness falloff).
#[inline]
pub fn dab_coverage(d: f32, radius: f32, hardness: f32) -> f32 {
    let r = radius.max(1e-3);
    tip_coverage(d / r, d - r, &Falloff::new(hardness, r))
}

/// Rasterize a stroke onto `target`, optionally limited by a selection (grayscale coverage surface).
/// Returns the damaged rectangle.
pub fn apply_stroke(target: &mut Surface, stroke: &Stroke, selection: Option<&Surface>, lock_transparency: bool) -> Rect {
    render_stroke(target, &stroke.brush, &stroke.points, selection, lock_transparency, 1.0)
}

/// Exponential moving-average smoothing of input points (Photoshop's "Smoothing" %, simplified).
pub fn smooth(points: &[StrokePoint], amount: f32) -> Vec<StrokePoint> {
    let a = (1.0 - amount.clamp(0.0, 0.95)) as f64;
    let mut out = Vec::with_capacity(points.len());
    let mut cur: Option<StrokePoint> = None;
    for p in points {
        let n = match cur {
            None => *p,
            Some(c) => StrokePoint { x: c.x + (p.x - c.x) * a, y: c.y + (p.y - c.y) * a, ..*p },
        };
        out.push(n);
        cur = Some(n);
    }
    if let (Some(last), Some(end)) = (out.last_mut(), points.last()) {
        *last = *end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_color::PixelFormat;

    fn stroke(points: &[(f64, f64)], size: f32) -> Stroke {
        Stroke {
            brush: BrushSettings { size, pressure_size: false, ..Default::default() },
            points: points.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.0)).collect(),
        }
    }

    #[test]
    fn dab_spacing() {
        let s = stroke(&[(0.0, 0.0), (100.0, 0.0)], 10.0);
        let d = dabs(&s);
        // spacing 0.1 * 10px = 1px → ~101 dabs
        assert!((95..=105).contains(&d.len()), "{}", d.len());
        let s = Stroke { brush: BrushSettings { spacing: 1.0, pressure_size: false, size: 10.0, ..Default::default() }, ..s };
        assert_eq!(dabs(&s).len(), 11);
    }

    #[test]
    fn coverage_profile() {
        assert_eq!(dab_coverage(0.0, 10.0, 1.0), 1.0);
        assert_eq!(dab_coverage(20.0, 10.0, 1.0), 0.0);
        let soft_mid = dab_coverage(5.0, 10.0, 0.0);
        assert!(soft_mid > 0.2 && soft_mid < 0.8);
        assert!(dab_coverage(9.5, 10.0, 1.0) > 0.5);
    }

    #[test]
    fn hard_edge_is_a_fixed_width_ramp_at_every_size() {
        // Photoshop's hard round tips: 50 % just inside the radius, 90 % → 10 % over ~1.55 px.
        for r in [5.0f32, 50.0, 250.0] {
            let at = |level: f32| {
                let (mut lo, mut hi) = (r - 3.0, r + 3.0);
                for _ in 0..40 {
                    let mid = (lo + hi) / 2.0;
                    if dab_coverage(mid, r, 1.0) > level { lo = mid } else { hi = mid }
                }
                lo
            };
            assert!((at(0.5) - (r - EDGE_INSET)).abs() < 1e-3, "r50 at {r}");
            let width = at(0.1) - at(0.9);
            assert!((width - 1.55).abs() < 0.02, "edge width {width} at {r}");
            assert_eq!(dab_coverage(r + EDGE_REACH, r, 1.0), 0.0);
            assert_eq!(dab_coverage(r - 2.0, r, 1.0), 1.0);
        }
    }

    #[test]
    fn hard_brush_paints_solid_line() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let dmg = apply_stroke(&mut s, &stroke(&[(10.0, 10.0), (90.0, 10.0)], 8.0), None, false);
        assert!(dmg.contains(50, 10));
        assert_eq!(s.pixel(50, 10), vec![0.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.pixel(50, 30)[3], 0.0);
    }

    #[test]
    fn opacity_caps_overlap() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let mut st = stroke(&[(10.0, 10.0), (60.0, 10.0), (10.0, 10.0)], 10.0);
        st.brush.opacity = 0.5;
        apply_stroke(&mut s, &st, None, false);
        let a = s.pixel(30, 10)[3];
        assert!((a - 0.5).abs() < 0.02, "{a}");
    }

    #[test]
    fn selection_limits_paint() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 50, 100), &[1.0]);
        apply_stroke(&mut s, &stroke(&[(10.0, 10.0), (90.0, 10.0)], 8.0), Some(&sel), false);
        assert_eq!(s.pixel(20, 10)[3], 1.0);
        assert_eq!(s.pixel(70, 10)[3], 0.0);
    }

    #[test]
    fn eraser_removes_alpha() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 100, 20), &[1.0, 0.0, 0.0, 1.0]);
        let mut st = stroke(&[(10.0, 10.0), (90.0, 10.0)], 8.0);
        st.brush.erase = true;
        apply_stroke(&mut s, &st, None, false);
        assert_eq!(s.pixel(50, 10)[3], 0.0);
        assert_eq!(s.pixel(50, 1)[3], 1.0);
    }

    #[test]
    fn lock_transparency_preserves_alpha() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 50, 20), &[1.0, 1.0, 1.0, 1.0]);
        apply_stroke(&mut s, &stroke(&[(10.0, 10.0), (90.0, 10.0)], 8.0), None, true);
        assert_eq!(s.pixel(20, 10), vec![0.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.pixel(70, 10)[3], 0.0);
    }

    #[test]
    fn pressure_changes_size() {
        let mut st = stroke(&[(0.0, 0.0), (100.0, 0.0)], 20.0);
        st.brush.pressure_size = true;
        st.points[0].pressure = 0.1;
        st.points[1].pressure = 1.0;
        let d = dabs(&st);
        assert!(d.first().unwrap().radius < d.last().unwrap().radius);
    }

    #[test]
    fn smoothing_keeps_endpoints() {
        let pts: Vec<StrokePoint> = (0..20).map(|i| StrokePoint::new(i as f64, if i % 2 == 0 { 0.0 } else { 10.0 }, 1.0)).collect();
        let s = smooth(&pts, 0.8);
        assert_eq!(s.first(), pts.first());
        assert_eq!(s.last(), pts.last());
        let jitter: f64 = s[1..19].iter().map(|p| (p.y - 5.0).abs()).sum();
        let orig: f64 = pts[1..19].iter().map(|p| (p.y - 5.0).abs()).sum();
        assert!(jitter < orig);
    }

    #[test]
    fn works_at_all_depths() {
        for f in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAYA8, PixelFormat::CMYKA8] {
            let mut s = Surface::new(f);
            let mut st = stroke(&[(5.0, 5.0), (40.0, 5.0)], 6.0);
            st.brush.color = [1.0, 1.0, 1.0, 1.0];
            apply_stroke(&mut s, &st, None, false);
            let rgba = s.rgba(20, 5);
            assert!(rgba[3] > 0.99 && rgba[0] > 0.98, "{f:?} {rgba:?}");
        }
    }
}

#[cfg(test)]
mod brush_tests;
