//! Brush-engine tests: tips, spacing, dynamics, masks, smoothing, determinism, formats.

#[test]
fn csp_correction_ignores_photoshop_smoothing_and_preserves_it_for_switching_back() {
    let points: Vec<_> = (0..60)
        .map(|i| StrokePoint { x: 10.0 + f64::from(i) * 2.0, y: 20.0 + (f64::from(i) * 0.7).sin() * 6.0, time: f64::from(i) * 8.0, ..Default::default() })
        .collect();
    for level in [0, 12] {
        let csp = BrushSettings {
            csp_correction: true,
            csp_stabilizer: (level > 0).then_some(crate::CspStabilizer { level, ..Default::default() }),
            size: 6.0,
            ..brush()
        };
        let reference = dabs(&Stroke { brush: csp.clone(), points: points.clone() });
        for amount in [0.0, 0.1, 1.0] {
            let photoshop = BrushSettings { smoothing: Smoothing { amount, ..Default::default() }, ..brush() };
            let selected = csp.clone().picked_over(&photoshop);
            assert_eq!(dabs(&Stroke { brush: selected.clone(), points: points.clone() }), reference);
            let mut live = StrokeRenderer::new(&selected, None, 1.0).record_dabs();
            for chunk in points.chunks(7) {
                live.push(chunk);
            }
            live.finish();
            assert_eq!(live.dabs(), reference.as_slice());
            assert_eq!(photoshop.clone().picked_over(&selected).smoothing, photoshop.smoothing);
            let restored: BrushSettings = serde_json::from_value(serde_json::to_value(&selected).unwrap()).unwrap();
            assert!(restored.csp_correction);
            assert_eq!(dabs(&Stroke { brush: restored, points: points.clone() }), reference);
        }
    }
    assert!(!serde_json::from_str::<BrushSettings>("{}").unwrap().csp_correction);
}

use prism_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use prism_geom::{Point, Rect};
use prism_raster::Surface;

use crate::brush::*;
use crate::dynamics::{Smoother, smooth_points};
use crate::render::{BrushContext, StrokeRenderer, mask_combine, render_stroke};
use crate::tile::GrayTile;
use crate::{Dab, PressureCurve, Stroke, StrokePoint, dabs};

#[test]
fn live_preview_clones_share_immutable_brush_pixels() {
    use std::sync::Arc;
    let mut source = BrushSettings { tip: TipShape::Sampled(GrayTile::from_fn(128, 128, |_, _| 1.0)), ..brush() };
    let context = BrushContext::new(&source);
    let builder = crate::dynamics::DabBuilder::new(&source);
    let dual = crate::dynamics::DualBuilder::new(&source);
    assert!(Arc::ptr_eq(&context.brush, &context.clone().brush));
    assert!(Arc::ptr_eq(&builder.brush, &builder.clone().brush));
    assert!(Arc::ptr_eq(&dual.brush, &dual.clone().brush));
    // Later tool edits cannot modify the immutable snapshot used by the live stroke.
    source.size = 123.0;
    if let TipShape::Sampled(tip) = &mut source.tip {
        tip.data.fill(0);
    }
    assert_ne!(context.brush.size, source.size);
    assert_eq!(context.brush.tip, builder.brush.tip);
    assert_ne!(context.brush.tip, source.tip);
}

#[test]
fn csp_fixed_sub_color_is_seed_independent_and_survives_serialization() {
    let mut b = BrushSettings { color: [1.0, 0.0, 0.0, 1.0], background: [0.0, 0.0, 1.0, 1.0], csp_sub_color: Some(0.25), ..brush() };
    let pts = vec![StrokePoint::new(10.0, 10.0, 1.0), StrokePoint::new(60.0, 10.0, 1.0)];
    for seed in [0, 7, 99] {
        b.seed = seed;
        let restored = serde_json::from_value(serde_json::to_value(&b).unwrap()).unwrap();
        let out = dabs(&Stroke { brush: restored, points: pts.clone() });
        assert!(!out.is_empty());
        for dab in out {
            assert_eq!(dab.color, [0.75, 0.0, 0.25, 1.0]);
        }
    }
    b.csp_sub_color = None;
    assert!(dabs(&Stroke { brush: b, points: pts }).iter().all(|d| d.color == [1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn csp_fixed_sub_color_renders_at_every_depth() {
    let b = BrushSettings { size: 10.0, color: [1.0, 0.0, 0.0, 1.0], background: [0.0, 0.0, 1.0, 1.0], csp_sub_color: Some(0.25), ..brush() };
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        render_stroke(&mut surface, &b, &[StrokePoint::new(20.0, 20.0, 1.0)], None, false, 1.0);
        let actual = surface.rgba(20, 20);
        for (actual, expected) in actual.iter().zip([0.75, 0.0, 0.25, 1.0]) {
            assert!((actual - expected).abs() < 0.005, "{sample:?}: {actual} vs {expected}");
        }
    }
}

fn brush() -> BrushSettings {
    BrushSettings { pressure_size: false, spacing: 0.25, ..Default::default() }
}

fn line(x0: f64, x1: f64, y: f64) -> Vec<StrokePoint> {
    vec![StrokePoint::new(x0, y, 1.0), StrokePoint::new(x1, y, 1.0)]
}

fn dabs_of(b: &BrushSettings, pts: &[StrokePoint]) -> Vec<Dab> {
    dabs(&Stroke { brush: b.clone(), points: pts.to_vec() })
}

#[test]
fn csp_spray_separates_area_particle_size_count_and_orientation() {
    let b = BrushSettings {
        size: 30.0,
        angle: 70.0,
        csp_spray: Some(crate::CspSpray { size: 2.0, count: 8, bias: -1.0, angle: 15.0, ..Default::default() }),
        ..brush()
    };
    let points = [StrokePoint::new(40.0, 50.0, 1.0)];
    let d = dabs_of(&b, &points);
    assert_eq!(d.len(), 8);
    for dab in &d {
        assert!(((dab.center.x - 40.0).hypot(dab.center.y - 50.0) - 15.0).abs() < 1e-5);
        assert_eq!(dab.radius, 1.0);
        assert!((dab.angle.to_degrees() - 15.0).abs() < 1e-5);
    }
    let mut wider = b.clone();
    wider.size = 60.0;
    let wide = dabs_of(&wider, &points);
    for (a, b) in d.iter().zip(wide) {
        assert_eq!(a.radius, b.radius);
        assert!(((a.center.x - 40.0) * 2.0 - (b.center.x - 40.0)).abs() < 1e-5);
    }
    let encoded = serde_json::to_vec(&b).unwrap();
    assert_eq!(serde_json::from_slice::<BrushSettings>(&encoded).unwrap(), b);
    assert!(serde_json::from_str::<BrushSettings>("{}").unwrap().csp_spray.is_none());
}

#[test]
fn csp_spray_chunking_pressure_and_work_limits() {
    let curve = PressureCurve::new(vec![[0.0, 0.0], [1.0, 0.5]]).unwrap();
    let dynamic = Dynamic { control: Control::PenPressure, pressure_curve: Some(curve), ..Default::default() };
    let b = BrushSettings {
        csp_spray: Some(crate::CspSpray { count: 8, size: 4.0, size_dynamic: dynamic.clone(), count_dynamic: dynamic, ..Default::default() }),
        ..brush()
    };
    let points = line(0.0, 20.0, 10.0);
    let whole = dabs_of(&b, &points);
    let mut genr = crate::dynamics::DabGenerator::new(&b, 1.0);
    let (mut chunked, mut dual) = (Vec::new(), Vec::new());
    for point in &points {
        genr.push(&[*point], &mut chunked, &mut dual);
    }
    genr.finish(&mut chunked, &mut dual);
    assert_eq!(whole, chunked);
    assert_eq!(dabs_of(&b, &points[..1]).len(), 4);
    assert!(whole.iter().all(|dab| dab.radius == 1.0));
    let mut hostile = b;
    let spray = hostile.csp_spray.as_mut().unwrap();
    spray.count = u32::MAX;
    spray.size = f32::INFINITY;
    spray.bias = f32::NAN;
    spray.angle = f32::INFINITY;
    let d = dabs_of(&hostile, &points[..1]);
    assert!(d.len() <= crate::csp_spray::MAX_PARTICLES as usize);
    assert!(d.iter().all(|d| d.center.x.is_finite() && d.center.y.is_finite() && d.angle.is_finite()));
}

fn raster(b: &BrushSettings, d: &Dab) -> (Rect, Vec<f32>) {
    let ctx = BrushContext::new(b);
    let r = ctx.dab_rect(d, false);
    let mut v = Vec::new();
    ctx.rasterize(d, false, r, &mut v);
    (r, v)
}

/// A rasterized dab's value at a pixel; nothing outside its rectangle.
fn at(r: &(Rect, Vec<f32>), x: i32, y: i32) -> f32 {
    if !r.0.contains(x, y) {
        return 0.0;
    }
    r.1[((y - r.0.y0) * r.0.width() as i32 + (x - r.0.x0)) as usize]
}

fn paint(b: &BrushSettings, pts: &[StrokePoint], w: i32, h: i32) -> Surface {
    let mut s = Surface::new(PixelFormat::RGBA32F);
    let _ = w + h;
    render_stroke(&mut s, b, pts, None, false, 1.0);
    s
}

fn alpha_sum(s: &Surface, r: Rect) -> f32 {
    let mut t = 0.0;
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            t += s.rgba(x, y)[3];
        }
    }
    t
}

// ---------- tips ----------

#[test]
fn round_tip_roundness_and_angle() {
    let b = brush();
    let mut d = Dab::round(Point::new(50.0, 50.0), 20.0, 1.0);
    d.roundness = 0.25;
    let r = raster(&b, &d);
    // Wide along x, thin along y.
    assert!(at(&r, 50 + 15, 50) > 0.99);
    assert!(at(&r, 50, 50 + 8) < 0.01);
    assert!(at(&r, 50, 50 + 3) > 0.99);
    // Rotated 90° counter-clockwise: now tall.
    d.angle = std::f32::consts::FRAC_PI_2;
    let r = raster(&b, &d);
    assert!(at(&r, 50, 50 - 15) > 0.99);
    assert!(at(&r, 50 + 8, 50) < 0.01);
    // 45°: the long axis runs up-right (y-down image space: x+, y−).
    d.angle = std::f32::consts::FRAC_PI_4;
    let r = raster(&b, &d);
    assert!(at(&r, 60, 40) > 0.99, "{}", at(&r, 60, 40));
    assert!(at(&r, 60, 60) < 0.01);
}

#[test]
fn hardness_profile_is_monotonic() {
    for h in [0.0f32, 0.5, 1.0] {
        let b = BrushSettings { hardness: h, ..brush() };
        let r = raster(&b, &Dab::round(Point::new(50.0, 50.0), 20.0, 1.0));
        let prof: Vec<f32> = (0..36).map(|i| at(&r, 50 + i, 50)).collect();
        assert!(prof.windows(2).all(|w| w[1] <= w[0] + 1e-6), "h={h} {prof:?}");
        assert!(prof[0] > 0.99);
        // Like Photoshop's, a soft tip's tail paints past the radius (20 px), and every tip stops
        // before 1.7 radii.
        assert_eq!(prof[21] > 0.03, h < 1.0, "h={h} {prof:?}");
        assert_eq!(prof[35], 0.0);
        let mid = at(&r, 60, 50);
        if h == 1.0 {
            assert!(mid > 0.99);
        } else if h == 0.0 {
            assert!(mid > 0.2 && mid < 0.8, "{mid}");
        }
    }
}

#[test]
fn hard_edge_is_sharp() {
    // A hardness of 1 is a steep rim: fully opaque inside the nominal radius, empty just past
    // it, with only the anti-aliasing band in between.
    let b = BrushSettings { hardness: 1.0, ..brush() };
    let r = raster(&b, &Dab::round(Point::new(50.0, 50.0), 20.0, 1.0));
    let prof: Vec<f32> = (0..23).map(|i| at(&r, 50 + i, 50)).collect();
    let band = prof.iter().filter(|&&a| (0.01..0.99).contains(&a)).count();
    assert!(band <= 3, "hardness 1 rim is {band} px wide: {prof:?}");
    assert!(prof[17] > 0.99, "translucent inside the rim: {prof:?}");
    assert_eq!(*prof.last().unwrap(), 0.0);
    // A soft tip spreads the same crossing over many pixels instead.
    let soft = BrushSettings { hardness: 0.0, ..brush() };
    let rs = raster(&soft, &Dab::round(Point::new(50.0, 50.0), 20.0, 1.0));
    let profs: Vec<f32> = (0..22).map(|i| at(&rs, 50 + i, 50)).collect();
    let soft_band = profs.iter().filter(|&&a| (0.01..0.99).contains(&a)).count();
    assert!(soft_band >= 10, "soft rim is only {soft_band} px: {profs:?}");
}

#[test]
fn sampled_tip_is_scaled_and_oriented() {
    // Tip: left half painted.
    let tip = GrayTile::from_fn(16, 16, |x, _| if x < 8 { 1.0 } else { 0.0 });
    let b = BrushSettings { tip: TipShape::Sampled(tip), ..brush() };
    let d = Dab::round(Point::new(50.0, 50.0), 20.0, 1.0);
    let r = raster(&b, &d);
    assert!(at(&r, 40, 50) > 0.99 && at(&r, 60, 50) < 0.01);
    // Flip X mirrors it.
    let r = raster(&b, &Dab { flip_x: true, ..d });
    assert!(at(&r, 40, 50) < 0.01 && at(&r, 60, 50) > 0.99);
    // Rotated 90° CCW: the painted (left) half moves to the bottom.
    let r = raster(&b, &Dab { angle: std::f32::consts::FRAC_PI_2, ..d });
    assert!(at(&r, 50, 60) > 0.99 && at(&r, 50, 40) < 0.01);
    // Heavily downscaled uses mips: still sensible coverage.
    let r = raster(&b, &Dab::round(Point::new(50.0, 50.0), 2.0, 1.0));
    assert!(r.1.iter().sum::<f32>() > 1.0);
}

#[test]
fn pencil_is_aliased() {
    let b = BrushSettings { aliased: true, hardness: 0.0, size: 9.0, ..brush() };
    let s = paint(&b, &line(10.0, 50.0, 20.5), 64, 40);
    for y in 10..32 {
        for x in 0..64 {
            let a = s.rgba(x, y)[3];
            assert!(a == 0.0 || a == 1.0, "({x},{y}) = {a}");
        }
    }
    assert_eq!(s.rgba(30, 20)[3], 1.0);
    // Anti-aliased brush has partial pixels.
    let s = paint(&BrushSettings { aliased: false, ..b }, &line(10.0, 50.0, 20.3), 64, 40);
    assert!((10..32).any(|y| (0..64).any(|x| (0.01..0.99).contains(&s.rgba(x, y)[3]))));
}

#[test]
fn pencil_dabs_sit_on_the_pixel_grid() {
    use crate::render::{grid_center, grid_square};
    // Odd sizes centre on the pixel holding the point, even ones on the nearest corner.
    assert_eq!(grid_center(10.0, 5.0, 1.0), (10.5, 5.5));
    assert_eq!(grid_center(10.9, 5.2, 3.0), (10.5, 5.5));
    assert_eq!(grid_center(10.4, 5.6, 2.0), (10.0, 6.0));
    assert_eq!(grid_square(10.2, 5.7, 1.0), [10.0, 5.0, 11.0, 6.0]);
    assert_eq!(grid_square(10.2, 5.7, 4.0), [8.0, 4.0, 12.0, 8.0]);
    assert_eq!(grid_square(f64::NAN, 0.0, f32::NAN).len(), 4);
    // A 1 px pencil paints exactly the pixel under each point, even on a pixel corner, with
    // no partial alpha.
    let b = BrushSettings { aliased: true, hardness: 1.0, size: 1.0, ..brush() };
    let s = paint(&b, &[StrokePoint::new(7.0, 4.0, 1.0)], 16, 16);
    for y in 0..16 {
        for x in 0..16 {
            assert_eq!(s.rgba(x, y)[3], if (x, y) == (7, 4) { 1.0 } else { 0.0 }, "({x},{y})");
        }
    }
    // A horizontal 1 px line is one pixel thick and unbroken.
    let s = paint(&b, &line(2.0, 20.0, 9.3), 24, 16);
    for x in 2..20 {
        assert_eq!(s.rgba(x, 9)[3], 1.0, "x {x}");
        assert_eq!(s.rgba(x, 8)[3] + s.rgba(x, 10)[3], 0.0, "x {x}");
    }
    // A 4 px dab stays inside its cursor square.
    let b = BrushSettings { size: 4.0, ..b };
    let s = paint(&b, &[StrokePoint::new(10.2, 5.7, 1.0)], 24, 16);
    let [x0, y0, x1, y1] = grid_square(10.2, 5.7, 4.0);
    for y in 0..16 {
        for x in 0..24 {
            let inside = (x as f64) >= x0 && (x as f64) < x1 && (y as f64) >= y0 && (y as f64) < y1;
            assert!(inside || s.rgba(x, y)[3] == 0.0, "({x},{y}) outside the square is painted");
        }
    }
    assert_eq!(s.rgba(9, 5)[3], 1.0);
}

// ---------- spacing ----------

#[test]
fn spacing_sets_dab_count() {
    for (spacing, len, expect) in [(0.25f32, 100.0, 21usize), (1.0, 100.0, 6), (0.5, 200.0, 21)] {
        let b = BrushSettings { spacing, size: 20.0, ..brush() };
        assert_eq!(dabs_of(&b, &line(0.0, len, 0.0)).len(), expect, "spacing {spacing}");
    }
    // Spacing carries across segments: a polyline gives the same count as the straight line.
    let b = BrushSettings { spacing: 0.5, size: 20.0, ..brush() };
    let pts: Vec<StrokePoint> = (0..=20).map(|i| StrokePoint::new(i as f64 * 10.0, 0.0, 1.0)).collect();
    assert_eq!(dabs_of(&b, &pts).len(), 21);
    let d = dabs_of(&b, &pts);
    assert!(d.windows(2).all(|w| ((w[1].center.x - w[0].center.x) - 10.0).abs() < 1e-6));
}

#[test]
fn stroke_has_no_seams() {
    // Overlapping dabs must fuse into one solid band: a seam is a periodic gap or a darker
    // join, so the minimum coverage inside the stroke stays at the opacity.
    for spacing in [0.1f32, 0.25, 0.5] {
        let b = BrushSettings { spacing, size: 20.0, ..brush() };
        let s = paint(&b, &line(20.0, 180.0, 100.5), 200, 200);
        for y in 98..=102 {
            for x in 40..=160 {
                let a = s.rgba(x, y)[3];
                assert!(a > 0.99, "spacing {spacing} seam at ({x},{y}): {a}");
            }
        }
    }
}

// ---------- dynamics ----------

#[test]
fn pressure_controls_size_with_minimum() {
    let b = BrushSettings {
        size: 40.0,
        shape_dynamics: ShapeDynamics {
            enabled: true,
            size: Dynamic { control: Control::PenPressure, minimum: 0.25, ..Default::default() },
            ..Default::default()
        },
        ..brush()
    };
    let pts = vec![StrokePoint::new(0.0, 0.0, 0.0), StrokePoint::new(200.0, 0.0, 1.0)];
    let d = dabs_of(&b, &pts);
    assert!((d[0].radius - 5.0).abs() < 1e-3, "min diameter 25 % of 40 → r 5, got {}", d[0].radius);
    assert!((d.last().unwrap().radius - 20.0).abs() < 0.5);
    assert!(d.windows(2).all(|w| w[1].radius >= w[0].radius - 1e-4));
}

#[test]
fn pressure_curves_are_independent_and_apply_before_the_minimum() {
    let dynamic =
        |output| Dynamic { pressure_curve: Some(PressureCurve::new(vec![[0.0, output], [1.0, output]]).unwrap()), ..Dynamic::controlled(Control::PenPressure) };
    let b = BrushSettings {
        size: 40.0,
        flow: 0.8,
        pressure_size: true,
        shape_dynamics: ShapeDynamics { enabled: true, size: Dynamic { minimum: 0.25, ..dynamic(0.2) }, ..Default::default() },
        transfer: Transfer { enabled: true, opacity: dynamic(0.3), flow: dynamic(0.5), ..Default::default() },
        texture: Texture { enabled: true, each_tip: true, depth: 0.8, depth_jitter: dynamic(0.75), ..Default::default() },
        ..brush()
    };
    for pressure in [0.0, 0.5, 1.0] {
        let dabs = dabs_of(&b, &[StrokePoint::new(20.0, 20.0, pressure)]);
        let dab = &dabs[0];
        assert!((dab.radius - 8.0).abs() < 1e-5, "size = 40 * (0.25 + 0.75 * 0.2)");
        assert!((dab.opacity - 0.3).abs() < 1e-5);
        assert!((dab.alpha - 0.4).abs() < 1e-5);
        assert!((dab.depth - 0.6).abs() < 1e-5);
    }
    // A pressure response must not reshape other input sources.
    let mut controlled = b.clone();
    controlled.shape_dynamics.size.control = Control::Fade;
    let with_curve = dabs_of(&controlled, &line(0.0, 100.0, 20.0));
    controlled.shape_dynamics.size.pressure_curve = None;
    let without_curve = dabs_of(&controlled, &line(0.0, 100.0, 20.0));
    assert_eq!(with_curve, without_curve);
}

#[test]
fn pressure_curve_settings_round_trip_and_old_brushes_keep_linear_pressure() {
    let old: BrushSettings = serde_json::from_str(r#"{"shapeDynamics":{"enabled":true,"size":{"control":"penPressure"}}}"#).unwrap();
    assert!(old.shape_dynamics.size.pressure_curve.is_none());
    let json = serde_json::to_value(&old).unwrap();
    assert!(json["shapeDynamics"]["size"].get("pressureCurve").is_none());
    let mut changed = old.clone();
    changed.shape_dynamics.size.pressure_curve = Some(PressureCurve::new(vec![[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]]).unwrap());
    let back: BrushSettings = serde_json::from_value(serde_json::to_value(&changed).unwrap()).unwrap();
    assert_eq!(back, changed);
    assert!(serde_json::from_str::<BrushSettings>(r#"{"shapeDynamics":{"size":{"pressureCurve":[[0,0],[0,1]]}}}"#).is_err());
    let pressure = [StrokePoint::new(20.0, 20.0, 0.5)];
    assert!((dabs_of(&old, &pressure)[0].radius - old.size * 0.25).abs() < 1e-5);
    assert!((dabs_of(&changed, &pressure)[0].radius - changed.size * 0.1875).abs() < 1e-5);
}

#[test]
fn independent_pressure_curves_render_identically_in_chunks_at_every_depth() {
    let dynamic = |points| Dynamic { pressure_curve: Some(PressureCurve::new(points).unwrap()), ..Dynamic::controlled(Control::PenPressure) };
    let b = BrushSettings {
        size: 16.0,
        shape_dynamics: ShapeDynamics { enabled: true, size: dynamic(vec![[0.0, 0.1], [0.5, 0.25], [1.0, 1.0]]), ..Default::default() },
        transfer: Transfer {
            enabled: true,
            opacity: dynamic(vec![[0.0, 0.2], [0.5, 0.75], [1.0, 1.0]]),
            flow: dynamic(vec![[0.0, 0.05], [0.5, 0.4], [1.0, 0.9]]),
            ..Default::default()
        },
        texture: Texture {
            enabled: true,
            each_tip: true,
            pattern: Pattern::Tile(GrayTile::from_fn(4, 4, |x, y| if x == y { 0.25 } else { 0.75 })),
            depth_jitter: dynamic(vec![[0.0, 1.0], [0.5, 0.25], [1.0, 0.75]]),
            ..Default::default()
        },
        ..brush()
    };
    let points: Vec<_> =
        [(10.0, 0.1), (22.0, 0.5), (34.0, 0.9), (46.0, 0.3), (58.0, 0.0)].into_iter().map(|(x, pressure)| StrokePoint::new(x, 20.0, pressure)).collect();
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let format = PixelFormat::RGBA8.with_sample(sample);
        let original = Surface::new(format);
        let mut one_shot = original.clone();
        render_stroke(&mut one_shot, &b, &points, None, false, 1.0);
        assert!(alpha_sum(&one_shot, Rect::new(0, 0, 70, 40)) > 0.0);
        for chunk in [1, 2, points.len()] {
            let mut incremental = original.clone();
            let mut renderer = StrokeRenderer::new(&b, Some(format), 1.0);
            for part in points.chunks(chunk) {
                renderer.push(part);
                renderer.composite(&original, &mut incremental, None, false, false);
            }
            renderer.finish();
            renderer.composite(&original, &mut incremental, None, false, false);
            for y in 0..40 {
                for x in 0..70 {
                    assert_eq!(one_shot.pixel(x, y), incremental.pixel(x, y), "{format:?} chunk {chunk} at ({x}, {y})");
                }
            }
        }
    }
}

#[test]
fn fade_ramps_over_steps() {
    let b = BrushSettings {
        size: 20.0,
        spacing: 0.5,
        shape_dynamics: ShapeDynamics {
            enabled: true,
            size: Dynamic { control: Control::Fade, fade_steps: 10, minimum: 0.0, ..Default::default() },
            ..Default::default()
        },
        ..brush()
    };
    let d = dabs_of(&b, &line(0.0, 300.0, 0.0));
    assert!((d[0].radius - 10.0).abs() < 1e-3);
    assert!(d[5].radius < d[0].radius * 0.6, "{}", d[5].radius);
    assert!(d.iter().skip(10).all(|x| x.radius <= 0.5 + 1e-6));
    // Fade on opacity (Transfer).
    let b = BrushSettings {
        transfer: Transfer { enabled: true, opacity: Dynamic { control: Control::Fade, fade_steps: 4, ..Default::default() }, ..Default::default() },
        ..brush()
    };
    let d = dabs_of(&b, &line(0.0, 100.0, 0.0));
    assert_eq!(d[0].opacity, 1.0);
    assert_eq!(d[4].opacity, 0.0);
}

#[test]
fn tilt_direction_and_rotation_drive_angle() {
    let mk = |c: Control| BrushSettings { shape_dynamics: ShapeDynamics { enabled: true, angle: Dynamic::controlled(c), ..Default::default() }, ..brush() };
    let mut p = StrokePoint::new(0.0, 0.0, 1.0);
    p.tilt_x = 0.0;
    p.tilt_y = -45.0; // pen leaning "up" the screen
    let pts = vec![p, StrokePoint { x: 50.0, ..p }];
    let d = dabs_of(&mk(Control::PenTilt), &pts);
    assert!((d[0].angle.to_degrees() - 90.0).abs() < 1e-3, "{}", d[0].angle.to_degrees());
    // Direction: travelling down the screen = -90° (counter-clockwise positive).
    let pts = vec![StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(0.0, 50.0, 1.0)];
    let d = dabs_of(&mk(Control::Direction), &pts);
    assert!(d.iter().all(|x| (x.angle.to_degrees() + 90.0).abs() < 1e-3));
    // Rotation.
    let mut q = StrokePoint::new(0.0, 0.0, 1.0);
    q.rotation = 30.0;
    let d = dabs_of(&mk(Control::Rotation), &[q, StrokePoint { x: 20.0, ..q }]);
    assert!((d[0].angle.to_degrees() - 30.0).abs() < 1e-3);
    // Pen tilt on size: upright pen full size, flat pen minimum.
    let b = BrushSettings {
        shape_dynamics: ShapeDynamics { enabled: true, size: Dynamic::controlled(Control::PenTilt), ..Default::default() },
        size: 20.0,
        ..brush()
    };
    let mut flat = StrokePoint::new(0.0, 0.0, 1.0);
    flat.tilt_x = 90.0;
    // A tip shrunk to nothing paints nothing (as Photoshop's size Fade to 0 does) …
    assert!(dabs_of(&b, &[flat]).is_empty());
    // … and a minimum diameter keeps it painting.
    let mut b = b;
    b.shape_dynamics.size.minimum = 0.25;
    let d = dabs_of(&b, &[flat]);
    assert!((d[0].radius - 2.5).abs() < 1e-6, "{}", d[0].radius);
}

#[test]
fn jitter_is_deterministic_per_seed() {
    let b = BrushSettings {
        size: 30.0,
        seed: 42,
        shape_dynamics: ShapeDynamics {
            enabled: true,
            size: Dynamic::jitter(1.0),
            angle: Dynamic::jitter(1.0),
            roundness: Dynamic { jitter: 1.0, minimum: 0.2, ..Default::default() },
            flip_x_jitter: true,
            flip_y_jitter: true,
            ..Default::default()
        },
        scattering: Scattering { enabled: true, scatter: Dynamic::jitter(2.0), both_axes: true, count: 3, count_jitter: Dynamic::jitter(1.0) },
        ..brush()
    };
    let pts = line(0.0, 300.0, 50.0);
    let a = dabs_of(&b, &pts);
    assert_eq!(a, dabs_of(&b, &pts));
    let c = dabs_of(&BrushSettings { seed: 43, ..b.clone() }, &pts);
    assert_ne!(a, c);
    // Jitter actually varies things, within their ranges.
    let radii: Vec<f32> = a.iter().map(|d| d.radius).collect();
    assert!(radii.iter().any(|&r| r < 10.0) && radii.iter().all(|&r| r <= 15.0 + 1e-4));
    assert!(a.iter().all(|d| d.roundness >= 0.2 - 1e-6 && d.roundness <= 1.0));
    assert!(a.iter().any(|d| d.flip_x) && a.iter().any(|d| !d.flip_x));
    // Chunked feeding gives identical dabs.
    let mut r = StrokeRenderer::new(&b, None, 1.0).record_dabs();
    let many: Vec<StrokePoint> = (0..=30).map(|i| StrokePoint::new(i as f64 * 10.0, 50.0, 1.0)).collect();
    for ch in many.chunks(4) {
        r.push(ch);
    }
    r.finish();
    assert_eq!(r.dabs(), dabs_of(&b, &many).as_slice());
}

#[test]
fn scattering_stays_in_bounds() {
    for both in [false, true] {
        let b = BrushSettings {
            size: 20.0,
            scattering: Scattering { enabled: true, scatter: Dynamic::jitter(2.0), both_axes: both, count: 4, ..Default::default() },
            ..brush()
        };
        let d = dabs_of(&b, &line(0.0, 400.0, 100.0));
        // count 4 per spacing step (5 px) → 81 steps.
        assert_eq!(d.len(), 81 * 4);
        let max_off = 2.0 * 10.0;
        assert!(d.iter().all(|x| (x.center.y - 100.0).abs() <= max_off + 1e-6));
        assert!(d.iter().any(|x| (x.center.y - 100.0).abs() > max_off * 0.5), "scatter too small");
        if !both {
            // Perpendicular only: x stays on the spacing grid.
            assert!(d.iter().all(|x| (x.center.x / 5.0 - (x.center.x / 5.0).round()).abs() < 1e-6));
        } else {
            assert!(d.iter().any(|x| (x.center.x / 5.0 - (x.center.x / 5.0).round()).abs() > 0.1));
        }
    }
}

#[test]
fn zero_scatter_keeps_dabs_on_the_stroke() {
    // Scatter at 0 % must not move a dab, even with the section switched on.
    let on = BrushSettings { scattering: Scattering { enabled: true, ..Default::default() }, ..brush() };
    for b in [brush(), on] {
        let d = dabs_of(&b, &line(10.0, 90.0, 40.5));
        assert!(d.len() > 4);
        assert!(
            d.iter().all(|x| (x.center.y - 40.5).abs() < 1e-9),
            "scatter moved dabs off the stroke: {:?}",
            d.iter().map(|x| x.center.y).collect::<Vec<_>>()
        );
    }
}

#[test]
fn color_dynamics_ranges() {
    let base = BrushSettings { color: [1.0, 0.0, 0.0, 1.0], background: [0.0, 0.0, 1.0, 1.0], seed: 5, ..brush() };
    // FG/BG jitter mixes between the two colours only.
    let b = BrushSettings { color_dynamics: ColorDynamics { enabled: true, fg_bg: Dynamic::jitter(1.0), ..Default::default() }, ..base.clone() };
    let d = dabs_of(&b, &line(0.0, 200.0, 0.0));
    assert!(d.iter().all(|x| x.color[1].abs() < 1e-5 && (x.color[0] + x.color[2] - 1.0).abs() < 1e-3 || x.color[0].max(x.color[2]) >= 0.999));
    assert!(d.iter().any(|x| x.color[2] > 0.5) && d.iter().any(|x| x.color[0] > 0.5));
    // Hue jitter keeps saturation/brightness of a pure colour.
    let b = BrushSettings { color_dynamics: ColorDynamics { enabled: true, hue_jitter: 1.0, ..Default::default() }, ..base.clone() };
    let d = dabs_of(&b, &line(0.0, 200.0, 0.0));
    for x in &d {
        let mx = x.color[0].max(x.color[1]).max(x.color[2]);
        let mn = x.color[0].min(x.color[1]).min(x.color[2]);
        assert!((mx - 1.0).abs() < 1e-4 && mn.abs() < 1e-4, "{:?}", x.color);
    }
    assert!(d.iter().any(|x| x.color[1] > 0.5 || x.color[2] > 0.5));
    // Purity -1 desaturates completely.
    let b = BrushSettings { color_dynamics: ColorDynamics { enabled: true, purity: -1.0, ..Default::default() }, ..base.clone() };
    let d = dabs_of(&b, &line(0.0, 20.0, 0.0));
    assert!(d.iter().all(|x| (x.color[0] - x.color[1]).abs() < 1e-5 && (x.color[1] - x.color[2]).abs() < 1e-5));
    // Per-stroke: one colour for all dabs.
    let b = BrushSettings { color_dynamics: ColorDynamics { enabled: true, per_tip: false, hue_jitter: 1.0, ..Default::default() }, ..base.clone() };
    let d = dabs_of(&b, &line(0.0, 200.0, 0.0));
    assert!(d.windows(2).all(|w| w[0].color == w[1].color));
    // Per-tip colours land in the pixels.
    let b =
        BrushSettings { size: 10.0, spacing: 3.0, color_dynamics: ColorDynamics { enabled: true, fg_bg: Dynamic::jitter(1.0), ..Default::default() }, ..base };
    let s = paint(&b, &line(10.0, 310.0, 20.0), 330, 40);
    let colours: Vec<[f32; 4]> = (0..11).map(|i| s.rgba(10 + i * 30, 20)).collect();
    assert!(colours.iter().any(|c| c[2] > 0.6) && colours.iter().any(|c| c[0] > 0.6), "{colours:?}");
}

#[test]
fn transfer_opacity_caps_and_flow_builds() {
    // Pressure on opacity caps coverage.
    let b =
        BrushSettings { transfer: Transfer { enabled: true, opacity: Dynamic::controlled(Control::PenPressure), ..Default::default() }, size: 10.0, ..brush() };
    let pts = vec![StrokePoint::new(10.0, 10.0, 0.4), StrokePoint::new(90.0, 10.0, 0.4)];
    let s = paint(&b, &pts, 100, 20);
    assert!((s.rgba(50, 10)[3] - 0.4).abs() < 0.01, "{}", s.rgba(50, 10)[3]);
    // Low flow builds up with overlap but never past the opacity.
    let b = BrushSettings { flow: 0.1, opacity: 0.8, size: 10.0, ..brush() };
    let one = paint(&b, &[StrokePoint::new(50.0, 10.0, 1.0)], 100, 20).rgba(50, 10)[3];
    let many = paint(&b, &line(40.0, 60.0, 10.0), 100, 20).rgba(50, 10)[3];
    assert!((one - 0.08).abs() < 0.01, "{one}");
    assert!(many > one * 3.0 && many <= 0.8 + 1e-4, "{many}");
    // Flow jitter varies the dab alpha deterministically.
    let b = BrushSettings { transfer: Transfer { enabled: true, flow: Dynamic::jitter(1.0), ..Default::default() }, ..brush() };
    let d = dabs_of(&b, &line(0.0, 100.0, 0.0));
    assert!(d.iter().any(|x| x.alpha < 0.5) && d.iter().all(|x| x.alpha <= 1.0));
}

#[test]
fn build_up_accumulates_over_time() {
    let b = BrushSettings { build_up: true, build_up_rate: 20.0, flow: 0.05, size: 20.0, hardness: 1.0, ..brush() };
    let hold = |ms: f64| {
        let mut a = StrokePoint::new(50.0, 50.0, 1.0);
        let mut z = a;
        a.time = 0.0;
        z.time = ms;
        let s = paint(&b, &[a, z], 100, 100);
        (dabs_of(&b, &[a, z]).len(), s.rgba(50, 50)[3])
    };
    let (n0, a0) = hold(0.0);
    let (n1, a1) = hold(500.0);
    let (n2, a2) = hold(2000.0);
    assert_eq!((n0, n1, n2), (1, 11, 41));
    assert!(a0 < a1 && a1 < a2, "{a0} {a1} {a2}");
    // Without build-up, lingering adds nothing.
    let nb = BrushSettings { build_up: false, ..b };
    let mut a = StrokePoint::new(50.0, 50.0, 1.0);
    let z = StrokePoint { time: 2000.0, ..a };
    a.time = 0.0;
    assert_eq!(dabs_of(&nb, &[a, z]).len(), 1);
}

#[test]
fn stationary_pressure_and_pose_changes_emit_without_motion() {
    let b = BrushSettings {
        size: 40.0,
        pressure_size: true,
        roundness: 0.3,
        shape_dynamics: ShapeDynamics { enabled: true, angle: Dynamic::controlled(Control::Rotation), ..Default::default() },
        ..brush()
    };
    let a = StrokePoint::new(50.0, 50.0, 0.1);
    let p = StrokePoint { pressure: 0.6, time: 10.0, ..a };
    let r = StrokePoint { rotation: 90.0, time: 20.0, ..p };
    let hold = StrokePoint { time: 5000.0, ..r };
    let d = dabs_of(&b, &[a, p, r, hold]);
    assert_eq!(d.len(), 3, "the unchanged hold must not accumulate ink");
    assert_eq!(d[0].radius, 2.0);
    assert_eq!(d[1].radius, 12.0);
    assert_eq!(d[2].radius, 12.0);
    assert!((d[2].angle - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    assert!(d.iter().all(|dab| dab.center == Point::new(50.0, 50.0)));
}

#[test]
fn stationary_pressure_growth_is_live_and_preserves_paint_at_every_depth() {
    let b = BrushSettings { size: 40.0, pressure_size: true, hardness: 1.0, flow: 0.3, ..brush() };
    let a = StrokePoint::new(50.0, 50.0, 0.1);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let fmt = PixelFormat::new(mode, sample, true);
            let pre = Surface::new(fmt);
            let mut target = pre.clone();
            let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0);
            r.push(&[a, StrokePoint { time: 10.0, ..a }]);
            r.composite(&pre, &mut target, None, false, false);
            assert!(target.rgba(50, 50)[3] > 0.0);
            assert_eq!(target.rgba(60, 50)[3], 0.0);
            r.push(&[StrokePoint { pressure: 0.8, time: 20.0, ..a }]);
            r.composite(&pre, &mut target, None, false, false);
            let grown = target.rgba(60, 50)[3];
            assert!(grown > 0.0, "pressure must grow the mark before finish: {fmt:?}");
            let center = target.rgba(50, 50)[3];
            r.push(&[StrokePoint { pressure: 0.8, time: 2000.0, ..a }]);
            r.composite(&pre, &mut target, None, false, false);
            assert_eq!(target.rgba(50, 50)[3], center, "holding constant pressure must not build up");
            r.push(&[StrokePoint { pressure: 0.1, time: 2010.0, ..a }]);
            r.composite(&pre, &mut target, None, false, false);
            assert_eq!(target.rgba(60, 50)[3], grown, "releasing pressure must not erase the outer mark");
        }
    }
}

#[test]
fn zero_pressure_size_does_not_leave_a_minimum_dab() {
    let b = BrushSettings { size: 40.0, pressure_size: true, ..brush() };
    let a = StrokePoint::new(50.0, 50.0, 0.0);
    assert!(dabs_of(&b, &[a]).is_empty());
    let d = dabs_of(&b, &[a, StrokePoint { pressure: 0.2, ..a }, a]);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].radius, 4.0);
}

#[test]
fn stationary_airbrush_pressure_does_not_duplicate_time_dabs() {
    let b = BrushSettings { size: 40.0, pressure_size: true, build_up: true, build_up_rate: 20.0, ..brush() };
    let a = StrokePoint::new(50.0, 50.0, 0.1);
    let z = StrokePoint { time: 100.0, pressure: 0.5, ..a };
    let d = dabs_of(&b, &[a, z]);
    assert_eq!(d.len(), 3, "initial dab and two 50 ms airbrush ticks");
    assert_eq!(d.last().unwrap().radius, 10.0);
    let early = StrokePoint { time: 10.0, ..z };
    let d = dabs_of(&b, &[a, early]);
    assert_eq!(d.len(), 2, "pressure must respond before the next airbrush tick");
    assert_eq!(d.last().unwrap().radius, 10.0);
}

#[test]
fn huge_airbrush_time_gap_emits_bounded_catch_up() {
    let b = BrushSettings { build_up: true, build_up_rate: 1000.0, ..brush() };
    let a = StrokePoint { time: 0.0, ..StrokePoint::new(20.0, 20.0, 1.0) };
    let brief_pause = StrokePoint { time: 200.0, ..a };
    assert_eq!(dabs_of(&b, &[a, brief_pause]).len(), 201, "a 200 ms pause should retain its airbrush build-up");

    let z = StrokePoint { time: f64::MAX, ..a };
    let d = dabs_of(&b, &[a, z]);
    assert!(d.len() <= 2, "a huge pause should resume with one dab, not replay its time backlog: {}", d.len());
    assert_eq!(d.last().map(|dab| dab.center), Some(Point::new(20.0, 20.0)));

    let infinite = StrokePoint { time: f64::INFINITY, ..a };
    assert!(dabs_of(&b, &[a, infinite]).len() <= 1);
}

#[test]
fn unbounded_brush_dimensions_are_safe_in_the_public_renderer() {
    let brush = BrushSettings { size: f32::MAX, dual_brush: DualBrush { enabled: true, size: f32::INFINITY, ..Default::default() }, ..brush() };
    let ctx = BrushContext::new(&brush);
    assert_eq!(ctx.brush.size, crate::MAX_BRUSH_SIZE);
    assert_eq!(ctx.brush.dual_brush.size, 0.5);

    let d = dabs_of(&brush, &[StrokePoint::new(10.0, 10.0, 1.0)]);
    assert!(d.iter().all(|dab| dab.radius <= crate::MAX_BRUSH_SIZE / 2.0));
}

#[test]
fn texture_modulates_coverage() {
    let tile = GrayTile::from_fn(8, 8, |x, _| if x < 4 { 1.0 } else { 0.0 });
    let tex = |mode: MaskMode, depth: f32, each_tip: bool| BrushSettings {
        size: 40.0,
        texture: Texture { enabled: true, pattern: Pattern::Tile(tile.clone()), mode, depth, each_tip, ..Default::default() },
        ..brush()
    };
    for each_tip in [false, true] {
        let s = paint(&tex(MaskMode::Multiply, 1.0, each_tip), &line(20.0, 80.0, 30.0), 100, 60);
        assert!(s.rgba(46, 30)[3] < 0.1, "dark texel blocks paint");
        assert!(s.rgba(41, 30)[3] > 0.9, "light texel passes");
        let half = paint(&tex(MaskMode::Multiply, 0.5, each_tip), &line(20.0, 80.0, 30.0), 100, 60);
        let v = half.rgba(46, 30)[3];
        if each_tip {
            // Per-tip texturing builds up across overlapping dabs.
            assert!(v > 0.5 && v < 1.0, "{v}");
        } else {
            assert!((v - 0.5).abs() < 0.02, "{v}");
        }
    }
    // Brightness and invert.
    let mut b = tex(MaskMode::Multiply, 1.0, false);
    b.texture.invert = true;
    let s = paint(&b, &line(20.0, 80.0, 30.0), 100, 60);
    assert!(s.rgba(46, 30)[3] > 0.9 && s.rgba(41, 30)[3] < 0.1);
    b.texture.invert = false;
    b.texture.brightness = 1.0;
    let s = paint(&b, &line(20.0, 80.0, 30.0), 100, 60);
    assert!(s.rgba(46, 30)[3] > 0.9);
    // All modes keep zero coverage at zero and stay in range.
    for m in [
        MaskMode::Multiply,
        MaskMode::Subtract,
        MaskMode::Darken,
        MaskMode::Overlay,
        MaskMode::ColorDodge,
        MaskMode::ColorBurn,
        MaskMode::LinearBurn,
        MaskMode::HardMix,
        MaskMode::LinearHeight,
        MaskMode::Height,
    ] {
        for t in [0.0, 0.3, 1.0] {
            assert_eq!(mask_combine(m, 0.0, t, 1.0), 0.0, "{m:?}");
            let v = mask_combine(m, 0.7, t, 0.8);
            assert!((0.0..=1.0).contains(&v), "{m:?}");
        }
        // Full-white texture never reduces multiply-like masks below the brush.
        assert!(mask_combine(m, 0.6, 1.0, 1.0) >= 0.6 - 1e-5 || matches!(m, MaskMode::Overlay), "{m:?}");
    }
    // Depth jitter per tip is deterministic and within [min depth, depth].
    let mut b = tex(MaskMode::Multiply, 0.8, true);
    b.texture.depth_jitter = Dynamic { jitter: 1.0, minimum: 0.25, ..Default::default() };
    let d = dabs_of(&b, &line(0.0, 200.0, 0.0));
    assert!(d.iter().all(|x| x.depth <= 0.8 + 1e-6 && x.depth >= 0.8 * 0.25 - 1e-6));
    assert!(d.iter().any(|x| x.depth < 0.6));
}

#[test]
fn csp_texture_rendering_and_pressure_curves_are_chunk_invariant_at_all_depths() {
    let response = PressureCurve::new(vec![[0.0, 0.1], [0.5, 0.8], [1.0, 1.0]]).unwrap();
    let points = [StrokePoint::new(10.0, 20.0, 0.2), StrokePoint::new(28.0, 20.0, 0.8), StrokePoint::new(46.0, 20.0, 0.4)];
    for mode in [
        CspTextureMode::Normal,
        CspTextureMode::Multiply,
        CspTextureMode::Subtract,
        CspTextureMode::Compare,
        CspTextureMode::Outline,
        CspTextureMode::Overlay,
        CspTextureMode::ColorDodge,
        CspTextureMode::ColorBurn,
        CspTextureMode::HardMix,
        CspTextureMode::Height,
    ] {
        for each_tip in [false, true] {
            let b = BrushSettings {
                size: 14.0,
                opacity: 0.7,
                flow: 0.6,
                texture: Texture {
                    enabled: true,
                    csp_mode: Some(mode),
                    each_tip,
                    depth: 0.5,
                    pattern: Pattern::Tile(GrayTile::from_fn(4, 4, |x, y| if x == y { 0.25 } else { 0.75 })),
                    depth_jitter: Dynamic { minimum: 0.2, pressure_curve: Some(response.clone()), ..Dynamic::controlled(Control::PenPressure) },
                    ..Default::default()
                },
                ..brush()
            };
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let format = PixelFormat::RGBA8.with_sample(sample);
                let original = Surface::new(format);
                let mut full = original.clone();
                render_stroke(&mut full, &b, &points, None, false, 1.0);
                let mut incremental = original.clone();
                let mut renderer = StrokeRenderer::new(&b, Some(format), 1.0);
                for point in &points {
                    renderer.push(std::slice::from_ref(point));
                    renderer.composite(&original, &mut incremental, None, false, false);
                }
                renderer.finish();
                renderer.composite(&original, &mut incremental, None, false, false);
                assert!(alpha_sum(&full, Rect::new(0, 0, 60, 40)) > 0.0);
                for y in 0..40 {
                    for x in 0..60 {
                        assert_eq!(full.pixel(x, y), incremental.pixel(x, y), "{mode:?}, {sample:?}, per-tip {each_tip}");
                    }
                }
            }
        }
    }
}

#[test]
fn csp_texture_tone_is_applied_after_bilinear_sampling_and_old_presets_stay_legacy() {
    let old: Texture = serde_json::from_str(r#"{"mode":"multiply","depth":0.5}"#).unwrap();
    assert_eq!(old.csp_mode, None);
    assert!(serde_json::to_value(&old).unwrap().get("cspMode").is_none());
    let b = BrushSettings {
        texture: Texture {
            enabled: true,
            csp_mode: Some(CspTextureMode::Multiply),
            scale: 4.0,
            invert: true,
            contrast: 0.5,
            pattern: Pattern::Tile(GrayTile::from_fn(2, 1, |x, _| if x == 0 { 0.0 } else { 1.0 })),
            ..Default::default()
        },
        ..brush()
    };
    let ctx = BrushContext::new(&b);
    // CSP samples integer destination coordinates: x=1 / scale4 => source .25.
    // Contrasting endpoints first then interpolating would incorrectly give .25.
    assert_eq!(ctx.texture_at(1, 0), 0.0);
}

#[test]
fn csp_texture_minification_keeps_grain_and_wraps_at_integer_phase() {
    let mut b = brush();
    b.texture = Texture {
        enabled: true,
        csp_mode: Some(CspTextureMode::Multiply),
        scale: 0.5,
        invert: true,
        pattern: Pattern::Tile(GrayTile::from_fn(4, 1, |x, _| if x == 1 || x == 2 { 1.0 } else { 0.0 })),
        ..Default::default()
    };
    let ctx = BrushContext::new(&b);
    assert_eq!(ctx.texture_at(0, 0), 0.0);
    assert_eq!(ctx.texture_at(1, 0), 1.0);
    assert_eq!(ctx.texture_at(-1, 0), 1.0);
    assert_eq!(ctx.texture_at(2, 0), 0.0);
    b.texture.scale = 2.0;
    assert_eq!(BrushContext::new(&b).texture_at(1, 0), 0.5);
    b.texture.csp_mode = None;
    b.texture.invert = false;
    assert_eq!(BrushContext::new(&b).texture_at(1, 0), 0.25);
    b.texture.scale = 0.5;
    assert_eq!(BrushContext::new(&b).texture_at(1, 0), 0.5);
}

#[test]
fn csp_texture_mips_drop_odd_edges_and_stop_after_two_levels() {
    let mut b = brush();
    // 9x9 -> 4x4 -> 2x2. The right/bottom odd edge is deliberately white;
    // including it, or selecting a third level, would change these samples.
    b.texture = Texture {
        enabled: true,
        csp_mode: Some(CspTextureMode::Multiply),
        scale: 0.25,
        invert: true,
        pattern: Pattern::Tile(GrayTile::from_fn(9, 9, |x, y| if x >= 8 || y >= 8 || (4..8).contains(&x) { 1.0 } else { 0.0 })),
        ..Default::default()
    };
    let ctx = BrushContext::new(&b);
    assert_eq!(ctx.texture_at(0, 0), 0.0);
    assert_eq!(ctx.texture_at(1, 0), 1.0);
    assert_eq!(ctx.texture_at(2, 0), 0.0);
    assert_eq!(ctx.texture_at(-1, 0), 1.0);
    b.texture.scale = 0.125;
    assert_eq!(BrushContext::new(&b).texture_at(1, 0), 0.0);
}

#[test]
fn csp_native_texture_plane_is_alpha_and_mip_rounding_is_downward() {
    let mut b = brush();
    b.texture = Texture {
        enabled: true,
        csp_mode: Some(CspTextureMode::Multiply),
        scale: 0.5,
        pattern: Pattern::Tile(GrayTile::from_fn(4, 4, |x, y| 1.0 - if (x + y) % 2 == 0 { 2.0 / 255.0 } else { 3.0 / 255.0 })),
        ..Default::default()
    };
    assert!((BrushContext::new(&b).texture_at(0, 0) - 2.0 / 255.0).abs() < 1e-6);
    b.texture.invert = true;
    assert!((BrushContext::new(&b).texture_at(0, 0) - (1.0 - 2.0 / 255.0)).abs() < 1e-6);
}

#[test]
fn csp_small_sampled_particles_keep_alpha_without_pencil_thresholding() {
    let b = BrushSettings {
        size: 2.0,
        csp_anti_alias: Some(0),
        tip: TipShape::Sampled(GrayTile::from_fn(16, 16, |x, y| if (4..12).contains(&x) && (4..12).contains(&y) { 0.4 } else { 0.0 })),
        ..brush()
    };
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        render_stroke(&mut surface, &b, &[StrokePoint::new(20.5, 20.5, 1.0)], None, false, 1.0);
        assert!((surface.rgba(20, 20)[3] - 0.4).abs() < 0.005, "{depth:?}");
    }
    let mut pencil = b.clone();
    pencil.aliased = true;
    assert_eq!(paint(&pencil, &[StrokePoint::new(20.5, 20.5, 1.0)], 40, 40).rgba(20, 20)[3], 0.0);
    let m = crate::tile::Mips::new(&GrayTile::from_fn(2, 2, |x, y| if x == y { 0.3 } else { 0.8 }));
    assert!((m.sample_original_nearest(0.1, 0.1) - 0.3).abs() < 0.0001);
    assert_eq!(m.sample_original_nearest(-0.01, 0.2), 0.0);
    assert_eq!(m.sample_original_nearest(f32::NAN, 0.2), 0.0);
}

#[test]
fn csp_anti_alias_levels_sample_like_csp_and_parse_legacy_flags() {
    // Older presets stored a boolean; levels are 0..=3.
    let parse = |json: &str| serde_json::from_str::<BrushSettings>(json).map(|b| b.csp_anti_alias);
    assert_eq!(parse(r#"{"cspAntiAlias": false}"#).unwrap(), Some(0));
    assert_eq!(parse(r#"{"cspAntiAlias": true}"#).unwrap(), Some(1));
    assert_eq!(parse(r#"{"cspAntiAlias": 3}"#).unwrap(), Some(3));
    assert_eq!(parse(r#"{"cspAntiAlias": null}"#).unwrap(), None);
    for bad in ["4", "1.5", "-1", "\"x\"", "1e40"] {
        assert!(parse(&format!(r#"{{"cspAntiAlias": {bad}}}"#)).is_err(), "{bad}");
    }
    let b = BrushSettings { csp_anti_alias: Some(3), ..brush() };
    assert_eq!(serde_json::from_str::<BrushSettings>(&serde_json::to_string(&b).unwrap()).unwrap(), b);
    assert_eq!(BrushSettings { csp_anti_alias: Some(9), ..brush() }.bounded_for_render().csp_anti_alias, Some(3));
    // A hard round dab, 20 px across, centred on a pixel corner: level 0 is binary; weak
    // averages 2×2 samples (values in quarters); medium and strong soften the edge further.
    let edge = |level: u8| {
        let b = BrushSettings { size: 20.0, hardness: 1.0, pressure_size: false, csp_anti_alias: Some(level), ..brush() };
        let s = paint(&b, &[StrokePoint::new(30.0, 30.0, 1.0)], 60, 60);
        (0..60).map(|x| s.rgba(x, 29)[3]).collect::<Vec<_>>()
    };
    let none = edge(0);
    assert!(none.iter().all(|a| *a == 0.0 || *a == 1.0), "{none:?}");
    assert_eq!(none.iter().filter(|a| **a == 1.0).count(), 20);
    let weak = edge(1);
    assert!(weak.iter().all(|a| (a * 4.0 - (a * 4.0).round()).abs() < 1e-3), "{weak:?}");
    let soft = |row: &[f32]| row.iter().filter(|a| **a > 0.0 && **a < 1.0).count();
    let (medium, strong) = (edge(2), edge(3));
    assert!(soft(&strong) >= soft(&medium) && soft(&medium) >= soft(&weak), "{weak:?} {medium:?} {strong:?}");
    for row in [&weak, &medium, &strong] {
        let total: f32 = row.iter().sum();
        assert!((total - 20.0).abs() < 1.0, "coverage across the dab stays its diameter: {total}");
    }
    // A sampled tip at strong reads a reduced level, so a one-texel checkerboard averages out.
    let checker = BrushSettings {
        size: 8.0,
        pressure_size: false,
        csp_anti_alias: Some(3),
        tip: TipShape::Sampled(GrayTile::from_fn(32, 32, |x, y| if (x + y) % 2 == 0 { 1.0 } else { 0.0 })),
        ..brush()
    };
    let s = paint(&checker, &[StrokePoint::new(20.0, 20.0, 1.0)], 40, 40);
    let a = s.rgba(20, 20)[3];
    assert!((a - 0.5).abs() < 0.05, "{a}");
}

#[test]
fn csp_tip_expansion_rotates_without_clipping_and_round_trips() {
    let base = BrushSettings { size: 10.0, csp_anti_alias: Some(0), tip: TipShape::Sampled(GrayTile::from_fn(10, 10, |_, _| 1.0)), ..brush() };
    assert_eq!(serde_json::from_str::<BrushSettings>("{}").unwrap().csp_tip_scale, None);
    for (scale, angle, probe) in [([1.0, 2.4], 0.0, (30, 40)), ([2.4, 1.0], 0.0, (40, 30)), ([1.0, 2.4], 90.0, (40, 30))] {
        let b = BrushSettings { csp_tip_scale: Some(scale), angle, ..base.clone() };
        let restored: BrushSettings = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(restored.csp_tip_scale, Some(scale));
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            render_stroke(&mut surface, &restored, &[StrokePoint::new(30.5, 30.5, 1.0)], None, false, 1.0);
            assert!(surface.rgba(probe.0, probe.1)[3] > 0.99, "{scale:?} {angle} {depth:?}");
            assert_eq!(surface.rgba(45, 45)[3], 0.0);
        }
    }
    let hostile = BrushSettings { size: 5000.0, csp_tip_scale: Some([f32::NAN, 100.0]), ..base };
    assert_eq!(hostile.bounded_for_render().csp_tip_scale, Some([1.0, 1.0]));
}

#[test]
fn csp_nonlinear_texture_uses_flow_scaled_coverage_and_height_ceiling() {
    for (mode, depth) in [(CspTextureMode::Subtract, 0.5), (CspTextureMode::Height, 0.1)] {
        let b = BrushSettings {
            size: 12.0,
            flow: 0.5,
            hardness: 1.0,
            texture: Texture {
                enabled: true,
                each_tip: true,
                csp_mode: Some(mode),
                depth,
                pattern: Pattern::Tile(GrayTile::from_fn(1, 1, |_, _| 0.5)),
                ..Default::default()
            },
            ..brush()
        };
        let s = paint(&b, &[StrokePoint::new(20.5, 20.5, 1.0)], 40, 40);
        assert!((s.rgba(20, 20)[3] - 0.25).abs() < 0.005, "{mode:?}");
    }
}

#[test]
fn dual_brush_intersects() {
    // Dual tip: small dabs far apart → the stroke survives only where they land.
    let b = BrushSettings { size: 30.0, dual_brush: DualBrush { enabled: true, size: 6.0, spacing: 5.0, hardness: 1.0, ..Default::default() }, ..brush() };
    let s = paint(&b, &line(20.0, 200.0, 30.0), 220, 60);
    // Dual dabs every 30 px starting at x = 20.
    assert!(s.rgba(20, 30)[3] > 0.9 && s.rgba(50, 30)[3] > 0.9);
    assert!(s.rgba(35, 30)[3] < 0.01, "between dual dabs: {}", s.rgba(35, 30)[3]);
    // Outside the primary (y beyond its radius) nothing, even if dual covers.
    assert_eq!(s.rgba(20, 50)[3], 0.0);
    // Without dual, the stroke is continuous.
    let s = paint(&BrushSettings { dual_brush: DualBrush::default(), ..b }, &line(20.0, 200.0, 30.0), 220, 60);
    assert!(s.rgba(35, 30)[3] > 0.9);
}

#[test]
fn wet_edges_and_noise() {
    let b = BrushSettings { size: 40.0, hardness: 0.0, wet_edges: true, ..brush() };
    let s = paint(&b, &line(20.0, 120.0, 50.0), 140, 100);
    let centre = s.rgba(70, 50)[3];
    assert!(centre < 0.6, "wet interior is lighter: {centre}");
    // Overlap doesn't build up past a single dab.
    let single = paint(&b, &[StrokePoint::new(70.0, 50.0, 1.0)], 140, 100).rgba(70, 50)[3];
    assert!((centre - single).abs() < 0.05, "{centre} vs {single}");
    // Rim is darker than the interior for a hard tip.
    let hb = BrushSettings { hardness: 1.0, ..b };
    let d = paint(&hb, &[StrokePoint::new(50.0, 50.0, 1.0)], 100, 100);
    assert!(d.rgba(50 + 18, 50)[3] > d.rgba(50, 50)[3] + 0.2);
    // Noise: soft tip coverage becomes grainy (more pixels near 0/1).
    let soft = BrushSettings { size: 40.0, hardness: 0.0, ..brush() };
    let a = paint(&soft, &[StrokePoint::new(50.0, 50.0, 1.0)], 100, 100);
    let n = paint(&BrushSettings { noise: true, ..soft }, &[StrokePoint::new(50.0, 50.0, 1.0)], 100, 100);
    let extreme = |s: &Surface| {
        (30..70)
            .flat_map(|y| (30..70).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let v = s.rgba(x, y)[3];
                !(0.15..=0.85).contains(&v)
            })
            .count()
    };
    assert!(extreme(&n) > extreme(&a) + 100, "{} vs {}", extreme(&n), extreme(&a));
}

#[test]
fn smoothing_modes() {
    let zig: Vec<StrokePoint> = (0..40).map(|i| StrokePoint::new(i as f64 * 5.0, if i % 2 == 0 { 0.0 } else { 6.0 }, 1.0)).collect();
    let dev = |pts: &[StrokePoint]| pts.iter().map(|p| (p.y - 3.0).abs()).sum::<f64>() / pts.len() as f64;
    // Exponential smoothing reduces the zig-zag.
    let cfg = Smoothing { amount: 0.8, catch_up_on_end: true, ..Default::default() };
    let sm = smooth_points(&cfg, 1.0, &zig);
    assert!(dev(&sm[5..sm.len() - 1]) < dev(&zig) * 0.5);
    // Catch-up on end finishes at the last input point; without it, the stroke ends short.
    assert_eq!(sm.last().map(|p| (p.x, p.y)), zig.last().map(|p| (p.x, p.y)));
    let no = smooth_points(&Smoothing { catch_up_on_end: false, ..cfg.clone() }, 1.0, &zig);
    assert!(no.last().unwrap().x < zig.last().unwrap().x - 1.0);
    // Pulled string: jitter smaller than the string moves nothing; the brush trails by the string.
    let ps = Smoothing { amount: 0.2, pulled_string: true, catch_up_on_end: false, ..Default::default() };
    let jitter: Vec<StrokePoint> = (0..20).map(|i| StrokePoint::new(if i % 2 == 0 { 0.0 } else { 5.0 }, 0.0, 1.0)).collect();
    assert_eq!(smooth_points(&ps, 1.0, &jitter).len(), 1, "within the 20 px string: no motion");
    let far = smooth_points(&ps, 1.0, &[StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(100.0, 0.0, 1.0)]);
    assert!((far.last().unwrap().x - 80.0).abs() < 1e-4);
    // Adjust for zoom: at 200 % zoom the string is half as long in image pixels.
    let far = smooth_points(&ps, 2.0, &[StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(100.0, 0.0, 1.0)]);
    assert!((far.last().unwrap().x - 90.0).abs() < 1e-4);
    // Stroke catch-up: a pause (timestamps) lets the brush converge on the pointer.
    let mut a = StrokePoint::new(0.0, 0.0, 1.0);
    a.time = 0.0;
    let b = StrokePoint { x: 100.0, time: 16.0, ..a };
    let c = StrokePoint { time: 500.0, ..b };
    let mut s = Smoother::new(&Smoothing { amount: 0.8, catch_up: true, catch_up_on_end: false, ..Default::default() }, 1.0);
    let mut out = Vec::new();
    for p in [a, b, c] {
        s.push(p, &mut out);
    }
    assert!(out.last().unwrap().x > 99.0, "{}", out.last().unwrap().x);
    let mut s = Smoother::new(&Smoothing { amount: 0.8, catch_up: false, catch_up_on_end: false, ..Default::default() }, 1.0);
    let mut out = Vec::new();
    for p in [a, b, c] {
        s.push(p, &mut out);
    }
    assert!(out.last().unwrap().x < 70.0, "{}", out.last().unwrap().x);
}

#[test]
fn maximum_smoothing_default_does_not_append_a_releasing_tail() {
    let points: Vec<_> = (0..80)
        .map(|i| StrokePoint { time: i as f64 * 8.0, ..StrokePoint::new(20.0 + i as f64 * 3.0, 30.0, if i < 76 { 0.8 } else { 0.8 - (i - 75) as f32 * 0.19 }) })
        .collect();
    // A stroke with a rounded filtered end must not acquire a long, narrow tail
    // just because it ends. The optional catch-up setting remains available.
    let cfg: Smoothing = serde_json::from_value(serde_json::json!({"amount": 1.0})).unwrap();
    let mut smoother = Smoother::new(&cfg, 1.0);
    let mut sm = Vec::new();
    for p in &points {
        smoother.push(*p, &mut sm);
    }
    let live_end = *sm.last().unwrap();
    assert!(live_end.pressure > 0.6);
    let count = sm.len();
    smoother.finish(&mut sm);
    assert_eq!(sm.len(), count, "default smoothing must stop at the filtered tip instead of extending to the low-pressure raw endpoint");

    let b = BrushSettings { size: 20.0, pressure_size: true, smoothing: cfg.clone(), ..brush() };
    let rounded = dabs_of(&b, &points);
    assert!(rounded.last().unwrap().radius > 6.0);
    let explicit: Smoothing = serde_json::from_value(serde_json::json!({"amount": 1.0, "catchUpOnEnd": true})).unwrap();
    let extended = dabs_of(&BrushSettings { smoothing: explicit, ..b }, &points);
    assert!(extended.last().unwrap().center.x > rounded.last().unwrap().center.x + 40.0);
    assert!(extended.last().unwrap().radius < 1.0, "explicitly enabled catch-up still follows measured ending pressure");
    // Disabling smoothing preserves the raw pressure tail, without a minimum-width clamp.
    assert_eq!(smooth_points(&Smoothing::default(), 1.0, &points), points);
}

#[test]
fn smoothing_keeps_pressure_at_the_smoothed_position() {
    let cfg = Smoothing { amount: 0.5, catch_up: false, catch_up_on_end: false, ..Default::default() };
    let a = StrokePoint { time: 0.0, ..StrokePoint::new(0.0, 0.0, 1.0) };
    let z = StrokePoint { x: 100.0, pressure: 0.0, tilt_x: 60.0, time: 16.0, ..a };
    for pulled_string in [false, true] {
        let cfg = Smoothing { pulled_string, ..cfg.clone() };
        let sm = smooth_points(&cfg, 1.0, &[a, z]);
        let last = sm.last().unwrap();
        assert_eq!(last.x, 50.0);
        assert_eq!(last.pressure, 0.5, "latest zero pressure must not be attached halfway along the stroke");
        assert_eq!(last.tilt_x, 30.0);
        assert_eq!(last.time, 16.0, "airbrush uses the actual event clock");
    }
}

#[test]
fn smoothing_preserves_stationary_pressure_and_airbrush_time() {
    let a = StrokePoint { time: 0.0, ..StrokePoint::new(50.0, 50.0, 0.1) };
    let z = StrokePoint { pressure: 0.7, time: 10.0, ..a };
    for pulled_string in [false, true] {
        let b = BrushSettings {
            size: 40.0,
            pressure_size: true,
            smoothing: Smoothing { amount: 0.8, pulled_string, catch_up_on_end: false, ..Default::default() },
            ..brush()
        };
        let d = dabs_of(&b, &[a, z]);
        assert_eq!(d.len(), 2);
        assert_eq!(d.last().unwrap().radius, 14.0);
        let b = BrushSettings { build_up: true, build_up_rate: 20.0, ..b };
        let hold = StrokePoint { time: 100.0, ..a };
        assert_eq!(dabs_of(&b, &[a, hold]).len(), 3);
    }
}

#[test]
fn pulled_string_does_not_ink_moving_pressure_samples_at_the_frozen_anchor() {
    let b = BrushSettings {
        size: 40.0,
        pressure_size: true,
        smoothing: Smoothing { amount: 0.8, pulled_string: true, catch_up_on_end: false, ..Default::default() },
        ..brush()
    };
    // These raw samples travel inside the 80 px string. Pressure changes must not be
    // mistaken for stationary painting simply because the smoothed brush does not move.
    let points: Vec<_> = (0..10).map(|i| StrokePoint { time: i as f64 * 8.0, ..StrokePoint::new(50.0 + i as f64, 50.0, 0.1 + i as f32 * 0.1) }).collect();
    let d = dabs_of(&b, &points);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].radius, 2.0, "keep the real weak first contact instead of growing an anchored blob");
    let mut held = points;
    held.push(StrokePoint { time: 100.0, ..StrokePoint::new(59.0, 50.0, 0.8) });
    let d = dabs_of(&b, &held);
    assert_eq!(d.len(), 2, "a subsequent real stationary pressure change is still responsive");
    assert_eq!(d[1].radius, 16.0);
}

#[test]
fn pressure_strokes_match_chunking_and_tail_preview_at_every_depth() {
    let pts: Vec<_> = [(20.0, 0.1), (20.0, 0.5), (20.0, 0.5), (32.0, 0.8), (48.0, 0.6), (60.0, 0.3), (72.0, 0.0)]
        .into_iter()
        .enumerate()
        .map(|(i, (x, p))| StrokePoint { time: i as f64 * 8.0, ..StrokePoint::new(x, 30.0, p) })
        .collect();
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for pulled_string in [false, true] {
            let b = BrushSettings { size: 20.0, pressure_size: true, smoothing: Smoothing { amount: 0.2, pulled_string, ..Default::default() }, ..brush() };
            let fmt = PixelFormat::RGBA8.with_sample(sample);
            let pre = Surface::new(fmt);
            let mut one = pre.clone();
            render_stroke(&mut one, &b, &pts, None, false, 1.0);
            for chunk in [1, 2, 4, pts.len()] {
                let mut inc = pre.clone();
                let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0);
                for points in pts.chunks(chunk) {
                    r.push(points);
                    r.composite(&pre, &mut inc, None, false, false);
                }
                let mut preview = inc.clone();
                if let Some(mut tail) = r.tail_preview() {
                    tail.composite(&pre, &mut preview, None, false, false);
                }
                r.finish();
                r.composite(&pre, &mut inc, None, false, false);
                for y in 15..45 {
                    for x in 5..85 {
                        assert_eq!(one.pixel(x, y), inc.pixel(x, y), "chunk={chunk} {fmt:?} ({x},{y})");
                        assert_eq!(preview.pixel(x, y), inc.pixel(x, y), "preview changed on finish: chunk={chunk} {fmt:?} ({x},{y})");
                    }
                }
            }
        }
    }
}

#[test]
fn pose_overrides_stylus_values() {
    let b = BrushSettings {
        size: 40.0,
        pose: Pose { enabled: true, pressure: 0.5, override_pressure: true, ..Default::default() },
        shape_dynamics: ShapeDynamics { enabled: true, size: Dynamic::controlled(Control::PenPressure), ..Default::default() },
        ..brush()
    };
    let d = dabs_of(&b, &line(0.0, 50.0, 0.0));
    assert!(d.iter().all(|x| (x.radius - 10.0).abs() < 1e-4));
}

#[test]
fn chunked_rendering_matches_one_shot() {
    let b = BrushSettings {
        size: 24.0,
        seed: 9,
        shape_dynamics: ShapeDynamics { enabled: true, size: Dynamic::jitter(0.5), angle: Dynamic::jitter(1.0), ..Default::default() },
        scattering: Scattering { enabled: true, scatter: Dynamic::jitter(1.0), both_axes: true, count: 2, ..Default::default() },
        texture: Texture { enabled: true, depth: 0.5, ..Default::default() },
        color_dynamics: ColorDynamics { enabled: true, hue_jitter: 0.5, ..Default::default() },
        color: [1.0, 0.0, 0.0, 1.0],
        ..brush()
    };
    let pts: Vec<StrokePoint> = (0..60).map(|i| StrokePoint::new(20.0 + i as f64 * 4.0, 60.0 + (i as f64 * 0.3).sin() * 30.0, 1.0)).collect();
    let mut one = Surface::new(PixelFormat::RGBA8);
    let d1 = render_stroke(&mut one, &b, &pts, None, false, 1.0);
    let mut inc = Surface::new(PixelFormat::RGBA8);
    let pre = inc.clone();
    let mut r = StrokeRenderer::new(&b, Some(inc.format()), 1.0);
    for ch in pts.chunks(7) {
        r.push(ch);
        r.composite(&pre, &mut inc, None, false, false);
    }
    r.finish();
    r.composite(&pre, &mut inc, None, false, false);
    assert_eq!(d1, r.bounds());
    for y in d1.y0..d1.y1 {
        for x in d1.x0..d1.x1 {
            assert_eq!(one.pixel(x, y), inc.pixel(x, y), "({x},{y})");
        }
    }
}

#[test]
fn paints_every_depth_and_model() {
    let b = BrushSettings { size: 8.0, color: [1.0, 0.0, 0.0, 1.0], ..brush() };
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for alpha in [true, false] {
                let fmt = PixelFormat::new(mode, sample, alpha);
                let mut s = Surface::with_default(fmt, &prism_raster::from_rgba(&fmt, [1.0, 1.0, 1.0, 1.0]));
                let fancy = BrushSettings {
                    shape_dynamics: ShapeDynamics { enabled: true, size: Dynamic::jitter(0.3), ..Default::default() },
                    color_dynamics: ColorDynamics { enabled: true, brightness_jitter: 0.1, ..Default::default() },
                    texture: Texture { enabled: true, depth: 0.2, ..Default::default() },
                    ..b.clone()
                };
                for br in [&b, &fancy] {
                    render_stroke(&mut s, br, &line(5.0, 40.0, 5.0), None, false, 1.0);
                }
                let c = s.rgba(20, 5);
                let expect = prism_raster::to_rgba(&fmt, &prism_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
                assert!((c[0] - expect[0]).abs() < 0.15 && (c[1] - expect[1]).abs() < 0.12, "{fmt:?} {c:?} vs {expect:?}");
                assert_eq!(s.pixel(20, 30), s.pixel(5000, 5000), "{fmt:?} untouched");
            }
        }
    }
}

#[test]
fn blend_mode_and_dissolve() {
    let mut s = Surface::new(PixelFormat::RGBA32F);
    s.fill_rect(Rect::new(0, 0, 100, 20), &[0.5, 0.5, 0.5, 1.0]);
    let b = BrushSettings { size: 10.0, color: [0.5, 0.5, 0.5, 1.0], mode: BlendMode::Multiply, ..brush() };
    render_stroke(&mut s, &b, &line(10.0, 90.0, 10.0), None, false, 1.0);
    assert!((s.rgba(50, 10)[0] - 0.25).abs() < 1e-4);
    let mut s = Surface::new(PixelFormat::RGBA32F);
    let b = BrushSettings { size: 30.0, opacity: 0.5, mode: BlendMode::Dissolve, ..brush() };
    render_stroke(&mut s, &b, &line(20.0, 80.0, 20.0), None, false, 1.0);
    let vals: Vec<f32> = (30..70).map(|x| s.rgba(x, 20)[3]).collect();
    assert!(vals.iter().all(|&v| v == 0.0 || v == 1.0) && vals.contains(&0.0) && vals.contains(&1.0));
}

#[test]
fn presets_render_and_round_trip() {
    let presets = crate::presets::builtin();
    assert!(presets.len() >= 12);
    for p in &presets {
        let j = serde_json::to_string(p).unwrap();
        let back: BrushPreset = serde_json::from_str(&j).unwrap();
        assert_eq!(&back, p, "{}", p.name);
        let mut s = Surface::new(PixelFormat::RGBA8);
        let pts: Vec<StrokePoint> = (0..20).map(|i| StrokePoint { time: i as f64 * 16.0, ..StrokePoint::new(40.0 + i as f64 * 8.0, 60.0, 0.8) }).collect();
        let dmg = render_stroke(&mut s, &p.brush, &pts, None, false, 1.0);
        assert!(!dmg.is_empty(), "{}", p.name);
        assert!(alpha_sum(&s, dmg) > 10.0, "{} painted nothing", p.name);
    }
    // Partial JSON fills defaults.
    let b: BrushSettings = serde_json::from_str(r#"{"size": 50, "scattering": {"enabled": true, "scatter": {"jitter": 2}}, "pressure_size": false}"#).unwrap();
    assert_eq!(b.size, 50.0);
    assert!(b.scattering.enabled && b.scattering.count == 1 && !b.pressure_size);
    assert_eq!(b.hardness, 1.0);
}

#[test]
fn protect_texture_keeps_pattern() {
    let presets = crate::presets::builtin();
    let chalk = crate::presets::find(&presets, "chalk").unwrap().brush.clone();
    let canvas = crate::presets::find(&presets, "Canvas Texture").unwrap().brush.clone();
    let cur = BrushSettings { protect_texture: true, ..chalk.clone() };
    let next = canvas.clone().with_protected_texture(&cur);
    assert_eq!(next.texture.pattern, chalk.texture.pattern);
    assert_eq!(next.texture.mode, canvas.texture.mode);
    assert_eq!(canvas.clone().with_protected_texture(&chalk).texture.pattern, canvas.texture.pattern);
}

#[test]
fn tail_preview_shows_the_stroke_as_finishing_it_would() {
    // Live previews draw the smoothing catch-up tail before the stroke ends: compositing the
    // tail preview over the incremental composite gives exactly the finished stroke.
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U16, true);
    let b = BrushSettings { size: 9.0, smoothing: Smoothing { amount: 0.7, catch_up_on_end: true, ..Default::default() }, ..brush() };
    let pts: Vec<StrokePoint> = (0..12).map(|i| StrokePoint::new(5.0 + i as f64 * 9.0, 20.0 + (i % 3) as f64 * 7.0, 1.0)).collect();
    let pre = Surface::new(fmt);
    let mut live = pre.clone();
    let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0);
    r.push(&pts);
    r.composite(&pre, &mut live, None, false, false);
    let mut tail = r.tail_preview().expect("the brush lags: there is a tail");
    tail.composite(&pre, &mut live, None, false, false);
    let mut done = pre.clone();
    render_stroke(&mut done, &b, &pts, None, false, 1.0);
    assert!((0..60).all(|y| (0..120).all(|x| live.rgba(x, y) == done.rgba(x, y))));
    // Without smoothing (and past the first dab) finishing adds nothing.
    let mut r = StrokeRenderer::new(&brush(), Some(fmt), 1.0);
    r.push(&pts);
    assert!(r.tail_preview().is_none());
}

// ---------- Tilt Scale, Brush Projection, speed spacing, locks, mixer transfer ----------

/// Coverage extent (pixels above half coverage) of a rasterised dab along x and y through its centre.
fn extents(b: &BrushSettings, d: &Dab) -> (usize, usize) {
    let r = raster(b, d);
    let (cx, cy) = (d.center.x.floor() as i32, d.center.y.floor() as i32);
    let w = (r.0.x0..r.0.x1).filter(|&x| at(&r, x, cy) > 0.5).count();
    let h = (r.0.y0..r.0.y1).filter(|&y| at(&r, cx, y) > 0.5).count();
    (w, h)
}

fn pen(x: f64, y: f64, tilt_x: f32, tilt_y: f32, rotation: f32) -> StrokePoint {
    StrokePoint { tilt_x, tilt_y, rotation, ..StrokePoint::new(x, y, 1.0) }
}

#[test]
fn tilt_scale_squashes_the_tip_height_with_pen_tilt() {
    // Size control on Pen Tilt with a 100 % minimum, so only Tilt Scale changes the shape.
    let mk = |scale: f32| BrushSettings {
        size: 40.0,
        shape_dynamics: ShapeDynamics {
            enabled: true,
            size: Dynamic { control: Control::PenTilt, minimum: 1.0, ..Default::default() },
            tilt_scale: scale,
            ..Default::default()
        },
        ..brush()
    };
    let upright = [pen(50.0, 50.0, 0.0, 0.0, 0.0)];
    let tilted = [pen(50.0, 50.0, 45.0, 0.0, 0.0)];
    // 0 % (the default) and an upright pen leave the tip round.
    let d = dabs_of(&mk(0.0), &tilted)[0];
    assert_eq!(d.roundness, 1.0);
    let d = dabs_of(&mk(2.0), &upright)[0];
    assert_eq!(d.roundness, 1.0);
    // 100 % at 45° tilt: height × (1 − 0.5 × 0.5) = 0.75; 200 %: 0.5.
    let d = dabs_of(&mk(1.0), &tilted)[0];
    assert!((d.roundness - 0.75).abs() < 1e-4, "{}", d.roundness);
    let d2 = dabs_of(&mk(2.0), &tilted)[0];
    assert!((d2.roundness - 0.5).abs() < 1e-4, "{}", d2.roundness);
    let (w, h) = extents(&mk(2.0), &d2);
    assert!(w >= 38 && (h as i32 - 20).abs() <= 2, "{w}×{h}");
    // Only with the size control on Pen Tilt (Photoshop greys the slider otherwise).
    let mut b = mk(2.0);
    b.shape_dynamics.size.control = Control::PenPressure;
    assert_eq!(dabs_of(&b, &tilted)[0].roundness, 1.0);
    // Rendered strokes differ.
    let a = paint(&mk(0.0), &[pen(20.0, 40.0, 60.0, 0.0, 0.0), pen(120.0, 40.0, 60.0, 0.0, 0.0)], 0, 0);
    let c = paint(&mk(2.0), &[pen(20.0, 40.0, 60.0, 0.0, 0.0), pen(120.0, 40.0, 60.0, 0.0, 0.0)], 0, 0);
    let area = Rect::new(0, 0, 160, 80);
    assert!(alpha_sum(&c, area) < alpha_sum(&a, area) * 0.75, "{} vs {}", alpha_sum(&c, area), alpha_sum(&a, area));
}

#[test]
fn brush_projection_foreshortens_along_tilt_and_turns_with_rotation() {
    let mk = |on: bool| BrushSettings { size: 40.0, shape_dynamics: ShapeDynamics { enabled: true, brush_projection: on, ..Default::default() }, ..brush() };
    // Tilted 60° towards +x: the tip is foreshortened to cos 60° = ½ along x.
    let pts = [pen(50.0, 50.0, 60.0, 0.0, 0.0)];
    let d = dabs_of(&mk(true), &pts)[0];
    assert!((d.proj_scale - 0.5).abs() < 1e-4 && d.proj_angle.abs() < 1e-5, "{d:?}");
    let (w, h) = extents(&mk(true), &d);
    assert!((w as i32 - 20).abs() <= 2 && h >= 38, "{w}×{h}");
    // Tilted along y instead: foreshortened vertically.
    let d = dabs_of(&mk(true), &[pen(50.0, 50.0, 0.0, 60.0, 0.0)])[0];
    let (w, h) = extents(&mk(true), &d);
    assert!(w >= 38 && (h as i32 - 20).abs() <= 2, "{w}×{h}");
    // Off: the tilt doesn't touch the tip.
    let d = dabs_of(&mk(false), &pts)[0];
    assert_eq!((d.proj_scale, d.angle), (1.0, 0.0));
    assert_eq!(extents(&mk(false), &d), (40, 40));
    // Barrel rotation turns the tip (an elliptical one shows it).
    let mut b = mk(true);
    b.roundness = 0.5;
    let d = dabs_of(&b, &[pen(50.0, 50.0, 0.0, 0.0, 90.0)])[0];
    assert!((d.angle.to_degrees() - 90.0).abs() < 1e-3);
    let (w, h) = extents(&b, &d);
    assert!(w < h, "{w}×{h}");
    // Dual-brush dabs are never projected.
    let ctx = BrushContext::new(&mk(true));
    let dd = Dab { proj_scale: 0.3, ..Dab::round(Point::new(50.0, 50.0), 20.0, 1.0) };
    let (r, mut a, mut c) = (ctx.dab_rect(&dd, true), Vec::new(), Vec::new());
    ctx.rasterize(&dd, true, r, &mut a);
    ctx.rasterize(&Dab { proj_scale: 1.0, ..dd }, true, r, &mut c);
    assert_eq!(a, c);
}

#[test]
fn spacing_off_spaces_dabs_by_pointer_speed() {
    let mut b = BrushSettings { size: 20.0, spacing_enabled: false, ..brush() };
    let timed = |ms: f64| vec![StrokePoint { time: 0.0, ..StrokePoint::new(0.0, 10.0, 1.0) }, StrokePoint { time: ms, ..StrokePoint::new(100.0, 10.0, 1.0) }];
    // 100 px in 400 ms: a dab every 8 ms = every 2 px; in 40 ms: every 20 px.
    let slow = dabs_of(&b, &timed(400.0)).len();
    let fast = dabs_of(&b, &timed(40.0)).len();
    assert_eq!((slow, fast), (51, 6));
    // Never denser than half a pixel, however slow.
    assert!(dabs_of(&b, &timed(1.0e9)).len() <= 201);
    // No timestamps (a ⇧-click line): spaced by distance like Spacing on, not a dab per point.
    let pts = [StrokePoint::new(0.0, 10.0, 1.0), StrokePoint::new(90.0, 10.0, 1.0), StrokePoint::new(100.0, 10.0, 1.0)];
    let checked = BrushSettings { spacing_enabled: true, ..b.clone() };
    assert_eq!(dabs_of(&b, &pts).len(), dabs_of(&checked, &pts).len());
    assert!(dabs_of(&b, &pts).len() > 3);
    // Checked: fixed spacing (25 % of 20 px = 5 px) whatever the speed.
    b.spacing_enabled = true;
    assert_eq!(dabs_of(&b, &timed(400.0)).len(), dabs_of(&b, &timed(40.0)).len());
    // Chunked input gives the same dabs.
    b.spacing_enabled = false;
    let pts: Vec<StrokePoint> = (0..20).map(|i| StrokePoint { time: f64::from(i) * 13.0, ..StrokePoint::new(f64::from(i) * 7.0, 10.0, 1.0) }).collect();
    let one = dabs_of(&b, &pts);
    let mut g = crate::dynamics::DabGenerator::new(&b, 1.0);
    let (mut out, mut dual) = (Vec::new(), Vec::new());
    for c in pts.chunks(3) {
        g.push(c, &mut out, &mut dual);
    }
    g.finish(&mut out, &mut dual);
    assert_eq!(one, out);
}

#[test]
fn locked_sections_survive_picking_a_preset() {
    let presets = crate::presets::builtin();
    let chalk = crate::presets::find(&presets, "chalk").unwrap().brush.clone();
    let mut cur = BrushSettings {
        scattering: Scattering { enabled: true, count: 5, ..Default::default() },
        transfer: Transfer { enabled: true, wetness: Dynamic::jitter(0.4), ..Default::default() },
        noise: true,
        ..brush()
    };
    // Unlocked: the preset's sections win.
    let next = chalk.clone().picked_over(&cur);
    assert_eq!(next.scattering, chalk.scattering);
    assert_eq!(next.noise, chalk.noise);
    // Locked: the current sections stay, and so do the locks.
    cur.locks = SectionLocks { scattering: true, transfer: true, noise: true, ..Default::default() };
    let next = chalk.clone().picked_over(&cur);
    assert_eq!(next.scattering, cur.scattering);
    assert_eq!(next.transfer, cur.transfer);
    assert!(next.noise);
    assert_eq!(next.locks, cur.locks);
    // Unlocked sections still come from the preset.
    assert_eq!(next.texture, chalk.texture);
    assert_eq!(next.tip, chalk.tip);
}

#[test]
fn transfer_wetness_and_mix_vary_per_dab() {
    let b = BrushSettings {
        transfer: Transfer {
            enabled: true,
            wetness: Dynamic::controlled(Control::PenPressure),
            mix: Dynamic { jitter: 1.0, minimum: 0.25, ..Default::default() },
            ..Default::default()
        },
        ..brush()
    };
    let pts = [StrokePoint::new(0.0, 0.0, 0.3), StrokePoint::new(60.0, 0.0, 0.3)];
    let d = dabs_of(&b, &pts);
    assert!(d.iter().all(|x| (x.wet - 0.3).abs() < 1e-5));
    assert!(d.iter().all(|x| (0.25..=1.0).contains(&x.mix)) && d.iter().any(|x| x.mix < 0.9));
    // Transfer off: as set.
    let d = dabs_of(&brush(), &pts);
    assert!(d.iter().all(|x| x.wet == 1.0 && x.mix == 1.0));
    // New fields round-trip and old JSON loads with neutral values.
    let full = BrushSettings {
        spacing_enabled: false,
        shape_dynamics: ShapeDynamics { tilt_scale: 1.5, brush_projection: true, ..Default::default() },
        locks: SectionLocks { texture: true, ..Default::default() },
        mixer: crate::mixer::MixerSettings { wet: 0.9, sample_all_layers: true, ..Default::default() },
        ..b
    };
    let back: BrushSettings = serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
    assert_eq!(back, full);
    let old: BrushSettings = serde_json::from_str(r#"{"size": 12, "shapeDynamics": {"enabled": true}, "transfer": {"enabled": true}}"#).unwrap();
    assert!(old.spacing_enabled && !old.shape_dynamics.brush_projection && old.shape_dynamics.tilt_scale == 0.0 && old.locks == SectionLocks::default());
    assert_eq!(old.mixer, crate::mixer::MixerSettings::default());
}

/// Primary and Dual Brush dabs of a stroke (the generator, as the renderer drives it).
fn all_dabs(b: &BrushSettings, pts: &[StrokePoint]) -> (Vec<Dab>, Vec<Dab>) {
    let mut g = crate::dynamics::DabGenerator::new(b, 1.0);
    let (mut dabs, mut duals) = (Vec::new(), Vec::new());
    g.push(pts, &mut dabs, &mut duals);
    g.finish(&mut dabs, &mut duals);
    (dabs, duals)
}

/// A Scatter amount far beyond the 1000 % maximum (or non-finite), on the primary tip or the
/// Dual Brush, used to push dab centres to infinity and overflow the dab rectangle (#977). The
/// renderer caps it at the maximum, so dabs stay finite and within reach, and painting works at
/// every bit depth.
#[test]
fn huge_scatter_is_capped_and_renders_at_every_depth() {
    let pts = line(10.0, 40.0, 10.0);
    let with = |j: f32| BrushSettings {
        size: 20.0,
        scattering: Scattering { enabled: true, scatter: Dynamic::jitter(j), both_axes: true, count: 4, ..Default::default() },
        dual_brush: DualBrush { enabled: true, size: 6.0, scatter: j, both_axes: true, ..Default::default() },
        ..brush()
    };
    let capped = all_dabs(&with(MAX_SCATTER), &pts);
    // Primary reach: MAX_SCATTER radii of 10 px; dual reach: MAX_SCATTER radii of 3 px.
    let near = |ds: &[Dab], r: f64| ds.iter().all(|d| (d.center.x - 25.0).abs() <= 15.0 + r && (d.center.y - 10.0).abs() <= r);
    assert!(near(&capped.0, f64::from(MAX_SCATTER) * 10.0) && near(&capped.1, f64::from(MAX_SCATTER) * 3.0));
    for j in [MAX_SCATTER + 0.01, 1e6, 1e38, f32::MAX, f32::INFINITY] {
        let b = with(j);
        assert_eq!(all_dabs(&b, &pts), capped, "scatter {j} paints like the maximum");
        let ctx = BrushContext::new(&b);
        // Radius 10 plus the hard edge's tail, plus a pixel of slack on each side.
        let side = 2 * ((10.0 + crate::Falloff::new(1.0, 10.0).reach_past_edge()).ceil() as u32 + 1) + 1;
        for d in &capped.0 {
            let r = ctx.dab_rect(d, false);
            assert!(!r.is_empty() && r.width() <= side && r.height() <= side, "{r:?}");
        }
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
            let damage = render_stroke(&mut s, &b, &pts, None, false, 1.0);
            assert!(!damage.is_empty() && damage.x0 > -200 && damage.x1 < 250, "scatter {j} at {sample:?}: {damage:?}");
        }
    }
    // A NaN amount scatters nothing, as a zero amount does.
    let none = all_dabs(&with(0.0), &pts);
    assert_eq!(all_dabs(&with(f32::NAN), &pts), none);
    assert!(none.0.iter().all(|d| d.center.y == 10.0));
}

/// Control: an ordinary Scatter amount above 100 % still scatters, unchanged by the cap (#977).
#[test]
fn ordinary_scatter_is_unchanged_by_the_cap() {
    let b = BrushSettings {
        size: 20.0,
        scattering: Scattering { enabled: true, scatter: Dynamic::jitter(2.0), both_axes: true, count: 3, ..Default::default() },
        dual_brush: DualBrush { enabled: true, size: 6.0, scatter: 1.5, both_axes: true, ..Default::default() },
        ..brush()
    };
    assert_eq!(b.bounded_for_render(), b);
    let (dabs, duals) = all_dabs(&b, &line(10.0, 200.0, 50.0));
    assert!(dabs.iter().any(|d| (d.center.y - 50.0).abs() > 10.0) && dabs.iter().all(|d| (d.center.y - 50.0).abs() <= 20.0));
    assert!(duals.iter().any(|d| (d.center.y - 50.0).abs() > 1.0) && duals.iter().all(|d| (d.center.y - 50.0).abs() <= 4.5));
}

/// The dab rectangle of a hostile centre (non-finite or beyond `i32`), from a direct caller of the
/// public API, saturates instead of overflowing (#977).
#[test]
fn dab_rect_of_a_far_or_non_finite_centre_does_not_overflow() {
    let ctx = BrushContext::new(&brush());
    for c in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 1e300, -1e300, f64::from(i32::MAX), f64::from(i32::MIN)] {
        let d = Dab::round(Point::new(c, c), 10.0, 1.0);
        for r in [ctx.dab_rect(&d, false), ctx.dab_rect(&d, true), crate::retouch::dab_rect(&d, 1.0), crate::retouch::dab_rect(&d, 0.0)] {
            assert!(r.x0 <= r.x1 && r.y0 <= r.y1, "centre {c}: {r:?}");
        }
    }
    let huge = Dab::round(Point::new(5.0, 5.0), f32::INFINITY, 1.0);
    for hardness in [1.0, 0.0] {
        let r = crate::retouch::dab_rect(&huge, hardness);
        assert!(r.x0 <= r.x1 && r.y0 <= r.y1, "{r:?}");
    }
}

/// The rasterizer's computed tips are exactly the texel model: texels at whole-pixel offsets from
/// the dab centre (the relative radius on the ellipse, the first-order signed distance to its edge,
/// `Falloff::at`) placed bilinearly; its shortcuts (empty beyond the tail, full inside) change
/// nothing. Tips narrower than their tail (1 px) included.
#[test]
fn computed_tips_are_bilinearly_placed_texels() {
    for (size, hardness, roundness, angle) in
        [(1.0f32, 1.0f32, 1.0f32, 0.0f32), (3.0, 1.0, 1.0, 0.0), (9.0, 0.3, 1.0, 0.0), (60.0, 1.0, 0.2, 30.0), (40.0, 0.0, 0.5, -70.0)]
    {
        let b = BrushSettings { size, hardness, roundness, ..brush() };
        let ctx = BrushContext::new(&b);
        for centre in [Point::new(20.0, 20.0), Point::new(20.5, 20.5), Point::new(20.3, 20.7)] {
            let mut d = Dab::round(centre, size / 2.0, 1.0);
            d.roundness = roundness;
            d.angle = angle.to_radians();
            let rect = ctx.dab_rect(&d, false);
            let mut got = Vec::new();
            ctx.rasterize(&d, false, rect, &mut got);
            let (r, ro) = (d.radius, d.roundness.max(0.5 / d.radius).min(1.0));
            let rm = (r * ro).max(0.5);
            let (major, (sn, cs)) = (rm / ro, d.angle.sin_cos());
            let falloff = crate::Falloff::new(hardness, rm);
            // Texel at offset (tx, ty) from the centre (y down).
            let texel = |tx: f32, ty: f32| {
                let (dx, dy) = (tx, -ty);
                let (u, w) = (dx * cs + dy * sn, -dx * sn + dy * cs);
                let (qu, qv) = (u / major, w / rm);
                let q = (qu * qu + qv * qv).sqrt();
                let g = if q > 1e-6 { ((qu / major).powi(2) + (qv / rm).powi(2)).sqrt() / q } else { 0.0 };
                falloff.at(q, if g > 0.0 { (q - 1.0) / g } else { -rm })
            };
            for (i, v) in got.iter().enumerate() {
                let (x, y) = (rect.x0 + (i as u32 % rect.width()) as i32, rect.y0 + (i as u32 / rect.width()) as i32);
                let (dx, dy) = (x as f32 + 0.5 - centre.x as f32, y as f32 + 0.5 - centre.y as f32);
                let (tx, ty) = (dx.floor(), dy.floor());
                let (fx, fy) = (dx - tx, dy - ty);
                let top = texel(tx, ty) * (1.0 - fx) + texel(tx + 1.0, ty) * fx;
                let bottom = texel(tx, ty + 1.0) * (1.0 - fx) + texel(tx + 1.0, ty + 1.0) * fx;
                let want = top * (1.0 - fy) + bottom * fy;
                assert!((v - want).abs() < 1e-5, "size {size} round {roundness} at {centre:?} pixel ({x}, {y}): {v} vs {want}");
            }
        }
    }
}

/// Flow and Opacity apply as Photoshop's 8-bit levels (halves up), whatever the document depth.
#[test]
fn flow_and_opacity_apply_as_photoshop_levels() {
    use crate::brush::photoshop_level;
    for (pct, level) in [(1.0f32, 3.0f32), (2.0, 5.0), (3.0, 8.0), (10.0, 26.0), (30.0, 77.0), (50.0, 128.0), (100.0, 255.0), (0.0, 0.0)] {
        assert_eq!(photoshop_level(pct / 100.0), level / 255.0, "{pct} %");
    }
    assert_eq!(photoshop_level(f32::NAN), 0.0);
    assert_eq!(photoshop_level(2.0), 1.0);
    let b = BrushSettings { flow: 0.01, opacity: 0.5, ..brush() }.bounded_for_render();
    assert_eq!((b.flow, b.opacity), (3.0 / 255.0, 128.0 / 255.0));
}

#[test]
fn sequence_tips_alternate_by_dab_and_survive_json() {
    use crate::{BrushContext, Dab};
    let b = BrushSettings { tip: TipShape::Sequence(vec![GrayTile::from_fn(4, 4, |_, _| 1.0), GrayTile::from_fn(4, 4, |_, _| 0.0)]), ..brush() };
    let cx = BrushContext::new(&b);
    let mut dab = Dab::round(prism_geom::Point::new(10.0, 10.0), 4.0, 1.0);
    let mut pixels = Vec::new();
    for index in [0, 1, 2] {
        dab.index = index;
        cx.rasterize(&dab, false, cx.dab_rect(&dab, false), &mut pixels);
        assert_eq!(pixels.iter().any(|v| *v > 0.5), index != 1);
    }
    let bytes = serde_json::to_vec(&b).unwrap();
    assert_eq!(serde_json::from_slice::<BrushSettings>(&bytes).unwrap(), b);
    let empty = BrushSettings { tip: TipShape::Sequence(vec![]), ..brush() };
    let cx = BrushContext::new(&empty);
    cx.rasterize(&dab, false, cx.dab_rect(&dab, false), &mut pixels);
}

/// Merging sampled sequences must retain rotated corners and computed soft-tip tails alike.
#[test]
fn sequence_and_round_tip_bounds_preserve_all_coverage() {
    for dual in [false, true] {
        for tip in [TipShape::Round, TipShape::Sequence(vec![GrayTile::from_fn(16, 16, |_, _| 1.0)])] {
            let mut b = brush();
            b.hardness = 0.0;
            b.tip = tip.clone();
            b.dual_brush.enabled = dual;
            b.dual_brush.hardness = 0.0;
            b.dual_brush.tip = tip;
            let cx = BrushContext::new(&b);
            let mut dab = Dab::round(Point::new(32.25, 32.75), 16.0, 1.0);
            dab.angle = std::f32::consts::FRAC_PI_4;
            let bounds = cx.dab_rect(&dab, dual);
            let wide = Rect::new(-16, -16, 80, 80);
            let mut pixels = Vec::new();
            cx.rasterize(&dab, dual, wide, &mut pixels);
            let mut beyond_radius = false;
            for y in wide.y0..wide.y1 {
                for x in wide.x0..wide.x1 {
                    let value = pixels[((y - wide.y0) as u32 * wide.width() + (x - wide.x0) as u32) as usize];
                    if value > 0.0 {
                        assert!(bounds.contains(x, y), "clipped ({x},{y}), dual={dual}, tip={:?}", b.tip);
                        beyond_radius |= (x as f64 - dab.center.x).abs() > f64::from(dab.radius);
                    }
                }
            }
            assert!(beyond_radius, "fixture must exercise coverage outside the nominal radius");
        }
    }
}

#[test]
fn csp_tilt_response_scales_size_by_altitude_and_mouse_is_vertical() {
    let curve = PressureCurve::new(vec![[0.0, 1.0], [0.5, 1.0], [0.8, 0.0], [1.0, 0.0]]).unwrap();
    let tilt = CspTilt::new(0.5, 1.0, curve.clone()).unwrap();
    let b = BrushSettings {
        size: 20.0,
        shape_dynamics: ShapeDynamics { enabled: true, size: Dynamic { csp_tilt: Some(tilt.clone()), ..Default::default() }, ..Default::default() },
        ..brush()
    };
    let at = |tilt_x: f32| dabs_of(&b, &[StrokePoint { tilt_x, ..StrokePoint::new(10.0, 10.0, 1.0) }])[0].radius * 2.0;
    assert!((at(0.0) - 10.0).abs() < 1e-4, "mouse or vertical pen: input 1, response 0, 50%");
    assert!((at(90.0) - 20.0).abs() < 1e-4, "horizontal pen: input 0, response 1, 100%");
    // Unordered values are valid (the source interpolates either way); non-finite or huge ones are not.
    assert!(CspTilt::new(1.5, 0.2, curve.clone()).is_ok());
    for (lo, hi) in [(f32::NAN, 1.0), (0.5, 6.0), (-0.1, 1.0)] {
        assert!(CspTilt::new(lo, hi, curve.clone()).is_err());
    }
    let json = serde_json::to_string(&tilt).unwrap();
    assert_eq!(serde_json::from_str::<CspTilt>(&json).unwrap(), tilt);
    assert!(serde_json::from_str::<CspTilt>(&json.replace("0.5", "9.0")).is_err());
}

#[test]
fn csp_spray_size_ratio_follows_the_brush_size() {
    let b = BrushSettings { size: 50.0, csp_spray: Some(crate::CspSpray { count: 4, size: 3.0, size_ratio: Some(0.2), ..Default::default() }), ..brush() };
    let points = line(0.0, 0.0, 10.0);
    assert!(dabs_of(&b, &points).iter().all(|d| (d.radius - 5.0).abs() < 1e-4), "particle diameter 0.2 × 50");
    let resized = BrushSettings { size: 20.0, ..b.clone() };
    assert!(dabs_of(&resized, &points).iter().all(|d| (d.radius - 2.0).abs() < 1e-4));
    // Hostile ratios fall back to the stored particle size.
    for k in [f32::NAN, -1.0, 0.0] {
        let mut hostile = b.clone();
        hostile.csp_spray.as_mut().unwrap().size_ratio = Some(k);
        assert!(dabs_of(&hostile, &points).iter().all(|d| (d.radius - 1.5).abs() < 1e-4), "{k}");
    }
}

#[test]
fn csp_random_scales_each_dab_between_its_minimum_and_full_size() {
    let size = Dynamic { csp_random: Some(0.5), ..Default::default() };
    let b = BrushSettings { size: 40.0, spacing: 0.05, shape_dynamics: crate::ShapeDynamics { enabled: true, size, ..Default::default() }, ..brush() };
    let d = dabs_of(&b, &line(0.0, 400.0, 10.0));
    let radii: Vec<f32> = d.iter().map(|d| d.radius).collect();
    assert!(radii.iter().all(|r| (10.0 - 1e-3..=20.0 + 1e-3).contains(r)), "{radii:?}");
    assert!(radii.iter().any(|r| *r < 15.0) && radii.iter().any(|r| *r > 15.0), "random sizes, not one size");
    assert_eq!(serde_json::from_value::<Dynamic>(serde_json::to_value(&b.shape_dynamics.size).unwrap()).unwrap().csp_random, Some(0.5));
}
