use super::*;

/// 96 × 64 canvas: vertical black stripes on white, a red blob, a selection around it.
fn blob(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 64, "depth": depth, "background": "white"})).unwrap();
    s.edit("stripes", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for x in (0..96).step_by(8) {
            surf.fill_rect(Rect::new(x, 0, x + 3, 64), &[0.0, 0.0, 0.0, 1.0]);
        }
        surf.fill_rect(Rect::new(44, 26, 52, 36), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 42, "y": 24, "width": 12, "height": 14})).unwrap();
    s
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

fn red(v: &[f32]) -> bool {
    v[0] > 0.9 && v[1] < 0.1
}

fn prep_of(s: &Session, p: Value) -> Prepared {
    let st = s.active().unwrap();
    let params = Params::parse(&st.doc, &p, CMD).unwrap();
    let g = geometry(&st.doc, &params, CMD).unwrap();
    prepare(&st.doc, st.active_layer.unwrap(), &params, &g, &Interrupt::NONE).unwrap()
}

#[test]
fn removes_the_blob_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut s = blob(depth);
        s.execute(CMD, json!({})).unwrap();
        for (x, y) in [(46, 28), (50, 33)] {
            assert!(!red(&px(&s, x, y)), "depth {depth}: red left at ({x},{y})");
        }
        assert_eq!(px(&s, 1, 1)[0], 0.0, "outside untouched");
    }
}

#[test]
fn sampling_brush_strokes_add_and_subtract() {
    let s = blob(8);
    // Custom with one added stroke: only that stroke is sampled.
    let p = prep_of(&s, json!({"sampling": "custom", "samplingStrokes": [{"mode": "add", "size": 10, "points": [[10, 10], [30, 10]]}]}));
    let at = |p: &Prepared, x: i32, y: i32| p.source[((y - p.window.y0) as usize) * p.window.width() as usize + (x - p.window.x0) as usize];
    assert!(at(&p, 20, 10) && !at(&p, 20, 20) && !at(&p, 46, 28));
    // Auto minus a subtracted stroke.
    let p = prep_of(&s, json!({"samplingStrokes": [{"mode": "subtract", "size": 8, "points": [[30, 30]]}]}));
    assert!(!at(&p, 30, 30) && at(&p, 30, 40));
    assert!(!at(&p, 46, 28), "never inside the fill area");
}

#[test]
fn lasso_ops_change_the_fill_area() {
    let s = blob(8);
    let p = prep_of(&s, json!({"fillOps": [{"op": "add", "points": [[5, 5], [15, 5], [15, 15], [5, 15]]}]}));
    let at = |p: &Prepared, x: i32, y: i32| p.cover[((y - p.window.y0) as usize) * p.window.width() as usize + (x - p.window.x0) as usize];
    assert!(at(&p, 10, 10) > 0.9 && at(&p, 46, 28) > 0.9);
    let p = prep_of(&s, json!({"fillOps": [{"op": "subtract", "points": [[40, 20], [48, 20], [48, 40], [40, 40]]}]}));
    assert!(at(&p, 44, 30) < 0.1 && at(&p, 50, 30) > 0.9);
    let p = prep_of(&s, json!({"fillOps": [{"op": "expand", "px": 3}]}));
    assert!(at(&p, 40, 30) > 0.9 && at(&p, 38, 30) < 0.1);
    let p = prep_of(&s, json!({"fillOps": [{"op": "contract", "px": 3}]}));
    assert!(at(&p, 43, 30) < 0.1 && at(&p, 46, 30) > 0.9);
}

#[test]
fn outputs_and_undo() {
    let mut s = blob(8);
    let base = s.active().unwrap().active_layer.unwrap();
    let r = s.execute(CMD, json!({"output": "new", "mirror": true, "colorAdaptation": "veryHigh"})).unwrap();
    let nid = LayerId(r["layer"].as_u64().unwrap());
    assert_ne!(nid, base);
    let b = s.active().unwrap().doc.layer(nid).unwrap().surface().unwrap().content_bounds();
    assert!(Rect::new(42, 24, 54, 38).contains_rect(&b), "{b:?}");
    let st = s.active().unwrap();
    assert!(red(&st.doc.layer(base).unwrap().surface().unwrap().pixel(46, 28)), "new layer: the base layer is untouched");
    s.undo();
    let r = s.execute(CMD, json!({"output": "duplicate", "rotationAdaptation": "medium", "scale": true})).unwrap();
    assert_ne!(LayerId(r["layer"].as_u64().unwrap()), base);
}

#[test]
fn sample_all_layers_reads_the_composite() {
    let mut s = blob(8);
    // An empty layer above: sampling it alone has nothing to copy, all layers fills from below.
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 42, "y": 24, "width": 12, "height": 14})).unwrap();
    let r = s.execute(CMD, json!({"sampleAllLayers": true})).unwrap();
    assert!(r["filled"].as_u64().unwrap() > 100);
    let top = px(&s, 46, 28);
    assert!(top[3] > 0.9 && !red(&top), "{top:?}");
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = blob(8);
    for p in [
        json!({"sampling": "everywhere"}),
        json!({"sampling": 3}),
        json!({"factor": 0.0}),
        json!({"factor": "big"}),
        json!({"colorAdaptation": "max"}),
        json!({"rotationAdaptation": 9}),
        json!({"output": "elsewhere"}),
        json!({"sampling": "custom"}),
        json!({"sampling": "custom", "samplingStrokes": [{"mode": "subtract", "size": 5, "points": [[1, 1]]}]}),
        json!({"samplingStrokes": "x"}),
        json!({"samplingStrokes": [{"mode": "paint", "size": 5, "points": [[1, 1]]}]}),
        json!({"samplingStrokes": [{"size": 0, "points": [[1, 1]]}]}),
        json!({"samplingStrokes": [{"size": 5, "points": []}]}),
        json!({"samplingStrokes": [{"size": 5, "points": [[1, "a"]]}]}),
        json!({"fillOps": [{"op": "add", "points": [[0, 0], [1, 1]]}]}),
        json!({"fillOps": [{"op": "expand", "px": -1}]}),
        json!({"fillOps": [{"op": "melt"}]}),
        json!({"fillOps": [{"points": []}]}),
        json!({"fillOps": [{"op": "subtract", "points": [[0, 0], [96, 0], [96, 64], [0, 64]]}]}),
        json!({"channel": "nope"}),
        json!({"margin": "wide"}),
    ] {
        let before = s.active().unwrap().history.past_len();
        assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().history.past_len(), before, "{p}");
    }
    // Strokes and polygons far off the canvas are clipped, not allocated.
    let r = s.execute(CMD, json!({"samplingStrokes": [{"size": 5000, "points": [[1e12, -1e12], [-1e12, 1e12]]}], "fillOps": [{"op": "add", "points": [[-1e9, -1e9], [-1e9 + 5.0, -1e9], [-1e9, -1e9 + 5.0]]}]}));
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn custom_area_from_strokes_only_fills() {
    let mut s = blob(8);
    let r = s.execute(CMD, json!({"sampling": "custom", "samplingStrokes": [{"size": 20, "points": [[10, 10], [80, 10], [80, 55], [10, 55], [10, 10]]}]})).unwrap();
    assert!(r["filled"].as_u64().unwrap() > 100);
    assert!(!red(&px(&s, 46, 28)));
}
