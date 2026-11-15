use super::*;

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    s
}

fn active(s: &Session) -> LayerId {
    s.active().unwrap().active_layer.unwrap()
}

fn layer(s: &Session, id: LayerId) -> Layer {
    s.active().unwrap().doc.layer(id).unwrap().clone()
}

fn bounds(s: &Session, id: LayerId) -> Rect {
    layer(s, id).surface().unwrap().content_bounds()
}

fn joint(s: &Session, id: LayerId, name: &str) -> [f64; 2] {
    joints(&s.active().unwrap().doc, id).into_iter().find(|j| j.name == name).unwrap().doc
}

const PEP: &str = r#"<?xml version="1.0" encoding="UTF-8"?><celsysdocument><pose version="1.0.14"><node name="root_group_" rotquat="1 0 0 0" trans="40 0 0"><node name="hips_bb_" rotquat="1 0 0 0" trans="0 90 0"><node name="spine_bb_" rotquat="1 0 0 0" trans="0 30 0"><node name="head_bb_" rotquat="1 0 0 0" trans="0 30 0"><node name="head_end_bb_" rotquat="1 0 0 0" trans="0 20 0"/></node></node></node></node></pose></celsysdocument>"#;

#[test]
fn a_new_figure_layer_renders_the_standing_figure_in_one_step() {
    let mut s = session(200, 300);
    let steps = s.active().unwrap().history.entries().len();
    let r = s.execute(NEW, json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(id, active(&s));
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    let l = layer(&s, id);
    assert!(l.figure.is_some() && l.name.starts_with("3D Figure"));
    let b = bounds(&s, id);
    assert!(!b.is_empty() && b.y0 >= 0 && b.y1 <= 300 && (180..=260).contains(&b.height()), "{b:?}");
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(id).is_none());
}

#[test]
fn camera_joints_and_reach_re_render() {
    let mut s = session(200, 300);
    let id = LayerId(s.execute(NEW, json!({})).unwrap()["layer"].as_u64().unwrap());
    let before = bounds(&s, id);
    // Orbit to the side: the figure gets narrower. Two coalesced drags are one history step.
    let steps = s.active().unwrap().history.entries().len();
    s.execute(CAMERA, json!({"orbitBy": [45.0, 0.0], "draft": true, "coalesce": "orbit"})).unwrap();
    s.execute(CAMERA, json!({"orbitBy": [45.0, 0.0], "coalesce": "orbit"})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    let side = bounds(&s, id);
    assert!(side.width() < before.width(), "{side:?} vs {before:?}");
    s.execute(CAMERA, json!({"yaw": 0.0})).unwrap();
    // Turn the left forearm about the view axis: the hand moves, the elbow stays.
    let (elbow, hand) = (joint(&s, id, "leftforearm_bb_"), joint(&s, id, "lefthand_bb_"));
    s.execute(JOINT, json!({"joint": "leftforearm_bb_", "rotateBy": {"axis": "view", "degrees": 60.0}})).unwrap();
    let moved = joint(&s, id, "lefthand_bb_");
    let (e2, d) = (joint(&s, id, "leftforearm_bb_"), (moved[0] - hand[0]).hypot(moved[1] - hand[1]));
    assert!((e2[0] - elbow[0]).abs() < 1e-6 && d > 5.0, "{d}");
    // Reach: the hand goes to the point.
    let goal = [moved[0] + 4.0, moved[1] - 6.0];
    s.execute(REACH, json!({"joint": "lefthand_bb_", "point": goal})).unwrap();
    let got = joint(&s, id, "lefthand_bb_");
    assert!((got[0] - goal[0]).abs() < 1e-3 && (got[1] - goal[1]).abs() < 1e-3, "{got:?} vs {goal:?}");
    s.execute(RESET, json!({})).unwrap();
    assert!((joint(&s, id, "lefthand_bb_")[1] - hand[1]).abs() < 1e-6, "back to standing");
    s.execute(JOINT, json!({"joint": "head_bb_", "rotation": [0.9, 0.0, 0.4, 0.0]})).unwrap();
    // Twisting a bone about itself keeps its far end where it is.
    let wrist = joint(&s, id, "lefthand_bb_");
    s.execute(JOINT, json!({"joint": "leftforearm_bb_", "rotateBy": {"axis": "bone", "degrees": 90.0}})).unwrap();
    let after = joint(&s, id, "lefthand_bb_");
    assert!((after[0] - wrist[0]).abs() < 1e-6 && (after[1] - wrist[1]).abs() < 1e-6);
    assert!(s.execute(JOINT, json!({"joint": "head_end_bb_", "rotateBy": {"axis": "bone", "degrees": 5.0}})).is_err(), "a tip has no bone");
    s.execute(LOOK, json!({"color": "#ff8000", "outline": false, "outlineWidth": 3.0})).unwrap();
    assert_eq!(layer(&s, id).figure.unwrap().look.color, [1.0, 128.0 / 255.0, 0.0]);
    s.execute(FRAME, json!({"faceFront": true})).unwrap();
}

#[test]
fn poses_from_files_add_figures_and_bad_params_fail() {
    let mut s = session(300, 200);
    let data = prism_paint::tile::b64_encode(PEP.as_bytes());
    let id = LayerId(s.execute(NEW, json!({"data": data, "name": "Posed"})).unwrap()["layer"].as_u64().unwrap());
    let one = layer(&s, id).figure.unwrap();
    assert_eq!(one.figures.len(), 1);
    assert_eq!(layer(&s, id).name, "Posed");
    s.execute(ADD, json!({"data": data})).unwrap();
    assert_eq!(layer(&s, id).figure.unwrap().figures.len(), 2);
    s.execute(REMOVE, json!({"figure": 1})).unwrap();
    assert!(s.execute(REMOVE, json!({"figure": 0})).is_err(), "keeps one");
    for (cmd, p) in [
        (NEW, json!({"data": "%%%"})),
        (NEW, json!({"data": prism_paint::tile::b64_encode(b"<pose/>")})),
        (NEW, json!({"uuid": "x", "root": "/nonexistent/prism-csp-root"})),
        (NEW, json!({"uuid": "x", "data": data})),
        (NEW, json!({"at": [1.0]})),
        (NEW, json!({"name": ""})),
        (ADD, json!({})),
        (CAMERA, json!({"pitch": 120.0})),
        (CAMERA, json!({"distance": 0.0})),
        (CAMERA, json!({"orbitBy": "x"})),
        (JOINT, json!({"joint": "hips_bb_"})),
        (JOINT, json!({"joint": "tail_bb_", "rotateBy": {"axis": "view", "degrees": 5.0}})),
        (JOINT, json!({"joint": "hips_bb_", "rotation": [0.0, 0.0, 0.0, 0.0]})),
        (JOINT, json!({"joint": "hips_bb_", "rotation": [1.0, 0.0, 0.0, 0.0], "rotateBy": {"axis": "view", "degrees": 5.0}})),
        (JOINT, json!({"figure": 7, "joint": "hips_bb_", "rotation": [1.0, 0.0, 0.0, 0.0]})),
        (REACH, json!({"joint": "head_end_bb_"})),
        (LOOK, json!({"color": "red"})),
        (LOOK, json!({"outlineWidth": -1.0})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
    }
    // Not a figure layer.
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(s.execute(CAMERA, json!({"orbitBy": [1.0, 0.0]})).is_err());
    assert!(s.execute(RASTERIZE, json!({})).is_err());
    assert!(Session::new().execute(NEW, json!({})).is_err(), "no document");
}

#[test]
fn pixels_are_locked_but_layer_geometry_follows_the_figure() {
    let mut s = session(200, 300);
    let id = LayerId(s.execute(NEW, json!({})).unwrap()["layer"].as_u64().unwrap());
    let err = s.execute("edit.fill", json!({"contents": "black"})).unwrap_err().to_string();
    assert!(err.contains("3D figure"), "{err}");
    assert!(s.execute("image.adjustments.invert", json!({})).is_err());
    assert!(s.execute("edit.transform.warp", json!({"style": "arc", "bend": 30})).is_err() || layer(&s, id).figure.is_some());
    // Move: pixels shift, and a re-render lands on them.
    let before = bounds(&s, id);
    s.execute("layer.translate", json!({"dx": 13, "dy": -7})).unwrap();
    let moved = bounds(&s, id);
    assert_eq!((moved.x0, moved.y0), (before.x0 + 13, before.y0 - 7));
    s.execute(LOOK, json!({"outline": true})).unwrap();
    let rerendered = bounds(&s, id);
    assert!((rerendered.x0 - moved.x0).abs() <= 1 && (rerendered.y0 - moved.y0).abs() <= 1, "{rerendered:?} vs {moved:?}");
    // Free Transform at half size: the scene shrinks with it and re-renders sharp.
    s.execute("edit.transform.numeric", json!({"scaleX": 50.0, "scaleY": 50.0})).unwrap();
    let half = bounds(&s, id);
    assert!((f64::from(half.height()) / f64::from(rerendered.height()) - 0.5).abs() < 0.05, "{half:?}");
    assert!(layer(&s, id).figure.is_some());
    // Image Size doubles the document: the figure follows.
    s.execute("image.imageSize", json!({"width": 400, "height": 600})).unwrap();
    let doubled = bounds(&s, id);
    assert!((f64::from(doubled.height()) / f64::from(half.height()) - 2.0).abs() < 0.1, "{doubled:?}");
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    let turned = bounds(&s, id);
    assert!((i64::from(turned.width()) - i64::from(doubled.height())).abs() <= 2, "{turned:?} vs {doubled:?}");
    // Rasterize: plain pixels that can be painted.
    s.execute("layer.rasterize.layer", json!({})).unwrap();
    assert!(layer(&s, id).figure.is_none());
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
}

#[test]
fn hand_poses_close_and_open_the_hands_in_one_step() {
    let mut s = session(200, 300);
    let id = LayerId(s.execute(NEW, json!({})).unwrap()["layer"].as_u64().unwrap());
    let rot = |s: &Session, n: &str| layer(s, id).figure.unwrap().figures[0].root.find(n).unwrap().rotation;
    let steps = s.active().unwrap().history.entries().len();
    s.execute(HAND, json!({"weights": [0, 0, 1], "closed": "tube", "locked": ["thumb"]})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    for side in ["left", "right"] {
        assert_ne!(rot(&s, &format!("{side}handmiddle2_bb_")), [1.0, 0.0, 0.0, 0.0], "{side} closed");
        assert_eq!(rot(&s, &format!("{side}handthumb2_bb_")), [1.0, 0.0, 0.0, 0.0], "{side} thumb locked");
    }
    // One side only.
    s.execute(HAND, json!({"weights": [1, 0, 0], "side": "left"})).unwrap();
    let left = rot(&s, "lefthandmiddle2_bb_");
    assert!(left[0] > 0.99, "the left hand opens: {left:?}");
    assert!(rot(&s, "righthandmiddle2_bb_")[0] < 0.9, "the right stays closed");
    assert!(s.undo());
    assert!(rot(&s, "lefthandmiddle2_bb_")[0] < 0.9);
    for bad in [
        json!({}),
        json!({"weights": [0, 0, 0]}),
        json!({"weights": [-1, 2, 0]}),
        json!({"weights": [1, 0]}),
        json!({"weights": "fist"}),
        json!({"weights": [1, 0, 0], "side": "middle"}),
        json!({"weights": [1, 0, 0], "closed": "paw"}),
        json!({"weights": [1, 0, 0], "locked": ["toe"]}),
        json!({"weights": [1, 0, 0], "locked": "thumb"}),
        json!({"weights": [1, 0, 0], "figure": 3}),
    ] {
        assert!(s.execute(HAND, bad.clone()).is_err(), "{bad}");
    }
    // A figure without hands.
    s.execute(NEW, json!({"data": prism_paint::tile::b64_encode(PEP.as_bytes())})).unwrap();
    assert!(s.execute(HAND, json!({"weights": [1, 0, 0]})).is_err());
}
