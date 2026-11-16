//! Image › Adjustments › Brightness/Contrast: the curves and the dialog's Auto button, written
//! from the maintainer's research notes on Photoshop 2026 (`docs/adjust-analysis.md`; the notes
//! give each function's address in the binary). Prism's code is independent; only the formulas and
//! constants are taken from the notes.
//!
//! * Modern brightness: `B = b · 0.02` split in two stages `s1 = min(|B|, 2)`, `s2 = max(|B| − 2, 0)`.
//!   A stage is a line of slope `p = 1.3703509847201236^s` up to output 0.5 (input `e = 0.5/p`),
//!   then a cubic Hermite to (1, 1) with end slope `k = max(0.1, 1/(1 + 12 (p − 1)))`. Brighter
//!   runs stage 1 then stage 2; darker runs the inverse of stage 2 then of stage 1 (secant method).
//! * Modern contrast: `x += c · 0.00095 · (x > 0.5 ? 1 − (4(x − 0.75))² : (4(x − 0.25))² − 1)`.
//! * Legacy: integer straight lines about the image's mean level (see [`mean_level`]).
//! * Auto: [`auto`] fits (brightness, contrast) to a target curve by minimising a weighted RMS.

/// Brightness factor per slider unit.
const B_SCALE: f64 = 0.02;
/// `2^(1/2.2)`: a stage's slope is this to the power of the stage amount.
const STAGE_BASE: f64 = 1.370_350_984_720_123_6;
/// Contrast factor per slider unit.
const C_SCALE: f64 = 0.000_95;

#[derive(Clone, Copy)]
struct Stage {
    p: f64,
    k: f64,
    e: f64,
}

fn stage(s: f64) -> Stage {
    let p = STAGE_BASE.powf(s);
    let k = (1.0 / ((p - 1.0) * 12.0 + 1.0)).max(0.1);
    Stage { p, k, e: 0.5 / p }
}

fn forward(st: Stage, x: f64) -> f64 {
    if st.e > x {
        return st.p * x;
    }
    let l = 1.0 - st.e;
    let t = (x - st.e) / l;
    let u = 1.0 - t;
    let a = 0.5 * (1.0 + 2.0 * t) + t * (l * st.p);
    let b = (3.0 - 2.0 * t) - u * (l * st.k);
    a * (u * u) + t * t * b
}

/// Solves `forward(st, x) = y` by the secant method from 0 and 1 (at most 30 steps).
fn inverse(st: Stage, y: f64) -> f64 {
    let (mut x_prev, mut x_cur) = (0.0, 1.0);
    let (mut f_prev, mut f_cur) = (forward(st, 0.0), forward(st, 1.0));
    let mut x = x_cur;
    for _ in 0..30 {
        let d = f_cur - f_prev;
        if d.abs() < 1e-10 {
            return x_cur;
        }
        x = (x_cur + (y - f_cur) * (x_cur - x_prev) / d).clamp(0.0, 1.0);
        let fx = forward(st, x);
        (f_prev, x_prev) = (f_cur, x_cur);
        (f_cur, x_cur) = (fx, x);
    }
    x
}

/// The modern brightness curve for a slider value.
#[derive(Clone, Copy)]
struct Brightness {
    negative: bool,
    s1: f64,
    s2: f64,
    st1: Stage,
    st2: Stage,
}

impl Brightness {
    fn new(brightness: i32) -> Self {
        let b = f64::from(brightness) * B_SCALE;
        let a = b.abs();
        let (s1, s2) = (a.min(2.0), (a - 2.0).max(0.0));
        Self { negative: b < 0.0, s1, s2, st1: stage(s1), st2: stage(s2) }
    }

    fn apply(&self, mut x: f64) -> f64 {
        if self.negative {
            if self.s2 != 0.0 {
                x = inverse(self.st2, x);
            }
            if self.s1 != 0.0 {
                x = inverse(self.st1, x);
            }
        } else {
            if self.s1 != 0.0 {
                x = forward(self.st1, x);
            }
            if self.s2 != 0.0 {
                x = forward(self.st2, x);
            }
        }
        x
    }
}

fn contrast(c: f64, x: f64) -> f64 {
    if c == 0.0 {
        return x;
    }
    let f = if x > 0.5 { 1.0 - ((x - 0.75) * 4.0).powi(2) } else { ((x - 0.25) * 4.0).powi(2) - 1.0 };
    x + c * f
}

/// A slider value as Photoshop's integer, whatever the float (NaN → 0).
fn slider(v: f32, lo: i32, hi: i32) -> i32 {
    if v.is_finite() { (v.round() as i32).clamp(lo, hi) } else { 0 }
}

/// Modern Brightness/Contrast on one value in 0..=1 (unclamped curve, then clamped).
pub fn modern(x: f32, brightness: f32, contrast_v: f32) -> f32 {
    let b = slider(brightness, -150, 150);
    let c = slider(contrast_v, -100, 100);
    let mut v = f64::from(x.clamp(0.0, 1.0));
    if b != 0 {
        v = Brightness::new(b).apply(v);
    }
    contrast(f64::from(c) * C_SCALE, v).clamp(0.0, 1.0) as f32
}

/// The 8-bit table (256 entries), as Photoshop builds it for 8-bit documents. `mean` is the legacy
/// contrast pivot.
pub fn table8(brightness: i32, contrast_v: i32, legacy: bool, mean: u8) -> [u8; 256] {
    let mut t = [0u8; 256];
    if !legacy {
        let bc = Brightness::new(brightness);
        let c = f64::from(contrast_v) * C_SCALE;
        for (i, o) in t.iter_mut().enumerate() {
            let mut x = f64::from(i as f32 * (1.0f32 / 255.0));
            if brightness != 0 {
                x = bc.apply(x);
            }
            x = contrast(c, x);
            *o = ((x as f32 * 255.0 + 0.5) as i32).clamp(0, 255) as u8;
        }
        return t;
    }
    let (b, c, pivot) = (brightness, contrast_v.clamp(-100, 100), i32::from(mean));
    for (i, o) in t.iter_mut().enumerate() {
        let i = i as i32;
        let v = if c >= 0 {
            let d = 255 - (c * 255) / 100;
            let thr = pivot - b;
            if c == 100 {
                if i < thr { 0 } else { 255 }
            } else if i < thr {
                pivot - ((thr - i) * 255 + d / 2) / d
            } else {
                pivot + ((i - thr) * 255 + d / 2) / d
            }
        } else {
            let m = (c * 255) / 100 + 255;
            let s = if i < pivot { -((m * (pivot - i) + 127) / 255) } else { (m * (i - pivot) + 127) / 255 };
            s + b + pivot
        };
        *o = v.clamp(0, 255) as u8;
    }
    t
}

/// Steps of the 16-bit table: Photoshop's 16-bit samples run 0..=32768.
const ONE16: i64 = 32768;

/// The 16-bit table (4097 entries over 0..=32768, every 8th sample), as Photoshop builds it for
/// 16-bit documents and for Auto's fit.
pub fn table16(brightness: i32, contrast_v: i32, legacy: bool, mean: u8) -> Vec<i32> {
    if !legacy {
        let bc = Brightness::new(brightness);
        let c = f64::from(contrast_v) * C_SCALE;
        return (0..=4096)
            .map(|j: i32| {
                let mut x = f64::from((j * 8) as f32 * (1.0f32 / 32768.0));
                if brightness != 0 {
                    x = bc.apply(x);
                }
                x = contrast(c, x);
                ((x as f32 * 32768.0 + 0.5) as i32).clamp(0, 32768)
            })
            .collect();
    }
    // The 8-bit lines with the pivot and the brightness moved to 16 bits as (v · 257 + 1) / 2.
    let c = i64::from(contrast_v.clamp(-100, 100));
    let p16 = (i64::from(mean) * 257 + 1) >> 1;
    let b16 = (i64::from(brightness) * 257 + 1) >> 1;
    let thr = p16 - b16;
    (0..=4096i64)
        .map(|j| {
            let i = j * 8;
            let v = if c >= 0 {
                let d = ONE16 - (c * ONE16) / 100;
                if c == 100 {
                    if i >= thr { ONE16 } else { 0 }
                } else if i < thr {
                    p16 - ((thr - i) * ONE16 + d / 2) / d
                } else {
                    p16 + ((i - thr) * ONE16 + d / 2) / d
                }
            } else {
                let m = ONE16 + (c * ONE16) / 100;
                let s = if i < p16 { -((m * (p16 - i) + 16383) / ONE16) } else { (m * (i - p16) + 16383) / ONE16 };
                s + p16 + b16
            };
            v.clamp(0, ONE16) as i32
        })
        .collect()
}

/// Brightness/Contrast as one channel LUT over 0..=1 for a document of `bits` (8 → Photoshop's
/// 8-bit table, anything else → its 16-bit table), sampled at `n` points (`n ≥ 2`).
pub fn lut(brightness: f32, contrast_v: f32, legacy: bool, mean: u8, eight_bit: bool, n: usize) -> Vec<f32> {
    let b = slider(brightness, -150, 150);
    let c = slider(contrast_v, -100, 100);
    let n = n.max(2);
    let sample = |t: &[f32], x: f32| {
        let f = x * (t.len() - 1) as f32;
        let i = (f.floor() as usize).min(t.len() - 1);
        let j = (i + 1).min(t.len() - 1);
        let w = f - i as f32;
        t.get(i).copied().unwrap_or(0.0) * (1.0 - w) + t.get(j).copied().unwrap_or(0.0) * w
    };
    let table: Vec<f32> = if eight_bit {
        table8(b, c, legacy, mean).iter().map(|&v| f32::from(v) / 255.0).collect()
    } else {
        table16(b, c, legacy, mean).iter().map(|&v| v as f32 / 32768.0).collect()
    };
    (0..n).map(|k| sample(&table, k as f32 / (n - 1) as f32)).collect()
}

/// Photoshop's mean level: the mean of a 256-bin histogram of the luminance
/// `0.299 R + 0.587 G + 0.114 B` (straight RGBA in 0..=1; transparent pixels left out), rounded.
/// Legacy contrast pivots about it. 127 for an empty image.
pub fn mean_level(rgba: &[[f32; 4]]) -> u8 {
    let mut h = [0u64; 256];
    for p in rgba.iter().filter(|p| p[3] > 0.0) {
        let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        let bin = if l.is_finite() { ((l * 255.0 + 0.5) as i32).clamp(0, 255) as usize } else { 0 };
        if let Some(c) = h.get_mut(bin) {
            *c += 1;
        }
    }
    let n: u64 = h.iter().sum();
    if n == 0 {
        return 127;
    }
    let s: f64 = h.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    (s / n as f64).round().clamp(0.0, 255.0) as u8
}

/// What Auto chose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Auto {
    pub brightness: i32,
    pub contrast: i32,
    /// The image's mean level (the legacy pivot).
    pub mean: u8,
    /// The fit's weighted RMS error.
    pub rms: f64,
}

/// A 16-bit table looked up the way Auto's error does: `((x · 32768) << 12 + 0x4000) >> 15`.
fn lookup16(t: &[i32], x: f64) -> f64 {
    let v = if x < 0.0 { 0.0 } else if x > 1.0 { 32768.0 } else { x * 32768.0 };
    let idx = (((v as u64) << 12) + 0x4000) >> 15;
    f64::from(t.get(idx.min(4096) as usize).copied().unwrap_or(0)) / 32768.0
}

struct Fit<'a> {
    x: &'a [f64],
    y: &'a [f64],
    w: &'a [f64],
    legacy: bool,
    mean: u8,
    lo: [f64; 2],
    hi: [f64; 2],
    evaluations: u32,
}

impl Fit<'_> {
    /// `f64::MAX` outside the slider ranges, else `sqrt(Σ w (f(x) − y)² / Σ w)`; the values are cut
    /// toward zero to whole steps first.
    fn error(&mut self, p: [f64; 2]) -> f64 {
        self.evaluations += 1;
        if (0..2).any(|k| !(self.lo[k]..=self.hi[k]).contains(&p[k])) {
            return f64::MAX;
        }
        let t = table16(p[0] as i32, p[1] as i32, self.legacy, self.mean);
        let (mut sw, mut se) = (0.0, 0.0);
        for (i, (&x, &y)) in self.x.iter().zip(self.y).enumerate() {
            let w = self.w.get(i).copied().unwrap_or(1.0);
            let e = lookup16(&t, x) - y;
            se += w * e * e;
            sw += w;
        }
        if sw > 0.0 { (se / sw).sqrt() } else { f64::NAN }
    }

    /// Two-variable Nelder–Mead from `p` (start simplex ±10 on each axis), stopping when the error
    /// spread is under `tol` and the simplex is within half a step, or after `max_eval` errors.
    fn minimise(&mut self, p: [f64; 2], max_eval: u32, tol: f64) -> [f64; 2] {
        let mut s = [p, [p[0] + 10.0, p[1]], [p[0], p[1] + 10.0]];
        let mut f = s.map(|q| self.error(q));
        let order = |f: &[f64; 3]| {
            let mut o = [0usize, 1, 2];
            o.sort_by(|&a, &b| f[a].total_cmp(&f[b]));
            o
        };
        while self.evaluations < max_eval {
            let [best, mid, worst] = order(&f);
            let size = (s[worst][0] - s[best][0]).abs().max((s[worst][1] - s[best][1]).abs());
            if (f[worst] - f[best]).abs() < tol && size < 0.5 {
                break;
            }
            let c = [(s[best][0] + s[mid][0]) / 2.0, (s[best][1] + s[mid][1]) / 2.0];
            let r = [2.0 * c[0] - s[worst][0], 2.0 * c[1] - s[worst][1]];
            let fr = self.error(r);
            if fr < f[best] {
                let e = [3.0 * c[0] - 2.0 * s[worst][0], 3.0 * c[1] - 2.0 * s[worst][1]];
                let fe = self.error(e);
                (s[worst], f[worst]) = if fe < fr { (e, fe) } else { (r, fr) };
            } else if fr < f[mid] {
                (s[worst], f[worst]) = (r, fr);
            } else {
                let h = [(c[0] + s[worst][0]) / 2.0, (c[1] + s[worst][1]) / 2.0];
                let fh = self.error(h);
                if fh < f[worst] {
                    (s[worst], f[worst]) = (h, fh);
                } else {
                    for i in [mid, worst] {
                        s[i] = [(s[i][0] + s[best][0]) / 2.0, (s[i][1] + s[best][1]) / 2.0];
                        f[i] = self.error(s[i]);
                    }
                }
            }
        }
        s[order(&f)[0]]
    }
}

/// Fits (brightness, contrast) to the target curve samples `(x, y)` weighted by `w`, from (0, 0):
/// at most 10000 errors, tolerance 0.0001. The result is cut toward zero; a value beyond its slider
/// becomes a fixed one (modern: brightness ±75, contrast −25 / 50; legacy: ±50), not the end.
pub fn fit(x: &[f64], y: &[f64], w: &[f64], legacy: bool, mean: u8) -> Auto {
    let (lo, hi) = if legacy { ([-100.0, -100.0], [100.0, 100.0]) } else { ([-150.0, -50.0], [150.0, 100.0]) };
    let mut f = Fit { x, y, w, legacy, mean, lo, hi, evaluations: 0 };
    let p = f.minimise([0.0, 0.0], 10_000, 0.0001);
    let rms = f.error(p);
    let (mut b, mut c) = (if p[0].is_finite() { p[0] as i32 } else { 0 }, if p[1].is_finite() { p[1] as i32 } else { 0 });
    if legacy {
        b = if b < -100 { -50 } else if b > 100 { 50 } else { b };
        c = if c < -100 { -50 } else if c > 100 { 50 } else { c };
    } else {
        b = if b < -150 { -75 } else if b > 150 { 75 } else { b };
        c = if c < -50 { -25 } else if c > 100 { 50 } else { c };
    }
    Auto { brightness: b, contrast: c, mean, rms }
}

/// Auto on an image (straight RGBA in 0..=1, `width × height`). The image is halved (2×2 box) until
/// its long side is at most 500 pixels. Photoshop's target curve comes from a trained model that
/// the research didn't port; in its place the target stretches the luminance histogram after
/// clipping 0.1 % at each end, weighted by each level's pixel count (the research's stand-in).
pub fn auto(rgba: &[[f32; 4]], width: usize, height: usize, legacy: bool) -> Auto {
    let (mut img, mut w, mut h) = (rgba.get(..width.saturating_mul(height)).unwrap_or(rgba).to_vec(), width, height);
    if img.len() != w.saturating_mul(h) {
        (w, h) = (img.len(), 1);
    }
    while w.max(h) > 500 && w > 1 && h > 1 {
        let (hw, hh) = (w / 2, h / 2);
        let mut half = vec![[0.0f32; 4]; hw * hh];
        for (k, o) in half.iter_mut().enumerate() {
            let (x, y) = (k % hw, k / hw);
            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let p = img.get((2 * y + dy) * w + 2 * x + dx).copied().unwrap_or_default();
                for c in 0..4 {
                    o[c] += p[c] / 4.0;
                }
            }
        }
        (img, w, h) = (half, hw, hh);
    }
    let mean = mean_level(&img);
    let mut hist = [0u64; 256];
    for p in img.iter().filter(|p| p[3] > 0.0) {
        let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        if l.is_finite()
            && let Some(c) = hist.get_mut(((l * 255.0 + 0.5) as i32).clamp(0, 255) as usize)
        {
            *c += 1;
        }
    }
    let n: u64 = hist.iter().sum();
    let cut = n / 1000;
    let (mut lo, mut hi) = (0usize, 255usize);
    let mut acc = 0;
    while lo < 255 {
        acc += hist[lo];
        if acc > cut {
            break;
        }
        lo += 1;
    }
    acc = 0;
    while hi > 0 {
        acc += hist[hi];
        if acc > cut {
            break;
        }
        hi -= 1;
    }
    if hi <= lo {
        (lo, hi) = (0, 255);
    }
    let xs: Vec<f64> = (0..256).map(|i| i as f64 / 255.0).collect();
    let ys: Vec<f64> = (0..256).map(|i| ((i as f64 - lo as f64) / (hi - lo) as f64).clamp(0.0, 1.0)).collect();
    let ws: Vec<f64> = hist.iter().map(|&c| c as f64).collect();
    fit(&xs, &ys, &ws, legacy, mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_curve_is_the_identity_at_zero_and_pins_the_ends() {
        let t = table8(0, 0, false, 127);
        assert!(t.iter().enumerate().all(|(i, &v)| usize::from(v) == i));
        for (b, c) in [(150, 100), (-150, -50), (73, 0), (0, -41)] {
            let t = table8(b, c, false, 127);
            assert_eq!((t[0], t[255]), (0, 255), "{b} {c}");
            assert!(t.windows(2).all(|w| w[0] <= w[1]), "monotone {b} {c}");
        }
    }

    #[test]
    fn darker_is_the_inverse_of_brighter() {
        for b in [30, 100, 140] {
            for x in [0.1, 0.45, 0.8] {
                let up = Brightness::new(b).apply(x);
                assert!((Brightness::new(-b).apply(up) - x).abs() < 1e-6, "{b} {x}");
            }
        }
    }

    #[test]
    fn legacy_pivots_about_the_mean() {
        // Contrast 50 doubles the distance from the mean; contrast 100 is a threshold there.
        let t = table8(0, 50, true, 60);
        assert_eq!((t[60], t[70], t[50]), (60, 80, 40));
        let t = table8(0, 100, true, 60);
        assert_eq!((t[59], t[60]), (0, 255));
        let t = table8(0, -100, true, 84);
        assert!(t.iter().all(|&v| v == 84));
        let t16 = table16(0, 50, true, 60);
        assert_eq!(t16.len(), 4097);
        assert!(t16.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn mean_level_is_the_luminance_histogram_mean() {
        assert_eq!(mean_level(&[[1.0, 0.0, 0.0, 1.0]]), 76);
        assert_eq!(mean_level(&[[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.0]]), 128);
        assert_eq!(mean_level(&[]), 127);
        assert_eq!(mean_level(&[[f32::NAN, 0.0, 0.0, 1.0]]), 0);
    }

    #[test]
    fn fit_finds_a_curve_it_was_given() {
        // Target = the modern curve for (40, 30): the fit comes back to it.
        let t = table16(40, 30, false, 127);
        let xs: Vec<f64> = (0..256).map(|i| f64::from(i) / 255.0).collect();
        let ys: Vec<f64> = xs.iter().map(|&x| lookup16(&t, x)).collect();
        let a = fit(&xs, &ys, &[], false, 127);
        assert!((a.brightness - 40).abs() <= 1 && (a.contrast - 30).abs() <= 1, "{a:?}");
    }

    #[test]
    fn auto_survives_any_image() {
        let ramp: Vec<[f32; 4]> = (0..1200).map(|i| [i as f32 / 1199.0 * 0.4 + 0.2; 4]).map(|mut p| {
            p[3] = 1.0;
            p
        }).collect();
        let a = auto(&ramp, 40, 30, false);
        assert!(a.contrast > 0, "a narrow ramp gets more contrast: {a:?}");
        for (img, w, h) in [(vec![], 0, 0), (vec![[0.5f32; 4]; 3], 9, 9), (vec![[f32::NAN; 4]; 4], 2, 2)] {
            let a = auto(&img, w, h, true);
            assert!((-100..=100).contains(&a.brightness) && (-100..=100).contains(&a.contrast), "{a:?}");
        }
        // A large image is reduced first.
        let big = vec![[0.3f32, 0.4, 0.5, 1.0]; 1100 * 600];
        let a = auto(&big, 1100, 600, false);
        assert_eq!(a.mean, mean_level(&[[0.3, 0.4, 0.5, 1.0]]));
    }
}
