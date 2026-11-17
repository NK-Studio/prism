use super::*;

/// 120 × 80 white canvas with black stripes and a red blob, a selection around the blob.
fn app_with_blob() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 120, "height": 80, "depth": 8, "background": "white"})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for x in (0..120).step_by(8) {
                s.fill_rect(Rect::new(x, 0, x + 3, 80), &[0.0, 0.0, 0.0, 1.0]);
            }
            s.fill_rect(Rect::new(54, 32, 64, 44), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 52, "y": 30, "width": 14, "height": 16})).unwrap();
    app
}

fn ui(app: &mut PrismApp, ctx: &egui::Context, v: Value) -> Value {
    crate::distort_ui::menu(app, ctx, CMD, &json!({"ui": v})).unwrap().unwrap()
}

fn red_at(app: &PrismApp, layer: LayerId, x: i32, y: i32) -> bool {
    let v = app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(x, y);
    v[0] > 0.9 && v[1] < 0.1 && v[3] > 0.5
}

#[test]
fn opens_previews_and_commits_one_step() {
    let ctx = egui::Context::default();
    let mut app = app_with_blob();
    let past = app.session.active().unwrap().history.past_len();
    let r = crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({})).unwrap().unwrap();
    assert_eq!(r["contentAwareFill"]["settings"]["sampling"], "auto");
    let d = ui(&mut app, &ctx, json!({"wait": true}));
    assert_eq!(d["previewReady"], true, "{d}");
    assert!(d["samplingPixels"].as_u64().unwrap() > 1000 && d["fillPixels"].as_u64().unwrap() == 14 * 16, "{d}");
    let base = app.session.active().unwrap().active_layer.unwrap();
    // Default output is a new layer (as Photoshop's workspace opens).
    let r = ui(&mut app, &ctx, json!({"commit": true}));
    assert_eq!(r["committed"], true);
    assert!(app.distort.caf.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), past + 1, "one step");
    let new_layer = st.active_layer.unwrap();
    assert_ne!(new_layer, base);
    assert!(!red_at(&app, new_layer, 58, 38));
    assert!(red_at(&app, base, 58, 38), "the original layer is untouched");
}

#[test]
fn brush_lasso_undo_and_redo_drive_the_params() {
    let ctx = egui::Context::default();
    let mut app = app_with_blob();
    crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({})).unwrap().unwrap();
    // A subtracting stroke from Auto: the panel shows Custom, the params keep auto + the stroke.
    let d = ui(&mut app, &ctx, json!({"stroke": {"points": [[10, 10], [40, 10]], "size": 12}, "wait": true}));
    assert_eq!(d["samplingShown"], "custom");
    assert_eq!(d["params"]["sampling"], "auto");
    assert_eq!(d["params"]["samplingStrokes"][0]["mode"], "subtract");
    let sampled = d["samplingPixels"].as_u64().unwrap();
    let d = ui(&mut app, &ctx, json!({"undo": true, "wait": true}));
    assert_eq!(d["strokes"], 0);
    assert!(d["samplingPixels"].as_u64().unwrap() > sampled, "undo gives the area back");
    let d = ui(&mut app, &ctx, json!({"redo": true}));
    assert_eq!(d["strokes"], 1);
    // Choosing Custom starts from an empty area; the brush adds.
    let d = ui(&mut app, &ctx, json!({"sampling": "custom", "wait": true}));
    assert_eq!((d["strokes"].as_u64(), d["brushMode"].as_str()), (Some(0), Some("add")));
    assert_eq!(d["samplingPixels"], 0);
    assert_eq!(d["previewEmpty"], true, "nothing to sample from: no fill in the Preview");
    assert!(ui_err(&mut app, &ctx, json!({"commit": true})).contains("nothing to copy from"));
    assert!(app.distort.caf.is_some(), "an error keeps the workspace open");
    let d = ui(&mut app, &ctx, json!({"stroke": {"points": [[20, 20], [100, 20], [100, 70], [20, 70], [20, 20]], "size": 16}, "wait": true}));
    assert!(d["samplingPixels"].as_u64().unwrap() > 1000);
    // Lasso and Expand change the fill area.
    let d = ui(&mut app, &ctx, json!({"lasso": {"points": [[5, 5], [15, 5], [15, 15], [5, 15]], "mode": "add"}, "expand": 2, "wait": true}));
    assert!(d["fillPixels"].as_u64().unwrap() > 14 * 16 + 100, "{d}");
    assert_eq!(d["params"]["fillOps"][1]["op"], "expand");
    ui(&mut app, &ctx, json!({"output": "current", "commit": true}));
    let layer = app.session.active().unwrap().active_layer.unwrap();
    assert!(!red_at(&app, layer, 58, 38));
}

fn ui_err(app: &mut PrismApp, ctx: &egui::Context, v: Value) -> String {
    crate::distort_ui::menu(app, ctx, CMD, &json!({"ui": v})).unwrap().unwrap_err()
}

#[test]
fn bad_control_input_is_an_error_and_cancel_records_nothing() {
    let ctx = egui::Context::default();
    let mut app = app_with_blob();
    let past = app.session.active().unwrap().history.past_len();
    crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({})).unwrap().unwrap();
    for v in [
        json!({"opacity": 300}),
        json!({"color": [1, 2]}),
        json!({"show": "all"}),
        json!({"sampling": "everywhere"}),
        json!({"colorAdaptation": 5}),
        json!({"output": "elsewhere"}),
        json!({"tool": "pen"}),
        json!({"brushMode": "paint"}),
        json!({"stroke": {"points": []}}),
        json!({"stroke": {"points": [[1, 1]], "size": 0}}),
        json!({"lasso": {"points": [[1, 1], [2, 2]]}}),
        json!({"expand": 0}),
        json!({"contract": "a"}),
    ] {
        assert!(crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({"ui": v.clone()})).unwrap().is_err(), "{v}");
    }
    ui(&mut app, &ctx, json!({"cancel": true}));
    assert!(app.distort.caf.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), past, "cancel records nothing");
}

#[test]
fn apply_keeps_the_workspace_open() {
    let ctx = egui::Context::default();
    let mut app = app_with_blob();
    crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({})).unwrap().unwrap();
    let r = ui(&mut app, &ctx, json!({"output": "current", "apply": true}));
    assert_eq!((r["applied"].as_bool(), r["open"].as_bool()), (Some(true), Some(true)), "{r}");
    let d = ui(&mut app, &ctx, json!({}));
    assert_eq!(d["settings"]["output"], "current", "settings carry over");
    ui(&mut app, &ctx, json!({"cancel": true}));
    let layer = app.session.active().unwrap().active_layer.unwrap();
    assert!(!red_at(&app, layer, 58, 38), "the applied fill stays after Cancel");
}

#[test]
fn settings_are_remembered_and_sanitized() {
    let ctx = egui::Context::default();
    let mut app = app_with_blob();
    crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({})).unwrap().unwrap();
    ui(&mut app, &ctx, json!({"colorAdaptation": "high", "mirror": true, "opacity": 30, "sampling": "rectangular"}));
    ui(&mut app, &ctx, json!({"cancel": true}));
    crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({})).unwrap().unwrap();
    let d = ui(&mut app, &ctx, json!({}));
    assert_eq!((d["settings"]["colorAdaptation"].as_str(), d["settings"]["mirror"].as_bool()), (Some("high"), Some(true)));
    assert_eq!(d["settings"]["sampling"], "auto", "each session starts from Auto");
    ui(&mut app, &ctx, json!({"cancel": true}));
    // A corrupt saved entry falls back to defaults.
    let bad = CafSettings { opacity: f32::NAN, output: "moon".into(), ..CafSettings::default() }.sanitized();
    assert_eq!((bad.opacity, bad.output.as_str()), (60.0, "new"));
}

#[test]
fn needs_a_selection() {
    let ctx = egui::Context::default();
    let mut app = app_with_blob();
    app.session.execute("select.deselect", json!({})).unwrap();
    assert!(crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({})).unwrap().is_err());
    assert!(app.distort.caf.is_none());
}

#[test]
fn draws_without_panicking() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app_with_blob();
    crate::distort_ui::menu(&mut app, &ctx, CMD, &json!({})).unwrap().unwrap();
    ui(&mut app, &ctx, json!({"wait": true}));
    for tool in ["samplingBrush", "lasso", "hand", "zoom"] {
        ui(&mut app, &ctx, json!({"tool": tool}));
        let raw = egui::RawInput { screen_rect: Some(ERect::from_min_size(Pos2::ZERO, vec2(1400.0, 860.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::caf_ui::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    assert!(app.distort.caf.is_some());
}
