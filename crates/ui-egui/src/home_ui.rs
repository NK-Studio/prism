//! The Home screen, laid out like Photoshop's: it fills the window under the menu bar (no options
//! bar, tools or panels), with the app mark in a bar across the top, New file and Open on the
//! left, and the recently opened Prism and Photoshop documents (`.prism`, `.psd`, `.psb`) as
//! thumbnail cards in a centred grid.
//! Thumbnails are read on a worker thread (`prism_io::thumbnail`), one file at a time, and cached
//! per path until the file changes.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use egui::{Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, pos2, vec2};

use crate::PrismApp;
use crate::theme::Tokens;

/// Width of the left column (New file, Open, Home).
const SIDEBAR_W: f32 = 200.0;
/// The bar across the top holding the app mark.
const TOP_BAR_H: f32 = 52.0;
/// Space between recent cards.
const GAP: f32 = 12.0;
/// The recent grid never grows wider than this many cards, so it stays centred on wide windows.
const MAX_COLUMNS: usize = 7;
/// A recent-file card and its thumbnail area.
const CARD: egui::Vec2 = vec2(170.0, 176.0);
const THUMB_H: f32 = 118.0;
/// Thumbnails are read at this size (longer side), enough for a sharp card on a 2x display.
const THUMB_PX: u32 = 320;
/// How often a cached thumbnail checks whether its file changed.
const RECHECK: Duration = Duration::from_secs(2);

/// Is the Home screen up? It shows over everything with no documents (Preferences › General ›
/// Auto show the Home Screen) or after the Home button, but not while files are still opening
/// (they have tabs before they have documents).
pub fn up(app: &PrismApp) -> bool {
    app.jobs.opens.is_empty() && app.ui.chrome.shows_home(app.session.documents().len(), app.session.prefs().general.auto_show_home_screen)
}

/// Leave the Home screen for the open documents (the app mark or Esc); nothing to go back to
/// without any.
fn leave(app: &mut PrismApp) {
    if !app.session.documents().is_empty() {
        app.ui.chrome.home = None;
    }
}

/// Is `path` a document the Home screen lists (Prism's own and Photoshop's)?
pub fn lists(path: &str) -> bool {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    matches!(ext.as_str(), "prism" | "psd" | "psb")
}

/// The recent files the Home screen shows, newest first.
pub fn recent_documents(recent: &[String]) -> Vec<String> {
    recent.iter().filter(|p| lists(p)).cloned().collect()
}

enum Slot {
    /// Queued or being read.
    Loading,
    /// Read; not yet a texture (`None`: the file has no preview Prism can show).
    Read(Option<egui::ColorImage>),
    Ready(Option<TextureHandle>),
}

struct Entry {
    slot: Slot,
    modified: Option<SystemTime>,
    checked: Instant,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    queue: VecDeque<String>,
    worker: bool,
}

type Shared = Arc<Mutex<Cache>>;

fn cache(ctx: &egui::Context) -> Shared {
    let id = egui::Id::new("home-thumbnails");
    if let Some(c) = ctx.data(|d| d.get_temp::<Shared>(id)) {
        return c;
    }
    let c: Shared = Arc::default();
    ctx.data_mut(|d| d.insert_temp(id, c.clone()));
    c
}

fn modified(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Read queued thumbnails one at a time until the queue is empty.
fn work(shared: Shared, ctx: egui::Context) {
    loop {
        let next = {
            let mut c = shared.lock().unwrap_or_else(PoisonError::into_inner);
            match c.queue.pop_front() {
                Some(p) => p,
                None => {
                    c.worker = false;
                    return;
                }
            }
        };
        // A damaged file must never take the Home screen down with it.
        let read = std::panic::catch_unwind(|| prism_io::thumbnail::read(std::path::Path::new(&next), THUMB_PX)).ok().and_then(Result::ok).flatten();
        let image = read.and_then(|t| {
            let size = [t.width as usize, t.height as usize];
            (t.rgba.len() == size[0] * size[1] * 4).then(|| egui::ColorImage::from_rgba_unmultiplied(size, &t.rgba))
        });
        if let Some(e) = shared.lock().unwrap_or_else(PoisonError::into_inner).entries.get_mut(&next) {
            e.slot = Slot::Read(image);
        }
        ctx.request_repaint();
    }
}

/// The thumbnail of `path`: `Some(None)` while loading or when there is none (the card shows a
/// placeholder), queued on first use and again when the file changes.
fn thumbnail(ctx: &egui::Context, path: &str) -> Option<TextureHandle> {
    let shared = cache(ctx);
    let mut c = shared.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    let stale = match c.entries.get_mut(path) {
        None => true,
        Some(e) if now.duration_since(e.checked) > RECHECK && !matches!(e.slot, Slot::Loading) => {
            e.checked = now;
            modified(path) != e.modified
        }
        Some(_) => false,
    };
    if stale {
        c.entries.insert(path.to_string(), Entry { slot: Slot::Loading, modified: modified(path), checked: now });
        c.queue.push_back(path.to_string());
        if !c.worker {
            let (worker_cache, worker_ctx) = (shared.clone(), ctx.clone());
            c.worker = std::thread::Builder::new().name("home-thumbnails".into()).spawn(move || work(worker_cache, worker_ctx)).is_ok();
            if !c.worker {
                // No thread: show placeholders rather than retry every frame.
                for p in std::mem::take(&mut c.queue) {
                    if let Some(e) = c.entries.get_mut(&p) {
                        e.slot = Slot::Ready(None);
                    }
                }
            }
        }
    }
    let e = c.entries.get_mut(path)?;
    if let Slot::Read(img) = &mut e.slot {
        let tex = img.take().map(|i| ctx.load_texture(format!("home-thumb:{path}"), i, egui::TextureOptions::LINEAR));
        e.slot = Slot::Ready(tex);
    }
    match &e.slot {
        Slot::Ready(t) => t.clone(),
        _ => None,
    }
}

fn badge(ui: &egui::Ui, at: egui::Pos2, path: &str) {
    let psd = !path.to_ascii_lowercase().ends_with(".prism");
    let (text, fill) = if psd { ("Ps", Color32::from_rgb(0, 30, 54)) } else { ("Pr", Color32::from_rgb(60, 32, 110)) };
    let ink = if psd { Color32::from_rgb(49, 168, 255) } else { Color32::from_rgb(196, 160, 255) };
    let r = Rect::from_min_size(at, vec2(16.0, 14.0));
    ui.painter().rect_filled(r, 2.5, fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, crate::theme::semibold(8.5), ink);
}

/// One recent file. Returns true when clicked.
fn card(ui: &mut egui::Ui, path: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(CARD, Sense::click());
    let name = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| crate::file_open::display_name(path));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    ui.painter().rect_filled(r, t.radius, if resp.hovered() { t.hover } else { t.card });
    let thumb_area = Rect::from_min_size(r.min, vec2(CARD.x, THUMB_H));
    ui.painter().rect_filled(thumb_area, egui::CornerRadius { nw: t.radius as u8, ne: t.radius as u8, sw: 0, se: 0 }, t.dock);
    match thumbnail(ui.ctx(), path) {
        Some(tex) => {
            let size = tex.size_vec2();
            let s = (thumb_area.width() / size.x.max(1.0)).min(thumb_area.height() / size.y.max(1.0));
            let img = Rect::from_center_size(thumb_area.center(), size * s);
            ui.painter().image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => crate::icons::paint(ui, Rect::from_center_size(thumb_area.center(), vec2(40.0, 40.0)), "image", 34.0, t.text_faint),
    }
    badge(ui, pos2(r.left() + 10.0, thumb_area.bottom() - 18.0), path);
    let x = r.left() + 10.0;
    let name_g = crate::tab_strip::elided(ui, &name, egui::FontId::proportional(12.5), t.text, CARD.x - 20.0);
    ui.painter().galley(pos2(x, thumb_area.bottom() + 10.0), name_g, t.text);
    let folder = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let folder_g = crate::tab_strip::elided(ui, &folder, egui::FontId::proportional(11.0), t.text_faint, CARD.x - 20.0);
    ui.painter().galley(pos2(x, thumb_area.bottom() + 32.0), folder_g, t.text_faint);
    if resp.hovered() {
        ui.painter().rect_stroke(r, t.radius, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    resp.on_hover_text(path).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The bar across the top: the app mark, which returns to the open documents.
fn top_bar(app: &mut PrismApp, ui: &mut egui::Ui, bar: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(bar, 0.0, t.dock);
    let mark = Rect::from_center_size(pos2(bar.left() + 32.0, bar.center().y), vec2(32.0, 32.0));
    let back = !app.session.documents().is_empty();
    let resp = ui.interact(mark, ui.id().with("home-mark"), if back { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, back, tl!("Prism")));
    match crate::panels::brand_texture(ui.ctx()) {
        Some(tex) => {
            ui.painter().image(tex.id(), mark, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().text(mark.center(), Align2::CENTER_CENTER, tl!("Prism"), crate::theme::semibold(14.0), t.text);
        }
    }
    if back && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        leave(app);
    }
}

fn sidebar(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(30.0);
    let new_tip = crate::shortcuts::command_label(app, "New document…", "file.new");
    if crate::widgets::primary_button(ui, tl!("New file"), 120.0).on_hover_text(new_tip).clicked() {
        let fields = app.new_document_fields();
        app.ui.open_dialog(crate::state::DialogKind::NewDocument, fields);
    }
    ui.add_space(6.0);
    let open_tip = crate::shortcuts::command_label(app, "Open…", "file.open");
    let open = ui.add(egui::Label::new(RichText::new(tl!("Open")).size(13.0).color(t.text)).sense(Sense::click()));
    if open.on_hover_text(open_tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        let _ = app.open_dialog_file();
    }
    ui.add_space(14.0);
    crate::widgets::hairline(ui);
    ui.add_space(14.0);
    // Home: the page this is, marked like Photoshop's left rail.
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().line_segment([pos2(row.left(), row.top() + 3.0), pos2(row.left(), row.bottom() - 3.0)], Stroke::new(2.0, t.text));
    crate::icons::paint(ui, Rect::from_center_size(pos2(row.left() + 20.0, row.center().y), vec2(16.0, 16.0)), "house", 15.0, t.text);
    ui.painter().text(pos2(row.left() + 36.0, row.center().y), Align2::LEFT_CENTER, tl!("Home"), crate::theme::semibold(13.0), t.text);
}

/// The recent grid's columns and its left edge, centred in `main` like Photoshop's.
fn grid_layout(main: Rect) -> (usize, f32) {
    let columns = ((main.width() + GAP) / (CARD.x + GAP)).floor().clamp(1.0, MAX_COLUMNS as f32) as usize;
    let width = columns as f32 * (CARD.x + GAP) - GAP;
    (columns, main.left() + ((main.width() - width) / 2.0).max(0.0))
}

/// The Home screen in `ui`'s remaining space.
pub fn show(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let area = ui.available_rect_before_wrap();
    ui.painter().rect_filled(area, 0.0, t.canvas);
    // Esc goes back to the open documents, unless a dialog or text field has it.
    if ui.ctx().memory(|m| m.focused().is_none()) && app.ui.dialogs.is_empty() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        leave(app);
    }
    let bar = Rect::from_min_size(area.min, vec2(area.width(), TOP_BAR_H));
    top_bar(app, ui, bar);
    let body = Rect::from_min_max(pos2(area.left(), bar.bottom()), area.max);
    let side = Rect::from_min_max(body.min + vec2(24.0, 0.0), pos2(body.left() + SIDEBAR_W, body.bottom()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(side), |ui| sidebar(app, ui));
    let main = Rect::from_min_max(pos2(body.left() + SIDEBAR_W + 40.0, body.top() + 34.0), body.max - vec2(SIDEBAR_W * 0.5, 0.0));
    let (columns, left) = grid_layout(main);
    let main = Rect::from_min_max(pos2(left, main.top()), pos2(main.right().max(left + CARD.x), main.bottom()));
    let recent = recent_documents(&app.ui.recent_files);
    let mut open = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
        ui.label(RichText::new(tl!("Recent")).font(crate::theme::semibold(16.0)).color(t.text));
        ui.add_space(14.0);
        if recent.is_empty() {
            ui.label(RichText::new(tl!("Prism and Photoshop files you open appear here.")).color(t.text_dim));
            ui.add_space(6.0);
            ui.label(RichText::new(tl!("Drop an image or PSD anywhere to open it.")).color(t.text_faint));
            return;
        }
        egui::ScrollArea::vertical().id_salt("home-recent").auto_shrink([false, false]).show(ui, |ui| {
            for row in recent.chunks(columns) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for path in row {
                        if card(ui, path) {
                            open = Some(path.clone());
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });
    });
    if let Some(path) = open
        && let Err(e) = app.open_path(&path)
    {
        app.open_failed(&crate::file_open::display_name(&path), &e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn only_prism_and_photoshop_documents_are_listed() {
        let recent = ["/a/one.PSD", "/a/two.png", "/b/three.prism", "/c/four.psb", "/d/five.jpg", "/e/noext"].map(String::from);
        assert_eq!(recent_documents(&recent), vec!["/a/one.PSD".to_string(), "/b/three.prism".into(), "/c/four.psb".into()]);
    }

    fn harness(recent: Vec<String>) -> Harness<'static, PrismApp> {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.ui.recent_files = recent;
        // The first frame runs while building, before the app's fonts exist: draw from the next.
        let ready = egui::Id::new("home-test-ready");
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
            move |ui, app| {
                if ui.ctx().data(|d| d.get_temp::<bool>(ready)).unwrap_or(false) {
                    show(app, ui);
                }
            },
            app,
        );
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.ctx.data_mut(|d| d.insert_temp(ready, true));
        h.run_steps(3);
        h
    }

    #[test]
    fn recent_cards_show_and_a_missing_file_reports_instead_of_panicking() {
        let mut h = harness(vec!["/nowhere/Poster.psd".into(), "/nowhere/photo.png".into(), "/nowhere/Sketch.prism".into()]);
        assert!(h.query_by_label("Poster").is_some() && h.query_by_label("Sketch").is_some());
        assert!(h.query_by_label("photo").is_none(), "flat images are left to File › Open Recent");
        h.get_by_label("Poster").click();
        h.run_steps(3);
        assert!(h.state().session.documents().is_empty());
        assert!(!h.state().ui.dialogs.is_empty() || !h.state().ui.status.is_empty(), "the failure is reported");
    }

    #[test]
    fn new_file_opens_the_new_document_dialog() {
        let mut h = harness(Vec::new());
        assert!(h.query_by_label("Prism and Photoshop files you open appear here.").is_some());
        h.get_by_label("New file").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.dialogs.first().map(|d| d.kind), Some(crate::state::DialogKind::NewDocument));
    }

    /// The whole app window, with `docs` new documents.
    fn app_window(docs: usize) -> Harness<'static, PrismApp> {
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
            PrismApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
            let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
            for _ in 0..docs {
                app.run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
            }
            app
        });
        h.run_steps(4);
        h
    }

    #[test]
    fn home_fills_the_window_without_tools_or_panels() {
        let mut h = app_window(0);
        assert!(up(h.state()));
        assert!(h.query_by_label("New file").is_some());
        assert!(h.query_by_label("Layers").is_none(), "no panel dock around Home");
        assert!(h.query_by_label("Home").is_none(), "no options bar (its first control is the Home button)");
        // A new document leaves Home, and the workspace comes back.
        h.state_mut().run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
        h.run_steps(4);
        assert!(!up(h.state()));
        assert!(h.query_by_label("Layers").is_some() && h.query_by_label("Home").is_some());
    }

    #[test]
    fn the_app_mark_and_esc_return_to_the_open_documents() {
        let mut h = app_window(1);
        assert!(!up(h.state()));
        h.state_mut().ui.chrome.home = Some(1);
        h.run_steps(3);
        assert!(up(h.state()) && h.query_by_label("Layers").is_none());
        h.get_by_label("Prism").click();
        h.run_steps(3);
        assert!(!up(h.state()), "the mark goes back to the document");
        h.state_mut().ui.chrome.home = Some(1);
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(!up(h.state()), "so does Esc");
    }

    #[test]
    fn with_no_documents_home_stays_and_the_grid_stays_centred() {
        let mut h = app_window(0);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(up(h.state()), "nothing to go back to");
        let (columns, left) = grid_layout(Rect::from_min_size(pos2(100.0, 0.0), vec2(5000.0, 500.0)));
        assert_eq!(columns, MAX_COLUMNS);
        let width = MAX_COLUMNS as f32 * (CARD.x + GAP) - GAP;
        assert!((left - (100.0 + (5000.0 - width) / 2.0)).abs() < 0.5);
        assert_eq!(grid_layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(50.0, 500.0))), (1, 0.0));
    }

    #[test]
    fn thumbnails_load_in_the_background_from_a_real_psd() {
        let dir = std::env::temp_dir().join(format!("prism-home-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Green.psd");
        let mut b = prism_psd::PsdBuilder::new(8, 4);
        b.composite(prism_psd::PixelData::Rgba8([0, 200, 0, 255].repeat(32)));
        std::fs::write(&path, b.to_bytes().unwrap()).unwrap();
        let p = path.to_string_lossy().into_owned();
        let mut h = harness(vec![p.clone()]);
        let mut tex = None;
        for _ in 0..200 {
            h.run_steps(1);
            tex = thumbnail(&h.ctx, &p);
            if tex.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(tex.map(|t| t.size()), Some([8, 4]));
    }
}
