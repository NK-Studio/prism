//! Tool Property of imported Clip Studio fill and auto select sub-tools (SUT property IDs
//! 5000–5300): the items CLIP STUDIO shows (`VariantShowParam`), in its order, with its labels
//! and controls — Close gap as the five-step indicator (1, 3, 5, 10, 20 px) or a 0.1–50 px
//! slider, Area scaling, Refer multiple with its layer kinds, Tolerance, Target color… Values are
//! the preset's SUT settings (`brush.sutSettings.set`); the ones Prism runs also set the tool
//! options. Settings Prism does not run yet stay visible and editable for a round trip, marked as
//! such. Evidence: docs/csp-subtool-gaps.md.
use egui::{Sense, Stroke, vec2};
use prism_engine::paint::sut::{SutBrushMetadata, SutToolKind};
use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::theme::Tokens;

/// CLIP STUDIO's default Close gap indicator steps, in pixels.
pub const CLOSE_GAP_STEPS: [f64; 5] = [1.0, 3.0, 5.0, 10.0, 20.0];
/// The Close gap slider's range in CLIP STUDIO, in pixels.
pub const CLOSE_GAP_RANGE: std::ops::RangeInclusive<f64> = 0.1..=50.0;

/// The indicator level (1–5) CLIP STUDIO lights for a close gap of `px`: the steps it reaches.
pub fn close_gap_level(px: f64) -> usize {
    CLOSE_GAP_STEPS.iter().filter(|s| px + 1e-6 >= **s).count().max(1)
}

/// How Prism runs an imported fill-family sub-tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runs {
    /// Enclose and fill (output 51) on the Lasso: closes gaps, scales the area.
    EncloseFill,
    /// Refer-layers fill (outputs 50, 54) on the Paint Bucket.
    Bucket,
    /// Auto select on the Magic Wand.
    AutoSelect,
}

pub fn runs(m: &SutBrushMetadata) -> Option<Runs> {
    match m.kind() {
        SutToolKind::Fill if m.output_op == 51 => Some(Runs::EncloseFill),
        SutToolKind::Fill if m.output_op == 50 || m.output_op == 54 => Some(Runs::Bucket),
        SutToolKind::Selection if m.node_icon == 401 || m.input_op == 50 => Some(Runs::AutoSelect),
        _ => None,
    }
}

/// Target color choices (`CloseFillReferColorType`), in CLIP STUDIO's order.
const TARGET_COLORS: [&str; 10] = [
    "Target all colors",
    "Only transparent",
    "Area surrounded by transparent",
    "Only black",
    "Area surrounded by black",
    "Only white and transparent",
    "Area surrounded by white and transparent",
    "Treat semi-transparent as transparent",
    "All enclosed areas except transparency",
    "All enclosed areas including transparency",
];
/// Refer multiple layer kinds (`FillReferLayerType`) with their icons.
const LAYER_KINDS: [(&str, &str); 4] = [("All layers", "layers"), ("Reference layer", "compass"), ("Selected layer", "check"), ("Layer in folder", "folder")];
/// Scaling modes (`FillExpandType`).
const SCALING_MODES: [&str; 3] = ["Rectangle", "Round", "To darkest pixel"];
/// Children of Refer multiple: (id, key, label).
const EXCLUSIONS: [(u32, &str, &str); 7] = [
    (5010, "FillNotReferText", "Exclude text"),
    (5012, "FillNotReferDraft", "Exclude draft layers"),
    (5013, "FillNotReferCurrent", "Exclude editing layer"),
    (5014, "FillNotReferPaper", "Exclude paper layer"),
    (5015, "FillNotReferLocked", "Exclude locked layers"),
    (5020, "FillReferVectorCenter", "Fill up to vector path"),
    (5030, "FillReferFrame", "Refer to image border"),
];

/// A CLIP STUDIO label in the UI language, with CLIP STUDIO's own terms (context `cspFill`).
fn tc(s: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cspFill", s)
}

/// Settings edited this frame.
struct Edits<'a> {
    params: &'a std::collections::BTreeMap<String, f64>,
    changes: Map<String, Value>,
}

impl Edits<'_> {
    fn get(&self, key: &str) -> Option<f64> {
        self.changes.get(key).and_then(Value::as_f64).or_else(|| self.params.get(key).copied()).filter(|v| v.is_finite())
    }
    fn on(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| v != 0.0)
    }
    fn set(&mut self, key: &str, v: f64) {
        self.changes.insert(key.into(), json!(v));
    }
}

/// A short wrapped note under a setting.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).small().weak()).wrap());
}

/// A note under a setting Prism keeps but does not run yet.
fn not_run(ui: &mut egui::Ui) {
    note(ui, tl!("Kept for CLIP STUDIO; Prism does not use this setting yet"));
}

/// A slider that fits the panel width (with its value box) under an optional label, laid out
/// like CLIP STUDIO's Tool Property (the name, then the bar).
fn slider(ui: &mut egui::Ui, label: Option<&str>, enabled: bool, value: &mut f64, range: std::ops::RangeInclusive<f64>, suffix: &str, log: bool) -> bool {
    if let Some(label) = label {
        ui.add_enabled(enabled, egui::Label::new(label).truncate());
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(40.0);
        ui.add_enabled(enabled, egui::Slider::new(value, range).logarithmic(log).suffix(suffix).max_decimals(1)).changed()
    })
    .inner
}

/// A checkbox bound to a 0/1 setting.
fn check(ui: &mut egui::Ui, e: &mut Edits, key: &str, label: &str) -> bool {
    let mut on = e.on(key);
    let changed = ui.checkbox(&mut on, tc(label)).changed();
    if changed {
        e.set(key, f64::from(u8::from(on)));
    }
    changed
}

/// The Close gap indicator: five boxes, lit up to the level of `px`; a click picks that step.
fn indicator(ui: &mut egui::Ui, px: f64, enabled: bool) -> Option<f64> {
    let t = Tokens::get(ui.ctx());
    let level = close_gap_level(px);
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let w = ((ui.available_width() - 40.0) / 5.0 - 2.0).clamp(14.0, 40.0);
        for (i, step) in CLOSE_GAP_STEPS.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 18.0), if enabled { Sense::click() } else { Sense::hover() });
            let lit = enabled && i < level;
            ui.painter().rect_filled(r, 2.0, if lit { t.accent } else { t.field });
            ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
            let resp = resp.on_hover_text(crate::i18n::fmt(tl!("{px} px"), &[("px", &crate::widgets::fmt_num(*step))]));
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, lit, crate::i18n::fmt(tl!("Close gap {px} px"), &[("px", &crate::widgets::fmt_num(*step))])));
            if resp.clicked() {
                pick = Some(*step);
            }
        }
        ui.small(crate::i18n::fmt(tl!("{px} px"), &[("px", &crate::widgets::fmt_num(px))]));
    });
    pick
}

/// Tool Property rows for the imported sub-tool `preset` (a fill-family sub-tool, see [`runs`]).
pub fn show(app: &mut PrismApp, ui: &mut egui::Ui, preset: &str) {
    let Some(m) = app.session.tools.presets.iter().find(|p| p.name == preset).and_then(|p| p.sut.clone()) else { return };
    let Some(how) = runs(&m) else { return };
    let mut e = Edits { params: &m.parameters, changes: Map::new() };
    let mut opacity_changed = None;
    for id in m.effective().visible.clone() {
        ui.add_space(2.0);
        match id {
            5101 => {
                let px = e.get("FillCloseGap").unwrap_or(CLOSE_GAP_STEPS[0]);
                let runs_it = how == Runs::EncloseFill;
                let slider_id = ui.id().with(("csp-close-gap-slider", preset));
                let mut as_slider = ui.data(|d| d.get_temp::<bool>(slider_id)).unwrap_or(false);
                ui.horizontal(|ui| {
                    check(ui, &mut e, "FillUseCloseGap", "Close gap");
                    if ui.small_button(if as_slider { "▭" } else { "›" }).on_hover_text(tl!("Switch between indicator and slider")).clicked() {
                        as_slider = !as_slider;
                        ui.data_mut(|d| d.insert_temp(slider_id, as_slider));
                    }
                });
                let on = e.on("FillUseCloseGap");
                if as_slider {
                    let mut v = px.clamp(*CLOSE_GAP_RANGE.start(), *CLOSE_GAP_RANGE.end());
                    if slider(ui, None, on, &mut v, CLOSE_GAP_RANGE, " px", true) {
                        e.set("FillCloseGap", v);
                    }
                } else if let Some(step) = indicator(ui, px, on) {
                    e.set("FillCloseGap", step);
                }
                if e.get("FillCloseGapUnit").is_some_and(|u| u != 0.0) {
                    note(ui, tl!("This length uses a unit other than pixels; Prism reads it as pixels"));
                }
                if !runs_it {
                    not_run(ui);
                }
            }
            5102 => {
                check(ui, &mut e, "FillSoakIntoNarrowArea", "Fill narrow areas");
                if how != Runs::EncloseFill {
                    not_run(ui);
                }
            }
            5111 => {
                check(ui, &mut e, "FillUseExpand", "Area scaling");
                let mut v = e.get("FillExpandLength").unwrap_or(0.0).clamp(-50.0, 50.0);
                let on = e.on("FillUseExpand");
                if slider(ui, None, on, &mut v, -50.0..=50.0, " px", false) {
                    e.set("FillExpandLength", v);
                }
                if e.get("FillExpandLengthUnit").is_some_and(|u| u != 0.0) {
                    note(ui, tl!("This length uses a unit other than pixels; Prism reads it as pixels"));
                }
                if how != Runs::EncloseFill {
                    not_run(ui);
                } else if on && v > 0.0 {
                    note(ui, tl!("Prism's enclose and fill already covers the line art; only shrinking changes it"));
                }
            }
            5112 => {
                let mut mode = e.get("FillExpandType").map_or(0, |v| (v.max(0.0) as usize).min(SCALING_MODES.len() - 1));
                let opts: Vec<(usize, &str)> = SCALING_MODES.iter().enumerate().map(|(i, l)| (i, tc(l))).collect();
                ui.horizontal(|ui| {
                    ui.label(tc("Scaling mode"));
                    if crate::widgets::dropdown(ui, "csp-scaling-mode", &mut mode, &opts, ui.available_width().min(200.0)) {
                        e.set("FillExpandType", mode as f64);
                    }
                });
                not_run(ui);
            }
            5120 => {
                let mut v = e.get("FillColorMargin").unwrap_or(10.0).clamp(0.0, 100.0);
                if slider(ui, Some(tc("Tolerance")), true, &mut v, 0.0..=100.0, "", false) {
                    e.set("FillColorMargin", v);
                }
            }
            5000 | 5001 => {
                check(ui, &mut e, "FillReferMultiLayer", "Refer multiple");
                let mut kind = e.get("FillReferLayerType").map_or(0, |v| (v.max(0.0) as usize).min(LAYER_KINDS.len() - 1));
                let on = e.on("FillReferMultiLayer");
                ui.add_enabled_ui(on, |ui| {
                    ui.horizontal(|ui| {
                        let t = Tokens::get(ui.ctx());
                        for (i, (label, icon)) in LAYER_KINDS.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
                            if i == kind {
                                ui.painter().rect_filled(r, t.radius_sm, t.accent_soft);
                            }
                            crate::icons::paint(ui, r, icon, 14.0, if on { t.text } else { t.text_faint });
                            let resp = resp.on_hover_text(tc(label));
                            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, on, i == kind, tc(label)));
                            if resp.clicked() {
                                kind = i;
                                e.set("FillReferLayerType", i as f64);
                            }
                        }
                    });
                });
                if on && kind != 0 {
                    note(ui, tl!("Prism refers to all layers for this choice"));
                }
                // Its exclusions follow when CLIP STUDIO shows them (indented, as when expanded).
                for (child, key, label) in EXCLUSIONS {
                    if m.effective().visible.contains(&child) {
                        ui.indent(("csp-refer", child), |ui| {
                            check(ui, &mut e, key, label);
                        });
                    }
                }
                if EXCLUSIONS.iter().any(|(child, ..)| m.effective().visible.contains(child)) {
                    ui.indent("csp-refer-note", not_run);
                }
            }
            5010 | 5012..=5015 | 5020 | 5030 => {
                // Drawn under Refer multiple when that is shown; on their own otherwise.
                if !m.effective().visible.iter().any(|v| matches!(v, 5000 | 5001))
                    && let Some((_, key, label)) = EXCLUSIONS.iter().find(|(c, ..)| *c == id)
                {
                    check(ui, &mut e, key, label);
                    not_run(ui);
                }
            }
            5130 => {
                check(ui, &mut e, "FillAntiAlias", "Anti-aliasing");
            }
            5200 => {
                check(ui, &mut e, "FillContinuousPixel", "Apply to connected pixels only");
                if how == Runs::EncloseFill {
                    not_run(ui);
                }
            }
            5300 => {
                let mut kind = e.get("CloseFillReferColorType").map_or(7, |v| (v.max(0.0) as usize).min(TARGET_COLORS.len() - 1));
                let opts: Vec<(usize, &str)> = TARGET_COLORS.iter().enumerate().map(|(i, l)| (i, tc(l))).collect();
                ui.label(tc("Target color"));
                if crate::widgets::dropdown(ui, "csp-target-color", &mut kind, &opts, ui.available_width().min(260.0)) {
                    e.set("CloseFillReferColorType", kind as f64);
                }
                if !matches!(kind, 7 | 9) {
                    note(ui, tl!("Prism fills as for “Treat semi-transparent as transparent”"));
                }
            }
            10 => {
                let mut v = e.get("Opacity").unwrap_or(100.0).clamp(0.0, 100.0);
                if slider(ui, Some(tc("Opacity")), true, &mut v, 0.0..=100.0, " %", false) {
                    e.set("Opacity", v);
                    opacity_changed = Some(v);
                }
            }
            40 => {
                let mut v = e.get("FlickerReduction").unwrap_or(0.0).clamp(0.0, 100.0);
                if slider(ui, Some(tc("Stabilization")), true, &mut v, 0.0..=100.0, "", false) {
                    e.set("FlickerReduction", v);
                }
                not_run(ui);
            }
            other => {
                ui.add_enabled_ui(false, |ui| {
                    ui.label(crate::i18n::fmt(tl!("CLIP STUDIO setting {id}"), &[("id", &other.to_string())]));
                });
                note(ui, tl!("This CSP setting is not supported in Prism yet"));
            }
        }
    }
    if !e.changes.is_empty() {
        let changes = std::mem::take(&mut e.changes);
        match app.run("brush.sutSettings.set", json!({"name": preset, "values": changes})) {
            Ok(_) => crate::sub_tool_panel::apply_imported(app, preset),
            Err(err) => app.ui.status = err,
        }
        if let Some(v) = opacity_changed {
            app.ui.tool_options.fill_opacity = v as f32;
        }
    }
}

#[cfg(test)]
#[path = "csp_fill_property_tests.rs"]
mod tests;
