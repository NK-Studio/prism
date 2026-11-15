//! AppKit local event monitor for tablet data, plus the synthetic pen used to test it. The
//! macOS module of the crate's `unsafe` boundary: three calls into AppKit's Objective-C API that
//! objc2 can't prove sound on its own, and two typed views of the Quartz window list (each has a
//! `SAFETY:` comment). Everything the monitor reads goes through the pure, tested mapping in
//! [`crate::appkit`]; the synthetic pen's event encoding is the pure
//! [`crate::synthetic_pen::QuartzPen`].
//!
//! A *local* monitor sees this app's events only, on the main thread, inside `-[NSApplication
//! sendEvent:]` before the event reaches winit's view, so the sample is current when the UI
//! handles the matching pointer event. The monitor passes every event through unchanged.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask};

use crate::appkit::{RawEvent, State};
use crate::windows_ink::PenQueue;
use crate::{Error, Sample, Update, deliver};

/// Keeps the monitor installed; dropping it removes the monitor. Main-thread only (not `Send`).
pub struct Monitor {
    token: Retained<AnyObject>,
    _handler: RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent>,
    /// AppKit's mouse coalescing before [`Monitor::install_with_queue`] turned it off.
    coalescing: Option<bool>,
}

impl Monitor {
    /// Install the monitor. `callback` gets `Some(sample)` for pen events and `None` when a mouse
    /// moves; it runs on the main thread. Call on the main thread (before or after the event loop
    /// starts).
    pub fn install(callback: impl Fn(Option<Sample>) + 'static) -> Result<Self, Error> {
        Self::install_inner(callback, None)
    }

    /// [`Monitor::install`], and also push every pen reading as a paired packet (window,
    /// position, time, pressure, pose) into `queue`, the transport Windows Ink uses. Turns
    /// AppKit's mouse coalescing off while installed: coalescing merges the drags that arrive
    /// between two frames into one, dropping the pen readings in between (2026-11-16: about 40 %
    /// at 125 Hz input and 60 fps); dropping the monitor restores it.
    pub fn install_with_queue(callback: impl Fn(Option<Sample>) + 'static, queue: PenQueue) -> Result<Self, Error> {
        Self::install_inner(callback, Some(queue))
    }

    fn install_inner(callback: impl Fn(Option<Sample>) + 'static, queue: Option<PenQueue>) -> Result<Self, Error> {
        if MainThreadMarker::new().is_none() {
            return Err(Error::NotMainThread);
        }
        let coalescing = queue.is_some().then(|| {
            let previous = NSEvent::isMouseCoalescingEnabled();
            NSEvent::setMouseCoalescingEnabled(false);
            previous
        });
        let state = RefCell::new(State::default());
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit calls a local monitor's handler with the event it is about to
            // dispatch, a valid `NSEvent` that it keeps alive for the duration of the call; we
            // only borrow it within the call.
            let e: &NSEvent = unsafe { event.as_ref() };
            let raw = read(e);
            // The handler can't re-enter itself (AppKit runs it synchronously on the main
            // thread), but `try_borrow_mut` keeps a re-entrant call from panicking anyway.
            if let Ok(mut st) = state.try_borrow_mut() {
                if let Update::Set(sample) = st.handle(&raw) {
                    deliver(&callback, sample);
                }
                if let Some(queue) = &queue
                    && let Some(packet) = st.packet(&raw)
                {
                    queue.push(packet);
                }
            }
            // Pass the event on unchanged.
            event.as_ptr()
        });
        let mask = NSEventMask(crate::appkit::event_mask());
        // SAFETY: the handler returns the (non-null, valid) event it was given, as the API
        // requires ("block's return must be a valid pointer or null"). We keep the block alive
        // in `Monitor` as well, although AppKit copies it.
        let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) };
        let token = token.ok_or_else(|| Error::Platform("addLocalMonitorForEventsMatchingMask returned nil".into()))?;
        Ok(Self { token, _handler: handler, coalescing })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: `token` is exactly the object `addLocalMonitorForEventsMatchingMask:handler:`
        // returned, removed once (here). `Monitor` is not `Send`, so this runs on the main thread.
        unsafe { NSEvent::removeMonitor(&self.token) };
        if let Some(previous) = self.coalescing {
            NSEvent::setMouseCoalescingEnabled(previous);
        }
    }
}

/// Read the tablet fields of an event. Every getter is safe for every event type: AppKit
/// returns 0 / zero points for fields an event doesn't carry (`subtype` and the proximity fields
/// are only read for the event types that define them, see [`crate::appkit`]).
fn read(e: &NSEvent) -> RawEvent {
    let kind = e.r#type().0;
    let tabletish = matches!(kind, crate::appkit::event_type::TABLET_POINT | crate::appkit::event_type::TABLET_PROXIMITY);
    let mouse = !tabletish && is_mouse(kind);
    // `subtype` raises an exception for event types without one; ask only mouse events.
    let subtype = if mouse { e.subtype().0 } else { 0 };
    let proximity = kind == crate::appkit::event_type::TABLET_PROXIMITY || (mouse && subtype == crate::appkit::subtype::TABLET_PROXIMITY);
    let point = kind == crate::appkit::event_type::TABLET_POINT || (mouse && subtype == crate::appkit::subtype::TABLET_POINT);
    let (pressure, tilt, rotation) = if point {
        let t = e.tilt();
        (e.pressure(), (t.x, t.y), e.rotation())
    } else {
        (0.0, (0.0, 0.0), 0.0)
    };
    let (device, entering) = if proximity { (e.pointingDeviceType().0, e.isEnteringProximity()) } else { (0, false) };
    // Where and when, for the paired packets: window-content points with a top-left origin, as
    // winit reports the matching cursor position (an event without a window, like one built in
    // a test, keeps its screen location).
    let (window, x, y) = if point || proximity {
        let at = e.locationInWindow();
        let height = MainThreadMarker::new().and_then(|mtm| e.window(mtm)).and_then(|w| w.contentView()).map(|v| v.frame().size.height);
        (usize::try_from(e.windowNumber()).unwrap_or(0), at.x, height.map_or(at.y, |h| h - at.y))
    } else {
        (0, 0.0, 0.0)
    };
    let seconds = e.timestamp();
    let time_ms = if seconds.is_finite() && seconds >= 0.0 { (seconds * 1000.0) as u64 as u32 } else { 0 };
    RawEvent { kind, subtype, pressure, tilt, rotation, device, entering, window, x, y, time_ms }
}

fn is_mouse(kind: usize) -> bool {
    use crate::appkit::event_type::*;
    matches!(
        kind,
        LEFT_MOUSE_DOWN
            | LEFT_MOUSE_UP
            | RIGHT_MOUSE_DOWN
            | RIGHT_MOUSE_UP
            | MOUSE_MOVED
            | LEFT_MOUSE_DRAGGED
            | RIGHT_MOUSE_DRAGGED
            | OTHER_MOUSE_DOWN
            | OTHER_MOUSE_UP
            | OTHER_MOUSE_DRAGGED
    )
}

// ---------------------------------------------------------------------------------------------
// Synthetic pen injection (test tooling): drives the real AppKit tablet path without a tablet.

use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventMouseSubtype, CGEventTapLocation, CGEventType, CGMouseButton, CGPreflightPostEventAccess, CGRequestPostEventAccess,
    CGWindowListCopyWindowInfo, CGWindowListOption,
};
use objc2_foundation::NSPoint;

use crate::synthetic_pen::{PenFrame, QuartzEvent, QuartzPen, Stroke};

/// The id the synthetic pen reports in its proximity and point events. Arbitrary but constant:
/// AppKit and applications pair a tablet's point events with its proximity events by it.
const DEVICE_ID: i64 = 0x5052;
/// `NX_TABLET_CAPABILITY_*` bits (IOLLEvent.h): device id, absolute x/y, buttons, tilt x/y,
/// pressure and rotation.
const CAPABILITIES: i64 = 0x0001 | 0x0002 | 0x0004 | 0x0040 | 0x0080 | 0x0100 | 0x0400 | 0x2000;
/// `NX_TABLET_POINTER_PEN` / `NX_TABLET_POINTER_ERASER`.
const POINTER_PEN: i64 = 1;
const POINTER_ERASER: i64 = 3;

/// A synthetic pen that posts the Quartz events of a tablet driver at the HID level
/// (`CGEventPost(kCGHIDEventTap)`): a proximity event, then mouse events with the tablet-point
/// subtype carrying pressure, tilt and rotation. They move the real cursor and reach whichever
/// window is under it through the window server and AppKit, exactly like a physical pen:
/// Prism's tablet monitor, or another application used as a comparison oracle.
///
/// Posting needs Accessibility access for the process that runs it (macOS attributes it to the
/// app that launched the tool: Terminal, an IDE, the Claude app). Dropping the pen lifts a tip
/// left in contact and takes the pen out of range.
pub struct SyntheticPen {
    encoder: QuartzPen,
    last: Option<PenFrame>,
}

impl SyntheticPen {
    /// Check that this process may post events. Without access it asks macOS to show the
    /// Accessibility prompt once and returns an error saying where to allow it.
    pub fn new() -> Result<Self, Error> {
        if !CGPreflightPostEventAccess() {
            let _ = CGRequestPostEventAccess();
            return Err(Error::Unsupported(
                "posting input events needs Accessibility access: allow the app that runs this tool (Terminal, your IDE, the Claude app) in \
                 System Settings > Privacy & Security > Accessibility, then run it again"
                    .into(),
            ));
        }
        Ok(Self { encoder: QuartzPen::default(), last: None })
    }

    /// Post one frame now (the caller paces frames with `delay_ms`).
    pub fn inject(&mut self, frame: &PenFrame) -> Result<(), Error> {
        let at = NSPoint::new(frame.x, frame.y);
        for event in self.encoder.events(frame) {
            post(event, at)?;
        }
        self.last = Some(*frame);
        Ok(())
    }

    /// Post a whole stroke, sleeping each frame's `delay_ms` first.
    pub fn play(&mut self, stroke: &Stroke) -> Result<(), Error> {
        for frame in stroke.frames() {
            if frame.delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(u64::from(frame.delay_ms)));
            }
            self.inject(frame)?;
        }
        Ok(())
    }
}

impl Drop for SyntheticPen {
    fn drop(&mut self) {
        let last = self.last.take();
        let at = last.map_or(NSPoint::new(0.0, 0.0), |f| NSPoint::new(f.x, f.y));
        for event in self.encoder.release(last.as_ref()) {
            let _ = post(event, at);
        }
    }
}

/// Post one encoded event at the HID level.
fn post(event: QuartzEvent, at: NSPoint) -> Result<(), Error> {
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&*quartz_event(event, at)?));
    Ok(())
}

/// The `CGEvent` for one encoded event at `at` (global display points).
fn quartz_event(event: QuartzEvent, at: NSPoint) -> Result<CFRetained<CGEvent>, Error> {
    Ok(match event {
        QuartzEvent::Proximity { entering, eraser } => {
            let cg = CGEvent::new(None).ok_or_else(|| Error::Platform("CGEventCreate failed".into()))?;
            CGEvent::set_type(Some(&cg), CGEventType::TabletProximity);
            CGEvent::set_location(Some(&cg), at);
            let int = |field, value| CGEvent::set_integer_value_field(Some(&cg), field, value);
            int(CGEventField::TabletProximityEventDeviceID, DEVICE_ID);
            int(CGEventField::TabletProximityEventSystemTabletID, 1);
            int(CGEventField::TabletProximityEventTabletID, 1);
            int(CGEventField::TabletProximityEventPointerID, if eraser { 2 } else { 1 });
            int(CGEventField::TabletProximityEventVendorUniqueID, DEVICE_ID);
            int(CGEventField::TabletProximityEventCapabilityMask, CAPABILITIES);
            int(CGEventField::TabletProximityEventPointerType, if eraser { POINTER_ERASER } else { POINTER_PEN });
            int(CGEventField::TabletProximityEventEnterProximity, i64::from(entering));
            cg
        }
        QuartzEvent::TabletMouse { kind, pressure, tilt_x, tilt_y, rotation, buttons } => {
            let kind = u32::try_from(kind).map_err(|_| Error::Platform(format!("event type {kind} out of range")))?;
            let cg = CGEvent::new_mouse_event(None, CGEventType(kind), at, CGMouseButton::Left)
                .ok_or_else(|| Error::Platform("CGEventCreateMouseEvent failed".into()))?;
            let int = |field, value| CGEvent::set_integer_value_field(Some(&cg), field, value);
            let dbl = |field, value| CGEvent::set_double_value_field(Some(&cg), field, value);
            int(CGEventField::MouseEventSubtype, i64::from(CGEventMouseSubtype::TabletPoint.0));
            if kind != CGEventType::MouseMoved.0 {
                int(CGEventField::MouseEventClickState, 1);
            }
            dbl(CGEventField::MouseEventPressure, pressure);
            dbl(CGEventField::TabletEventPointPressure, pressure);
            dbl(CGEventField::TabletEventTiltX, tilt_x);
            dbl(CGEventField::TabletEventTiltY, tilt_y);
            dbl(CGEventField::TabletEventRotation, rotation);
            int(CGEventField::TabletEventDeviceID, DEVICE_ID);
            int(CGEventField::TabletEventPointButtons, buttons);
            // Absolute tablet coordinates: the screen position stands in for the tablet's.
            int(CGEventField::TabletEventPointX, at.x.round() as i64);
            int(CGEventField::TabletEventPointY, at.y.round() as i64);
            cg
        }
    })
}

/// An on-screen application window, with its frame in global display points (top-left origin).
/// Prism draws under its title bar (full-size content view), so its frame is its client area;
/// a window with a standard title bar has its content below the frame's top edge.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowTarget {
    pub window_id: u32,
    pub pid: i32,
    /// The owning application's name (always readable).
    pub owner: String,
    /// The window title; empty for other applications' windows unless this process has Screen
    /// Recording access.
    pub title: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// On-screen normal-level windows, frontmost first (at most 4096).
pub fn list_windows() -> Result<Vec<WindowTarget>, Error> {
    let options = CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let list = CGWindowListCopyWindowInfo(options, 0).ok_or_else(|| Error::Platform("CGWindowListCopyWindowInfo returned nil".into()))?;
    // SAFETY: CGWindowListCopyWindowInfo returns a CFArray of CFDictionary objects (CGWindow.h),
    // all CF objects. Viewing the elements as `CFType` only retains them and checks their dynamic
    // type (`downcast`) before any typed use.
    let list: CFRetained<CFArray<CFType>> = unsafe { CFRetained::cast_unchecked(list) };
    let key = |name: &'static str| CFString::from_static_str(name);
    let mut found = Vec::new();
    for item in list.iter().take(4096) {
        let Some(info) = string_dictionary(item) else { continue };
        let number = |d: &CFDictionary<CFString, CFType>, name| d.get(&key(name)).and_then(|v| v.downcast::<CFNumber>().ok());
        let text = |name| info.get(&key(name)).and_then(|v| v.downcast::<CFString>().ok()).map(|s| s.to_string()).unwrap_or_default();
        if number(&info, "kCGWindowLayer").and_then(|n| n.as_i64()) != Some(0) {
            continue;
        }
        let Some(bounds) = info.get(&key("kCGWindowBounds")).and_then(string_dictionary) else { continue };
        let coord = |name| number(&bounds, name).and_then(|n| n.as_f64()).filter(|v| v.is_finite());
        let (Some(x), Some(y), Some(width), Some(height)) = (coord("X"), coord("Y"), coord("Width"), coord("Height")) else { continue };
        found.push(WindowTarget {
            window_id: number(&info, "kCGWindowNumber").and_then(|n| n.as_i64()).and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
            pid: number(&info, "kCGWindowOwnerPID").and_then(|n| n.as_i64()).and_then(|n| i32::try_from(n).ok()).unwrap_or(0),
            owner: text("kCGWindowOwnerName"),
            title: text("kCGWindowName"),
            x,
            y,
            width,
            height,
        });
    }
    Ok(found)
}

/// A window-list dictionary viewed with `CFString` keys, if `value` is a dictionary at all.
fn string_dictionary(value: CFRetained<CFType>) -> Option<CFRetained<CFDictionary<CFString, CFType>>> {
    let dictionary = value.downcast::<CFDictionary>().ok()?;
    // SAFETY: window-list dictionaries (and their bounds dictionaries) map CFString keys to CF
    // values (CGWindow.h). The typed view only hashes/compares keys and retains values through
    // CF, and every value is type-checked (`downcast`) before use; a key of another type simply
    // never equals a CFString.
    Some(unsafe { CFRetained::cast_unchecked(dictionary) })
}

/// The frontmost window titled `needle` (case-insensitive), else the first whose title contains
/// it, else the same by application name (other applications' titles are hidden without Screen
/// Recording access). `pid` restricts the search to one process, to tell apart two instances of
/// the same application.
pub fn find_window(needle: &str, pid: Option<i32>) -> Result<Option<WindowTarget>, Error> {
    let needle = needle.to_lowercase();
    let windows: Vec<WindowTarget> = list_windows()?.into_iter().filter(|w| pid.is_none_or(|p| w.pid == p)).collect();
    let size = |w: &WindowTarget| (w.width.round() as i32, w.height.round() as i32);
    let pick = crate::synthetic_pen::pick_window(windows.iter().map(|w| (w.title.as_str(), size(w).0, size(w).1)), &needle)
        .or_else(|| crate::synthetic_pen::pick_window(windows.iter().map(|w| (w.owner.as_str(), size(w).0, size(w).1)), &needle));
    Ok(pick.and_then(|i| windows.into_iter().nth(i)))
}

/// Ask macOS to activate the window's application (all its windows come forward). macOS may
/// decline; a pen press on a background Prism window still reaches it (first mouse).
pub fn bring_to_front(target: &WindowTarget) -> bool {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    NSRunningApplication::runningApplicationWithProcessIdentifier(target.pid)
        .is_some_and(|app| app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSPoint;

    #[test]
    fn install_off_the_main_thread_is_an_error() {
        // `cargo test` runs tests on worker threads, never on the main thread.
        let r = std::thread::spawn(|| Monitor::install(|_| {}).err()).join().unwrap();
        assert_eq!(r, Some(Error::NotMainThread));
    }

    /// A real `NSEvent` built from a Quartz event, as the tablet driver posts them.
    fn event(kind: CGEventType, set: impl Fn(&CGEvent)) -> objc2::rc::Retained<NSEvent> {
        // Quartz builds mouse events only; other types start blank and get their type set.
        let cg = CGEvent::new_mouse_event(None, kind, NSPoint::new(10.0, 10.0), CGMouseButton::Left)
            .or_else(|| {
                let e = CGEvent::new(None)?;
                CGEvent::set_type(Some(&e), kind);
                Some(e)
            })
            .expect("CGEvent");
        set(&cg);
        NSEvent::eventWithCGEvent(&cg).expect("NSEvent")
    }

    fn int(e: &CGEvent, f: CGEventField, v: i64) {
        CGEvent::set_integer_value_field(Some(e), f, v);
    }

    fn dbl(e: &CGEvent, f: CGEventField, v: f64) {
        CGEvent::set_double_value_field(Some(e), f, v);
    }

    /// Tablet mouse events read back through the same path as live ones.
    #[test]
    fn tablet_events_read_through_the_monitor_path() {
        let mut st = State::default();
        let drag = event(CGEventType::LeftMouseDragged, |e| {
            int(e, CGEventField::MouseEventSubtype, i64::from(CGEventMouseSubtype::TabletPoint.0));
            dbl(e, CGEventField::MouseEventPressure, 0.25);
            dbl(e, CGEventField::TabletEventPointPressure, 0.25);
            dbl(e, CGEventField::TabletEventTiltX, 0.5);
            dbl(e, CGEventField::TabletEventTiltY, -0.5);
            dbl(e, CGEventField::TabletEventRotation, 45.0);
        });
        let raw = read(&drag);
        assert_eq!((raw.kind, raw.subtype), (6, 1));
        let Update::Set(Some(s)) = st.handle(&raw) else { panic!("{raw:?}") };
        assert!((s.pressure - 0.25).abs() < 0.01, "{s:?}"); // Quartz keeps 8 bits of mouse pressure.
        assert!((s.tilt_x - 30.0).abs() < 0.1 && (s.tilt_y - 30.0).abs() < 0.1, "{s:?}");
        assert!((s.rotation - 45.0).abs() < 0.1, "{s:?}");
        assert!(!s.eraser);

        // Eraser end entering proximity, then a press with it.
        let prox = event(CGEventType::TabletProximity, |e| {
            int(e, CGEventField::TabletProximityEventPointerType, crate::appkit::device::ERASER as i64);
            int(e, CGEventField::TabletProximityEventEnterProximity, 1);
        });
        let raw = read(&prox);
        assert_eq!((raw.kind, raw.device, raw.entering), (24, 3, true));
        assert!(matches!(st.handle(&raw), Update::Set(Some(Sample { eraser: true, .. }))));

        // A plain mouse event (subtype 0) is a mouse, whatever its pressure.
        let mouse = event(CGEventType::LeftMouseDown, |e| dbl(e, CGEventField::MouseEventPressure, 0.7));
        let raw = read(&mouse);
        assert_eq!((raw.kind, raw.subtype), (1, 0));
        assert_eq!(st.handle(&raw), Update::Set(None));
    }
    /// The synthetic pen's Quartz events, as AppKit reads them back through the monitor's
    /// path: the fields it sets are the ones Prism decodes (built here, never posted).
    #[test]
    fn synthetic_pen_events_read_back_as_the_frame() {
        use crate::synthetic_pen::{PenFrame, Phase};
        let mut encoder = QuartzPen::default();
        let mut st = State::default();
        let mut frame = PenFrame::new(Phase::Down, 40.0, 30.0, 0.5, 0);
        (frame.tilt_x, frame.tilt_y, frame.rotation) = (30.0, 15.0, 90.0);
        let mut last = None;
        for event in encoder.events(&frame) {
            let ns = NSEvent::eventWithCGEvent(&quartz_event(event, NSPoint::new(frame.x, frame.y)).unwrap()).expect("NSEvent");
            last = Some(st.handle(&read(&ns)));
        }
        assert!(!st.eraser);
        let Some(Update::Set(Some(s))) = last else { panic!("{last:?}") };
        assert!((s.pressure - 0.5).abs() < 0.01, "{s:?}");
        assert!((s.tilt_x - 30.0).abs() < 0.1 && (s.tilt_y - 15.0).abs() < 0.1, "{s:?}");
        assert!((s.rotation - 90.0).abs() < 0.1, "{s:?}");
        // The eraser end, then leaving range: back to a mouse.
        let eraser = PenFrame { phase: Phase::Hover, eraser: true, pressure: 0.0, ..frame };
        let mut updates = Vec::new();
        for f in [eraser, PenFrame { phase: Phase::Leave, ..eraser }] {
            for event in encoder.events(&f) {
                let ns = NSEvent::eventWithCGEvent(&quartz_event(event, NSPoint::new(f.x, f.y)).unwrap()).expect("NSEvent");
                updates.push(st.handle(&read(&ns)));
            }
        }
        assert!(updates.iter().any(|u| matches!(u, Update::Set(Some(Sample { eraser: true, .. })))), "{updates:?}");
        assert_eq!(updates.last(), Some(&Update::Set(None)));
    }

    #[test]
    fn find_window_misses_cleanly() {
        assert_eq!(find_window("no window is titled like this 7f3a9c", None).ok().flatten(), None);
        assert_eq!(find_window("", Some(-1)).ok().flatten(), None, "no process has pid -1");
    }
}
