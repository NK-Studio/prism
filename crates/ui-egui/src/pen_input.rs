//! Ordered pen packets, independent of the frame's emulated mouse state.

use egui::{Context, Pos2, RawInput, TouchPhase, ViewportId};

use crate::stylus::PenSample;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Down,
    Move,
    Up,
    Cancel,
    Hover,
}

/// Positions are viewport-local egui points; timestamps are monotonic milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct Packet {
    pub source: &'static str,
    pub id: u64,
    pub device: u64,
    pub phase: Phase,
    pub pos: Pos2,
    pub sample: PenSample,
    pub pressure_valid: bool,
    pub pressure_supported: Option<bool>,
    pub time: f64,
    pub barrel: bool,
}

type ReadPackets = Box<dyn FnMut(&Context, &RawInput) -> Vec<Packet> + Send + Sync>;

/// Install once on the shared context: egui invokes plugins for detached viewports too.
pub fn install(ctx: &Context, read: ReadPackets) {
    ctx.add_plugin(NativePackets { read });
}

struct NativePackets {
    read: ReadPackets,
}

fn packet_key(viewport: ViewportId) -> egui::Id {
    egui::Id::new(("native-pen-packets", viewport))
}

impl egui::plugin::Plugin for NativePackets {
    fn debug_name(&self) -> &'static str {
        "Prism pen packets"
    }

    fn input_hook(&mut self, ctx: &Context, raw: &mut RawInput) {
        let packets = (self.read)(ctx, raw);
        // winit also exposes Windows pens as Touch. egui's independent long-touch
        // recognizer clears the canvas interaction after its timeout, hiding the
        // brush cursor even when Windows press-and-hold is disabled. Remove only
        // the known native pen's touch emulation before egui sees it. Keep pointer
        // events for widget hit testing/buttons and unrelated touchscreen input.
        // Read first: the Windows adapter uses these touch IDs to bind viewports.
        raw.events.retain(|event| {
            !matches!(event, egui::Event::Touch { id, .. }
                if packets.iter().any(|packet| packet.source == "windowsInk" && packet.id == id.0))
        });
        ctx.data_mut(|d| d.insert_temp(packet_key(raw.viewport_id), packets));
    }
}

/// A viewport consumes only its own packets. Hover remains available to normal UI hit testing.
pub(crate) fn take(ctx: &Context) -> Vec<Packet> {
    let key = packet_key(ctx.viewport_id());
    ctx.data_mut(|d| d.remove_temp::<Vec<Packet>>(key)).unwrap_or_default()
}

/// Cancel the uncommitted preview; no engine history entry was made yet.
pub(crate) fn cancel(app: &mut crate::PrismApp) {
    app.drag = None;
    app.live_stroke = None;
    app.press_stroke = false;
    app.alt_sampling = false;
    app.defer_live_stroke = false;
    app.stylus.contact = None;
    app.stylus.contact_document = None;
    app.stylus.feed.set(None);
    app.stylus.time = 0.0;
    app.stylus.touch_packets = TouchPackets::default();
}

pub(crate) fn paint_packets(
    app: &mut crate::PrismApp,
    viewport: ViewportId,
    packets: &[Packet],
    rect: egui::Rect,
    xf: &crate::canvas::ViewXform,
    modifiers: egui::Modifiers,
) -> bool {
    use crate::canvas::{ToolEvent, tool_event};
    let mut handled = app.stylus.contact.is_some_and(|(v, _)| v == viewport);
    let was_deferred = app.defer_live_stroke;
    app.defer_live_stroke = true;
    for packet in packets {
        if app.stylus.trace_enabled && packet.phase != Phase::Hover {
            let [x, y] = if packet.phase == Phase::Up {
                app.drag.as_ref().and_then(|d| d.points.last()).map(|p| [p[0], p[1]]).unwrap_or_else(|| xf.to_doc(packet.pos))
            } else {
                xf.to_doc(packet.pos)
            };
            if app.stylus.trace.len() >= 4096 {
                app.stylus.trace.pop_front();
            }
            app.stylus.trace.push_back(serde_json::json!({"source":packet.source,"id":packet.id,"device":packet.device,"kind":format!("{:?}",packet.phase).to_lowercase(),"x":x,"y":y,"pressure":packet.sample.pressure,"pressureValid":packet.pressure_valid,"pressureSupported":packet.pressure_supported,"time":packet.time,"tiltX":packet.sample.tilt_x,"tiltY":packet.sample.tilt_y,"rotation":packet.sample.rotation,"barrel":packet.barrel}));
        }
        let contact = (viewport, packet.id);
        if packet.barrel && app.stylus.contact == Some(contact) {
            cancel(app);
            handled = true;
        }
        if packet.phase == Phase::Hover || packet.barrel {
            continue;
        }
        // Never let emulated mouse input restart an ignored/canceled pen contact.
        handled = true;
        if !packet.pos.is_finite() || !(0.0..=1.0).contains(&packet.sample.pressure) || !packet.time.is_finite() || packet.time < 0.0 {
            cancel(app);
            handled = true;
            continue;
        }
        let [x, y] = xf.to_doc(packet.pos);
        if packet.phase == Phase::Down {
            // A new contact cannot inherit the previous stroke's pressure or pose.
            if app.stylus.contact.is_some() {
                cancel(app);
            }
            if !rect.contains(packet.pos) || app.drag.is_some() {
                continue;
            }
            app.stylus.feed.set(Some(packet.sample));
            crate::stylus::Stylus::sync_eraser_tool(app);
            app.stylus.time = packet.time;
            app.stylus.contact = Some(contact);
            app.stylus.contact_document = app.session.active().map(|s| s.doc.id);
            let pressure = app.stylus.pressure();
            tool_event(app, ToolEvent::Down { x, y, pressure }, modifiers);
            handled = true;
        } else if app.stylus.contact == Some(contact) {
            handled = true;
            match packet.phase {
                Phase::Move => {
                    app.stylus.feed.set(Some(packet.sample));
                    app.stylus.time = packet.time.max(app.stylus.time);
                    let pressure = app.stylus.pressure();
                    tool_event(app, ToolEvent::Move { x, y, pressure }, modifiers);
                }
                Phase::Up => {
                    app.defer_live_stroke = false;
                    crate::canvas::feed_live_stroke(app);
                    // Up coordinates/force can already describe a hovering pen. Finish at the
                    // last contact point instead of extending a tail using release pressure.
                    let last = app.drag.as_ref().and_then(|d| d.points.last()).map(|p| [p[0], p[1]]).unwrap_or([x, y]);
                    tool_event(app, ToolEvent::Up { x: last[0], y: last[1] }, modifiers);
                    app.stylus.contact = None;
                    app.stylus.contact_document = None;
                    app.stylus.feed.set(None);
                    app.stylus.time = 0.0;
                    app.defer_live_stroke = true;
                }
                Phase::Cancel => cancel(app),
                Phase::Down | Phase::Hover => {}
            }
        }
    }
    app.defer_live_stroke = was_deferred;
    crate::canvas::feed_live_stroke(app);
    handled
}

/// Fallback for platforms delivering ordered egui touch force. Native Windows uses full history.
#[derive(Clone, Debug, Default)]
pub(crate) struct TouchPackets {
    contact: Option<(egui::TouchDeviceId, egui::TouchId, f32)>,
}

impl TouchPackets {
    pub fn collect(&mut self, events: &[egui::Event]) -> Vec<Packet> {
        let mut out = Vec::new();
        for event in events {
            let egui::Event::Touch { device_id, id, phase, pos, force } = event else { continue };
            if *phase == TouchPhase::Start && self.contact.is_none() {
                self.contact = Some((*device_id, *id, 0.0));
            }
            // Orphan move/release still identifies a pen-origin emulated mouse event. Keep
            // it so a canceled contact cannot be reinterpreted as a fresh full-pressure click.
            let mut orphan = (*device_id, *id, 0.0);
            let (device, contact, pressure) = self.contact.as_mut().unwrap_or(&mut orphan);
            if device != device_id || contact != id {
                continue;
            }
            let valid_force = force.filter(|p| p.is_finite() && (0.0..=1.0).contains(p));
            let valid = valid_force.is_some();
            // Lift force is usually zero: it is a lifecycle event, not a dab.
            if matches!(phase, TouchPhase::Start | TouchPhase::Move)
                && let Some(p) = valid_force
            {
                *pressure = p;
            }
            out.push(Packet {
                source: "eguiTouch",
                id: id.0,
                device: device_id.0,
                phase: match phase {
                    TouchPhase::Start => Phase::Down,
                    TouchPhase::Move => Phase::Move,
                    TouchPhase::End => Phase::Up,
                    TouchPhase::Cancel => Phase::Cancel,
                },
                pos: *pos,
                sample: PenSample { pressure: *pressure, ..Default::default() },
                pressure_valid: valid,
                pressure_supported: valid.then_some(true),
                time: 0.0,
                barrel: false,
            });
            if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
                self.contact = None;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(phase: TouchPhase, pressure: Option<f32>) -> egui::Event {
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase, pos: egui::pos2(10.0, 20.0), force: pressure }
    }

    #[test]
    fn batching_preserves_each_pressure_and_lift_does_not_stamp_zero() {
        let events =
            [event(TouchPhase::Start, Some(0.1)), event(TouchPhase::Move, Some(0.8)), event(TouchPhase::Move, Some(0.2)), event(TouchPhase::End, Some(0.0))];
        let batched = TouchPackets::default().collect(&events);
        let mut state = TouchPackets::default();
        let split: Vec<_> = events.iter().flat_map(|e| state.collect(std::slice::from_ref(e))).collect();
        assert_eq!(batched.iter().map(|p| p.sample.pressure).collect::<Vec<_>>(), vec![0.1, 0.8, 0.2, 0.2]);
        assert_eq!(batched.iter().map(|p| p.sample.pressure).collect::<Vec<_>>(), split.iter().map(|p| p.sample.pressure).collect::<Vec<_>>());
    }

    #[test]
    fn unknown_initial_pressure_is_not_full_and_contacts_do_not_inherit() {
        let mut state = TouchPackets::default();
        let packets = state.collect(&[
            event(TouchPhase::Start, Some(2.0)),
            event(TouchPhase::Move, Some(0.4)),
            event(TouchPhase::Move, Some(f32::NAN)),
            event(TouchPhase::Cancel, None),
            event(TouchPhase::Start, None),
        ]);
        assert_eq!(packets.iter().map(|p| p.sample.pressure).collect::<Vec<_>>(), vec![0.0, 0.4, 0.4, 0.4, 0.0]);
    }

    fn app() -> crate::PrismApp {
        let mut app = crate::PrismApp::new(prism_engine::Session::new(), Default::default());
        app.session.execute("file.new", serde_json::json!({"width": 128, "height": 128})).unwrap();
        app.session.execute("layer.new.layer", serde_json::json!({})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        app.session.tools.brush.size = 30.0;
        app
    }

    fn packet(phase: Phase, x: f32, pressure: f32, time: f64) -> Packet {
        Packet {
            source: "synthetic",
            id: 1,
            device: 10,
            phase,
            pos: egui::pos2(x, 64.0),
            sample: PenSample { pressure, ..Default::default() },
            pressure_valid: true,
            pressure_supported: Some(true),
            time,
            barrel: false,
        }
    }

    #[test]
    fn native_hook_removes_only_matched_pen_touch_after_reading() {
        use egui::plugin::Plugin as _;

        for phase in [TouchPhase::Start, TouchPhase::Move, TouchPhase::End, TouchPhase::Cancel] {
            let ctx = Context::default();
            let pen = Packet { source: "windowsInk", id: 2, ..packet(Phase::Down, 10.0, 0.2, 0.0) };
            let mut plugin = NativePackets {
                read: Box::new(move |_, raw| {
                    assert!(
                        raw.events.iter().any(|e| matches!(e, egui::Event::Touch { id, .. } if id.0 == 2)),
                        "viewport binding must see the original pen touch ID"
                    );
                    vec![pen]
                }),
            };
            let finger = egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(3), phase, pos: egui::pos2(20.0, 30.0), force: None };
            let pointer = egui::Event::PointerMoved(pen.pos);
            let primary = egui::Event::PointerButton { pos: pen.pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
            let secondary =
                egui::Event::PointerButton { pos: pen.pos, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE };
            let retained = vec![finger, pointer, primary, secondary];
            let mut raw = RawInput { events: vec![event(phase, Some(0.2))], ..Default::default() };
            raw.events.extend(retained.clone());
            plugin.input_hook(&ctx, &mut raw);
            assert_eq!(raw.events, retained, "real touch, hover and mouse/barrel buttons must remain intact");
            assert_eq!(take(&ctx).len(), 1);
        }
    }

    #[test]
    fn native_hook_without_packets_preserves_touch_fallback() {
        use egui::plugin::Plugin as _;

        let ctx = Context::default();
        let mut plugin = NativePackets { read: Box::new(|_, _| Vec::new()) };
        let events = vec![event(TouchPhase::Start, Some(0.1)), event(TouchPhase::End, Some(0.0))];
        let mut raw = RawInput { events: events.clone(), ..Default::default() };
        plugin.input_hook(&ctx, &mut raw);
        assert_eq!(raw.events, events);
    }

    fn paint(app: &mut crate::PrismApp, packets: &[Packet]) {
        let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(128.0, 128.0));
        let xf = crate::canvas::ViewXform { rect, zoom: 1.0, center: [64.0, 64.0], flip: false, rotation: 0.0 };
        paint_packets(app, ViewportId::ROOT, packets, rect, &xf, egui::Modifiers::NONE);
    }

    #[test]
    fn tap_and_stationary_pressure_are_preserved_in_one_undoable_stroke() {
        let mut app = app();
        paint(
            &mut app,
            &[
                packet(Phase::Down, 64.0, 0.1, 10.0),
                packet(Phase::Move, 64.0, 0.8, 20.0),
                packet(Phase::Move, 64.0, 0.8, 30.0),
                packet(Phase::Up, 90.0, 0.0, 40.0),
            ],
        );
        let strokes: Vec<_> = app.session.journal.iter().filter(|(id, _)| id == "paint.stroke").collect();
        assert_eq!(strokes.len(), 1);
        let points = strokes[0].1["points"].as_array().unwrap();
        assert_eq!(points.len(), 2, "unchanged hold and hovering release do not add dabs");
        assert_eq!(points[0][2], serde_json::json!(f64::from(0.1f32)));
        assert_eq!(points[1][2], serde_json::json!(f64::from(0.8f32)));
        assert_eq!(points[1][6], 20.0);
        assert_eq!(points[1][0], 64.0);
        assert!(app.drag.is_none() && app.live_stroke.is_none());
    }

    #[test]
    fn cancellation_leaves_no_history_and_next_contact_starts_fresh() {
        let mut app = app();
        let before = app.session.journal.len();
        paint(&mut app, &[packet(Phase::Down, 30.0, 0.8, 10.0), packet(Phase::Move, 80.0, 0.9, 20.0), packet(Phase::Cancel, 80.0, 0.0, 30.0)]);
        assert_eq!(app.session.journal.len(), before);
        assert!(app.drag.is_none() && app.live_stroke.is_none() && app.stylus.contact.is_none());
        paint(&mut app, &[packet(Phase::Down, 30.0, 0.1, 40.0), packet(Phase::Up, 30.0, 0.0, 50.0)]);
        assert_eq!(app.session.journal.last().unwrap().1["points"][0][2], serde_json::json!(f64::from(0.1f32)));
    }

    #[test]
    fn batching_does_not_change_committed_sample_stream() {
        let packets =
            [packet(Phase::Down, 20.0, 0.1, 1.0), packet(Phase::Move, 40.0, 0.9, 4.0), packet(Phase::Move, 80.0, 0.2, 7.0), packet(Phase::Up, 80.0, 0.0, 8.0)];
        let mut batched = app();
        let mut split = app();
        paint(&mut batched, &packets);
        for packet in &packets {
            paint(&mut split, std::slice::from_ref(packet));
        }
        assert_eq!(batched.session.journal.last().unwrap().1["points"], split.session.journal.last().unwrap().1["points"]);
    }
}
