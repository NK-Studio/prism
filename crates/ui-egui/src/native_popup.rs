//! Windows: the menu bar's menus open as native popup menus, as Photoshop's do. Prism draws its
//! menu bar in its own title bar, so the titles stay egui buttons; a press on one asks the platform
//! ([`crate::Services::popup_menu`], the desktop app's Win32 one) to show that menu at the title.
//! A native popup is a window of its own above every other: the floating document windows (always
//! on top while Prism is active) no longer cover the menus, and stay visible under them.
//!
//! Pure and platform-free: this module lays the menus out ([`entries`]), the same rows in the same
//! order as the in-window menus ([`crate::menus::menu_bar`]), and runs what was chosen. The
//! platform shows the popup, and closes it to report [`Outcome::Switch`] when the pointer moves
//! onto another title or ← / → leave the menu, so the next one opens, as on a native menu bar.
//!
//! The popup's loop blocks the app, so it opens at the start of a frame ([`perform`]), two frames
//! after the press: the title is drawn pressed first. Help keeps its in-window menu (its search
//! field has no native equivalent), and so does automation input, which must never block.

use egui::{Context, Id, Rect, ViewportId};

use crate::PrismApp;
use crate::i18n::Lang;
use crate::menus::MenuItem;

/// A row of a native popup menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item {
        /// The command it runs.
        id: String,
        label: String,
        /// As shown (`Ctrl+Shift+N`).
        shortcut: Option<String>,
        enabled: bool,
        /// `Some` for check items.
        checked: Option<bool>,
    },
    Separator,
    Submenu {
        label: String,
        enabled: bool,
        children: Vec<Entry>,
    },
}

/// What the platform is asked to show. Rectangles are in desktop pixels.
#[derive(Clone, Debug)]
pub struct Request {
    pub entries: Vec<Entry>,
    /// Which title (index into `titles`) the menu belongs to; it opens below that title.
    pub index: usize,
    /// Every title of the bar, in order: moving onto another one switches menus.
    pub titles: Vec<Rect>,
    /// The main window (the popup's owner).
    pub window: Option<Rect>,
}

/// How the popup ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// An item was chosen; `alt` when ⌥/Alt was held (Merge Down → Stamp Down).
    Chosen { id: String, alt: bool },
    /// Closed to open the menu of title `n` instead.
    Switch(usize),
    /// Closed without a choice.
    Closed,
}

/// Shows a native popup menu and returns once it closes.
pub type PopupMenuFn = Box<dyn FnMut(&Request) -> Outcome>;

/// Menus with an in-window popup only.
fn in_window_only(top: &str) -> bool {
    top == "Help"
}

/// Does title `top` open as a native popup now (`ctx` is the window being drawn)?
pub(crate) fn native(app: &PrismApp, ctx: &Context, top: &str) -> bool {
    app.services.popup_menu.is_some() && !app.automation_input && !ctx.embed_viewports() && ctx.viewport_id() == ViewportId::ROOT && !in_window_only(top)
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    index: usize,
    /// The frame it was asked for.
    frame: u64,
}

fn pending_key() -> Id {
    Id::new("native-popup-pending")
}

/// The title whose native menu is about to open (draw it pressed).
pub(crate) fn pending(ctx: &Context) -> Option<usize> {
    ctx.data(|d| d.get_temp::<Pending>(pending_key())).map(|p| p.index)
}

fn request(ctx: &Context, index: usize, frame: u64) {
    ctx.data_mut(|d| d.insert_temp(pending_key(), Pending { index, frame }));
    ctx.request_repaint();
}

/// Title `index` of the bar was drawn: a press on it (`open`), or its in-window menu opened some
/// other way (keyboard, hovering from Help), asks for its native menu instead.
pub(crate) fn title(ctx: &Context, index: usize, title: &egui::Response, open: Option<egui::SetOpenCommand>, frame: u64) {
    let id = egui::Popup::default_response_id(title);
    let opening = matches!(open, Some(egui::SetOpenCommand::Bool(true)));
    if opening || egui::Popup::is_id_open(ctx, id) {
        egui::Popup::close_id(ctx, id);
        request(ctx, index, frame);
    }
}

/// The bar's titles as laid out: desktop pixels and their in-window popup ids.
#[derive(Clone, Default)]
struct Titles {
    rects: Vec<Rect>,
    ids: Vec<Id>,
}

fn titles_key() -> Id {
    Id::new("native-popup-titles")
}

/// `ctx`'s client origin (points) and pixels per point.
fn placement(ctx: &Context) -> (egui::Vec2, f32) {
    (ctx.input(|i| i.viewport().inner_rect.map_or(egui::Vec2::ZERO, |r| r.min.to_vec2())), ctx.pixels_per_point())
}

/// Remember where the bar's titles are (after the bar is drawn).
pub(crate) fn record_titles(ctx: &Context, buttons: &[egui::Response]) {
    let (origin, scale) = placement(ctx);
    let titles = Titles { rects: buttons.iter().map(|b| b.rect.translate(origin) * scale).collect(), ids: buttons.iter().map(egui::Popup::default_response_id).collect() };
    ctx.data_mut(|d| d.insert_temp(titles_key(), titles));
}

/// The rows of a menu at `depth` (1: a top-level menu), as the in-window menus draw them: leaves
/// and separators in order, a submenu where its first child is, no separator first, last or
/// twice in a row.
pub fn entries(items: &[&MenuItem], depth: usize, lang: Lang) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut shown: Vec<&str> = Vec::new();
    for it in items {
        if it.path.len() == depth {
            if it.label == "---" {
                if !out.is_empty() && !matches!(out.last(), Some(Entry::Separator)) {
                    out.push(Entry::Separator);
                }
                continue;
            }
            out.push(Entry::Item {
                id: it.id.clone(),
                label: crate::menus::translated_menu_label(lang, it),
                shortcut: it.shortcut.as_deref().map(crate::shortcuts::pretty),
                enabled: it.enabled,
                checked: it.checked,
            });
        } else if let Some(name) = it.path.get(depth) {
            let name = name.as_str();
            if shown.contains(&name) {
                continue;
            }
            shown.push(name);
            let child: Vec<&MenuItem> = items.iter().copied().filter(|c| c.path.get(depth).map(String::as_str) == Some(name)).collect();
            let children = entries(&child, depth + 1, lang);
            let enabled = children.iter().any(|c| !matches!(c, Entry::Separator));
            out.push(Entry::Submenu { label: crate::i18n::tr(lang, name).to_string(), enabled, children });
        }
    }
    if matches!(out.last(), Some(Entry::Separator)) {
        out.pop();
    }
    out
}

/// At the start of a frame: show the native menu asked for (two frames ago, so its title is drawn
/// pressed), then run what was chosen or open the next menu.
pub fn perform(app: &mut PrismApp, ctx: &Context) {
    let Some(p) = ctx.data(|d| d.get_temp::<Pending>(pending_key())) else { return };
    let top = crate::menus::TOP_MENUS.get(p.index).copied().unwrap_or("");
    if !native(app, ctx, top) || !app.ui.dialogs.is_empty() || app.discard.is_some() {
        ctx.data_mut(|d| d.remove::<Pending>(pending_key()));
        return;
    }
    if app.frame < p.frame.saturating_add(2) {
        ctx.request_repaint();
        return;
    }
    ctx.data_mut(|d| d.remove::<Pending>(pending_key()));
    let titles = ctx.data(|d| d.get_temp::<Titles>(titles_key())).unwrap_or_default();
    if titles.rects.get(p.index).is_none() {
        return;
    }
    let lang = crate::i18n::current();
    let mut items = crate::menus::menu_items(app);
    for it in &mut items {
        it.enabled &= crate::menus::modal_allows(app, &it.id);
    }
    let mine: Vec<&MenuItem> = items.iter().filter(|i| i.path.first().map(String::as_str) == Some(top)).collect();
    let window = ctx.input(|i| i.viewport().outer_rect).map(|r| r * ctx.pixels_per_point());
    let req = Request { entries: entries(&mine, 1, lang), index: p.index, titles: titles.rects.clone(), window };
    let Some(show) = app.services.popup_menu.as_mut() else { return };
    let outcome = show(&req);
    // The popup's loop ran outside egui: draw the result now.
    ctx.request_repaint();
    match outcome {
        Outcome::Chosen { id, alt } => {
            if !crate::menus::modal_allows(app, &id) {
                return;
            }
            let id = crate::menus::alt_click(id, alt);
            if let Err(e) = crate::menus::invoke(app, ctx, &id, serde_json::json!({})) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        Outcome::Switch(n) => match crate::menus::TOP_MENUS.get(n) {
            Some(next) if native(app, ctx, next) => request(ctx, n, app.frame),
            Some(_) => {
                if let Some(id) = titles.ids.get(n) {
                    egui::Popup::open_id(ctx, *id);
                }
            }
            None => {}
        },
        Outcome::Closed => {}
    }
}

#[cfg(test)]
mod tests;
