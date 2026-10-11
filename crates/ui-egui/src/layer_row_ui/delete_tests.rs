//! Protected layers' trash button must be disabled before destructive input is accepted.

use crate::{PrismApp, theme::ThemeKind};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

fn button(h: &mut Harness<'_, PrismApp>, at: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn single(theme: ThemeKind, scale: f32) -> Harness<'static, PrismApp> {
    let mut s = prism_engine::Session::new();
    s.execute("file.new", json!({"width":64,"height":48,"background":"transparent"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1000.0, 720.0)).with_pixels_per_point(scale).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, theme);
        PrismApp::new(s, Default::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) =
        crate::control::ControlRequest::new("ui.set", json!({"theme":theme.id(),"dock":{"collapsed":["color","properties","history","navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    h
}

#[test]
fn last_layer_trash_is_disabled_and_recovers_after_new_undo_and_redo() {
    for theme in ThemeKind::ALL {
        for scale in [1.0, 2.0] {
            let mut h = single(theme, scale);
            assert!(h.get_by_label("Delete layer").accesskit_node().is_disabled());
            assert!(!crate::menus::is_enabled(h.state(), "layer.delete"));
            let at = h.get_by_label("Delete layer").rect().center();
            let history = h.state().session.active().unwrap().history.past_len();
            h.hover_at(at);
            h.run_steps(2);
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
            button(&mut h, at, true);
            button(&mut h, at, false);
            assert!(!h.state().ui.status_error, "disabled button should not report an attempted delete");
            for key in [Key::Delete, Key::Backspace] {
                h.key_press(key);
                h.run_steps(2);
            }
            assert_eq!(h.state().session.active().unwrap().doc.layer_count(), 1);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
            assert!(h.state().ui.status.contains("a document must keep at least one layer"));
            h.state_mut().run("layer.new.layer", json!({})).unwrap();
            h.run_steps(3);
            assert!(!h.get_by_label("Delete layer").accesskit_node().is_disabled());
            let at = h.get_by_label("Delete layer").rect().center();
            button(&mut h, at, true);
            button(&mut h, at, false);
            h.run_steps(3);
            assert_eq!(h.state().session.active().unwrap().doc.layer_count(), 1);
            assert!(h.get_by_label("Delete layer").accesskit_node().is_disabled());
            assert!(h.state_mut().session.undo());
            h.run_steps(3);
            assert!(!h.get_by_label("Delete layer").accesskit_node().is_disabled());
            assert!(h.state_mut().session.redo());
            h.run_steps(3);
            assert!(h.get_by_label("Delete layer").accesskit_node().is_disabled());
        }
    }
}

#[test]
fn locked_layer_trash_is_disabled_and_recovers_after_unlock_undo_and_redo() {
    for theme in ThemeKind::ALL {
        for scale in [1.0, 2.0] {
            let mut h = single(theme, scale);
            h.state_mut().run("layer.new.layer", json!({})).unwrap();
            h.state_mut().run("layer.setProps", json!({"locked":true})).unwrap();
            h.run_steps(3);
            assert!(h.get_by_label("Delete layer").accesskit_node().is_disabled());
            assert!(!crate::menus::is_enabled(h.state(), "layer.delete"));
            let at = h.get_by_label("Delete layer").rect().center();
            let history = h.state().session.active().unwrap().history.past_len();
            h.hover_at(at);
            h.run_steps(2);
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
            button(&mut h, at, true);
            button(&mut h, at, false);
            assert!(!h.state().ui.status_error);
            for key in [Key::Delete, Key::Backspace] {
                h.key_press(key);
                h.run_steps(2);
            }
            assert_eq!(h.state().session.active().unwrap().doc.layer_count(), 2);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
            assert!(h.state().ui.status.contains("is locked"));
            h.state_mut().run("layer.setProps", json!({"locked":false})).unwrap();
            h.run_steps(3);
            assert!(!h.get_by_label("Delete layer").accesskit_node().is_disabled());
            assert!(h.state_mut().session.undo());
            h.run_steps(3);
            assert!(h.get_by_label("Delete layer").accesskit_node().is_disabled());
            assert!(h.state_mut().session.redo());
            h.run_steps(3);
            assert!(!h.get_by_label("Delete layer").accesskit_node().is_disabled());
        }
    }
}

#[test]
fn dropping_a_locked_layer_on_trash_does_not_dispatch_or_edit() {
    let mut h = single(ThemeKind::ProMedium, 1.0);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.state_mut().run("layer.setProps", json!({"locked":true})).unwrap();
    h.run_steps(3);
    let from = super::recorded(&h.ctx)[0].row.center();
    let to = h.get_by_label("Delete layer").rect().center();
    let history = h.state().session.active().unwrap().history.past_len();
    h.hover_at(from);
    h.step();
    button(&mut h, from, true);
    for k in 1..=8 {
        h.hover_at(from + (to - from) * (k as f32 / 8.0));
        h.step();
    }
    assert!(h.get_by_label("Delete layer").accesskit_node().is_disabled());
    button(&mut h, to, false);
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layer_count(), 2);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    assert!(!h.state().ui.status_error);
}

#[test]
fn dropping_the_only_layer_on_trash_does_not_dispatch_or_edit() {
    let mut h = single(ThemeKind::ProMedium, 1.0);
    let from = super::recorded(&h.ctx)[0].row.center();
    let to = h.get_by_label("Delete layer").rect().center();
    let history = h.state().session.active().unwrap().history.past_len();
    h.hover_at(from);
    h.step();
    button(&mut h, from, true);
    for k in 1..=8 {
        h.hover_at(from + (to - from) * (k as f32 / 8.0));
        h.step();
    }
    assert!(h.get_by_label("Delete layer").accesskit_node().is_disabled());
    button(&mut h, to, false);
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layer_count(), 1);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    assert!(!h.state().ui.status_error);
}
