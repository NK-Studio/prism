//! #790: paths can be reshaped after they're drawn, as in Photoshop: the Direct Selection tool
//! (A), and the Pen with ⌘/Ctrl (Direct Selection) or ⌥/Alt (Convert Point).

use egui::Modifiers;
use prism_doc::vector::Path;
use prism_geom::Point;
use serde_json::json;

use super::{Drag, Edit, PathRef};
use crate::PrismApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(tool: Tool) -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A square work path; knot 1 at (120, 40) is smooth with horizontal handles.
fn with_path(app: &mut PrismApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [
        [40, 40],
        {"anchor": [120, 40], "in": [100, 40], "out": [140, 40], "smooth": true},
        [120, 120],
        [40, 120],
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

fn work(app: &PrismApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn anchor(app: &PrismApp, k: usize) -> Point {
    work(app).subpaths[0].knots[k].anchor
}

fn steps(app: &PrismApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn drag(app: &mut PrismApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn click(app: &mut PrismApp, at: [f64; 2], mods: Modifiers) {
    drag(app, at, at, mods);
}

#[test]
fn dragging_an_anchor_moves_it_in_one_undoable_step() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    drag(&mut app, [40.5, 120.5], [50.5, 130.5], Modifiers::NONE);
    assert_eq!(anchor(&app, 3), Point::new(50.0, 130.0));
    assert_eq!(anchor(&app, 0), Point::new(40.0, 40.0), "only the dragged anchor moved");
    assert_eq!(steps(&app), n + 1, "one history step for the whole drag");
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Work), vec![[0, 3]]));
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn the_preview_is_what_the_drag_commits() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    tool_event(&mut app, ToolEvent::Down { x: 80.0, y: 120.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 80.0, y: 150.0, pressure: 1.0 }, Modifiers::NONE);
    let Some(Drag::Edit { preview: Some(shown), .. }) = app.ui.direct_selection.drag.clone() else { panic!("a segment drag with a preview") };
    assert_ne!(shown, work(&app), "the document changes only on release");
    tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 150.0 }, Modifiers::NONE);
    assert_eq!(work(&app), shown);
    // The straight bottom segment moved with both its anchors.
    assert_eq!((anchor(&app, 2), anchor(&app, 3)), (Point::new(120.0, 150.0), Point::new(40.0, 150.0)));
}

#[test]
fn a_smooth_points_handles_turn_together_and_alt_breaks_them() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    click(&mut app, [120.0, 40.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "a click selects the anchor and shows its handles");
    drag(&mut app, [140.0, 40.0], [120.0, 60.0], Modifiers::NONE);
    let k = work(&app).subpaths[0].knots[1];
    assert_eq!(k.out_ctrl, Point::new(120.0, 60.0));
    assert!((k.in_ctrl.x - 120.0).abs() < 1e-9 && (k.in_ctrl.y - 20.0).abs() < 1e-9, "the opposite handle turned with it: {:?}", k.in_ctrl);
    drag(&mut app, [120.0, 20.0], [100.0, 20.0], Modifiers::ALT);
    let k = work(&app).subpaths[0].knots[1];
    assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(100.0, 20.0), Point::new(120.0, 60.0), false));
}

#[test]
fn marquee_shift_and_empty_clicks_select_like_photoshop() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    drag(&mut app, [100.0, 100.0], [180.0, 180.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]]);
    click(&mut app, [40.0, 120.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2], [0, 3]], "⇧-click adds");
    // Dragging one selected anchor moves them all.
    drag(&mut app, [40.0, 120.0], [40.0, 110.0], Modifiers::NONE);
    assert_eq!((anchor(&app, 2), anchor(&app, 3)), (Point::new(120.0, 110.0), Point::new(40.0, 110.0)));
    click(&mut app, [40.0, 110.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]], "⇧-click on a selected anchor deselects it");
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    assert!(app.ui.direct_selection.anchors.is_empty(), "a click on empty canvas deselects");
    click(&mut app, [80.0, 40.0], Modifiers::ALT);
    assert_eq!(app.ui.direct_selection.anchors.len(), 4, "⌥-click selects the whole subpath");
}

#[test]
fn pen_with_command_edits_paths_and_ends_the_one_being_drawn() {
    let mut app = app(Tool::Pen);
    for p in [[30.0, 30.0], [90.0, 30.0]] {
        click(&mut app, p, Modifiers::NONE);
    }
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(2));
    // ⌘-drag (Ctrl-drag off the Mac) on the second anchor: the path is finished and edited.
    drag(&mut app, [90.0, 30.0], [90.0, 70.0], Modifiers::COMMAND);
    assert!(app.ui.pen.is_none());
    assert_eq!(app.ui.tool, Tool::Pen, "the Pen stays the tool");
    let wp = work(&app);
    assert_eq!((wp.subpaths[0].closed, wp.subpaths[0].knots[1].anchor), (false, Point::new(90.0, 70.0)));
    // Without ⌘ the Pen draws again.
    click(&mut app, [150.0, 150.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(1));
}

#[test]
fn pen_with_alt_converts_points() {
    let mut app = app(Tool::Pen);
    with_path(&mut app);
    click(&mut app, [120.0, 40.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots[1], prism_doc::Knot::corner(120.0, 40.0), "⌥-click makes a corner");
    drag(&mut app, [40.0, 120.0], [40.0, 140.0], Modifiers::ALT);
    let k = work(&app).subpaths[0].knots[3];
    assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(40.0, 100.0), Point::new(40.0, 140.0), true), "⌥-drag pulls out smooth handles");
    assert!(app.ui.pen.is_none(), "no pen path was started");
    // ⌥ off the path is a plain Pen click.
    click(&mut app, [170.0, 170.0], Modifiers::ALT);
    assert!(app.ui.pen.is_some());
}

/// With the Pen, the paths show their anchors, as in Photoshop (Auto Add/Delete, ⌥ and ⌘ act
/// on them), and a ⌥-drag out of an anchor shows both handles as they are pulled out, on a
/// shape layer drawn with the Pen too.
#[test]
fn the_pen_shows_anchors_and_the_handles_it_pulls_out() {
    let mut app = app(Tool::Pen);
    app.ui.tool_options.vector_mode = "shape".into();
    for p in [[40.0, 40.0], [140.0, 40.0], [90.0, 140.0], [40.0, 40.0]] {
        crate::vector_ui::pen_down(&mut app, p[0], p[1]);
        crate::vector_ui::pen_up(&mut app);
    }
    assert!(app.ui.pen.is_none(), "closed: a shape layer");
    assert!(super::shows(&app, Modifiers::NONE), "the Pen draws the anchors overlay");
    let (r, _) = super::candidates(&app).into_iter().next().expect("the shape's path");
    assert!(matches!(r, PathRef::Layer(_)));
    // The new shape starts with every anchor selected, as Path Selection would pick it.
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(r), vec![[0, 0], [0, 1], [0, 2]]));
    // The bottom anchor is selected (⌘-click); ⌥ work below keeps it selected.
    click(&mut app, [90.0, 140.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.anchors, [[0, 2]]);
    // ⌥-drag out of the top-right anchor: its handles show mid-drag.
    tool_event(&mut app, ToolEvent::Down { x: 140.0, y: 40.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: 160.0, y: 40.0, pressure: 1.0 }, Modifiers::ALT);
    let ds = &app.ui.direct_selection;
    let Some(Drag::Edit { edit: Edit::Convert(k), preview: Some(preview), .. }) = &ds.drag else { panic!("a convert drag: {:?}", ds.drag) };
    assert_eq!(*k, [0, 1]);
    let handles = super::overlay_handles(ds, r, preview, false);
    assert!(handles.contains(&([0, 1], prism_vector::edit::Handle::In)) && handles.contains(&([0, 1], prism_vector::edit::Handle::Out)), "{handles:?}");
    tool_event(&mut app, ToolEvent::Up { x: 160.0, y: 40.0 }, Modifiers::ALT);
    let (_, path) = super::candidates(&app).into_iter().next().unwrap();
    let kn = path.subpaths[0].knots[1];
    assert!(kn.smooth && kn.out_ctrl == Point::new(160.0, 40.0), "smooth with the pulled-out handle: {kn:?}");
    // After the release the anchor stays selected and its handles keep showing (they used to
    // vanish), and one can be dragged on with ⌥ alone.
    let ds = &app.ui.direct_selection;
    assert_eq!((ds.target, ds.anchors.clone()), (Some(r), vec![[0, 2], [0, 1]]), "added to the selection, which stays");
    let handles = super::overlay_handles(ds, r, &path, false);
    assert!(handles.contains(&([0, 1], prism_vector::edit::Handle::In)) && handles.contains(&([0, 1], prism_vector::edit::Handle::Out)), "{handles:?}");
    drag(&mut app, [160.0, 40.0], [160.0, 60.0], Modifiers::ALT);
    let (_, path) = super::candidates(&app).into_iter().next().unwrap();
    let kn = path.subpaths[0].knots[1];
    assert_eq!((kn.out_ctrl, kn.in_ctrl, kn.smooth), (Point::new(160.0, 60.0), Point::new(120.0, 40.0), false), "⌥ on a handle moves it alone");
    // ⌥-click makes it a corner again.
    click(&mut app, [140.0, 40.0], Modifiers::ALT);
    let (_, path) = super::candidates(&app).into_iter().next().unwrap();
    assert_eq!(path.subpaths[0].knots[1], prism_doc::Knot::corner(140.0, 40.0));
}

/// The Pen's cursor says what a press does, as in Photoshop (cursor pack artwork): ⌥ shows
/// Convert Point, also through its drag; ⌘ shows Direct Selection's white arrow; with Auto
/// Add/Delete a segment shows the pen with a plus, an anchor the pen with a minus.
#[test]
fn the_pen_cursor_follows_option_command_and_auto_add_delete() {
    use crate::vector_ui::pen_cursor_asset;
    let mut app = app(Tool::Pen);
    with_path(&mut app);
    let none = Modifiers::NONE;
    assert_eq!(pen_cursor_asset(&app, [170.0, 170.0], none), 1063, "a new path");
    assert_eq!(pen_cursor_asset(&app, [80.0, 120.0], none), 1031, "on a segment: add an anchor");
    assert_eq!(pen_cursor_asset(&app, [40.0, 120.0], none), 1032, "on an anchor: delete it");
    for at in [[170.0, 170.0], [40.0, 120.0]] {
        assert_eq!(pen_cursor_asset(&app, at, Modifiers::ALT), 1035, "⌥: Convert Point");
    }
    assert!(super::direct_cursor(&app, Modifiers::COMMAND), "⌘: Direct Selection's arrow");
    assert!(!super::direct_cursor(&app, Modifiers::ALT));
    // Mid ⌥-drag the Convert Point cursor stays, even if ⌥ is let go.
    tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 120.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: 40.0, y: 140.0, pressure: 1.0 }, Modifiers::ALT);
    assert!(!super::direct_cursor(&app, none));
    assert_eq!(pen_cursor_asset(&app, [40.0, 140.0], none), 1035);
    tool_event(&mut app, ToolEvent::Up { x: 40.0, y: 140.0 }, Modifiers::ALT);
    // Auto Add/Delete off: the plain pen.
    app.ui.tool_options.pen_auto_add_delete = false;
    assert_eq!(pen_cursor_asset(&app, [80.0, 120.0], none), 1063);
}

/// The Pen with ⌘ held is Direct Selection, as in Photoshop: a click inside a component makes it
/// the active one, its anchors showing unselected, the others' hidden; a click on empty canvas
/// drops it. Only selected or active components show anchors.
#[test]
fn command_click_inside_a_component_activates_it() {
    let mut app = app(Tool::Pen);
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[20, 20], [80, 20], [80, 80], [20, 80]]},
        {"closed": true, "knots": [[120, 120], [180, 120], [180, 180], [120, 180]]},
    ]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app.ui.direct_selection = Default::default();
    let visible = |app: &PrismApp| {
        let (r, path) = super::candidates(app).into_iter().next().unwrap();
        super::visible_components(&app.ui.direct_selection, r, &path).into_iter().map(|c| (c.start, c.end)).collect::<Vec<_>>()
    };
    assert!(visible(&app).is_empty(), "nothing selected: outlines only");
    click(&mut app, [50.0, 50.0], Modifiers::COMMAND);
    let ds = &app.ui.direct_selection;
    assert_eq!((ds.target, ds.anchors.is_empty(), ds.shown.clone()), (Some(PathRef::Work), true, vec![0]));
    assert_eq!(visible(&app), [(0, 1)], "that component's anchors show, unselected");
    click(&mut app, [150.0, 150.0], Modifiers::COMMAND);
    assert_eq!(visible(&app), [(1, 2)]);
    assert!(app.ui.pen.is_none(), "no pen path was started");
    click(&mut app, [100.0, 10.0], Modifiers::COMMAND);
    assert!(visible(&app).is_empty(), "empty canvas drops it");
    // ⌘ let go: a plain click draws with the Pen again.
    click(&mut app, [100.0, 10.0], Modifiers::NONE);
    assert!(app.ui.pen.is_some());
}

#[test]
fn shape_layer_paths_are_edited_too() {
    let mut app = app(Tool::DirectSelection);
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 60], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    drag(&mut app, [80.0, 80.0], [100.0, 90.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Layer(id)));
    let info = app.run("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"][2]["anchor"], json!([100.0, 90.0]));
    let b: [i64; 4] = serde_json::from_value(info["bounds"].clone()).unwrap();
    assert!(b[0] + b[2] >= 100 && b[1] + b[3] >= 90, "the shape re-rendered: {b:?}");
}
