//! Docking driven through the whole app with real pointer drags: tabs torn off into floating
//! windows, merged into other groups, groups stacked and docked left/right/bottom, columns
//! collapsed to icons, the Tools and Tool Sliders rules, and the layout surviving a restart.

use egui::{Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::PrismApp;
use crate::dock::{Group, last_rects, last_strips};
use crate::dock_model::{PanelId, Side};

fn app() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app
}

fn harness(app: PrismApp) -> Harness<'static, PrismApp> {
    let mut slot = Some(app);
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, Default::default());
        slot.take().unwrap_or_else(self::app)
    });
    h.run_steps(8);
    h
}

fn drag(h: &mut Harness<'static, PrismApp>, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
}

fn click(h: &mut Harness<'static, PrismApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

/// The rect of `panel`'s tab on its group's strip.
fn tab_rect(h: &Harness<'static, PrismApp>, panel: PanelId) -> Rect {
    let pro = crate::theme::Tokens::get(&h.ctx).pro;
    let g = h.state().ui.dock.find_panel(panel, pro).unwrap_or_else(|| panic!("{panel:?} in no group"));
    let tabs = h.state().ui.dock.tabs_of(g, pro);
    let i = tabs.iter().position(|p| *p == panel).unwrap();
    let strip = last_strips(&h.ctx).into_iter().find(|s| s.group == g).unwrap_or_else(|| panic!("{g:?} strip not drawn"));
    strip.tabs.iter().find(|(k, _)| *k == i).map(|(_, r)| *r).unwrap_or_else(|| panic!("{panel:?} tab not on the strip"))
}

fn group_rect(h: &Harness<'static, PrismApp>, g: Group) -> Rect {
    last_rects(&h.ctx).into_iter().find(|(x, _)| *x == g).map(|(_, r)| r).unwrap_or_else(|| panic!("{g:?} not drawn"))
}

/// The background of `g`'s tab strip, right of its tabs.
fn strip_background(h: &Harness<'static, PrismApp>, g: Group) -> Pos2 {
    let s = last_strips(&h.ctx).into_iter().find(|s| s.group == g).unwrap();
    let right = s.tabs.iter().map(|(_, r)| r.right()).fold(f32::MIN, f32::max);
    assert!(s.menu.left() - right > 6.0, "{g:?}'s tabs fill its strip");
    Pos2::new((right + s.menu.left()) / 2.0, s.menu.center().y)
}

#[test]
fn dragging_a_tab_off_its_strip_makes_a_floating_window() {
    let mut h = harness(app());
    let from = tab_rect(&h, PanelId::Paths).center();
    let canvas = h.state().last_canvas_rect;
    drag(&mut h, from, canvas.center());
    let app = h.state();
    assert_eq!(app.ui.dock.floats.len(), 1, "{:?}", app.ui.dock);
    let g = app.ui.dock.floats[0].groups[0];
    assert_eq!(app.ui.dock.tabs_of(g, true), vec![PanelId::Paths]);
    // The window is drawn there, with its group.
    let r = group_rect(&h, g);
    assert!(canvas.contains(r.center()), "{r:?} outside the canvas {canvas:?}");
    // Its title bar drags it, and back onto the Layers strip it docks as a tab again.
    let title = Pos2::new(r.left() + 20.0, r.top() - 8.0);
    let layers = tab_rect(&h, PanelId::Layers);
    drag(&mut h, title, Pos2::new(layers.right() + 2.0, layers.center().y));
    let app = h.state();
    assert!(app.ui.dock.floats.is_empty(), "{:?}", app.ui.dock);
    assert!(app.ui.dock.tabs_of(Group::Layers, true).contains(&PanelId::Paths));
}

#[test]
fn a_tab_dropped_on_another_strip_joins_that_group() {
    let mut h = harness(app());
    let from = tab_rect(&h, PanelId::Paths).center();
    // Right of the middle of Color's last tab: after it.
    let last = tab_rect(&h, PanelId::Patterns);
    drag(&mut h, from, Pos2::new(last.right() - 3.0, last.center().y));
    let tabs = h.state().ui.dock.tabs_of(Group::Color, true);
    assert_eq!(tabs.last(), Some(&PanelId::Paths), "{tabs:?}");
    assert_eq!(h.state().ui.dock.tabs_of(Group::Layers, true), vec![PanelId::Layers, PanelId::Channels]);
    // Paths is now Color's selected tab, and drawn there.
    assert!(crate::dock_model::panel_showing(&h.state().ui, PanelId::Paths, true));
}

#[test]
fn groups_stack_and_dock_left_right_and_bottom() {
    let mut h = harness(app());
    // Layers dragged by its strip onto the top half of Color: stacked above it.
    let color = group_rect(&h, Group::Color);
    let from = strip_background(&h, Group::Layers);
    drag(&mut h, from, Pos2::new(color.center().x, color.top() + color.height() * 0.3));
    assert_eq!(h.state().ui.dock.order().first(), Some(&Group::Layers), "{:?}", h.state().ui.dock.order());
    // Properties to the canvas' left edge: a new left column.
    let canvas = h.state().last_canvas_rect;
    let from = strip_background(&h, Group::Properties);
    drag(&mut h, from, Pos2::new(canvas.left() + 6.0, canvas.center().y));
    let cols = h.state().ui.dock.columns.clone();
    assert_eq!(cols.len(), 1, "{cols:?}");
    assert_eq!((cols[0].side, cols[0].groups.clone()), (Side::Left, vec![Group::Properties]));
    let r = group_rect(&h, Group::Properties);
    assert!(r.right() <= h.state().last_canvas_rect.left() + 1.0, "{r:?} is left of the canvas");
    // Layers to the bottom edge: a bottom row.
    let canvas = h.state().last_canvas_rect;
    let from = strip_background(&h, Group::Layers);
    drag(&mut h, from, Pos2::new(canvas.center().x, canvas.bottom() - 6.0));
    let bottom = h.state().ui.dock.columns.iter().find(|c| c.side == Side::Bottom).cloned().expect("a bottom row");
    assert_eq!(bottom.groups, vec![Group::Layers]);
    assert!(group_rect(&h, Group::Layers).top() >= h.state().last_canvas_rect.bottom() - 1.0);
    // The Swatches tab to the canvas' right edge: a right column inside the main dock.
    let canvas = h.state().last_canvas_rect;
    let from = tab_rect(&h, PanelId::Swatches).center();
    drag(&mut h, from, Pos2::new(canvas.right() - 6.0, canvas.center().y));
    let right = h.state().ui.dock.columns.iter().find(|c| c.side == Side::Right).cloned().expect("a right column");
    assert_eq!(h.state().ui.dock.tabs_of(right.groups[0], true), vec![PanelId::Swatches]);
    let r = group_rect(&h, right.groups[0]);
    assert!(r.left() >= h.state().last_canvas_rect.right() - 1.0 && r.right() <= group_rect(&h, Group::Color).left(), "{r:?}");
    // Never at the top: a drop near the canvas top floats instead.
    let canvas = h.state().last_canvas_rect;
    let from = tab_rect(&h, PanelId::Channels).center();
    drag(&mut h, from, Pos2::new(canvas.center().x, canvas.top() + 4.0));
    assert!(h.state().ui.dock.columns.iter().all(|c| c.groups.iter().all(|g| !h.state().ui.dock.tabs_of(*g, true).contains(&PanelId::Channels))));
    assert_eq!(h.state().ui.dock.floats.len(), 1);
}

#[test]
fn columns_collapse_to_icons_and_open_flyouts() {
    let mut h = harness(app());
    let layers = group_rect(&h, Group::Layers);
    // The bar above the main column: » collapses it to icons.
    let toggle = Pos2::new(layers.right() - 8.0, last_rects(&h.ctx).iter().map(|(_, r)| r.top()).fold(f32::MAX, f32::min) - 10.0);
    click(&mut h, toggle);
    assert!(h.state().ui.dock.main_iconic, "clicked {toggle:?}");
    let icons = crate::dock_float::last_icons(&h.ctx);
    let (g, _, row) = icons.iter().find(|(_, p, _)| *p == PanelId::Channels).copied().expect("a Channels icon");
    assert_eq!(g, Group::Layers);
    click(&mut h, row.center());
    let fl = crate::dock_float::open_flyout(&h.ctx).expect("a flyout");
    assert_eq!(fl.group, Group::Layers);
    assert!(crate::dock_model::panel_showing(&h.state().ui, PanelId::Channels, true));
    // The flyout opens left of the icon column, and draws the group.
    let r = group_rect(&h, Group::Layers);
    assert!(r.right() <= row.left(), "{r:?} beside {row:?}");
    // A click elsewhere closes it.
    let canvas = h.state().last_canvas_rect;
    click(&mut h, canvas.center());
    assert!(crate::dock_float::open_flyout(&h.ctx).is_none());
}

#[test]
fn the_tools_panel_never_lands_in_a_panel_group() {
    let mut h = harness(app());
    let before = h.state().ui.dock.clone();
    // Its header row tears it off; dropped on the Layers tab strip it docks at the right, as a
    // column of its own, and the group is untouched.
    let to = tab_rect(&h, PanelId::Layers).center();
    // The header row's column chevrons tear it off like the rest of the header.
    let header = h.get_by_label("Toolbar").rect();
    drag(&mut h, header.center(), to);
    assert_eq!(h.state().ui.dock, before);
    assert_ne!(h.state().ui.tools.dock, crate::tools_window::ToolsDock::Left, "the drag moved the Tools panel");
    assert!(h.state().ui.dock.tabs.is_empty());
}

#[test]
fn tool_sliders_docks_into_a_group_as_a_tab() {
    let mut a = app();
    a.ui.panels.tool_sliders = true;
    a.ui.tool_sliders.pos = Some([500.0, 200.0]);
    let mut h = harness(a);
    // Its own floating window's strip, dropped on the Layers strip.
    let to = strip_background(&h, Group::Layers);
    drag(&mut h, Pos2::new(530.0, 212.0), to);
    let tabs = h.state().ui.dock.tabs_of(Group::Layers, true);
    assert_eq!(tabs.last(), Some(&PanelId::ToolSliders), "{tabs:?} {:?}", h.state().ui.tool_sliders);
    assert!(crate::dock_model::sliders_grouped(&h.state().ui));
    // Selected, it draws in the group with its sliders laid out vertically or horizontally by shape.
    assert!(crate::dock_model::panel_showing(&h.state().ui, PanelId::ToolSliders, true));
    let r = group_rect(&h, Group::Layers);
    assert!(crate::tool_sliders::vertical(r.size()) == (r.height() > r.width()));
}

#[test]
fn the_layout_is_remembered_across_launches() {
    let mut h = harness(app());
    let from = tab_rect(&h, PanelId::Paths).center();
    let canvas = h.state().last_canvas_rect;
    drag(&mut h, from, canvas.center());
    let from = strip_background(&h, Group::Properties);
    drag(&mut h, from, Pos2::new(canvas.left() + 6.0, canvas.center().y));
    assert_eq!(h.state().ui.dock.columns.len(), 1);
    let ctx = h.ctx.clone();
    crate::dock::persist(h.state_mut(), &ctx);
    let saved = h.state().session.prefs().panel_layout.clone();
    let mut again = app();
    again.session.prefs.edit(|p| p.panel_layout = saved);
    crate::dock::restore(&mut again);
    assert_eq!(again.ui.dock, h.state().ui.dock);
    // Reset Workspace brings back the Photoshop arrangement.
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
    assert_eq!(h.state().ui.dock, crate::dock::DockLayout::default());
}

#[test]
fn ui_dock_drives_the_layout_and_reports_rects() {
    let mut h = harness(app());
    let ctx = h.ctx.clone();
    let call = |h: &mut Harness<'static, PrismApp>, p: serde_json::Value| crate::dock_float::control(h.state_mut(), &ctx, &p);
    let r = call(&mut h, json!({"drop": {"payload": {"tab": {"group": "layers", "panel": "channels"}}, "target": {"float": {"pos": [500, 300]}}}})).unwrap();
    assert_eq!(r["changed"], true);
    h.run_steps(3);
    let r = call(&mut h, json!({})).unwrap();
    assert_eq!(r["dock"]["floats"].as_array().map(Vec::len), Some(1), "{r}");
    assert!(r["rects"]["strips"].as_array().is_some_and(|s| s.iter().any(|s| s["tabs"].as_array().is_some_and(|t| t.iter().any(|t| t["panel"] == "channels")))), "{r}");
    call(&mut h, json!({"iconic": {"main": true, "on": true}})).unwrap();
    assert!(h.state().ui.dock.main_iconic);
    call(&mut h, json!({"toggle": "toolSliders"})).unwrap();
    assert!(crate::dock_model::sliders_grouped(&h.state().ui));
    for bad in [json!({"bogus": 1}), json!({"toggle": "nope"}), json!({"drop": {"payload": 3}}), json!({"iconic": {"column": 77, "on": true}}), json!({"close": "paths", "toggle": 5})] {
        assert!(call(&mut h, bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn the_panel_menu_floats_a_group() {
    let mut h = harness(app());
    let menu = last_strips(&h.ctx).into_iter().find(|s| s.group == Group::Layers).unwrap().menu;
    click(&mut h, menu.center());
    let item = h.query_by_label("Float Panel Group").or_else(|| h.query_by_label("패널 그룹 띄우기")).expect("the menu item").rect();
    click(&mut h, item.center());
    let app = h.state();
    assert_eq!(app.ui.dock.floats.len(), 1, "{:?}", app.ui.dock);
    assert_eq!(app.ui.dock.floats[0].groups, vec![Group::Layers]);
    assert!(!app.ui.dock.order().contains(&Group::Layers));
    // Drawn in its window, on screen.
    let r = group_rect(&h, Group::Layers);
    assert!(h.ctx.content_rect().intersects(r), "{r:?}");
}

#[test]
fn the_tools_panel_docks_between_the_panel_groups_and_the_icon_rail() {
    let mut h = harness(app());
    let layers = group_rect(&h, Group::Layers);
    let header = h.get_by_label("Toolbar").rect();
    drag(&mut h, header.center(), Pos2::new(layers.left() + 10.0, layers.center().y));
    assert_eq!(h.state().ui.tools.dock, crate::tools_window::ToolsDock::Panels);
    assert_eq!(h.state().ui.dock, crate::dock::DockLayout::default(), "no group took it");
    // Drawn right beside the groups: its tools sit between the canvas (and rail) and the groups.
    let tools = h.get_by_label("Toolbar").rect();
    let layers = group_rect(&h, Group::Layers);
    let canvas = h.state().last_canvas_rect;
    assert!(tools.right() <= layers.left() + 1.0 && tools.left() > canvas.right(), "{tools:?} {layers:?} {canvas:?}");
    // ⇧Tab hides the groups: it stays on screen at the canvas' right.
    h.state_mut().ui.panels.dock = false;
    h.run_steps(3);
    let tools = h.get_by_label("Toolbar").rect();
    assert!(tools.left() >= h.state().last_canvas_rect.right() - 1.0, "{tools:?}");
}

#[test]
fn tool_sliders_docks_between_the_panel_groups_and_the_icon_rail() {
    use crate::tool_sliders::SlidersDock;
    let mut a = app();
    a.ui.panels.tool_sliders = true;
    a.ui.tool_sliders.pos = Some([500.0, 200.0]);
    let mut h = harness(a);
    let layers = group_rect(&h, Group::Layers);
    // Its strip dropped on the groups' edge, below their tab strips: a column beside them.
    drag(&mut h, Pos2::new(530.0, 212.0), Pos2::new(layers.left() + 10.0, layers.center().y));
    assert_eq!(h.state().ui.tool_sliders.dock, SlidersDock::Panels);
    assert!(!crate::dock_model::sliders_grouped(&h.state().ui), "no group took it");
    let sliders = h.get_by_label("Tool Sliders").rect();
    let layers = group_rect(&h, Group::Layers);
    let canvas = h.state().last_canvas_rect;
    assert!(sliders.right() <= layers.left() + 1.0 && sliders.left() > canvas.right(), "{sliders:?} {layers:?} {canvas:?}");
    // Docked there it is tall: its sliders run vertically.
    assert!(crate::tool_sliders::vertical(vec2(128.0, canvas.height())));
    // ⇧Tab hides the groups: it stays on screen at the canvas' right.
    h.state_mut().ui.panels.dock = false;
    h.run_steps(3);
    assert!(h.get_by_label("Tool Sliders").rect().left() >= h.state().last_canvas_rect.right() - 1.0);
}
