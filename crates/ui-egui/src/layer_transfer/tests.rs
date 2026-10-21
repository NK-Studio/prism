//! #589: layers dragged onto another document's tab, then dropped on its canvas, are copied there;
//! so are layers dragged between the main window and floating document windows.

use egui::accesskit::Role;
use egui::{Context, Modifiers, PointerButton, Pos2, Rect, ViewportId, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use prism_doc::LayerId;
use serde_json::json;

use crate::PrismApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// "source-doc" (40×30) with a red 10×10 square at (4, 6) on layer "paint", and "dest-doc"
/// (60×50). The source is active.
fn session() -> (prism_engine::Session, LayerId) {
    let mut s = prism_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "name": "source-doc"})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.new", json!({"width": 60, "height": 50, "name": "dest-doc"})).unwrap();
    s.set_active(0);
    (s, paint)
}

fn harness() -> (Harness<'static, PrismApp>, LayerId) {
    let (s, paint) = session();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, Default::default());
        PrismApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    (h, paint)
}

/// A document tab, found by its accessible title ("name @ 100% (…)", see `pro_tabs`).
fn tab(h: &Harness<'_, PrismApp>, name: &str) -> Pos2 {
    h.get_by_label_contains(&format!("{name} @")).rect().center()
}

/// Press at `path[0]`, move through the rest (a few frames at each) and release at the last
/// point with `release` held.
fn drag_along(h: &mut Harness<'_, PrismApp>, path: &[Pos2], release: Modifiers) {
    let first = path[0];
    h.event(egui::Event::PointerMoved(first));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: first, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for w in path.windows(2) {
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(w[0] + (w[1] - w[0]) * (k as f32 / 4.0)));
            h.run_steps(1);
        }
        h.run_steps(2);
    }
    let last = path[path.len() - 1];
    h.event(egui::Event::ModifiersChanged(release));
    h.event(egui::Event::PointerButton { pos: last, button: PointerButton::Primary, pressed: false, modifiers: release });
    h.run_steps(3);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

/// The copy in "dest-doc" (its active layer) and its bounds.
fn copy(h: &Harness<'_, PrismApp>) -> (String, Rect) {
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.name, "dest-doc", "the destination is shown");
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    let b = prism_engine::layer_multi_cmds::layer_bounds(l).unwrap();
    (l.name.clone(), Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
}

fn source_square(h: &Harness<'_, PrismApp>, paint: LayerId) -> prism_geom::Rect {
    let doc = &h.state().session.documents()[0].doc;
    prism_engine::layer_multi_cmds::layer_bounds(doc.layer(paint).unwrap()).unwrap()
}

#[test]
fn a_layers_panel_drag_onto_a_tab_drops_the_layer_where_it_is_released() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[row, over_tab, drop], Modifiers::NONE);
    let at = ViewXform::active(h.state()).unwrap().to_doc(drop);
    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    assert!((r.center().x - at[0] as f32).abs() <= 1.0 && (r.center().y - at[1] as f32).abs() <= 1.0, "centred on the drop point {at:?}: {r:?}");
    assert_eq!(h.state().session.documents()[0].doc.layers.len(), 2, "the source keeps its layers");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 2);
}

#[test]
fn shift_centres_the_copy_when_the_documents_differ_in_size() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center() + vec2(80.0, 60.0);
    drag_along(&mut h, &[row, over_tab, drop], Modifiers::SHIFT);
    assert_eq!(copy(&h).1, Rect::from_min_max(egui::pos2(25.0, 20.0), egui::pos2(35.0, 30.0)));
}

#[test]
fn a_move_tool_drag_onto_a_tab_copies_the_layer_under_the_pointer_and_leaves_the_source() {
    let (mut h, paint) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(2);
    let before = source_square(&h, paint);
    let steps = h.state().session.documents()[0].history.entries().len();
    let grab = ViewXform::active(h.state()).unwrap().to_screen(9.0, 11.0);
    let grab_doc = ViewXform::active(h.state()).unwrap().to_doc(grab);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);
    let at = ViewXform::active(h.state()).unwrap().to_doc(drop);
    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    // The grabbed pixel lands under the pointer.
    let want = egui::pos2((9.0 + at[0] - grab_doc[0]) as f32, (11.0 + at[1] - grab_doc[1]) as f32);
    assert!((r.center() - want).length() <= 1.5, "{r:?} around {want:?}");
    assert_eq!(source_square(&h, paint), before, "the source layer did not move");
    assert_eq!(h.state().session.documents()[0].history.entries().len(), steps, "no Move step in the source");
}

#[test]
fn dropping_back_on_the_source_copies_nothing() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let (dest_tab, source_tab) = (tab(&h, "dest-doc"), tab(&h, "source-doc"));
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[row, dest_tab, source_tab, drop], Modifiers::NONE);
    let docs = h.state().session.documents();
    assert_eq!((docs[0].doc.layers.len(), docs[1].doc.layers.len()), (2, 1));
    assert_eq!(h.state().session.active_index(), Some(0));
}

// ---------------------------------------------------------------- floating document windows

/// Separate native viewport passes, as eframe runs them (see `doc_windows`'s tests): the main
/// window's client area starts at (100, 100) on the desktop and a floating document window's at
/// (200, 200), both at 100%; the floating window doesn't have the OS focus. Document `float`
/// floats; the other stays in the main window and is active there. The Move tool is picked.
fn native(s: prism_engine::Session, float: usize) -> (Harness<'static, PrismApp>, u64) {
    Context::set_immediate_viewport_renderer(|ctx, mut v| {
        let mut raw = egui::RawInput {
            viewport_id: v.ids.this,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(ctx.input_for(v.ids.this, |i| i.time) + 1.0 / 60.0),
            focused: false,
            ..Default::default()
        };
        raw.viewports = ctx.input(|i| i.raw.viewports.clone());
        raw.viewports.insert(
            v.ids.this,
            egui::ViewportInfo {
                parent: Some(v.ids.parent),
                native_pixels_per_point: Some(1.0),
                inner_rect: Some(Rect::from_min_size(egui::pos2(200.0, 200.0), vec2(800.0, 600.0))),
                focused: Some(false),
                ..Default::default()
            },
        );
        let mut output = ctx.run_ui(raw, |ui| (v.viewport_ui_cb)(ui));
        output.textures_delta.clear();
    });
    let mut app = PrismApp::new(s, crate::Services::default());
    app.ui.tool = Tool::Move;
    let window = crate::doc_windows::detach(&mut app, &Context::default(), float, None).unwrap()["window"].as_u64().unwrap();
    crate::doc_windows::activate_main(&mut app, 1 - float);
    let mut h = Harness::builder().with_size(vec2(1200.0, 850.0)).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::Pro);
        cc.egui_ctx.set_embed_viewports(false);
        app
    });
    let root = h.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap();
    root.inner_rect = Some(Rect::from_min_size(egui::pos2(100.0, 100.0), vec2(1200.0, 850.0)));
    h.run_steps(8);
    (h, window)
}

/// Where the document called `name` is laid out.
fn view_of(h: &Harness<'_, PrismApp>, name: &str) -> super::Target {
    let id = h.state().session.documents().iter().find(|d| d.doc.name == name).unwrap().doc.id;
    let views = h.ctx.data(|d| d.get_temp::<super::Targets>(super::targets_key())).unwrap().0;
    views.into_iter().rev().find(|t| t.document == id).unwrap()
}

/// A point of a view on the desktop (minus a window's client origin: in that window's points).
fn desktop(t: &super::Target, local: Pos2) -> Pos2 {
    (local + t.origin.to_vec2()) * t.scale
}
const MAIN: egui::Vec2 = vec2(100.0, 100.0);
const FLOATING: egui::Vec2 = vec2(200.0, 200.0);

/// Drag in floating window `window` (window points), as automation does.
fn drag_in(h: &mut Harness<'_, PrismApp>, window: u64, from: Pos2, to: Pos2) {
    let ctx = h.ctx.clone();
    let (req, _) = crate::control::ControlRequest::new("ui.drag", json!({"window": window, "from": [from.x, from.y], "to": [to.x, to.y]}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(10);
}

/// The active document's active layer and its bounds.
fn active_copy(h: &Harness<'_, PrismApp>) -> (prism_doc::Layer, Rect) {
    let st = h.state().session.active().unwrap();
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap().clone();
    let b = prism_engine::layer_multi_cmds::layer_bounds(&l).unwrap();
    (l, Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
}

#[test]
fn a_move_drag_from_the_main_window_onto_a_floating_document_copies_the_layer_there() {
    let (s, paint) = session();
    let (mut h, window) = native(s, 1);
    let before = source_square(&h, paint);
    let steps = h.state().session.documents()[0].history.entries().len();
    let source = view_of(&h, "source-doc");
    let grab = source.xf.to_screen(9.0, 11.0);
    let dest = view_of(&h, "dest-doc");
    assert_eq!(dest.window, Some(window));
    // The floating window covers the main canvas there: it is on top.
    let drop = desktop(&dest, dest.xf.to_screen(30.0, 25.0)) - MAIN;
    assert!(source.xf.rect.contains(drop));
    drag_along(&mut h, &[grab, grab + vec2(30.0, 0.0), drop], Modifiers::NONE);
    let (l, r) = active_copy(&h);
    assert_eq!(h.state().session.active().unwrap().doc.name, "dest-doc");
    assert_eq!(h.state().ui.active_window, Some(window), "the floating window is brought forward");
    assert_eq!(l.name, "paint");
    let grab_doc = source.xf.to_doc(grab);
    let want = egui::pos2((9.0 + 30.0 - grab_doc[0]) as f32, (11.0 + 25.0 - grab_doc[1]) as f32);
    assert!((r.center() - want).length() <= 1.5, "the grabbed pixel lands under the pointer: {r:?} around {want:?}");
    assert_eq!(source_square(&h, paint), before, "the source layer did not move");
    assert_eq!(h.state().session.documents()[0].history.entries().len(), steps, "no Move step in the source");
}

#[test]
fn a_group_dragged_from_the_layers_panel_onto_a_floating_document_comes_with_its_layers() {
    let (mut s, paint) = session();
    s.execute("layer.groupLayers", json!({"layer": paint.0, "name": "Group 1"})).unwrap();
    let (mut h, _) = native(s, 1);
    let row = h.get_by_role_and_label(Role::Button, "Group 1").rect().center();
    let dest = view_of(&h, "dest-doc");
    let drop = desktop(&dest, dest.xf.to_screen(30.0, 25.0)) - MAIN;
    drag_along(&mut h, &[row, row + vec2(0.0, 30.0), drop], Modifiers::NONE);
    let (l, r) = active_copy(&h);
    assert_eq!(h.state().session.active().unwrap().doc.name, "dest-doc");
    assert_eq!(l.name, "Group 1");
    assert_eq!(l.children().unwrap().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["paint"]);
    assert!((r.center() - egui::pos2(30.0, 25.0)).length() <= 1.0, "centred on the drop point: {r:?}");
    let src = &h.state().session.documents()[0].doc;
    assert!(src.layers.iter().any(|g| g.name == "Group 1" && g.children().is_some_and(|c| c.len() == 1)), "the source keeps its group");
}

#[test]
fn a_move_drag_from_a_floating_document_onto_the_main_window_copies_the_layer_there() {
    let (s, paint) = session();
    let (mut h, window) = native(s, 0);
    let before = source_square(&h, paint);
    let source = view_of(&h, "source-doc");
    assert_eq!(source.window, Some(window));
    let dest = view_of(&h, "dest-doc");
    assert_eq!(dest.window, None);
    // A main-canvas point the floating window doesn't cover.
    let floating = Rect::from_min_size(FLOATING.to_pos2(), vec2(800.0, 600.0));
    let target = [(0.05, 0.05), (0.95, 0.95), (0.05, 0.95), (0.95, 0.05)]
        .into_iter()
        .map(|(u, v)| dest.xf.rect.lerp_inside(vec2(u, v)))
        .find(|p| !floating.contains(desktop(&dest, *p)))
        .expect("part of the main canvas is uncovered");
    let at = dest.xf.to_doc(target);
    let grab = source.xf.to_screen(9.0, 11.0);
    drag_in(&mut h, window, grab, desktop(&dest, target) - FLOATING);
    let (l, r) = active_copy(&h);
    assert_eq!(h.state().session.active().unwrap().doc.name, "dest-doc");
    assert_eq!(h.state().ui.active_window, None, "the main window shows the destination");
    assert_eq!(l.name, "paint");
    let grab_doc = source.xf.to_doc(grab);
    let want = egui::pos2((9.0 + at[0] - grab_doc[0]) as f32, (11.0 + at[1] - grab_doc[1]) as f32);
    assert!((r.center() - want).length() <= 1.5, "{r:?} around {want:?}");
    assert_eq!(source_square(&h, paint), before, "the source layer did not move");
}

#[test]
fn a_move_drag_that_stays_in_its_floating_window_still_moves() {
    let (s, paint) = session();
    let (mut h, window) = native(s, 0);
    let source = view_of(&h, "source-doc");
    // Past egui's click distance at the window's zoom (where it lands depends on View › Snap).
    let from = source.xf.to_screen(9.0, 11.0);
    drag_in(&mut h, window, from, from + vec2(20.0, 0.0));
    assert!(source_square(&h, paint).x0 > 4, "moved right: {:?}", source_square(&h, paint));
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 1, "nothing copied");
}
