//! Edit › Content-Aware Fill (the workspace): the sampling area and the patch-based fill.
//!
//! The structure follows the maintainer's read-only analysis of Photoshop 2026's `AdbePM`
//! framework (`docs/content-aware-fill-analysis.md`); the code is written independently from
//! the documented rules and the published papers:
//!
//! * **Search bounds** ([`search_bounds`]): a rectangle centred on the hole whose area is
//!   `factor² × hole area`, each side at least twice the hole's, pushed inside the image and
//!   grown from a 50 px base in 5 px steps (below 175 px) while it holds too little to sample.
//! * **Sampling area** ([`sampling_area`]): *Rectangular* is the search bounds; *Auto* is the
//!   search bounds minus colours unlike the hole's surroundings (an estimate of what Photoshop
//!   shows, see the analysis notes); *Custom* is whatever the caller paints.
//! * **Fill** ([`fill`]): coarse-to-fine over a pyramid whose coarsest level has the hole a few
//!   patches across. The coarsest hole starts from an onion-peel average of its border; finer
//!   levels start from the upsampled result and inherit the match field (doubled offsets). Each
//!   level runs EM iterations of a PatchMatch search (C. Barnes, E. Shechtman, A. Finkelstein,
//!   D. B. Goldman, *PatchMatch*, SIGGRAPH 2009: propagation plus an exponentially shrinking
//!   random search) and a vote (Y. Wexler, E. Shechtman, M. Irani, *Space-Time Completion of
//!   Video*, TPAMI 2007) weighted by `1 / (cost / patch area + 1e-3)`.
//! * **Mirror** adds horizontally flipped source patches to the search; **colour adaptation**
//!   lets each match carry a per-channel brightness offset (the patch mean difference, clamped
//!   to ± a limit per level: none 0, default 0.06, high 0.15, very high 0.3) that is added when
//!   voting. **Rotation** and **scale** adaptation search rotated and rescaled copies of the
//!   sampling area laid out beside the image ([`crate::content_aware`]'s variants).
//!
//! Buffers are interleaved `w × h × ch` normalised floats (any model/depth). Deterministic:
//! fixed-seed RNG per row band, so results don't depend on the thread count.

use crate::content_aware::{Src, variant};
use prism_raster::{Cancelled, Interrupt};

/// Default patch size (pixels across; Photoshop's value wasn't found, estimate).
pub const PATCH_SIZE: usize = 7;
/// Default EM iterations per level (estimate).
pub const EM_ITERS: usize = 4;
/// Search-bounds factor used by Auto sampling: the crop factor Photoshop's UXP engine path sets
/// (4.0, from the research notes).
pub const AUTO_FACTOR: f32 = 4.0;
/// Search-bounds factor used by Rectangular sampling (estimate: the same as Auto; Photoshop
/// shows no setting).
pub const RECT_FACTOR: f32 = 4.0;
/// Vote weight width: the cost quantile used as σ² (Wexler et al. use the 75th percentile); the
/// last vote of the finest level uses a lower one so only good matches shape the result
/// (estimates, from the research notes' app).
const VOTE_QUANTILE: f32 = 0.75;
const FINAL_VOTE_QUANTILE: f32 = 0.25;
/// Below this many grid cells a level runs single-threaded.
const PAR_MIN: usize = 128 * 128;
/// Rows per PatchMatch band (each band propagates within itself; bands run in parallel).
const BAND: usize = 16;

/// Half-open pixel rectangle `[x0, x1) × [y0, y1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Bounds {
    pub fn width(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }
    pub fn height(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }
    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// Bounding box of the set cells of a `w × h` mask.
pub fn mask_bounds(w: usize, h: usize, m: &[bool]) -> Option<Bounds> {
    let mut b: Option<Bounds> = None;
    for y in 0..h {
        for x in 0..w {
            if m.get(y * w + x).copied().unwrap_or(false) {
                b = Some(match b {
                    None => Bounds { x0: x, y0: y, x1: x + 1, y1: y + 1 },
                    Some(b) => Bounds { x0: b.x0.min(x), y0: b.y0.min(y), x1: b.x1.max(x + 1), y1: b.y1.max(y + 1) },
                });
            }
        }
    }
    b
}

/// The search window around `hole` in a `w × h` image (see module docs). `known(b)` says how many
/// sampleable pixels a candidate window holds; the base size grows while it holds fewer than the
/// hole's area (estimate of Photoshop's minimum-size check).
pub fn search_bounds_with(w: usize, h: usize, hole: Bounds, factor: f32, known: impl Fn(Bounds) -> usize) -> Bounds {
    if w == 0 || h == 0 {
        return Bounds { x0: 0, y0: 0, x1: 0, y1: 0 };
    }
    let factor = if factor.is_finite() { factor.clamp(1.0, 64.0) } else { AUTO_FACTOR } as f64;
    let (hw, hh) = (hole.width().max(1) as f64, hole.height().max(1) as f64);
    let need = hole.width().saturating_mul(hole.height());
    let (cx, cy) = ((hole.x0 + hole.x1) as f64 / 2.0, (hole.y0 + hole.y1) as f64 / 2.0);
    let mut base = 50.0f64;
    let mut out = Bounds { x0: 0, y0: 0, x1: w, y1: h };
    while base < 175.0 {
        let area = factor * factor * hw * hh;
        let tall = area.sqrt().max(2.0 * hh).max(base);
        let wide = (area / tall).max(2.0 * hw).max(base);
        let (bw, bh) = (wide.min(w as f64), tall.min(h as f64));
        let x0 = (cx - bw / 2.0).clamp(0.0, (w as f64 - bw).max(0.0));
        let y0 = (cy - bh / 2.0).clamp(0.0, (h as f64 - bh).max(0.0));
        let (x0, y0) = (x0.floor() as usize, y0.floor() as usize);
        out = Bounds { x0, y0, x1: (x0 + bw.round() as usize).min(w), y1: (y0 + bh.round() as usize).min(h) };
        if known(out) >= need || (out.width() == w && out.height() == h) {
            break;
        }
        base += 5.0;
    }
    out
}

/// [`search_bounds_with`] counting every pixel outside the hole rectangle as sampleable.
pub fn search_bounds(w: usize, h: usize, hole: Bounds, factor: f32) -> Bounds {
    search_bounds_with(w, h, hole, factor, |b| b.width() * b.height() - overlap(b, hole))
}

fn overlap(a: Bounds, b: Bounds) -> usize {
    let (x0, y0, x1, y1) = (a.x0.max(b.x0), a.y0.max(b.y0), a.x1.min(b.x1), a.y1.min(b.y1));
    x1.saturating_sub(x0) * y1.saturating_sub(y0)
}

/// How the sampling area is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sampling {
    /// Similar content around the hole.
    Auto,
    /// A rectangle around the hole (search bounds with this factor).
    Rectangular { factor: f32 },
}

/// The sampling area for Auto or Rectangular: `true` where the fill may copy from. Never
/// includes hole pixels. `img` is used by Auto to leave out colours unlike the hole's border.
pub fn sampling_area(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], mode: Sampling) -> Vec<bool> {
    let mut out = vec![false; w * h];
    if img.len() != w * h * ch || hole.len() != w * h {
        return out;
    }
    let Some(hb) = mask_bounds(w, h, hole) else { return out };
    let factor = match mode {
        Sampling::Auto => AUTO_FACTOR,
        Sampling::Rectangular { factor } => factor,
    };
    let b = search_bounds(w, h, hb, factor);
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            out[y * w + x] = !hole[y * w + x];
        }
    }
    if mode == Sampling::Auto && ch > 0 {
        exclude_unlike(w, h, ch, img, hole, &mut out);
    }
    out
}

/// Auto sampling: drop pixels whose colour is far from every colour cluster of the hole's
/// border ring, then drop specks smaller than a patch (estimate; see the analysis notes).
pub fn exclude_unlike(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], area: &mut [bool]) {
    // Never cancelled.
    let _ = exclude_unlike_with(w, h, ch, img, hole, area, &Interrupt::NONE);
}

/// [`exclude_unlike`] that can be cancelled between its passes (and per row band).
pub fn exclude_unlike_with(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], area: &mut [bool], ctl: &Interrupt) -> Result<(), Cancelled> {
    if img.len() != w * h * ch || hole.len() != w * h || area.len() != w * h || ch == 0 {
        return Ok(());
    }
    let ring_r = PATCH_SIZE;
    let near = dilate(w, h, hole, ring_r);
    ctl.check()?;
    let ring: Vec<usize> = (0..w * h).filter(|&i| near[i] && !hole[i]).collect();
    if ring.is_empty() {
        return Ok(());
    }
    // k-means (k ≤ 4) over the ring colours, seeded by evenly spaced ring samples.
    let k = ring.len().min(4);
    let mut centres: Vec<Vec<f32>> = (0..k).map(|j| px(img, ch, ring[j * ring.len() / k]).to_vec()).collect();
    let mut assign = vec![0usize; ring.len()];
    for _ in 0..6 {
        ctl.check()?;
        for (a, &i) in assign.iter_mut().zip(&ring) {
            *a = nearest(&centres, px(img, ch, i)).0;
        }
        let mut sum = vec![vec![0.0f64; ch]; k];
        let mut n = vec![0usize; k];
        for (&a, &i) in assign.iter().zip(&ring) {
            n[a] += 1;
            for (s, v) in sum[a].iter_mut().zip(px(img, ch, i)) {
                *s += f64::from(*v);
            }
        }
        for j in 0..k {
            if n[j] > 0 {
                for c in 0..ch {
                    centres[j][c] = (sum[j][c] / n[j] as f64) as f32;
                }
            }
        }
    }
    // Spread of the ring around its clusters sets the tolerance.
    let spread = ring.iter().map(|&i| nearest(&centres, px(img, ch, i)).1).sum::<f32>() / ring.len() as f32;
    let tol = (1.5 * spread.sqrt()).max(0.12);
    let tol2 = tol * tol * ch as f32;
    ctl.check()?;
    {
        use rayon::prelude::*;
        let rows = 64 * w.max(1);
        area.par_chunks_mut(rows).enumerate().for_each(|(bi, chunk)| {
            if ctl.cancelled() {
                return;
            }
            for (j, a) in chunk.iter_mut().enumerate() {
                if *a && nearest(&centres, px(img, ch, bi * rows + j)).1 > tol2 {
                    *a = false;
                }
            }
        });
    }
    ctl.check()?;
    // Opening: specks narrower than a patch can't hold a source patch anyway.
    let r = PATCH_SIZE / 2;
    let eroded: Vec<bool> = dilate(w, h, &area.iter().map(|a| !a).collect::<Vec<_>>(), r).iter().map(|e| !e).collect();
    ctl.check()?;
    let opened = dilate(w, h, &eroded, r);
    for (a, o) in area.iter_mut().zip(opened) {
        *a = *a && o;
    }
    Ok(())
}

#[inline]
fn px(img: &[f32], ch: usize, i: usize) -> &[f32] {
    img.get(i * ch..(i + 1) * ch).unwrap_or(&[])
}

/// Index and squared distance of the nearest centre.
fn nearest(centres: &[Vec<f32>], p: &[f32]) -> (usize, f32) {
    let mut best = (0, f32::INFINITY);
    for (j, c) in centres.iter().enumerate() {
        let d: f32 = c.iter().zip(p).map(|(a, b)| (a - b) * (a - b)).sum();
        if d < best.1 {
            best = (j, d);
        }
    }
    best
}

/// Square dilation by `r` (separable running max).
pub fn dilate(w: usize, h: usize, m: &[bool], r: usize) -> Vec<bool> {
    if r == 0 || m.len() != w * h {
        return m.to_vec();
    }
    let mut tmp = vec![false; w * h];
    for y in 0..h {
        let row = &m[y * w..(y + 1) * w];
        let mut last: Option<usize> = None;
        // Distance to the nearest set cell on the left, then on the right.
        for x in 0..w {
            if row[x] {
                last = Some(x);
            }
            tmp[y * w + x] = last.is_some_and(|l| x - l <= r);
        }
        last = None;
        for x in (0..w).rev() {
            if row[x] {
                last = Some(x);
            }
            if last.is_some_and(|l| l - x <= r) {
                tmp[y * w + x] = true;
            }
        }
    }
    let mut out = vec![false; w * h];
    for x in 0..w {
        let mut last: Option<usize> = None;
        for y in 0..h {
            if tmp[y * w + x] {
                last = Some(y);
            }
            out[y * w + x] = last.is_some_and(|l| y - l <= r);
        }
        last = None;
        for y in (0..h).rev() {
            if tmp[y * w + x] {
                last = Some(y);
            }
            if last.is_some_and(|l| l - y <= r) {
                out[y * w + x] = true;
            }
        }
    }
    out
}

/// Colour adaptation limit for Photoshop's levels: none, default, high, veryHigh (estimate).
pub fn color_limit(level: &str) -> Option<f32> {
    Some(match level {
        "none" => 0.0,
        "default" => 0.06,
        "high" => 0.15,
        "veryHigh" => 0.3,
        _ => return None,
    })
}

/// Strength of the final low-frequency correction for the same levels (Prism's choice; with the
/// card photo, default took the fill's brightness gap to its border from +0.033 to +0.010).
pub fn membrane_strength(level: &str) -> Option<f32> {
    Some(match level {
        "none" => 0.0,
        "default" => 0.65,
        "high" => 0.85,
        "veryHigh" => 1.0,
        _ => return None,
    })
}

/// Options for [`fill`].
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Patch size in pixels across (odd; 5..=15).
    pub patch_size: usize,
    /// EM iterations per level (the finest level runs half, at least one).
    pub em_iters: usize,
    /// Per-channel offset limit a match may carry (colour adaptation), `0` = off.
    pub color_limit: f32,
    /// Strength (`0..=1`) of the final low-frequency pull towards the border's membrane.
    pub membrane: f32,
    /// Extra source rotations in degrees (rotation adaptation).
    pub rotations: Vec<f32>,
    /// Search downscaled and upscaled copies of the source.
    pub scale: bool,
    /// Search horizontally flipped patches.
    pub mirror: bool,
    /// Channel that carries alpha (no colour offset on it).
    pub alpha: Option<usize>,
    pub seed: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self { patch_size: PATCH_SIZE, em_iters: EM_ITERS, color_limit: 0.06, membrane: 0.65, rotations: Vec::new(), scale: false, mirror: false, alpha: None, seed: 1 }
    }
}

/// SplitMix64.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next() % (hi - lo + 1) as u64) as i32
    }
}

/// One pyramid level of the working buffer.
#[derive(Clone)]
struct Level {
    w: usize,
    h: usize,
    img: Vec<f32>,
    /// To be filled.
    hole: Vec<bool>,
    /// May be copied from.
    ok: Vec<bool>,
    /// Part of the picture (false in the gaps between source copies).
    ctx: Vec<bool>,
}

fn downsample(l: &Level, ch: usize) -> Level {
    let (w, h) = (l.w.div_ceil(2), l.h.div_ceil(2));
    let mut out = Level { w, h, img: vec![0.0; w * h * ch], hole: vec![false; w * h], ok: vec![true; w * h], ctx: vec![false; w * h] };
    for y in 0..h {
        for x in 0..w {
            let o = y * w + x;
            let mut n = 0.0f32;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((x * 2 + dx).min(l.w - 1), (y * 2 + dy).min(l.h - 1));
                let si = sy * l.w + sx;
                out.hole[o] |= l.hole[si];
                out.ok[o] &= l.ok[si];
                out.ctx[o] |= l.ctx[si];
                if !l.hole[si] && l.ctx[si] {
                    for c in 0..ch {
                        out.img[o * ch + c] += l.img[si * ch + c];
                    }
                    n += 1.0;
                }
            }
            if n > 0.0 {
                for c in 0..ch {
                    out.img[o * ch + c] /= n;
                }
            }
        }
    }
    out
}

/// Coarsest-level first guess: peel the hole from its border inwards, each ring the mean of its
/// already-known 8-neighbours.
fn onion_fill(l: &mut Level, ch: usize, ctl: &Interrupt) -> Result<(), Cancelled> {
    let (w, h) = (l.w, l.h);
    let mut known: Vec<bool> = (0..w * h).map(|i| !l.hole[i] && l.ctx[i]).collect();
    let mut front: Vec<usize> = (0..w * h).filter(|&i| l.hole[i]).collect();
    let mut acc = vec![0.0f32; ch];
    while !front.is_empty() {
        ctl.check()?;
        let mut done = Vec::new();
        let mut rest = Vec::new();
        for &i in &front {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            acc.iter_mut().for_each(|a| *a = 0.0);
            let mut n = 0.0f32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                        continue;
                    }
                    let q = ny as usize * w + nx as usize;
                    if known[q] {
                        for (a, v) in acc.iter_mut().zip(&l.img[q * ch..(q + 1) * ch]) {
                            *a += v;
                        }
                        n += 1.0;
                    }
                }
            }
            if n > 0.0 {
                done.push((i, acc.iter().map(|a| a / n).collect::<Vec<f32>>()));
            } else {
                rest.push(i);
            }
        }
        if done.is_empty() {
            // Nothing known anywhere near: leave the rest as is.
            break;
        }
        for (i, v) in done {
            l.img[i * ch..(i + 1) * ch].copy_from_slice(&v);
            known[i] = true;
        }
        front = rest;
    }
    Ok(())
}

/// Summed-area table of a boolean grid (`(w+1) × (h+1)`).
fn integral(w: usize, h: usize, m: &[bool]) -> Vec<u32> {
    let mut s = vec![0u32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += u32::from(m[y * w + x]);
            s[(y + 1) * (w + 1) + x + 1] = s[y * (w + 1) + x + 1] + row;
        }
    }
    s
}

/// Set cells in the clipped window `[x0, x1) × [y0, y1)`.
fn count(s: &[u32], gw: usize, gh: usize, x0: i32, y0: i32, x1: i32, y1: i32) -> u32 {
    let (x0, y0) = (x0.max(0) as usize, y0.max(0) as usize);
    let (x1, y1) = ((x1.max(0) as usize).min(gw), (y1.max(0) as usize).min(gh));
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    let w1 = gw + 1;
    s[y1 * w1 + x1] + s[y0 * w1 + x0] - s[y0 * w1 + x1] - s[y1 * w1 + x0]
}

/// A match: source centre, flip, cost and the colour offset it was costed with.
#[derive(Clone, Copy, Debug)]
struct Match {
    x: i32,
    y: i32,
    flip: bool,
    cost: f32,
}

const NO_MATCH: Match = Match { x: 0, y: 0, flip: false, cost: f32::INFINITY };
/// Most channels a buffer has (CMYK + alpha + spare); offsets live in a fixed array.
const MAX_CH: usize = 8;

struct Solver<'a> {
    l: Level,
    ch: usize,
    r: i32,
    limit: f32,
    alpha: Option<usize>,
    mirror: bool,
    /// Valid source centres (whole patch inside `ok`).
    valid: Vec<bool>,
    valid_list: Vec<(i32, i32)>,
    /// Target centres (the patch overlaps the hole).
    targets: Vec<bool>,
    /// Random-search span (the sampleable region's longest side).
    span: i32,
    ctl: Interrupt<'a>,
}

impl Solver<'_> {
    fn new<'a>(l: Level, ch: usize, opts: &Options, ctl: Interrupt<'a>) -> Result<Solver<'a>, Cancelled> {
        let r = (opts.patch_size / 2).max(1) as i32;
        let (w, h) = (l.w, l.h);
        let bad_src: Vec<bool> = l.ok.iter().map(|o| !o).collect();
        let bad = integral(w, h, &bad_src);
        let holes = integral(w, h, &l.hole);
        let mut valid = vec![false; w * h];
        let mut valid_list = Vec::new();
        let mut targets = vec![false; w * h];
        let (mut sx0, mut sy0, mut sx1, mut sy1) = (i32::MAX, i32::MAX, 0, 0);
        for y in 0..h as i32 {
            if y % 32 == 0 {
                ctl.check()?;
            }
            for x in 0..w as i32 {
                let i = y as usize * w + x as usize;
                let inside = x >= r && y >= r && x + r < w as i32 && y + r < h as i32;
                if inside && count(&bad, w, h, x - r, y - r, x + r + 1, y + r + 1) == 0 {
                    valid[i] = true;
                    valid_list.push((x, y));
                    (sx0, sy0, sx1, sy1) = (sx0.min(x), sy0.min(y), sx1.max(x), sy1.max(y));
                }
                targets[i] = l.ctx[i] && count(&holes, w, h, x - r, y - r, x + r + 1, y + r + 1) > 0;
            }
        }
        let span = (sx1 - sx0).max(sy1 - sy0).max(1);
        let alpha = opts.alpha.filter(|a| *a < ch);
        Ok(Solver { l, ch: ch.min(MAX_CH), r, limit: opts.color_limit.max(0.0), alpha, mirror: opts.mirror, valid, valid_list, targets, span, ctl })
    }

    #[inline]
    fn is_valid(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.l.w && (y as usize) < self.l.h && self.valid[y as usize * self.l.w + x as usize]
    }

    /// Per-channel offset a match carries (clamped mean difference), from `Σ(t − s)` and count.
    #[inline]
    fn offsets(&self, sum: &[f32; MAX_CH], n: f32) -> [f32; MAX_CH] {
        let mut b = [0.0f32; MAX_CH];
        if self.limit > 0.0 && n > 0.0 {
            for c in 0..self.ch {
                if Some(c) != self.alpha {
                    b[c] = (sum[c] / n).clamp(-self.limit, self.limit);
                }
            }
        }
        b
    }

    /// Colour offset from the patch's known pixels only (outside the hole): the hole's current
    /// estimate is a blurry guess, and offsets fitted to it hold the fill at that guess's
    /// brightness. Patches with less than a quarter known carry none (Prism's choice; the
    /// research app fits all pixels).
    #[inline]
    fn offsets_known(&self, known: &[f32; MAX_CH], nk: f32, n: f32) -> [f32; MAX_CH] {
        if nk < 0.25 * n { [0.0; MAX_CH] } else { self.offsets(known, nk) }
    }

    /// Sum of squared differences between the target patch at `t` and the source patch at `s`
    /// (flipped horizontally when `flip`), after the clamped colour offset; ∞ once it can't beat
    /// `cutoff`.
    fn dist(&self, t: (i32, i32), s: (i32, i32), flip: bool, cutoff: f32) -> f32 {
        let (w, h, ch, r) = (self.l.w as i32, self.l.h as i32, self.ch, self.r);
        let img = &self.l.img;
        let mut sq = [0.0f32; MAX_CH];
        let mut sum = [0.0f32; MAX_CH];
        let mut known = [0.0f32; MAX_CH];
        let mut n = 0.0f32;
        let mut nk = 0.0f32;
        for dy in -r..=r {
            let ty = t.1 + dy;
            if ty < 0 || ty >= h {
                continue;
            }
            let sy = (s.1 + dy) as usize;
            for dx in -r..=r {
                let tx = t.0 + dx;
                if tx < 0 || tx >= w {
                    continue;
                }
                let tp = ty as usize * self.l.w + tx as usize;
                if !self.l.ctx[tp] {
                    continue;
                }
                let sx = if flip { s.0 - dx } else { s.0 + dx } as usize;
                let (ti, si) = (tp * self.ch, (sy * self.l.w + sx) * self.ch);
                let k = !self.l.hole[tp];
                for c in 0..ch {
                    let e = img[ti + c] - img[si + c];
                    sq[c] += e * e;
                    sum[c] += e;
                    if k {
                        known[c] += e;
                    }
                }
                n += 1.0;
                if k {
                    nk += 1.0;
                }
            }
            // Early out on a lower bound: with the best possible offset the cost is Σe² − (Σe)²/n.
            if n > 0.0 {
                let lb: f32 = (0..ch).map(|c| if self.limit > 0.0 && Some(c) != self.alpha { sq[c] - sum[c] * sum[c] / n } else { sq[c] }).sum();
                if lb >= cutoff {
                    return f32::INFINITY;
                }
            }
        }
        if n == 0.0 {
            return f32::INFINITY;
        }
        let b = self.offsets_known(&known, nk, n);
        (0..ch).map(|c| sq[c] - 2.0 * b[c] * sum[c] + n * b[c] * b[c]).sum::<f32>().max(0.0)
    }

    /// The offsets of a match (recomputed for voting; cheap next to the search).
    fn match_offsets(&self, t: (i32, i32), m: &Match) -> [f32; MAX_CH] {
        if self.limit <= 0.0 {
            return [0.0; MAX_CH];
        }
        let (w, h, ch, r) = (self.l.w as i32, self.l.h as i32, self.ch, self.r);
        let mut sum = [0.0f32; MAX_CH];
        let mut n = 0.0f32;
        let mut nk = 0.0f32;
        for dy in -r..=r {
            let ty = t.1 + dy;
            if ty < 0 || ty >= h {
                continue;
            }
            for dx in -r..=r {
                let tx = t.0 + dx;
                if tx < 0 || tx >= w {
                    continue;
                }
                let tp = ty as usize * self.l.w + tx as usize;
                if !self.l.ctx[tp] {
                    continue;
                }
                n += 1.0;
                if self.l.hole[tp] {
                    continue;
                }
                let sx = if m.flip { m.x - dx } else { m.x + dx } as usize;
                let si = ((m.y + dy) as usize * self.l.w + sx) * self.ch;
                let (t, src) = (&self.l.img[tp * self.ch..tp * self.ch + ch], &self.l.img[si..si + ch]);
                for ((s, a), b) in sum.iter_mut().zip(t).zip(src) {
                    *s += a - b;
                }
                nk += 1.0;
            }
        }
        self.offsets_known(&sum, nk, n)
    }

    fn random_match(&self, rng: &mut Rng) -> Match {
        let n = self.valid_list.len().max(1) as u64;
        let (x, y) = self.valid_list.get((rng.next() % n) as usize).copied().unwrap_or((0, 0));
        Match { x, y, flip: self.mirror && rng.next() & 1 == 1, cost: f32::INFINITY }
    }

    fn try_candidate(&self, t: (i32, i32), best: &mut Match, c: Match) {
        let flip = c.flip && self.mirror;
        if !self.is_valid(c.x, c.y) || (c.x == best.x && c.y == best.y && flip == best.flip) {
            return;
        }
        let d = self.dist(t, (c.x, c.y), flip, best.cost);
        if d < best.cost {
            *best = Match { x: c.x, y: c.y, flip, cost: d };
        }
    }

    /// One PatchMatch pass (propagation from visited neighbours, then random search), in
    /// parallel row bands; odd passes run backwards.
    fn search(&self, nnf: &mut [Match], pass: usize, seed: u64, radius: i32) {
        let w = self.l.w;
        let reverse = pass % 2 == 1;
        let band = |bi: usize, nn: &mut [Match]| {
            if self.ctl.cancelled() {
                return;
            }
            let mut rng = Rng::new(seed ^ (bi as u64).wrapping_mul(0x2545_F491_4F6C_DD1D) ^ ((pass as u64) << 40));
            let y0 = bi * BAND;
            let rows = nn.len() / w.max(1);
            let d: i32 = if reverse { 1 } else { -1 };
            for ry in 0..rows {
                let ly = if reverse { rows - 1 - ry } else { ry };
                for rx in 0..w {
                    let x = if reverse { w - 1 - rx } else { rx };
                    if !self.targets[(y0 + ly) * w + x] {
                        continue;
                    }
                    let t = (x as i32, (y0 + ly) as i32);
                    let mut best = nn[ly * w + x];
                    // Propagation: the already-visited horizontal and vertical neighbours.
                    let nx = x as i32 + d;
                    if nx >= 0 && (nx as usize) < w {
                        let nb = nn[ly * w + nx as usize];
                        if nb.cost.is_finite() {
                            let step = if nb.flip { d } else { -d };
                            self.try_candidate(t, &mut best, Match { x: nb.x + step, ..nb });
                        }
                    }
                    let ny = ly as i32 + d;
                    if ny >= 0 && (ny as usize) < rows {
                        let nb = nn[ny as usize * w + x];
                        if nb.cost.is_finite() {
                            self.try_candidate(t, &mut best, Match { y: nb.y - d, ..nb });
                        }
                    }
                    // Random search in exponentially shrinking windows.
                    let mut rad = radius;
                    while rad >= 1 {
                        let flip = if self.mirror { rng.next() & 1 == 1 } else { false };
                        let c = Match { x: best.x + rng.range(-rad, rad), y: best.y + rng.range(-rad, rad), flip, cost: f32::INFINITY };
                        self.try_candidate(t, &mut best, c);
                        rad /= 2;
                    }
                    nn[ly * w + x] = best;
                }
            }
        };
        if w * self.l.h >= PAR_MIN {
            use rayon::prelude::*;
            nnf.par_chunks_mut(BAND * w).enumerate().for_each(|(bi, nn)| band(bi, nn));
        } else {
            for (bi, nn) in nnf.chunks_mut(BAND * w).enumerate() {
                band(bi, nn);
            }
        }
    }

    /// New hole values: per pixel, the weighted mean of what every overlapping match puts there
    /// (source pixel plus the match's colour offset), weight `exp(−(cost / area) / 2σ²)` with σ²
    /// the `quantile` of the matches' per-pixel costs (Wexler et al.).
    fn vote(&self, nnf: &[Match], quantile: f32) -> Vec<(usize, Vec<f32>)> {
        let (w, h, ch, r) = (self.l.w, self.l.h, self.ch, self.r);
        let area = ((2 * r + 1) * (2 * r + 1)) as f32;
        let stride = self.ch;
        let mut costs: Vec<f32> = nnf.iter().zip(&self.targets).filter(|(m, t)| **t && m.cost.is_finite()).map(|(m, _)| m.cost / area).collect();
        let sigma2 = if costs.is_empty() {
            1.0
        } else {
            let k = ((quantile.clamp(0.0, 1.0) * costs.len() as f32) as usize).min(costs.len() - 1);
            let (_, v, _) = costs.select_nth_unstable_by(k, f32::total_cmp);
            v.max(1e-6)
        };
        // Offsets once per target.
        let offs: Vec<[f32; MAX_CH]> = if self.limit > 0.0 {
            let one = |i: usize| {
                if self.targets[i] && nnf[i].cost.is_finite() && !self.ctl.cancelled() {
                    self.match_offsets(((i % w) as i32, (i / w) as i32), &nnf[i])
                } else {
                    [0.0; MAX_CH]
                }
            };
            if w * h >= PAR_MIN {
                use rayon::prelude::*;
                (0..w * h).into_par_iter().map(one).collect()
            } else {
                (0..w * h).map(one).collect()
            }
        } else {
            Vec::new()
        };
        let row = |y: usize| -> Vec<(usize, Vec<f32>)> {
            let mut out = Vec::new();
            if self.ctl.cancelled() {
                return out;
            }
            for x in 0..w {
                let p = y * w + x;
                if !self.l.hole[p] {
                    continue;
                }
                let mut acc = [0.0f32; MAX_CH];
                let mut wsum = 0.0f32;
                for j in -r..=r {
                    let ty = y as i32 - j;
                    if ty < 0 || ty as usize >= h {
                        continue;
                    }
                    for i in -r..=r {
                        let tx = x as i32 - i;
                        if tx < 0 || tx as usize >= w {
                            continue;
                        }
                        let ti = ty as usize * w + tx as usize;
                        let m = nnf[ti];
                        if !self.targets[ti] || !m.cost.is_finite() {
                            continue;
                        }
                        let sx = if m.flip { m.x - i } else { m.x + i } as usize;
                        let si = ((m.y + j) as usize * w + sx) * stride;
                        let wt = (-(m.cost / area) / (2.0 * sigma2)).exp() + 1e-8;
                        let b = offs.get(ti).copied().unwrap_or([0.0; MAX_CH]);
                        for c in 0..ch {
                            acc[c] += wt * (self.l.img[si + c] + b[c]);
                        }
                        wsum += wt;
                    }
                }
                if wsum > 0.0 {
                    out.push((p, (0..ch).map(|c| (acc[c] / wsum).clamp(0.0, 1.0)).collect()));
                }
            }
            out
        };
        if w * h >= PAR_MIN {
            use rayon::prelude::*;
            (0..h).into_par_iter().flat_map_iter(row).collect()
        } else {
            (0..h).flat_map(row).collect()
        }
    }
}

/// Fill `hole` in `img` copying only from `source` pixels (outside the hole). Returns the full
/// buffer (unchanged outside the hole), or `None` when no whole patch fits in the sampling area.
pub fn fill(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], source: &[bool], opts: &Options) -> Option<Vec<f32>> {
    fill_with(w, h, ch, img, hole, source, opts, &Interrupt::NONE).unwrap_or(None)
}

/// [`fill`] that can be cancelled and reports progress.
#[allow(clippy::too_many_arguments)]
pub fn fill_with(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    source: &[bool],
    opts: &Options,
    ctl: &Interrupt,
) -> Result<Option<Vec<f32>>, Cancelled> {
    if w == 0 || h == 0 || ch == 0 || ch > MAX_CH || img.len() != w * h * ch || hole.len() != w * h || source.len() != w * h {
        return Ok(None);
    }
    let Some(hb) = mask_bounds(w, h, hole) else { return Ok(Some(img.to_vec())) };
    let opts = Options { patch_size: (opts.patch_size.clamp(5, 15)) | 1, em_iters: opts.em_iters.clamp(1, 16), ..opts.clone() };
    let psz = opts.patch_size;
    let ok: Vec<bool> = (0..w * h).map(|i| source[i] && !hole[i]).collect();
    // Rotated / rescaled copies of the sampling area beside the image.
    let mut extra = Vec::new();
    let src = Src { w, h, ch, img, ok: &ok };
    for &deg in &opts.rotations {
        if deg.is_finite() {
            extra.push(variant(src, deg, 1.0, false));
        }
    }
    if opts.scale {
        extra.push(variant(src, 0.0, 0.8, false));
        extra.push(variant(src, 0.0, 1.25, false));
    }
    let gap = psz + 2;
    let col_w = extra.iter().map(|v| v.w).max().unwrap_or(0);
    let col_h: usize = extra.iter().map(|v| v.h + gap).sum();
    let mw = if extra.is_empty() { w } else { w + gap + col_w };
    let mh = h.max(col_h);
    let mut base = Level { w: mw, h: mh, img: vec![0.0; mw * mh * ch], hole: vec![false; mw * mh], ok: vec![false; mw * mh], ctx: vec![false; mw * mh] };
    for y in 0..h {
        if y % 64 == 0 {
            ctl.check()?;
        }
        for x in 0..w {
            let (s, d) = (y * w + x, y * mw + x);
            base.img[d * ch..(d + 1) * ch].copy_from_slice(&img[s * ch..(s + 1) * ch]);
            base.hole[d] = hole[s];
            base.ok[d] = ok[s];
            base.ctx[d] = true;
        }
    }
    let mut oy = 0;
    for v in &extra {
        for y in 0..v.h {
            for x in 0..v.w {
                let (s, d) = (y * v.w + x, (oy + y) * mw + w + gap + x);
                base.img[d * ch..(d + 1) * ch].copy_from_slice(&v.img[s * ch..(s + 1) * ch]);
                base.ok[d] = v.ok[s];
                base.ctx[d] = v.ok[s];
            }
        }
        oy += v.h + gap;
    }
    ctl.check()?;
    // Pyramid down to where the hole's long side is about three patches.
    let mut levels = vec![base];
    let mut ext = hb.width().max(hb.height());
    while let Some(l) = levels.last() {
        ctl.check()?;
        if ext <= 3 * psz || l.w / 2 < 4 * psz || l.h / 2 < 4 * psz {
            break;
        }
        let d = downsample(l, ch);
        levels.push(d);
        ext = ext.div_ceil(2);
    }
    let total: usize = levels.iter().map(|l| l.w * l.h).sum();
    let mut done = 0usize;
    let mut prev: Option<(usize, Vec<Match>, Level)> = None;
    let nlev = levels.len();
    for li in (0..nlev).rev() {
        ctl.check()?;
        let mut lvl = levels[li].clone();
        match &prev {
            None => onion_fill(&mut lvl, ch, ctl)?,
            Some((_, _, c)) => {
                for y in 0..lvl.h {
                    for x in 0..lvl.w {
                        let k = y * lvl.w + x;
                        if lvl.hole[k] {
                            let ck = (y / 2).min(c.h - 1) * c.w + (x / 2).min(c.w - 1);
                            lvl.img[k * ch..(k + 1) * ch].copy_from_slice(&c.img[ck * ch..(ck + 1) * ch]);
                        }
                    }
                }
            }
        }
        let mut solver = Solver::new(lvl, ch, &opts, *ctl)?;
        if solver.valid_list.is_empty() {
            if li == 0 {
                return Ok(None);
            }
            // Nothing to sample at this scale (a thin sampling area): carry the estimate down.
            done += solver.l.w * solver.l.h;
            prev = Some((solver.l.w, Vec::new(), solver.l));
            continue;
        }
        let mut rng = Rng::new(opts.seed ^ ((li as u64) << 20));
        let mut nnf = vec![NO_MATCH; solver.l.w * solver.l.h];
        for y in 0..solver.l.h {
            for x in 0..solver.l.w {
                let i = y * solver.l.w + x;
                if !solver.targets[i] {
                    continue;
                }
                // Inherit the coarser match (doubled offset, flip kept) where it is still valid.
                let mut m = None;
                if let Some((pw, pn, _)) = &prev
                    && let Some(c) = pn.get((y / 2) * pw + (x / 2))
                    && c.cost.is_finite()
                {
                    let dx = (x % 2) as i32;
                    let n = Match { x: c.x * 2 + if c.flip { -dx } else { dx }, y: c.y * 2 + (y % 2) as i32, flip: c.flip, cost: f32::INFINITY };
                    if solver.is_valid(n.x, n.y) {
                        m = Some(n);
                    }
                }
                nnf[i] = m.unwrap_or_else(|| solver.random_match(&mut rng));
            }
        }
        let recost = |solver: &Solver, nnf: &mut [Match]| {
            let w = solver.l.w;
            let one = |(i, m): (usize, &mut Match)| {
                if solver.targets[i] && !solver.ctl.cancelled() {
                    m.cost = solver.dist(((i % w) as i32, (i / w) as i32), (m.x, m.y), m.flip, f32::INFINITY);
                }
            };
            if nnf.len() >= PAR_MIN {
                use rayon::prelude::*;
                nnf.par_iter_mut().enumerate().for_each(one);
            } else {
                nnf.iter_mut().enumerate().for_each(one);
            }
        };
        let apply = |solver: &mut Solver, upd: Vec<(usize, Vec<f32>)>| {
            for (p, v) in upd {
                solver.l.img[p * ch..(p + 1) * ch].copy_from_slice(&v);
            }
        };
        if prev.as_ref().is_some_and(|(_, pn, _)| !pn.is_empty()) {
            // Vote with the inherited matches first: searching against the blurry upsampled guess
            // would only pick blurry source patches and lose the texture (research notes).
            recost(&solver, &mut nnf);
            ctl.check()?;
            let upd = solver.vote(&nnf, VOTE_QUANTILE);
            apply(&mut solver, upd);
        }
        let em = if li == 0 && nlev > 1 { (opts.em_iters / 2).max(1) } else { opts.em_iters };
        let local = (4 * psz as i32).min(solver.span);
        for it in 0..em {
            ctl.check()?;
            // Hole pixels moved with the last vote: re-cost every match before searching.
            recost(&solver, &mut nnf);
            // Wide random search on the coarsest level and first iterations; local refinement
            // on finer levels once the inherited field has settled (keeps big holes fast).
            let radius = if li + 1 == nlev || it == 0 { solver.span } else { local };
            for pass in 0..2 {
                ctl.check()?;
                solver.search(&mut nnf, pass + 2 * it, opts.seed.wrapping_add((li * 1000 + it) as u64), radius);
            }
            ctl.check()?;
            let q = if li == 0 && it + 1 == em { FINAL_VOTE_QUANTILE } else { VOTE_QUANTILE };
            let upd = solver.vote(&nnf, q);
            ctl.check()?;
            apply(&mut solver, upd);
        }
        done += solver.l.w * solver.l.h;
        ctl.progress(done as f32 / total.max(1) as f32 * if opts.membrane > 0.0 { 0.9 } else { 1.0 });
        prev = Some((solver.l.w, nnf, solver.l));
    }
    let Some((_, _, fin)) = prev else { return Ok(None) };
    let mut out = img.to_vec();
    for y in 0..h {
        for x in 0..w {
            if hole[y * w + x] {
                let (s, d) = ((y * fin.w + x) * ch, (y * w + x) * ch);
                out[d..d + ch].copy_from_slice(&fin.img[s..s + ch]);
            }
        }
    }
    if opts.membrane > 0.0 {
        // Colour adaptation, second part (Prism's choice): pull the fill's low frequencies
        // towards the smooth membrane of the hole's border (P. Pérez, M. Gangnet, A. Blake,
        // *Poisson Image Editing*, 2003), so a fill sampled from lighter or darker parts of the
        // picture meets its border without a visible edge.
        crate::content_aware::adapt_colors(w, h, ch, img, hole, &mut out, opts.membrane.min(1.0), ctl)?;
    }
    ctl.progress(1.0);
    Ok(Some(out))
}

#[cfg(test)]
mod tests;
