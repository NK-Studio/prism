use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

fn app() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 160, "height": 160, "background": "transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 4, "hardness": 1.0}})).unwrap();
    app.session.tools.brush.smoothing.amount = 0.0;
    // The CLIP STUDIO "Isometric ruler" material: three parallel vanishing points.
    app.run("layer.specialRuler.addPerspective", json!({"points": [{"parallel": 30.6163181116636}, {"parallel": 149.39613684716832}, {"parallel": 270}]})).unwrap();
    app.ui.tool = Tool::Brush;
    app
}

fn last_stroke_points(app: &PrismApp) -> Vec<[f64; 2]> {
    let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
    p["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap()]).collect()
}

fn on_line(p: [f64; 2], start: [f64; 2], angle_deg: f64) -> bool {
    let d = angle_dir(angle_deg);
    ((p[0] - start[0]) * d[1] - (p[1] - start[1]) * d[0]).abs() < 1e-6
}

/// A wobbly drag at about 28° (up and to the right).
fn drag(app: &mut PrismApp, mods: egui::Modifiers) -> [f64; 2] {
    let start = [20.0, 120.0];
    tool_event(app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, mods);
    for i in 1..=10 {
        let t = f64::from(i);
        let wobble = if i % 2 == 0 { 2.0 } else { -2.0 };
        tool_event(app, ToolEvent::Move { x: start[0] + 10.0 * t, y: start[1] - 5.3 * t + wobble, pressure: 1.0 }, mods);
    }
    tool_event(app, ToolEvent::Up { x: 125.0, y: 64.0 }, mods);
    start
}

#[test]
fn brush_strokes_follow_the_isometric_ruler() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser] {
        let mut app = app();
        app.ui.tool = tool;
        let start = drag(&mut app, egui::Modifiers::NONE);
        let pts = if tool == Tool::Pencil {
            let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.pencil").map(|(_, p)| p.clone()).unwrap();
            p["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap()]).collect()
        } else {
            last_stroke_points(&app)
        };
        assert!(pts.len() >= 5, "{tool:?}: {pts:?}");
        assert!(pts.iter().all(|p| on_line(*p, start, 30.6163181116636)), "{tool:?}: {pts:?}");
        assert!(app.drag.is_none());
    }
}

#[test]
fn shift_and_the_snap_toggle_override_the_ruler() {
    // ⇧ keeps Photoshop's 0/45/90° constraint.
    let mut a = app();
    let start = drag(&mut a, egui::Modifiers::SHIFT);
    let pts = last_stroke_points(&a);
    assert!(pts.iter().all(|p| on_line(*p, start, 45.0)), "{pts:?}");
    // View › Snap To › Special Ruler off: the stroke is freehand.
    let mut a = app();
    a.ui.view.snap_to.special_ruler = false;
    let start = drag(&mut a, egui::Modifiers::NONE);
    assert!(!last_stroke_points(&a).iter().all(|p| on_line(*p, start, 30.6163181116636)));
    // A hidden ruler does not snap either.
    let mut a = app();
    a.run("layer.specialRuler.setShow", json!({"shown": false})).unwrap();
    assert!(snapper(&a, Tool::Brush, [20.0, 120.0]).is_none());
    // Nor does a tool that is not brush-like.
    let a = app();
    assert!(snapper(&a, Tool::Move, [20.0, 120.0]).is_none());
    assert!(snapper(&a, Tool::Brush, [20.0, 120.0]).is_some());
}

#[test]
fn the_snap_to_menu_item_toggles_ruler_snapping() {
    let mut app = app();
    let ctx = egui::Context::default();
    assert!(crate::menus::is_live("view.snapTo.specialRuler"));
    let item = |app: &PrismApp| crate::menus::menu_items(app).into_iter().find(|i| i.id == "view.snapTo.specialRuler").unwrap();
    let it = item(&app);
    assert_eq!((it.checked, it.path.as_slice()), (Some(true), ["View".to_string(), "Snap To".to_string()].as_slice()));
    crate::menus::invoke(&mut app, &ctx, "view.snapTo.specialRuler", json!({})).unwrap();
    assert!(!app.ui.view.snap_to.special_ruler);
    assert_eq!(item(&app).checked, Some(false));
    crate::menus::invoke(&mut app, &ctx, "view.snapTo.all", json!({})).unwrap();
    assert!(app.ui.view.snap_to.special_ruler);
}
