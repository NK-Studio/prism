//! Preset files opened like documents (File › Open, drag-and-drop, the Brushes panel's
//! Import Brushes…, the Preset Manager's Load…): Photoshop brushes (`.abr`) go to the brush
//! library through `brush.presets.importAbr`, gradients (`.grd`) to the Gradients panel through
//! `gradient.presets.importGrd`, swatches (`.aco`, `.ase`) to the Swatches panel through
//! `swatches.import`, and Photoshop/CLIP STUDIO actions (`.atn`/`.laf`) to the Actions panel,
//! instead of opening as documents.

use serde_json::{Value, json};

use crate::PrismApp;

/// Extensions handled here rather than by the document importer (`.aco`/`.ase` swatches go to
/// the Swatches panel through [`crate::swatches_ui::import_bytes`]).
pub const PRESET_EXTS: &[&str] = &["abr", "sut", "tool", "laf", "atn", "grd", "aco", "ase"];

fn ext(name: &str) -> String {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Is `name` a preset file this module imports?
pub fn is_preset_file(name: &str) -> bool {
    PRESET_EXTS.contains(&ext(name).as_str())
}

/// Command params for the file: a real path keeps the journal small; browser files arrive as bytes.
fn source(name: &str, bytes: &[u8]) -> Value {
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": prism_engine::paint::tile::b64_encode(bytes)})
}

/// Import a preset file; `None` when `name` isn't one.
pub fn open(app: &mut PrismApp, name: &str, bytes: &[u8]) -> Option<Result<(), String>> {
    let (cmd, what) = match ext(name).as_str() {
        "sut" | "tool" => ("brush.presets.importSut", "brushes"),
        "laf" => ("actions.importLaf", "actions"),
        "atn" => ("actions.importAtn", "actions"),
        "abr" => ("brush.presets.importAbr", "brushes"),
        "grd" => ("gradient.presets.importGrd", "gradients"),
        "aco" | "ase" => return Some(crate::swatches_ui::import_bytes(app, name, bytes, false).map(|_| ())),
        _ => return None,
    };
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!("Imported").into());
    let mut p = source(name, bytes);
    if ext(name) != "tool" {
        p["group"] = json!(stem);
    }
    if what == "brushes" {
        p["select"] = json!(true);
    }
    Some(app.run(cmd, p).map(|r| {
        // A background import (#210): `jobs_ui` reports the count when it lands.
        if r.get("pending").and_then(Value::as_bool) == Some(true) {
            app.ui.status_error = false;
            app.ui.status = format!("Importing {what} from {stem}…");
            return;
        }
        let n = r.get("count").and_then(Value::as_u64).unwrap_or(0);
        let warnings: Vec<String> = r.get("warnings").and_then(|w| serde_json::from_value(w.clone()).ok()).unwrap_or_default();
        app.ui.status_error = false;
        app.ui.status = match warnings.first() {
            Some(w) => format!("Imported {n} {what} from {stem} ({} notes: {w})", warnings.len()),
            None => format!("Imported {n} {what} from {stem}"),
        };
        if what == "brushes" {
            // Show them: Brush Settings › Brushes tab.
            app.ui.panels.brush_settings = true;
            app.ui.brush_tab = 1;
        }
        if what == "actions" {
            let pro = crate::prefs_ui::layout(app) == crate::theme::Layout::Compact;
            crate::dock_model::toggle_panel(&mut app.ui, crate::dock_model::PanelId::Actions, pro, Some(true));
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_preset_files() {
        assert!(is_preset_file("/a/b/Set.ABR"));
        assert!(is_preset_file("세모네모펜.SUT"));
        assert!(is_preset_file("material_0.TOOL"));
        assert!(is_preset_file("material_0.LAF"));
        assert!(is_preset_file("Retouch.ATN"));
        assert!(is_preset_file("sunsets.grd"));
        assert!(is_preset_file("Brand.ACO"));
        assert!(is_preset_file("exchange.ase"));
        assert!(!is_preset_file("photo.psd"));
        assert!(!is_preset_file("abr"));
    }

    /// Self-authored version-16 action fixture: one enabled Invert command, no descriptor.
    fn invert_atn() -> Vec<u8> {
        fn unicode(out: &mut Vec<u8>, value: &str) {
            let units: Vec<_> = value.encode_utf16().collect();
            out.extend_from_slice(&(units.len() as u32).to_be_bytes());
            for unit in units {
                out.extend_from_slice(&unit.to_be_bytes());
            }
        }
        let mut out = 16_u32.to_be_bytes().to_vec();
        unicode(&mut out, "Synthetic Photoshop Set");
        out.push(1);
        out.extend_from_slice(&1_u32.to_be_bytes());
        out.extend_from_slice(&[0; 6]); // Shortcut index, modifiers and color.
        unicode(&mut out, "Synthetic Invert");
        out.push(1);
        out.extend_from_slice(&1_u32.to_be_bytes());
        out.extend_from_slice(&[0, 1, 0, 0]); // Expanded, enabled, dialog, dialog options.
        out.extend_from_slice(b"longInvr");
        out.extend_from_slice(&6_u32.to_be_bytes());
        out.extend_from_slice(b"Invert");
        out.extend_from_slice(&0_u32.to_be_bytes());
        out
    }

    #[test]
    fn opening_atn_imports_actions_without_opening_a_document_or_running_steps() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.open_bytes("Synthetic.ATN", &invert_atn()).unwrap();
        assert!(app.session.active().is_none(), "import does not create a document");
        assert_eq!(app.session.actions.list.len(), 1);
        let action = &app.session.actions.list[0];
        assert_eq!(action.name, "Synthetic Invert");
        assert_eq!(action.set, "Synthetic Photoshop Set");
        assert_eq!(action.steps.len(), 1);
        assert_eq!(action.steps[0].source.as_ref().unwrap().format, "atn");
        assert!(action.steps[0].enabled);
        assert!(!app.ui.status_error);
        let pro = crate::prefs_ui::layout(&app) == crate::theme::Layout::Compact;
        assert!(crate::dock_model::panel_showing(&app.ui, crate::dock_model::PanelId::Actions, pro));
    }
}
