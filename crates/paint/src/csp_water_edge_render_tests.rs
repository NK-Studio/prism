//! Watercolor edge through the stroke renderer (see `csp_water_edge` for the equations).
use prism_color::{ColorMode, PixelFormat, SampleType};
use prism_geom::Rect;
use prism_raster::Surface;

use crate::StrokePoint;
use crate::brush::BrushSettings;
use crate::csp_water_edge::CspWaterEdge;
use crate::render::{StrokeRenderer, render_stroke};

fn brush(edge: Option<CspWaterEdge>) -> BrushSettings {
    BrushSettings { size: 20.0, opacity: 0.9, pressure_size: false, spacing: 0.1, color: [1.0, 0.0, 0.0, 1.0], csp_water_edge: edge, ..Default::default() }
}

fn edge() -> CspWaterEdge {
    CspWaterEdge { radius: 2.0, opacity: 0.15, ..Default::default() }
}

fn line() -> Vec<StrokePoint> {
    // Crosses several 64-pixel stroke-buffer tiles.
    (0..=40).map(|i| StrokePoint::new(30.0 + f64::from(i) * 2.5, 60.0 + f64::from(i % 7), 1.0)).collect()
}

fn formats() -> [PixelFormat; 3] {
    [SampleType::U8, SampleType::U16, SampleType::F32].map(|d| PixelFormat::new(ColorMode::Rgb, d, true))
}

fn alphas(s: &Surface, r: Rect) -> Vec<f32> {
    (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| (x, y))).map(|(x, y)| s.rgba(x, y)[3]).collect()
}

#[test]
fn edge_pixels_gain_alpha_beyond_the_stroke_opacity_at_every_depth() {
    for fmt in formats() {
        let (mut plain, mut wet) = (Surface::new(fmt), Surface::new(fmt));
        render_stroke(&mut plain, &brush(None), &line(), None, false, 1.0);
        render_stroke(&mut wet, &brush(Some(edge())), &line(), None, false, 1.0);
        let area = Rect::new(0, 0, 180, 120);
        let (a, b) = (alphas(&plain, area), alphas(&wet, area));
        let max_plain = a.iter().copied().fold(0.0, f32::max);
        let max_wet = b.iter().copied().fold(0.0, f32::max);
        // Coverage itself, unquantized: tiny coverage that rounds to 0 at 8 bits still has an edge.
        let mut coverage = Surface::new(PixelFormat::RGBA32F);
        render_stroke(&mut coverage, &brush(None), &line(), None, false, 1.0);
        let c = alphas(&coverage, area);
        assert!(max_plain <= 0.905, "{fmt:?} {max_plain}");
        assert!(max_wet > 0.99, "{fmt:?}: the edge adds alpha past 90% ({max_wet})");
        // Only covered pixels change; the interior beyond the radius keeps the stroke opacity.
        for ((p, w), c) in a.iter().zip(&b).zip(&c) {
            assert!(*w >= *p - 0.003, "{fmt:?}");
            if *c == 0.0 {
                assert_eq!(*w, 0.0, "{fmt:?}: without after-drag blur nothing leaks outside");
            }
        }
        let centre = wet.rgba(80, 69)[3];
        assert!((centre - 0.9).abs() < 0.005, "{fmt:?} {centre}");
    }
}

#[test]
fn incremental_live_edges_match_the_committed_stroke() {
    for fmt in formats() {
        let b = brush(Some(edge()));
        let pts = line();
        let mut once = Surface::new(fmt);
        render_stroke(&mut once, &b, &pts, None, false, 1.0);
        let pre = Surface::new(fmt);
        let mut live = pre.clone();
        let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0);
        for chunk in pts.chunks(3) {
            r.push(chunk);
            r.composite(&pre, &mut live, None, false, false);
        }
        r.finish();
        r.composite(&pre, &mut live, None, false, false);
        let area = Rect::new(0, 0, 180, 120);
        assert_eq!(alphas(&once, area), alphas(&live, area), "{fmt:?}");
    }
}

#[test]
fn after_drag_defers_the_edge_and_blur_spreads_a_faint_halo() {
    let deferred = CspWaterEdge { after_drag: true, blur: 4.0, ..edge() };
    let fmt = PixelFormat::RGBA32F;
    let pts = line();
    // While drawing: no edge yet.
    let pre = Surface::new(fmt);
    let (mut drawing, mut plain) = (pre.clone(), pre.clone());
    let mut r = StrokeRenderer::new(&brush(Some(deferred.clone())), Some(fmt), 1.0);
    r.push(&pts);
    r.finish();
    r.composite(&pre, &mut drawing, None, false, false);
    render_stroke(&mut plain, &brush(None), &pts, None, false, 1.0);
    let area = Rect::new(0, 0, 180, 120);
    assert_eq!(alphas(&drawing, area), alphas(&plain, area));
    // Committed: the edge is blurred and leaks a little outside the stroke, within the blur.
    let mut done = pre.clone();
    let bounds = r.bounds();
    let damage = r.composite(&pre, &mut done, None, false, true);
    assert!(damage.x0 < bounds.x0 && damage.y1 > bounds.y1, "{damage:?} {bounds:?}");
    let (p, d) = (alphas(&plain, area), alphas(&done, area));
    let halo = p.iter().zip(&d).filter(|(p, d)| **p == 0.0 && **d > 0.0).count();
    assert!(halo > 0, "a halo outside the stroke");
    assert!(d.iter().copied().fold(0.0, f32::max) > 0.9);
    for y in 0..120 {
        for x in 0..180 {
            if done.rgba(x, y)[3] > 0.0 {
                let near = (-5..=5).any(|dy| (-5..=5).any(|dx| plain.rgba(x + dx, y + dy)[3] > 0.0));
                assert!(near, "halo at {x},{y} beyond the blur radius");
            }
        }
    }
    // Without blur, after-drag still applies the plain edge on commit.
    let mut sharp = pre.clone();
    render_stroke(&mut sharp, &brush(Some(CspWaterEdge { after_drag: true, ..edge() })), &pts, None, false, 1.0);
    let mut live = pre.clone();
    render_stroke(&mut live, &brush(Some(edge())), &pts, None, false, 1.0);
    assert_eq!(alphas(&sharp, area), alphas(&live, area));
}

#[test]
fn darkness_lowers_the_value_of_edge_pixels_only() {
    let fmt = PixelFormat::RGBA32F;
    let mut s = Surface::new(fmt);
    render_stroke(&mut s, &brush(Some(CspWaterEdge { darkness: 1.0, ..edge() })), &line(), None, false, 1.0);
    let centre = s.rgba(80, 69);
    assert!((centre[0] - 1.0).abs() < 1e-4, "{centre:?}");
    let darkest = (0..120).flat_map(|y| (0..180).map(move |x| (x, y))).map(|(x, y)| s.rgba(x, y)).filter(|p| p[3] > 0.5).map(|p| p[0]).fold(1.0, f32::min);
    assert!(darkest < 0.9, "{darkest}");
}

#[test]
fn hostile_settings_render_without_panicking() {
    for e in [
        CspWaterEdge { radius: f32::NAN, opacity: f32::INFINITY, darkness: f32::NAN, blur: f32::NAN, after_drag: true },
        CspWaterEdge { radius: 1e9, opacity: -3.0, darkness: 9.0, blur: 1e9, after_drag: true },
        CspWaterEdge { radius: 0.0, ..edge() },
    ] {
        let mut s = Surface::new(PixelFormat::RGBA8);
        render_stroke(&mut s, &brush(Some(e)), &[StrokePoint::new(10.0, 10.0, 1.0)], None, false, 1.0);
    }
}
