//! File › New: Photoshop's New Document dialog. Tabs (Recent, Saved and the blank-document preset
//! categories) over a grid of preset cards on the left, Preset Details on the right. Values live
//! in the dialog fields (`width`/`height` in pixels, `resolution` in ppi, `mode`, `depth`,
//! `background`, `artboards`, `profile`, `name`), so `ui.dialog.set` drives it and Create runs
//! `file.new` with them (then `document.recent.add`, so the size shows under Recent next time).

use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Map, Value, json};

use prism_engine::document_preset_cmds::{DocumentPreset, DocumentSettings};

use crate::theme::Tokens;
use crate::widgets;

/// A preset card (four to a row).
const CARD: egui::Vec2 = vec2(158.0, 172.0);
/// The gap between cards, across and down.
const CARD_GAP: f32 = 8.0;
/// Padding inside the dark grid ground, on every side.
const GRID_PAD: f32 = 20.0;
/// Room kept right of the cards for the grid's scrollbar, so it never covers a card.
const GRID_BAR: f32 = 12.0;
/// Width of the preset grid on the left: four cards, their gaps, the padding and the scrollbar.
const GRID_W: f32 = 2.0 * GRID_PAD + 4.0 * CARD.x + 3.0 * CARD_GAP + GRID_BAR;
/// The grid's heading row ("BLANK DOCUMENT PRESETS (14)") and the space under it.
const HEADING_H: f32 = 26.0;
/// Height of the scrolling preset grid: the heading and three whole rows of cards, so the
/// view ends between rows instead of cutting a card's text.
const GRID_H: f32 = HEADING_H + 3.0 * CARD.y + 2.0 * CARD_GAP;
/// Between the grid and Preset Details.
const COLUMN_GAP: f32 = 24.0;
/// Width of Preset Details on the right.
const DETAILS_W: f32 = 276.0;
/// The horizontal gap between controls in a Preset Details row.
const FIELD_GAP: f32 = 6.0;
/// The dialog's content width: the grid, the gap and Preset Details, so the details' right edge
/// lines up with Create / Close.
pub const WIDTH: f32 = GRID_W + COLUMN_GAP + DETAILS_W;

/// A blank-document preset: (name, width, height, ppi, unit). Width and height are in `unit`
/// (`px`, `in`, `cm`, `mm`), as Photoshop shows them ("8.5 x 11 in @ 300 ppi").
pub type Preset = (&'static str, f32, f32, f32, &'static str);

const PHOTO: &[Preset] = &[
    ("Default Photoshop Size", 16.0, 12.0, 300.0, "cm"),
    ("Landscape, 3 x 2", 3.0, 2.0, 300.0, "in"),
    ("Landscape, 6 x 4", 6.0, 4.0, 300.0, "in"),
    ("Landscape, 7 x 5", 7.0, 5.0, 300.0, "in"),
    ("Landscape, 10 x 8", 10.0, 8.0, 300.0, "in"),
    ("Portrait, 2 x 3", 2.0, 3.0, 300.0, "in"),
    ("Portrait, 4 x 6", 4.0, 6.0, 300.0, "in"),
    ("Portrait, 5 x 7", 5.0, 7.0, 300.0, "in"),
    ("Portrait, 8 x 10", 8.0, 10.0, 300.0, "in"),
];

const PRINT: &[Preset] = &[
    ("Letter", 8.5, 11.0, 300.0, "in"),
    ("Legal", 8.5, 14.0, 300.0, "in"),
    ("Tabloid", 11.0, 17.0, 300.0, "in"),
    ("A4", 210.0, 297.0, 300.0, "mm"),
    ("A6", 105.0, 148.0, 300.0, "mm"),
    ("A5", 148.0, 210.0, 300.0, "mm"),
    ("A3", 297.0, 420.0, 300.0, "mm"),
    ("B5", 176.0, 250.0, 300.0, "mm"),
    ("B4", 250.0, 353.0, 300.0, "mm"),
    ("B3", 353.0, 500.0, 300.0, "mm"),
    ("C4", 229.0, 324.0, 300.0, "mm"),
    ("C5", 162.0, 229.0, 300.0, "mm"),
    ("C6", 114.0, 162.0, 300.0, "mm"),
    ("DL", 110.0, 220.0, 300.0, "mm"),
];

const ART: &[Preset] = &[
    ("1000 pixel grid", 1000.0, 1000.0, 300.0, "px"),
    ("2000 pixel grid", 2000.0, 2000.0, 300.0, "px"),
    ("Poster", 18.0, 24.0, 300.0, "in"),
    ("Postcard", 4.0, 6.0, 300.0, "in"),
    ("1080p", 1920.0, 1080.0, 300.0, "px"),
    ("720p", 1280.0, 720.0, 300.0, "px"),
];

const WEB: &[Preset] = &[
    ("Web Most Common", 1366.0, 768.0, 72.0, "px"),
    ("Web Large", 1920.0, 1080.0, 72.0, "px"),
    ("Web Medium", 1440.0, 900.0, 72.0, "px"),
    ("Web Minimum", 1024.0, 768.0, 72.0, "px"),
    ("Web Small", 1280.0, 800.0, 72.0, "px"),
    ("MacBook Pro 13 (Retina)", 2560.0, 1600.0, 72.0, "px"),
    ("MacBook Pro 15 (Retina)", 2880.0, 1800.0, 72.0, "px"),
    ("iMac 27", 2560.0, 1440.0, 72.0, "px"),
    ("Desktop HD Design", 1440.0, 1024.0, 72.0, "px"),
];

const MOBILE: &[Preset] = &[
    ("iPhone X", 1125.0, 2436.0, 72.0, "px"),
    ("iPhone 8/7/6 Plus", 1242.0, 2208.0, 72.0, "px"),
    ("iPhone 8/7/6", 750.0, 1334.0, 72.0, "px"),
    ("iPad Pro 12.9 in", 2048.0, 2732.0, 72.0, "px"),
    ("iPad Pro 10.5 in", 1668.0, 2226.0, 72.0, "px"),
    ("iPad Retina", 1536.0, 2048.0, 72.0, "px"),
    ("Android 1080p", 1080.0, 1920.0, 72.0, "px"),
    ("Microsoft Surface Pro 4", 2736.0, 1824.0, 72.0, "px"),
    ("Microsoft Surface Pro 3", 2160.0, 1440.0, 72.0, "px"),
    ("iPhone 5", 640.0, 1136.0, 72.0, "px"),
    ("iPad Mini", 768.0, 1024.0, 72.0, "px"),
    ("Apple Watch 42mm", 312.0, 390.0, 72.0, "px"),
    ("Apple Watch 38mm", 272.0, 340.0, 72.0, "px"),
    ("Mobile Design", 320.0, 1024.0, 72.0, "px"),
    ("iOS 7 iPad App Icon", 152.0, 152.0, 72.0, "px"),
    ("iOS 7 iPhone App icon", 120.0, 120.0, 72.0, "px"),
    ("iOS 7 Spotlight", 80.0, 80.0, 72.0, "px"),
    ("iOS 7 Settings", 58.0, 58.0, 72.0, "px"),
    ("Mac Icon 1024", 1024.0, 1024.0, 72.0, "px"),
    ("Mac Icon 512", 512.0, 512.0, 72.0, "px"),
    ("Mac Icon 256", 256.0, 256.0, 72.0, "px"),
    ("Mac Icon 128", 128.0, 128.0, 72.0, "px"),
    ("Mac Icon 48", 48.0, 48.0, 72.0, "px"),
    ("Mac Icon 32", 32.0, 32.0, 72.0, "px"),
    ("Mac Icon 16", 16.0, 16.0, 72.0, "px"),
    ("Legacy iPad App Icon", 144.0, 144.0, 72.0, "px"),
    ("Legacy iPhone App icon", 114.0, 114.0, 72.0, "px"),
    ("Legacy iPad Spotlight", 100.0, 100.0, 72.0, "px"),
];

const FILM: &[Preset] = &[
    ("HDTV 1080p", 1920.0, 1080.0, 72.0, "px"),
    ("HDV/HDTV 720p", 1280.0, 720.0, 72.0, "px"),
    ("HDV 1080p", 1440.0, 1080.0, 72.0, "px"),
    ("DVCPRO HD 720p", 960.0, 720.0, 72.0, "px"),
    ("DVCPRO HD 1080p", 1280.0, 1080.0, 72.0, "px"),
    ("DCI 2K 1080p", 2048.0, 1080.0, 72.0, "px"),
    ("UHDTV/4K 2160p", 3840.0, 2160.0, 72.0, "px"),
    ("DCI 4K 2160p", 4096.0, 2160.0, 72.0, "px"),
    ("FUHDTV/8K 4320p", 7680.0, 4320.0, 72.0, "px"),
    ("DCI 8K 4320p", 8192.0, 4320.0, 72.0, "px"),
    ("NTSC DV", 720.0, 480.0, 72.0, "px"),
    ("NTSC DV Widescreen", 720.0, 480.0, 72.0, "px"),
    ("NTSC D1", 720.0, 486.0, 72.0, "px"),
    ("NTSC D1 Widescreen", 720.0, 486.0, 72.0, "px"),
    ("NTSC D1 Square Pixel", 720.0, 534.0, 72.0, "px"),
    ("NTSC D1 Widescreen Square Pixel", 872.0, 486.0, 72.0, "px"),
    ("PAL D1/DV", 720.0, 576.0, 72.0, "px"),
    ("PAL D1/DV Widescreen", 720.0, 576.0, 72.0, "px"),
    ("PAL D1/DV Square Pixel", 788.0, 576.0, 72.0, "px"),
    ("PAL D1/DV Widescreen Square Pixel", 1050.0, 576.0, 72.0, "px"),
    ("Cineon Half", 1828.0, 1332.0, 72.0, "px"),
    ("Cineon Full", 3656.0, 2664.0, 72.0, "px"),
    ("Film (2K)", 2048.0, 1556.0, 72.0, "px"),
    ("Film (4K)", 4096.0, 3112.0, 72.0, "px"),
    ("Film (8K)", 8192.0, 6224.0, 72.0, "px"),
];

/// Photoshop's blank-document preset categories, in tab order.
pub const CATEGORIES: &[(&str, &[Preset])] = &[("Photo", PHOTO), ("Print", PRINT), ("Art & Illustration", ART), ("Web", WEB), ("Mobile", MOBILE), ("Film & Video", FILM)];

/// The tab names, in Photoshop's order.
pub fn tabs() -> impl Iterator<Item = &'static str> {
    ["Recent", "Saved"].into_iter().chain(CATEGORIES.iter().map(|c| c.0))
}

const DEPTH_OPTIONS: &[(u64, &str, &str)] = &[(8, "8 bit", "Integer"), (16, "16 bit", "Integer"), (32, "32 bit (float)", "Floating point")];

/// Width/Height units: (key, label, units per inch; 0 = pixels).
pub const UNITS: &[(&str, &str, f32)] =
    &[("px", "Pixels", 0.0), ("in", "Inches", 1.0), ("cm", "Centimeters", 2.54), ("mm", "Millimeters", 25.4), ("pt", "Points", 72.0), ("pica", "Picas", 6.0)];

/// Pixels to the display unit at `ppi`.
pub fn to_unit(px: f32, unit: &str, ppi: f32) -> f32 {
    match UNITS.iter().find(|u| u.0 == unit) {
        Some((_, _, per_in)) if *per_in > 0.0 => px / ppi.max(1.0) * per_in,
        _ => px,
    }
}

/// Display unit back to pixels at `ppi`.
pub fn from_unit(v: f32, unit: &str, ppi: f32) -> f32 {
    match UNITS.iter().find(|u| u.0 == unit) {
        Some((_, _, per_in)) if *per_in > 0.0 => (v / per_in * ppi.max(1.0)).round(),
        _ => v.round(),
    }
}

/// A preset's size in whole pixels.
pub fn preset_px(p: &Preset) -> (u32, u32) {
    let px = |v: f32| from_unit(v, p.4, p.3).clamp(1.0, 300_000.0) as u32;
    (px(p.1), px(p.2))
}

/// A typed size as the whole pixel count `file.new` takes (#254: a float like `512.0` isn't one, so
/// the command fell back to its 1920 x 1080 default). Clamped to the command's 1–300000 range.
pub fn px_value(px: f32) -> Value {
    json!(if px.is_finite() { px.round().clamp(1.0, 300_000.0) as u32 } else { 1 })
}

/// Name of the preset that takes the clipboard image's size.
pub const CLIPBOARD: &str = "Clipboard";

/// Offer the Clipboard preset (`w` × `h` px, at 72 ppi) first under Recent, and select it.
pub fn set_clipboard(f: &mut Map<String, Value>, w: u32, h: u32) {
    if w == 0 || h == 0 {
        return;
    }
    f.insert("__clipboard".into(), json!([w, h]));
    apply_preset(f, "Recent", &clipboard(w, h));
}

fn clipboard(w: u32, h: u32) -> Preset {
    (CLIPBOARD, w as f32, h as f32, 72.0, "px")
}

/// The clipboard image's size, when the dialog offers the Clipboard preset.
fn clipboard_preset(f: &Map<String, Value>) -> Option<Preset> {
    let size = f.get("__clipboard")?.as_array()?;
    let dim = |i: usize| size.get(i)?.as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
    Some(clipboard(dim(0)?, dim(1)?))
}

/// Set Width or Height (`key`) from a value typed in `unit`: whole pixels for `file.new`, plus the
/// typed value so the field keeps showing it (see [`shown_size`]).
pub fn set_size(f: &mut Map<String, Value>, key: &str, v: f32, unit: &str, ppi: f32) {
    f.insert(key.into(), px_value(from_unit(v, unit, ppi)));
    f.insert(typed_key(key), json!({"value": v, "unit": unit}));
    deselect(f);
}

/// Editing a value leaves whichever card was selected.
fn deselect(f: &mut Map<String, Value>) {
    f.remove("__preset");
    f.remove("__savedPreset");
    f.remove("__recent");
}

/// What Width or Height (`key`, `d` pixels when unset) shows in `unit`: the value the user typed
/// while it still gives the field's pixel count, else the pixels converted. Converting the rounded
/// pixels back on every keystroke turned a typed `5` mm into 14 px and the text into `4.9` (#1147).
/// A size over the 300000 px limit shows the limit, not the typed value the document won't have.
pub fn shown_size(f: &Map<String, Value>, key: &str, d: f32, unit: &str, ppi: f32) -> f32 {
    let px = get_f(f, key, d);
    let typed = f.get(&typed_key(key)).and_then(Value::as_object).filter(|t| unit != "px" && t.get("unit").and_then(Value::as_str) == Some(unit));
    match typed.and_then(|t| t.get("value")).and_then(Value::as_f64) {
        Some(v) if from_unit(v as f32, unit, ppi).max(1.0) == px => v as f32,
        _ => to_unit(px, unit, ppi),
    }
}

fn typed_key(key: &str) -> String {
    format!("__{key}Typed")
}

/// Swap Width and Height (the Orientation buttons), with the values typed for them, so a typed
/// 841 x 1189 mm turns into 1189 x 841 mm, not 1188.9 x 841.
pub fn swap_size(f: &mut Map<String, Value>) {
    f.remove("__savedPreset");
    f.remove("__recent");
    let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
    f.insert("width".into(), px_value(h));
    f.insert("height".into(), px_value(w));
    let (typed_w, typed_h) = (f.remove(&typed_key("width")), f.remove(&typed_key("height")));
    if let Some(t) = typed_h {
        f.insert(typed_key("width"), t);
    }
    if let Some(t) = typed_w {
        f.insert(typed_key("height"), t);
    }
}

/// What a category's presets create besides their size: (artboards, colour profile). Photoshop
/// makes screen designs (Web, Mobile) on an artboard, and leaves screen and video documents
/// unmanaged; print and photo documents get the working space.
pub fn category_defaults(category: &str) -> (bool, &'static str) {
    match category {
        "Web" | "Mobile" => (true, "none"),
        "Film & Video" | "Recent" => (false, "none"),
        _ => (false, "working"),
    }
}

/// Select a preset of `category`: its size, resolution and unit, and the category's artboard and
/// profile choices.
pub fn apply_preset(f: &mut Map<String, Value>, category: &str, p: &Preset) {
    deselect(f);
    let (w, h) = preset_px(p);
    f.insert("width".into(), json!(w));
    f.insert("height".into(), json!(h));
    f.insert("resolution".into(), json!(p.3));
    f.insert("__preset".into(), json!(p.0));
    f.insert("__unit".into(), json!(p.4));
    f.insert("__resUnit".into(), json!("in"));
    // Keep the preset's own numbers on screen (210 x 297 mm, not 209.97).
    f.remove(&typed_key("width"));
    f.remove(&typed_key("height"));
    if p.4 != "px" {
        f.insert(typed_key("width"), json!({"value": p.1, "unit": p.4}));
        f.insert(typed_key("height"), json!({"value": p.2, "unit": p.4}));
    }
    let (artboards, profile) = category_defaults(category);
    f.insert("artboards".into(), json!(artboards));
    f.insert("profile".into(), json!(profile));
}

/// Fields `file.new` takes (drops the dialog's `__` UI keys).
pub fn command_params(f: &Map<String, Value>) -> Value {
    Value::Object(f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// The form as document settings (`document.presets.save`, `document.recent.add`).
pub fn settings_params(f: &Map<String, Value>) -> Value {
    let mut settings = command_params(f);
    if let Some(o) = settings.as_object_mut() {
        o.remove("name");
    }
    settings["unit"] = json!(get_s(f, "__unit", "px"));
    settings["resolutionUnit"] = json!(get_s(f, "__resUnit", "in"));
    settings
}

/// The name a created document is remembered by under Recent: the preset it came from, else
/// "Custom".
pub fn recent_name(f: &Map<String, Value>) -> String {
    match (f.get("__preset").and_then(Value::as_str), f.get("__savedPreset").and_then(Value::as_str), f.get("__recentName").and_then(Value::as_str)) {
        (Some(p), _, _) if p != CLIPBOARD => p.to_string(),
        (None, Some(s), _) => s.to_string(),
        (None, None, Some(r)) if f.contains_key("__recent") => r.to_string(),
        _ => "Custom".into(),
    }
}

/// Set the resolution (pixels/inch) the way Photoshop's New Document does (#758): with Width/Height
/// in a physical unit the physical size is kept and the pixel count changes; in pixels the pixels
/// are kept.
pub fn set_resolution(f: &mut Map<String, Value>, new_ppi: f32) {
    let old_ppi = get_f(f, "resolution", 72.0);
    f.insert("resolution".into(), json!(new_ppi));
    if old_ppi != new_ppi {
        f.remove("__savedPreset");
        f.remove("__recent");
    }
    if get_s(f, "__unit", "px") == "px" || !(old_ppi > 0.0 && new_ppi > 0.0) || old_ppi == new_ppi {
        return;
    }
    let scale = new_ppi / old_ppi;
    let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
    f.insert("width".into(), px_value(w * scale));
    f.insert("height".into(), px_value(h * scale));
    deselect(f);
}

/// Can Create run? Not on an empty Saved tab, where Photoshop shows no details at all.
pub fn can_create(f: &Map<String, Value>) -> bool {
    get_s(f, "__category", "Recent") != "Saved" || f.contains_key("__savedPreset")
}

fn get_f(f: &Map<String, Value>, k: &str, d: f32) -> f32 {
    f.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}

fn get_s(f: &Map<String, Value>, k: &str, d: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or(d).to_string()
}

fn small_label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).size(11.5).color(t.text_dim));
}

/// Lay out a preset card's title centred in `width`: wrapped onto at most two lines, the rest
/// elided, so long translations stay inside the card.
fn card_title(painter: &egui::Painter, title: &str, width: f32, color: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(title.to_owned(), egui::FontId::proportional(12.0), color, width);
    job.wrap.max_rows = 2;
    job.halign = egui::Align::Center;
    painter.layout_job(job)
}

/// Restore a snapshot (a saved preset or a recent item) without replacing the new document's name
/// or its clipboard offer.
pub fn apply_settings(f: &mut Map<String, Value>, settings: &DocumentSettings) {
    deselect(f);
    f.remove("backgroundColor");
    f.remove("artboards");
    f.insert("profile".into(), json!("none"));
    if let Some(params) = settings.command_params().as_object() {
        f.extend(params.clone());
    }
    f.insert("__unit".into(), json!(settings.unit));
    f.insert("__resUnit".into(), json!(settings.resolution_unit));
    // Typed Width/Height belong to the previous values (`shown_size`).
    f.remove(&typed_key("width"));
    f.remove(&typed_key("height"));
}

/// Select a saved preset.
pub fn apply_saved_preset(f: &mut Map<String, Value>, preset: &DocumentPreset) {
    apply_settings(f, &preset.settings);
    f.insert("__savedPreset".into(), json!(preset.name));
}

/// Select recent item `i`.
fn apply_recent(f: &mut Map<String, Value>, i: usize, item: &DocumentPreset) {
    apply_settings(f, &item.settings);
    f.insert("__recent".into(), json!(i));
    f.insert("__recentName".into(), json!(item.name));
}

/// Open on Recent with its first item selected: the clipboard image when there is one (already
/// applied by [`set_clipboard`]), else the newest recent document.
pub fn select_first_recent(f: &mut Map<String, Value>, recent: &[DocumentPreset]) {
    if clipboard_preset(f).is_none()
        && let Some(item) = recent.first()
    {
        apply_recent(f, 0, item);
    }
}

/// Switch tab, selecting its first card like Photoshop.
fn select_tab(app: &crate::PrismApp, f: &mut Map<String, Value>, tab: &str) {
    f.insert("__category".into(), json!(tab));
    f.remove("__savingPreset");
    match tab {
        "Recent" => match clipboard_preset(f) {
            Some(c) => apply_preset(f, "Recent", &c),
            None => {
                if let Some(item) = app.session.presets.recent_documents.first().cloned() {
                    apply_recent(f, 0, &item);
                }
            }
        },
        "Saved" => match app.session.presets.documents.first().cloned() {
            Some(p) => apply_saved_preset(f, &p),
            None => deselect(f),
        },
        cat => {
            if let Some(p) = CATEGORIES.iter().find(|c| c.0 == cat).and_then(|c| c.1.first()) {
                apply_preset(f, cat, p);
            }
        }
    }
}

/// "8.5 x 11 in @ 300 ppi" for a size in `unit`.
fn size_label(w: f32, h: f32, unit: &str, ppi: f32) -> String {
    let n = |v: f32| widgets::fmt_num2(f64::from(v));
    format!("{} x {} {unit} @ {} ppi", n(w), n(h), n(ppi))
}

// ------------------------------------------------------------------ card icons

/// What a card's icon shows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Glyph {
    Clipboard,
    Custom,
    Page,
    Photo,
    Brush,
    Grid,
    Browser,
    Phone,
    Tablet,
    Watch,
    AppIcon,
    Film(Option<&'static str>, bool, bool),
}

fn glyph(category: &str, name: &str) -> Glyph {
    match category {
        "Photo" => Glyph::Photo,
        "Print" | "Saved" => Glyph::Page,
        "Art & Illustration" if name.contains("grid") => Glyph::Grid,
        "Art & Illustration" => Glyph::Brush,
        "Web" => Glyph::Browser,
        "Mobile" if name.contains("Watch") => Glyph::Watch,
        "Mobile" if name.contains("Icon") || name.contains("icon") || name.contains("Spotlight") || name.contains("Settings") => Glyph::AppIcon,
        "Mobile" if name.contains("Surface") => Glyph::Tablet,
        "Mobile" => Glyph::Phone,
        "Film & Video" => {
            let badge = ["8K", "4K", "2K"].into_iter().find(|b| name.contains(b));
            Glyph::Film(badge, name.contains("Widescreen") && !name.contains("Square"), name.contains("Square Pixel"))
        }
        _ if name == CLIPBOARD => Glyph::Clipboard,
        _ => Glyph::Custom,
    }
}

/// The icon's outline for a `w` x `h` document, `scale` (0..1) of the largest, centred in `area`.
fn icon_rect(area: Rect, w: f32, h: f32, scale: f32) -> Rect {
    let side = area.width().min(area.height()) * scale.clamp(0.35, 1.0);
    let (w, h) = (w.max(1.0), h.max(1.0));
    let s = side / w.max(h);
    // Very thin documents still get a visible icon.
    Rect::from_center_size(area.center(), vec2((w * s).max(side * 0.18), (h * s).max(side * 0.18)))
}

fn paint_glyph(p: &egui::Painter, area: Rect, g: Glyph, w: f32, h: f32, scale: f32, c: Color32) {
    let st = Stroke::new(1.5, c);
    let r = icon_rect(area, w, h, scale);
    let line = |a: Pos2, b: Pos2| p.line_segment([a, b], st);
    let fold = |r: Rect| {
        // A page with its top-right corner folded.
        let k = (r.width().min(r.height()) * 0.28).clamp(4.0, 14.0);
        let pts = vec![r.left_top(), pos2(r.right() - k, r.top()), pos2(r.right(), r.top() + k), r.right_bottom(), r.left_bottom()];
        p.add(Shape::closed_line(pts, st));
        p.add(Shape::line(vec![pos2(r.right() - k, r.top()), pos2(r.right() - k, r.top() + k), pos2(r.right(), r.top() + k)], st));
    };
    match g {
        Glyph::Page => fold(r),
        Glyph::Custom => {
            // A page with crop marks off its top-left corner.
            let page = Rect::from_min_max(r.min + vec2(6.0, 6.0), r.max);
            fold(page);
            line(pos2(page.left(), page.top() - 14.0), pos2(page.left(), page.top() - 4.0));
            line(pos2(page.left() - 14.0, page.top()), pos2(page.left() - 4.0, page.top()));
        }
        Glyph::Clipboard => {
            let board = Rect::from_center_size(area.center() + vec2(0.0, 4.0), vec2(48.0, 60.0));
            p.rect_stroke(board, 3.0, st, StrokeKind::Middle);
            let clip = Rect::from_center_size(pos2(board.center().x, board.top()), vec2(22.0, 8.0));
            p.rect_filled(clip.expand(1.0), 2.0, Color32::TRANSPARENT);
            p.rect_stroke(clip, 2.0, st, StrokeKind::Middle);
            p.circle_stroke(pos2(clip.center().x, clip.top() - 3.0), 3.0, st);
        }
        Glyph::Photo => {
            p.rect_stroke(r, 1.0, st, StrokeKind::Middle);
            let d = r.width().min(r.height());
            p.circle_stroke(pos2(r.right() - d * 0.2, r.top() + d * 0.22), d * 0.09, st);
            let at = |x: f32, y: f32| pos2(r.left() + r.width() * x, r.top() + r.height() * y);
            p.add(Shape::line(vec![at(0.05, 0.92), at(0.38, 0.5), at(0.62, 0.8), at(0.75, 0.62), at(0.95, 0.88)], st));
        }
        Glyph::Brush => {
            p.rect_stroke(r, 1.0, st, StrokeKind::Middle);
            let d = r.width().min(r.height()) * 0.5;
            let c0 = r.center();
            line(c0 + vec2(-d * 0.15, d * 0.15), c0 + vec2(d * 0.45, -d * 0.45));
            p.add(Shape::closed_line(vec![c0 + vec2(-d * 0.15, d * 0.15), c0 + vec2(-d * 0.45, d * 0.45), c0 + vec2(-d * 0.05, d * 0.35)], st));
        }
        Glyph::Grid => {
            p.rect_stroke(r, 2.0, st, StrokeKind::Middle);
            let n = 8;
            for i in 1..n {
                for j in 1..n {
                    let at = pos2(r.left() + r.width() * i as f32 / n as f32, r.top() + r.height() * j as f32 / n as f32);
                    p.circle_filled(at, 0.9, c);
                }
            }
        }
        Glyph::Browser => {
            p.rect_stroke(r, 2.0, st, StrokeKind::Middle);
            let y = r.top() + (r.height() * 0.16).max(5.0);
            line(pos2(r.left(), y), pos2(r.right(), y));
            line(pos2(r.left() + 4.0, r.top() + 3.0), pos2(r.left() + 8.0, r.top() + 3.0));
        }
        Glyph::Phone | Glyph::Tablet => {
            p.rect_stroke(r, 4.0, st, StrokeKind::Middle);
            if g == Glyph::Tablet {
                line(pos2(r.right() + 5.0, r.top() + 2.0), pos2(r.right() + 5.0, r.bottom() - 4.0));
                p.circle_filled(pos2(r.center().x - 3.0, r.top() + 4.0), 1.0, c);
                p.circle_filled(pos2(r.center().x + 3.0, r.top() + 4.0), 1.0, c);
            } else {
                p.circle_stroke(pos2(r.center().x, r.bottom() - 6.0), 2.5, st);
            }
        }
        Glyph::Watch => {
            let body = Rect::from_center_size(area.center(), vec2(r.width().min(40.0), r.height().min(48.0)));
            p.rect_stroke(body, 8.0, st, StrokeKind::Middle);
            let strap = |y0: f32, y1: f32| p.rect_stroke(Rect::from_x_y_ranges(body.left() + 7.0..=body.right() - 7.0, y0..=y1), 2.0, st, StrokeKind::Middle);
            strap(body.top() - 14.0, body.top());
            strap(body.bottom(), body.bottom() + 14.0);
        }
        Glyph::AppIcon => {
            p.rect_stroke(r, r.width() * 0.2, st, StrokeKind::Middle);
            let rad = r.width() * 0.3;
            for k in 0..16 {
                let a = k as f32 / 16.0 * std::f32::consts::TAU;
                p.circle_filled(r.center() + vec2(a.cos(), a.sin()) * rad, 0.9, c);
            }
        }
        Glyph::Film(badge, wide, square) => {
            p.rect_stroke(r, 2.0, st, StrokeKind::Middle);
            if wide {
                let k = r.width() * 0.12;
                line(pos2(r.left() + k, r.top()), pos2(r.left() + k, r.bottom()));
                line(pos2(r.right() - k, r.top()), pos2(r.right() - k, r.bottom()));
            }
            if square {
                for i in 1..10 {
                    for j in 1..6 {
                        p.circle_filled(pos2(r.left() + r.width() * i as f32 / 10.0, r.top() + r.height() * j as f32 / 6.0), 0.7, c);
                    }
                }
            }
            let s = (r.height() * 0.22).clamp(5.0, 10.0);
            let o = r.center();
            p.add(Shape::closed_line(vec![o + vec2(-s * 0.6, -s), o + vec2(s, 0.0), o + vec2(-s * 0.6, s)], st));
            if let Some(b) = badge {
                p.text(pos2(r.right() - 3.0, r.top() + 2.0), Align2::RIGHT_TOP, b, egui::FontId::proportional(9.5), c);
            }
        }
    }
}

/// A card: icon, title and size line. Returns its response.
fn card(ui: &mut egui::Ui, g: Glyph, (w, h): (f32, f32), scale: f32, title: &str, size: &str, selected: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(CARD, Sense::click());
    if resp.hovered() && !selected {
        ui.painter().rect_filled(r.shrink(2.0), t.radius, t.hover);
    }
    if selected {
        ui.painter().rect_stroke(r.shrink(3.0), t.radius, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    }
    let icon_area = Rect::from_min_size(r.min + vec2(20.0, 16.0), vec2(CARD.x - 40.0, 96.0));
    paint_glyph(ui.painter(), icon_area, g, w, h, scale, t.text_dim);
    let title_g = card_title(ui.painter(), title, CARD.x - 16.0, t.text);
    let elided = title_g.elided;
    let rows = title_g.rows.len() as f32;
    ui.painter().galley(pos2(r.center().x, r.top() + 122.0 - (rows - 1.0) * 7.0), title_g, t.text);
    ui.painter().text(pos2(r.center().x, r.top() + 152.0), Align2::CENTER_CENTER, size, egui::FontId::proportional(11.0), t.text_dim);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if elided { resp.on_hover_text(title) } else { resp }
}

/// Icon scale of each preset in a category: the physically larger ones draw larger (Tabloid over
/// Letter), as in Photoshop.
fn scales(presets: &[Preset]) -> Vec<f32> {
    let long = |p: &Preset| {
        let (w, h) = preset_px(p);
        w.max(h) as f32 / p.3.max(1.0)
    };
    let max = presets.iter().map(long).fold(0.0f32, f32::max).max(1e-3);
    presets.iter().map(|p| (long(p) / max).sqrt()).collect()
}

/// A row-major grid of cards, four to a row.
fn grid<T>(ui: &mut egui::Ui, items: &[T], mut each: impl FnMut(&mut egui::Ui, usize, &T)) {
    for (row, chunk) in items.chunks(4).enumerate() {
        if row > 0 {
            ui.add_space(CARD_GAP);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP;
            for (k, item) in chunk.iter().enumerate() {
                each(ui, row * 4 + k, item);
            }
        });
    }
}

/// The grid's heading, in a row of fixed height so three rows of cards fill the view exactly.
fn heading(ui: &mut egui::Ui, text: &str, n: usize) {
    let t = Tokens::get(ui.ctx());
    let label = RichText::new(crate::i18n::fmt(text, &[("n", &n.to_string())])).size(12.0).color(t.text_dim).strong();
    ui.allocate_ui_with_layout(vec2(ui.available_width(), HEADING_H), egui::Layout::left_to_right(egui::Align::Min), |ui| {
        ui.set_min_height(HEADING_H);
        ui.label(label);
    });
}

fn recent_cards(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let recent = app.session.presets.recent_documents.clone();
    let clip = clipboard_preset(f);
    let n = recent.len() + usize::from(clip.is_some());
    heading(ui, tl!("YOUR RECENT ITEMS ({n})"), n);
    if n == 0 {
        let t = Tokens::get(ui.ctx());
        ui.label(RichText::new(tl!("Documents you create appear here.")).color(t.text_faint));
        return;
    }
    // The clipboard image takes the first cell, then the recent documents, newest first.
    let cells: Vec<Option<usize>> = clip.iter().map(|_| None).chain((0..recent.len()).map(Some)).collect();
    let chosen = f.get("__recent").and_then(Value::as_u64).map(|i| i as usize);
    let clip_selected = get_s(f, "__preset", "") == CLIPBOARD;
    let mut pick: Option<Option<usize>> = None;
    grid(ui, &cells, |ui, _, cell| match cell {
        None => {
            if let Some(c) = clip
                && card(ui, Glyph::Clipboard, (c.1, c.2), 1.0, tl!("Clipboard"), &size_label(c.1, c.2, "px", c.3), clip_selected).clicked()
            {
                pick = Some(None);
            }
        }
        Some(i) => {
            let Some(item) = recent.get(*i) else { return };
            let s = &item.settings;
            let (w, h) = (to_unit(s.width as f32, &s.unit, s.resolution), to_unit(s.height as f32, &s.unit, s.resolution));
            let title = tl!(item.name.as_str());
            if card(ui, Glyph::Custom, (s.width as f32, s.height as f32), 0.85, title, &size_label(w, h, &s.unit, s.resolution), chosen == Some(*i)).clicked() {
                pick = Some(Some(*i));
            }
        }
    });
    match pick {
        Some(None) => {
            if let Some(c) = clip {
                apply_preset(f, "Recent", &c);
            }
        }
        Some(Some(i)) => {
            if let Some(item) = recent.get(i) {
                apply_recent(f, i, item);
            }
        }
        None => {}
    }
}

fn saved_cards(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let presets = app.session.presets.documents.clone();
    heading(ui, tl!("SAVED BLANK DOCUMENT PRESETS ({n})"), presets.len());
    if presets.is_empty() {
        let t = Tokens::get(ui.ctx());
        ui.label(RichText::new(tl!("No saved presets yet.")).color(t.text_faint));
        return;
    }
    let chosen = get_s(f, "__savedPreset", "");
    let mut action: Option<(usize, bool)> = None;
    grid(ui, &presets, |ui, i, preset| {
        let s = &preset.settings;
        let (w, h) = (to_unit(s.width as f32, &s.unit, s.resolution), to_unit(s.height as f32, &s.unit, s.resolution));
        let resp = card(ui, Glyph::Page, (s.width as f32, s.height as f32), 0.85, &preset.name, &size_label(w, h, &s.unit, s.resolution), chosen == preset.name);
        if resp.clicked() {
            action = Some((i, false));
        }
        // Photoshop deletes a saved preset from its card's context menu.
        resp.context_menu(|ui| {
            if ui.button(tl!("Delete Preset")).clicked() {
                action = Some((i, true));
                ui.close();
            }
        });
    });
    match action.and_then(|(i, del)| presets.get(i).map(|p| (p, del))) {
        Some((p, false)) => apply_saved_preset(f, p),
        Some((p, true)) => match app.run("document.presets.delete", json!({"name":p.name})) {
            Ok(_) => {
                f.remove("__presetError");
                if chosen == p.name {
                    f.remove("__savedPreset");
                }
            }
            Err(e) => {
                f.insert("__presetError".into(), json!(e.to_string()));
            }
        },
        None => {}
    }
}

fn preset_cards(ui: &mut egui::Ui, f: &mut Map<String, Value>, cat: &str, presets: &[Preset]) {
    heading(ui, tl!("BLANK DOCUMENT PRESETS ({n})"), presets.len());
    let chosen = get_s(f, "__preset", "");
    let sc = scales(presets);
    let mut pick = None;
    grid(ui, presets, |ui, i, p| {
        let (w, h) = preset_px(p);
        let g = glyph(cat, p.0);
        if card(ui, g, (w as f32, h as f32), sc.get(i).copied().unwrap_or(1.0), tl!(p.0), &size_label(p.1, p.2, p.4, p.3), chosen == p.0).clicked() {
            pick = Some(*p);
        }
    });
    if let Some(p) = pick {
        apply_preset(f, cat, &p);
    }
}

// ------------------------------------------------------------------ details

/// The Save Document Preset icon (a tray with an arrow into it).
fn save_icon(ui: &mut egui::Ui) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let c = if resp.hovered() { t.text } else { t.text_dim };
    let st = Stroke::new(1.5, c);
    let b = r.shrink(4.0);
    ui.painter().add(Shape::line(vec![pos2(b.left() + 4.0, b.top() + 6.0), pos2(b.left(), b.top() + 6.0), b.left_bottom(), b.right_bottom(), pos2(b.right(), b.top() + 6.0), pos2(b.right() - 4.0, b.top() + 6.0)], st));
    let x = b.center().x;
    ui.painter().line_segment([pos2(x, b.top() - 1.0), pos2(x, b.bottom() - 5.0)], st);
    ui.painter().add(Shape::line(vec![pos2(x - 4.0, b.bottom() - 9.0), pos2(x, b.bottom() - 5.0), pos2(x + 4.0, b.bottom() - 9.0)], st));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Save Document Preset")));
    resp.on_hover_text(tl!("Save Document Preset"))
}

fn orientation_button(ui: &mut egui::Ui, portrait: bool, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let fill = if on { t.accent } else if resp.hovered() { t.hover } else { t.field };
    ui.painter().rect_filled(r, t.radius_sm, fill);
    ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, if on { t.accent } else { t.field_border }), StrokeKind::Inside);
    // A person on a page.
    let page = if portrait { Rect::from_center_size(r.center(), vec2(16.0, 22.0)) } else { Rect::from_center_size(r.center(), vec2(22.0, 16.0)) };
    let ink = if on { Color32::WHITE } else { t.text_dim };
    let (bg_page, fg) = if on { (t.accent, ink) } else { (Color32::from_gray(220), Color32::from_gray(60)) };
    ui.painter().rect_filled(page, 1.0, if on { Color32::from_white_alpha(40) } else { bg_page });
    let head = pos2(page.center().x, page.center().y - page.height() * 0.15);
    ui.painter().circle_filled(head, page.height().min(page.width()) * 0.16, fg);
    let body = Rect::from_center_size(pos2(page.center().x, page.bottom() - page.height() * 0.17), vec2(page.width() * 0.6, page.height() * 0.32));
    ui.painter().rect_filled(body, 2.0, fg);
    let label = if portrait { tl!("Portrait") } else { tl!("Landscape") };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, label));
    resp.on_hover_text(label)
}

/// The Background Contents swatch.
fn background_swatch(app: &crate::PrismApp, ui: &mut egui::Ui, f: &Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
    let rgb = |c: [f32; 3]| Color32::from_rgb((c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8);
    match get_s(f, "background", "white").as_str() {
        "transparent" => {
            let half = r.width() / 2.0;
            for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let cell = Rect::from_min_size(r.min + vec2(i as f32 * half, j as f32 * half), vec2(half, half));
                ui.painter().rect_filled(cell, 0.0, if (i + j) % 2 == 0 { Color32::WHITE } else { Color32::from_gray(204) });
            }
        }
        "black" => {
            ui.painter().rect_filled(r, 2.0, Color32::BLACK);
        }
        "backgroundColor" => {
            let c = f.get("backgroundColor").and_then(|v| serde_json::from_value::<[f32; 3]>(v.clone()).ok());
            let bg = app.session.tools.background;
            ui.painter().rect_filled(r, 2.0, rgb(c.unwrap_or([bg[0], bg[1], bg[2]])));
        }
        other => {
            let hex = other.trim_start_matches('#');
            let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
            let c = match (byte(0), byte(2), byte(4)) {
                (Some(r), Some(g), Some(b)) => Color32::from_rgb(r, g, b),
                _ => Color32::WHITE,
            };
            ui.painter().rect_filled(r, 2.0, c);
        }
    }
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
}

/// Color Profile choices for `mode`: Don't Color Manage, the working space, then the built-in
/// profiles of the mode's colour space.
fn profile_options(app: &crate::PrismApp, mode: &str) -> Vec<(String, String)> {
    use prism_cms::ColorSpace;
    let (cm, space, kind) = match mode {
        "gray" => (prism_color::ColorMode::Grayscale, ColorSpace::Gray, "Gray"),
        "cmyk" => (prism_color::ColorMode::Cmyk, ColorSpace::Cmyk, "CMYK"),
        "lab" => return vec![("none".into(), tl!("Don't Color Manage").into())],
        _ => (prism_color::ColorMode::Rgb, ColorSpace::Rgb, "RGB"),
    };
    let working = app.session.color.working(cm);
    let mut v = vec![
        ("none".to_string(), tl!("Don't Color Manage").to_string()),
        ("working".to_string(), crate::i18n::fmt(tl!("Working {kind}: {profile}"), &[("kind", kind), ("profile", &working.description)])),
    ];
    for b in prism_cms::builtin::Builtin::ALL {
        if b.profile().color_space == space {
            v.push((b.id().to_string(), b.description().to_string()));
        }
    }
    v
}

fn details(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let saving = f.get("__savingPreset").and_then(Value::as_bool) == Some(true);
    ui.label(RichText::new(if saving { tl!("SAVE DOCUMENT PRESET") } else { tl!("PRESET DETAILS") }).size(12.0).color(t.text_dim).strong());
    ui.add_space(8.0);
    if saving {
        save_preset_form(app, ui, f);
    } else {
        ui.horizontal(|ui| {
            let mut name = get_s(f, "name", tl!("Untitled-1"));
            let edit = egui::TextEdit::singleline(&mut name).desired_width(DETAILS_W - 26.0 - 2.0 * FIELD_GAP).font(egui::FontId::proportional(14.0)).frame(egui::Frame::NONE);
            let r = ui.add(edit);
            ui.painter().line_segment([r.rect.left_bottom() + vec2(0.0, 3.0), r.rect.right_bottom() + vec2(0.0, 3.0)], Stroke::new(1.0, t.field_border));
            if r.changed() {
                f.insert("name".into(), json!(name));
            }
            if save_icon(ui).clicked() {
                f.insert("__savingPreset".into(), json!(true));
                f.insert("__presetName".into(), json!(crate::i18n::fmt(tl!("Saved {n}"), &[("n", &(app.session.presets.documents.len() + 1).to_string())])));
                f.remove("__presetError");
            }
        });
        ui.add_space(16.0);
    }
    let ppi = get_f(f, "resolution", 72.0);
    let mut unit = get_s(f, "__unit", "px");
    let field_w = 96.0;
    let menu_w = DETAILS_W - field_w - FIELD_GAP;
    small_label(ui, tl!("Width"));
    ui.horizontal(|ui| {
        let mut w = shown_size(f, "width", 1920.0, &unit, ppi);
        if widgets::value_field(ui, &mut w, 0.01..=300_000.0, "", field_w).changed() {
            set_size(f, "width", w, &unit, ppi);
        }
        let opts: Vec<(String, &str)> = UNITS.iter().map(|u| (u.0.to_string(), u.1)).collect();
        if widgets::dropdown(ui, "nd-unit", &mut unit, &opts, menu_w) {
            f.insert("__unit".into(), json!(unit));
            f.remove("__savedPreset");
            f.remove("__recent");
        }
    });
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        // Left-aligned column heads over the field, the Orientation buttons and the checkbox.
        let head = |ui: &mut egui::Ui, text: &str, w: f32| {
            ui.allocate_ui_with_layout(vec2(w, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(w);
                ui.label(RichText::new(text).size(11.5).color(t.text_dim));
            });
        };
        head(ui, tl!("Height"), field_w);
        head(ui, tl!("Orientation"), 84.0);
        head(ui, tl!("Artboards"), 60.0);
    });
    ui.horizontal(|ui| {
        let mut h = shown_size(f, "height", 1080.0, &unit, ppi);
        let r = widgets::value_field(ui, &mut h, 0.01..=300_000.0, "", field_w);
        if r.changed() {
            set_size(f, "height", h, &unit, ppi);
        }
        let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
        for portrait in [true, false] {
            if orientation_button(ui, portrait, (h > w) == portrait).clicked() && (h > w) != portrait {
                swap_size(f);
            }
        }
        ui.add_space(18.0);
        let mut on = f.get("artboards").and_then(Value::as_bool).unwrap_or(false);
        if ui.checkbox(&mut on, "").on_hover_text(tl!("Artboards")).changed() {
            f.insert("artboards".into(), json!(on));
            f.remove("__savedPreset");
            f.remove("__recent");
        }
    });
    ui.add_space(10.0);
    small_label(ui, tl!("Resolution"));
    ui.horizontal(|ui| {
        let per_cm = get_s(f, "__resUnit", "in") == "cm";
        let mut r = if per_cm { ppi / 2.54 } else { ppi };
        if widgets::value_field(ui, &mut r, 1.0..=30_000.0, "", field_w).changed() {
            set_resolution(f, if per_cm { r * 2.54 } else { r });
        }
        let mut ru = get_s(f, "__resUnit", "in");
        if widgets::dropdown(ui, "nd-resunit", &mut ru, &[("in".to_string(), tl!("Pixels/Inch")), ("cm".to_string(), tl!("Pixels/Centimeter"))], menu_w) {
            f.insert("__resUnit".into(), json!(ru));
            f.remove("__savedPreset");
            f.remove("__recent");
        }
    });
    ui.add_space(10.0);
    small_label(ui, tl!("Color Mode"));
    ui.horizontal(|ui| {
        let mut mode = get_s(f, "mode", "rgb");
        let modes = [
            ("gray".to_string(), tl!("Grayscale")),
            ("rgb".to_string(), tl!("RGB Color")),
            ("cmyk".to_string(), tl!("CMYK Color")),
            ("lab".to_string(), tl!("Lab Color")),
        ];
        if widgets::dropdown(ui, "nd-mode", &mut mode, &modes, DETAILS_W - 100.0 - FIELD_GAP) {
            f.insert("mode".into(), json!(mode));
            // A profile of the old mode's colour space can't tag the new one.
            let p = get_s(f, "profile", "none");
            if p != "none" && p != "working" {
                f.insert("profile".into(), json!("working"));
            }
            if mode == "lab" {
                f.insert("profile".into(), json!("none"));
            }
            f.remove("__savedPreset");
            f.remove("__recent");
        }
        let mut depth = f.get("depth").and_then(Value::as_u64).unwrap_or(8);
        if widgets::dropdown_with_tooltips(ui, "nd-depth", &mut depth, DEPTH_OPTIONS, 100.0) {
            f.insert("depth".into(), json!(depth));
            f.remove("__savedPreset");
            f.remove("__recent");
        }
    });
    ui.add_space(10.0);
    small_label(ui, tl!("Background Contents"));
    ui.horizontal(|ui| {
        let mut bg = get_s(f, "background", "white");
        let opts = [
            ("white".to_string(), tl!("White")),
            ("black".to_string(), tl!("Black")),
            ("backgroundColor".to_string(), tl!("Background Color")),
            ("transparent".to_string(), tl!("Transparent")),
        ];
        if widgets::dropdown(ui, "nd-bg", &mut bg, &opts, DETAILS_W - 32.0 - FIELD_GAP) {
            f.insert("background".into(), json!(bg));
            f.remove("__savedPreset");
            f.remove("__recent");
            // A fresh choice of Background Color uses today's toolbox colour. Selecting a
            // saved preset instead restores the colour captured when it was saved.
            f.remove("backgroundColor");
        }
        background_swatch(app, ui, f);
    });
    ui.add_space(16.0);
    // Advanced Options: open by default, like Photoshop.
    let open = f.get("__advanced").and_then(Value::as_bool).unwrap_or(true);
    let header = ui.add(
        egui::Label::new(RichText::new(format!("{}  {}", if open { "⌄" } else { "›" }, tl!("Advanced Options"))).size(12.0).color(t.text_dim)).sense(Sense::click()),
    );
    if header.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        f.insert("__advanced".into(), json!(!open));
    }
    if open {
        ui.add_space(10.0);
        small_label(ui, tl!("Color Profile"));
        let mode = get_s(f, "mode", "rgb");
        let opts = profile_options(app, &mode);
        let mut profile = get_s(f, "profile", "none");
        if !opts.iter().any(|(k, _)| *k == profile) {
            profile = "none".into();
        }
        let refs: Vec<(String, &str)> = opts.iter().map(|(k, l)| (k.clone(), l.as_str())).collect();
        if widgets::dropdown(ui, "nd-profile", &mut profile, &refs, DETAILS_W) {
            f.insert("profile".into(), json!(profile));
            f.remove("__savedPreset");
            f.remove("__recent");
        }
        ui.add_space(10.0);
        small_label(ui, tl!("Pixel Aspect Ratio"));
        // Prism's pixels are always square.
        let mut par = "square".to_string();
        widgets::dropdown(ui, "nd-par", &mut par, &[("square".to_string(), tl!("Square Pixels"))], DETAILS_W);
    }
}

fn save_preset_form(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    // Consume before TextEdit sees Enter, so naming can never confirm the outer dialog.
    let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let mut name = get_s(f, "__presetName", "");
    let r = ui.add(
        egui::TextEdit::singleline(&mut name).id_salt("document-preset-name").desired_width(DETAILS_W).font(egui::FontId::proportional(14.0)).frame(egui::Frame::NONE).char_limit(255),
    );
    ui.painter().line_segment([r.rect.left_bottom() + vec2(0.0, 3.0), r.rect.right_bottom() + vec2(0.0, 3.0)], Stroke::new(1.0, t.accent));
    let r = r.on_hover_text(tl!("Preset Name"));
    if r.changed() {
        f.insert("__presetName".into(), json!(name));
    }
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(DETAILS_W - 2.0 * 100.0 - 8.0);
        if widgets::primary_button(ui, tl!("Save Preset"), 100.0).clicked() || enter {
            match app.run("document.presets.save", json!({"name":name,"settings":settings_params(f)})) {
                Ok(_) => {
                    if let Some(preset) = app.session.presets.documents.last().cloned() {
                        apply_saved_preset(f, &preset);
                    }
                    f.insert("__category".into(), json!("Saved"));
                    f.remove("__savingPreset");
                    f.remove("__presetName");
                    f.remove("__presetError");
                    ui.ctx().request_repaint();
                }
                Err(e) => {
                    f.insert("__presetError".into(), json!(e.to_string()));
                }
            }
        }
        if widgets::secondary_button(ui, tl!("Cancel"), 100.0).clicked() {
            f.remove("__savingPreset");
            f.remove("__presetError");
        }
    });
    if let Some(error) = f.get("__presetError").and_then(Value::as_str) {
        ui.add(egui::Label::new(RichText::new(error).color(t.danger)).wrap());
    }
    ui.add_space(12.0);
}

pub fn body(app: &mut crate::PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let cat = get_s(f, "__category", "Recent");
    // Tabs.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        ui.add_space(8.0);
        for name in tabs() {
            let on = cat == name;
            let text = RichText::new(tl!(name)).size(14.0).color(if on { t.text } else { t.text_dim });
            let r = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if name == "Recent" {
                        let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        crate::icons::paint(ui, ir, "clock", 15.0, if on { t.text } else { t.text_dim });
                    }
                    ui.add(egui::Label::new(if on { text.strong() } else { text }).sense(Sense::click()))
                })
                .inner;
            if on {
                ui.painter().line_segment([r.rect.left_bottom() + vec2(if name == "Recent" { -22.0 } else { 0.0 }, 8.0), r.rect.right_bottom() + vec2(0.0, 8.0)], Stroke::new(2.0, t.text));
            }
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                select_tab(app, f, name);
            }
        }
    });
    ui.add_space(10.0);
    widgets::hairline(ui);
    let cat = get_s(f, "__category", "Recent");
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        // Left: the cards, on a darker ground.
        let (left, _) = ui.allocate_exact_size(vec2(GRID_W, GRID_H + 2.0 * GRID_PAD), Sense::hover());
        ui.painter().rect_filled(left, t.radius, t.canvas);
        // The cards keep GRID_PAD on every side; the scrollbar sits in the right padding.
        let inner = Rect::from_min_max(left.min + vec2(GRID_PAD, GRID_PAD), left.max - vec2(GRID_PAD - GRID_BAR, GRID_PAD));
        let mut grid_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
        egui::ScrollArea::vertical().id_salt(("nd-grid", cat.as_str())).max_height(GRID_H).auto_shrink([false, false]).show(&mut grid_ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            match cat.as_str() {
                "Recent" => recent_cards(app, ui, f),
                "Saved" => saved_cards(app, ui, f),
                c => {
                    let presets = CATEGORIES.iter().find(|x| x.0 == c).map_or(PHOTO, |x| x.1);
                    preset_cards(ui, f, c, presets);
                }
            }
        });
        // Right: Preset Details (nothing on an empty Saved tab, like Photoshop), its heading level
        // with the grid's.
        ui.add_space(COLUMN_GAP);
        ui.vertical(|ui| {
            ui.set_width(DETAILS_W);
            ui.spacing_mut().item_spacing = vec2(FIELD_GAP, 4.0);
            ui.add_space(GRID_PAD);
            if can_create(f) {
                details(app, ui, f);
            }
        });
    });
    // Create / Close sit clear of the grid, as in Photoshop.
    ui.add_space(10.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(cat: &str, name: &str) -> Preset {
        *CATEGORIES.iter().find(|c| c.0 == cat).unwrap().1.iter().find(|p| p.0 == name).unwrap()
    }

    #[test]
    fn units_round_trip_through_pixels() {
        assert_eq!(to_unit(2100.0, "in", 300.0), 7.0);
        assert_eq!(from_unit(7.0, "in", 300.0), 2100.0);
        assert_eq!(from_unit(2.54, "cm", 300.0), 300.0);
        assert_eq!(to_unit(640.0, "px", 72.0), 640.0);
    }

    #[test]
    fn photoshop_preset_counts_and_sizes() {
        let counts: Vec<(&str, usize)> = CATEGORIES.iter().map(|c| (c.0, c.1.len())).collect();
        assert_eq!(counts, vec![("Photo", 9), ("Print", 14), ("Art & Illustration", 6), ("Web", 9), ("Mobile", 28), ("Film & Video", 25)]);
        assert_eq!(preset_px(&find("Photo", "Default Photoshop Size")), (1890, 1417));
        assert_eq!(preset_px(&find("Print", "Letter")), (2550, 3300));
        assert_eq!(preset_px(&find("Print", "A4")), (2480, 3508));
        assert_eq!(preset_px(&find("Web", "Web Most Common")), (1366, 768));
        assert_eq!(tabs().collect::<Vec<_>>(), vec!["Recent", "Saved", "Photo", "Print", "Art & Illustration", "Web", "Mobile", "Film & Video"]);
        assert_eq!(size_label(8.5, 11.0, "in", 300.0), "8.5 x 11 in @ 300 ppi");
        assert_eq!(size_label(210.0, 297.0, "mm", 300.0), "210 x 297 mm @ 300 ppi");
    }

    #[test]
    fn a_preset_shows_its_own_numbers_and_category_options() {
        let mut f = crate::state::UiState::new_document_fields();
        apply_preset(&mut f, "Print", &find("Print", "A4"));
        assert_eq!(f["__unit"], "mm");
        assert_eq!((shown_size(&f, "width", 0.0, "mm", 300.0), shown_size(&f, "height", 0.0, "mm", 300.0)), (210.0, 297.0));
        assert_eq!((f["artboards"].clone(), f["profile"].clone()), (json!(false), json!("working")));
        apply_preset(&mut f, "Web", &find("Web", "Web Large"));
        assert_eq!((f["artboards"].clone(), f["profile"].clone(), f["__unit"].clone()), (json!(true), json!("none"), json!("px")));
        assert!(!f.contains_key("__widthTyped"));
        apply_preset(&mut f, "Film & Video", &find("Film & Video", "HDTV 1080p"));
        assert_eq!((f["artboards"].clone(), f["profile"].clone()), (json!(false), json!("none")));
    }

    /// #1147: 5 mm at 72 ppi is 14 px, which is 4.94 mm. The field must keep showing the typed 5.
    #[test]
    fn typed_size_in_a_physical_unit_shows_as_typed() {
        let mut f = crate::state::UiState::new_document_fields();
        set_size(&mut f, "height", 5.0, "mm", 72.0);
        assert_eq!(command_params(&f)["height"], json!(14));
        assert_eq!(shown_size(&f, "height", 1080.0, "mm", 72.0), 5.0);
        // Another unit, or pixels changed elsewhere (a preset, the orientation swap): converted.
        assert_eq!(shown_size(&f, "height", 1080.0, "cm", 72.0), to_unit(14.0, "cm", 72.0));
        f.insert("height".into(), json!(1080));
        assert_eq!(shown_size(&f, "height", 1080.0, "mm", 72.0), to_unit(1080.0, "mm", 72.0));
        // Pixels show whole pixels.
        set_size(&mut f, "width", 512.4, "px", 72.0);
        assert_eq!(shown_size(&f, "width", 1920.0, "px", 72.0), 512.0);
        // Over the 300000 px limit the field shows the limit.
        set_size(&mut f, "width", 300_000.0, "mm", 72.0);
        assert_eq!(command_params(&f)["width"], json!(300_000));
        assert_eq!(shown_size(&f, "width", 1920.0, "mm", 72.0), to_unit(300_000.0, "mm", 72.0));
    }

    /// #1147: the Orientation buttons swap the typed values too: 841 x 1189 mm becomes 1189 x 841.
    #[test]
    fn orientation_swap_keeps_the_typed_values() {
        let mut f = crate::state::UiState::new_document_fields();
        set_size(&mut f, "width", 841.0, "mm", 72.0);
        set_size(&mut f, "height", 1189.0, "mm", 72.0);
        swap_size(&mut f);
        let p = command_params(&f);
        assert_eq!((p["width"].clone(), p["height"].clone()), (json!(3370), json!(2384)));
        assert_eq!((shown_size(&f, "width", 1920.0, "mm", 72.0), shown_size(&f, "height", 1080.0, "mm", 72.0)), (1189.0, 841.0));
    }

    #[test]
    fn resolution_keeps_physical_size_in_physical_units_and_pixels_in_px() {
        let a4 = find("Print", "A4");
        let mut f = crate::state::UiState::new_document_fields();
        apply_preset(&mut f, "Print", &a4);
        set_resolution(&mut f, 150.0);
        let p = command_params(&f);
        assert_eq!((p["width"].clone(), p["height"].clone(), p["resolution"].clone()), (json!(1240), json!(1754), json!(150.0)));
        assert!(!f.contains_key("__preset"));

        let mut f = crate::state::UiState::new_document_fields();
        apply_preset(&mut f, "Print", &a4);
        f.insert("__unit".into(), json!("px"));
        set_resolution(&mut f, 150.0);
        let p = command_params(&f);
        assert_eq!((p["width"].clone(), p["height"].clone(), p["resolution"].clone()), (json!(2480), json!(3508), json!(150.0)));
    }

    #[test]
    fn preset_card_titles_fit_the_card_in_every_language() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for lang in crate::i18n::Lang::all() {
                for p in CATEGORIES.iter().flat_map(|c| c.1.iter()) {
                    let title = crate::i18n::tr(lang, p.0);
                    let g = card_title(ui.painter(), title, CARD.x - 16.0, Color32::WHITE);
                    assert!(g.size().x <= CARD.x - 16.0 && g.rows.len() <= 2 && !g.elided, "{}: {title}", lang.code());
                }
            }
        });
        out.textures_delta.clear();
    }

    #[test]
    fn new_document_depth_labels_and_tooltips_are_translated() {
        for lang in crate::i18n::Lang::all().filter(|lang| lang.code() != "en") {
            for (_, label, tooltip) in DEPTH_OPTIONS {
                assert_ne!(crate::i18n::tr(lang, label), *label, "{}: {label}", lang.code());
                assert_ne!(crate::i18n::tr(lang, tooltip), *tooltip, "{}: {tooltip}", lang.code());
            }
        }
    }

    #[test]
    fn clipboard_preset_comes_first_and_is_selected() {
        // Nothing on the clipboard: the dialog opens as before.
        let mut app = crate::PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let f = app.new_document_fields();
        assert_eq!(f, crate::state::UiState::new_document_fields());
        assert!(clipboard_preset(&f).is_none());
        // Pixels copied in the app: the Clipboard preset takes their size, at 72 ppi, selected.
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 20, "width": 123, "height": 45})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let f = app.new_document_fields();
        assert_eq!(clipboard_preset(&f), Some(clipboard(123, 45)));
        assert_eq!((f["width"].as_u64(), f["height"].as_u64(), f["__preset"].as_str()), (Some(123), Some(45), Some(CLIPBOARD)));
        let p = command_params(&f);
        assert!(p.get("__clipboard").is_none(), "file.new never sees the dialog's keys");
        assert_eq!((p["width"].as_u64(), p["height"].as_u64(), p["resolution"].as_f64()), (Some(123), Some(45), Some(72.0)));
    }

    #[test]
    fn the_newest_recent_item_is_selected_when_the_clipboard_is_empty() {
        let mut app = crate::PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("document.recent.add", json!({"name":"Letter","settings":{"width":2550,"height":3300,"resolution":300.0,"unit":"in"}})).unwrap();
        let f = app.new_document_fields();
        assert_eq!((f["width"].as_u64(), f["__recent"].as_u64(), f["__unit"].as_str()), (Some(2550), Some(0), Some("in")));
        assert_eq!(recent_name(&f), "Letter");
    }

    #[test]
    fn preset_sets_size_resolution_and_create_params_drop_ui_keys() {
        let mut f = crate::state::UiState::new_document_fields();
        apply_preset(&mut f, "Print", &find("Print", "A4"));
        let p = command_params(&f);
        assert_eq!(p["width"], 2480);
        assert_eq!(p["height"], 3508);
        assert_eq!(p["resolution"], 300.0);
        assert!(p.get("__preset").is_none() && p.get("__unit").is_none());
        let mut s = prism_engine::Session::new();
        s.execute("file.new", p).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height, d.resolution_dpi), (2480, 3508, 300.0));
        assert!(d.icc_profile.is_some(), "print presets embed the working space");
        // A Web preset makes an artboard.
        apply_preset(&mut f, "Web", &find("Web", "Web Minimum"));
        s.execute("file.new", command_params(&f)).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.artboards().len(), 1);
        assert!(d.icc_profile.is_none());
    }

    #[test]
    fn saved_selection_restores_all_settings_but_keeps_the_document_name() {
        let mut session = prism_engine::Session::new();
        session
            .execute(
                "document.presets.save",
                json!({"name":"Print proof","settings":{
                    "width":1600,"height":800,"resolution":254.0,"mode":"cmyk","depth":16,
                    "background":"backgroundColor","backgroundColor":[0.2,0.3,0.4],"unit":"mm","resolutionUnit":"cm"
                }}),
            )
            .unwrap();
        let preset = session.presets.documents[0].clone();
        let mut f = crate::state::UiState::new_document_fields();
        set_clipboard(&mut f, 20, 30);
        f.insert("name".into(), json!("Catalog cover"));
        apply_saved_preset(&mut f, &preset);
        assert_eq!(f["name"], "Catalog cover");
        assert_eq!(f["__unit"], "mm");
        assert_eq!(f["__resUnit"], "cm");
        assert_eq!(f["__savedPreset"], "Print proof");
        assert!(f.get("__preset").is_none());
        assert_eq!(clipboard_preset(&f), Some(clipboard(20, 30)));
        let mut expected = preset.settings.command_params();
        expected["name"] = json!("Catalog cover");
        expected["profile"] = json!("none");
        assert_eq!(command_params(&f), expected);
        f.insert("width".into(), json!(42));
        assert_eq!(session.presets.documents[0], preset, "editing the form does not edit the snapshot");
        apply_preset(&mut f, "Recent", &clipboard(20, 30));
        assert!(f.get("__savedPreset").is_none());
        assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(20), Some(30)));
    }

    #[test]
    fn empty_saved_tab_cannot_create() {
        let mut f = crate::state::UiState::new_document_fields();
        assert!(can_create(&f));
        f.insert("__category".into(), json!("Saved"));
        assert!(!can_create(&f));
        f.insert("__savedPreset".into(), json!("x"));
        assert!(can_create(&f));
    }

    /// The real dialog (#254): a typed size must reach `file.new`, however it is confirmed.
    mod dialog {
        use super::super::{CATEGORIES, apply_preset};
        use crate::PrismApp;
        use crate::state::{DialogKind, UiState};
        use egui::accesskit::Role;
        use egui_kittest::{Harness, kittest::{NodeT, Queryable}};
        use std::sync::{Arc, Mutex};

        /// The real preference services, backed by an in-memory store shared across app restarts.
        fn harness_with_store(store: &Arc<Mutex<Option<String>>>) -> Harness<'static, PrismApp> {
            let (read, write) = (store.clone(), store.clone());
            let services = crate::Services {
                load_prefs: Some(Box::new(move || read.lock().unwrap().clone())),
                save_prefs: Some(Box::new(move |text| {
                    *write.lock().unwrap() = Some(text.to_string());
                    Ok(())
                })),
                ..Default::default()
            };
            let app = PrismApp::new(prism_engine::Session::new(), services);
            let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_ui_state(
                |ui, app| {
                    crate::prefs_ui::tick(app, ui.ctx());
                    crate::dialogs::show(app, ui.ctx());
                },
                app,
            );
            PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run_steps(3);
            h
        }

        fn name_preset(h: &mut Harness<'static, PrismApp>, name: &str) {
            h.get_by_label("Save Document Preset").click();
            h.run_steps(2);
            // The field opens with a suggested name ("Saved 1"); replace it.
            h.state_mut().ui.dialogs[0].fields.insert("__presetName".into(), serde_json::json!(""));
            h.run_steps(1);
            h.get_by_role(Role::TextInput).click();
            h.run_steps(1);
            h.event(egui::Event::Text(name.into()));
            h.run_steps(1);
        }

        #[test]
        fn save_restart_select_create_and_delete_through_the_dialog() {
            // A user who has already been through the first-run setup (an existing preferences file).
            let store = Arc::new(Mutex::new(Some("{}".to_string())));
            let mut h = harness_with_store(&store);
            type_into(&mut h, 0, "1600");
            type_into(&mut h, 1, "1600");
            let mut f = fields(&h);
            f.insert("background".into(), serde_json::json!("transparent"));
            set_fields(&mut h, f);
            name_preset(&mut h, " Product square ");
            h.get_by_label("Save Preset").click();
            h.run_steps(3);
            assert!(h.state().session.documents().is_empty(), "saving does not create a document");
            assert_eq!(h.state().session.presets.documents[0].name, "Product square");
            assert_eq!(fields(&h)["__category"], "Saved", "the new preset shows under Saved");
            type_into(&mut h, 0, "32");
            assert_eq!(h.state().session.presets.documents[0].settings.width, 1600);
            drop(h);

            let mut h = harness_with_store(&store);
            h.get_by_label("Saved").click();
            h.run_steps(2);
            // Switching to Saved selects its first preset.
            assert_eq!(fields(&h)["__savedPreset"], "Product square");
            h.get_by_label("Product square").click();
            h.run_steps(2);
            assert!(h.state().session.documents().is_empty(), "selection only populates the form");
            assert_eq!(fields(&h)["name"], "Untitled-1");
            assert_eq!(fields(&h)["background"], "transparent");
            h.get_by_label("Create").click();
            h.run_steps(3);
            assert_eq!(created(&h), (1600, 1600, 72.0));
            let pixel = h.state_mut().session.execute("document.pixel", serde_json::json!({"x":0,"y":0})).unwrap();
            assert_eq!(pixel[3], 0.0);
            // Created from the dialog: remembered under Recent by the preset's name.
            let recent = &h.state().session.presets.recent_documents;
            assert_eq!((recent.len(), recent[0].name.as_str(), recent[0].settings.width), (1, "Product square", 1600));

            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run_steps(2);
            h.get_by_label("Saved").click();
            h.run_steps(2);
            h.get_by_label("Product square").click_secondary();
            h.run_steps(2);
            h.get_by_label("Delete Preset").click();
            h.run_steps(3);
            drop(h);
            let mut h = harness_with_store(&store);
            assert_eq!(h.state().session.presets.recent_documents.len(), 1, "recent items survive a restart");
            h.get_by_label("Saved").click();
            h.run_steps(2);
            assert!(h.state().session.presets.documents.is_empty());
            assert!(h.query_by_label("No saved presets yet.").is_some());
        }

        #[test]
        fn enter_saves_the_preset_and_bad_names_leave_the_form_open() {
            let mut h = harness();
            name_preset(&mut h, "Enter test");
            enter(&mut h);
            assert_eq!(h.state().session.presets.documents.len(), 1);
            assert!(h.state().session.documents().is_empty(), "Enter must not also press Create");
            assert_eq!(h.state().ui.dialogs.len(), 1);
            name_preset(&mut h, " ENTER TEST ");
            h.get_by_label("Save Preset").click();
            h.run_steps(2);
            assert!(fields(&h).get("__presetError").is_some());
            assert_eq!(h.state().session.presets.documents.len(), 1);
            assert!(h.state().session.documents().is_empty());
            h.get_by_label("Cancel").click();
            h.run_steps(2);
            name_preset(&mut h, "");
            h.state_mut().ui.dialogs[0].fields.insert("__presetName".into(), serde_json::json!("   "));
            h.run_steps(2);
            h.get_by_label("Save Preset").click();
            h.run_steps(2);
            assert!(fields(&h).get("__presetError").is_some());
            assert_eq!(h.state().session.presets.documents.len(), 1);
        }

        #[test]
        fn empty_saved_tab_disables_create() {
            let mut h = harness();
            h.get_by_label("Saved").click();
            h.run_steps(2);
            assert!(h.query_by_label("Width").is_none(), "no details without a preset");
            assert!(h.get_by_label("Create").accesskit_node().is_disabled());
            enter(&mut h);
            assert!(h.state().session.documents().is_empty(), "Enter doesn't create either");
        }

        fn harness() -> Harness<'static, PrismApp> {
            let app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
            let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
            PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            h.run_steps(3);
            h
        }

        fn click_at(h: &mut Harness<'static, PrismApp>, at: egui::Pos2) {
            h.hover_at(at);
            h.run_steps(1);
            h.drag_at(at);
            h.run_steps(1);
            h.drop_at(at);
            h.run_steps(2);
        }

        /// The Width (0), Height (1) and Resolution (2) fields.
        fn field(h: &Harness<'static, PrismApp>, i: usize) -> egui::Rect {
            h.query_all_by_role(Role::SpinButton).nth(i).map(|n| n.rect()).expect("a size field")
        }

        /// Click into field `i` (which selects its text) and type `text`, as a user does.
        fn type_into(h: &mut Harness<'static, PrismApp>, i: usize, text: &str) {
            let r = field(h, i);
            click_at(h, r.center());
            for c in text.chars() {
                h.event(egui::Event::Text(c.to_string()));
                h.run_steps(1);
            }
        }

        fn fields(h: &Harness<'static, PrismApp>) -> serde_json::Map<String, serde_json::Value> {
            h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
        }

        fn set_fields(h: &mut Harness<'static, PrismApp>, f: serde_json::Map<String, serde_json::Value>) {
            h.state_mut().ui.dialogs[0].fields = f;
            h.run_steps(2);
        }

        fn enter(h: &mut Harness<'static, PrismApp>) {
            h.key_press(egui::Key::Enter);
            h.run_steps(3);
        }

        fn created(h: &Harness<'static, PrismApp>) -> (u32, u32, f32) {
            assert!(h.state().ui.dialogs.is_empty(), "the dialog closed");
            let d = &h.state().session.active().expect("a new document").doc;
            (d.size.width, d.size.height, d.resolution_dpi)
        }

        #[test]
        fn typed_size_then_enter_creates_that_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "512");
            let f = fields(&h);
            assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(512), Some(512)), "whole pixels: {f:?}");
            enter(&mut h);
            assert_eq!(created(&h), (512, 512, 72.0));
            assert_eq!(h.state().session.presets.recent_documents[0].name, "Custom");
        }

        #[test]
        fn typed_size_then_create_without_leaving_the_field_creates_that_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "300");
            // Still editing Height: click Create straight away.
            let create = h.get_by_label("Create").rect();
            click_at(&mut h, create.center());
            assert_eq!(created(&h), (512, 300, 72.0));
        }

        #[test]
        fn typing_over_a_preset_wins() {
            let mut h = harness();
            let mut f = fields(&h);
            let web = CATEGORIES.iter().find(|c| c.0 == "Web").unwrap().1.iter().find(|p| p.0 == "Web Minimum").unwrap();
            apply_preset(&mut f, "Web", web);
            set_fields(&mut h, f);
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "512");
            assert!(fields(&h).get("__preset").is_none(), "typing deselects the preset");
            enter(&mut h);
            assert_eq!(created(&h), (512, 512, 72.0));
        }

        #[test]
        fn the_clipboard_card_is_first_and_creates_the_clipboard_size() {
            let mut h = harness();
            let mut f = fields(&h);
            super::super::set_clipboard(&mut f, 640, 360);
            set_fields(&mut h, f);
            assert!(h.query_by_label_contains("YOUR RECENT ITEMS (1)").is_some());
            h.get_by_label("Photo").click();
            h.run_steps(2);
            assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some("Default Photoshop Size"), "a tab selects its first card");
            h.get_by_label("Recent").click();
            h.run_steps(2);
            assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some(super::super::CLIPBOARD));
            enter(&mut h);
            assert_eq!(created(&h), (640, 360, 72.0));
        }

        #[test]
        fn clicking_a_preset_card_after_typing_sets_its_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            h.get_by_label("Photo").click();
            h.run_steps(2);
            h.get_by_label("Landscape, 6 x 4").click();
            h.run_steps(2);
            assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some("Landscape, 6 x 4"));
            enter(&mut h);
            assert_eq!(created(&h), (1800, 1200, 300.0));
            assert_eq!(h.state().session.presets.recent_documents[0].name, "Landscape, 6 x 4");
        }

        #[test]
        fn a_recent_item_card_recreates_its_document() {
            let mut h = harness();
            h.state_mut().session.execute("document.recent.add", serde_json::json!({"settings":{"width":777,"height":333,"resolution":144.0}})).unwrap();
            h.get_by_label("Photo").click();
            h.run_steps(2);
            h.get_by_label("Recent").click();
            h.run_steps(2);
            assert_eq!(fields(&h).get("__recent").and_then(|v| v.as_u64()), Some(0));
            h.get_by_label("Custom").click();
            h.run_steps(2);
            enter(&mut h);
            assert_eq!(created(&h), (777, 333, 144.0));
        }

        #[test]
        fn typed_size_in_inches_converts_at_the_resolution() {
            let mut h = harness();
            type_into(&mut h, 2, "300");
            let mut f = fields(&h);
            f.insert("__unit".into(), serde_json::json!("in"));
            set_fields(&mut h, f);
            type_into(&mut h, 0, "2");
            type_into(&mut h, 1, "1.5");
            enter(&mut h);
            assert_eq!(created(&h), (600, 450, 300.0));
        }

        /// #1147: typed digit by digit, millimetres used to be rewritten after each keystroke
        /// (`8` became `8.1`), so 841 never arrived.
        #[test]
        fn typed_size_in_millimetres_keeps_every_digit() {
            let mut h = harness();
            let mut f = fields(&h);
            f.insert("__unit".into(), serde_json::json!("mm"));
            set_fields(&mut h, f);
            type_into(&mut h, 0, "841");
            type_into(&mut h, 1, "1189");
            enter(&mut h);
            assert_eq!(created(&h), (2384, 3370, 72.0));
        }

        #[test]
        fn changing_units_keeps_the_typed_pixel_size() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "512");
            for unit in ["in", "cm", "mm", "pt", "pica", "px"] {
                let mut f = fields(&h);
                f.insert("__unit".into(), serde_json::json!(unit));
                set_fields(&mut h, f);
            }
            enter(&mut h);
            assert_eq!(created(&h), (512, 512, 72.0));
        }

        #[test]
        fn orientation_swap_keeps_whole_pixels() {
            let mut h = harness();
            type_into(&mut h, 0, "512");
            type_into(&mut h, 1, "256");
            h.get_by_label("Portrait").click();
            h.run_steps(2);
            enter(&mut h);
            assert_eq!(created(&h), (256, 512, 72.0));
        }
    }
}
