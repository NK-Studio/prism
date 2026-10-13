//! Parametric ("live") shapes → paths: rectangles with corner radii, ellipses, polygons, stars
//! and lines. Knots run clockwise on screen (y down), starting at the top.

use prism_doc::{CornerStyle, Knot, LiveShape, Path, Subpath};
use prism_geom::Point;

/// Handle length of a quarter-circle cubic, `4/3·(√2 − 1)`.
pub const KAPPA: f64 = 0.552_284_749_830_793_4;

fn smooth(anchor: (f64, f64), inc: (f64, f64), out: (f64, f64)) -> Knot {
    Knot::smooth(Point::new(anchor.0, anchor.1), Point::new(inc.0, inc.1), Point::new(out.0, out.1))
}

/// Axis-aligned rectangle.
pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Path {
    Path::new(vec![Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])])
}

/// Clamps corner radii `[tl, tr, br, bl]` so adjacent radii never exceed a side (scaled down
/// uniformly, like CSS `border-radius`).
pub fn clamp_radii(w: f64, h: f64, radii: [f64; 4]) -> [f64; 4] {
    let r = radii.map(|v| v.max(0.0));
    let mut f: f64 = 1.0;
    for (a, b, side) in [(r[0], r[1], w), (r[1], r[2], h), (r[2], r[3], w), (r[3], r[0], h)] {
        if a + b > side && a + b > 0.0 {
            f = f.min(side / (a + b));
        }
    }
    r.map(|v| v * f)
}

/// Rectangle with corner radii `[top-left, top-right, bottom-right, bottom-left]`.
pub fn rounded_rect(x: f64, y: f64, w: f64, h: f64, radii: [f64; 4]) -> Path {
    let (w, h) = (w.abs(), h.abs());
    let [tl, tr, br, bl] = clamp_radii(w, h, radii);
    if tl <= 0.0 && tr <= 0.0 && br <= 0.0 && bl <= 0.0 {
        return rect(x, y, w, h);
    }
    let (x1, y1) = (x + w, y + h);
    let mut k = Vec::with_capacity(8);
    let corner = |p: (f64, f64)| Knot::corner(p.0, p.1);
    let push_arc = |k: &mut Vec<Knot>, start: (f64, f64), end: (f64, f64), c_start: (f64, f64), c_end: (f64, f64), r: f64| {
        if r <= 0.0 {
            k.push(corner(start));
            return;
        }
        k.push(Knot { anchor: Point::new(start.0, start.1), in_ctrl: Point::new(start.0, start.1), out_ctrl: Point::new(c_start.0, c_start.1), smooth: false });
        k.push(Knot { anchor: Point::new(end.0, end.1), in_ctrl: Point::new(c_end.0, c_end.1), out_ctrl: Point::new(end.0, end.1), smooth: false });
    };
    // Each corner arc from the end of one edge to the start of the next (clockwise).
    push_arc(&mut k, (x1 - tr, y), (x1, y + tr), (x1 - tr + KAPPA * tr, y), (x1, y + tr - KAPPA * tr), tr);
    push_arc(&mut k, (x1, y1 - br), (x1 - br, y1), (x1, y1 - br + KAPPA * br), (x1 - br + KAPPA * br, y1), br);
    push_arc(&mut k, (x + bl, y1), (x, y1 - bl), (x + bl - KAPPA * bl, y1), (x, y1 - bl + KAPPA * bl), bl);
    push_arc(&mut k, (x, y + tl), (x + tl, y), (x, y + tl - KAPPA * tl), (x + tl - KAPPA * tl, y), tl);
    // Start at the top-left arc end so the first knot is on the top edge.
    let n = k.len();
    k.rotate_left(n - 1);
    // Drop duplicated consecutive anchors (zero-length straight edges).
    let mut out: Vec<Knot> = Vec::with_capacity(k.len());
    for kn in k {
        if let Some(last) = out.last_mut()
            && (last.anchor.x - kn.anchor.x).abs() < 1e-12
            && (last.anchor.y - kn.anchor.y).abs() < 1e-12
        {
            last.out_ctrl = kn.out_ctrl;
            continue;
        }
        out.push(kn);
    }
    if out.len() > 1 {
        let (f, l) = (out[0], out[out.len() - 1]);
        if (f.anchor.x - l.anchor.x).abs() < 1e-12 && (f.anchor.y - l.anchor.y).abs() < 1e-12 {
            out[0].in_ctrl = l.in_ctrl;
            out.pop();
        }
    }
    Path::new(vec![Subpath { closed: true, knots: out, op: Default::default() }])
}

/// Ellipse inscribed in the rectangle.
pub fn ellipse(x: f64, y: f64, w: f64, h: f64) -> Path {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let knots = vec![
        smooth((cx, cy - ry), (cx - kx, cy - ry), (cx + kx, cy - ry)),
        smooth((cx + rx, cy), (cx + rx, cy - ky), (cx + rx, cy + ky)),
        smooth((cx, cy + ry), (cx + kx, cy + ry), (cx - kx, cy + ry)),
        smooth((cx - rx, cy), (cx - rx, cy + ky), (cx - rx, cy - ky)),
    ];
    Path::new(vec![Subpath { closed: true, knots, op: Default::default() }])
}

/// Regular polygon (or star when `star_ratio < 1`) inscribed in the rectangle's ellipse, first
/// vertex at the top.
pub fn polygon(x: f64, y: f64, w: f64, h: f64, sides: u32, star_ratio: f64) -> Path {
    let sides = sides.clamp(3, 100) as usize;
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let star = star_ratio > 0.0 && star_ratio < 1.0;
    let count = if star { sides * 2 } else { sides };
    let pts: Vec<(f64, f64)> = (0..count)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * i as f64 / count as f64;
            let k = if star && i % 2 == 1 { star_ratio } else { 1.0 };
            (cx + rx * k * a.cos(), cy + ry * k * a.sin())
        })
        .collect();
    Path::new(vec![Subpath::polygon(&pts)])
}

/// A line drawn as a filled bar of `weight` px from `a` to `b`.
pub fn line(a: (f64, f64), b: (f64, f64), weight: f64) -> Path {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = dx.hypot(dy);
    let hw = weight.max(0.0) / 2.0;
    let (nx, ny) = if l > 0.0 { (-dy / l * hw, dx / l * hw) } else { (0.0, hw) };
    Path::new(vec![Subpath::polygon(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)])])
}

/// Filled line with independently retained round-tip geometry. Square tips use the legacy path.
pub fn line_with_tips(a: [f64; 2], b: [f64; 2], weight: f64, round_tips: bool) -> Path {
    if !a.iter().chain(b.iter()).chain(std::iter::once(&weight)).all(|v| v.is_finite()) || weight < 0.0 {
        return Path::default();
    }
    if !round_tips {
        return line((a[0], a[1]), (b[0], b[1]), weight);
    }
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let length = dx.hypot(dy);
    if !length.is_finite() {
        return Path::default();
    }
    let (ux, uy) = if length > 1e-9 { (dx / length, dy / length) } else { (1.0, 0.0) };
    let r = weight / 2.0;
    rounded_rect(-r, -r, length + weight, weight, [r; 4]).transform(&prism_geom::Affine { m: [ux, uy, -uy, ux, a[0], a[1]] })
}

/// Straight shaft and open V head, with a 60° included angle. Invalid/zero lengths
/// produce no geometry; the head is shortened so its wings cannot pass the tail.
pub fn arrow(from: [f64; 2], to: [f64; 2], head_size: f64) -> Path {
    arrow_with_angle(from, to, head_size, 60.0)
}

/// Open arrow with an included head angle of 10–160 degrees.
pub fn arrow_with_angle(from: [f64; 2], to: [f64; 2], head_size: f64, head_angle: f64) -> Path {
    let [ax, ay] = from;
    let [bx, by] = to;
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx.hypot(dy);
    if ![ax, ay, bx, by, length, head_size].iter().all(|v| v.is_finite())
        || length <= 1e-9
        || head_size <= 0.0
        || !head_angle.is_finite()
        || !(10.0..=160.0).contains(&head_angle)
    {
        return Path::default();
    }
    let (ux, uy) = (dx / length, dy / length);
    let wing = head_size.min(length);
    let half = head_angle.to_radians() / 2.0;
    let back = wing * half.cos();
    let side = wing * half.sin();
    Path::new(vec![
        Subpath::polyline(&[(ax, ay), (bx, by)]),
        Subpath::polyline(&[(bx - ux * back - uy * side, by - uy * back + ux * side), (bx, by), (bx - ux * back + uy * side, by - uy * back - ux * side)]),
    ])
}

/// Rectangle with chamfered corners: each corner cut by a straight line from `size` along one
/// side to `size` along the other, sizes `[top-left, top-right, bottom-right, bottom-left]`
/// (clamped like radii, so cuts never overlap).
pub fn chamfered_rect(x: f64, y: f64, w: f64, h: f64, sizes: [f64; 4]) -> Path {
    let (w, h) = (w.abs(), h.abs());
    let [tl, tr, br, bl] = clamp_radii(w, h, sizes);
    let (x1, y1) = (x + w, y + h);
    let points = [(x + tl, y), (x1 - tr, y), (x1, y + tr), (x1, y1 - br), (x1 - br, y1), (x + bl, y1), (x, y1 - bl), (x, y + tl)];
    // Zero-size corners and cuts meeting mid-side give repeated points: keep one of each.
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_none_or(|q| (q.0 - p.0).abs() > 1e-9 || (q.1 - p.1).abs() > 1e-9) {
            out.push(p);
        }
    }
    if out.len() > 1 && out.first().zip(out.last()).is_some_and(|(a, b)| (a.0 - b.0).abs() <= 1e-9 && (a.1 - b.1).abs() <= 1e-9) {
        out.pop();
    }
    Path::new(vec![Subpath::polygon(&out)])
}

/// The path of a live shape.
pub fn live_path(s: &LiveShape) -> Path {
    match s {
        LiveShape::Rect { rect: [x, y, w, h], radii, corners: CornerStyle::Round } => rounded_rect(*x, *y, *w, *h, *radii),
        LiveShape::Rect { rect: [x, y, w, h], radii, corners: CornerStyle::Chamfer } => chamfered_rect(*x, *y, *w, *h, *radii),
        LiveShape::Ellipse { rect: [x, y, w, h] } => ellipse(*x, *y, *w, *h),
        LiveShape::Polygon { rect: [x, y, w, h], sides, star_ratio } => polygon(*x, *y, *w, *h, *sides, *star_ratio),
        LiveShape::Arrow { from, to, head_size, head_angle } => arrow_with_angle(*from, *to, *head_size, *head_angle),
        LiveShape::Line { from, to, weight, round_tips } => line_with_tips(*from, *to, *weight, *round_tips),
    }
}

#[cfg(test)]
mod chamfer_tests {
    use super::*;

    fn anchors(p: &Path) -> Vec<(f64, f64)> {
        p.subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect()
    }

    #[test]
    fn chamfered_corners_are_straight_cuts() {
        let p = chamfered_rect(0.0, 0.0, 100.0, 40.0, [10.0; 4]);
        assert!(p.subpaths[0].closed);
        assert_eq!(anchors(&p), [(10.0, 0.0), (90.0, 0.0), (100.0, 10.0), (100.0, 30.0), (90.0, 40.0), (10.0, 40.0), (0.0, 30.0), (0.0, 10.0)]);
        // Straight: every knot is a corner, no curve handles.
        assert!(p.subpaths[0].knots.iter().all(|k| k.in_ctrl == k.anchor && k.out_ctrl == k.anchor));
        // Through live_path.
        let live = LiveShape::Rect { rect: [0.0, 0.0, 100.0, 40.0], radii: [10.0; 4], corners: CornerStyle::Chamfer };
        assert_eq!(live_path(&live), p);
    }

    #[test]
    fn chamfer_sizes_per_corner_and_clamped() {
        // Only the top-right corner cut: five points.
        let p = chamfered_rect(0.0, 0.0, 50.0, 50.0, [0.0, 20.0, 0.0, 0.0]);
        assert_eq!(anchors(&p), [(0.0, 0.0), (30.0, 0.0), (50.0, 20.0), (50.0, 50.0), (0.0, 50.0)]);
        // No cuts: the plain rectangle's four corners.
        assert_eq!(anchors(&chamfered_rect(0.0, 0.0, 8.0, 4.0, [0.0; 4])).len(), 4);
        // Oversized cuts meet mid-side (a diamond on a square), never cross.
        assert_eq!(anchors(&chamfered_rect(0.0, 0.0, 10.0, 10.0, [100.0; 4])), [(5.0, 0.0), (10.0, 5.0), (5.0, 10.0), (0.0, 5.0)]);
        // Degenerate input never panics.
        for r in [[f64::NAN; 4], [-5.0; 4], [f64::INFINITY; 4]] {
            let _ = chamfered_rect(0.0, 0.0, 10.0, 10.0, r);
        }
        let _ = chamfered_rect(0.0, 0.0, 0.0, 0.0, [1.0; 4]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rect_knots() {
        let p = rounded_rect(0.0, 0.0, 100.0, 50.0, [10.0; 4]);
        assert_eq!(p.subpaths[0].knots.len(), 8);
        let sharp = rounded_rect(0.0, 0.0, 100.0, 50.0, [0.0; 4]);
        assert_eq!(sharp.subpaths[0].knots.len(), 4);
        let mixed = rounded_rect(0.0, 0.0, 100.0, 50.0, [10.0, 0.0, 10.0, 0.0]);
        assert_eq!(mixed.subpaths[0].knots.len(), 6);
        assert_eq!(clamp_radii(10.0, 10.0, [10.0; 4]), [5.0; 4]);
    }

    #[test]
    fn star_vertices() {
        assert_eq!(polygon(0.0, 0.0, 10.0, 10.0, 5, 0.5).subpaths[0].knots.len(), 10);
        assert_eq!(polygon(0.0, 0.0, 10.0, 10.0, 6, 1.0).subpaths[0].knots.len(), 6);
        let top = polygon(0.0, 0.0, 10.0, 10.0, 3, 1.0).subpaths[0].knots[0].anchor;
        assert!((top.x - 5.0).abs() < 1e-9 && top.y.abs() < 1e-9);
    }
}

#[cfg(test)]
mod arrow_tests {
    use super::*;
    #[test]
    fn arrow_is_open_sixty_degree_and_never_extends_behind_tail() {
        for angle in 0..16 {
            let angle = angle as f64 * std::f64::consts::TAU / 16.0;
            for length in [0.1, 2.0, 100.0] {
                let (ux, uy) = (angle.cos(), angle.sin());
                let p = arrow([10.0, 20.0], [10.0 + ux * length, 20.0 + uy * length], 16.0);
                assert_eq!(p.subpaths.len(), 2);
                assert!(p.subpaths.iter().all(|s| !s.closed));
                let k = &p.subpaths[1].knots;
                let (a, b, c) = (k[0].anchor, k[1].anchor, k[2].anchor);
                let dot = (a.x - b.x) * (c.x - b.x) + (a.y - b.y) * (c.y - b.y);
                assert!((dot / (a.x - b.x).hypot(a.y - b.y) / (c.x - b.x).hypot(c.y - b.y) - 0.5).abs() < 1e-9);
                for q in [a, c] {
                    assert!((q.x - 10.0) * ux + (q.y - 20.0) * uy >= -1e-9);
                }
            }
        }
        assert!(arrow([0.0; 2], [0.0; 2], 16.0).subpaths.is_empty());
        assert!(arrow([0.0; 2], [f64::INFINITY, 0.0], 16.0).subpaths.is_empty());
    }
}

#[cfg(test)]
mod arrow_angle_tests {
    use super::*;
    #[test]
    fn variable_angles_have_equal_wing_lengths_and_roundtrip_old_metadata() {
        for angle in [10.0_f64, 30.0, 60.0, 100.0, 160.0] {
            let p = arrow_with_angle([0.0, 0.0], [100.0, 0.0], 20.0, angle);
            let k = &p.subpaths[1].knots;
            let a = k[0].anchor;
            let c = k[2].anchor;
            assert!(((a.x - 100.0).hypot(a.y) - 20.0).abs() < 1e-9);
            assert!(((c.x - 100.0).hypot(c.y) - 20.0).abs() < 1e-9);
            let dot = ((a.x - 100.0) * (c.x - 100.0) + a.y * c.y) / 400.0;
            assert!((dot - angle.to_radians().cos()).abs() < 1e-9);
        }
        for angle in [0.0, 161.0, f64::NAN, f64::INFINITY] {
            assert!(arrow_with_angle([0.0; 2], [100.0, 0.0], 20.0, angle).subpaths.is_empty());
        }
    }
}

#[cfg(test)]
mod line_tip_tests {
    use super::*;
    #[test]
    fn square_is_legacy_and_round_is_a_capsule_in_every_direction() {
        for angle in 0..16 {
            let a = angle as f64 * std::f64::consts::TAU / 16.0;
            for length in [0.0, 0.1, 100.0] {
                let to = [20.0 + length * a.cos(), 30.0 + length * a.sin()];
                assert_eq!(line_with_tips([20.0, 30.0], to, 12.0, false), line((20.0, 30.0), (to[0], to[1]), 12.0));
                let p = line_with_tips([20.0, 30.0], to, 12.0, true);
                assert_eq!(p.subpaths.len(), 1);
                assert!(p.subpaths[0].closed);
                assert_eq!(p.subpaths[0].knots.len(), if length == 0.0 { 4 } else { 6 });
                assert!(p.subpaths[0].knots.iter().all(|k| k.anchor.x.is_finite() && k.anchor.y.is_finite()));
            }
        }
        let p = line_with_tips([20.0, 30.0], [120.0, 30.0], 12.0, true);
        let xs: Vec<_> = p.subpaths[0].knots.iter().map(|k| k.anchor.x).collect();
        assert_eq!(xs.iter().copied().fold(f64::INFINITY, f64::min), 14.0);
        assert_eq!(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max), 126.0);
        assert!(line_with_tips([f64::NAN, 0.0], [1.0, 1.0], 12.0, true).subpaths.is_empty());
        assert!(line_with_tips([0.0; 2], [1.0; 2], -1.0, true).subpaths.is_empty());
    }
}
