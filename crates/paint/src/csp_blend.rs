//! CLIP STUDIO brush blending modes (`CompositeMode`, property 30) and dual brush modes
//! (`DualBrushCompositeMode`) and how Prism runs them. Code evidence: docs/sut-analysis.md ›
//! Blending modes and dual brush.
use crate::{CspDualAdd, CspStrokeMode, MaskMode};
use prism_color::{BlendMode, PixelFormat};

/// How Prism runs one CLIP STUDIO brush blending mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CspBlend {
    /// A Prism blend mode; `exact` is false where CLIP STUDIO's formula is known to differ.
    Mode { mode: BlendMode, exact: bool },
    /// The brush erases (Erase).
    Erase { exact: bool },
    /// A CLIP STUDIO-only stroke mode Prism runs itself ([`stroke_native`]).
    Stroke(CspStrokeMode),
    /// No Prism equivalent: the brush paints in Normal.
    Unsupported,
}

use CspBlend::{Erase, Mode, Stroke, Unsupported};
const fn exact(mode: BlendMode) -> CspBlend {
    Mode { mode, exact: true }
}
const fn approx(mode: BlendMode) -> CspBlend {
    Mode { mode, exact: false }
}

/// Every brush blending mode code with CLIP STUDIO's English label. Codes 0–8, 10, 12–27, 29 and
/// 36 are linked to Photoshop's mode keys by CLIP STUDIO's ABR import, 32 and 33 by its
/// dodge/burn tool import, 27 and 35 are the erasing modes of its brush builder; the rest
/// follow the label order those anchors establish.
pub const CSP_BLEND_MODES: &[(i64, &str, CspBlend)] = &[
    (0, "Normal", exact(BlendMode::Normal)),
    (1, "Darken", exact(BlendMode::Darken)),
    (2, "Multiply", exact(BlendMode::Multiply)),
    (3, "Color burn", exact(BlendMode::ColorBurn)),
    (4, "Linear burn", exact(BlendMode::LinearBurn)),
    (5, "Subtract", exact(BlendMode::Subtract)),
    (6, "Darker color", exact(BlendMode::DarkerColor)),
    (7, "Lighten", exact(BlendMode::Lighten)),
    (8, "Screen", exact(BlendMode::Screen)),
    // CLIP STUDIO imports Photoshop's Color Dodge as Glow dodge and Linear Dodge as Add (Glow);
    // its own Color dodge and Add differ from them.
    (9, "Color dodge", approx(BlendMode::ColorDodge)),
    (10, "Glow dodge", exact(BlendMode::ColorDodge)),
    (11, "Add", approx(BlendMode::LinearDodge)),
    (12, "Add (Glow)", exact(BlendMode::LinearDodge)),
    (13, "Lighter color", exact(BlendMode::LighterColor)),
    (14, "Overlay", exact(BlendMode::Overlay)),
    (15, "Soft light", exact(BlendMode::SoftLight)),
    (16, "Hard light", exact(BlendMode::HardLight)),
    (17, "Vivid light", exact(BlendMode::VividLight)),
    (18, "Linear light", exact(BlendMode::LinearLight)),
    (19, "Pin light", exact(BlendMode::PinLight)),
    (20, "Hard mix", exact(BlendMode::HardMix)),
    (21, "Difference", exact(BlendMode::Difference)),
    (22, "Exclusion", exact(BlendMode::Exclusion)),
    (23, "Hue", exact(BlendMode::Hue)),
    (24, "Saturation", exact(BlendMode::Saturation)),
    (25, "Color", exact(BlendMode::Color)),
    (26, "Brightness", exact(BlendMode::Luminosity)),
    (27, "Erase", Erase { exact: true }),
    (28, "Dissolve", exact(BlendMode::Dissolve)),
    (29, "Behind", Stroke(CspStrokeMode::Behind)),
    (30, "Through", Unsupported),
    (31, "Replace alpha", Stroke(CspStrokeMode::ReplaceAlpha)),
    (32, "Black burn", Unsupported),
    (33, "White burn", Unsupported),
    (34, "Compare density", Stroke(CspStrokeMode::CompareDensity)),
    (35, "Erase (compare)", Stroke(CspStrokeMode::EraseCompare)),
    (36, "Divide", exact(BlendMode::Divide)),
];

/// The label and Prism behaviour of brush blending mode `code` (`None`: not a CLIP STUDIO code).
pub fn csp_blend(code: f64) -> Option<(&'static str, CspBlend)> {
    CSP_BLEND_MODES.iter().find(|(c, _, _)| *c as f64 == code).map(|(_, label, run)| (*label, *run))
}

/// Composite one pixel of a stroke in a CLIP STUDIO-only mode, in native straight channels
/// (colour channels then alpha, any colour model and depth). `k` is the stroke's alpha there
/// (coverage × opacity × selection), `opacity` the stroke opacity; the source alpha scales the
/// paint like [`crate::retouch::over_native`]. The formulas are inferred from CLIP STUDIO's labels
/// (docs/sut-analysis.md › Blending modes, colour jitter and dual brush):
///
/// - Behind: the existing pixel over the stroke (`a = d + s·(1 − d)`).
/// - Replace alpha: the brush colour at the stroke opacity replaces the pixel, weighted by the
///   coverage `k / opacity` (premultiplied interpolation).
/// - Compare density: alpha `max(d, s)`; the colour of the denser one.
/// - Erase (compare): alpha `min(d, 1 − k)`.
///
/// Locked transparency keeps the alpha (Behind and Erase (compare) then change nothing). A format
/// without alpha is opaque: only Replace alpha changes it (by `k`, like Normal).
pub fn stroke_native(mode: CspStrokeMode, fmt: &PixelFormat, dst: &mut [f32], src: &[f32], k: f32, opacity: f32, lock_transparency: bool) {
    let k = if k.is_finite() { k.clamp(0.0, 1.0) } else { 0.0 };
    let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 0.0 };
    let n = dst.len().min(src.len());
    let Some(a) = crate::retouch::alpha_index(fmt).filter(|a| *a < n) else {
        if mode == CspStrokeMode::ReplaceAlpha {
            for (d, s) in dst.iter_mut().zip(src).take(n) {
                *d += (*s - *d) * k;
            }
        }
        return;
    };
    let (da, src_a) = (dst[a].clamp(0.0, 1.0), src[a].clamp(0.0, 1.0));
    // Colour channels and alpha from premultiplied weights of the pixel and the paint.
    let mix = |dst: &mut [f32], keep: f32, paint: f32| {
        let oa = keep + paint;
        if oa > 1e-7 {
            for (d, s) in dst.iter_mut().zip(src).take(a) {
                *d = (*d * keep + *s * paint) / oa;
            }
        }
        if !lock_transparency {
            dst[a] = oa.clamp(0.0, 1.0);
        }
    };
    match mode {
        CspStrokeMode::Behind => {
            if !lock_transparency {
                mix(dst, da, k * src_a * (1.0 - da));
            }
        }
        CspStrokeMode::ReplaceAlpha => {
            let w = if opacity > 0.0 { (k / opacity).min(1.0) } else { 0.0 };
            let target = opacity * src_a;
            if lock_transparency {
                // The alpha stays: only the colour moves towards the brush by the coverage.
                let w = if target > 0.0 { w } else { 0.0 };
                for (d, s) in dst.iter_mut().zip(src).take(a) {
                    *d += (*s - *d) * w;
                }
            } else {
                mix(dst, da * (1.0 - w), target * w);
            }
        }
        CspStrokeMode::CompareDensity => {
            let sa = k * src_a;
            if sa > da {
                dst[..a].copy_from_slice(&src[..a]);
                if !lock_transparency {
                    dst[a] = sa;
                }
            }
        }
        CspStrokeMode::EraseCompare => {
            if !lock_transparency {
                dst[a] = da.min(1.0 - k);
            }
        }
    }
}

/// How Prism's dual brush runs one CLIP STUDIO dual brush mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CspDualMode {
    /// The second brush masks the first with this mode.
    Mask(MaskMode),
    /// The second brush adds its opacity outside the first (Normal, Add (Glow), Lighten,
    /// Screen): [`crate::DualBrush::csp_add`].
    Adds(CspDualAdd),
}

/// Dual brush modes with CLIP STUDIO's labels. Codes 1, 4 and 7–12 are linked to Photoshop's
/// dual brush mode keys by CLIP STUDIO's ABR import; 0, 2, 3, 5 and 6 follow the label order.
pub const CSP_DUAL_MODES: &[(i64, &str, CspDualMode)] = &[
    (0, "Normal", CspDualMode::Adds(CspDualAdd::Normal)),
    (1, "Multiply", CspDualMode::Mask(MaskMode::Multiply)),
    (2, "Add (Glow)", CspDualMode::Adds(CspDualAdd::Add)),
    (3, "Subtract", CspDualMode::Mask(MaskMode::Subtract)),
    (4, "Darken", CspDualMode::Mask(MaskMode::Darken)),
    (5, "Lighten", CspDualMode::Adds(CspDualAdd::Lighten)),
    (6, "Screen", CspDualMode::Adds(CspDualAdd::Screen)),
    (7, "Overlay", CspDualMode::Mask(MaskMode::Overlay)),
    (8, "Color dodge", CspDualMode::Mask(MaskMode::ColorDodge)),
    (9, "Color burn", CspDualMode::Mask(MaskMode::ColorBurn)),
    (10, "Linear burn", CspDualMode::Mask(MaskMode::LinearBurn)),
    (11, "Hard mix", CspDualMode::Mask(MaskMode::HardMix)),
    (12, "Height (Linear)", CspDualMode::Mask(MaskMode::LinearHeight)),
];

/// The label and Prism behaviour of dual brush mode `code` (`None`: not a CLIP STUDIO code).
pub fn csp_dual_mode(code: f64) -> Option<(&'static str, CspDualMode)> {
    CSP_DUAL_MODES.iter().find(|(c, _, _)| *c as f64 == code).map(|(_, label, run)| (*label, *run))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_dense() {
        for (i, (code, _, _)) in CSP_BLEND_MODES.iter().enumerate() {
            assert_eq!(*code, i as i64);
        }
        for (i, (code, _, _)) in CSP_DUAL_MODES.iter().enumerate() {
            assert_eq!(*code, i as i64);
        }
    }

    #[test]
    fn hostile_codes_are_not_modes() {
        for code in [-1.0, 0.5, 37.0, 1e300, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(csp_blend(code), None, "{code}");
            assert_eq!(csp_dual_mode(code), None, "{code}");
        }
        assert_eq!(csp_dual_mode(13.0), None);
    }

    #[test]
    fn photoshop_linked_codes() {
        assert_eq!(csp_blend(2.0), Some(("Multiply", exact(BlendMode::Multiply))));
        assert_eq!(csp_blend(10.0).map(|m| m.1), Some(exact(BlendMode::ColorDodge)));
        assert_eq!(csp_blend(26.0).map(|m| m.1), Some(exact(BlendMode::Luminosity)));
        assert_eq!(csp_blend(27.0).map(|m| m.1), Some(Erase { exact: true }));
        assert_eq!(csp_blend(36.0).map(|m| m.1), Some(exact(BlendMode::Divide)));
        assert_eq!(csp_dual_mode(12.0).map(|m| m.1), Some(CspDualMode::Mask(MaskMode::LinearHeight)));
        assert_eq!(csp_dual_mode(0.0).map(|m| m.1), Some(CspDualMode::Adds(CspDualAdd::Normal)));
        assert_eq!(csp_dual_mode(6.0).map(|m| m.1), Some(CspDualMode::Adds(CspDualAdd::Screen)));
        assert_eq!(csp_blend(34.0), Some(("Compare density", Stroke(CspStrokeMode::CompareDensity))));
        assert_eq!(csp_blend(35.0).map(|m| m.1), Some(Stroke(CspStrokeMode::EraseCompare)));
    }
}
