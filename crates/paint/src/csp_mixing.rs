//! Independently implemented CSP color mixing ("Color mixing" with Amount of paint, Density of
//! paint and Color stretch). Source evidence and limits are in docs/sut-analysis.md.
//!
//! While each dab is laid, the pixels under its footprint (the layer with the stroke so far
//! composited over it, the stroke coverage acting as alpha) are sampled. Before the next dab,
//! those samples update a picked-up colour: the mean alpha over all sampled pixels and the
//! alpha-weighted mean colour, blended into the previous pick with weight
//! `0.05 / (stretch + 0.05)` (the first pick is taken as is). Each dab then paints
//! `(brush · a² + picked · (1 − a²) · A) / (a² + (1 − a²) · A)` for amount of paint `a` and
//! picked alpha `A`, with its opacity ceiling scaled by `(1 − d) · A + d` for density of paint `d`.
//! The perceptual mixing mode does all colour averaging in [`to_perceptual`] space.
use serde::{Deserialize, Serialize};

use crate::Dynamic;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CspColorMixing {
    /// Amount of paint, 0..1 (1 = only the brush colour).
    pub amount: f32,
    /// Density of paint, 0..1 (1 = full opacity wherever the brush goes).
    pub density: f32,
    /// Color stretch, 0..1 (how long a picked colour persists).
    pub stretch: f32,
    pub amount_dynamic: Dynamic,
    pub density_dynamic: Dynamic,
    /// Perceptual colour mixing (CSP "Color mixing mode"): average colours in a PQ-encoded cone
    /// space instead of RGB.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub perceptual: bool,
}

impl Default for CspColorMixing {
    fn default() -> Self {
        Self { amount: 1.0, density: 1.0, stretch: 0.0, amount_dynamic: Dynamic::default(), density_dynamic: Dynamic::default(), perceptual: false }
    }
}

impl CspColorMixing {
    pub fn bounded(&self) -> Self {
        let unit = |v: f32, d: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
        Self { amount: unit(self.amount, 1.0), density: unit(self.density, 1.0), stretch: unit(self.stretch, 0.0), ..self.clone() }
    }
}

/// sRGB primaries (D65) to XYZ, rows.
const RGB_TO_XYZ: [[f64; 3]; 3] = [[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]];
/// Hunt-Pointer-Estevez cone response, rows.
const XYZ_TO_LMS: [[f64; 3]; 3] = [[0.38971, 0.68898, -0.07868], [-0.22981, 1.18340, 0.04641], [0.0, 0.0, 1.0]];
const WHITE: [f64; 3] = [0.95047, 1.0, 1.08883];
// PQ (SMPTE ST 2084) with the constants CSP uses, white at 80 of 10000 cd/m².
const PQ_M1: f64 = 0.159_301_757_812_5;
const PQ_M2: f64 = 78.843_75;
const PQ_C1: f64 = 0.835_937_5;
const PQ_C2: f64 = 18.875;
const PQ_C3: f64 = 18.6875;
const PQ_WHITE: f64 = 80.0 / 10000.0;

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}

fn inverse(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let c = |r: usize, k: usize| m[(r + 1) % 3][(k + 1) % 3] * m[(r + 2) % 3][(k + 2) % 3] - m[(r + 1) % 3][(k + 2) % 3] * m[(r + 2) % 3][(k + 1) % 3];
    let det = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (k, v) in row.iter_mut().enumerate() {
            *v = c(k, r) / det;
        }
    }
    out
}

/// RGB to white-normalised cone response.
fn rgb_to_lms() -> [[f64; 3]; 3] {
    let white = mul(&XYZ_TO_LMS, WHITE);
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (k, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|j| XYZ_TO_LMS[i][j] * RGB_TO_XYZ[j][k]).sum::<f64>() / white[i];
        }
    }
    m
}

/// CSP's perceptual mixing space: the RGB values (taken as linear, CSP's default curve) through
/// sRGB primaries to white-normalised Hunt-Pointer-Estevez cones, each PQ-encoded. Mixing
/// averages here and converts back with [`from_perceptual`].
pub(crate) fn to_perceptual(rgb: [f64; 3]) -> [f64; 3] {
    mul(&rgb_to_lms(), rgb.map(finite)).map(|v| {
        let n = (v.clamp(0.0, 1.0) * PQ_WHITE).powf(PQ_M1);
        ((PQ_C1 + PQ_C2 * n) / (1.0 + PQ_C3 * n)).powf(PQ_M2)
    })
}

pub(crate) fn from_perceptual(p: [f64; 3]) -> [f64; 3] {
    let lms = p.map(|v| {
        let e = finite(v).clamp(0.0, 1.0).powf(1.0 / PQ_M2);
        ((e - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * e)).powf(1.0 / PQ_M1) / PQ_WHITE
    });
    mul(&inverse(&rgb_to_lms()), lms).map(|v| v.clamp(0.0, 1.0))
}

/// Weighted average of two colours (weights `wa`, `wb`), in RGB or perceptually.
fn blend(a: [f64; 3], wa: f64, b: [f64; 3], wb: f64, perceptual: bool) -> [f64; 3] {
    let sum = wa + wb;
    if sum <= 0.0 {
        return a;
    }
    let (pa, pb) = if perceptual { (to_perceptual(a), to_perceptual(b)) } else { (a, b) };
    let mut out = [0.0; 3];
    for (o, (x, y)) in out.iter_mut().zip(pa.iter().zip(pb)) {
        *o = (x * wa + y * wb) / sum;
    }
    if perceptual { from_perceptual(out) } else { out }
}

/// Pixels sampled under one dab.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Samples {
    alpha: f64,
    color: [f64; 3],
    count: u64,
    /// The colour sums are in perceptual space.
    perceptual: bool,
}

impl Samples {
    /// Add one layer pixel (straight RGBA) blended towards the stroke paint (straight RGBA, its
    /// alpha the target alpha) with stroke weight `c`.
    pub fn add(&mut self, layer: [f32; 4], c: f32, stroke: [f32; 4]) {
        let c = f64::from(c.clamp(0.0, 1.0));
        let under = f64::from(layer[3].clamp(0.0, 1.0)) * (1.0 - c);
        let over = f64::from(stroke[3].clamp(0.0, 1.0)) * c;
        let alpha = under + over;
        self.count += 1;
        if alpha <= 0.0 {
            return;
        }
        self.alpha += alpha;
        let mut color = [0.0; 3];
        for (o, (l, s)) in color.iter_mut().zip(layer.iter().zip(stroke.iter())) {
            *o = (f64::from(*l) * under + f64::from(*s) * over) / alpha;
        }
        if self.perceptual {
            color = to_perceptual(color);
        }
        for (sum, v) in self.color.iter_mut().zip(color) {
            *sum += v * alpha;
        }
    }

    pub fn new(perceptual: bool) -> Self {
        Self { perceptual, ..Default::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// The picked-up colour carried along the stroke.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Reservoir {
    picked: bool,
    alpha: f64,
    color: [f64; 3],
}

impl Reservoir {
    pub fn picked(&self) -> bool {
        self.picked
    }

    /// Fold the samples of the previous dab in.
    pub fn update(&mut self, s: &Samples, stretch: f32) {
        if s.count == 0 {
            return;
        }
        let alpha = s.alpha / s.count as f64;
        // No paint under the dab reads as white (it is weighted by zero alpha below).
        let color = if s.alpha > 0.0 { s.color.map(|c| c / s.alpha) } else { [1.0; 3] };
        let color = if s.perceptual && s.alpha > 0.0 { from_perceptual(color) } else { color };
        if !self.picked {
            (self.picked, self.alpha, self.color) = (true, alpha, color);
            return;
        }
        let k = if stretch > 0.0 { 0.05 / (f64::from(stretch) + 0.05) } else { 1.0 };
        if self.alpha <= 0.0 {
            self.color = color;
            self.alpha = alpha * k;
        } else if alpha <= 0.0 {
            self.alpha *= 1.0 - k;
        } else {
            self.color = blend(self.color, 1.0 - k, color, k, s.perceptual);
            self.alpha = self.alpha * (1.0 - k) + alpha * k;
        }
    }

    /// The dab's colour (straight RGB; alpha kept) and opacity-ceiling multiplier.
    pub fn mix(&self, brush: [f32; 4], amount: f32, density: f32, perceptual: bool) -> ([f32; 4], f32) {
        if !self.picked {
            return (brush, 1.0);
        }
        let a = self.alpha.clamp(0.0, 1.0);
        let d = f64::from(density.clamp(0.0, 1.0));
        let flow = ((1.0 - d) * a + d) as f32;
        let amount = f64::from(amount.clamp(0.0, 1.0));
        if flow <= 0.0 || amount >= 0.99999999 {
            return (brush, flow);
        }
        let rgb = if amount <= 1e-8 {
            if a <= 0.0 {
                return (brush, flow);
            }
            self.color
        } else {
            let wb = amount * amount;
            let brush_rgb = [f64::from(brush[0]), f64::from(brush[1]), f64::from(brush[2])];
            blend(brush_rgb, wb, self.color, (1.0 - wb) * a, perceptual)
        };
        ([rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, brush[3]], flow)
    }
}

/// Blend a layer pixel towards mixed paint: colour and alpha both move by weight `k` (paint
/// can lower the alpha of what it covers). Straight RGBA in and out.
pub(crate) fn blend_towards(under: [f32; 4], paint: [f32; 4], target_alpha: f32, k: f32, lock_alpha: bool) -> [f32; 4] {
    let k = k.clamp(0.0, 1.0);
    let (ua, ta) = (under[3].clamp(0.0, 1.0), target_alpha.clamp(0.0, 1.0));
    let a = ua * (1.0 - k) + ta * k;
    let mut out = [0.0, 0.0, 0.0, a];
    if a > 0.0 {
        for (o, (u, p)) in out.iter_mut().zip(under.iter().zip(paint.iter())).take(3) {
            *o = (u * ua * (1.0 - k) + p * ta * k) / a;
        }
    }
    if lock_alpha {
        out[3] = under[3];
    }
    out
}

#[cfg(test)]
#[path = "csp_mixing_tests.rs"]
mod tests;
