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
const OPTIONS_W: f32 = 232.0;
/// Right column: the buttons.
const BUTTON_W: f32 = 112.0;
const GAP: f32 = 16.0;
const FIELD_W: f32 = 56.0;
/// The dialog's content width.
pub const WIDTH: f32 = OPTIONS_W + GAP + BUTTON_W;

/// Levels' options column (its histogram is 256 levels wide plus the handles' margins).
const LEVELS_W: f32 = 290.0;
/// Levels' button column.
const LEVELS_BUTTON_W: f32 = 100.0;

/// Curves' channel frame (Output gutter, bar and 256-level graph).
const CURVES_W: f32 = 340.0;
/// Curves' display options column.
const CURVES_VIEW_W: f32 = 150.0;

fn kind(f: &Map<String, Value>) -> &str {
    f.get("__adjust").and_then(Value::as_str).unwrap_or_default()
}

/// Is this dialog drawn in Photoshop's classic adjustment layout?
pub fn owns(f: &Map<String, Value>) -> bool {
    matches!(kind(f), "brightnessContrast" | "levels" | "curves")
}

/// The dialog's content width.
pub fn width(f: &Map<String, Value>) -> f32 {
    match kind(f) {
        "levels" => LEVELS_W + GAP + LEVELS_BUTTON_W,
        "curves" => CURVES_W + GAP + CURVES_VIEW_W + GAP + LEVELS_BUTTON_W,
        _ => WIDTH,
    }
}

/// A label in this dialog, with Photoshop's own wording where it differs from Prism's elsewhere.
fn label(s: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "adjust", s)
}

fn num(f: &Map<String, Value>, key: &str) -> f32 {
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
fn preview_checkbox(ui: &mut egui::Ui, f: &mut Map<String, Value>, shortcut_in_label: bool) {
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
                let custom = values != defaults;
                let mut pick = if custom { "custom" } else { "default" }.to_string();
                let options = [("default".to_string(), tl!("Default")), ("custom".to_string(), tl!("Custom"))];
                let opts: Vec<(String, &str)> = options.iter().filter(|(k, _)| custom || k == "default").map(|(k, l)| (k.clone(), *l)).collect();
                if widgets::dropdown(ui, "pslv-preset", &mut pick, &opts, ui.available_width()) && pick == "default" && custom {
                    values = defaults.clone();
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

/// A titled group box (Photoshop's "Show Amount of:", "Grid size:", "Show:").
fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(title).color(t.text_dim));
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        add(ui);
    });
}

/// A radio button with its label.
fn radio(ui: &mut egui::Ui, on: bool, text: &str) -> bool {
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
                let custom = values != defaults;
                let mut pick = if custom { "custom" } else { "default" }.to_string();
                let options = [("default".to_string(), tl!("Default")), ("custom".to_string(), tl!("Custom"))];
                let opts: Vec<(String, &str)> = options.iter().filter(|(k, _)| custom || k == "default").map(|(k, l)| (k.clone(), *l)).collect();
                if widgets::dropdown(ui, "pscv-preset", &mut pick, &opts, CURVES_W) && pick == "default" && custom {
                    values = defaults.clone();
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
        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
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
