use crate::{PhotocraftApp, state::DialogKind};
use serde_json::{Value, json};
pub fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    let fields = json!({"__command":"image.duplicate","__label":"Duplicate Image","__duplicate":true,
        "__sourceName":st.doc.name,"name":crate::i18n::fmt(tl!("{name} copy"), &[("name", &st.doc.name)]),
        "mergedOnly":false,"__mergeEnabled":st.doc.layer_count()>1})
    .as_object()
    .cloned()
    .unwrap_or_default();
    let id = app.ui.open_dialog(DialogKind::Command, fields);
    crate::native_dialogs::make_native(app, id);
    Ok(id)
}
pub fn body(ui: &mut egui::Ui, f: &mut serde_json::Map<String, Value>) {
    ui.horizontal(|ui| {
        ui.label(tl!("Duplicate"));
        ui.label(f.get("__sourceName").and_then(Value::as_str).unwrap_or(""));
    });
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.label(tl!("As:"));
        let mut name = f.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        ui.add(egui::TextEdit::singleline(&mut name).desired_width((ui.available_width() - 20.0).max(100.0)).char_limit(255));
        f.insert("name".into(), json!(name));
    });
    ui.add_space(12.0);
    ui.add_enabled_ui(f.get("__mergeEnabled").and_then(Value::as_bool).unwrap_or(false), |ui| {
        let mut merged = f.get("mergedOnly").and_then(Value::as_bool).unwrap_or(false);
        if crate::widgets::checkbox(ui, &mut merged, tl!("Duplicate Merged Layers Only")).changed() {
            f.insert("mergedOnly".into(), json!(merged));
        }
    });
}

pub fn finish(app: &mut PhotocraftApp, source: &crate::state::Dialog, mut result: Value) -> Value {
    let Some(document) = app.session.active_index() else { return result };
    app.sync_views();
    if let Some(w) = source.window.and_then(|id| app.ui.windows.iter().find(|w| w.id == id)).cloned() {
        let id = app.ui.alloc_id();
        let position = w.position.map(|p| [p[0] + 24.0, p[1] + 24.0]);
        app.ui.windows.push(crate::state::DocWindow {
            id,
            document,
            view: w.view,
            size: w.size,
            position,
            kind: crate::state::DocWindowKind::Detached,
            open: true,
        });
        app.ui.active_window = Some(id);
        if let Some(o) = result.as_object_mut() {
            o.insert("window".into(), json!(id));
        }
    } else {
        app.ui.active_window = None;
        crate::doc_windows::activate_main(app, document);
    }
    result
}
