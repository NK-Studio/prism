//! Live arrow handles. Endpoint, wing-length and included-angle drags preview through the
//! compositor and commit a single shape.edit on release; Escape restores the original arrow.

use std::sync::Arc;

use egui::{Event, Modifiers, PointerButton, Stroke, vec2};
use prism_doc::{Document, LayerContent, LayerId, LiveShape};
use prism_geom::Rect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    PrismApp,
    canvas::{ToolEvent, ViewXform},
    state::Tool,
    theme::Tokens,
};

const GRAB: f64 = 9.0;
const TAG: u64 = 1 << 55;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArrowGeometry {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub head_size: f64,
    pub head_angle: f64,
}
impl ArrowGeometry {
    fn live(self) -> LiveShape {
        LiveShape::Arrow { from: self.from, to: self.to, head_size: self.head_size, head_angle: self.head_angle }
    }
    fn length(self) -> f64 {
        (self.to[0] - self.from[0]).hypot(self.to[1] - self.from[1])
    }
    fn path(self) -> prism_doc::Path {
        prism_vector::shapes::live_path(&self.live())
    }
}

/// Temporary pointer state exposed through `ui.inspect`, never restored as a held gesture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArrowDrag {
    pub document: u64,
    pub revision: u64,
    pub layer: u64,
    pub tool: Tool,
    pub handle: usize,
    pub base: ArrowGeometry,
    pub geometry: ArrowGeometry,
    pub start: [f64; 2],
}

struct Preview {
    document: u64,
    revision: u64,
    layer: u64,
    geometry: ArrowGeometry,
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
    matches!(tool, Tool::Arrow | Tool::Move | Tool::PathSelection | Tool::DirectSelection)
}

fn arrow(app: &PrismApp) -> Option<(LayerId, ArrowGeometry)> {
    if !allowed(app.ui.tool)
        || app.drag.as_ref().is_some_and(|d| d.tool == Tool::Move)
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
    if !visible || locks.all || locks.pixels || locks.position || st.selected_layers().len() > 1 {
        return None;
    }
    let LayerContent::Shape(shape) = &layer.content else { return None };
    let Some(LiveShape::Arrow { from, to, head_size, head_angle }) = shape.live else { return None };
    let geometry = ArrowGeometry { from, to, head_size, head_angle };
    if !from.iter().chain(to.iter()).all(|v| v.is_finite() && v.abs() <= 1e6)
        || !head_size.is_finite()
        || !(0.0..=10000.0).contains(&head_size)
        || head_size == 0.0
        || !head_angle.is_finite()
        || !(10.0..=160.0).contains(&head_angle)
        || geometry.length() < 1e-6
    {
        return None;
    }
    Some((id, geometry))
}

/// Tail/tip squares, two wing-length circles, and an angle diamond beyond the first wing.
fn handles(g: ArrowGeometry, zoom: f32) -> Option<[[f64; 2]; 5]> {
    let path = g.path();
    let wings = &path.subpaths.get(1)?.knots;
    let left = wings.first()?.anchor;
    let right = wings.get(2)?.anchor;
    let length = g.head_size.min(g.length());
    let extra = 24.0 / f64::from(zoom.max(0.01));
    let angle = [left.x + (left.x - g.to[0]) / length * extra, left.y + (left.y - g.to[1]) / length * extra];
    Some([g.from, g.to, [left.x, left.y], [right.x, right.y], angle])
}

fn hit(app: &PrismApp, p: [f64; 2]) -> Option<usize> {
    let (_, geometry) = arrow(app)?;
    let reach = GRAB / f64::from(app.current_zoom().max(0.01));
    handles(geometry, app.current_zoom())?
        .iter()
        .enumerate()
        .map(|(i, q)| (i, (p[0] - q[0]).hypot(p[1] - q[1])))
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

pub(crate) fn cancel_stale(app: &mut PrismApp) {
    let valid = app.ui.arrow_handles.as_ref().is_none_or(|drag| {
        app.session.active().is_some_and(|st| st.doc.id.0 == drag.document && st.revision == drag.revision)
            && app.ui.tool == drag.tool
            && arrow(app).is_some_and(|(id, _)| id.0 == drag.layer)
    });
    if !valid {
        app.ui.arrow_handles = None;
        app.arrow_handle_runtime.preview = None;
    }
    if app.arrow_handle_runtime.preview.as_ref().is_some_and(|p| !app.session.active().is_some_and(|st| st.doc.id.0 == p.document && st.revision == p.revision))
    {
        app.arrow_handle_runtime.preview = None;
    }
}

fn update(app: &mut PrismApp, p: [f64; 2], mods: Modifiers) {
    let zoom = f64::from(app.current_zoom().max(0.01));
    let Some(d) = app.ui.arrow_handles.as_mut() else { return };
    d.geometry = d.base;
    if (p[0] - d.start[0]).hypot(p[1] - d.start[1]) * zoom < 0.5 {
        return;
    }
    let delta = [p[0] - d.start[0], p[1] - d.start[1]];
    if d.handle < 2 {
        let (base, pivot) = if d.handle == 0 { (d.base.from, d.base.to) } else { (d.base.to, d.base.from) };
        let mut point = [base[0] + delta[0], base[1] + delta[1]];
        if mods.shift {
            let dx = point[0] - pivot[0];
            let dy = point[1] - pivot[1];
            let angle = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
            point = [pivot[0] + dx.hypot(dy) * angle.cos(), pivot[1] + dx.hypot(dy) * angle.sin()];
        }
        if point.iter().any(|v| !v.is_finite() || v.abs() > 1e6) || (point[0] - pivot[0]).hypot(point[1] - pivot[1]) < 1.0 {
            return;
        }
        if d.handle == 0 {
            d.geometry.from = point;
        } else {
            d.geometry.to = point;
        }
    } else if d.handle < 4 {
        let Some(points) = handles(d.base, zoom as f32) else { return };
        let Some(wing) = points.get(d.handle) else { return };
        let initial = d.base.head_size.min(d.base.length());
        let unit = [(wing[0] - d.base.to[0]) / initial, (wing[1] - d.base.to[1]) / initial];
        d.geometry.head_size = (initial + delta[0] * unit[0] + delta[1] * unit[1]).clamp(0.1, d.base.length().clamp(0.1, 10000.0));
    } else {
        let Some(points) = handles(d.base, zoom as f32) else { return };
        let Some(initial) = points.get(4) else { return };
        let v = [initial[0] + delta[0] - d.base.to[0], initial[1] + delta[1] - d.base.to[1]];
        let length = d.base.length();
        let u = [(d.base.to[0] - d.base.from[0]) / length, (d.base.to[1] - d.base.from[1]) / length];
        let back = -(v[0] * u[0] + v[1] * u[1]);
        let side = -v[0] * u[1] + v[1] * u[0];
        if v[0].hypot(v[1]) < 1e-6 {
            return;
        }
        d.geometry.head_angle = (2.0 * side.atan2(back).to_degrees()).clamp(10.0, 160.0);
    }
}

fn commit(app: &mut PrismApp) {
    cancel_stale(app);
    let Some(drag) = app.ui.arrow_handles.take() else { return };
    if drag.geometry == drag.base {
        return;
    }
    if let Err(e) = app.run("shape.edit", json!({"layer": drag.layer, "from":drag.geometry.from, "to":drag.geometry.to, "headSize":drag.geometry.head_size, "headAngle":drag.geometry.head_angle})) {
        app.ui.status = e;
        app.ui.status_error = true;
    } else {
        crate::canvas::shown_as_document(app, prism_doc::DocId(drag.document), |key| (TAG..TAG * 2).contains(&key));
    }
}

/// Shared native/control pointer path. Misses fall through to the active tool.
pub(crate) fn pointer(app: &mut PrismApp, event: ToolEvent, mods: Modifiers) -> bool {
    cancel_stale(app);
    let p = match event {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    };
    if !p.iter().all(|v| v.is_finite()) {
        return app.ui.arrow_handles.is_some();
    }
    match event {
        ToolEvent::Down { .. } => {
            if app.drag.is_some() {
                return false;
            }
            let Some(handle) = hit(app, p) else { return false };
            let Some((id, base)) = arrow(app) else { return false };
            let Some(st) = app.session.active() else { return false };
            app.ui.arrow_handles =
                Some(ArrowDrag { document: st.doc.id.0, revision: st.revision, layer: id.0, tool: app.ui.tool, handle, base, geometry: base, start: p });
        }
        ToolEvent::Move { .. } => {
            if app.ui.arrow_handles.is_none() {
                return false;
            }
            update(app, p, mods);
        }
        ToolEvent::Up { .. } => {
            if app.ui.arrow_handles.is_none() {
                return false;
            }
            update(app, p, mods);
            commit(app);
        }
    }
    true
}

/// Own the actual press, including tiny drags below egui's normal drag threshold.
pub(crate) fn canvas_input(app: &mut PrismApp, ctx: &egui::Context, xf: &ViewXform, response: &egui::Response) -> bool {
    cancel_stale(app);
    if !app.arrow_handle_runtime.native_press && (app.drag.is_some() || arrow(app).is_none()) {
        return false;
    }
    let mut consumed = app.arrow_handle_runtime.native_press;
    for event in ctx.input(|i| i.events.clone()) {
        match event {
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers }
                if response.contains_pointer() && response.rect.contains(pos) && !ctx.input(|i| i.key_down(egui::Key::Space)) =>
            {
                let p = xf.to_doc(pos);
                let mods = crate::workspace_ui::sticky_mods(app, modifiers);
                if pointer(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, mods) {
                    app.arrow_handle_runtime.native_press = true;
                    consumed = true;
                }
            }
            Event::PointerMoved(pos) if app.arrow_handle_runtime.native_press => {
                let p = xf.to_doc(pos);
                pointer(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, ctx.input(|i| i.modifiers));
            }
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, .. } if app.arrow_handle_runtime.native_press => {
                let p = xf.to_doc(pos);
                pointer(app, ToolEvent::Up { x: p[0], y: p[1] }, ctx.input(|i| i.modifiers));
                app.arrow_handle_runtime.native_press = false;
                consumed = true;
            }
            Event::WindowFocused(false) => {
                app.ui.arrow_handles = None;
                app.arrow_handle_runtime.native_press = false;
            }
            _ => {}
        }
    }
    // Some platforms lose the release when the pointer leaves the window.
    if app.arrow_handle_runtime.native_press && !ctx.input(|i| i.pointer.primary_down()) {
        app.arrow_handle_runtime.native_press = false;
        app.ui.arrow_handles = None;
    }
    consumed
}

pub(crate) fn keys(app: &mut PrismApp, ctx: &egui::Context) -> bool {
    cancel_stale(app);
    if app.ui.arrow_handles.is_none() {
        return false;
    }
    let mods = ctx.input(|i| i.modifiers);
    if ctx.input_mut(|i| i.consume_key(mods, egui::Key::Escape)) {
        app.ui.arrow_handles = None;
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(mods, egui::Key::Enter)) {
        commit(app);
        return true;
    }
    false
}

pub(crate) fn display_doc(app: &mut PrismApp, index: usize) -> Option<(Arc<Document>, u64)> {
    cancel_stale(app);
    if app.session.active_index() != Some(index) {
        return None;
    }
    let drag = app.ui.arrow_handles.as_ref()?;
    if drag.geometry == drag.base {
        return None;
    }
    let runtime = &mut app.arrow_handle_runtime;
    if let Some(p) = &runtime.preview
        && (p.document, p.revision, p.layer, p.geometry) == (drag.document, drag.revision, drag.layer, drag.geometry)
    {
        return Some((p.shown.clone(), p.key));
    }
    let st = app.session.documents().get(index)?;
    let mut shown = (*st.doc).clone();
    let LayerContent::Shape(mut shape) = shown.layer(LayerId(drag.layer))?.content.clone() else { return None };
    shape.live = Some(drag.geometry.live());
    shape.path = prism_vector::shapes::live_path(shape.live.as_ref()?);
    shape.psd_raw = None;
    prism_engine::vector_cmds::refresh_shape(&shown, &mut shape);
    let stroke = shape.stroke.as_ref().map_or(0.0, |s| f64::from(s.width).abs() * f64::from(s.miter_limit.max(1.0))).ceil().min(1e7) as i32;
    let footprint = |g: ArrowGeometry| {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for k in g.path().subpaths.iter().flat_map(|s| &s.knots) {
            min[0] = min[0].min(k.anchor.x);
            min[1] = min[1].min(k.anchor.y);
            max[0] = max[0].max(k.anchor.x);
            max[1] = max[1].max(k.anchor.y);
        }
        Rect::new(min[0].floor() as i32, min[1].floor() as i32, max[0].ceil() as i32, max[1].ceil() as i32).inflate(stroke.saturating_add(2))
    };
    let mut area = footprint(drag.base).union(&footprint(drag.geometry));
    if let Some(prev) = &runtime.preview {
        area = area.union(&prev.area);
    }
    shown.layer_mut(LayerId(drag.layer))?.content = LayerContent::Shape(shape);
    let shown = Arc::new(shown);
    runtime.serial = runtime.serial.wrapping_add(1);
    let key = TAG | (runtime.serial & (TAG - 1));
    runtime.preview =
        Some(Preview { document: drag.document, revision: drag.revision, layer: drag.layer, geometry: drag.geometry, key, shown: shown.clone(), area });
    Some((shown, key))
}

/// Invalidate both previous and current arrow footprints, including stroke and effects.
pub(crate) fn damage(app: &PrismApp, document: u64, revision: u64, from: u64, to: u64) -> Option<Rect> {
    let p = app.arrow_handle_runtime.preview.as_ref()?;
    let ours = |key| (TAG..TAG * 2).contains(&key);
    (p.document == document && p.revision == revision && (ours(from) || ours(to)) && (from == 0 || ours(from)) && (to == 0 || ours(to))).then_some(p.area)
}

pub(crate) fn cursor(app: &PrismApp, point: [f64; 2]) -> Option<egui::CursorIcon> {
    app.ui.arrow_handles.as_ref().map(|d| d.handle).or_else(|| hit(app, point))?;
    Some(if app.ui.arrow_handles.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab })
}

pub(crate) fn outline(app: &PrismApp, layer: LayerId) -> Option<prism_doc::Path> {
    Some(app.ui.arrow_handles.as_ref().filter(|d| d.layer == layer.0)?.geometry.path())
}

pub(crate) fn draw(app: &PrismApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some((id, base)) = arrow(app) else { return };
    let drag = app.ui.arrow_handles.as_ref().filter(|d| d.layer == id.0);
    let geometry = drag.map_or(base, |d| d.geometry);
    let Some(points) = handles(geometry, xf.zoom) else { return };
    let t = Tokens::get(painter.ctx());
    let hovered = app.hover_doc.and_then(|p| hit(app, p));
    let screen = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    if let (Some(a), Some(b)) = (points.get(2), points.get(4)) {
        painter.line_segment([screen(*a), screen(*b)], Stroke::new(1.0, t.accent));
    }
    for (i, point) in points.iter().enumerate() {
        let at = screen(*point);
        let on = drag.is_some_and(|d| d.handle == i) || hovered == Some(i);
        let radius = if on { 6.0 } else { 5.0 };
        let stroke = Stroke::new(if on { 2.0 } else { 1.5 }, t.accent);
        if i == 4 {
            painter.add(egui::Shape::convex_polygon(
                vec![at + vec2(0.0, -radius), at + vec2(radius, 0.0), at + vec2(0.0, radius), at + vec2(-radius, 0.0)],
                t.card,
                stroke,
            ));
        } else if i < 2 {
            let rect = egui::Rect::from_center_size(at, vec2(radius * 2.0, radius * 2.0));
            painter.rect_filled(rect, 1.0, t.card);
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
        } else {
            painter.circle_filled(at, radius, t.card);
            painter.circle_stroke(at, radius, stroke);
        }
        if on && i >= 2 {
            let (label, value, unit) = if i == 4 {
                (crate::i18n::t("Arrowhead angle"), geometry.head_angle, "°")
            } else {
                (crate::i18n::t("Arrowhead length"), geometry.head_size.min(geometry.length()), " px")
            };
            let text = painter.layout_no_wrap(format!("{label}: {}{unit}", crate::widgets::fmt_num(value)), crate::theme::medium(12.0), t.text);
            let size = text.size() + vec2(12.0, 8.0);
            let clip = painter.clip_rect();
            let pos = (at + vec2(12.0, 12.0)).min(clip.max - size).max(clip.min);
            let rect = egui::Rect::from_min_size(pos, size);
            painter.rect_filled(rect, t.radius_sm, t.card);
            painter.rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
            painter.galley(rect.min + vec2(6.0, 4.0), text, t.text);
        }
    }
}

pub(crate) fn inspect(app: &PrismApp) -> Value {
    let Some((layer, base)) = arrow(app) else { return Value::Null };
    let geometry = app.ui.arrow_handles.as_ref().map_or(base, |d| d.geometry);
    json!({"layer":layer.0,"geometry":geometry,"handles":handles(geometry,app.current_zoom()),"roles":["tail","tip","length","length","angle"],"drag":app.ui.arrow_handles})
}

#[cfg(test)]
mod tests;

/// A native document view changed during a held arrow gesture. Keep the last preview footprint
/// so its pixels are invalidated, but discard the pointer and the uncommitted geometry.
pub(crate) fn cancel_view_gesture(app: &mut PrismApp) {
    app.ui.arrow_handles = None;
    app.arrow_handle_runtime.native_press = false;
}
