//! Unsharp Mask, Smart Sharpen (basic), High Pass.

use prism_geom::Rect;

use crate::Ctx;
use crate::blur::{conv_sep, gaussian_kernel};
use crate::image::Image;

fn with_blur(src: &Image, out: Rect, ctx: &Ctx, radius: f32, mut f: impl FnMut(f32, f32) -> f32) -> Vec<f32> {
    let k = gaussian_kernel(radius);
    let blurred = conv_sep(src, out, &k, &k, ctx.alpha);
    let mut res = src.crop(out);
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    for (px, bl) in res.chunks_exact_mut(n).zip(blurred.chunks_exact(n)) {
        if ctx.alpha && px[n - 1] <= 0.0 {
            continue;
        }
        for c in 0..cc {
            px[c] = f(px[c], bl[c]);
        }
    }
    res
}

/// Unsharp Mask as Photoshop computes it (measured, `docs/filter-analysis.md`): the difference from
/// the blur, scaled by the amount, loses `threshold` levels of its size, so differences the
/// threshold covers change nothing and larger ones fade in instead of jumping.
pub(crate) fn unsharp(src: &Image, out: Rect, ctx: &Ctx, amount: f32, radius: f32, threshold: f32) -> Vec<f32> {
    let k = amount / 100.0;
    let t = threshold.max(0.0) / 255.0;
    with_blur(src, out, ctx, radius, |o, b| {
        let d = k * (o - b);
        o + d.signum() * (d.abs() - t).max(0.0)
    })
}

/// The colour channels of `src` around `out`, with what a 3×3 kernel needs: the pixel, the sum of
/// its four side neighbours, the sum of all eight, and its Sobel gradient size in levels.
fn neighbourhood(src: &Image, out: Rect, ctx: &Ctx, mut f: impl FnMut(f32, f32, f32, f32) -> f32) -> Vec<f32> {
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    let mut res = src.crop(out);
    let w = out.width() as usize;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        if ctx.alpha && px[n - 1] <= 0.0 {
            continue;
        }
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        for (c, v) in px.iter_mut().enumerate().take(cc) {
            let g = |dx: i32, dy: i32| src.get(x + dx, y + dy, c);
            let sides = g(0, -1) + g(-1, 0) + g(1, 0) + g(0, 1);
            let corners = g(-1, -1) + g(1, -1) + g(-1, 1) + g(1, 1);
            let gx = (g(1, -1) + 2.0 * g(1, 0) + g(1, 1)) - (g(-1, -1) + 2.0 * g(-1, 0) + g(-1, 1));
            let gy = (g(-1, 1) + 2.0 * g(0, 1) + g(1, 1)) - (g(-1, -1) + 2.0 * g(0, -1) + g(1, -1));
            *v = f(*v, sides, sides + corners, gx.hypot(gy) * 255.0);
        }
    }
    res
}

/// Sharpen and Sharpen More: Photoshop's fixed kernels (measured): Sharpen is 2 at the centre and
/// −¼ on the four sides, Sharpen More 3 at the centre and −¼ on all eight neighbours.
pub(crate) fn sharpen(src: &Image, out: Rect, ctx: &Ctx, more: bool) -> Vec<f32> {
    neighbourhood(src, out, ctx, |p, sides, ring, _| if more { 3.0 * p - ring / 4.0 } else { 2.0 * p - sides / 4.0 })
}

/// Blur and Blur More: Photoshop's fixed kernels (least-squares fit exact): Blur is ½ at the
/// centre and ⅛ on the four sides; Blur More is [1 2 1; 2 2 2; 1 2 1] / 14.
pub(crate) fn blur_kernel(src: &Image, out: Rect, ctx: &Ctx, more: bool) -> Vec<f32> {
    neighbourhood(src, out, ctx, |p, sides, ring, _| if more { (2.0 * p + sides + ring) / 14.0 } else { p / 2.0 + sides / 8.0 })
}

/// Sharpen Edges: Sharpen where the Sobel gradient is strong, faded in from 65 levels (none) to
/// 256 (all) (measured).
pub(crate) fn sharpen_edges(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    neighbourhood(src, out, ctx, |p, sides, _, grad| {
        let w = ((grad - 65.0) / 191.0).clamp(0.0, 1.0);
        p + w * (p - sides / 4.0)
    })
}

/// Smart Sharpen (not Legacy): fitted to Photoshop's output (`docs/filter-analysis.md`), it acts
/// like an unsharp mask of 0.9 × the amount over a blur of 0.9 × the radius; Lens Blur removes a
/// box of the radius, Motion Blur a streak twice the radius long at `angle`. Noise reduction keeps
/// small (noise-level) differences from being sharpened.
#[allow(clippy::too_many_arguments)]
pub(crate) fn smart(src: &Image, out: Rect, ctx: &Ctx, amount: f32, radius: f32, reduce_noise: f32, remove: crate::SharpenRemove, angle: f32) -> Vec<f32> {
    let k = 0.9 * amount / 100.0;
    let nr = (reduce_noise / 100.0).clamp(0.0, 1.0);
    let blurred = match remove {
        crate::SharpenRemove::GaussianBlur => {
            let kern = gaussian_kernel(0.9 * radius);
            conv_sep(src, out, &kern, &kern, ctx.alpha)
        }
        crate::SharpenRemove::LensBlur => crate::blur::boxed(src, out, ctx, radius.max(1.0).round()),
        crate::SharpenRemove::MotionBlur => crate::blur::motion(src, out, ctx, angle, (2.0 * radius).max(1.0)),
    };
    let mut res = src.crop(out);
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    for (px, bl) in res.chunks_exact_mut(n).zip(blurred.chunks_exact(n)) {
        if ctx.alpha && px[n - 1] <= 0.0 {
            continue;
        }
        for c in 0..cc {
            let d = px[c] - bl[c];
            let keep = 1.0 - nr * (-(d.abs() * 40.0)).exp();
            px[c] += k * d * keep;
        }
    }
    res
}

pub(crate) fn high_pass(src: &Image, out: Rect, ctx: &Ctx, radius: f32) -> Vec<f32> {
    with_blur(src, out, ctx, radius, |o, b| 0.5 + (o - b))
}

#[cfg(test)]
mod tests {
    use prism_color::ColorMode;

    use super::*;

    fn ctx(rect: Rect) -> Ctx {
        Ctx { bounds: rect, mode: ColorMode::Grayscale, alpha: false }
    }

    /// A 5×5 grey of 128 levels with one 200-level pixel in the middle.
    fn speck() -> (Image, Rect) {
        let rect = Rect::new(0, 0, 5, 5);
        let mut img = Image::new(rect, 1);
        img.data.fill(128.0 / 255.0);
        img.data[12] = 200.0 / 255.0;
        (img, rect)
    }

    fn levels(v: &[f32]) -> Vec<i32> {
        v.iter().map(|x| (x * 255.0).round() as i32).collect()
    }

    #[test]
    fn sharpen_kernels_are_photoshops() {
        let (img, rect) = speck();
        let centre = Rect::new(1, 1, 4, 4);
        // Sharpen: 2 × 200 − ¼ × 4 × 128 = 272 at the speck, 128 − 72/4 beside it, corners kept.
        assert_eq!(levels(&sharpen(&img, centre, &ctx(rect), false)), vec![128, 110, 128, 110, 272, 110, 128, 110, 128]);
        // Sharpen More: 3 × 200 − ¼ × 8 × 128 = 344, and every neighbour 110.
        assert_eq!(levels(&sharpen(&img, centre, &ctx(rect), true)), vec![110, 110, 110, 110, 344, 110, 110, 110, 110]);
    }

    #[test]
    fn blur_kernels_are_photoshops() {
        let (img, rect) = speck();
        let centre = Rect::new(1, 1, 4, 4);
        // Blur: ½ × 200 + ⅛ × 4 × 128 = 164 at the speck, 128 + 72/8 = 137 beside it.
        assert_eq!(levels(&blur_kernel(&img, centre, &ctx(rect), false)), vec![128, 137, 128, 137, 164, 137, 128, 137, 128]);
        // Blur More: (2 × 72) / 14 over 128 at the speck, 2 × 72 / 14 beside, 72 / 14 at corners.
        assert_eq!(levels(&blur_kernel(&img, centre, &ctx(rect), true)), vec![133, 138, 133, 138, 138, 138, 133, 138, 133]);
    }

    #[test]
    fn sharpen_edges_leaves_flat_and_symmetric_spots() {
        // The speck's centre has no gradient (symmetric), so it is kept; a side neighbour's gradient
        // is 144 levels, so it takes (144 − 65) / 191 of Sharpen's −18 (Photoshop: 120).
        let (img, rect) = speck();
        let out = levels(&sharpen_edges(&img, Rect::new(1, 1, 4, 4), &ctx(rect)));
        assert_eq!(out[4], 200);
        assert_eq!(out[1], 121);
        let flat = Image::new(rect, 1);
        assert!(sharpen_edges(&flat, rect, &ctx(rect)).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn unsharp_threshold_softens_instead_of_switching() {
        let (img, rect) = speck();
        let at = Rect::new(2, 2, 3, 3);
        let full = unsharp(&img, at, &ctx(rect), 100.0, 1.0, 0.0)[0] * 255.0 - 200.0;
        let less = unsharp(&img, at, &ctx(rect), 100.0, 1.0, 10.0)[0] * 255.0 - 200.0;
        assert!((full - less - 10.0).abs() < 1e-3, "{full} {less}");
        assert_eq!(unsharp(&img, at, &ctx(rect), 100.0, 1.0, 255.0)[0], 200.0 / 255.0);
    }
}
