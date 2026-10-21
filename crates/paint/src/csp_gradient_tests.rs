use super::*;

/// A stop of a synthetic blob: (rgb, opacity, flags, position, curve points).
type Stop<'a> = ([u32; 3], u32, u32, u32, &'a [[f64; 2]]);

/// A synthetic `GradationData` blob.
fn blob(stops: &[Stop]) -> Vec<u8> {
    let mut b: Vec<u8> = [24u32, 28, stops.len() as u32, 16, 1, 3].into_iter().flat_map(u32::to_be_bytes).collect();
    for (rgb, a, flags, pos, curve) in stops {
        b.extend([rgb[0], rgb[1], rgb[2], *a, *flags, *pos, curve.len() as u32].into_iter().flat_map(u32::to_be_bytes));
    }
    for (.., curve) in stops {
        b.extend(curve.iter().flat_map(|p| p.iter().flat_map(|v| v.to_be_bytes())));
    }
    b
}

const FULL: u32 = u32::MAX;

#[test]
fn decodes_live_colours_opacity_positions_and_curves() {
    // Main colour to transparent white.
    let g = CspGradient::decode(&blob(&[([0; 3], FULL, 1, 0, &[]), ([FULL; 3], 0, 0, 0x8000, &[])])).unwrap();
    assert_eq!(g.stops.len(), 2);
    assert_eq!((g.stops[0].color, g.stops[0].opacity, g.stops[0].position), (CspStopColor::Foreground, 1.0, 0.0));
    assert_eq!((g.stops[1].color, g.stops[1].opacity, g.stops[1].position), (CspStopColor::Rgb([1.0; 3]), 0.0, 1.0));
    // Main to sub colour with a step (the 0x10 flag marks a curve), stop at the middle.
    let g = CspGradient::decode(&blob(&[([0; 3], FULL, 0x11, 0, &[[0.0, 0.0], [1.0, 0.0]]), ([FULL; 3], FULL, 2, 0x4000, &[])])).unwrap();
    assert_eq!(g.stops[0].curve, vec![[0.0, 0.0], [1.0, 0.0]]);
    assert_eq!((g.stops[1].color, g.stops[1].position), (CspStopColor::Background, 0.5));
    // Fixed colours keep their channels.
    let g = CspGradient::decode(&blob(&[([FULL, 0, 0], FULL, 0, 0, &[]), ([0, 0x80808080, FULL], FULL, 0x10, 0x8000, &[])])).unwrap();
    let CspStopColor::Rgb(c) = g.stops[1].color else { panic!() };
    assert!((c[1] - 0.502).abs() < 1e-3 && c[2] == 1.0 && c[0] == 0.0);
}

#[test]
fn malformed_data_is_an_error_never_a_panic() {
    let good = blob(&[([0; 3], FULL, 1, 0, &[[0.0, 0.0], [0.5, 0.2], [1.0, 1.0]]), ([FULL; 3], 0, 0, 0x8000, &[])]);
    assert!(CspGradient::decode(&good).is_ok());
    for n in 0..good.len() {
        assert!(CspGradient::decode(&good[..n]).is_err(), "truncated at {n}");
    }
    let mut one = blob(&[([0; 3], FULL, 1, 0, &[])]);
    assert!(CspGradient::decode(&one).is_err(), "one stop");
    one[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(CspGradient::decode(&one).is_err(), "huge count");
    let mut small = good.clone();
    small[4..8].copy_from_slice(&4u32.to_be_bytes());
    assert!(CspGradient::decode(&small).is_err(), "record too small");
    let mut nan = good.clone();
    let at = good.len() - 8;
    nan[at..].copy_from_slice(&f64::NAN.to_be_bytes());
    assert!(CspGradient::decode(&nan).is_err());
    let mut points = good.clone();
    points[24 + 24..24 + 28].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(CspGradient::decode(&points).is_err(), "huge curve");
    // Positions past the end clamp; unsorted stops sort.
    let g = CspGradient::decode(&blob(&[([0; 3], FULL, 0, 0xffff_ffff, &[]), ([FULL; 3], FULL, 0, 0, &[])])).unwrap();
    assert_eq!([g.stops[0].position, g.stops[1].position], [0.0, 1.0]);
    assert!(g.is_valid());
}

#[test]
fn prism_stops_follow_curves_where_stops_can_hold_them() {
    let g = |stops: &[Stop]| CspGradient::decode(&blob(stops)).unwrap();
    // Straight: one to one.
    let s = g(&[([0; 3], FULL, 1, 0, &[]), ([FULL; 3], 0, 0, 0x8000, &[])]).prism_stops();
    assert_eq!(s.colors, vec![(0.0, CspStopColor::Foreground), (1.0, CspStopColor::Rgb([1.0; 3]))]);
    assert_eq!(s.opacity, vec![(0.0, 1.0), (1.0, 0.0)]);
    assert!(!s.approximated);
    // A step from the main colour: a hard edge, exact although the colour is live.
    let s = g(&[([0; 3], FULL, 0x11, 0, &[[0.0, 0.0], [1.0, 0.0]]), ([FULL; 3], FULL, 2, 0x4000, &[])]).prism_stops();
    assert_eq!(s.colors.iter().map(|c| c.1).collect::<Vec<_>>(), [CspStopColor::Foreground, CspStopColor::Foreground, CspStopColor::Background]);
    assert!(s.colors[1].0 < 0.5 && s.colors[1].0 > 0.4999 && s.colors[2].0 == 0.5);
    assert!(!s.approximated);
    // A curve between fixed colours is sampled; its middle follows the curve, not a straight line.
    let curve: &[[f64; 2]] = &[[0.0, 0.0], [0.5, 0.1], [1.0, 1.0]];
    let s = g(&[([0; 3], FULL, 0x10, 0, curve), ([FULL; 3], 0, 0, 0x8000, &[])]).prism_stops();
    let mid = s.colors.iter().find(|(t, _)| (*t - 0.5).abs() < 1e-6).unwrap().1;
    assert_eq!(mid, CspStopColor::Rgb([0.1; 3]));
    assert!(s.opacity.iter().any(|(t, a)| (*t - 0.5).abs() < 1e-6 && (*a - 0.9).abs() < 1e-5));
    assert!(!s.approximated);
    // The same curve from the main colour to a fixed one runs straight in colour.
    let s = g(&[([0; 3], FULL, 0x11, 0, curve), ([FULL; 3], FULL, 0, 0x8000, &[])]).prism_stops();
    assert_eq!(s.colors, vec![(0.0, CspStopColor::Foreground), (1.0, CspStopColor::Rgb([1.0; 3]))]);
    assert!(s.approximated);
    // Sampling stays within what Prism's gradient takes, however many stops.
    let many: Vec<_> = (0..MAX_STOPS as u32).map(|i| ([i; 3], FULL, 0x10, i * 0x80, curve)).collect();
    let s = g(&many).prism_stops();
    assert!(s.colors.len() <= MAX_PRISM_STOPS && s.opacity.len() <= MAX_PRISM_STOPS, "{} {}", s.colors.len(), s.opacity.len());
}

#[test]
fn sample_reverse_and_curves() {
    let curve: &[[f64; 2]] = &[[0.0, 0.0], [1.0, 0.0]];
    let g = CspGradient::decode(&blob(&[([0; 3], FULL, 0x11, 0, curve), ([FULL; 3], FULL, 2, 0x8000, &[])])).unwrap();
    let (fg, bg) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    assert_eq!(g.sample(0.9, fg, bg), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(g.sample(1.0, fg, bg), [0.0, 0.0, 1.0, 1.0]);
    let r = g.reversed();
    assert_eq!(r.stops[0].color, CspStopColor::Background);
    assert_eq!(r.stops[0].curve, vec![[0.0, 1.0], [1.0, 1.0]]);
    assert!(r.stops[1].curve.is_empty() && r.is_valid());
    assert_eq!(r.sample(0.1, fg, bg), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(r.reversed(), g);
    // Curves: straight without points, flat outside, monotone between.
    assert_eq!(curve_at(&[], 0.3), 0.3);
    let c = [[0.2, 0.0], [0.5, 0.2], [0.8, 1.0]];
    assert_eq!((curve_at(&c, 0.1), curve_at(&c, 0.9)), (0.0, 1.0));
    let ys: Vec<f32> = (0..=100).map(|i| curve_at(&c, i as f32 / 100.0)).collect();
    assert!(ys.windows(2).all(|w| w[1] >= w[0] - 1e-6), "{ys:?}");
    assert!(curve_at(&[[0.5, 0.0], [0.5, 1.0]], 0.5).is_finite());
    assert!(CspGradient::default().sample(0.5, fg, bg)[3] == 1.0);
}
