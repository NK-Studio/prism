//! Native pen packets and their emulated mouse events through the actual egui canvas route.
//! These regressions deliberately give emulated Touch events the wrong force: native packets
//! must remain authoritative even for taps, stationary pressure, and non-first document tiles.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui::{Event, Modifiers, PointerButton, Pos2, TouchPhase, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PrismApp;
use crate::pen_input::{Packet, Phase};
use crate::stylus::PenSample;

struct CanvasHarness {
    ui: Harness<'static, PrismApp>,
    pending: Arc<Mutex<Vec<Packet>>>,
    covered: Arc<AtomicBool>,
}

impl CanvasHarness {
    fn new(documents: usize) -> Self {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        for _ in 0..documents {
            app.run("file.new", json!({"width": 128, "height": 128})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
        }
        app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
        app.run(
            "tools.setBrush",
            json!({"brush": {
                "size": 30, "hardness": 1.0, "spacing": 0.05,
                "pressureSize": true, "pressureOpacity": false, "smoothing": {"amount": 0}
            }}),
        )
        .unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        if documents > 1 {
            app.ui.view.arrange = "twoUpVertical".into();
        }
        app.sync_views();
        let covered = Arc::new(AtomicBool::new(false));
        let show_cover = covered.clone();
        let mut ui = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            move |ui, app: &mut PrismApp| {
                let ctx = ui.ctx().clone();
                // As in the existing mouse harness, font setup applies after the first pass.
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
                if show_cover.load(Ordering::Relaxed) {
                    egui::Area::new(egui::Id::new("pen-test-covered-widget"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(app.last_canvas_rect.center() - vec2(80.0, 40.0))
                        .show(&ctx, |ui| {
                            ui.add_sized(vec2(160.0, 80.0), egui::Button::new("Pen test overlay"));
                        });
                }
            },
            app,
        );
        PrismApp::setup_context(&ui.ctx, crate::theme::ThemeKind::ALL[0]);
        let pending = Arc::new(Mutex::new(Vec::new()));
        let queued = pending.clone();
        crate::pen_input::install(&ui.ctx, Box::new(move |_, _| std::mem::take(&mut *queued.lock().unwrap())));
        ui.run_steps(4);
        Self { ui, pending, covered }
    }

    fn center(&self) -> Pos2 {
        let app = self.ui.state();
        crate::canvas::arranged_cells(app, app.last_canvas_rect)
            .and_then(|cells| cells.into_iter().find(|(document, _)| Some(*document) == app.session.active_index()).map(|(_, rect)| rect.center()))
            .unwrap_or_else(|| app.last_canvas_rect.center())
    }

    fn hover(&mut self, position: Pos2) {
        self.ui.input_mut().events.push(Event::PointerMoved(position));
        self.ui.run_steps(2);
    }

    fn native(&mut self, packets: &[Packet]) {
        self.pending.lock().unwrap().extend_from_slice(packets);
        for packet in packets {
            // Simulates winit's historical-coordinate/latest-pressure mismatch. The correct
            // native force must win over this deliberately full-pressure emulation.
            self.emulated(*packet, Some(1.0));
        }
        self.ui.run_steps(1);
    }

    fn emulated(&mut self, packet: Packet, force: Option<f32>) {
        let phase = match packet.phase {
            Phase::Down => TouchPhase::Start,
            Phase::Move => TouchPhase::Move,
            Phase::Up => TouchPhase::End,
            Phase::Cancel => TouchPhase::Cancel,
            Phase::Hover => return,
        };
        let input = self.ui.input_mut();
        input.events.push(Event::Touch { device_id: egui::TouchDeviceId(packet.device), id: egui::TouchId(packet.id), phase, pos: packet.pos, force });
        input.events.push(Event::PointerMoved(packet.pos));
        if matches!(packet.phase, Phase::Down | Phase::Up) {
            input.events.push(Event::PointerButton {
                pos: packet.pos,
                button: PointerButton::Primary,
                pressed: packet.phase == Phase::Down,
                modifiers: Modifiers::NONE,
            });
        }
    }

    fn strokes(&self) -> Vec<Value> {
        self.ui.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").map(|(_, params)| params.clone()).collect()
    }
}

fn packet(phase: Phase, pos: Pos2, pressure: f32, time: f64) -> Packet {
    Packet {
        source: "windowsInk",
        id: 17,
        device: 23,
        phase,
        pos,
        sample: PenSample { pressure, ..Default::default() },
        pressure_valid: true,
        pressure_supported: Some(true),
        time,
        barrel: false,
    }
}

#[test]
fn native_weak_tap_in_one_frame_is_one_small_stroke() {
    let mut h = CanvasHarness::new(1);
    let center = h.center();
    h.hover(center);
    let undo = h.ui.state().session.active().unwrap().history.past_len();
    h.native(&[packet(Phase::Down, center, 0.05, 10.0), packet(Phase::Up, center, 0.0, 11.0)]);
    h.ui.run_steps(2);
    let strokes = h.strokes();
    assert_eq!(strokes.len(), 1, "the emulated click must not paint a second stroke");
    let points = strokes[0]["points"].as_array().unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0][2], json!(f64::from(0.05f32)), "first native pressure must beat emulated pressure 1");
    assert_eq!(h.ui.state().session.active().unwrap().history.past_len(), undo + 1);
    let doc = h.ui.state().session.active().unwrap();
    let surface = doc.doc.layer(doc.active_layer.unwrap()).unwrap().surface().unwrap();
    let painted = (0..128).flat_map(|y| (0..128).map(move |x| (x, y))).filter(|&(x, y)| surface.rgba(x, y)[3] > 0.0).count();
    assert!(painted > 0 && painted < 100, "weak contact must not leave a full-size initial blob ({painted} pixels)");
}

#[test]
fn native_batch_preserves_stationary_pressure_and_each_sample_time() {
    let mut h = CanvasHarness::new(1);
    let center = h.center();
    h.hover(center);
    h.native(&[
        packet(Phase::Down, center, 0.05, 10.0),
        packet(Phase::Move, center, 0.8, 20.0),
        packet(Phase::Move, center + vec2(40.0, 0.0), 0.3, 30.0),
        packet(Phase::Up, center + vec2(90.0, 0.0), 0.0, 40.0),
    ]);
    let strokes = h.strokes();
    assert_eq!(strokes.len(), 1);
    let points = strokes[0]["points"].as_array().unwrap();
    assert_eq!(points.len(), 3, "stationary pressure is retained, hovering release is not a dab");
    assert_eq!(points.iter().map(|p| p[2].as_f64().unwrap()).collect::<Vec<_>>(), [0.05f32, 0.8, 0.3].map(f64::from));
    assert_eq!(points.iter().map(|p| p[6].as_f64().unwrap()).collect::<Vec<_>>(), vec![10.0, 20.0, 30.0]);
    assert_eq!(points[0][0], points[1][0]);
    assert_eq!(points[0][1], points[1][1]);
    assert!(points[2][0].as_f64().unwrap() > points[1][0].as_f64().unwrap());
}

#[test]
fn egui_touch_fallback_preserves_each_force_in_a_single_frame() {
    let mut h = CanvasHarness::new(1);
    let center = h.center();
    h.hover(center);
    for (phase, pressure, position) in
        [(Phase::Down, 0.1, center), (Phase::Move, 0.6, center), (Phase::Move, 0.2, center + vec2(30.0, 0.0)), (Phase::Up, 0.0, center + vec2(30.0, 0.0))]
    {
        h.emulated(packet(phase, position, pressure, 0.0), Some(pressure));
    }
    h.ui.run_steps(1);
    let strokes = h.strokes();
    assert_eq!(strokes.len(), 1);
    let pressures: Vec<_> = strokes[0]["points"].as_array().unwrap().iter().map(|p| p[2].as_f64().unwrap()).collect();
    assert_eq!(pressures, [0.1f32, 0.6, 0.2].map(f64::from));
}

#[test]
fn canceled_native_contact_later_emulated_release_does_not_stamp() {
    let mut h = CanvasHarness::new(1);
    let center = h.center();
    h.hover(center);
    let undo = h.ui.state().session.active().unwrap().history.past_len();
    h.native(&[packet(Phase::Down, center, 0.2, 10.0)]);
    assert!(h.ui.state().stylus.contact.is_some());
    // Native focus/capture cancellation need not have a matching egui touch event that frame.
    h.pending.lock().unwrap().push(packet(Phase::Cancel, center, 0.2, 20.0));
    h.ui.run_steps(1);
    assert!(h.ui.state().stylus.contact.is_none());
    assert!(h.ui.state().drag.is_none() && h.ui.state().live_stroke.is_none());
    // The OS can still send the delayed release after the native contact has been canceled.
    h.emulated(packet(Phase::Up, center, 0.0, 30.0), Some(0.0));
    h.ui.run_steps(2);
    assert!(h.strokes().is_empty(), "canceled contact must not become a fresh emulated mouse click");
    assert_eq!(h.ui.state().session.active().unwrap().history.past_len(), undo);
}

#[test]
fn earlier_inactive_document_tile_cannot_consume_native_packets() {
    let mut h = CanvasHarness::new(2);
    assert_eq!(h.ui.state().session.active_index(), Some(1));
    let cells = crate::canvas::arranged_cells(h.ui.state(), h.ui.state().last_canvas_rect).unwrap();
    assert_eq!(cells[0].0, 0, "regression requires inactive document rendered before active document");
    let center = h.center();
    h.hover(center);
    let other_undo = h.ui.state().session.documents()[0].history.past_len();
    h.native(&[packet(Phase::Down, center, 0.07, 10.0), packet(Phase::Up, center, 0.0, 11.0)]);
    let strokes = h.strokes();
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0]["points"][0][2], json!(f64::from(0.07f32)), "the active tile must read native force, not fallback force 1");
    assert_eq!(h.ui.state().session.documents()[0].history.past_len(), other_undo);
}

#[test]
#[cfg(target_os = "windows")]
fn stationary_native_pen_keeps_brush_cursor_after_long_press_timeout() {
    let mut h = CanvasHarness::new(1);
    let center = h.center();
    h.hover(center);
    h.native(&[packet(Phase::Down, center, 0.3, 10.0)]);
    assert!(h.ui.output().platform_output.cursor_image.is_some());
    for frame in 0..180 {
        h.ui.run_steps(1);
        assert!(h.ui.output().platform_output.cursor_image.is_some(), "brush cursor vanished during stationary contact at frame {frame}");
        assert!(h.ui.state().ui.brush_picker.is_none());
        assert!(h.ui.state().stylus.contact.is_some());
    }
    h.native(&[packet(Phase::Up, center, 0.0, 3010.0)]);
    assert_eq!(h.strokes().len(), 1);
    assert_eq!(h.strokes()[0]["points"].as_array().unwrap().len(), 1, "cursor persistence cannot add stationary ink");
}

#[test]
fn native_tap_on_foreground_widget_does_not_paint_underneath() {
    let mut h = CanvasHarness::new(1);
    h.covered.store(true, Ordering::Relaxed);
    h.ui.run_steps(3);
    let rect = h.ui.ctx.memory(|m| m.area_rect(egui::Id::new("pen-test-covered-widget"))).unwrap();
    let center = rect.center();
    h.hover(center);
    assert_eq!(h.ui.ctx.layer_id_at(center).unwrap().order, egui::Order::Foreground);
    h.native(&[packet(Phase::Down, center, 0.5, 10.0), packet(Phase::Up, center, 0.0, 11.0)]);
    h.ui.run_steps(2);
    assert!(h.strokes().is_empty(), "overlay owns the press even though its position lies in the canvas");
}
