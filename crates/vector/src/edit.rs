//! Point-level path editing, Photoshop's Direct Selection and Convert Point tools: move anchors,
//! drag direction handles, reshape a segment by dragging it, convert between smooth and corner
//! points, and hit-test a path's parts. Pure geometry on [`Path`]; the engine's `path.*` edit
//! commands and the canvas previews share it, so a preview is exactly what the command commits.
//!
//! Knots are addressed as `[subpath, knot]`. Every edit validates its indices and returns an
//! error message instead of panicking.

use prism_doc::{Knot, Path};
use prism_geom::Point;

/// One of a knot's two direction handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// The control point of the segment arriving at the knot.
    In,
    /// The control point of the segment leaving the knot.
    Out,
}

impl Handle {
    pub fn parse(s: &str) -> Option<Handle> {
        match s {
            "in" => Some(Handle::In),
            "out" => Some(Handle::Out),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Handle::In => "in",
            Handle::Out => "out",
        }
    }
}

/// The part of a path under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Anchor([usize; 2]),
    Handle([usize; 2], Handle),
    /// The segment leaving knot `[subpath, knot]`, at curve parameter `t`.
    Segment([usize; 2], f64),
}

fn add(p: Point, d: [f64; 2]) -> Point {
    Point::new(p.x + d[0], p.y + d[1])
}

fn dist(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn knot_mut(path: &mut Path, [s, k]: [usize; 2]) -> Result<&mut Knot, String> {
    path.subpaths.get_mut(s).and_then(|sp| sp.knots.get_mut(k)).ok_or_else(|| format!("no knot {k} in subpath {s}"))
}

/// The knot ending the segment that leaves `[s, k]` (wrapping on closed subpaths).
fn segment_end(path: &Path, [s, k]: [usize; 2]) -> Result<[usize; 2], String> {
    let sp = path.subpaths.get(s).ok_or_else(|| format!("no subpath {s}"))?;
    let n = sp.knots.len();
    match k.checked_add(1) {
        Some(next) if k < n && next < n => Ok([s, next]),
        Some(_) if k < n && sp.closed && n > 1 => Ok([s, 0]),
        _ => Err(format!("no segment leaves knot {k} of subpath {s}")),
    }
}

fn handle_mut(k: &mut Knot, h: Handle) -> &mut Point {
    match h {
        Handle::In => &mut k.in_ctrl,
        Handle::Out => &mut k.out_ctrl,
    }
}

fn handle(k: &Knot, h: Handle) -> Point {
    match h {
        Handle::In => k.in_ctrl,
        Handle::Out => k.out_ctrl,
    }
}

fn other(h: Handle) -> Handle {
    match h {
        Handle::In => Handle::Out,
        Handle::Out => Handle::In,
    }
}

/// A smooth knot keeps its handles collinear: point handle `h`'s opposite away from it, keeping
/// the opposite's length (a retracted handle stays retracted, as in Photoshop).
fn keep_smooth(k: &mut Knot, h: Handle) {
    if !k.smooth {
        return;
    }
    let (a, moved) = (k.anchor, handle(k, h));
    let opp = handle_mut(k, other(h));
    let (len, d) = (dist(*opp, a), dist(moved, a));
    if len > 1e-9 && d > 1e-9 {
        *opp = Point::new(a.x - (moved.x - a.x) / d * len, a.y - (moved.y - a.y) / d * len);
    }
}

/// Moves anchors (with their handles) by `d`. Each knot moves once, however often it is listed.
pub fn move_anchors(path: &mut Path, anchors: &[[usize; 2]], d: [f64; 2]) -> Result<(), String> {
    let mut list = anchors.to_vec();
    list.sort_unstable();
    list.dedup();
    for r in &list {
        knot_mut(path, *r)?;
    }
    for r in list {
        let k = knot_mut(path, r)?;
        *k = Knot { anchor: add(k.anchor, d), in_ctrl: add(k.in_ctrl, d), out_ctrl: add(k.out_ctrl, d), smooth: k.smooth };
    }
    Ok(())
}

/// Puts handle `h` of a knot at `to`. A smooth knot turns its other handle to stay collinear,
/// unless `independent` (Convert Point / ⌥), which makes it a corner whose handles move apart.
pub fn move_handle(path: &mut Path, knot: [usize; 2], h: Handle, to: Point, independent: bool) -> Result<(), String> {
    let k = knot_mut(path, knot)?;
    *handle_mut(k, h) = to;
    if independent {
        k.smooth = false;
    }
    keep_smooth(k, h);
    Ok(())
}

/// Convert Point: without `out`, a corner with retracted handles (a click on an anchor); with
/// `out`, a smooth knot whose handles are `out` and its mirror (a drag from an anchor).
pub fn convert_point(path: &mut Path, knot: [usize; 2], out: Option<Point>) -> Result<(), String> {
    let k = knot_mut(path, knot)?;
    let a = k.anchor;
    *k = match out {
        Some(o) => Knot::smooth(a, Point::new(2.0 * a.x - o.x, 2.0 * a.y - o.y), o),
        None => Knot::corner(a.x, a.y),
    };
    Ok(())
}

/// Drags the segment leaving `knot` at curve parameter `t` by `d`. A straight segment (both
/// handles retracted) moves with its two anchors; a curve reshapes so the point at `t` follows
/// the pointer exactly, with the nearer handle moving more. Smooth end knots stay smooth.
pub fn bend_segment(path: &mut Path, knot: [usize; 2], t: f64, d: [f64; 2]) -> Result<(), String> {
    let end = segment_end(path, knot)?;
    let retracted = |k: &Knot, h: Handle| dist(handle(k, h), k.anchor) < 1e-9;
    let a = *knot_mut(path, knot)?;
    let b = *knot_mut(path, end)?;
    if retracted(&a, Handle::Out) && retracted(&b, Handle::In) {
        return move_anchors(path, &[knot, end], d);
    }
    // B(t) moves by 3(1−t)²t·Δc1 + 3(1−t)t²·Δc2. With Δc1 = (1−t)·s·d and Δc2 = t·s·d that is
    // 3t(1−t)((1−t)² + t²)·s·d, so s below makes it exactly d. The ends are clamped: a drag right
    // at an anchor would need an unbounded handle move.
    let t = if t.is_finite() { t.clamp(0.05, 0.95) } else { 0.5 };
    let u = 1.0 - t;
    let s = 1.0 / (3.0 * t * u * (u * u + t * t));
    let (w1, w2) = (u * s, t * s);
    let ka = knot_mut(path, knot)?;
    ka.out_ctrl = add(ka.out_ctrl, [d[0] * w1, d[1] * w1]);
    keep_smooth(ka, Handle::Out);
    let kb = knot_mut(path, end)?;
    kb.in_ctrl = add(kb.in_ctrl, [d[0] * w2, d[1] * w2]);
    keep_smooth(kb, Handle::In);
    Ok(())
}

/// Every anchor of subpath `s` (⌥-click with Direct Selection selects the whole subpath).
pub fn subpath_anchors(path: &Path, s: usize) -> Vec<[usize; 2]> {
    path.subpaths.get(s).map_or_else(Vec::new, |sp| (0..sp.knots.len()).map(|k| [s, k]).collect())
}

/// Anchors inside the box `[x0, y0, x1, y1]` (a Direct Selection marquee), in path order.
pub fn anchors_in(path: &Path, [x0, y0, x1, y1]: [f64; 4]) -> Vec<[usize; 2]> {
    let (x0, x1, y0, y1) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
    let mut out = Vec::new();
    for (s, sp) in path.subpaths.iter().enumerate() {
        for (k, kn) in sp.knots.iter().enumerate() {
            if (x0..=x1).contains(&kn.anchor.x) && (y0..=y1).contains(&kn.anchor.y) {
                out.push([s, k]);
            }
        }
    }
    out
}

fn eval(seg: &[Point; 4], t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(a * seg[0].x + b * seg[1].x + c * seg[2].x + d * seg[3].x, a * seg[0].y + b * seg[1].y + c * seg[2].y + d * seg[3].y)
}

/// The part of `path` within `tol` of `p`: one of the `handles` shown (they sit on top), else an
/// anchor, else a segment; the nearest wins within each kind.
pub fn hit(path: &Path, p: Point, tol: f64, handles: &[([usize; 2], Handle)]) -> Option<Hit> {
    let nearest = |it: &mut dyn Iterator<Item = (f64, Hit)>| it.filter(|(d, _)| *d <= tol).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, h)| h);
    let knot = |r: [usize; 2]| path.subpaths.get(r[0]).and_then(|sp| sp.knots.get(r[1]));
    let on_handle = nearest(&mut handles.iter().filter_map(|&(r, h)| {
        let k = knot(r)?;
        Some((dist(handle(k, h), p), Hit::Handle(r, h)))
    }));
    if on_handle.is_some() {
        return on_handle;
    }
    let anchors =
        path.subpaths.iter().enumerate().flat_map(|(s, sp)| sp.knots.iter().enumerate().map(move |(k, kn)| (dist(kn.anchor, p), Hit::Anchor([s, k]))));
    if let Some(h) = nearest(&mut anchors.into_iter()) {
        return Some(h);
    }
    // Segments: the nearest of 64 samples, refined once around it (plenty at screen tolerances).
    const N: usize = 64;
    let mut segs = path.subpaths.iter().enumerate().flat_map(|(s, sp)| {
        sp.segments().into_iter().enumerate().map(move |(k, seg)| {
            let best = (0..=N).map(|i| i as f64 / N as f64).map(|t| (dist(eval(&seg, t), p), t)).min_by(|a, b| a.0.total_cmp(&b.0)).unwrap_or((f64::MAX, 0.5));
            let step = 1.0 / N as f64;
            let fine = (0..=16)
                .map(|i| (best.1 - step + 2.0 * step * i as f64 / 16.0).clamp(0.0, 1.0))
                .map(|t| (dist(eval(&seg, t), p), t))
                .fold(best, |a, b| if b.0 < a.0 { b } else { a });
            (fine.0, Hit::Segment([s, k], fine.1))
        })
    });
    nearest(&mut segs)
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// Adds an anchor on the segment leaving `knot` at curve parameter `t` (the Pen's Auto Add, Add
/// Anchor Point). The curve is split exactly, so the path's shape doesn't change: a straight
/// segment gets a corner point, a curve a smooth one. Returns the new knot.
pub fn add_anchor(path: &mut Path, knot: [usize; 2], t: f64) -> Result<[usize; 2], String> {
    if !t.is_finite() {
        return Err("`t` must be a finite number".into());
    }
    let t = t.clamp(0.0, 1.0);
    let end = segment_end(path, knot)?;
    let a = *knot_mut(path, knot)?;
    let b = *knot_mut(path, end)?;
    let (p0, p1, p2, p3) = (a.anchor, a.out_ctrl, b.in_ctrl, b.anchor);
    let (q0, q1, q2) = (lerp(p0, p1, t), lerp(p1, p2, t), lerp(p2, p3, t));
    let (r0, r1) = (lerp(q0, q1, t), lerp(q1, q2, t));
    let m = lerp(r0, r1, t);
    let straight = dist(p1, p0) < 1e-9 && dist(p2, p3) < 1e-9;
    let new = if straight { Knot::corner(m.x, m.y) } else { Knot::smooth(m, r0, r1) };
    if !straight {
        knot_mut(path, knot)?.out_ctrl = q0;
        knot_mut(path, end)?.in_ctrl = q2;
    }
    // `segment_end` checked `knot[1] < len`, so this is at most `len` (the closing segment's
    // new knot goes last).
    let at = knot[1].saturating_add(1);
    let sp = path.subpaths.get_mut(knot[0]).ok_or_else(|| format!("no subpath {}", knot[0]))?;
    if at > sp.knots.len() {
        return Err(format!("no segment leaves knot {} of subpath {}", knot[1], knot[0]));
    }
    sp.knots.insert(at, new);
    Ok([knot[0], at])
}

/// Deletes an anchor (the Pen's Auto Delete, Delete Anchor Point): its neighbours join, keeping
/// their own handles. A subpath left without anchors is removed.
pub fn delete_anchor(path: &mut Path, [s, k]: [usize; 2]) -> Result<(), String> {
    let sp = path.subpaths.get_mut(s).ok_or_else(|| format!("no subpath {s}"))?;
    if k >= sp.knots.len() {
        return Err(format!("no knot {k} in subpath {s}"));
    }
    sp.knots.remove(k);
    if sp.knots.is_empty() {
        path.subpaths.remove(s);
    }
    Ok(())
}

/// An edge or centre line that path components align or distribute on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

impl Edge {
    pub fn parse(s: &str) -> Option<Edge> {
        Some(match s {
            "left" => Edge::Left,
            "hcenter" => Edge::HCenter,
            "right" => Edge::Right,
            "top" => Edge::Top,
            "vcenter" => Edge::VCenter,
            "bottom" => Edge::Bottom,
            _ => return None,
        })
    }
    /// Whether aligning on this edge moves components horizontally.
    fn horizontal(self) -> bool {
        matches!(self, Edge::Left | Edge::HCenter | Edge::Right)
    }
    /// This edge's coordinate in bounds `[x0, y0, x1, y1]`.
    fn of(self, b: [f64; 4]) -> f64 {
        match self {
            Edge::Left => b[0],
            Edge::HCenter => (b[0] + b[2]) / 2.0,
            Edge::Right => b[2],
            Edge::Top => b[1],
            Edge::VCenter => (b[1] + b[3]) / 2.0,
            Edge::Bottom => b[3],
        }
    }
    fn offset(self, d: f64) -> [f64; 2] {
        if self.horizontal() { [d, 0.0] } else { [0.0, d] }
    }
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Bounds `[x0, y0, x1, y1]` of subpath `s`'s outline (anchors exactly, curves sampled).
pub fn subpath_bounds(path: &Path, s: usize) -> Option<[f64; 4]> {
    let sp = path.subpaths.get(s)?;
    let mut pts: Vec<Point> = sp.knots.iter().map(|k| k.anchor).collect();
    for seg in sp.segments() {
        pts.extend((1..32).map(|i| eval(&seg, f64::from(i) / 32.0)));
    }
    let first = pts.first()?;
    Some(pts.iter().fold([first.x, first.y, first.x, first.y], |b, p| union(b, [p.x, p.y, p.x, p.y])))
}

/// The shape components (see [`Path::components`]) holding any of subpaths `subs`, in path
/// order. Photoshop's path operations, alignment and arrangement act on whole components, so a
/// custom shape's holes stay with it.
pub fn components_of(path: &Path, subs: &[usize]) -> Result<Vec<std::ops::Range<usize>>, String> {
    if let Some(bad) = subs.iter().find(|&&s| s >= path.subpaths.len()) {
        return Err(format!("no subpath {bad}"));
    }
    Ok(path.components().into_iter().filter(|r| subs.iter().any(|s| r.contains(s))).collect())
}

fn component_bounds(path: &Path, r: &std::ops::Range<usize>) -> Option<[f64; 4]> {
    r.clone().filter_map(|s| subpath_bounds(path, s)).reduce(union)
}

fn move_component(path: &mut Path, r: &std::ops::Range<usize>, d: [f64; 2]) -> Result<(), String> {
    let anchors: Vec<[usize; 2]> = r.clone().flat_map(|s| subpath_anchors(path, s)).collect();
    if anchors.is_empty() || d == [0.0, 0.0] {
        return Ok(());
    }
    move_anchors(path, &anchors, d)
}

/// Aligns the components holding subpaths `subs` on `edge` of `reference` (the canvas), or of
/// their combined bounds when `None` (Photoshop's Path Alignment, "Align To: Selection").
pub fn align_components(path: &mut Path, subs: &[usize], edge: Edge, reference: Option<[f64; 4]>) -> Result<(), String> {
    let comps: Vec<_> = components_of(path, subs)?.into_iter().filter_map(|r| component_bounds(path, &r).map(|b| (r, b))).collect();
    let Some(all) = comps.iter().map(|(_, b)| *b).reduce(union) else { return Err("no path components to align".into()) };
    let target = edge.of(reference.unwrap_or(all));
    for (r, b) in &comps {
        move_component(path, r, edge.offset(target - edge.of(*b)))?;
    }
    Ok(())
}

/// Spaces the components holding subpaths `subs` evenly by `edge`, between the outermost two.
pub fn distribute_components(path: &mut Path, subs: &[usize], edge: Edge) -> Result<(), String> {
    let mut comps: Vec<_> = components_of(path, subs)?.into_iter().filter_map(|r| component_bounds(path, &r).map(|b| (r, edge.of(b)))).collect();
    if comps.len() < 3 {
        return Err("distributing needs three or more path components".into());
    }
    comps.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (first, last) = (comps.first().map_or(0.0, |c| c.1), comps.last().map_or(0.0, |c| c.1));
    let step = (last - first) / (comps.len() - 1) as f64;
    for (i, (r, at)) in comps.iter().enumerate() {
        move_component(path, r, edge.offset(first + step * i as f64 - at))?;
    }
    Ok(())
}

/// Where Path Arrangement moves components in the stack (later subpaths are applied on top).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    Front,
    Forward,
    Backward,
    Back,
}

impl Arrange {
    pub fn parse(s: &str) -> Option<Arrange> {
        Some(match s {
            "front" => Arrange::Front,
            "forward" => Arrange::Forward,
            "backward" => Arrange::Backward,
            "back" => Arrange::Back,
            _ => return None,
        })
    }
}

/// Restacks the components holding subpaths `subs`. Returns the subpath indices they occupy
/// afterwards.
pub fn arrange_components(path: &mut Path, subs: &[usize], to: Arrange) -> Result<Vec<usize>, String> {
    let picked = components_of(path, subs)?;
    if picked.is_empty() {
        return Err("no path components to arrange".into());
    }
    let mut units: Vec<(std::ops::Range<usize>, bool)> = path.components().into_iter().map(|r| (r.clone(), picked.contains(&r))).collect();
    match to {
        Arrange::Front => units.sort_by_key(|u| u.1),
        Arrange::Back => units.sort_by_key(|u| !u.1),
        Arrange::Forward => {
            for i in (0..units.len().saturating_sub(1)).rev() {
                if units.get(i).is_some_and(|u| u.1) && units.get(i + 1).is_some_and(|u| !u.1) {
                    units.swap(i, i + 1);
                }
            }
        }
        Arrange::Backward => {
            for i in 1..units.len() {
                if units.get(i).is_some_and(|u| u.1) && units.get(i - 1).is_some_and(|u| !u.1) {
                    units.swap(i, i - 1);
                }
            }
        }
    }
    let old = std::mem::take(&mut path.subpaths);
    let mut moved = Vec::new();
    for (r, sel) in units {
        for s in r {
            if let Some(sp) = old.get(s) {
                if sel {
                    moved.push(path.subpaths.len());
                }
                path.subpaths.push(sp.clone());
            }
        }
    }
    Ok(moved)
}

/// Sets the path operation of the components holding subpaths `subs` (their first subpath's;
/// the rest of a component stays joined to it).
pub fn set_operation(path: &mut Path, subs: &[usize], op: prism_doc::PathOp) -> Result<(), String> {
    for r in components_of(path, subs)? {
        if let Some(sp) = path.subpaths.get_mut(r.start) {
            sp.op = op;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_doc::Subpath;

    fn tri() -> Path {
        Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)])])
    }

    /// A smooth knot at (50, 0) between corners, handles 20 px either side.
    fn curve() -> Path {
        let mut sp = Subpath::polyline(&[(0.0, 50.0), (50.0, 0.0), (100.0, 50.0)]);
        sp.knots[1] = Knot::smooth(Point::new(50.0, 0.0), Point::new(30.0, 0.0), Point::new(70.0, 0.0));
        Path::new(vec![sp])
    }

    fn at(path: &Path, s: usize, k: usize) -> Knot {
        path.subpaths[s].knots[k]
    }

    #[test]
    fn anchors_move_with_their_handles_once_each() {
        let mut p = curve();
        move_anchors(&mut p, &[[0, 1], [0, 1]], [5.0, -2.0]).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!((k.anchor, k.in_ctrl, k.out_ctrl), (Point::new(55.0, -2.0), Point::new(35.0, -2.0), Point::new(75.0, -2.0)));
        assert_eq!(at(&p, 0, 0).anchor, Point::new(0.0, 50.0), "other anchors stay");
    }

    #[test]
    fn bad_indices_are_errors_and_change_nothing() {
        let mut p = tri();
        assert!(move_anchors(&mut p, &[[0, 0], [0, 9]], [1.0, 1.0]).is_err());
        assert_eq!(p, tri(), "validated before any knot moved");
        assert!(move_handle(&mut p, [3, 0], Handle::In, Point::new(0.0, 0.0), false).is_err());
        assert!(bend_segment(&mut p, [0, 7], 0.5, [1.0, 1.0]).is_err());
        assert!(convert_point(&mut p, [usize::MAX, usize::MAX], None).is_err());
        let mut open = curve();
        assert!(bend_segment(&mut open, [0, 2], 0.5, [1.0, 1.0]).is_err(), "an open subpath's last knot starts no segment");
        assert!(bend_segment(&mut open, [0, usize::MAX], 0.5, [1.0, 1.0]).is_err());
    }

    #[test]
    fn smooth_handles_stay_collinear_and_keep_their_length() {
        let mut p = curve();
        move_handle(&mut p, [0, 1], Handle::Out, Point::new(50.0, 30.0), false).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!(k.out_ctrl, Point::new(50.0, 30.0));
        assert!(dist(k.in_ctrl, Point::new(50.0, -20.0)) < 1e-9, "opposite turned, length 20 kept: {:?}", k.in_ctrl);
        assert!(k.smooth);
    }

    #[test]
    fn independent_handles_break_the_knot() {
        let mut p = curve();
        move_handle(&mut p, [0, 1], Handle::In, Point::new(40.0, 20.0), true).unwrap();
        let k = at(&p, 0, 1);
        assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(40.0, 20.0), Point::new(70.0, 0.0), false));
    }

    #[test]
    fn convert_point_retracts_or_pulls_mirrored_handles() {
        let mut p = curve();
        convert_point(&mut p, [0, 1], None).unwrap();
        assert_eq!(at(&p, 0, 1), Knot::corner(50.0, 0.0));
        convert_point(&mut p, [0, 0], Some(Point::new(10.0, 60.0))).unwrap();
        let k = at(&p, 0, 0);
        assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(-10.0, 40.0), Point::new(10.0, 60.0), true));
    }

    #[test]
    fn straight_segments_move_with_both_anchors() {
        let mut p = tri();
        // The closing segment (2 → 0) wraps.
        bend_segment(&mut p, [0, 2], 0.5, [0.0, 10.0]).unwrap();
        assert_eq!((at(&p, 0, 2).anchor, at(&p, 0, 0).anchor, at(&p, 0, 1).anchor), (Point::new(50.0, 90.0), Point::new(0.0, 10.0), Point::new(100.0, 0.0)));
    }

    #[test]
    fn curves_bend_through_the_dragged_point() {
        for t in [0.2, 0.5, 0.8] {
            let mut p = curve();
            let seg = |p: &Path| p.subpaths[0].segments()[1];
            let before = eval(&seg(&p), t);
            bend_segment(&mut p, [0, 1], t, [3.0, 12.0]).unwrap();
            let after = eval(&seg(&p), t);
            assert!(dist(after, Point::new(before.x + 3.0, before.y + 12.0)) < 1e-9, "t={t}: {after:?}");
            let k = at(&p, 0, 1);
            let (vi, vo) = ((k.in_ctrl.x - 50.0, k.in_ctrl.y), (k.out_ctrl.x - 50.0, k.out_ctrl.y));
            assert!((vi.0 * vo.1 - vi.1 * vo.0).abs() < 1e-9 && vi.0 * vo.0 + vi.1 * vo.1 < 0.0, "the smooth knot stayed smooth");
        }
    }

    #[test]
    fn hits_prefer_handles_then_anchors_then_segments() {
        let p = curve();
        let shown = [([0, 1], Handle::Out)];
        assert_eq!(hit(&p, Point::new(71.0, 1.0), 3.0, &shown), Some(Hit::Handle([0, 1], Handle::Out)));
        assert_eq!(hit(&p, Point::new(71.0, 1.0), 3.0, &[]), None, "hidden handles aren't hit");
        assert_eq!(hit(&p, Point::new(49.0, 1.0), 3.0, &shown), Some(Hit::Anchor([0, 1])));
        let Some(Hit::Segment(r, t)) = hit(&tri(), Point::new(50.0, 1.0), 3.0, &[]) else { panic!("segment") };
        assert_eq!(r, [0, 0]);
        assert!((t - 0.5).abs() < 0.02, "{t}");
        assert_eq!(hit(&tri(), Point::new(50.0, 40.0), 3.0, &[]), None);
    }

    #[test]
    fn marquee_and_subpath_selections() {
        let p = tri();
        assert_eq!(anchors_in(&p, [110.0, -5.0, 40.0, 5.0]), vec![[0, 1]]);
        assert_eq!(subpath_anchors(&p, 0), vec![[0, 0], [0, 1], [0, 2]]);
        assert!(subpath_anchors(&p, 4).is_empty());
    }

    #[test]
    fn add_anchor_splits_without_changing_the_shape() {
        // Straight closing segment of the triangle: a corner point at its midpoint, added last.
        let mut p = tri();
        assert_eq!(add_anchor(&mut p, [0, 2], 0.5).unwrap(), [0, 3]);
        let k = at(&p, 0, 3);
        assert_eq!((k.anchor, k.smooth), (Point::new(25.0, 40.0), false));
        // A curve: a smooth point on the curve, and both halves trace the original.
        let mut c = curve();
        let before = c.subpaths[0].segments()[0];
        let k = add_anchor(&mut c, [0, 0], 0.25).unwrap();
        assert_eq!(k, [0, 1]);
        assert!(at(&c, 0, 1).smooth);
        let segs = c.subpaths[0].segments();
        for (t, half, u) in [(0.125, 0, 0.5), (0.25, 1, 0.0), (0.625, 1, 0.5)] {
            let (a, b) = (eval(&before, t), eval(&segs[half], u));
            assert!(dist(a, b) < 1e-9, "t={t}: {a:?} vs {b:?}");
        }
        assert_eq!(c.subpaths[0].knots.len(), 4);
        // Bad input changes nothing.
        let mut q = curve();
        assert!(add_anchor(&mut q, [0, 2], 0.5).is_err(), "an open path's last knot has no segment");
        assert!(add_anchor(&mut q, [3, 0], 0.5).is_err());
        assert!(add_anchor(&mut q, [0, 0], f64::NAN).is_err());
        assert_eq!(q, curve());
    }

    #[test]
    fn delete_anchor_joins_neighbours_and_drops_empty_subpaths() {
        let mut p = tri();
        delete_anchor(&mut p, [0, 1]).unwrap();
        assert_eq!(p.subpaths[0].knots.iter().map(|k| k.anchor).collect::<Vec<_>>(), vec![Point::new(0.0, 0.0), Point::new(50.0, 80.0)]);
        assert!(delete_anchor(&mut p, [0, 5]).is_err());
        delete_anchor(&mut p, [0, 0]).unwrap();
        delete_anchor(&mut p, [0, 0]).unwrap();
        assert!(p.subpaths.is_empty());
        assert!(delete_anchor(&mut p, [0, 0]).is_err());
    }

    /// Three squares of side 10 at x = 0, 30 and 100 (y = 0, 5, 20).
    fn squares() -> Path {
        let sq = |x: f64, y: f64| Subpath::polygon(&[(x, y), (x + 10.0, y), (x + 10.0, y + 10.0), (x, y + 10.0)]);
        Path::new(vec![sq(0.0, 0.0), sq(30.0, 5.0), sq(100.0, 20.0)])
    }

    #[test]
    fn components_align_distribute_and_arrange() {
        let mut p = squares();
        align_components(&mut p, &[0, 1, 2], Edge::Top, None).unwrap();
        assert!((0..3).all(|s| subpath_bounds(&p, s).unwrap()[1] == 0.0));
        align_components(&mut p, &[1], Edge::Right, Some([0.0, 0.0, 200.0, 100.0])).unwrap();
        assert_eq!(subpath_bounds(&p, 1).unwrap()[2], 200.0);
        assert_eq!(subpath_bounds(&p, 0).unwrap()[0], 0.0, "unpicked components stay");

        let mut p = squares();
        distribute_components(&mut p, &[0, 1, 2], Edge::Left).unwrap();
        assert_eq!(subpath_bounds(&p, 1).unwrap()[0], 50.0);
        assert!(distribute_components(&mut p, &[0, 1], Edge::Left).is_err());
        assert!(align_components(&mut p, &[7], Edge::Left, None).is_err());

        let mut p = squares();
        let first = p.subpaths[0].clone();
        assert_eq!(arrange_components(&mut p, &[0], Arrange::Forward).unwrap(), vec![1]);
        assert_eq!(p.subpaths[1], first);
        assert_eq!(arrange_components(&mut p, &[1], Arrange::Front).unwrap(), vec![2]);
        assert_eq!(arrange_components(&mut p, &[2], Arrange::Back).unwrap(), vec![0]);
        assert_eq!(arrange_components(&mut p, &[0], Arrange::Backward).unwrap(), vec![0], "already at the back");
        assert_eq!(p.subpaths[0], first);
        assert!(arrange_components(&mut p, &[9], Arrange::Front).is_err());
    }

    #[test]
    fn components_keep_joined_subpaths_together() {
        let mut p = squares();
        p.subpaths[1].op = prism_doc::PathOp::Join; // a hole of the first square's component
        assert_eq!(components_of(&p, &[1]).unwrap(), vec![0..2]);
        set_operation(&mut p, &[1], prism_doc::PathOp::Subtract).unwrap();
        assert_eq!((p.subpaths[0].op, p.subpaths[1].op), (prism_doc::PathOp::Subtract, prism_doc::PathOp::Join));
        let joined = p.subpaths[1].clone();
        assert_eq!(arrange_components(&mut p, &[0], Arrange::Front).unwrap(), vec![1, 2]);
        assert_eq!(p.subpaths[2], joined);
    }
}
