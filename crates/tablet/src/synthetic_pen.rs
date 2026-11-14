//! Synthetic pen strokes for testing pressure without a physical tablet.
//!
//! A [`Stroke`] is a list of timed [`PenFrame`]s in screen pixels. On Windows,
//! `windows::SyntheticPen` injects them through `InjectSyntheticPointerInput`, so they travel the
//! same Windows Ink path as a real pen: into Prism's native capture, and equally into another
//! application (CLIP STUDIO PAINT in TabletPC mode, Photoshop) to render a comparison oracle.
//!
//! This module is pure and safe: it validates frames and encodes the native flag/mask words, so
//! the encoding is tested on every platform. Pointer flag, pen flag and pen mask values are the
//! documented Win32 constants (`winuser.h`), restated here so the mapping compiles everywhere.

use crate::Error;

/// Pointer flags (`POINTER_FLAG_*`).
pub const FLAG_INRANGE: u32 = 0x0000_0002;
pub const FLAG_INCONTACT: u32 = 0x0000_0004;
pub const FLAG_FIRSTBUTTON: u32 = 0x0000_0010;
pub const FLAG_PRIMARY: u32 = 0x0000_2000;
pub const FLAG_DOWN: u32 = 0x0001_0000;
pub const FLAG_UPDATE: u32 = 0x0002_0000;
pub const FLAG_UP: u32 = 0x0004_0000;
/// Pen flags (`PEN_FLAG_*`).
pub const PEN_BARREL: u32 = 0x1;
pub const PEN_INVERTED: u32 = 0x2;
pub const PEN_ERASER: u32 = 0x4;
/// Pen masks (`PEN_MASK_*`): which optional fields are valid.
pub const MASK_PRESSURE: u32 = 0x1;
pub const MASK_ROTATION: u32 = 0x2;
pub const MASK_TILT_X: u32 = 0x4;
pub const MASK_TILT_Y: u32 = 0x8;

/// Most frames one stroke may hold (the same bound as the native capture transport).
pub const MAX_FRAMES: usize = 4096;
/// Longest pause between two frames; a longer one is almost certainly a unit mistake.
pub const MAX_DELAY_MS: u32 = 10_000;
/// Hover frames before contact in [`Stroke::from_contact`].
pub const HOVER_FRAMES: usize = 8;

/// What a frame does to the pen contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// In range above the surface, no contact.
    Hover,
    /// Tip touches down.
    Down,
    /// Tip moves (or changes pressure) while in contact.
    Move,
    /// Tip lifts; the pen stays in range.
    Up,
    /// Pen leaves the detection range.
    Leave,
}

/// One injected pen sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenFrame {
    pub phase: Phase,
    /// Screen position in physical pixels.
    pub x: f64,
    pub y: f64,
    /// Tip pressure 0..1.
    pub pressure: f32,
    /// Tilt in degrees, -90..90.
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Barrel rotation in degrees, 0..360.
    pub rotation: f32,
    pub eraser: bool,
    pub barrel: bool,
    /// Pause before this frame is injected, in milliseconds.
    pub delay_ms: u32,
}

impl PenFrame {
    pub fn new(phase: Phase, x: f64, y: f64, pressure: f32, delay_ms: u32) -> Self {
        Self { phase, x, y, pressure, tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, eraser: false, barrel: false, delay_ms }
    }

    /// `POINTER_FLAG_*` for this frame, per the documented injection sequence.
    pub fn pointer_flags(&self) -> u32 {
        match self.phase {
            Phase::Hover => FLAG_INRANGE | FLAG_UPDATE | FLAG_PRIMARY,
            Phase::Down => FLAG_INRANGE | FLAG_INCONTACT | FLAG_FIRSTBUTTON | FLAG_DOWN | FLAG_PRIMARY,
            Phase::Move => FLAG_INRANGE | FLAG_INCONTACT | FLAG_FIRSTBUTTON | FLAG_UPDATE | FLAG_PRIMARY,
            Phase::Up => FLAG_INRANGE | FLAG_UP | FLAG_PRIMARY,
            Phase::Leave => FLAG_UPDATE | FLAG_PRIMARY,
        }
    }

    pub fn pen_flags(&self) -> u32 {
        let mut flags = 0;
        if self.barrel {
            flags |= PEN_BARREL;
        }
        if self.eraser {
            flags |= PEN_INVERTED | PEN_ERASER;
        }
        flags
    }

    /// Native pressure, 0..=1024. Zero unless the tip is in contact.
    pub fn native_pressure(&self) -> u32 {
        match self.phase {
            Phase::Down | Phase::Move => (self.pressure.clamp(0.0, 1.0) * 1024.0).round() as u32,
            _ => 0,
        }
    }

    pub fn native_tilt(&self) -> (i32, i32) {
        (self.tilt_x.clamp(-90.0, 90.0).round() as i32, self.tilt_y.clamp(-90.0, 90.0).round() as i32)
    }

    /// Native rotation, 0..=359.
    pub fn native_rotation(&self) -> u32 {
        (self.rotation.rem_euclid(360.0).round() as u32) % 360
    }

    /// Rounded screen position.
    pub fn native_position(&self) -> (i32, i32) {
        (self.x.round() as i32, self.y.round() as i32)
    }

    fn validate(&self, index: usize) -> Result<(), Error> {
        let bad = |what: &str| Err(Error::Unsupported(format!("synthetic pen frame {index}: {what}")));
        if !(self.x.is_finite() && self.y.is_finite()) || self.x.abs() > 1e6 || self.y.abs() > 1e6 {
            return bad("position must be finite screen pixels");
        }
        if !self.pressure.is_finite() || !(0.0..=1.0).contains(&self.pressure) {
            return bad("pressure must be within 0..1");
        }
        if ![self.tilt_x, self.tilt_y].iter().all(|t| t.is_finite() && (-90.0..=90.0).contains(t)) {
            return bad("tilt must be within -90..90 degrees");
        }
        if !self.rotation.is_finite() {
            return bad("rotation must be finite");
        }
        if self.delay_ms > MAX_DELAY_MS {
            return bad("delay is longer than 10 s");
        }
        Ok(())
    }
}

/// A validated sequence of pen frames: contact phases are well ordered, so an injected stroke
/// never leaves the system pen stuck down.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    frames: Vec<PenFrame>,
}

impl Stroke {
    /// Validate `frames`. Contact must open with `Down`, continue with `Move` and close with
    /// `Up` before any `Hover`/`Leave`; the last frame must not leave the tip in contact.
    pub fn new(frames: Vec<PenFrame>) -> Result<Self, Error> {
        if frames.is_empty() || frames.len() > MAX_FRAMES {
            return Err(Error::Unsupported(format!("a synthetic stroke needs 1..={MAX_FRAMES} frames")));
        }
        let mut contact = false;
        for (index, frame) in frames.iter().enumerate() {
            frame.validate(index)?;
            let ok = match frame.phase {
                Phase::Down => !contact,
                Phase::Move | Phase::Up => contact,
                Phase::Hover | Phase::Leave => !contact,
            };
            if !ok {
                return Err(Error::Unsupported(format!("synthetic pen frame {index}: {:?} out of order", frame.phase)));
            }
            contact = matches!(frame.phase, Phase::Down | Phase::Move);
        }
        if contact {
            return Err(Error::Unsupported("synthetic stroke ends with the tip still down".into()));
        }
        Ok(Self { frames })
    }

    /// Build a complete stroke from contact samples `(x, y, pressure, delay_ms)`: hover in at the
    /// first point, touch down, move through the rest, lift and leave at the last point.
    pub fn from_contact(samples: &[(f64, f64, f32, u32)]) -> Result<Self, Error> {
        Self::from_contact_with_hover(samples, HOVER_FRAMES)
    }

    /// [`Stroke::from_contact`] with `hover_frames` (1..=64) 16 ms hover frames before contact.
    /// Few hover frames model a pen that touches down right after entering range.
    pub fn from_contact_with_hover(samples: &[(f64, f64, f32, u32)], hover_frames: usize) -> Result<Self, Error> {
        if !(1..=64).contains(&hover_frames) {
            return Err(Error::Unsupported("hover frames must be 1..=64".into()));
        }
        let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
            return Err(Error::Unsupported("a synthetic stroke needs at least one sample".into()));
        };
        let mut frames = Vec::with_capacity(samples.len().saturating_add(hover_frames + 2));
        // A real pen hovers before it touches: give the target several frames to see the pen
        // over the window before contact (HOVER_FRAMES x 16 ms).
        for i in 0..hover_frames {
            frames.push(PenFrame::new(Phase::Hover, first.0, first.1, 0.0, if i == 0 { 0 } else { 16 }));
        }
        for (i, &(x, y, pressure, delay)) in samples.iter().enumerate() {
            frames.push(PenFrame::new(if i == 0 { Phase::Down } else { Phase::Move }, x, y, pressure, if i == 0 { 16 } else { delay }));
        }
        frames.push(PenFrame::new(Phase::Up, last.0, last.1, 0.0, 8));
        frames.push(PenFrame::new(Phase::Leave, last.0, last.1, 0.0, 16));
        Self::new(frames)
    }

    pub fn frames(&self) -> &[PenFrame] {
        &self.frames
    }

    /// Shift every frame by `(dx, dy)` screen pixels (window-relative strokes onto the screen).
    pub fn offset(mut self, dx: f64, dy: f64) -> Result<Self, Error> {
        for frame in &mut self.frames {
            frame.x += dx;
            frame.y += dy;
        }
        Self::new(self.frames)
    }
}

/// Which of the candidate windows `(title, client width, client height)` to target for
/// `needle` (lowercase): an exact title match first, else the first containing it, skipping any
/// with an empty client area (minimized windows).
pub fn pick_window<'a>(candidates: impl Iterator<Item = (&'a str, i32, i32)>, needle: &str) -> Option<usize> {
    let usable: Vec<(usize, String)> = candidates.enumerate().filter(|(_, (_, w, h))| *w > 0 && *h > 0).map(|(i, (t, _, _))| (i, t.to_lowercase())).collect();
    usable.iter().find(|(_, t)| t == needle).or_else(|| usable.iter().find(|(_, t)| t.contains(needle))).map(|(i, _)| *i)
}

/// A canonical pressure test: a horizontal line of `length` pixels starting at `(x, y)`, sampled
/// every 4 px at 8 ms, whose pressure rises 0→1 over the first half and falls back to 0.
pub fn pressure_ramp(x: f64, y: f64, length: f64) -> Result<Stroke, Error> {
    pressure_ramp_with_hover(x, y, length, HOVER_FRAMES)
}

/// [`pressure_ramp`] with an explicit hover lead-in (see [`Stroke::from_contact_with_hover`]).
pub fn pressure_ramp_with_hover(x: f64, y: f64, length: f64, hover_frames: usize) -> Result<Stroke, Error> {
    if !length.is_finite() || !(8.0..=20_000.0).contains(&length) {
        return Err(Error::Unsupported("ramp length must be 8..=20000 px".into()));
    }
    let steps = (length / 4.0).round() as usize;
    let samples: Vec<_> = (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            let pressure = (1.0 - (2.0 * t - 1.0).abs()) as f32;
            (x + t * length, y, pressure, 8)
        })
        .collect();
    Stroke::from_contact_with_hover(&samples, hover_frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_follow_the_injection_sequence() {
        let f = |phase| PenFrame::new(phase, 0.0, 0.0, 0.5, 0);
        assert_eq!(f(Phase::Down).pointer_flags() & (FLAG_DOWN | FLAG_INCONTACT | FLAG_INRANGE), FLAG_DOWN | FLAG_INCONTACT | FLAG_INRANGE);
        assert_eq!(f(Phase::Move).pointer_flags() & (FLAG_UPDATE | FLAG_INCONTACT), FLAG_UPDATE | FLAG_INCONTACT);
        assert_eq!(f(Phase::Up).pointer_flags() & (FLAG_UP | FLAG_INCONTACT), FLAG_UP);
        assert_eq!(f(Phase::Leave).pointer_flags() & FLAG_INRANGE, 0);
        assert_eq!(f(Phase::Hover).pointer_flags() & FLAG_INCONTACT, 0);
    }

    #[test]
    fn native_values_are_scaled_and_bounded() {
        let mut frame = PenFrame::new(Phase::Move, 10.4, -3.6, 0.5, 0);
        assert_eq!(frame.native_pressure(), 512);
        assert_eq!(frame.native_position(), (10, -4));
        frame.pressure = 1.0;
        assert_eq!(frame.native_pressure(), 1024);
        frame.phase = Phase::Hover;
        assert_eq!(frame.native_pressure(), 0, "no pressure without contact");
        frame.rotation = 359.7;
        assert_eq!(frame.native_rotation(), 0);
        frame.rotation = -90.0;
        assert_eq!(frame.native_rotation(), 270);
        frame.tilt_x = -45.4;
        frame.tilt_y = 30.6;
        assert_eq!(frame.native_tilt(), (-45, 31));
        frame.eraser = true;
        frame.barrel = true;
        assert_eq!(frame.pen_flags(), PEN_BARREL | PEN_INVERTED | PEN_ERASER);
    }

    #[test]
    fn strokes_reject_bad_order_and_values() {
        let f = |phase, pressure: f32| PenFrame::new(phase, 1.0, 1.0, pressure, 0);
        assert!(Stroke::new(vec![]).is_err());
        assert!(Stroke::new(vec![f(Phase::Move, 0.5)]).is_err(), "move without down");
        assert!(Stroke::new(vec![f(Phase::Down, 0.5)]).is_err(), "left down");
        assert!(Stroke::new(vec![f(Phase::Down, 0.5), f(Phase::Down, 0.5), f(Phase::Up, 0.0)]).is_err());
        assert!(Stroke::new(vec![f(Phase::Down, 0.5), f(Phase::Hover, 0.0)]).is_err());
        assert!(Stroke::new(vec![f(Phase::Down, 1.5), f(Phase::Up, 0.0)]).is_err());
        assert!(Stroke::new(vec![f(Phase::Down, f32::NAN), f(Phase::Up, 0.0)]).is_err());
        let mut far = f(Phase::Hover, 0.0);
        far.x = f64::INFINITY;
        assert!(Stroke::new(vec![far]).is_err());
        let mut slow = f(Phase::Hover, 0.0);
        slow.delay_ms = MAX_DELAY_MS + 1;
        assert!(Stroke::new(vec![slow]).is_err());
        let mut tilted = f(Phase::Hover, 0.0);
        tilted.tilt_x = 91.0;
        assert!(Stroke::new(vec![tilted]).is_err());
        assert!(Stroke::new(vec![f(Phase::Hover, 0.0); MAX_FRAMES + 1]).is_err());
        assert!(Stroke::new(vec![f(Phase::Hover, 0.0), f(Phase::Down, 0.2), f(Phase::Move, 0.9), f(Phase::Up, 0.0), f(Phase::Leave, 0.0)]).is_ok());
        assert!(Stroke::from_contact(&[]).is_err());
    }

    #[test]
    fn window_pick_prefers_exact_titles_and_skips_minimized() {
        let w = [("prism – README.md", 0, 0), ("Prism - notes", 800, 600), ("Prism", 3840, 2088)];
        assert_eq!(pick_window(w.iter().copied(), "prism"), Some(2), "exact title wins");
        assert_eq!(pick_window(w[..2].iter().copied(), "prism"), Some(1), "minimized skipped");
        assert_eq!(pick_window(w[..1].iter().copied(), "prism"), None);
        assert_eq!(pick_window(w.iter().copied(), "clip studio"), None);
    }

    #[test]
    fn ramp_rises_and_falls_and_offsets() {
        let stroke = pressure_ramp(100.0, 50.0, 400.0).unwrap();
        let contact: Vec<_> = stroke.frames().iter().filter(|f| matches!(f.phase, Phase::Down | Phase::Move)).collect();
        assert_eq!(contact.len(), 101);
        assert_eq!(contact[0].pressure, 0.0);
        assert!((contact[50].pressure - 1.0).abs() < 1e-6);
        assert_eq!(contact[100].pressure, 0.0);
        assert_eq!(stroke.frames().first().map(|f| f.phase), Some(Phase::Hover));
        assert_eq!(stroke.frames().last().map(|f| f.phase), Some(Phase::Leave));
        let moved = stroke.offset(10.0, 20.0).unwrap();
        assert_eq!(moved.frames()[0].x, 110.0);
        assert_eq!(moved.frames()[0].y, 70.0);
        assert!(pressure_ramp(0.0, 0.0, 1.0).is_err());
        assert!(pressure_ramp(0.0, 0.0, f64::NAN).is_err());
        let quick = pressure_ramp_with_hover(0.0, 0.0, 40.0, 1).unwrap();
        assert_eq!(quick.frames().iter().take_while(|f| f.phase == Phase::Hover).count(), 1);
        assert!(pressure_ramp_with_hover(0.0, 0.0, 40.0, 0).is_err());
        assert!(pressure_ramp_with_hover(0.0, 0.0, 40.0, 65).is_err());
    }
}
