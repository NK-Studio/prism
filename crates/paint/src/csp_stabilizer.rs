//! Independently implemented CSP Stabilization ("손떨림 보정") and Adjust by speed. Source evidence
//! and limits are recorded in docs/sut-analysis.md.
//!
//! Each raw input sample enters a history; the brush follows the average of the newest `W`
//! samples (position and pressure), where `W` is fractional: the next older sample contributes
//! with the remaining weight. Missing history, and samples older than one second, are replaced by
//! the oldest sample still used. `W` starts small at pen-down and grows towards the
//! stabilization level; with Adjust by speed it instead follows the pointer speed, shrinking for
//! fast movement and growing (by at most half a sample per input) for slow movement. The other
//! speed type (levels up to 30) instead aims the window at the recent path: slow, straight
//! movement widens it beyond the level, by a quarter sample per input.
use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::StrokePoint;

/// CSP's Stabilization slider maximum; also bounds the averaging work per input.
pub const MAX_LEVEL: u32 = 100;
/// Samples are retained at most this long before they stop counting (milliseconds).
const MAX_AGE_MS: f64 = 1000.0;
/// History cap of the source.
const MAX_HISTORY: usize = 1000;
/// Speed type 1: samples examined, the step length (screen pixels) at which the window stops
/// widening, the widening gain, its per-input step and the highest level it applies to.
const PATH_SAMPLES: usize = 30;
const PATH_SLOW_STEP: f64 = 2.5;
const PATH_GAIN: f64 = 20.0;
const PATH_WINDOW_STEP: f64 = 0.25;
const PATH_MAX_LEVEL: u32 = 30;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CspStabilizer {
    /// Stabilization level, 0..=100 (0 = off).
    pub level: u32,
    /// Adjust by speed: the averaging window follows the pointer speed.
    pub adjust_by_speed: bool,
    /// The source's speed-adjustment type: 0 follows the pointer speed, any other value the
    /// recent path's speed and straightness (levels up to 30).
    pub speed_type: u32,
    /// The source's `Stickness` value, which bounds the starting window.
    pub stickiness: u32,
}

impl CspStabilizer {
    pub fn bounded(&self) -> Self {
        Self { level: self.level.min(MAX_LEVEL), stickiness: self.stickiness.min(MAX_LEVEL), ..self.clone() }
    }

    /// Window at pen-down: half the smaller of level and stickiness, kept within 2..=15
    /// (levels up to 2 use the level itself).
    fn start_window(&self) -> f64 {
        let s = self.level;
        if s <= 2 { f64::from(s) } else { f64::from((s.min(self.stickiness) >> 1).clamp(2, 15)) }
    }
}

/// Stateful filter for one stroke.
#[derive(Clone, Debug)]
pub struct Stabilizer {
    cfg: CspStabilizer,
    /// Pointer distance per screen pixel.
    zoom: f64,
    history: VecDeque<StrokePoint>,
    window: f64,
    drawing: bool,
}

impl Stabilizer {
    pub fn new(cfg: &CspStabilizer, zoom: f32) -> Self {
        let cfg = cfg.bounded();
        let zoom = if zoom.is_finite() && zoom > 0.0 { f64::from(zoom) } else { 1.0 };
        Self { window: cfg.start_window(), cfg, zoom, history: VecDeque::new(), drawing: false }
    }

    pub fn active(&self) -> bool {
        self.cfg.level >= 1
    }

    /// Feed one raw input sample; returns the stabilized brush position.
    pub fn push(&mut self, p: StrokePoint) -> StrokePoint {
        if !self.active() {
            return p;
        }
        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(p);
        let level = f64::from(self.cfg.level);
        let mut averaged = true;
        let previous = self.history.len().checked_sub(2).and_then(|i| self.history.get(i)).copied();
        if self.cfg.adjust_by_speed && self.cfg.speed_type != 0 && self.cfg.level <= PATH_MAX_LEVEL {
            self.follow_path();
            self.drawing = true;
            return if self.window < 1.0 { p } else { self.average(p) };
        }
        if let (true, Some(prev)) = (self.cfg.adjust_by_speed && self.cfg.speed_type == 0 && self.cfg.level > 2, previous) {
            if p.time - prev.time > MAX_AGE_MS {
                averaged = false;
            } else {
                let k = level * 2.0 + 100.0;
                let d = (p.x - prev.x).hypot(p.y - prev.y) * self.zoom;
                let mut change = (k - d) / k;
                if change > 0.0 {
                    change *= 0.5;
                }
                self.window += change;
                if !self.window.is_finite() || self.window < 2.0 {
                    self.window = 2.0;
                } else {
                    let max = if self.drawing { level } else { self.cfg.start_window() };
                    self.window = self.window.min(max);
                }
            }
        } else if !self.drawing {
            self.window = self.cfg.start_window();
        } else if self.window < level {
            self.window = (self.window + ((level - self.window + 1.0) / level).min(0.5)).min(level);
        }
        self.drawing = true;
        if !averaged { p } else { self.average(p) }
    }

    /// Pen-up has two phases: initial repeats, then a bounded release-pressure drain until
    /// the filtered motion settles or pressure reaches zero. Omitting the second phase can
    /// leave a short flick halfway to the cursor when its window is smaller than stickiness.
    pub fn finish(&mut self, out: &mut Vec<StrokePoint>) {
        let Some(last) = self.history.back().copied().filter(|_| self.active() && self.drawing) else { return };
        let moved = self.history.iter().any(|p| p.x != last.x || p.y != last.y);
        let repeats = (self.window.floor() as i64 - i64::from(self.cfg.stickiness)).clamp(0, i64::from(MAX_LEVEL));
        for _ in 0..repeats {
            let p = self.push(last);
            out.push(p);
        }
        // A stationary click retains its contact pressure and dot size.
        if moved {
            let mut previous = self.average(last);
            let mut previous_delta: Option<(f64, f64)> = None;
            let threshold = if self.cfg.stickiness == 0 { 2.0 } else { (2.0 - 0.04 * (f64::from(self.cfg.stickiness) + self.window)).max(0.5) };
            for _ in 0..MAX_LEVEL.saturating_mul(2).saturating_add(2) {
                let mut pressure = 0.0;
                if let Some(recent) = self.history.back()
                    && let Some(older) = self.history.len().checked_sub(2).and_then(|i| self.history.get(i))
                    && recent.pressure + 0.02 < older.pressure
                {
                    pressure = (2.0 * recent.pressure - older.pressure).clamp(-0.2, 0.0);
                }
                let mut p = self.push(StrokePoint { pressure, ..last });
                let delta = ((p.x - previous.x) * self.zoom, (p.y - previous.y) * self.zoom);
                let step2 = delta.0 * delta.0 + delta.1 * delta.1;
                let settled =
                    previous_delta.is_some_and(|d| d.0 * delta.0 + d.1 * delta.1 >= 0.0 && step2 <= threshold * threshold && step2 < d.0 * d.0 + d.1 * d.1);
                let ended = p.pressure <= 0.0;
                if ended {
                    let span = previous.pressure - p.pressure;
                    if previous.pressure > 0.0 && span.is_finite() && span > 0.0 {
                        let t = f64::from((previous.pressure / span).clamp(0.0, 1.0));
                        p.x = previous.x + (p.x - previous.x) * t;
                        p.y = previous.y + (p.y - previous.y) * t;
                    }
                    p.pressure = 0.0;
                }
                out.push(p);
                if ended || settled {
                    break;
                }
                previous_delta = Some(delta);
                previous = p;
            }
        }
        self.drawing = false;
    }

    /// Speed type 1: move the window towards a target from the newest samples' mean step length
    /// and straightness (bounding-box diagonal over path length): slow, straight input widens it.
    fn follow_path(&mut self) {
        let n = self.history.len();
        let span = n.min(PATH_SAMPLES);
        let (mut path, mut steps) = (0.0f64, 0u32);
        let newest = self.history.back().copied();
        let (mut lo, mut hi) = newest.map_or(((0.0, 0.0), (0.0, 0.0)), |p| ((p.x, p.y), (p.x, p.y)));
        let mut last = newest;
        for i in 1..span {
            let Some(s) = n.checked_sub(1 + i).and_then(|at| self.history.get(at)).copied() else { break };
            if let Some(prev) = last {
                let d = (s.x - prev.x).hypot(s.y - prev.y) * self.zoom;
                if d > 0.0 {
                    path += d;
                    steps += 1;
                    lo = (lo.0.min(s.x), lo.1.min(s.y));
                    hi = (hi.0.max(s.x), hi.1.max(s.y));
                    last = Some(s);
                }
            }
        }
        if steps == 0 || path <= 0.0 {
            return;
        }
        let level = self.cfg.level;
        let slack = PATH_SLOW_STEP - path / f64::from(steps);
        let mut target = if slack > 0.0 {
            let diagonal = (hi.0 - lo.0).hypot(hi.1 - lo.1) * self.zoom;
            ((diagonal / path) * slack * PATH_GAIN) as i64
        } else {
            i64::from(level)
        };
        target = if self.drawing { target.max(i64::from(level)) } else { target.max(self.cfg.start_window() as i64) };
        let target = target as f64;
        if (self.window - target).abs() < PATH_WINDOW_STEP {
            self.window = target;
        } else if self.window < target {
            self.window += PATH_WINDOW_STEP;
        } else {
            self.window -= PATH_WINDOW_STEP;
        }
        self.window = self.window.min(span as f64).min(n.saturating_sub(1) as f64).min(f64::from(MAX_LEVEL));
        if !self.window.is_finite() || self.window < 0.0 {
            self.window = 0.0;
        }
    }

    /// Fractional moving average over the newest `window` samples.
    fn average(&self, p: StrokePoint) -> StrokePoint {
        let w = self.window.max(1.0);
        let whole = w.floor() as usize;
        let n = self.history.len();
        let (mut x, mut y, mut pressure) = (0.0f64, 0.0f64, 0.0f64);
        let mut used = p;
        let mut exhausted = false;
        // The i-th newest sample, if it exists and is recent enough.
        let recent = |i: usize| n.checked_sub(1 + i).and_then(|at| self.history.get(at)).filter(|s| p.time - s.time <= MAX_AGE_MS).copied();
        for i in 0..whole {
            if !exhausted {
                match recent(i) {
                    Some(s) => used = s,
                    None => exhausted = true,
                }
            }
            x += used.x;
            y += used.y;
            pressure += f64::from(used.pressure);
        }
        let frac = w - whole as f64;
        if frac > 0.0 {
            let older = if exhausted { used } else { recent(whole).unwrap_or(used) };
            x += older.x * frac;
            y += older.y * frac;
            pressure += f64::from(older.pressure) * frac;
        }
        StrokePoint { x: x / w, y: y / w, pressure: (pressure / w) as f32, ..p }
    }
}

#[cfg(test)]
#[path = "csp_stabilizer_tests.rs"]
mod tests;
