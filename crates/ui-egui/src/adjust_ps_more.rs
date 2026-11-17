//! The rest of Image › Adjustments in Photoshop 27.4's layouts (captured 2026-11-17, see
//! `docs/adjust-analysis.md`): Posterize, Threshold, Gradient Map, Selective Color, Color Lookup,
//! Shadows/Highlights, HDR Toning, Match Color and Replace Color.
//!
//! Each dialog edits the command's params (plus private `__` keys) like the other adjustment
//! dialogs, so the preview, OK / Cancel (Alt: Reset), Cmd+Z and automation work the same way.
//! Controls Photoshop has but Prism's adjustments don't model (Load… / Save…, Show Clipping-style
//! extras, Localized Color Clusters, Abstract / Device Link profiles, Edge Smoothness, Gradient
//! Map's Method) are left out rather than shown doing nothing.

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::adjust_ps_ui::{BUTTON_W, FIELD_W, GAP, OPTIONS_W, fine_row, group, label, preview_checkbox, radio, signed};
use crate::widgets;

/// The kinds drawn here.
pub fn owns(kind: &str) -> bool {
    matches!(
        kind,
        "posterize" | "threshold" | "gradientMap" | "selectiveColor" | "colorLookup" | "shadowsHighlights" | "hdrToning" | "matchColor" | "replaceColor"
    )
}

/// Width of the options column.
fn options_w(kind: &str) -> f32 {
    match kind {
        "threshold" => 270.0,
        "gradientMap" => 290.0,
        "selectiveColor" => 300.0,
        "colorLookup" => 300.0,
        "shadowsHighlights" => 300.0,
        "hdrToning" => 300.0,
        "matchColor" => 320.0,
        "replaceColor" => 330.0,
        _ => OPTIONS_W,
    }
}

/// The dialog's content width.
pub fn width(kind: &str) -> f32 {
    options_w(kind) + GAP + BUTTON_W
}

/// What the right-hand column holds besides OK / Cancel.
struct Column {
    /// Preview's label carries Opt+P / Alt+P (the redesigned dialogs).
    shortcut: bool,
}

/// The dialog: the options column, then OK, Cancel (Alt: Reset to `defaults`) and Preview.
pub fn body(kind: &str, app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let mut outcome = None;
    let mut values = crate::filter_dialog::params_of(f);
    let defaults = defaults(kind, app);
    let alt = ui.input(|i| i.modifiers.alt);
    let mut changed = false;
    let column = Column { shortcut: matches!(kind, "posterize" | "selectiveColor" | "colorLookup" | "shadowsHighlights") };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(options_w(kind));
            changed |= match kind {
                "posterize" => posterize(ui, &mut values),
                "threshold" => threshold(app, ui, &mut values),
                "gradientMap" => gradient_map(app, ui, &mut values),
                "selectiveColor" => selective_color(ui, &mut values),
                "colorLookup" => color_lookup(app, ui, &mut values, f),
                "shadowsHighlights" => shadows_highlights(ui, &mut values),
                "hdrToning" => hdr_toning(app, ui, &mut values),
                "matchColor" => match_color(app, ui, &mut values),
                "replaceColor" => replace_color(app, ui, &mut values, f),
                _ => false,
            };
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
            preview_checkbox(ui, f, column.shortcut);
        });
    });
    if changed && let Value::Object(v) = values {
        f.retain(|k, _| k.starts_with("__"));
        f.extend(v);
    }
    outcome
}

/// The settings Alt+Cancel (Reset) puts back.
fn defaults(kind: &str, app: &crate::PrismApp) -> Value {
    let mode = app.session.active().map_or(prism_doc::ColorMode::Rgb, |s| s.doc.mode);
    if let Ok(d) = prism_engine::adjust_params::default_for(kind, mode) {
        return prism_engine::adjust_params::to_params(&d);
    }
    match kind {
        "selectiveColor" => {
            let mut v = json!({"method": "relative"});
            for r in SELECTIVE_RANGES {
                v[r] = json!([0.0, 0.0, 0.0, 0.0]);
            }
            v
        }
        "colorLookup" => json!({"lut": "none", "dither": false}),
        "shadowsHighlights" => json!({"shadowAmount": 35, "shadowTone": 50, "shadowRadius": 30, "highlightAmount": 0, "highlightTone": 50, "highlightRadius": 30, "color": 20, "midtone": 0, "blackClip": 0.01, "whiteClip": 0.01}),
        "hdrToning" => json!({"radius": 30, "strength": 0.5, "gamma": 1, "exposure": 0, "detail": 30, "shadow": 0, "highlight": 0, "vibrance": 0, "saturation": 20}),
        "matchColor" => json!({"luminance": 100, "intensity": 100, "fade": 0, "neutralize": false, "useSelectionInSource": false, "useSelectionInTarget": false}),
        "replaceColor" => json!({"fuzziness": 40, "hue": 0, "saturation": 0, "lightness": 0}),
        _ => Value::Null,
    }
}

fn get(v: &Value, key: &str, default: f32) -> f32 {
    v.get(key).and_then(Value::as_f64).filter(|x| x.is_finite()).map_or(default, |x| x as f32)
}

fn plain(v: f64) -> String {
    signed(v, 0).trim_start_matches('+').to_string()
}

/// A labelled row as the redesigned dialogs draw it: the label with its field at the right, the
/// slider under them. `step` rounds, `unit` follows the number.
fn row(ui: &mut egui::Ui, text: &str, x: &mut f32, range: std::ops::RangeInclusive<f32>, step: f32, unit: &str, track: Option<&[Color32]>) -> bool {
    let before = *x;
    ui.horizontal(|ui| {
        ui.label(text);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            widgets::value_field(ui, x, range.clone(), unit, FIELD_W + 8.0);
        });
    });
    widgets::slider(ui, x, range.clone(), track);
    *x = ((*x / step).round() * step).clamp(*range.start(), *range.end());
    ui.add_space(6.0);
    *x != before
}

/// A classic-dialog row: right-aligned label, slider, field (Shadows/Highlights, HDR Toning).
fn classic_row(ui: &mut egui::Ui, text: &str, x: &mut f32, range: std::ops::RangeInclusive<f32>, step: f32, unit: &str) -> bool {
    let before = *x;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(88.0, 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.set_min_width(88.0);
            ui.add_space(4.0);
            ui.label(text);
        });
        let slider_w = (ui.available_width() - FIELD_W - 24.0).max(90.0);
        ui.allocate_ui(vec2(slider_w, 22.0), |ui| {
            ui.set_width(slider_w);
            widgets::slider(ui, x, range.clone(), None);
        });
        widgets::value_field(ui, x, range.clone(), unit, FIELD_W + 12.0);
    });
    *x = ((*x / step).round() * step).clamp(*range.start(), *range.end());
    *x != before
}

fn put(v: &mut Value, key: &str, x: f32) {
    v[key] = json!(x);
}

// ---------------------------------------------------------------------------------------------
// Posterize

/// Photoshop 27.4: "Levels" with its field and a 2..255 slider.
fn posterize(ui: &mut egui::Ui, v: &mut Value) -> bool {
    let mut levels = get(v, "levels", 4.0).clamp(2.0, 255.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    let changed = fine_row(ui, crate::adjust_ps_ui::label("Levels"), &mut levels, 2.0..=255.0, 1.0, plain, |x| x, |p| p);
    if changed {
        put(v, "levels", levels);
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// Threshold

/// Photoshop 27.4 (classic): "Threshold Level:" with its field centred over the histogram and a
/// house-shaped slider under it.
fn threshold(app: &mut crate::PrismApp, ui: &mut egui::Ui, v: &mut Value) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut level = get(v, "level", 128.0).clamp(1.0, 255.0);
    let before = level;
    ui.horizontal(|ui| {
        ui.add_space(52.0);
        ui.label(label("Threshold Level:"));
        widgets::value_field(ui, &mut level, 1.0..=255.0, "", FIELD_W);
    });
    ui.add_space(4.0);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 100.0), Sense::hover());
    let p = ui.painter_at(r.expand(1.0));
    p.rect_filled(r, 0.0, t.field);
    let hist = app.session.active().and_then(|s| s.active_layer).map(|id| crate::tone::histograms(app, crate::tone::HistSource::Layer(id), prism_doc::adjust::ToneSpace::Rgb));
    if let Some(h) = hist.as_ref().and_then(|h| h.ch.first()) {
        crate::tone::draw_histogram(&p, r, h, Color32::from_gray(if t.pro { 146 } else { 120 }));
    }
    // The house-shaped slider under the histogram.
    let (strip, resp) = ui.allocate_exact_size(vec2(w, 16.0), Sense::click_and_drag());
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        level = ((pos.x - r.left()) / r.width() * 255.0).round().clamp(1.0, 255.0);
    }
    let x = r.left() + level / 255.0 * r.width();
    let y0 = strip.top() + 2.0;
    let house = vec![pos2(x, y0), pos2(x + 6.0, y0 + 6.0), pos2(x + 6.0, y0 + 12.0), pos2(x - 6.0, y0 + 12.0), pos2(x - 6.0, y0 + 6.0)];
    ui.painter().add(egui::Shape::convex_polygon(house, Color32::from_gray(225), Stroke::new(1.0, Color32::from_gray(90))));
    level = level.round().clamp(1.0, 255.0);
    let changed = level != before;
    if changed {
        put(v, "level", level);
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// Gradient Map

/// Photoshop 27.4 (classic): "Gradient Used for Grayscale Mapping" (the gradient, a click on it
/// edits its stops, ▾ picks a preset) and "Gradient Options" (Dither, Reverse).
fn gradient_map(app: &mut crate::PrismApp, ui: &mut egui::Ui, v: &mut Value) -> bool {
    let mut changed = false;
    let cx = crate::adjust_editors::EditorCx {
        mem: crate::adjust_dialog::editor_mem("gradientMap"),
        hist: None,
        gray: false,
        swatches: crate::adjust_editors::swatches(app),
        gradients: crate::adjust_editors::gradient_presets(app, "gradientMap"),
        dialog: true,
    };
    group(ui, label("Gradient Used for Grayscale Mapping"), |ui| {
        let open_id = cx.mem.with("gm-stops-open");
        let mut open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(false);
        ui.horizontal(|ui| {
            let w = ui.available_width() - 44.0;
            if crate::adjust_editors::gradient_map_bar(ui, v, w).clicked() {
                open = !open;
            }
            changed |= crate::adjust_editors::gradient_map_presets(ui, v, &cx, true).changed;
        });
        if open {
            ui.add_space(6.0);
            changed |= crate::adjust_editors::gradient_map_stops(ui, v).changed;
        }
        ui.data_mut(|d| d.insert_temp(open_id, open));
    });
    ui.add_space(10.0);
    group(ui, label("Gradient Options"), |ui| {
        for (key, text) in [("dither", tl!("Dither")), ("reverse", tl!("Reverse"))] {
            let mut b = v.get(key).and_then(Value::as_bool).unwrap_or(false);
            if widgets::checkbox(ui, &mut b, text).changed() {
                v[key] = json!(b);
                changed = true;
            }
        }
    });
    changed
}

// ---------------------------------------------------------------------------------------------
// Selective Color

const SELECTIVE_RANGES: [&str; 9] = ["reds", "yellows", "greens", "cyans", "blues", "magentas", "whites", "neutrals", "blacks"];

/// Photoshop 27.4: "Preset" (Default / Custom), nine round colour swatches (Reds … Blacks), the
/// Cyan / Magenta / Yellow / Black rows ("0%", -100..100 on colour tracks), Method (Relative /
/// Absolute).
fn selective_color(ui: &mut egui::Ui, v: &mut Value) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut changed = false;
    // The single-range keys of the parameter form are folded into the per-range arrays.
    if let Some(c) = v.get("colors").and_then(Value::as_str).map(str::to_owned)
        && SELECTIVE_RANGES.contains(&c.as_str())
        && ["cyan", "magenta", "yellow", "black"].iter().any(|k| v.get(*k).and_then(Value::as_f64).is_some_and(|x| x != 0.0))
    {
        let row: Vec<f64> = ["cyan", "magenta", "yellow", "black"].iter().map(|k| v.get(*k).and_then(Value::as_f64).unwrap_or(0.0)).collect();
        v[c.as_str()] = json!(row);
    }
    if let Some(o) = v.as_object_mut() {
        for k in ["colors", "cyan", "magenta", "yellow", "black"] {
            o.remove(k);
        }
    }
    let range_id = crate::adjust_dialog::editor_mem("selectiveColor").with("range");
    let mut range: usize = ui.data(|d| d.get_temp(range_id)).unwrap_or(0).min(8);
    let custom = SELECTIVE_RANGES.iter().any(|r| v.get(*r).and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| x.as_f64().is_some_and(|x| x != 0.0))));
    ui.horizontal(|ui| {
        ui.label(label("Preset"));
        let mut pick = if custom { "custom" } else { "default" }.to_string();
        let mut opts = vec![("default".to_string(), tl!("Default"))];
        if custom {
            opts.push(("custom".to_string(), tl!("Custom")));
        }
        if widgets::dropdown(ui, "psselc-preset", &mut pick, &opts, ui.available_width()) && pick == "default" && custom {
            for r in SELECTIVE_RANGES {
                v[r] = json!([0.0, 0.0, 0.0, 0.0]);
            }
            changed = true;
        }
    });
    ui.add_space(6.0);
    let colours = [
        Color32::from_rgb(255, 0, 0),
        Color32::from_rgb(255, 255, 0),
        Color32::from_rgb(0, 255, 0),
        Color32::from_rgb(0, 255, 255),
        Color32::from_rgb(0, 0, 255),
        Color32::from_rgb(255, 0, 255),
        Color32::WHITE,
        Color32::from_gray(128),
        Color32::BLACK,
    ];
    let names = ["Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas", "Whites", "Neutrals", "Blacks"];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, (c, name)) in colours.iter().zip(names).enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 30.0), Sense::click());
            let centre = rect.center();
            ui.painter().circle_filled(centre, 11.0, *c);
            ui.painter().circle_stroke(centre, 11.0, Stroke::new(1.0, Color32::from_black_alpha(90)));
            if i == range {
                ui.painter().circle_stroke(centre, 13.5, Stroke::new(2.0, t.text));
            }
            let name = tl!(name);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, i == range, name));
            if resp.on_hover_text(name).clicked() {
                range = i;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(range_id, range));
    ui.add_space(4.0);
    let key = SELECTIVE_RANGES[range];
    let arr = v.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut cmyk: [f32; 4] = std::array::from_fn(|i| arr.get(i).and_then(Value::as_f64).filter(|x| x.is_finite()).map_or(0.0, |x| (x as f32).clamp(-100.0, 100.0)));
    let before = cmyk;
    ui.spacing_mut().item_spacing.y = 2.0;
    let tracks = [
        [Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 255)],
        [Color32::from_rgb(0, 255, 0), Color32::from_rgb(255, 0, 255)],
        [Color32::from_rgb(0, 0, 255), Color32::from_rgb(255, 255, 0)],
        [Color32::WHITE, Color32::BLACK],
    ];
    for (i, name) in ["Cyan", "Magenta", "Yellow", "Black"].into_iter().enumerate() {
        let (Some(x), Some(tr)) = (cmyk.get_mut(i), tracks.get(i)) else { continue };
        let grey = Color32::from_gray(128);
        row(ui, label(name), x, -100.0..=100.0, 1.0, "%", Some(&[tr[0], tr[0].lerp_to_gamma(grey, 0.6), tr[1].lerp_to_gamma(grey, 0.6), tr[1]]));
    }
    if cmyk != before || v.get(key).and_then(Value::as_array).is_none() {
        v[key] = json!(cmyk);
        changed = true;
    }
    ui.label(label("Method"));
    let mut method = v.get("method").and_then(Value::as_str).unwrap_or("relative").to_string();
    ui.horizontal(|ui| {
        for (m, text) in [("relative", tl!("Relative")), ("absolute", tl!("Absolute"))] {
            if radio(ui, method == m, text) && method != m {
                method = m.to_string();
                v["method"] = json!(m);
                changed = true;
            }
            ui.add_space(10.0);
        }
    });
    changed
}

// ---------------------------------------------------------------------------------------------
// Color Lookup

/// Photoshop 27.4: "3DLUT File" with its dropdown (Load 3D LUT…, then the looks), Dither.
/// (Abstract and Device Link profiles aren't supported, so they aren't offered.)
fn color_lookup(app: &mut crate::PrismApp, ui: &mut egui::Ui, v: &mut Value, f: &Map<String, Value>) -> bool {
    let mut changed = false;
    // The parameter form's empty placeholders aren't a LUT: a `data` that isn't the file's text
    // would fail the command.
    if v.get("data").is_some_and(|d| !d.is_string())
        && let Some(o) = v.as_object_mut()
    {
        o.remove("data");
        changed = true;
    }
    let builtins = prism_engine::adjust_cmds::LOOKS;
    let current = v.get("lut").and_then(Value::as_str).unwrap_or("none").to_string();
    let loaded = v.get("fileName").and_then(Value::as_str).map(str::to_owned);
    ui.horizontal(|ui| {
        radio(ui, true, label("3DLUT File"));
        ui.add_space(6.0);
        let mut sel = if loaded.is_some() { "file".to_string() } else { current.clone() };
        let mut opts: Vec<(String, &str)> = vec![("load".into(), tl!("Load 3D LUT…"))];
        if let Some(name) = loaded.as_deref() {
            opts.push(("file".into(), name));
        }
        opts.push(("none".into(), tl!("None")));
        opts.extend(builtins.iter().map(|(id, label)| (id.to_string(), *label)));
        let before = sel.clone();
        if widgets::dropdown(ui, "psclrl-lut", &mut sel, &opts, ui.available_width()) && sel != before {
            if sel == "load" {
                let id = f.get("__dialog").and_then(Value::as_u64);
                let _ = app.pick_file_bytes(move |app, name, bytes| {
                    let text = String::from_utf8(bytes).map_err(|_| format!("{name}: not a text LUT file"))?;
                    let target = id.or_else(|| app.ui.dialogs.last().map(|d| d.id));
                    if let Some(d) = target.and_then(|id| app.ui.dialog_mut(id)) {
                        d.fields.insert("data".into(), json!(text));
                        d.fields.insert("fileName".into(), json!(name));
                        d.fields.insert("lut".into(), json!("none"));
                    }
                    Ok(Value::Null)
                });
            } else if sel != "file" {
                v["lut"] = json!(sel);
                if let Some(o) = v.as_object_mut() {
                    o.remove("data");
                    o.remove("fileName");
                    o.remove("file");
                }
                changed = true;
            }
        }
    });
    ui.add_space(10.0);
    let mut dither = v.get("dither").and_then(Value::as_bool).unwrap_or(false);
    if widgets::checkbox(ui, &mut dither, tl!("Dither")).changed() {
        v["dither"] = json!(dither);
        changed = true;
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// Shadows/Highlights

/// Photoshop 27.4 (classic): Shadows and Highlights amounts; "Show More Options" adds each one's
/// Tone and Radius and the Adjustments (Color, Midtone, Black / White Clip), in hairline-separated
/// groups. Fields "35%", "30 px", "+20", "0.01%".
fn shadows_highlights(ui: &mut egui::Ui, v: &mut Value) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let more_id = crate::adjust_dialog::editor_mem("shadowsHighlights").with("more");
    let mut more: bool = ui.data(|d| d.get_temp(more_id)).unwrap_or(false);
    let mut changed = false;
    let hairline = |ui: &mut egui::Ui| {
        ui.add_space(6.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, t.separator));
        ui.add_space(8.0);
    };
    let mut field = |ui: &mut egui::Ui, key: &str, text: &str, d: f32, range: std::ops::RangeInclusive<f32>, step: f32, unit: &str| {
        let mut x = get(v, key, d).clamp(*range.start(), *range.end());
        if classic_row(ui, text, &mut x, range, step, unit) {
            put(v, key, x);
            changed = true;
        }
    };
    ui.spacing_mut().item_spacing.y = 6.0;
    field(ui, "shadowAmount", label("Shadows"), 35.0, 0.0..=100.0, 1.0, "%");
    if more {
        field(ui, "shadowTone", label("Tone"), 50.0, 0.0..=100.0, 1.0, "%");
        field(ui, "shadowRadius", label("Radius"), 30.0, 0.0..=2500.0, 1.0, "px");
        hairline(ui);
    }
    field(ui, "highlightAmount", label("Highlights"), 0.0, 0.0..=100.0, 1.0, "%");
    if more {
        field(ui, "highlightTone", label("Tone"), 50.0, 0.0..=100.0, 1.0, "%");
        field(ui, "highlightRadius", label("Radius"), 30.0, 0.0..=2500.0, 1.0, "px");
        hairline(ui);
        field(ui, "color", label("Color"), 20.0, -100.0..=100.0, 1.0, "");
        field(ui, "midtone", label("Midtone"), 0.0, -100.0..=100.0, 1.0, "");
        field(ui, "blackClip", label("Black Clip"), 0.01, 0.0..=50.0, 0.01, "%");
        field(ui, "whiteClip", label("White Clip"), 0.01, 0.0..=50.0, 0.01, "%");
        hairline(ui);
    }
    ui.add_space(6.0);
    widgets::checkbox(ui, &mut more, label("Show More Options"));
    ui.data_mut(|d| d.insert_temp(more_id, more));
    changed
}

// ---------------------------------------------------------------------------------------------
// HDR Toning

/// Photoshop 27.4 (classic): "Method: Local Adaptation" framing collapsible groups: Edge Glow
/// (Radius, Strength), Tone and Detail (Gamma, Exposure, Detail), Advanced (Shadow, Highlight,
/// Vibrance, Saturation) and Toning Curve and Histogram (collapsed).
fn hdr_toning(app: &mut crate::PrismApp, ui: &mut egui::Ui, v: &mut Value) -> bool {
    let mut changed = false;
    let mem = crate::adjust_dialog::editor_mem("hdrToning");
    ui.horizontal(|ui| {
        ui.label(format!("{}:", label("Method")));
        let mut m = 0usize;
        widgets::dropdown(ui, "pshdr-method", &mut m, &[(0usize, label("Local Adaptation"))], 160.0);
    });
    ui.add_space(4.0);
    let section = |ui: &mut egui::Ui, id: &str, title: &str, open_default: bool, add: &mut dyn FnMut(&mut egui::Ui)| {
        egui::CollapsingHeader::new(title).id_salt(mem.with(id)).default_open(open_default).show(ui, |ui| add(ui));
    };
    let num_row = |ui: &mut egui::Ui, v: &mut Value, key: &str, text: &str, d: f32, range: std::ops::RangeInclusive<f32>, step: f32, unit: &str| -> bool {
        let mut x = get(v, key, d).clamp(*range.start(), *range.end());
        let c = classic_row(ui, text, &mut x, range, step, unit);
        if c {
            put(v, key, x);
        }
        c
    };
    section(ui, "edge", label("Edge Glow"), true, &mut |ui| {
        changed |= num_row(ui, v, "radius", label("Radius:"), 30.0, 1.0..=500.0, 1.0, "px");
        changed |= num_row(ui, v, "strength", label("Strength:"), 0.5, 0.1..=4.0, 0.01, "");
    });
    section(ui, "tone", label("Tone and Detail"), true, &mut |ui| {
        changed |= num_row(ui, v, "gamma", label("Gamma:"), 1.0, 0.1..=2.0, 0.01, "");
        changed |= num_row(ui, v, "exposure", label("Exposure:"), 0.0, -5.0..=5.0, 0.01, "");
        changed |= num_row(ui, v, "detail", label("Detail:"), 30.0, -100.0..=300.0, 1.0, "%");
    });
    section(ui, "advanced", label("Advanced"), true, &mut |ui| {
        changed |= num_row(ui, v, "shadow", label("Shadow:"), 0.0, -100.0..=100.0, 1.0, "%");
        changed |= num_row(ui, v, "highlight", label("Highlight:"), 0.0, -100.0..=100.0, 1.0, "%");
        changed |= num_row(ui, v, "vibrance", label("Vibrance:"), 0.0, -100.0..=100.0, 1.0, "%");
        changed |= num_row(ui, v, "saturation", label("Saturation:"), 20.0, -100.0..=100.0, 1.0, "%");
    });
    section(ui, "curve", label("Toning Curve and Histogram"), false, &mut |ui| {
        // The curve edits like Curves' graph: the same widget over the curve's points.
        let mut temp = json!({"points": v.get("curve").cloned().unwrap_or(json!([[0, 0], [255, 255]]))});
        let cx = crate::adjust_editors::EditorCx {
            mem: mem.with("curve-graph"),
            hist: app.session.active().and_then(|s| s.active_layer).map(|id| crate::tone::histograms(app, crate::tone::HistSource::Layer(id), prism_doc::adjust::ToneSpace::Rgb)),
            gray: true,
            swatches: crate::adjust_editors::swatches(app),
            gradients: Vec::new(),
            dialog: true,
        };
        let view = crate::adjust_editors::CurvesView::default_for(prism_doc::adjust::ToneSpace::Rgb);
        if crate::adjust_editors::curves_graph(ui, &mut temp, &cx, 0, &view).changed {
            v["curve"] = temp["points"].clone();
            changed = true;
        }
    });
    changed
}

// ---------------------------------------------------------------------------------------------
// Match Color

/// Photoshop 27.4 (classic): "Destination Image" (Target: the document and its mode), "Image
/// Options" (Luminance, Color Intensity, Fade, Neutralize), "Image Statistics" (Use Selection in
/// Source / Target, Source: None or an open document, Layer).
fn match_color(app: &mut crate::PrismApp, ui: &mut egui::Ui, v: &mut Value) -> bool {
    let mut changed = false;
    let target = app.session.active().map(|s| format!("{} ({}/{})", s.doc.name, crate::canvas::mode_label(&s.doc), s.doc.depth.bits()));
    group(ui, label("Destination Image"), |ui| {
        ui.label(format!("{}  {}", label("Target:"), target.unwrap_or_default()));
    });
    ui.add_space(8.0);
    group(ui, label("Image Options"), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (key, text, d, lo, hi) in [("luminance", "Luminance", 100.0, 1.0, 200.0), ("intensity", "Color Intensity", 100.0, 1.0, 200.0), ("fade", "Fade", 0.0, 0.0, 100.0)] {
            let mut x = get(v, key, d).clamp(lo, hi);
            if row(ui, label(text), &mut x, lo..=hi, 1.0, "", None) {
                put(v, key, x);
                changed = true;
            }
        }
        let mut n = v.get("neutralize").and_then(Value::as_bool).unwrap_or(false);
        if widgets::checkbox(ui, &mut n, label("Neutralize")).changed() {
            v["neutralize"] = json!(n);
            changed = true;
        }
    });
    ui.add_space(8.0);
    group(ui, label("Image Statistics"), |ui| {
        for (key, text) in [("useSelectionInSource", "Use Selection in Source to Calculate Colors"), ("useSelectionInTarget", "Use Selection in Target to Calculate Adjustment")] {
            let mut b = v.get(key).and_then(Value::as_bool).unwrap_or(false);
            if widgets::checkbox(ui, &mut b, label(text)).changed() {
                v[key] = json!(b);
                changed = true;
            }
        }
        ui.add_space(4.0);
        let docs: Vec<(i64, String)> = app.session.documents().iter().enumerate().map(|(i, d)| (i as i64, d.doc.name.clone())).collect();
        let mut source = v.get("source").and_then(Value::as_i64).filter(|i| *i >= 0 && (*i as usize) < docs.len()).unwrap_or(-1);
        ui.horizontal(|ui| {
            ui.label(format!("{}:", label("Source")));
            let mut opts: Vec<(i64, &str)> = vec![(-1, tl!("None"))];
            opts.extend(docs.iter().map(|(i, n)| (*i, n.as_str())));
            let before = source;
            if widgets::dropdown(ui, "psmc-source", &mut source, &opts, 180.0) && source != before {
                v["source"] = json!(source);
                if let Some(o) = v.as_object_mut() {
                    o.remove("sourceLayer");
                }
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(format!("{}:", label("Layer")));
            let layers: Vec<(i64, String)> = usize::try_from(source)
                .ok()
                .and_then(|i| app.session.documents().get(i))
                .map(|d| d.doc.layers.iter().map(|l| (l.id.0 as i64, l.name.clone())).collect())
                .unwrap_or_default();
            let mut opts: Vec<(i64, &str)> = vec![(-1, tl!("Merged"))];
            opts.extend(layers.iter().map(|(i, n)| (*i, n.as_str())));
            let mut layer = v.get("sourceLayer").and_then(Value::as_i64).unwrap_or(-1);
            let before = layer;
            ui.add_enabled_ui(source >= 0, |ui| {
                if widgets::dropdown(ui, "psmc-layer", &mut layer, &opts, 180.0) && layer != before {
                    if layer < 0 {
                        if let Some(o) = v.as_object_mut() {
                            o.remove("sourceLayer");
                        }
                    } else {
                        v["sourceLayer"] = json!(layer);
                    }
                    changed = true;
                }
            });
        });
    });
    changed
}

// ---------------------------------------------------------------------------------------------
// Replace Color

/// The Replace Color eyedropper armed in the dialog (`__rcPicker`: "sample").
pub(crate) const RC_PICKER: &str = "__rcPicker";

fn hex_rgb(v: &Value, key: &str, default: [f32; 3]) -> [f32; 3] {
    let h = v.get(key).and_then(Value::as_str).unwrap_or_default().trim_start_matches('#').to_string();
    let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|x| f32::from(x) / 255.0);
    match (b(0), b(2), b(4)) {
        (Some(r), Some(g), Some(bb)) => [r, g, bb],
        _ => default,
    }
}

fn swatch(ui: &mut egui::Ui, c: [f32; 3], size: egui::Vec2) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    ui.painter().rect_filled(r, 2.0, Color32::from_rgb(b(c[0]), b(c[1]), b(c[2])));
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, Color32::from_gray(40)), StrokeKind::Inside);
    resp
}

/// Photoshop 27.4 (classic): the Eyedropper and "Color:" swatch, Fuzziness, the Selection / Image
/// preview, then the replacement Hue / Saturation / Lightness with the Result swatch. (Add to /
/// Subtract from Sample and Localized Color Clusters aren't modelled: one colour picks the area.)
fn replace_color(app: &mut crate::PrismApp, ui: &mut egui::Ui, v: &mut Value, f: &mut Map<String, Value>) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut changed = false;
    let fg = app.session.tools.foreground;
    let mut colour = hex_rgb(v, "color", [fg[0], fg[1], fg[2]]);
    let armed = f.get(RC_PICKER).and_then(Value::as_str) == Some("sample");
    ui.horizontal(|ui| {
        let r = crate::icons::button(ui, "pipette", 26.0, armed, tl!("Eyedropper Tool"));
        let name = tl!("Eyedropper Tool");
        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, armed, name));
        if r.clicked() {
            if armed {
                f.remove(RC_PICKER);
            } else {
                f.insert(RC_PICKER.into(), json!("sample"));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut srgb = colour.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
            if widgets::color_edit_button_srgb(ui, &mut srgb).changed() {
                colour = srgb.map(|x| f32::from(x) / 255.0);
                v["color"] = json!(prism_engine::adjust_params::hex(colour));
                changed = true;
            }
            ui.label(format!("{}:", label("Color")));
        });
    });
    ui.add_space(4.0);
    let mut fuzz = get(v, "fuzziness", 40.0).clamp(0.0, 200.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    if row(ui, label("Fuzziness:"), &mut fuzz, 0.0..=200.0, 1.0, "", None) {
        put(v, "fuzziness", fuzz);
        changed = true;
    }
    // The preview: the selected area (white) or the image.
    let show_id = crate::adjust_dialog::editor_mem("replaceColor").with("image");
    let mut show_image: bool = ui.data(|d| d.get_temp(show_id)).unwrap_or(false);
    let w = ui.available_width() - 40.0;
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), w * 0.62), Sense::hover());
    let area = Rect::from_center_size(r.center(), vec2(w, w * 0.62));
    if let Some(tex) = replace_preview(app, ui.ctx(), colour, fuzz, show_image) {
        let [tw, th] = tex.size();
        let k = (area.width() / tw as f32).min(area.height() / th as f32);
        let img = Rect::from_center_size(area.center(), vec2(tw as f32 * k, th as f32 * k));
        ui.painter().image(tex.id(), img, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        ui.painter().rect_filled(area, 0.0, t.field);
    }
    ui.horizontal(|ui| {
        ui.add_space(30.0);
        if radio(ui, !show_image, label("Selection")) {
            show_image = false;
        }
        ui.add_space(40.0);
        if radio(ui, show_image, label("Image")) {
            show_image = true;
        }
    });
    ui.data_mut(|d| d.insert_temp(show_id, show_image));
    ui.add_space(6.0);
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0, t.separator));
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - 48.0);
            for (key, text, lo) in [("hue", "Hue:", -180.0), ("saturation", "Saturation:", -100.0), ("lightness", "Lightness:", -100.0)] {
                let hi = -lo;
                let mut x = get(v, key, 0.0).clamp(lo, hi);
                if classic_row(ui, label(text), &mut x, lo..=hi, 1.0, "") {
                    put(v, key, x);
                    changed = true;
                }
            }
        });
        ui.vertical(|ui| {
            // The colour after the replacement.
            let mut px = [[colour[0], colour[1], colour[2], 1.0]];
            prism_algo::tone::replace_color(&mut px, colour, fuzz.max(1.0), get(v, "hue", 0.0), get(v, "saturation", 0.0), get(v, "lightness", 0.0));
            swatch(ui, [px[0][0], px[0][1], px[0][2]], vec2(40.0, 40.0));
            ui.label(label("Result"));
        });
    });
    if v.get("color").and_then(Value::as_str).is_none() {
        v["color"] = json!(prism_engine::adjust_params::hex(colour));
        changed = true;
    }
    changed
}

/// The Replace Color preview, cached per document revision and settings: the area the colour
/// picks (white = fully picked) or the image, at most 240 px on its long side.
fn replace_preview(app: &crate::PrismApp, ctx: &egui::Context, colour: [f32; 3], fuzz: f32, image: bool) -> Option<egui::TextureHandle> {
    let s = app.session.active()?;
    let key = egui::Id::new(("rc-preview", s.doc.id.0, s.revision, colour.map(f32::to_bits), fuzz.to_bits(), image));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(t);
    }
    let flat = prism_compose::flatten(&s.doc);
    let (w, h) = (flat.rect.width().max(1) as usize, flat.rect.height().max(1) as usize);
    let step = (w.max(h) / 240).max(1);
    let (pw, ph) = (w.div_ceil(step), h.div_ceil(step));
    let mut px: Vec<[f32; 4]> = Vec::with_capacity(pw * ph);
    for y in 0..ph {
        for x in 0..pw {
            px.push(flat.px.get((y * step).min(h - 1) * w + (x * step).min(w - 1)).copied().unwrap_or([0.0; 4]));
        }
    }
    let rgba: Vec<u8> = if image {
        px.iter().flat_map(|p| [p[0], p[1], p[2], 1.0].map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)).collect()
    } else {
        let mask = prism_algo::selection::color_range(&px, colour, fuzz);
        mask.iter().flat_map(|m| {
            let g = (m.clamp(0.0, 1.0) * 255.0).round() as u8;
            [g, g, g, 255]
        })
        .collect()
    };
    let img = egui::ColorImage::from_rgba_unmultiplied([pw, ph], &rgba);
    let tex = ctx.load_texture("rc-preview", img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
    Some(tex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_width_and_defaults() {
        let app = crate::PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        for k in ["posterize", "threshold", "gradientMap", "selectiveColor", "colorLookup", "shadowsHighlights", "hdrToning", "matchColor", "replaceColor"] {
            assert!(owns(k));
            assert!(width(k) > 300.0, "{k}");
            assert!(defaults(k, &app).is_object(), "{k}");
        }
        assert_eq!(plain(4.0), "4");
    }
}

#[cfg(test)]
mod dialog_tests {
    use crate::PrismApp;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::{Value, json};

    fn open(cmd: &str) -> (Harness<'static, PrismApp>, u64) {
        let mut s = prism_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("edit.fill", json!({"color": "#806040"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 48})).unwrap();
        s.execute("edit.fill", json!({"color": "#2050c0"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 1000.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), PrismApp::new(s, crate::Services::default()));
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = crate::adjust_dialog::open(h.state_mut(), cmd).or_else(|| crate::filter_dialog::open(h.state_mut(), cmd)).unwrap();
        h.run_steps(4);
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

    /// Each of the nine opens in Photoshop's layout with its own controls, applies once on OK
    /// and keeps the document on Cancel.
    #[test]
    fn the_rest_of_image_adjustments_open_in_photoshops_layout() {
        for (cmd, labels) in [
            ("image.adjustments.posterize", &["Levels", "OK", "Cancel"][..]),
            ("image.adjustments.threshold", &["Threshold Level:", "OK"][..]),
            ("image.adjustments.gradientMap", &["Gradient Used for Grayscale Mapping", "Gradient Options", "Dither", "Reverse"][..]),
            ("image.adjustments.selectiveColor", &["Reds", "Neutrals", "Blacks", "Cyan", "Magenta", "Yellow", "Black", "Relative", "Absolute"][..]),
            ("image.adjustments.colorLookup", &["3DLUT File", "Dither"][..]),
            ("image.adjustments.shadowsHighlights", &["Shadows", "Highlights", "Show More Options"][..]),
            ("image.adjustments.hdrToning", &["Edge Glow", "Tone and Detail", "Advanced", "Toning Curve and Histogram", "Radius:", "Saturation:"][..]),
            ("image.adjustments.matchColor", &["Destination Image", "Image Options", "Luminance", "Color Intensity", "Fade", "Neutralize", "Image Statistics"][..]),
            ("image.adjustments.replaceColor", &["Fuzziness:", "Selection", "Image", "Hue:", "Saturation:", "Lightness:", "Result"][..]),
        ] {
            let (mut h, _) = open(cmd);
            assert!(crate::adjust_ps_ui::owns(&h.state().ui.dialogs[0].fields), "{cmd}");
            for l in labels {
                assert!(h.query_all_by_label(l).next().is_some(), "{cmd}: {l}");
            }
            let before = h.state().session.active().unwrap().revision;
            click(&mut h, "Cancel", egui::Modifiers::NONE);
            assert!(h.state().ui.dialogs.is_empty(), "{cmd}: Cancel closes");
            assert_eq!(h.state().session.active().unwrap().revision, before, "{cmd}: Cancel leaves the document");
            let (mut h, id) = open(cmd);
            if cmd.ends_with("colorLookup") {
                // No look picked does nothing (as in Photoshop): pick one.
                h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("lut".into(), json!("warm"));
                h.run_steps(2);
            }
            let steps = h.state().session.active().unwrap().history.past_len();
            click(&mut h, "OK", egui::Modifiers::NONE);
            assert!(h.state().ui.dialogs.is_empty(), "{cmd}: OK closes");
            assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "{cmd}: one history step ({})", h.state().ui.status);
        }
    }

    /// Shadows/Highlights: Show More Options adds Tone / Radius and the Adjustments rows.
    #[test]
    fn shadows_highlights_shows_more_options() {
        let (mut h, _) = open("image.adjustments.shadowsHighlights");
        assert!(h.query_by_label("Black Clip").is_none());
        click(&mut h, "Show More Options", egui::Modifiers::NONE);
        for l in ["Black Clip", "White Clip", "Midtone", "Color"] {
            assert!(h.query_all_by_label(l).next().is_some(), "{l}");
        }
    }

    /// Selective Color: a swatch picks the range the rows edit; Absolute is a method.
    #[test]
    fn selective_color_swatches_pick_the_range() {
        let (mut h, id) = open("image.adjustments.selectiveColor");
        click(&mut h, "Blues", egui::Modifiers::NONE);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("blues".into(), json!([10, 0, -20, 5]));
        h.run_steps(3);
        click(&mut h, "Absolute", egui::Modifiers::NONE);
        assert_eq!(field(&h, id, "method"), json!("absolute"));
        assert_eq!(field(&h, id, "blues"), json!([10, 0, -20, 5]));
        assert!(field(&h, id, "colors").is_null(), "the single-range keys are folded away");
    }

    /// A note after the parameter list doesn't eat the last parameter's default: Replace Color
    /// opens with lightness 0, not -100.
    #[test]
    fn replace_color_opens_at_its_defaults() {
        let (h, id) = open("image.adjustments.replaceColor");
        assert_eq!(field(&h, id, "lightness").as_f64(), Some(0.0));
        let p = crate::filter_dialog::parse_spec(r#"{"a":0..10=3,"b":-100..100=0} (note: "x", {y})"#);
        assert_eq!(p.len(), 2);
        assert!(matches!(p[1].kind, crate::filter_dialog::Kind::Range { default, .. } if default == 0.0));
    }

    /// Replace Color's eyedropper takes the canvas colour.
    #[test]
    fn replace_color_eyedropper_takes_the_sample() {
        let (mut h, id) = open("image.adjustments.replaceColor");
        click(&mut h, "Eyedropper Tool", egui::Modifiers::NONE);
        assert!(crate::adjust_dialog::picker_armed(h.state()));
        assert!(crate::adjust_dialog::sample_at(h.state_mut(), 5.0, 5.0));
        h.run_steps(2);
        assert_eq!(field(&h, id, "color"), json!("#2050c0"));
    }

    /// Run explicitly for visual QA: each of the nine, in the dark theme.
    #[test]
    #[ignore = "writes target/adjust-more/*.png for visual inspection"]
    fn write_more_dialog_sheet() {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/adjust-more");
        std::fs::create_dir_all(&out).unwrap();
        for cmd in [
            "posterize",
            "threshold",
            "gradientMap",
            "selectiveColor",
            "colorLookup",
            "shadowsHighlights",
            "hdrToning",
            "matchColor",
            "replaceColor",
        ] {
            let mut s = prism_engine::Session::new();
            s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
            s.execute("edit.fill", json!({"color": "#806040"})).unwrap();
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 48})).unwrap();
            s.execute("edit.fill", json!({"color": "#2050c0"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            let mut h = Harness::builder()
                .with_size(egui::vec2(640.0, 760.0))
                .with_pixels_per_point(2.0)
                .wgpu()
                .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), PrismApp::new(s, crate::Services::default()));
            PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
            let c = format!("image.adjustments.{cmd}");
            crate::adjust_dialog::open(h.state_mut(), &c).or_else(|| crate::filter_dialog::open(h.state_mut(), &c)).unwrap();
            h.run_steps(6);
            h.render().unwrap().save(out.join(format!("{cmd}.png"))).unwrap();
        }
    }
}
