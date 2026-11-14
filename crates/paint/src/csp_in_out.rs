//! Independently implemented CSP "Start and end" (입·출). Source evidence and limits are in
//! docs/sut-analysis.md.
//!
//! The source tapers the finished stroke path: points within the start length get the factor
//! `sin(90° · s / start)` (distance `s` from the first point) and points within the end length
//! `sin(90° · d / end)` (distance `d` to the last point). Each enabled target parameter is then
//! multiplied by `minimum + (1 − minimum) · factor`. Lengths are pixels or, in the percentage
//! mode, fractions of the path length; when start and end together exceed the path they are
//! scaled down to fit. The end needs the whole path, so it applies when the stroke is committed;
//! while drawing only a pixel-length start is known.
//!
//! "Fade" (CLIP STUDIO's third way to specify, `BrushInOutType` 2) is different: only the
//! Ending length is used, and it is measured from where the stroke starts. The targets fall from
//! their full value to the minimum over that many pixels and stay there for the rest of the
//! stroke, whatever its speed. Measured: 물번짐구름 브러쉬 (Brush density, ending 194) loses
//! density over about 200 px and then stays level; the quarter-sine shape is carried over from
//! the other modes, not fitted (docs/sut-analysis.md › Fade).
use serde::{Deserialize, Serialize};

use crate::StrokePoint;

/// Prism work limit for start/end lengths in pixels.
pub const MAX_LENGTH: f64 = 100_000.0;

/// How start and end lengths are specified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspInOutMode {
    /// Lengths in document pixels.
    #[default]
    Length,
    /// Fractions of the stroke's path length.
    Percent,
    /// The targets fade from full to the minimum over `end` pixels from the stroke's start and
    /// stay at the minimum; `start` and `by_speed` are not used.
    Fade,
}

/// Parameters a start/end taper can scale (the source's effector targets that Prism renders).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspInOutParam {
    /// Brush size (the spray area for spraying brushes).
    Size,
    Flow,
    TextureDensity,
    SpraySize,
    SprayDensity,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CspInOutTarget {
    pub param: CspInOutParam,
    /// Value at the very start/end, 0..1 of the parameter.
    pub minimum: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CspInOut {
    pub mode: CspInOutMode,
    /// Start length (pixels, or 0..1 of the path); `None` = no start taper.
    pub start: Option<f64>,
    pub end: Option<f64>,
    /// Limit each length to the path covered by the first/last `length · 0.04` input segments,
    /// so slow strokes taper over fewer pixels.
    pub by_speed: bool,
    pub targets: Vec<CspInOutTarget>,
}

impl CspInOut {
    pub fn bounded(&self) -> Self {
        let length = |v: Option<f64>| match self.mode {
            CspInOutMode::Length | CspInOutMode::Fade => v.filter(|v| v.is_finite() && *v > 0.0).map(|v| v.min(MAX_LENGTH)),
            CspInOutMode::Percent => v.filter(|v| v.is_finite() && *v > 0.0).map(|v| v.min(1.0)),
        };
        let mut targets: Vec<CspInOutTarget> = Vec::new();
        for t in &self.targets {
            if !targets.iter().any(|o| o.param == t.param) {
                targets.push(CspInOutTarget { param: t.param, minimum: if t.minimum.is_finite() { t.minimum.clamp(0.0, 1.0) } else { 0.0 } });
            }
        }
        let fade = self.mode == CspInOutMode::Fade;
        Self {
            mode: self.mode,
            start: length(self.start).filter(|_| !fade),
            end: length(self.end),
            by_speed: self.by_speed && !fade,
            targets,
        }
    }

    pub fn minimum(&self, param: CspInOutParam) -> Option<f32> {
        self.targets.iter().find(|t| t.param == param).map(|t| t.minimum)
    }
}

/// Start/end lengths resolved for one stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct InOutRuntime {
    cfg: CspInOut,
    start: f64,
    end: f64,
    /// Path length, when the whole stroke is known.
    total: Option<f64>,
}

impl InOutRuntime {
    /// While drawing: only a pixel-length start can be applied.
    pub fn live(cfg: &CspInOut) -> Option<Self> {
        let cfg = cfg.bounded();
        // A fade is known from the first point on.
        if cfg.mode == CspInOutMode::Fade {
            let end = cfg.end.filter(|_| !cfg.targets.is_empty())?;
            return Some(Self { cfg, start: 0.0, end, total: None });
        }
        if cfg.targets.is_empty() || cfg.mode != CspInOutMode::Length || cfg.by_speed {
            return None;
        }
        let start = cfg.start?;
        Some(Self { cfg, start, end: 0.0, total: None })
    }

    /// For a finished stroke whose walked path has these segment lengths.
    pub fn finished(cfg: &CspInOut, segments: &[f64]) -> Option<Self> {
        if cfg.mode == CspInOutMode::Fade {
            // The committed stroke fades exactly as the live one.
            return Self::live(cfg);
        }
        let cfg = cfg.bounded();
        if cfg.targets.is_empty() || (cfg.start.is_none() && cfg.end.is_none()) {
            return None;
        }
        let total: f64 = segments.iter().filter(|l| l.is_finite() && **l > 0.0).sum();
        let resolve = |v: Option<f64>| v.map_or(0.0, |v| if cfg.mode == CspInOutMode::Percent { v * total } else { v });
        let (mut start, mut end) = (resolve(cfg.start), resolve(cfg.end));
        if cfg.by_speed {
            let limit = |length: f64, segs: &mut dyn Iterator<Item = f64>| {
                let max = (length * 0.04) as usize;
                let mut covered = 0.0;
                for (n, seg) in segs.enumerate() {
                    covered += seg;
                    if length <= covered || max <= n + 1 {
                        return length.min(covered);
                    }
                }
                length.min(covered)
            };
            let positive = || segments.iter().copied().filter(|l| l.is_finite() && *l > 0.0);
            start = limit(start, &mut positive());
            end = limit(end, &mut positive().rev());
        }
        if start + end > total && start + end > 0.0 {
            let k = total / (start + end);
            start *= k;
            end *= k;
        }
        Some(Self { cfg, start, end, total: Some(total) })
    }

    /// Taper factor 0..1 at path distance `s`.
    pub fn factor(&self, s: f64) -> f32 {
        let ramp = |d: f64, len: f64| if len > 0.0 && d < len { (d.max(0.0) / len * std::f64::consts::FRAC_PI_2).sin() } else { 1.0 };
        if self.cfg.mode == CspInOutMode::Fade {
            // Full at the start, the minimum from `end` pixels on (a non-finite distance: faded).
            return if s.is_finite() { ramp(self.end - s, self.end) as f32 } else { 0.0 };
        }
        let mut f = ramp(s, self.start);
        if let Some(total) = self.total {
            f *= ramp(total - s, self.end);
        }
        f as f32
    }

    /// Multiplier for `param` at path distance `s` (1 when the parameter is not a target).
    pub fn multiplier(&self, param: CspInOutParam, s: f64) -> f32 {
        self.cfg.minimum(param).map_or(1.0, |m| m + (1.0 - m) * self.factor(s))
    }
}

/// Lengths of the consecutive segments of a walked polyline.
pub fn segment_lengths(points: &[StrokePoint]) -> Vec<f64> {
    points.windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).collect()
}

#[cfg(test)]
#[path = "csp_in_out_tests.rs"]
mod tests;
