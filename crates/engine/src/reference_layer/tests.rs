use serde_json::json;

use super::*;

/// A 200×100 document: a reference layer with a vertical line at x = 100..103, and a paint
/// layer above it (active).
fn session() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "background": "transparent"})).unwrap();
    let lines = s.active().unwrap().active_layer.unwrap().0;
    s.execute("paint.stroke", json!({"points": [[101.5, -5], [101.5, 105]], "brush": {"size": 3, "hardness": 1.0, "pressureSize": false, "smoothing": {"amount": 0}}}))
        .unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let paint = s.active().unwrap().active_layer.unwrap().0;
    (s, lines, paint)
}

fn alpha(s: &Session, layer: u64, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.layer(LayerId(layer)).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn brush(refer: bool) -> Value {
    json!({"size": 40, "hardness": 1.0, "pressureSize": false, "spacing": 0.1, "smoothing": {"amount": 0}, "cspReferLayer": refer})
}

#[test]
fn set_reference_toggles_is_undoable_and_rejects_bad_params() {
    let (mut s, lines, _) = session();
    let flag = |s: &Session| s.active().unwrap().doc.layer(LayerId(lines)).unwrap().reference;
    assert!(!flag(&s));
    assert_eq!(s.execute("layer.setReference", json!({"layer": lines})).unwrap(), json!({"layer": lines, "reference": true}));
    assert!(flag(&s));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(!flag(&s));
    s.execute("layer.setReference", json!({"layer": lines, "on": true})).unwrap();
    s.execute("layer.setReference", json!({"layer": lines, "on": true})).unwrap();
    assert!(flag(&s));
    s.execute("layer.setReference", json!({"layer": lines, "on": false})).unwrap();
    assert!(!flag(&s));
    for p in [json!({"layer": "x"}), json!({"layer": 987_654_321u64}), json!({"layer": -1}), json!({"on": 3}), json!({"layer": lines, "on": "yes"})] {
        assert!(s.execute("layer.setReference", p.clone()).is_err(), "{p}");
    }
    assert!(Session::new().execute("layer.setReference", json!({})).is_err());
}

#[test]
fn a_referring_brush_stops_at_the_reference_line() {
    // Painted left of the line with a wide brush: a plain brush crosses it.
    let stroke = |refer: bool, reference: bool| {
        let (mut s, lines, paint) = session();
        if reference {
            s.execute("layer.setReference", json!({"layer": lines})).unwrap();
        }
        s.execute("paint.stroke", json!({"points": [[60, 50], [90, 50]], "brush": brush(refer), "seed": 1})).unwrap();
        (alpha(&s, paint, 80, 50), alpha(&s, paint, 100, 50), alpha(&s, paint, 106, 50))
    };
    assert_eq!(stroke(false, true), (1.0, 1.0, 1.0), "a brush without the option ignores the reference layer");
    assert_eq!(stroke(true, false), (1.0, 1.0, 1.0), "without a reference layer the option does nothing");
    let (inside, on_line, beyond) = stroke(true, true);
    assert_eq!((inside, beyond), (1.0, 0.0));
    assert_eq!(on_line, 1.0, "the paint meets the line");
}

#[test]
fn crossing_the_line_paints_both_sides_and_the_selection_still_applies() {
    let (mut s, lines, paint) = session();
    s.execute("layer.setReference", json!({"layer": lines})).unwrap();
    s.execute("paint.stroke", json!({"points": [[60, 50], [140, 50]], "brush": brush(true), "seed": 1})).unwrap();
    assert!(alpha(&s, paint, 80, 50) == 1.0 && alpha(&s, paint, 130, 50) == 1.0);
    // A selection on the left half still clips.
    let (mut s, lines, paint) = session();
    s.execute("layer.setReference", json!({"layer": lines})).unwrap();
    s.execute("select.lasso", json!({"points": [[0, 0], [70, 0], [70, 100], [0, 100]], "antiAlias": false})).unwrap();
    s.execute("paint.stroke", json!({"points": [[60, 50], [90, 50]], "brush": brush(true), "seed": 1})).unwrap();
    assert!(alpha(&s, paint, 60, 50) == 1.0 && alpha(&s, paint, 80, 50) == 0.0);
    // A hidden reference layer, or the layer being painted, does not constrain.
    let (mut s, lines, paint) = session();
    s.execute("layer.setReference", json!({"layer": lines})).unwrap();
    s.edit("Hide", |doc, _| {
        doc.layer_mut(LayerId(lines)).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    s.execute("paint.stroke", json!({"points": [[60, 50], [90, 50]], "brush": brush(true), "seed": 1})).unwrap();
    assert_eq!(alpha(&s, paint, 106, 50), 1.0);
}

#[test]
fn hostile_strokes_and_documents_never_panic() {
    let (mut s, lines, _) = session();
    s.execute("layer.setReference", json!({"layer": lines})).unwrap();
    for pts in [
        json!([[1e300, 50], [-1e300, 50]]),
        json!([[60, 50]]),
        json!([[-500, -500], [900, 900]]),
        json!([[101, 50], [101, 60]]),
    ] {
        let _ = s.execute("paint.stroke", json!({"points": pts, "brush": brush(true)}));
    }
    for size in [json!(0.5), json!(5000), json!(1e30)] {
        let mut b = brush(true);
        b["size"] = size;
        let _ = s.execute("paint.stroke", json!({"points": [[60, 50], [90, 50]], "brush": b}));
    }
    let doc = &s.active().unwrap().doc;
    let b = BrushSettings { csp_refer_layer: true, size: f32::NAN, ..Default::default() };
    let nan = [StrokePoint::new(f64::NAN, f64::INFINITY, 1.0)];
    assert!(constrain(doc, None, &b, &nan, None).is_none());
    assert!(constrain(doc, None, &b, &[], None).is_none());
}
