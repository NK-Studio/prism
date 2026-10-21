use super::*;

fn session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Ruled"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    (s, id)
}

fn isometric(s: &mut Session) {
    s.execute(ADD_PERSPECTIVE, json!({"points": [{"parallel": 30.6163}, {"parallel": 149.3961}, {"parallel": 270}]})).unwrap();
}

#[test]
fn add_list_edit_delete_are_undoable_layer_data() {
    let (mut s, id) = session();
    let past = s.active().unwrap().history.past_len();
    isometric(&mut s);
    let r = s.execute(ADD_PARALLEL, json!({"angle": -45, "snap": false})).unwrap();
    assert_eq!(r["index"], 1);
    let list = s.execute(LIST, json!({})).unwrap();
    assert_eq!(list["own"]["layer"], id.0);
    assert_eq!(list["own"]["rulers"].as_array().unwrap().len(), 2);
    assert_eq!(list["own"]["rulers"][1]["angleDeg"], 315.0, "angles are stored in [0, 360)");
    assert_eq!(list["own"]["rulers"][1]["center"], json!([100.0, 50.0]), "the centre defaults to the document's");
    assert_eq!(list["applies"].as_array().unwrap().len(), 1);
    assert_eq!(s.active().unwrap().history.past_len(), past + 2);

    s.execute(EDIT, json!({"index": 0, "point": 2, "x": 10, "y": -500, "coalesce": "drag"})).unwrap();
    s.execute(EDIT, json!({"index": 0, "point": 2, "x": 12, "y": -520, "coalesce": "drag"})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), past + 3, "a coalesced handle drag is one step");
    let r = s.active().unwrap().doc.layer(id).unwrap().special_rulers.clone().unwrap();
    let RulerKind::Perspective(p) = &r.rulers[0].kind else { panic!("perspective") };
    assert_eq!(p.points[2].target, VanishTarget::Point([12.0, -520.0]));
    assert_eq!(p.points[2].handle, [12.0, -520.0]);

    s.execute(SET_SHOW, json!({"shown": false, "range": "editing"})).unwrap();
    assert!(s.execute(LIST, json!({})).unwrap()["applies"].as_array().unwrap().is_empty());
    s.execute(DELETE, json!({"index": 1})).unwrap();
    assert_eq!(s.execute(DELETE, json!({})).unwrap()["removed"], 1);
    // A hidden, editing-only empty set is kept: those choices are still the user's.
    assert!(s.active().unwrap().doc.layer(id).unwrap().special_rulers.is_some());
    s.execute(SET_SHOW, json!({"shown": true, "range": "all"})).unwrap();
    assert!(s.active().unwrap().doc.layer(id).unwrap().special_rulers.is_none(), "an empty default set is dropped");
    while s.active().unwrap().history.past_len() > past {
        s.undo();
    }
    assert!(s.active().unwrap().doc.layer(id).unwrap().special_rulers.is_none());
}

#[test]
fn snap_points_follow_the_isometric_ruler_and_keep_pressure() {
    let (mut s, _) = session();
    isometric(&mut s);
    // A drag at about 28° (y up), so up and to the right on screen.
    let pts: Vec<Value> = (0..12).map(|i| json!([20.0 + 4.0 * i as f64, 80.0 - 2.1 * i as f64, 0.5])).collect();
    let r = s.execute(SNAP_POINTS, json!({"points": pts})).unwrap();
    let out = r["points"].as_array().unwrap();
    assert_eq!(out.len(), 12);
    let dir = prism_doc::special_ruler::angle_dir(30.6163);
    for p in out {
        let (x, y) = (p[0].as_f64().unwrap() - 20.0, p[1].as_f64().unwrap() - 80.0);
        assert!((x * dir[1] - y * dir[0]).abs() < 1e-6, "{p}");
        assert_eq!(p[2], 0.5, "pressure is kept");
    }
    assert_eq!(r["line"]["point"], 0);
    // No rulers: points pass through.
    s.execute(DELETE, json!({})).unwrap();
    let r = s.execute(SNAP_POINTS, json!({"points": [[1, 2], [30, 9]]})).unwrap();
    assert_eq!(r["points"], json!([[1.0, 2.0], [30.0, 9.0]]));
    assert!(r["line"].is_null());
}

#[test]
fn bad_params_fail_without_changing_the_document() {
    let (mut s, _) = session();
    isometric(&mut s);
    let before = s.active().unwrap().doc.clone();
    let many: Vec<Value> = (0..=MAX_VANISHING_POINTS).map(|_| json!({"parallel": 0})).collect();
    for (cmd, p) in [
        (ADD_PARALLEL, json!({})),
        (ADD_PARALLEL, json!({"angle": "x"})),
        (ADD_PARALLEL, json!({"angle": 1, "center": [1]})),
        (ADD_PARALLEL, json!({"angle": 1, "center": [1e12, 0]})),
        (ADD_PARALLEL, json!({"angle": 1, "layer": 999_999})),
        (ADD_PARALLEL, json!({"angle": 1, "layer": -1})),
        (ADD_PERSPECTIVE, json!({"points": []})),
        (ADD_PERSPECTIVE, json!({"points": many})),
        (ADD_PERSPECTIVE, json!({"points": [{"x": 1}]})),
        (ADD_PERSPECTIVE, json!({"points": [{"x": 1, "y": 2, "parallel": 3}]})),
        (ADD_PERSPECTIVE, json!({"points": [7]})),
        (ADD_PERSPECTIVE, json!({"points": [{"parallel": 1}], "eyeLevel": "up"})),
        (EDIT, json!({})),
        (EDIT, json!({"index": 9})),
        (EDIT, json!({"index": 0, "angle": 3})),
        (EDIT, json!({"index": 0, "x": 3, "y": 4})),
        (EDIT, json!({"index": 0, "point": 9, "x": 3, "y": 4})),
        (EDIT, json!({"index": 0, "point": 0, "x": 3})),
        (EDIT, json!({"index": 0, "point": 0, "x": 3, "y": 4, "parallel": 5})),
        (EDIT, json!({"index": 0, "guides": [[1, 2]]})),
        (EDIT, json!({"index": 0, "point": 0, "guides": [[1]]})),
        (EDIT, json!({"index": 0, "point": 0, "guides": "x"})),
        (EDIT, json!({"index": 0, "point": 0, "guides": vec![[0, 0]; MAX_GUIDES + 1]})),
        (DELETE, json!({"index": 5})),
        (DELETE, json!({"index": "all"})),
        (SET_SHOW, json!({})),
        (SET_SHOW, json!({"range": "everywhere"})),
        (SET_SHOW, json!({"shown": 1})),
        (SNAP_POINTS, json!({})),
        (SNAP_POINTS, json!({"points": [[1]]})),
        (SNAP_POINTS, json!({"points": [[1, "y"]]})),
        (SNAP_POINTS, json!({"points": [], "zoom": 0})),
        (LIST, json!({"layer": "x"})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
        assert_eq!(s.active().unwrap().doc, before, "{cmd} {p}");
    }
    assert_eq!(s.execute(SNAP_POINTS, json!({"points": []})).unwrap()["points"], json!([]));
}

#[test]
fn rulers_need_a_document_and_layer() {
    let mut s = Session::new();
    assert!(s.execute(ADD_PARALLEL, json!({"angle": 0})).is_err());
    assert!(s.execute(SNAP_POINTS, json!({"points": [[0, 0]]})).is_err());
}

#[test]
fn guides_are_replaced_and_a_coalesced_drag_is_one_step() {
    let (mut s, id) = session();
    isometric(&mut s);
    let past = s.active().unwrap().history.past_len();
    for a in [40.0, 44.0, 45.0] {
        s.execute(EDIT, json!({"index": 0, "point": 0, "parallel": a, "guides": [[10, 20]], "coalesce": "ruler-handle-1"})).unwrap();
    }
    assert_eq!(s.active().unwrap().history.past_len(), past + 1);
    let r = s.active().unwrap().doc.layer(id).unwrap().special_rulers.clone().unwrap();
    let RulerKind::Perspective(p) = &r.rulers[0].kind else { panic!("perspective") };
    assert_eq!((p.points[0].target, p.points[0].guides.clone()), (VanishTarget::Parallel { angle_deg: 45.0 }, vec![[10.0, 20.0]]));
    s.undo();
    let r = s.active().unwrap().doc.layer(id).unwrap().special_rulers.clone().unwrap();
    let RulerKind::Perspective(p) = &r.rulers[0].kind else { panic!("perspective") };
    assert!(p.points[0].guides.is_empty());
}
