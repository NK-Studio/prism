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
//!
//! The layers are carried visibly ([`look`]): a picture of them follows the pointer, shows where
//! they will land over another document, and settles there on release.

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
    /// A picture of the layers (`None`: they show no pixels).
    preview: Option<look::Preview>,
    count: usize,
    /// When the drag began (the card rises then).
    started: f64,
    /// How the carried layers look (Preferences › Tools › Layer drag look).
    style: look::Style,
    /// A Move-tool drag: the picture's top left from the grabbed point (document pixels) and the
    /// zoom of the view it came from, so a ghost hangs from the pointer where it was grabbed.
    anchor: Option<(egui::Vec2, f32)>,
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
    let preview = look::preview(ctx, &st.doc, &layers);
    let anchor = grab.zip(preview.as_ref()).map(|(g, pv)| {
        let zoom = view_zoom(ctx, st.doc.id).unwrap_or(1.0);
        (pv.bounds.min - egui::pos2(g[0] as f32, g[1] as f32), zoom)
    });
    let t = Transfer {
        source: st.doc.id,
        layers: layers.iter().map(|l| l.0).collect(),
        grab,
        label,
        viewport: ctx.viewport_id(),
        preview,
        count: layers.len(),
        started: ctx.input(|i| i.time),
        style: app.session.prefs().tools.layer_drag_look,
        anchor,
    };
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
        drop(app, ctx, index, None, None, None);
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
/// over another document's canvas drops the layers there; while the button is held the layers
/// follow the pointer, or show where they would land over another document.
pub fn finish(app: &mut PrismApp, ctx: &egui::Context) {
    look::paint_settle(ctx);
    let Some(t) = get(ctx) else { return };
    if ctx.viewport_id() != t.viewport {
        let source_open = t.viewport == ViewportId::ROOT || app.ui.windows.iter().any(|w| w.open && crate::doc_windows::viewport(Some(w.id)) == t.viewport);
        if source_open {
            paint_hover(ctx, &t);
        } else {
            clear(ctx);
        }
        return;
    }
    let (pos, released, down, shift, now) =
        ctx.input(|i| (i.pointer.latest_pos(), i.pointer.any_released(), i.pointer.primary_down(), i.modifiers.shift, i.time));
    let target = target(app, ctx, t.source);
    let landing = target.as_ref().and_then(|(hit, at)| landing(app, &t, hit.document, *at, shift).map(|r| look::quad(&hit.xf, r)));
    if !released {
        if !down {
            clear(ctx);
            return;
        }
        let hover = target.as_ref().map(|(at, _)| Hover { viewport: at.viewport, canvas: at.xf.rect, pointer: at.local, landing });
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
        match (&target, pos) {
            (Some(_), _) => paint_hover(ctx, &t),
            // Carried: the layers follow the pointer.
            (None, Some(p)) => look::carry(ctx, t.style, p, t.preview.as_ref(), &t.label, t.count, now - t.started, t.anchor),
            (None, None) => {}
        }
        // The other windows show where the layers would land.
        ctx.request_repaint();
        return;
    }
    if let Some((at, point)) = target
        && let Some(dest) = app.session.documents().iter().position(|d| d.doc.id == at.document)
    {
        drop(app, ctx, dest, Some(point), Some(at.window), landing.map(|q| (at.viewport, q)));
    }
    clear(ctx);
}

/// Where the dragged layers land in document `dest` (its pixels) when released at `at`, as
/// [`drop`] places them: the grabbed pixel under the pointer (Move tool) or centred on it (Layers
/// panel); with Shift in place, or centred when the documents differ in size.
fn landing(app: &PrismApp, t: &Transfer, dest: DocId, at: [f64; 2], shift: bool) -> Option<Rect> {
    let b = t.preview.as_ref()?.bounds;
    let docs = app.session.documents();
    let size = |id: DocId| docs.iter().find(|d| d.doc.id == id).map(|d| d.doc.size);
    let (from, to) = (size(t.source)?, size(dest)?);
    let c = b.center();
    let (dx, dy) = match (shift, t.grab) {
        (true, _) if from == to => (0.0, 0.0),
        (true, _) => (to.width as f32 / 2.0 - c.x, to.height as f32 / 2.0 - c.y),
        (false, Some(g)) => ((at[0] - g[0]).round() as f32, (at[1] - g[1]).round() as f32),
        (false, None) => (at[0] as f32 - c.x, at[1] as f32 - c.y),
    };
    Some(b.translate(egui::vec2(dx.round(), dy.round())))
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

/// The zoom of document `document`'s view in `ctx`'s window.
fn view_zoom(ctx: &egui::Context, document: DocId) -> Option<f32> {
    let here = ctx.viewport_id();
    let list = ctx.data(|d| d.get_temp::<Targets>(targets_key())).unwrap_or_default().0;
    list.iter().rev().find(|t| t.viewport == here && t.document == document).map(|t| t.xf.zoom)
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

/// The other document's canvas under the pointer, drawn in its own window.
#[derive(Clone, Debug)]
struct Hover {
    viewport: ViewportId,
    /// The canvas, outlined.
    canvas: Rect,
    pointer: Pos2,
    /// Where the layers would land there (`None`: they show no pixels).
    landing: Option<[Pos2; 4]>,
}

fn hover_key() -> egui::Id {
    egui::Id::new("pc-layer-transfer-hover")
}

fn paint_hover(ctx: &egui::Context, t: &Transfer) {
    let Some(h) = ctx.data(|d| d.get_temp::<Hover>(hover_key())).filter(|h| h.viewport == ctx.viewport_id()) else { return };
    if let (Some(q), Some(pv)) = (h.landing, t.preview.as_ref()) {
        look::landing(ctx, t.style, h.canvas, q, pv);
        return;
    }
    let tokens = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("layer-transfer-target")));
    painter.rect_stroke(h.canvas, 0.0, egui::Stroke::new(2.0, tokens.accent), egui::StrokeKind::Inside);
    look::carry(ctx, t.style, h.pointer, t.preview.as_ref(), &t.label, t.count, ctx.input(|i| i.time) - t.started, t.anchor);
}

/// Are layers being carried (the Layers panel's reorder drag then shows them, not its label)?
pub fn carrying(ctx: &egui::Context) -> bool {
    get(ctx).is_some()
}

/// Copy the dragged layers into document `dest`, at document point `at` (see the module docs),
/// then bring forward the window it was dropped in (`window`: `Some(None)` is the main window,
/// `None` leaves the windows as they are). `settle`: where they land, in which window.
fn drop(app: &mut PrismApp, ctx: &egui::Context, dest: usize, at: Option<[f64; 2]>, window: Option<Option<u64>>, settle: Option<(ViewportId, [Pos2; 4])>) {
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
    if let (Some((viewport, q)), Some(pv)) = (settle, t.preview.as_ref()) {
        look::settle(ctx, t.style, viewport, q, pv);
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

mod look;

#[cfg(test)]
mod tests;
