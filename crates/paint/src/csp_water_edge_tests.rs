use super::*;
use prism_geom::Rect;

#[test]
fn bayer_is_the_standard_ordered_dither_permutation() {
    let mut seen = [false; 256];
    for y in 0..16 {
        for x in 0..16 {
            seen[bayer(x, y) as usize] = true;
        }
    }
    assert!(seen.iter().all(|&s| s));
    // First row/column of the recursive 2×2 construction, and 16-periodicity.
    assert_eq!([bayer(0, 0), bayer(1, 0), bayer(2, 0), bayer(4, 0), bayer(8, 0)], [0, 128, 32, 8, 2]);
    assert_eq!([bayer(0, 1), bayer(1, 1), bayer(15, 15)], [192, 64, 85]);
    assert_eq!(bayer(17, 33), bayer(1, 1));
    assert_eq!(bayer(-1, -1), bayer(15, 15));
}

#[test]
fn amount_follows_distance_and_window_maximum() {
    let e = CspWaterEdge { radius: 2.0, opacity: 0.15, ..Default::default() };
    let p = EdgeParams::new(&e).unwrap();
    assert_eq!(p.n, 2);
    let noise = |x: i32, y: i32| f64::from(bayer(x, y)) * 0.0004 + 0.95;
    // Adjacent pixel: 3a.
    let a1 = p.amount(1, 1.0, 0, 0);
    assert!((f64::from(a1) - 0.45 * noise(0, 0)).abs() < 1e-6);
    // Diagonal neighbour: (R - d)·10a/R with R = 3.
    let a2 = p.amount(2, 0.5, 1, 0);
    let want = (3.0 - 2f64.sqrt()) * 1.5 / 3.0 * 0.5 * noise(1, 0);
    assert!((f64::from(a2) - want).abs() < 1e-6, "{a2} {want}");
    // Nothing in reach: (n+1)² = R² gives zero; beyond R² is excluded.
    assert_eq!(p.amount(9, 1.0, 0, 0), 0.0);
    assert_eq!(p.amount(10, 1.0, 0, 0), 0.0);
    assert_eq!(p.amount(0, 1.0, 0, 0), 0.0);
    // The opacity influence has a 1% floor.
    let tiny = EdgeParams::new(&CspWaterEdge { radius: 1.0, opacity: 0.0, ..Default::default() }).unwrap();
    assert!(tiny.amount(1, 1.0, 0, 0) > 0.0);
    assert!(EdgeParams::new(&CspWaterEdge { radius: 0.0, ..Default::default() }).is_none());
}

#[test]
fn live_amounts_measure_euclidean_distance_to_empty_pixels() {
    // 9×9 solid square inside an 11×11 area, radius 2.
    let p = EdgeParams::new(&CspWaterEdge { radius: 2.0, opacity: 0.2, ..Default::default() }).unwrap();
    let rect = Rect::new(0, 0, 11, 11);
    let pad_rect = rect.inflate(p.n);
    let pw = pad_rect.width() as usize;
    let mut pad = vec![0.0f32; pw * pad_rect.height() as usize];
    for y in 1..10 {
        for x in 1..10 {
            pad[((y + 2) * pw as i32 + x + 2) as usize] = 1.0;
        }
    }
    let a = live_amounts(&p, &pad, rect);
    let at = |x: usize, y: usize| a[y * 11 + x];
    assert_eq!(at(0, 0), 0.0, "empty pixels get nothing");
    assert!((at(1, 5) - p.amount(1, 1.0, 1, 5)).abs() < 1e-7, "edge pixel");
    assert!((at(2, 5) - p.amount(4, 1.0, 2, 5)).abs() < 1e-7, "two pixels in");
    assert!((at(2, 2) - p.amount(4, 1.0, 2, 2)).abs() < 1e-7, "straight distances win in a square");
    assert_eq!(at(3, 5), 0.0, "distance R adds nothing");
    assert_eq!(at(5, 5), 0.0, "interior beyond the radius");
    // A wrongly sized buffer is ignored, never indexed out of bounds.
    assert!(live_amounts(&p, &[1.0; 3], rect).iter().all(|&v| v == 0.0));
}

#[test]
fn coverage_below_the_fifteen_bit_step_is_empty() {
    assert!(empty(0.0));
    assert!(empty(1.0 / 70000.0));
    assert!(!empty(1.0 / 60000.0));
    assert!(empty(f32::NAN));
}

#[test]
fn gaussian_taps_end_at_one_255th_of_the_centre() {
    let t = gaussian_taps(10.0);
    assert_eq!(t.len(), 11);
    assert_eq!(t[0], 65535);
    assert_eq!(t[10], 257);
    assert!(t.windows(2).all(|w| w[0] >= w[1]));
    assert_eq!(gaussian_taps(0.0), vec![65535]);
    assert_eq!(gaussian_taps(2.5).len(), 3);
    assert_eq!(gaussian_taps(f32::INFINITY).len(), gaussian_taps(MAX_BLUR).len());
}

#[test]
fn blur_keeps_constants_and_spreads_with_floor_division() {
    let (w, h) = (16, 16);
    let mut flat = vec![200u8; w * h];
    blur_u8(&mut flat, w, h, 3.0);
    assert!(flat.iter().all(|&v| v == 200), "edge-renormalized constant stays constant");
    let mut dot = vec![0u8; w * h];
    dot[8 * w + 8] = 255;
    blur_u8(&mut dot, w, h, 3.0);
    assert!(dot[8 * w + 8] < 255 && dot[8 * w + 8] > 0);
    assert!(dot[8 * w + 10] > 0, "spreads within the radius");
    assert_eq!(dot[8 * w + 12], 0, "nothing beyond the truncated radius");
    assert_eq!(dot[0], 0);
    let total: u32 = dot.iter().map(|&v| u32::from(v)).sum();
    assert!(total <= 255, "floor division never creates energy: {total}");
    // Too small a buffer is ignored.
    let mut short = vec![9u8; 3];
    blur_u8(&mut short, w, h, 3.0);
    assert_eq!(short, [9, 9, 9]);
}

#[test]
fn eight_bit_round_trip_and_darkening() {
    assert_eq!(to_u8(0.0), 0);
    assert_eq!(to_u8(1.0), 255);
    assert_eq!(to_u8(2.0), 255);
    assert_eq!(to_u8(0.5), 128);
    assert_eq!(from_u8(255), 32768.0 / 32768.0);
    assert!((from_u8(128) - 0.501_953_1).abs() < 1e-6);
    let c = darken([0.8, 0.4, 0.2, 1.0], 100.0 / 255.0, 0.5);
    let k = (0.8 - 50.0 / 255.0) / 0.8;
    assert!((c[0] - 0.8 * k).abs() < 1e-6 && (c[1] - 0.4 * k).abs() < 1e-6 && c[3] == 1.0);
    assert_eq!(darken([0.1, 0.1, 0.1, 1.0], 1.0, 1.0), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(darken([0.5, 0.5, 0.5, 1.0], 0.0, 1.0), [0.5, 0.5, 0.5, 1.0]);
}

#[test]
fn bounded_rejects_non_finite_and_out_of_range_values() {
    let e = CspWaterEdge { radius: f32::NAN, opacity: 7.0, darkness: -1.0, blur: f32::INFINITY, after_drag: true }.bounded();
    assert_eq!(e.radius, 0.0);
    assert_eq!(e.opacity, 1.0);
    assert_eq!(e.darkness, 0.0);
    assert_eq!(e.blur, 0.0);
    assert!(!e.blurs());
    assert_eq!(CspWaterEdge { radius: 1e9, ..Default::default() }.bounded().radius, MAX_RADIUS);
    assert!(CspWaterEdge { blur: 1.0, after_drag: true, ..Default::default() }.blurs());
    assert!(!CspWaterEdge { blur: 1.0, after_drag: false, ..Default::default() }.blurs());
}

/// Straightforward two-pass reference for the span-restricted, strip-transposed blur.
fn naive_blur(buf: &[u8], w: usize, h: usize, blur: f32) -> Vec<u8> {
    let taps = gaussian_taps(blur);
    let n = taps.len() - 1;
    let pass = |src: &[u8], len: usize, at: &dyn Fn(usize, usize) -> usize, lines: usize| {
        let mut out = src.to_vec();
        for l in 0..lines {
            for i in 0..len {
                let (mut sw, mut swv) = (0u64, 0u64);
                for j in i.saturating_sub(n)..(i + n + 1).min(len) {
                    let t = taps[j.abs_diff(i)];
                    sw += t;
                    swv += t * u64::from(src[at(l, j)]);
                }
                out[at(l, i)] = if swv == 0 { 0 } else { (swv / sw).min(255) as u8 };
            }
        }
        out
    };
    let horizontal = pass(buf, w, &|y, x| y * w + x, h);
    pass(&horizontal, h, &|x, y| y * w + x, w)
}

#[test]
fn optimized_blur_matches_the_reference_exactly() {
    let mut seed = 0x1234_5678u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for (w, h, blur) in [(37, 29, 3.7), (130, 70, 10.0), (5, 140, 2.0), (70, 3, 6.5)] {
        let buf: Vec<u8> = (0..w * h).map(|_| if next() % 9 == 0 { (next() % 256) as u8 } else { 0 }).collect();
        let mut fast = buf.clone();
        blur_u8(&mut fast, w, h, blur);
        assert_eq!(fast, naive_blur(&buf, w, h, blur), "{w}x{h} blur {blur}");
    }
}
