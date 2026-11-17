use super::*;

/// Vertical stripes (period 6) in grey with a square hole in the middle.
fn stripes(w: usize, h: usize, half: usize) -> (Vec<f32>, Vec<bool>) {
    let img: Vec<f32> = (0..w * h).map(|i| if (i % w) % 6 < 3 { 0.2 } else { 0.8 }).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (i % w).abs_diff(w / 2) < half && (i / w).abs_diff(h / 2) < half).collect();
    (img, hole)
}

#[test]
fn search_bounds_follow_the_rule() {
    // 20 × 10 hole in a large image, factor 3: area 9 × 200 = 1800 → tall = max(√1800, 20, 50) = 50,
    // wide = max(1800 / 50, 40, 50) = 50, centred on the hole.
    let hole = Bounds { x0: 490, y0: 495, x1: 510, y1: 505 };
    let b = search_bounds(1000, 1000, hole, 3.0);
    assert_eq!((b.width(), b.height()), (50, 50));
    assert_eq!(((b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2), (500, 500));
    // A big hole: sides ≥ 2 × the hole's, area ≈ factor² × hole area.
    let hole = Bounds { x0: 400, y0: 450, x1: 600, y1: 550 };
    let b = search_bounds(2000, 2000, hole, 3.0);
    assert!(b.width() >= 400 && b.height() >= 200, "{b:?}");
    let ratio = (b.width() * b.height()) as f32 / (9.0 * 200.0 * 100.0);
    assert!((0.95..1.6).contains(&ratio), "{ratio}");
    // Pushed inside the image at a corner.
    let b = search_bounds(300, 200, Bounds { x0: 0, y0: 0, x1: 40, y1: 30 }, 3.0);
    assert_eq!((b.x0, b.y0), (0, 0));
    assert!(b.x1 <= 300 && b.y1 <= 200);
}

#[test]
fn search_bounds_grow_when_too_little_is_sampleable() {
    let hole = Bounds { x0: 95, y0: 95, x1: 105, y1: 105 };
    let small = search_bounds_with(400, 400, hole, 2.0, |_| 0);
    let base = search_bounds(400, 400, hole, 2.0);
    assert!(small.width() > base.width(), "{small:?} vs {base:?}");
    assert!(small.width() < 180, "growth stops below 175 px: {small:?}");
}

#[test]
fn search_bounds_never_panic_on_odd_input() {
    for f in [f32::NAN, f32::INFINITY, -3.0, 0.0, 1e30] {
        let b = search_bounds(64, 48, Bounds { x0: 10, y0: 10, x1: 20, y1: 20 }, f);
        assert!(b.x1 <= 64 && b.y1 <= 48);
    }
    assert!(search_bounds(0, 0, Bounds { x0: 0, y0: 0, x1: 1, y1: 1 }, 3.0).is_empty());
}

#[test]
fn fills_only_the_hole_with_known_texture() {
    let (w, h) = (64, 40);
    let (img, hole) = stripes(w, h, 5);
    let src = vec![true; w * h];
    let out = fill(w, h, 1, &img, &hole, &src, &Options::default()).unwrap();
    for i in 0..w * h {
        if !hole[i] {
            assert_eq!(out[i], img[i]);
        }
    }
    let vals: Vec<f32> = (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).collect();
    assert!(vals.iter().any(|v| *v < 0.35) && vals.iter().any(|v| *v > 0.65), "stripes, not a flat average: {vals:?}");
}

#[test]
fn continues_the_stripes_in_phase() {
    let (w, h) = (72, 48);
    let (img, hole) = stripes(w, h, 6);
    let out = fill(w, h, 1, &img, &hole, &vec![true; w * h], &Options { color_limit: 0.0, membrane: 0.0, ..Options::default() }).unwrap();
    let err: f32 = (0..w * h).filter(|i| hole[*i]).map(|i| (out[i] - img[i]).abs()).sum::<f32>() / (0..w * h).filter(|i| hole[*i]).count() as f32;
    assert!(err < 0.1, "mean error {err}");
}

#[test]
fn sampling_area_restricts_sources() {
    let (w, h) = (48, 24);
    let img: Vec<f32> = (0..w * h).map(|i| if i % w < 24 { 0.1 } else { 0.9 }).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (20..28).contains(&(i % w)) && (8..16).contains(&(i / w))).collect();
    let right: Vec<bool> = (0..w * h).map(|i| i % w >= 30).collect();
    let out = fill(w, h, 1, &img, &hole, &right, &Options { color_limit: 0.0, membrane: 0.0, ..Options::default() }).unwrap();
    let mean: f32 = (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).sum::<f32>() / 64.0;
    assert!(mean > 0.8, "mean {mean}");
}

#[test]
fn empty_sampling_area_gives_none() {
    let (w, h) = (40, 30);
    let (img, hole) = stripes(w, h, 4);
    assert!(fill(w, h, 1, &img, &hole, &vec![false; w * h], &Options::default()).is_none());
}

#[test]
fn hostile_buffers_return_none_instead_of_panicking() {
    let o = Options::default();
    assert!(fill(10, 10, 1, &[0.0; 5], &[false; 100], &[true; 100], &o).is_none());
    assert!(fill(10, 10, 0, &[], &[false; 100], &[true; 100], &o).is_none());
    assert!(fill(0, 0, 1, &[], &[], &[], &o).is_none());
    assert!(fill(2, 2, 9, &[0.0; 36], &[true; 4], &[false; 4], &o).is_none());
    // Absurd options are clamped.
    let (img, hole) = stripes(30, 20, 3);
    let wild = Options { patch_size: 0, em_iters: 10_000, color_limit: f32::NAN, rotations: vec![f32::NAN, 1e30], ..o };
    let out = fill(30, 20, 1, &img, &hole, &vec![true; 600], &wild).unwrap();
    assert!(out.iter().all(|v| v.is_finite()));
}

#[test]
fn colour_offset_makes_a_brighter_copy_cheap() {
    // Left half a texture, right half the same texture 0.05 brighter.
    let (w, h) = (40, 20);
    let tex = |x: usize, y: usize| 0.3 + 0.2 * (((x * 7 + y * 3) % 5) as f32 / 4.0);
    let img: Vec<f32> = (0..w * h).map(|i| tex(i % 20, i / w) + if i % w >= 20 { 0.05 } else { 0.0 }).collect();
    let hole = vec![false; w * h];
    let lvl = Level { w, h, img, hole: hole.clone(), ok: vec![true; w * h], ctx: vec![true; w * h] };
    let plain = Solver::new(lvl.clone(), 1, &Options { color_limit: 0.0, membrane: 0.0, ..Options::default() }, Interrupt::NONE).unwrap();
    let adapt = Solver::new(lvl.clone(), 1, &Options { color_limit: 0.06, ..Options::default() }, Interrupt::NONE).unwrap();
    let (t, s) = ((30, 10), (10, 10));
    assert!(plain.dist(t, s, false, f32::INFINITY) > 0.05);
    assert!(adapt.dist(t, s, false, f32::INFINITY) < 1e-4);
    // The offset is clamped: 0.05 needs more than a 0.02 limit.
    let tight = Solver::new(lvl.clone(), 1, &Options { color_limit: 0.02, ..Options::default() }, Interrupt::NONE).unwrap();
    assert!(tight.dist(t, s, false, f32::INFINITY) > 1e-3);
}

#[test]
fn flipped_match_costs_nothing_on_a_mirrored_ramp() {
    // Left half a ramp up, right half the same ramp mirrored: a flipped source matches exactly.
    let (w, h) = (40, 12);
    let img: Vec<f32> = (0..w * h).map(|i| { let x = i % w; if x < 20 { x as f32 / 40.0 } else { (39 - x) as f32 / 40.0 } }).collect();
    let lvl = Level { w, h, img, hole: vec![false; w * h], ok: vec![true; w * h], ctx: vec![true; w * h] };
    let s = Solver::new(lvl.clone(), 1, &Options { mirror: true, color_limit: 0.0, membrane: 0.0, ..Options::default() }, Interrupt::NONE).unwrap();
    assert!(s.dist((30, 6), (9, 6), true, f32::INFINITY) < 1e-9);
    assert!(s.dist((30, 6), (9, 6), false, f32::INFINITY) > 1e-3);
}

#[test]
fn adaptations_are_deterministic_and_in_range() {
    let (w, h) = (48, 32);
    let (img, hole) = stripes(w, h, 4);
    let rgba: Vec<f32> = img.iter().flat_map(|v| [*v, *v * 0.5, 1.0 - *v, 1.0]).collect();
    let opts = Options { color_limit: 0.15, rotations: vec![-10.0, 10.0], scale: true, mirror: true, alpha: Some(3), seed: 7, ..Options::default() };
    let src = vec![true; w * h];
    let a = fill(w, h, 4, &rgba, &hole, &src, &opts).unwrap();
    let b = fill(w, h, 4, &rgba, &hole, &src, &opts).unwrap();
    assert_eq!(a, b);
    assert!(a.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    // Alpha stays opaque (no offset on it).
    assert!(a.chunks(4).all(|p| (p[3] - 1.0).abs() < 1e-6));
}

#[test]
fn cancel_stops_and_progress_reaches_one() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (w, h) = (64, 48);
    let (img, hole) = stripes(w, h, 6);
    let src = vec![true; w * h];
    let yes = || true;
    assert!(fill_with(w, h, 1, &img, &hole, &src, &Options::default(), &Interrupt::cancel_only(&yes)).is_err());
    let last = AtomicU32::new(0);
    let no = || false;
    let progress = |f: f32| {
        assert!(f >= f32::from_bits(last.load(Ordering::Relaxed)));
        last.store(f.to_bits(), Ordering::Relaxed);
    };
    let out = fill_with(w, h, 1, &img, &hole, &src, &Options::default(), &Interrupt::new(&no, &progress)).unwrap();
    assert_eq!(f32::from_bits(last.load(Ordering::Relaxed)), 1.0);
    assert_eq!(out, fill(w, h, 1, &img, &hole, &src, &Options::default()));
}

#[test]
fn larger_holes_use_the_pyramid() {
    let (w, h) = (200, 140);
    let (img, hole) = stripes(w, h, 30);
    let out = fill(w, h, 1, &img, &hole, &vec![true; w * h], &Options::default()).unwrap();
    let vals: Vec<f32> = (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).collect();
    let dark = vals.iter().filter(|v| **v < 0.4).count() as f32 / vals.len() as f32;
    assert!((0.25..0.75).contains(&dark), "texture, not a flat fill: {dark}");
}

#[test]
fn rectangular_area_is_the_search_bounds_without_the_hole() {
    let (w, h) = (400, 300);
    let hole: Vec<bool> = (0..w * h).map(|i| (190..210).contains(&(i % w)) && (140..160).contains(&(i / w))).collect();
    let img = vec![0.5f32; w * h];
    let a = sampling_area(w, h, 1, &img, &hole, Sampling::Rectangular { factor: 3.0 });
    let b = mask_bounds(w, h, &a).unwrap();
    assert_eq!(b, search_bounds(w, h, mask_bounds(w, h, &hole).unwrap(), 3.0));
    assert!(!(0..w * h).any(|i| hole[i] && a[i]));
}

#[test]
fn auto_area_leaves_out_unlike_colours() {
    // Grey noise everywhere, a saturated blob inside the search window, a hole in the middle.
    let (w, h) = (120, 90);
    let mut img = Vec::with_capacity(w * h * 3);
    for i in 0..w * h {
        let (x, y) = (i % w, i / w);
        let g = 0.45 + 0.1 * (((x * 13 + y * 7) % 11) as f32 / 10.0);
        if (31..41).contains(&x) && (16..26).contains(&y) { img.extend([0.95, 0.05, 0.05]) } else { img.extend([g, g, g]) }
    }
    let hole: Vec<bool> = (0..w * h).map(|i| (50..70).contains(&(i % w)) && (35..55).contains(&(i / w))).collect();
    let a = sampling_area(w, h, 3, &img, &hole, Sampling::Auto);
    assert!(!a[21 * w + 36], "the red blob is left out");
    assert!(a[40 * w + 40] && a[65 * w + 85], "grey around the hole is kept");
    assert!(!(0..w * h).any(|i| hole[i] && a[i]));
}

#[test]
fn dilate_grows_by_r() {
    let mut m = vec![false; 9 * 9];
    m[4 * 9 + 4] = true;
    let d = dilate(9, 9, &m, 2);
    assert_eq!(d.iter().filter(|v| **v).count(), 25);
    assert!(d[2 * 9 + 2] && d[6 * 9 + 6] && !d[9 + 4]);
}

#[test]
fn colour_levels() {
    assert_eq!(color_limit("none"), Some(0.0));
    assert_eq!(color_limit("default"), Some(0.06));
    assert_eq!(color_limit("veryHigh"), Some(0.3));
    assert_eq!(color_limit("bogus"), None);
}

#[test]
fn membrane_strength_pulls_a_lighter_fill_to_its_border() {
    // Dark stripes around the hole; the only source is a lighter copy of them far away.
    let (w, h) = (96, 48);
    let img: Vec<f32> = (0..w * h).map(|i| { let (x, _) = (i % w, i / w); let base = if x % 6 < 3 { 0.2 } else { 0.4 }; if x >= 64 { base + 0.3 } else { base } }).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (20..36).contains(&(i % w)) && (16..32).contains(&(i / w))).collect();
    let src: Vec<bool> = (0..w * h).map(|i| i % w >= 66).collect();
    let mean = |out: &[f32]| (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).sum::<f32>() / 256.0;
    let plain = fill(w, h, 1, &img, &hole, &src, &Options { color_limit: 0.0, membrane: 0.0, ..Options::default() }).unwrap();
    let pulled = fill(w, h, 1, &img, &hole, &src, &Options { color_limit: 0.0, membrane: 1.0, ..Options::default() }).unwrap();
    // The border's mean is 0.3; the lighter source's 0.6.
    assert!(mean(&plain) > 0.5, "{}", mean(&plain));
    assert!((mean(&pulled) - 0.3).abs() < 0.08, "{}", mean(&pulled));
    assert_eq!(membrane_strength("veryHigh"), Some(1.0));
    assert_eq!(membrane_strength("x"), None);
}
