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
    /// The input angle each particle adds to `angle` (`BrushRotationEffectorInSpray`).
    #[serde(skip_serializing_if = "CspParticleAngle::is_fixed")]
    pub angle_input: CspParticleAngle,
    /// Random turn per particle: `(r − 0.5) · angle_jitter · 360°` (flag 0x80 with
    /// `BrushRotationRandomInSpray`/100). 0 is off.
    #[serde(skip_serializing_if = "is_zero")]
    pub angle_jitter: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// Where a spray particle's input angle comes from. The flag values are CSP's
/// `BrushRotationEffectorInSpray` bits; evidence in docs/sut-analysis.md › Spray particle angle input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspParticleAngle {
    /// Only the base particle angle.
    #[default]
    Fixed,
    /// 0x10: the direction the pen leans (tilt azimuth).
    PenTilt,
    /// 0x20: the pen's barrel rotation.
    Rotation,
    /// 0x40: the direction of the line.
    Direction,
    /// 0x100: the angle of the whole spray (the brush area's own rotation).
    Spray,
    /// 0x200: from the spray centre towards the particle.
    TowardCenter,
}

impl CspParticleAngle {
    fn is_fixed(&self) -> bool {
        *self == Self::Fixed
    }

    /// The input selected by `BrushRotationEffectorInSpray` flags, in the order CSP's choice
    /// list offers them. Several input bits at once keep the first; `None` when none is set.
    pub fn from_flags(flags: u32) -> Option<Self> {
        [(0x10, Self::PenTilt), (0x20, Self::Rotation), (0x40, Self::Direction), (0x100, Self::Spray), (0x200, Self::TowardCenter)]
            .into_iter()
            .find(|(bit, _)| flags & bit != 0)
            .map(|(_, input)| input)
    }
}

/// Flag bits of `BrushRotationEffectorInSpray` Prism reads (enable, the stored-only 0x02, the
/// five inputs and random).
pub const PARTICLE_ANGLE_FLAGS: u32 = 0x01 | 0x02 | 0x10 | 0x20 | 0x40 | 0x80 | 0x100 | 0x200;

impl Default for CspSpray {
    fn default() -> Self {
        Self {
            size: 2.0,
            count: 8,
            bias: 0.0,
            angle: 0.0,
            size_dynamic: Dynamic::default(),
            count_dynamic: Dynamic::default(),
            size_ratio: None,
            angle_input: CspParticleAngle::Fixed,
            angle_jitter: 0.0,
        }
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
    fn particle_angle_flags_pick_the_first_input_in_choice_order() {
        assert_eq!(CspParticleAngle::from_flags(0x03), None);
        assert_eq!(CspParticleAngle::from_flags(0x11), Some(CspParticleAngle::PenTilt));
        assert_eq!(CspParticleAngle::from_flags(0x21), Some(CspParticleAngle::Rotation));
        assert_eq!(CspParticleAngle::from_flags(0x41), Some(CspParticleAngle::Direction));
        assert_eq!(CspParticleAngle::from_flags(0x101), Some(CspParticleAngle::Spray));
        assert_eq!(CspParticleAngle::from_flags(0x201), Some(CspParticleAngle::TowardCenter));
        assert_eq!(CspParticleAngle::from_flags(0x61), Some(CspParticleAngle::Rotation));
        // Older presets without the new fields still load, and the defaults are not written.
        let old: CspSpray = serde_json::from_str(r#"{"size":3.0}"#).unwrap();
        assert_eq!((old.angle_input, old.angle_jitter), (CspParticleAngle::Fixed, 0.0));
        let json = serde_json::to_string(&CspSpray::default()).unwrap();
        assert!(!json.contains("angleInput") && !json.contains("angleJitter"), "{json}");
    }

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
