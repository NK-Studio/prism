//! "Replace?" prompt: a save that would write over a file already on disk (⌘S writing back to
//! the document's own file, or the Save window aimed at an existing name) asks first, while
//! "Ask before overwriting" is on in Preferences › File Handling. "Don't show again" clears it.
//!
//! The prompt is an ordinary dialog in the UI state (`ui.inspect` shows it, `ui.dialog.confirm`
//! and `ui.dialog.cancel` answer it). Replace finishes the save; Cancel writes nothing and, when
//! the save came from the Save window, brings that window back with its choices.

use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::state::DialogKind;

const MARK: &str = "__overwrite";
/// Set on Save window fields the user already agreed to write over.
pub const AGREED: &str = "__overwriteOk";

/// Whether saving to `path` should ask first: the preference is on and the file exists.
pub fn wanted(app: &PrismApp, path: &str) -> bool {
    app.session.prefs().file_handling.ask_before_overwriting && std::path::Path::new(path).is_file()
}

/// Opens the prompt for `path`. `window`: the Save window's fields to save with on Replace;
/// `None` saves the active document to `path` as ⌘S does.
pub fn open(app: &mut PrismApp, path: &str, window: Option<&Map<String, Value>>) -> Result<Value, String> {
    let doc = app.active_doc_id()?;
    let mut f = Map::new();
    f.insert(MARK.into(), json!(path));
    f.insert("__label".into(), json!("Prism"));
    f.insert("doc".into(), json!(doc.0));
    f.insert("dontAsk".into(), json!(false));
    if let Some(w) = window {
        f.insert("window".into(), Value::Object(w.clone()));
    }
    let id = app.ui.open_dialog(DialogKind::Command, f);
    Ok(json!({ "dialog": id }))
}

/// Is this dialog the prompt?
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// The prompt's body: the question, with a caution mark, and "Don't show again".
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let path = f.get(MARK).and_then(Value::as_str).unwrap_or("");
    let name = crate::file_open::display_name(path);
    let text = crate::i18n::fmt(tl!("“{name}” already exists. Do you want to replace it?"), &[("name", &name)]);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (r, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
        crate::icons::paint(ui, r, "triangle-alert", 30.0, t.warning);
        ui.vertical(|ui| {
            ui.set_max_width(360.0);
            ui.add(egui::Label::new(egui::RichText::new(text).color(t.text).size(13.0)).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new(egui::RichText::new(tl!("Its contents will be overwritten by this save.")).color(t.text_dim).size(12.0)).wrap());
        });
    });
    ui.add_space(10.0);
    let mut dont_ask = f.get("dontAsk").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut dont_ask, tl!("Don't show again")).changed() {
        f.insert("dontAsk".into(), json!(dont_ask));
    }
}

/// Replace: finish the save the prompt was holding.
pub fn confirm(app: &mut PrismApp, f: &Map<String, Value>) -> Result<Value, String> {
    let path = f.get(MARK).and_then(Value::as_str).ok_or("not an overwrite prompt")?.to_string();
    if f.get("dontAsk").and_then(Value::as_bool) == Some(true) {
        app.session.edit_prefs(|prefs| prefs.file_handling.ask_before_overwriting = false);
    }
    let doc = f.get("doc").and_then(Value::as_u64).ok_or("the prompt has no document")?;
    app.refocus(prism_doc::DocId(doc))?;
    match f.get("window").and_then(Value::as_object) {
        Some(w) => {
            let mut w = w.clone();
            w.insert(AGREED.into(), json!(true));
            crate::save_ui::confirm(app, &w)
        }
        None => app.save_as(Some(path)),
    }
}

/// Cancel: nothing is written; a save from the Save window returns to it.
pub fn cancel(app: &mut PrismApp, f: &Map<String, Value>) {
    if let Some(w) = f.get("window").and_then(Value::as_object) {
        app.ui.open_dialog(DialogKind::Command, w.clone());
    }
    app.ui.status = tl!("Save cancelled").into();
    app.ui.status_error = false;
}

#[cfg(test)]
#[path = "overwrite_prompt_tests.rs"]
mod tests;
