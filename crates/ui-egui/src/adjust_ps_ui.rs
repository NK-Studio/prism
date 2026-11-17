//! Photoshop's own layout for Image › Adjustments dialogs that it draws as a classic panel
//! (Photoshop 27.4, compared side by side): the title centred over a hairline, the options on the
//! left, OK / Cancel in a right-hand column (Alt turns Cancel into Reset), and the last row with a
//! checkbox under the options and Preview under the buttons.
//!
//! Brightness/Contrast: "Brightness" and "Contrast", each a label with its number field at the
//! right and a slider under it, then "Use Legacy" at the bottom left. Its contrast slider has 0 in
//! the middle: the left half runs to -50 (-100 with Use Legacy), the right half to 100. Auto (under
//! Cancel) fills in the values `prism_compose::bright_contrast::auto` chooses for the layer.
//!
//! Levels: "Preset:" with Default / Custom, then a frame headed by "Channel:" holding Input Levels
//! (histogram, black / gamma / white handles and fields) and Output Levels; the right-hand column
//! has OK, Cancel (Alt: Reset), Auto, the Set Black / Gray / White Point eyedroppers and Preview.
//!
//! Curves: "Preset:", then the channel and its graph (Output at the left, the gradient bars, the
//! black / white input sliders, Input, the eyedroppers) beside Photoshop's Curve Display Options
//! (Show Amount of: Light / Pigment/Ink %, Grid size, Show: Channel Overlays, Histogram,
//! Baseline, Intersection Line); OK, Cancel (Alt: Reset), Auto and Preview at the right.
//! Option/Alt+2…5 pick the composite, red, green and blue channels.
//!
//! The fields are the command's params plus `__preview`, so `ui.dialog.set` / `ui.dialog.confirm`
//! drive it like any adjustment dialog; the canvas preview is `adjust_preview`'s.

use egui::vec2;
use serde_json::{Map, Value, json};

use crate::widgets;

/// Left column: the options.
pub(crate) const OPTIONS_W: f32 = 232.0;
/// Right column: the buttons.
pub(crate) const BUTTON_W: f32 = 112.0;
pub(crate) const GAP: f32 = 16.0;
pub(crate) const FIELD_W: f32 = 56.0;
/// The dialog's content width.
pub const WIDTH: f32 = OPTIONS_W + GAP + BUTTON_W;

/// Levels' options column (its histogram is 256 levels wide plus the handles' margins).
const LEVELS_W: f32 = 290.0;
/// Levels' button column.
const LEVELS_BUTTON_W: f32 = 100.0;

/// Hue/Saturation's options column (seven range swatches wide).
const HS_W: f32 = 276.0;

/// Color Balance's options column (the three tone swatches with their names).
const CB_W: f32 = 300.0;

/// Black & White's options column.
const BW_W: f32 = 250.0;

/// Curves' channel frame (Output gutter, bar and 256-level graph).
const CURVES_W: f32 = 340.0;
/// Curves' display options column.
const CURVES_VIEW_W: f32 = 150.0;

fn kind(f: &Map<String, Value>) -> &str {
    if let Some(k) = f.get("__adjust").and_then(Value::as_str) {
        return k;
    }
    // Adjustments that run as commands with a parameter dialog (not adjustment layers' editors).
    match f.get("__command").and_then(Value::as_str) {
        Some("image.adjustments.selectiveColor") => "selectiveColor",
        Some("image.adjustments.colorLookup") => "colorLookup",
        Some("image.adjustments.shadowsHighlights") => "shadowsHighlights",
        Some("image.adjustments.hdrToning") => "hdrToning",
        Some("image.adjustments.matchColor") => "matchColor",
        Some("image.adjustments.replaceColor") => "replaceColor",
        _ => "",
    }
}

/// Is this dialog drawn in Photoshop's classic adjustment layout?
pub fn owns(f: &Map<String, Value>) -> bool {
    matches!(kind(f), "brightnessContrast" | "levels" | "curves" | "exposure" | "vibrance" | "hueSaturation" | "colorBalance" | "blackWhite" | "photoFilter" | "channelMixer")
        || crate::adjust_ps_more::owns(kind(f))
}

/// The dialog's content width.
pub fn width(f: &Map<String, Value>) -> f32 {
    match kind(f) {
        "levels" => LEVELS_W + GAP + LEVELS_BUTTON_W,
        "hueSaturation" => HS_W + GAP + BUTTON_W,
        "colorBalance" => CB_W + GAP + BUTTON_W,
        "blackWhite" => BW_W + GAP + BUTTON_W,
        k if crate::adjust_ps_more::owns(k) => crate::adjust_ps_more::width(k),
        "curves" => CURVES_W + GAP + CURVES_VIEW_W + GAP + LEVELS_BUTTON_W,
        _ => WIDTH,
    }
}

/// A label in this dialog, with Photoshop's own wording where it differs from Prism's elsewhere.
pub(crate) fn label(s: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "adjust", s)
}

pub(crate) fn num(f: &Map<String, Value>, key: &str) -> f32 {
    f.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0) as f32
}

/// The contrast slider's position in -1..=1 (0 in the middle) for a value whose low end is `lo`.
pub fn contrast_pos(c: f32, lo: f32) -> f32 {
    if c < 0.0 { (c / -lo).clamp(-1.0, 0.0) } else { (c / 100.0).clamp(0.0, 1.0) }
}

/// The contrast value at slider position `p` (see [`contrast_pos`]), in whole steps.
pub fn contrast_at(p: f32, lo: f32) -> f32 {
    let p = if p.is_finite() { p.clamp(-1.0, 1.0) } else { 0.0 };
    (if p < 0.0 { -p * lo } else { p * 100.0 }).round()
}

/// One option: the label with its number field at the right, then the slider. The slider works on
/// its own scale `prange`: `to_pos` maps the value onto it and `from_pos` back.
fn row(
    ui: &mut egui::Ui,
    text: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    prange: std::ops::RangeInclusive<f32>,
    to_pos: impl Fn(f32) -> f32,
    from_pos: impl Fn(f32) -> f32,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(text);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let before = *value;
            widgets::value_field(ui, value, range.clone(), "", FIELD_W);
            *value = value.round().clamp(*range.start(), *range.end());
            changed |= *value != before;
        });
    });
    let mut p = to_pos(*value);
    if widgets::slider(ui, &mut p, prange, None).changed() {
        let v = from_pos(p).clamp(*range.start(), *range.end());
        changed |= v != *value;
        *value = v;
    }
    changed
}

/// The dialog body with its button column. `Some(true)` for OK, `Some(false)` for Cancel.
pub fn body(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    undo_keys(ui, f);
    let outcome = match kind(f) {
        "levels" => levels(app, ui, f),
        "curves" => curves(app, ui, f),
        "exposure" => exposure(app, ui, f),
        "vibrance" => vibrance(ui, f),
        "hueSaturation" => hue_saturation(app, ui, f),
        "colorBalance" => color_balance(ui, f),
        "blackWhite" => black_white(app, ui, f),
        "photoFilter" => photo_filter(ui, f),
        "channelMixer" => channel_mixer(app, ui, f),
        k if crate::adjust_ps_more::owns(k) => {
            let k = k.to_string();
            crate::adjust_ps_more::body(&k, app, ui, f)
        }
        _ => brightness_contrast(app, ui, f),
    };
    settle(ui, f);
    outcome
}

/// The dialog's own undo history (Cmd+Z / Shift+Cmd+Z while it is open): the settings before each
/// finished edit in `__undo`, the undone ones in `__redo`, the last settled settings in
/// `__settled`. An edit settles once no pointer button is down, so a whole drag is one step.
const UNDO: &str = "__undo";
const REDO: &str = "__redo";
const SETTLED: &str = "__settled";
/// Steps kept (older ones are dropped).
const UNDO_LIMIT: usize = 100;

fn stack(f: &mut Map<String, Value>, key: &str) -> Vec<Value> {
    f.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn restore(f: &mut Map<String, Value>, values: Value) {
    if let Value::Object(v) = values.clone() {
        f.retain(|k, _| k.starts_with("__"));
        f.extend(v);
        f.insert(SETTLED.into(), values);
        crate::adjust_dialog::rebase_picker(f);
    }
}

/// Records a finished edit as one undo step.
fn settle(ui: &egui::Ui, f: &mut Map<String, Value>) {
    let now = crate::filter_dialog::params_of(f);
    let Some(before) = f.get(SETTLED).cloned() else {
        f.insert(SETTLED.into(), now);
        return;
    };
    if before == now || ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let mut undo = stack(f, UNDO);
    undo.push(before);
    if undo.len() > UNDO_LIMIT {
        undo.remove(0);
    }
    f.insert(UNDO.into(), Value::Array(undo));
    f.insert(REDO.into(), json!([]));
    f.insert(SETTLED.into(), now);
}

/// Cmd+Z steps back through the dialog's edits, Shift+Cmd+Z forward again (a focused text field
/// keeps its own Cmd+Z).
fn undo_keys(ui: &egui::Ui, f: &mut Map<String, Value>) {
    if ui.ctx().text_edit_focused() {
        return;
    }
    let redo = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z));
    let undo = !redo && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
    if !(undo || redo) {
        return;
    }
    // An edit still in progress settles first, so it is the step undone.
    settle(ui, f);
    let (from, to) = if undo { (UNDO, REDO) } else { (REDO, UNDO) };
    let mut source = stack(f, from);
    let Some(values) = source.pop() else { return };
    let current = crate::filter_dialog::params_of(f);
    let mut target = stack(f, to);
    target.push(current);
    f.insert(from.into(), Value::Array(source));
    f.insert(to.into(), Value::Array(target));
    restore(f, values);
}

/// Alt+P (Option+P) turns the preview on and off; the checkbox is labelled with it.
pub(crate) fn preview_checkbox(ui: &mut egui::Ui, f: &mut Map<String, Value>, shortcut_in_label: bool) {
    let mut preview = f.get("__preview").and_then(Value::as_bool).unwrap_or(true);
    if ui.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::P)) {
        preview = !preview;
    }
    let text = if shortcut_in_label {
        let shortcut = if ui.ctx().os() == egui::os::OperatingSystem::Mac { "Opt+P" } else { "Alt+P" };
        format!("{}({shortcut})", tl!("Preview"))
    } else {
        tl!("Preview").to_string()
    };
    widgets::checkbox(ui, &mut preview, &text);
    f.insert("__preview".into(), json!(preview));
}

/// Image › Adjustments › Levels in Photoshop's layout.
fn levels(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    use crate::adjust_editors::{self, EditorCx};
    let mut outcome = None;
    let mode = app.session.active().map_or(prism_doc::ColorMode::Rgb, |s| s.doc.mode);
    let defaults = prism_engine::adjust_params::default_for("levels", mode).map(|d| prism_engine::adjust_params::to_params(&d)).unwrap_or(Value::Null);
    let mut values = crate::filter_dialog::params_of(f);
    let hist = app
        .session
        .active()
        .and_then(|s| s.active_layer)
        .map(|id| crate::tone::histograms(app, crate::tone::HistSource::Layer(id), adjust_editors::space_of(&values)));
    let cx = EditorCx {
        mem: crate::adjust_dialog::editor_mem("levels"),
        hist,
        gray: app.session.active().is_some_and(|s| adjust_editors::is_gray(s.doc.mode)),
        swatches: adjust_editors::swatches(app),
        gradients: Vec::new(),
        dialog: true,
    };
    let mut changed = false;
    let alt = ui.input(|i| i.modifiers.alt);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(LEVELS_W);
            // Preset: Default, or Custom once anything differs from it.
            ui.horizontal(|ui| {
                ui.label(format!("{}:", label("Preset")));
                if crate::adjust_presets::dropdown(ui, "pslv-preset", "levels", mode, &mut values, &defaults, ui.available_width()) {
                    changed = true;
                }
            });
            ui.add_space(8.0);
            let ch = adjust_editors::levels_channel(ui, &values, &cx);
            let t = crate::theme::Tokens::get(ui.ctx());
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let e = adjust_editors::levels_ranges(ui, &mut values, &cx, ch, Some(tl!("Input Levels:")));
                changed |= e.changed;
            });
            f.insert("__levelsChannel".into(), json!(ch));
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(LEVELS_BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), LEVELS_BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, LEVELS_BUTTON_W).clicked() {
                if alt {
                    values = defaults.clone();
                    changed = true;
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(8.0);
            let ch = f.get("__levelsChannel").and_then(Value::as_u64).unwrap_or(0) as usize;
            if widgets::secondary_button(ui, tl!("Auto"), LEVELS_BUTTON_W).clicked() && adjust_editors::levels_auto(&mut values, &cx, ch) {
                changed = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                crate::adjust_dialog::picker_buttons(ui, f, mode);
            });
            ui.add_space(6.0);
            preview_checkbox(ui, f, false);
        });
    });
    if changed && let Value::Object(v) = values {
        f.retain(|k, _| k.starts_with("__"));
        f.extend(v);
        crate::adjust_dialog::rebase_picker(f);
    }
    outcome
}

/// Photoshop 27.4's Exposure fields: up to `decimals` places, trailing zeros dropped, and a "+"
/// before values of 1 and more ("+20", "+11.86", "+1", but "0.5", "0.42", "-0.5"; measured).
pub(crate) fn signed(v: f64, decimals: i32) -> String {
    let k = 10f64.powi(decimals);
    let r = (v * k).round() / k;
    let r = if r == 0.0 { 0.0 } else { r };
    let text = format!("{r:.*}", decimals as usize);
    let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text };
    if r >= 1.0 { format!("+{text}") } else { text }
}

fn exposure_text(v: f64) -> String {
    signed(v, 2)
}

fn offset_text(v: f64) -> String {
    signed(v, 4)
}

/// Exposure's gamma slider (measured): 9.99 at the left end, 1 in the middle, 0.01 at the right,
/// each half linear.
pub fn gamma_pos(g: f32) -> f32 {
    if g >= 1.0 { -((g - 1.0) / 8.99).clamp(0.0, 1.0) } else { ((1.0 - g) / 0.99).clamp(0.0, 1.0) }
}

/// The gamma at slider position `p` in -1..=1 (see [`gamma_pos`]), to two decimals.
pub fn gamma_at(p: f32) -> f32 {
    let p = if p.is_finite() { p.clamp(-1.0, 1.0) } else { 0.0 };
    let g = if p < 0.0 { 1.0 - p * 8.99 } else { 1.0 - p * 0.99 };
    (g * 100.0).round() / 100.0
}

/// An Exposure option: the label with its field at the right (written by `format`), then the
/// slider on its own scale `prange` (`to_pos` / `from_pos`), values kept to `step`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fine_row(
    ui: &mut egui::Ui,
    text: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    format: fn(f64) -> String,
    to_pos: impl Fn(f32) -> f32,
    from_pos: impl Fn(f32) -> f32,
) -> bool {
    let mut changed = false;
    let snap = |v: f32| ((v / step).round() * step).clamp(*range.start(), *range.end());
    ui.horizontal(|ui| {
        ui.label(text);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let before = *value;
            let look = widgets::FieldLook { format: Some(format), ..widgets::FieldLook::compact() };
            widgets::value_field_look(ui, value, range.clone(), "", FIELD_W + 4.0, &look);
            *value = snap(*value);
            changed |= *value != before;
        });
    });
    let mut p = to_pos(*value);
    let (lo, hi) = (to_pos(*range.start()).min(to_pos(*range.end())), to_pos(*range.start()).max(to_pos(*range.end())));
    if widgets::slider(ui, &mut p, lo..=hi, None).changed() {
        let v = snap(from_pos(p));
        changed |= v != *value;
        *value = v;
    }
    changed
}

/// Image › Adjustments › Exposure in Photoshop 27.4's layout: "Preset", then Exposure, Offset and
/// Gamma Correction (each a label with its field and a slider); OK, Cancel (Alt: Reset), the
/// Set Black / Gray / White Point eyedroppers and Preview (Opt+P) at the right.
fn exposure(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let mut outcome = None;
    let mode = app.session.active().map_or(prism_doc::ColorMode::Rgb, |s| s.doc.mode);
    let mut e = num(f, "exposure").clamp(-20.0, 20.0);
    let mut o = num(f, "offset").clamp(-0.5, 0.5);
    let mut g = f.get("gamma").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(1.0).clamp(0.01, 9.99) as f32;
    let alt = ui.input(|i| i.modifiers.alt);
    let mut reset = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(OPTIONS_W);
            ui.horizontal(|ui| {
                ui.label(label("Preset"));
                let defaults = json!({"exposure": 0.0, "offset": 0.0, "gamma": 1.0});
                let mut values = json!({"exposure": e, "offset": o, "gamma": g});
                if crate::adjust_presets::dropdown(ui, "psex-preset", "exposure", mode, &mut values, &defaults, ui.available_width()) {
                    let n = |k: &str, d: f32| values.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32);
                    (e, o, g) = (n("exposure", 0.0), n("offset", 0.0), n("gamma", 1.0));
                }
            });
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            fine_row(ui, tl!("Exposure"), &mut e, -20.0..=20.0, 0.01, exposure_text, |v| v / 20.0, |p| p * 20.0);
            ui.add_space(6.0);
            fine_row(ui, tl!("Offset"), &mut o, -0.5..=0.5, 0.0001, offset_text, |v| v / 0.5, |p| p * 0.5);
            ui.add_space(6.0);
            fine_row(ui, label("Gamma Correction"), &mut g, 0.01..=9.99, 0.01, exposure_text, gamma_pos, gamma_at);
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if alt {
                    reset = true;
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                crate::adjust_dialog::picker_buttons(ui, f, mode);
            });
            ui.add_space(4.0);
            preview_checkbox(ui, f, true);
        });
    });
    if reset {
        (e, o, g) = (0.0, 0.0, 1.0);
    }
    let changed = [("exposure", e), ("offset", o), ("gamma", g)].iter().any(|(k, v)| f.get(*k).and_then(Value::as_f64) != Some(f64::from(*v)));
    f.insert("exposure".into(), json!(e));
    f.insert("offset".into(), json!(o));
    f.insert("gamma".into(), json!(g));
    if changed {
        crate::adjust_dialog::rebase_picker(f);
    }
    outcome
}

/// One of Hue/Saturation's range eyedroppers (the Eyedropper, Add to Sample, Subtract from
/// Sample): the pipette with a "+" or "−" badge.
fn hs_picker_button(ui: &mut egui::Ui, badge: &str, selected: bool, enabled: bool, tooltip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(28.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let t = crate::theme::Tokens::get(ui.ctx());
    let tint = if enabled { crate::icons::button_chrome(ui, rect, selected, resp.hovered()) } else { t.text_faint };
    crate::icons::paint(ui, rect, "adjust-eyedropper-fill", 22.0, if enabled { egui::Color32::from_gray(160) } else { egui::Color32::from_gray(90) });
    crate::icons::paint(ui, rect, "adjust-eyedropper", 22.0, tint);
    if !badge.is_empty() {
        ui.painter().text(rect.right_bottom() - vec2(7.0, 6.0), egui::Align2::CENTER_CENTER, badge, egui::FontId::proportional(11.0), tint);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, tooltip));
    resp.on_hover_text(tooltip)
}

/// Image › Adjustments › Hue/Saturation in Photoshop 27.4's layout: "Preset:", the range
/// swatches (Master, Reds … Magentas), Hue / Saturation / Lightness rows, Colorize with the three
/// range eyedroppers and Invert, then "Before - After" with the range's sliders; OK, Cancel (Alt:
/// Reset) and Preview (Opt+P) at the right. (Photoshop's On-image Adjustment tool isn't here.)
fn hue_saturation(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    use crate::adjust_dialog::{HS_PICKER, HS_RANGE};
    use crate::adjust_editors::{self, EditorCx};
    let mut outcome = None;
    let mode = app.session.active().map_or(prism_doc::ColorMode::Rgb, |s| s.doc.mode);
    let defaults = prism_engine::adjust_params::default_for("hueSaturation", mode).map(|d| prism_engine::adjust_params::to_params(&d)).unwrap_or(Value::Null);
    let mut values = crate::filter_dialog::params_of(f);
    let cx = EditorCx {
        mem: crate::adjust_dialog::editor_mem("hueSaturation"),
        hist: None,
        gray: false,
        swatches: adjust_editors::swatches(app),
        gradients: Vec::new(),
        dialog: true,
    };
    let colorize = values.get("colorize").and_then(Value::as_bool).unwrap_or(false);
    let mut range = f.get(HS_RANGE).and_then(Value::as_u64).filter(|r| *r <= 6).unwrap_or(0) as usize;
    if colorize {
        range = 0;
    }
    let mut changed = false;
    let alt = ui.input(|i| i.modifiers.alt);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(HS_W);
            ui.horizontal(|ui| {
                ui.label(format!("{}:", label("Preset")));
                if crate::adjust_presets::dropdown(ui, "pshs-preset", "hueSaturation", mode, &mut values, &defaults, ui.available_width()) {
                    changed = true;
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if let Some(r) = adjust_editors::hs_swatches(ui, &values, range, colorize) {
                    range = r;
                    f.remove(HS_PICKER);
                }
            });
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            changed |= adjust_editors::hs_rows(ui, &mut values, range, colorize).changed;
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let mut c = colorize;
                if widgets::checkbox(ui, &mut c, tl!("Colorize")).changed() {
                    adjust_editors::hs_set_colorize(&mut values, c, cx.swatches[0]);
                    f.remove(HS_PICKER);
                    changed = true;
                }
                ui.add_space(6.0);
                let enabled = range > 0 && !c && mode == prism_doc::ColorMode::Rgb;
                let armed = f.get(HS_PICKER).and_then(Value::as_str).map(str::to_owned);
                for (id, badge, tip) in [("sample", "", tl!("Eyedropper Tool")), ("add", "+", tl!("Add to Sample")), ("subtract", "−", tl!("Subtract from Sample"))] {
                    if hs_picker_button(ui, badge, armed.as_deref() == Some(id), enabled, tip).clicked() {
                        f.insert(HS_PICKER.into(), json!(id));
                    }
                }
                ui.add_space(6.0);
                let inv = ui.add_enabled_ui(enabled, |ui| widgets::secondary_button(ui, label("Invert"), 64.0)).inner;
                if inv.clicked() {
                    let b = adjust_editors::hs_bounds(&values, range);
                    adjust_editors::hs_set_bounds(&mut values, range, prism_engine::adjust_params::invert_hue_range(b));
                    changed = true;
                }
            });
            ui.add_space(8.0);
            changed |= adjust_editors::hs_spectrum(ui, &mut values, &cx, range, colorize).changed;
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if alt {
                    values = defaults.clone();
                    changed = true;
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(4.0);
            preview_checkbox(ui, f, true);
        });
    });
    if range == 0 {
        f.remove(HS_PICKER);
    }
    f.insert(HS_RANGE.into(), json!(range));
    if changed && let Value::Object(v) = values {
        f.retain(|k, _| k.starts_with("__"));
        f.extend(v);
    }
    outcome
}

/// Image › Adjustments › Color Balance in Photoshop 27.4's layout (measured): "Tone" with round
/// Shadows / Midtones / Highlights swatches (the chosen one ringed), "Cyan to Red", "Magenta to
/// Green" and "Yellow to Blue" rows (fields written "+58", sliders on colour tracks; a
/// double-click zeroes one, the wheel steps it, as measured in Photoshop 25.4), Preserve
/// Luminosity; OK, Cancel (Alt: Reset, which zeroes every tone and shows Midtones) and Preview.
fn color_balance(ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let mut outcome = None;
    let mem = crate::adjust_dialog::editor_mem("colorBalance");
    let tone_id = crate::adjust_editors::color_balance_tone_id(mem);
    let mut tone: usize = ui.data(|d| d.get_temp(tone_id)).unwrap_or(1).min(2);
    let key = ["shadows", "midtones", "highlights"][tone];
    let read = |k: &str| -> [f32; 3] {
        let a = f.get(k).and_then(Value::as_array);
        std::array::from_fn(|i| a.and_then(|a| a.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(0.0, |v| (v as f32).clamp(-100.0, 100.0)))
    };
    let mut vals = read(key);
    let before = (vals, f.get(key).cloned());
    let mut preserve = f.get("preserveLuminosity").and_then(Value::as_bool).unwrap_or(true);
    let preserve_before = f.get("preserveLuminosity").cloned();
    let alt = ui.input(|i| i.modifiers.alt);
    let mut reset = false;
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(CB_W);
            ui.horizontal(|ui| {
                ui.label(label("Tone"));
                ui.add_space(4.0);
                for (i, (name, grey)) in [(tl!("Shadows"), 0u8), (tl!("Midtones"), 128), (tl!("Highlights"), 255)].into_iter().enumerate() {
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let galley = ui.painter().layout_no_wrap(name.to_string(), font, t.text);
                    let (rect, resp) = ui.allocate_exact_size(vec2(36.0 + galley.size().x, 30.0), egui::Sense::click());
                    let c = egui::pos2(rect.left() + 14.0, rect.center().y);
                    ui.painter().circle_filled(c, 11.0, egui::Color32::from_gray(grey));
                    ui.painter().circle_stroke(c, 11.0, egui::Stroke::new(1.0, egui::Color32::from_black_alpha(90)));
                    if i == tone {
                        ui.painter().circle_stroke(c, 13.5, egui::Stroke::new(2.0, t.text));
                    }
                    ui.painter().galley(egui::pos2(rect.left() + 33.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, i == tone, name));
                    if resp.clicked() {
                        tone = i;
                    }
                }
            });
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            let rows = [
                ("Cyan to Red", egui::Color32::from_rgb(0, 230, 230), egui::Color32::from_rgb(235, 20, 20)),
                ("Magenta to Green", egui::Color32::from_rgb(230, 20, 230), egui::Color32::from_rgb(20, 220, 20)),
                ("Yellow to Blue", egui::Color32::from_rgb(235, 235, 20), egui::Color32::from_rgb(20, 20, 235)),
            ];
            let gestures = widgets::RowGestures { reset: Some(0.0), wheel_step: Some(1.0) };
            for (i, (text, a, b)) in rows.into_iter().enumerate() {
                let Some(x) = vals.get_mut(i) else { continue };
                ui.horizontal(|ui| {
                    ui.label(label(text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let look = widgets::FieldLook { format: Some(whole_text), ..widgets::FieldLook::compact() };
                        widgets::value_field_look(ui, x, -100.0..=100.0, "", FIELD_W, &look);
                    });
                });
                let grey = egui::Color32::from_gray(128);
                widgets::slider_with(ui, x, -100.0..=100.0, Some(&[a, a.lerp_to_gamma(grey, 0.6), b.lerp_to_gamma(grey, 0.6), b]), gestures);
                *x = x.round().clamp(-100.0, 100.0);
                ui.add_space(6.0);
            }
            ui.add_space(8.0);
            widgets::checkbox(ui, &mut preserve, tl!("Preserve Luminosity"));
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if alt {
                    reset = true;
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(4.0);
            preview_checkbox(ui, f, true);
        });
    });
    // The values belong to the tone shown when the frame began; written only when they change
    // (or weren't valid), so the dialog's undo history sees real edits only.
    if vals != before.0 || before.1.as_ref().and_then(Value::as_array).is_none_or(|a| a.len() != 3) {
        f.insert(key.into(), json!(vals));
    }
    if preserve_before.as_ref().and_then(Value::as_bool) != Some(preserve) {
        f.insert("preserveLuminosity".into(), json!(preserve));
    }
    if reset {
        let mut values = crate::filter_dialog::params_of(f);
        crate::adjust_editors::reset_color_balance(ui.ctx(), mem, &mut values);
        if let Value::Object(v) = values {
            f.extend(v);
        }
    } else {
        ui.data_mut(|d| d.insert_temp(tone_id, tone));
    }
    outcome
}

/// Image › Adjustments › Channel Mixer in Photoshop 27.4's layout (measured): "Preset" (its six
/// monochrome presets), "Output Channel" with round red / green / blue swatches (one grey swatch
/// with Monochrome), Monochrome, the Red / Green / Blue rows (fields "200", "-200"; sliders
/// -200..200 on black → colour → white tracks), "Total" as text with a warning sign above 100 %,
/// a hairline, then Constant; OK, Cancel (Alt: Reset) and Preview at the right.
fn channel_mixer(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let mut outcome = None;
    let mode = app.session.active().map_or(prism_doc::ColorMode::Rgb, |s| s.doc.mode);
    let defaults = prism_engine::adjust_params::default_for("channelMixer", mode).map(|d| prism_engine::adjust_params::to_params(&d)).unwrap_or(Value::Null);
    let mut values = crate::filter_dialog::params_of(f);
    let mut changed = false;
    let alt = ui.input(|i| i.modifiers.alt);
    let t = crate::theme::Tokens::get(ui.ctx());
    let out_id = crate::adjust_dialog::editor_mem("channelMixer").with("cm-out");
    let mut out: usize = ui.data(|d| d.get_temp(out_id)).unwrap_or(0).min(2);
    let mono = values.get("monochrome").and_then(Value::as_bool).unwrap_or(false);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(OPTIONS_W);
            ui.horizontal(|ui| {
                ui.label(label("Preset"));
                if crate::adjust_presets::dropdown(ui, "pscm-preset", "channelMixer", mode, &mut values, &defaults, ui.available_width()) {
                    changed = true;
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(label("Output Channel"));
                ui.add_space(6.0);
                let swatches: &[(egui::Color32, &str)] = if mono {
                    &[(egui::Color32::from_gray(128), "Gray")]
                } else {
                    &[(egui::Color32::from_rgb(255, 0, 0), "Red"), (egui::Color32::from_rgb(0, 255, 0), "Green"), (egui::Color32::from_rgb(0, 0, 255), "Blue")]
                };
                for (i, (c, name)) in swatches.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
                    let centre = rect.center();
                    ui.painter().circle_filled(centre, 11.0, *c);
                    ui.painter().circle_stroke(centre, 11.0, egui::Stroke::new(1.0, egui::Color32::from_black_alpha(90)));
                    let on = mono || i == out;
                    if on {
                        ui.painter().circle_stroke(centre, 13.5, egui::Stroke::new(2.0, t.text));
                    }
                    let name = tl!(name);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, name));
                    if resp.on_hover_text(name).clicked() {
                        out = i;
                    }
                }
            });
            ui.add_space(4.0);
            let mut m = mono;
            if widgets::checkbox(ui, &mut m, tl!("Monochrome")).changed() {
                values["monochrome"] = json!(m);
                if m {
                    values["gray"] = json!([40.0, 40.0, 20.0, 0.0]);
                } else if let Some(o) = values.as_object_mut() {
                    o.remove("gray");
                }
                changed = true;
            }
            ui.add_space(8.0);
            let key = if m { "gray" } else { ["red", "green", "blue"][out] };
            let identity: [f32; 4] = if m { [40.0, 40.0, 20.0, 0.0] } else { std::array::from_fn(|i| if i == out { 100.0 } else { 0.0 }) };
            let read = values.get(key).and_then(Value::as_array).cloned();
            let mut row: [f32; 4] = std::array::from_fn(|i| {
                read.as_ref().and_then(|a| a.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(identity[i], |v| v as f32).clamp(-200.0, 200.0)
            });
            let before = row;
            ui.spacing_mut().item_spacing.y = 2.0;
            let grey = egui::Color32::from_gray(128);
            let mix_row = |ui: &mut egui::Ui, text: &str, x: &mut f32, track: &[egui::Color32]| {
                ui.horizontal(|ui| {
                    ui.label(text);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::value_field(ui, x, -200.0..=200.0, "", FIELD_W);
                    });
                });
                widgets::slider(ui, x, -200.0..=200.0, Some(track));
                *x = x.round().clamp(-200.0, 200.0);
                ui.add_space(6.0);
            };
            for (i, (name, c)) in [("Red", egui::Color32::from_rgb(255, 0, 0)), ("Green", egui::Color32::from_rgb(0, 255, 0)), ("Blue", egui::Color32::from_rgb(0, 0, 255))]
                .into_iter()
                .enumerate()
            {
                if let Some(x) = row.get_mut(i) {
                    mix_row(ui, label(name), x, &[egui::Color32::BLACK, c, egui::Color32::WHITE]);
                }
            }
            let total = row[0] + row[1] + row[2];
            ui.horizontal(|ui| {
                ui.label(label("Total"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("%");
                    ui.label(format!("{total:.0}"));
                    if total > 100.0 {
                        ui.label(egui::RichText::new("⚠").color(t.text)).on_hover_text(tl!("Total exceeds 100%"));
                    }
                });
            });
            ui.add_space(4.0);
            let line = ui.available_rect_before_wrap();
            ui.painter().hline(line.x_range(), line.top(), egui::Stroke::new(1.0, t.separator));
            ui.add_space(8.0);
            mix_row(ui, label("Constant"), &mut row[3], &[egui::Color32::BLACK, grey, egui::Color32::WHITE]);
            if row != before {
                values[key] = json!(row);
                changed = true;
            }
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if alt {
                    values = defaults.clone();
                    out = 0;
                    changed = true;
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(4.0);
            preview_checkbox(ui, f, true);
        });
    });
    ui.data_mut(|d| d.insert_temp(out_id, out));
    if changed && let Value::Object(v) = values {
        f.retain(|k, _| k.starts_with("__"));
        f.extend(v);
    }
    outcome
}

/// Image › Adjustments › Photo Filter in Photoshop 27.4's layout (captured): "Filter" (a radio
/// with the dropdown of its twenty filters, named in English even in the Korean UI) or "Color" (a
/// radio with the colour swatch), "Density" ("25%" with a slider), Preserve Luminosity; OK, Cancel
/// (Alt: Reset) and Preview at the right.
fn photo_filter(ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    use prism_engine::adjust_params::{PHOTO_FILTERS, hex};
    let mut outcome = None;
    let default_colour = [236.0 / 255.0, 138.0 / 255.0, 0.0];
    let colour_of = |f: &Map<String, Value>| -> [f32; 3] {
        let h = f.get("color").and_then(Value::as_str).unwrap_or("#ec8a00").trim_start_matches('#').to_string();
        let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|x| f32::from(x) / 255.0);
        match (b(0), b(2), b(4)) {
            (Some(r), Some(g), Some(bb)) => [r, g, bb],
            _ => default_colour,
        }
    };
    let mut colour = colour_of(f);
    let preset = PHOTO_FILTERS.iter().position(|p| hex(p.2.map(|x| f32::from(x) / 255.0)) == hex(colour));
    let mode_id = crate::adjust_dialog::editor_mem("photoFilter").with("pf-mode");
    let mut use_colour: bool = ui.data(|d| d.get_temp(mode_id)).unwrap_or(preset.is_none());
    let mut density = f.get("density").and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(25.0, |v| v as f32).clamp(1.0, 100.0);
    let mut preserve = f.get("preserveLuminosity").and_then(Value::as_bool).unwrap_or(true);
    // As stored, so an out-of-range value is written back clamped.
    let before = (
        f.get("color").and_then(Value::as_str).unwrap_or_default().to_string(),
        f.get("density").and_then(Value::as_f64).map_or(f32::NAN, |v| v as f32),
        f.get("preserveLuminosity").and_then(Value::as_bool),
    );
    let alt = ui.input(|i| i.modifiers.alt);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(OPTIONS_W);
            ui.horizontal(|ui| {
                if radio(ui, !use_colour, label("Filter")) {
                    use_colour = false;
                    if preset.is_none() {
                        colour = default_colour;
                    }
                }
                ui.add_space(4.0);
                let mut sel = preset.map_or_else(|| "custom".to_string(), |i| i.to_string());
                let mut opts: Vec<(String, &str)> = PHOTO_FILTERS.iter().enumerate().map(|(i, p)| (i.to_string(), p.1)).collect();
                if preset.is_none() {
                    opts.insert(0, ("custom".into(), tl!("Custom")));
                }
                let picked = ui.add_enabled_ui(!use_colour, |ui| widgets::dropdown(ui, "pspf-filter", &mut sel, &opts, ui.available_width())).inner;
                if picked && let Some(p) = sel.parse::<usize>().ok().and_then(|i| PHOTO_FILTERS.get(i)) {
                    colour = p.2.map(|x| f32::from(x) / 255.0);
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if radio(ui, use_colour, label("Color")) {
                    use_colour = true;
                }
                ui.add_space(4.0);
                // Photoshop keeps the swatch live under Filter too: picking a colour selects Color.
                let mut srgb = colour.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
                if widgets::color_edit_button_srgb(ui, &mut srgb).changed() {
                    colour = srgb.map(|x| f32::from(x) / 255.0);
                    use_colour = true;
                }
            });
            ui.add_space(8.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.label(label("Density"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    widgets::value_field(ui, &mut density, 1.0..=100.0, "%", FIELD_W);
                });
            });
            widgets::slider(ui, &mut density, 1.0..=100.0, None);
            density = density.round().clamp(1.0, 100.0);
            ui.add_space(10.0);
            widgets::checkbox(ui, &mut preserve, tl!("Preserve Luminosity"));
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if alt {
                    (colour, density, preserve, use_colour) = (default_colour, 25.0, true, false);
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(4.0);
            preview_checkbox(ui, f, true);
        });
    });
    ui.data_mut(|d| d.insert_temp(mode_id, use_colour));
    // Written only when they change, so the dialog's undo history sees real edits only.
    if (hex(colour), density, Some(preserve)) != before {
        f.insert("color".into(), json!(hex(colour)));
        f.insert("density".into(), json!(density));
        f.insert("preserveLuminosity".into(), json!(preserve));
        f.remove("filter");
    }
    outcome
}

/// HSV of an RGB colour: hue in degrees, saturation and value in 0..=1.
fn hsv(c: [f32; 3]) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let s = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
    [prism_engine::adjust_params::hue_of(c).unwrap_or(0.0), s, mx]
}

/// RGB (as stored, 0..=1) of a hue in degrees, saturation and value (Photoshop's HSB).
fn from_hsv([h, s, v]: [f32; 3]) -> [f32; 3] {
    let (h, s, v) = (if h.is_finite() { h.rem_euclid(360.0) } else { 0.0 } / 60.0, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// Image › Adjustments › Black & White in Photoshop 27.4's layout (measured): "Preset:" with its
/// twelve presets; Reds … Magentas rows ("40%", sliders -200..300 % on black → colour → white
/// tracks); Tint with its colour swatch, then Hue (°) and Saturation (%) of the tint (greyed while
/// Tint is off); OK, Cancel (Alt: Reset) and Preview at the right. (Photoshop's Auto isn't here.)
fn black_white(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    use prism_engine::adjust_params::{BW_DEFAULTS, HUE_RANGES};
    let mut outcome = None;
    let mode = app.session.active().map_or(prism_doc::ColorMode::Rgb, |s| s.doc.mode);
    let defaults = prism_engine::adjust_params::default_for("blackWhite", mode).map(|d| prism_engine::adjust_params::to_params(&d)).unwrap_or(Value::Null);
    let mut values = crate::filter_dialog::params_of(f);
    let mut changed = false;
    let alt = ui.input(|i| i.modifiers.alt);
    let names = ["Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas"];
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(BW_W);
            ui.horizontal(|ui| {
                ui.label(format!("{}:", label("Preset")));
                if crate::adjust_presets::dropdown(ui, "psbw-preset", "blackWhite", mode, &mut values, &defaults, ui.available_width()) {
                    changed = true;
                }
            });
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, (key, name)) in HUE_RANGES.iter().zip(names).enumerate() {
                let d = BW_DEFAULTS.get(i).copied().unwrap_or(0.0);
                let mut x = values.get(*key).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(d, |v| v as f32).clamp(-200.0, 300.0);
                let before = x;
                ui.horizontal(|ui| {
                    ui.label(format!("{}:", label(name)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::value_field(ui, &mut x, -200.0..=300.0, "%", FIELD_W);
                    });
                });
                let colour = crate::adjust_editors::hs_range_color(i + 1);
                widgets::slider(ui, &mut x, -200.0..=300.0, Some(&[egui::Color32::BLACK, colour, egui::Color32::WHITE]));
                x = x.round().clamp(-200.0, 300.0);
                if x != before {
                    values[*key] = json!(x);
                    changed = true;
                }
                ui.add_space(6.0);
            }
            // Tint: the checkbox and its colour, then its hue and saturation.
            let mut tint = values.get("tint").and_then(Value::as_bool).unwrap_or(false);
            let hex = values.get("tintColor").and_then(Value::as_str).unwrap_or("#e1d3b3").to_string();
            let parse = |i: usize| hex.trim_start_matches('#').get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map_or(0.8, |x| f32::from(x) / 255.0);
            let mut colour = [parse(0), parse(2), parse(4)];
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if widgets::checkbox(ui, &mut tint, label("Tint")).changed() {
                    values["tint"] = json!(tint);
                    changed = true;
                }
                ui.add_space(8.0);
                let mut srgb = colour.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
                if ui.add_enabled_ui(tint, |ui| widgets::color_edit_button_srgb(ui, &mut srgb)).inner.changed() {
                    colour = srgb.map(|x| f32::from(x) / 255.0);
                    values["tintColor"] = json!(prism_engine::adjust_params::hex(colour));
                    changed = true;
                }
            });
            ui.add_space(4.0);
            // Shown in whole degrees and percent, as Photoshop shows them (42°, 20%).
            let [h, s, v] = hsv(colour);
            let (mut h, mut sat) = (h.round(), (s * 100.0).round());
            ui.add_enabled_ui(tint, |ui| {
                let tint_row = |ui: &mut egui::Ui, text: &str, x: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str, track: &[egui::Color32]| -> bool {
                    let before = *x;
                    ui.horizontal(|ui| {
                        ui.label(text);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            widgets::value_field(ui, x, range.clone(), unit, FIELD_W);
                        });
                    });
                    widgets::slider(ui, x, range.clone(), Some(track));
                    *x = x.round().clamp(*range.start(), *range.end());
                    *x != before
                };
                let dull: Vec<egui::Color32> = (0..=12).map(|k| {
                    let c = from_hsv([k as f32 * 30.0, 0.55, 0.75]).map(|x| (x * 255.0).round() as u8);
                    egui::Color32::from_rgb(c[0], c[1], c[2])
                }).collect();
                let hue_changed = tint_row(ui, label("Hue"), &mut h, 0.0..=360.0, "°", &dull);
                ui.add_space(6.0);
                let full = from_hsv([h, 1.0, v.max(0.5)]).map(|x| (x * 255.0).round() as u8);
                let sat_changed = tint_row(ui, label("Saturation"), &mut sat, 0.0..=100.0, "%", &[egui::Color32::from_gray(128), egui::Color32::from_rgb(full[0], full[1], full[2])]);
                if hue_changed || sat_changed {
                    values["tintColor"] = json!(prism_engine::adjust_params::hex(from_hsv([h, sat / 100.0, v])));
                    changed = true;
                }
            });
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if alt {
                    values = defaults.clone();
                    changed = true;
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(4.0);
            preview_checkbox(ui, f, true);
        });
    });
    if changed && let Value::Object(v) = values {
        f.retain(|k, _| k.starts_with("__"));
        f.extend(v);
    }
    outcome
}

pub(crate) fn whole_text(v: f64) -> String {
    signed(v, 0)
}

/// Image › Adjustments › Vibrance in Photoshop 27.4's layout (measured): Vibrance and Saturation,
/// each a label with its field ("+53", "-100") and a linear -100..100 slider; OK, Cancel (Alt:
/// Reset) and Preview (Opt+P) at the right.
fn vibrance(ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let mut outcome = None;
    let mut v = num(f, "vibrance").clamp(-100.0, 100.0);
    let mut s = num(f, "saturation").clamp(-100.0, 100.0);
    let alt = ui.input(|i| i.modifiers.alt);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(OPTIONS_W);
            ui.spacing_mut().item_spacing.y = 2.0;
            fine_row(ui, label("Vibrance"), &mut v, -100.0..=100.0, 1.0, whole_text, |x| x, |p| p);
            ui.add_space(6.0);
            fine_row(ui, tl!("Saturation"), &mut s, -100.0..=100.0, 1.0, whole_text, |x| x, |p| p);
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if alt {
                    (v, s) = (0.0, 0.0);
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(4.0);
            preview_checkbox(ui, f, true);
        });
    });
    f.insert("vibrance".into(), json!(v));
    f.insert("saturation".into(), json!(s));
    outcome
}

/// A titled group box (Photoshop's "Show Amount of:", "Grid size:", "Show:").
pub(crate) fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(title).color(t.text_dim));
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        add(ui);
    });
}

/// A radio button with its label.
pub(crate) fn radio(ui: &mut egui::Ui, on: bool, text: &str) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0 + galley.size().x, galley.size().y.max(16.0)), egui::Sense::click());
    let dot = egui::pos2(rect.left() + 7.0, rect.center().y);
    ui.painter().circle_stroke(dot, 6.0, egui::Stroke::new(1.0, t.text_faint));
    if on {
        ui.painter().circle_filled(dot, 3.0, t.text);
    }
    ui.painter().galley(egui::pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, text));
    resp.clicked()
}

/// A grid-size button: a square with `n` × `n` cells.
fn grid_button(ui: &mut egui::Ui, n: usize, on: bool, tooltip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(26.0), egui::Sense::click());
    let tint = crate::icons::button_chrome(ui, rect, on, resp.hovered());
    let r = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(14.0));
    let p = ui.painter();
    let s = egui::Stroke::new(1.0, tint);
    p.rect_stroke(r, 0.0, s, egui::StrokeKind::Inside);
    for i in 1..n {
        let f = i as f32 / n as f32;
        p.line_segment([egui::pos2(r.left() + f * r.width(), r.top()), egui::pos2(r.left() + f * r.width(), r.bottom())], s);
        p.line_segment([egui::pos2(r.left(), r.top() + f * r.height()), egui::pos2(r.right(), r.top() + f * r.height())], s);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, tooltip));
    resp.on_hover_text(tooltip).clicked()
}

/// Image › Adjustments › Curves in Photoshop's layout.
fn curves(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    use crate::adjust_editors::{self, CurvesView, EditorCx};
    let mut outcome = None;
    let mode = app.session.active().map_or(prism_doc::ColorMode::Rgb, |s| s.doc.mode);
    let defaults = prism_engine::adjust_params::default_for("curves", mode).map(|d| prism_engine::adjust_params::to_params(&d)).unwrap_or(Value::Null);
    let mut values = crate::filter_dialog::params_of(f);
    let space = adjust_editors::space_of(&values);
    let hist = app.session.active().and_then(|s| s.active_layer).map(|id| crate::tone::histograms(app, crate::tone::HistSource::Layer(id), space));
    let cx = EditorCx {
        mem: crate::adjust_dialog::editor_mem("curves"),
        hist,
        gray: app.session.active().is_some_and(|s| adjust_editors::is_gray(s.doc.mode)),
        swatches: adjust_editors::swatches(app),
        gradients: Vec::new(),
        dialog: true,
    };
    let view_id = cx.mem.with("view");
    let mut view: CurvesView = ui.data(|d| d.get_temp(view_id)).unwrap_or_else(|| CurvesView::default_for(space));
    let channels = adjust_editors::tone_channels(space, cx.gray).len();
    // Option/Alt+2…5: composite, then each channel.
    for (k, key) in [egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6].into_iter().enumerate() {
        if k < channels && ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, key)) {
            adjust_editors::set_curves_channel(ui.ctx(), &cx, k);
        }
    }
    let mut changed = false;
    let alt = ui.input(|i| i.modifiers.alt);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(CURVES_W + GAP + CURVES_VIEW_W);
            // Preset: Default, or Custom once anything differs from it.
            ui.horizontal(|ui| {
                ui.label(format!("{}:", label("Preset")));
                if crate::adjust_presets::dropdown(ui, "pscv-preset", "curves", mode, &mut values, &defaults, CURVES_W) {
                    changed = true;
                }
            });
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(CURVES_W);
                    let ch = adjust_editors::curves_channel(ui, &values, &cx);
                    let t = crate::theme::Tokens::get(ui.ctx());
                    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(egui::Margin::symmetric(8, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let e = adjust_editors::curves_graph(ui, &mut values, &cx, ch, &view);
                        changed |= e.changed;
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(110.0);
                            ui.spacing_mut().item_spacing.x = 2.0;
                            crate::adjust_dialog::picker_buttons(ui, f, mode);
                        });
                    });
                    f.insert("__curvesChannel".into(), json!(ch));
                });
                ui.add_space(GAP);
                ui.vertical(|ui| {
                    ui.set_width(CURVES_VIEW_W);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    group(ui, label("Show Amount of:"), |ui| {
                        if radio(ui, !view.pigment, label("Light (0-255)")) {
                            view.pigment = false;
                        }
                        if radio(ui, view.pigment, label("Pigment/Ink %")) {
                            view.pigment = true;
                        }
                    });
                    ui.add_space(6.0);
                    group(ui, label("Grid size:"), |ui| {
                        ui.horizontal(|ui| {
                            if grid_button(ui, 2, !view.fine_grid, tl!("Show Grid in 25% increments")) {
                                view.fine_grid = false;
                            }
                            if grid_button(ui, 4, view.fine_grid, tl!("Show Grid in 10% increments")) {
                                view.fine_grid = true;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    group(ui, label("Show:"), |ui| {
                        widgets::checkbox(ui, &mut view.overlays, label("Channel Overlays"));
                        widgets::checkbox(ui, &mut view.histogram, label("Histogram"));
                        widgets::checkbox(ui, &mut view.baseline, label("Baseline"));
                        widgets::checkbox(ui, &mut view.intersection, label("Intersection Line"));
                    });
                });
            });
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(LEVELS_BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), LEVELS_BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, LEVELS_BUTTON_W).clicked() {
                if alt {
                    values = defaults.clone();
                    changed = true;
                } else {
                    outcome = Some(false);
                }
            }
            ui.add_space(8.0);
            let ch = f.get("__curvesChannel").and_then(Value::as_u64).unwrap_or(0) as usize;
            if widgets::secondary_button(ui, tl!("Auto"), LEVELS_BUTTON_W).clicked() && adjust_editors::curves_auto(&mut values, &cx, ch) {
                changed = true;
            }
            ui.add_space(6.0);
            preview_checkbox(ui, f, false);
        });
    });
    ui.data_mut(|d| d.insert_temp(view_id, view));
    if changed && let Value::Object(v) = values {
        f.retain(|k, _| k.starts_with("__"));
        f.extend(v);
        crate::adjust_dialog::rebase_picker(f);
    }
    outcome
}

/// Image › Adjustments › Brightness/Contrast in Photoshop's layout.
fn brightness_contrast(app: &crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let mut outcome = None;
    let legacy = f.get("legacy").and_then(Value::as_bool).unwrap_or(false);
    let lo = if legacy { -100.0 } else { -50.0 };
    let mut b = num(f, "brightness").clamp(-150.0, 150.0);
    let mut c = num(f, "contrast").clamp(lo, 100.0);
    let mut preview = f.get("__preview").and_then(Value::as_bool).unwrap_or(true);
    let mut use_legacy = legacy;
    let alt = ui.input(|i| i.modifiers.alt);
    // Photoshop: Alt+P (Option+P) turns the preview on and off.
    if ui.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::P)) {
        preview = !preview;
    }
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(OPTIONS_W);
            ui.spacing_mut().item_spacing.y = 2.0;
            row(ui, label("Brightness"), &mut b, -150.0..=150.0, -150.0..=150.0, |v| v, f32::round);
            ui.add_space(6.0);
            row(ui, tl!("Contrast"), &mut c, lo..=100.0, -1.0..=1.0, |v| contrast_pos(v, lo), |p| contrast_at(p, lo));
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            if widgets::secondary_button(ui, if alt { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if alt {
                    (b, c, use_legacy) = (0.0, 0.0, false);
                } else {
                    outcome = Some(false);
                }
            }
            if widgets::secondary_button(ui, tl!("Auto"), BUTTON_W).clicked()
                && let Some(a) = prism_engine::adjust_cmds::brightness_contrast_auto(&app.session, use_legacy)
            {
                (b, c) = (a.brightness as f32, a.contrast as f32);
                f.insert("mean".into(), json!(a.mean));
            }
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(OPTIONS_W, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(OPTIONS_W);
            widgets::checkbox(ui, &mut use_legacy, label("Use Legacy"));
        });
        ui.add_space(GAP);
        let shortcut = if ui.ctx().os() == egui::os::OperatingSystem::Mac { "Opt+P" } else { "Alt+P" };
        widgets::checkbox(ui, &mut preview, &format!("{}({shortcut})", tl!("Preview")));
    });
    if !use_legacy {
        // Back from legacy: modern contrast stops at -50.
        c = c.max(-50.0);
    }
    f.insert("brightness".into(), json!(b));
    f.insert("contrast".into(), json!(c));
    f.insert("legacy".into(), json!(use_legacy));
    f.insert("__preview".into(), json!(preview));
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PrismApp;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn dialog() -> (Harness<'static, PrismApp>, u64) {
        dialog_for("image.adjustments.brightnessContrast")
    }

    fn dialog_for(cmd: &str) -> (Harness<'static, PrismApp>, u64) {
        let mut s = prism_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("edit.fill", json!({"color": "#806040"})).unwrap();
        // Real frame times: a double-click's two clicks must land within the double-click time.
        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), PrismApp::new(s, crate::Services::default()));
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = crate::adjust_dialog::open(h.state_mut(), cmd).unwrap();
        h.run_steps(3);
        (h, id)
    }

    fn field(h: &Harness<'static, PrismApp>, id: u64, key: &str) -> Value {
        h.state().ui.dialogs.iter().find(|d| d.id == id).and_then(|d| d.fields.get(key).cloned()).unwrap_or(Value::Null)
    }

    fn click(h: &mut Harness<'static, PrismApp>, label: &str, mods: egui::Modifiers) {
        h.event(egui::Event::ModifiersChanged(mods));
        h.run_steps(2);
        let at = h.get_by_label(label).rect().center();
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: mods });
            h.run_steps(1);
        }
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(3);
    }

    /// Photoshop 27.4: OK and Cancel in the right-hand column, Alt turns Cancel into Reset (the
    /// dialog stays open), Use Legacy widens contrast to -100 and leaving it clamps back to -50.
    #[test]
    fn brightness_contrast_dialog_works_like_photoshops() {
        let (mut h, id) = dialog();
        assert!(owns(&h.state().ui.dialogs[0].fields));
        assert!(h.query_by_label("Brightness").is_some() && h.query_by_label("Use Legacy").is_some());
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        d.fields.insert("legacy".into(), json!(true));
        d.fields.insert("contrast".into(), json!(-90));
        d.fields.insert("brightness".into(), json!(40));
        h.run_steps(2);
        assert_eq!(field(&h, id, "contrast"), json!(-90.0), "legacy keeps -90");
        click(&mut h, "Use Legacy", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "legacy"), json!(false));
        assert_eq!(field(&h, id, "contrast"), json!(-50.0), "modern stops at -50");
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert!(h.state().ui.dialogs.iter().any(|d| d.id == id), "Reset keeps the dialog open");
        assert_eq!((field(&h, id, "brightness"), field(&h, id, "contrast")), (json!(0.0), json!(0.0)));
        // OK applies once: one history step.
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("brightness".into(), json!(60));
        h.run_steps(2);
        let steps = h.state().session.active().unwrap().history.past_len();
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty(), "OK closes");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    /// Auto fills in the fitted values; a low-contrast layer gets more contrast.
    #[test]
    fn auto_fills_in_brightness_and_contrast() {
        let (mut h, id) = dialog();
        // Two close greys: a low-contrast layer.
        let s = &mut h.state_mut().session;
        s.execute("edit.fill", json!({"color": "#707070"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
        s.execute("edit.fill", json!({"color": "#909090"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        h.run_steps(2);
        click(&mut h, "Auto", egui::Modifiers::NONE);
        let c = field(&h, id, "contrast").as_f64().unwrap();
        assert!(c > 0.0, "contrast {c}");
        assert!(field(&h, id, "mean").as_u64().is_some());
    }

    /// Photoshop's Levels: Preset, Channel, Input / Output Levels, OK / Cancel (Alt: Reset), Auto,
    /// the three eyedroppers and Preview. An armed eyedropper samples the canvas into the channels.
    #[test]
    fn levels_dialog_has_photoshops_controls_and_eyedroppers() {
        let (mut h, id) = dialog_for("image.adjustments.levels");
        for label in ["Input Levels:", "Output Levels:", "OK", "Cancel", "Auto", "Set Black Point", "Set Neutral Gray Point", "Set White Point"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        // The fill is #806040: the black point takes each channel's input black from it.
        click(&mut h, "Set Black Point", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "__curvePicker"), json!("black"));
        assert!(crate::adjust_dialog::picker_armed(h.state()));
        assert!(crate::adjust_dialog::sample_at(h.state_mut(), 5.0, 5.0));
        h.run_steps(2);
        let ib = |h: &Harness<'static, PrismApp>, k: &str| field(h, id, k).get("inBlack").and_then(Value::as_f64);
        assert_eq!((ib(&h, "red"), ib(&h, "green"), ib(&h, "blue")), (Some(128.0), Some(96.0), Some(64.0)));
        // Alt+Cancel (Reset) puts the defaults back.
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert!(h.state().ui.dialogs.iter().any(|d| d.id == id));
        assert_eq!(ib(&h, "red"), Some(0.0));
        // OK applies once.
        let steps = h.state().session.active().unwrap().history.past_len();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("inWhite".into(), json!(200));
        h.run_steps(2);
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    /// Run explicitly for visual QA: the Levels dialog (handles and eyedroppers) in two themes.
    #[test]
    #[ignore = "writes target/levels-dialog/*.png for visual inspection"]
    fn write_levels_dialog_sheet() {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/levels-dialog");
        std::fs::create_dir_all(&out).unwrap();
        for kind in [crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::StudioLight] {
            let mut s = prism_engine::Session::new();
            s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
            s.execute("edit.fill", json!({"color": "#806040"})).unwrap();
            let mut h = Harness::builder()
                .with_size(egui::vec2(700.0, 520.0))
                .with_pixels_per_point(2.0)
                .wgpu()
                .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), PrismApp::new(s, crate::Services::default()));
            PrismApp::setup_context(&h.ctx, kind);
            crate::adjust_dialog::open(h.state_mut(), "image.adjustments.levels").unwrap();
            h.run_steps(3);
            click(&mut h, "Set Neutral Gray Point", egui::Modifiers::NONE);
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}.png"))).unwrap();
        }
    }

    /// Photoshop 27.4's Curves: Preset, Channel, the graph with Output / Input, the eyedroppers,
    /// the Curve Display Options and OK / Cancel (Alt: Reset) / Auto / Preview. Pigment/Ink %
    /// shows a point in ink; Option/Alt+3 shows the red channel.
    #[test]
    fn curves_dialog_has_photoshops_controls() {
        let (mut h, id) = dialog_for("image.adjustments.curves");
        assert!(owns(&h.state().ui.dialogs[0].fields));
        for label in [
            "Output:",
            "Input:",
            "Show Amount of:",
            "Light (0-255)",
            "Pigment/Ink %",
            "Grid size:",
            "Channel Overlays",
            "Histogram",
            "Baseline",
            "Intersection Line",
            "OK",
            "Cancel",
            "Auto",
            "Set Black Point",
            "Set Neutral Gray Point",
            "Set White Point",
        ] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        // A click on the graph adds a point there and selects it: its Input / Output fields show.
        let graph = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::adjust_dialog::editor_mem("curves").with("curves-graph"))).unwrap();
        let at = graph.left_bottom() + egui::vec2(graph.width() * 64.0 / 255.0, -graph.height() * 128.0 / 255.0);
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(2);
        let pts = field(&h, id, "points");
        assert_eq!(pts.as_array().map(Vec::len), Some(3), "{pts}");
        assert_eq!(pts[1], json!([64.0, 128.0]));
        // Pigment/Ink %: the same curve, both axes in ink.
        click(&mut h, "Pigment/Ink %", egui::Modifiers::NONE);
        let view = h.ctx.data(|d| d.get_temp::<crate::adjust_editors::CurvesView>(crate::adjust_dialog::editor_mem("curves").with("view"))).unwrap();
        assert!(view.pigment);
        assert_eq!(field(&h, id, "points")[1], json!([64.0, 128.0]), "the view doesn't edit the curve");
        // Alt+Cancel (Reset) puts the straight line back.
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert_eq!(field(&h, id, "points").as_array().map(Vec::len), Some(2));
        // Option/Alt+3: the red channel.
        h.event(egui::Event::Key { key: egui::Key::Num3, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT });
        h.run_steps(3);
        assert_eq!(field(&h, id, "__curvesChannel"), json!(1));
        // Auto stretches the channel to its histogram; OK applies once.
        click(&mut h, "Auto", egui::Modifiers::NONE);
        assert_ne!(field(&h, id, "red"), json!([[0.0, 0.0], [255.0, 255.0]]));
        let steps = h.state().session.active().unwrap().history.past_len();
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    fn key(h: &mut Harness<'static, PrismApp>, key: egui::Key, modifiers: egui::Modifiers) {
        h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
        h.run_steps(3);
    }

    /// Cmd+Z inside the dialog steps back through its edits (a whole drag is one step) and
    /// Shift+Cmd+Z forward again; the document is untouched until OK.
    #[test]
    fn curves_dialog_undoes_and_redoes_its_own_edits() {
        let (mut h, id) = dialog_for("image.adjustments.curves");
        let graph = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::adjust_dialog::editor_mem("curves").with("curves-graph"))).unwrap();
        let at = |x: f32, y: f32| graph.left_bottom() + egui::vec2(graph.width() * x / 255.0, -graph.height() * y / 255.0);
        // Step 1: a click adds a point at (64, 64).
        let p = at(64.0, 64.0);
        h.event(egui::Event::PointerMoved(p));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(2);
        // Step 2: one drag moves it up through several frames.
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        for k in 1..=5 {
            h.event(egui::Event::PointerMoved(at(64.0, 64.0 + 10.0 * k as f32)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: at(64.0, 114.0), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
        let pts = |h: &Harness<'static, PrismApp>| field(h, id, "points").as_array().map(|a| a.iter().map(|q| q[1].as_f64().unwrap_or(-1.0)).collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!(pts(&h), vec![0.0, 114.0, 255.0]);
        let revision = h.state().session.active().unwrap().revision;
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(pts(&h), vec![0.0, 64.0, 255.0], "the drag is one step");
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(pts(&h), vec![0.0, 255.0], "then the added point");
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(pts(&h), vec![0.0, 255.0], "nothing older");
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(pts(&h), vec![0.0, 64.0, 255.0]);
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(pts(&h), vec![0.0, 114.0, 255.0]);
        // A new edit after an undo drops the redo steps.
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        click(&mut h, "Reset", egui::Modifiers::ALT);
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(pts(&h), vec![0.0, 255.0], "Reset was the last edit");
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(pts(&h), vec![0.0, 64.0, 255.0]);
        assert_eq!(h.state().session.active().unwrap().revision, revision, "the document waits for OK");
        assert!(h.state().ui.dialogs.iter().any(|d| d.id == id));
    }

    /// The same history in Brightness/Contrast and Levels.
    #[test]
    fn other_photoshop_dialogs_undo_too() {
        let (mut h, id) = dialog();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("brightness".into(), json!(40));
        h.run_steps(3);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("brightness".into(), json!(80));
        h.run_steps(3);
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "brightness"), json!(40.0));
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "brightness"), json!(0.0));
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(field(&h, id, "brightness"), json!(40.0));
        let (mut h, id) = dialog_for("image.adjustments.levels");
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("inWhite".into(), json!(200));
        h.run_steps(3);
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "inWhite").as_f64(), Some(255.0));
    }

    /// Photoshop 27.4's Exposure (measured): fields written "+20", "+11.86", "+1", "0.5", "0.42",
    /// "-0.5", "-20"; the gamma slider runs 9.99 (left), 1 (middle), 0.01 (right), each half
    /// linear (0.42 three quarters of the way right).
    #[test]
    fn exposure_fields_and_gamma_slider_match_photoshop() {
        assert_eq!([20.0, 11.86, 1.0, 0.5, 0.42, -0.5, -20.0, 0.0, 9.99].map(exposure_text), ["+20", "+11.86", "+1", "0.5", "0.42", "-0.5", "-20", "0", "+9.99"]);
        assert_eq!(offset_text(0.1234), "0.1234");
        assert_eq!(offset_text(-0.00001), "0");
        assert_eq!((gamma_at(-1.0), gamma_at(0.0), gamma_at(1.0)), (9.99, 1.0, 0.01));
        assert_eq!(gamma_at(0.596), 0.41);
        for g in [0.01, 0.42, 1.0, 2.5, 9.99] {
            assert!((gamma_at(gamma_pos(g)) - g).abs() < 0.006, "{g}");
        }
        assert_eq!(gamma_at(f32::NAN), 1.0);
    }

    /// The Exposure dialog: Preset, the three rows, OK / Cancel (Alt: Reset), the eyedroppers
    /// (white sets the exposure from the canvas) and Preview (Opt+P); Cmd+Z steps back.
    #[test]
    fn exposure_dialog_has_photoshops_controls() {
        let (mut h, id) = dialog_for("image.adjustments.exposure");
        assert!(owns(&h.state().ui.dialogs[0].fields));
        for label in ["Offset", "Gamma Correction", "OK", "Cancel", "Set Black Point", "Set Neutral Gray Point", "Set White Point"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        // The fill is #806040: White Point takes its luminance to white.
        click(&mut h, "Set White Point", egui::Modifiers::NONE);
        assert!(crate::adjust_dialog::picker_armed(h.state()));
        assert!(crate::adjust_dialog::sample_at(h.state_mut(), 5.0, 5.0));
        h.run_steps(3);
        let e = field(&h, id, "exposure").as_f64().unwrap();
        assert!(e > 1.0 && e < 3.0, "exposure {e}");
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "exposure").as_f64(), Some(0.0), "Cmd+Z undoes the eyedropper");
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("gamma".into(), json!(2.5));
        h.run_steps(3);
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert_eq!(field(&h, id, "gamma").as_f64(), Some(1.0));
        assert!(h.state().ui.dialogs.iter().any(|d| d.id == id));
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("offset".into(), json!(0.1));
        h.run_steps(2);
        let steps = h.state().session.active().unwrap().history.past_len();
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    #[test]
    fn exposure_dialog_survives_odd_values() {
        let (mut h, id) = dialog_for("image.adjustments.exposure");
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        d.fields.insert("exposure".into(), json!("lots"));
        d.fields.insert("offset".into(), json!(1e9));
        d.fields.insert("gamma".into(), json!(-3));
        h.run_steps(3);
        assert_eq!((field(&h, id, "exposure"), field(&h, id, "offset")), (json!(0.0), json!(0.5)));
        assert!((field(&h, id, "gamma").as_f64().unwrap() - 0.01).abs() < 1e-6);
    }

    /// Photoshop 27.4's Vibrance (measured): two linear -100..100 rows written "+53" / "-100",
    /// OK / Cancel (Alt: Reset), Preview; Cmd+Z steps back.
    #[test]
    fn vibrance_dialog_works_like_photoshops() {
        assert_eq!([100.0, 53.0, 0.0, -100.0].map(whole_text), ["+100", "+53", "0", "-100"]);
        let (mut h, id) = dialog_for("image.adjustments.vibrance");
        assert!(owns(&h.state().ui.dialogs[0].fields));
        for label in ["Saturation", "OK", "Cancel"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("vibrance".into(), json!(53));
        h.run_steps(3);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("saturation".into(), json!(-1e9));
        h.run_steps(3);
        assert_eq!(field(&h, id, "saturation"), json!(-100.0));
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!((field(&h, id, "vibrance"), field(&h, id, "saturation")), (json!(53.0), json!(0.0)));
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert_eq!(field(&h, id, "vibrance"), json!(0.0));
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("vibrance".into(), json!(40));
        h.run_steps(2);
        let steps = h.state().session.active().unwrap().history.past_len();
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    /// Photoshop 27.4's Hue/Saturation: the range swatches, Hue / Saturation / Lightness, Colorize
    /// (the foreground's hue, saturation 25), the range eyedroppers and Invert (reds 315/345/15/45
    /// → 15/45/315/345), Before - After with the range's degrees; Alt: Reset; Cmd+Z.
    #[test]
    fn hue_saturation_dialog_works_like_photoshops() {
        let (mut h, id) = dialog_for("image.adjustments.hueSaturation");
        assert!(owns(&h.state().ui.dialogs[0].fields));
        for label in ["Master", "Reds", "Magentas", "Hue:", "Saturation:", "Lightness:", "Colorize", "Invert", "Before - After", "OK", "Cancel"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        // Master: no eyedroppers, no Invert, no range degrees to edit.
        assert!(!crate::adjust_dialog::picker_armed(h.state()));
        click(&mut h, "Reds", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "__hsRange"), json!(1));
        assert!(h.query_by_label("315°/345°").is_some(), "the reds' degrees");
        click(&mut h, "Invert", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "reds")["range"], json!([15.0, 45.0, 315.0, 345.0]));
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "reds")["range"], json!([-45.0, -15.0, 15.0, 45.0]), "Cmd+Z undoes Invert");
        // Eyedropper: the fill #806040 (hue 30°) centres the reds on it.
        click(&mut h, "Eyedropper Tool", egui::Modifiers::NONE);
        assert!(crate::adjust_dialog::picker_armed(h.state()));
        assert!(crate::adjust_dialog::sample_at(h.state_mut(), 5.0, 5.0));
        h.run_steps(3);
        assert_eq!(field(&h, id, "reds")["range"], json!([-15.0, 15.0, 45.0, 75.0]));
        // Colorize: master only, the foreground's hue, saturation 25.
        click(&mut h, "Colorize", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "colorize"), json!(true));
        assert_eq!(field(&h, id, "__hsRange"), json!(0));
        assert_eq!(field(&h, id, "saturation"), json!(25.0));
        assert!(!crate::adjust_dialog::picker_armed(h.state()));
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert_eq!(field(&h, id, "colorize"), json!(false));
        let steps = h.state().session.active().unwrap().history.past_len();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("hue".into(), json!(40));
        h.run_steps(2);
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    /// Photoshop 27.4's Color Balance: Tone swatches switch the rows' tone; Cmd+Z steps back; the
    /// values carry no undo step until they change.
    #[test]
    fn color_balance_dialog_works_like_photoshops() {
        assert_eq!(whole_text(58.0), "+58");
        let (mut h, id) = dialog_for("image.adjustments.colorBalance");
        assert!(owns(&h.state().ui.dialogs[0].fields));
        for label in ["Shadows", "Midtones", "Highlights", "Cyan to Red", "Magenta to Green", "Yellow to Blue", "Preserve Luminosity", "OK", "Cancel"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("midtones".into(), json!([58, 0, 0]));
        h.run_steps(3);
        click(&mut h, "Shadows", egui::Modifiers::NONE);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("shadows".into(), json!([0, -20, 0]));
        h.run_steps(3);
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "shadows").get(1).and_then(Value::as_f64), Some(0.0));
        assert_eq!(field(&h, id, "midtones").get(0).and_then(Value::as_f64), Some(58.0), "the midtones edit stays");
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "midtones").get(0).and_then(Value::as_f64), Some(0.0));
        let steps = h.state().session.active().unwrap().history.past_len();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("highlights".into(), json!([0, 0, 30]));
        h.run_steps(2);
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    /// Photoshop 27.4's Black & White: Preset (its twelve), the six weights, Tint with its
    /// colour, hue and saturation (greyed while off); Alt: Reset; Cmd+Z.
    #[test]
    fn black_white_dialog_works_like_photoshops() {
        let (mut h, id) = dialog_for("image.adjustments.blackWhite");
        assert!(owns(&h.state().ui.dialogs[0].fields));
        for label in ["Reds:", "Yellows:", "Greens:", "Cyans:", "Blues:", "Magentas:", "Tint", "OK", "Cancel"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        click(&mut h, "Tint", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "tint"), json!(true));
        // The tint's hue and saturation edit its colour: hue 42, saturation 20 is the default.
        assert_eq!(hsv([225.0 / 255.0, 211.0 / 255.0, 179.0 / 255.0]).map(|x| (x * 100.0).round() / 100.0), [41.74, 0.2, 0.88]);
        let c = from_hsv([200.0, 0.5, 0.8]);
        assert!((hsv(c)[0] - 200.0).abs() < 0.01 && (hsv(c)[1] - 0.5).abs() < 0.001);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("reds".into(), json!(300));
        h.run_steps(3);
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "reds").as_f64(), Some(40.0));
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert_eq!(field(&h, id, "tint"), json!(false));
        // A preset's weights resolve to valid settings.
        let p = crate::adjust_presets::presets("blackWhite", prism_doc::ColorMode::Rgb);
        assert_eq!(p.len(), 12);
        let steps = h.state().session.active().unwrap().history.past_len();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("blues".into(), json!(110));
        h.run_steps(2);
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    /// Photoshop 27.4's Photo Filter: Filter (twenty, English names) or Color, Density, Preserve
    /// Luminosity; Alt: Reset; Cmd+Z.
    #[test]
    fn photo_filter_dialog_works_like_photoshops() {
        let (mut h, id) = dialog_for("image.adjustments.photoFilter");
        assert!(owns(&h.state().ui.dialogs[0].fields));
        for label in ["Filter", "Color", "Density", "Preserve Luminosity", "OK", "Cancel"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        // Color: the swatch takes over; Filter again comes back to a filter.
        click(&mut h, "Color", egui::Modifiers::NONE);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#3366cc"));
        h.run_steps(3);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("density".into(), json!(60));
        h.run_steps(3);
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "density").as_f64(), Some(25.0));
        click(&mut h, "Filter", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "color"), json!("#ec8a00"), "a custom colour gives way to Warming Filter (85)");
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("density".into(), json!(1e9));
        h.run_steps(3);
        assert_eq!(field(&h, id, "density").as_f64(), Some(100.0));
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert_eq!(field(&h, id, "density").as_f64(), Some(25.0));
        let steps = h.state().session.active().unwrap().history.past_len();
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    /// Photoshop 27.4's Channel Mixer: Output Channel swatches, Monochrome (Photoshop's grey mix
    /// 40/40/20), the rows, Total with a warning above 100 %, Constant; Alt: Reset; Cmd+Z.
    #[test]
    fn channel_mixer_dialog_works_like_photoshops() {
        let (mut h, id) = dialog_for("image.adjustments.channelMixer");
        assert!(owns(&h.state().ui.dialogs[0].fields));
        for label in ["Output Channel", "Monochrome", "Total", "Constant", "OK", "Cancel"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        // Red / Green / Blue: an output swatch and a row each.
        for label in ["Red", "Green", "Blue"] {
            assert_eq!(h.query_all_by_label(label).count(), 2, "{label}");
        }
        assert!(h.query_by_label("⚠").is_none(), "100 % is no warning");
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("red".into(), json!([200, 0, 0, 0]));
        h.run_steps(3);
        assert!(h.query_by_label("⚠").is_some(), "200 % warns");
        click(&mut h, "Monochrome", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "gray"), json!([40.0, 40.0, 20.0, 0.0]));
        key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(field(&h, id, "monochrome"), json!(false));
        click(&mut h, "Reset", egui::Modifiers::ALT);
        assert_eq!(field(&h, id, "red").get(0).and_then(Value::as_f64), Some(100.0));
        let steps = h.state().session.active().unwrap().history.past_len();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("blue".into(), json!([0, 20, 80, 0]));
        h.run_steps(2);
        click(&mut h, "OK", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    /// A double-click on a Hue/Saturation slider's knob puts it back to 0.
    #[test]
    fn hue_saturation_slider_double_click_resets_to_zero() {
        let (mut h, id) = dialog_for("image.adjustments.hueSaturation");
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        d.fields.insert("hue".into(), json!(90));
        d.fields.insert("lightness".into(), json!(-40));
        h.run_steps(3);
        // The hue slider lies between the "Hue:" and "Saturation:" rows, across the column; its
        // knob three quarters along (90 of -180..180).
        let (hue, sat) = (h.get_by_label("Hue:").rect(), h.get_by_label("Saturation:").rect());
        let at = egui::pos2(hue.left() + HS_W * 0.75, (hue.bottom() + sat.top()) / 2.0);
        h.event(egui::Event::PointerMoved(at));
        for _ in 0..2 {
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
            }
        }
        h.run_steps(2);
        assert_eq!(field(&h, id, "hue").as_f64(), Some(0.0));
        assert_eq!(field(&h, id, "lightness").as_f64(), Some(-40.0), "only that slider");
    }

    #[test]
    fn hue_saturation_dialog_survives_odd_values() {
        let (mut h, id) = dialog_for("image.adjustments.hueSaturation");
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        d.fields.insert("__hsRange".into(), json!(99));
        d.fields.insert("hue".into(), json!("lots"));
        d.fields.insert("reds".into(), json!({"range": [f64::MAX, -1e30, 3, "x"], "hue": 1e9}));
        h.run_steps(3);
        assert_eq!(field(&h, id, "__hsRange"), json!(0));
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("__hsRange".into(), json!(1));
        h.run_steps(3);
        assert!(h.state().ui.dialogs.iter().any(|d| d.id == id));
    }

    /// Run explicitly for visual QA: the Exposure dialog in two themes.
    #[test]
    #[ignore = "writes target/exposure-dialog/*.png for visual inspection"]
    fn write_exposure_dialog_sheet() {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/exposure-dialog");
        std::fs::create_dir_all(&out).unwrap();
        for kind in [crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::StudioLight] {
            let mut s = prism_engine::Session::new();
            s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
            let mut h = Harness::builder()
                .with_size(egui::vec2(600.0, 700.0))
                .with_pixels_per_point(2.0)
                .wgpu()
                .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), PrismApp::new(s, crate::Services::default()));
            PrismApp::setup_context(&h.ctx, kind);
            let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.exposure").unwrap();
            h.run_steps(3);
            let d = h.state_mut().ui.dialog_mut(id).unwrap();
            d.fields.insert("exposure".into(), json!(11.86));
            d.fields.insert("gamma".into(), json!(0.42));
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}.png"))).unwrap();
            let ok = h.state().ui.dialogs.last().map(|d| d.id).unwrap();
            h.state_mut().ui.dialogs.retain(|d| d.id != ok);
            let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.vibrance").unwrap();
            h.run_steps(3);
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("vibrance".into(), json!(53));
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}-vibrance.png"))).unwrap();
            let ok = h.state().ui.dialogs.last().map(|d| d.id).unwrap();
            h.state_mut().ui.dialogs.retain(|d| d.id != ok);
            let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.hueSaturation").unwrap();
            let d = h.state_mut().ui.dialog_mut(id).unwrap();
            d.fields.insert("__hsRange".into(), json!(1));
            d.fields.insert("reds".into(), json!({"hue": 120, "saturation": 30}));
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}-hue-saturation.png"))).unwrap();
            let ok = h.state().ui.dialogs.last().map(|d| d.id).unwrap();
            h.state_mut().ui.dialogs.retain(|d| d.id != ok);
            let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.colorBalance").unwrap();
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("midtones".into(), json!([58, -20, 0]));
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}-color-balance.png"))).unwrap();
            let ok = h.state().ui.dialogs.last().map(|d| d.id).unwrap();
            h.state_mut().ui.dialogs.retain(|d| d.id != ok);
            let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.blackWhite").unwrap();
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("tint".into(), json!(true));
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}-black-white.png"))).unwrap();
            let ok = h.state().ui.dialogs.last().map(|d| d.id).unwrap();
            h.state_mut().ui.dialogs.retain(|d| d.id != ok);
            crate::adjust_dialog::open(h.state_mut(), "image.adjustments.photoFilter").unwrap();
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}-photo-filter.png"))).unwrap();
            let ok = h.state().ui.dialogs.last().map(|d| d.id).unwrap();
            h.state_mut().ui.dialogs.retain(|d| d.id != ok);
            let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.channelMixer").unwrap();
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("red".into(), json!([200, 0, 0, 0]));
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}-channel-mixer.png"))).unwrap();
        }
    }

    #[test]
    fn curves_pigment_view_reverses_both_axes() {
        use crate::adjust_editors::curves_field_value;
        assert_eq!(curves_field_value(255.0, true), 100.0);
        assert_eq!(curves_field_value(64.0, true), 25.0);
        assert_eq!(curves_field_value(64.4, false), 64.0);
    }

    /// Run explicitly for visual QA: the Curves dialog with a point, in light and in ink.
    #[test]
    #[ignore = "writes target/curves-dialog/*.png for visual inspection"]
    fn write_curves_dialog_sheet() {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/curves-dialog");
        std::fs::create_dir_all(&out).unwrap();
        for (kind, pigment) in [(crate::theme::ThemeKind::ProMedium, false), (crate::theme::ThemeKind::ProMedium, true), (crate::theme::ThemeKind::StudioLight, false)] {
            let mut s = prism_engine::Session::new();
            s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
            s.execute("edit.fill", json!({"color": "#806040"})).unwrap();
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 48})).unwrap();
            s.execute("edit.fill", json!({"color": "#d0c0a0"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            let mut h = Harness::builder()
                .with_size(egui::vec2(860.0, 520.0))
                .with_pixels_per_point(2.0)
                .wgpu()
                .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), PrismApp::new(s, crate::Services::default()));
            PrismApp::setup_context(&h.ctx, kind);
            let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.curves").unwrap();
            h.run_steps(3);
            let d = h.state_mut().ui.dialog_mut(id).unwrap();
            d.fields.insert("points".into(), json!([[0, 0], [70, 40], [190, 220], [255, 255]]));
            d.fields.insert("blue".into(), json!([[0, 20], [255, 235]]));
            h.run_steps(2);
            if pigment {
                click(&mut h, "Pigment/Ink %", egui::Modifiers::NONE);
            }
            let graph = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::adjust_dialog::editor_mem("curves").with("curves-graph"))).unwrap();
            let x = if pigment { 255.0 - 70.0 } else { 70.0 };
            let y = if pigment { 255.0 - 40.0 } else { 40.0 };
            let at = graph.left_bottom() + egui::vec2(graph.width() * x / 255.0, -graph.height() * y / 255.0);
            h.event(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
            }
            // The pointer leaves the graph: the Intersection Line stays with the selected point.
            h.event(egui::Event::PointerMoved(egui::pos2(5.0, 5.0)));
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}-{}.png", if pigment { "ink" } else { "light" }))).unwrap();
        }
    }

    #[test]
    fn cancel_leaves_the_document_alone() {
        let (mut h, id) = dialog();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("brightness".into(), json!(-120));
        h.run_steps(2);
        let before = h.state().session.active().unwrap().revision;
        click(&mut h, "Cancel", egui::Modifiers::NONE);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().revision, before);
    }

    #[test]
    fn odd_field_values_draw_and_clamp() {
        let (mut h, id) = dialog();
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        d.fields.insert("brightness".into(), json!("lots"));
        d.fields.insert("contrast".into(), json!(1e12));
        d.fields.insert("legacy".into(), json!(null));
        h.run_steps(2);
        assert_eq!((field(&h, id, "brightness"), field(&h, id, "contrast"), field(&h, id, "legacy")), (json!(0.0), json!(100.0), json!(false)));
    }

    #[test]
    fn contrast_slider_has_zero_in_the_middle() {
        // Photoshop 27.4: -41 sits 0.78 of the way from the middle to the left end, 50 halfway right.
        assert_eq!(contrast_pos(0.0, -50.0), 0.0);
        assert_eq!(contrast_pos(-50.0, -50.0), -1.0);
        assert_eq!(contrast_pos(100.0, -50.0), 1.0);
        assert_eq!(contrast_pos(50.0, -50.0), 0.5);
        assert!((contrast_pos(-41.0, -50.0) + 0.82).abs() < 1e-6);
        assert_eq!(contrast_pos(-100.0, -100.0), -1.0);
        for lo in [-50.0, -100.0] {
            for c in [-50.0, -13.0, 0.0, 7.0, 100.0] {
                assert_eq!(contrast_at(contrast_pos(c, lo), lo), c);
            }
        }
        assert_eq!(contrast_at(f32::NAN, -50.0), 0.0);
        assert_eq!(contrast_at(9.0, -50.0), 100.0);
    }
}
