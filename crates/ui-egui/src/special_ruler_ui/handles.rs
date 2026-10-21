//! Editing special rulers on the canvas, as CLIP STUDIO's Operation tool does: with the Move
//! tool on a layer that owns shown rulers, every parallel vanishing point gets an anchor on its
//! line (drag: move its guides) and an angle handle along it (drag: turn them around the anchor;
//! ⇧ steps by 15°), and
//! every converging point can be dragged. A drag is one `layer.specialRuler.edit` per move under
//! one `coalesce` key, so it is one history step and replays like any command.
//!
//! A parallel point whose guides all lie outside the document (the "Isometric ruler"'s vertical
//! direction) gets a virtual anchor at the document centre, so its line shows and can be grabbed.

use prism_doc::special_ruler::{RulerKind, SpecialRulers, VanishTarget, angle_dir, normalize_angle};
use prism_doc::{Document, LayerId};
use serde::Serialize;
use serde_json::{Value, json};

use crate::PrismApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// Screen points from an anchor to its angle handle.
pub const ANGLE_REACH: f64 = 64.0;
/// Screen points between virtual anchors, so they never sit on each other.
const VIRTUAL_STEP: f64 = 24.0;
/// Grab radius in screen points.
pub const HIT: f64 = 7.0;
/// Angle steps while ⇧ is held (degrees).
const SHIFT_STEP: f64 = 15.0;
/// Converging points further away than this get no handle.
const MAX_POINT: f64 = 1e6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A point on a parallel line (a guide, or the virtual centre anchor): moves the line.
    Anchor,
    /// Along the line from its anchor: turns the direction.
    Angle,
    /// A converging vanishing point (or a parallel line ruler's centre).
    Point,
}

/// One grabbable handle, in document pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handle {
    pub ruler: usize,
    /// Vanishing point index (0 for a parallel line ruler).
    pub point: usize,
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    /// The anchor an angle handle turns around (its own position for the others).
    #[serde(skip)]
    pub anchor: [f64; 2],
    /// The anchor is not one of the point's guides.
    #[serde(skip)]
    pub virtual_anchor: bool,
}

/// A handle being dragged.
#[derive(Clone, Debug)]
pub struct RulerDrag {
    pub handle: Handle,
    /// The point's guides when the drag began (moves are computed from them, not accumulated).
    pub guides: Vec<[f64; 2]>,
    /// The direction when the drag began (y-up degrees; 0 for converging points).
    pub angle: f64,
    pub key: String,
    /// An edit has run (a press without movement changes nothing).
    pub moved: bool,
}

fn inside(doc: &Document, p: [f64; 2]) -> bool {
    p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= f64::from(doc.size.width) && p[1] <= f64::from(doc.size.height)
}

/// The shown rulers `layer` owns: the ones its handles edit.
pub fn own(doc: &Document, layer: LayerId) -> Option<&SpecialRulers> {
    doc.layer(layer)?.special_rulers.as_ref().filter(|r| r.shown && !r.is_empty())
}

/// Where the line of parallel point `point` (the `n`-th without an in-document guide) is held.
fn anchor(doc: &Document, guides: &[[f64; 2]], angle_deg: f64, n: usize, zoom: f64) -> ([f64; 2], bool) {
    if let Some(g) = guides.iter().find(|g| inside(doc, **g)) {
        return (*g, false);
    }
    let d = angle_dir(angle_deg);
    let off = n as f64 * VIRTUAL_STEP / zoom;
    ([f64::from(doc.size.width) / 2.0 - d[1] * off, f64::from(doc.size.height) / 2.0 + d[0] * off], true)
}

/// Every handle of `layer`'s own rulers at `zoom` (screen points per document pixel).
pub fn handles(doc: &Document, layer: LayerId, zoom: f64) -> Vec<Handle> {
    let zoom = if zoom.is_finite() { zoom.max(1e-3) } else { 1.0 };
    let Some(set) = own(doc, layer) else { return Vec::new() };
    let mut out = Vec::new();
    let mut virtual_n = 0;
    let push_pair = |out: &mut Vec<Handle>, ruler: usize, point: usize, (a, virt): ([f64; 2], bool), angle_deg: f64| {
        let d = angle_dir(angle_deg);
        let tip = [a[0] + d[0] * ANGLE_REACH / zoom, a[1] + d[1] * ANGLE_REACH / zoom];
        out.push(Handle { ruler, point, kind: Kind::Anchor, x: a[0], y: a[1], anchor: a, virtual_anchor: virt });
        out.push(Handle { ruler, point, kind: Kind::Angle, x: tip[0], y: tip[1], anchor: a, virtual_anchor: virt });
    };
    for (ri, r) in set.rulers.iter().enumerate() {
        match &r.kind {
            RulerKind::Parallel { angle_deg, center } => push_pair(&mut out, ri, 0, (*center, false), *angle_deg),
            RulerKind::Perspective(per) => {
                for (pi, vp) in per.points.iter().enumerate() {
                    match vp.target {
                        VanishTarget::Parallel { angle_deg } => {
                            let a = anchor(doc, &vp.guides, angle_deg, virtual_n, zoom);
                            if a.1 {
                                virtual_n += 1;
                            }
                            push_pair(&mut out, ri, pi, a, angle_deg);
                        }
                        VanishTarget::Point(q) if q.iter().all(|v| v.is_finite() && v.abs() <= MAX_POINT) => {
                            out.push(Handle { ruler: ri, point: pi, kind: Kind::Point, x: q[0], y: q[1], anchor: q, virtual_anchor: false })
                        }
                        VanishTarget::Point(_) => {}
                    }
                }
            }
        }
    }
    out
}

/// The handle within [`HIT`] screen points of `p`, nearest first.
pub fn at(handles: &[Handle], p: [f64; 2], zoom: f64) -> Option<Handle> {
    let reach = HIT / zoom.max(1e-3);
    handles
        .iter()
        .map(|h| (h, (h.x - p[0]).hypot(h.y - p[1])))
        .filter(|(_, d)| d.is_finite() && *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| *h)
}

/// The direction from `from` to `to` in y-up degrees, `None` when they (almost) coincide.
fn angle_to(from: [f64; 2], to: [f64; 2], shift: bool) -> Option<f64> {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let len = dx.hypot(dy);
    if !len.is_finite() || len <= 1e-6 {
        return None;
    }
    let a = (-dy).atan2(dx).to_degrees();
    Some(normalize_angle(if shift { (a / SHIFT_STEP).round() * SHIFT_STEP } else { a }))
}

/// What `layer.specialRuler.edit` receives for dragging `drag` to `p`; `None` for no change.
pub fn edit_params(doc: &Document, layer: LayerId, drag: &RulerDrag, p: [f64; 2], shift: bool) -> Option<Value> {
    if !p.iter().all(|v| v.is_finite()) {
        return None;
    }
    let h = drag.handle;
    let parallel_ruler = matches!(own(doc, layer)?.rulers.get(h.ruler)?.kind, RulerKind::Parallel { .. });
    let mut v = match (h.kind, parallel_ruler) {
        (Kind::Angle, true) => json!({ "angle": angle_to(h.anchor, p, shift)? }),
        (Kind::Anchor, true) | (Kind::Point, true) => json!({ "center": p }),
        (Kind::Angle, false) => {
            // The guides turn with the line around the anchor, so a guide line keeps both ends.
            let a = angle_to(h.anchor, p, shift)?;
            let (sin, cos) = (a - drag.angle).to_radians().sin_cos();
            let guides: Vec<[f64; 2]> = drag
                .guides
                .iter()
                .map(|g| {
                    let (ux, uy) = (g[0] - h.anchor[0], h.anchor[1] - g[1]);
                    [h.anchor[0] + ux * cos - uy * sin, h.anchor[1] - (ux * sin + uy * cos)]
                })
                .collect();
            json!({ "point": h.point, "parallel": a, "guides": guides })
        }
        (Kind::Anchor, false) => {
            // The guides move together, so a guide line keeps both ends; a virtual anchor's
            // line gets its first guide on the document instead.
            let guides: Vec<[f64; 2]> = if h.virtual_anchor {
                vec![p]
            } else {
                let (dx, dy) = (p[0] - h.anchor[0], p[1] - h.anchor[1]);
                drag.guides.iter().map(|g| [g[0] + dx, g[1] + dy]).collect()
            };
            json!({ "point": h.point, "guides": guides })
        }
        (Kind::Point, false) => json!({ "point": h.point, "x": p[0], "y": p[1] }),
    };
    v["layer"] = json!(layer.0);
    v["index"] = json!(h.ruler);
    v["coalesce"] = json!(drag.key);
    Some(v)
}

/// The handles are live: Move tool, no transform box, View › Extras on.
pub fn active(app: &PrismApp) -> bool {
    app.ui.tool == Tool::Move && app.ui.view.extras && !crate::transform_tool::modal(app)
}

/// The handles shown now (empty when they are not live).
pub fn current(app: &PrismApp) -> Vec<Handle> {
    let Some(st) = app.session.active().filter(|_| active(app)) else { return Vec::new() };
    let Some(layer) = st.active_layer else { return Vec::new() };
    handles(&st.doc, layer, f64::from(app.current_zoom()))
}

/// The pointer path: a press on a handle starts a drag, moves edit the ruler, release ends it.
/// `true` when the event was a ruler handle's.
pub fn pointer(app: &mut PrismApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if app.drag.is_some() || app.ruler_drag.is_some() {
                return false;
            }
            let zoom = f64::from(app.current_zoom());
            let Some(h) = at(&current(app), [x, y], zoom) else { return false };
            let (guides, angle) = app
                .session
                .active()
                .and_then(|st| own(&st.doc, st.active_layer?)?.rulers.get(h.ruler).cloned())
                .and_then(|r| match r.kind {
                    RulerKind::Perspective(p) => p.points.get(h.point).map(|v| {
                        let a = if let VanishTarget::Parallel { angle_deg } = v.target { angle_deg } else { 0.0 };
                        (v.guides.clone(), a)
                    }),
                    RulerKind::Parallel { angle_deg, .. } => Some((Vec::new(), angle_deg)),
                })
                .unwrap_or_default();
            app.ruler_handle_seq = app.ruler_handle_seq.wrapping_add(1);
            app.ruler_drag = Some(RulerDrag { handle: h, guides, angle, key: format!("ruler-handle-{}", app.ruler_handle_seq), moved: false });
            true
        }
        ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => {
            let Some(drag) = app.ruler_drag.clone() else { return false };
            let up = matches!(ev, ToolEvent::Up { .. });
            let params = app.session.active().and_then(|st| edit_params(&st.doc, st.active_layer?, &drag, [x, y], mods.shift));
            if let Some(p) = params {
                let angle = p.get("parallel").or_else(|| p.get("angle")).and_then(Value::as_f64);
                match app.run("layer.specialRuler.edit", p) {
                    Ok(_) => {
                        if let Some(d) = app.ruler_drag.as_mut() {
                            d.moved = true;
                        }
                        if let Some(a) = angle {
                            app.ui.status = format!("{a:.1}°");
                            app.ui.status_error = false;
                        }
                    }
                    Err(e) => {
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                }
            }
            if up {
                app.ruler_drag = None;
            }
            true
        }
    }
}

/// The cursor over a handle (or while dragging one).
pub fn cursor(app: &PrismApp, p: [f64; 2]) -> Option<egui::CursorIcon> {
    if app.ruler_drag.is_some() {
        return Some(egui::CursorIcon::Grabbing);
    }
    at(&current(app), p, f64::from(app.current_zoom())).map(|_| egui::CursorIcon::Grab)
}

/// Lines through virtual anchors (otherwise nothing shows for a direction whose guides are all
/// outside the document) and, while handles are live, the handles. Called by
/// [`super::draw`] for the active layer's own rulers.
pub fn draw(app: &PrismApp, painter: &egui::Painter, xf: &crate::canvas::ViewXform, doc: &Document, layer: LayerId, color: egui::Color32) {
    use egui::{Shape, Stroke, vec2};
    let zoom = f64::from(xf.zoom);
    let all = handles(doc, layer, zoom);
    let Some(set) = own(doc, layer) else { return };
    for h in all.iter().filter(|h| h.kind == Kind::Anchor && h.virtual_anchor) {
        if let Some(RulerKind::Perspective(per)) = set.rulers.get(h.ruler).map(|r| &r.kind)
            && let Some(VanishTarget::Parallel { angle_deg }) = per.points.get(h.point).map(|v| v.target)
        {
            super::line(painter, xf, h.anchor, angle_dir(angle_deg), Stroke::new(1.0, color));
        }
    }
    if !active(app) {
        return;
    }
    let grabbed = app.ruler_drag.as_ref().map(|d| d.handle);
    for h in &all {
        let at = xf.to_screen(h.x as f32, h.y as f32);
        let hot = grabbed.is_some_and(|g| g.ruler == h.ruler && g.point == h.point && g.kind == h.kind);
        let fill = if hot { color } else { egui::Color32::WHITE };
        match h.kind {
            Kind::Anchor => {
                painter.circle(at, 4.5, fill, Stroke::new(1.5, color));
            }
            Kind::Angle => {
                let a = xf.to_screen(h.anchor[0] as f32, h.anchor[1] as f32);
                painter.line_segment([a, at], Stroke::new(1.5, color));
                let r = 5.0;
                let pts = vec![at + vec2(0.0, -r), at + vec2(r, 0.0), at + vec2(0.0, r), at + vec2(-r, 0.0)];
                painter.add(Shape::convex_polygon(pts, fill, Stroke::new(1.5, color)));
            }
            Kind::Point => super::handle(painter, at, color),
        }
    }
}

#[cfg(test)]
#[path = "handles_tests.rs"]
mod tests;
