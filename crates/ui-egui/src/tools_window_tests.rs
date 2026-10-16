use egui::{Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1200.0, 800.0);
/// Width of the stand-in panel dock at the right of the test window.
const DOCK_W: f32 = 260.0;

/// The chrome around the canvas as `PrismApp::ui` lays it out: the toolbar docked left, a panel
/// dock, the toolbar docked right, the drag tracker, the canvas and the floating window.
fn harness_with(app: PrismApp) -> Harness<'static, PrismApp> {
    harness_sized(app, SIZE)
}

fn harness_sized(app: PrismApp, size: egui::Vec2) -> Harness<'static, PrismApp> {
    let mut h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PrismApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let work = ui.available_rect_before_wrap();
            if app.ui.panels.toolbar && app.ui.tools.dock == ToolsDock::Left {
                crate::panels::toolbar(app, ui);
            }
            egui::Panel::right("test-dock").exact_size(DOCK_W).show(ui, |_| {});
            let right = ui.available_rect_before_wrap().right();
            if app.ui.panels.toolbar && app.ui.tools.dock == ToolsDock::Right {
                crate::panels::toolbar(app, ui);
            }
            track_drag(app, &ctx, work, right);
            egui::CentralPanel::default().show(ui, |_| {});
            floating(app, &ctx);
        },
        app,
    );
    PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.state_mut().ui.theme = crate::theme::ThemeKind::ProMedium;
    h.run_steps(4);
    h
}

fn harness() -> Harness<'static, PrismApp> {
    harness_with(PrismApp::new(prism_engine::Session::new(), crate::Services::default()))
}

/// Press at `from`, drag through to `to` in steps, release.
fn drag(h: &mut Harness<'static, PrismApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for k in 1..=8 {
        h.hover_at(from + (to - from) * (k as f32 / 8.0));
        h.run();
    }
    h.drop_at(to);
    h.run_steps(3);
}

/// The floating window's rect, if it is showing.
fn floating_rect(h: &Harness<'static, PrismApp>) -> Option<Rect> {
    h.query_by_label("Tools")?;
    h.ctx.memory(|m| m.area_rect(egui::Id::new("tools-floating")))
}

/// The docked toolbar's rect (one column, Pro), found from its header chevron.
fn toolbar_panel_rect(h: &Harness<'static, PrismApp>) -> Option<Rect> {
    if h.query_by_label("Tools").is_some() {
        return None;
    }
    let chevron = h.query_by_label("Toolbar")?.rect();
    Some(Rect::from_min_size(chevron.min - vec2(5.0, 8.0), vec2(40.0, 60.0)))
}

#[test]
fn drops_dock_only_at_the_vertical_edges() {
    let work = Rect::from_min_max(pos2(0.0, 60.0), pos2(1200.0, 780.0));
    let right = 940.0;
    assert_eq!(drop_target(pos2(5.0, 400.0), work, right), Some(ToolsDock::Left));
    assert_eq!(drop_target(pos2(DOCK_ZONE, 70.0), work, right), Some(ToolsDock::Left));
    assert_eq!(drop_target(pos2(930.0, 400.0), work, right), Some(ToolsDock::Right));
    // Over the panel dock still docks right, next to it.
    assert_eq!(drop_target(pos2(1150.0, 400.0), work, right), Some(ToolsDock::Right));
    // The top and bottom edges are no dock: the panel stays floating there.
    assert_eq!(drop_target(pos2(600.0, 62.0), work, right), None);
    assert_eq!(drop_target(pos2(600.0, 778.0), work, right), None);
    assert_eq!(drop_target(pos2(600.0, 400.0), work, right), None);
    // Outside the work area (options bar, status bar) and junk input float too.
    assert_eq!(drop_target(pos2(5.0, 20.0), work, right), None);
    assert_eq!(drop_target(pos2(5.0, 790.0), work, right), None);
    assert_eq!(drop_target(pos2(f32::NAN, 400.0), work, right), None);
    assert_eq!(drop_target(pos2(1190.0, 400.0), work, f32::NAN), Some(ToolsDock::Right));
    assert_eq!(drop_target(pos2(600.0, 400.0), Rect::NAN, right), None);
}

#[test]
fn stored_positions_are_kept_on_screen() {
    let screen = Rect::from_min_size(Pos2::ZERO, SIZE);
    assert_eq!(sane_pos([300.0, 200.0], 60.0, screen), pos2(300.0, 200.0));
    let far = sane_pos([1e9, -1e9], 60.0, screen);
    assert!(screen.contains(far) && far.x <= SIZE.x - 40.0 && far.y == 0.0, "{far:?}");
    let left = sane_pos([-5000.0, 5000.0], 60.0, screen);
    assert!(left.x + 60.0 >= 40.0 && left.y <= SIZE.y - TITLE_H, "{left:?}");
    for bad in [[f32::NAN, 0.0], [0.0, f32::INFINITY]] {
        assert!(sane_pos(bad, 60.0, screen).is_finite());
    }
    assert!(sane_pos([10.0, 10.0], f32::NAN, Rect::NAN).is_finite());
    assert!(sane_pos([10.0, 10.0], 60.0, Rect::from_min_size(Pos2::ZERO, vec2(10.0, 5.0))).is_finite());
}

#[test]
fn tear_off_float_and_dock_again_only_vertically() {
    let mut h = harness();
    assert_eq!(h.state().ui.tools.dock, ToolsDock::Left);
    let docked = toolbar_panel_rect(&h).expect("toolbar docked left");
    assert!(docked.left() < 10.0, "{docked:?}");
    assert!(floating_rect(&h).is_none());

    // Drag the header off the edge: the panel floats under the pointer.
    let grab = pos2(docked.right() - 6.0, docked.top() + 6.0);
    drag(&mut h, grab, pos2(500.0, 300.0));
    assert_eq!(h.state().ui.tools.dock, ToolsDock::Floating);
    assert!(toolbar_panel_rect(&h).is_none(), "no longer docked");
    let win = floating_rect(&h).expect("floating window");
    assert!((win.left() - (500.0 - (grab.x - docked.left()))).abs() < 3.0, "{win:?}");
    assert!((win.top() - (300.0 - (grab.y - docked.top()))).abs() < 3.0, "{win:?}");

    // Dropped at the top or bottom of the work area it stays floating (no horizontal dock).
    let title = |h: &Harness<'static, PrismApp>| {
        let r = floating_rect(h).expect("floating window");
        pos2(r.left() + 6.0, r.top() + TITLE_H / 2.0)
    };
    let from = title(&h);
    drag(&mut h, from, pos2(500.0, 3.0));
    assert_eq!(h.state().ui.tools.dock, ToolsDock::Floating);
    let from = title(&h);
    drag(&mut h, from, pos2(500.0, SIZE.y - 3.0));
    assert_eq!(h.state().ui.tools.dock, ToolsDock::Floating);

    // The right edge (the canvas side of the panel dock) docks it there.
    let from = title(&h);
    drag(&mut h, from, pos2(SIZE.x - DOCK_W - 10.0, 400.0));
    assert_eq!(h.state().ui.tools.dock, ToolsDock::Right);
    h.run_steps(2);
    let r = toolbar_panel_rect(&h).expect("toolbar docked right");
    let edge = SIZE.x - DOCK_W;
    assert!(r.left() < edge - 30.0 && r.left() > edge - 90.0, "between the canvas and the dock: {r:?}");
    assert!(floating_rect(&h).is_none());

    // Tear it off the right and drop it at the left edge.
    let grab = pos2(r.right() - 6.0, r.top() + 6.0);
    drag(&mut h, grab, pos2(4.0, 400.0));
    assert_eq!(h.state().ui.tools.dock, ToolsDock::Left);
    h.run_steps(2);
    assert!(toolbar_panel_rect(&h).is_some_and(|r| r.left() < 10.0));
}

#[test]
fn locked_workspace_keeps_the_panel_docked() {
    let mut h = harness();
    h.state_mut().session.prefs.edit(|p| p.workspace_locked = true);
    h.run_steps(2);
    let docked = toolbar_panel_rect(&h).unwrap();
    drag(&mut h, pos2(docked.right() - 6.0, docked.top() + 6.0), pos2(500.0, 300.0));
    assert_eq!(h.state().ui.tools.dock, ToolsDock::Left);
}

#[test]
fn floating_buttons_switch_columns_and_close() {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.ui.tools.dock = ToolsDock::Floating;
    // Tall enough for one column, so « decides.
    let mut h = harness_sized(app, vec2(1200.0, 1400.0));
    let w1 = floating_rect(&h).unwrap().width();
    h.get_by_label("Toolbar").click();
    h.run_steps(3);
    assert!(h.state().ui.panels.toolbar_double);
    let w2 = floating_rect(&h).unwrap().width();
    assert!(w2 > w1 + 20.0, "{w1} -> {w2}");
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(!h.state().ui.panels.toolbar);
    assert!(floating_rect(&h).is_none());
}

#[test]
fn window_tools_hides_and_shows_the_panel_where_it_was() {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.ui.tools = ToolsLayout { dock: ToolsDock::Floating, pos: [400.0, 250.0] };
    let mut h = harness_with(app);
    let ctx = h.ctx.clone();
    let before = floating_rect(&h).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "window.toggle.toolbar", json!({})).unwrap();
    h.run_steps(3);
    assert!(!h.state().ui.panels.toolbar);
    assert!(floating_rect(&h).is_none());
    crate::menus::invoke(h.state_mut(), &ctx, "window.toggle.toolbar", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(floating_rect(&h), Some(before));
    // Reset Workspace docks it back at the left.
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
    assert_eq!(h.state().ui.tools, ToolsLayout::default());
}

#[test]
fn placement_survives_a_restart_and_ui_set() {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.ui.tools = ToolsLayout { dock: ToolsDock::Floating, pos: [321.0, 222.0] };
    app.ui.panels.toolbar_double = true;
    let h = harness_with(app);
    // UI state round trip (`ui.inspect` / `ui.set`, workspaces).
    let saved = serde_json::to_value(&h.state().ui).unwrap();
    assert_eq!(saved["tools"], json!({"dock": "floating", "pos": [321.0, 222.0]}));
    let back: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(back.tools, h.state().ui.tools);
    // Old state without the field loads docked left.
    let mut old = serde_json::to_value(&h.state().ui).unwrap();
    old.as_object_mut().unwrap().remove("tools");
    let old: crate::state::UiState = serde_json::from_value(old).unwrap();
    assert_eq!(old.tools, ToolsLayout::default());

    // Remembered in the preferences and restored at the next launch.
    let mut h = h;
    let ctx = h.ctx.clone();
    crate::dock::persist(h.state_mut(), &ctx);
    let prefs = h.state().session.prefs_to_json();
    let mut s2 = prism_engine::Session::new();
    s2.load_prefs_json(&prefs).unwrap();
    let mut app2 = PrismApp::new(s2, crate::Services::default());
    crate::dock::restore(&mut app2);
    assert_eq!(app2.ui.tools, ToolsLayout { dock: ToolsDock::Floating, pos: [321.0, 222.0] });
    assert!(app2.ui.panels.toolbar_double);
    let h2 = harness_with(app2);
    let r = floating_rect(&h2).unwrap();
    assert_eq!(r.min, pos2(321.0, 222.0));

    // Junk from `ui.set` or a saved workspace is rejected or sanitised, never a panic.
    let bad: Result<ToolsLayout, _> = serde_json::from_value(json!({"dock": "top"}));
    assert!(bad.is_err(), "there is no horizontal dock");
    let mut app3 = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    crate::dock::apply(&mut app3, &json!({"tools": {"dock": "bottom"}}));
    assert_eq!(app3.ui.tools.dock, ToolsDock::Left);
    app3.ui.tools = ToolsLayout { dock: ToolsDock::Floating, pos: [f32::NAN, 1e30] };
    let h3 = harness_with(app3);
    assert!(floating_rect(&h3).is_some_and(|r| r.is_finite()));
}

#[test]
fn drops_at_the_panel_groups_edge_dock_beside_them() {
    let work = Rect::from_min_max(pos2(0.0, 60.0), pos2(1200.0, 780.0));
    let (right, edge) = (1100.0, 1140.0);
    // Just left of the groups' edge or a little into them: beside the groups.
    assert_eq!(drop_target_at(pos2(1130.0, 400.0), work, right, Some(edge)), Some(ToolsDock::Panels));
    assert_eq!(drop_target_at(pos2(1180.0, 400.0), work, right, Some(edge)), Some(ToolsDock::Panels));
    // Nearer the canvas it is still the canvas' right; without a dock the old targets apply.
    assert_eq!(drop_target_at(pos2(1090.0, 400.0), work, right, Some(edge)), Some(ToolsDock::Right));
    assert_eq!(drop_target_at(pos2(1130.0, 400.0), work, right, None), Some(ToolsDock::Right));
    // Never above or below the work area, and bad numbers are no target.
    assert_eq!(drop_target_at(pos2(1150.0, 20.0), work, right, Some(edge)), None);
    assert_eq!(drop_target_at(pos2(f32::NAN, 400.0), work, right, Some(edge)), None);
    assert_eq!(drop_target_at(pos2(1150.0, 400.0), work, right, Some(f32::NAN)), Some(ToolsDock::Right));
    let v = serde_json::to_value(ToolsLayout { dock: ToolsDock::Panels, pos: [0.0, 0.0] }).unwrap();
    assert_eq!(v["dock"], "panels");
}
