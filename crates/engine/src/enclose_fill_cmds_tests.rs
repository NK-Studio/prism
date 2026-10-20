use super::*;
use prism_geom::Rect;

/// A white Background with a black square outline (10..=30) and an empty, active layer above.
fn session(depth: u32) -> (Session, prism_doc::LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 40, "depth": depth, "background": "white"})).unwrap();
    let bottom = s.active().unwrap().active_layer.unwrap();
    s.edit("lines", |doc, _| {
        let surf = doc.layer_mut(bottom).unwrap().surface_mut().unwrap();
        let ink = [0.0, 0.0, 0.0, 1.0];
        surf.fill_rect(Rect::new(10, 10, 31, 11), &ink);
        surf.fill_rect(Rect::new(10, 30, 31, 31), &ink);
        surf.fill_rect(Rect::new(10, 10, 11, 31), &ink);
        surf.fill_rect(Rect::new(30, 10, 31, 31), &ink);
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    (s, bottom)
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

fn lasso(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
}

#[test]
fn fills_the_enclosed_area_on_the_active_layer_at_all_depths() {
    for depth in [8, 16, 32] {
        let (mut s, bottom) = session(depth);
        let before = s.active().unwrap().doc.layer(bottom).unwrap().clone();
        let r = s.execute(CMD, json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "color": "#ff0000", "antiAlias": false})).unwrap();
        assert_eq!(r, json!({"filled": true, "areas": 1}), "depth {depth}");
        assert_eq!(px(&s, 20, 20), vec![1.0, 0.0, 0.0, 1.0], "inside the outline, depth {depth}");
        assert_eq!(px(&s, 6, 6), vec![0.0; 4], "the open ring touching the lasso, depth {depth}");
        assert_eq!(px(&s, 10, 20), vec![1.0, 0.0, 0.0, 1.0], "under the line itself, depth {depth}");
        assert_eq!(px(&s, 9, 20), vec![0.0; 4], "not past the line, depth {depth}");
        assert_eq!(s.active().unwrap().doc.layer(bottom).unwrap(), &before, "only the active layer is written");
        assert!(s.undo(), "one history step");
        assert_eq!(px(&s, 20, 20), vec![0.0; 4]);
    }
}

#[test]
fn a_lasso_cutting_through_the_area_fills_only_the_lines_inside_it() {
    let (mut s, _) = session(8);
    let r = s.execute(CMD, json!({"points": lasso(4.0, 4.0, 20.0, 36.0), "antiAlias": false})).unwrap();
    assert_eq!(r, json!({"filled": true, "areas": 1}));
    assert_eq!(px(&s, 15, 20), vec![0.0; 4], "the open inside continues outside the lasso");
    assert_eq!(px(&s, 10, 20), vec![0.0, 0.0, 0.0, 1.0], "the line inside the lasso");
    assert_eq!(px(&s, 25, 10), vec![0.0; 4], "the line outside the lasso");
}

#[test]
fn a_lasso_around_open_space_fills_nothing() {
    let (mut s, _) = session(8);
    let r = s.execute(CMD, json!({"points": lasso(34.0, 4.0, 46.0, 36.0)})).unwrap();
    assert_eq!(r, json!({"filled": false, "areas": 0}));
    let layers = |s: &Session| s.active().unwrap().doc.layers.len();
    assert_eq!(layers(&s), 2);
    assert!(s.undo());
    assert_eq!(layers(&s), 1, "no history step when nothing is filled: undo removes the new layer");
}

#[test]
fn close_gap_and_expand_params_apply() {
    let (mut s, bottom) = session(8);
    // Open a two-pixel gap in the top edge.
    s.edit("gap", |doc, _| {
        doc.layer_mut(bottom).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 10, 22, 11), &[1.0, 1.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let pts = lasso(4.0, 4.0, 40.0, 36.0);
    s.execute(CMD, json!({"points": pts})).unwrap();
    assert_eq!(px(&s, 20, 20), vec![0.0; 4], "the gap leaks: only the lines are filled");
    assert!(s.undo());
    let r = s.execute(CMD, json!({"points": pts, "closeGap": 3, "antiAlias": false})).unwrap();
    assert_eq!(r["filled"], true);
    assert_eq!(px(&s, 20, 20), vec![0.0, 0.0, 0.0, 1.0], "foreground colour inside");
    assert_eq!(px(&s, 10, 20), vec![0.0, 0.0, 0.0, 1.0], "under the line");
    assert_eq!(px(&s, 20, 6), vec![0.0; 4], "nothing leaks through the sealed gap");
    assert!(s.undo());
    s.execute(CMD, json!({"points": pts, "closeGap": 3, "expand": -1, "antiAlias": false})).unwrap();
    assert_eq!(px(&s, 10, 20), vec![0.0; 4], "shrunk off the line's outer pixel");
    assert_eq!(px(&s, 11, 20), vec![0.0, 0.0, 0.0, 1.0], "but not off the inside");
}

#[test]
fn sample_active_layer_only_sees_no_lines_there() {
    let (mut s, _) = session(8);
    // The empty active layer has no line art: the whole lasso is one open area touching it.
    let r = s.execute(CMD, json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "sampleAllLayers": false})).unwrap();
    assert_eq!(r["filled"], false);
}

#[test]
fn bad_params_are_errors_not_panics() {
    let (mut s, _) = session(8);
    let bad = [
        json!({}),
        json!({"points": "lasso"}),
        json!({"points": [[0, 0], [1, 1]]}),
        json!({"points": [[0, 0], [1, "x"], [2, 2]]}),
        json!({"points": [[0, 0], [1e30, 1], [2, 2]]}),
        json!({"points": [[0], [1, 1], [2, 2]]}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "tolerance": -1}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "tolerance": "high"}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "closeGap": 1e9}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "expand": -1000}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "opacity": 101}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "antiAlias": "yes"}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "color": "#12"}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "color": [2, 0, 0]}),
        json!({"points": lasso(4.0, 4.0, 40.0, 36.0), "color": 7}),
    ];
    for p in bad {
        assert!(matches!(s.execute(CMD, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    assert_eq!(px(&s, 20, 20), vec![0.0; 4], "nothing was filled");
    // Off-canvas or degenerate outlines are valid and simply fill nothing.
    assert_eq!(s.execute(CMD, json!({"points": lasso(-500.0, -500.0, -400.0, -400.0)})).unwrap()["filled"], false);
    assert_eq!(s.execute(CMD, json!({"points": [[5, 5], [5, 5], [5, 5]]})).unwrap()["filled"], false);
}

#[test]
fn needs_a_paintable_layer() {
    let mut s = Session::new();
    assert!(s.execute(CMD, json!({"points": lasso(0.0, 0.0, 4.0, 4.0)})).is_err(), "no document");
}
