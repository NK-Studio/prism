//! CSP Ribbon through the dab builder and the stroke renderer (see `csp_ribbon`).
use prism_color::{ColorMode, PixelFormat, SampleType};
use prism_geom::Point;
use prism_raster::Surface;

use crate::StrokePoint;
use crate::brush::{BrushSettings, TipShape};
use crate::csp_ribbon::{CspRibbon, RIBBON_STEP, RibbonEdge, RibbonSlice};
use crate::render::{StrokeRenderer, render_stroke};
use crate::tile::GrayTile;

/// A `w × h` tip painted where `f(x, y)`.
fn tip(w: u32, h: u32, f: impl Fn(u32, u32) -> bool) -> GrayTile {
    GrayTile::from_fn(w, h, |x, y| if f(x, y) { 1.0 } else { 0.0 })
}

fn ribbon_brush(tip: TipShape, size: f32, quarter_turns: u8) -> BrushSettings {
    BrushSettings {
        size,
        tip,
        spacing: 3.0,
        pressure_size: false,
        csp_anti_alias: Some(2),
        csp_ribbon: Some(CspRibbon { quarter_turns }),
        ..Default::default()
    }
}

/// A straight stroke to the right along y = 50.
fn line(x0: f64, x1: f64) -> Vec<StrokePoint> {
    let n = ((x1 - x0) / 5.0).ceil() as usize;
    (0..=n).map(|i| StrokePoint::new(x0 + (x1 - x0) * i as f64 / n as f64, 50.0, 1.0)).collect()
}

fn paint(b: &BrushSettings, pts: &[StrokePoint]) -> Surface {
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, b, pts, None, false, 1.0);
    s
}

fn alpha(s: &Surface, x: i32, y: i32) -> f32 {
    s.rgba(x, y)[3]
}

#[test]
fn a_ribbon_is_continuous_where_dabs_at_the_same_spacing_leave_gaps() {
    let solid = TipShape::Sampled(tip(20, 8, |_, _| true));
    let ribbon = paint(&ribbon_brush(solid.clone(), 20.0, 0), &line(10.0, 190.0));
    let dabs = paint(&BrushSettings { csp_ribbon: None, ..ribbon_brush(solid, 20.0, 0) }, &line(10.0, 190.0));
    let row = |s: &Surface, y: i32| (14..180).map(|x| alpha(s, x, y)).fold(1.0, f32::min);
    for y in [42, 46, 50, 54, 57] {
        assert!(row(&ribbon, y) > 0.99, "row {y}: {}", row(&ribbon, y));
    }
    assert!(row(&dabs, 50) < 0.01, "300% spacing leaves gaps between stamped dabs");
    // The ribbon is the brush diameter wide: the tip's cross axis spans it.
    assert!(alpha(&ribbon, 100, 37) < 0.01 && alpha(&ribbon, 100, 63) < 0.01);
    // Any chunking of the input gives the same ribbon.
    let pts = line(10.0, 190.0);
    let mut r = StrokeRenderer::new(&ribbon_brush(TipShape::Sampled(tip(20, 8, |_, _| true)), 20.0, 0), Some(PixelFormat::RGBA32F), 1.0);
    for chunk in pts.chunks(3) {
        r.push(chunk);
    }
    r.finish();
    let mut chunked = Surface::new(PixelFormat::RGBA32F);
    r.composite(&Surface::new(PixelFormat::RGBA32F), &mut chunked, None, false, true);
    for x in (0..200).step_by(7) {
        for y in 35..65 {
            assert_eq!(alpha(&chunked, x, y), alpha(&ribbon, x, y), "({x}, {y})");
        }
    }
}

#[test]
fn the_tip_repeats_along_the_stroke_at_its_own_aspect() {
    // Rows 0..9 painted, rows 10..19 empty; 10 texels across a 10 px ribbon is 1 texel per pixel,
    // so the ribbon alternates 10 px painted and 10 px empty from the stroke's start.
    let half = TipShape::Sampled(tip(10, 20, |_, y| y < 10));
    let s = paint(&ribbon_brush(half.clone(), 10.0, 0), &line(20.0, 180.0));
    for period in 0..6 {
        let x0 = 20 + period * 20;
        assert!(alpha(&s, x0 + 5, 50) > 0.99, "period {period} starts painted");
        assert!(alpha(&s, x0 + 15, 50) < 0.01, "period {period} ends empty");
    }
    // Along thickness 200% doubles the period.
    let s = paint(&BrushSettings { csp_tip_scale: Some([1.0, 2.0]), ..ribbon_brush(half, 10.0, 0) }, &line(20.0, 180.0));
    assert!(alpha(&s, 20 + 15, 50) > 0.99 && alpha(&s, 20 + 30, 50) < 0.01);
    assert!(alpha(&s, 20 + 45, 50) > 0.99);
}

#[test]
fn quarter_turns_rotate_the_tip_without_mirroring() {
    // Left half painted (columns 0..9 of 20), 10 rows: at one quarter turn the columns run along
    // the stroke and the rows across, upright on a stroke drawn to the right.
    let top = TipShape::Sampled(tip(20, 10, |_, y| y < 5));
    let s = paint(&ribbon_brush(top.clone(), 10.0, 1), &line(20.0, 180.0));
    assert!(alpha(&s, 100, 47) > 0.99, "the tip's top is above the path");
    assert!(alpha(&s, 100, 53) < 0.01, "the tip's bottom half is empty");
    // Three quarter turns put it upside down.
    let s = paint(&ribbon_brush(top, 10.0, 3), &line(20.0, 180.0));
    assert!(alpha(&s, 100, 47) < 0.01 && alpha(&s, 100, 53) > 0.99);
    // At no turn the tip's columns run across: column 0 on the right of the travel direction.
    let left = TipShape::Sampled(tip(10, 4, |x, _| x < 5));
    let s = paint(&ribbon_brush(left.clone(), 10.0, 0), &line(20.0, 180.0));
    assert!(alpha(&s, 100, 53) > 0.99 && alpha(&s, 100, 47) < 0.01);
    let s = paint(&ribbon_brush(left.clone(), 10.0, 2), &line(20.0, 180.0));
    assert!(alpha(&s, 100, 47) > 0.99 && alpha(&s, 100, 53) < 0.01);
    // Flip horizontal mirrors the tip's columns.
    let s = paint(&BrushSettings { flip_x: true, ..ribbon_brush(left, 10.0, 0) }, &line(20.0, 180.0));
    assert!(alpha(&s, 100, 47) > 0.99 && alpha(&s, 100, 53) < 0.01);
}

#[test]
fn a_sequence_moves_to_the_next_tip_when_one_runs_out() {
    let tips = TipShape::Sequence(vec![tip(10, 10, |_, _| true), tip(10, 10, |_, _| false)]);
    let s = paint(&ribbon_brush(tips, 10.0, 0), &line(20.0, 180.0));
    for period in 0..6 {
        let x0 = 20 + period * 20;
        assert!(alpha(&s, x0 + 5, 50) > 0.99 && alpha(&s, x0 + 15, 50) < 0.01, "period {period}");
    }
}

#[test]
fn ribbons_follow_curves_and_keep_flow_and_colour_at_every_depth() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        let b = BrushSettings { flow: 0.5, color: [1.0, 0.0, 0.0, 1.0], ..ribbon_brush(TipShape::Sampled(tip(16, 16, |_, _| true)), 16.0, 0) };
        let arc: Vec<_> = (0..=60).map(|i| f64::from(i) * std::f64::consts::PI / 60.0).map(|a| StrokePoint::new(100.0 - 60.0 * a.cos(), 100.0 - 60.0 * a.sin(), 1.0)).collect();
        render_stroke(&mut s, &b, &arc, None, false, 1.0);
        for i in 3..58 {
            let a = f64::from(i) * std::f64::consts::PI / 60.0;
            let (x, y) = ((100.0 - 60.0 * a.cos()) as i32, (100.0 - 60.0 * a.sin()) as i32);
            let p = s.rgba(x, y);
            assert!((p[3] - 0.5).abs() < 0.03 && p[0] > 0.97, "{depth:?} at {i}: {p:?}");
        }
    }
}

#[test]
fn hostile_ribbons_never_panic() {
    let tips = [
        TipShape::Round,
        TipShape::Sampled(GrayTile { width: 0, height: 0, data: Vec::new() }),
        TipShape::Sampled(GrayTile { width: 3, height: 3, data: vec![1] }),
        TipShape::Sampled(tip(1, 1, |_, _| true)),
        TipShape::Sampled(tip(400, 1, |_, _| true)),
        TipShape::Sequence(Vec::new()),
        TipShape::Sequence(vec![tip(1, 300, |_, _| true), GrayTile { width: 2, height: 2, data: Vec::new() }]),
    ];
    let strokes = [
        line(10.0, 60.0),
        vec![StrokePoint::new(10.0, 10.0, 1.0)],
        vec![StrokePoint::new(10.0, 10.0, 1.0); 5],
        vec![StrokePoint::new(f64::NAN, 10.0, 1.0), StrokePoint::new(20.0, 12.0, f32::NAN), StrokePoint::new(30.0, 10.0, 0.0)],
    ];
    // Far from the origin (only with tips that draw ribbons: the dab path is not under test).
    let far = [
        vec![StrokePoint::new(3e9, -3e9, 1.0), StrokePoint::new(3e9 + 9.0, -3e9 + 4.0, 1.0)],
        vec![StrokePoint::new(2147483640.0, 5.0, 1.0), StrokePoint::new(2147483660.0, 5.0, 1.0)],
    ];
    for t in &tips {
        for turns in [0, 1, 3, 4, 255] {
            for size in [0.5, 3.0, 60.0, f32::NAN] {
                for scale in [None, Some([0.0, f32::INFINITY]), Some([f32::NAN, -1.0]), Some([10.0, 0.01])] {
                    let b = BrushSettings { csp_tip_scale: scale, ..ribbon_brush(t.clone(), size, turns) };
                    let ribbon = matches!(t, TipShape::Sampled(g) if g.is_valid()) || matches!(t, TipShape::Sequence(v) if v.iter().any(GrayTile::is_valid));
                    for pts in strokes.iter().chain(far.iter().filter(|_| ribbon)) {
                        let mut s = Surface::new(PixelFormat::RGBA8);
                        render_stroke(&mut s, &b, pts, None, false, 1.0);
                    }
                }
            }
        }
    }
    // A slice with no extent, or non-finite geometry, covers nothing.
    let edge = |x: f64, half: f32| RibbonEdge { center: Point::new(x, 0.0), half, cross: [0.0, 1.0] };
    for (a, b) in [(edge(0.0, 5.0), edge(0.0, 5.0)), (edge(0.0, 0.0), edge(4.0, 0.0)), (edge(f64::NAN, 5.0), edge(4.0, f32::INFINITY))] {
        let slice = RibbonSlice { start: a, end: b, u0: 0.0, u1: 4.0, tip: 0, quarter_turns: 0 };
        assert_eq!(slice.locate(2.0, 0.5), None);
        let _ = slice.rect();
    }
}

#[test]
fn locate_inverts_the_slice_and_shares_edges() {
    let edge = |x: f64, half: f32, cross: [f32; 2]| RibbonEdge { center: Point::new(x, 0.0), half, cross };
    // A trapezoid that also turns: start 4 px wide, end 8 px wide and tilted.
    let (dir_s, dir_c) = 0.3f32.sin_cos();
    let a = RibbonSlice { start: edge(0.0, 2.0, [0.0, 1.0]), end: edge(RIBBON_STEP, 4.0, [dir_s, dir_c]), u0: 0.0, u1: 4.0, tip: 0, quarter_turns: 0 };
    let b = RibbonSlice { start: a.end, end: edge(2.0 * RIBBON_STEP, 4.0, [0.0, 1.0]), ..a };
    for (s, t) in [(0.5, 0.5), (0.1, 0.9), (0.9, 0.05), (0.0, 0.0)] {
        let [p00, p10, p11, p01] = a.corners();
        let lerp = |p: (f64, f64), q: (f64, f64), k: f64| (p.0 + (q.0 - p.0) * k, p.1 + (q.1 - p.1) * k);
        let p = lerp(lerp(p00, p10, s), lerp(p01, p11, s), t);
        let (gs, gt) = a.locate(p.0, p.1).unwrap();
        assert!((gs - s).abs() < 1e-6 && (gt - t).abs() < 1e-6, "({s}, {t}) → ({gs}, {gt})");
    }
    // Points on the shared edge belong to the second slice only.
    let mid = (a.end.center.x, a.end.center.y);
    assert!(a.locate(mid.0, mid.1).is_none());
    assert!(b.locate(mid.0, mid.1).is_some());
}

#[test]
fn a_transparent_padding_line_leaves_no_seam_between_copies() {
    // CSP materials can end in one transparent column or row; at a large reduction an odd mip
    // chain would weight it like real content and print a light line at every join.
    let row_padded = TipShape::Sampled(tip(361, 97, |_, y| y < 96));
    let col_padded = TipShape::Sampled(tip(97, 361, |x, _| x < 96));
    for (t, turns) in [(row_padded, 0), (col_padded, 1)] {
        let s = paint(&ribbon_brush(t, 12.0, turns), &line(10.0, 190.0));
        let min = (16..180).map(|x| alpha(&s, x, 50)).fold(1.0, f32::min);
        assert!(min > 0.98, "turns {turns}: {min}");
    }
    // A tip that is transparent all over keeps its size and stays empty.
    let s = paint(&ribbon_brush(TipShape::Sampled(tip(9, 9, |_, _| false)), 12.0, 0), &line(10.0, 190.0));
    assert_eq!((0..200).map(|x| alpha(&s, x, 50)).fold(0.0, f32::max), 0.0);
}
