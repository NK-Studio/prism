use super::*;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::{Harness, kittest::Queryable};

fn radii(app: &PrismApp) -> [f64; 4] {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape") };
    let Some(prism_doc::LiveShape::Rect { radii, .. }) = sh.live else { panic!("rectangle") };
    radii
}

fn harness(initial: [f64; 4], style: prism_doc::CornerStyle) -> Harness<'static, PrismApp> {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("shape.create", json!({"kind":"roundedRect", "rect":[0,0,200,100], "radii":initial, "corners":style.as_str()})).unwrap();
    let mut h = Harness::builder().with_size(vec2(300.0, 100.0)).build_ui_state(
        move |ui, app| {
            if let Some(patch) = corner_grid(ui, "scrub-test", radii(app), style) {
                app.run("shape.edit", patch).unwrap();
            }
        },
        app,
    );
    PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(4);
    h
}

fn drag(h: &mut Harness<'_, PrismApp>, label: &str, delta: f32) {
    let start = h.get_by_label(label).rect().center();
    h.hover_at(start);
    h.step();
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for fraction in [0.5, 1.0] {
        h.event(Event::PointerMoved(start + vec2(delta * fraction, 0.0)));
        h.step();
    }
    let held = radii(h.state());
    h.run_steps(3);
    assert_eq!(radii(h.state()), held, "holding the pointer must not keep adding its displacement");
    h.event(Event::PointerButton { pos: start + vec2(delta, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn each_corner_icon_scrubs_only_its_corner_when_unlinked() {
    for style in [prism_doc::CornerStyle::Round, prism_doc::CornerStyle::Chamfer] {
        for (corner, label) in
            ["Top-left corner radius", "Top-right corner radius", "Bottom-right corner radius", "Bottom-left corner radius"].iter().enumerate()
        {
            let mut h = harness([10.0, 20.0, 30.0, 40.0], style);
            let history = h.state().session.active().unwrap().history.past_len();
            drag(&mut h, label, 16.0);
            let mut expected = [10.0, 20.0, 30.0, 40.0];
            expected[corner] += 16.0;
            assert_eq!(radii(h.state()), expected);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(radii(h.state()), [10.0, 20.0, 30.0, 40.0]);
        }
    }
}

#[test]
fn linked_corner_scrubs_clamp_at_zero_and_each_gesture_has_its_own_undo() {
    let mut h = harness([20.0; 4], prism_doc::CornerStyle::Round);
    let history = h.state().session.active().unwrap().history.past_len();
    drag(&mut h, "Top-left corner radius", 16.0);
    assert_eq!(radii(h.state()), [36.0; 4]);
    drag(&mut h, "Top-right corner radius", -60.0);
    assert_eq!(radii(h.state()), [0.0; 4]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 2);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(radii(h.state()), [36.0; 4]);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(radii(h.state()), [20.0; 4]);
}
