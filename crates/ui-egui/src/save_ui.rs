//! File › Save, Save As and Save a Copy: one window for the editable original (PSD, PSB, .prism)
//! and for images (PNG, JPEG, …), with a searchable format list, per-layer saving and options
//! folded away under Advanced. Its state lives in the dialog's fields, so automation can set them
//! (`ui.dialog.set`) and confirm like a user.
//!
//! Saving an image never rebinds the document: it stays tied to its original, so the next ⌘S
//! can't flatten layers over it. Only an original format saved from Save / Save As becomes the
//! document's file.

use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::state::DialogKind;
use crate::theme::Tokens;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    /// Editable: keeps every layer.
    Original,
    /// The everyday image formats.
    Image,
    /// Folded under "More formats" until searched for.
    Other,
}

struct Fmt {
    key: &'static str,
    label: &'static str,
    group: Group,
    /// One-line hint (English source; shown through `tl!`).
    note: &'static str,
    /// Extra search words.
    aliases: &'static str,
}

const FORMATS: &[Fmt] = &[
    Fmt { key: "psd", label: "PSD", group: Group::Original, note: "Keeps layers", aliases: "photoshop original layers 원본 레이어" },
    Fmt { key: "psb", label: "PSB", group: Group::Original, note: "Large document", aliases: "photoshop large big 대용량" },
    Fmt { key: "prism", label: "Prism", group: Group::Original, note: "Prism native file", aliases: "native pcraft 원본" },
    Fmt { key: "png", label: "PNG", group: Group::Image, note: "Lossless, keeps transparency", aliases: "portable transparent 투명" },
    Fmt { key: "jpg", label: "JPEG", group: Group::Image, note: "Photos, small files", aliases: "jpeg jpe jfif photo 사진" },
    Fmt { key: "webp", label: "WebP", group: Group::Image, note: "Web, keeps transparency", aliases: "web 웹" },
    Fmt { key: "tif", label: "TIFF", group: Group::Image, note: "Print, can keep layers", aliases: "tiff print 인쇄" },
    Fmt { key: "avif", label: "AVIF", group: Group::Other, note: "Small web images", aliases: "av1 web" },
    Fmt { key: "gif", label: "GIF", group: Group::Other, note: "256 colors", aliases: "animation" },
    Fmt { key: "bmp", label: "BMP", group: Group::Other, note: "", aliases: "bitmap dib" },
    Fmt { key: "tga", label: "Targa", group: Group::Other, note: "", aliases: "tga" },
    Fmt { key: "exr", label: "OpenEXR", group: Group::Other, note: "32-bit HDR", aliases: "hdr float" },
    Fmt { key: "hdr", label: "Radiance HDR", group: Group::Other, note: "", aliases: "rgbe" },
    Fmt { key: "qoi", label: "QOI", group: Group::Other, note: "", aliases: "" },
    Fmt { key: "ico", label: "ICO", group: Group::Other, note: "Icon", aliases: "icon favicon 아이콘" },
    Fmt { key: "pnm", label: "Netpbm", group: Group::Other, note: "", aliases: "pbm pgm ppm pam" },
];

/// JPEG / WebP quality per named choice: (JPEG, WebP, JPEG at 4:4:4).
const QUALITY: [(&str, &str, u8, u8, bool); 4] =
    [("best", "Highest", 95, 95, true), ("high", "High", 85, 82, false), ("web", "For web", 75, 72, false), ("small", "Small", 60, 55, false)];

fn fmt_of(key: &str) -> Option<&'static Fmt> {
    FORMATS.iter().find(|f| f.key == key)
}

fn original(key: &str) -> bool {
    fmt_of(key).is_some_and(|f| f.group == Group::Original)
}

/// Whether this build writes `key`.
fn writable(key: &str) -> bool {
    original(key) || prism_codecs::from_extension(key).is_some_and(|f| f.caps().write)
}

/// Whether the list offers `key`: writable, and an image format when saving each layer as a file
/// (a layer on its own is an image; originals are for the whole document).
fn offered(key: &str, per_layer: bool) -> bool {
    writable(key) && !(per_layer && original(key))
}

/// The format key for a typed extension (`jpeg` → `jpg`, `tiff` → `tif`).
fn key_for_extension(ext: &str) -> Option<&'static str> {
    let ext = ext.to_ascii_lowercase();
    let key = match ext.as_str() {
        "jpeg" | "jpe" | "jfif" => "jpg",
        "tiff" => "tif",
        "pcraft" => "prism",
        "pbm" | "pgm" | "ppm" | "pam" => "pnm",
        e => e,
    };
    fmt_of(key).map(|f| f.key).filter(|k| writable(k))
}

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__save")
}

fn s<'a>(f: &'a Map<String, Value>, k: &str, default: &'a str) -> &'a str {
    f.get(k).and_then(Value::as_str).unwrap_or(default)
}

fn b(f: &Map<String, Value>, k: &str, default: bool) -> bool {
    f.get(k).and_then(Value::as_bool).unwrap_or(default)
}

fn n(f: &Map<String, Value>, k: &str, default: f64) -> f64 {
    f.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(default)
}

fn copy_mode(f: &Map<String, Value>) -> bool {
    s(f, "__save", "save") == "copy"
}

/// The options every image format starts with (Advanced).
fn option_defaults() -> Map<String, Value> {
    let v = json!({"pngEffort": "default", "quality": "high", "jpegQuality": 85, "webpQuality": 82, "chroma444": false, "tiffLayers": true,
        "icc": true, "metadata": true, "layers": "visible", "groups": "merge", "trim": false, "pattern": "{layer}"});
    v.as_object().cloned().unwrap_or_default()
}

const OPTION_KEYS: [&str; 12] = ["pngEffort", "quality", "jpegQuality", "webpQuality", "chroma444", "tiffLayers", "icc", "metadata", "layers", "groups", "trim", "pattern"];

fn home() -> Option<std::path::PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir()
}

fn parent(path: &str) -> Option<String> {
    path.rfind(['/', '\\']).map(|i| path[..i].to_string()).filter(|p| !p.is_empty())
}

fn file_stem(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() || dir == "." {
        name.to_string()
    } else if dir.ends_with('/') || dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Opens the Save window for the active document. `copy`: Save a Copy (starts on the last image
/// format; nothing is rebound). Otherwise Save / Save As (starts on the original format).
pub fn open(app: &mut PrismApp, copy: bool) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let prefs = app.ui.save.clone();
    let own = st.path.as_deref().and_then(|p| p.rsplit_once('.')).and_then(|(_, e)| key_for_extension(e));
    let recent = |want: fn(&str) -> bool| prefs.recent_formats.iter().find(|k| want(k) && writable(k)).cloned();
    let format = if copy {
        recent(|k| !original(k)).unwrap_or_else(|| "png".into())
    } else {
        own.filter(|k| original(k)).map(str::to_string).or_else(|| recent(original)).unwrap_or_else(|| "psd".into())
    };
    let mut name = st.path.as_deref().map_or_else(|| file_stem(&st.doc.name), file_stem);
    if copy && original(&format) {
        name = format!("{name} {}", tl!("copy"));
    }
    let dir = st
        .path
        .as_deref()
        .and_then(parent)
        .or_else(|| prefs.recent_dirs.first().cloned())
        .or_else(|| home().map(|h| h.join("Desktop").to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".into());
    let mut f = option_defaults();
    if let Some(saved) = prefs.options.get("image") {
        for k in OPTION_KEYS {
            if let Some(v) = saved.get(k) {
                f.insert(k.into(), v.clone());
            }
        }
    }
    f.insert("__save".into(), json!(if copy { "copy" } else { "save" }));
    f.insert("__label".into(), json!(if copy { "Save a Copy…" } else { "Save As…" }));
    f.insert("name".into(), json!(name));
    f.insert("dir".into(), json!(dir));
    f.insert("format".into(), json!(format));
    f.insert("perLayer".into(), json!(false));
    let id = app.ui.open_dialog(DialogKind::Command, f);
    Ok(json!({ "dialog": id }))
}

/// The layers a per-layer save writes, as `file.export.layersToFiles` picks them.
fn layers_to_save(doc: &prism_doc::Document, selected: &[prism_doc::LayerId], f: &Map<String, Value>) -> usize {
    use prism_doc::{Layer, LayerContent};
    fn walk(layers: &[Layer], each: bool, shown: bool, depth: usize, out: &mut Vec<(prism_doc::LayerId, bool)>) {
        for l in layers.iter().rev() {
            match &l.content {
                LayerContent::Adjustment(_) => {}
                LayerContent::Group(g) if each && depth < 64 => walk(&g.children, each, shown && l.visible, depth + 1, out),
                _ => out.push((l.id, shown && l.visible)),
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.layers, s(f, "groups", "merge") == "each", true, 0, &mut out);
    out.iter()
        .filter(|(id, shown)| match s(f, "layers", "visible") {
            "selected" => selected.contains(id),
            "all" => true,
            _ => *shown,
        })
        .count()
}

/// How many layers a flat save merges (one lone layer merges nothing).
fn layer_count(doc: &prism_doc::Document) -> usize {
    fn count(layers: &[prism_doc::Layer], depth: usize) -> usize {
        layers
            .iter()
            .map(|l| match &l.content {
                prism_doc::LayerContent::Group(g) if depth < 64 => count(&g.children, depth + 1),
                _ => 1,
            })
            .sum()
    }
    count(&doc.layers, 0)
}

/// The export settings the fields describe.
fn settings(f: &Map<String, Value>) -> crate::ExportSettings {
    let format = s(f, "format", "psd");
    let quality = QUALITY.iter().find(|q| q.0 == s(f, "quality", "high"));
    let (jpeg, webp, chroma) = match quality {
        Some(q) => (q.2, q.3, q.4),
        None => (n(f, "jpegQuality", 85.0).clamp(1.0, 100.0) as u8, n(f, "webpQuality", 82.0).clamp(1.0, 100.0) as u8, b(f, "chroma444", false)),
    };
    crate::ExportSettings {
        jpeg_quality: (format == "jpg").then_some(jpeg),
        webp_lossless: format != "webp",
        webp_quality: (format == "webp").then_some(webp),
        tiff_layers: b(f, "tiffLayers", true),
        xmp_all: b(f, "metadata", true),
        png_compression: match s(f, "pngEffort", "default") {
            "fast" => Some(prism_codecs::PngCompression::Fast),
            "best" => Some(prism_codecs::PngCompression::Best),
            "none" => Some(prism_codecs::PngCompression::None),
            _ => None,
        },
        jpeg_chroma_444: format == "jpg" && chroma,
        embed_icc: b(f, "icc", true),
    }
}

/// The file (or, per layer, the folder) the fields name.
fn target(f: &Map<String, Value>) -> String {
    let name = s(f, "name", "").trim().to_string();
    if b(f, "perLayer", false) { join(s(f, "dir", "."), &name) } else { join(s(f, "dir", "."), &format!("{name}.{}", s(f, "format", "psd"))) }
}

/// Why the fields can't be saved yet, if they can't.
fn problem(f: &Map<String, Value>) -> Option<&'static str> {
    let name = s(f, "name", "").trim();
    if name.is_empty() {
        Some(tl!("Enter a name"))
    } else if name.contains(['/', '\\']) {
        Some(tl!("A name can't contain slashes"))
    } else if !writable(s(f, "format", "")) {
        Some(tl!("Prism can't write this format"))
    } else if b(f, "perLayer", false) && original(s(f, "format", "")) {
        Some(tl!("Each layer is saved as an image: pick an image format"))
    } else {
        None
    }
}

/// The one line the window shows under the format: what this save loses or replaces.
fn warning(app: &PrismApp, f: &Map<String, Value>) -> Option<String> {
    let st = app.session.active()?;
    let format = s(f, "format", "psd");
    if b(f, "perLayer", false) {
        let count = layers_to_save(&st.doc, &st.selected_layers, f);
        return Some(if count == 0 {
            tl!("There are no layers to save with these settings").to_string()
        } else {
            crate::i18n::fmt(tl!("Saves {count} layers as separate files in this folder"), &[("count", &count.to_string())])
        });
    }
    let mut lines = Vec::new();
    let layers = layer_count(&st.doc);
    let keeps_layers = original(format) || (format == "tif" && b(f, "tiffLayers", true));
    if !keeps_layers && layers > 1 {
        lines.push(crate::i18n::fmt(tl!("{count} layers are merged into one image. The open document stays as it is."), &[("count", &layers.to_string())]));
    }
    if format == "jpg" {
        lines.push(tl!("JPEG doesn't keep transparency.").to_string());
    }
    let depth = st.doc.pixel_format().sample;
    if depth != prism_color::SampleType::U8 && matches!(format, "jpg" | "webp" | "gif" | "bmp" | "ico" | "qoi" | "avif") {
        lines.push(tl!("This format saves 8 bits per channel.").to_string());
    }
    if std::path::Path::new(&target(f)).exists() {
        lines.push(tl!("A file with this name already exists and will be replaced.").to_string());
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// Estimated file size from a small proxy encode, cached until the document or options change.
fn estimate(app: &PrismApp, ui: &egui::Ui, f: &Map<String, Value>) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let st = app.session.active()?;
    let export = app.services.export.as_ref()?;
    let format = s(f, "format", "psd");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (st.doc.id.0, st.revision, format, serde_json::to_string(&settings_key(f)).unwrap_or_default()).hash(&mut h);
    let key = egui::Id::new(("save-estimate", h.finish()));
    if let Some(v) = ui.data(|d| d.get_temp::<Option<u64>>(key)) {
        return v;
    }
    let doc = &st.doc;
    let long = doc.size.width.max(doc.size.height).max(1) as f64;
    let mut proxy: prism_doc::Document = (**doc).clone();
    if long > 512.0 {
        let mut s = prism_engine::Session::new();
        s.add_document(proxy.clone(), None);
        let scale = 512.0 / long;
        let (w, h) = ((doc.size.width as f64 * scale).round().max(1.0), (doc.size.height as f64 * scale).round().max(1.0));
        s.execute("image.imageSize", json!({"width": w, "height": h, "resample": "bilinear"})).ok()?;
        proxy = (*s.active()?.doc).clone();
    }
    let value = export(&proxy, &format!("estimate.{format}"), &settings(f)).ok().map(|(bytes, _)| {
        let full = doc.size.width as f64 * doc.size.height as f64;
        let small = (proxy.size.width as f64 * proxy.size.height as f64).max(1.0);
        (bytes.len() as f64 * full / small) as u64
    });
    ui.data_mut(|d| d.insert_temp(key, value));
    value
}

fn settings_key(f: &Map<String, Value>) -> Map<String, Value> {
    OPTION_KEYS.iter().filter_map(|k| f.get(*k).map(|v| (k.to_string(), v.clone()))).collect()
}

fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1.0 { format!("{mb:.1} MB") } else { format!("{:.0} KB", (bytes as f64 / 1024.0).max(1.0)) }
}

/// The row label, left-aligned in the label column: rows start on the same edge as the title
/// and the options below them.
fn label(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    // A fixed-width cell, so every row's control starts on the same line.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(label_width(ui), 24.0), egui::Sense::hover());
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, text, font, t.text_dim);
}

/// Every row label the window can show: the label column is as wide as the longest of them in
/// the UI language, so it is snug but doesn't move when Advanced opens.
const ROW_LABELS: [&str; 13] =
    ["Name", "Folder", "Location", "Format", "File size", "Quality", "Layers", "Include", "Groups", "Size", "File names", "Compression", "Color detail"];

fn label_key() -> egui::Id {
    egui::Id::new("save-label-width")
}

/// Measures the label column for this frame (see [`ROW_LABELS`]).
fn measure_labels(ui: &egui::Ui) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let widest =
        ROW_LABELS.iter().map(|l| ui.painter().layout_no_wrap(tl!(l).to_string(), font.clone(), egui::Color32::WHITE).size().x).fold(0.0_f32, f32::max);
    let width = (widest.ceil() + 2.0).clamp(32.0, 140.0);
    ui.data_mut(|d| d.insert_temp(label_key(), width));
}

/// Width of the label column (the controls line up after it).
fn label_width(ui: &egui::Ui) -> f32 {
    ui.data(|d| d.get_temp::<f32>(label_key())).unwrap_or(72.0)
}

/// Where the controls start: past the label column and its gap.
fn column(ui: &egui::Ui) -> f32 {
    label_width(ui) + ui.spacing().item_spacing.x
}
/// The window's content width.
pub const WIDTH: f32 = 420.0;

fn footer_key() -> egui::Id {
    egui::Id::new("save-footer")
}

/// The button row's left side: the estimated file size, beside Cancel and Save.
pub fn footer(ui: &mut egui::Ui) {
    let Some(text) = ui.data(|d| d.get_temp::<Option<String>>(footer_key())).flatten() else { return };
    let t = Tokens::get(ui.ctx());
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new(text).size(11.5).color(t.text_faint));
    });
}

fn note(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(column(ui));
        ui.add(egui::Label::new(egui::RichText::new(text).size(11.5).color(t.text_faint)).wrap());
    });
}

/// A row of named choices (pill tabs); returns whether one was picked.
fn choices(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, options: &[(&str, &str)]) -> bool {
    let current = s(f, key, "").to_string();
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (value, text) in options {
            if crate::widgets::pill_tab(ui, text, current == *value).clicked() {
                picked = Some(value.to_string());
            }
        }
    });
    if let Some(v) = picked {
        f.insert(key.into(), json!(v));
        return true;
    }
    false
}

/// Advanced / Direct settings fold header: a turning chevron and the title.
fn fold(ui: &mut egui::Ui, t: &Tokens, open: &mut bool, text: &str) {
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let (rect, icon) = ui.allocate_exact_size(egui::vec2(12.0, 16.0), egui::Sense::click());
            let c = rect.center();
            let points = if *open {
                vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
            } else {
                vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(points, t.text_dim, egui::Stroke::NONE));
            let l = ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_dim)).selectable(false).sense(egui::Sense::click()));
            icon.union(l)
        })
        .inner;
    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        *open = !*open;
    }
}

/// The searchable format list: recent formats, then the original and image groups, then the rest
/// folded away until searched for or asked for. Enter takes the first match.
fn format_picker(app: &PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let current = s(f, "format", "psd").to_string();
    let per_layer = b(f, "perLayer", false);
    let shown = fmt_of(&current).map_or(current.to_uppercase(), |m| m.label.to_string());
    let search_id = ui.id().with("save-format-search");
    let mut picked: Option<&'static str> = None;
    egui::ComboBox::from_id_salt("save-format")
        .selected_text(shown)
        .width(ui.available_width())
        .height(440.0)
        .icon(crate::widgets::chevron_icon)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            ui.set_min_width(300.0);
            let (pass_id, more_id) = (search_id.with("pass"), search_id.with("more"));
            let pass = ui.ctx().cumulative_pass_nr();
            let opened = ui.data(|d| d.get_temp::<u64>(pass_id)).is_none_or(|p| p.saturating_add(1) < pass);
            let mut q: String = if opened { String::new() } else { ui.data(|d| d.get_temp(search_id)).unwrap_or_default() };
            let mut more: bool = !opened && ui.data(|d| d.get_temp(more_id)).unwrap_or(false);
            let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text(tl!("Search formats: png, jpg, layers…")).desired_width(f32::INFINITY));
            // Freshly opened: focus the search field, and keep asking until it has focus (the first
            // pass is egui's invisible sizing pass). Read focus outside `data_mut`: both lock memory.
            let focused = r.has_focus();
            let pending = opened || (!focused && ui.data(|d| d.get_temp::<bool>(search_id.with("focus"))).unwrap_or(false));
            if pending && !focused {
                r.request_focus();
            }
            let mut refocus = pending && !focused;
            let ql = q.trim().to_lowercase();
            let matches = |m: &Fmt| {
                offered(m.key, per_layer)
                    && (ql.is_empty()
                        || m.key.contains(&ql)
                        || m.label.to_lowercase().contains(&ql)
                        || m.aliases.to_lowercase().contains(&ql)
                        || (!m.note.is_empty() && tl!(m.note).to_lowercase().contains(&ql)))
            };
            let mut first: Option<&'static str> = None;
            let mut row = |ui: &mut egui::Ui, m: &'static Fmt, first: &mut Option<&'static str>| {
                first.get_or_insert(m.key);
                let text = if m.note.is_empty() { m.label.to_string() } else { format!("{}   {}", m.label, tl!(m.note)) };
                if ui.selectable_label(m.key == current, text).clicked() {
                    picked = Some(m.key);
                }
            };
            let heading = |ui: &mut egui::Ui, text: &str| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(text).size(11.0).color(Tokens::get(ui.ctx()).text_faint));
            };
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                if ql.is_empty() {
                    let recent: Vec<&'static Fmt> =
                        app.ui.save.recent_formats.iter().filter_map(|k| fmt_of(k)).filter(|m| offered(m.key, per_layer)).take(2).collect();
                    if !recent.is_empty() {
                        heading(ui, tl!("Recent"));
                        for m in recent {
                            row(ui, m, &mut first);
                        }
                    }
                }
                for (group, title) in [(Group::Original, tl!("Original (editable)")), (Group::Image, tl!("Image (merged into one)"))] {
                    let items: Vec<&'static Fmt> = FORMATS.iter().filter(|m| m.group == group && matches(m)).collect();
                    if !items.is_empty() {
                        heading(ui, title);
                        for m in items {
                            row(ui, m, &mut first);
                        }
                    }
                }
                let others: Vec<&'static Fmt> = FORMATS.iter().filter(|m| m.group == Group::Other && matches(m)).collect();
                if !others.is_empty() {
                    if more || !ql.is_empty() {
                        heading(ui, tl!("More formats"));
                        for m in others {
                            row(ui, m, &mut first);
                        }
                    } else if ui.selectable_label(false, crate::i18n::fmt(tl!("Show {count} more formats"), &[("count", &others.len().to_string())])).clicked() {
                        more = true;
                        // Typing goes on in the search field.
                        refocus = true;
                    }
                }
                if first.is_none() {
                    ui.label(egui::RichText::new(tl!("No matching format")).color(Tokens::get(ui.ctx()).text_faint));
                }
            });
            // Enter picks the first match and stops there: it must not also press Save.
            // (A single-line field gives up focus on Enter, so this frame it has just lost it.)
            if (r.has_focus() || r.lost_focus()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                picked = picked.or(first);
            }
            ui.data_mut(|d| {
                d.insert_temp(pass_id, pass);
                d.insert_temp(search_id, q);
                d.insert_temp(more_id, more);
                d.insert_temp(search_id.with("focus"), refocus);
            });
            if picked.is_some() {
                ui.close();
            }
        });
    if let Some(k) = picked {
        f.insert("format".into(), json!(k));
    }
}

/// Where to save: the document's folder, recent folders, the Desktop, or another folder (the
/// system's folder picker).
fn location_picker(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let dir = s(f, "dir", ".").to_string();
    // Desktop and Downloads by their names in the UI language; any other folder by its own name.
    let specials: Vec<(String, &str)> = home()
        .map(|h| {
            vec![(h.join("Desktop").to_string_lossy().into_owned(), tl!("Desktop")), (h.join("Downloads").to_string_lossy().into_owned(), tl!("Downloads"))]
        })
        .unwrap_or_default();
    let name_of = |d: &str| {
        specials.iter().find(|(p, _)| p == d).map_or_else(|| d.rsplit(['/', '\\']).find(|p| !p.is_empty()).unwrap_or(d).to_string(), |(_, t)| t.to_string())
    };
    let mut options: Vec<(String, String)> = Vec::new();
    let mut add = |d: String, text: String| {
        if !d.is_empty() && !options.iter().any(|(o, _)| *o == d) {
            options.push((d, text));
        }
    };
    if let Some(p) = app.session.active().and_then(|st| st.path.as_deref()).and_then(parent) {
        add(p.clone(), format!("{} — {}", name_of(&p), tl!("the file's folder")));
    }
    add(dir.clone(), name_of(&dir));
    for d in app.ui.save.recent_dirs.clone() {
        add(d.clone(), name_of(&d));
    }
    for (d, text) in &specials {
        add(d.clone(), text.to_string());
    }
    let shown = options.iter().find(|(d, _)| *d == dir).map_or_else(|| name_of(&dir), |(_, t)| t.clone());
    let mut chosen = None;
    let mut other = false;
    let r =
        egui::ComboBox::from_id_salt("save-location").selected_text(shown).width(ui.available_width()).icon(crate::widgets::chevron_icon).show_ui(ui, |ui| {
            for (d, text) in &options {
                if ui.selectable_label(*d == dir, text).on_hover_text(d).clicked() {
                    chosen = Some(d.clone());
                }
            }
            ui.separator();
            if ui.selectable_label(false, tl!("Other Folder…")).clicked() {
                other = true;
            }
        });
    r.response.on_hover_text(&dir);
    if let Some(d) = chosen {
        f.insert("dir".into(), json!(d));
    }
    if other {
        let picked = app.pick_folder(|app, dir| {
            if let Some(d) = app.ui.dialogs.iter_mut().find(|d| owns(&d.fields)) {
                d.fields.insert("dir".into(), json!(dir));
            }
            Ok(Value::Null)
        });
        if let Err(e) = picked
            && e != crate::file_dialog::CANCELLED
        {
            f.insert("__error".into(), json!(e));
        }
    }
}

pub fn body(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.set_width(WIDTH);
    measure_labels(ui);
    ui.spacing_mut().item_spacing.y = 8.0;
    let per_layer = b(f, "perLayer", false);
    // Name (or, per layer, the folder to create) with the extension the format writes.
    ui.horizontal(|ui| {
        label(ui, &t, if per_layer { tl!("Folder") } else { tl!("Name") });
        let mut name = s(f, "name", "").to_string();
        let ext = if per_layer { "/".to_string() } else { format!(".{}", s(f, "format", "psd")) };
        // The extension sits inside the field's right end, so this row ends where the dropdowns do.
        let font = egui::TextStyle::Body.resolve(ui.style());
        let ext_width = ui.painter().layout_no_wrap(ext.clone(), font.clone(), t.text_dim).size().x;
        let r = ui.add(
            egui::TextEdit::singleline(&mut name)
                .desired_width(ui.available_width())
                .margin(egui::Margin { left: 10, right: (ext_width + 14.0).round() as i8, top: 4, bottom: 4 }),
        );
        let at = egui::pos2(r.rect.right() - 10.0, r.rect.center().y);
        ui.painter().text(at, egui::Align2::RIGHT_CENTER, &ext, font, t.text_faint);
        if r.changed() {
            // Typing an extension picks its format: "logo.jpg" saves as JPEG.
            if let Some((stem, ext)) = name.rsplit_once('.')
                && let Some(key) = key_for_extension(ext).filter(|k| offered(k, per_layer))
                && !stem.is_empty()
            {
                f.insert("format".into(), json!(key));
                name = stem.to_string();
            }
            f.insert("name".into(), json!(name));
            f.remove("__error");
        }
    });
    ui.horizontal(|ui| {
        label(ui, &t, tl!("Location"));
        location_picker(app, ui, f);
    });
    ui.horizontal(|ui| {
        label(ui, &t, tl!("Format"));
        format_picker(app, ui, f);
    });
    if let Some(w) = warning(app, f) {
        note(ui, &t, &w);
    }
    ui.add_space(2.0);
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        let mut on = per_layer;
        if crate::widgets::checkbox(ui, &mut on, "Save each layer as a file").changed() {
            f.insert("perLayer".into(), json!(on));
            // Layers are saved as images: an original format gives way to the last image format.
            if on && original(s(f, "format", "psd")) {
                let image = app.ui.save.recent_formats.iter().find(|k| offered(k, true)).cloned().unwrap_or_else(|| "png".into());
                f.insert("format".into(), json!(image));
            }
        }
    });
    // Open as it was last time; the field lets automation open it too.
    let mut advanced = b(f, "advanced", app.ui.save.advanced);
    let was = advanced;
    // The size of the file this saves goes in the button row (`footer`); per layer, the warning
    // line counts the files instead.
    let size = if b(f, "perLayer", false) {
        None
    } else {
        estimate(app, ui, f).map(|bytes| crate::i18n::fmt(tl!("Estimated size {size}"), &[("size", &size_text(bytes))]))
    };
    ui.data_mut(|d| d.insert_temp(footer_key(), size));
    fold(ui, &t, &mut advanced, tl!("Advanced"));
    if advanced != was {
        app.ui.save.advanced = advanced;
        f.insert("advanced".into(), json!(advanced));
    }
    if advanced {
        advanced_body(app, ui, &t, f);
    }
    if let Some(e) = f.get("__error").and_then(Value::as_str) {
        ui.horizontal(|ui| {
            ui.add_space(column(ui));
            ui.add(egui::Label::new(egui::RichText::new(e).color(t.danger)).wrap());
        });
    }
}

fn advanced_body(app: &PrismApp, ui: &mut egui::Ui, t: &Tokens, f: &mut Map<String, Value>) {
    let format = s(f, "format", "psd").to_string();
    let direct_id = ui.id().with("save-direct");
    let mut direct = ui.data(|d| d.get_temp::<bool>(direct_id)).unwrap_or(false);
    egui::Frame::NONE.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        match format.as_str() {
            "png" => {
                ui.horizontal(|ui| {
                    label(ui, t, tl!("File size"));
                    choices(ui, f, "pngEffort", &[("default", "Normal"), ("best", "Smallest"), ("fast", "Fast")]);
                });
                note(ui, t, tl!("PNG is lossless: the image looks the same. Only the size and saving time change."));
            }
            "jpg" | "webp" => {
                ui.horizontal(|ui| {
                    label(ui, t, tl!("Quality"));
                    let options: Vec<(&str, &str)> = QUALITY.iter().map(|q| (q.0, q.1)).collect();
                    choices(ui, f, "quality", &options);
                });
                let hint = match s(f, "quality", "high") {
                    "best" => tl!("Hard to tell from the original"),
                    "high" => tl!("Almost no visible difference"),
                    "web" => tl!("Slightly soft up close"),
                    "small" => tl!("Edges can look rough"),
                    _ => tl!("Custom quality"),
                };
                note(ui, t, hint);
            }
            "tif" => {
                ui.horizontal(|ui| {
                    label(ui, t, tl!("Layers"));
                    let mut keep = if b(f, "tiffLayers", true) { "keep" } else { "merge" }.to_string();
                    let mut tmp = Map::new();
                    tmp.insert("v".into(), json!(keep));
                    if choices(ui, &mut tmp, "v", &[("keep", "Keep layers"), ("merge", "Merge into one")]) {
                        keep = s(&tmp, "v", "keep").to_string();
                        f.insert("tiffLayers".into(), json!(keep == "keep"));
                    }
                });
            }
            _ => {}
        }
        if original(&format) {
            note(ui, t, tl!("Original formats keep everything, so there is nothing to set."));
        } else {
            ui.horizontal(|ui| {
                label(ui, t, tl!("Include"));
                let mut icc = b(f, "icc", true);
                if crate::widgets::checkbox(ui, &mut icc, "Color profile").changed() {
                    f.insert("icc".into(), json!(icc));
                }
                ui.add_space(10.0);
                let mut meta = b(f, "metadata", true);
                if crate::widgets::checkbox(ui, &mut meta, "Metadata").changed() {
                    f.insert("metadata".into(), json!(meta));
                }
            });
        }
        if b(f, "perLayer", false) {
            crate::widgets::hairline(ui);
            per_layer_options(app, ui, t, f);
        }
        if matches!(format.as_str(), "png" | "jpg" | "webp") {
            ui.horizontal(|ui| {
                ui.add_space(column(ui));
                fold(ui, t, &mut direct, tl!("Direct settings"));
            });
            if direct {
                direct_settings(ui, t, f, &format);
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(direct_id, direct));
}

fn per_layer_options(app: &PrismApp, ui: &mut egui::Ui, t: &Tokens, f: &mut Map<String, Value>) {
    let mut pick = |ui: &mut egui::Ui, title: &str, key: &str, id: &str, options: &[(&str, &str)]| {
        ui.horizontal(|ui| {
            label(ui, t, title);
            let mut v = s(f, key, options.first().map_or("", |o| o.0)).to_string();
            let opts: Vec<(String, &str)> = options.iter().map(|(k, l)| (k.to_string(), *l)).collect();
            if crate::widgets::dropdown(ui, id, &mut v, &opts, ui.available_width()) {
                f.insert(key.into(), json!(v));
            }
        });
    };
    pick(ui, tl!("Layers"), "layers", "save-layers", &[("visible", "Visible layers"), ("selected", "Selected layers"), ("all", "All layers")]);
    pick(ui, tl!("Groups"), "groups", "save-groups", &[("merge", "One file per group"), ("each", "Each layer inside")]);
    ui.horizontal(|ui| {
        label(ui, t, tl!("Size"));
        let mut trim = b(f, "trim", false);
        if crate::widgets::dropdown(ui, "save-trim", &mut trim, &[(false, "Keep canvas size"), (true, "Trim to content")], ui.available_width()) {
            f.insert("trim".into(), json!(trim));
        }
    });
    ui.horizontal(|ui| {
        label(ui, t, tl!("File names"));
        let mut pattern = s(f, "pattern", "{layer}").to_string();
        if ui.add(egui::TextEdit::singleline(&mut pattern).desired_width(ui.available_width()).margin(egui::Margin { left: 10, right: 10, top: 4, bottom: 4 })).on_hover_text(tl!("{layer}, {index} and {document} are replaced")).changed() {
            f.insert("pattern".into(), json!(pattern));
        }
    });
    if let Some(st) = app.session.active() {
        let names: Vec<String> = preview_names(&st.doc, s(f, "pattern", "{layer}"), s(f, "format", "png"));
        if !names.is_empty() {
            note(ui, t, &format!("{} …", names.join(", ")));
        }
    }
}

/// The first few file names a per-layer save would write (top layers first).
fn preview_names(doc: &prism_doc::Document, pattern: &str, format: &str) -> Vec<String> {
    let stem = file_stem(&doc.name);
    doc.layers
        .iter()
        .rev()
        .filter(|l| !matches!(l.content, prism_doc::LayerContent::Adjustment(_)))
        .take(3)
        .enumerate()
        .map(|(i, l)| {
            let pattern = if pattern.trim().is_empty() { "{layer}" } else { pattern };
            format!("{}.{format}", pattern.replace("{layer}", &l.name).replace("{index}", &format!("{:02}", i + 1)).replace("{document}", &stem))
        })
        .collect()
}

fn direct_settings(ui: &mut egui::Ui, t: &Tokens, f: &mut Map<String, Value>, format: &str) {
    match format {
        "png" => {
            ui.horizontal(|ui| {
                label(ui, t, tl!("Compression"));
                let mut v = s(f, "pngEffort", "default").to_string();
                let opts = [("none".to_string(), "None"), ("fast".to_string(), "Fast"), ("default".to_string(), "Normal"), ("best".to_string(), "Smallest")];
                if crate::widgets::dropdown(ui, "save-png-compression", &mut v, &opts, 160.0) {
                    f.insert("pngEffort".into(), json!(v));
                }
            });
        }
        _ => {
            let key = if format == "jpg" { "jpegQuality" } else { "webpQuality" };
            // The named choice's number until it is changed here; then "custom".
            let preset = QUALITY.iter().find(|q| q.0 == s(f, "quality", "high"));
            let current = preset.map_or_else(|| n(f, key, 85.0), |q| if format == "jpg" { q.2 as f64 } else { q.3 as f64 });
            let mut v = current as f32;
            ui.horizontal(|ui| {
                ui.set_max_width(380.0);
                if crate::widgets::slider_row(ui, tl!("Quality"), &mut v, 1.0..=100.0, "", None).changed() {
                    f.insert(key.into(), json!(v.round() as u8));
                    f.insert("quality".into(), json!("custom"));
                    if let Some(q) = preset {
                        f.insert("chroma444".into(), json!(q.4));
                        let other = if format == "jpg" { ("webpQuality", q.3) } else { ("jpegQuality", q.2) };
                        f.insert(other.0.into(), json!(other.1));
                    }
                }
            });
            if format == "jpg" {
                ui.horizontal(|ui| {
                    label(ui, t, tl!("Color detail"));
                    let mut full = preset.map_or_else(|| b(f, "chroma444", false), |q| q.4);
                    if crate::widgets::checkbox(ui, &mut full, "Full color resolution (4:4:4)").changed() {
                        f.insert("chroma444".into(), json!(full));
                        f.insert("quality".into(), json!("custom"));
                        if let Some(q) = preset {
                            f.insert("jpegQuality".into(), json!(q.2));
                            f.insert("webpQuality".into(), json!(q.3));
                        }
                    }
                });
            }
        }
    }
}

/// Checks the fields before the window closes: `Err` keeps it open with the reason shown.
pub fn validate(f: &Map<String, Value>) -> Result<(), String> {
    problem(f).map_or(Ok(()), |p| Err(p.to_string()))
}

/// Remembers this save's folder, format and options for next time.
fn remember(app: &mut PrismApp, f: &Map<String, Value>) {
    let prefs = &mut app.ui.save;
    let dir = s(f, "dir", "").to_string();
    if !dir.is_empty() {
        prefs.recent_dirs.retain(|d| *d != dir);
        prefs.recent_dirs.insert(0, dir);
        prefs.recent_dirs.truncate(5);
    }
    let format = s(f, "format", "psd").to_string();
    prefs.recent_formats.retain(|k| *k != format);
    prefs.recent_formats.insert(0, format);
    prefs.recent_formats.truncate(6);
    prefs.options.insert("image".into(), settings_key(f));
}

/// Saves as the fields say. An original format from Save / Save As becomes the document's file;
/// everything else (images, Save a Copy, per-layer files) is written beside it.
pub fn confirm(app: &mut PrismApp, f: &Map<String, Value>) -> Result<Value, String> {
    validate(f)?;
    remember(app, f);
    let format = s(f, "format", "psd").to_string();
    let result = if b(f, "perLayer", false) {
        let quality = QUALITY.iter().find(|q| q.0 == s(f, "quality", "high"));
        let mut p = json!({
            "dir": target(f),
            "format": format,
            "layers": s(f, "layers", "visible"),
            "groups": s(f, "groups", "merge"),
            "trim": b(f, "trim", false),
            "name": s(f, "pattern", "{layer}"),
            "tiffLayers": b(f, "tiffLayers", true),
            "pngCompression": match s(f, "pngEffort", "default") { "fast" => "fast", "best" => "best", _ => "default" },
            "jpegQuality": quality.map_or(n(f, "jpegQuality", 85.0), |q| q.2 as f64),
            "webpQuality": quality.map_or(n(f, "webpQuality", 82.0), |q| q.3 as f64),
            "chroma444": quality.map_or(b(f, "chroma444", false), |q| q.4),
        });
        if format == "png" && s(f, "pngEffort", "default") == "none" {
            p["pngCompression"] = json!("default");
        }
        app.run("file.export.layersToFiles", p).map(|r| {
            let count = r["files"].as_array().map_or(0, Vec::len);
            app.ui.status = crate::i18n::fmt(tl!("Saved {count} files in {folder}"), &[("count", &count.to_string()), ("folder", &target(f))]);
            app.ui.status_error = false;
            crate::notices::toast(app, app.ui.status.clone());
            r
        })
    } else {
        let path = target(f);
        let rebind = original(&format) && !copy_mode(f);
        app.write_document(path, &settings(f), !rebind).map(|(path, warnings)| json!({"path": path, "warnings": warnings}))
    };
    if let Err(e) = &result {
        // Keep the window, with its choices, so the save can be retried.
        let mut again = f.clone();
        again.insert("__error".into(), json!(e));
        app.ui.open_dialog(DialogKind::Command, again);
    }
    result
}

#[cfg(test)]
#[path = "save_ui/tests.rs"]
mod tests;
