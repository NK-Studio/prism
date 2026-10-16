use egui::{Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

fn app(dock: SlidersDock) -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.ui.tool = crate::state::Tool::Brush;
    app.ui.panels.tool_sliders = true;
    app.ui.tool_sliders.dock = dock;
    app
}

/// The panel docked at `dock` in a window of `size`.
fn harness(dock: SlidersDock, size: egui::Vec2) -> Harness<'static, PrismApp> {
    harness_with(app(dock), size)
}

fn harness_with(app: PrismApp, size: egui::Vec2) -> Harness<'static, PrismApp> {
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut PrismApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            docked(app, ui);
            // The canvas takes what the docked panel leaves, as in the app.
            app.last_canvas_rect = ui.available_rect_before_wrap();
            window(app, ui.ctx());
        },
        app,
    );
    PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(4);
    h
}

/// Press at `from`, drag through to `to` in steps, release.
fn drag(h: &mut Harness<'static, PrismApp>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for k in 1..=8 {
        h.hover_at(from + (to - from) * (k as f32 / 8.0));
        h.run();
    }
    h.drop_at(to);
    h.run_steps(2);
}

#[test]
fn size_track_is_logarithmic_and_safe() {
    assert_eq!(Slider::Size.value_at(0.0), 1.0);
    assert_eq!(Slider::Size.value_at(1.0), MAX_BRUSH_SIZE);
    assert_eq!(Slider::Size.value_at(f32::NAN), 1.0);
    // The low half of the travel covers small sizes only.
    assert!(Slider::Size.value_at(0.5) < 100.0);
    let mut b = BrushSettings { size: 12.0, ..Default::default() };
    let f = Slider::Size.fraction(&b);
    assert_eq!(Slider::Size.value_at(f), 12.0);
    b.size = f32::INFINITY;
    assert!(Slider::Size.fraction(&b).is_finite());
    assert_eq!(Slider::Opacity.value_at(0.37), 0.37);
    assert_eq!(Slider::Opacity.value_at(2.0), 1.0);
    assert_eq!(Slider::Size.badge(&BrushSettings { size: 12.0, ..Default::default() }), ("12.0".to_string(), "px"));
    assert_eq!(Slider::Opacity.badge(&BrushSettings { opacity: 1.0, ..Default::default() }), ("100".to_string(), "%"));
}

#[test]
fn layout_follows_the_panel_shape() {
    // Taller than wide: two vertical tracks side by side, badges on top.
    let tall = layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(110.0, 460.0)));
    for (_, badge, track) in tall {
        assert!(track.height() > 300.0 && track.width() < 10.0, "{track:?}");
        assert!(badge.y < track.top());
    }
    assert!(tall[0].2.center().x < tall[1].2.center().x);
    // Wider than tall: two horizontal tracks stacked, badges on the left.
    let wide = layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 100.0)));
    for (_, badge, track) in wide {
        assert!(track.width() > 700.0 && track.height() < 10.0, "{track:?}");
        assert!(badge.x < track.left());
    }
    assert!(wide[0].2.center().y < wide[1].2.center().y);
    // A degenerate panel never inverts a track.
    for (_, _, track) in layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(1.0, 1.0))) {
        assert!(track.width() >= 0.0 && track.height() >= 0.0);
    }
}

#[test]
fn docked_column_is_vertical_and_a_drag_is_one_set_brush() {
    let mut h = harness(SlidersDock::Left, vec2(400.0, 600.0));
    let size = h.get_by_label("Brush Size").rect();
    let opacity = h.get_by_label("Brush Opacity").rect();
    assert!(size.height() > size.width() * 3.0, "vertical in a column: {size:?}");
    assert!(size.center().x < opacity.center().x, "side by side");
    let n = h.state().session.journal.len();
    h.run_steps(3);
    assert_eq!(h.state().session.journal.len(), n, "showing the panel issues nothing");
    let was = h.state().session.tools.brush.size;
    let x = size.center().x;
    drag(&mut h, pos2(x, size.bottom() - 12.0), pos2(x, size.top() + size.height() * 0.2));
    let b = h.state().session.tools.brush.clone();
    assert!(b.size > was, "dragging up grows the brush: {} -> {}", was, b.size);
    let j = &h.state().session.journal;
    assert_eq!(j.len(), n + 1, "{:?}", &j[n..]);
    let (id, p) = j.last().unwrap();
    assert_eq!(id, "tools.setBrush");
    assert_eq!(p["brush"]["size"].as_f64().map(|v| v as f32), Some(b.size));
    // The column remembers its width.
    assert!(h.state().ui.tool_sliders.column >= MIN_THICK);
}

#[test]
fn docked_row_is_horizontal_and_edits_opacity() {
    let mut h = harness(SlidersDock::Bottom, vec2(900.0, 400.0));
    let size = h.get_by_label("Brush Size").rect();
    let opacity = h.get_by_label("Brush Opacity").rect();
    assert!(opacity.width() > opacity.height() * 3.0, "horizontal in a row: {opacity:?}");
    assert!(size.center().y < opacity.center().y, "stacked");
    let n = h.state().session.journal.len();
    let y = opacity.center().y;
    drag(&mut h, pos2(opacity.right() - 10.0, y), pos2(opacity.left() + opacity.width() * 0.5, y));
    let o = h.state().session.tools.brush.opacity;
    assert!(o < 0.7 && o > 0.3, "{o}");
    let j = &h.state().session.journal;
    assert_eq!(j.len(), n + 1);
    assert_eq!(j.last().unwrap().1["brush"]["opacity"].as_f64().map(|v| v as f32), Some(o));
}

#[test]
fn floating_window_turns_with_its_shape() {
    let h = harness(SlidersDock::Floating, vec2(1000.0, 800.0));
    let size = h.get_by_label("Brush Size").rect();
    assert!(size.height() > size.width(), "the default window is tall: {size:?}");
    let mut a = app(SlidersDock::Floating);
    a.ui.tool_sliders.floating = [700.0, 110.0];
    let h = harness_with(a, vec2(1000.0, 800.0));
    let size = h.get_by_label("Brush Size").rect();
    assert!(size.width() > size.height(), "a wide window lays the sliders out across: {size:?}");
}

#[test]
fn window_menu_toggles_and_checks_the_panel() {
    let mut h = harness(SlidersDock::Right, vec2(400.0, 600.0));
    let ctx = h.ctx.clone();
    let item = |h: &Harness<'static, PrismApp>| crate::menus::menu_items(h.state()).into_iter().find(|i| i.id == "window.toggle.toolSliders").unwrap();
    let it = item(&h);
    assert_eq!((it.path.as_slice(), it.label.as_str(), it.checked, it.enabled), (&["Window".to_string()][..], "Tool Sliders", Some(true), true));
    crate::menus::invoke(h.state_mut(), &ctx, "window.toggle.toolSliders", json!({})).unwrap();
    h.run_steps(2);
    assert!(!h.state().ui.panels.tool_sliders);
    assert_eq!(item(&h).checked, Some(false));
    assert!(h.query_by_label("Brush Size").is_none(), "closed");
    crate::menus::invoke(h.state_mut(), &ctx, "window.toggle.toolSliders", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label("Brush Size").is_some(), "open again");
    // It sits with the other Window panels, after Tool Presets.
    let items = crate::menus::menu_items(h.state());
    let at = |id: &str| items.iter().position(|i| i.id == id).unwrap();
    assert_eq!(at("window.toggle.toolSliders"), at("window.panel.toolPresets") + 1);
}

#[test]
fn panel_menu_moves_and_closes_the_panel() {
    let mut h = harness(SlidersDock::Left, vec2(400.0, 600.0));
    h.get_by_label("Tool Sliders menu").click();
    h.run_steps(3);
    h.get_by_label("Dock Bottom").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_sliders.dock, SlidersDock::Bottom);
    let size = h.get_by_label("Brush Size").rect();
    assert!(size.width() > size.height(), "now a row: {size:?}");
    h.get_by_label("Tool Sliders menu").click();
    h.run_steps(3);
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(!h.state().ui.panels.tool_sliders);
}

#[test]
fn placement_is_saved_with_the_workspace_and_hostile_values_are_tamed() {
    let mut a = app(SlidersDock::Floating);
    let saved = json!({"toolSliders": {"dock": "top", "row": 1e30, "column": f64::NAN.to_string(), "floating": [-5.0, 9e9]}, "panels": {"toolSliders": true, "layers": true, "history": false, "properties": true, "color": true, "navigator": false, "toolbar": true, "optionsBar": true, "statusBar": true}});
    crate::dock::apply(&mut a, &saved);
    // An invalid part (column is a string) leaves the layout alone.
    assert_eq!(a.ui.tool_sliders.dock, SlidersDock::Floating);
    let saved = json!({"toolSliders": {"dock": "top", "row": 1e30, "floating": [-5.0, 9e9]}});
    crate::dock::apply(&mut a, &saved);
    assert_eq!(a.ui.tool_sliders.dock, SlidersDock::Top);
    // Drawing clamps them.
    let mut h = harness_with(a, vec2(800.0, 600.0));
    assert!(h.state().ui.tool_sliders.row <= MAX_THICK);
    h.state_mut().ui.tool_sliders.dock = SlidersDock::Floating;
    h.run_steps(4);
    let [w, hh] = h.state().ui.tool_sliders.floating;
    assert!((MIN_THICK..=MAX_LONG).contains(&w) && (MIN_THICK..=MAX_LONG).contains(&hh), "{w} {hh}");
}

#[test]
fn dock_target_is_the_nearest_edge_within_the_zone() {
    let work = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
    let at = |x: f32, y: f32| snap_target(pos2(x, y), work);
    assert_eq!(at(400.0, 300.0), None, "mid-canvas floats");
    assert_eq!(at(130.0, 300.0), Some(SlidersDock::Left));
    assert_eq!(at(80.0, 300.0), Some(SlidersDock::Left), "just past the edge still docks");
    assert_eq!(at(880.0, 300.0), Some(SlidersDock::Right));
    assert_eq!(at(400.0, 70.0), Some(SlidersDock::Top));
    assert_eq!(at(400.0, 640.0), Some(SlidersDock::Bottom));
    // In a corner the closer edge wins.
    assert_eq!(at(105.0, 80.0), Some(SlidersDock::Left));
    assert_eq!(at(130.0, 52.0), Some(SlidersDock::Top));
    // Far outside the work area, NaN or an empty area never docks.
    assert_eq!(at(10.0, 300.0), None);
    assert_eq!(at(400.0, 760.0), None);
    assert_eq!(at(f32::NAN, 300.0), None);
    assert_eq!(snap_target(pos2(5.0, 5.0), Rect::NOTHING), None);
    // The highlight is the docked column or row.
    let layout = ToolSlidersLayout::default();
    assert_eq!(docked_rect(&layout, work, SlidersDock::Bottom), Rect::from_min_max(pos2(100.0, 554.0), pos2(900.0, 650.0)));
    assert_eq!(docked_rect(&layout, work, SlidersDock::Left).width(), 128.0);
}

/// The floating panel's tab strip (left part, clear of the ⋯ button).
fn strip(h: &Harness<'static, PrismApp>) -> egui::Pos2 {
    let r = strip_rect(h);
    pos2(r.left() + 30.0, r.center().y)
}

/// The tab strip's rect (the egui window carries the same label, but is taller).
fn strip_rect(h: &Harness<'static, PrismApp>) -> Rect {
    h.query_all_by_label("Tool Sliders").map(|n| n.rect()).find(|r| (r.height() - HEADER).abs() < 0.5).expect("tab strip")
}

#[test]
fn dropping_the_strip_near_an_edge_docks_there() {
    let mut h = harness(SlidersDock::Floating, vec2(1000.0, 700.0));
    let from = strip(&h);
    let n = h.state().session.journal.len();
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    // Mid-canvas: follows the pointer, no snap.
    let mid = pos2(500.0, 300.0);
    h.hover_at(mid);
    h.run_steps(2);
    assert_eq!(dragging(&h.ctx), Some(None));
    // Near the left edge: a drop would dock there; the panel still just follows the pointer.
    let near = pos2(25.0, 300.0);
    h.hover_at(near);
    h.run_steps(2);
    assert_eq!(dragging(&h.ctx), Some(Some(SlidersDock::Left)));
    let pos = h.state().ui.tool_sliders.pos.unwrap();
    // Held 30 points into the strip: the panel sits where the pointer put it, not pulled to x = 0.
    assert_eq!(pos[0], near.x - 30.0, "{pos:?}");
    h.drop_at(near);
    h.run_steps(3);
    assert_eq!(dragging(&h.ctx), None);
    assert_eq!(h.state().ui.tool_sliders.dock, SlidersDock::Left);
    let size = h.get_by_label("Brush Size").rect();
    assert!(size.height() > size.width(), "docked as a column: {size:?}");
    assert_eq!(h.state().session.journal.len(), n, "moving the panel edits no brush");
}

#[test]
fn dragging_a_docked_strip_tears_it_off() {
    let mut h = harness(SlidersDock::Bottom, vec2(1000.0, 700.0));
    let from = strip(&h);
    let to = pos2(500.0, 250.0);
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for k in 1..=6 {
        h.hover_at(from + (to - from) * (k as f32 / 6.0));
        h.run();
    }
    assert_eq!(h.state().ui.tool_sliders.dock, SlidersDock::Floating, "torn off");
    h.drop_at(to);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_sliders.dock, SlidersDock::Floating, "let go mid-canvas: stays floating");
    let r = strip_rect(&h);
    assert!(r.contains(to) || (r.center().y - to.y).abs() < 20.0, "under the pointer: {r:?}");
}

#[test]
fn a_stored_position_off_screen_is_brought_back() {
    let mut a = app(SlidersDock::Floating);
    a.ui.tool_sliders.pos = Some([-1e9, 1e9]);
    let h = harness_with(a, vec2(1000.0, 700.0));
    let r = strip_rect(&h);
    // 40 points of the window (the strip plus its 26-point ⋯ button) stay on screen.
    assert!(r.right() + 26.0 >= 39.0 && r.top() <= 700.0 - HEADER + 2.0, "{r:?}");
}

#[test]
fn drops_at_the_panel_groups_edge_dock_beside_them() {
    let work = Rect::from_min_max(pos2(0.0, 60.0), pos2(1000.0, 780.0));
    assert_eq!(snap_target_at(pos2(1130.0, 400.0), work, Some(1140.0)), Some(SlidersDock::Panels));
    assert_eq!(snap_target_at(pos2(1180.0, 400.0), work, Some(1140.0)), Some(SlidersDock::Panels));
    assert_eq!(snap_target_at(pos2(990.0, 400.0), work, Some(1140.0)), Some(SlidersDock::Right));
    assert_eq!(snap_target_at(pos2(1130.0, 400.0), work, None), None);
    assert_eq!(snap_target_at(pos2(1150.0, 20.0), work, Some(1140.0)), None);
    assert_eq!(snap_target_at(pos2(f32::NAN, 400.0), work, Some(1140.0)), None);
    assert!(SlidersDock::ALL.contains(&SlidersDock::Panels));
    assert_eq!(serde_json::to_value(SlidersDock::Panels).unwrap(), "panels");
}

#[test]
fn the_dock_preview_is_a_thin_line_along_the_edge() {
    let work = Rect::from_min_max(pos2(0.0, 60.0), pos2(1000.0, 780.0));
    for d in [SlidersDock::Left, SlidersDock::Right, SlidersDock::Panels] {
        let l = snap_line(work, d, Some(1140.0));
        assert!(l.width() <= 4.0 && l.height() == work.height(), "{d:?}: {l:?}");
    }
    for d in [SlidersDock::Top, SlidersDock::Bottom] {
        let l = snap_line(work, d, None);
        assert!(l.height() <= 4.0 && l.width() == work.width(), "{d:?}: {l:?}");
    }
    assert_eq!(snap_line(work, SlidersDock::Right, None).right(), 1000.0);
    assert_eq!(snap_line(work, SlidersDock::Panels, Some(1140.0)).center().x, 1140.0);
    assert_eq!(snap_line(work, SlidersDock::Panels, Some(f32::NAN)).center().x, 1000.0);
}
