//! Layer › New › Layer… (⇧⌘N): Photoshop's New Layer dialog in front of `layer.new.layer`.
//!
//! Name and Color on the first row, "Use Previous Layer to Create Clipping Mask" under it, then
//! Mode and Opacity, then "Fill with neutral color" for the modes that have one; OK and Cancel
//! in the dialog's own right-hand column. The fields are the command's params, so
//! `ui.dialog.set` / `ui.dialog.confirm` drive it like any dialog.
//!
//! The dialog is the menu item and its shortcut. ⌥⇧⌘N ([`NO_DIALOG`]), the Layers panel's New
//! Layer button and `engine.execute` make the layer straight away with the defaults.

use egui::{Sense, vec2};
use prism_color::BlendMode;
use prism_doc::LabelColor;
use prism_engine::new_layer::neutral;
use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::theme::Tokens;
use crate::widgets;

pub const COMMAND: &str = "layer.new.layer";
/// The shell command behind Photoshop's ⌥⇧⌘N: a new layer with the defaults, no dialog.
pub const NO_DIALOG: &str = "layer.new.layerNoDialog";
const MARK: &str = "__newLayer";
/// Set once the Name field has been given focus with its text selected.
const FOCUSED: &str = "__nameFocused";

const LABEL_W: f32 = 52.0;
const NAME_W: f32 = 220.0;
const BUTTON_W: f32 = 84.0;

/// Is this dialog the New Layer dialog?
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// The dialog's fields: Photoshop's defaults and the next free "Layer N" name.
pub fn fields(app: &PrismApp) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("New Layer"));
    let name = app.session.active().map(|st| st.doc.next_layer_name("Layer")).unwrap_or_else(|| "Layer 1".into());
    f.insert("name".into(), json!(name));
    f.insert("color".into(), json!(LabelColor::None.id()));
    f.insert("clipped".into(), json!(false));
    f.insert("mode".into(), json!(mode_key(BlendMode::Normal)));
    f.insert("opacity".into(), json!(100.0));
    f.insert("fillNeutral".into(), json!(false));
    f
}

/// Open the New Layer dialog; returns its id. Refused, with the engine's reason, when a layer
/// can't be made (no document open).
pub fn open(app: &mut PrismApp) -> Result<u64, String> {
    if let Some(Err(why)) = prism_engine::commands::find(COMMAND).map(|c| (c.enabled)(&app.session)) {
        return Err(why);
    }
    let f = fields(app);
    Ok(app.ui.open_dialog(crate::state::DialogKind::Command, f))
}

/// The `mode` field naming a blend mode (`"normal"`, `"colorBurn"`…), which the engine parses.
fn mode_key(m: BlendMode) -> String {
    let s = format!("{m:?}");
    let mut c = s.chars();
    c.next().map(|first| first.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

fn mode_of(f: &Map<String, Value>) -> BlendMode {
    f.get("mode")
        .and_then(Value::as_str)
        .and_then(prism_engine::commands::blend_from_str)
        .filter(|m| *m != BlendMode::PassThrough)
        .unwrap_or(BlendMode::Normal)
}

fn flag(f: &Map<String, Value>, key: &str) -> bool {
    f.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The `layer.new.layer` params of the dialog's fields. A blank name leaves the engine's
/// "Layer N"; Fill with neutral color only counts for a mode that has one.
pub fn params(f: &Map<String, Value>) -> Value {
    let mode = mode_of(f);
    let opacity = f.get("opacity").and_then(Value::as_f64).filter(|o| o.is_finite()).unwrap_or(100.0).clamp(0.0, 100.0);
    let color = f.get("color").and_then(Value::as_str).and_then(LabelColor::from_id).unwrap_or_default();
    let mut p = json!({
        "color": color.id(),
        "clipped": flag(f, "clipped"),
        "blend": mode_key(mode),
        "opacity": opacity / 100.0,
        "fillNeutral": flag(f, "fillNeutral") && neutral(mode).is_some(),
    });
    if let Some(name) = f.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) {
        p["name"] = json!(name);
    }
    p
}

/// OK: make the layer.
pub fn confirm(app: &mut PrismApp, f: &Map<String, Value>) -> Result<Value, String> {
    app.run(COMMAND, params(f))
}

/// A right-aligned row label, `width` wide.
fn label(ui: &mut egui::Ui, text: &str, width: f32) {
    let t = Tokens::get(ui.ctx());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, t.text_dim);
    let (r, resp) = ui.allocate_exact_size(vec2(width.max(galley.size().x), 24.0), Sense::hover());
    // Painted, so named here for screen readers (and tests).
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    ui.painter().galley(egui::pos2(r.right() - galley.size().x, r.center().y - galley.size().y / 2.0), galley, t.text_dim);
}

/// The colour label's swatch: its colour, or Photoshop's crossed box for No Color.
fn label_swatch(ui: &mut egui::Ui, color: LabelColor) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
    let p = ui.painter();
    if let Some((swatch, _)) = t.layer_label_colors(color) {
        p.rect_filled(rect, t.radius_sm, swatch);
    } else {
        let stroke = egui::Stroke::new(1.0, t.text_dim);
        p.rect_stroke(rect, t.radius_sm, stroke, egui::StrokeKind::Inside);
        let r = rect.shrink(4.0);
        p.line_segment([r.left_top(), r.right_bottom()], stroke);
        p.line_segment([r.right_top(), r.left_bottom()], stroke);
    }
}

/// The checkbox text of Fill with neutral color for `mode`, in the UI language.
fn neutral_label(mode: BlendMode) -> String {
    match neutral(mode) {
        Some(n) => crate::i18n::fmt(tl!("Fill with {mode}-neutral color ({color})"), &[("mode", tl!(mode.label())), ("color", tl!(n.label()))]),
        None => tl!("Fill with neutral color").to_string(),
    }
}

/// The dialog body with its OK / Cancel column. Returns `Some(true)` for OK and `Some(false)`
/// for Cancel.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let mut outcome = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| form(ui, f));
        ui.add_space(18.0);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, tl!("Cancel"), BUTTON_W).clicked() {
                outcome = Some(false);
            }
        });
    });
    outcome
}

fn form(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.horizontal(|ui| {
        label(ui, tl!("Name"), LABEL_W);
        let mut name = f.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let id = ui.id().with("new-layer-name");
        let r = ui.add(egui::TextEdit::singleline(&mut name).id(id).desired_width(NAME_W).char_limit(255));
        if r.changed() {
            f.insert("name".into(), json!(name));
        }
        // Photoshop opens with the name selected, so typing replaces it.
        if !flag(f, FOCUSED) && !ui.is_sizing_pass() {
            r.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), id);
            }
            f.insert(FOCUSED.into(), json!(true));
        }
        ui.add_space(10.0);
        label(ui, tl!("Color"), 0.0);
        let mut color = f.get("color").and_then(Value::as_str).and_then(LabelColor::from_id).unwrap_or_default();
        label_swatch(ui, color);
        let lang = crate::i18n::current();
        let opts: Vec<(LabelColor, &str)> = LabelColor::ALL.iter().map(|c| (*c, crate::i18n::tr_ctx(lang, "layerLabel", c.label()))).collect();
        if widgets::dropdown(ui, "new-layer-color", &mut color, &opts, 120.0) {
            f.insert("color".into(), json!(color.id()));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
        let mut on = flag(f, "clipped");
        if widgets::checkbox(ui, &mut on, "Use Previous Layer to Create Clipping Mask").changed() {
            f.insert("clipped".into(), json!(on));
        }
    });
    let mut mode = mode_of(f);
    ui.horizontal(|ui| {
        label(ui, tl!("Mode"), LABEL_W);
        let opts: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
        if widgets::dropdown(ui, "new-layer-mode", &mut mode, &opts, 150.0) {
            f.insert("mode".into(), json!(mode_key(mode)));
        }
        ui.add_space(10.0);
        label(ui, tl!("Opacity"), 0.0);
        let mut o = f.get("opacity").and_then(Value::as_f64).unwrap_or(100.0) as f32;
        if widgets::popup_value_field(ui, tl!("Opacity"), &mut o, 0.0..=100.0, "%", 70.0).changed {
            f.insert("opacity".into(), json!(o.round()));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
        let fill = neutral(mode);
        let mut on = flag(f, "fillNeutral") && fill.is_some();
        let text = neutral_label(mode);
        if ui.add_enabled_ui(fill.is_some(), |ui| widgets::checkbox(ui, &mut on, &text)).inner.changed() {
            f.insert("fillNeutral".into(), json!(on));
        }
        // The colour the layer would be filled with.
        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        let p = ui.painter();
        if let Some(n) = fill.filter(|_| on) {
            let [r, g, b, _] = n.rgba();
            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            p.rect_filled(rect, t.radius_sm, egui::Color32::from_rgb(c(r), c(g), c(b)));
        }
        p.rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.text_faint), egui::StrokeKind::Inside);
    });
}

#[cfg(test)]
#[path = "new_layer_ui_tests.rs"]
mod tests;
