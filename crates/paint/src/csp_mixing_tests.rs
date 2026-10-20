use super::*;

fn samples(pixels: &[([f32; 4], f32, [f32; 4])]) -> Samples {
    let mut s = Samples::default();
    for (layer, c, stroke) in pixels {
        s.add(*layer, *c, *stroke);
    }
    s
}

#[test]
fn samples_composite_the_stroke_over_the_layer() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    // Half-covered red pixel, an empty pixel: mean alpha 0.5, alpha-weighted colour 50/50.
    let s = samples(&[(red, 0.5, blue), ([0.0; 4], 0.0, blue)]);
    assert_eq!(s.count, 2);
    assert!((s.alpha - 1.0).abs() < 1e-9);
    assert!((s.color[0] - 0.5).abs() < 1e-9 && (s.color[2] - 0.5).abs() < 1e-9);
}

#[test]
fn reservoir_takes_the_first_pick_then_blends_by_stretch() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let green = [0.0, 1.0, 0.0, 1.0];
    let mut r = Reservoir::default();
    r.update(&Samples::default(), 0.5);
    assert!(!r.picked, "nothing sampled yet");
    r.update(&samples(&[(red, 0.0, red), ([0.0; 4], 0.0, red)]), 0.5);
    assert_eq!((r.alpha, r.color), (0.5, [1.0, 0.0, 0.0]));
    // Stretch 0.45: k = 0.05 / 0.5 = 0.1.
    r.update(&samples(&[(green, 0.0, green)]), 0.45);
    assert!((r.color[0] - 0.9).abs() < 1e-6 && (r.color[1] - 0.1).abs() < 1e-6);
    assert!((r.alpha - (0.5 * 0.9 + 0.1)).abs() < 1e-6);
    // Nothing under the dab: the colour stays, the alpha fades.
    let before = r.color;
    r.update(&samples(&[([0.0; 4], 0.0, green)]), 0.45);
    assert_eq!(r.color, before);
    assert!((r.alpha - 0.55 * 0.9).abs() < 1e-6);
    // No stretch: the new pick replaces the old one.
    r.update(&samples(&[(green, 0.0, green)]), 0.0);
    assert_eq!((r.alpha, r.color), (1.0, [0.0, 1.0, 0.0]));
}

#[test]
fn mix_follows_amount_squared_and_density() {
    let brush = [0.0, 0.0, 1.0, 1.0];
    let unpicked = Reservoir::default();
    assert_eq!(unpicked.mix(brush, 0.0, 0.0, false), (brush, 1.0), "the first dab paints the brush colour");
    let r = Reservoir { picked: true, alpha: 0.5, color: [1.0, 0.0, 0.0] };
    // Density: (1 - d) · A + d.
    assert!((r.mix(brush, 1.0, 0.0, false).1 - 0.5).abs() < 1e-6);
    assert!((r.mix(brush, 1.0, 0.5, false).1 - 0.75).abs() < 1e-6);
    assert_eq!(r.mix(brush, 1.0, 1.0, false).0, brush, "full amount keeps the brush colour");
    assert_eq!(r.mix(brush, 0.0, 1.0, false).0, [1.0, 0.0, 0.0, 1.0], "no paint: the picked colour");
    // a = 0.5: brush weight 0.25, picked weight 0.75 · 0.5.
    let (c, _) = r.mix(brush, 0.5, 1.0, false);
    assert!((c[0] - 0.375 / 0.625).abs() < 1e-6 && (c[2] - 0.25 / 0.625).abs() < 1e-6, "{c:?}");
    let empty = Reservoir { picked: true, alpha: 0.0, color: [1.0; 3] };
    assert_eq!(empty.mix(brush, 0.0, 1.0, false).0, brush, "nothing picked up: brush colour");
}

#[test]
fn blend_towards_moves_colour_and_alpha_and_can_thin_paint() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    // Half weight towards blue at target alpha 0.5: alpha 0.75, colour weighted 1 : 0.5.
    let o = blend_towards(red, blue, 0.5, 0.5, false);
    assert!((o[3] - 0.75).abs() < 1e-6 && (o[0] - 2.0 / 3.0).abs() < 1e-6 && (o[2] - 1.0 / 3.0).abs() < 1e-6, "{o:?}");
    // Full weight replaces the pixel, lowering its alpha to the paint's.
    assert_eq!(blend_towards(red, blue, 0.4, 1.0, false), [0.0, 0.0, 1.0, 0.4]);
    // Locked transparency keeps the alpha; an empty result stays empty.
    assert_eq!(blend_towards(red, blue, 0.4, 1.0, true)[3], 1.0);
    assert_eq!(blend_towards([0.0; 4], blue, 0.0, 0.5, false), [0.0; 4]);
    // Hostile weights clamp.
    assert_eq!(blend_towards(red, blue, 1.0, 7.0, false), blue);
}

#[test]
fn perceptual_space_round_trips_and_mixes_differently_from_rgb() {
    for rgb in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [1.0, 0.0, 0.0], [0.1, 0.6, 0.3], [0.5, 0.5, 0.5]] {
        let back = from_perceptual(to_perceptual(rgb));
        // sRGB white lands a hair above the D65 cone white and clamps there, as in CSP.
        for (a, b) in rgb.iter().zip(back) {
            assert!((a - b).abs() < 5e-4, "{rgb:?} -> {back:?}");
        }
    }
    // White maps to equal cone signals; out-of-range input clamps instead of producing NaN.
    let w = to_perceptual([1.0; 3]);
    assert!((w[0] - w[1]).abs() < 1e-5 && (w[1] - w[2]).abs() < 1e-5, "{w:?}");
    assert!(to_perceptual([f64::NAN, -1.0, 9.0]).iter().all(|v| v.is_finite()));
    // Half red, half blue: RGB averages to (0.5, 0, 0.5); the PQ midpoint of the cone signals
    // is darker and picks up some green.
    let red = Reservoir { picked: true, alpha: 1.0, color: [1.0, 0.0, 0.0] };
    let blue = [0.0, 0.0, 1.0, 1.0];
    let rgb = red.mix(blue, std::f32::consts::FRAC_1_SQRT_2, 1.0, false).0;
    let perceptual = red.mix(blue, std::f32::consts::FRAC_1_SQRT_2, 1.0, true).0;
    assert!((rgb[0] - 0.5).abs() < 1e-5 && (rgb[2] - 0.5).abs() < 1e-5, "{rgb:?}");
    assert!(perceptual[0] < 0.3 && perceptual[2] < 0.3 && perceptual[1] > 0.05, "{perceptual:?}");
    // Perceptual pick-up: a uniform colour samples back to itself.
    let mut s = Samples::new(true);
    s.add([0.2, 0.4, 0.8, 1.0], 0.0, blue);
    s.add([0.2, 0.4, 0.8, 1.0], 0.0, blue);
    let mut r = Reservoir::default();
    r.update(&s, 0.5);
    for (a, b) in r.color.iter().zip([0.2, 0.4, 0.8]) {
        assert!((a - b).abs() < 1e-5, "{:?}", r.color);
    }
}

#[test]
fn bounded_clamps_hostile_values() {
    let b = CspColorMixing { amount: f32::NAN, density: 7.0, stretch: -1.0, ..Default::default() }.bounded();
    assert_eq!((b.amount, b.density, b.stretch), (1.0, 1.0, 0.0));
}

#[test]
fn strokes_pick_up_the_layer_colour_and_are_chunk_invariant() {
    use crate::render::CoverageMap;
    use crate::{BrushSettings, StrokePoint, StrokeRenderer, render_stroke};
    use prism_color::{ColorMode, PixelFormat, SampleType};
    use prism_geom::Rect;
    use prism_raster::Surface;
    let brush = BrushSettings {
        size: 12.0,
        pressure_size: false,
        spacing: 0.1,
        color: [0.0, 0.0, 1.0, 1.0],
        csp_mixing: Some(CspColorMixing { amount: 0.3, density: 1.0, stretch: 0.3, ..Default::default() }),
        ..Default::default()
    };
    let pts: Vec<_> = (0..=40).map(|i| StrokePoint::new(10.0 + f64::from(i) * 3.0, 30.0, 1.0)).collect();
    // Mixing dabs paint over the stroke so far: a later dab's colour replaces an earlier one
    // even where coverage is already full.
    let mut cov = CoverageMap::new(2);
    cov.paint_over = true;
    let rect = Rect::new(0, 0, 1, 1);
    cov.accumulate(rect, &[1.0], 1.0, false, Some(&[1.0, 0.5]));
    cov.accumulate(rect, &[1.0], 1.0, false, Some(&[0.0, 0.9]));
    assert_eq!(cov.color_at(0, 0), Some(&[0.0, 0.9][..]));
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
        // Opaque red under the first half of the stroke.
        let mut layer = Surface::new(fmt);
        let mut px = [0.0f32; 8];
        let n = prism_raster::from_rgba_into(&fmt, [1.0, 0.0, 0.0, 1.0], &mut px);
        let red: Vec<f32> = (0..60 * 60).flat_map(|_| px[..n].to_vec()).collect();
        layer.write_region(Rect::new(0, 0, 60, 60), &red);
        let mut mixed = layer.clone();
        render_stroke(&mut mixed, &brush, &pts, None, false, 1.0);
        let mut plain = layer.clone();
        render_stroke(&mut plain, &BrushSettings { csp_mixing: None, ..brush.clone() }, &pts, None, false, 1.0);
        // Past the red area the plain stroke is pure blue; the mixing stroke drags red along.
        let (m, p) = (mixed.rgba(80, 30), plain.rgba(80, 30));
        assert!(p[0] < 0.01 && m[0] > 0.1, "{depth:?}: {m:?} vs {p:?}");
        // Streaming in chunks with the same source gives the same pixels.
        let mut live = layer.clone();
        let mut r = StrokeRenderer::new(&brush, Some(fmt), 1.0).with_mixing_source(&layer);
        for chunk in pts.chunks(5) {
            r.push(chunk);
            r.composite(&layer, &mut live, None, false, false);
        }
        r.finish();
        r.composite(&layer, &mut live, None, false, false);
        for x in (0..140).step_by(7) {
            assert_eq!(live.rgba(x, 30), mixed.rgba(x, 30), "{depth:?} x {x}");
        }
    }
}
