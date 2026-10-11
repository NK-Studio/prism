//! Photoshop-style Image Size and Canvas Size dialogs.
//!
//! Field values are stored in pixels (plus a display unit), so automation can set `width`/`height`
//! directly and the dialog maps them to `image.imageSize` / `image.canvasSize` unchanged.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::state::DialogKind;
use crate::theme::Tokens;

const UNITS: [(&str, &str); 7] =
    [("px", "Pixels"), ("percent", "Percent"), ("in", "Inches"), ("cm", "Centimeters"), ("mm", "Millimeters"), ("pt", "Points"), ("pica", "Picas")];

/// Dialog unit for the Units & Rulers preference.
fn pref_unit(u: prism_engine::prefs::Unit) -> &'static str {
    use prism_engine::prefs::Unit;
    match u {
        Unit::Pixels => "px",
        Unit::Percent => "percent",
        Unit::Inches => "in",
        Unit::Centimeters => "cm",
        Unit::Millimeters => "mm",
        Unit::Points => "pt",
        Unit::Picas => "pica",
    }
}

/// Image Size resampling for Preferences › General › Image Interpolation.
fn pref_resample(i: prism_engine::prefs::Interpolation) -> &'static str {
    use prism_engine::prefs::Interpolation;
    match i {
        Interpolation::BicubicAutomatic => "auto",
        Interpolation::Nearest => "nearest",
        Interpolation::Bilinear => "bilinear",
        Interpolation::BicubicSharper => "lanczos",
        Interpolation::PreserveDetails => "preserveDetails",
        _ => "bicubic",
    }
}
const ANCHORS: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];

pub fn is_sizing(command: &str) -> bool {
    matches!(command, "image.imageSize" | "image.canvasSize")
}

pub fn open(app: &mut PrismApp, command: &str) -> Option<u64> {
    let unit = pref_unit(app.session.prefs().units_and_rulers.rulers);
    let resample = pref_resample(app.session.prefs().general.image_interpolation);
    let d = &app.session.active()?.doc;
    let (w, h, res) = (d.size.width as f64, d.size.height as f64, d.resolution_dpi as f64);
    let label = prism_engine::commands::find(command).map(|c| c.label).unwrap_or("Image Size…");
    let mut f = Map::new();
    f.insert("__command".into(), json!(command));
    f.insert("__label".into(), json!(label));
    f.insert("__sizing".into(), json!(true));
    f.insert("__origW".into(), json!(w));
    f.insert("__origH".into(), json!(h));
    f.insert("__origRes".into(), json!(res));
    f.insert("__unit".into(), json!(unit));
    f.insert("__widthUnit".into(), json!(unit));
    f.insert("__heightUnit".into(), json!(unit));
    f.insert("__resolutionUnit".into(), json!("ppi"));
    f.insert("__sourceDoc".into(), json!(d.id.0));
    let prefs = &app.session.prefs().units_and_rulers;
    f.insert("__columnWidth".into(), json!(prefs.column_width));
    f.insert("__gutter".into(), json!(prefs.gutter));
    f.insert("__pointsPerInch".into(), json!(prefs.point_size.per_inch()));
    f.insert("__background".into(), json!(d.layers.first().is_some_and(|l| l.name == "Background" && l.locks.transparency && l.surface().is_some())));
    f.insert("__bytesPerPixel".into(), json!(d.mode.color_channels() * d.depth.bytes()));
    f.insert("width".into(), json!(w));
    f.insert("height".into(), json!(h));
    if command == "image.imageSize" {
        f.insert("__constrain".into(), json!(true));
        f.insert("resolution".into(), json!(res));
        f.insert("resample".into(), json!(resample));
        f.insert("scaleStyles".into(), json!(true));
        f.insert("__fit".into(), json!("original"));
    } else {
        f.insert("relative".into(), json!(false));
        f.insert("anchor".into(), json!("center"));
        f.insert("extensionColor".into(), json!("background"));
    }
    if command == "image.canvasSize" && f.get("__background").and_then(Value::as_bool) != Some(true) {
        f.insert("extensionColor".into(), json!("transparent"));
    }
    let id = app.ui.open_dialog(DialogKind::Command, f);
    crate::native_dialogs::make_native(app, id);
    Some(id)
}

fn num(f: &Map<String, Value>, k: &str) -> f64 {
    f.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Pixels → display unit.
fn to_unit(px: f64, unit: &str, orig: f64, res: f64) -> f64 {
    match unit {
        "percent" => px / orig.max(1.0) * 100.0,
        "in" => px / res.max(1.0),
        "cm" => px / res.max(1.0) * 2.54,
        "mm" => px / res.max(1.0) * 25.4,
        "pt" => px / res.max(1.0) * 72.0,
        "pica" => px / res.max(1.0) * 6.0,
        _ => px,
    }
}

fn from_unit(v: f64, unit: &str, orig: f64, res: f64) -> f64 {
    match unit {
        "percent" => v * orig / 100.0,
        "in" => v * res,
        "cm" => v / 2.54 * res,
        "mm" => v / 25.4 * res,
        "pt" => v / 72.0 * res,
        "pica" => v / 6.0 * res,
        _ => v,
    }
}

pub fn human_bytes(b: f64) -> String {
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2}G", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1}M", b / (1024.0 * 1024.0))
    } else {
        format!("{:.0}K", (b / 1024.0).max(1.0))
    }
}

pub fn body(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if f.get("__command").and_then(Value::as_str) == Some("image.canvasSize") {
        canvas_size(app, ui, f);
    } else {
        if ui.available_width() >= 700.0 {
            let height = ui.available_height().clamp(180.0, 390.0);
            ui.horizontal_top(|ui| {
                let preview_width = (ui.available_width() - 412.0).max(250.0);
                ui.allocate_ui_with_layout(vec2(preview_width, height), egui::Layout::top_down(egui::Align::Min), |ui| preview(app, ui, f, height - 32.0));
                ui.allocate_ui_with_layout(vec2(400.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    image_size(ui, f);
                });
            });
        } else {
            image_size(ui, f);
        }
    }
}

fn label(ui: &mut egui::Ui, text: &str, width: f32) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    ui.painter().text(pos2(r.right() - 6.0, r.center().y), Align2::RIGHT_CENTER, text, egui::FontId::proportional(12.0), t.text_dim);
}

fn info_row(ui: &mut egui::Ui, k: &str, v: String) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        label(ui, tl!(k), 90.0);
        ui.label(egui::RichText::new(v).color(t.text).size(12.0));
    });
}

/// Width / height field in the current unit, writing back pixels. Returns true when edited.
fn unit_key(key: &str) -> &'static str {
    if key == "width" { "__widthUnit" } else { "__heightUnit" }
}
fn column_span(f: &Map<String, Value>, n: f64) -> f64 {
    let width = num(f, "__columnWidth").max(0.01);
    let gutter = num(f, "__gutter").max(0.0);
    n.signum() * (n.abs() * width + (n.abs() - 1.0).max(0.0) * gutter)
}
fn display_value(f: &Map<String, Value>, px: f64, unit: &str, orig: f64, res: f64) -> f64 {
    if unit == "columns" {
        let points = px.abs() / res.max(1.0) * 72.0;
        let width = num(f, "__columnWidth").max(0.01);
        let gutter = num(f, "__gutter").max(0.0);
        px.signum() * if points <= width { points / width } else { (points + gutter) / (width + gutter) }
    } else if unit == "pt" || unit == "pica" {
        px / res.max(1.0) * num(f, "__pointsPerInch").max(1.0) / if unit == "pica" { 12.0 } else { 1.0 }
    } else {
        to_unit(px, unit, orig, res)
    }
}
fn pixel_value(f: &Map<String, Value>, value: f64, unit: &str, orig: f64, res: f64) -> f64 {
    if unit == "columns" {
        column_span(f, value) / 72.0 * res
    } else if unit == "pt" || unit == "pica" {
        value / num(f, "__pointsPerInch").max(1.0) * res * if unit == "pica" { 12.0 } else { 1.0 }
    } else {
        from_unit(value, unit, orig, res)
    }
}
fn dim_field(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, orig_key: &str, id: &str) -> bool {
    let unit = f.get(unit_key(key)).or(f.get("__unit")).and_then(Value::as_str).unwrap_or("px").to_string();
    let res = f.get("resolution").and_then(Value::as_f64).unwrap_or(num(f, "__origRes"));
    let orig = num(f, orig_key);
    let frozen = f.get("resample").and_then(Value::as_str) == Some("none");
    let mut v = display_value(f, num(f, key), &unit, orig, res) as f32;
    let changed = ui
        .push_id(id, |ui| {
            ui.add_enabled_ui(!frozen || !matches!(unit.as_str(), "px" | "percent"), |ui| {
                crate::widgets::value_field(ui, &mut v, -300000.0..=300000.0, "", 100.0).changed()
            })
            .inner
        })
        .inner;
    if changed {
        let px = pixel_value(f, f64::from(v), &unit, orig, res);
        if frozen {
            if px > 0.0 {
                f.insert("resolution".into(), json!((res * orig / px).clamp(1.0, 10000.0)));
            }
            f.insert("width".into(), json!(num(f, "__origW")));
            f.insert("height".into(), json!(num(f, "__origH")));
        } else {
            f.insert(key.into(), json!(if unit == "px" { px.round() } else { px }));
        }
        f.insert("__fit".into(), json!("custom"));
    }
    changed
}
fn unit_dropdown(ui: &mut egui::Ui, f: &mut Map<String, Value>, id: &str, key: &str) {
    let mut unit = f.get(unit_key(key)).or(f.get("__unit")).and_then(Value::as_str).unwrap_or("px").to_string();
    let mut opts: Vec<(String, &str)> = UNITS.iter().map(|(k, l)| (k.to_string(), *l)).collect();
    if key == "width" {
        opts.push(("columns".into(), "Columns"));
    }
    if crate::widgets::dropdown(ui, id, &mut unit, &opts, 145.0) {
        f.insert(unit_key(key).into(), json!(unit));
    }
}
pub fn validate(f: &Map<String, Value>) -> Result<(), String> {
    let relative = f.get("relative").and_then(Value::as_bool).unwrap_or(false);
    for (key, orig) in [("width", "__origW"), ("height", "__origH")] {
        let value = num(f, key) + if relative { num(f, orig) } else { 0.0 };
        if !value.is_finite() || !(1.0..=300000.0).contains(&value) {
            return Err(format!("invalid {key}: choose 1–300000 pixels"));
        }
    }
    if let Some(r) = f.get("resolution")
        && !r.as_f64().is_some_and(|r| r.is_finite() && (1.0..=10000.0).contains(&r))
    {
        return Err("invalid resolution: choose 1–10000 ppi".into());
    }
    Ok(())
}

fn image_size(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let (ow, oh) = (num(f, "__origW"), num(f, "__origH"));
    let bpp = num(f, "__bytesPerPixel").max(1.0);
    let resample = f.get("resample").and_then(Value::as_str).unwrap_or("bicubic").to_string();
    let resampling = resample != "none";
    let (w, h) = (num(f, "width").round(), num(f, "height").round());
    info_row(ui, tl!("Image Size:"), crate::i18n::fmt(tl!("{size} (was {old})"), &[("size", &human_bytes(w * h * bpp)), ("old", &human_bytes(ow * oh * bpp))]));
    info_row(ui, tl!("Dimensions:"), format!("{} px × {} px", w as i64, h as i64));
    ui.horizontal(|ui| {
        label(ui, tl!("Fit To:"), 90.0);
        let mut current = f.get("__fit").and_then(Value::as_str).unwrap_or("original").to_string();
        let mut presets = vec![("original".to_string(), tl!("Original Size")), ("custom".to_string(), tl!("Custom"))];
        for (category, items) in crate::new_doc_ui::CATEGORIES {
            if matches!(*category, "Print" | "Web") {
                for p in *items {
                    presets.push((p.0.to_string(), p.0));
                }
            }
        }
        if crate::widgets::dropdown(ui, "image-size-fit", &mut current, &presets, 240.0) {
            apply_fit(f, &current);
        }
        egui::containers::menu::MenuButton::new("⚙").ui(ui, |ui| {
            let mut enabled = f.get("scaleStyles").and_then(Value::as_bool).unwrap_or(true);
            if crate::widgets::checkbox(ui, &mut enabled, tl!("Scale Styles")).changed() {
                f.insert("scaleStyles".into(), json!(enabled));
            }
        });
    });
    ui.add_space(6.0);
    let mut constrain = f.get("__constrain").and_then(Value::as_bool).unwrap_or(true);
    let w_row = ui.horizontal(|ui| {
        label(ui, tl!("Width:"), 90.0);
        let c = dim_field(ui, f, "width", "__origW", "is-w");
        unit_dropdown(ui, f, "is-unit-w", "width");
        c
    });
    let h_row = ui.horizontal(|ui| {
        label(ui, tl!("Height:"), 90.0);
        let c = dim_field(ui, f, "height", "__origH", "is-h");
        unit_dropdown(ui, f, "is-unit-h", "height");
        c
    });
    let (w_changed, h_changed) = (w_row.inner, h_row.inner);
    // `dim_field` allows negatives for Canvas Size's relative mode, but an image is at least
    // one pixel: clamp here so the preview shows the size `image.imageSize` will apply.
    for (key, changed) in [("width", w_changed), ("height", h_changed)] {
        if changed && num(f, key) < 1.0 {
            f.insert(key.into(), json!(1.0));
        }
    }
    // Photoshop's chain bracket linking width and height (right of the unit dropdowns).
    let x0 = w_row.response.rect.right().max(h_row.response.rect.right()) + 4.0;
    let bracket = Rect::from_min_max(pos2(x0, w_row.response.rect.center().y), pos2(x0 + 14.0, h_row.response.rect.center().y));
    let link = ui.interact(bracket.expand(2.0), ui.id().with("constrain"), Sense::click());
    let col = if constrain { t.text } else { t.text_faint };
    if constrain {
        let p = ui.painter();
        let x = bracket.left() + 8.0;
        p.line_segment([pos2(bracket.left(), bracket.top()), pos2(x, bracket.top())], Stroke::new(1.0, col));
        p.line_segment([pos2(x, bracket.top()), pos2(x, bracket.center().y - 8.0)], Stroke::new(1.0, col));
        p.line_segment([pos2(x, bracket.center().y + 8.0), pos2(x, bracket.bottom())], Stroke::new(1.0, col));
        p.line_segment([pos2(x, bracket.bottom()), pos2(bracket.left(), bracket.bottom())], Stroke::new(1.0, col));
    }
    let chain = Rect::from_center_size(pos2(bracket.left() + 8.0, bracket.center().y), vec2(14.0, 14.0));
    ui.painter().rect_filled(chain, 2.0, if link.hovered() { t.hover } else { Color32::TRANSPARENT });
    crate::icons::paint(ui, chain, if constrain { "link" } else { "unlink" }, 11.0, col);
    if link.on_hover_text(tl!("Constrain proportions")).clicked() {
        constrain = !constrain;
        f.insert("__constrain".into(), json!(constrain));
    }
    if constrain && resampling {
        if w_changed {
            f.insert("height".into(), json!((num(f, "width") * oh / ow.max(1.0)).round().max(1.0)));
        } else if h_changed {
            f.insert("width".into(), json!((num(f, "height") * ow / oh.max(1.0)).round().max(1.0)));
        }
    }
    ui.horizontal(|ui| {
        label(ui, tl!("Resolution:"), 90.0);
        let mut res_unit = f.get("__resolutionUnit").and_then(Value::as_str).unwrap_or("ppi").to_string();
        let divisor = if res_unit == "ppcm" { 2.54 } else { 1.0 };
        let mut r = (f.get("resolution").and_then(Value::as_f64).unwrap_or(72.0) / divisor) as f32;
        if crate::widgets::value_field(ui, &mut r, 1.0..=10000.0, "", 90.0).changed() {
            let r = (r as f64 * divisor).clamp(1.0, 10000.0);
            if !resampling {
                // Without resampling the pixel count is fixed: physical size follows resolution.
                f.insert("width".into(), json!(ow));
                f.insert("height".into(), json!(oh));
            }
            f.insert("resolution".into(), json!(r));
        }
        if crate::widgets::dropdown(
            ui,
            "image-size-resolution-unit",
            &mut res_unit,
            &[("ppi".into(), tl!("Pixels/Inch")), ("ppcm".into(), tl!("Pixels/Centimeter"))],
            145.0,
        ) {
            f.insert("__resolutionUnit".into(), json!(res_unit));
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        label(ui, "", 90.0);
        let mut on = resampling;
        if crate::widgets::checkbox(ui, &mut on, tl!("Resample")).changed() {
            f.insert("resample".into(), json!(if on { "auto" } else { "none" }));
            if !on {
                f.insert("width".into(), json!(ow));
                f.insert("height".into(), json!(oh));
            }
        }
        if resampling {
            let mut cur = resample.clone();
            let opts = [
                ("auto".to_string(), tl!("Automatic")),
                ("preserveDetails".to_string(), tl!("Preserve Details")),
                ("bicubic".to_string(), tl!("Bicubic (smooth gradients)")),
                ("lanczos".to_string(), tl!("Lanczos (sharp)")),
                ("bilinear".to_string(), tl!("Bilinear")),
                ("nearest".to_string(), tl!("Nearest Neighbor (hard edges)")),
            ];
            if crate::widgets::dropdown(ui, "is-resample", &mut cur, &opts, 200.0) {
                f.insert("resample".into(), json!(cur));
            }
        }
    });
}

fn canvas_size(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let (ow, oh) = (num(f, "__origW"), num(f, "__origH"));
    let bpp = num(f, "__bytesPerPixel").max(1.0);
    let relative = f.get("relative").and_then(Value::as_bool).unwrap_or(false);
    ui.label(egui::RichText::new(tl!("Current Size")).font(crate::theme::semibold(12.0)).color(t.text));
    info_row(ui, tl!("Size:"), human_bytes(ow * oh * bpp));
    info_row(ui, tl!("Width:"), format!("{} px", ow as i64));
    info_row(ui, tl!("Height:"), format!("{} px", oh as i64));
    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    ui.add_space(6.0);
    let (nw, nh) = if relative { (ow + num(f, "width"), oh + num(f, "height")) } else { (num(f, "width"), num(f, "height")) };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("New Size")).font(crate::theme::semibold(12.0)).color(t.text));
        ui.label(egui::RichText::new(human_bytes(nw.max(0.0) * nh.max(0.0) * bpp)).color(t.text_dim).size(12.0));
    });
    ui.horizontal(|ui| {
        label(ui, tl!("Width:"), 90.0);
        dim_field(ui, f, "width", "__origW", "cs-w");
        unit_dropdown(ui, f, "cs-unit-w", "width");
    });
    ui.horizontal(|ui| {
        label(ui, tl!("Height:"), 90.0);
        dim_field(ui, f, "height", "__origH", "cs-h");
        unit_dropdown(ui, f, "cs-unit-h", "height");
    });
    ui.horizontal(|ui| {
        label(ui, "", 90.0);
        let mut rel = relative;
        if crate::widgets::checkbox(ui, &mut rel, tl!("Relative")).changed() {
            // Convert between absolute and delta so the resulting canvas is unchanged.
            let (w, h) = (num(f, "width"), num(f, "height"));
            let (w, h) = if rel { (w - ow, h - oh) } else { (w + ow, h + oh) };
            f.insert("width".into(), json!(w));
            f.insert("height".into(), json!(h));
            f.insert("relative".into(), json!(rel));
        }
    });
    ui.horizontal(|ui| {
        label(ui, tl!("Anchor:"), 90.0);
        let cur = f.get("anchor").and_then(Value::as_str).unwrap_or("center").to_string();
        if let Some(a) = anchor_grid(ui, &cur, nw - ow, nh - oh) {
            f.insert("anchor".into(), json!(a));
        }
    });
    ui.add_space(4.0);
    ui.add_enabled_ui(f.get("__background").and_then(Value::as_bool).unwrap_or(false), |ui| {
        ui.horizontal(|ui| {
            label(ui, tl!("Canvas extension color:"), 150.0);
            let mut cur = f.get("extensionColor").and_then(Value::as_str).unwrap_or("background").to_string();
            let opts = [
                ("foreground".to_string(), tl!("Foreground")),
                ("background".to_string(), tl!("Background")),
                ("white".to_string(), tl!("White")),
                ("black".to_string(), tl!("Black")),
                ("transparent".to_string(), tl!("Transparent")),
                (if cur.starts_with('#') { cur.clone() } else { "#ffffff".into() }, tl!("Other…")),
            ];
            if crate::widgets::dropdown(ui, "cs-ext", &mut cur, &opts, 140.0) {
                f.insert("extensionColor".into(), json!(cur.clone()));
            }
            let mut rgb = match cur.as_str() {
                "foreground" => {
                    let q = app.session.tools.foreground;
                    [q[0], q[1], q[2]]
                }
                "white" => [1.0; 3],
                "black" => [0.0; 3],
                hex if hex.starts_with('#') => {
                    let component = |i: usize| hex.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).map_or(0.0, |v| f32::from(v) / 255.0);
                    [component(1), component(3), component(5)]
                }
                _ => {
                    let q = app.session.tools.background;
                    [q[0], q[1], q[2]]
                }
            };
            if crate::widgets::color_edit_button_rgb(ui, &mut rgb).changed() {
                let component = |n: f32| (n.clamp(0.0, 1.0) * 255.0).round() as u8;
                f.insert("extensionColor".into(), json!(format!("#{:02x}{:02x}{:02x}", component(rgb[0]), component(rgb[1]), component(rgb[2]))));
            }
        });
    });
}

/// 3×3 anchor picker; arrows point away from the anchor in the directions the canvas grows.
fn anchor_grid(ui: &mut egui::Ui, current: &str, dw: f64, dh: f64) -> Option<&'static str> {
    let t = Tokens::get(ui.ctx());
    let cell = 24.0;
    let (rect, _) = ui.allocate_exact_size(vec2(cell * 3.0, cell * 3.0), Sense::hover());
    let ci = ANCHORS.iter().position(|a| *a == current).unwrap_or(4) as i32;
    let (ax, ay) = (ci % 3, ci / 3);
    let mut picked = None;
    for (i, a) in ANCHORS.iter().enumerate() {
        let (x, y) = (i as i32 % 3, i as i32 / 3);
        let r = Rect::from_min_size(rect.min + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell)).shrink(1.0);
        let resp = ui.interact(r, ui.id().with(("anchor", i)), Sense::click());
        let selected = i as i32 == ci;
        ui.painter().rect_filled(
            r,
            2.0,
            if selected {
                t.field.gamma_multiply(1.6)
            } else if resp.hovered() {
                t.hover
            } else {
                t.field
            },
        );
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
        if selected {
            ui.painter().rect_filled(Rect::from_center_size(r.center(), vec2(8.0, 8.0)), 1.0, t.text);
        } else {
            let (dx, dy) = (x - ax, y - ay);
            // Only neighbouring cells show arrows (Photoshop behaviour); direction flips when shrinking.
            if dx.abs() <= 1 && dy.abs() <= 1 {
                let sx = if dw < 0.0 { -1.0 } else { 1.0 };
                let sy = if dh < 0.0 { -1.0 } else { 1.0 };
                let dir = vec2(dx as f32 * sx, dy as f32 * sy);
                if dir != egui::Vec2::ZERO {
                    let dir = dir.normalized();
                    let c = r.center();
                    let tip = c + dir * 6.0;
                    let tail = c - dir * 5.0;
                    let side = vec2(-dir.y, dir.x) * 3.5;
                    let col = t.text_dim;
                    ui.painter().line_segment([tail, tip], Stroke::new(1.3, col));
                    ui.painter().add(egui::Shape::convex_polygon(vec![tip + dir * 1.5, tip - dir * 3.5 + side, tip - dir * 3.5 - side], col, Stroke::NONE));
                }
            }
        }
        if resp.clicked() {
            picked = Some(*a);
        }
    }
    let _ = Color32::WHITE;
    picked
}

fn apply_fit(f: &mut Map<String, Value>, preset: &str) {
    let (ow, oh) = (num(f, "__origW"), num(f, "__origH"));
    if preset == "original" {
        f.insert("width".into(), json!(ow));
        f.insert("height".into(), json!(oh));
        f.insert("resolution".into(), json!(num(f, "__origRes")));
    } else if let Some(p) =
        crate::new_doc_ui::CATEGORIES.iter().filter(|(c, _)| matches!(*c, "Print" | "Web")).flat_map(|(_, p)| p.iter()).find(|p| p.0 == preset)
    {
        let scale = (f64::from(p.1) / ow.max(1.0)).min(f64::from(p.2) / oh.max(1.0));
        if f.get("resample").and_then(Value::as_str) == Some("none") {
            f.insert("resample".into(), json!("auto"));
        }
        f.insert("width".into(), json!((ow * scale).round().max(1.0)));
        f.insert("height".into(), json!((oh * scale).round().max(1.0)));
        f.insert("resolution".into(), json!(p.3));
        f.insert("__constrain".into(), json!(true));
    }
    f.insert("__fit".into(), json!(preset));
}

#[derive(Clone)]
struct Preview {
    revision: u64,
    receiver: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<prism_compose::Buffer>>>,
    buffer: Option<std::sync::Arc<prism_compose::Buffer>>,
    texture: Option<egui::TextureHandle>,
    key: String,
    zoom: f32,
    pan: egui::Vec2,
    failed: bool,
}
fn preview(app: &mut PrismApp, ui: &mut egui::Ui, f: &Map<String, Value>, height: f32) {
    let Some(st) = app.session.documents().iter().find(|s| Some(s.doc.id.0) == f.get("__sourceDoc").and_then(Value::as_u64)) else { return };
    let doc = st.doc.clone();
    let revision = st.revision;
    let id = ui.id().with(("size-preview", doc.id.0));
    let mut cached = ui.ctx().data(|d| d.get_temp::<Preview>(id));
    if cached.as_ref().is_none_or(|p| p.revision != revision) {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker_doc = doc.clone();
        let failed = std::thread::Builder::new()
            .name("size-preview".into())
            .spawn(move || {
                let image = prism_compose::thumbnail_buffer(&worker_doc, 512);
                let _ = tx.send(image);
            })
            .is_err();
        cached = Some(Preview {
            revision,
            receiver: std::sync::Arc::new(std::sync::Mutex::new(rx)),
            buffer: None,
            texture: None,
            key: String::new(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            failed,
        });
    }
    let Some(mut cache) = cached else { return };
    if cache.buffer.is_none() && !cache.failed {
        let received = cache.receiver.lock().unwrap_or_else(std::sync::PoisonError::into_inner).try_recv();
        match received {
            Ok(b) => cache.buffer = Some(std::sync::Arc::new(b)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => cache.failed = true,
            Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(20)),
        }
    }
    let (display, display_key) = crate::canvas::canvas_display(app, &doc, crate::monitor_status::view_display(app, ui.ctx()));
    let (w, h) = (num(f, "width").max(1.0), num(f, "height").max(1.0));
    let method = f.get("resample").and_then(Value::as_str).unwrap_or("auto");
    let key = format!("{w}:{h}:{method}:{display_key}");
    if cache.key != key
        && let Some(buffer) = cache.buffer.as_deref()
    {
        // Resample a bounded proxy, never a target-size document, for live visual feedback.
        let scale = (512.0 / w.max(h)).min(1.0);
        let (pw, ph) = ((w * scale).round().clamp(1.0, 512.0) as u32, (h * scale).round().clamp(1.0, 512.0) as u32);
        let filter = match method {
            "nearest" => prism_algo::resample::Resample::Nearest,
            "bilinear" => prism_algo::resample::Resample::Bilinear,
            "lanczos" => prism_algo::resample::Resample::Lanczos,
            "preserveDetails" => prism_algo::resample::Resample::PreserveDetails,
            "auto" if w >= num(f, "__origW") && h >= num(f, "__origH") && (w > num(f, "__origW") || h > num(f, "__origH")) => {
                prism_algo::resample::Resample::PreserveDetails
            }
            "auto" if w <= num(f, "__origW") && h <= num(f, "__origH") && (w < num(f, "__origW") || h < num(f, "__origH")) => {
                prism_algo::resample::Resample::Lanczos
            }
            _ => prism_algo::resample::Resample::Bicubic,
        };
        let mut surface = prism_raster::Surface::new(prism_color::PixelFormat::RGBA32F);
        let pixels: Vec<_> = buffer.px.iter().flat_map(|p| p.iter().copied()).collect();
        surface.write_region(buffer.rect, &pixels);
        let resized = prism_algo::resample::resize_surface_in_canvas(
            &surface,
            f64::from(pw) / f64::from(buffer.rect.width().max(1)),
            f64::from(ph) / f64::from(buffer.rect.height().max(1)),
            filter,
            buffer.rect,
        );
        let rect = prism_geom::Rect::from_xywh(0, 0, pw, ph);
        let pixels = resized.read_region(rect);
        let image = prism_compose::Buffer {
            rect,
            px: pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| {
                    [p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0), p.get(2).copied().unwrap_or(0.0), p.get(3).copied().unwrap_or(0.0)]
                })
                .collect(),
        };
        cache.texture = Some(ui.ctx().load_texture("size-preview", crate::canvas::display_image(display.as_deref(), &image), egui::TextureOptions::LINEAR));
        cache.key = key;
    }
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    ui.painter().rect_filled(rect, 0.0, Tokens::get(ui.ctx()).canvas);
    if response.dragged() {
        cache.pan += response.drag_delta();
    }
    if response.double_clicked() {
        cache.zoom = 1.0;
        cache.pan = egui::Vec2::ZERO;
    }
    if response.hovered() {
        let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        cache.zoom = (cache.zoom * (scroll * 0.005).exp()).clamp(0.1, 16.0);
    }
    if let Some(texture) = &cache.texture {
        let dimensions = vec2(texture.size()[0] as f32, texture.size()[1] as f32);
        let fit = (rect.width() / dimensions.x).min(rect.height() / dimensions.y);
        let image = Rect::from_center_size(rect.center() + cache.pan, dimensions * fit * cache.zoom);
        let painter = ui.painter().with_clip_rect(rect);
        painter.image(
            crate::canvas::checker(app, ui.ctx()),
            image,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(image.width() / 16.0, image.height() / 16.0)),
            Color32::WHITE,
        );
        painter.image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            if cache.failed { tl!("Error") } else { tl!("Preparing preview…") },
            egui::FontId::proportional(12.0),
            Tokens::get(ui.ctx()).text_dim,
        );
    }
    ui.horizontal(|ui| {
        if ui.small_button("−").clicked() {
            cache.zoom = (cache.zoom / 1.25).max(0.1);
        }
        ui.label(format!("{:.0}%", cache.zoom * 100.0));
        if ui.small_button("+").clicked() {
            cache.zoom = (cache.zoom * 1.25).min(16.0);
        }
        if ui.small_button(tl!("Fit")).clicked() {
            cache.zoom = 1.0;
            cache.pan = egui::Vec2::ZERO;
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, cache));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_forms_fit_their_width_in_every_theme_at_both_scales() {
        for theme in crate::theme::ThemeKind::ALL {
            for scale in [1.0, 2.0] {
                for (command, width) in [("image.imageSize", 860.0), ("image.imageSize", 480.0), ("image.canvasSize", 500.0), ("image.canvasSize", 440.0)] {
                    let ctx = egui::Context::default();
                    crate::PrismApp::setup_context(&ctx, Default::default());
                    crate::theme::apply(&ctx, theme);
                    ctx.set_pixels_per_point(scale);
                    let mut app = crate::PrismApp::new(prism_engine::Session::new(), Default::default());
                    app.session.execute("file.new", json!({"width":320,"height":240})).unwrap();
                    let id = open(&mut app, command).unwrap();
                    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
                    let height = if command == "image.canvasSize" { 416.0 } else { 396.0 };
                    let mut out = ctx.run_ui(
                        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, height))), ..Default::default() },
                        |root| {
                            egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(20)).show(root, |ui| {
                                let right = ui.max_rect().right();
                                let bottom = ui.max_rect().bottom();
                                body(&mut app, ui, &mut fields);
                                assert!(ui.min_rect().right() <= right + 1.0, "{theme:?} @ {scale}x {command}: {:?} > {right}", ui.min_rect());
                                assert!(ui.min_rect().bottom() <= bottom + 1.0, "{theme:?} @ {scale}x {command}: {:?} > {bottom}", ui.min_rect());
                            });
                        },
                    );
                    out.textures_delta.clear();
                }
            }
        }
    }

    #[test]
    fn columns_fit_resolution_and_validation() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":3000,"height":1500})).unwrap();
        let id = open(&mut app, "image.imageSize").unwrap();
        let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
        for n in [-3.0, -0.5, 0.0, 0.5, 1.0, 2.0, 3.0] {
            let px = pixel_value(&f, n, "columns", 3000.0, 300.0);
            assert!((display_value(&f, px, "columns", 3000.0, 300.0) - n).abs() < 1e-9);
        }
        assert_eq!(pixel_value(&f, 2.0, "columns", 3000.0, 300.0), 1550.0);
        apply_fit(&mut f, "A4");
        assert!(num(&f, "width") <= 2480.0 && num(&f, "height") <= 3508.0);
        assert!((num(&f, "width") / num(&f, "height") - 2.0).abs() < 0.01);
        apply_fit(&mut f, "original");
        assert_eq!(num(&f, "width"), 3000.0);
        assert_eq!(f["resample"], "auto");
        f.insert("resolution".into(), json!(0));
        assert!(validate(&f).is_err());
    }
    #[test]
    fn transparent_canvas_disables_extension_and_relative_zero_is_valid() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":20,"height":10,"background":"transparent"})).unwrap();
        let id = open(&mut app, "image.canvasSize").unwrap();
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!(f["__background"], false);
        assert_eq!(f["extensionColor"], "transparent");
        f.insert("relative".into(), json!(true));
        f.insert("width".into(), json!(0));
        f.insert("height".into(), json!(0));
        assert!(validate(f).is_ok());
        f.insert("width".into(), json!(-20));
        assert!(validate(f).is_err());
    }
    #[test]
    fn unit_conversions_round_trip() {
        for unit in ["px", "percent", "in", "cm", "mm", "pt", "pica"] {
            let v = to_unit(1500.0, unit, 3000.0, 300.0);
            assert!((from_unit(v, unit, 3000.0, 300.0) - 1500.0).abs() < 1e-9, "{unit}");
        }
        assert_eq!(to_unit(1500.0, "percent", 3000.0, 300.0), 50.0);
        assert_eq!(to_unit(600.0, "in", 3000.0, 300.0), 2.0);
        assert_eq!(human_bytes(6016.0 * 6016.0 * 4.0), "138.1M");
    }

    #[test]
    fn dialog_maps_to_engine_command() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = open(&mut app, "image.imageSize").unwrap();
        let d = app.ui.dialog_mut(id).unwrap();
        d.fields.insert("width".into(), json!(100));
        d.fields.insert("height".into(), json!(50));
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.active().unwrap().doc.size, prism_doc::Size::new(100, 50));
        let id = open(&mut app, "image.canvasSize").unwrap();
        let d = app.ui.dialog_mut(id).unwrap();
        d.fields.insert("width".into(), json!(120));
        d.fields.insert("anchor".into(), json!("topLeft"));
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.active().unwrap().doc.size, prism_doc::Size::new(120, 50));
    }

    /// #441: a typed zero width showed "0 px × 1 px" while OK resized to 1×1.
    #[test]
    fn typed_zero_width_shows_the_applied_size() {
        use egui::accesskit::Role;
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
            |ui, app| {
                if let Some(d) = app.ui.dialogs.first().cloned() {
                    let mut fields = d.fields;
                    egui::CentralPanel::default().show(ui, |ui| body(app, ui, &mut fields));
                    if let Some(dialog) = app.ui.dialog_mut(d.id) {
                        dialog.fields = fields;
                    }
                }
            },
            app,
        );
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(h.state_mut(), "image.imageSize").unwrap();
        h.run_steps(3);
        let width = h.query_all_by_role(Role::SpinButton).next().map(|n| n.rect()).expect("the Width field");
        h.hover_at(width.center());
        h.run_steps(1);
        h.drag_at(width.center());
        h.run_steps(1);
        h.drop_at(width.center());
        h.run_steps(2);
        h.event(egui::Event::Text("0".into()));
        h.run_steps(1);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        let f = h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open");
        assert_eq!((num(&f, "width"), num(&f, "height")), (1.0, 1.0), "{f:?}");
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.size, prism_doc::Size::new(1, 1));
    }
}
