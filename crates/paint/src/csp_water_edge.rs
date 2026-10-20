//! Independently implemented CSP watercolor edge ("Watercolor edge" in the SUT brush settings).
//! Source evidence, constants and remaining limits are recorded in docs/sut-analysis.md.
//!
//! The effect is stroke-level: when the stroke buffer composites, each covered pixel near the
//! buffer's empty pixels gains extra alpha. The amount depends on the Euclidean distance to the
//! nearest empty pixel inside a square window, the strongest coverage in that window, and a
//! standard 16×16 ordered-dither threshold. "Process after drag" defers the effect until the
//! stroke is committed and may then blur the edge amounts with an integer Gaussian, which also
//! spreads a faint halo outside the stroke.
use serde::{Deserialize, Serialize};

/// Prism work limit for the edge radius in pixels, not a verified CSP maximum.
pub const MAX_RADIUS: f32 = 100.0;
/// The source clamps the blur radius to this many pixels.
pub const MAX_BLUR: f32 = 500.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CspWaterEdge {
    /// Edge range in document pixels.
    pub radius: f32,
    /// Opacity influence, 0..1.
    pub opacity: f32,
    /// Darkness influence, 0..1 (lowers the edge colour's value).
    pub darkness: f32,
    /// Blur in document pixels; only used with `after_drag`.
    pub blur: f32,
    /// Process after drag: apply when the stroke is committed, not while drawing.
    pub after_drag: bool,
}

impl Default for CspWaterEdge {
    fn default() -> Self {
        Self { radius: 2.0, opacity: 0.15, darkness: 0.0, blur: 0.0, after_drag: false }
    }
}

impl CspWaterEdge {
    /// Out-of-range values clamped; non-finite values become the defaults.
    pub fn bounded(&self) -> Self {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        Self {
            radius: fin(self.radius, 0.0).clamp(0.0, MAX_RADIUS),
            opacity: fin(self.opacity, 0.0).clamp(0.0, 1.0),
            darkness: fin(self.darkness, 0.0).clamp(0.0, 1.0),
            blur: fin(self.blur, 0.0).clamp(0.0, MAX_BLUR),
            after_drag: self.after_drag,
        }
    }

    /// Whether committing applies the Gaussian blur pass.
    pub fn blurs(&self) -> bool {
        self.after_drag && self.blur > 1e-8
    }
}

/// Coverage this small rounds to zero in the source's 15-bit stroke mask.
#[inline]
pub(crate) fn empty(c: f32) -> bool {
    c.is_nan() || c * 32768.0 < 0.5
}

/// Standard 16×16 Bayer ordered-dither threshold (0..255), built from the 2×2 base pattern.
#[inline]
pub(crate) fn bayer(x: i32, y: i32) -> u8 {
    const BASE: [[u8; 2]; 2] = [[0, 2], [3, 1]];
    let mut v = 0u8;
    for k in 0..4 {
        let row = BASE[((y >> k) & 1) as usize];
        v = v.wrapping_add(row[((x >> k) & 1) as usize] << (2 * (3 - k)));
    }
    v
}

/// Precomputed per-stroke constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EdgeParams {
    /// Window half-size: ceil(radius).
    pub n: i32,
    /// radius + 1.
    outer: f64,
    /// Amount for the pixels adjacent to an empty one.
    near: f64,
    /// Slope of the amount over the distance for the others.
    slope: f64,
    /// Darkness in 8-bit value levels, normalized (0..200/255).
    pub darkness: f32,
}

impl EdgeParams {
    /// `None` when the edge has no reach.
    pub fn new(e: &CspWaterEdge) -> Option<Self> {
        let e = e.bounded();
        if e.radius < 1e-8 {
            return None;
        }
        let r = f64::from(e.radius);
        let a = f64::from(e.opacity).clamp(0.01, 1.0);
        let outer = r + 1.0;
        let darkness = (f64::from(e.darkness) * 200.0).round() as f32 / 255.0;
        Some(Self { n: r.ceil() as i32, outer, near: a * 3.0, slope: a * 10.0 / outer, darkness })
    }

    /// Extra alpha for a covered pixel: `dist2` is the squared distance to the nearest empty
    /// pixel ((n+1)² when none is in the window), `max_cov` the strongest coverage in the window.
    #[inline]
    pub fn amount(&self, dist2: i64, max_cov: f32, x: i32, y: i32) -> f32 {
        let d2 = dist2 as f64;
        if dist2 <= 0 || d2 > self.outer * self.outer {
            return 0.0;
        }
        let d = d2.sqrt();
        let f = if d > 1.0 { (self.outer - d) * self.slope } else { self.near };
        let noise = f64::from(bayer(x, y)) * 0.0004 + 0.95;
        (f * f64::from(max_cov) * noise).max(0.0) as f32
    }
}

/// Edge amounts for every pixel of `rect` (rows), from `pad`: dense coverage over
/// `rect.inflate(p.n)`. Empty pixels get 0.
pub(crate) fn live_amounts(p: &EdgeParams, pad: &[f32], rect: prism_geom::Rect) -> Vec<f32> {
    let n = p.n.max(0);
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let n_us = n as usize;
    let pw = w + 2 * n_us;
    let mut out = vec![0.0f32; w * h];
    if pad.len() < pw * (h + 2 * n_us) {
        return out;
    }
    // Per padded column and output row: nearest empty row distance and window maximum.
    const NONE: i64 = i64::MAX;
    let mut col_d = vec![NONE; pw * h];
    let mut col_max = vec![0.0f32; pw * h];
    for yi in 0..h {
        for xi in 0..pw {
            let (mut best, mut mx) = (NONE, 0.0f32);
            for dy in 0..=2 * n_us {
                let c = pad[(yi + dy) * pw + xi];
                if empty(c) {
                    let ad = (dy as i64 - n as i64).abs();
                    best = best.min(ad);
                } else if c > mx {
                    mx = c;
                }
            }
            col_d[yi * pw + xi] = best;
            col_max[yi * pw + xi] = mx;
        }
    }
    let cap = (i64::from(n) + 1) * (i64::from(n) + 1);
    for yi in 0..h {
        for xi in 0..w {
            if empty(pad[(yi + n_us) * pw + xi + n_us]) {
                continue;
            }
            let (mut dist2, mut mx) = (cap, 0.0f32);
            for dx in 0..=2 * n_us {
                let k = yi * pw + xi + dx;
                let cd = col_d[k];
                if cd != NONE {
                    let ddx = dx as i64 - i64::from(n);
                    dist2 = dist2.min(ddx * ddx + cd * cd);
                }
                mx = mx.max(col_max[k]);
            }
            out[yi * w + xi] = p.amount(dist2, mx, rect.x0 + xi as i32, rect.y0 + yi as i32);
        }
    }
    out
}

/// Integer Gaussian taps for `blur` pixels: weight 65535 at the centre and 1/255 of it at the
/// blur radius; the half-width is the truncated radius.
pub(crate) fn gaussian_taps(blur: f32) -> Vec<u64> {
    let b = f64::from(blur.clamp(0.0, MAX_BLUR));
    let l = (1.0f64 / 255.0).ln();
    let s = (-(b * b) / (l + l)).sqrt();
    let t = (s + s) * s;
    let n = if t > 0.0 { (-t * l).sqrt() as usize } else { 0 };
    let mut taps = Vec::with_capacity(n + 1);
    taps.push(65535);
    for i in 1..=n {
        let i2 = (i * i) as f64;
        taps.push(((-i2 / t).exp() * 65535.0) as u64);
    }
    taps
}

/// One separable pass over a line, in place: floor(Σw·v / Σw) over the taps inside the line
/// (edge-renormalized). Every nonzero sample lies in `first..=last`; outputs farther than the
/// half-width from that range stay zero. `cum[k]` is the sum of the first `k + 1` taps.
fn blur_line(data: &mut [u8], first: usize, last: usize, taps: &[u64], cum: &[u64], line: &mut Vec<u8>) {
    let len = data.len();
    if first > last || last >= len {
        return;
    }
    let n = taps.len().saturating_sub(1);
    line.clear();
    line.extend_from_slice(data.get(first..=last).unwrap_or(&[]));
    for (i, out) in data.iter_mut().enumerate().take((last + n + 1).min(len)).skip(first.saturating_sub(n)) {
        let (lo, hi) = (i.saturating_sub(n), (i + n + 1).min(len));
        // Weights depend only on how far the line extends on each side of `i`.
        let sw = cum.get(i - lo).copied().unwrap_or(0) + cum.get(hi - 1 - i).copied().unwrap_or(0) - taps.first().copied().unwrap_or(0);
        let mut swv = 0u64;
        for j in lo.max(first)..hi.min(last + 1) {
            let v = line.get(j - first).copied().unwrap_or(0);
            if v != 0 {
                swv += taps.get(j.abs_diff(i)).copied().unwrap_or(0) * u64::from(v);
            }
        }
        *out = if swv == 0 || sw == 0 { 0 } else { (swv / sw).min(255) as u8 };
    }
}

/// Horizontal then vertical integer Gaussian over a `w`×`h` 8-bit buffer. Only the reach of the
/// nonzero samples is computed; columns are blurred in 64-wide strips copied to contiguous memory.
pub(crate) fn blur_u8(buf: &mut [u8], w: usize, h: usize, blur: f32) {
    const STRIP: usize = 64;
    let taps = gaussian_taps(blur);
    if taps.len() <= 1 || w == 0 || h == 0 || buf.len() < w * h {
        return;
    }
    let n = taps.len() - 1;
    let cum: Vec<u64> = taps
        .iter()
        .scan(0u64, |acc, &t| {
            *acc += t;
            Some(*acc)
        })
        .collect();
    let mut line = Vec::new();
    // Nonzero column range of each row after the horizontal pass.
    let mut rows: Vec<Option<(usize, usize)>> = vec![None; h];
    for (row, span) in buf.chunks_exact_mut(w).take(h).zip(rows.iter_mut()) {
        let (Some(first), Some(last)) = (row.iter().position(|&v| v != 0), row.iter().rposition(|&v| v != 0)) else { continue };
        blur_line(row, first, last, &taps, &cum, &mut line);
        *span = Some((first.saturating_sub(n), (last + n).min(w - 1)));
    }
    let mut strip = vec![0u8; STRIP * h];
    for x0 in (0..w).step_by(STRIP) {
        let x1 = (x0 + STRIP).min(w);
        let cw = x1 - x0;
        let touches = |s: &Option<(usize, usize)>| s.is_some_and(|(a, b)| a < x1 && b >= x0);
        let (Some(y0), Some(y1)) = (rows.iter().position(touches), rows.iter().rposition(touches)) else { continue };
        for column in strip.chunks_exact_mut(h).take(cw) {
            column.fill(0);
        }
        for (y, row) in buf.chunks_exact(w).enumerate().take(y1 + 1).skip(y0) {
            for (c, &v) in row.iter().skip(x0).take(cw).enumerate() {
                if let Some(slot) = strip.get_mut(c * h + y) {
                    *slot = v;
                }
            }
        }
        for column in strip.chunks_exact_mut(h).take(cw) {
            blur_line(column, y0, y1, &taps, &cum, &mut line);
        }
        for (y, row) in buf.chunks_exact_mut(w).enumerate().take((y1 + n + 1).min(h)).skip(y0.saturating_sub(n)) {
            for (c, slot) in row.iter_mut().skip(x0).take(cw).enumerate() {
                *slot = strip.get(c * h + y).copied().unwrap_or(0);
            }
        }
    }
}

/// 8-bit edge amount, as stored before blurring.
#[inline]
pub(crate) fn to_u8(amount: f32) -> u8 {
    let v = f64::from(amount) * 255.0;
    if v >= 255.0 { 255 } else { (v + 0.5).max(0.0) as u8 }
}

/// Alpha added by a blurred 8-bit amount (8-bit to 15-bit expansion).
#[inline]
pub(crate) fn from_u8(v: u8) -> f32 {
    ((u32::from(v) * 257 + 1) >> 1) as f32 / 32768.0
}

/// Lower the value of a straight RGB colour by `darkness` (normalized levels) times the share of
/// the pixel's alpha that the edge contributed, keeping its hue.
#[inline]
pub(crate) fn darken(rgba: [f32; 4], darkness: f32, share: f32) -> [f32; 4] {
    let m = rgba[0].max(rgba[1]).max(rgba[2]);
    if m <= 0.0 || darkness <= 0.0 || share <= 0.0 {
        return rgba;
    }
    let nm = m - darkness * share;
    let k = if nm <= 0.0 { 0.0 } else { nm / m };
    [rgba[0] * k, rgba[1] * k, rgba[2] * k, rgba[3]]
}

#[cfg(test)]
#[path = "csp_water_edge_tests.rs"]
mod tests;
