use super::*;

fn cfg(mode: CspInOutMode, start: Option<f64>, end: Option<f64>, by_speed: bool) -> CspInOut {
    CspInOut { mode, start, end, by_speed, targets: vec![CspInOutTarget { param: CspInOutParam::Size, minimum: 0.0 }] }
}

#[test]
fn factor_is_a_quarter_sine_from_each_end() {
    let t = InOutRuntime::finished(&cfg(CspInOutMode::Length, Some(20.0), Some(10.0), false), &[50.0, 50.0]).unwrap();
    assert_eq!(t.factor(0.0), 0.0);
    assert!((t.factor(10.0) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6, "sin 45°");
    assert_eq!(t.factor(50.0), 1.0);
    assert!((t.factor(95.0) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6, "5 px before a 10 px end");
    assert_eq!(t.factor(100.0), 0.0);
    // Minimum value of the target.
    let m = InOutRuntime::finished(
        &CspInOut { targets: vec![CspInOutTarget { param: CspInOutParam::Flow, minimum: 0.4 }], ..cfg(CspInOutMode::Length, Some(20.0), None, false) },
        &[100.0],
    )
    .unwrap();
    assert!((m.multiplier(CspInOutParam::Flow, 0.0) - 0.4).abs() < 1e-6);
    assert_eq!(m.multiplier(CspInOutParam::Size, 0.0), 1.0, "not a target");
}

#[test]
fn percentages_scale_with_the_path_and_overlong_lengths_fit() {
    let p = InOutRuntime::finished(&cfg(CspInOutMode::Percent, Some(0.3), Some(0.3), false), &[100.0, 100.0]).unwrap();
    assert_eq!((p.start, p.end), (60.0, 60.0));
    let long = InOutRuntime::finished(&cfg(CspInOutMode::Length, Some(300.0), Some(100.0), false), &[100.0]).unwrap();
    assert_eq!((long.start, long.end), (75.0, 25.0));
}

#[test]
fn by_speed_limits_lengths_to_the_first_and_last_input_segments() {
    // 50 px → at most 2 segments (50 · 0.04), from each end.
    let t = InOutRuntime::finished(&cfg(CspInOutMode::Length, Some(50.0), Some(50.0), true), &[3.0, 4.0, 5.0, 40.0, 40.0, 6.0, 7.0]).unwrap();
    assert_eq!((t.start, t.end), (7.0, 13.0));
    // Fast strokes: long segments reach the full length.
    let fast = InOutRuntime::finished(&cfg(CspInOutMode::Length, Some(50.0), None, true), &[30.0, 30.0, 30.0]).unwrap();
    assert_eq!(fast.start, 50.0);
}

#[test]
fn live_strokes_only_know_a_pixel_start() {
    assert!(InOutRuntime::live(&cfg(CspInOutMode::Length, Some(20.0), Some(10.0), false)).is_some_and(|t| t.end == 0.0 && t.factor(1e9) == 1.0));
    assert!(InOutRuntime::live(&cfg(CspInOutMode::Percent, Some(0.2), None, false)).is_none());
    assert!(InOutRuntime::live(&cfg(CspInOutMode::Length, None, Some(10.0), false)).is_none());
    assert!(InOutRuntime::finished(&CspInOut { targets: vec![], ..cfg(CspInOutMode::Length, Some(5.0), None, false) }, &[10.0]).is_none());
}

#[test]
fn bounded_rejects_hostile_values() {
    let b = CspInOut {
        mode: CspInOutMode::Percent,
        start: Some(f64::NAN),
        end: Some(7.0),
        by_speed: false,
        targets: vec![CspInOutTarget { param: CspInOutParam::Size, minimum: f32::INFINITY }, CspInOutTarget { param: CspInOutParam::Size, minimum: 0.5 }],
    }
    .bounded();
    assert_eq!((b.start, b.end), (None, Some(1.0)));
    assert_eq!(b.targets, vec![CspInOutTarget { param: CspInOutParam::Size, minimum: 0.0 }]);
    let t = InOutRuntime::finished(&cfg(CspInOutMode::Length, Some(1e300), Some(-4.0), false), &[f64::NAN, 10.0, f64::INFINITY]).unwrap();
    assert!(t.factor(5.0).is_finite());
}

#[test]
fn committed_strokes_taper_size_at_both_ends() {
    use crate::{BrushSettings, Stroke};
    let brush = BrushSettings {
        size: 20.0,
        pressure_size: false,
        spacing: 0.05,
        csp_in_out: Some(cfg(CspInOutMode::Length, Some(30.0), Some(30.0), false)),
        ..Default::default()
    };
    let pts: Vec<_> = (0..=20).map(|i| StrokePoint::new(f64::from(i) * 10.0, 0.0, 1.0)).collect();
    let dabs = crate::dabs(&Stroke { brush: brush.clone(), points: pts.clone() });
    let first = dabs.first().unwrap().radius;
    let middle = dabs[dabs.len() / 2].radius;
    let last = dabs.last().unwrap().radius;
    assert!(first < 1.0 && last < 1.0 && (middle - 10.0).abs() < 1e-4, "{first} {middle} {last}");
    // A live (streamed) stroke has the start taper but not the end.
    let mut r = crate::StrokeRenderer::new(&brush, None, 1.0).record_dabs();
    r.push(&pts);
    r.finish();
    assert!(r.dabs()[0].radius < 1.0 && (r.dabs().last().unwrap().radius - 10.0).abs() < 1e-4);
}
