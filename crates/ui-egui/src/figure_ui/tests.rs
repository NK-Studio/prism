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

#[test]
fn finger_joint_names_parse() {
    assert_eq!(finger_joint("lefthandindex3_bb_"), Some(("lefthand_bb_".to_string(), 3)));
    assert_eq!(finger_joint("RightHandThumb1_bb_"), Some(("righthand_bb_".to_string(), 1)));
    for name in ["lefthand_bb_", "leftforearm_bb_", "lefthandindex_bb_", "lefthandindexx_bb_", "lefthandindex3", "hand", "ハンド", ""] {
        assert_eq!(finger_joint(name), None, "{name}");
    }
    assert!(is_end("lefthandpinky4_bb_") && !is_end("lefthandpinky3_bb_") && is_end("righthand_bb_"));
}

#[test]
fn zoomed_in_hands_show_their_finger_joints() {
    let mut app = app();
    assert!(!current(&app).iter().any(|h| h.finger), "a small hand shows no finger joints");
    app.ui.views[0].zoom = 8.0;
    let hs = current(&app);
    for side in ["left", "right"] {
        for digit in DIGITS {
            for seg in 1..=4 {
                let name = format!("{side}hand{digit}{seg}_bb_");
                let h = hs.iter().find(|h| h.joint == name).unwrap_or_else(|| panic!("{name}"));
                assert!(h.finger && h.end == (seg == 4), "{h:?}");
            }
        }
    }
    let v = inspect(&app);
    assert!(v.as_array().unwrap().iter().any(|h| h["finger"] == json!(true)));
}

#[test]
fn dragging_a_finger_joint_bends_that_finger_only() {
    let mut app = app();
    app.ui.views[0].zoom = 8.0;
    let (knuckle, tip) = (find(&app, "lefthandmiddle3_bb_"), find(&app, "lefthandmiddle4_bb_"));
    let others = |app: &PrismApp| ["lefthand_bb_", "lefthandmiddle3_bb_", "lefthandindex4_bb_", "righthandmiddle4_bb_"].map(|n| find(app, n));
    let before_others = others(&app);
    let before = steps(&app);
    // Curl the tip a quarter turn round its knuckle.
    let r = (tip.x - knuckle.x).hypot(tip.y - knuckle.y);
    assert!(r * 8.0 > 6.0, "the segment is long enough on screen to grab: {r}");
    let to = [knuckle.x - (tip.y - knuckle.y), knuckle.y + (tip.x - knuckle.x)];
    drag(&mut app, [tip.x, tip.y], to, egui::Modifiers::NONE);
    assert_eq!(steps(&app), before + 1, "one history step");
    let moved = find(&app, "lefthandmiddle4_bb_");
    assert!((moved.x - to[0]).hypot(moved.y - to[1]) < 0.25 * r, "the tip follows the pointer: {moved:?} vs {to:?}");
    for (a, b) in before_others.iter().zip(others(&app)) {
        assert!((a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6, "{} stays", a.joint);
    }
    // ⌘/Ctrl on a fingertip: reach with IK, bending the middle and end segments.
    let (base, tip) = (find(&app, "lefthandmiddle2_bb_"), find(&app, "lefthandmiddle4_bb_"));
    let goal = [(base.x + tip.x) / 2.0 + 0.3, (base.y + tip.y) / 2.0 + 0.3];
    drag(&mut app, [tip.x, tip.y], goal, egui::Modifiers::COMMAND);
    let reached = find(&app, "lefthandmiddle4_bb_");
    assert!((reached.x - goal[0]).hypot(reached.y - goal[1]) < 0.2, "{reached:?} vs {goal:?}");
    assert!((find(&app, "lefthandmiddle2_bb_").x - base.x).abs() < 1e-6, "the finger's base stays");
}

#[test]
fn overlapping_dots_pick_the_one_in_front() {
    let h = |joint: &str, x: f64, depth: f64| Handle { figure: 0, joint: joint.into(), parent: None, x, y: 0.0, end: false, finger: true, depth };
    let dots = [h("behind", 0.0, 120.0), h("front", 0.5, 100.0), h("far-off", 30.0, 10.0)];
    assert_eq!(at(&dots, [0.0, 0.0], 1.0).unwrap().joint, "front", "within the overlap, the nearer to the camera wins");
    // Clearly nearer on screen beats depth.
    let dots = [h("behind", 0.0, 120.0), h("front", 5.0, 100.0)];
    assert_eq!(at(&dots, [0.0, 0.0], 1.0).unwrap().joint, "behind");
    assert!(at(&dots, [50.0, 0.0], 1.0).is_none());
}

#[test]
fn the_hand_pad_maps_corners_to_shapes() {
    let r = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 120.0));
    let c = pad_corners(r);
    for (k, corner) in c.iter().enumerate() {
        let w = pad_weights(c, *corner);
        assert!((w[k] - 1.0).abs() < 1e-5, "{k}: {w:?}");
        assert!((pad_point(c, w) - *corner).length() < 1e-3);
    }
    // Outside the triangle clamps onto it; the centre blends all three.
    let w = pad_weights(c, egui::pos2(-50.0, -50.0));
    assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5 && w.iter().all(|v| *v >= 0.0));
    let centre = egui::pos2((c[0].x + c[1].x + c[2].x) / 3.0, (c[0].y + c[1].y + c[2].y) / 3.0);
    assert!(pad_weights(c, centre).iter().all(|v| (v - 1.0 / 3.0).abs() < 1e-4));
    assert_eq!(pad_weights([egui::Pos2::ZERO; 3], egui::Pos2::ZERO), [1.0, 0.0, 0.0], "a degenerate pad");
}

#[test]
fn the_hand_pad_runs_set_hand() {
    let mut app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap().0;
    let pad = HandPad { weights: [0.0, 0.0, 1.0], closed: 2, locked: [true, false, false, false, false], side: 1, ..Default::default() };
    let p = pad.params(layer);
    assert_eq!(p["closed"], "tube");
    assert_eq!(p["side"], "left");
    assert_eq!(p["locked"], json!(["thumb"]));
    app.run("layer.figure.setHand", p).unwrap();
    let f = scene(&app).figures[0].clone();
    assert_ne!(f.root.find("lefthandmiddle2_bb_").unwrap().rotation, [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(f.root.find("righthandmiddle2_bb_").unwrap().rotation, [1.0, 0.0, 0.0, 0.0], "left only");
    assert_eq!(f.root.find("lefthandthumb2_bb_").unwrap().rotation, [1.0, 0.0, 0.0, 0.0], "thumb locked");
    // The labels have their own translations: "Ring" is a finger, not a ring.
    let ko = crate::i18n::Lang::from_pref("ko");
    assert_eq!(crate::i18n::tr_ctx(ko, "hand", "Ring"), "약지");
    assert_eq!(crate::i18n::tr_ctx(ko, "hand", "Spread"), "벌린 손");
}

fn click_at(app: &mut PrismApp, p: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE);
    // A wobble of a point is still a click.
    tool_event(app, ToolEvent::Move { x: p[0] + 0.2, y: p[1], pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: p[0] + 0.2, y: p[1] }, egui::Modifiers::NONE);
}

#[test]
fn clicking_a_hand_picks_it_for_hand_poses() {
    let mut app = app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let rot = |app: &PrismApp, n: &str| scene(app).figures[0].root.find(n).unwrap().rotation;
    let steps = steps(&app);
    let hand = find(&app, "righthand_bb_");
    click_at(&mut app, [hand.x, hand.y]);
    assert_eq!(steps, crate::figure_ui::tests::steps(&app), "a click poses nothing");
    let pick = picked(&app, layer).expect("the right hand is picked");
    assert!(pick.hand == Some(false) && pick.figure == 0);
    assert_eq!(inspect_pick(&app)["hand"], json!("right"));
    // The hand setup and hand poses go to that hand only.
    let pad = HandPad { weights: [0.0, 0.0, 1.0], ..Default::default() };
    run_hand(&mut app, &pad, layer.0, false);
    assert!(rot(&app, "righthandmiddle2_bb_")[0] < 0.9);
    assert_eq!(rot(&app, "lefthandmiddle2_bb_"), [1.0, 0.0, 0.0, 0.0], "the left hand stays");
    // A click on another joint picks the whole figure (both hands again); a finger picks its
    // hand; empty canvas drops the pick.
    let knee = find(&app, "leftleg_bb_");
    click_at(&mut app, [knee.x, knee.y]);
    let pick = picked(&app, layer).expect("the figure is picked");
    assert!(pick.hand.is_none() && pick.figure == 0);
    assert!(inspect_pick(&app)["hand"].is_null());
    run_hand(&mut app, &HandPad { weights: [0.0, 1.0, 0.0], ..Default::default() }, layer.0, false);
    assert!(rot(&app, "lefthandmiddle2_bb_") != [1.0, 0.0, 0.0, 0.0] && rot(&app, "righthandmiddle2_bb_")[0] > 0.95, "both hands of the picked figure");
    app.ui.views[0].zoom = 8.0;
    let tip = find(&app, "lefthandindex4_bb_");
    click_at(&mut app, [tip.x, tip.y]);
    assert_eq!(picked(&app, layer).unwrap().hand, Some(true), "a finger picks its hand");
    app.ui.views[0].zoom = 1.0;
    click_at(&mut app, [5.0, 5.0]);
    assert!(picked(&app, layer).is_none() && inspect_pick(&app).is_null());
    // A drag still poses (and picks nothing).
    let (elbow, wrist) = (find(&app, "rightforearm_bb_"), find(&app, "righthand_bb_"));
    drag(&mut app, [wrist.x, wrist.y], [elbow.x - (wrist.y - elbow.y), elbow.y], egui::Modifiers::NONE);
    assert!(picked(&app, layer).is_none());
    assert!(crate::figure_ui::tests::steps(&app) > steps);
    // A pick on another layer counts for nothing here.
    app.figure_pick = Some(FigurePick { layer: LayerId(9999), figure: 0, hand: Some(true) });
    assert!(picked(&app, layer).is_none());
}

/// A tiny glTF character (JSON, buffer inline): hips with a hair bone, a quad on the hips.
fn tiny_character() -> Vec<u8> {
    let floats = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut bin = floats(&[-0.2, 1.0, 0.0, 0.2, 1.0, 0.0, -0.2, 1.4, 0.0, 0.2, 1.4, 0.0]);
    bin.extend([0u8, 0, 1, 0, 3, 0, 0, 0, 3, 0, 2, 0]);
    let uri = format!("data:application/octet-stream;base64,{}", prism_paint::tile::b64_encode(&bin));
    json!({
        "asset": {"version": "2.0"},
        "buffers": [{"uri": uri, "byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 48}, {"buffer": 0, "byteOffset": 48, "byteLength": 12}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"}, {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"}],
        "nodes": [{"name": "Hips", "translation": [0.0, 1.0, 0.0], "children": [1, 2]}, {"name": "Hair", "translation": [0.0, 0.6, 0.0]}, {"name": "Body", "mesh": 0}],
        "skins": [{"joints": [0, 1]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "extensions": {"VRMC_vrm": {"humanoid": {"humanBones": {"hips": {"node": 0}}}}}
    })
    .to_string()
    .into_bytes()
}

#[test]
fn characters_open_and_show_only_their_humanoid_joints() {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.open_bytes("tiny.gltf", &tiny_character()).unwrap();
    app.ui.tool = Tool::Move;
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let f = &l.figure.as_ref().unwrap().figures[0];
    assert!(f.model.is_some() && f.root.find("node:Hair").is_some());
    let names: Vec<String> = current(&app).into_iter().map(|h| h.joint).collect();
    assert!(names.contains(&"hips_bb_".to_string()), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("node:")), "hair bones get no dots: {names:?}");
}
