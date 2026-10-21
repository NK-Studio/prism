//! Window › Extensions › CLIP STUDIO Materials: a dockable extension panel (`PanelId::ClipStudio`)
//! for bringing CLIP STUDIO materials into Prism. It lists the sub-tools downloaded into the
//! installed CLIP STUDIO PAINT (`brush.cspLibrary.list`) by collection, with CLIP STUDIO's own
//! previews, and imports the checked ones (`brush.cspLibrary.import`) without a SUT export.
//! Layer materials (images, special rulers) are listed too and go into the open document as
//! layers (`layer.cspMaterial.import`).
//! Materials narrow by kind, search text and CLIP STUDIO's user tags, and show as a one-column
//! list or a two-column grid of larger previews.
//! Exported files (`.sut`, `.tool`, `.laf`) come in through Open Files…. Materials that are not
//! ready say why; the licence caveat stays visible. Nothing in CLIP STUDIO is changed.
use crate::{PrismApp, dock_model::PanelId, icons, notices, theme::Tokens, widgets};
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Which kinds of material the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KindFilter {
    #[default]
    All,
    /// Brushes, pens, erasers…: they show in the Brushes panel.
    Brushes,
    /// Fill, selection and other sub-tools: they import for the Clip Studio workspace.
    SubTools,
    /// Images and special rulers: they are added to the open document as layers.
    Layers,
}

/// How the materials are laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum View {
    /// One column of rows: preview, name, status.
    #[default]
    List,
    /// Two columns of cards with larger previews.
    Grid,
}

/// Panel state, readable and drivable through the control channel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CspLibraryState {
    /// Checked material ids.
    pub selected: Vec<String>,
    /// The last `brush.cspLibrary.list` result.
    pub listing: Option<Value>,
    /// Why the last listing failed.
    pub error: Option<String>,
    /// Name filter (matches material and collection names).
    pub search: String,
    pub filter: KindFilter,
    /// User tags a material must all carry (its own or its collection's).
    pub tags: Vec<String>,
    /// The tag list is expanded.
    pub tags_open: bool,
    pub view: View,
    /// A CLIP STUDIO user data folder chosen by the user; the platform default otherwise.
    pub root: Option<String>,
    /// Collections folded shut.
    pub collapsed: Vec<String>,
    /// The licence notice is expanded.
    pub license_open: bool,
}

/// Show the panel (Window › Extensions › CLIP STUDIO Materials) with a fresh listing.
pub fn open(app: &mut PrismApp) {
    let pro = crate::prefs_ui::layout(app) == crate::theme::Layout::Compact;
    crate::dock_model::toggle_panel(&mut app.ui, PanelId::ClipStudio, pro, Some(true));
    refresh(app);
}

/// Hide the panel's group.
pub fn close(app: &mut PrismApp) {
    let pro = crate::prefs_ui::layout(app) == crate::theme::Layout::Compact;
    crate::dock_model::toggle_panel(&mut app.ui, PanelId::ClipStudio, pro, Some(false));
}

pub fn refresh(app: &mut PrismApp) {
    let params = match &app.ui.csp_library.root {
        Some(root) => json!({ "root": root }),
        None => json!({}),
    };
    let result = app.session.execute("brush.cspLibrary.list", params);
    let state = &mut app.ui.csp_library;
    match result {
        Ok(listing) => {
            let ready: Vec<&str> = materials(&listing).filter(|b| is_ready(b)).filter_map(|b| b["uuid"].as_str()).collect();
            state.selected.retain(|id| ready.contains(&id.as_str()));
            state.listing = Some(listing);
            state.error = None;
        }
        Err(e) => {
            state.listing = None;
            state.selected.clear();
            state.error = Some(e.to_string());
        }
    }
}

/// Ask for a CLIP STUDIO user data folder, then list it.
fn choose_folder(app: &mut PrismApp) {
    let r = app.pick_folder(|app, dir| {
        app.ui.csp_library.root = Some(dir);
        app.ui.csp_library.selected.clear();
        refresh(app);
        Ok(Value::Null)
    });
    report(app, r);
}

/// Exported CLIP STUDIO files (`.sut`, `.tool`, `.laf`) open like File › Open would.
fn open_files(app: &mut PrismApp) {
    let r = app.open_dialog_file();
    report(app, r);
}

fn report(app: &mut PrismApp, r: Result<Value, String>) {
    if let Err(e) = r.as_ref()
        && e != crate::file_dialog::CANCELLED
    {
        notices::error(app, format!("{}: {e}", tl!("CLIP STUDIO Materials")));
    }
}

fn materials(listing: &Value) -> impl Iterator<Item = &Value> {
    listing["brushes"].as_array().into_iter().flatten().chain(listing["layers"].as_array().into_iter().flatten())
}

fn is_ready(b: &Value) -> bool {
    b["status"] == "ready"
}

fn is_sub_tool(b: &Value) -> bool {
    b["kind"] == "other_tool"
}

/// A layer material (image or special ruler), added to the document rather than the library.
fn is_layer(b: &Value) -> bool {
    b["kind"] == "image"
}

fn is_ruler(b: &Value) -> bool {
    is_layer(b) && b["ruler"] == true
}

/// The brush group an import lands in (the collection name, as `brush.cspLibrary.import` picks).
fn group_of(b: &Value) -> &str {
    b["catalog"].as_str().map(str::trim).filter(|c| !c.is_empty()).unwrap_or("CLIP STUDIO")
}

/// Is a material of this name already in the brush library, in its collection's group?
fn in_library(app: &PrismApp, b: &Value) -> bool {
    if is_layer(b) {
        return false;
    }
    let (Some(name), group) = (b["name"].as_str(), group_of(b)) else { return false };
    app.session.tools.presets.iter().any(|p| !p.builtin && p.name == name && p.group == group)
}

/// `b`'s user tags: its own, then its collection's (materials in a collection often have none).
fn tags_of<'a>(listing: &'a Value, b: &'a Value) -> Vec<&'a str> {
    let catalog = b["catalog"].as_str().unwrap_or_default();
    let collection = listing["catalogs"].as_array().into_iter().flatten().find(|c| !catalog.is_empty() && c["name"] == catalog);
    let mut tags: Vec<&str> = Vec::new();
    for tag in [Some(b), collection].into_iter().flatten().flat_map(|v| v["tags"].as_array().into_iter().flatten()).filter_map(Value::as_str) {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

/// Does `b` pass kind `filter`, the search text (name, collection or tag) and every chosen tag?
fn passes(state: &CspLibraryState, filter: KindFilter, listing: &Value, b: &Value) -> bool {
    let kind_ok = match filter {
        KindFilter::All => true,
        KindFilter::Brushes => !is_sub_tool(b) && !is_layer(b),
        KindFilter::SubTools => is_sub_tool(b),
        KindFilter::Layers => is_layer(b),
    };
    if !kind_ok {
        return false;
    }
    let tags = tags_of(listing, b);
    let needle = state.search.trim().to_lowercase();
    let text_ok = needle.is_empty()
        || [b["name"].as_str(), b["catalog"].as_str()].into_iter().flatten().chain(tags.iter().copied()).any(|s| s.to_lowercase().contains(&needle));
    text_ok && state.tags.iter().all(|t| tags.contains(&t.as_str()))
}

/// Does `b` pass every filter?
fn shown(state: &CspLibraryState, listing: &Value, b: &Value) -> bool {
    passes(state, state.filter, listing, b)
}

/// The tags of the materials the filters leave, most used first, with their counts. Chosen
/// tags always appear, so they can be cleared.
fn tag_counts(state: &CspLibraryState, listing: &Value) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = state.tags.iter().map(|t| (t.clone(), 0)).collect();
    for b in materials(listing).filter(|b| shown(state, listing, b)) {
        for tag in tags_of(listing, b) {
            match counts.iter_mut().find(|(t, _)| t == tag) {
                Some((_, n)) => *n += 1,
                None => counts.push((tag.to_string(), 1)),
            }
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    counts
}

/// Any material or collection in the listing carries a tag.
fn has_tags(listing: &Value) -> bool {
    let tagged = |v: &Value| v["tags"].as_array().is_some_and(|a| !a.is_empty());
    materials(listing).any(tagged) || listing["catalogs"].as_array().into_iter().flatten().any(tagged)
}

/// Add `tag` to the chosen tags, or remove it.
fn toggle_tag(state: &mut CspLibraryState, tag: &str) {
    if state.tags.iter().any(|t| t == tag) {
        state.tags.retain(|t| t != tag);
    } else {
        state.tags.push(tag.to_string());
    }
}

/// Report a finished import (synchronously or from its background job).
pub fn imported(app: &mut PrismApp, v: &Value) {
    let n = v["count"].as_u64().unwrap_or(0);
    let brushes = v["brushes"].as_u64().unwrap_or(0).min(n);
    // Sub-tools (fill, …) are not brushes: say where they went.
    app.ui.status = if brushes == n {
        crate::i18n::fmt(tl!("Imported {n} brushes from CLIP STUDIO"), &[("n", &n.to_string())])
    } else {
        crate::i18n::fmt(
            tl!("Imported {brushes} brushes and {tools} sub-tools from CLIP STUDIO; sub-tools are in the Clip Studio workspace"),
            &[("brushes", &brushes.to_string()), ("tools", &(n - brushes).to_string())],
        )
    };
    app.ui.status_error = false;
    if v["brushes"].as_u64().unwrap_or(0) > 0 {
        app.ui.panels.brush_settings = true;
        app.ui.brush_tab = 1;
    }
    let mut warnings: Vec<String> = v["warnings"].as_array().into_iter().flatten().filter_map(|w| w.as_str().map(str::to_string)).collect();
    for f in v["failed"].as_array().into_iter().flatten() {
        warnings.push(format!("{}: {}", tl!("Not imported"), f["error"].as_str().unwrap_or_default()));
    }
    notices::io_warnings(app, tl!("Imported from CLIP STUDIO"), &warnings);
    app.ui.csp_library.selected.clear();
}

fn import_selected(app: &mut PrismApp) {
    let selected = app.ui.csp_library.selected.clone();
    if selected.is_empty() {
        return;
    }
    let layer_ids: Vec<String> = app
        .ui
        .csp_library
        .listing
        .as_ref()
        .map(|l| materials(l).filter(|b| is_layer(b)).filter_map(|b| b["uuid"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let (layers, uuids): (Vec<String>, Vec<String>) = selected.into_iter().partition(|id| layer_ids.contains(id));
    if !layers.is_empty() {
        add_layers(app, &layers);
    }
    if uuids.is_empty() {
        return;
    }
    match app.run("brush.cspLibrary.import", json!({ "uuids": uuids, "select": true })) {
        Ok(v) if v.get("pending").is_none() => imported(app, &v),
        Ok(_) => {}
        Err(e) => notices::error(app, format!("{}: {e}", tl!("Import from CLIP STUDIO"))),
    }
}

/// Add layer materials to the open document, each as its own history step.
fn add_layers(app: &mut PrismApp, uuids: &[String]) {
    if app.session.active().is_none() {
        notices::error(app, tl!("Open or create a document first: layer materials are added to it as layers").to_string());
        return;
    }
    let root = app.ui.csp_library.root.clone();
    let (mut added, mut rulers, mut warnings) = (0, 0, Vec::new());
    for uuid in uuids {
        let mut p = json!({ "uuid": uuid });
        if let Some(root) = &root {
            p["root"] = json!(root);
        }
        match app.run("layer.cspMaterial.import", p) {
            Ok(v) => {
                added += 1;
                rulers += v["rulers"].as_u64().unwrap_or(0);
                let name = v["name"].as_str().unwrap_or_default().to_string();
                warnings.extend(v["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|w| format!("{name}: {w}")));
            }
            Err(e) => warnings.push(format!("{}: {e}", tl!("Not added"))),
        }
    }
    app.ui.status = if rulers > 0 {
        crate::i18n::fmt(tl!("Added {n} CLIP STUDIO materials with {rulers} special rulers; strokes snap to them"), &[("n", &added.to_string()), ("rulers", &rulers.to_string())])
    } else {
        crate::i18n::fmt(tl!("Added {n} CLIP STUDIO materials to the document"), &[("n", &added.to_string())])
    };
    app.ui.status_error = added == 0;
    notices::io_warnings(app, tl!("Added from CLIP STUDIO"), &warnings);
    app.ui.csp_library.selected.retain(|id| !uuids.contains(id));
}

fn status_label(status: &str) -> &'static str {
    match status {
        "ready" => tl!("Ready"),
        "downloading" => tl!("Downloading"),
        "noCache" => tl!("Not prepared"),
        _ => tl!("Unavailable"),
    }
}

/// Engine messages are English; translate the known ones.
fn tl_dyn(s: &str) -> String {
    crate::i18n::t(s).to_string()
}

/// CLIP STUDIO's preview images, decoded once per context and path (`None`: unreadable).
#[derive(Clone, Default)]
struct Thumbs(HashMap<String, Option<egui::TextureHandle>>);

/// Previews decoded per frame; the rest come on the next frames, so a long list stays smooth.
const DECODES_PER_FRAME: usize = 4;
/// Largest preview decoded (pixels); CLIP STUDIO's are a few hundred pixels square.
const MAX_THUMB_PIXELS: usize = 1 << 22;

fn thumbnail(ui: &egui::Ui, path: Option<&str>, budget: &mut usize) -> Option<egui::TextureHandle> {
    let path = path?;
    let id = egui::Id::new("csp-library-thumbs");
    let ctx = ui.ctx();
    if let Some(hit) = ctx.data(|d| d.get_temp::<Thumbs>(id).and_then(|t| t.0.get(path).cloned())) {
        return hit;
    }
    if *budget == 0 {
        ctx.request_repaint();
        return None;
    }
    *budget -= 1;
    let texture = decode(ctx, path);
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Thumbs>(id).0.insert(path.to_string(), texture.clone()));
    texture
}

fn decode(ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
    let bytes = prism_io::sut::library::thumbnail_bytes(std::path::Path::new(path)).ok()?;
    let icon = eframe::icon_data::from_png_bytes(&bytes).ok()?;
    let size = [usize::try_from(icon.width).ok()?, usize::try_from(icon.height).ok()?];
    let pixels = size[0].checked_mul(size[1]).filter(|n| *n > 0 && *n <= MAX_THUMB_PIXELS)?;
    if pixels.checked_mul(4) != Some(icon.rgba.len()) {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &icon.rgba);
    Some(ctx.load_texture(format!("csp-thumb:{path}"), image, egui::TextureOptions::LINEAR))
}

/// A preview in `rect`, letterboxed on a checkerboard; a kind glyph while there is none.
fn paint_thumb(ui: &egui::Ui, rect: Rect, texture: Option<&egui::TextureHandle>, glyph: &str, t: &Tokens) {
    let p = ui.painter();
    p.rect_filled(rect, t.radius_sm, t.field);
    match texture {
        Some(tex) => {
            let [w, h] = tex.size().map(|v| v as f32);
            let scale = (rect.width() / w.max(1.0)).min(rect.height() / h.max(1.0));
            let img = Rect::from_center_size(rect.center(), vec2(w * scale, h * scale));
            widgets::checker(p, img, 4.0);
            p.image(tex.id(), img, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => icons::paint(ui, Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), glyph, 18.0, t.text_faint),
    }
    p.rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.card_border), StrokeKind::Inside);
}

const ROW_H: f32 = 52.0;
const FOOTER_H: f32 = 40.0;
/// Grid cards: gap between the two columns, text block under the preview, largest preview height.
const GRID_GAP: f32 = 6.0;
const CARD_TEXT_H: f32 = 40.0;
const CARD_THUMB_MAX: f32 = 160.0;
/// Card previews are wider than tall, like CLIP STUDIO's stroke previews.
const CARD_ASPECT: f32 = 0.75;
/// The expanded tag list scrolls past this height.
const TAG_CLOUD_H: f32 = 96.0;

/// The panel: search and kind filter, notices, the collections, and the import footer.
pub fn body(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    // First showing (and after a restart, when no listing was kept): read the library.
    if app.ui.csp_library.listing.is_none() && app.ui.csp_library.error.is_none() {
        refresh(app);
    }
    let mut action = Action::None;
    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
    header(app, ui, &t, &mut action);

    let full = ui.available_rect_before_wrap();
    let list_h = (full.height() - FOOTER_H).max(60.0);
    let list_rect = Rect::from_min_size(full.min, vec2(full.width(), list_h));
    let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    match (app.ui.csp_library.listing.clone(), app.ui.csp_library.error.clone()) {
        (Some(listing), _) if materials(&listing).next().is_some() => list(app, &mut list_ui, &listing, &t),
        (Some(_), _) => empty(
            &mut list_ui,
            &t,
            tl!("No downloaded materials yet"),
            tl!("Download brushes or sub-tools in CLIP STUDIO, then refresh. Exported .sut, .tool and .laf files can be opened directly."),
            &mut action,
        ),
        (None, Some(error)) => empty(&mut list_ui, &t, tl!("CLIP STUDIO was not found"), &tl_dyn(&error), &mut action),
        (None, None) => {}
    }
    let footer_rect = Rect::from_min_max(egui::pos2(full.left(), full.bottom() - FOOTER_H), full.max);
    ui.allocate_rect(full, Sense::hover());
    let mut footer_ui = ui.new_child(egui::UiBuilder::new().max_rect(footer_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    footer(app, &mut footer_ui, &t, &mut action);

    match action {
        Action::None => {}
        Action::Refresh => refresh(app),
        Action::ChooseFolder => choose_folder(app),
        Action::OpenFiles => open_files(app),
        Action::Import => import_selected(app),
    }
}

enum Action {
    None,
    Refresh,
    ChooseFolder,
    OpenFiles,
    Import,
}

fn header(app: &mut PrismApp, ui: &mut egui::Ui, t: &Tokens, action: &mut Action) {
    let state = &mut app.ui.csp_library;
    ui.horizontal(|ui| {
        let search_w = (ui.available_width() - 2.0 * 30.0).max(80.0);
        ui.add(egui::TextEdit::singleline(&mut state.search).hint_text(tl!("Search materials")).desired_width(search_w));
        if icons::button(ui, "rotate-cw", 24.0, false, tl!("Refresh")).clicked() {
            *action = Action::Refresh;
        }
        let folder_tip = match &state.root {
            Some(root) => format!("{}\n{root}", tl!("Choose CLIP STUDIO data folder…")),
            None => tl!("Choose CLIP STUDIO data folder…").to_string(),
        };
        if icons::button(ui, "folder-open", 24.0, state.root.is_some(), &folder_tip).clicked() {
            *action = Action::ChooseFolder;
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let count = |f: KindFilter| state.listing.as_ref().map_or(0, |l| materials(l).filter(|b| passes(state, f, l, b)).count());
        let counts = [KindFilter::All, KindFilter::Brushes, KindFilter::SubTools, KindFilter::Layers].map(count);
        let kinds = [(KindFilter::All, tl!("All")), (KindFilter::Brushes, tl!("Brushes")), (KindFilter::SubTools, tl!("Sub tools")), (KindFilter::Layers, tl!("Layers"))];
        for ((f, label), n) in kinds.into_iter().zip(counts) {
            if widgets::pill_tab(ui, &format!("{label} {n}"), state.filter == f).clicked() {
                state.filter = f;
            }
        }
        // One column or two, on the right.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (view, icon, tip) in [(View::Grid, "grid-2x2", tl!("Grid view")), (View::List, "list", tl!("List view"))] {
                if icons::button(ui, icon, 22.0, state.view == view, tip).clicked() {
                    state.view = view;
                }
            }
        });
    });
    if state.listing.as_ref().is_some_and(has_tags) || !state.tags.is_empty() {
        tag_bar(state, ui, t);
    }
    // The licence caveat stays in view, folded to one line; other notices are warnings.
    if let Some(listing) = &state.listing {
        let license = prism_io::sut::library::LICENSE_NOTICE;
        for notice in listing["notices"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if notice == license {
                let open = state.license_open;
                let resp = ui
                    .horizontal(|ui| {
                        icons::paint(ui, Rect::from_min_size(ui.cursor().min + vec2(0.0, 1.0), vec2(14.0, 14.0)), "info", 14.0, t.text_faint);
                        ui.add_space(18.0);
                        let text = if open { tl_dyn(notice) } else { tl!("Check each material's licence before using it in Prism").to_string() };
                        ui.add(egui::Label::new(egui::RichText::new(text).small().color(t.text_faint)).wrap().sense(Sense::click()))
                    })
                    .inner;
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    state.license_open = !open;
                }
            } else {
                ui.add(egui::Label::new(egui::RichText::new(format!("⚠ {}", tl_dyn(notice))).small().color(t.warning)).wrap());
            }
        }
    }
    widgets::hairline(ui);
}

/// A rounded tag chip; `count` follows the name, a chosen chip ends in a remove mark.
fn tag_chip(ui: &mut egui::Ui, tag: &str, count: Option<usize>, on: bool, t: &Tokens) -> egui::Response {
    let font = egui::FontId::proportional(11.5);
    let name = ui.painter().layout_no_wrap(tag.to_string(), font.clone(), if on { t.text } else { t.text_dim });
    let extra = count.map(|n| ui.painter().layout_no_wrap(n.to_string(), font, t.text_faint));
    let close = if on { 12.0 } else { 0.0 };
    let natural = name.size().x + extra.as_ref().map_or(0.0, |g| g.size().x + 5.0) + close + 16.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(natural.min(ui.available_width().max(40.0)), 20.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    let (fill, stroke) = match (on, resp.hovered()) {
        (true, _) => (t.accent_soft, t.accent),
        (false, true) => (t.hover, t.field_border),
        (false, false) => (t.field, t.field_border),
    };
    p.rect_filled(rect, 10.0, fill);
    p.rect_stroke(rect, 10.0, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let clip = p.with_clip_rect(rect.shrink2(vec2(6.0, 0.0)));
    let name_w = name.size().x;
    clip.galley(egui::pos2(rect.left() + 8.0, rect.center().y - name.size().y / 2.0), name, t.text);
    if let Some(g) = extra {
        clip.galley(egui::pos2(rect.left() + 13.0 + name_w, rect.center().y - g.size().y / 2.0), g, t.text_faint);
    }
    if on {
        icons::paint(ui, Rect::from_center_size(egui::pos2(rect.right() - 11.0, rect.center().y), vec2(10.0, 10.0)), "x", 10.0, t.text_dim);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, tag));
    resp
}

/// The tag filter: a fold toggle with the chosen tags beside it, then (open) every tag of the
/// materials the filters leave, most used first.
fn tag_bar(state: &mut CspLibraryState, ui: &mut egui::Ui, t: &Tokens) {
    let mut toggle: Option<String> = None;
    let mut clear = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        let open = state.tags_open;
        let label = tl!("Tags");
        let galley = ui.painter().layout_no_wrap(label.to_string(), crate::theme::medium(12.0), t.text_dim);
        let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 34.0, 20.0), Sense::click());
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tl!("Filter by CLIP STUDIO user tags"));
        if resp.hovered() {
            ui.painter().rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.6));
        }
        icons::paint(ui, Rect::from_center_size(rect.left_center() + vec2(8.0, 0.0), vec2(12.0, 12.0)), "tag", 12.0, t.text_dim);
        ui.painter().galley(egui::pos2(rect.left() + 18.0, rect.center().y - galley.size().y / 2.0), galley, t.text_dim);
        let chevron = if open { "chevron-down" } else { "chevron-right" };
        icons::paint(ui, Rect::from_center_size(rect.right_center() - vec2(7.0, 0.0), vec2(10.0, 10.0)), chevron, 10.0, t.text_faint);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, label));
        if resp.clicked() {
            state.tags_open = !open;
        }
        for tag in &state.tags {
            if tag_chip(ui, tag, None, true, t).on_hover_text(tl!("Remove this tag from the filter")).clicked() {
                toggle = Some(tag.clone());
            }
        }
        if !state.tags.is_empty() && ui.add(egui::Button::new(egui::RichText::new(tl!("Clear tags")).small()).frame(false)).clicked() {
            clear = true;
        }
    });
    if state.tags_open
        && let Some(listing) = &state.listing
    {
        let counts = tag_counts(state, listing);
        if counts.iter().all(|(tag, _)| state.tags.contains(tag)) {
            ui.label(egui::RichText::new(tl!("No other tags among these materials")).small().color(t.text_faint));
        } else {
            egui::ScrollArea::vertical().id_salt("csp-library-tags").max_height(TAG_CLOUD_H).auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for (tag, n) in counts.iter().filter(|(tag, _)| !state.tags.contains(tag)) {
                        if tag_chip(ui, tag, Some(*n), false, t).clicked() {
                            toggle = Some(tag.clone());
                        }
                    }
                });
            });
        }
    }
    if clear {
        state.tags.clear();
    }
    if let Some(tag) = toggle {
        toggle_tag(state, &tag);
    }
}

fn list(app: &mut PrismApp, ui: &mut egui::Ui, listing: &Value, t: &Tokens) {
    let covers: HashMap<&str, &str> =
        listing["catalogs"].as_array().into_iter().flatten().filter_map(|c| Some((c["name"].as_str()?, c["thumbnail"].as_str()?))).collect();
    // Collections in catalog order, loose materials last.
    let mut groups: Vec<(&str, Vec<&Value>)> = Vec::new();
    for b in materials(listing).filter(|b| shown(&app.ui.csp_library, listing, b)) {
        let name = b["catalog"].as_str().unwrap_or_default();
        match groups.iter_mut().find(|(g, _)| *g == name) {
            Some((_, items)) => items.push(b),
            None => groups.push((name, vec![b])),
        }
    }
    groups.sort_by_key(|(g, _)| g.is_empty());
    if groups.is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| ui.label(egui::RichText::new(tl!("No materials match the filter")).color(t.text_faint)));
        return;
    }
    let mut budget = DECODES_PER_FRAME;
    let grid = app.ui.csp_library.view == View::Grid;
    egui::ScrollArea::vertical().id_salt("csp-library-list").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = if grid { GRID_GAP } else { 2.0 };
        for (group, items) in groups {
            collection_header(app, ui, group, &items, covers.get(group).copied(), t, &mut budget);
            if app.ui.csp_library.collapsed.iter().any(|c| c == group) {
                continue;
            }
            if grid {
                let cell_w = ((ui.available_width() - GRID_GAP) / 2.0).max(40.0);
                let card_h = ((cell_w - 12.0) * CARD_ASPECT).clamp(28.0, CARD_THUMB_MAX) + 12.0 + CARD_TEXT_H;
                for pair in items.chunks(2) {
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
                    for (i, b) in pair.iter().enumerate() {
                        let left = line.left() + i as f32 * (cell_w + GRID_GAP);
                        let cell = Rect::from_min_size(egui::pos2(left, line.top()), vec2(cell_w, card_h));
                        material(app, ui, cell, b, listing, t, &mut budget, true);
                    }
                }
            } else {
                for b in items {
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                    material(app, ui, rect, b, listing, t, &mut budget, false);
                }
            }
            ui.add_space(4.0);
        }
    });
}

fn collection_header(app: &mut PrismApp, ui: &mut egui::Ui, group: &str, items: &[&Value], cover: Option<&str>, t: &Tokens, budget: &mut usize) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let state = &mut app.ui.csp_library;
    let folded = state.collapsed.iter().any(|c| c == group);
    if resp.hovered() {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.5));
    }
    let chevron = if folded { "chevron-right" } else { "chevron-down" };
    icons::paint(ui, Rect::from_center_size(rect.left_center() + vec2(9.0, 0.0), vec2(12.0, 12.0)), chevron, 12.0, t.text_dim);
    let cover_rect = Rect::from_min_size(rect.left_top() + vec2(20.0, 3.0), vec2(24.0, 24.0));
    paint_thumb(ui, cover_rect, thumbnail(ui, cover, budget).as_ref(), "folder", t);
    let ready: Vec<String> = items.iter().filter(|b| is_ready(b)).filter_map(|b| b["uuid"].as_str().map(str::to_string)).collect();
    let all_on = !ready.is_empty() && ready.iter().all(|id| state.selected.contains(id));
    // "Select all" on the right; the rest of the row folds the collection.
    let pick_rect = Rect::from_min_size(egui::pos2(rect.right() - 74.0, rect.top() + 5.0), vec2(70.0, 20.0));
    let pick = ui.put(pick_rect, egui::Button::new(egui::RichText::new(if all_on { tl!("Clear selection") } else { tl!("Select all") }).small()).frame(false));
    let pick = pick.on_hover_text(tl!("Check every ready material in this collection"));
    let title = if group.is_empty() { tl!("Other materials") } else { group };
    let font = crate::theme::medium(12.5);
    let name_w = (pick_rect.left() - cover_rect.right() - 12.0).max(10.0);
    let galley = ui.painter().layout(format!("{title}  ·  {}", items.len()), font, t.text, name_w);
    ui.painter().galley(egui::pos2(cover_rect.right() + 8.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
    if pick.clicked() {
        state.selected.retain(|id| !ready.contains(id));
        if !all_on {
            state.selected.extend(ready);
        }
    } else if resp.clicked() {
        if folded {
            state.collapsed.retain(|c| c != group);
        } else {
            state.collapsed.push(group.to_string());
        }
    }
}

/// A check box in `check`; dimmed when the material cannot be checked.
fn paint_check(p: &egui::Painter, check: Rect, on: bool, ready: bool, t: &Tokens) {
    if on {
        p.rect_filled(check, 2.0, t.accent);
        let (a, m, c) = (check.left_center() + vec2(3.0, 0.5), check.center_bottom() + vec2(-1.0, -3.5), check.right_top() + vec2(-3.0, 3.5));
        p.line_segment([a, m], Stroke::new(1.8, Color32::WHITE));
        p.line_segment([m, c], Stroke::new(1.8, Color32::WHITE));
    } else {
        p.rect_filled(check, 2.0, t.field);
        p.rect_stroke(check, 2.0, Stroke::new(1.5, if ready { t.text_faint } else { t.separator }), StrokeKind::Inside);
    }
}

/// The "In library" badge, its right edge at `right` and centred on `y`; returns its rect.
fn paint_badge(p: &egui::Painter, right: f32, y: f32, fill: Option<Color32>, t: &Tokens) -> Rect {
    let galley = p.layout_no_wrap(tl!("In library").to_string(), egui::FontId::proportional(10.5), t.text_dim);
    let badge = Rect::from_min_size(egui::pos2(right - galley.size().x - 10.0, y - 9.0), vec2(galley.size().x + 10.0, 18.0));
    if let Some(fill) = fill {
        p.rect_filled(badge, 9.0, fill);
    }
    p.rect_stroke(badge, 9.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    p.galley(badge.center() - galley.size() / 2.0, galley, t.text_dim);
    badge
}

/// One line of text from `left`, at most `width` wide (truncated), its middle at `y`.
fn paint_line(p: &egui::Painter, text: String, font: egui::FontId, color: Color32, left: f32, y: f32, width: f32) {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    let galley = p.layout_job(job);
    p.galley(egui::pos2(left, y - galley.size().y / 2.0), galley, color);
}

/// One material in `rect`: a list row, or (`grid`) a card with a large preview. Clicking a
/// ready one checks it.
#[allow(clippy::too_many_arguments)]
fn material(app: &mut PrismApp, ui: &mut egui::Ui, rect: Rect, b: &Value, listing: &Value, t: &Tokens, budget: &mut usize, grid: bool) {
    let (Some(uuid), Some(name)) = (b["uuid"].as_str(), b["name"].as_str()) else { return };
    let ready = is_ready(b);
    let owned = in_library(app, b);
    let tags = tags_of(listing, b);
    let resp = ui.interact(rect, ui.id().with(("csp-material", uuid)), if ready { Sense::click() } else { Sense::hover() });
    let state = &mut app.ui.csp_library;
    let on = state.selected.iter().any(|s| s == uuid);
    let p = ui.painter();
    if on {
        p.rect_filled(rect, t.radius_sm, t.accent_soft);
    } else if resp.hovered() && ready {
        p.rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.6));
    }
    let (glyph, kind) = if is_ruler(b) {
        ("ruler", tl!("Special ruler"))
    } else if is_layer(b) {
        ("image", tl!("Image"))
    } else if is_sub_tool(b) {
        ("paint-bucket", tl!("Sub tool"))
    } else {
        ("brush", tl!("Brush"))
    };
    let detail = format!("{}  ·  {kind}", status_label(b["status"].as_str().unwrap_or_default()));
    let name_color = if ready { t.text } else { t.text_dim };
    let dot_color = if ready { Color32::from_rgb(0x4c, 0xaf, 0x50) } else { t.warning };
    if grid {
        // Card: the preview with the check box over its corner, then the name and status.
        let thumb_h = ((rect.width() - 12.0) * CARD_ASPECT).min(rect.height() - 12.0 - CARD_TEXT_H).clamp(20.0, CARD_THUMB_MAX);
        let thumb_rect = Rect::from_min_size(rect.left_top() + vec2(6.0, 6.0), vec2(rect.width() - 12.0, thumb_h));
        paint_thumb(ui, thumb_rect, thumbnail(ui, b["thumbnail"].as_str(), budget).as_ref(), glyph, t);
        let p = ui.painter();
        if !ready {
            p.rect_filled(thumb_rect, t.radius_sm, t.dock.gamma_multiply(0.55));
        }
        let check = Rect::from_min_size(thumb_rect.left_top() + vec2(5.0, 5.0), vec2(14.0, 14.0));
        p.rect_filled(check.expand(2.0), 3.0, t.dock.gamma_multiply(0.7));
        paint_check(p, check, on, ready, t);
        if owned {
            paint_badge(p, thumb_rect.right() - 5.0, thumb_rect.top() + 14.0, Some(t.dock.gamma_multiply(0.85)), t);
        }
        let (left, width, top) = (rect.left() + 6.0, rect.width() - 12.0, thumb_rect.bottom() + 4.0);
        paint_line(p, name.to_string(), crate::theme::medium(12.5), name_color, left, top + 9.0, width);
        let dot = egui::pos2(left + 3.0, top + 27.0);
        p.circle_filled(dot, 3.0, dot_color);
        paint_line(p, detail, egui::FontId::proportional(11.0), t.text_faint, left + 10.0, dot.y, width - 10.0);
    } else {
        // Row: check box, preview, the name over status · kind · own tags, "In library" on the right.
        paint_check(p, Rect::from_center_size(rect.left_center() + vec2(12.0, 0.0), vec2(14.0, 14.0)), on, ready, t);
        let thumb_rect = Rect::from_min_size(rect.left_top() + vec2(26.0, 4.0), vec2(ROW_H - 8.0, ROW_H - 8.0));
        paint_thumb(ui, thumb_rect, thumbnail(ui, b["thumbnail"].as_str(), budget).as_ref(), glyph, t);
        let p = ui.painter();
        if !ready {
            p.rect_filled(thumb_rect, t.radius_sm, t.dock.gamma_multiply(0.55));
        }
        let text_right = if owned { paint_badge(p, rect.right() - 6.0, rect.center().y, None, t).left() - 6.0 } else { rect.right() - 6.0 };
        let text_left = thumb_rect.right() + 10.0;
        let width = (text_right - text_left).max(10.0);
        paint_line(p, name.to_string(), crate::theme::medium(13.0), name_color, text_left, rect.center().y - 9.0, width);
        let dot = egui::pos2(text_left + 3.0, rect.center().y + 9.0);
        p.circle_filled(dot, 3.0, dot_color);
        let own: Vec<&str> = b["tags"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let detail = if own.is_empty() { detail } else { format!("{detail}  ·  {}", own.join(", ")) };
        paint_line(p, detail, egui::FontId::proportional(11.0), t.text_faint, text_left + 10.0, dot.y, width - 10.0);
    }

    let resp = resp.on_hover_ui(|ui| {
        ui.label(egui::RichText::new(name).strong());
        if let Some(guidance) = b["guidance"].as_str() {
            ui.label(tl_dyn(guidance));
        } else if b["source"] == "toolLibrary" {
            ui.label(tl!("Read from CLIP STUDIO PAINT's current sub-tool library; edits made there are included"));
        }
        if is_sub_tool(b) {
            ui.label(tl!("Not a brush: it imports for the Clip Studio workspace (Window › Workspace › Clip Studio)"));
        }
        if is_ruler(b) {
            ui.label(tl!("Adds a layer carrying its special ruler to the open document; brush strokes snap to it (View › Snap To › Special Ruler)"));
        } else if is_layer(b) {
            ui.label(tl!("Adds a layer to the open document"));
        }
        if owned {
            ui.label(tl!("Already in the brush library; importing again replaces it"));
        }
        if !tags.is_empty() {
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Tags: {tags}"), &[("tags", &tags.join(", "))])).small());
        }
    });
    if ready {
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, name));
    }
    if resp.clicked() {
        state.selected.retain(|s| s != uuid);
        if !on {
            state.selected.push(uuid.to_string());
        }
    }
}

fn empty(ui: &mut egui::Ui, t: &Tokens, title: &str, detail: &str, action: &mut Action) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        icons::paint(ui, rect, "puzzle", 32.0, t.text_faint);
        ui.label(egui::RichText::new(title).strong());
        ui.add(egui::Label::new(egui::RichText::new(detail).color(t.text_faint)).wrap());
        ui.add_space(8.0);
        if widgets::secondary_button(ui, tl!("Choose CLIP STUDIO data folder…"), 180.0).clicked() {
            *action = Action::ChooseFolder;
        }
    });
}

fn footer(app: &mut PrismApp, ui: &mut egui::Ui, t: &Tokens, action: &mut Action) {
    widgets::hairline(ui);
    let n = app.ui.csp_library.selected.len();
    ui.horizontal(|ui| {
        let label = if n == 0 { tl!("Nothing selected").to_string() } else { crate::i18n::fmt(tl!("{n} selected"), &[("n", &n.to_string())]) };
        ui.label(egui::RichText::new(label).color(t.text_faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The primary button only once there is something to import.
            if n > 0 {
                if widgets::primary_button(ui, &crate::i18n::fmt(tl!("Import {n}"), &[("n", &n.to_string())]), 72.0).clicked() {
                    *action = Action::Import;
                }
            } else {
                ui.add_enabled_ui(false, |ui| widgets::secondary_button(ui, tl!("Import"), 72.0));
            }
            if widgets::secondary_button(ui, tl!("Open Files…"), 0.0).on_hover_text(tl!("Import exported CLIP STUDIO files (.sut, .tool, .laf)")).clicked() {
                *action = Action::OpenFiles;
            }
        });
    });
}

#[cfg(test)]
#[path = "csp_library_ui_tests.rs"]
mod tests;
