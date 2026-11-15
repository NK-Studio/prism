use super::*;
use crate::canvas::tool_event;

fn app() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 300, "background": "white"})).unwrap();
    app.run("layer.figure.new", json!({})).unwrap();
    app.ui.tool = Tool::Move;
    app
}

fn find(app: &PrismApp, joint: &str) -> Handle {
    current(app).into_iter().find(|h| h.joint == joint).unwrap()
}

fn steps(app: &PrismApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn scene(app: &PrismApp) -> prism_doc::FigureScene {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().figure.clone().unwrap()
}

fn drag(app: &mut PrismApp, from: [f64; 2], to: [f64; 2], mods: egui::Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    for t in [0.3, 0.7, 1.0] {
        tool_event(app, ToolEvent::Move { x: from[0] + (to[0] - from[0]) * t, y: from[1] + (to[1] - from[1]) * t, pressure: 1.0 }, mods);
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn the_main_joints_get_handles_only_with_the_move_tool() {
    let mut app = app();
    let hs = current(&app);
    for name in ["head_bb_", "lefthand_bb_", "rightforearm_bb_", "leftleg_bb_", "rightfoot_bb_"] {
        assert!(hs.iter().any(|h| h.joint == name), "{name}");
    }
    assert!(!hs.iter().any(|h| h.joint.contains("index") || h.joint.contains("_end_") || h.joint == "root_group_"));
    assert!(find(&app, "lefthand_bb_").end && !find(&app, "leftforearm_bb_").end);
    assert_eq!(inspect(&app).as_array().unwrap().len(), hs.len());
    app.ui.tool = Tool::Brush;
    assert!(current(&app).is_empty());
    app.ui.tool = Tool::Move;
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(current(&app).is_empty(), "only on a figure layer");
}

#[test]
fn dragging_a_joint_turns_its_bone_in_one_step() {
    let mut app = app();
    let (elbow, hand) = (find(&app, "leftforearm_bb_"), find(&app, "lefthand_bb_"));
    let before = steps(&app);
    // Swing the hand round the elbow, a quarter turn clockwise on screen.
    let r = (hand.x - elbow.x).hypot(hand.y - elbow.y);
    let to = [elbow.x - r, elbow.y];
    drag(&mut app, [hand.x, hand.y], to, egui::Modifiers::NONE);
    assert_eq!(steps(&app), before + 1, "one history step");
    let moved = find(&app, "lefthand_bb_");
    assert!((moved.x - to[0]).abs() < 1.5 && (moved.y - to[1]).abs() < 1.5, "the hand follows the pointer: {moved:?} vs {to:?}");
    assert!((find(&app, "leftforearm_bb_").x - elbow.x).abs() < 1e-6, "the elbow stays");
    assert!(app.figure_drag.is_none());
    // ⌘/Ctrl on the hand: reach with IK, towards the shoulder.
    let shoulder = find(&app, "leftarm_bb_");
    let goal = [(shoulder.x + moved.x) / 2.0, (shoulder.y + moved.y) / 2.0 + 4.0];
    drag(&mut app, [moved.x, moved.y], goal, egui::Modifiers::COMMAND);
    let reached = find(&app, "lefthand_bb_");
    assert!((reached.x - goal[0]).abs() < 1.0 && (reached.y - goal[1]).abs() < 1.0, "{reached:?} vs {goal:?}");
}

#[test]
fn dragging_elsewhere_moves_the_camera() {
    let mut app = app();
    let cam = scene(&app).camera;
    let before = steps(&app);
    drag(&mut app, [20.0, 20.0], [60.0, 20.0], egui::Modifiers::NONE);
    assert_eq!(steps(&app), before + 1);
    assert!((scene(&app).camera.yaw - cam.yaw).abs() > 1.0, "orbit");
    let placement = scene(&app).placement;
    drag(&mut app, [20.0, 20.0], [30.0, 25.0], egui::Modifiers::SHIFT);
    let moved = scene(&app).placement;
    assert!((moved.m[4] - placement.m[4] - 10.0).abs() < 1e-6 && (moved.m[5] - placement.m[5] - 5.0).abs() < 1e-6, "pan");
    let d = scene(&app).camera.distance;
    drag(&mut app, [20.0, 20.0], [20.0, 80.0], egui::Modifiers::ALT);
    assert!(scene(&app).camera.distance > d, "dolly out");
    // A plain press and release changes nothing.
    let n = steps(&app);
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 50.0 }, egui::Modifiers::NONE);
    assert_eq!(steps(&app), n);
}
