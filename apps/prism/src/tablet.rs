//! Native pen adapters. Windows reads paired pointer history before winit consumes it; macOS
//! reads each AppKit tablet event in a local monitor. Both feed the same paired-packet transport
//! (`windows_ink::PenQueue`) into the canvas's pen packets (`pen_input`).

#[cfg(any(target_os = "macos", test))]
use prism_tablet::Sample;
use prism_ui_egui::stylus::PenSample;
#[cfg(any(target_os = "macos", test))]
use prism_ui_egui::stylus::StylusFeed;

#[cfg(target_os = "windows")]
pub fn configure_windows(options: &mut eframe::NativeOptions) -> prism_tablet::windows_ink::PenQueue {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    let queue = prism_tablet::windows_ink::PenQueue::default();
    let events = queue.clone();
    let previous = options.event_loop_builder.take();
    options.event_loop_builder = Some(Box::new(move |builder| {
        if let Some(previous) = previous {
            previous(builder);
        }
        builder.with_msg_hook(prism_tablet::windows::message_hook(events));
    }));
    queue
}

#[cfg(target_os = "windows")]
pub fn connect_windows(ctx: &egui::Context, queue: prism_tablet::windows_ink::PenQueue) {
    let mut windows = std::collections::HashMap::new();
    prism_ui_egui::pen_input::install(
        ctx,
        Box::new(move |ctx, raw| {
            windows.retain(|id, _| raw.viewports.contains_key(id));
            let events = if let Some(window) = windows.get(&raw.viewport_id) {
                queue.drain(*window)
            } else {
                let ids: Vec<_> = raw
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::Touch { id, .. } => Some(id.0),
                        _ => None,
                    })
                    .collect();
                let events = queue.drain_for_pointers(&ids);
                if let Some(event) = events.first() {
                    windows.insert(raw.viewport_id, event.window);
                }
                events
            };
            for error in queue.take_errors() {
                log::warn!("pen input: {error}");
            }
            // Client physical pixels → egui points.
            let scale = raw.viewports.get(&raw.viewport_id).and_then(|v| v.native_pixels_per_point).unwrap_or(1.0) * ctx.zoom_factor();
            events.into_iter().map(|event| packet("windowsInk", event, scale)).collect()
        }),
    );
}

/// A transport reading as the canvas's pen packet; `scale` turns its position into egui points.
#[cfg(any(target_os = "windows", target_os = "macos", test))]
fn packet(source: &'static str, event: prism_tablet::windows_ink::PenEvent, scale: f32) -> prism_ui_egui::pen_input::Packet {
    use prism_tablet::windows_ink::PenPhase;
    use prism_ui_egui::pen_input::{Packet, Phase};
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    Packet {
        source,
        id: u64::from(event.pointer_id),
        device: event.device_id as u64,
        phase: match event.phase {
            PenPhase::Down => Phase::Down,
            PenPhase::Move => Phase::Move,
            PenPhase::Up => Phase::Up,
            PenPhase::Cancel => Phase::Cancel,
            PenPhase::Hover => Phase::Hover,
        },
        pos: egui::pos2(event.x / scale, event.y / scale),
        sample: PenSample {
            pressure: event.pressure.unwrap_or(if event.pressure_supported == Some(false) { 1.0 } else { 0.0 }),
            tilt_x: event.tilt_x,
            tilt_y: event.tilt_y,
            rotation: event.rotation,
            eraser: event.eraser,
        },
        pressure_valid: event.pressure_valid,
        pressure_supported: event.pressure_supported,
        time: event.time_ms as f64,
        barrel: event.barrel_button,
    }
}

/// Which native window a viewport's packets come from, for transports keyed by window (macOS
/// window numbers). egui does not expose a viewport's native window, so a viewport is bound to
/// the one unclaimed window with pending packets in the first frame that brings it pointer
/// events (one pen: only the window under it produces packets). Packets of a window no viewport
/// claims (a native dialog, a closed window) are dropped after a while instead of filling the
/// bounded queue.
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct WindowBinding {
    windows: std::collections::HashMap<egui::ViewportId, usize>,
    unclaimed: std::collections::HashMap<usize, u32>,
}

#[cfg(any(target_os = "macos", test))]
impl WindowBinding {
    /// Frames an unclaimed window's packets may wait for a viewport (about 4 s at 60 fps).
    const MAX_UNCLAIMED_FRAMES: u32 = 240;

    /// The window to drain for `raw`'s viewport, given the windows with pending packets.
    /// Returns windows whose packets should be discarded as the second value.
    fn window_for(&mut self, raw: &egui::RawInput, pending: &[usize]) -> (Option<usize>, Vec<usize>) {
        self.windows.retain(|id, _| raw.viewports.contains_key(id));
        let claimed = |w: &usize, windows: &std::collections::HashMap<egui::ViewportId, usize>| windows.values().any(|v| v == w);
        let unclaimed: Vec<usize> = pending.iter().copied().filter(|w| !claimed(w, &self.windows)).collect();
        if !self.windows.contains_key(&raw.viewport_id)
            && let [only] = unclaimed.as_slice()
            && raw.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }))
        {
            self.windows.insert(raw.viewport_id, *only);
        }
        let mut stale = Vec::new();
        self.unclaimed.retain(|w, _| unclaimed.contains(w) && !claimed(w, &self.windows));
        for w in unclaimed.into_iter().filter(|w| !claimed(w, &self.windows)) {
            let age = self.unclaimed.entry(w).or_insert(0);
            *age = age.saturating_add(1);
            if *age > Self::MAX_UNCLAIMED_FRAMES {
                stale.push(w);
            }
        }
        for w in &stale {
            self.unclaimed.remove(w);
        }
        (self.windows.get(&raw.viewport_id).copied(), stale)
    }
}

/// Route the AppKit monitor's paired packets into each viewport's canvas.
#[cfg(target_os = "macos")]
fn connect_macos(ctx: &egui::Context, queue: prism_tablet::windows_ink::PenQueue) {
    let mut binding = WindowBinding::default();
    prism_ui_egui::pen_input::install(
        ctx,
        Box::new(move |ctx, raw| {
            let (window, stale) = binding.window_for(raw, &queue.pending_windows());
            for w in stale {
                queue.drain(w);
            }
            for error in queue.take_errors() {
                log::warn!("pen input: {error}");
            }
            let Some(window) = window else { return Vec::new() };
            // Window-content points → egui points.
            let scale = ctx.zoom_factor();
            queue.drain(window).into_iter().map(|event| packet("appKit", event, scale)).collect()
        }),
    );
}

/// A tablet sample as the UI's pen sample.
#[cfg(any(target_os = "macos", test))]
pub fn pen_sample(s: Sample) -> PenSample {
    PenSample { pressure: s.pressure, tilt_x: s.tilt_x, tilt_y: s.tilt_y, rotation: s.rotation, eraser: s.eraser }.sanitized()
}

/// Writes tablet samples into `feed` (`None` = a mouse).
#[cfg(any(target_os = "macos", test))]
pub fn sink(feed: StylusFeed) -> impl Fn(Option<Sample>) + Send + 'static {
    move |s| feed.set(s.map(pen_sample))
}

/// Install the AppKit tablet monitor (main thread, once winit created the shared application:
/// from eframe's app creation, not before the event loop) and route its paired packets to the
/// canvas. Keep the result until the event loop returns. On failure this logs why: pens then
/// paint like a mouse.
#[cfg(target_os = "macos")]
pub fn install_macos(ctx: &egui::Context, feed: &StylusFeed) -> Option<prism_tablet::macos::Monitor> {
    let queue = prism_tablet::windows_ink::PenQueue::default();
    let monitor = prism_tablet::macos::Monitor::install_with_queue(sink(feed.clone()), queue.clone()).map_err(|e| log::warn!("{e}")).ok()?;
    connect_macos(ctx, queue);
    Some(monitor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_tablet::{Update, appkit};
    use prism_ui_egui::PrismApp;
    use prism_ui_egui::canvas::{ToolEvent, tool_event};
    use prism_ui_egui::state::Tool;
    use serde_json::{Value, json};

    fn raw(viewport: u64, pointer: bool) -> egui::RawInput {
        let id = egui::ViewportId::from_hash_of(viewport);
        let mut raw = egui::RawInput { viewport_id: id, ..Default::default() };
        raw.viewports.insert(id, Default::default());
        raw.viewports.insert(egui::ViewportId::from_hash_of(2), Default::default());
        raw.viewports.insert(egui::ViewportId::from_hash_of(1), Default::default());
        if pointer {
            raw.events.push(egui::Event::PointerMoved(egui::pos2(1.0, 1.0)));
        }
        raw
    }

    /// Each viewport drains only the native window it was bound to; a window nobody claims
    /// (a native dialog) is dropped after a while instead of filling the queue.
    #[test]
    fn viewports_bind_to_the_window_their_pointer_events_come_from() {
        let mut b = WindowBinding::default();
        assert_eq!(b.window_for(&raw(1, false), &[40]), (None, vec![]), "no pointer events: no binding");
        assert_eq!(b.window_for(&raw(1, true), &[40]).0, Some(40));
        assert_eq!(b.window_for(&raw(1, false), &[]).0, Some(40), "stays bound");
        // A second viewport takes the other window, never the first one's.
        assert_eq!(b.window_for(&raw(2, true), &[40]).0, None);
        assert_eq!(b.window_for(&raw(2, true), &[40, 41]).0, Some(41));
        // Two unclaimed windows at once are ambiguous: wait.
        let mut b = WindowBinding::default();
        assert_eq!(b.window_for(&raw(1, true), &[40, 41]).0, None);
        // An unclaimed window's packets go stale.
        let mut b = WindowBinding::default();
        b.window_for(&raw(1, true), &[40]);
        let mut dropped = Vec::new();
        for _ in 0..=WindowBinding::MAX_UNCLAIMED_FRAMES {
            dropped.extend(b.window_for(&raw(1, false), &[40, 99]).1);
        }
        assert_eq!(dropped, vec![99]);
        // A viewport that closed releases its window.
        let mut gone = raw(2, true);
        gone.viewports.remove(&egui::ViewportId::from_hash_of(1));
        assert_eq!(b.window_for(&gone, &[40]).0, Some(40));
    }

    #[test]
    fn transport_readings_become_canvas_packets() {
        use prism_tablet::windows_ink::{PenEvent, PenPhase};
        let event = PenEvent {
            window: 3,
            pointer_id: 1,
            device_id: 1,
            phase: PenPhase::Move,
            x: 100.0,
            y: 50.0,
            time_ms: 1234,
            pressure: Some(0.4),
            pressure_valid: true,
            pressure_supported: Some(true),
            tilt_x: 10.0,
            tilt_y: -5.0,
            rotation: 30.0,
            eraser: false,
            barrel_button: false,
        };
        let p = packet("appKit", event, 2.0);
        assert_eq!((p.pos, p.sample.pressure, p.time, p.source), (egui::pos2(50.0, 25.0), 0.4, 1234.0, "appKit"));
        assert_eq!(packet("appKit", event, f32::NAN).pos, egui::pos2(100.0, 50.0), "a bad scale is 1");
        let none = PenEvent { pressure: None, pressure_supported: None, ..event };
        assert_eq!(packet("appKit", none, 1.0).sample.pressure, 0.0, "missing pressure is never full");
    }

    #[test]
    fn samples_map_and_sanitize() {
        let p = pen_sample(Sample { pressure: 0.25, tilt_x: 30.0, tilt_y: -10.0, rotation: 90.0, eraser: true });
        assert_eq!(p, PenSample { pressure: 0.25, tilt_x: 30.0, tilt_y: -10.0, rotation: 90.0, eraser: true });
        let p = pen_sample(Sample { pressure: f32::NAN, tilt_x: 500.0, tilt_y: f32::NEG_INFINITY, rotation: -45.0, eraser: false });
        assert_eq!(p, PenSample { pressure: 1.0, tilt_x: 90.0, tilt_y: 0.0, rotation: 315.0, eraser: false });
        let feed = StylusFeed::default();
        let f = sink(feed.clone());
        f(Some(Sample { pressure: 0.5, ..Default::default() }));
        assert_eq!(feed.get().map(|p| p.pressure), Some(0.5));
        f(None);
        assert_eq!(feed.get(), None);
    }

    /// The platform path end to end, minus the OS: raw AppKit values → the crate's
    /// mapping → this module's sink → the stylus feed → the canvas's tool events → `paint.stroke`.
    /// A light-to-heavy pen stroke must paint thin and faint at the light end and wide and opaque
    /// at the heavy end (Size and Opacity on Pen Pressure); a mouse paints full strength.
    fn pressure_stroke(mut next: impl FnMut(f32) -> Option<Sample>) -> (f64, f64, f64, f64) {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        let feed = app.stylus.feed.clone();
        let sink = sink(feed);
        app.session.execute("file.new", json!({"width": 240, "height": 80, "background": "white"})).unwrap();
        app.session.execute("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
        let brush = json!({"size": 30, "hardness": 1.0, "spacing": 0.05, "pressureSize": true, "pressureOpacity": true, "smoothing": {"amount": 0}});
        app.session.execute("tools.setBrush", json!({ "brush": brush })).unwrap();
        app.ui.tool = Tool::Brush;
        let m = egui::Modifiers::NONE;
        let n = 20;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            sink(next(0.1 + 0.9 * t));
            let (x, y) = (20.0 + 200.0 * f64::from(t), 40.0);
            let pressure = app.stylus.pressure();
            let ev = if i == 0 { ToolEvent::Down { x, y, pressure } } else { ToolEvent::Move { x, y, pressure } };
            tool_event(&mut app, ev, m);
        }
        tool_event(&mut app, ToolEvent::Up { x: 220.0, y: 40.0 }, m);
        sink(None);
        // Darkness (1 - luma) along x = 50 (light end) and x = 200 (heavy end), and the painted
        // half-width there.
        let mut dark = |x: i32, y: i32| -> f64 {
            let px: Value = app.session.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
            1.0 - px[0].as_f64().unwrap()
        };
        let (light, heavy) = (dark(50, 40), dark(200, 40));
        let light_w = (0..40).take_while(|d| dark(50, 40 + d) > 0.02).count() as f64;
        let heavy_w = (0..40).take_while(|d| dark(200, 40 + d) > 0.02).count() as f64;
        (light, light_w, heavy, heavy_w)
    }

    #[test]
    fn appkit_tablet_pressure_drives_stroke_width_and_opacity() {
        let mut st = appkit::State::default();
        let (light_dark, light_w, heavy_dark, heavy_w) = pressure_stroke(|p| {
            let e =
                appkit::RawEvent { kind: appkit::event_type::LEFT_MOUSE_DRAGGED, subtype: appkit::subtype::TABLET_POINT, pressure: p, ..Default::default() };
            match st.handle(&e) {
                Update::Set(s) => s,
                Update::Keep => None,
            }
        });
        assert!(heavy_w > light_w * 2.0, "width responds: {light_w} → {heavy_w}");
        assert!(heavy_dark > light_dark + 0.3, "opacity responds: {light_dark} → {heavy_dark}");
        assert!(heavy_dark > 0.9, "full pressure paints (nearly) opaque: {heavy_dark}");

        // A mouse (non-tablet subtype) through the same path: full size and opacity throughout.
        let mut st = appkit::State::default();
        let (m_dark, m_w, _, _) = pressure_stroke(|p| {
            let e = appkit::RawEvent { kind: appkit::event_type::LEFT_MOUSE_DRAGGED, subtype: appkit::subtype::MOUSE, pressure: p, ..Default::default() };
            match st.handle(&e) {
                Update::Set(s) => s,
                Update::Keep => None,
            }
        });
        assert!(m_dark > 0.9 && m_w >= heavy_w - 1.0, "mouse = pressure 1: {m_dark}, {m_w}");
    }

    #[test]
    fn use_tablet_pressure_off_paints_like_a_mouse() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.session.execute("prefs.set", json!({"path": "tools.useTabletPressure", "value": false})).unwrap();
        // The canvas copies the preference each frame; do what it does.
        app.stylus.use_pressure = app.session.prefs().tools.use_tablet_pressure;
        sink(app.stylus.feed.clone())(Some(Sample { pressure: 0.2, tilt_x: 40.0, ..Default::default() }));
        assert_eq!(app.stylus.pressure(), 1.0);
        assert_eq!(app.stylus.sample(), None);
    }

    #[test]
    fn eraser_end_switches_to_the_eraser_and_back() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        let sink = sink(app.stylus.feed.clone());
        let mut st = appkit::State::default();
        let mut send = |e: appkit::RawEvent| {
            if let Update::Set(s) = st.handle(&e) {
                sink(s);
            }
        };
        let prox = |device, entering| appkit::RawEvent { kind: appkit::event_type::TABLET_PROXIMITY, device, entering, ..Default::default() };
        app.ui.tool = Tool::Brush;
        send(prox(appkit::device::PEN, true));
        assert!(!prism_ui_egui::stylus::Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Brush, "the tip changes nothing");
        send(prox(appkit::device::PEN, false));
        send(prox(appkit::device::ERASER, true));
        assert!(prism_ui_egui::stylus::Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Eraser);
        // Mouse moves in between change nothing; the tip coming back restores the Brush.
        send(appkit::RawEvent { kind: appkit::event_type::MOUSE_MOVED, ..Default::default() });
        assert!(!prism_ui_egui::stylus::Stylus::sync_eraser_tool(&mut app));
        send(prox(appkit::device::PEN, true));
        assert!(prism_ui_egui::stylus::Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Brush);
    }
}
