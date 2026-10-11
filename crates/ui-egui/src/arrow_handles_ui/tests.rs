use super::*;
use crate::canvas::tool_event;
use prism_doc::{Color, ColorMode, SampleType, Size};

fn app(depth: SampleType) -> PrismApp {
    let mut session = prism_engine::Session::new();
    session.add_document(Document::with_background("handles", Size::new(400, 300), ColorMode::Rgb, depth, Color::WHITE), None);
    let mut a = PrismApp::new(session, Default::default());
    a.run("shape.create", json!({"kind":"arrow","from":[60,150],"to":[300,150],"headSize":60,"stroke":{"width":8,"color":"#ff0000"}})).unwrap();
    a.ui.tool = Tool::Arrow;
    a.ui.extras.snap = false;
    a.ui.extras.rulers = false;
    a.ui.view.show.smart_guides = false;
    a.sync_views();
    a.ui.views[0].zoom = 1.0;
    a.ui.views[0].center = [200.0, 150.0];
    a
}
fn event(a: &mut PrismApp, kind: &str, p: [f64; 2]) {
    let [x, y] = p;
    tool_event(
        a,
        match kind {
            "down" => ToolEvent::Down { x, y, pressure: 1.0 },
            "move" => ToolEvent::Move { x, y, pressure: 1.0 },
            _ => ToolEvent::Up { x, y },
        },
        Modifiers::NONE,
    );
}
fn geometry(a: &PrismApp) -> ArrowGeometry {
    arrow(a).unwrap().1
}

#[test]
fn move_drag_hides_handles_until_release_without_blocking_handle_edits() {
    let mut a = app(SampleType::U8);
    a.ui.tool = Tool::Move;
    let base = geometry(&a);
    assert!(!inspect(&a).is_null());
    event(&mut a, "down", [150.0, 150.0]);
    event(&mut a, "move", [180.0, 170.0]);
    assert!(a.drag.as_ref().is_some_and(|d| d.tool == Tool::Move));
    assert!(inspect(&a).is_null());
    assert!(cursor(&a, base.to).is_none());
    event(&mut a, "up", [180.0, 170.0]);
    assert!(a.drag.is_none());
    assert!(!inspect(&a).is_null());
    let moved = geometry(&a);
    assert_eq!(moved.from, [base.from[0] + 30.0, base.from[1] + 20.0]);
    let wing = handles(moved, 1.0).unwrap()[2];
    event(&mut a, "down", wing);
    assert!(a.ui.arrow_handles.is_some());
    assert!(!inspect(&a).is_null(), "editing a handle remains visible with Move selected");
    event(&mut a, "up", wing);
}

#[test]
fn wing_length_and_angle_handles_preview_then_commit_one_step_at_each_depth() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for index in [2, 3, 4] {
            let mut a = app(depth);
            let (id, base) = arrow(&a).unwrap();
            let points = handles(base, 1.0).unwrap();
            let start = points[index];
            let end = if index < 4 {
                [base.to[0] + (start[0] - base.to[0]) * 1.5, base.to[1] + (start[1] - base.to[1]) * 1.5]
            } else {
                let r = (start[0] - base.to[0]).hypot(start[1] - base.to[1]);
                [base.to[0] - r * 0.5_f64.sqrt(), base.to[1] + r * 0.5_f64.sqrt()]
            };
            let st = a.session.active().unwrap();
            let original = st.doc.clone();
            let history = st.history.past_len();
            event(&mut a, "down", start);
            event(&mut a, "move", end);
            assert!(a.drag.is_none());
            assert!(Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
            let (shown, key) = display_doc(&mut a, 0).unwrap();
            let (same, key2) = display_doc(&mut a, 0).unwrap();
            assert!(Arc::ptr_eq(&shown, &same));
            assert_eq!(key, key2);
            let g = a.ui.arrow_handles.as_ref().unwrap().geometry;
            assert!((g.head_size - if index < 4 { 90.0 } else { 60.0 }).abs() < 1e-8);
            assert!((g.head_angle - if index < 4 { 60.0 } else { 90.0 }).abs() < 1e-8);
            assert_eq!(g.from, base.from);
            assert_eq!(g.to, base.to);
            event(&mut a, "up", end);
            assert_eq!(a.session.active().unwrap().history.past_len(), history + 1);
            assert_eq!(a.session.active().unwrap().doc.layer(id).unwrap().content, shown.layer(id).unwrap().content);
            a.run("edit.undo", json!({})).unwrap();
            assert_eq!(geometry(&a), base);
            a.run("edit.redo", json!({})).unwrap();
            assert_eq!(geometry(&a), g);
        }
    }
}
#[test]
fn endpoints_move_without_changing_wing_settings_and_noops_leave_no_history() {
    for index in [0, 1] {
        let mut a = app(SampleType::U8);
        let base = geometry(&a);
        let start = handles(base, 1.0).unwrap()[index];
        let end = [start[0], start[1] - 40.0];
        let history = a.session.active().unwrap().history.past_len();
        event(&mut a, "down", start);
        event(&mut a, "up", start);
        assert_eq!(a.session.active().unwrap().history.past_len(), history);
        event(&mut a, "down", start);
        event(&mut a, "up", end);
        let g = geometry(&a);
        assert_eq!(g.head_size, base.head_size);
        assert_eq!(g.head_angle, base.head_angle);
        assert_eq!(if index == 0 { g.from } else { g.to }, end);
    }
}
#[test]
fn locked_hidden_and_stale_targets_cannot_be_edited() {
    for mode in 0..7 {
        let mut a = app(SampleType::U8);
        let (id, g) = arrow(&a).unwrap();
        let p = handles(g, 1.0).unwrap()[2];
        event(&mut a, "down", p);
        event(&mut a, "move", [p[0] - 20.0, p[1] + 10.0]);
        display_doc(&mut a, 0).unwrap();
        match mode {
            0 => a.ui.tool = Tool::Brush,
            1 => a.ui.view.extras = false,
            2 => Arc::make_mut(&mut a.session.active_mut().unwrap().doc).layer_mut(id).unwrap().locks.all = true,
            3 => Arc::make_mut(&mut a.session.active_mut().unwrap().doc).layer_mut(id).unwrap().locks.position = true,
            4 => Arc::make_mut(&mut a.session.active_mut().unwrap().doc).layer_mut(id).unwrap().visible = false,
            5 => {
                a.run("shape.edit", json!({"headAngle":100})).unwrap();
            }
            _ => {
                a.run("file.new", json!({"width":100,"height":100})).unwrap();
            }
        }
        cancel_stale(&mut a);
        assert!(a.ui.arrow_handles.is_none());
        assert!(a.arrow_handle_runtime.preview.is_none());
        assert!(!pointer(&mut a, ToolEvent::Up { x: p[0], y: p[1] }, Modifiers::NONE));
    }
}
type Harness = egui_kittest::Harness<'static, PrismApp>;
fn harness(zoom: f32, rotation: f32, flip: bool) -> Harness {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, Default::default());
        let mut a = app(SampleType::U8);
        a.ui.views[0].zoom = zoom;
        a.ui.views[0].rotation = rotation;
        a.ui.view.flip_horizontal = flip;
        a
    });
    h.run_steps(4);
    h
}
fn screen(h: &Harness, p: [f64; 2]) -> egui::Pos2 {
    let a = h.state();
    let v = &a.ui.views[0];
    ViewXform {
        rect: crate::rulers::content_rect(a, a.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: a.ui.view.flip_horizontal,
        rotation: v.rotation,
    }
    .to_screen(p[0] as f32, p[1] as f32)
}
fn button(h: &mut Harness, p: [f64; 2], pressed: bool) {
    let pos = screen(h, p);
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}
#[test]
fn native_handles_follow_rotated_flipped_zoomed_views_and_cancel_without_extra_shapes() {
    for (zoom, rotation, flip) in [(0.5, 0.0, false), (1.0, 30.0, false), (2.0, 30.0, true)] {
        for cancel in [false, true] {
            let mut h = harness(zoom, rotation, flip);
            let base = geometry(h.state());
            let start = handles(base, zoom).unwrap()[2];
            let end = [start[0] - 25.0, start[1] + 15.0];
            let history = h.state().session.active().unwrap().history.past_len();
            button(&mut h, start, true);
            assert!(h.state().ui.arrow_handles.is_some());
            h.event(Event::PointerMoved(screen(&h, end)));
            h.run_steps(2);
            if cancel {
                h.key_press(egui::Key::Escape);
                h.run_steps(2);
            }
            button(&mut h, end, false);
            h.run_steps(2);
            assert!(h.state().ui.arrow_handles.is_none());
            assert!(h.state().drag.is_none());
            assert_eq!(h.state().session.active().unwrap().doc.layer_count(), 2);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + usize::from(!cancel));
            if cancel {
                assert_eq!(geometry(h.state()), base);
            } else {
                assert!(geometry(h.state()).head_size > base.head_size);
            }
        }
    }
}
#[test]
fn inspection_exposes_roles_and_saved_state_never_restores_a_drag() {
    let mut a = app(SampleType::U8);
    let g = geometry(&a);
    let p = handles(g, 1.0).unwrap()[4];
    event(&mut a, "down", p);
    let inspected = crate::control::inspect(&a, &egui::Context::default());
    assert_eq!(inspected["arrowHandles"]["roles"][4], "angle");
    let restored: crate::state::UiState = serde_json::from_value(serde_json::to_value(&a.ui).unwrap()).unwrap();
    assert!(restored.arrow_handles.is_none());
}
