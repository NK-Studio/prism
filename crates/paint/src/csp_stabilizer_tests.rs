use super::*;

fn pt(x: f64, y: f64, pressure: f32, time: f64) -> StrokePoint {
    StrokePoint { time, ..StrokePoint::new(x, y, pressure) }
}

fn run(cfg: &CspStabilizer, zoom: f32, pts: &[StrokePoint]) -> Vec<StrokePoint> {
    let mut s = Stabilizer::new(cfg, zoom);
    pts.iter().map(|p| s.push(*p)).collect()
}

#[test]
fn level_zero_passes_input_through() {
    let pts: Vec<_> = (0..5).map(|i| pt(f64::from(i) * 3.0, 1.0, 0.5, f64::from(i) * 8.0)).collect();
    assert_eq!(run(&CspStabilizer::default(), 1.0, &pts), pts);
}

#[test]
fn start_window_uses_half_the_smaller_of_level_and_stickiness() {
    let w = |level, stickiness| CspStabilizer { level, stickiness, ..Default::default() }.start_window();
    assert_eq!([w(1, 0), w(2, 0), w(10, 0), w(10, 8), w(40, 40), w(100, 100)], [1.0, 2.0, 2.0, 4.0, 15.0, 15.0]);
}

#[test]
fn window_ramps_by_half_a_sample_and_averages_fractionally() {
    let cfg = CspStabilizer { level: 10, ..Default::default() };
    let pts: Vec<_> = (0..4).map(|i| pt(f64::from(i) * 10.0, 0.0, 0.2 * (i + 1) as f32, f64::from(i) * 8.0)).collect();
    let out = run(&cfg, 1.0, &pts);
    // Pen-down: window 2 padded with the only sample.
    assert_eq!((out[0].x, out[0].pressure), (0.0, 0.2));
    // Window 2.5: newest two plus half of the (missing, so oldest-used) third.
    assert!((out[1].x - (10.0 + 0.0 + 0.5 * 0.0) / 2.5).abs() < 1e-9);
    // Window 3: samples 2, 1, 0.
    assert!((out[2].x - 10.0).abs() < 1e-9, "{}", out[2].x);
    assert!((out[2].pressure - 0.4).abs() < 1e-6, "pressure is averaged too");
    // Window 3.5: samples 3, 2, 1 and half of sample 0.
    assert!((out[3].x - (30.0 + 20.0 + 10.0) / 3.5).abs() < 1e-9);
    // Non-position fields follow the newest sample.
    assert_eq!(out[3].time, 24.0);
}

#[test]
fn adjust_by_speed_shrinks_for_fast_and_grows_for_slow_movement() {
    let cfg = CspStabilizer { level: 20, adjust_by_speed: true, ..Default::default() };
    // Slow: 1 px per sample grows the window by (140-1)/140/2 per sample up to the level.
    let slow: Vec<_> = (0..80).map(|i| pt(f64::from(i), 0.0, 1.0, f64::from(i) * 8.0)).collect();
    let mut s = Stabilizer::new(&cfg, 1.0);
    for p in &slow {
        s.push(*p);
    }
    assert_eq!(s.window, 20.0);
    // Fast: a 421 px move shrinks it by the full (unhalved) (K - d) / K, K = 2·20 + 100.
    s.push(pt(500.0, 0.0, 1.0, 650.0));
    assert!((s.window - (20.0 + (140.0 - 421.0) / 140.0)).abs() < 1e-9, "{}", s.window);
    for i in 1..20 {
        s.push(pt(500.0 + f64::from(i) * 400.0, 0.0, 1.0, 650.0 + f64::from(i) * 8.0));
    }
    assert_eq!(s.window, 2.0, "never below two samples");
    // Zoom converts document distance to screen pixels: the same move is slower zoomed out.
    let mut zoomed = Stabilizer::new(&cfg, 0.1);
    for p in &slow {
        zoomed.push(*p);
    }
    zoomed.push(pt(500.0, 0.0, 1.0, 650.0));
    assert_eq!(zoomed.window, 20.0, "42 screen px is slow");
    // A second-long pause passes the sample through unchanged.
    let late = pt(600.0, 0.0, 1.0, 9000.0);
    assert_eq!(s.push(late), late);
}

#[test]
fn old_samples_stop_counting() {
    let cfg = CspStabilizer { level: 30, ..Default::default() };
    let mut s = Stabilizer::new(&cfg, 1.0);
    for i in 0..40 {
        s.push(pt(0.0, 0.0, 1.0, f64::from(i)));
    }
    let out = s.push(pt(100.0, 0.0, 1.0, 2000.0));
    assert_eq!(out.x, 100.0, "only the newest sample is recent");
}

#[test]
fn bounded_limits_level_and_high_levels_of_speed_type_1_ramp() {
    assert_eq!(CspStabilizer { level: 5000, stickiness: 900, ..Default::default() }.bounded().level, MAX_LEVEL);
    // Above level 30 the path-following window does not apply: the plain ramp does.
    let typed = CspStabilizer { level: 40, adjust_by_speed: true, speed_type: 1, ..Default::default() };
    let pts: Vec<_> = (0..4).map(|i| pt(f64::from(i) * 400.0, 0.0, 1.0, f64::from(i) * 8.0)).collect();
    let mut s = Stabilizer::new(&typed, 1.0);
    for p in &pts {
        s.push(*p);
    }
    assert_eq!(s.window, 3.5);
}

#[test]
fn speed_type_1_widens_for_slow_straight_input_and_holds_the_level_for_fast_input() {
    let cfg = CspStabilizer { level: 6, adjust_by_speed: true, speed_type: 1, stickiness: 6 };
    let window_after = |step: f64, zigzag: f64, zoom: f32| {
        let mut s = Stabilizer::new(&cfg, zoom);
        for i in 0..200 {
            let y = if i % 2 == 0 { 0.0 } else { zigzag };
            s.push(pt(f64::from(i) * step, y, 1.0, f64::from(i) * 8.0));
        }
        s.window
    };
    // 0.5 px steps in a straight line: target (1)(2.0)(20) = 40, capped by the 30 samples examined.
    assert_eq!(window_after(0.5, 0.0, 1.0), 30.0);
    // Fast input stays at the level (never below it while drawing).
    assert_eq!(window_after(5.0, 0.0, 1.0), 6.0);
    // Zoom scales the step into screen pixels: 0.5 document px at 10x is fast.
    assert_eq!(window_after(0.5, 0.0, 10.0), 6.0);
    // A zigzag is slow but not straight: the window grows less than for the straight line.
    let zig = window_after(0.2, 1.0, 1.0);
    assert!((6.0..30.0).contains(&zig), "{zig}");
    // Still stabilizes towards the input and never panics on a single repeated sample.
    let mut s = Stabilizer::new(&cfg, 1.0);
    for _ in 0..50 {
        let out = s.push(pt(3.0, 4.0, 0.5, 0.0));
        assert_eq!((out.x, out.y), (3.0, 4.0));
    }
}

#[test]
fn stabilized_strokes_are_chunk_invariant_and_lag_behind_the_pointer() {
    use crate::{BrushSettings, Stroke, StrokeRenderer};
    let brush = BrushSettings {
        size: 6.0,
        pressure_size: false,
        csp_stabilizer: Some(CspStabilizer { level: 12, adjust_by_speed: true, ..Default::default() }),
        ..Default::default()
    };
    let pts: Vec<_> = (0..60).map(|i| pt(10.0 + f64::from(i) * 2.0, 20.0 + (f64::from(i) * 0.7).sin() * 6.0, 1.0, f64::from(i) * 8.0)).collect();
    let whole = crate::dabs(&Stroke { brush: brush.clone(), points: pts.clone() });
    let mut r = StrokeRenderer::new(&brush, None, 1.0).record_dabs();
    for chunk in pts.chunks(7) {
        r.push(chunk);
    }
    r.finish();
    assert_eq!(r.dabs(), whole.as_slice());
    let plain = crate::dabs(&Stroke { brush: BrushSettings { csp_stabilizer: None, ..brush }, points: pts.clone() });
    let (end, raw_end) = (whole.last().unwrap().center.x, plain.last().unwrap().center.x);
    assert!((end - raw_end).abs() < 0.5, "pen-up catches up with the release point: {end} vs {raw_end}");
}

#[test]
fn pen_up_repeats_then_drains_release_pressure() {
    let pts: Vec<_> = (0..30).map(|i| pt(f64::from(i) * 4.0, 0.0, 1.0, f64::from(i) * 8.0)).collect();
    for (stickiness, caught_up) in [(0, true), (8, false)] {
        let mut s = Stabilizer::new(&CspStabilizer { level: 10, stickiness, ..Default::default() }, 1.0);
        let lagging = pts.iter().map(|p| s.push(*p)).last().unwrap();
        assert!(lagging.x < 112.0);
        let mut tail = Vec::new();
        s.finish(&mut tail);
        let start = CspStabilizer { level: 10, stickiness, ..Default::default() }.start_window();
        assert!(!tail.is_empty() && tail.len() <= 212, "{stickiness}: {} repeats from window {start}", tail.len());
        assert_eq!(tail.last().unwrap().x == 116.0, caught_up, "{stickiness}: {:?}", tail.last());
    }
    let mut off = Stabilizer::new(&CspStabilizer::default(), 1.0);
    off.push(pts[0]);
    let mut tail = Vec::new();
    off.finish(&mut tail);
    assert!(tail.is_empty());
}

#[test]
fn sparse_fast_flick_finishes_near_the_release_instead_of_halfway() {
    let mut s = Stabilizer::new(&CspStabilizer { level: 10, stickiness: 4, ..Default::default() }, 1.0);
    s.push(pt(0.0, 0.0, 1.0, 0.0));
    assert!(s.push(pt(378.0, 0.0, 1.0, 8.0)).x < 200.0);
    let mut tail = Vec::new();
    s.finish(&mut tail);
    let end = tail.last().unwrap();
    assert!((end.x - 378.0).abs() < 2.0, "{end:?}");
    assert!(tail.iter().all(|p| p.x >= 0.0 && p.x <= 378.0 && p.pressure >= 0.0));
    assert!(tail.len() <= 202);
    let n = tail.len();
    s.finish(&mut tail);
    assert_eq!(tail.len(), n, "finishing twice adds no second release");
}

#[test]
fn release_drain_keeps_stationary_dot_pressure() {
    let mut s = Stabilizer::new(&CspStabilizer { level: 10, stickiness: 4, ..Default::default() }, 1.0);
    s.push(pt(4.0, 5.0, 1.0, 0.0));
    let mut tail = Vec::new();
    s.finish(&mut tail);
    assert!(tail.iter().all(|p| p.pressure == 1.0 && p.x == 4.0 && p.y == 5.0));
}

#[test]
fn sparse_input_averages_the_same_time_span_as_reference_rate_input() {
    // 茂み1: level 10, stickiness 4. A steady 600 px/s line at 125 Hz (reference) and at 60 Hz
    // (a Windows mouse, one move per frame): the brush should trail the pointer by about the
    // same time, not twice as long at the lower rate.
    let cfg = CspStabilizer { level: 10, stickiness: 4, ..Default::default() };
    let lag_ms = |interval: f64| {
        let speed = 0.6; // px per ms
        let pts: Vec<_> = (0..120).map(|i| pt(f64::from(i) * interval * speed, 0.0, 1.0, f64::from(i) * interval)).collect();
        let out = run(&cfg, 1.0, &pts);
        let (p, o) = (pts.last().unwrap(), out.last().unwrap());
        (p.x - o.x) / speed
    };
    let (reference, mouse) = (lag_ms(8.0), lag_ms(1000.0 / 60.0));
    assert!((reference - 36.0).abs() < 1.0, "reference-rate lag {reference}");
    assert!(mouse < reference * 1.35, "60 Hz lag {mouse} vs {reference}");
    // Untimed input (automation without times) keeps the sample-count window.
    let untimed: Vec<_> = (0..40).map(|i| pt(f64::from(i) * 10.0, 0.0, 1.0, 0.0)).collect();
    let out = run(&cfg, 1.0, &untimed);
    assert!((untimed.last().unwrap().x - out.last().unwrap().x - 45.0).abs() < 1e-6, "{}", out.last().unwrap().x);
    // Release still reaches the pointer at the sparse rate.
    let pts: Vec<_> = (0..30).map(|i| pt(f64::from(i) * 10.0, 0.0, 1.0, f64::from(i) * 16.7)).collect();
    let mut s = Stabilizer::new(&cfg, 1.0);
    for p in &pts {
        s.push(*p);
    }
    let mut tail = Vec::new();
    s.finish(&mut tail);
    assert!((tail.last().unwrap().x - 290.0).abs() < 2.0, "{:?}", tail.last());
}
