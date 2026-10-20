//! Independent normalized texture equations for Clip Studio brush imports.
//! Format/dispatch evidence and the limits of this float model are in docs/sut-analysis.md.
//! These equations deliberately do not change the Photoshop/dual-tip mask modes.

use crate::CspTextureMode;

/// Adjust a sampled native black/alpha texel, returning luminance (white = 1).
/// Sampling happens first: applying a clipped contrast curve before interpolation differs.
pub fn luminance(gray: f32, invert: bool, brightness: f32, contrast: f32) -> f32 {
    let q = if invert { gray } else { 1.0 - gray };
    let b = brightness.clamp(-1.0, 1.0) * if invert { 1.0 } else { -1.0 };
    let c = contrast.clamp(-1.0, 1.0);
    let threshold = (b + c) * 0.5;
    let darkness = if c < 0.0 {
        q * (1.0 + c) - threshold
    } else if c < 1.0 {
        (q - threshold) / (1.0 - c)
    } else if q > threshold {
        1.0
    } else {
        0.0
    };
    1.0 - darkness.clamp(0.0, 1.0)
}

/// Combine normalized brush coverage with adjusted luminance at the requested density.
/// Coverage is normalized to a ceiling of one.
pub fn combine(mode: CspTextureMode, coverage: f32, luminance: f32, density: f32) -> f32 {
    combine_with_ceiling(mode, coverage, luminance, density, 1.0)
}

/// Apply texture to already scaled coverage. The ceiling shares that coverage scale;
/// Height uses it explicitly. Scaling coverage after a nonlinear mode would change its result.
pub fn combine_with_ceiling(mode: CspTextureMode, coverage: f32, luminance: f32, density: f32, ceiling: f32) -> f32 {
    let v = coverage.clamp(0.0, 1.0);
    if v <= 0.0 {
        return 0.0;
    }
    let t = luminance.clamp(0.0, 1.0);
    let d = density.clamp(0.0, 1.0);
    let x = (1.0 - t) * d;
    let result = match mode {
        CspTextureMode::Normal => v * (1.5 - x),
        CspTextureMode::Multiply => v * (1.0 - x),
        CspTextureMode::Subtract => v - x,
        CspTextureMode::Compare => v.min(1.0 - x),
        CspTextureMode::Outline => v * (1.0 + d) - x,
        CspTextureMode::Overlay => {
            if v < 0.5 {
                2.0 * v * (1.0 - x)
            } else {
                1.0 - 2.0 * (1.0 - v) * x
            }
        }
        CspTextureMode::ColorDodge => {
            if v > x {
                1.0
            } else if x > 0.0 {
                v / x
            } else {
                0.0
            }
        }
        CspTextureMode::ColorBurn => {
            if v < x {
                0.0
            } else if x >= 1.0 || v >= 1.0 {
                1.0
            } else {
                1.0 - (1.0 - v) / (1.0 - x)
            }
        }
        CspTextureMode::HardMix => 2.0 * v - x,
        CspTextureMode::Height => (10.0 * v * d - t * ceiling.clamp(0.0, 1.0)).min(ceiling.clamp(0.0, 1.0)),
    };
    result.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [CspTextureMode; 10] = [
        CspTextureMode::Normal,
        CspTextureMode::Multiply,
        CspTextureMode::Subtract,
        CspTextureMode::Compare,
        CspTextureMode::Outline,
        CspTextureMode::Overlay,
        CspTextureMode::ColorDodge,
        CspTextureMode::ColorBurn,
        CspTextureMode::HardMix,
        CspTextureMode::Height,
    ];

    #[test]
    fn partial_density_analytic_vectors_and_singular_endpoints() {
        // v=1/4, t=1/2, density=1/2 => darkness*density=1/4.
        let expected = [0.3125, 0.1875, 0.0, 0.25, 0.125, 0.375, 1.0, 0.0, 0.25, 0.75];
        for (mode, expected) in MODES.into_iter().zip(expected) {
            assert!((combine(mode, 0.25, 0.5, 0.5) - expected).abs() < 1e-6, "{mode:?}");
            for v in [0.0, 0.1, 0.75, 1.0] {
                for t in [0.0, 0.3, 1.0] {
                    for d in [0.0, 0.4, 1.0] {
                        let result = combine(mode, v, t, d);
                        assert!(result.is_finite() && (0.0..=1.0).contains(&result));
                        if v == 0.0 {
                            assert_eq!(result, 0.0);
                        }
                    }
                }
            }
        }
        assert_eq!(combine(CspTextureMode::ColorBurn, 1.0, 0.0, 1.0), 1.0);
        assert_eq!(combine(CspTextureMode::ColorDodge, 0.5, 1.0, 1.0), 1.0);
        // CSP's Hard mix is continuous, unlike the binary Photoshop mask.
        assert_eq!(combine(CspTextureMode::HardMix, 0.4, 0.25, 0.5), 0.425);
        assert_eq!(combine(CspTextureMode::Height, 0.25, 0.5, 0.0), 0.0);
    }

    #[test]
    fn brightness_contrast_and_reverse_follow_darkness_coordinates() {
        assert_eq!(luminance(0.25, false, 0.0, 0.0), 0.25);
        assert_eq!(luminance(0.25, true, 0.0, 0.0), 0.75);
        assert_eq!(luminance(0.25, false, 0.5, 0.0), 0.0);
        assert_eq!(luminance(0.25, true, 0.5, 0.0), 1.0);
        assert_eq!(luminance(0.25, false, 0.0, -0.5), 0.375);
        assert_eq!(luminance(0.25, false, 0.0, 0.5), 0.0);
        assert_eq!(luminance(0.5, false, 0.0, 1.0), 1.0);
        assert_eq!(luminance(0.25, false, 0.0, 1.0), 0.0);
    }
}
