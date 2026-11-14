//! CLIP STUDIO "Post correction" (`BrushUseRevision` / `BrushRevision`, property 1151): the line
//! is smoothed once more after the pen is lifted, on top of the live stabilization.
//!
//! Prism's own smoothing of the committed input path: every point moves to the average of the
//! path within a window around it (measured along the path), twice, which rounds wobbles
//! shorter than the window and leaves long curves alone. The window shrinks to nothing at both
//! ends, so a stroke still starts and ends where it was drawn. Pressure, tilt, rotation and
//! time stay with their points: only the positions change. CLIP STUDIO's own curve fit (its
//! "Bezier curve" option) is not traced; the window per level is Prism's choice
//! (docs/sut-analysis.md › Post correction).

use crate::StrokePoint;

/// The largest level (CLIP STUDIO's slider ends at 100).
pub const MAX_LEVEL: f32 = 100.0;
/// Screen pixels of smoothing window (each side) per level.
const WINDOW_PER_LEVEL: f64 = 0.6;
/// Strokes longer than this many points are left as drawn (bounded work).
const MAX_POINTS: usize = 200_000;

/// `pts` smoothed for post correction `level` (0..=100) at view `zoom`: CLIP STUDIO adjusts the
/// correction by the display scale, so the window is in screen pixels. A level of 0, fewer than
/// three points, or any non-finite position returns the points unchanged.
pub fn apply(level: f32, zoom: f32, pts: &[StrokePoint]) -> Vec<StrokePoint> {
    let mut out = pts.to_vec();
    let level = if level.is_finite() { level.clamp(0.0, MAX_LEVEL) } else { 0.0 };
    let zoom = if zoom.is_finite() && zoom > 0.0 { f64::from(zoom).clamp(1e-3, 1e3) } else { 1.0 };
    if level <= 0.0 || pts.len() < 3 || pts.len() > MAX_POINTS || pts.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return out;
    }
    let window = f64::from(level) * WINDOW_PER_LEVEL / zoom;
    for _ in 0..2 {
        smooth_once(&mut out, window);
    }
    out
}

/// One box-average pass over the positions, by arc length.
fn smooth_once(pts: &mut [StrokePoint], window: f64) {
    let n = pts.len();
    // Arc length at each point, and prefix sums of the positions.
    let mut s = Vec::with_capacity(n);
    let (mut sx, mut sy) = (Vec::with_capacity(n + 1), Vec::with_capacity(n + 1));
    let mut len = 0.0;
    sx.push(0.0);
    sy.push(0.0);
    for (i, p) in pts.iter().enumerate() {
        if let Some(q) = i.checked_sub(1).and_then(|j| pts.get(j)) {
            len += (p.x - q.x).hypot(p.y - q.y);
        }
        s.push(len);
        sx.push(sx.last().copied().unwrap_or(0.0) + p.x);
        sy.push(sy.last().copied().unwrap_or(0.0) + p.y);
    }
    if !(len.is_finite() && len > 0.0) {
        return;
    }
    let (mut lo, mut hi) = (0usize, 0usize);
    for i in 0..n {
        let Some(&at) = s.get(i) else { continue };
        // The ends stay put: the window never reaches past either end of the path.
        let w = window.min(at).min(len - at);
        while s.get(lo).is_some_and(|v| *v < at - w) {
            lo += 1;
        }
        while s.get(hi + 1).is_some_and(|v| *v <= at + w) {
            hi += 1;
        }
        let (lo, hi) = (lo.min(i), hi.max(i));
        let count = (hi - lo + 1) as f64;
        if let (Some(p), Some(x1), Some(x0), Some(y1), Some(y0)) = (pts.get_mut(i), sx.get(hi + 1), sx.get(lo), sy.get(hi + 1), sy.get(lo)) {
            p.x = (x1 - x0) / count;
            p.y = (y1 - y0) / count;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wobbly() -> Vec<StrokePoint> {
        (0..=200).map(|i| StrokePoint::new(f64::from(i), 50.0 + if i % 2 == 0 { 1.0 } else { -1.0 }, 0.25 + f32::from(i as u8) / 400.0)).collect()
    }

    fn wobble(pts: &[StrokePoint]) -> f64 {
        pts.get(20..180).unwrap_or(&[]).iter().map(|p| (p.y - 50.0).abs()).fold(0.0, f64::max)
    }

    #[test]
    fn smooths_wobbles_and_keeps_the_ends_and_the_pressure() {
        let raw = wobbly();
        let out = apply(10.0, 1.0, &raw);
        assert_eq!(out.len(), raw.len());
        assert!(wobble(&out) < 0.5, "{}", wobble(&out));
        assert!(wobble(&raw) >= 1.0);
        for (a, b) in [(raw.first(), out.first()), (raw.last(), out.last())] {
            assert_eq!(a.map(|p| (p.x, p.y)), b.map(|p| (p.x, p.y)));
        }
        assert!(raw.iter().zip(&out).all(|(a, b)| a.pressure == b.pressure && a.time == b.time));
        // A higher level smooths more; zooming in smooths less (the window is in screen pixels).
        assert!(wobble(&apply(1.0, 1.0, &raw)) > wobble(&apply(10.0, 1.0, &raw)));
        assert!(wobble(&apply(10.0, 16.0, &raw)) > wobble(&apply(10.0, 1.0, &raw)));
        // A straight line is unchanged.
        let line: Vec<_> = (0..50).map(|i| StrokePoint::new(f64::from(i) * 2.0, 10.0, 1.0)).collect();
        assert!(apply(100.0, 1.0, &line).iter().zip(&line).all(|(a, b)| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9));
    }

    #[test]
    fn off_short_and_hostile_input_is_returned_unchanged() {
        let raw = wobbly();
        for (level, zoom) in [(0.0, 1.0), (-5.0, 1.0), (f32::NAN, 1.0)] {
            assert_eq!(apply(level, zoom, &raw), raw, "{level} {zoom}");
        }
        for zoom in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(apply(1e9, zoom, &raw).iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{zoom}");
        }
        let two = &raw[..2];
        assert_eq!(apply(50.0, 1.0, two), two);
        assert!(apply(50.0, 1.0, &[]).is_empty());
        let mut bad = raw.clone();
        bad[7].x = f64::NAN;
        assert!(apply(50.0, 1.0, &bad)[8].y == bad[8].y);
        // Every point in one place: nothing to smooth, nothing divides by zero.
        let dot = vec![StrokePoint::new(5.0, 5.0, 1.0); 10];
        assert_eq!(apply(50.0, 1.0, &dot), dot);
    }
}
