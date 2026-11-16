//! Distortions (inverse mapping + bilinear resampling): Twirl, Pinch,
//! Spherize, Wave, Ripple, Polar Coordinates. All are centred on the
//! reference bounds.

use prism_geom::Rect;

use crate::image::{Edge, Image};
use crate::noise::hash01;
use crate::other::edge_of;
use crate::{Ctx, PolarMode, RippleSize, SpherizeMode, UndefinedAreas, WaveType};

pub(crate) fn remap(src: &Image, out: Rect, ctx: &Ctx, edge: Edge, f: impl Fn(f32, f32) -> (f32, f32)) -> Vec<f32> {
    let n = src.ch;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (sx, sy) = f(x as f32 + 0.5, y as f32 + 0.5);
            src.sample(sx, sy, edge, ctx.bounds, ctx.alpha, &mut tmp);
            res.extend_from_slice(&tmp);
        }
    }
    res
}

pub(crate) fn centre(b: Rect) -> (f32, f32, f32) {
    let (w, h) = (b.width() as f32, b.height() as f32);
    (b.x0 as f32 + w / 2.0, b.y0 as f32 + h / 2.0, w.min(h) / 2.0)
}

/// The point's offset from the centre of `b` in units of the half width and half height: inside
/// the inscribed ellipse its length is under 1. Photoshop's Twirl, Pinch and Spherize act inside
/// that ellipse only (`docs/filter-analysis.md`).
fn ellipse(b: Rect, x: f32, y: f32) -> (f32, f32, f32, f32, f32, f32) {
    let (cx, cy, _) = centre(b);
    let (hx, hy) = ((b.width() as f32 / 2.0).max(0.5), (b.height() as f32 / 2.0).max(0.5));
    let (nx, ny) = ((x - cx) / hx, (y - cy) / hy);
    (cx, cy, hx, hy, nx, ny)
}

/// Twirl (research formula from Photoshop's plug-in): the angle turns by −angle/57 × (1 − r)²
/// radians (Photoshop divides by 57, not 57.3), r measured to the inscribed ellipse.
pub(crate) fn twirl(src: &Image, out: Rect, ctx: &Ctx, angle: f32) -> Vec<f32> {
    let k = -angle / 57.0;
    remap(src, out, ctx, Edge::Repeat, |x, y| {
        let (cx, cy, hx, hy, nx, ny) = ellipse(ctx.bounds, x, y);
        let t = 1.0 - nx.hypot(ny);
        if t <= 0.0 {
            return (x, y);
        }
        let (s, c) = (k * t * t).sin_cos();
        (cx + (nx * c - ny * s) * hx, cy + (nx * s + ny * c) * hy)
    })
}

/// Pinch (research formula): with u = 1 − r inside the inscribed ellipse, a pixel samples the
/// centre's offset scaled by 1 + u²(1 − u/2)² × amount/32.
pub(crate) fn pinch(src: &Image, out: Rect, ctx: &Ctx, amount: f32) -> Vec<f32> {
    let a = amount.clamp(-100.0, 100.0) / 32.0;
    remap(src, out, ctx, Edge::Repeat, |x, y| {
        let (cx, cy, _, _, nx, ny) = ellipse(ctx.bounds, x, y);
        let u = 1.0 - nx.hypot(ny);
        if u <= 0.0 {
            return (x, y);
        }
        let k = u * u * (1.0 - u / 2.0) * (1.0 - u / 2.0) * a;
        (cx + (x - cx) * (1.0 + k), cy + (y - cy) * (1.0 + k))
    })
}

/// Spherize's radius map on [0, 1): a positive amount samples r(1 − a) + a·asin(r)/(π/2), a negative
/// one r(1 − |a|) + |a|·sin(πr/2) (research formula).
fn sphere_1d(rn: f32, a: f32) -> f32 {
    let target = if a >= 0.0 { rn.asin() * 2.0 / std::f32::consts::PI } else { (rn * std::f32::consts::FRAC_PI_2).sin() };
    rn + a.abs() * (target - rn)
}

/// Spherize inside the inscribed ellipse (Normal), or along one axis only.
pub(crate) fn spherize(src: &Image, out: Rect, ctx: &Ctx, amount: f32, mode: SpherizeMode) -> Vec<f32> {
    let a = (amount / 100.0).clamp(-1.0, 1.0);
    remap(src, out, ctx, Edge::Repeat, |x, y| {
        let (cx, cy, hx, hy, nx, ny) = ellipse(ctx.bounds, x, y);
        let (nx, ny) = match mode {
            SpherizeMode::Normal => (nx, ny),
            SpherizeMode::HorizontalOnly => (nx, 0.0),
            SpherizeMode::VerticalOnly => (0.0, ny),
        };
        let r = nx.hypot(ny);
        if r >= 1.0 || r <= 0.0 {
            return (x, y);
        }
        let k = sphere_1d(r, a) / r;
        let sx = if mode == SpherizeMode::VerticalOnly { x } else { cx + nx * k * hx };
        let sy = if mode == SpherizeMode::HorizontalOnly { y } else { cy + ny * k * hy };
        (sx, sy)
    })
}

/// Wave generator settings.
pub(crate) struct WaveSpec {
    pub generators: u32,
    pub wavelength: (f32, f32),
    pub amplitude: (f32, f32),
    pub wave_type: WaveType,
    pub undefined: UndefinedAreas,
    pub seed: u32,
}

fn shape(t: WaveType, phase: f32) -> f32 {
    let p = phase.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
    match t {
        WaveType::Sine => (phase).sin(),
        WaveType::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
        WaveType::Square => {
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}

pub(crate) fn wave(src: &Image, out: Rect, ctx: &Ctx, w: WaveSpec) -> Vec<f32> {
    let n = w.generators.clamp(1, 64);
    let lerp = |(a, b): (f32, f32), t: f32| a + (b - a).max(0.0) * t;
    let gens: Vec<(f32, f32, f32, f32)> = (0..n)
        .map(|i| {
            let r = |k: u32| hash01(i as i32, k as i32, 7, w.seed);
            (lerp(w.wavelength, r(1)).max(1.0), lerp(w.amplitude, r(2)), r(3) * std::f32::consts::TAU, r(4) * std::f32::consts::TAU)
        })
        .collect();
    let b = ctx.bounds;
    remap(src, out, ctx, edge_of(w.undefined), |x, y| {
        let (lx, ly) = (x - b.x0 as f32, y - b.y0 as f32);
        let (mut dx, mut dy) = (0.0, 0.0);
        for &(len, amp, p1, p2) in &gens {
            dx += amp * shape(w.wave_type, std::f32::consts::TAU * ly / len + p1);
            dy += amp * shape(w.wave_type, std::f32::consts::TAU * lx / len + p2);
        }
        (x + dx / n as f32, y + dy / n as f32)
    })
}

/// Ripple. Photoshop tiles a stored 100 × 100 displacement table per size and moves each pixel by
/// table × amount / 128 (research, `docs/filter-analysis.md`). Prism does not copy those tables: it
/// builds its own 100-periodic field of twelve sine waves with the same strength (RMS 0.74 / 1.07
/// / 3.20 px for Small / Medium / Large) and grain (correlation gone after 7 / 8 / 14 px).
pub(crate) fn ripple(src: &Image, out: Rect, ctx: &Ctx, amount: f32, size: RippleSize) -> Vec<f32> {
    const N: usize = 100;
    let (rms, grain, salt) = match size {
        RippleSize::Small => (0.742f32, 7.0f32, 1u32),
        RippleSize::Medium => (1.074, 8.0, 2),
        RippleSize::Large => (3.200, 14.0, 3),
    };
    let k0 = ((N as f32 / (4.0 * grain)).round() as i32).max(1);
    let mut fx = vec![0.0f32; N * N];
    let mut fy = vec![0.0f32; N * N];
    let tau = std::f32::consts::TAU;
    for g in 0..12 {
        let pick = |c: u32| hash01(g, c as i32, 70, salt);
        let kx = k0 + (pick(0) * 3.0) as i32 - 1;
        let ky = (pick(1) * 3.0) as i32 - 1;
        let (p1, p2) = (pick(2) * tau, pick(3) * tau);
        for j in 0..N {
            for i in 0..N {
                let (fi, fj) = (i as f32, j as f32);
                fx[j * N + i] += (tau * (kx as f32 * fi + ky as f32 * fj) / N as f32 + p1).sin();
                fy[j * N + i] += (tau * (ky as f32 * fi + kx as f32 * fj) / N as f32 + p2).sin();
            }
        }
    }
    let power = (fx.iter().map(|v| v * v).sum::<f32>() / (N * N) as f32).sqrt().max(1e-6);
    let scale = rms / power * amount / 128.0;
    let b = ctx.bounds;
    remap(src, out, ctx, Edge::Repeat, |x, y| {
        let i = ((x - b.x0 as f32).floor() as i64).rem_euclid(N as i64) as usize;
        let j = ((y - b.y0 as f32).floor() as i64).rem_euclid(N as i64) as usize;
        (x + fx[j * N + i] * scale, y + fy[j * N + i] * scale)
    })
}

/// Polar Coordinates as Photoshop maps it (measured, `docs/filter-analysis.md`): counter-clockwise
/// from 12 o'clock, over the ellipse inscribed in the bounds (the corners past it repeat the
/// edge rows), taking the nearest source pixel. Rectangular to Polar: the angle picks the source
/// column (wrapping), the distance the row (top row at the centre). Polar to Rectangular: the
/// inverse.
pub(crate) fn polar(src: &Image, out: Rect, ctx: &Ctx, mode: PolarMode) -> Vec<f32> {
    let b = ctx.bounds;
    let (w, h) = (b.width().max(1) as f32, b.height().max(1) as f32);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let (cx, cy) = (b.x0 as f32 + hw, b.y0 as f32 + hh);
    let n = src.ch;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let tau = std::f32::consts::TAU;
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (sx, sy, wrap) = match mode {
                PolarMode::RectangularToPolar => {
                    let (dx, dy) = ((x as f32 - cx) / hw, (y as f32 - cy) / hh);
                    let theta = (-dx).atan2(-dy).rem_euclid(tau);
                    (b.x0 as f32 + theta / tau * w, b.y0 as f32 + dx.hypot(dy) * h, true)
                }
                PolarMode::PolarToRectangular => {
                    let theta = ((x - b.x0) as f32 + 1.0) / w * tau;
                    let r = ((y - b.y0) as f32 + 0.5) / h;
                    (cx - theta.sin() * r * hw + 0.5, cy - theta.cos() * r * hh + 0.5, false)
                }
            };
            let (sx, sy) = (sx.round() as i32, sy.round() as i32);
            let sx = if wrap { b.x0 + (sx - b.x0).rem_euclid(b.width().max(1) as i32) } else { sx };
            for c in 0..n {
                res.push(src.get_edge(sx, sy, c, Edge::Repeat, b));
            }
        }
    }
    res
}
