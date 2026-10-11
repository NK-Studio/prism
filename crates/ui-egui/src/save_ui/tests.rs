use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Map, Value, json};

use super::*;
use crate::Services;

type Written = Rc<RefCell<Vec<(String, crate::ExportSettings)>>>;

/// An app with one 8×8 document and an exporter that records what it was asked for.
fn app() -> (PrismApp, Written) {
    let written: Written = Rc::default();
    let (w, e) = (written.clone(), written.clone());
    let services = Services {
        export: Some(Box::new(move |_doc: &prism_doc::Document, path: &str, s: &crate::ExportSettings| {
            e.borrow_mut().push((path.to_string(), s.clone()));
            Ok((b"out".to_vec(), Vec::new()))
        })),
        write: Some(Box::new(move |_p: &str, _b: &[u8]| {
            let _ = &w;
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PrismApp::new(prism_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app.sync_views();
    (app, written)
}

fn save_dialog(app: &PrismApp) -> (u64, Map<String, Value>) {
    let d = app.ui.dialogs.iter().find(|d| owns(&d.fields)).expect("the Save window is open");
    (d.id, d.fields.clone())
}

fn set(app: &mut PrismApp, id: u64, fields: Value) {
    let d = app.ui.dialog_mut(id).unwrap();
    for (k, v) in fields.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
}

/// Exports recorded by the fake exporter, minus the size-estimate proxies.
fn saves(w: &Written) -> Vec<(String, crate::ExportSettings)> {
    w.borrow().iter().filter(|(p, _)| !p.starts_with("estimate.")).cloned().collect()
}

#[test]
fn save_starts_on_the_original_and_binds_the_document_to_it() {
    let (mut app, written) = app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    let (id, f) = save_dialog(&app);
    assert_eq!((s(&f, "format", ""), s(&f, "__save", "")), ("psd", "save"));
    set(&mut app, id, json!({"dir": "/work"}));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(saves(&written).last().unwrap().0, "/work/Untitled.psd");
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/work/Untitled.psd"));
    assert_eq!(st.saved_revision, st.revision);
}

#[test]
fn an_image_is_written_beside_the_document_without_rebinding_it() {
    let (mut app, written) = app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    let (id, _) = save_dialog(&app);
    set(&mut app, id, json!({"dir": "/work", "name": "logo", "format": "jpg", "quality": "web"}));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let (path, settings) = saves(&written).last().cloned().unwrap();
    assert_eq!(path, "/work/logo.jpg");
    assert_eq!((settings.jpeg_quality, settings.jpeg_chroma_444), (Some(75), false));
    let st = app.session.active().unwrap();
    assert!(st.path.is_none(), "the next ⌘S still asks for the original");
    assert_ne!(st.saved_revision, st.revision, "still unsaved");
    // Remembered for next time: the folder, the format and its options.
    assert_eq!(app.ui.save.recent_dirs.first().map(String::as_str), Some("/work"));
    crate::menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    let (_, f) = save_dialog(&app);
    assert_eq!((s(&f, "format", ""), s(&f, "quality", ""), s(&f, "dir", "")), ("jpg", "web", "/work"));
}

#[test]
fn named_quality_choices_and_direct_settings_map_to_the_encoder() {
    let mut f = option_defaults();
    f.insert("format".into(), json!("jpg"));
    for (choice, jpeg, full) in [("best", 95, true), ("high", 85, false), ("web", 75, false), ("small", 60, false)] {
        f.insert("quality".into(), json!(choice));
        let s = settings(&f);
        assert_eq!((s.jpeg_quality, s.jpeg_chroma_444, s.webp_quality), (Some(jpeg), full, None), "{choice}");
    }
    f.insert("quality".into(), json!("custom"));
    f.insert("jpegQuality".into(), json!(400));
    assert_eq!(settings(&f).jpeg_quality, Some(100), "clamped");
    f.insert("format".into(), json!("webp"));
    f.insert("quality".into(), json!("high"));
    let s = settings(&f);
    assert_eq!((s.webp_quality, s.webp_lossless, s.jpeg_quality), (Some(82), false, None));
    f.insert("format".into(), json!("png"));
    for (effort, want) in [("default", None), ("best", Some(prism_codecs::PngCompression::Best)), ("fast", Some(prism_codecs::PngCompression::Fast))] {
        f.insert("pngEffort".into(), json!(effort));
        assert_eq!(settings(&f).png_compression, want);
    }
    f.insert("icc".into(), json!(false));
    assert!(!settings(&f).embed_icc);
}

#[test]
fn a_bad_name_keeps_the_window_open_with_the_reason() {
    let (mut app, written) = app();
    crate::save_ui::open(&mut app, false).unwrap();
    for (name, format) in [("  ", "psd"), ("a/b", "psd"), ("ok", "heic")] {
        let (id, _) = save_dialog(&app);
        set(&mut app, id, json!({"name": name, "format": format, "dir": "/work"}));
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "{name} {format}");
        let (_, f) = save_dialog(&app);
        assert!(f.get("__error").and_then(Value::as_str).is_some_and(|e| !e.is_empty()));
    }
    assert!(saves(&written).is_empty());
}

#[test]
fn a_typed_extension_picks_its_format() {
    assert_eq!(key_for_extension("JPEG"), Some("jpg"));
    assert_eq!(key_for_extension("tiff"), Some("tif"));
    assert_eq!(key_for_extension("psb"), Some("psb"));
    assert_eq!(key_for_extension("heic"), None, "read-only formats aren't offered");
    assert_eq!(key_for_extension("txt"), None);
}

#[test]
fn the_warning_says_what_the_save_loses() {
    let (mut app, _) = app();
    crate::save_ui::open(&mut app, true).unwrap();
    let (id, mut f) = save_dialog(&app);
    f.insert("dir".into(), json!("/nonexistent-prism-dir"));
    assert!(warning(&app, &f).is_some_and(|w| w.contains('2')), "two layers merge into one PNG");
    f.insert("format".into(), json!("psd"));
    assert_eq!(warning(&app, &f), None, "an original loses nothing");
    f.insert("format".into(), json!("tif"));
    assert_eq!(warning(&app, &f), None, "a layered TIFF loses nothing");
    f.insert("tiffLayers".into(), json!(false));
    assert!(warning(&app, &f).is_some());
    f.insert("perLayer".into(), json!(true));
    assert!(warning(&app, &f).is_some_and(|w| w.contains('2')), "two visible layers become two files");
    f.insert("layers".into(), json!("selected"));
    assert!(warning(&app, &f).is_some_and(|w| w.contains('1')));
    let _ = id;
}

#[test]
fn each_layer_is_saved_into_a_new_folder() {
    let dir = std::env::temp_dir().join(format!("prism-save-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut app, _) = app();
    crate::save_ui::open(&mut app, false).unwrap();
    let (id, _) = save_dialog(&app);
    set(&mut app, id, json!({"dir": dir.to_string_lossy(), "name": "parts", "format": "png", "perLayer": true}));
    let r = crate::dialogs::confirm(&mut app, id).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files.iter().all(|f| f.contains("parts") && std::path::Path::new(f).is_file()), "{files:?}");
    assert!(files[0].ends_with("Ink.png"), "named after the layer: {files:?}");
    assert!(app.session.active().unwrap().path.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_window_draws_every_format_and_option_without_panicking() {
    let (mut app, _) = app();
    crate::save_ui::open(&mut app, false).unwrap();
    app.ui.save.advanced = true;
    let ctx = egui::Context::default();
    crate::PrismApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    for m in FORMATS {
        for per_layer in [false, true] {
            let (id, _) = save_dialog(&app);
            set(&mut app, id, json!({"format": m.key, "perLayer": per_layer, "dir": "/nonexistent-prism-dir"}));
            for _ in 0..2 {
                let mut out = ctx.run_ui(Default::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
                out.textures_delta.clear();
            }
        }
    }
}

#[test]
fn the_format_list_searches_and_enter_takes_the_first_match() {
    use egui_kittest::kittest::Queryable;
    let (app, _) = app();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    crate::PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    crate::save_ui::open(h.state_mut(), false).unwrap();
    h.run_steps(3);
    // The second dropdown (after Location) is the format list.
    h.get_all_by_role(egui::accesskit::Role::ComboBox).nth(1).expect("the format dropdown").click();
    h.run_steps(3);
    // The popup opens with the search field focused and the rarer formats folded away.
    let focused = |h: &egui_kittest::Harness<'_, PrismApp>| h.get_all_by_role(egui::accesskit::Role::TextInput).last().is_some_and(|n| n.is_focused());
    assert!(focused(&h), "the search field has focus as the list opens");
    assert!(h.query_by_label("OpenEXR   32-bit HDR").is_none(), "folded until searched for");
    let more = FORMATS.iter().filter(|m| m.group == Group::Other && writable(m.key)).count();
    h.get_by_label(&format!("Show {more} more formats")).click();
    h.run_steps(3);
    assert!(h.query_by_label("OpenEXR   32-bit HDR").is_some());
    let search = h.get_all_by_role(egui::accesskit::Role::TextInput).last().expect("the search field");
    assert!(search.is_focused(), "still typing in the search field after unfolding");
    search.type_text("jp");
    h.run_steps(2);
    assert!(h.query_by_label("PNG   Lossless, keeps transparency").is_none(), "filtered");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let (_, f) = save_dialog(h.state());
    assert_eq!(s(&f, "format", ""), "jpg", "picked, and the window is still open: Enter didn't also save");
}

#[test]
fn saving_each_layer_offers_only_image_formats() {
    assert!(offered("psd", false) && !offered("psd", true) && !offered("prism", true) && offered("png", true) && offered("tif", true));
    let (app, _) = app();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    crate::PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    crate::save_ui::open(h.state_mut(), false).unwrap();
    h.run_steps(3);
    // Ticking the box while PSD is chosen moves to an image format.
    use egui_kittest::kittest::Queryable;
    h.get_by_label("Save each layer as a file").click();
    h.run_steps(3);
    let (id, f) = save_dialog(h.state());
    assert_eq!((b(&f, "perLayer", false), s(&f, "format", "")), (true, "png"));
    h.get_all_by_role(egui::accesskit::Role::ComboBox).nth(1).unwrap().click();
    h.run_steps(3);
    assert!(h.query_by_label("PSD   Keeps layers").is_none(), "originals aren't offered per layer");
    assert!(h.query_by_label("PNG   Lossless, keeps transparency").is_some());
    // An original set some other way (automation) is refused, with the reason shown.
    set(h.state_mut(), id, json!({"format": "psd"}));
    assert!(crate::dialogs::confirm(h.state_mut(), id).is_err());
    assert!(save_dialog(h.state()).1.get("__error").is_some());
}
