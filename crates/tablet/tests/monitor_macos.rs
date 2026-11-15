//! The AppKit monitor end to end, on the main thread (`harness = false`): install it, dispatch
//! real tablet `NSEvent`s through `-[NSApplication sendEvent:]` (where AppKit runs local
//! monitors), and check the samples the callback gets; dropping the monitor removes it.

#[cfg(target_os = "macos")]
fn main() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent};
    use objc2_core_graphics::{CGEvent, CGEventField, CGEventMouseSubtype, CGEventType, CGMouseButton};
    use objc2_foundation::NSPoint;
    use prism_tablet::Sample;
    use prism_tablet::macos::Monitor;

    let mtm = MainThreadMarker::new().expect("harness = false runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let event = |kind: CGEventType, pressure: f64, tilt: (f64, f64), subtype: CGEventMouseSubtype| {
        let cg = CGEvent::new_mouse_event(None, kind, NSPoint::new(5.0, 5.0), CGMouseButton::Left).expect("CGEvent");
        let set = |f, v| CGEvent::set_double_value_field(Some(&cg), f, v);
        CGEvent::set_integer_value_field(Some(&cg), CGEventField::MouseEventSubtype, i64::from(subtype.0));
        set(CGEventField::MouseEventPressure, pressure);
        set(CGEventField::TabletEventPointPressure, pressure);
        set(CGEventField::TabletEventTiltX, tilt.0);
        set(CGEventField::TabletEventTiltY, tilt.1);
        NSEvent::eventWithCGEvent(&cg).expect("NSEvent")
    };

    let seen: Rc<RefCell<Vec<Option<Sample>>>> = Rc::default();
    let monitor = {
        let seen = seen.clone();
        Monitor::install(move |s| seen.borrow_mut().push(s)).expect("install on the main thread")
    };
    app.sendEvent(&event(CGEventType::LeftMouseDragged, 0.5, (0.5, 0.0), CGEventMouseSubtype::TabletPoint));
    app.sendEvent(&event(CGEventType::LeftMouseDragged, 1.0, (0.0, 0.0), CGEventMouseSubtype::Default));
    {
        let seen = seen.borrow();
        assert_eq!(seen.len(), 2, "{seen:?}");
        let s = seen[0].expect("pen sample");
        assert!((s.pressure - 0.5).abs() < 0.01 && (s.tilt_x - 30.0).abs() < 0.1, "{s:?}");
        assert_eq!(seen[1], None, "a mouse event resets to mouse");
    }
    drop(monitor);
    app.sendEvent(&event(CGEventType::LeftMouseDragged, 0.5, (0.0, 0.0), CGEventMouseSubtype::TabletPoint));
    assert_eq!(seen.borrow().len(), 2, "the monitor is gone after drop");

    // With a packet queue: every left-button tablet event becomes a paired packet, and mouse
    // coalescing is off while installed and restored afterwards.
    use prism_tablet::windows_ink::{PenPhase, PenQueue};
    let before = NSEvent::isMouseCoalescingEnabled();
    let queue = PenQueue::default();
    let monitor = Monitor::install_with_queue(|_| {}, queue.clone()).expect("install with queue");
    assert!(!NSEvent::isMouseCoalescingEnabled(), "coalescing off while the monitor runs");
    let tablet = CGEventMouseSubtype::TabletPoint;
    app.sendEvent(&event(CGEventType::LeftMouseDown, 0.2, (0.0, 0.0), tablet));
    app.sendEvent(&event(CGEventType::LeftMouseDragged, 0.6, (0.0, 0.0), tablet));
    app.sendEvent(&event(CGEventType::LeftMouseDragged, 0.8, (0.0, 0.0), tablet));
    app.sendEvent(&event(CGEventType::LeftMouseUp, 0.0, (0.0, 0.0), tablet));
    let windows = queue.pending_windows();
    let packets: Vec<_> = windows.iter().flat_map(|w| queue.drain(*w)).collect();
    let phases: Vec<_> = packets.iter().map(|p| p.phase).collect();
    assert_eq!(phases, [PenPhase::Down, PenPhase::Move, PenPhase::Move, PenPhase::Up], "{packets:?}");
    let pressures: Vec<_> = packets.iter().filter_map(|p| p.pressure).map(|p| (p * 100.0).round() / 100.0).collect();
    assert_eq!(pressures, [0.2, 0.6, 0.8, 0.8]);
    drop(monitor);
    assert_eq!(NSEvent::isMouseCoalescingEnabled(), before, "restored on drop");
    println!("monitor_macos: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
