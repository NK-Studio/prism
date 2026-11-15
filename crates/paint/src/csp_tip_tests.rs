//! CSP tip order and colour tips: index sequences, hostile values, serde, rendering.

use prism_color::PixelFormat;
use prism_geom::Point;
use prism_raster::Surface;

use crate::brush::{BrushSettings, TipShape};
use crate::csp_tip::{CspTipColor, CspTipOrder};
use crate::render::{BrushContext, render_stroke};
use crate::tile::GrayTile;
use crate::{Dab, StrokePoint};

fn order(o: CspTipOrder, count: usize, n: u64) -> Vec<usize> {
    (0..n).map(|i| o.tip_index(i, count, 7)).collect()
}

#[test]
fn tip_orders_follow_csp_selection() {
    assert_eq!(order(CspTipOrder::Repeat, 3, 7), [0, 1, 2, 0, 1, 2, 0]);
    // Round trip: CSP's `k = n mod (2c − 2)`, mirrored at `c − 1`.
    assert_eq!(order(CspTipOrder::Reverse, 3, 9), [0, 1, 2, 1, 0, 1, 2, 1, 0]);
    assert_eq!(order(CspTipOrder::Reverse, 2, 5), [0, 1, 0, 1, 0]);
    assert_eq!(order(CspTipOrder::Once, 3, 6), [0, 1, 2, 2, 2, 2]);
    let random = order(CspTipOrder::Random, 4, 4000);
    for tip in 0..4 {
        let share = random.iter().filter(|t| **t == tip).count();
        assert!((800..1200).contains(&share), "tip {tip}: {share} of 4000");
    }
    assert_eq!(random, order(CspTipOrder::Random, 4, 4000), "deterministic per seed and dab");
}

#[test]
fn tip_order_survives_hostile_counts_and_codes() {
    for o in [CspTipOrder::Repeat, CspTipOrder::Reverse, CspTipOrder::Once, CspTipOrder::Random] {
        for count in [0, 1, 2, 64, usize::MAX] {
            for index in [0, 1, u64::MAX - 1, u64::MAX] {
                let i = o.tip_index(index, count, u64::MAX);
                assert!(i < count.max(1), "{o:?} {count} {index} -> {i}");
            }
        }
    }
    assert_eq!(CspTipOrder::from_code(2), Some(CspTipOrder::Once));
    for code in [-1, 4, 5, i64::MAX, i64::MIN] {
        assert_eq!(CspTipOrder::from_code(code), None);
    }
}

fn solid(w: u32, h: u32) -> GrayTile {
    GrayTile::from_fn(w, h, |_, _| 1.0)
}

fn red(w: u32, h: u32) -> CspTipColor {
    CspTipColor { width: w, height: h, rgb: [255, 0, 0].repeat((w * h) as usize).into() }
}

#[test]
fn colour_tip_planes_premultiply_and_reject_mismatches() {
    let tip = GrayTile::from_fn(2, 1, |x, _| if x == 0 { 1.0 } else { 0.5 });
    let planes = red(2, 1).premultiplied(&tip).unwrap();
    assert_eq!(planes[0].data, [65535, 32768]);
    assert_eq!(planes[1].data, [0, 0]);
    assert!(red(3, 1).premultiplied(&tip).is_none());
    assert!(CspTipColor { width: 2, height: 1, rgb: vec![1, 2].into() }.premultiplied(&tip).is_none());
    assert!(red(0, 0).premultiplied(&GrayTile { width: 0, height: 0, data: Vec::new().into() }).is_none());
    let colour = red(2, 1);
    let json = serde_json::to_string(&colour).unwrap();
    assert_eq!(serde_json::from_str::<CspTipColor>(&json).unwrap(), colour);
    assert!(serde_json::from_str::<CspTipColor>(r#"{"width":1,"height":1,"rgb":"%%%"}"#).is_err());
}

#[test]
fn colour_tip_mean_is_alpha_weighted_and_uniformity_ignores_clear_texels() {
    let tip = GrayTile::from_fn(3, 1, |x, _| [1.0, 0.5, 0.0][x as usize]);
    let c = CspTipColor { width: 3, height: 1, rgb: vec![255, 0, 0, 0, 0, 255, 0, 255, 0].into() };
    let m = c.mean(&tip).unwrap();
    assert!((m[0] - 2.0 / 3.0).abs() < 1e-3 && m[1].abs() < 1e-6 && (m[2] - 1.0 / 3.0).abs() < 1e-3, "{m:?}");
    assert!(!c.is_uniform(&tip, 2));
    let one = CspTipColor { width: 3, height: 1, rgb: vec![250, 250, 250, 252, 251, 250, 0, 0, 0].into() };
    assert!(one.is_uniform(&tip, 2), "the clear texel does not count");
    assert_eq!(c.mean(&GrayTile::from_fn(3, 1, |_, _| 0.0)), None);
    assert_eq!(c.mean(&GrayTile::from_fn(2, 1, |_, _| 1.0)), None);
}

fn stamp(b: &BrushSettings) -> Surface {
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, b, &[StrokePoint::new(20.0, 20.0, 1.0)], None, false, 1.0);
    s
}

#[test]
fn colour_tips_paint_their_own_colours() {
    let base = BrushSettings { size: 10.0, pressure_size: false, color: [0.0, 0.0, 1.0, 1.0], tip: TipShape::Sampled(solid(8, 8)), ..Default::default() };
    let coloured = BrushSettings { csp_tip_colors: vec![Some(red(8, 8))], ..base.clone() };
    let px = stamp(&coloured).rgba(20, 20);
    assert!(px[0] > 0.99 && px[2] < 0.01 && px[3] > 0.99, "{px:?}");
    // Brush colour: without colours, or with colours that do not fit.
    for b in [
        base.clone(),
        BrushSettings { csp_tip_colors: vec![Some(red(7, 8))], ..base.clone() },
        BrushSettings { csp_tip_colors: vec![None, Some(red(8, 8))], ..base.clone() },
    ] {
        let px = stamp(&b).rgba(20, 20);
        assert!(px[0] < 0.01, "{px:?}");
    }
    // Colour mixing mixes the tip's (mean) colour, not the brush colour.
    let px = stamp(&BrushSettings { csp_mixing: Some(Default::default()), ..coloured.clone() }).rgba(20, 20);
    assert!(px[3] > 0.0 && px[0] > 0.99 && px[2] < 0.01, "{px:?}");
    let json = serde_json::to_vec(&coloured).unwrap();
    assert_eq!(serde_json::from_slice::<BrushSettings>(&json).unwrap(), coloured);
    // Older presets have no colours.
    assert!(serde_json::from_slice::<BrushSettings>(&serde_json::to_vec(&base).unwrap()).unwrap().csp_tip_colors.is_empty());
}

#[test]
fn a_sequence_mixes_colour_and_brush_colour_tips_in_tip_order() {
    let b = BrushSettings {
        size: 10.0,
        spacing: 3.0,
        pressure_size: false,
        color: [0.0, 0.0, 1.0, 1.0],
        tip: TipShape::Sequence(vec![solid(8, 8), solid(8, 8)]),
        csp_tip_colors: vec![Some(red(8, 8)), None],
        csp_tip_order: Some(CspTipOrder::Once),
        ..Default::default()
    };
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, &b, &[StrokePoint::new(10.0, 20.0, 1.0), StrokePoint::new(130.0, 20.0, 1.0)], None, false, 1.0);
    // First dab: the red tip; every later dab stays on the last (brush-coloured) tip.
    let (first, later) = (s.rgba(10, 20), s.rgba(100, 20));
    assert!(first[0] > 0.99 && first[2] < 0.01, "{first:?}");
    assert!(later[2] > 0.99 && later[0] < 0.01, "{later:?}");
    let cx = BrushContext::new(&b);
    let mut dab = Dab::round(Point::new(20.0, 20.0), 5.0, 1.0);
    let mut rgb = Vec::new();
    for (index, coloured) in [(0, true), (1, false), (9, false)] {
        dab.index = index;
        assert_eq!(cx.rasterize_tip_color(&dab, cx.dab_rect(&dab, false), &mut rgb), coloured);
    }
}

/// A 16×16 opaque tip, gray 0 on the left half and `right` on the right.
fn gray_brush(right: u8) -> BrushSettings {
    let gray: Vec<u8> = (0..16 * 16).flat_map(|i| [if i % 16 < 8 { 0 } else { right }; 3]).collect();
    BrushSettings {
        size: 16.0,
        pressure_size: false,
        color: [1.0, 0.0, 0.0, 1.0],
        background: [0.0, 0.0, 1.0, 1.0],
        tip: TipShape::Sampled(solid(16, 16)),
        csp_tip_colors: vec![Some(CspTipColor { width: 16, height: 16, rgb: gray.into() })],
        csp_tip_grays: vec![true],
        ..Default::default()
    }
}

#[test]
fn gray_tips_blend_the_dab_colour_towards_the_sub_colour_at_every_depth() {
    use prism_color::{ColorMode, SampleType};
    let b = gray_brush(128);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        render_stroke(&mut s, &b, &[StrokePoint::new(20.0, 20.0, 1.0)], None, false, 1.0);
        let (left, right) = (s.rgba(16, 20), s.rgba(24, 20));
        for (actual, expected) in left.iter().zip([1.0, 0.0, 0.0, 1.0]) {
            assert!((actual - expected).abs() < 0.01, "{sample:?} left {left:?}");
        }
        // Gray 128/255: about half way to the sub colour, and fully opaque.
        for (actual, expected) in right.iter().zip([0.498, 0.0, 0.502, 1.0]) {
            assert!((actual - expected).abs() < 0.01, "{sample:?} right {right:?}");
        }
    }
    // The dab's own colour (here a fixed sub-colour blend) is what the gray starts from.
    let tinted = BrushSettings { csp_sub_color: Some(0.5), ..gray_brush(255) };
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, &tinted, &[StrokePoint::new(20.0, 20.0, 1.0)], None, false, 1.0);
    let (left, right) = (s.rgba(16, 20), s.rgba(24, 20));
    assert!((left[0] - 0.5).abs() < 0.01 && (left[2] - 0.5).abs() < 0.01, "{left:?}");
    assert!(right[0] < 0.01 && right[2] > 0.99, "{right:?}");
}

#[test]
fn gray_tips_erase_and_mix_with_their_gray_as_transparency() {
    // Erasing an opaque layer: the gray half erases only by its darkness (1 − gray).
    let erase = BrushSettings { erase: true, ..gray_brush(255) };
    let mut s = Surface::new(PixelFormat::RGBA32F);
    s.fill_rect(prism_geom::Rect::new(0, 0, 64, 64), &[1.0, 1.0, 1.0, 1.0]);
    render_stroke(&mut s, &erase, &[StrokePoint::new(20.0, 20.0, 1.0)], None, false, 1.0);
    assert!(s.rgba(16, 20)[3] < 0.01, "black half erases");
    assert!(s.rgba(24, 20)[3] > 0.99, "white half keeps the layer");
    // Colour mixing lays one colour a dab: the white half stays transparent.
    let mixing = BrushSettings { csp_mixing: Some(crate::csp_mixing::CspColorMixing::default()), ..gray_brush(255) };
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, &mixing, &[StrokePoint::new(20.0, 20.0, 1.0)], None, false, 1.0);
    assert!(s.rgba(16, 20)[3] > 0.99 && s.rgba(24, 20)[3] < 0.01, "{:?} {:?}", s.rgba(16, 20), s.rgba(24, 20));
    // Hostile flags (longer than the colours, on a tip without colours) change nothing and never panic.
    let hostile = BrushSettings { csp_tip_colors: vec![None], csp_tip_grays: vec![true, true, true], ..gray_brush(255) };
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, &hostile, &[StrokePoint::new(20.0, 20.0, 1.0)], None, false, 1.0);
    assert!(s.rgba(24, 20)[0] > 0.99, "{:?}", s.rgba(24, 20));
}

#[test]
fn ribbons_lay_colour_tips_pixel_by_pixel() {
    // Left half red, right half blue; at no turn the tip's column 0 runs on the right of travel.
    let split =
        |w: u32, h: u32| CspTipColor { width: w, height: h, rgb: (0..w * h).flat_map(|i| if i % w < w / 2 { [255, 0, 0] } else { [0, 0, 255] }).collect() };
    let ribbon = |turns: f64, tip: GrayTile, colors: CspTipColor| BrushSettings {
        size: 10.0,
        pressure_size: false,
        color: [0.0, 1.0, 0.0, 1.0],
        tip: TipShape::Sampled(tip),
        csp_tip_colors: vec![Some(colors)],
        csp_ribbon: Some(crate::csp_ribbon::CspRibbon::from_rotation(turns)),
        ..Default::default()
    };
    let line = [StrokePoint::new(20.0, 50.0, 1.0), StrokePoint::new(180.0, 50.0, 1.0)];
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, &ribbon(0.0, solid(10, 10), split(10, 10)), &line, None, false, 1.0);
    let (right, left) = (s.rgba(100, 53), s.rgba(100, 47));
    assert!(right[0] > 0.99 && right[1] < 0.01 && left[2] > 0.99 && left[1] < 0.01, "{right:?} {left:?}");
    // A tip whose empty last row the ribbon drops keeps its colours aligned (and nothing panics).
    let trimmed = GrayTile::from_fn(10, 11, |_, y| if y < 10 { 1.0 } else { 0.0 });
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, &ribbon(90.0, trimmed, split(10, 11)), &line, None, false, 1.0);
    let painted: Vec<_> = (40..60).map(|y| s.rgba(100, y)).filter(|p| p[3] > 0.99).collect();
    assert!(!painted.is_empty() && painted.iter().all(|p| p[1] < 0.01 && (p[0] > 0.99 || p[2] > 0.99 || p[0] + p[2] > 0.99)), "{painted:?}");
}
