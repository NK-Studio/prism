//! Live rectangle corner widgets. Pointer drags preview through the regular compositor and
//! release dispatches one `shape.edit`; Alt/Option at the press edits just the grabbed corner.

use std::sync::Arc;

use egui::{Event, Modifiers, PointerButton, Stroke, vec2};
use photocraft_doc::{Document, LayerContent, LayerId, LiveShape};
use photocraft_geom::Rect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    PhotocraftApp,
    canvas::{ToolEvent, ViewXform},
    state::Tool,
    theme::Tokens,
};

const GRAB: f64 = 9.0;
const TAG: u64 = 1 << 57;
const DIRECTIONS: [[f64; 2]; 4] = [[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]];

/// Temporary pointer state exposed through `ui.inspect`, never restored as a held gesture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CornerDrag {
    pub document: u64,
    pub revision: u64,
    pub layer: u64,
    pub tool: Tool,
    pub corner: usize,
    pub rect: [f64; 4],
    pub base: [f64; 4],
    pub radii: [f64; 4],
    pub start: [f64; 2],
    pub linked: bool,
}

struct Preview {
    document: u64,
    revision: u64,
    layer: u64,
    radii: [f64; 4],
    key: u64,
    shown: Arc<Document>,
    area: Rect,
}

#[derive(Default)]
pub(crate) struct Runtime {
    preview: Option<Preview>,
    serial: u64,
    /// The native press stays owned through cancellation and its eventual release.
    native_press: bool,
}

fn allowed(tool: Tool) -> bool {
    matches!(tool, Tool::Rectangle | Tool::PathSelection | Tool::DirectSelection)
}

fn rectangle(app: &PhotocraftApp) -> Option<(LayerId, [f64; 4], [f64; 4])> {
    if !allowed(app.ui.tool)
        || app.ui.transform.is_some()
        || app.ui.mask_target
        || app.ui.vector_mask_target
        || !app.ui.view.shows(app.ui.view.show.target_path)
    {
        return None;
    }
    let st = app.session.active()?;
    let id = st.active_layer?;
    let path = st.doc.path_of(id)?;
    let layer = st.doc.layer_at(&path)?;
    let locks = st.doc.locks_at(&path);
    let visible = (1..=path.len()).all(|n| path.get(..n).and_then(|p| st.doc.layer_at(p)).is_some_and(|l| l.visible));
    if !visible || locks.all || locks.pixels || st.selected_layers().len() > 1 {
        return None;
    }
    let LayerContent::Shape(shape) = &layer.content else { return None };
    let Some(LiveShape::Rect { rect, radii }) = &shape.live else { return None };
    if !rect.iter().chain(radii).all(|v| v.is_finite()) || rect[2] <= 0.0 || rect[3] <= 0.0 || rect.iter().chain(radii).any(|v| v.abs() > 1e7) {
        return None;
    }
    Some((id, *rect, *radii))
}

/// Widgets stay distinct at the maximum radius and remain reachable on sharp rectangles.
fn handles(rect: [f64; 4], radii: [f64; 4], zoom: f32) -> [[f64; 2]; 4] {
    let [x, y, w, h] = rect;
    let cap = w.min(h) / 4.0;
    let minimum = (12.0 / f64::from(zoom.max(0.01))).min(cap);
    let radii = photocraft_vector::shapes::clamp_radii(w, h, radii);
    std::array::from_fn(|i| {
        let inset = radii.get(i).copied().unwrap_or(0.0).clamp(minimum, cap);
        match i {
            0 => [x + inset, y + inset],
            1 => [x + w - inset, y + inset],
            2 => [x + w - inset, y + h - inset],
            _ => [x + inset, y + h - inset],
        }
    })
}

fn hit(app: &PhotocraftApp, p: [f64; 2]) -> Option<usize> {
    let (_, rect, radii) = rectangle(app)?;
    let reach = GRAB / f64::from(app.current_zoom().max(0.01));
    handles(rect, radii, app.current_zoom())
        .iter()
        .enumerate()
        .map(|(i, q)| (i, (p[0] - q[0]).hypot(p[1] - q[1])))
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

pub(crate) fn cancel_stale(app: &mut PhotocraftApp) {
    let valid = app.ui.shape_corners.as_ref().is_none_or(|drag| {
        app.session.active().is_some_and(|st| st.doc.id.0 == drag.document && st.revision == drag.revision)
            && app.ui.tool == drag.tool
            && rectangle(app).is_some_and(|(id, _, _)| id.0 == drag.layer)
    });
    if !valid {
        app.ui.shape_corners = None;
        app.shape_corner_runtime.preview = None;
    }
    if app.shape_corner_runtime.preview.as_ref().is_some_and(|p| !app.session.active().is_some_and(|st| st.doc.id.0 == p.document && st.revision == p.revision))
    {
        app.shape_corner_runtime.preview = None;
    }
}

fn update(app: &mut PhotocraftApp, p: [f64; 2]) {
    let zoom = f64::from(app.current_zoom().max(0.01));
    let Some(d) = app.ui.shape_corners.as_mut() else { return };
    if (p[0] - d.start[0]).hypot(p[1] - d.start[1]) * zoom < 0.5 {
        d.radii = d.base;
        return;
    }
    let Some(direction) = DIRECTIONS.get(d.corner) else { return };
    let effective = photocraft_vector::shapes::clamp_radii(d.rect[2], d.rect[3], d.base);
    let Some(initial) = effective.get(d.corner).copied() else { return };
    let delta = ((p[0] - d.start[0]) * direction[0] + (p[1] - d.start[1]) * direction[1]) / 2.0;
    let max = if d.linked {
        d.rect[2].min(d.rect[3]) / 2.0
    } else {
        let prev = d.base.get((d.corner + 3) % 4).copied().unwrap_or(0.0).max(0.0);
        let next = d.base.get((d.corner + 1) % 4).copied().unwrap_or(0.0).max(0.0);
        let (horizontal, vertical) = if d.corner % 2 == 0 { (next, prev) } else { (prev, next) };
        (d.rect[2] - horizontal).min(d.rect[3] - vertical).max(0.0)
    };
    let radius = (initial + delta).round().clamp(0.0, max);
    d.radii = if d.linked { [radius; 4] } else { d.base };
    if let Some(r) = d.radii.get_mut(d.corner) {
        *r = radius;
    }
}

fn commit(app: &mut PhotocraftApp) {
    cancel_stale(app);
    let Some(drag) = app.ui.shape_corners.take() else { return };
    if drag.radii == drag.base {
        return;
    }
    if let Err(e) = app.run("shape.edit", json!({"layer": drag.layer, "radii": drag.radii})) {
        app.ui.status = e;
        app.ui.status_error = true;
    } else {
        crate::canvas::shown_as_document(app, photocraft_doc::DocId(drag.document), |key| (TAG..TAG * 2).contains(&key));
    }
}

/// Shared native/control pointer path. Misses fall through to drawing a new rectangle.
pub(crate) fn pointer(app: &mut PhotocraftApp, event: ToolEvent, mods: Modifiers) -> bool {
    cancel_stale(app);
    let p = match event {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    };
    if !p.iter().all(|v| v.is_finite()) {
        return app.ui.shape_corners.is_some();
    }
    match event {
        ToolEvent::Down { .. } => {
            if app.drag.is_some() {
                return false;
            }
            let Some(corner) = hit(app, p) else { return false };
            let Some((id, rect, base)) = rectangle(app) else { return false };
            let Some(st) = app.session.active() else { return false };
            app.ui.shape_corners = Some(CornerDrag {
                document: st.doc.id.0,
                revision: st.revision,
                layer: id.0,
                tool: app.ui.tool,
                corner,
                rect,
                base,
                radii: base,
                start: p,
                linked: !mods.alt,
            });
        }
        ToolEvent::Move { .. } => {
            if app.ui.shape_corners.is_none() {
                return false;
            }
            update(app, p);
        }
        ToolEvent::Up { .. } => {
            if app.ui.shape_corners.is_none() {
                return false;
            }
            update(app, p);
            commit(app);
        }
    }
    true
}

/// Own the actual press, including tiny drags below egui's normal drag threshold.
pub(crate) fn canvas_input(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, response: &egui::Response) -> bool {
    cancel_stale(app);
    if !app.shape_corner_runtime.native_press && (app.drag.is_some() || rectangle(app).is_none()) {
        return false;
    }
    let mut consumed = app.shape_corner_runtime.native_press;
    for event in ctx.input(|i| i.events.clone()) {
        match event {
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers }
                if response.contains_pointer() && response.rect.contains(pos) && !ctx.input(|i| i.key_down(egui::Key::Space)) =>
            {
                let p = xf.to_doc(pos);
                let mods = crate::workspace_ui::sticky_mods(app, modifiers);
                if pointer(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, mods) {
                    app.shape_corner_runtime.native_press = true;
                    consumed = true;
                }
            }
            Event::PointerMoved(pos) if app.shape_corner_runtime.native_press => {
                let p = xf.to_doc(pos);
                pointer(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, Modifiers::NONE);
            }
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, .. } if app.shape_corner_runtime.native_press => {
                let p = xf.to_doc(pos);
                pointer(app, ToolEvent::Up { x: p[0], y: p[1] }, Modifiers::NONE);
                app.shape_corner_runtime.native_press = false;
                consumed = true;
            }
            Event::WindowFocused(false) => {
                app.ui.shape_corners = None;
                app.shape_corner_runtime.native_press = false;
            }
            _ => {}
        }
    }
    // Some platforms lose the release when the pointer leaves the window.
    if app.shape_corner_runtime.native_press && !ctx.input(|i| i.pointer.primary_down()) {
        app.shape_corner_runtime.native_press = false;
        app.ui.shape_corners = None;
    }
    consumed
}

pub(crate) fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    cancel_stale(app);
    if app.ui.shape_corners.is_none() {
        return false;
    }
    let mods = ctx.input(|i| i.modifiers);
    if ctx.input_mut(|i| i.consume_key(mods, egui::Key::Escape)) {
        app.ui.shape_corners = None;
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(mods, egui::Key::Enter)) {
        commit(app);
        return true;
    }
    false
}

pub(crate) fn display_doc(app: &mut PhotocraftApp, index: usize) -> Option<(Arc<Document>, u64)> {
    cancel_stale(app);
    if app.session.active_index() != Some(index) {
        return None;
    }
    let drag = app.ui.shape_corners.as_ref()?;
    if drag.radii == drag.base {
        return None;
    }
    let runtime = &mut app.shape_corner_runtime;
    if let Some(p) = &runtime.preview
        && (p.document, p.revision, p.layer, p.radii) == (drag.document, drag.revision, drag.layer, drag.radii)
    {
        return Some((p.shown.clone(), p.key));
    }
    let st = app.session.documents().get(index)?;
    let mut shown = (*st.doc).clone();
    let LayerContent::Shape(mut shape) = shown.layer(LayerId(drag.layer))?.content.clone() else { return None };
    shape.live = Some(LiveShape::Rect { rect: drag.rect, radii: drag.radii });
    shape.path = photocraft_vector::shapes::live_path(shape.live.as_ref()?);
    shape.psd_raw = None;
    photocraft_engine::vector_cmds::refresh_shape(&shown, &mut shape);
    let [x, y, w, h] = drag.rect;
    let stroke = shape.stroke.as_ref().map_or(0.0, |s| f64::from(s.width).abs()).ceil().min(1e7) as i32;
    let area = Rect::new(x.floor() as i32, y.floor() as i32, (x + w).ceil() as i32, (y + h).ceil() as i32).inflate(stroke.saturating_add(2));
    shown.layer_mut(LayerId(drag.layer))?.content = LayerContent::Shape(shape);
    let shown = Arc::new(shown);
    runtime.serial = runtime.serial.wrapping_add(1);
    let key = TAG | (runtime.serial & (TAG - 1));
    runtime.preview = Some(Preview { document: drag.document, revision: drag.revision, layer: drag.layer, radii: drag.radii, key, shown: shown.clone(), area });
    Some((shown, key))
}

/// A radius edit changes only the rectangle's footprint, including its stroke and effects.
pub(crate) fn damage(app: &PhotocraftApp, document: u64, revision: u64, from: u64, to: u64) -> Option<Rect> {
    let p = app.shape_corner_runtime.preview.as_ref()?;
    let ours = |key| (TAG..TAG * 2).contains(&key);
    (p.document == document && p.revision == revision && (ours(from) || ours(to)) && (from == 0 || ours(from)) && (to == 0 || ours(to))).then_some(p.area)
}

pub(crate) fn cursor(app: &PhotocraftApp, point: [f64; 2]) -> Option<egui::CursorIcon> {
    let corner = app.ui.shape_corners.as_ref().map(|d| d.corner).or_else(|| hit(app, point))?;
    Some(if corner % 2 == 0 { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw })
}

/// Canvas overlays otherwise use the committed document, so their outline needs the same
/// pending radii as the compositor (also on the first frame of native pointer movement).
pub(crate) fn outline(app: &PhotocraftApp, layer: LayerId) -> Option<photocraft_doc::Path> {
    let drag = app.ui.shape_corners.as_ref().filter(|d| d.layer == layer.0)?;
    Some(photocraft_vector::shapes::live_path(&LiveShape::Rect { rect: drag.rect, radii: drag.radii }))
}

pub(crate) fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some((id, rect, base)) = rectangle(app) else { return };
    let drag = app.ui.shape_corners.as_ref().filter(|d| d.layer == id.0);
    let radii = drag.map_or(base, |d| d.radii);
    let t = Tokens::get(painter.ctx());
    let hovered = app.hover_doc.and_then(|p| hit(app, p));
    for (i, point) in handles(rect, radii, xf.zoom).iter().enumerate() {
        let at = xf.to_screen(point[0] as f32, point[1] as f32);
        let on = drag.is_some_and(|d| d.corner == i) || hovered == Some(i);
        painter.circle_filled(at, if on { 7.0 } else { 6.0 }, t.text);
        painter.circle_filled(at, 4.5, t.card);
        painter.circle_stroke(at, 4.5, Stroke::new(if on { 2.0 } else { 1.5 }, t.accent));
        painter.circle_filled(at, 1.5, t.accent);
        if on {
            let radius = photocraft_vector::shapes::clamp_radii(rect[2], rect[3], radii).get(i).copied().unwrap_or(0.0);
            let text = format!("{}: {} px", crate::i18n::t("Corner radius"), crate::widgets::fmt_num(radius));
            let text = painter.layout_no_wrap(text, crate::theme::medium(12.0), t.text);
            let size = text.size() + vec2(12.0, 8.0);
            let clip = painter.clip_rect();
            let pos = (at + vec2(12.0, 12.0)).min(clip.max - size).max(clip.min);
            let box_rect = egui::Rect::from_min_size(pos, size);
            painter.rect_filled(box_rect, t.radius_sm, t.card);
            painter.rect_stroke(box_rect, t.radius_sm, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
            painter.galley(box_rect.min + vec2(6.0, 4.0), text, t.text);
        }
    }
}

pub(crate) fn inspect(app: &PhotocraftApp) -> Value {
    let Some((layer, rect, base)) = rectangle(app) else { return Value::Null };
    let radii = app.ui.shape_corners.as_ref().map_or(base, |d| d.radii);
    json!({"layer": layer.0, "rect": rect, "radii": radii, "handles": handles(rect, radii, app.current_zoom()), "drag": app.ui.shape_corners})
}

#[cfg(test)]
mod tests;

/// A native document view changed during a held corner gesture. Keep the last preview footprint
/// so its pixels are invalidated, but discard the pointer and the uncommitted radii.
pub(crate) fn cancel_view_gesture(app: &mut PhotocraftApp) {
    app.ui.shape_corners = None;
    app.shape_corner_runtime.native_press = false;
}
