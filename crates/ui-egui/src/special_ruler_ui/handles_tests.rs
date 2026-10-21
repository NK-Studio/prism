use super::*;
use crate::canvas::tool_event;
use prism_doc::special_ruler::VanishingPoint;

const ISO: [f64; 3] = [30.6163181116636, 149.39613684716832, 270.0];

/// The imported "Isometric ruler" shape on a 160 × 160 document: the 30.6° point has a guide on
/// the document, the 149.4° point none, the vertical point's guides lie outside it.
fn app() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 160, "height": 160, "background": "transparent"})).unwrap();
    app.run("layer.specialRuler.addPerspective", json!({"points": ISO.map(|a| json!({"parallel": a}))})).unwrap();
    // Two ends of one guide line, as in the downloaded material.
    let d = angle_dir(ISO[0]);
    app.run("layer.specialRuler.edit", json!({"index": 0, "point": 0, "guides": [[40, 120], [40.0 + 100.0 * d[0], 120.0 + 100.0 * d[1]]]})).unwrap();
    app.run("layer.specialRuler.edit", json!({"index": 0, "point": 2, "guides": [[-500, 10], [-480, 40]]})).unwrap();
    app.ui.tool = Tool::Move;
    app
}

fn points(app: &PrismApp) -> Vec<VanishingPoint> {
    let st = app.session.active().unwrap();
    let r = own(&st.doc, st.active_layer.unwrap()).unwrap();
    let RulerKind::Perspective(p) = &r.rulers[0].kind else { panic!("perspective") };
    p.points.clone()
}

fn angle(app: &PrismApp, i: usize) -> f64 {
    match points(app)[i].target {
        VanishTarget::Parallel { angle_deg } => angle_deg,
        VanishTarget::Point(_) => panic!("parallel"),
    }
}

fn find(app: &PrismApp, point: usize, kind: Kind) -> Handle {
    current(app).into_iter().find(|h| h.point == point && h.kind == kind).unwrap()
}

fn drag(app: &mut PrismApp, from: [f64; 2], to: [f64; 2], mods: egui::Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    for t in [0.3, 0.7] {
        tool_event(app, ToolEvent::Move { x: from[0] + (to[0] - from[0]) * t, y: from[1] + (to[1] - from[1]) * t, pressure: 1.0 }, mods);
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn every_direction_gets_an_anchor_and_an_angle_handle() {
    let app = app();
    let hs = current(&app);
    assert_eq!(hs.len(), 6);
    let a0 = find(&app, 0, Kind::Anchor);
    assert_eq!(([a0.x, a0.y], a0.virtual_anchor), ([40.0, 120.0], false), "an in-document guide anchors its line");
    let (a1, a2) = (find(&app, 1, Kind::Anchor), find(&app, 2, Kind::Anchor));
    assert!(a1.virtual_anchor && a2.virtual_anchor);
    assert_eq!([a1.x, a1.y], [80.0, 80.0], "the first virtual anchor is the document centre");
    assert!((a1.x - a2.x).hypot(a1.y - a2.y) > HIT / f64::from(app.current_zoom()), "virtual anchors do not overlap");
    let tip = find(&app, 0, Kind::Angle);
    let d = angle_dir(ISO[0]);
    let reach = ANGLE_REACH / f64::from(app.current_zoom());
    assert!((tip.x - (40.0 + d[0] * reach)).abs() < 1e-9 && (tip.y - (120.0 + d[1] * reach)).abs() < 1e-9);
}

#[test]
fn handles_live_only_with_the_move_tool_on_the_ruler_layer() {
    let mut a = app();
    a.ui.tool = Tool::Brush;
    assert!(current(&a).is_empty());
    let mut a = app();
    a.ui.view.extras = false;
    assert!(current(&a).is_empty());
    let mut a = app();
    a.run("layer.new.layer", json!({})).unwrap();
    assert!(current(&a).is_empty(), "another layer's rulers are not edited from here");
    let mut a = app();
    a.run("layer.specialRuler.setShow", json!({"shown": false})).unwrap();
    assert!(current(&a).is_empty());
}

#[test]
fn dragging_the_angle_handle_turns_the_direction_in_one_step() {
    let mut a = app();
    let past = a.session.active().unwrap().history.past_len();
    let tip = find(&a, 0, Kind::Angle);
    let to = [40.0 + 30.0 * 45f64.to_radians().cos(), 120.0 - 30.0 * 45f64.to_radians().sin()];
    drag(&mut a, [tip.x, tip.y], to, egui::Modifiers::NONE);
    assert!((angle(&a, 0) - 45.0).abs() < 1e-9, "{}", angle(&a, 0));
    assert_eq!(a.session.active().unwrap().history.past_len(), past + 1);
    // The guide line turned with it: its far end is still on the one line through the anchor.
    let g = points(&a)[0].guides.clone();
    assert_eq!(g[0], [40.0, 120.0]);
    let d = angle_dir(45.0);
    assert!(((g[1][0] - 40.0) * d[1] - (g[1][1] - 120.0) * d[0]).abs() < 1e-9 && (g[1][0] - 40.0 - 100.0 * d[0]).abs() < 1e-9, "{g:?}");
    assert!(a.ruler_drag.is_none());
    assert_eq!(a.ui.status, "45.0°");
    // Strokes follow the new direction.
    let r = a.session.execute("specialRuler.snapPoints", json!({"points": [[40, 120], [60, 108], [100, 90]]})).unwrap();
    let p = &r["points"][2];
    let got = (-(p[1].as_f64().unwrap() - 120.0)).atan2(p[0].as_f64().unwrap() - 40.0).to_degrees();
    assert!((got - 45.0).abs() < 1e-6, "{got}");
    a.session.undo();
    assert_eq!(angle(&a, 0), ISO[0]);
    // ⇧ steps by 15°.
    let tip = find(&a, 0, Kind::Angle);
    let to = [40.0 + 30.0 * 49f64.to_radians().cos(), 120.0 - 30.0 * 49f64.to_radians().sin()];
    drag(&mut a, [tip.x, tip.y], to, egui::Modifiers::SHIFT);
    assert!((angle(&a, 0) - 45.0).abs() < 1e-9);
}

#[test]
fn dragging_an_anchor_moves_the_line_and_keeps_its_direction() {
    let mut a = app();
    // The vertical direction's virtual anchor: its line lands on the document.
    let anchor = find(&a, 2, Kind::Anchor);
    drag(&mut a, [anchor.x, anchor.y], [30.0, 70.0], egui::Modifiers::NONE);
    let p = points(&a);
    assert_eq!((p[2].guides.clone(), angle(&a, 2)), (vec![[30.0, 70.0]], 270.0));
    let now = find(&a, 2, Kind::Anchor);
    assert_eq!(([now.x, now.y], now.virtual_anchor), ([30.0, 70.0], false));
    // A guide line moves as a whole: both ends shift by the drag.
    let far = points(&a)[0].guides[1];
    drag(&mut a, [40.0, 120.0], [50.0, 125.0], egui::Modifiers::NONE);
    assert_eq!(points(&a)[0].guides, [[50.0, 125.0], [far[0] + 10.0, far[1] + 5.0]]);
    assert_eq!(angle(&a, 0), ISO[0]);
}

#[test]
fn presses_off_the_handles_still_move_the_layer() {
    let mut a = app();
    assert!(!pointer(&mut a, ToolEvent::Down { x: 150.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE));
    assert!(a.ruler_drag.is_none());
    // A press that does not move changes nothing.
    let past = a.session.active().unwrap().history.past_len();
    let tip = find(&a, 0, Kind::Angle);
    tool_event(&mut a, ToolEvent::Down { x: tip.x, y: tip.y, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut a, ToolEvent::Up { x: tip.x, y: tip.y }, egui::Modifiers::NONE);
    assert!((angle(&a, 0) - ISO[0]).abs() < 1e-9);
    assert!(a.session.active().unwrap().history.past_len() <= past + 1);
    // Hostile input is ignored.
    assert!(at(&current(&a), [f64::NAN, 0.0], 1.0).is_none());
    assert!(handles(&a.session.active().unwrap().doc, LayerId(999), f64::NAN).is_empty());
}

#[test]
fn agents_see_the_handles() {
    let a = app();
    let v = crate::control::inspect(&a, &egui::Context::default());
    let hs = v["specialRulerHandles"].as_array().unwrap();
    assert_eq!(hs.len(), 6);
    assert_eq!((hs[0]["kind"].as_str(), hs[0]["x"].as_f64()), (Some("anchor"), Some(40.0)));
}
