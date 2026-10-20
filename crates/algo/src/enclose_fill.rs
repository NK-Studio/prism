//! Enclose and fill (CLIP STUDIO PAINT's enclose-and-fill sub-tool, output operation 51): the
//! user draws a lasso, and everything inside it that open space from outside the lasso can't
//! reach is filled: the closed areas *and the line art itself*, so the colour runs under the
//! lines. Open space that continues outside the lasso stays unfilled.
//!
//! Prism's own definition (see `docs/sut-analysis.md` › Enclose and fill), measured against
//! CLIP STUDIO PAINT output (A/B, 2026-10-19):
//!
//! * **Line art** is any pixel whose ink (`alpha × (1 − luminance)`, 0–255) exceeds the colour
//!   margin. Everything else (transparent, white or light pixels) is open.
//! * **Close gap** treats open passages narrower than the gap (in pixels) as closed: open pixels
//!   closer than half the gap to line art don't connect areas.
//! * The **outer** space is every open area (4-connected) that touches a pixel outside the lasso,
//!   soaked back into the narrow pixels the gap rule held back next to it. Canvas edges inside
//!   the lasso count as closed. Everything else inside the lasso is filled, line art included
//!   (a line only partly inside the lasso is filled up to the lasso).
//! * A negative **expand** shrinks the fill away from the outer space (not from the lasso or the
//!   canvas edge). A positive one has nothing left to reach: the fill already covers the lines
//!   and never enters the outer space.

use prism_geom::Rect;

use crate::selection::{Region, edt, polygon};

/// Enclose-and-fill settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EncloseOptions {
    /// Colour margin, 0–255: pixels with at most this much ink are open (not line art).
    pub tolerance: f32,
    /// Open passages narrower than this many pixels don't connect areas (0 = off).
    pub close_gap: f32,
    /// Pixels to shrink the fill by away from the outer space when negative (positive values
    /// change nothing: the fill already covers the line art).
    pub expand: f32,
    /// Soften the fill's edge.
    pub anti_alias: bool,
}

impl Default for EncloseOptions {
    fn default() -> Self {
        EncloseOptions { tolerance: 51.0, close_gap: 0.0, expand: 0.0, anti_alias: true }
    }
}

/// The filled pixels and how many separate (4-connected) pieces they form.
#[derive(Clone, Debug, PartialEq)]
pub struct Enclosed {
    pub region: Region,
    pub areas: usize,
}

/// Largest gap or expansion honoured, in pixels.
pub const MAX_DISTANCE: f32 = 256.0;

/// Ink of an 8-bit RGBA pixel, 0–255: opaque black is 255, white or transparent 0.
fn ink(p: [u8; 4]) -> f32 {
    let luma = 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]);
    f32::from(p[3]) * (255.0 - luma) / 255.0
}

/// Enclose and fill on the reference image `img` (8-bit RGBA, row-major over `area`) with the
/// lasso `points` in document coordinates. `None` if no enclosed area is inside the lasso.
pub fn enclose_region(img: &[[u8; 4]], area: Rect, points: &[(f32, f32)], o: &EncloseOptions) -> Option<Enclosed> {
    if points.len() < 3 || area.is_empty() || img.len() != area.width() as usize * area.height() as usize {
        return None;
    }
    let pts: Vec<(f32, f32)> = points.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
    if pts.len() < 3 {
        return None;
    }
    // Work over the lasso's box plus one pixel (so its outline has an outside), within one
    // pixel of the canvas. That ring outside the canvas is never filled or crossed, and inside
    // the lasso it isn't the outside either: canvas edges close areas.
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in &pts {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    let clamp = |v: f32| v.clamp(-1.0e9, 1.0e9) as i32;
    let lasso_box = Rect::new(clamp(x0.floor()).saturating_sub(1), clamp(y0.floor()).saturating_sub(1), clamp(x1.ceil()).saturating_add(2), clamp(y1.ceil()).saturating_add(2));
    let work = lasso_box.intersect(&area.inflate(1));
    if work.is_empty() {
        return None;
    }
    let (w, h) = (work.width() as usize, work.height() as usize);
    let n = w.checked_mul(h)?;
    let inside: Vec<bool> = polygon(&pts, work, false).iter().map(|v| *v >= 0.5).collect();
    let on_canvas = |i: usize| area.contains(work.x0 + (i % w) as i32, work.y0 + (i / w) as i32);
    // Inside the lasso and on the canvas: the pixels that may be filled.
    let target: Vec<bool> = (0..n).map(|i| inside[i] && on_canvas(i)).collect();
    let aw = area.width() as usize;
    let ink_at: Vec<f32> = (0..n)
        .map(|i| {
            let (x, y) = (work.x0 + (i % w) as i32, work.y0 + (i / w) as i32);
            if area.contains(x, y) { img.get((y - area.y0) as usize * aw + (x - area.x0) as usize).copied().map_or(0.0, ink) } else { 0.0 }
        })
        .collect();
    let tol = if o.tolerance.is_finite() { o.tolerance.clamp(0.0, 255.0) } else { 0.0 };
    let open: Vec<bool> = ink_at.iter().map(|v| *v <= tol).collect();
    let gap = if o.close_gap.is_finite() { o.close_gap.clamp(0.0, MAX_DISTANCE) } else { 0.0 };
    // Open pixels this close to line art (centre distance) are part of a narrow gap.
    let passable: Vec<bool> = if gap > 0.0 {
        let walls: Vec<bool> = open.iter().map(|o| !o).collect();
        let d = edt(&walls, w, h);
        let min = (gap + 1.0) / 2.0;
        (0..n).map(|i| open[i] && target[i] && d.get(i).is_some_and(|d| *d > min)).collect()
    } else {
        (0..n).map(|i| open[i] && target[i]).collect()
    };

    let neighbours = |i: usize| {
        let (x, y) = (i % w, i / w);
        [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
    };
    // The outer space: open areas that touch the outside of the lasso.
    let mut outer = vec![false; n];
    let mut seen = vec![false; n];
    let mut stack = Vec::new();
    let mut members = Vec::new();
    for start in 0..n {
        if seen[start] || !passable[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        members.clear();
        let mut leaks = false;
        while let Some(i) = stack.pop() {
            members.push(i);
            for q in neighbours(i).into_iter().flatten() {
                if !inside[q] {
                    leaks = true;
                } else if passable[q] && !seen[q] {
                    seen[q] = true;
                    stack.push(q);
                }
            }
        }
        if leaks {
            for &i in &members {
                outer[i] = true;
            }
        }
    }
    // The outer space soaks back into the narrow open pixels the gap rule held back next to it
    // (no further than the gap rule reached, so it stops inside a sealed gap).
    if gap > 0.0 {
        grow(&mut outer, w, h, (gap + 1.0) / 2.0 + 1.0, |_, to| open[to] && target[to]);
    }
    // Everything else inside the lasso is filled, line art included.
    let mut filled: Vec<bool> = (0..n).map(|i| target[i] && !outer[i]).collect();
    let areas = pieces(&filled, w, h);
    if areas == 0 {
        return None;
    }
    // Shrinking pulls the fill back from the outer space only.
    let expand = if o.expand.is_finite() { o.expand.clamp(-MAX_DISTANCE, MAX_DISTANCE) } else { 0.0 };
    if expand < 0.0 {
        let d = edt(&outer, w, h);
        for (i, f) in filled.iter_mut().enumerate() {
            *f = *f && d.get(i).is_some_and(|d| *d > -expand);
        }
    }

    // Build the region (with a soft edge when anti-aliased).
    region(&filled, work, area, o.anti_alias).map(|region| Enclosed { region, areas })
}

/// Number of 4-connected pieces of `mask`.
fn pieces(mask: &[bool], w: usize, h: usize) -> usize {
    let mut seen = vec![false; mask.len()];
    let mut stack = Vec::new();
    let mut count = 0;
    for start in 0..mask.len() {
        if seen[start] || !mask[start] {
            continue;
        }
        count += 1;
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            for q in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)].into_iter().flatten() {
                if mask[q] && !seen[q] {
                    seen[q] = true;
                    stack.push(q);
                }
            }
        }
    }
    count
}

/// Grows `mask` along 4-connected paths, moving from a filled pixel to `to` only when
/// `allow(from, to)`, to pixels within `limit` (Euclidean) of the filled pixel the path left
/// from. Paths never cross pixels that `allow` refuses, so growth can't jump over thin lines.
fn grow(mask: &mut [bool], w: usize, h: usize, limit: f32, allow: impl Fn(usize, usize) -> bool) {
    let limit2 = limit * limit;
    // Each reached pixel remembers the filled pixel its path started from.
    let mut origin: Vec<Option<usize>> = mask.iter().enumerate().map(|(i, m)| m.then_some(i)).collect();
    let mut front: Vec<usize> = (0..mask.len()).filter(|&i| mask[i]).collect();
    while !front.is_empty() {
        let mut next = Vec::new();
        for &i in &front {
            let Some(o) = origin.get(i).copied().flatten() else { continue };
            let (x, y) = (i % w, i / w);
            for q in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)].into_iter().flatten() {
                if mask[q] || origin[q].is_some() || !allow(i, q) {
                    continue;
                }
                let (dx, dy) = ((q % w) as f32 - (o % w) as f32, (q / w) as f32 - (o / w) as f32);
                if dx * dx + dy * dy <= limit2 {
                    origin[q] = Some(o);
                    next.push(q);
                }
            }
        }
        for &q in &next {
            mask[q] = true;
        }
        front = next;
    }
}

/// The 8-bit region of `mask` (over `work`), clipped to `area`. With `anti_alias`, edge pixels
/// take the average of their 3×3 neighbourhood, and one pixel of fringe is added.
fn region(mask: &[bool], work: Rect, area: Rect, anti_alias: bool) -> Option<Region> {
    let w = work.width() as usize;
    let (mut bx0, mut by0, mut bx1, mut by1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (i, _) in mask.iter().enumerate().filter(|(_, m)| **m) {
        let (x, y) = (work.x0 + (i % w) as i32, work.y0 + (i / w) as i32);
        (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
    }
    if bx0 > bx1 {
        return None;
    }
    let pad = i32::from(anti_alias);
    let bbox = Rect::new(bx0 - pad, by0 - pad, bx1 + 1 + pad, by1 + 1 + pad).intersect(&area).intersect(&work);
    let at = |x: i32, y: i32| work.contains(x, y) && mask.get((y - work.y0) as usize * w + (x - work.x0) as usize).copied().unwrap_or(false);
    let bw = bbox.width() as usize;
    let mut out = vec![0u8; bw * bbox.height() as usize];
    for (i, o) in out.iter_mut().enumerate() {
        let (x, y) = (bbox.x0 + (i % bw) as i32, bbox.y0 + (i / bw) as i32);
        let v = at(x, y);
        if !anti_alias {
            *o = if v { 255 } else { 0 };
            continue;
        }
        let edge = [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| at(x + dx, y + dy) != v);
        *o = if edge {
            let count = (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))).filter(|(dx, dy)| at(x + dx, y + dy)).count();
            // Never drop a filled pixel below half, nor raise an empty one above it.
            let c = (count as f32 / 9.0 * 255.0).round() as u8;
            if v { c.max(128) } else { c.min(127) }
        } else if v {
            255
        } else {
            0
        };
    }
    Some(Region { bbox, mask: out })
}

#[cfg(test)]
#[path = "enclose_fill_tests.rs"]
mod tests;
