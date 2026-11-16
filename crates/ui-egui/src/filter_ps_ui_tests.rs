use serde_json::{Map, Value, json};

use super::*;

fn fields(cmd: &str) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("__command".into(), json!(cmd));
    f.insert("__filter".into(), json!(true));
    f
}

#[test]
fn classic_filters_get_photoshops_dialog_and_others_keep_the_form() {
    for cmd in ["filter.noise.addNoise", "filter.noise.median", "filter.noise.dustAndScratches", "filter.blur.gaussianBlur"] {
        assert!(owns(&fields(cmd)), "{cmd}");
    }
    // A 5×5 kernel grid (Custom) and a plug-in's own spec keep the generic form.
    assert!(!owns(&fields("filter.other.custom")));
    let mut plugin = fields("filter.noise.addNoise");
    plugin.insert("__spec".into(), json!("{\"amount\":0..1=0}"));
    assert!(!owns(&plugin));
    assert!(!owns(&fields("image.adjustments.levels")));
}

#[test]
fn preview_view_survives_hostile_fields() {
    let canvas = Rect::new(0, 0, 100, 60);
    let mut f = fields("filter.noise.median");
    f.insert("__zoom".into(), json!(1e9));
    f.insert("__centerX".into(), json!(f64::MAX));
    let (zoom, c) = view(&f, canvas, None);
    assert_eq!(zoom, ZOOMS[ZOOMS.len() - 1]);
    assert!(c[0].is_finite() || c[0] == f32::INFINITY);
    f.insert("__zoom".into(), json!("big"));
    f.remove("__centerX");
    assert_eq!(view(&f, canvas, None), (1.0, [50.0, 30.0]));
    // Centred on the selection when there is one.
    assert_eq!(view(&f, canvas, Some(Rect::new(10, 10, 20, 30))).1, [15.0, 20.0]);
}

#[test]
fn the_preview_renders_the_filtered_layer() {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 40, "fill": "white"})).unwrap();
    let vis = Rect::new(0, 0, 64, 40);
    let plain = render(&app, "filter.noise.median", &json!({"radius": 1}), vis, 1.0).unwrap();
    assert_eq!(plain.size, [64, 40]);
    assert!(plain.pixels.iter().all(|p| *p == Color32::WHITE));
    let noisy = render(&app, "filter.noise.addNoise", &json!({"amount": 50, "distribution": "uniform"}), vis, 0.5).unwrap();
    assert_eq!(noisy.size, [32, 20]);
    assert!(noisy.pixels.iter().any(|p| *p != Color32::WHITE));
}

#[test]
fn smart_sharpen_angle_follows_motion_blur() {
    let mut f = fields("filter.sharpen.smartSharpen");
    f.insert("remove".into(), json!("gaussianBlur"));
    assert!(!enabled(&f, "filter.sharpen.smartSharpen", "angle"));
    f.insert("remove".into(), json!("motionBlur"));
    assert!(enabled(&f, "filter.sharpen.smartSharpen", "angle"));
    assert!(enabled(&f, "filter.sharpen.smartSharpen", "amount"));
}
