//! Safe, bounded Windows Ink sample transport. Coordinates and pressure stay paired even
//! when the window system coalesces many hardware readings into a single message.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use crate::Error;

const MAX_EVENTS: usize = 4096;
const MAX_CONTACTS: usize = 32;
const MAX_ERRORS: usize = 32;

/// Contact transitions; `Up` and `Cancel` end a contact, never imply a zero-pressure dab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenPhase {
    Down,
    Move,
    Up,
    Cancel,
    Hover,
}

/// A single Windows Ink reading in client physical pixels (divide by pixels-per-point in UI).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenEvent {
    pub window: usize,
    pub pointer_id: u32,
    pub device_id: usize,
    pub phase: PenPhase,
    pub x: f32,
    pub y: f32,
    /// Device milliseconds, extended over the Windows 32-bit timestamp wrap per contact.
    pub time_ms: u64,
    /// Current pressure, or this contact's last valid pressure when the field is missing.
    /// `None` means no valid pressure has arrived: do not substitute maximum pressure.
    pub pressure: Option<f32>,
    /// Whether this individual hardware reading supplied valid pressure.
    pub pressure_valid: bool,
    /// Device capability from its HID properties. Only `Some(false)` permits a mouse-like
    /// constant-pressure fallback; `None` is unknown, never evidence for full pressure.
    pub pressure_supported: Option<bool>,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub rotation: f32,
    pub eraser: bool,
    pub barrel_button: bool,
}

/// Unnormalized platform data; public so deterministic recordings can exercise the same
/// mapping path without a physical tablet. Missing pose fields retain their contact value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawPenEvent {
    pub window: usize,
    pub pointer_id: u32,
    pub device_id: usize,
    pub phase: PenPhase,
    pub x: f32,
    pub y: f32,
    pub time_ms: u32,
    pub pressure: Option<f32>,
    pub tilt_x: Option<f32>,
    pub tilt_y: Option<f32>,
    pub rotation: Option<f32>,
    pub eraser: bool,
    pub barrel_button: bool,
}

#[derive(Default)]
struct State {
    events: VecDeque<PenEvent>,
    contacts: HashMap<(usize, u32), PenEvent>,
    pressure_support: HashMap<usize, Option<bool>>,
    errors: VecDeque<Error>,
}

/// The event-loop producer and viewport consumers share this queue. Poison recovery avoids
/// cascading failures. Overflow cancels all unfinished strokes instead of silently losing
/// samples or a release; each queued event and diagnostic has a fixed upper bound.
#[derive(Clone, Default)]
pub struct PenQueue(Arc<Mutex<State>>);

impl PenQueue {
    /// Record a native device capability query. A valid pressure packet later overrides an
    /// incorrect descriptor. The bounded cache does not grow with adversarial device IDs.
    pub fn set_pressure_support(&self, device: usize, supported: Option<bool>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.pressure_support.contains_key(&device) || state.pressure_support.len() < MAX_CONTACTS {
            state.pressure_support.insert(device, supported);
        }
    }

    pub fn push(&self, raw: RawPenEvent) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(raw);
    }

    pub fn drain(&self, window: usize) -> Vec<PenEvent> {
        self.drain_matching(|e| e.window == window)
    }

    /// Used to associate an egui viewport's first touch IDs with their native HWND.
    /// Once associated, use `drain(window)` to include cancellations without a touch event.
    pub fn drain_for_pointers(&self, ids: &[u64]) -> Vec<PenEvent> {
        self.drain_matching(|e| ids.contains(&u64::from(e.pointer_id)))
    }

    fn drain_matching(&self, matching: impl Fn(&PenEvent) -> bool) -> Vec<PenEvent> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut result = Vec::new();
        state.events.retain(|e| {
            if matching(e) {
                result.push(*e);
                false
            } else {
                true
            }
        });
        result
    }

    pub fn pending_windows(&self) -> Vec<usize> {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut windows = Vec::new();
        for event in &state.events {
            if !windows.contains(&event.window) {
                windows.push(event.window);
            }
        }
        windows
    }

    /// Cancel this window's unfinished contacts (focus loss, capture loss, or destruction).
    pub fn cancel_window(&self, window: usize) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let contacts: Vec<_> = state.contacts.values().filter(|e| e.window == window).copied().collect();
        for event in contacts {
            state.contacts.remove(&(event.window, event.pointer_id));
            state.enqueue(PenEvent { phase: PenPhase::Cancel, ..event });
        }
    }

    pub fn cancel_pointer(&self, window: usize, pointer: u32) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(event) = state.contacts.remove(&(window, pointer)) {
            state.enqueue(PenEvent { phase: PenPhase::Cancel, ..event });
        }
    }

    pub fn report_error(&self, error: Error) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).report_error(error);
    }

    pub fn take_errors(&self) -> Vec<Error> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).errors.drain(..).collect()
    }
}

impl State {
    fn report_error(&mut self, error: Error) {
        if self.errors.back() == Some(&error) {
            return;
        }
        if self.errors.len() >= MAX_ERRORS {
            self.errors.pop_front();
        }
        self.errors.push_back(error);
    }

    fn enqueue(&mut self, event: PenEvent) {
        if self.events.len() >= MAX_EVENTS {
            self.events.clear();
            // Preserve a cancel even when the overflowing event already removed its contact.
            let mut canceled = event;
            canceled.phase = PenPhase::Cancel;
            self.events.push_back(canceled);
            for (_, contact) in self.contacts.drain() {
                if (contact.window, contact.pointer_id) != (event.window, event.pointer_id) {
                    self.events.push_back(PenEvent { phase: PenPhase::Cancel, ..contact });
                }
            }
            self.report_error(Error::Platform("Windows Ink sample queue overflow; unfinished strokes canceled".into()));
            return;
        }
        self.events.push_back(event);
    }

    fn push(&mut self, raw: RawPenEvent) {
        if !raw.x.is_finite() || !raw.y.is_finite() {
            self.report_error(Error::Platform("Windows Ink rejected a non-finite position".into()));
            if let Some(last) = self.contacts.remove(&(raw.window, raw.pointer_id)) {
                self.enqueue(PenEvent { phase: PenPhase::Cancel, ..last });
            }
            return;
        }
        let key = (raw.window, raw.pointer_id);
        let previous = self.contacts.get(&key).copied();
        let previous = previous.filter(|last| last.device_id == raw.device_id && raw.phase != PenPhase::Down);
        if let Some(old) = self.contacts.get(&key).copied()
            && previous.is_none()
        {
            self.contacts.remove(&key);
            self.enqueue(PenEvent { phase: PenPhase::Cancel, ..old });
        }
        // An update after cancellation/overflow must not resurrect a partial stroke.
        if previous.is_none() && matches!(raw.phase, PenPhase::Move | PenPhase::Up | PenPhase::Cancel) {
            return;
        }
        if raw.phase == PenPhase::Down && self.contacts.len() >= MAX_CONTACTS {
            self.report_error(Error::Platform("Windows Ink contact limit reached".into()));
            return;
        }
        let time_ms = if let Some(last) = previous {
            let delta = raw.time_ms.wrapping_sub(last.time_ms as u32);
            // History is delivered oldest first. Reject stale history without confusing wrap.
            if delta > i32::MAX as u32 {
                return;
            }
            last.time_ms.saturating_add(u64::from(delta))
        } else {
            u64::from(raw.time_ms)
        };
        let valid_pressure = raw.pressure.filter(|p| p.is_finite() && (0.0..=1.0).contains(p));
        let pressure_supported = if valid_pressure.is_some() {
            if self.pressure_support.contains_key(&raw.device_id) || self.pressure_support.len() < MAX_CONTACTS {
                self.pressure_support.insert(raw.device_id, Some(true));
            }
            Some(true)
        } else {
            self.pressure_support.get(&raw.device_id).copied().flatten()
        };
        let previous_pressure = previous.and_then(|e| e.pressure);
        let ending = matches!(raw.phase, PenPhase::Up | PenPhase::Cancel);
        let pressure = if ending { previous_pressure } else { valid_pressure.or(previous_pressure) };
        let pose = |value: Option<f32>, fallback: f32| value.filter(|v| v.is_finite()).unwrap_or(fallback).clamp(-90.0, 90.0);
        let rotation = raw.rotation.filter(|v| v.is_finite()).unwrap_or_else(|| previous.map_or(0.0, |e| e.rotation)).rem_euclid(360.0);
        let event = PenEvent {
            window: raw.window,
            pointer_id: raw.pointer_id,
            device_id: raw.device_id,
            phase: raw.phase,
            x: raw.x,
            y: raw.y,
            time_ms,
            pressure,
            pressure_valid: valid_pressure.is_some(),
            pressure_supported,
            tilt_x: pose(raw.tilt_x, previous.map_or(0.0, |e| e.tilt_x)),
            tilt_y: pose(raw.tilt_y, previous.map_or(0.0, |e| e.tilt_y)),
            rotation: if rotation >= 360.0 { 0.0 } else { rotation },
            eraser: raw.eraser,
            barrel_button: raw.barrel_button,
        };
        if previous == Some(event) {
            return;
        }
        // A device can leave contact without a usable Up (capture changes, malformed driver
        // stream). Tell the consumer to drop its preview before forgetting the contact;
        // merely removing it here would leave the UI waiting forever for an ignored Up.
        if event.phase == PenPhase::Hover
            && let Some(last) = previous
        {
            self.contacts.remove(&key);
            self.enqueue(PenEvent { phase: PenPhase::Cancel, ..last });
        }
        match event.phase {
            PenPhase::Down | PenPhase::Move => {
                self.contacts.insert(key, event);
            }
            PenPhase::Up | PenPhase::Cancel | PenPhase::Hover => {
                self.contacts.remove(&key);
            }
        }
        self.enqueue(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(phase: PenPhase, pressure: Option<f32>) -> RawPenEvent {
        RawPenEvent {
            window: 10,
            pointer_id: 1,
            device_id: 20,
            phase,
            x: 12.0,
            y: 34.0,
            time_ms: 10,
            pressure,
            tilt_x: None,
            tilt_y: None,
            rotation: None,
            eraser: false,
            barrel_button: false,
        }
    }

    #[test]
    fn pressure_is_paired_and_stationary_updates_are_not_lost() {
        let q = PenQueue::default();
        q.push(raw(PenPhase::Down, Some(0.02)));
        q.push(RawPenEvent { x: 13.0, time_ms: 11, ..raw(PenPhase::Move, Some(0.1)) });
        q.push(RawPenEvent { x: 13.0, time_ms: 12, ..raw(PenPhase::Move, Some(0.8)) });
        q.push(RawPenEvent { time_ms: 13, ..raw(PenPhase::Up, Some(0.0)) });
        let events = q.drain(10);
        assert_eq!(events.iter().map(|e| e.pressure).collect::<Vec<_>>(), vec![Some(0.02), Some(0.1), Some(0.8), Some(0.8)]);
        assert_eq!(events.iter().map(|e| e.phase).collect::<Vec<_>>(), vec![PenPhase::Down, PenPhase::Move, PenPhase::Move, PenPhase::Up]);
    }

    #[test]
    fn out_of_range_pressure_is_missing_not_maximum() {
        let q = PenQueue::default();
        q.push(raw(PenPhase::Down, Some(2.0)));
        q.push(RawPenEvent { time_ms: 11, ..raw(PenPhase::Move, Some(0.2)) });
        q.push(RawPenEvent { time_ms: 12, ..raw(PenPhase::Move, Some(-0.5)) });
        let events = q.drain(10);
        assert_eq!(events.iter().map(|e| e.pressure).collect::<Vec<_>>(), [None, Some(0.2), Some(0.2)]);
        assert_eq!(events.iter().map(|e| e.pressure_valid).collect::<Vec<_>>(), [false, true, false]);
    }

    #[test]
    fn missing_pressure_never_defaults_to_full_or_previous_contact() {
        let q = PenQueue::default();
        q.push(raw(PenPhase::Down, None));
        q.push(raw(PenPhase::Move, Some(0.2)));
        q.push(raw(PenPhase::Move, Some(f32::NAN)));
        q.push(raw(PenPhase::Up, None));
        q.push(raw(PenPhase::Down, None));
        let events = q.drain(10);
        assert_eq!(events.iter().map(|e| e.pressure).collect::<Vec<_>>(), vec![None, Some(0.2), Some(0.2), Some(0.2), None]);
        assert!(!events[2].pressure_valid);
    }

    #[test]
    fn timestamp_wrap_and_stale_history() {
        let q = PenQueue::default();
        q.push(RawPenEvent { time_ms: u32::MAX - 1, ..raw(PenPhase::Down, Some(0.2)) });
        q.push(RawPenEvent { time_ms: 1, ..raw(PenPhase::Move, Some(0.3)) });
        q.push(RawPenEvent { time_ms: u32::MAX, ..raw(PenPhase::Move, Some(0.4)) });
        let events = q.drain(10);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].time_ms, u64::from(u32::MAX) + 2);
    }

    #[test]
    fn window_pointer_routing_preserves_other_windows() {
        let q = PenQueue::default();
        q.push(raw(PenPhase::Down, Some(0.2)));
        q.push(RawPenEvent { window: 11, pointer_id: 2, ..raw(PenPhase::Down, Some(0.3)) });
        assert_eq!(q.drain_for_pointers(&[2]).len(), 1);
        assert_eq!(q.pending_windows(), vec![10]);
        q.cancel_window(10);
        let events = q.drain(10);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].phase, PenPhase::Cancel);
    }

    #[test]
    fn overflow_is_bounded_and_cancels_instead_of_committing_a_partial_stroke() {
        let q = PenQueue::default();
        q.push(raw(PenPhase::Down, Some(0.2)));
        for time_ms in 11..=5000 {
            q.push(RawPenEvent { time_ms, ..raw(PenPhase::Move, Some(0.3)) });
        }
        let events = q.drain(10);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].phase, PenPhase::Cancel);
        assert_eq!(q.take_errors().len(), 1);
    }

    #[test]
    fn invalid_positions_cancel_and_new_device_does_not_inherit_pressure() {
        let q = PenQueue::default();
        q.push(raw(PenPhase::Down, Some(0.2)));
        q.push(RawPenEvent { x: f32::INFINITY, ..raw(PenPhase::Move, Some(0.4)) });
        q.push(RawPenEvent { device_id: 21, ..raw(PenPhase::Down, None) });
        let events = q.drain(10);
        assert_eq!(events[1].phase, PenPhase::Cancel);
        assert_eq!(events[2].pressure, None);
        assert_eq!(q.take_errors().len(), 1);
    }

    #[test]
    fn frame_batching_does_not_change_mapped_values() {
        let whole = PenQueue::default();
        let split = PenQueue::default();
        let input = [raw(PenPhase::Down, Some(0.01)), raw(PenPhase::Move, Some(0.2)), raw(PenPhase::Move, Some(0.05)), raw(PenPhase::Up, None)];
        let mut split_events = Vec::new();
        for event in input {
            whole.push(event);
            split.push(event);
            split_events.extend(split.drain(10));
        }
        assert_eq!(whole.drain(10), split_events);
    }

    #[test]
    fn unsupported_pressure_is_explicit_and_valid_packets_override_bad_descriptors() {
        let q = PenQueue::default();
        q.set_pressure_support(20, Some(false));
        q.push(raw(PenPhase::Down, None));
        q.push(raw(PenPhase::Move, Some(0.05)));
        q.push(raw(PenPhase::Move, None));
        let events = q.drain(10);
        assert_eq!(events[0].pressure_supported, Some(false));
        assert_eq!(events[0].pressure, None);
        assert_eq!(events[1].pressure_supported, Some(true));
        assert_eq!(events[2].pressure_supported, Some(true));
        assert_eq!(events[2].pressure, Some(0.05));
    }

    #[test]
    fn hover_without_release_cancels_active_contact_before_forgetting_it() {
        let q = PenQueue::default();
        q.push(raw(PenPhase::Down, Some(0.2)));
        q.push(RawPenEvent { time_ms: 11, ..raw(PenPhase::Move, Some(0.7)) });
        q.push(RawPenEvent { time_ms: 12, ..raw(PenPhase::Hover, Some(0.0)) });
        // A late release cannot commit the canceled stroke or transfer its pressure.
        q.push(RawPenEvent { time_ms: 13, ..raw(PenPhase::Up, None) });
        q.push(RawPenEvent { time_ms: 14, ..raw(PenPhase::Down, None) });
        let events = q.drain(10);
        assert_eq!(events.iter().map(|e| e.phase).collect::<Vec<_>>(), vec![PenPhase::Down, PenPhase::Move, PenPhase::Cancel, PenPhase::Hover, PenPhase::Down]);
        assert_eq!(events[2].pressure, Some(0.7));
        assert_eq!(events[4].pressure, None);
    }
}
