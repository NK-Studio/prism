//! Tool Property of imported CLIP STUDIO sub-tools other than brushes (fill, auto select,
//! selection, gradient, figure, text, eyedropper, operation): the items CLIP STUDIO shows
//! (`VariantShowParam`), in its order, drawn from the catalogue `prism_paint::sut_catalog` with
//! CLIP STUDIO's controls and labels (translation context `cspTool`). Every value is the preset's
//! sub-tool setting, edited with `brush.sutSettings.set`; the ones Prism runs also set the options
//! of the Prism tool behind the sub-tool ([`apply`]): font names pick the Type tool's font, the
//! gradient's stops become the Gradient tool's gradient. Settings Prism does not run stay visible
//! and editable for a round trip, marked as such. Evidence: docs/csp-subtool-gaps.md.
use std::collections::BTreeMap;

use egui::{Sense, Stroke, vec2};
use prism_engine::paint::csp_gradient::{CspGradient, CspStopColor};
use prism_engine::paint::sut::{SutBrushMetadata, SutToolKind};
use prism_engine::paint::sut_catalog::{self as cat, Field, Item, Kind, MAX_SETTING_VALUE};
use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::state::Tool;
use crate::theme::Tokens;

pub use cat::CLOSE_GAP_STEPS;

/// The indicator level (1–5) CLIP STUDIO lights for a close gap of `px`: the steps it reaches.
pub fn close_gap_level(px: f64) -> usize {
    CLOSE_GAP_STEPS.iter().filter(|s| px + 1e-6 >= **s).count().max(1)
}

/// Whether this panel draws `m`'s Tool Property: an imported sub-tool that is not a brush and
/// that Prism runs on one of its tools.
pub fn handles(m: &SutBrushMetadata) -> bool {
    !m.kind().is_brush() && crate::sub_tool_panel::imported_tool(m).is_some()
}

/// What Prism does with an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Runs,
    /// Runs with the given difference.
    Partly(&'static str),
    /// Kept and editable, not used.
    NotRun,
}

const ALL_LAYERS: &str = "Prism refers to all layers for this choice";
const SEMI_TRANSPARENT: &str = "Prism fills as for “Treat semi-transparent as transparent”";
const SHRINK_ONLY: &str = "Prism's enclose and fill already covers the line art; only shrinking changes it";
const ELLIPSE: &str = "Prism draws an ellipse gradient as a circle";
const GRADIENT_DATA: &str = "No gradient is stored; Prism paints its current gradient";
const GRADIENT_CURVE: &str = "Curves next to the main or sub color run straight in Prism";
const FONT_MISSING: &str = "This font is not installed; the Type tool keeps its font";
const NORMAL_ONLY: &str = "Prism uses the Normal blending mode";
const DISPLAYED: &str = "Prism picks the displayed color";

/// Settings read and edited this frame.
struct Edits<'a> {
    params: &'a BTreeMap<String, f64>,
    strings: &'a BTreeMap<String, String>,
    gradient: Option<&'a CspGradient>,
    changes: Map<String, Value>,
    /// The gradient as edited this frame (`Some(None)` removes it).
    new_gradient: Option<Option<CspGradient>>,
}

impl<'a> Edits<'a> {
    fn new(m: &'a SutBrushMetadata) -> Self {
        Edits { params: &m.parameters, strings: &m.strings, gradient: m.gradient.as_ref(), changes: Map::new(), new_gradient: None }
    }
    fn text(&self, key: &str) -> Option<&str> {
        match self.changes.get(key) {
            Some(v) => v.as_str(),
            None => self.strings.get(key).map(String::as_str),
        }
    }
    fn set_text(&mut self, key: &str, v: Option<&str>) {
        self.changes.insert(key.into(), v.map_or(Value::Null, |v| json!(v)));
    }
    fn grad(&self) -> Option<&CspGradient> {
        match &self.new_gradient {
            Some(g) => g.as_ref(),
            None => self.gradient,
        }
    }

    fn get(&self, key: &str) -> Option<f64> {
        self.changes.get(key).and_then(Value::as_f64).or_else(|| self.params.get(key).copied()).filter(|v| v.is_finite())
    }
    fn on(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| v != 0.0)
    }
    fn code(&self, key: &str) -> Option<i64> {
        self.get(key).map(|v| v.round().clamp(-MAX_SETTING_VALUE, MAX_SETTING_VALUE) as i64)
    }
    fn set(&mut self, key: &str, v: f64) {
        self.changes.insert(key.into(), json!(v));
    }
}

/// What Prism does with item `id` of sub-tool `m` running as `tool`.
fn status(m: &SutBrushMetadata, tool: Tool, id: u32, e: &Edits) -> Status {
    use Status::*;
    let kind = m.kind();
    let enclose = kind == SutToolKind::Fill && tool == Tool::Lasso;
    let fills = enclose || matches!(tool, Tool::PaintBucket | Tool::MagicWand);
    let marquee = matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee);
    match id {
        5101 | 5102 if enclose => Runs,
        5111 if enclose => {
            if e.on("FillUseExpand") && e.get("FillExpandLength").is_some_and(|v| v > 0.0) {
                Partly(SHRINK_ONLY)
            } else {
                Runs
            }
        }
        5120 | 5130 if fills => Runs,
        5200 if fills && !enclose => Runs,
        5000 | 5001 if fills => {
            if e.on("FillReferMultiLayer") && e.code("FillReferLayerType").unwrap_or(0) != 0 {
                Partly(ALL_LAYERS)
            } else {
                Runs
            }
        }
        5300 if fills => {
            if matches!(e.code("CloseFillReferColorType").unwrap_or(7), 7 | 9) {
                Runs
            } else {
                Partly(SEMI_TRANSPARENT)
            }
        }
        10 if fills && kind == SutToolKind::Fill => Runs,
        3000 if kind == SutToolKind::Selection => Runs,
        20 if kind == SutToolKind::Selection && tool != Tool::MagicWand => Runs,
        3310..=3312 if marquee => Runs,
        5400 if kind == SutToolKind::Gradient => match e.grad() {
            None => Partly(GRADIENT_DATA),
            Some(g) if g.prism_stops().approximated => Partly(GRADIENT_CURVE),
            Some(_) => Runs,
        },
        5410 if kind == SutToolKind::Gradient => {
            if e.code("GradationShapeKind") == Some(2) {
                Partly(ELLIPSE)
            } else {
                Runs
            }
        }
        5440 if kind == SutToolKind::Gradient => Runs,
        30 if kind == SutToolKind::Gradient => {
            if e.code("CompositeMode").unwrap_or(0) == 0 {
                Runs
            } else {
                Partly(NORMAL_ONLY)
            }
        }
        1000 if kind == SutToolKind::Figure => Runs,
        3200 if kind == SutToolKind::Figure && tool != Tool::Line => Runs,
        3301 if tool == Tool::Polygon => Runs,
        3322 if tool == Tool::Rectangle => Runs,
        7001 | 7003 | 7004 if kind == SutToolKind::Text => Runs,
        7000 if kind == SutToolKind::Text => {
            if font_of(|k| e.text(k).map(str::to_string)).is_some_and(|name| match_family(&name, &crate::type_tool::families()).is_none()) {
                Partly(FONT_MISSING)
            } else {
                Runs
            }
        }
        4100 if kind == SutToolKind::Eyedropper => {
            if e.code("SpuitReferType") == Some(2) {
                Runs
            } else {
                Partly(DISPLAYED)
            }
        }
        4010 if tool == Tool::Move => Runs,
        _ => NotRun,
    }
}

/// A CLIP STUDIO label in the UI language, with CLIP STUDIO's own terms: the item's own context
/// first, then the shared `cspTool` one.
fn tc(item: &Item, s: &'static str) -> &'static str {
    let lang = crate::i18n::current();
    if !item.ctx.is_empty() {
        let t = crate::i18n::tr_ctx(lang, item.ctx, s);
        if t != s {
            return t;
        }
    }
    crate::i18n::tr_ctx(lang, "cspTool", s)
}

/// A short wrapped note under a setting.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).small().weak()).wrap());
}

/// The note over a list with settings Prism does not run (those are dimmed).
const DIMMED: &str = "Dimmed settings are kept for CLIP STUDIO; Prism does not use them yet";

/// Draw an item, dimmed when Prism does not run it, with the note on a difference.
fn with_status(ui: &mut egui::Ui, s: Status, draw: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        if s == Status::NotRun {
            ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
        }
        draw(ui);
    });
    if let Status::Partly(text) = s {
        note(ui, tl!(text));
    }
}

/// A slider's range, unit and steps.
#[derive(Clone, Copy)]
struct Bar {
    min: f64,
    max: f64,
    unit: cat::Unit,
    int: bool,
    log: bool,
}

/// A slider that fits the panel width (with its value box) under an optional label, laid out
/// like CLIP STUDIO's Tool Property (the name, then the bar).
fn slider(ui: &mut egui::Ui, label: Option<&str>, enabled: bool, value: &mut f64, bar: Bar) -> bool {
    let Bar { min, max, unit, int, log } = bar;
    let caption = label.map(|label| ui.add_enabled(enabled, egui::Label::new(label).truncate()).id);
    ui.horizontal(|ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(40.0);
        let mut s = egui::Slider::new(value, min..=max).logarithmic(log).suffix(unit.suffix()).max_decimals(if int { 0 } else { 1 });
        if int {
            s = s.step_by(1.0);
        }
        let r = ui.add_enabled(enabled, s);
        match caption {
            Some(id) => r.labelled_by(id).changed(),
            None => r.changed(),
        }
    })
    .inner
}

/// CLIP STUDIO's indicator: boxes lit up to the step `px` reaches; a click picks that step.
fn indicator(ui: &mut egui::Ui, label: &str, steps: &[f64], px: f64, enabled: bool) -> Option<f64> {
    let t = Tokens::get(ui.ctx());
    let level = steps.iter().filter(|s| px + 1e-6 >= **s).count().max(1);
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let n = steps.len().max(1) as f32;
        let w = ((ui.available_width() - 40.0) / n - 2.0).clamp(14.0, 40.0);
        for (i, step) in steps.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 18.0), if enabled { Sense::click() } else { Sense::hover() });
            let lit = enabled && i < level;
            ui.painter().rect_filled(r, 2.0, if lit { t.accent } else { t.field });
            ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
            let px_text = crate::widgets::fmt_num(*step);
            let resp = resp.on_hover_text(crate::i18n::fmt(tl!("{px} px"), &[("px", &px_text)]));
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, lit, format!("{label} {px_text} px")));
            if resp.clicked() {
                pick = Some(*step);
            }
        }
        ui.small(crate::i18n::fmt(tl!("{px} px"), &[("px", &crate::widgets::fmt_num(px))]));
    });
    pick
}

/// One field of `item`. `label` is its caption, if it shows one; `enabled` follows the item's
/// switch. `salt` keeps widget ids apart between items.
#[allow(clippy::too_many_arguments)]
fn field(app: &mut PrismApp, ui: &mut egui::Ui, item: &Item, fl: &Field, label: Option<&'static str>, enabled: bool, e: &mut Edits, salt: &str) {
    let text = label.map(|l| tc(item, l));
    match fl.kind {
        Kind::Slider { min, max, unit, int, log } => {
            // Missing values show CLIP STUDIO's usual default: 100 % or 0.
            let default = if unit == cat::Unit::Percent { 100.0 } else { 0.0 };
            let mut v = e.get(fl.key).unwrap_or(default).clamp(min, max);
            if slider(ui, text, enabled, &mut v, Bar { min, max, unit, int, log }) {
                e.set(fl.key, if int { v.round() } else { v });
            }
        }
        Kind::Check => {
            let mut on = e.on(fl.key);
            if ui.add_enabled(enabled, egui::Checkbox::new(&mut on, text.unwrap_or_else(|| tc(item, item.label)))).changed() {
                e.set(fl.key, f64::from(u8::from(on)));
            }
        }
        Kind::List(choices) => {
            let mut code = e.code(fl.key).unwrap_or(choices.first().map_or(0, |c| c.0));
            let unknown = (!choices.iter().any(|c| c.0 == code)).then(|| crate::i18n::fmt(tl!("CLIP STUDIO value {n}"), &[("n", &code.to_string())]));
            let mut opts: Vec<(i64, &str)> = choices.iter().map(|(c, l)| (*c, tc(item, l))).collect();
            if let Some(u) = &unknown {
                opts.push((code, u.as_str()));
            }
            if let Some(t) = text {
                ui.add_enabled(enabled, egui::Label::new(t).truncate());
            }
            let changed = ui.add_enabled_ui(enabled, |ui| crate::widgets::dropdown(ui, &format!("csp-prop-{salt}-{}", fl.key), &mut code, &opts, ui.available_width().min(260.0))).inner;
            if changed {
                e.set(fl.key, code as f64);
            }
        }
        Kind::Icons(choices) => {
            let current = e.code(fl.key).unwrap_or(0);
            if let Some(t) = text {
                ui.add_enabled(enabled, egui::Label::new(t).truncate());
            }
            ui.add_enabled_ui(enabled, |ui| {
                ui.horizontal(|ui| {
                    let t = Tokens::get(ui.ctx());
                    for (code, label, icon) in choices {
                        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
                        if *code == current {
                            ui.painter().rect_filled(r, t.radius_sm, t.accent_soft);
                        }
                        crate::icons::paint(ui, r, icon, 14.0, if enabled { t.text } else { t.text_faint });
                        let name = tc(item, label);
                        let resp = resp.on_hover_text(name);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, *code == current, name));
                        if resp.clicked() && enabled {
                            e.set(fl.key, *code as f64);
                        }
                    }
                });
            });
        }
        Kind::Indicator { steps, min, max, unit } => {
            let px = e.get(fl.key).unwrap_or(steps.first().copied().unwrap_or(min));
            let slider_id = ui.id().with(("csp-indicator-slider", salt, fl.key));
            let as_slider = ui.data(|d| d.get_temp::<bool>(slider_id)).unwrap_or(false);
            if as_slider {
                let mut v = px.clamp(min, max);
                if slider(ui, None, enabled, &mut v, Bar { min, max, unit, int: false, log: true }) {
                    e.set(fl.key, v);
                }
            } else if let Some(step) = indicator(ui, tc(item, item.label), steps, px, enabled) {
                e.set(fl.key, step);
            }
        }
        Kind::Color { green, blue } => {
            let to8 = |v: Option<f64>| (v.unwrap_or(0.0).clamp(0.0, MAX_SETTING_VALUE) / MAX_SETTING_VALUE * 255.0).round() as u8;
            let mut rgb = [to8(e.get(fl.key)), to8(e.get(green)), to8(e.get(blue))];
            ui.horizontal(|ui| {
                if let Some(t) = text {
                    ui.add_enabled(enabled, egui::Label::new(t));
                }
                let r = ui.add_enabled_ui(enabled, |ui| ui.color_edit_button_srgb(&mut rgb)).inner;
                if r.changed() {
                    // 255 × 0x01010101 is the full 32-bit channel CLIP STUDIO stores.
                    for (key, c) in [fl.key, green, blue].into_iter().zip(rgb) {
                        e.set(key, f64::from(c) * 16_843_009.0);
                    }
                }
            });
        }
        Kind::Code { max } => {
            let mut n = e.code(fl.key).unwrap_or(0).clamp(0, max);
            ui.horizontal(|ui| {
                let caption = text.map(|t| ui.add_enabled(enabled, egui::Label::new(t)).id);
                let mut r = ui
                    .add_enabled(enabled, egui::DragValue::new(&mut n).range(0..=max))
                    .on_hover_text(tl!("CLIP STUDIO's codes for this setting are not traced; it shows the stored number"));
                if let Some(id) = caption {
                    r = r.labelled_by(id);
                }
                if r.changed() {
                    e.set(fl.key, n as f64);
                }
            });
        }
        Kind::Font { unique } => {
            if let Some(t) = text {
                ui.add_enabled(enabled, egui::Label::new(t));
            }
            let stored = e.text(fl.key).or_else(|| unique.and_then(|u| e.text(u))).map(str::to_string);
            let families = crate::type_tool::families();
            // The installed family the name picks, else the name itself, else the Type tool's font.
            let mut current = match &stored {
                Some(name) => match_family(name, &families).unwrap_or_else(|| name.clone()),
                None => app.ui.tool_options.type_font.clone(),
            };
            let width = ui.available_width().min(260.0);
            if ui.add_enabled_ui(enabled, |ui| crate::type_tool::font_picker(app, ui, &mut current, width)).inner {
                e.set_text(fl.key, Some(&current));
                // The unique name named the old font.
                if let Some(u) = unique.filter(|u| e.text(u).is_some()) {
                    e.set_text(u, None);
                }
            }
        }
        Kind::Gradient => gradient_field(app, ui, enabled, e, salt),
        Kind::Data => {
            if let Some(t) = text {
                ui.add_enabled(false, egui::Label::new(t));
            }
            note(ui, tl!("Prism does not import this setting yet"));
        }
    }
}

/// The font name a text sub-tool stores: its family name, else its unique name.
fn font_of(text: impl Fn(&str) -> Option<String>) -> Option<String> {
    text("TextFontName").or_else(|| text("TextFontUniqueName"))
}

/// The installed font family a CLIP STUDIO font name picks: the same name (ignoring case, spaces
/// and punctuation), else the longest family the name starts with (a unique name such as
/// `SourceHanSansJP-Bold` names a face of a family).
pub(crate) fn match_family(name: &str, families: &[String]) -> Option<String> {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let n = norm(name);
    if n.is_empty() {
        return None;
    }
    let keyed: Vec<(String, &String)> = families.iter().map(|f| (norm(f), f)).filter(|(k, _)| !k.is_empty()).collect();
    keyed.iter().find(|(k, _)| *k == n).or_else(|| keyed.iter().filter(|(k, _)| n.starts_with(k.as_str())).max_by_key(|(k, _)| k.len())).map(|(_, f)| (*f).clone())
}

/// The gradient's bar (with the current drawing colours), its Reverse button and its stops.
fn gradient_field(app: &mut PrismApp, ui: &mut egui::Ui, enabled: bool, e: &mut Edits, salt: &str) {
    // Without stops the item's note says Prism paints its current gradient.
    let Some(g) = e.grad().cloned() else { return };
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let (fg, bg) = ([fg[0], fg[1], fg[2]], [bg[0], bg[1], bg[2]]);
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(260.0), 20.0), Sense::hover());
    crate::gradient_ui::paint_ramp(ui.painter(), r, |u| g.sample(u, fg, bg));
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, enabled, tl!("Gradient bar")));
    let mut edited: Option<CspGradient> = None;
    ui.add_enabled_ui(enabled, |ui| {
        if ui.button(tl!("Reverse gradient")).clicked() {
            edited = Some(g.reversed());
        }
        egui::CollapsingHeader::new(tl!("Stops")).id_salt(("csp-gradient-stops", salt)).show(ui, |ui| {
            for (i, stop) in g.stops.iter().enumerate() {
                ui.push_id(i, |ui| {
                    ui.horizontal(|ui| {
                        ui.small(format!("{:.0}%", stop.position * 100.0));
                        let kinds = [(0, tl!("Main color")), (1, tl!("Sub color")), (2, tl!("Specified color"))];
                        let mut kind = match stop.color {
                            CspStopColor::Foreground => 0,
                            CspStopColor::Background => 1,
                            CspStopColor::Rgb(_) => 2,
                        };
                        let mut next = stop.clone();
                        if crate::widgets::dropdown(ui, &format!("csp-stop-kind-{salt}-{i}"), &mut kind, &kinds, 104.0) {
                            let [r, g2, b, _] = g.sample(stop.position, fg, bg);
                            next.color = match kind {
                                0 => CspStopColor::Foreground,
                                1 => CspStopColor::Background,
                                _ => CspStopColor::Rgb([r, g2, b]),
                            };
                        }
                        if let CspStopColor::Rgb(c) = &mut next.color {
                            ui.color_edit_button_rgb(c).on_hover_text(tl!("Stop color"));
                        }
                        let mut pct = next.opacity * 100.0;
                        if ui.add(egui::DragValue::new(&mut pct).range(0.0..=100.0).suffix(" %").max_decimals(0)).on_hover_text(tl!("Stop opacity")).changed() {
                            next.opacity = (pct / 100.0).clamp(0.0, 1.0);
                        }
                        if next != *stop {
                            let mut copy = edited.clone().unwrap_or_else(|| g.clone());
                            if let Some(s) = copy.stops.get_mut(i) {
                                *s = next;
                            }
                            edited = Some(copy);
                        }
                    });
                });
            }
        });
    });
    if let Some(g) = edited.and_then(CspGradient::sanitized) {
        e.new_gradient = Some(Some(g));
    }
}

/// One item: its switch, its fields and the note on what Prism does with it.
fn draw_item(app: &mut PrismApp, ui: &mut egui::Ui, item: &Item, e: &mut Edits, salt: &str) {
    let mut enabled = true;
    let indicator_switch = item.fields.iter().any(|fl| matches!(fl.kind, Kind::Indicator { .. }));
    if let Some(toggle) = item.toggle {
        ui.horizontal(|ui| {
            let mut on = e.on(toggle);
            if ui.checkbox(&mut on, tc(item, item.label)).changed() {
                e.set(toggle, f64::from(u8::from(on)));
            }
            if indicator_switch {
                let slider_id = ui.id().with(("csp-indicator-slider", salt, item.fields.first().map_or("", |fl| fl.key)));
                let as_slider = ui.data(|d| d.get_temp::<bool>(slider_id)).unwrap_or(false);
                if ui.small_button(if as_slider { "▭" } else { "›" }).on_hover_text(tl!("Switch between indicator and slider")).clicked() {
                    ui.data_mut(|d| d.insert_temp(slider_id, !as_slider));
                }
            }
        });
        enabled = e.on(toggle);
    }
    let lone = item.toggle.is_none() && item.fields.len() == 1 && item.fields.first().is_some_and(|fl| fl.label.is_empty());
    if item.toggle.is_none() && !lone {
        // A heading over several fields (Gradient, Alignment…) or over none (a group Prism
        // does not map).
        ui.label(tc(item, item.label));
    }
    for fl in item.fields {
        let label = if !fl.label.is_empty() {
            Some(fl.label)
        } else if lone {
            Some(item.label)
        } else {
            None
        };
        // The indicator and the switch already carry the label, as in CLIP STUDIO.
        field(app, ui, item, fl, label, enabled, e, salt);
    }
    if item.fields.is_empty() {
        note(ui, tl!("Prism does not import this setting yet"));
    }
    if let Some(unit) = item.unit_key
        && e.get(unit).is_some_and(|u| u != 0.0)
    {
        note(ui, tl!("This length uses a unit other than pixels; Prism reads it as pixels"));
    }
}

/// Tool Property rows for the imported sub-tool `preset` (see [`handles`]).
pub fn show(app: &mut PrismApp, ui: &mut egui::Ui, preset: &str) {
    let Some(m) = app.session.tools.presets.iter().find(|p| p.name == preset).and_then(|p| p.sut.clone()) else { return };
    let Some(tool) = crate::sub_tool_panel::imported_tool(&m) else { return };
    let mut e = Edits::new(&m);
    let visible = m.effective().visible.clone();
    let shown = |id: u32| visible.contains(&id) || (id == 5001 && visible.contains(&5000));
    let mut drawn = std::collections::HashSet::new();
    let top = |id: &u32| !cat::owner(*id).is_some_and(|o| shown(o.id));
    let any_not_run = visible.iter().filter(|id| top(id)).filter_map(|id| cat::item(*id)).any(|i| {
        status(&m, tool, i.id, &e) == Status::NotRun || i.children.iter().any(|c| shown(*c) && status(&m, tool, *c, &e) == Status::NotRun)
    });
    if any_not_run {
        note(ui, tl!(DIMMED));
    }
    for &id in &visible {
        // Drawn inside the item it belongs to when that one is shown.
        if cat::owner(id).is_some_and(|o| shown(o.id)) {
            continue;
        }
        ui.add_space(2.0);
        let Some(item) = cat::item(id) else {
            ui.add_enabled_ui(false, |ui| {
                ui.label(crate::i18n::fmt(tl!("CLIP STUDIO setting {id}"), &[("id", &id.to_string())]));
            });
            note(ui, tl!("This CSP setting is not supported in Prism yet"));
            continue;
        };
        if !drawn.insert(item.id) {
            continue;
        }
        let salt = format!("{preset}-{}", item.id);
        ui.push_id(("csp-prop", item.id), |ui| {
            let s = status(&m, tool, item.id, &e);
            with_status(ui, s, |ui| draw_item(app, ui, item, &mut e, &salt));
            // Its children follow when CLIP STUDIO shows them (indented, as when expanded).
            let children: Vec<&Item> = item.children.iter().filter(|c| shown(**c)).filter_map(|c| cat::item(*c)).collect();
            if !children.is_empty() {
                ui.indent(("csp-children", item.id), |ui| {
                    for child in &children {
                        let s = status(&m, tool, child.id, &e);
                        with_status(ui, s, |ui| draw_item(app, ui, child, &mut e, &format!("{preset}-{}", child.id)));
                    }
                });
            }
        });
    }
    if !e.changes.is_empty() || e.new_gradient.is_some() {
        let mut p = json!({"name": preset});
        if !e.changes.is_empty() {
            p["values"] = Value::Object(std::mem::take(&mut e.changes));
        }
        if let Some(g) = e.new_gradient.take() {
            p["gradient"] = json!(g);
        }
        match app.run("brush.sutSettings.set", p) {
            Ok(_) => apply(app, preset),
            Err(err) => app.ui.status = err,
        }
    }
}

/// Set the options of the Prism tool behind the imported sub-tool `preset` from the sub-tool's
/// settings that Prism runs (see `status`).
pub(crate) fn apply(app: &mut PrismApp, preset: &str) {
    let Some(m) = app.session.tools.presets.iter().find(|p| p.name == preset).and_then(|p| p.sut.clone()) else { return };
    let tool = crate::sub_tool_panel::imported_tool(&m);
    let get = |k: &str| m.parameters.get(k).copied().filter(|v| v.is_finite());
    let kind = m.kind();
    let o = &mut app.ui.tool_options;
    if let Some(v) = get("FillReferMultiLayer") {
        o.sample_all_layers = v != 0.0;
    }
    // CSP's colour margin is a percentage of the channel range.
    if let Some(v) = get("FillColorMargin") {
        o.tolerance = (v.clamp(0.0, 100.0) * 2.55) as f32;
    }
    if let Some(v) = get("FillContinuousPixel") {
        o.contiguous = v != 0.0;
    }
    if let Some(v) = get("FillAntiAlias") {
        o.anti_alias = v != 0.0;
    }
    // A fill sub-tool's opacity is the fill opacity (percent, as in CLIP STUDIO).
    if let Some(v) = get("Opacity").filter(|_| m.parameters.contains_key("FillColorMargin")) {
        o.fill_opacity = v.clamp(0.0, 100.0) as f32;
    }
    match kind {
        SutToolKind::Selection => {
            if let Some(v) = get("SelectCreateType") {
                app.ui.selection_mode = v.round().clamp(0.0, 3.0) as u8;
            }
            if tool != Some(Tool::MagicWand)
                && let Some(v) = get("AntiAlias")
            {
                o.anti_alias = v != 0.0;
            }
            if matches!(tool, Some(Tool::RectMarquee | Tool::EllipseMarquee)) {
                let (w, h) = (get("FixedShapeWidth"), get("FixedShapeHeight"));
                match get("FixedShapeAspectType").map(|v| v.round() as i64) {
                    Some(1) => o.marquee_style = "fixedRatio".into(),
                    Some(2) => o.marquee_style = "fixedSize".into(),
                    Some(_) => o.marquee_style = "normal".into(),
                    None => {}
                }
                if let Some(w) = w {
                    o.marquee_width = w.clamp(0.001, 100_000.0) as f32;
                }
                if let Some(h) = h {
                    o.marquee_height = h.clamp(0.001, 100_000.0) as f32;
                }
            }
        }
        SutToolKind::Gradient => {
            if let Some(v) = get("GradationShapeKind") {
                o.gradient_style = if v == 0.0 { "linear" } else { "radial" }.into();
            }
            if let Some(v) = get("GradationDither") {
                o.gradient_dither = v != 0.0;
            }
            // The stops run from the drag's start (every CLIP STUDIO default stores
            // `GradationReverseSaved` 1 with its first stop at the start): it is not a flip.
            if m.gradient.is_some() {
                o.gradient_reverse = false;
            }
            if get("CompositeMode") == Some(0.0) {
                o.gradient_blend_mode = prism_color::BlendMode::Normal;
            }
        }
        SutToolKind::Figure => {
            if let Some(v) = get("BrushSize") {
                let px = v.clamp(0.0, 1000.0) as f32;
                if tool == Some(Tool::Line) {
                    o.line_weight = px.max(1.0);
                } else {
                    o.stroke_width = px;
                }
            }
            if tool != Some(Tool::Line) {
                match get("ShapeStrokeAndFillType").map(|v| v.round() as i64) {
                    Some(0) => {
                        o.shape_fill = true;
                        o.stroke_width = 0.0;
                    }
                    Some(1) => o.shape_fill = false,
                    Some(2) => o.shape_fill = true,
                    _ => {}
                }
            }
            if tool == Some(Tool::Polygon)
                && let Some(v) = get("FixedShapeVertexNumber")
            {
                o.polygon_sides = v.round().clamp(3.0, 100.0) as u32;
            }
            if tool == Some(Tool::Rectangle)
                && let Some(v) = get("FixedShapeRoundSize")
            {
                let round = get("FixedShapeUseRoundCorner").is_none_or(|u| u != 0.0);
                o.corner_radius = if round { v.clamp(0.0, 10_000.0) as f32 } else { 0.0 };
            }
        }
        SutToolKind::Text => {
            if let Some(v) = get("TextFontSize") {
                o.type_size = v.clamp(1.0, 1296.0) as f32;
            }
            match get("TextAlignment").map(|v| v.round() as i64) {
                Some(0) => o.type_align = "left".into(),
                Some(1) => o.type_align = "center".into(),
                Some(2) => o.type_align = "right".into(),
                _ => {}
            }
            if let Some(family) = font_of(|k| m.strings.get(k).cloned()).and_then(|name| match_family(&name, &crate::type_tool::families())) {
                let styles = crate::type_tool::styles(&family);
                if !styles.contains(&o.type_style) {
                    o.type_style = styles.first().cloned().unwrap_or_else(|| "Regular".into());
                }
                o.type_font = family;
            }
        }
        SutToolKind::Operation if tool == Some(Tool::Move) => {
            if let Some(v) = get("LayerMoveClicked") {
                o.move_auto_select = v != 0.0;
            }
        }
        _ => {}
    }
    if kind == SutToolKind::Gradient
        && let Some(g) = &m.gradient
    {
        select_gradient(app, preset, g);
    }
}

/// Make `g` the Gradient tool's gradient (Prism's stops follow the drawing colours live, as CLIP
/// STUDIO's main and sub colour stops do).
fn select_gradient(app: &mut PrismApp, name: &str, g: &CspGradient) {
    let s = g.prism_stops();
    let hex = |c: [f32; 3]| {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
    };
    let stops: Vec<Value> = s
        .colors
        .iter()
        .map(|(t, c)| match c {
            CspStopColor::Foreground => json!([t, "foreground"]),
            CspStopColor::Background => json!([t, "background"]),
            CspStopColor::Rgb(c) => json!([t, hex(*c)]),
        })
        .collect();
    let transparency: Vec<Value> = s.opacity.iter().map(|(t, a)| json!([t, a * 100.0])).collect();
    let p = json!({"stops": stops, "transparency": transparency, "name": name, "applyToLayer": false});
    if let Err(err) = app.run("gradient.presets.select", p) {
        app.ui.status = err;
    }
}

#[cfg(test)]
#[path = "csp_tool_property_tests.rs"]
mod tests;
