//! The Path Selection tool (A) picks, moves, copies, deletes and nudges whole path components,
//! with hover feedback, as in Photoshop. It used to only move everything on release.

use egui::Modifiers;
use prism_doc::vector::Path;
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

fn app() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PathSelection;
    app
}

/// A work path of two components: a square at (20..60), and a square at (100..180) with a hole
/// at (120..160) joined to it.
fn with_paths(app: &mut PrismApp) {
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[20, 20], [60, 20], [60, 60], [20, 60]]},
        {"closed": true, "knots": [[100, 100], [180, 100], [180, 180], [100, 180]]},
        {"closed": true, "op": "join", "knots": [[120, 120], [160, 120], [160, 160], [120, 160]]},
    ]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

fn work(app: &PrismApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn first(app: &PrismApp, s: usize) -> [f64; 2] {
    let k = work(app).subpaths[s].knots[0].anchor;
    [k.x, k.y]
}

fn steps(app: &PrismApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The subpaths with selected anchors.
fn picked(app: &PrismApp) -> Vec<usize> {
    selected_subpaths(&app.ui.direct_selection)
}

fn press(app: &mut PrismApp, at: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, mods);
}

fn drag(app: &mut PrismApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    press(app, from, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn click(app: &mut PrismApp, at: [f64; 2], mods: Modifiers) {
    drag(app, at, at, mods);
}

/// Presses `key` (with `mods`) in a frame running Path Selection's keys.
fn key(app: &mut PrismApp, key: egui::Key, mods: Modifiers) -> bool {
    let ctx = egui::Context::default();
    let input = egui::RawInput { events: vec![egui::Event::ModifiersChanged(mods), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods }], ..Default::default() };
    let mut used = false;
    let mut out = ctx.run_ui(input, |ui| used = keys(app, ui.ctx()));
    // epaint refuses to drop an unapplied font atlas upload in debug builds.
    out.textures_delta.clear();
    used
}

/// A click on a component's outline or inside it selects the whole component (a hole with its
/// shape); ⇧ adds or removes one; empty canvas deselects. Clicks change no pixels or history.
#[test]
fn clicks_select_whole_components() {
    let mut app = app();
    with_paths(&mut app);
    let n = steps(&app);
    click(&mut app, [40.0, 20.0], Modifiers::NONE);
    assert_eq!(picked(&app), [0], "on its outline");
    click(&mut app, [110.0, 110.0], Modifiers::NONE);
    assert_eq!(picked(&app), [1, 2], "inside it: the shape and its hole");
    click(&mut app, [140.0, 120.0], Modifiers::NONE);
    assert_eq!(picked(&app), [1, 2], "on the hole's outline: the same component");
    click(&mut app, [30.0, 30.0], Modifiers::SHIFT);
    assert_eq!(picked(&app), [0, 1, 2], "⇧ adds");
    click(&mut app, [30.0, 30.0], Modifiers::SHIFT);
    assert_eq!(picked(&app), [1, 2], "⇧ again removes");
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    assert!(picked(&app).is_empty(), "empty canvas deselects");
    assert_eq!(steps(&app), n);
}

/// A drag moves the selected components together, previewed live and committed as one step;
/// the others stay.
#[test]
fn dragging_moves_the_selected_components_in_one_step() {
    let mut app = app();
    with_paths(&mut app);
    let n = steps(&app);
    // Mid-drag the canvas shows it moved, the document doesn't yet.
    press(&mut app, [110.0, 110.0], Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 120.0, y: 115.0, pressure: 1.0 }, Modifiers::NONE);
    let Some(Drag::Components { preview: Some(p), .. }) = &app.ui.direct_selection.drag else { panic!("a component drag") };
    assert_eq!((p.subpaths[1].knots[0].anchor.x, p.subpaths[2].knots[0].anchor.y), (110.0, 125.0));
    assert_eq!(first(&app, 1), [100.0, 100.0]);
    tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 115.0 }, Modifiers::NONE);
    assert_eq!((first(&app, 0), first(&app, 1), first(&app, 2)), ([20.0, 20.0], [110.0, 105.0], [130.0, 125.0]));
    assert_eq!(steps(&app), n + 1);
    // ⇧-add the square, then drag either: both move.
    click(&mut app, [40.0, 20.0], Modifiers::SHIFT);
    drag(&mut app, [40.0, 20.0], [40.0, 30.0], Modifiers::NONE);
    assert_eq!((first(&app, 0), first(&app, 1)), ([20.0, 30.0], [110.0, 115.0]));
    assert!(app.session.undo());
    assert_eq!(first(&app, 0), [20.0, 20.0]);
}

/// ⌥-drag copies the selected components on top, moved, in one step; the copies are selected.
#[test]
fn alt_drag_copies_components() {
    let mut app = app();
    with_paths(&mut app);
    let n = steps(&app);
    drag(&mut app, [30.0, 30.0], [30.0, 80.0], Modifiers::ALT);
    let w = work(&app);
    assert_eq!(w.subpaths.len(), 4);
    assert_eq!((first(&app, 0), first(&app, 3)), ([20.0, 20.0], [20.0, 70.0]));
    assert_eq!(picked(&app), [3], "the copy is selected");
    assert_eq!(steps(&app), n + 1);
}

/// A marquee on empty canvas selects the anchors inside it, as Direct Selection's does (as in
/// Photoshop); dragging one of them moves just those, and ⇧ adds.
#[test]
fn a_marquee_selects_anchors_like_direct_selection() {
    let mut app = app();
    with_paths(&mut app);
    // The big square's two right-hand anchors.
    drag(&mut app, [170.0, 90.0], [190.0, 190.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, [[1, 1], [1, 2]]);
    assert_eq!(app.ui.tool, Tool::PathSelection);
    drag(&mut app, [180.0, 100.0], [190.0, 100.0], Modifiers::NONE);
    let w = work(&app);
    let x = |s: usize, k: usize| w.subpaths[s].knots[k].anchor.x;
    assert_eq!((x(1, 0), x(1, 1), x(1, 2), x(1, 3), x(2, 0)), (100.0, 190.0, 190.0, 100.0, 120.0), "only the picked anchors moved");
    drag(&mut app, [5.0, 5.0], [25.0, 25.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, [[1, 1], [1, 2], [0, 0]]);
    // A press elsewhere on the partly selected component picks it whole.
    click(&mut app, [100.0, 140.0], Modifiers::NONE);
    assert_eq!(picked(&app), [1, 2]);
    assert_eq!(app.ui.direct_selection.anchors.len(), 8);
}

/// ⌘/Ctrl swaps the two path tools while held, as in Photoshop: Path Selection edits like Direct
/// Selection (its white arrow shows), and a ⌘-press on an anchor switches to the Direct Selection
/// tool; Direct Selection with ⌘ picks whole components (the black arrow).
#[test]
fn command_swaps_the_path_tools() {
    let mut app = app();
    with_paths(&mut app);
    let cmd = Modifiers::COMMAND;
    assert!(!crate::direct_select::direct_cursor(&app, Modifiers::NONE), "Path Selection: black arrow");
    assert!(crate::direct_select::direct_cursor(&app, cmd), "⌘: white arrow, still Path Selection");
    assert_eq!(app.ui.tool, Tool::PathSelection);
    // ⌘ on a segment edits it as Direct Selection would, the tool stays.
    press(&mut app, [40.0, 20.0], cmd);
    assert!(matches!(app.ui.direct_selection.drag, Some(Drag::Edit { .. })));
    assert!(crate::direct_select::direct_cursor(&app, Modifiers::NONE), "an anchor or segment drag keeps the white arrow");
    tool_event(&mut app, ToolEvent::Up { x: 40.0, y: 20.0 }, cmd);
    assert_eq!(app.ui.tool, Tool::PathSelection);
    // ⌘ on an anchor of a wholly selected component: just that anchor, the Direct Selection
    // tool, and the drag carries on.
    click(&mut app, [30.0, 30.0], Modifiers::NONE);
    assert_eq!(picked(&app), [0]);
    press(&mut app, [60.0, 60.0], cmd);
    assert_eq!(app.ui.direct_selection.anchors, [[0, 2]], "only the pressed anchor");
    assert_eq!(app.ui.tool, Tool::DirectSelection);
    tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 65.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 65.0 }, Modifiers::NONE);
    assert_eq!(work(&app).subpaths[0].knots[2].anchor.x, 70.0);
    assert_eq!(first(&app, 0), [20.0, 20.0], "only that anchor");
    // Direct Selection with ⌘ picks whole components, with the black arrow.
    assert!(crate::direct_select::direct_cursor(&app, Modifiers::NONE));
    assert!(!crate::direct_select::direct_cursor(&app, cmd));
    click(&mut app, [110.0, 110.0], cmd);
    assert_eq!(picked(&app), [1, 2]);
    assert_eq!(app.ui.tool, Tool::DirectSelection);
}

/// ⌫ deletes the selected components; the arrow keys nudge them (⇧: 10 px). Without a selection
/// the keys are left to the rest (Edit › Clear, the pixel selection).
#[test]
fn keys_delete_and_nudge_the_selection() {
    let mut app = app();
    with_paths(&mut app);
    assert!(!key(&mut app, egui::Key::Backspace, Modifiers::NONE), "nothing selected");
    click(&mut app, [30.0, 30.0], Modifiers::NONE);
    assert!(key(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
    assert!(key(&mut app, egui::Key::ArrowDown, Modifiers::SHIFT));
    assert_eq!(first(&app, 0), [21.0, 30.0]);
    assert_eq!(first(&app, 1), [100.0, 100.0], "the other component stays");
    assert!(key(&mut app, egui::Key::Backspace, Modifiers::NONE));
    assert_eq!(work(&app).subpaths.len(), 2);
    assert_eq!(first(&app, 0), [100.0, 100.0]);
    assert!(picked(&app).is_empty());
    // Deleting the last component removes the work path.
    click(&mut app, [110.0, 110.0], Modifiers::NONE);
    assert!(key(&mut app, egui::Key::Delete, Modifiers::NONE));
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

/// A shape layer's whole path moves by whole pixels and stays a live shape; ⌘ makes the tool
/// Direct Selection, which moves one anchor.
#[test]
fn shapes_stay_live_and_command_edits_anchors() {
    let mut app = app();
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 40]})).unwrap()["layer"].as_u64().unwrap();
    drag(&mut app, [50.0, 40.0], [60.4, 45.6], Modifiers::NONE);
    let shape = |app: &PrismApp| match &app.session.active().unwrap().doc.layer(prism_doc::LayerId(id)).unwrap().content {
        LayerContent::Shape(sh) => sh.clone(),
        _ => panic!("shape"),
    };
    assert!(matches!(shape(&app).live, Some(prism_doc::LiveShape::Rect { rect: [30.0, 26.0, 60.0, 40.0], .. })), "{:?}", shape(&app).live);
    // The whole path is selected; a ⌘-press on one anchor picks just it, as in Photoshop, so
    // only it moves (the rectangle stops being live), and the tool becomes Direct Selection.
    assert_eq!(app.ui.direct_selection.anchors.len(), 4);
    drag(&mut app, [30.0, 26.0], [25.0, 21.0], Modifiers::COMMAND);
    assert_eq!(app.ui.tool, Tool::DirectSelection, "a ⌘-press on an anchor switches tools");
    assert_eq!(app.ui.direct_selection.anchors, [[0, 0]]);
    let k = &shape(&app).path.subpaths[0].knots;
    assert_eq!((k[0].anchor.x, k[0].anchor.y, k[1].anchor.x), (25.0, 21.0, 90.0));
}

/// Under the pointer, the component a click would pick is the hover feedback.
#[test]
fn hovering_finds_the_component_a_click_picks() {
    let mut app = app();
    with_paths(&mut app);
    assert_eq!(component_at(&app, [40.0, 21.0]).map(|c| c.2), Some(0..1));
    assert_eq!(component_at(&app, [140.0, 140.0]).map(|c| c.2), None, "inside the hole is outside the shape");
    assert_eq!(component_at(&app, [110.0, 140.0]).map(|c| c.2), Some(1..3));
    assert_eq!(component_at(&app, [80.0, 80.0]), None);
}
