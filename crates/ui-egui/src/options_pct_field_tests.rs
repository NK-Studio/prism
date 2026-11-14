//! The options bar's percentages (Opacity, Flow, Smoothing) work as Photoshop's do: the ▾ in the
//! field opens a pop-up slider, and a sideways drag on the label scrubs the value.

use egui::{Pos2, Rect, accesskit::Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::scrub_delta;
use crate::PrismApp;

fn harness() -> Harness<'static, PrismApp> {
    let mut s = prism_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 120})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, Default::default());
        PrismApp::new(s, crate::Services::default())
    });
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    h.state_mut().session.tools.brush.flow = 0.5;
    h.run_steps(8);
    h
}

fn flow(h: &Harness<'_, PrismApp>) -> f32 {
    h.state().session.tools.brush.flow
}

/// The topmost node with this role whose label is `name`, with or without the options bar's colon.
fn by_role(h: &Harness<'_, PrismApp>, name: &str, role: Role) -> Option<Rect> {
    h.query_all_by_role(role)
        .filter(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == name))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
}

fn click(h: &mut Harness<'_, PrismApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
}

fn drag(h: &mut Harness<'_, PrismApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
}

#[test]
fn flow_arrow_opens_a_slider_that_sets_the_brush_flow() {
    let mut h = harness();
    assert!(by_role(&h, "Flow", Role::Slider).is_none(), "closed until its ▾ is clicked");
    let arrow = by_role(&h, "Flow", Role::Button).expect("the Flow field has a ▾");
    click(&mut h, arrow.center());
    let slider = by_role(&h, "Flow", Role::Slider).expect("the slider opened");
    assert!(slider.top() >= arrow.bottom(), "the slider opens below its field");
    let y = slider.center().y;
    drag(&mut h, Pos2::new(slider.center().x, y), Pos2::new(slider.right() - 7.0, y));
    assert!((flow(&h) - 1.0).abs() < 1e-3, "dragged to the right end: {}", flow(&h));
}

#[test]
fn dragging_the_flow_label_scrubs_the_value() {
    let mut h = harness();
    // The label is the text just left of the Flow number (labels carry no accessible name).
    let arrow = by_role(&h, "Flow", Role::Button).expect("the Flow field has a ▾");
    let label = h
        .query_all_by_role(Role::Label)
        .map(|n| n.rect())
        .filter(|r| r.y_range().contains(arrow.center().y) && r.right() < arrow.left())
        .max_by(|a, b| a.right().total_cmp(&b.right()))
        .expect("the Flow label");
    let from = label.center();
    drag(&mut h, from, from + vec2(20.0, 0.0));
    let up = flow(&h);
    assert!(up > 0.6 && up <= 0.75, "20 pt right of 50% is about 70%: {up}");
    drag(&mut h, from, from - vec2(400.0, 0.0));
    assert!((flow(&h) - 0.01).abs() < 1e-4, "stops at the field's minimum: {}", flow(&h));
}

#[test]
fn scrub_steps_follow_the_modifiers_and_ignore_bad_deltas() {
    assert_eq!(scrub_delta(3.0, false, false), 3.0);
    assert_eq!(scrub_delta(3.0, true, false), 30.0);
    assert!((scrub_delta(3.0, false, true) - 0.3).abs() < 1e-6);
    assert_eq!(scrub_delta(f32::NAN, false, false), 0.0);
    assert_eq!(scrub_delta(f32::INFINITY, true, false), 0.0);
}

/// Whether `v` (0–1) is a whole percent.
fn whole_percent(v: f32) -> bool {
    ((v * 100.0) - (v * 100.0).round()).abs() < 1e-3
}

#[test]
fn drags_move_in_whole_percents_and_no_decimals_show() {
    // A brush that arrives with a fraction shows it rounded, and keeps it until edited.
    let mut h = harness();
    h.state_mut().session.tools.brush.flow = 0.555;
    h.run_steps(3);
    assert_eq!(flow(&h), 0.555, "showing the field changes nothing");
    let arrow = by_role(&h, "Flow", Role::Button).expect("the Flow field has a ▾");
    let number = h
        .query_all_by_role(Role::SpinButton)
        .find(|n| n.rect().y_range().contains(arrow.center().y) && (n.rect().right() - arrow.left()).abs() < 30.0)
        .expect("the Flow number");
    assert_eq!(number.accesskit_node().value().as_deref(), Some("56"));
    let at = number.rect().center();

    // Scrubbing the number by an odd distance (half a unit per point) lands on a whole percent.
    drag(&mut h, at, at - vec2(15.0, 0.0));
    let scrubbed = flow(&h);
    assert!(scrubbed < 0.55 && whole_percent(scrubbed), "{scrubbed}");

    // So does a slow (Alt) scrub of the label, and it still gets somewhere.
    let label = h
        .query_all_by_role(Role::Label)
        .map(|n| n.rect())
        .filter(|r| r.y_range().contains(arrow.center().y) && r.right() < arrow.left())
        .max_by(|a, b| a.right().total_cmp(&b.right()))
        .expect("the Flow label");
    h.key_down(egui::Key::Escape);
    h.key_up(egui::Key::Escape);
    h.run_steps(2);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    drag(&mut h, label.center(), label.center() + vec2(34.0, 0.0));
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    let slow = flow(&h);
    assert!(whole_percent(slow), "{slow}");
    assert!((slow - scrubbed - 0.03).abs() < 0.011, "34 pt at a tenth is 3%: {scrubbed} → {slow}");

    // And the pop-up slider.
    click(&mut h, arrow.center());
    let slider = by_role(&h, "Flow", Role::Slider).expect("the slider opened");
    drag(&mut h, slider.center(), slider.center() + vec2(13.3, 0.0));
    assert!(whole_percent(flow(&h)), "{}", flow(&h));
}

#[test]
fn no_decimals_show_while_the_label_is_dragged() {
    let mut h = harness();
    let arrow = by_role(&h, "Flow", Role::Button).expect("the Flow field has a ▾");
    let label = h
        .query_all_by_role(Role::Label)
        .map(|n| n.rect())
        .filter(|r| r.y_range().contains(arrow.center().y) && r.right() < arrow.left())
        .max_by(|a, b| a.right().total_cmp(&b.right()))
        .expect("the Flow label");
    let from = label.center();
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=9 {
        // Fractions of a point, as a high-resolution pointer reports.
        h.hover_at(from + vec2(i as f32 * 2.3, 0.0));
        h.run_steps(1);
        let shown: Vec<String> = h
            .query_all_by_role(Role::SpinButton)
            .filter(|n| n.rect().y_range().contains(arrow.center().y) && (n.rect().right() - arrow.left()).abs() < 30.0)
            .filter_map(|n| n.accesskit_node().value())
            .collect();
        assert!(shown.len() == 1 && !shown[0].contains('.'), "step {i}: the field shows {shown:?}");
    }
    h.drop_at(from + vec2(20.7, 0.0));
    h.run_steps(2);
    assert!(whole_percent(flow(&h)) && flow(&h) > 0.6, "{}", flow(&h));
}
