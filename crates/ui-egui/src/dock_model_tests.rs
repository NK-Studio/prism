//! Dock model tests: tearing tabs off, merging, stacking, docking columns, the Tools and Tool
//! Sliders rules, Window-menu toggles and persistence, without a window.

use serde_json::json;

use super::*;
use crate::dock::Group;
use crate::state::UiState;

const PRO: bool = true;

fn drop(ui: &mut UiState, payload: Payload, target: DropTarget) -> bool {
    let mut dt = ui.dock_tabs;
    let r = ui.dock.apply_drop(payload, target, &mut dt, PRO).unwrap();
    ui.dock_tabs = dt;
    r
}

fn paths_tab() -> Payload {
    Payload::Tab { group: Group::Layers, panel: PanelId::Paths }
}

#[test]
fn a_torn_off_tab_floats_in_a_window_of_its_own() {
    let mut ui = UiState::default();
    assert!(drop(&mut ui, paths_tab(), DropTarget::Float { pos: [400.0, 200.0] }));
    assert_eq!(ui.dock.tabs_of(Group::Layers, PRO), vec![PanelId::Layers, PanelId::Channels]);
    let f = ui.dock.floats.first().expect("a floating window");
    assert_eq!(f.pos, [400.0, 200.0]);
    let g = *f.groups.first().unwrap();
    assert!(matches!(g, Group::Custom(_)));
    assert_eq!(ui.dock.tabs_of(g, PRO), vec![PanelId::Paths]);
    assert_eq!(ui.dock.find_panel(PanelId::Paths, PRO), Some(g));
    assert_eq!(visible_groups(&ui, Slot::Float(0), PRO), vec![g]);
    // The custom group is not in the main column.
    assert!(!ui.dock.order().contains(&g));
}

#[test]
fn tabs_merge_into_other_groups_and_reorder() {
    let mut ui = UiState::default();
    assert!(drop(&mut ui, paths_tab(), DropTarget::Tabs { group: Group::Color, index: 1 }));
    assert_eq!(ui.dock.tabs_of(Group::Color, PRO), vec![PanelId::Color, PanelId::Paths, PanelId::Swatches, PanelId::Gradients, PanelId::Patterns]);
    let tabs = ui.dock.tabs_of(Group::Color, PRO);
    assert_eq!(ui.dock.active_panel(Group::Color, &tabs, &ui.dock_tabs, PRO), Some(PanelId::Paths), "the dropped tab is selected");
    // Reorder within a strip: Channels to the front of Layers.
    let ch = Payload::Tab { group: Group::Layers, panel: PanelId::Channels };
    assert!(drop(&mut ui, ch, DropTarget::Tabs { group: Group::Layers, index: 0 }));
    assert_eq!(ui.dock.tabs_of(Group::Layers, PRO), vec![PanelId::Channels, PanelId::Layers]);
    // Dropping a tab where it already is changes nothing.
    assert!(!drop(&mut ui, ch, DropTarget::Tabs { group: Group::Layers, index: 1 }));
    // A whole group into another's strip: its tabs join, the emptied group draws nothing.
    assert!(drop(&mut ui, Payload::Group(Group::History), DropTarget::Tabs { group: Group::Layers, index: 9 }));
    assert_eq!(ui.dock.tabs_of(Group::Layers, PRO), vec![PanelId::Channels, PanelId::Layers, PanelId::History, PanelId::Actions, PanelId::LayerComps]);
    ui.panels.history = true;
    assert!(!visible_groups(&ui, Slot::Main, PRO).contains(&Group::History));
    // Paths is selected in Color through `active`; picking a default tab goes back to DockTabs.
    let mut dt = ui.dock_tabs;
    ui.dock.set_active(Group::Color, PanelId::Swatches, &mut dt, PRO);
    assert_eq!((dt.color, ui.dock.active.get(&Group::Color)), (1, None));
}

#[test]
fn groups_stack_above_and_below_each_other() {
    let mut ui = UiState::default();
    assert!(drop(&mut ui, Payload::Group(Group::Layers), DropTarget::Stack { anchor: Group::Color, after: false }));
    assert_eq!(ui.dock.order().first(), Some(&Group::Layers));
    assert!(drop(&mut ui, Payload::Group(Group::Layers), DropTarget::Stack { anchor: Group::Properties, after: true }));
    let order = ui.dock.order();
    let at = |g| order.iter().position(|x| *x == g).unwrap();
    assert_eq!(at(Group::Layers), at(Group::Properties) + 1);
    // On itself: nothing.
    assert!(!drop(&mut ui, Payload::Group(Group::Layers), DropTarget::Stack { anchor: Group::Layers, after: true }));
    // A tab dropped below a group makes a group of its own there.
    assert!(drop(&mut ui, paths_tab(), DropTarget::Stack { anchor: Group::Color, after: true }));
    let order = ui.dock.order();
    let new = order.get(order.iter().position(|g| *g == Group::Color).unwrap() + 1).copied().unwrap();
    assert_eq!(ui.dock.tabs_of(new, PRO), vec![PanelId::Paths]);
    // Its only tab dropped beside itself: nothing.
    assert!(!drop(&mut ui, Payload::Tab { group: new, panel: PanelId::Paths }, DropTarget::Stack { anchor: new, after: false }));
}

#[test]
fn groups_dock_as_columns_left_right_and_bottom() {
    let mut ui = UiState::default();
    ui.panels.history = true;
    assert!(drop(&mut ui, Payload::Group(Group::Color), DropTarget::Column { side: Side::Left, index: 0 }));
    assert!(drop(&mut ui, Payload::Group(Group::History), DropTarget::Column { side: Side::Bottom, index: 0 }));
    assert!(drop(&mut ui, paths_tab(), DropTarget::Column { side: Side::Right, index: 0 }));
    let sides: Vec<Side> = ui.dock.columns.iter().map(|c| c.side).collect();
    assert_eq!(sides, vec![Side::Left, Side::Bottom, Side::Right]);
    assert_eq!(ui.dock.columns[0].groups, vec![Group::Color]);
    assert!(!ui.dock.order().contains(&Group::Color) && !ui.dock.order().contains(&Group::History));
    assert_eq!(visible_groups(&ui, Slot::Column(1), PRO), vec![Group::History]);
    // A second left column goes outside the first with index 0.
    assert!(drop(&mut ui, Payload::Group(Group::Layers), DropTarget::Column { side: Side::Left, index: 0 }));
    assert_eq!(ui.dock.columns[0].groups, vec![Group::Layers]);
    // Stacking into a column, then emptying it, removes the column.
    assert!(drop(&mut ui, Payload::Group(Group::Properties), DropTarget::Stack { anchor: Group::Color, after: true }));
    assert_eq!(ui.dock.columns[1].groups, vec![Group::Color, Group::Properties]);
    assert!(drop(&mut ui, Payload::Group(Group::Color), DropTarget::Stack { anchor: Group::Character, after: false }));
    assert!(drop(&mut ui, Payload::Group(Group::Properties), DropTarget::Stack { anchor: Group::Character, after: false }));
    assert_eq!(ui.dock.columns.iter().filter(|c| c.side == Side::Left).count(), 1);
    assert!(ui.dock.order().contains(&Group::Color));
}

#[test]
fn the_tools_panel_never_joins_or_takes_tabs_and_docks_only_at_the_sides() {
    let tabs = DropTarget::Tabs { group: Group::Layers, index: 0 };
    let stack = DropTarget::Stack { anchor: Group::Layers, after: false };
    assert!(!allowed(DragKind::Tools, &tabs));
    assert!(!allowed(DragKind::Tools, &stack));
    assert!(!allowed(DragKind::Tools, &DropTarget::Column { side: Side::Bottom, index: 0 }));
    assert!(!allowed(DragKind::Tools, &DropTarget::Float { pos: [0.0, 0.0] }));
    assert!(allowed(DragKind::Tools, &DropTarget::Column { side: Side::Left, index: 0 }));
    assert!(allowed(DragKind::Tools, &DropTarget::Column { side: Side::Right, index: 0 }));
    // Tools is not a panel: no group can hold it, so nothing can be merged into it either.
    assert!(PanelId::ALL.iter().all(|p| !p.label().eq_ignore_ascii_case("tools")));
    // Panels and groups go anywhere.
    for t in [tabs, stack, DropTarget::Column { side: Side::Bottom, index: 0 }] {
        assert!(allowed(DragKind::Panel, &t) && allowed(DragKind::Group, &t));
    }
}

#[test]
fn tool_sliders_joins_groups_like_any_panel() {
    let mut ui = UiState::default();
    ui.panels.tool_sliders = true;
    assert!(drop(&mut ui, Payload::Panel(PanelId::ToolSliders), DropTarget::Tabs { group: Group::Layers, index: 3 }));
    assert_eq!(ui.dock.tabs_of(Group::Layers, PRO).last(), Some(&PanelId::ToolSliders));
    assert!(sliders_grouped(&ui));
    // Torn off again it floats like any tab.
    assert!(drop(&mut ui, Payload::Tab { group: Group::Layers, panel: PanelId::ToolSliders }, DropTarget::Float { pos: [10.0, 10.0] }));
    assert_eq!(ui.dock.floats.len(), 1);
    // Closing Tool Sliders from the Window menu takes it out of the dock.
    ui.panels.tool_sliders = false;
    sync(&mut ui);
    assert!(!sliders_grouped(&ui) && ui.dock.floats.is_empty());
    // Window › Tool Sliders through the dock: a floating window of its own.
    assert!(toggle_panel(&mut ui, PanelId::ToolSliders, PRO, None));
    assert!(ui.panels.tool_sliders && sliders_grouped(&ui) && ui.dock.floats.len() == 1);
}

#[test]
fn window_menu_finds_panels_wherever_they_are() {
    let mut ui = UiState::default();
    drop(&mut ui, paths_tab(), DropTarget::Float { pos: [300.0, 300.0] });
    assert!(panel_showing(&ui, PanelId::Paths, PRO));
    // Showing: the menu item hides its group, then brings it back.
    assert!(!toggle_panel(&mut ui, PanelId::Paths, PRO, None));
    assert!(visible_groups(&ui, Slot::Float(0), PRO).is_empty());
    assert!(toggle_panel(&mut ui, PanelId::Paths, PRO, None));
    assert_eq!(visible_groups(&ui, Slot::Float(0), PRO).len(), 1);
    // A closed tab goes home, in its default place.
    let g = ui.dock.find_panel(PanelId::Paths, PRO).unwrap();
    close_panel(&mut ui, g, PanelId::Paths, PRO);
    assert!(ui.dock.floats.is_empty() && ui.dock.find_panel(PanelId::Paths, PRO).is_none());
    assert!(toggle_panel(&mut ui, PanelId::Paths, PRO, None));
    assert_eq!(ui.dock.tabs_of(Group::Layers, PRO), vec![PanelId::Layers, PanelId::Channels, PanelId::Paths]);
    assert!(panel_showing(&ui, PanelId::Paths, PRO));
    // A collapsed group is expanded rather than hidden.
    ui.dock.set_collapsed(Group::Layers, true);
    assert!(toggle_panel(&mut ui, PanelId::Paths, PRO, None) && !ui.dock.is_collapsed(Group::Layers));
    // Closing every tab of an Essentials group closes it; the menu refills it with one tab.
    for p in [PanelId::Layers, PanelId::Channels, PanelId::Paths] {
        close_panel(&mut ui, Group::Layers, p, PRO);
    }
    assert!(!ui.panels.layers);
    assert!(toggle_panel(&mut ui, PanelId::Channels, PRO, None));
    assert!(ui.panels.layers && ui.dock.tabs_of(Group::Layers, PRO) == vec![PanelId::Channels]);
}

#[test]
fn floating_windows_dock_back_and_merge() {
    let mut ui = UiState::default();
    drop(&mut ui, paths_tab(), DropTarget::Float { pos: [300.0, 300.0] });
    let id = ui.dock.floats[0].id;
    let g = ui.dock.floats[0].groups[0];
    // Another tab joins the floating window's group, another group stacks under it.
    drop(&mut ui, Payload::Tab { group: Group::Layers, panel: PanelId::Channels }, DropTarget::Tabs { group: g, index: 1 });
    let h = ui.dock.floats[0].size[1];
    drop(&mut ui, Payload::Group(Group::Color), DropTarget::Stack { anchor: g, after: true });
    assert_eq!(ui.dock.floats[0].groups, vec![g, Group::Color]);
    assert!(ui.dock.floats[0].size[1] > h, "the window grows for the stacked group");
    // The whole window into the Layers strip: every tab joins, the window goes.
    assert!(drop(&mut ui, Payload::Float(id), DropTarget::Tabs { group: Group::Layers, index: 1 }));
    assert!(ui.dock.floats.is_empty());
    assert_eq!(ui.dock.tabs_of(Group::Layers, PRO)[..3], [PanelId::Layers, PanelId::Paths, PanelId::Channels]);
    assert!(ui.dock.tabs.keys().all(|k| !matches!(k, Group::Custom(_))), "emptied custom groups are gone: {:?}", ui.dock.tabs);
}

#[test]
fn layouts_survive_a_save_and_restore() {
    let mut ui = UiState::default();
    drop(&mut ui, paths_tab(), DropTarget::Float { pos: [300.0, 300.0] });
    drop(&mut ui, Payload::Group(Group::Color), DropTarget::Column { side: Side::Left, index: 0 });
    ui.dock.floats[0].iconic = true;
    ui.dock.main_iconic = true;
    ui.dock.set_collapsed(Group::Layers, true);
    let v = serde_json::to_value(&ui.dock).unwrap();
    assert_eq!(v["floats"][0]["groups"][0].as_str().map(|s| s.starts_with('g')), Some(true), "{v}");
    let back: crate::dock::DockLayout = serde_json::from_value(v).unwrap();
    assert_eq!(back, ui.dock);
    // The whole UI state round-trips too.
    let s = serde_json::to_value(&ui).unwrap();
    let back: UiState = serde_json::from_value(s).unwrap();
    assert_eq!(back.dock, ui.dock);
}

#[test]
fn bad_drops_and_bad_layouts_are_errors_not_crashes() {
    let mut ui = UiState::default();
    let mut dt = ui.dock_tabs;
    let l = &mut ui.dock;
    assert!(l.apply_drop(Payload::Group(Group::Custom(99)), DropTarget::Float { pos: [0.0, 0.0] }, &mut dt, PRO).is_err());
    assert!(l.apply_drop(paths_tab(), DropTarget::Tabs { group: Group::Custom(7), index: 0 }, &mut dt, PRO).is_err());
    assert!(l.apply_drop(paths_tab(), DropTarget::Float { pos: [f32::NAN, 0.0] }, &mut dt, PRO).is_err());
    assert!(l.apply_drop(Payload::Tab { group: Group::Color, panel: PanelId::Paths }, DropTarget::Float { pos: [0.0, 0.0] }, &mut dt, PRO).is_err());
    assert!(l.apply_drop(Payload::Float(42), DropTarget::Float { pos: [0.0, 0.0] }, &mut dt, PRO).is_err());
    assert!(l.apply_drop(paths_tab(), DropTarget::Tabs { group: Group::Layers, index: usize::MAX }, &mut dt, PRO).is_ok());
    assert_eq!(*l, {
        let mut want = crate::dock::DockLayout::default();
        want.set_tabs(Group::Layers, vec![PanelId::Layers, PanelId::Channels, PanelId::Paths]);
        want.active.clear();
        want
    });
    assert_eq!(Group::from_key("gx"), None);
    assert!(serde_json::from_value::<crate::dock::DockLayout>(json!({"order": ["nope"]})).is_err());
    // Garbage columns and windows load and draw nothing (cleanup drops them).
    let mut l: crate::dock::DockLayout =
        serde_json::from_value(json!({"columns": [{"groups": ["g5", "g5"], "size": "x"}], "floats": [{"groups": ["layers", "layers"], "pos": [1e30, -1e30]}]})).unwrap_or_default();
    l.cleanup();
    assert!(l.columns.is_empty());
    assert!(l.floats.iter().all(|f| f.groups.len() <= 1));
    // Hundreds of drops never grow without bound.
    let mut ui = UiState::default();
    for i in 0..300 {
        let mut dt = ui.dock_tabs;
        let p = PanelId::ALL[i % 17];
        let _ = ui.dock.apply_drop(Payload::Panel(p), DropTarget::Float { pos: [i as f32, 0.0] }, &mut dt, PRO);
    }
    assert!(ui.dock.floats.len() <= 64);
}
