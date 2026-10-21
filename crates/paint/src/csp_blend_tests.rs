//! CLIP STUDIO-only stroke blending modes and adding dual brush modes through the stroke renderer.
use prism_color::{ColorMode, PixelFormat, SampleType};
use prism_geom::Rect;
use prism_raster::{Surface, from_rgba};

use crate::brush::{BrushSettings, CspDualAdd, CspStrokeMode, DualBrush, MaskMode};
use crate::csp_blend::stroke_native;
use crate::render::{StrokeRenderer, render_stroke};
use crate::StrokePoint;

const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn formats() -> [PixelFormat; 3] {
    [SampleType::U8, SampleType::U16, SampleType::F32].map(|d| PixelFormat::new(ColorMode::Rgb, d, true))
}

/// Opaque blue over x 0..60, blue at 30 % alpha over 60..120, transparent beyond.
fn layer(fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(0, 0, 60, 40), &from_rgba(&fmt, BLUE));
    s.fill_rect(Rect::new(60, 0, 120, 40), &from_rgba(&fmt, [0.0, 0.0, 1.0, 0.3]));
    s
}

/// A hard red stroke at 60 % opacity along y = 20 across the three areas.
fn brush(mode: CspStrokeMode) -> BrushSettings {
    BrushSettings {
        size: 10.0,
        hardness: 1.0,
        opacity: 0.6,
        pressure_size: false,
        color: [1.0, 0.0, 0.0, 1.0],
        csp_stroke: Some(mode),
        ..Default::default()
    }
}

fn line() -> Vec<StrokePoint> {
    (0..=32).map(|i| StrokePoint::new(10.0 + f64::from(i) * 5.0, 20.5, 1.0)).collect()
}

fn painted(fmt: PixelFormat, b: &BrushSettings, lock: bool) -> Surface {
    let mut s = layer(fmt);
    render_stroke(&mut s, b, &line(), None, lock, 1.0);
    s
}

fn close(got: [f32; 4], want: [f32; 4], what: &str) {
    for (g, w) in got.iter().zip(&want) {
        assert!((g - w).abs() < 0.01, "{what}: {got:?} != {want:?}");
    }
}

/// The stroke centre over the opaque, 30 % and transparent areas, and a pixel outside the stroke.
const OPAQUE: (i32, i32) = (30, 20);
const THIN: (i32, i32) = (90, 20);
const EMPTY: (i32, i32) = (150, 20);
const OUTSIDE: (i32, i32) = (30, 35);

fn at(s: &Surface, p: (i32, i32)) -> [f32; 4] {
    s.rgba(p.0, p.1)
}

#[test]
fn behind_paints_under_existing_alpha_at_every_depth() {
    for fmt in formats() {
        let s = painted(fmt, &brush(CspStrokeMode::Behind), false);
        close(at(&s, OPAQUE), BLUE, "opaque paint stays on top");
        // a = 0.3 + 0.6 × 0.7; the red shows through by its share of that.
        close(at(&s, THIN), [0.42 / 0.72, 0.0, 0.3 / 0.72, 0.72], "thin");
        close(at(&s, EMPTY), [1.0, 0.0, 0.0, 0.6], "transparent");
        close(at(&s, OUTSIDE), BLUE, "outside");
        // Locked transparency: Behind changes nothing.
        let locked = painted(fmt, &brush(CspStrokeMode::Behind), true);
        close(at(&locked, THIN), [0.0, 0.0, 1.0, 0.3], "locked");
        assert_eq!(at(&locked, EMPTY)[3], 0.0);
    }
}

#[test]
fn compare_density_keeps_the_denser_at_every_depth() {
    for fmt in formats() {
        let s = painted(fmt, &brush(CspStrokeMode::CompareDensity), false);
        close(at(&s, OPAQUE), BLUE, "denser layer wins");
        close(at(&s, THIN), [1.0, 0.0, 0.0, 0.6], "denser stroke wins");
        close(at(&s, EMPTY), [1.0, 0.0, 0.0, 0.6], "transparent");
        close(at(&s, OUTSIDE), BLUE, "outside");
        // The stroke's soft edge never thins the layer.
        for y in 10..32 {
            assert!(s.rgba(90, y)[3] >= 0.3 - 0.01, "{fmt:?} y {y}");
        }
    }
}

#[test]
fn erase_compare_keeps_the_more_transparent_at_every_depth() {
    for fmt in formats() {
        for erase in [false, true] {
            let b = BrushSettings { erase, ..brush(CspStrokeMode::EraseCompare) };
            let s = painted(fmt, &b, false);
            close(at(&s, OPAQUE), [0.0, 0.0, 1.0, 0.4], "erased to 1 − 0.6");
            close(at(&s, THIN), [0.0, 0.0, 1.0, 0.3], "already more transparent");
            assert_eq!(at(&s, EMPTY)[3], 0.0);
            close(at(&s, OUTSIDE), BLUE, "outside");
            // Repeating the stroke changes nothing more (plain erasing would halve it again).
            let mut again = s.clone();
            render_stroke(&mut again, &b, &line(), None, false, 1.0);
            close(at(&again, OPAQUE), [0.0, 0.0, 1.0, 0.4], "repeat");
            let locked = painted(fmt, &b, true);
            close(at(&locked, OPAQUE), BLUE, "locked");
        }
    }
}

#[test]
fn replace_alpha_sets_the_stroke_alpha_at_every_depth() {
    for fmt in formats() {
        let s = painted(fmt, &brush(CspStrokeMode::ReplaceAlpha), false);
        for (p, what) in [(OPAQUE, "opaque"), (THIN, "thin"), (EMPTY, "transparent")] {
            close(at(&s, p), [1.0, 0.0, 0.0, 0.6], what);
        }
        close(at(&s, OUTSIDE), BLUE, "outside");
        // Locked: the alpha stays and the colour is replaced.
        let locked = painted(fmt, &brush(CspStrokeMode::ReplaceAlpha), true);
        close(at(&locked, THIN), [1.0, 0.0, 0.0, 0.3], "locked");
        assert_eq!(at(&locked, EMPTY)[3], 0.0);
    }
}

#[test]
fn opaque_formats_and_hostile_amounts_stay_bounded() {
    let rgb = PixelFormat::new(ColorMode::Rgb, SampleType::U8, false);
    for mode in [CspStrokeMode::Behind, CspStrokeMode::CompareDensity, CspStrokeMode::EraseCompare] {
        let s = painted(rgb, &brush(mode), false);
        close(at(&s, OPAQUE), BLUE, "opaque pixels keep their colour");
    }
    let s = painted(rgb, &brush(CspStrokeMode::ReplaceAlpha), false);
    close(at(&s, OPAQUE), [0.6, 0.0, 0.4, 1.0], "replaced by the stroke amount");
    let fmt = PixelFormat::RGBA32F;
    for mode in [CspStrokeMode::Behind, CspStrokeMode::ReplaceAlpha, CspStrokeMode::CompareDensity, CspStrokeMode::EraseCompare] {
        for (k, opacity) in [(f32::NAN, 0.5), (f32::INFINITY, 0.5), (-3.0, 0.5), (0.5, 0.0), (0.5, f32::NAN), (2.0, 1.0)] {
            let mut px = [0.2, 0.4, 0.6, 0.5];
            stroke_native(mode, &fmt, &mut px, &[1.0, 0.0, 0.0, 1.0], k, opacity, false);
            assert!(px.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{mode:?} {k} {opacity}: {px:?}");
        }
        // Short pixel slices are not indexed past their end.
        let mut short = [0.5];
        stroke_native(mode, &fmt, &mut short, &[1.0], 0.5, 0.5, false);
    }
}

fn dual_brush(add: Option<CspDualAdd>) -> BrushSettings {
    BrushSettings {
        size: 4.0,
        hardness: 1.0,
        pressure_size: false,
        spacing: 0.1,
        color: [1.0, 0.0, 0.0, 1.0],
        dual_brush: DualBrush { enabled: true, size: 20.0, hardness: 1.0, spacing: 0.1, csp_add: add, mode: MaskMode::Multiply, ..Default::default() },
        ..Default::default()
    }
}

/// Along y = 56: the main tip (radius 2) stays in the first tile row, the dual one (radius 10)
/// reaches the second (y ≥ 64).
fn dual_line() -> Vec<StrokePoint> {
    (0..=40).map(|i| StrokePoint::new(20.0 + f64::from(i) * 2.0, 56.5, 1.0)).collect()
}

#[test]
fn adding_dual_modes_paint_outside_the_first_brush_at_every_depth() {
    for fmt in formats() {
        let render = |add| {
            let mut s = Surface::new(fmt);
            render_stroke(&mut s, &dual_brush(add), &dual_line(), None, false, 1.0);
            s
        };
        let masked = render(None);
        assert_eq!(masked.rgba(60, 62)[3], 0.0, "{fmt:?}: a masking dual brush stays inside the stroke");
        for add in [CspDualAdd::Normal, CspDualAdd::Add, CspDualAdd::Lighten, CspDualAdd::Screen] {
            let s = render(Some(add));
            close(s.rgba(60, 56), [1.0, 0.0, 0.0, 1.0], "inside both");
            // Outside the main tip, in the dual tip: in the first tile row and the second.
            close(s.rgba(60, 62), [1.0, 0.0, 0.0, 1.0], "dual only");
            close(s.rgba(60, 65), [1.0, 0.0, 0.0, 1.0], "dual-only tile");
            assert_eq!(s.rgba(60, 70)[3], 0.0, "{fmt:?} {add:?}: beyond both");
        }
    }
}

#[test]
fn adding_dual_modes_combine_coverage() {
    let cases = [
        (CspDualAdd::Normal, 0.5 + 0.4 * 0.5),
        (CspDualAdd::Screen, 0.5 + 0.4 * 0.5),
        (CspDualAdd::Add, 0.9),
        (CspDualAdd::Lighten, 0.5),
    ];
    for (add, want) in cases {
        assert!((add.combine(0.5, 0.4) - want).abs() < 1e-6, "{add:?}");
        assert_eq!(add.combine(0.0, 0.0), 0.0, "{add:?}");
        assert!((add.combine(0.0, 0.3) - 0.3).abs() < 1e-6, "{add:?}: the second brush alone");
        for (c, t) in [(f32::NAN, 0.5), (2.0, 2.0), (-1.0, f32::INFINITY), (f32::NEG_INFINITY, f32::NAN)] {
            let v = add.combine(c, t);
            assert!((0.0..=1.0).contains(&v), "{add:?} {c} {t}: {v}");
        }
    }
    // Soft tips at partial opacity: Add never paints less than Lighten, both under the opacity
    // (Photoshop-level 128/255).
    let soft = |add| {
        let mut b = dual_brush(Some(add));
        b.opacity = 0.5;
        b.hardness = 0.0;
        b.dual_brush.hardness = 0.0;
        let mut s = Surface::new(PixelFormat::RGBA32F);
        render_stroke(&mut s, &b, &dual_line(), None, false, 1.0);
        (40..74).map(|y| s.rgba(60, y)[3]).collect::<Vec<_>>()
    };
    let (add, lighten) = (soft(CspDualAdd::Add), soft(CspDualAdd::Lighten));
    assert!(add.iter().zip(&lighten).all(|(a, l)| a + 1e-5 >= *l), "{add:?} {lighten:?}");
    assert!(add.iter().all(|a| *a <= 128.0 / 255.0 + 1e-4), "opacity still caps the stroke: {add:?}");
    assert!(add[0] > 0.0 && add[16] > 0.5, "{add:?}");
}

#[test]
fn adding_dual_brush_coverage_and_incremental_composites_match() {
    let b = dual_brush(Some(CspDualAdd::Lighten));
    let pts = dual_line();
    let fmt = PixelFormat::RGBA16;
    let pre = Surface::new(fmt);
    let mut once = pre.clone();
    render_stroke(&mut once, &b, &pts, None, false, 1.0);
    // Fed in chunks and composited as it goes, like interactive painting.
    let mut live = pre.clone();
    let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0).with_full_path(&pts);
    for chunk in pts.chunks(3) {
        r.push(chunk);
        r.composite(&pre, &mut live, None, false, false);
    }
    r.finish();
    r.composite(&pre, &mut live, None, false, false);
    let area = Rect::new(0, 30, 130, 80);
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            assert_eq!(live.rgba(x, y), once.rgba(x, y), "({x}, {y})");
        }
    }
    // Coverage queries see the dual-only pixels too.
    assert!(r.bounds().y1 > 64);
    assert!(r.coverage_at(60, 65) > 0.99);
    let (bounds, dense) = r.dense_coverage();
    let w = bounds.width() as usize;
    let i = (65 - bounds.y0) as usize * w + (60 - bounds.x0) as usize;
    assert!(dense.get(i).is_some_and(|v| *v > 0.99));
    assert_eq!(r.coverage_in(Rect::new(60, 65, 61, 66)), vec![r.coverage_at(60, 65)]);
}
