//! Independently implemented CSP particle spraying. Source evidence and limitations
//! are recorded in docs/sut-analysis.md; randomness uses Prism's replayable streams.
use crate::Dynamic;
use serde::{Deserialize, Serialize};

/// Bounded per-step work, independent of malformed preset values.
pub const MAX_PARTICLES: u32 = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CspSpray {
    /// Particle diameter in document pixels, independent of the brush-area diameter.
    pub size: f32,
    pub count: u32,
    /// -1 places centres on the outer circumference; 0 fills the disk uniformly.
    pub bias: f32,
    /// Particle orientation, independent of the brush-area rotation, in CCW degrees.
    pub angle: f32,
    pub size_dynamic: Dynamic,
    pub count_dynamic: Dynamic,
    /// Particle size synchronized with the brush size: this fraction of it, replacing `size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_ratio: Option<f32>,
}

impl Default for CspSpray {
    fn default() -> Self {
        Self { size: 2.0, count: 8, bias: 0.0, angle: 0.0, size_dynamic: Dynamic::default(), count_dynamic: Dynamic::default(), size_ratio: None }
    }
}

/// Normalized radial distance. Negative bias moves an area-uniform disk towards
/// its circumference. Positive bias uses the source's asymmetric two-sample warp;
/// at +1 it becomes `min(u, 1-v)`. This is not a Gaussian scatter distribution.
pub(crate) fn radial(bias: f32, u: f32, v: f32) -> f32 {
    let b = bias.clamp(-1.0, 1.0);
    if b <= 0.0 {
        return (1.0 + b) * u.max(v) - b;
    }
    let c = 1.0 - b;
    if u <= c {
        if v >= c * u { c * v } else { u }
    } else if v >= (1.0 - u) / b {
        1.0 - b * v
    } else {
        u
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bias_endpoints_and_positive_warp() {
        for (u, v) in [(0.25, 0.15), (0.25, 0.05), (0.75, 0.75), (0.75, 0.25)] {
            assert_eq!(radial(-1.0, u, v), 1.0);
            assert_eq!(radial(0.0, u, v), u.max(v));
            assert_eq!(radial(1.0, u, v), u.min(1.0 - v));
        }
        for (u, v, r) in [(0.25, 0.15, 0.075), (0.25, 0.05, 0.25), (0.75, 0.75, 0.625), (0.75, 0.25, 0.75)] {
            assert!((radial(0.5, u, v) - r).abs() < 1e-6);
        }
    }
}
