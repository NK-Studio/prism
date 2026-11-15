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

/// A CLIP STUDIO-style left hand pose (its `handsetup_*.pep` layout; synthetic numbers): the
/// fingers' chains as top nodes, curled about −Z.
const HAND_PEP: &str = r#"<celsysdocument><pose alias_name="Fist" search_tags="hand_pose" version="1.0.0">
<node name="lefthandthumb1_bb_" rotquat="0.9 0.3 0 0" trans="1.3 -1.1 1.9"><node name="lefthandthumb2_bb_" rotquat="0.9 0 0 -0.4" trans="3.4 0 0"/></node>
<node name="lefthandindex1_bb_" rotquat="0.7 0 0 -0.7" trans="6.6 0 1.5"><node name="lefthandindex2_bb_" rotquat="0.6 0 0 -0.8" trans="3.3 0 0"><node name="lefthandindex3_bb_" rotquat="0.7 0 0 -0.7" trans="2.0 0 0"/></node></node>
<node name="lefthandmiddle1_bb_" rotquat="0.7 0 0 -0.7" trans="6.6 0 0"><node name="lefthandmiddle2_bb_" rotquat="0.6 0 0 -0.8" trans="3.8 0 0"/></node>
<node name="lefthandring1_bb_" rotquat="0.7 0 0 -0.7" trans="6.3 0 -1.5"/>
<node name="lefthandpinky1_bb_" rotquat="0.7 0 0 -0.7" trans="5.8 -0.4 -2.8"/>
</pose></celsysdocument>"#;

#[test]
fn hand_poses_pose_the_hands_of_a_figure_layer() {
    let mut s = session(200, 300);
    let id = LayerId(s.execute(NEW, json!({})).unwrap()["layer"].as_u64().unwrap());
    let rot = |s: &Session, id: LayerId, n: &str| layer(s, id).figure.unwrap().figures[0].root.find(n).unwrap().rotation;
    let data = prism_paint::tile::b64_encode(HAND_PEP.as_bytes());
    let steps = s.active().unwrap().history.entries().len();
    s.execute(HAND_POSE, json!({"data": data})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    for side in ["left", "right"] {
        assert!(rot(&s, id, &format!("{side}handmiddle2_bb_"))[0] < 0.9, "{side} hand closed, mirrored for the right");
    }
    assert_eq!(rot(&s, id, "lefthandring2_bb_"), [1.0, 0.0, 0.0, 0.0], "joints the pose lacks stay");
    // One hand only; locked fingers stay.
    s.execute(HAND, json!({"weights": [1, 0, 0]})).unwrap();
    s.execute(HAND_POSE, json!({"data": data, "side": "right", "locked": ["index"]})).unwrap();
    assert!(rot(&s, id, "lefthandmiddle2_bb_")[0] > 0.99, "the left hand stays open");
    assert!(rot(&s, id, "righthandmiddle2_bb_")[0] < 0.9);
    assert!(rot(&s, id, "righthandindex2_bb_")[0] > 0.99, "the index is locked");
    // A hand pose alone makes Prism's standing figure with those hands.
    let hands = LayerId(s.execute(NEW, json!({"data": data})).unwrap()["layer"].as_u64().unwrap());
    assert!(rot(&s, hands, "righthandmiddle2_bb_")[0] < 0.9);
    // Where it is no hand pose, or the params are wrong.
    let body = prism_paint::tile::b64_encode(PEP.as_bytes());
    for bad in [
        json!({}),
        json!({"data": body}),
        json!({"data": data, "side": "middle"}),
        json!({"data": data, "figure": 4}),
        json!({"data": data, "locked": "thumb"}),
        json!({"data": data, "at": [1, "x"]}),
        json!({"data": "%%%"}),
    ] {
        assert!(s.execute(HAND_POSE, bad.clone()).is_err(), "{bad}");
    }
    assert!(s.execute(ADD, json!({"data": data})).is_err(), "a hand pose adds no figure");
    // A figure without hands.
    s.execute(NEW, json!({"data": body})).unwrap();
    assert!(s.execute(HAND_POSE, json!({"data": data})).is_err());
    // Dropped at a point: the figure nearest it.
    s.execute(NEW, json!({})).unwrap();
    let pair = active(&s);
    s.execute(ADD, json!({"data": body})).unwrap();
    let head = joint(&s, pair, "head_bb_");
    s.execute(HAND_POSE, json!({"data": data, "at": head})).unwrap();
    assert!(rot(&s, pair, "lefthandmiddle2_bb_")[0] < 0.9, "the standing figure, nearest the point");
}

#[test]
fn a_pose_goes_onto_one_figure_where_it_stands() {
    let mut s = session(200, 300);
    let id = LayerId(s.execute(NEW, json!({})).unwrap()["layer"].as_u64().unwrap());
    let fig = |s: &Session, i: usize| layer(s, id).figure.unwrap().figures[i].clone();
    // Put the figure somewhere else first: its root keeps that.
    s.execute(JOINT, json!({"joint": "root_group_", "rotation": [0.92387953, 0.0, 0.38268343, 0.0]})).ok();
    let before = fig(&s, 0);
    let data = prism_paint::tile::b64_encode(PEP.as_bytes());
    let steps = s.active().unwrap().history.entries().len();
    s.execute(POSE, json!({"data": data})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    let after = fig(&s, 0);
    assert_eq!((after.root.offset, after.root.rotation), (before.root.offset, before.root.rotation), "it stays where it stood");
    assert!(after.root.find("head_end_bb_").is_some() && after.root.find("lefthand_bb_").is_none(), "the joints are the pose's");
    assert_eq!(after.root.find("hips_bb_").unwrap().offset, [0.0, 90.0, 0.0]);
    assert!(s.undo());
    assert!(fig(&s, 0).root.find("lefthand_bb_").is_some());
    // Two figures: name one (or a point); a hand pose or two poses are refused.
    s.execute(ADD, json!({"data": data})).unwrap();
    assert!(s.execute(POSE, json!({"data": data})).is_err(), "which figure?");
    s.execute(POSE, json!({"data": data, "figure": 0})).unwrap();
    assert!(fig(&s, 0).root.find("lefthand_bb_").is_none());
    let hand = prism_paint::tile::b64_encode(HAND_PEP.as_bytes());
    for bad in [json!({"data": hand, "figure": 0}), json!({"figure": 0}), json!({"data": data, "figure": 7}), json!({"data": "%%%", "figure": 0}), json!({"data": data, "at": ["x", 1]})] {
        assert!(s.execute(POSE, bad.clone()).is_err(), "{bad}");
    }
    // A pose rooted at the hips hangs under the figure's own root.
    let hips_only = r#"<pose><node name="hips_bb_" rotquat="1 0 0 0" trans="0 80 0"><node name="spine_bb_" rotquat="1 0 0 0" trans="0 20 0"/></node></pose>"#;
    s.execute(POSE, json!({"data": prism_paint::tile::b64_encode(hips_only.as_bytes()), "figure": 1})).unwrap();
    let f = fig(&s, 1);
    assert_eq!(f.root.name, "root_group_");
    assert_eq!(f.root.children.first().map(|c| c.name.as_str()), Some("hips_bb_"));
}

/// A tiny glTF character as JSON with its buffer inline: hips (1 m up) and chest (50 cm above),
/// a 40 × 50 cm quad skinned half to each, a VRM 1.0 humanoid map naming both.
fn character_gltf() -> String {
    let floats = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut bin = floats(&[-0.2, 1.0, 0.0, 0.2, 1.0, 0.0, -0.2, 1.5, 0.0, 0.2, 1.5, 0.0]);
    bin.extend(floats(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]));
    bin.extend([0u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
    bin.extend(floats(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]));
    bin.extend([0u8, 0, 1, 0, 3, 0, 0, 0, 3, 0, 2, 0]);
    let ibm: Vec<f32> = [-1.0f32, -1.5].iter().flat_map(|y| [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, *y, 0.0, 1.0]).collect();
    bin.extend(floats(&ibm));
    let uri = format!("data:application/octet-stream;base64,{}", prism_paint::tile::b64_encode(&bin));
    json!({
        "asset": {"version": "2.0"},
        "buffers": [{"uri": uri, "byteLength": bin.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48}, {"buffer": 0, "byteOffset": 48, "byteLength": 48},
            {"buffer": 0, "byteOffset": 96, "byteLength": 16}, {"buffer": 0, "byteOffset": 112, "byteLength": 64},
            {"buffer": 0, "byteOffset": 176, "byteLength": 12}, {"buffer": 0, "byteOffset": 188, "byteLength": 128}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"}, {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5121, "count": 4, "type": "VEC4"}, {"bufferView": 3, "componentType": 5126, "count": 4, "type": "VEC4"},
            {"bufferView": 4, "componentType": 5123, "count": 6, "type": "SCALAR"}, {"bufferView": 5, "componentType": 5126, "count": 2, "type": "MAT4"}
        ],
        "nodes": [{"name": "Hips", "translation": [0.0, 1.0, 0.0], "children": [1]}, {"name": "Chest", "translation": [0.0, 0.5, 0.0]}, {"name": "Body", "mesh": 0, "skin": 0}],
        "skins": [{"joints": [0, 1], "inverseBindMatrices": 5}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "JOINTS_0": 2, "WEIGHTS_0": 3}, "indices": 4}]}],
        "extensions": {"VRMC_vrm": {"meta": {"name": "Quad Kid"}, "humanoid": {"humanBones": {"hips": {"node": 0}, "chest": {"node": 1}}}}}
    })
    .to_string()
}

#[test]
fn characters_import_render_pose_and_reset() {
    let mut s = session(200, 300);
    let data = prism_paint::tile::b64_encode(character_gltf().as_bytes());
    let steps = s.active().unwrap().history.entries().len();
    let r = s.execute(MODEL, json!({"data": data, "fileName": "kid.gltf"})).unwrap();
    assert_eq!(r["name"], "Quad Kid");
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    let l = layer(&s, id);
    assert_eq!(l.name, "Quad Kid");
    let f = &l.figure.as_ref().unwrap().figures[0];
    assert_eq!(f.model.as_ref().unwrap().file_name, "kid.gltf");
    assert!(f.root.find("hips_bb_").is_some() && f.root.find("spine1_bb_").is_some());
    let before = bounds(&s, id);
    // Framed with its root on the ground: the 50 cm quad 1 m up fills the top part.
    assert!(!before.is_empty() && before.height() > 50, "the quad is drawn and framed: {before:?}");
    // Posing the chest moves the drawing; Reset Pose brings the model's rest pose back.
    s.execute(JOINT, json!({"joint": "spine1_bb_", "rotateBy": {"axis": [0, 0, 1], "degrees": 90}})).unwrap();
    assert_ne!(bounds(&s, id), before);
    s.execute(RESET, json!({})).unwrap();
    assert_eq!(bounds(&s, id), before);
    assert!(layer(&s, id).figure.unwrap().figures[0].root.find("lefthand_bb_").is_none(), "its own skeleton, not the mannequin's");
    // A CLIP STUDIO pose is carried onto the character's own bones (not swapped in).
    s.execute(POSE, json!({"data": prism_paint::tile::b64_encode(PEP.as_bytes())})).unwrap();
    let posed = layer(&s, id).figure.unwrap().figures[0].clone();
    assert!(posed.root.find("spine1_bb_").is_some() && posed.root.find("head_end_bb_").is_none());
    assert!(posed.model.is_some());
    s.execute(RESET, json!({})).unwrap();
    // A second character into the same layer, beside the first; saved and loaded back whole.
    s.execute(MODEL, json!({"data": data, "layer": id.0})).unwrap();
    let scene = layer(&s, id).figure.unwrap();
    assert_eq!(scene.figures.len(), 2);
    assert!(scene.figures[1].root.offset[0] > 30.0);
    let doc = s.active().unwrap().doc.clone();
    let saved = prism_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(prism_format::load_from_bytes(&saved).unwrap(), *doc);
    // Bad files are errors.
    for bad in [json!({}), json!({"data": "%%%"}), json!({"data": prism_paint::tile::b64_encode(b"glTF broken")}), json!({"data": data, "layer": 999_999})] {
        assert!(s.execute(MODEL, bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn opening_or_placing_a_character_file_makes_a_figure_layer() {
    let bytes = character_gltf().into_bytes();
    let mut s = Session::new();
    let r = crate::file_cmds::open_bytes_as(&mut s, "kid.gltf", &bytes, None, None).unwrap();
    assert!(r["layer"].is_u64());
    let st = s.active().unwrap();
    assert_eq!(st.doc.name, "Quad Kid");
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(l.figure.as_ref().unwrap().figures[0].model.is_some());
    // Placed into an open document: a figure layer, not a flat smart object.
    let mut s = session(100, 100);
    let r = crate::file_cmds::place_bytes(&mut s, "kid.gltf", bytes.clone(), None, &json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert!(layer(&s, id).figure.is_some());
    // A broken character file opens nothing.
    let mut s = Session::new();
    assert!(crate::file_cmds::open_bytes_as(&mut s, "bad.vrm", b"glTFbroken", None, None).is_err());
    assert!(s.documents().is_empty());
}
