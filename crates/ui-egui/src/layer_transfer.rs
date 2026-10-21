//! Dragging layers to another document (#589), as in Photoshop: drag layers from the Layers panel,
//! or with the Move tool from the canvas, onto another document's tab or canvas. The tab shows its
//! document while the pointer is over it (spring-loaded), and releasing over another document's
//! canvas copies the layers there with `layer.copyToDocument` (one history step in the
//! destination). The canvas can be in another window: floating document windows (Window ›
//! Arrange › Float in Window) take drops from the main window and from each other.
//!
//! Where the copies land: a Move-tool drag puts the pixel that was grabbed under the pointer, and
//! a Layers panel drag centres the layers on it. With ⇧, or when released on the tab itself, they
//! keep their place when both documents have the same pixel size and are centred otherwise
//! (Photoshop's ⇧-drag). With Window › Arrange tiling several documents, releasing over another
//! document's tile drops there. Groups come along with their contents.
//!
//! A Move-tool drag that leaves its canvas for another document's ends as a move of nothing: the
//! source layers stay where they were (an ⌥-drag's copy is taken back).
//!
//! A Move-tool drag starting inside a selection or moving a floating piece stays a move: Photoshop
//! would copy just the selected pixels, which is not supported yet.

use egui::{Pos2, Rect, ViewportId};
use prism_doc::{DocId, LayerId};
use serde_json::json;

use crate::PrismApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// The layers being dragged and where from.
#[derive(Clone, Debug)]
struct Transfer {
    source: DocId,
    layers: Vec<u64>,
    /// The document point a Move-tool drag grabbed (`None` for a Layers panel drag).
    grab: Option<[f64; 2]>,
    /// What the ghost by the pointer says.
    label: String,
    /// The window holding the pointer (the button went down there): it gets the pointer events.
    viewport: ViewportId,
}

fn key() -> egui::Id {
    egui::Id::new("pc-layer-transfer")
}

fn get(ctx: &egui::Context) -> Option<Transfer> {
    ctx.data(|d| d.get_temp::<Transfer>(key()))
}

fn clear(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<Transfer>(key());
        d.remove::<Hover>(hover_key());
    });
}

/// The layers `id` stands for: the selection when it is one of the selected layers, else itself.
fn begin(app: &PrismApp, ctx: &egui::Context, id: LayerId, grab: Option<[f64; 2]>) -> bool {
    let Some(st) = app.session.active() else { return false };
    let layers = if st.is_layer_selected(id) { st.selected_layers() } else { vec![id] };
    let label = match layers.as_slice() {
        [one] => st.doc.layer(*one).map_or_else(String::new, |l| l.name.clone()),
        many => crate::i18n::trn(crate::i18n::current(), many.len() as u64, "{n} layer", "{n} layers"),
    };
    let t = Transfer { source: st.doc.id, layers: layers.iter().map(|l| l.0).collect(), grab, label, viewport: ctx.viewport_id() };
    ctx.data_mut(|d| d.insert_temp(key(), t));
    true
}

/// A Layers panel row drag began on `layer`: it can be dropped on another document.
pub fn begin_from_panel(app: &PrismApp, ctx: &egui::Context, layer: LayerId) {
    begin(app, ctx, layer, None);
}

/// A Move-tool drag of whole layers (not a selection or a floating piece) under way.
fn move_drag(app: &PrismApp) -> Option<[f64; 2]> {
    let d = app.drag.as_ref().filter(|d| d.tool == Tool::Move && d.sel_move.is_none())?;
    let st = app.session.active()?;
    let plain = app.ui.transform.is_none() && app.guide_drag.is_none() && st.doc.selection.is_none() && st.floating.is_none();
    plain.then_some(d.start)
}

/// The Move drag ends without moving anything and becomes a transfer of the active layers.
fn take_over_move(app: &mut PrismApp, ctx: &egui::Context, grab: [f64; 2]) -> bool {
    let Some(active) = app.session.active().and_then(|st| st.active_layer) else { return false };
    // An ⌥-drag's copy is taken back.
    app.drag = None;
    app.move_preview = None;
    crate::move_mods::abandon(app);
    let active = app.session.active().and_then(|st| st.active_layer).unwrap_or(active);
    begin(app, ctx, active, Some(grab))
}

/// The pointer, while layers are dragged (or a Move drag could become a transfer) with the button
/// held in `ctx`'s window: tabs only hit-test it then. Another window's pointer is where it was
/// last over that window.
pub fn pointer_if_armed(app: &PrismApp, ctx: &egui::Context) -> Option<egui::Pos2> {
    let here = ctx.viewport_id();
    let armed = match get(ctx) {
        Some(t) => t.viewport == here,
        None => move_drag(app).is_some() && crate::doc_windows::viewport(app.ui.active_window) == here,
    };
    if !armed {
        return None;
    }
    ctx.input(|i| i.pointer.latest_pos())
}

/// The pointer is over document tab `index` while layers are dragged: a Move drag becomes a
/// transfer (the source layers stay where they were), the tab's document is shown, and a release
/// right here drops the layers into it.
pub fn over_tab(app: &mut PrismApp, ctx: &egui::Context, index: usize) {
    if get(ctx).is_none() {
        let Some(grab) = move_drag(app) else { return };
        if app.session.active_index() == Some(index) || !take_over_move(app, ctx, grab) {
            return;
        }
    }
    if app.session.active_index() != Some(index) {
        crate::doc_windows::activate_main(app, index);
        // The Layers panel now lists the other document: its rows don't take the drag.
        ctx.data_mut(|d| d.remove::<u64>(egui::Id::new("layer-drag")));
    }
    if ctx.input(|i| i.pointer.any_released()) {
        drop(app, ctx, index, None, None);
    }
}

/// The canvas with a Move drag (`ctx` is its window) asks before it moves the layers: over another
/// document's canvas, in this window or another, the drag becomes a transfer instead and the
/// canvas lets go of it. True when it did.
pub fn leave_canvas(app: &mut PrismApp, ctx: &egui::Context) -> bool {
    if get(ctx).is_some() {
        return false;
    }
    let Some(grab) = move_drag(app) else { return false };
    let Some(source) = app.session.active().map(|st| st.doc.id) else { return false };
    if target(app, ctx, source).is_none() {
        return false;
    }
    take_over_move(app, ctx, grab)
}

/// Every frame in every window, after its documents: in the window holding the pointer, a release
/// over another document's canvas drops the layers there; while the button is held the canvas
/// under the pointer is outlined and a ghost follows the pointer.
pub fn finish(app: &mut PrismApp, ctx: &egui::Context) {
    let Some(t) = get(ctx) else { return };
    if ctx.viewport_id() != t.viewport {
        let source_open = t.viewport == ViewportId::ROOT || app.ui.windows.iter().any(|w| w.open && crate::doc_windows::viewport(Some(w.id)) == t.viewport);
        if source_open {
            paint_hover(ctx);
        } else {
            clear(ctx);
        }
        return;
    }
    let (pos, released, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.any_released(), i.pointer.primary_down()));
    let target = target(app, ctx, t.source);
    if !released {
        if !down {
            clear(ctx);
            return;
        }
        let hover = match &target {
            Some((at, _)) => Some(Hover { viewport: at.viewport, canvas: at.xf.rect, pointer: at.local, label: t.label.clone() }),
            // Spring-loaded onto another document's tab: the ghost follows the pointer here.
            None => pos.filter(|_| app.session.active().is_some_and(|st| st.doc.id != t.source)).map(|p| Hover {
                viewport: t.viewport,
                canvas: Rect::NOTHING,
                pointer: p,
                label: t.label.clone(),
            }),
        };
        if target.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::Copy);
        }
        ctx.data_mut(|d| {
            if let Some(h) = hover {
                d.insert_temp(hover_key(), h);
            } else {
                d.remove::<Hover>(hover_key());
            }
        });
        paint_hover(ctx);
        // The other windows outline the target and follow the ghost.
        ctx.request_repaint();
        return;
    }
    if let Some((at, point)) = target
        && let Some(dest) = app.session.documents().iter().position(|d| d.doc.id == at.document)
    {
        drop(app, ctx, dest, Some(point), Some(at.window));
    }
    clear(ctx);
}

/// A document canvas laid out this frame (see [`register`]), so a drag from another view or
/// window can find it.
#[derive(Clone, Copy, Debug)]
struct Target {
    viewport: ViewportId,
    /// The viewport's pass when it was laid out: views not laid out lately are gone.
    pass: u64,
    /// The floating document window it is in (`None`: the main window).
    window: Option<u64>,
    document: DocId,
    xf: ViewXform,
    /// The viewport's client origin in its points, and its pixels per point: a desktop pixel is
    /// `(local + origin) * scale`, whatever the window's display scale.
    origin: Pos2,
    scale: f32,
}

#[derive(Clone, Default)]
struct Targets(Vec<Target>);

fn targets_key() -> egui::Id {
    egui::Id::new("pc-layer-transfer-targets")
}

/// `ctx`'s client origin (points) and pixels per point. Without a native window (tests) the
/// origin is the desktop's.
fn placement(ctx: &egui::Context) -> (Pos2, f32) {
    (ctx.input(|i| i.viewport().inner_rect.map_or(Pos2::ZERO, |r| r.min)), ctx.pixels_per_point())
}

/// A document view (`ctx` is its window) is laid out with mapping `xf`.
pub fn register(app: &PrismApp, ctx: &egui::Context, document: DocId, xf: &ViewXform) {
    let viewport = ctx.viewport_id();
    let pass = ctx.cumulative_pass_nr();
    let window = app.ui.windows.iter().find(|w| w.open && crate::doc_windows::viewport(Some(w.id)) == viewport).map(|w| w.id);
    let (origin, scale) = placement(ctx);
    ctx.data_mut(|d| {
        let list = &mut d.get_temp_mut_or_default::<Targets>(targets_key()).0;
        // Last pass's views stay until this pass lays them out: tiles drawn after the one with
        // the drag can still be found.
        list.retain(|t| t.viewport != viewport || (t.document != document && pass.saturating_sub(t.pass) <= 1));
        list.push(Target { viewport, pass, window, document, xf: *xf, origin, scale });
    });
}

/// The view under the pointer of `ctx`'s window, and the pointer in that view's points.
struct Hit {
    viewport: ViewportId,
    window: Option<u64>,
    document: DocId,
    xf: ViewXform,
    local: Pos2,
}

/// The topmost document view under the pointer of `ctx`'s window: floating document windows
/// float above the main window, and of the views in one window the last drawn is on top.
fn under_pointer(app: &PrismApp, ctx: &egui::Context) -> Option<Hit> {
    let p = ctx.input(|i| i.pointer.latest_pos())?;
    let (origin, scale) = placement(ctx);
    let desktop = (p + origin.to_vec2()) * scale;
    let list = ctx.data(|d| d.get_temp::<Targets>(targets_key())).unwrap_or_default().0;
    let live = |t: &Target| {
        ctx.cumulative_pass_nr_for(t.viewport).saturating_sub(t.pass) <= 1 && t.window.is_none_or(|id| app.ui.windows.iter().any(|w| w.open && w.id == id))
    };
    let hit = |floating: bool| {
        list.iter().rev().filter(|t| t.window.is_some() == floating && t.scale > 0.0 && live(t)).find_map(|t| {
            let local = desktop / t.scale - t.origin.to_vec2();
            t.xf.rect.contains(local).then_some(Hit { viewport: t.viewport, window: t.window, document: t.document, xf: t.xf, local })
        })
    };
    hit(true).or_else(|| hit(false))
}

/// Another document's canvas under the pointer (not `source`'s, in any view), and the document
/// point there.
fn target(app: &PrismApp, ctx: &egui::Context, source: DocId) -> Option<(Hit, [f64; 2])> {
    let hit = under_pointer(app, ctx).filter(|h| h.document != source)?;
    let point = hit.xf.to_doc(hit.local);
    Some((hit, point))
}

/// Where the ghost is drawn: in the window showing the canvas under the pointer.
#[derive(Clone, Debug)]
struct Hover {
    viewport: ViewportId,
    /// The canvas to outline (`Rect::NOTHING`: none).
    canvas: Rect,
    pointer: Pos2,
    label: String,
}

fn hover_key() -> egui::Id {
    egui::Id::new("pc-layer-transfer-hover")
}

fn paint_hover(ctx: &egui::Context) {
    let Some(h) = ctx.data(|d| d.get_temp::<Hover>(hover_key())).filter(|h| h.viewport == ctx.viewport_id()) else { return };
    if h.canvas.is_positive() {
        let t = crate::theme::Tokens::get(ctx);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("layer-transfer-target")));
        painter.rect_stroke(h.canvas, 0.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    ghost(ctx, h.pointer, &h.label);
}

/// Copy the dragged layers into document `dest`, at document point `at` (see the module docs),
/// then bring forward the window it was dropped in (`window`: `Some(None)` is the main window,
/// `None` leaves the windows as they are).
fn drop(app: &mut PrismApp, ctx: &egui::Context, dest: usize, at: Option<[f64; 2]>, window: Option<Option<u64>>) {
    let Some(t) = get(ctx) else { return };
    clear(ctx);
    let docs = app.session.documents();
    let Some(src) = docs.iter().position(|d| d.doc.id == t.source) else { return };
    let Some(to) = docs.get(dest).filter(|_| dest != src) else { return };
    let same_size = to.doc.size == docs[src].doc.size;
    let mut p = json!({"document": dest, "source": src, "layers": t.layers});
    match at.filter(|_| !ctx.input(|i| i.modifiers.shift)) {
        Some(at) => match t.grab {
            Some(g) => p["offset"] = json!([(at[0] - g[0]).round(), (at[1] - g[1]).round()]),
            None => p["at"] = json!(at),
        },
        None => p["center"] = json!(!same_size),
    }
    if let Err(e) = app.run("layer.copyToDocument", p) {
        app.ui.status = e;
        app.ui.status_error = true;
        return;
    }
    match window {
        Some(Some(id)) => {
            if let Err(e) = crate::doc_windows::focus(app, ctx, id) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        // Dropped from a floating window onto the main one.
        Some(None) if t.viewport != ViewportId::ROOT => crate::doc_windows::focus_main(app, ctx, dest),
        Some(None) => crate::doc_windows::activate_main(app, dest),
        None => {}
    }
}

/// The dragged layers' name next to the pointer (also the Layers panel's reorder drag).
pub fn ghost(ctx: &egui::Context, p: egui::Pos2, label: &str) {
    let t = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("layer-transfer-ghost")));
    let g = painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), t.text);
    let r = egui::Rect::from_min_size(p + egui::vec2(12.0, -10.0), g.size() + egui::vec2(16.0, 8.0));
    painter.rect_filled(r, t.radius_sm, t.card.gamma_multiply(0.95));
    painter.rect_stroke(r, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    painter.galley(r.min + egui::vec2(8.0, 4.0), g, t.text);
}

#[cfg(test)]
mod tests;
