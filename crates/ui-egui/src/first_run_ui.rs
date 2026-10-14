//! The first-run setup: on the very first launch (no preferences file yet), pick the interface
//! language, then the theme. Each choice applies to the whole app at once, so the next step is
//! already drawn in it. Finishing or skipping marks the setup done (`Preferences::setup_complete`);
//! Help › First-Run Setup… runs it again. Its state lives in the dialog's fields (`step`,
//! `language`, `theme`), so automation can drive it like a user.

use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::i18n::Lang;
use crate::state::DialogKind;
use crate::theme::{ThemeKind, Tokens};

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__setup")
}

/// 0: language, 1: theme.
pub fn step(f: &Map<String, Value>) -> u64 {
    f.get("step").and_then(Value::as_u64).unwrap_or(0).min(1)
}

/// The content width (wide enough for two language cards or four theme cards in two rows).
pub const WIDTH: f32 = 460.0;

/// Opens the setup, starting on the language the system asks for and the current theme.
pub fn open(app: &mut PrismApp) -> u64 {
    let language = Lang::from_pref(&app.session.prefs().interface.language).code();
    let mut f = Map::new();
    f.insert("__setup".into(), json!(true));
    f.insert("__label".into(), json!("Welcome to Prism"));
    f.insert("step".into(), json!(0));
    f.insert("language".into(), json!(language));
    f.insert("theme".into(), json!(app.ui.theme.id()));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// First launch only: no preferences file was found, and the setup hasn't run.
pub fn open_on_first_launch(app: &mut PrismApp, had_prefs_file: bool) {
    if !had_prefs_file && !app.session.prefs().setup_complete {
        open(app);
    }
}

/// The setup is done (finished or skipped): it doesn't open again on its own.
pub fn finish(app: &mut PrismApp) {
    if !app.session.prefs().setup_complete {
        app.session.prefs.edit(|p| p.setup_complete = true);
    }
}

/// Confirm on the theme step: done. (Next, on the language step, is handled by `dialogs.rs`.)
pub fn confirm(app: &mut PrismApp, _f: &Map<String, Value>) -> Result<Value, String> {
    finish(app);
    Ok(json!({"setup": "complete"}))
}

/// The default button's label on this step.
pub fn ok_label(f: &Map<String, Value>) -> &'static str {
    if step(f) == 0 { tl!("Next") } else { tl!("Get Started") }
}

/// The other button's label: Skip on the first step, Back on the second.
pub fn cancel_label(f: &Map<String, Value>) -> &'static str {
    if step(f) == 0 { tl!("Skip") } else { tl!("Back") }
}

/// A large selectable card named `name` (for screen readers and automation).
fn card(ui: &mut egui::Ui, t: &Tokens, size: egui::Vec2, name: &str, selected: bool, draw: impl FnOnce(&egui::Painter, egui::Rect)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, name));
    let p = ui.painter();
    let fill = if resp.hovered() { t.hover } else { t.field };
    p.rect_filled(rect, t.radius, fill);
    let stroke = if selected { egui::Stroke::new(2.0, t.accent) } else { egui::Stroke::new(1.0, t.field_border) };
    p.rect_stroke(rect, t.radius, stroke, egui::StrokeKind::Inside);
    draw(p, rect);
    resp
}

fn heading(ui: &mut egui::Ui, t: &Tokens, title: &str, sub: &str) {
    ui.label(egui::RichText::new(title).font(crate::theme::semibold(16.0)).color(t.text));
    ui.add_space(2.0);
    ui.label(egui::RichText::new(sub).color(t.text_dim));
    ui.add_space(12.0);
}

pub fn body(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.set_width(WIDTH);
    let font = egui::TextStyle::Body.resolve(ui.style());
    if step(f) == 0 {
        heading(ui, &t, tl!("Choose your language"), tl!("Prism starts in the language your system uses."));
        let current = f.get("language").and_then(Value::as_str).unwrap_or("en").to_string();
        let gap = 12.0;
        let size = egui::vec2((WIDTH - gap) / 2.0, 76.0);
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for lang in Lang::all() {
                let name = lang.name();
                let big = egui::FontId::proportional(18.0);
                let r = card(ui, &t, size, name, current == lang.code(), |p, rect| {
                    p.text(rect.center(), egui::Align2::CENTER_CENTER, name, big, t.text);
                });
                if r.clicked() {
                    picked = Some(lang.code());
                }
            }
        });
        if let Some(code) = picked {
            f.insert("language".into(), json!(code));
            // The whole app switches now: the rest of the setup is drawn in it.
            if let Err(e) = app.run("prefs.set", json!({"path": "interface.language", "value": code})) {
                f.insert("__error".into(), json!(e));
            }
        }
    } else {
        heading(ui, &t, tl!("Choose a theme"), tl!("Pick the brightness you like to work in."));
        let current = app.ui.theme;
        let gap = 12.0;
        let size = egui::vec2((WIDTH - gap) / 2.0, 104.0);
        let mut picked = None;
        for row in ThemeKind::ALL.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for kind in row {
                    let k = Tokens::for_kind(*kind);
                    let label = tl!(kind.label()).to_string();
                    let r = card(ui, &t, size, &label, current == *kind, |p, rect| {
                        // A tiny workspace in the theme's own colours: chrome bar, toolbar,
                        // canvas with a document, panel, and the accent.
                        let preview = egui::Rect::from_min_max(rect.min + egui::vec2(10.0, 10.0), egui::pos2(rect.max.x - 10.0, rect.max.y - 30.0));
                        p.rect_filled(preview, 4.0, k.canvas);
                        let bar = egui::Rect::from_min_max(preview.min, egui::pos2(preview.max.x, preview.min.y + 9.0));
                        p.rect_filled(bar, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, k.chrome);
                        let tools = egui::Rect::from_min_max(egui::pos2(preview.min.x, bar.max.y), egui::pos2(preview.min.x + 12.0, preview.max.y));
                        p.rect_filled(tools, egui::CornerRadius { sw: 4, ..Default::default() }, k.dock);
                        let panel = egui::Rect::from_min_max(egui::pos2(preview.max.x - 46.0, bar.max.y), preview.max);
                        p.rect_filled(panel, egui::CornerRadius { se: 4, ..Default::default() }, k.dock);
                        let row = egui::Rect::from_min_size(panel.min + egui::vec2(5.0, 6.0), egui::vec2(36.0, 6.0));
                        p.rect_filled(row, 2.0, k.accent);
                        let doc = egui::Rect::from_center_size(egui::pos2((tools.max.x + panel.min.x) / 2.0, (bar.max.y + preview.max.y) / 2.0), egui::vec2(40.0, 28.0));
                        p.rect_filled(doc, 1.0, egui::Color32::WHITE);
                        p.text(egui::pos2(rect.center().x, rect.max.y - 15.0), egui::Align2::CENTER_CENTER, &label, font.clone(), t.text);
                    });
                    if r.clicked() {
                        picked = Some(*kind);
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
        if let Some(kind) = picked {
            f.insert("theme".into(), json!(kind.id()));
            let ctx = ui.ctx().clone();
            app.set_theme(&ctx, kind);
        }
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("You can change these later in Preferences.")).size(11.5).color(t.text_faint));
    if let Some(e) = f.get("__error").and_then(Value::as_str) {
        ui.colored_label(t.danger, e);
    }
}

#[cfg(test)]
#[path = "first_run_ui/tests.rs"]
mod tests;
