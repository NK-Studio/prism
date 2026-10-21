//! Imported CLIP STUDIO brush modes that change how a stroke lands: the Erase blending mode
//! erases with the Brush tool, and a size synchronized with the view scale paints in screen
//! pixels.

use prism_engine::Session;
use serde_json::{Value, json};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 60, "height": 40})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[2, 20], [58, 20]], "size": 30, "hardness": 1.0, "color": "#000000"})).unwrap();
    s
}

fn alpha(s: &Session, x: i32, y: i32) -> f32 {
    let st = s.active().unwrap();
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    l.surface().unwrap().rgba(x, y)[3]
}

fn stroke(s: &mut Session, brush: Value, extra: Value) {
    let mut p = json!({"points": [[10, 20], [50, 20]], "brush": brush});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    s.execute("paint.stroke", p).unwrap();
}

#[test]
fn csp_erase_mode_erases_with_the_brush_tool() {
    let mut s = session();
    assert_eq!(alpha(&s, 30, 20), 1.0);
    stroke(&mut s, json!({"size": 6, "hardness": 1.0, "cspErase": true}), json!({"erase": false}));
    assert_eq!(alpha(&s, 30, 20), 0.0, "erased");
    assert_eq!(alpha(&s, 30, 30), 1.0, "outside the stroke untouched");
    // Without the flag the same stroke paints.
    let mut s = session();
    stroke(&mut s, json!({"size": 6, "hardness": 1.0, "color": [1.0, 0.0, 0.0, 1.0]}), json!({}));
    assert_eq!(alpha(&s, 30, 20), 1.0);
}

#[test]
fn csp_size_synchronized_with_the_view_scale_paints_in_screen_pixels() {
    let reach = |zoom: f64, by_view: bool| {
        let mut s = session();
        stroke(&mut s, json!({"size": 16, "hardness": 1.0, "cspErase": true, "cspSizeByView": by_view}), json!({"zoom": zoom}));
        // Rows erased below the stroke centre.
        (21..40).take_while(|y| alpha(&s, 30, *y) == 0.0).count()
    };
    assert_eq!(reach(1.0, true), reach(1.0, false));
    assert!(reach(2.0, true) < reach(2.0, false), "half the image size at 200 %");
    assert!(reach(0.5, true) > reach(0.5, false), "twice the image size at 50 %");
    // Hostile zooms are bounded, not a panic.
    for zoom in [0.0, -3.0, 1e30] {
        let mut s = session();
        stroke(&mut s, json!({"size": 16, "cspSizeByView": true}), json!({"zoom": zoom}));
    }
}
