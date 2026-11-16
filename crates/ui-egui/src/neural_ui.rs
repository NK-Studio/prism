//! Filter › Neural Filters…: the Super Zoom window (Real-ESRGAN, `prism_engine::neural_cmds`).
//!
//! A native command window like Duplicate: pick the model (photo or illustration), the factor and
//! where the result goes; OK runs `filter.neuralFilters` as a background job. "This document"
//! enlarges it in place as one undo step (⌘Z / Ctrl+Z); "New document" opens the result beside it.
//! Until Real-ESRGAN is installed, the window says the first run downloads it.

use prism_engine::neural_cmds::{self, COMMAND, Model, SCALES};
use serde_json::{Map, Value, json};

use crate::{PrismApp, state::DialogKind};

/// The window's fields carry this key.
pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__neural")
}

/// Opens the window for the active document (the area is the selection's bounds, if any).
pub fn open(app: &mut PrismApp) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let area = st.doc.selection.as_ref().map(|s| s.content_bounds().intersect(&canvas)).filter(|r| !r.is_empty()).unwrap_or(canvas);
    let installed: Map<String, Value> = Model::ALL.iter().map(|m| (m.id().to_string(), json!(neural_cmds::installed(*m)))).collect();
    let fields = json!({
        "__command": COMMAND,
        "__label": "Neural Filters…",
        "__neural": true,
        "__width": canvas.width(),
        "__height": canvas.height(),
        "__selWidth": area.width(),
        "__selHeight": area.height(),
        "__installed": installed,
        "__canInstall": neural_cmds::can_install(),
        "filter": "superZoom",
        "model": Model::AnimeVideoV3.id(),
        "scale": 4,
        "output": "document",
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    let id = app.ui.open_dialog(DialogKind::Command, fields);
    crate::native_dialogs::make_native(app, id);
    Ok(id)
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.heading(tl!("Super Zoom"));
    ui.label(tl!("Enlarges the image with Real-ESRGAN."));
    ui.add_space(14.0);
    egui::Grid::new("neural-super-zoom").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
        ui.label(tl!("Model:"));
        let mut model = f.get("model").and_then(Value::as_str).and_then(Model::parse).unwrap_or(Model::AnimeVideoV3);
        let options = [
            (Model::AnimeVideoV3, tl!("Fast (Real-ESRGAN animevideov3)")),
            (Model::X4Plus, tl!("Photo, best quality (Real-ESRGAN x4plus)")),
            (Model::X4PlusAnime, tl!("Illustration (Real-ESRGAN x4plus anime)")),
        ];
        if crate::widgets::dropdown(ui, "neural-model", &mut model, &options, 300.0) {
            f.insert("model".into(), json!(model.id()));
        }
        ui.end_row();
        ui.label(tl!("Scale:"));
        let mut scale = f.get("scale").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|n| SCALES.contains(n)).unwrap_or(4);
        let labels: Vec<(u32, String)> = SCALES.iter().map(|s| (*s, format!("{s}×"))).collect();
        let options: Vec<(u32, &str)> = labels.iter().map(|(s, l)| (*s, l.as_str())).collect();
        if crate::widgets::dropdown(ui, "neural-scale", &mut scale, &options, 300.0) {
            f.insert("scale".into(), json!(scale));
        }
        ui.end_row();
        ui.label(tl!("Output:"));
        let mut output = f.get("output").and_then(Value::as_str).unwrap_or("document").to_string();
        let outputs = [("document".to_string(), tl!("This document (undoable)")), ("newDocument".to_string(), tl!("New document"))];
        if crate::widgets::dropdown(ui, "neural-output", &mut output, &outputs, 300.0) {
            f.insert("output".into(), json!(output));
        }
        ui.end_row();
        // In place the whole canvas grows; a new document takes the selection's bounds.
        let (wk, hk) = if output == "newDocument" { ("__selWidth", "__selHeight") } else { ("__width", "__height") };
        let (w, h) = (f.get(wk).and_then(Value::as_u64).unwrap_or(0), f.get(hk).and_then(Value::as_u64).unwrap_or(0));
        ui.label(tl!("Size:"));
        ui.label(format!("{w} × {h} px → {} × {} px", w.saturating_mul(u64::from(scale)), h.saturating_mul(u64::from(scale))));
        ui.end_row();
        let installed = f.get("__installed").and_then(|v| v.get(model.id())).and_then(Value::as_bool).unwrap_or(false);
        if !installed {
            ui.label("");
            let can = f.get("__canInstall").and_then(Value::as_bool).unwrap_or(false);
            let note = if can {
                tl!("The first run downloads Real-ESRGAN (about 50 MB) from GitHub.")
            } else {
                tl!("Real-ESRGAN is not installed.")
            };
            ui.add(egui::Label::new(egui::RichText::new(note).weak()).wrap());
            ui.end_row();
        }
    });
}

/// The Super Zoom job finished. In place: refit the grown document. A new document: show it in
/// the main window, fitted (the main window shows `main_document`, not just the session's active
/// document). Either way the status bar says what it made.
pub fn finished(app: &mut PrismApp, v: &Value) {
    app.sync_views();
    let size = format!("{} × {} px", v.get("width").and_then(Value::as_u64).unwrap_or(0), v.get("height").and_then(Value::as_u64).unwrap_or(0));
    if let Some(index) = v.get("document").and_then(Value::as_u64).map(|i| i as usize) {
        crate::doc_windows::activate_main(app, index);
        app.update_view(index, |view| view.fit_pending = true);
        app.ui.status = crate::i18n::fmt(tl!("Super Zoom: new document, {size}"), &[("size", &size)]);
    } else {
        if let Some(index) = app.session.active_index() {
            app.update_view(index, |view| view.fit_pending = true);
        }
        app.ui.status = crate::i18n::fmt(tl!("Super Zoom: enlarged to {size} (Undo to go back)"), &[("size", &size)]);
    }
    app.ui.status_error = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A job's new document must replace what the main window shows: it used to land in a
    /// background tab while the canvas kept showing the source.
    #[test]
    fn finished_job_shows_its_document_in_the_main_window() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 12, "height": 10})).unwrap();
        app.sync_views();
        crate::doc_windows::activate_main(&mut app, 0);
        let source = app.session.documents()[0].doc.id;
        assert_eq!(app.ui.main_document, Some(source));
        // Stand-in for the job's result: a second document, as the engine adds it.
        app.session.execute("image.duplicate", json!({"name": "zoomed"})).unwrap();
        finished(&mut app, &json!({"document": 1, "width": 48, "height": 40}));
        let shown = app.ui.main_document;
        assert_eq!(shown, Some(app.session.documents()[1].doc.id));
        assert_ne!(shown, Some(source));
        assert!(app.ui.status.contains("48 × 40"));
    }

    #[test]
    fn finished_in_place_keeps_the_document_and_mentions_undo() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 12, "height": 10})).unwrap();
        app.sync_views();
        crate::doc_windows::activate_main(&mut app, 0);
        let source = app.session.documents()[0].doc.id;
        finished(&mut app, &json!({"inPlace": true, "width": 48, "height": 40}));
        assert_eq!(app.ui.main_document, Some(source));
        assert!(app.ui.status.contains("48 × 40") && app.ui.status.contains("Undo"), "{}", app.ui.status);
    }

    #[test]
    fn window_fields_are_complete() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        assert!(open(&mut app).is_err());
        app.session.execute("file.new", json!({"width": 12, "height": 10})).unwrap();
        app.sync_views();
        let id = open(&mut app).unwrap();
        let d = app.ui.dialogs.iter().find(|d| d.id == id).unwrap();
        assert!(owns(&d.fields));
        assert_eq!(d.fields["__width"], 12);
        let params = crate::filter_dialog::params_of(&d.fields);
        assert_eq!(params, json!({"filter": "superZoom", "model": "realesr-animevideov3", "scale": 4, "output": "document"}));
    }
}
