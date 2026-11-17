//! From input points to dabs: smoothing, brush pose, path walking (spacing, airbrush build-up) and
//! the per-dab dynamics (shape, scattering, transfer, colour, texture depth).
//!
//! Everything is incremental (feed points in any chunking, get the same dabs) and deterministic:
//! jitter comes from [`crate::rng`] keyed by the stroke seed and the step/dab index.

use prism_geom::Point;
use std::sync::Arc;

use crate::brush::{BrushSettings, Control, Dynamic, Smoothing};
use crate::rng::{rand_signed, rand01, stream};
use crate::{Dab, StrokePoint};

/// Maximum pulled-string length at 100 % smoothing, in screen pixels.
pub const MAX_STRING_PX: f64 = 100.0;

/// With Spacing unchecked, one dab per this many milliseconds of stroke time, so faster strokes
/// space their dabs further apart (Photoshop: "the speed of the cursor determines the spacing").
pub const SPEED_SPACING_MS: f64 = 8.0;

/// Densest speed spacing, in pixels between dabs (a slow, long stroke never floods the buffer).
const MIN_SPEED_STEP: f64 = 0.5;

/// Smallest distance between dabs along the path, in pixels, whatever the Spacing percentage:
/// Photoshop lays the dabs of a 1–100 px tip no closer than about 1 px (measured: a 20 px tip at
/// 1–5 % spacing paints the same density as at 1 px; `cargo xtask brush-compare`, `spc-*` cases).
/// Denser dabs would build flow and the anti-aliased edge up further than Photoshop does, which
/// made thin lines too dark.
pub const MIN_DAB_STEP: f64 = 1.0;
/// Maximum airbrush catch-up dabs for one input segment.
const MAX_AIRBRUSH_DABS_PER_SEGMENT: usize = 512;

/// Photoshop's resting pen: one dab every this many milliseconds while the pen rests on the
/// canvas, with the airbrush off or on and at any flow, plus one on the first resting sample of a
/// stroke that has not moved yet; lifting adds nothing (measured with the synthetic pen,
/// docs/pen-input-validation.md › Photoshop pen rules). A pressure change at rest lays nothing
/// by itself.
pub const PS_REST_INTERVAL_MS: f64 = 100.0;

/// The most a pen sample may move, in screen pixels, and still count as resting: Photoshop rests
/// a pen creeping 1 px or less per sample and not one moving 1.5 px or more.
pub const PS_REST_MAX_STEP_PX: f64 = 1.25;

#[inline]
fn lerp_pt(a: &StrokePoint, b: &StrokePoint, f: f64) -> StrokePoint {
    let ff = f as f32;
    StrokePoint {
        x: a.x + (b.x - a.x) * f,
        y: a.y + (b.y - a.y) * f,
        pressure: a.pressure + (b.pressure - a.pressure) * ff,
        tilt_x: a.tilt_x + (b.tilt_x - a.tilt_x) * ff,
        tilt_y: a.tilt_y + (b.tilt_y - a.tilt_y) * ff,
        rotation: a.rotation + (b.rotation - a.rotation) * ff,
        wheel: a.wheel + (b.wheel - a.wheel) * ff,
        time: a.time + (b.time - a.time) * f,
    }
}

/// A stationary stylus can still change the brush footprint or transfer. Timestamps alone
/// must not add ink unless the brush explicitly enables time-based build-up.
fn stylus_changed(a: &StrokePoint, b: &StrokePoint) -> bool {
    a.pressure != b.pressure || a.tilt_x != b.tilt_x || a.tilt_y != b.tilt_y || a.rotation != b.rotation || a.wheel != b.wheel
}

// ---------------------------------------------------------------------------
// Smoothing
// ---------------------------------------------------------------------------

/// Incremental stroke smoothing: exponential (with optional time-based catch-up) or pulled string.
#[derive(Clone, Debug)]
pub struct Smoother {
    cfg: Smoothing,
    zoom: f64,
    pos: Option<StrokePoint>,
    last_input: Option<StrokePoint>,
}

impl Smoother {
    /// `zoom` is the view zoom (only used with `adjust_for_zoom`).
    pub fn new(cfg: &Smoothing, zoom: f32) -> Self {
        Self { cfg: cfg.clone(), zoom: f64::from(zoom.max(0.01)), pos: None, last_input: None }
    }

    fn active(&self) -> bool {
        self.cfg.amount > 0.0
    }

    /// Feed one input point; returns the brush positions to paint through.
    pub fn push(&mut self, p: StrokePoint, out: &mut Vec<StrokePoint>) {
        let prev_input = self.last_input.replace(p);
        if !self.active() {
            out.push(p);
            return;
        }
        let Some(pos) = self.pos else {
            self.pos = Some(p);
            out.push(p);
            return;
        };
        let amount = f64::from(self.cfg.amount.clamp(0.0, 1.0));
        if self.cfg.pulled_string {
            let mut len = amount * MAX_STRING_PX;
            if self.cfg.adjust_for_zoom {
                len /= self.zoom;
            }
            let (dx, dy) = (p.x - pos.x, p.y - pos.y);
            let d = (dx * dx + dy * dy).sqrt();
            if d > len {
                let k = (d - len) / d;
                let np = StrokePoint { time: p.time, ..lerp_pt(&pos, &p, k) };
                self.pos = Some(np);
                out.push(np);
            } else if prev_input.is_some_and(|q| q.x == p.x && q.y == p.y) && (stylus_changed(&pos, &p) || p.time > pos.time) {
                // A pen held in place can change pressure/pose or run the airbrush clock.
                // Movement inside the string must stay suppressed: applying every moving
                // sample's pressure at the frozen brush anchor would inflate the stroke head.
                let np = StrokePoint { x: pos.x, y: pos.y, ..p };
                self.pos = Some(np);
                out.push(np);
            }
            return;
        }
        if p.x == pos.x && p.y == pos.y {
            // No positional lag to compensate: a pen pressed in place must resize immediately.
            self.pos = Some(p);
            out.push(p);
            return;
        }
        // Exponential: each iteration moves `a` of the way towards the pointer.
        let a = 1.0 - amount.min(0.95);
        let iters = match prev_input {
            Some(q) if self.cfg.catch_up && p.time > q.time => ((p.time - q.time) / 16.0).round().clamp(1.0, 64.0) as usize,
            _ => 1,
        };
        let mut cur = pos;
        for _ in 0..iters {
            // Keep the sensor values at the same position along the smoothed segment. Copying
            // the latest pressure onto a lagging position makes a releasing pen abruptly thin.
            cur = StrokePoint { time: p.time, ..lerp_pt(&cur, &p, a) };
            out.push(cur);
        }
        self.pos = Some(cur);
    }

    /// End of stroke: with Catch-Up On Stroke End the brush finishes at the last pointer position.
    pub fn finish(&mut self, out: &mut Vec<StrokePoint>) {
        if !self.active() || !self.cfg.catch_up_on_end {
            return;
        }
        if let (Some(pos), Some(last)) = (self.pos, self.last_input)
            && (pos.x != last.x || pos.y != last.y || stylus_changed(&pos, &last))
        {
            self.pos = Some(last);
            out.push(last);
        }
    }
}

/// Smooth a whole point list with the brush's smoothing settings.
pub fn smooth_points(cfg: &Smoothing, zoom: f32, pts: &[StrokePoint]) -> Vec<StrokePoint> {
    let mut s = Smoother::new(cfg, zoom);
    let mut out = Vec::with_capacity(pts.len());
    for p in pts {
        s.push(*p, &mut out);
    }
    s.finish(&mut out);
    out
}

// ---------------------------------------------------------------------------
// Path walking
// ---------------------------------------------------------------------------

/// One spacing (or airbrush time) step along the path.
#[derive(Clone, Copy, Debug)]
pub struct StepInput {
    pub point: StrokePoint,
    /// Visual (counter-clockwise, y-up) direction of travel in radians.
    pub direction: f32,
    pub initial_direction: f32,
    /// Sequential step number (0 = first).
    pub step: u64,
    /// Path distance from the stroke's first point to this step (CSP start and end).
    pub distance: f64,
    /// On-screen speed of the segment this step lies on, in screen pixels per millisecond (0 at
    /// rest or without timestamps; CSP's velocity input).
    pub speed: f32,
}

/// The rest CLIP STUDIO counts before a stroke's first segment for its velocity input (ms): a
/// fast first segment of 16 ms reads two thirds of its speed, one of 8 ms half (measured,
/// docs/sut-analysis.md › Effector velocity input › Stroke start).
pub const CSP_REST_MS: f64 = 8.0;

/// Walks a polyline and emits steps, each as far from the last as the step's own callback says (the
/// spacing of the dab it laid), plus airbrush steps in time.
#[derive(Clone, Debug)]
pub struct PathWalker {
    last: Option<StrokePoint>,
    /// Path distance from `last` to the next step.
    next_at: f64,
    /// First point, emitted lazily once the initial direction is known.
    pending_first: Option<StrokePoint>,
    initial_dir: Option<f32>,
    dir: f32,
    step: u64,
    /// Airbrush interval in ms (None = off).
    interval: Option<f64>,
    time_acc: f64,
    /// Speed spacing (Spacing unchecked): one step per this many ms of stroke time.
    speed: Option<f64>,
    speed_acc: f64,
    /// Path length up to `last`.
    travelled: f64,
    /// A pressure or pose change without movement lays a step in place (off while a CSP
    /// stabilizer drains at pen-up: those samples are not input).
    in_place: bool,
    /// The first step waits for the path to move (a CSP tip turned by the line's direction):
    /// a stroke that never moves lays nothing.
    wait_for_direction: bool,
    /// Screen pixels per document pixel, for the on-screen speed.
    zoom: f64,
    /// The previous segment's on-screen length (screen pixels) and duration (ms); before the
    /// first segment, a rest of [`CSP_REST_MS`].
    seg_prev: (f64, f64),
    /// CSP's velocity input at the last input point: the speed over the two segments before it.
    speed_at_last: f32,
    /// The velocity input at the current segment's two ends; steps between them interpolate.
    seg_from: f32,
    seg_to: f32,
    /// Photoshop's resting pen ([`PS_REST_INTERVAL_MS`]): the longest step, in document pixels,
    /// that still counts as resting (None = off, the Clip Studio rules).
    rest_step: Option<f64>,
    /// Time spent resting since the last rest dab (ms).
    rest_acc: f64,
    /// The pen has moved (a step longer than `rest_step`) since it touched down.
    moved: bool,
    /// The dab Photoshop lays on the first resting sample of an unmoved stroke is down.
    start_rest_dab: bool,
}

impl PathWalker {
    pub fn new(build_up_interval_ms: Option<f64>) -> Self {
        Self {
            last: None,
            next_at: 0.0,
            pending_first: None,
            initial_dir: None,
            dir: 0.0,
            step: 0,
            interval: build_up_interval_ms,
            time_acc: 0.0,
            speed: None,
            speed_acc: 0.0,
            travelled: 0.0,
            in_place: true,
            wait_for_direction: false,
            zoom: 1.0,
            seg_prev: (0.0, CSP_REST_MS),
            speed_at_last: 0.0,
            seg_from: 0.0,
            seg_to: 0.0,
            rest_step: None,
            rest_acc: 0.0,
            moved: false,
            start_rest_dab: false,
        }
    }

    /// Photoshop's pen rules at view `zoom`: a resting pen lays a dab every
    /// [`PS_REST_INTERVAL_MS`] (the airbrush's own interval when it is on) and one on the first
    /// resting sample of a stroke that has not moved, and a pressure or pose change without
    /// movement lays nothing by itself.
    pub fn photoshop_rest(mut self, zoom: f32) -> Self {
        let zoom = if zoom.is_finite() && zoom > 0.0 { f64::from(zoom).clamp(1e-3, 1e3) } else { 1.0 };
        self.rest_step = Some(PS_REST_MAX_STEP_PX / zoom);
        self.in_place = false;
        self
    }

    /// The view zoom (screen pixels per document pixel) the stroke is drawn at: speeds are on-screen.
    pub fn with_zoom(mut self, zoom: f32) -> Self {
        self.zoom = if zoom.is_finite() && zoom > 0.0 { f64::from(zoom).clamp(1e-3, 1e3) } else { 1.0 };
        self
    }

    /// Lay the first step only once the path has moved; a stroke that never moves lays nothing.
    pub fn wait_for_direction(mut self, on: bool) -> Self {
        self.wait_for_direction = on;
        self
    }

    /// Stop laying steps for pressure or pose changes without movement.
    pub fn stop_in_place_steps(&mut self) {
        self.in_place = false;
    }

    /// Space steps by stroke time instead of distance ([`SPEED_SPACING_MS`]): faster movement
    /// spreads them out. Segments without timestamps are spaced by distance.
    pub fn speed_spacing(mut self, interval_ms: f64) -> Self {
        self.speed = Some(interval_ms.max(0.1));
        self
    }

    /// Emit a step at fraction `f` of the current segment; returns the path distance `out` asks
    /// for to the next one.
    fn emit(&mut self, point: StrokePoint, distance: f64, f: f64, out: &mut impl FnMut(StepInput) -> f64) -> f64 {
        let f = if f.is_finite() { f.clamp(0.0, 1.0) as f32 } else { 1.0 };
        let s = StepInput {
            point,
            direction: self.dir,
            initial_direction: self.initial_dir.unwrap_or(self.dir),
            step: self.step,
            distance,
            speed: self.seg_from + (self.seg_to - self.seg_from) * f,
        };
        self.step += 1;
        out(s)
    }

    /// Feed one point. `out` builds a step's dabs and returns the distance to the next step.
    pub fn push(&mut self, p: StrokePoint, out: &mut impl FnMut(StepInput) -> f64) {
        self.push_with_speed(p, None, out);
    }

    /// [`Self::push`] with the velocity input at `p` given (document pixels per ms, as a CSP
    /// stabilizer reports it) instead of measured from the path.
    pub fn push_with_speed(&mut self, p: StrokePoint, speed: Option<f64>, out: &mut impl FnMut(StepInput) -> f64) {
        let Some(a) = self.last else {
            self.last = Some(p);
            self.pending_first = Some(p);
            return;
        };
        let (dx, dy) = (p.x - a.x, p.y - a.y);
        let len = (dx * dx + dy * dy).sqrt();
        // CSP's velocity input at an input point is the on-screen distance over the time of the two
        // segments before it (at the stroke's start the segment before counts as a rest of
        // `CSP_REST_MS`), and steps between two points interpolate it (measured,
        // docs/sut-analysis.md › Effector velocity input). A move without a duration repeats the
        // previous segment; an untimed stop is a rest as long as the previous segment.
        let seg_dt = p.time - a.time;
        let seg = if seg_dt > 0.0 && seg_dt.is_finite() && len.is_finite() {
            ((len * self.zoom).min(1e9), seg_dt.min(1e9))
        } else if len > 1e-9 {
            self.seg_prev
        } else {
            (0.0, self.seg_prev.1)
        };
        let (dist, dt) = (seg.0 + self.seg_prev.0, seg.1 + self.seg_prev.1);
        let v = match speed.filter(|v| v.is_finite()) {
            Some(v) => v.max(0.0) * self.zoom,
            None if dt > 0.0 => dist / dt,
            None => 0.0,
        };
        self.seg_from = self.speed_at_last;
        self.seg_to = if v.is_finite() { v.min(1e6) as f32 } else { 0.0 };
        self.speed_at_last = self.seg_to;
        self.seg_prev = seg;
        if len > 1e-9 {
            self.dir = (-dy).atan2(dx) as f32;
            if self.initial_dir.is_none() {
                self.initial_dir = Some(self.dir);
            }
        }
        if self.wait_for_direction && self.pending_first.is_some() && len <= 1e-9 {
            // Still in place: keep waiting, with the newest pressure and pose.
            self.pending_first = Some(p);
            self.last = Some(p);
            return;
        }
        if let Some(f) = self.pending_first.take() {
            self.next_at = self.emit(f, self.travelled, 0.0, out).max(0.25);
        }
        if len > 1e-9 {
            // Speed spacing needs the segment's duration; without timestamps (a straight ⇧-click
            // line, or input that carries no times) the segment is spaced by distance instead.
            let dt = p.time - a.time;
            match self.speed.filter(|_| dt > 0.0 && dt.is_finite()) {
                None => {
                    while self.next_at <= len + 1e-9 {
                        let t = (self.next_at / len).min(1.0);
                        let q = lerp_pt(&a, &p, t);
                        let at = self.travelled + self.next_at.min(len);
                        self.next_at += self.emit(q, at, t, out).max(0.25);
                    }
                    self.next_at -= len;
                }
                Some(iv) => {
                    self.speed_acc += dt;
                    let max_n = (len / MIN_SPEED_STEP).ceil().max(1.0) as usize;
                    let mut n = 0;
                    while self.speed_acc >= iv && n < max_n {
                        self.speed_acc -= iv;
                        let f = ((dt - self.speed_acc) / dt).clamp(0.0, 1.0);
                        self.emit(lerp_pt(&a, &p, f), self.travelled + f * len, f, out);
                        n += 1;
                    }
                    if n == max_n {
                        self.speed_acc = self.speed_acc.rem_euclid(iv);
                    }
                }
            }
        }
        let mut emitted_endpoint = false;
        if let Some(iv) = self.interval {
            let dt = p.time - a.time;
            if dt > 0.0 && dt.is_finite() {
                if dt > iv * MAX_AIRBRUSH_DABS_PER_SEGMENT as f64 {
                    // A stalled or hostile timestamp must not replay an arbitrarily long backlog.
                    self.time_acc = 0.0;
                    self.emit(p, self.travelled + len, 1.0, out);
                    self.travelled += len;
                    self.last = Some(p);
                    return;
                }
                self.time_acc += dt;
                let mut n = 0;
                while self.time_acc >= iv && n < MAX_AIRBRUSH_DABS_PER_SEGMENT {
                    self.time_acc -= iv;
                    let f = ((dt - self.time_acc) / dt).clamp(0.0, 1.0);
                    self.emit(lerp_pt(&a, &p, f), self.travelled + f * len, f, out);
                    emitted_endpoint = f >= 1.0;
                    n += 1;
                }
            }
        }
        if let Some(max_step) = self.rest_step {
            let dt = p.time - a.time;
            if len > max_step {
                self.rest_acc = 0.0;
                self.moved = true;
            } else if dt > 0.0 && dt.is_finite() {
                if !self.moved && !self.start_rest_dab {
                    // Photoshop lays a second dab where an unmoved stroke first rests (a 40 ms
                    // hold leaves 2; a stroke that rests only after moving gets none).
                    self.start_rest_dab = true;
                    self.emit(p, self.travelled + len, 1.0, out);
                }
                if self.interval.is_none() {
                    self.rest_acc += dt;
                    let mut n = 0;
                    while self.rest_acc >= PS_REST_INTERVAL_MS && n < MAX_AIRBRUSH_DABS_PER_SEGMENT {
                        self.rest_acc -= PS_REST_INTERVAL_MS;
                        self.emit(p, self.travelled + len, 1.0, out);
                        n += 1;
                    }
                    if n == MAX_AIRBRUSH_DABS_PER_SEGMENT {
                        // A stalled or hostile timestamp must not replay an arbitrarily long backlog.
                        self.rest_acc = 0.0;
                    }
                }
            }
        }
        if self.in_place && len <= 1e-9 && stylus_changed(&a, &p) && !emitted_endpoint {
            // Distance spacing cannot represent a pressure/pose change in place. Emit it once,
            // keeping unchanged stationary samples inert and avoiding a duplicate airbrush dab.
            self.next_at = self.emit(p, self.travelled + len, 1.0, out).max(0.25);
        }
        self.travelled += len;
        self.last = Some(p);
    }

    pub fn finish(&mut self, out: &mut impl FnMut(StepInput) -> f64) {
        if let Some(f) = self.pending_first.take()
            && !self.wait_for_direction
        {
            self.emit(f, self.travelled, 0.0, out);
        }
    }
}

// ---------------------------------------------------------------------------
// Dynamics
// ---------------------------------------------------------------------------

/// The tilt, in degrees, at which Photoshop's Pen Tilt controls reach their end: the full tilt a
/// pen reports (AppKit's tilt of 1; Wacom pens stop at about 60°). Measured on macOS with a
/// synthetic pen (docs/pen-input-validation.md › Photoshop pressure and tilt): roundness and
/// opacity on Pen Tilt run `1 − f` exactly, `f` being the tilt as a fraction of the full tilt.
pub const FULL_TILT_DEGREES: f32 = 60.0;

/// The pen's lean as a fraction of the device's full tilt: 0 upright (or a mouse), 1 at
/// [`FULL_TILT_DEGREES`] or more. Both axes count (the length of the tilt vector).
pub fn tilt_fraction(p: &StrokePoint) -> f32 {
    let t = p.tilt_x.hypot(p.tilt_y) / FULL_TILT_DEGREES;
    if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
}

/// Brush Projection, as Photoshop draws it (measured, same section): the tip stands across the
/// pen's axis and is thrown onto the canvas from a point up the pen, 1 / [`PROJECTION_CONE`] tip
/// radii from the tip (a cone, so the footprint of a round tip is an ellipse). The pen's lean per
/// axis is [`PROJECTION_FULL_DEGREES`] at the device's full tilt.
pub const PROJECTION_FULL_DEGREES: f32 = 64.0;
/// Tangent of the projection cone's half angle (the tip's radius over the distance to the apex).
pub const PROJECTION_CONE: f32 = 0.24;

/// A tilted pen's Brush Projection: the direction the pen leans (radians, counter-clockwise, y
/// up), the tip's scale along that direction and across it, and how far the footprint's centre
/// moves away from the lean, in tip radii. `None` for an upright pen.
pub fn tilt_projection(p: &StrokePoint) -> Option<(f32, f32, f32, f32)> {
    let axis = |degrees: f32| {
        let t = if degrees.is_finite() { (degrees / FULL_TILT_DEGREES).clamp(-1.0, 1.0) } else { 0.0 };
        (t * PROJECTION_FULL_DEGREES).to_radians().tan()
    };
    // The pen's direction is (tan θx, tan θy, 1): its lean from upright and where it points.
    let (lx, ly) = (axis(p.tilt_x), -axis(p.tilt_y));
    let lean = lx.hypot(ly).atan().min(PROJECTION_FULL_DEGREES.to_radians());
    if lean <= 1e-4 {
        return None;
    }
    let cone = PROJECTION_CONE.atan();
    // Where the cone's two edge rays in the plane of the lean meet the canvas, in tip radii from
    // the pen tip: further on the side the pen leans away from.
    let (far, near) = (cone.cos() / (lean + cone).cos().max(0.02), cone.cos() / (lean - cone).cos().max(0.02));
    let across = cone.cos() * lean.cos() / ((lean + cone).cos() * (lean - cone).cos()).max(1e-4).sqrt();
    Some((ly.atan2(lx), (far + near) / 2.0, across, (far - near) / 2.0))
}

/// Scalar control value in 0..1.
pub fn control_value(c: Control, fade_steps: u32, s: &StepInput) -> f32 {
    let p = &s.point;
    match c {
        Control::Off | Control::InitialDirection | Control::Direction => 1.0,
        Control::Fade => 1.0 - (s.step as f32 / fade_steps.max(1) as f32).min(1.0),
        Control::PenPressure => p.pressure.clamp(0.0, 1.0),
        // Upright pen = 1, pen at the device's full tilt = 0 (a mouse reports no tilt, so paints
        // at full value).
        Control::PenTilt => 1.0 - tilt_fraction(p),
        Control::StylusWheel => p.wheel.clamp(0.0, 1.0),
        Control::Rotation => (p.rotation.rem_euclid(360.0)) / 360.0,
    }
}

/// Angle contributed by a control, in degrees (counter-clockwise).
pub fn control_angle(c: Control, fade_steps: u32, s: &StepInput) -> f32 {
    let p = &s.point;
    match c {
        Control::Off => 0.0,
        // 360° × the control value, as for pressure: Photoshop's Fade turns the tip clockwise by
        // 360° / steps per step (measured, `fade2-angle`), and stops at 0° after `fade_steps`.
        Control::Fade => 360.0 * control_value(c, fade_steps, s),
        Control::PenPressure => 360.0 * p.pressure.clamp(0.0, 1.0),
        Control::PenTilt => {
            if p.tilt_x == 0.0 && p.tilt_y == 0.0 {
                0.0
            } else {
                (-p.tilt_y).atan2(p.tilt_x).to_degrees()
            }
        }
        Control::StylusWheel => 360.0 * p.wheel.clamp(0.0, 1.0),
        Control::Rotation => p.rotation,
        Control::InitialDirection => s.initial_direction.to_degrees(),
        Control::Direction => s.direction.to_degrees(),
    }
}

/// A 0..1 multiplier from a dynamic with control value `cv`: `minimum` plus the rest scaled by the
/// control and reduced by the jitter, `min + (1 − min)·cv·(1 − jitter·r)`. So jitter spreads evenly
/// down to the minimum rather than piling up at it, as Photoshop's does (measured, `jit-roundness`
/// and `jit-size-50`: uniform over [minimum, 1]).
fn jittered(d: &Dynamic, cv: f32, r: f32) -> f32 {
    let min = d.minimum.clamp(0.0, 1.0);
    min + (1.0 - min) * cv.clamp(0.0, 1.0) * (1.0 - d.jitter.clamp(0.0, 1.0) * r)
}

/// CSP "Adjust brush density by gap" (`BrushSettings::csp_flow_by_spacing`): the per-dab density
/// whose overlap at the brush's spacing builds up to `flow`. A point of the stroke is crossed by
/// about `1 / spacing` dabs, each adding `c + f·(1 − c)`, so `f = 1 − (1 − flow)^spacing`. A gap
/// of the diameter or more leaves the density alone. The formula is Prism's (constant density
/// across gaps); CLIP STUDIO's own curve is not measured (docs/sut-analysis.md).
fn spacing_flow(b: &BrushSettings, flow: f32) -> f32 {
    if !b.csp_flow_by_spacing || !flow.is_finite() || !b.spacing.is_finite() || b.spacing >= 1.0 {
        return flow;
    }
    1.0 - (1.0 - flow.clamp(0.0, 1.0)).powf(b.spacing.max(0.01))
}

/// [`jittered`] with the dynamic's own control (1 when the dynamic is off).
fn scalar(d: &Dynamic, s: &StepInput, r: f32) -> f32 {
    if !d.is_active() {
        return 1.0;
    }
    jittered(d, dynamic_control(d, s), r) * csp_tilt(d, s) * csp_speed(d, s) * csp_random(d, r)
}

/// The CSP velocity response's factor at step `s` (1 when the control is off).
fn csp_speed(d: &Dynamic, s: &StepInput) -> f32 {
    d.csp_speed.as_ref().map_or(1.0, |v| v.multiplier(s.speed))
}

fn csp_tilt(d: &Dynamic, s: &StepInput) -> f32 {
    d.csp_tilt.as_ref().map_or(1.0, |tilt| tilt.multiplier(s.point.tilt_x, s.point.tilt_y))
}

/// The CSP random control's factor for random number `r` (1 when the control is off).
fn csp_random(d: &Dynamic, r: f32) -> f32 {
    d.csp_random.filter(|m| m.is_finite()).map_or(1.0, |m| {
        let m = m.clamp(0.0, 1.0);
        m + (1.0 - m) * (1.0 - r.clamp(0.0, 1.0))
    })
}

/// A curve belongs to its parameter, so evaluate it after input interpolation rather than
/// transforming the shared pressure samples used by every parameter.
fn dynamic_control(d: &Dynamic, s: &StepInput) -> f32 {
    let value = control_value(d.control, d.fade_steps, s);
    if d.control == Control::PenPressure { d.pressure_curve.as_ref().map_or(value, |curve| curve.sample(value)) } else { value }
}

fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let h = if d <= 0.0 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    } / 6.0;
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// CLIP STUDIO's sub-colour blend for one dab: the amount times its pen input response (the
/// per-dab evaluator multiplies a colour amount by its effector's output).
fn csp_sub_amount(b: &BrushSettings, s: &StepInput, seed: u64, index: u64) -> f32 {
    let amount = b.csp_sub_color.filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0);
    let response = b.csp_sub_color_dynamic.as_ref().map_or(1.0, |d| scalar(d, s, rand01(seed, index, stream::FG_BG)));
    Some(amount * response).filter(|t| t.is_finite()).map_or(amount, |t| t.clamp(0.0, 1.0))
}

/// Colour Dynamics for one dab (or once per stroke, with `index = 0`).
pub fn dynamic_color(b: &BrushSettings, s: &StepInput, seed: u64, index: u64) -> [f32; 4] {
    let cd = &b.color_dynamics;
    let (fg, bg) = (b.color, b.background);
    if !cd.enabled {
        let t = csp_sub_amount(b, s, seed, index);
        return std::array::from_fn(|i| fg[i] + (bg[i] - fg[i]) * t);
    }
    let r = |st: u64| rand01(seed, index, st);
    let base = if cd.fg_bg.control != Control::Off { 1.0 - dynamic_control(&cd.fg_bg, s) } else { 0.0 };
    let fixed = csp_sub_amount(b, s, seed, index);
    let base = fixed + (1.0 - fixed) * base;
    let t = (base + (1.0 - base) * cd.fg_bg.jitter.clamp(0.0, 1.0) * r(stream::FG_BG)).clamp(0.0, 1.0);
    let rgb = [fg[0] + (bg[0] - fg[0]) * t, fg[1] + (bg[1] - fg[1]) * t, fg[2] + (bg[2] - fg[2]) * t];
    let [mut h, mut sat, mut v] = rgb_to_hsv(rgb);
    h += rand_signed(seed, index, stream::HUE) * cd.hue_jitter.clamp(0.0, 1.0) * 0.5;
    sat = (sat + rand_signed(seed, index, stream::SAT) * cd.saturation_jitter.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let pu = cd.purity.clamp(-1.0, 1.0);
    sat = if pu >= 0.0 { sat + (1.0 - sat) * pu } else { sat * (1.0 + pu) };
    v = (v + rand_signed(seed, index, stream::BRIGHT) * cd.brightness_jitter.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let o = hsv_to_rgb(h, sat, v);
    [o[0], o[1], o[2], fg[3] + (bg[3] - fg[3]) * t]
}

/// A dab's diameter at a step: size × the size control (or the options bar's pressure-for-size)
/// × the size jitter, `r` being the jitter's random number.
pub fn dab_diameter(b: &BrushSettings, s: &StepInput, r: f32) -> f32 {
    let sd = &b.shape_dynamics;
    let cv = if sd.enabled && sd.size.control == Control::PenTilt {
        // Photoshop's size control on Pen Tilt leaves the diameter alone at every tilt and Minimum
        // Diameter (measured): it stretches the tip's height by Tilt Scale instead (`tilt_tall`).
        1.0
    } else if sd.enabled && sd.size.control != Control::Off {
        dynamic_control(&sd.size, s)
    } else if b.pressure_size {
        s.point.pressure.clamp(0.0, 1.0)
    } else {
        1.0
    };
    (b.size * if sd.enabled { jittered(&sd.size, cv, r) * csp_tilt(&sd.size, s) * csp_speed(&sd.size, s) * csp_random(&sd.size, r) } else { cv }).min(crate::brush::MAX_BRUSH_SIZE)
}

/// Tilt Scale: the factor on the tip's height with the size control on Pen Tilt. Photoshop
/// stretches the height by `1 + tiltScale × f` (measured: ×3 at 200 % and full tilt, ×2 at 100 %,
/// nothing at 0 %), `f` being [`tilt_fraction`].
pub fn tilt_tall(b: &BrushSettings, p: &StrokePoint) -> f32 {
    let sd = &b.shape_dynamics;
    if !sd.enabled || sd.size.control != Control::PenTilt || !sd.tilt_scale.is_finite() {
        return 1.0;
    }
    1.0 + sd.tilt_scale.clamp(0.0, 2.0) * tilt_fraction(p)
}

/// Whether the brush's Pen Tilt controls follow Photoshop (a CLIP STUDIO brush keeps its own
/// tilt direction: docs/sut-analysis.md).
fn photoshop_tilt(b: &BrushSettings) -> bool {
    b.csp_anti_alias.is_none() && b.csp_tip_scale.is_none()
}

/// Builds the primary dabs of a step (Shape Dynamics, Scattering, Transfer, Color, Texture depth).
#[derive(Clone, Debug)]
pub struct DabBuilder {
    /// Immutable stroke settings; cloning a preview must not copy sampled tip pixels.
    pub brush: Arc<BrushSettings>,
    pub seed: u64,
    /// Per-stroke colour (Color Dynamics with per-tip off), computed at the first step.
    stroke_color: Option<[f32; 4]>,
    next_index: u64,
    /// CSP start and end taper, when the brush has one.
    pub in_out: Option<crate::csp_in_out::InOutRuntime>,
    /// CSP Ribbon progress, when the brush draws one.
    ribbon: Option<crate::csp_ribbon::RibbonState>,
}

impl DabBuilder {
    pub fn new(brush: &BrushSettings) -> Self {
        let in_out = brush.csp_in_out.as_ref().and_then(crate::csp_in_out::InOutRuntime::live);
        // CSP draws ribbons only with tip images and without spraying.
        let ribbon = brush.csp_ribbon.filter(|_| brush.csp_spray.is_none()).and_then(|ribbon| {
            let tips = match &brush.tip {
                crate::brush::TipShape::Sampled(t) if t.is_valid() => vec![(t.width, t.height)],
                crate::brush::TipShape::Sequence(v) => v.iter().take(64).filter(|t| t.is_valid()).map(|t| (t.width, t.height)).collect(),
                _ => Vec::new(),
            };
            crate::csp_ribbon::RibbonState::new(ribbon, tips)
        });
        Self { brush: Arc::new(brush.clone()), seed: brush.seed, stroke_color: None, next_index: 0, in_out, ribbon }
    }

    /// Distance to the next step after a dab of `diameter`: Photoshop spaces dabs by the diameter of
    /// the dab just laid (after size jitter and control; measured, `jit-size-50`: gap / previous
    /// diameter = 2.03 ± 0.002 at 200 %) times the tip's roundness (its short axis; `spc-round30-a90`).
    fn step_after(&self, diameter: f32) -> f64 {
        let b = &self.brush;
        f64::from(diameter * b.roundness.clamp(0.01, 1.0) * b.spacing.max(0.01)).max(MIN_DAB_STEP)
    }

    /// Build a step's dabs; returns the distance to the next step.
    pub fn build(&mut self, s: &StepInput, out: &mut Vec<Dab>) -> f64 {
        let first = out.len();
        let step = self.build_dabs(s, out);
        let Some(ribbon) = self.ribbon.as_mut() else { return step };
        // A ribbon lays the tip between this step and the last one, as one slice that takes the
        // step's dab (size, flow, opacity, colour) without its scatter or angle.
        let dab = out.get(first).copied();
        out.truncate(first);
        let center = Point::new(s.point.x, s.point.y);
        let radius = dab.map_or(0.0, |d| d.radius);
        let scale = self.brush.csp_tip_scale.unwrap_or([1.0, 1.0]);
        let (slice, distance) = ribbon.step(center, radius, s.direction, scale);
        if let (Some(slice), Some(mut d)) = (slice, dab) {
            d.center = Point::new((slice.start.center.x + slice.end.center.x) / 2.0, (slice.start.center.y + slice.end.center.y) / 2.0);
            d.radius = slice.start.half.max(slice.end.half).max(0.5);
            d.ribbon = Some(slice);
            out.push(d);
        }
        distance
    }

    fn build_dabs(&mut self, s: &StepInput, out: &mut Vec<Dab>) -> f64 {
        let b = &self.brush;
        let seed = self.seed;
        let sd = &b.shape_dynamics;
        let sc = &b.scattering;
        let spray = b.csp_spray.as_ref();
        let in_out = self.in_out.as_ref();
        let taper = |param| in_out.map_or(1.0, |t| t.multiplier(param, s.distance));
        use crate::csp_in_out::CspInOutParam as Taper;
        let count = if let Some(spray) = spray {
            let k = scalar(&spray.count_dynamic, s, rand01(seed, s.step, stream::COUNT)) * taper(Taper::SprayDensity);
            ((spray.count as f32 * k).round() as u32).clamp(1, crate::csp_spray::MAX_PARTICLES)
        } else if sc.enabled {
            let n = sc.count.clamp(1, 16) as f32;
            let k = scalar(&sc.count_jitter, s, rand01(seed, s.step, stream::COUNT));
            ((n * k).round() as u32).clamp(1, 16)
        } else {
            1
        };
        let mut step = None;
        for _ in 0..count {
            let i = self.next_index;
            self.next_index += 1;
            let r = |st: u64| rand01(seed, i, st);
            // Shape.
            let area_diameter = dab_diameter(b, s, r(stream::SIZE)) * taper(Taper::Size);
            let diameter = spray.map_or(area_diameter, |spray| spray.size * scalar(&spray.size_dynamic, s, r(stream::SPRAY_SIZE)) * taper(Taper::SpraySize));
            // CSP "at least 1 pixel": a sub-pixel dab (pressure, taper) is laid and spaced as 1 px.
            let diameter = if b.csp_at_least_1px && diameter.is_finite() { diameter.max(1.0) } else { diameter };
            // The step's first dab sets the spacing to the next step (CSP: times the spacing's
            // pen response).
            if step.is_none() {
                let k = b.csp_spacing_dynamic.as_ref().map_or(1.0, |d| scalar(d, s, r(stream::SPACING)));
                let k = if k.is_finite() { f64::from(k.clamp(0.01, 1.0)) } else { 1.0 };
                step = Some((self.step_after(diameter) * k).max(MIN_DAB_STEP));
            }
            let mut angle = b.angle;
            let mut roundness = b.roundness.clamp(0.01, 1.0);
            let (mut fx, mut fy) = (b.flip_x, b.flip_y);
            if sd.enabled {
                // Angle jitter 100 % is ±360° (Photoshop: 25 % already spreads a flat tip over every
                // orientation, `jit2-angle-25`).
                let controlled_angle = if sd.angle.control == Control::PenPressure {
                    360.0 * dynamic_control(&sd.angle, s)
                } else if sd.angle.control == Control::PenTilt && photoshop_tilt(b) && (s.point.tilt_x != 0.0 || s.point.tilt_y != 0.0) {
                    // Photoshop turns the tip's height towards the pen's lean (measured: the long
                    // axis of a flat tip lies across it), a quarter turn from the lean itself.
                    control_angle(Control::PenTilt, 0, s) + 90.0
                } else {
                    control_angle(sd.angle.control, sd.angle.fade_steps, s)
                };
                roundness = (roundness * scalar(&sd.roundness, s, r(stream::ROUNDNESS))).clamp(0.01, 1.0);
                // Photoshop doesn't turn a fully round computed tip (measured with the angle on Pen
                // Tilt: with Tilt Scale its stretched height stays upright; taken to hold for the
                // other angle controls and the jitter, which only Tilt Scale could show).
                let turns = !(matches!(b.tip, crate::brush::TipShape::Round) && roundness >= 0.999 && photoshop_tilt(b));
                if turns {
                    angle += controlled_angle + rand_signed(seed, i, stream::ANGLE) * sd.angle.jitter.clamp(0.0, 1.0) * 360.0;
                }
                if sd.flip_x_jitter && r(stream::FLIP_X) < 0.5 {
                    fx = !fx;
                }
                if sd.flip_y_jitter && r(stream::FLIP_Y) < 0.5 {
                    fy = !fy;
                }
            }
            // A spray particle has its own angle; the area's angle is one of its inputs.
            let area_angle = angle;
            if let Some(spray) = spray {
                angle = spray.angle;
            }
            // Tilt Scale and Brush Projection (Shape Dynamics).
            let (mut proj_angle, mut proj_scale, mut proj_cross, mut proj_shift) = (0.0, 1.0, 1.0, 0.0);
            let tall = tilt_tall(b, &s.point);
            if sd.enabled && sd.brush_projection {
                angle += s.point.rotation;
                if let Some((lean, along, across, shift)) = tilt_projection(&s.point) {
                    (proj_angle, proj_scale, proj_cross, proj_shift) = (lean, along, across, shift);
                }
            }
            // A dab shrunk to nothing paints nothing (Photoshop's size Fade to 0 % stops painting);
            // below 1 px it keeps a 1 px tip and lays ink in proportion to its area.
            if !diameter.is_finite() || diameter <= 1e-3 {
                continue;
            }
            let tiny = diameter.min(1.0).powi(2);
            let radius = (diameter / 2.0).max(0.5);
            // Scatter.
            let mut c = Point::new(s.point.x, s.point.y);
            if proj_shift != 0.0 {
                // The projected footprint lies away from the pen's lean (y-down image space).
                let d = f64::from(proj_shift * radius);
                c.x -= d * f64::from(proj_angle.cos());
                c.y += d * f64::from(proj_angle.sin());
            }
            if let Some(spray) = spray {
                let radial = crate::csp_spray::radial(spray.bias, r(stream::SPRAY_RADIUS_A), r(stream::SPRAY_RADIUS_B));
                let distance = f64::from(area_diameter.max(0.0) * 0.5 * radial);
                let position = f64::from(r(stream::SPRAY_POSITION)) * std::f64::consts::TAU;
                c.x += distance * position.cos();
                c.y += distance * position.sin();
                use crate::csp_spray::CspParticleAngle as Input;
                angle += match spray.angle_input {
                    Input::Fixed => 0.0,
                    Input::PenTilt => control_angle(Control::PenTilt, 0, s),
                    Input::Rotation => control_angle(Control::Rotation, 0, s),
                    Input::Direction => control_angle(Control::Direction, 0, s),
                    Input::Spray => area_angle,
                    // CCW degrees in y-down image space: the offset (cos p, sin p) points at −p.
                    Input::TowardCenter => -(position.to_degrees() as f32),
                };
                if spray.angle_jitter > 0.0 {
                    angle += rand_signed(seed, i, stream::SPRAY_ANGLE) * spray.angle_jitter.clamp(0.0, 2.0) * 180.0;
                }
            } else if sc.enabled && sc.scatter.jitter > 0.0 {
                let cv = dynamic_control(&sc.scatter, s);
                let amt = f64::from(sc.scatter.jitter * cv * radius);
                if sc.both_axes {
                    // Any direction, up to the amount (Photoshop: the offset across the path has
                    // sd ≈ amount / √6, `jit-scatter-200-both`, which a square spread doesn't give).
                    let (a, d) = (f64::from(rand01(seed, i, stream::SCATTER_X)) * std::f64::consts::TAU, f64::from(rand01(seed, i, stream::SCATTER_Y)) * amt);
                    c.x += d * a.cos();
                    c.y += d * a.sin();
                } else {
                    let ox = rand_signed(seed, i, stream::SCATTER_X) as f64 * amt;
                    // Perpendicular to the direction of travel (y-down image space).
                    let (dx, dy) = (f64::from(s.direction.cos()), -f64::from(s.direction.sin()));
                    c.x += -dy * ox;
                    c.y += dx * ox;
                }
            }
            // Transfer.
            let tr = &b.transfer;
            let (mut opacity, mut flow) = (1.0, b.flow);
            if tr.enabled {
                opacity = scalar(&tr.opacity, s, r(stream::OPACITY));
                flow *= scalar(&tr.flow, s, r(stream::FLOW));
            }
            flow *= taper(Taper::Flow);
            flow = spacing_flow(b, flow);
            let (wet, mix) = if tr.enabled { (scalar(&tr.wetness, s, r(stream::WET)), scalar(&tr.mix, s, r(stream::MIX))) } else { (1.0, 1.0) };
            if b.pressure_opacity && !(tr.enabled && tr.opacity.control != Control::Off) {
                opacity *= s.point.pressure.clamp(0.0, 1.0);
            }
            // Colour.
            let color = if b.color_dynamics.enabled || b.csp_sub_color.is_some() {
                if b.color_dynamics.per_tip { dynamic_color(b, s, seed, i) } else { *self.stroke_color.get_or_insert_with(|| dynamic_color(b, s, seed, 0)) }
            } else {
                b.color
            };
            // Texture depth (per tip).
            let tx = &b.texture;
            let depth =
                if tx.enabled && tx.each_tip { tx.depth.clamp(0.0, 1.0) * scalar(&tx.depth_jitter, s, r(stream::DEPTH)) } else { tx.depth.clamp(0.0, 1.0) };
            let depth = depth * taper(Taper::TextureDensity);
            // Color mixing: the evaluated amount and density never exceed their base values.
            let (paint_amount, paint_density) = b.csp_mixing.as_ref().map_or((1.0, 1.0), |m| {
                (
                    (m.amount * scalar(&m.amount_dynamic, s, r(stream::PAINT_AMOUNT))).clamp(0.0, 1.0),
                    (m.density * scalar(&m.density_dynamic, s, r(stream::PAINT_DENSITY))).clamp(0.0, 1.0),
                )
            });
            let thickness = b.csp_thickness_dynamic.as_ref().map_or(1.0, |t| scalar(&t.dynamic, s, r(stream::THICKNESS)));
            let thickness = if thickness.is_finite() { thickness.clamp(0.0, 1.0) } else { 1.0 };
            out.push(Dab {
                center: c,
                radius,
                alpha: (flow * tiny).clamp(0.0, 1.0),
                angle: angle.to_radians(),
                roundness,
                flip_x: fx,
                flip_y: fy,
                opacity: opacity.clamp(0.0, 1.0),
                color,
                depth,
                index: i,
                proj_angle,
                proj_scale,
                proj_cross,
                tall,
                wet,
                mix,
                paint_amount,
                paint_density,
                ribbon: None,
                thickness,
            });
        }
        step.unwrap_or(MIN_DAB_STEP)
    }
}

/// Builds Dual Brush dabs (the second tip: size, spacing, scatter, count, flip).
#[derive(Clone, Debug)]
pub struct DualBuilder {
    pub brush: Arc<BrushSettings>,
    next_index: u64,
}

impl DualBuilder {
    pub fn new(brush: &BrushSettings) -> Self {
        Self { brush: Arc::new(brush.clone()), next_index: 0 }
    }
    /// The Dual Brush tip's fixed spacing.
    fn step(&self) -> f64 {
        let d = &self.brush.dual_brush;
        f64::from(d.size.max(1.0) * d.spacing.max(0.01)).max(MIN_DAB_STEP)
    }
    /// Build a step's dabs; returns the distance to the next step.
    pub fn build(&mut self, s: &StepInput, out: &mut Vec<Dab>) -> f64 {
        let d = &self.brush.dual_brush;
        let seed = self.brush.seed ^ 0xD0A1;
        for _ in 0..d.count.clamp(1, 16) {
            let i = self.next_index;
            self.next_index += 1;
            let radius = (d.size / 2.0).max(0.5);
            let amt = f64::from(d.scatter * radius);
            let (ox, oy) = (rand_signed(seed, i, stream::DUAL_X) as f64 * amt, rand_signed(seed, i, stream::DUAL_Y) as f64 * amt);
            let mut c = Point::new(s.point.x, s.point.y);
            if d.both_axes {
                c.x += ox;
                c.y += oy;
            } else {
                let (dx, dy) = (f64::from(s.direction.cos()), -f64::from(s.direction.sin()));
                c.x += -dy * ox;
                c.y += dx * ox;
            }
            let flip = d.flip && rand01(seed, i, stream::DUAL_FLIP) < 0.5;
            out.push(Dab {
                center: c,
                radius,
                alpha: 1.0,
                angle: d.angle.to_radians(),
                roundness: d.roundness.clamp(0.01, 1.0),
                flip_x: flip,
                flip_y: false,
                opacity: 1.0,
                color: [0.0, 0.0, 0.0, 1.0],
                depth: 1.0,
                index: i,
                proj_angle: 0.0,
                proj_scale: 1.0,
                proj_cross: 1.0,
                tall: 1.0,
                wet: 1.0,
                mix: 1.0,
                paint_amount: 1.0,
                paint_density: 1.0,
                ribbon: None,
                thickness: 1.0,
            });
        }
        self.step()
    }
}

/// Apply Brush Pose to an input point.
pub fn apply_pose(b: &BrushSettings, mut p: StrokePoint) -> StrokePoint {
    let pose = &b.pose;
    if pose.enabled {
        if pose.override_tilt {
            p.tilt_x = pose.tilt_x;
            p.tilt_y = pose.tilt_y;
        }
        if pose.override_rotation {
            p.rotation = pose.rotation;
        }
        if pose.override_pressure {
            p.pressure = pose.pressure;
        }
    }
    p
}

/// Incremental dab generation for a whole brush (primary and optional dual dabs).
#[derive(Clone, Debug)]
pub struct DabGenerator {
    zoom: f32,
    stabilizer: crate::csp_stabilizer::Stabilizer,
    smoother: Smoother,
    walker: PathWalker,
    dual_walker: Option<PathWalker>,
    builder: DabBuilder,
    dual: Option<DualBuilder>,
    scratch: Vec<StrokePoint>,
}

impl DabGenerator {
    /// Physical screen pixels per view point (HiDPI): CSP's velocity input is the speed in screen
    /// pixels, while `zoom` is in view points. Only the speed uses it.
    pub fn with_pixels_per_point(mut self, ppp: f32) -> Self {
        let ppp = if ppp.is_finite() && ppp > 0.0 { ppp.clamp(0.1, 10.0) } else { 1.0 };
        self.walker = self.walker.with_zoom(self.zoom * ppp);
        self.dual_walker = self.dual_walker.map(|w| w.with_zoom(self.zoom * ppp));
        self
    }

    /// Photoshop's resting pen ([`PathWalker::photoshop_rest`]) for the primary and dual brush.
    /// Off for the Clip Studio rules.
    pub fn with_photoshop_rest(mut self) -> Self {
        self.walker = self.walker.photoshop_rest(self.zoom);
        self.dual_walker = self.dual_walker.map(|w| w.photoshop_rest(self.zoom));
        self
    }

    pub fn new(brush: &BrushSettings, zoom: f32) -> Self {
        let brush = brush.bounded_for_render();
        let interval = brush.build_up.then(|| 1000.0 / f64::from(brush.build_up_rate.clamp(0.1, 1000.0)));
        let dual_on = brush.dual_brush.enabled;
        Self {
            zoom,
            stabilizer: crate::csp_stabilizer::Stabilizer::new(&brush.csp_stabilizer.clone().unwrap_or_default(), zoom),
            smoother: Smoother::new(&brush.effective_smoothing(), zoom),
            // A CSP tip turned by the direction of the line has no angle until the pen moves:
            // CLIP STUDIO lays nothing for a tap with such a brush (measured,
            // docs/sut-analysis.md › 가을낙엽).
            walker: if brush.spacing_enabled { PathWalker::new(interval) } else { PathWalker::new(interval).speed_spacing(SPEED_SPACING_MS) }
                .wait_for_direction(brush.csp_follows_line())
                .with_zoom(zoom),
            dual_walker: dual_on.then(|| PathWalker::new(None).wait_for_direction(brush.csp_follows_line()).with_zoom(zoom)),
            builder: DabBuilder::new(&brush),
            dual: dual_on.then(|| DualBuilder::new(&brush)),
            scratch: Vec::new(),
        }
    }

    /// Walk the points; `speeds`, when it lines up with them, gives each point's velocity input
    /// (a CSP stabilizer's, see [`crate::csp_stabilizer::Stabilizer::speed`]).
    fn walk(&mut self, pts: &[StrokePoint], speeds: &[Option<f64>], dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        let aligned = speeds.len() == pts.len();
        for (i, p) in pts.iter().enumerate() {
            let p = apply_pose(&self.builder.brush, *p);
            let speed = if aligned { speeds.get(i).copied().flatten() } else { None };
            let builder = &mut self.builder;
            self.walker.push_with_speed(p, speed, &mut |s| builder.build(&s, dabs));
            if let (Some(w), Some(db)) = (self.dual_walker.as_mut(), self.dual.as_mut()) {
                w.push_with_speed(p, speed, &mut |s| db.build(&s, dual));
            }
        }
    }

    /// The whole stroke's input is known (a committed stroke): resolve CSP start/end lengths from
    /// the path these points walk (after stabilization and smoothing), enabling the end taper.
    pub fn set_full_path(&mut self, pts: &[StrokePoint]) {
        let Some(cfg) = self.builder.brush.csp_in_out.clone() else { return };
        let b = &self.builder.brush;
        let mut stabilizer = crate::csp_stabilizer::Stabilizer::new(&b.csp_stabilizer.clone().unwrap_or_default(), self.zoom);
        let mut smoother = Smoother::new(&b.effective_smoothing(), self.zoom);
        let mut walked = Vec::with_capacity(pts.len());
        let mut caught_up = Vec::new();
        for p in pts {
            smoother.push(stabilizer.push(*p), &mut walked);
        }
        stabilizer.finish(&mut caught_up);
        for p in caught_up {
            smoother.push(p, &mut walked);
        }
        smoother.finish(&mut walked);
        self.builder.in_out = crate::csp_in_out::InOutRuntime::finished(&cfg, &crate::csp_in_out::segment_lengths(&walked));
    }

    /// Feed input points; appends the new primary and dual dabs.
    pub fn push(&mut self, pts: &[StrokePoint], dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        let mut sm = std::mem::take(&mut self.scratch);
        sm.clear();
        let mut speeds = Vec::new();
        for p in pts {
            let p = self.stabilizer.push(*p);
            let before = sm.len();
            self.smoother.push(p, &mut sm);
            // The smoother may add or hold points; the velocity input follows one point each.
            speeds.extend(std::iter::repeat_n(self.stabilizer.speed(), sm.len() - before));
        }
        self.walk(&sm, &speeds, dabs, dual);
        self.scratch = sm;
    }

    /// End of stroke.
    pub fn finish(&mut self, dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        let mut sm = Vec::new();
        let mut caught_up = Vec::new();
        self.stabilizer.finish(&mut caught_up);
        // The stabilizer's pen-up samples repeat the last position while their pressure drains.
        // They are not input: laid in place they pile dabs on the end of the stroke, which shows
        // on a brush that ignores pressure (CLIP STUDIO ends such a stroke with its last spaced
        // dab; measured, docs/sut-analysis.md › 가을낙엽).
        if !caught_up.is_empty() {
            self.walker.stop_in_place_steps();
            if let Some(w) = self.dual_walker.as_mut() {
                w.stop_in_place_steps();
            }
        }
        let mut speeds = Vec::new();
        for p in caught_up {
            let before = sm.len();
            self.smoother.push(p, &mut sm);
            speeds.extend(std::iter::repeat_n(self.stabilizer.speed(), sm.len() - before));
        }
        self.smoother.finish(&mut sm);
        speeds.resize(sm.len(), self.stabilizer.speed());
        self.walk(&sm, &speeds, dabs, dual);
        let builder = &mut self.builder;
        self.walker.finish(&mut |s| builder.build(&s, dabs));
        if let (Some(w), Some(db)) = (self.dual_walker.as_mut(), self.dual.as_mut()) {
            w.finish(&mut |s| db.build(&s, dual));
        }
    }
}
