//! Vector tools: Shape tools (U), Pen (P), Path Selection (A), path overlays and the Paths panel.
//! Direct Selection (A) lives in `direct_select`.
//! All edits go through the engine's `shape.*` / `path.*` commands.

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use prism_doc::vector::Path;
use prism_doc::{Document, LayerContent};
use prism_geom::Affine;
use prism_vector::edit::Hit;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PrismApp;
use crate::canvas::ViewXform;
use crate::state::{Tool, ToolOptions};
use crate::theme::Tokens;

/// Pen tool path under construction: knots as [anchor, in, out] (document px).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PenPath {
    pub knots: Vec<[[f64; 2]; 3]>,
    /// Indices of cusp knots whose incoming and outgoing handles are not linked.
    #[serde(default)]
    pub unlinked: Vec<usize>,
    #[serde(skip)]
    pub dragging: bool,
    /// Re-dragging the final anchor changes its outgoing control only; the incoming curve stays put.
    #[serde(skip)]
    pub adjusting_last: bool,
}

pub fn is_shape_tool(t: Tool) -> bool {
    matches!(t, Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Line | Tool::Arrow | Tool::CustomShape)
}

fn rgb32(c: [f32; 4]) -> Color32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(b(c[0]), b(c[1]), b(c[2]))
}

fn hex(c: [f32; 4]) -> String {
    let c = rgb32(c);
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn stroke_param(app: &PrismApp) -> Value {
    let o = &app.ui.tool_options;
    if o.stroke_width > 0.0 { json!({"width": o.stroke_width, "color": hex(app.session.tools.background)}) } else { Value::Null }
}

/// The rectangle corner styles the options bar and Properties offer.
const CORNER_STYLES: [(prism_doc::CornerStyle, &str); 2] =
    [(prism_doc::CornerStyle::Round, "Rounded corners"), (prism_doc::CornerStyle::Chamfer, "Chamfered corners")];

/// Geometry of a Shape-tool drag as `shape.create` params, without fill and stroke (None when too
/// small): ⇧ constrains proportions, ⌥ draws from the centre. Custom shapes give their box as a `rect`.
fn shape_geometry(o: &ToolOptions, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) -> Option<Value> {
    Some(if matches!(tool, Tool::Line | Tool::Arrow) {
        let (mut dx, mut dy) = (end[0] - start[0], end[1] - start[1]);
        if mods.shift {
            // Snap to 45°.
            let a = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
            let len = dx.hypot(dy);
            (dx, dy) = (len * a.cos(), len * a.sin());
        }
        if dx.hypot(dy) < 1.0 {
            return None;
        }
        if tool == Tool::Arrow {
            json!({"kind": "arrow", "from": start, "to": [start[0] + dx, start[1] + dy], "headSize": o.arrow.head_size, "headAngle": o.arrow.head_angle})
        } else {
            json!({"kind": "line", "from": start, "to": [start[0] + dx, start[1] + dy], "weight": o.line_weight.max(1.0), "roundTips":o.line_round_tips})
        }
    } else {
        let (mut w, mut h) = (end[0] - start[0], end[1] - start[1]);
        if mods.shift {
            let m = w.abs().max(h.abs());
            (w, h) = (m * w.signum(), m * h.signum());
        }
        let (x0, y0) = if mods.alt { (start[0] - w, start[1] - h) } else { (start[0], start[1]) };
        let (w, h) = if mods.alt { (w * 2.0, h * 2.0) } else { (w, h) };
        let rect = [x0.min(x0 + w).round(), y0.min(y0 + h).round(), w.abs().round(), h.abs().round()];
        if rect[2] < 1.0 || rect[3] < 1.0 {
            return None;
        }
        match tool {
            Tool::Rectangle if o.corner_radius > 0.0 => {
                json!({"kind": "roundedRect", "rect": rect, "radii": vec![o.corner_radius; 4], "corners": o.corner_style.as_str()})
            }
            // Sharp for now, but the corner widgets dragged later cut in the chosen style.
            Tool::Rectangle => json!({"kind": "rect", "rect": rect, "corners": o.corner_style.as_str()}),
            Tool::CustomShape => json!({"kind": "rect", "rect": rect}),
            Tool::EllipseShape => json!({"kind": "ellipse", "rect": rect}),
            Tool::Triangle => json!({"kind": "polygon", "rect": rect, "sides": 3}),
            _ => json!({"kind": "polygon", "rect": rect, "sides": o.polygon_sides.max(3)}),
        }
    })
}

/// Finish a Shape-tool drag: ⇧ constrains proportions, ⌥ draws from the centre.
pub fn finish_shape(app: &mut PrismApp, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let Some(mut p) = shape_geometry(&app.ui.tool_options, tool, start, end, mods) else { return };
    let fill = if app.ui.tool_options.shape_fill { json!(hex(app.session.tools.foreground)) } else { Value::Null };
    let stroke = stroke_param(app);
    if tool == Tool::CustomShape {
        // ⇧ keeps the shape's proportions (the rect is already squared).
        if let Ok(rect) = serde_json::from_value(p["rect"].take()) {
            crate::preset_panels::finish_custom_shape(app, rect, mods.shift, fill, stroke);
        }
        return;
    }
    p["fill"] = if tool == Tool::Arrow { Value::Null } else { fill };
    p["stroke"] = if tool == Tool::Arrow { arrow_stroke(app) } else { stroke };
    if let Err(e) = app.run("shape.create", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Shape-tool drag preview: the shape `finish_shape` will create, filled and stroked, under its
/// path outline. Custom shapes show their box outline only.
pub fn draw_shape_preview(app: &PrismApp, painter: &egui::Painter, xf: &ViewXform, tool: Tool, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let o = &app.ui.tool_options;
    let Some(path) = shape_geometry(o, tool, start, end, mods).and_then(|p| prism_engine::vector_cmds::shape_path(&p).ok()) else { return };
    if tool == Tool::Arrow {
        let (cap, join) = arrow_ends(o.arrow.rounded);
        let style = prism_vector::StrokeStyle { width: f64::from(o.arrow.width), cap, join, ..Default::default() };
        let polys = prism_vector::stroke_polygons(&prism_vector::flatten_path(&path, 0.01), &style, 0.05 / f64::from(xf.zoom.max(0.01)));
        for poly in polys {
            let pts = poly.into_iter().map(|(x, y)| xf.to_screen(x as f32, y as f32)).collect();
            painter.add(egui::Shape::convex_polygon(pts, rgb32(app.session.tools.foreground), Stroke::NONE));
        }
        return;
    }
    let custom = tool == Tool::CustomShape;
    // ponytail: preview colours skip the canvas's colour management; the commit renders them exactly.
    let fill = if o.shape_fill && !custom { rgb32(app.session.tools.foreground) } else { Color32::TRANSPARENT };
    let stroke = if o.stroke_width > 0.0 && !custom { Stroke::new(o.stroke_width * xf.zoom, rgb32(app.session.tools.background)) } else { Stroke::NONE };
    let accent = Tokens::get(painter.ctx()).accent;
    for (pts, _) in path_lines(&path, &|q| xf.to_screen(q[0] as f32, q[1] as f32)) {
        // Every tool's own shape is convex (custom shapes aren't, and draw no fill).
        painter.add(egui::Shape::convex_polygon(pts.clone(), fill, stroke));
        // The accent path over a dark halo: visible on any pixels (#172).
        painter.extend(crate::tool_feedback::contrast_path(pts, true, accent));
    }
}

// ---------------------------------------------------------------------------------------------
// Pen

fn pen_to_json(pen: &PenPath, closed: bool) -> Value {
    json!({"subpaths": [{"closed": closed, "knots": pen.knots.iter().enumerate().map(|(i, k)| json!({"anchor": k[0], "in": k[1], "out": k[2], "smooth": !pen.unlinked.contains(&i) && (k[1] != k[0] || k[2] != k[0])})).collect::<Vec<_>>()}]})
}

/// Pen press: close on the first anchor, reshape the last anchor's outgoing handle,
/// or add a new anchor (dragging a new anchor pulls symmetrical handles).
/// Cursor intent mirrors the same first/last-anchor hit tests used by `pen_down`.
pub(crate) fn pen_cursor_asset(app: &PrismApp, p: [f64; 2]) -> u32 {
    let Some(pen) = app.ui.pen.as_ref().filter(|pen| !pen.knots.is_empty()) else { return 1063 };
    let tol = pen_hit_tol(app);
    if pen_closes_at(pen, p, tol) {
        1034
    } else if pen.knots.last().is_some_and(|k| (k[0][0] - p[0]).hypot(k[0][1] - p[1]) < tol) {
        1066
    } else {
        1030
    }
}

/// Anchor hit radius in document units: 6 screen pixels at the current zoom.
fn pen_hit_tol(app: &PrismApp) -> f64 {
    6.0 / f64::from(app.current_zoom().max(0.01))
}

/// Whether a press at `p` closes the path: it has two or more anchors and `p` is on the first.
fn pen_closes_at(pen: &PenPath, p: [f64; 2], tol: f64) -> bool {
    pen.knots.len() >= 2 && pen.knots.first().is_some_and(|k| (k[0][0] - p[0]).hypot(k[0][1] - p[1]) < tol)
}

/// What Auto Add/Delete does for a press at `p` before a new path is started: on a shown path's
/// anchor it deletes that anchor, on a segment it adds one there. The path (as Direct Selection
/// targets it) and the part hit; `None` when it's off or the press starts or continues a path.
fn auto_edit_at(app: &PrismApp, p: [f64; 2]) -> Option<(crate::direct_select::PathRef, Hit)> {
    if app.ui.tool != Tool::Pen || !app.ui.tool_options.pen_auto_add_delete || app.ui.pen.as_ref().is_some_and(|pen| !pen.knots.is_empty()) {
        return None;
    }
    let tol = pen_hit_tol(app);
    crate::direct_select::candidates(app).into_iter().find_map(|(r, path)| {
        match prism_vector::edit::hit(&path, prism_geom::Point::new(p[0], p[1]), tol, &[])? {
            h @ (Hit::Anchor(_) | Hit::Segment(..)) => Some((r, h)),
            Hit::Handle(..) => None,
        }
    })
}

/// Auto Add/Delete for a press at `p`; whether it handled the press.
fn auto_add_delete(app: &mut PrismApp, p: [f64; 2]) -> bool {
    let Some((target, hit)) = auto_edit_at(app, p) else { return false };
    let mut q = target.params();
    let r = match hit {
        Hit::Anchor([s, k]) => {
            q["subpath"] = json!(s);
            q["knot"] = json!(k);
            app.run("path.deleteAnchor", q)
        }
        Hit::Segment([s, k], t) => {
            q["subpath"] = json!(s);
            q["knot"] = json!(k);
            q["t"] = json!(t.clamp(0.0, 1.0));
            app.run("path.addAnchor", q)
        }
        Hit::Handle(..) => return false,
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

/// Align Edges (Shape mode): new anchors land on whole pixels, so straight edges stay crisp.
fn pen_snap(app: &PrismApp, p: [f64; 2]) -> [f64; 2] {
    let o = &app.ui.tool_options;
    if o.vector_mode == "shape" && o.pen_align_edges { [p[0].round(), p[1].round()] } else { p }
}

pub fn pen_down(app: &mut PrismApp, x: f64, y: f64) {
    if auto_add_delete(app, [x, y]) {
        return;
    }
    let [x, y] = pen_snap(app, [x, y]);
    let tol = pen_hit_tol(app);
    let pen = app.ui.pen.get_or_insert_with(PenPath::default);
    if pen_closes_at(pen, [x, y], tol) {
        pen_commit(app, true);
        return;
    }
    // Clicking the last anchor breaks its outgoing handle without throwing away the incoming
    // curve. Dragging from that anchor then sets the outgoing handle independently (#1482).
    // This must precede adding a point, or a re-click creates a zero-length segment.
    if let Some((i, last)) = pen.knots.len().checked_sub(1).zip(pen.knots.last_mut())
        && (last[0][0] - x).hypot(last[0][1] - y) < tol
    {
        last[2] = last[0];
        if !pen.unlinked.contains(&i) {
            pen.unlinked.push(i);
        }
        pen.dragging = true;
        pen.adjusting_last = true;
        return;
    }
    pen.knots.push([[x, y]; 3]);
    pen.dragging = true;
    pen.adjusting_last = false;
}

pub fn pen_move(app: &mut PrismApp, x: f64, y: f64) {
    if let Some(pen) = app.ui.pen.as_mut()
        && pen.dragging
    {
        let adjusting_last = pen.adjusting_last;
        if let Some(k) = pen.knots.last_mut() {
            let a = k[0];
            k[2] = [x, y];
            if !adjusting_last {
                k[1] = [2.0 * a[0] - x, 2.0 * a[1] - y];
            }
        }
    }
}

pub fn pen_up(app: &mut PrismApp) {
    if let Some(pen) = app.ui.pen.as_mut() {
        pen.dragging = false;
        pen.adjusting_last = false;
    }
}

/// A fingerprint of `path`'s geometry, to tell whether a hidden work path has changed since.
fn path_fingerprint(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path.inverted, format!("{:?}", path.fill_rule)).hash(&mut h);
    for sp in &path.subpaths {
        (sp.closed, format!("{:?}", sp.op), sp.knots.len()).hash(&mut h);
        for k in &sp.knots {
            for p in [k.anchor, k.in_ctrl, k.out_ctrl] {
                (p.x.to_bits(), p.y.to_bits()).hash(&mut h);
            }
        }
    }
    h.finish()
}

/// Whether `doc`'s work path is the one hidden with ↩ (unchanged since). A hidden work path stays
/// in the Paths panel but isn't drawn, and the path tools don't edit it, as in Photoshop.
pub(crate) fn work_path_hidden(app: &PrismApp, doc: &Document) -> bool {
    match (app.ui.hidden_work_path, &doc.work_path) {
        (Some((id, print)), Some(wp)) => id == doc.id.0 && print == path_fingerprint(wp),
        _ => false,
    }
}

/// The active document's work path, unless hidden.
pub(crate) fn shown_work_path(app: &PrismApp) -> Option<Path> {
    let st = app.session.active()?;
    (!work_path_hidden(app, &st.doc)).then(|| st.doc.work_path.clone()).flatten()
}

/// Hides the active document's work path from the canvas and deselects it in the Paths panel
/// (the Pen's ↩, as in Photoshop). Whether there was one to hide.
pub(crate) fn hide_work_path(app: &mut PrismApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let Some(wp) = st.doc.work_path.as_ref().filter(|_| !work_path_hidden(app, &st.doc)) else { return false };
    app.ui.hidden_work_path = Some((st.doc.id.0, path_fingerprint(wp)));
    if app.ui.selected_path.as_deref() == Some("work") {
        app.ui.selected_path = None;
    }
    true
}

/// ↩ with the Pen: ends a path being drawn and, in Path mode, deselects it, so it leaves the
/// canvas (it stays the work path in the Paths panel), as in Photoshop; with no path being
/// drawn it hides the work path. Whether it did anything.
pub(crate) fn pen_enter(app: &mut PrismApp) -> bool {
    let drawing = app.ui.pen.is_some();
    // Shape mode and a targeted vector mask finish into a layer: nothing to hide.
    let path_mode = app.ui.tool_options.vector_mode != "shape" && targeted_vector_mask(app).is_none();
    if drawing {
        pen_commit(app, false);
    }
    let hid = path_mode && hide_work_path(app);
    drawing || hid
}

/// Does a finished Pen subpath join the work path rather than replace it? While the work path is
/// selected in the Paths panel.
fn appends_to_work_path(app: &PrismApp) -> bool {
    app.ui.selected_path.as_deref() == Some("work") && app.session.active().is_some_and(|st| st.doc.work_path.is_some())
}

/// Finish the pen path: a work path (Path mode) or a new shape layer (Shape mode).
pub fn pen_commit(app: &mut PrismApp, closed: bool) {
    let Some(pen) = app.ui.pen.take() else { return };
    if pen.knots.len() < 2 {
        return;
    }
    let mut path = pen_to_json(&pen, closed);
    let o = &app.ui.tool_options;
    let op = o.pen_path_op.clone();
    let r = if o.vector_mode == "shape" {
        let fill = if closed && o.shape_fill { json!(hex(app.session.tools.foreground)) } else { Value::Null };
        let stroke = if closed { stroke_param(app) } else { json!({"width": o.stroke_width.max(1.0), "color": hex(app.session.tools.foreground)}) };
        let mut q = json!({"kind": "path", "path": path, "fill": fill, "stroke": stroke});
        // Path Operations other than New Layer add to the selected shape layer.
        if o.pen_shape_op != "new"
            && let Some((id, _)) = active_shape_path(app)
        {
            q["addTo"] = json!(id);
            q["op"] = json!(o.pen_shape_op);
        }
        app.run("shape.create", q)
    } else if let Some((id, existing)) = targeted_vector_mask(app) {
        // A targeted vector mask takes the new subpath (#196), as in Photoshop.
        let mut p = prism_engine::vector_cmds::path_json(&existing);
        if let (Some(subs), Some(new)) = (p.get_mut("subpaths").and_then(Value::as_array_mut), path.get_mut("subpaths").and_then(Value::as_array_mut)) {
            for sp in new.iter_mut() {
                sp["op"] = json!(op);
            }
            subs.extend(new.iter().cloned());
        }
        app.run("layer.vectorMask.edit", json!({"layer": id, "path": p}))
    } else {
        // While the work path is selected in the Paths panel, each new subpath joins it with the
        // Path Operation; otherwise it starts a new work path (Photoshop: deselect to start over).
        let mut q = json!({"name": "work", "path": path});
        if appends_to_work_path(app) {
            q["op"] = json!(op);
        }
        // The drawn work path is selected in the Paths panel, as in Photoshop.
        app.run("path.set", q).inspect(|_| {
            app.ui.selected_path = Some("work".into());
            app.ui.hidden_work_path = None;
        })
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The path that path commands act on without a name: the one selected in the Paths panel, else
/// the work path, else the selected layer's shape path or vector mask; `None` when there's none.
pub fn active_path_name(app: &PrismApp) -> Option<String> {
    let st = app.session.active()?;
    let rows = path_rows(&st.doc, st.active_layer);
    let key = |r: &PathEntry| match r.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => r.name.clone(),
    };
    let selected = app.ui.selected_path.as_deref().filter(|s| rows.iter().any(|r| key(r) == *s)).map(str::to_string);
    selected.or_else(|| rows.iter().find(|r| r.kind == PathRow::Work).map(key)).or_else(|| rows.iter().find(|r| r.kind == PathRow::Layer).map(key))
}

/// The commands that create a fill or adjustment layer and take a `"path"` as its vector mask.
pub fn takes_path_mask(id: &str) -> bool {
    id.starts_with("layer.newFillLayer.") || id.starts_with("layer.newAdjustmentLayer.")
}

/// New fill and adjustment layers take the path selected in the Paths panel as their vector mask,
/// as in Photoshop (#1419): the command gets it as `"path"` unless the caller set one (`null`
/// opts out). Only an explicitly selected work or saved path counts; deselect it by clicking the
/// panel's empty area.
pub fn with_active_path(app: &PrismApp, id: &str, params: Value) -> Value {
    if !takes_path_mask(id) || params.get("path").is_some() {
        return params;
    }
    let Some(st) = app.session.active() else { return params };
    let Some(sel) = app.ui.selected_path.as_deref() else { return params };
    let listed = path_rows(&st.doc, st.active_layer).iter().any(|r| match r.kind {
        PathRow::Work => sel == "work",
        PathRow::Saved => r.name == sel,
        PathRow::Layer => false,
    });
    if !listed {
        return params;
    }
    match params {
        Value::Object(mut m) => {
            m.insert("path".into(), json!(sel));
            Value::Object(m)
        }
        _ => json!({ "path": sel }),
    }
}

/// ⌘↩ / Ctrl+Enter (#306): load a path as a selection, like Photoshop with a Pen or Path
/// Selection tool or a path selected in the Paths panel. A path being drawn with the Pen is
/// finished first and loads; otherwise [`active_path_name`]. `params` may set feather, mode…
pub fn path_to_selection(app: &mut PrismApp, params: Value) -> Result<Value, String> {
    if app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2) {
        // The finished path becomes the work path, or a shape layer's path or a vector mask.
        let on_layer = app.ui.tool_options.vector_mode == "shape" || targeted_vector_mask(app).is_some();
        pen_commit(app, false);
        app.ui.selected_path = Some(if on_layer { "layer" } else { "work" }.into());
    }
    let name = active_path_name(app)
        .ok_or_else(|| tl!("No path to make a selection from: draw one with the Pen tool or select one in the Paths panel.").to_string())?;
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.insert("name".into(), json!(name));
    app.run("path.toSelection", Value::Object(p))
}

// ---------------------------------------------------------------------------------------------
// Path Selection

/// What Path Selection edits.
enum PathTarget {
    Shape(u64),
    /// The active layer's vector mask, when its Layers thumbnail is targeted (#196).
    VectorMask(u64),
    Work,
}

/// The targeted vector mask of the active layer, if the Layers panel targets it.
pub(crate) fn targeted_vector_mask(app: &PrismApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    (app.ui.vector_mask_target && !matches!(l.content, LayerContent::Shape(_))).then_some(())?;
    l.vector_mask.as_ref().map(|m| (l.id.0, m.path.clone()))
}

/// The active shape layer's path, if the active layer is a shape layer.
pub(crate) fn active_shape_path(app: &PrismApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    match &l.content {
        LayerContent::Shape(sh) => Some((l.id.0, sh.path.clone())),
        _ => None,
    }
}

/// The path Path Selection edits: the targeted vector mask, the active shape layer's path, else
/// the work path.
fn target_path(app: &PrismApp) -> Option<(PathTarget, Path)> {
    if let Some((id, p)) = targeted_vector_mask(app) {
        return Some((PathTarget::VectorMask(id), p));
    }
    if let Some((id, p)) = active_shape_path(app) {
        return Some((PathTarget::Shape(id), p));
    }
    shown_work_path(app).map(|p| (PathTarget::Work, p))
}

pub fn path_selection_finish(app: &mut PrismApp, start: [f64; 2], end: [f64; 2]) {
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    if dx.abs() + dy.abs() < 0.5 {
        return;
    }
    let Some((target, path)) = target_path(app) else { return };
    let _ = match target {
        PathTarget::Shape(id) => app.run("shape.edit", json!({"layer": id, "move": [dx.round(), dy.round()]})),
        PathTarget::VectorMask(id) => {
            let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
            app.run("layer.vectorMask.edit", json!({"layer": id, "path": prism_engine::vector_cmds::path_json(&moved)}))
        }
        PathTarget::Work => {
            let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
            app.run("path.set", json!({"name": "work", "path": prism_engine::vector_cmds::path_json(&moved)}))
        }
    };
}

// ---------------------------------------------------------------------------------------------
// Drawing

fn bezier(p0: [f64; 2], c0: [f64; 2], c1: [f64; 2], p1: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let f = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
    [f(p0[0], c0[0], c1[0], p1[0]), f(p0[1], c0[1], c1[1], p1[1])]
}

/// Screen polylines for a path (one per subpath).
pub(crate) fn path_lines(path: &Path, xf: &dyn Fn([f64; 2]) -> Pos2) -> Vec<(Vec<Pos2>, bool)> {
    path.subpaths
        .iter()
        .map(|s| {
            let n = s.knots.len();
            let mut pts = Vec::new();
            let segs = if s.closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                let (a, b) = (&s.knots[i], &s.knots[(i + 1) % n]);
                for k in 0..=16 {
                    let q =
                        bezier([a.anchor.x, a.anchor.y], [a.out_ctrl.x, a.out_ctrl.y], [b.in_ctrl.x, b.in_ctrl.y], [b.anchor.x, b.anchor.y], k as f64 / 16.0);
                    pts.push(xf(q));
                }
            }
            if pts.is_empty()
                && let Some(k) = s.knots.first()
            {
                pts.push(xf([k.anchor.x, k.anchor.y]));
            }
            (pts, s.closed)
        })
        .collect()
}

/// A path's outline in the accent colour.
pub(crate) fn draw_outline(painter: &egui::Painter, p: &Path, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32) {
    for (pts, closed) in path_lines(p, to_scr) {
        if closed {
            painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0, accent)));
        } else {
            painter.add(egui::Shape::line(pts, Stroke::new(1.0, accent)));
        }
    }
}

/// An anchor point's square: filled with the accent when selected, else hollow (white).
pub(crate) fn draw_anchor(painter: &egui::Painter, c: Pos2, selected: bool, accent: Color32) {
    let r = Rect::from_center_size(c, vec2(6.0, 6.0));
    painter.rect_filled(r, 0.0, if selected { accent } else { Color32::WHITE });
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
}

/// A direction handle: the line from its anchor and the round handle end.
pub(crate) fn draw_handle(painter: &egui::Painter, anchor: Pos2, handle: Pos2, accent: Color32) {
    painter.line_segment([anchor, handle], Stroke::new(1.0, accent));
    painter.circle_filled(handle, 3.0, accent);
}

/// Work path / active shape path outlines, anchors, and the pen path in progress.
pub fn draw_overlay(app: &PrismApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let tool = app.ui.tool;
    let vector_tool =
        matches!(tool, Tool::Pen | Tool::PathSelection) || is_shape_tool(tool) || app.ui.shape_corners.is_some() || app.ui.arrow_handles.is_some();
    let accent = Tokens::get(painter.ctx()).accent;
    let to_scr = |q: [f64; 2]| xf.to_screen(q[0] as f32, q[1] as f32);
    let draw_path = |p: &Path, anchors: bool| {
        draw_outline(painter, p, &to_scr, accent);
        if anchors {
            for k in p.subpaths.iter().flat_map(|s| &s.knots) {
                draw_anchor(painter, to_scr([k.anchor.x, k.anchor.y]), false, accent);
            }
        }
    };
    // View › Show › Target Path (under Extras) hides the paths; the Pen's path in progress stays.
    let paths = app.ui.view.shows(app.ui.view.show.target_path);
    if app.ui.shape_corners.is_none() && app.ui.arrow_handles.is_none() && crate::direct_select::shows(app, painter.ctx().input(|i| i.modifiers)) {
        // Direct Selection (or the Pen with ⌘/Ctrl held) draws the paths it edits (#790).
        crate::direct_select::draw_overlay(app, painter, &to_scr, accent, paths);
    } else if paths {
        if vector_tool {
            if let Some(wp) = doc.work_path.as_ref().filter(|_| !work_path_hidden(app, doc)) {
                draw_path(wp, tool == Tool::PathSelection);
            }
            let st = app.session.active();
            if let Some(l) = st.and_then(|s| s.active_layer.and_then(|id| doc.layer(id)))
                && let LayerContent::Shape(sh) = &l.content
            {
                let preview = crate::arrow_handles_ui::outline(app, l.id).or_else(|| crate::shape_corners_ui::outline(app, l.id));
                draw_path(preview.as_ref().unwrap_or(&sh.path), tool == Tool::PathSelection);
            }
        }
        // A targeted vector mask shows its path with any tool (#196).
        if let Some((_, p)) = targeted_vector_mask(app) {
            draw_path(&p, tool == Tool::PathSelection);
        }
    }
    // Pen path in progress, with handles of the last knot and a rubber band to the pointer.
    if let Some(pen) = &app.ui.pen {
        let mut pts = Vec::new();
        for w in pen.knots.windows(2) {
            for k in 0..=16 {
                pts.push(to_scr(bezier(w[0][0], w[0][2], w[1][1], w[1][0], k as f64 / 16.0)));
            }
        }
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, accent)));
        // Hovering the first anchor previews the closing segment and highlights that anchor,
        // so it's clear a click will close the path.
        let closing = !pen.dragging && app.hover_doc.is_some_and(|h| pen_closes_at(pen, h, pen_hit_tol(app)));
        if let (Some(first), Some(last)) = (pen.knots.first(), pen.knots.last())
            && closing
        {
            let seg: Vec<Pos2> = (0..=16).map(|k| to_scr(bezier(last[0], last[2], first[1], first[0], k as f64 / 16.0))).collect();
            painter.add(egui::Shape::line(seg, Stroke::new(1.5, accent)));
        } else if let (Some(last), Some(h)) = (pen.knots.last(), app.hover_doc)
            && !pen.dragging
            && app.ui.tool_options.pen_rubber_band
        {
            // Rubber Band: the segment a click here would add, curved by the last handle.
            let seg: Vec<Pos2> = (0..=16).map(|k| to_scr(bezier(last[0], last[2], h, h, k as f64 / 16.0))).collect();
            painter.add(egui::Shape::line(seg, Stroke::new(1.0, accent)));
        }
        if let Some(first) = pen.knots.first()
            && closing
        {
            let c = to_scr(first[0]);
            painter.circle_stroke(c, 8.0, Stroke::new(1.5, accent));
            painter.rect_filled(Rect::from_center_size(c, vec2(8.0, 8.0)), 0.0, accent);
        }
        for (i, k) in pen.knots.iter().enumerate() {
            if i == 0 && closing {
                continue;
            }
            let c = to_scr(k[0]);
            let last = i + 1 == pen.knots.len();
            if last && k[2] != k[0] {
                for hnd in [k[1], k[2]] {
                    draw_handle(painter, c, to_scr(hnd), accent);
                }
            }
            draw_anchor(painter, c, last, accent);
        }
    }
    // Auto Add/Delete: a + or − beside the pointer over a path segment or anchor.
    if let Some(h) = app.hover_doc
        && !crate::direct_select::shows(app, painter.ctx().input(|i| i.modifiers))
        && let Some((_, hit)) = auto_edit_at(app, h)
    {
        let badge = if matches!(hit, Hit::Anchor(_)) { crate::tool_feedback::Badge::Subtract } else { crate::tool_feedback::Badge::Add };
        crate::tool_feedback::draw_badge(painter, to_scr(h), badge, false);
    }
}

// ---------------------------------------------------------------------------------------------
// Options bars

/// Options bar for vector tools; false for other tools.
pub fn options_bar(app: &mut PrismApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if !(is_shape_tool(tool) || matches!(tool, Tool::Pen | Tool::PathSelection | Tool::DirectSelection)) {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
    };
    if tool == Tool::Arrow {
        arrow_options(app, ui);
        return true;
    }
    if tool == Tool::Pen {
        pen_options(app, ui);
        return true;
    }
    if tool == Tool::PathSelection {
        lbl(ui, if app.ui.vector_mask_target { "Drag to move the targeted vector mask" } else { tl!("Drag to move the active shape's path or the Work Path") });
        return true;
    }
    if tool == Tool::DirectSelection {
        let (shift, alt) = (crate::shortcuts::pretty("Shift"), crate::shortcuts::pretty("Alt"));
        lbl(
            ui,
            &crate::i18n::fmt(
                tl!("Click: select anchor · Drag: move anchor, handle or segment · {shift}-click: add · {alt}-click: whole subpath"),
                &[("shift", &shift), ("alt", &alt)],
            ),
        );
        return true;
    }
    let mut mode = "shape".to_string();
    crate::widgets::dropdown(ui, "shape-mode", &mut mode, &[("shape".to_string(), tl!("Shape"))], 80.0);
    crate::widgets::vline(ui, 22.0);
    fill_stroke_fields(app, ui);
    let o = &mut app.ui.tool_options;
    match tool {
        Tool::Rectangle => {
            crate::widgets::vline(ui, 22.0);
            lbl(ui, tl!("Radius:"));
            crate::widgets::value_field(ui, &mut o.corner_radius, 0.0..=10000.0, "px", 62.0);
            crate::widgets::dropdown(ui, "shape-corner-style", &mut o.corner_style, &CORNER_STYLES, 104.0);
        }
        Tool::Polygon => {
            crate::widgets::vline(ui, 22.0);
            lbl(ui, tl!("Sides:"));
            let mut s = o.polygon_sides as f32;
            if crate::widgets::value_field(ui, &mut s, 3.0..=100.0, "", 48.0).changed() {
                o.polygon_sides = s.round().clamp(3.0, 100.0) as u32;
            }
        }
        Tool::Line => {
            crate::widgets::vline(ui, 22.0);
            lbl(ui, tl!("Weight:"));
            crate::widgets::value_field(ui, &mut o.line_weight, 1.0..=1000.0, "px", 58.0);
            lbl(ui, tl!("Line tips"));
            line_tip_picker(ui, "line-tips", &mut o.line_round_tips);
        }
        Tool::CustomShape => {
            crate::widgets::vline(ui, 22.0);
            crate::preset_panels::shape_picker(app, ui);
        }
        _ => {}
    }
    true
}

fn dim_label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
}

/// Shape options: Fill with the foreground colour, and the stroke width.
fn fill_stroke_fields(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let fg = app.session.tools.foreground;
    let o = &mut app.ui.tool_options;
    dim_label(ui, tl!("Fill:"));
    crate::widgets::checkbox(ui, &mut o.shape_fill, "");
    let (r, _) = ui.allocate_exact_size(vec2(22.0, 16.0), Sense::hover());
    ui.painter().rect_filled(r, 2.0, rgb32(fg));
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    dim_label(ui, tl!("Stroke:"));
    crate::widgets::value_field(ui, &mut o.stroke_width, 0.0..=288.0, "px", 58.0);
}

/// An options-bar icon button that opens a menu (a small chevron in its corner).
fn menu_icon(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    let r = crate::icons::button(ui, icon, 26.0, false, tip);
    let t = Tokens::get(ui.ctx());
    crate::icons::paint(ui, Rect::from_min_size(r.rect.right_bottom() - vec2(9.0, 9.0), vec2(8.0, 8.0)), "chevron-down", 7.0, t.text_dim);
    r
}

/// The path the Pen bar's operations, alignment and arrangement act on, and its picked subpaths
/// (those with anchors selected by Direct Selection; `None` when there are none). Path mode acts
/// on the work path, Shape mode on the shape layer (or a targeted vector mask).
fn pen_bar_target(app: &PrismApp) -> Option<(crate::direct_select::PathRef, Path, Option<Vec<usize>>)> {
    use crate::direct_select::PathRef;
    let candidates = crate::direct_select::candidates(app);
    let want_work = app.ui.tool_options.vector_mode != "shape" && !app.ui.vector_mask_target;
    let (r, path) = candidates.iter().find(|(r, _)| !want_work || *r == PathRef::Work).or(candidates.first()).cloned()?;
    let ds = &app.ui.direct_selection;
    let picked = (ds.target == Some(r) && !ds.anchors.is_empty()).then(|| {
        let mut v: Vec<usize> = ds.anchors.iter().map(|a| a[0]).collect();
        v.sort_unstable();
        v.dedup();
        v
    });
    Some((r, path, picked))
}

/// Runs a component command on the Pen bar's path with `extra` params, reporting a failure.
fn run_on_bar_path(app: &mut PrismApp, id: &str, target: crate::direct_select::PathRef, extra: Value) -> Option<Value> {
    let mut q = target.params();
    if let (Some(q), Some(extra)) = (q.as_object_mut(), extra.as_object()) {
        q.extend(extra.clone());
    }
    match app.run(id, q) {
        Ok(v) => Some(v),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            None
        }
    }
}

/// Pen options bar, as in Photoshop: mode, Make (Path mode) or Fill/Stroke (Shape mode), Path
/// Operations, Path Alignment, Path Arrangement, the gear menu (Rubber Band), Auto Add/Delete
/// and Align Edges.
fn pen_options(app: &mut PrismApp, ui: &mut egui::Ui) {
    let opts = [("path".to_string(), tl!("Path")), ("shape".to_string(), tl!("Shape"))];
    crate::widgets::dropdown(ui, "pen-mode", &mut app.ui.tool_options.vector_mode, &opts, 80.0);
    crate::widgets::vline(ui, 22.0);
    let shape_mode = app.ui.tool_options.vector_mode == "shape";
    if shape_mode {
        fill_stroke_fields(app, ui);
    } else {
        dim_label(ui, tl!("Make:"));
        for (label, cmd) in [(tl!("Selection…"), "path.toSelection"), (tl!("Mask"), "layer.vectorMask.fromPath"), (tl!("Shape"), "path.makeShape")] {
            let on = crate::canvas_tool_menu::pen_bar_enabled(app, cmd);
            if ui.add_enabled_ui(on, |ui| crate::widgets::secondary_button(ui, label, 0.0)).inner.clicked() {
                crate::canvas_tool_menu::pen_bar_run(app, ui.ctx(), cmd);
            }
        }
    }
    crate::widgets::vline(ui, 22.0);
    pen_operations_menu(app, ui, shape_mode);
    pen_alignment_menu(app, ui);
    pen_arrangement_menu(app, ui);
    crate::widgets::vline(ui, 22.0);
    let gear = menu_icon(ui, "settings", tl!("Settings"));
    egui::Popup::menu(&gear).show(|ui| {
        ui.set_min_width(160.0);
        crate::widgets::checkbox(ui, &mut app.ui.tool_options.pen_rubber_band, tl!("Rubber Band"));
    });
    crate::widgets::checkbox(ui, &mut app.ui.tool_options.pen_auto_add_delete, tl!("Auto Add/Delete"));
    // Align Edges snaps shape edges to pixels; a path has no pixels, so it's off in Path mode.
    ui.add_enabled_ui(shape_mode, |ui| crate::widgets::checkbox(ui, &mut app.ui.tool_options.pen_align_edges, tl!("Align Edges")));
}

/// Path Operations: how the next subpath combines (and the picked or newest component's
/// operation, as Photoshop changes the component just drawn), plus Merge Shape Components.
fn pen_operations_menu(app: &mut PrismApp, ui: &mut egui::Ui, shape_mode: bool) {
    let resp = menu_icon(ui, "squares-subtract", tl!("Path Operations"));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(220.0);
        let current = if shape_mode { app.ui.tool_options.pen_shape_op.clone() } else { app.ui.tool_options.pen_path_op.clone() };
        let target = pen_bar_target(app);
        let mut ops = vec![
            ("combine", tl!("Combine Shapes")),
            ("subtract", tl!("Subtract Front Shape")),
            ("intersect", tl!("Intersect Shape Areas")),
            ("exclude", tl!("Exclude Overlapping Shapes")),
        ];
        if shape_mode {
            ops.insert(0, ("new", tl!("New Layer")));
        }
        for (op, label) in ops {
            if ui.selectable_label(current == op, label).clicked() {
                if shape_mode {
                    app.ui.tool_options.pen_shape_op = op.into();
                } else {
                    app.ui.tool_options.pen_path_op = op.into();
                }
                if let Some((r, path, picked)) = target.clone()
                    && op != "new"
                    && path.subpaths.len() > 1
                {
                    let subs = picked.unwrap_or_else(|| vec![path.subpaths.len() - 1]);
                    run_on_bar_path(app, "path.setOperation", r, json!({"op": op, "subpaths": subs}));
                }
                ui.close();
            }
        }
        ui.separator();
        let merge = target.as_ref().is_some_and(|(_, p, _)| p.subpaths.len() > 1);
        if ui.add_enabled(merge, egui::Button::new(tl!("Merge Shape Components"))).clicked() {
            if let Some((r, _, _)) = target {
                run_on_bar_path(app, "path.mergeComponents", r, json!({}));
                app.ui.direct_selection.anchors.clear();
            }
            ui.close();
        }
    });
}

/// Path Alignment: align or distribute the picked components (all when none are picked), to
/// their own bounds or the canvas.
fn pen_alignment_menu(app: &mut PrismApp, ui: &mut egui::Ui) {
    let resp = menu_icon(ui, "align-left", tl!("Path Alignment"));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(200.0);
        let target = pen_bar_target(app);
        let count = target.as_ref().map_or(0, |(_, p, picked)| match picked {
            Some(subs) => prism_vector::edit::components_of(p, subs).map_or(0, |c| c.len()),
            None => p.components().len(),
        });
        let canvas = app.ui.tool_options.pen_align_canvas;
        let edges = [
            ("left", tl!("Left Edges")),
            ("hcenter", tl!("Horizontal Centers")),
            ("right", tl!("Right Edges")),
            ("top", tl!("Top Edges")),
            ("vcenter", tl!("Vertical Centers")),
            ("bottom", tl!("Bottom Edges")),
        ];
        for (section, id, min) in [(tl!("Align"), "path.alignComponents", if canvas { 1 } else { 2 }), (tl!("Distribute"), "path.distributeComponents", 3)] {
            ui.label(egui::RichText::new(section).small().color(Tokens::get(ui.ctx()).text_dim));
            for (edge, label) in edges {
                if ui.add_enabled(count >= min, egui::Button::new(label)).clicked() {
                    if let Some((r, _, picked)) = target.clone() {
                        let mut q = json!({"edge": edge, "to": if canvas { "canvas" } else { "selection" }});
                        if let Some(subs) = picked {
                            q["subpaths"] = json!(subs);
                        }
                        run_on_bar_path(app, id, r, q);
                    }
                    ui.close();
                }
            }
            ui.separator();
        }
        dim_label(ui, tl!("Align To:"));
        for (on, label) in [(false, tl!("Selection")), (true, tl!("Canvas"))] {
            if ui.selectable_label(canvas == on, label).clicked() {
                app.ui.tool_options.pen_align_canvas = on;
            }
        }
    });
}

/// Path Arrangement: restack the picked components (the newest when none are picked).
fn pen_arrangement_menu(app: &mut PrismApp, ui: &mut egui::Ui) {
    let resp = menu_icon(ui, "layers", tl!("Path Arrangement"));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(200.0);
        let target = pen_bar_target(app);
        let on = target.as_ref().is_some_and(|(_, p, _)| p.components().len() > 1);
        for (to, label) in
            [("front", tl!("Bring to Front")), ("forward", tl!("Bring Forward")), ("backward", tl!("Send Backward")), ("back", tl!("Send to Back"))]
        {
            if ui.add_enabled(on, egui::Button::new(label)).clicked() {
                if let Some((r, path, picked)) = target.clone() {
                    let subs = picked.unwrap_or_else(|| vec![path.subpaths.len().saturating_sub(1)]);
                    // The picked anchors' indices change; Direct Selection drops them.
                    if run_on_bar_path(app, "path.arrangeComponents", r, json!({"to": to, "subpaths": subs})).is_some() {
                        app.ui.direct_selection.anchors.clear();
                        app.ui.direct_selection.segment = None;
                    }
                }
                ui.close();
            }
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Properties (shape layers)

fn color_of(f: &prism_doc::Fill) -> Option<Color32> {
    match f {
        prism_doc::Fill::Solid(c) => {
            let v = c.to_rgba8();
            Some(Color32::from_rgb(v[0], v[1], v[2]))
        }
        _ => None,
    }
}

/// A colour swatch that opens a picker; returns the new `#rrggbb` when changed.
fn swatch(ui: &mut egui::Ui, fill: Option<&prism_doc::Fill>, tip: &str) -> Option<String> {
    swatch_with_none(ui, fill, tip, true)
}

fn swatch_with_none(ui: &mut egui::Ui, fill: Option<&prism_doc::Fill>, tip: &str, allow_none: bool) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 18.0), Sense::click());
    let current = fill.and_then(color_of);
    match (current, fill) {
        (Some(c), _) => {
            ui.painter().rect_filled(r, 2.0, c);
        }
        (None, Some(prism_doc::Fill::Gradient { stops, .. })) if !stops.is_empty() => {
            // Gradient fills preview as a left-to-right ramp through their stops.
            let rgb = |c: &prism_doc::Color| {
                let v = c.to_rgba8();
                Color32::from_rgb(v[0], v[1], v[2])
            };
            let mut mesh = egui::Mesh::default();
            let mut ramp: Vec<(f32, Color32)> = stops.iter().map(|(p, c)| (p.clamp(0.0, 1.0), rgb(c))).collect();
            ramp.insert(0, (0.0, ramp[0].1));
            ramp.push((1.0, ramp[ramp.len() - 1].1));
            for (i, (p, c)) in ramp.iter().enumerate() {
                let x = r.left() + r.width() * p;
                mesh.colored_vertex(egui::pos2(x, r.top()), *c);
                mesh.colored_vertex(egui::pos2(x, r.bottom()), *c);
                if i > 0 {
                    let k = (i as u32) * 2;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k, k + 1);
                }
            }
            ui.painter().add(mesh);
        }
        (None, Some(_)) => {
            // Pattern fill: a neutral checker hint.
            crate::widgets::checker(ui.painter(), r, 4.0);
        }
        (None, None) => {
            // "No colour": white with a red slash, like Photoshop.
            ui.painter().rect_filled(r, 2.0, Color32::WHITE);
            ui.painter().line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.5, Color32::from_rgb(220, 40, 40)));
        }
    };
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    let resp = resp.on_hover_text(tip);
    let screen_id = resp.id.with("screen-color");
    let mut out = crate::screen_picker::take(ui.ctx(), screen_id).map(crate::color_picker_ui::hex);
    crate::widgets::swatch_popup(&resp).show(|ui| {
        let mut c = current.unwrap_or(Color32::BLACK);
        if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
            out = Some(format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()));
        }
        if let Some(rgb) = crate::screen_picker::button(ui, screen_id) {
            out = Some(crate::color_picker_ui::hex(rgb));
        }
        if allow_none && fill.is_some() && ui.button(tl!("No Color")).clicked() {
            out = Some("none".into());
        }
    });
    out
}

/// One corner of the live rectangle, preserving the other three radii.
/// The engine takes the full [top-left, top-right, bottom-right, bottom-left] array.
fn corner_radii_patch(radii: [f64; 4], corner: usize, radius: f32) -> Value {
    let mut next = radii;
    if let Some(r) = next.get_mut(corner) {
        *r = f64::from(radius.max(0.0));
    }
    json!({"radii": next})
}

/// A complete rectangle with just the field's corner rounded or cut (clockwise from top-left).
fn corner_glyph(ui: &mut egui::Ui, key: &str, corner: usize, style: prism_doc::CornerStyle, radius: &mut f32) -> Option<u64> {
    let names = match style {
        prism_doc::CornerStyle::Round => ["corner-round-top-left", "corner-round-top-right", "corner-round-bottom-right", "corner-round-bottom-left"],
        prism_doc::CornerStyle::Chamfer => ["corner-chamfer-top-left", "corner-chamfer-top-right", "corner-chamfer-bottom-right", "corner-chamfer-bottom-left"],
    };
    let name = names.get(corner)?;
    let label = match corner {
        0 => tl!("Top-left corner radius"),
        1 => tl!("Top-right corner radius"),
        2 => tl!("Bottom-right corner radius"),
        _ => tl!("Bottom-left corner radius"),
    };
    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
    let response =
        ui.interact(rect, egui::Id::new((key, "corner-scrub", corner)), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal).on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    crate::icons::paint(ui, rect, name, 20.0, Tokens::get(ui.ctx()).text_dim);
    let drag_id = response.id.with("origin");
    if response.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        let gesture = ui.ctx().cumulative_pass_nr();
        ui.data_mut(|d| d.insert_temp(drag_id, (*radius, origin.x, gesture)));
    }
    let mut changed = None;
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        if let Some((base, start, gesture)) = ui.data(|d| d.get_temp::<(f32, f32, u64)>(drag_id))
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let value = (base + pointer.x - start).round().clamp(0.0, 100000.0);
            if value.is_finite() && value != *radius {
                *radius = value;
                changed = Some(gesture);
            }
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<(f32, f32, u64)>(drag_id));
    }
    changed
}

/// Vertical gap between the rows of Properties › Shape (fields are 24 pt tall).
const ROW_GAP: f32 = 6.0;

/// Properties › Shape corner sizes, laid out like the rectangle: a link button, then a 2 × 2
/// grid of fields, each beside a glyph of its corner. Linked (the default while all four match),
/// a change sizes every corner. Returns the `shape.edit` patch.
fn corner_grid(ui: &mut egui::Ui, key: &str, radii: [f64; 4], style: prism_doc::CornerStyle) -> Option<Value> {
    let link_id = egui::Id::new(key).with("link");
    let same = radii.iter().all(|r| (r - radii[0]).abs() < 1e-9);
    let mut linked = ui.data(|d| d.get_temp::<bool>(link_id)).unwrap_or(same);
    let mut out = None;
    ui.horizontal(|ui| {
        let tip = if linked { tl!("Unlink corner radius values") } else { tl!("Link corner radius values") };
        // The link button sits level with the middle of the two rows of fields below.
        let (column, _) = ui.allocate_exact_size(vec2(22.0, 2.0 * 24.0 + ROW_GAP), Sense::hover());
        let slot = Rect::from_center_size(column.center(), vec2(22.0, 22.0));
        let link = ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| crate::icons::button(ui, if linked { "link" } else { "unlink" }, 22.0, linked, tip)).inner;
        // Named for screen readers and the control channel.
        link.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, linked, tl!(tip)));
        if link.clicked() {
            linked = !linked;
            ui.data_mut(|d| d.insert_temp(link_id, linked));
            // Linking evens the corners out to the top-left one, as the fields then show.
            if linked && !same {
                out = Some(json!({"radii": radii[0]}));
            }
        }
        ui.add_space(4.0);
        ui.vertical(|ui| {
            // Breathing room between the two rows of fields.
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            for row in [[0, 1], [3, 2]] {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for (i, corner) in row.into_iter().enumerate() {
                        let mut v = radii[corner] as f32;
                        let mut scrub = if i == 0 { corner_glyph(ui, key, corner, style, &mut v) } else { None };
                        let typed = crate::widgets::value_field(ui, &mut v, 0.0..=100000.0, "px", 80.0).changed();
                        if i == 1 {
                            scrub = corner_glyph(ui, key, corner, style, &mut v);
                        }
                        if typed || scrub.is_some() {
                            let mut patch = if linked { json!({"radii": f64::from(v.max(0.0))}) } else { corner_radii_patch(radii, corner, v) };
                            patch["coalesce"] = json!(match scrub {
                                Some(gesture) => format!("{key}-corner-scrub-{corner}-{gesture}"),
                                None => format!("{key}-{}", if linked { 4 } else { corner }),
                            });
                            out = Some(patch);
                        }
                    }
                });
            }
        });
    });
    out
}

/// Properties panel for a shape layer: Appearance (fill, stroke) and live shape geometry.
pub fn shape_properties(app: &mut PrismApp, ui: &mut egui::Ui, id: prism_doc::LayerId) {
    let Some(sh) = app.session.active().and_then(|s| s.doc.layer(id)).and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(sh.clone()),
        _ => None,
    }) else {
        return;
    };
    if matches!(sh.live, Some(prism_doc::LiveShape::Arrow { .. })) {
        arrow_properties(app, ui, id, &sh);
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mut edit: Option<Value> = None;
    let key = |k: &str| format!("shape-{}-{k}", id.0);
    // The shared collapsible section headers (#155).
    if crate::props_layout::section(ui, "appearance", tl!("Appearance")) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Fill")).color(t.text_dim).size(12.0));
            if let Some(c) = swatch(ui, sh.fill.as_ref(), tl!("Set shape fill type")) {
                edit = Some(if c == "none" { json!({"fill": null}) } else { json!({"fill": c, "coalesce": key("fill")}) });
            }
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tl!("Stroke")).color(t.text_dim).size(12.0));
            if let Some(c) = swatch(ui, sh.stroke.as_ref().map(|s| &s.paint), tl!("Set shape stroke type")) {
                edit = Some(if c == "none" {
                    json!({"stroke": null})
                } else {
                    json!({"stroke": {"color": c, "width": sh.stroke.as_ref().map_or(3.0, |s| s.width)}, "coalesce": key("stroke")})
                });
            }
            let mut w = sh.stroke.as_ref().map_or(0.0, |s| s.width);
            if crate::widgets::value_field(ui, &mut w, 0.0..=288.0, "px", 60.0).changed() {
                edit = Some(if w <= 0.0 { json!({"stroke": null}) } else { json!({"stroke": {"width": w}, "coalesce": key("stroke-w")}) });
            }
            if sh.stroke.is_some() {
                let mut align = match sh.stroke.as_ref().map(|s| s.align) {
                    Some(prism_doc::vector::StrokeAlign::Inside) => "inside",
                    Some(prism_doc::vector::StrokeAlign::Outside) => "outside",
                    _ => "center",
                }
                .to_string();
                let opts = [("inside".to_string(), "Inside"), ("center".to_string(), "Center"), ("outside".to_string(), "Outside")];
                if crate::widgets::dropdown(ui, &key("align"), &mut align, &opts, 84.0) {
                    edit = Some(json!({"stroke": {"align": align}}));
                }
            }
        });
    }
    if let Some(live) = &sh.live
        && crate::props_layout::section(ui, "liveShape", tl!("Shape"))
    {
        let num = |ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str| -> bool {
            ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim).size(12.0));
            crate::widgets::value_field(ui, v, range, unit, 64.0).changed()
        };
        match live {
            prism_doc::vector::LiveShape::Rect { rect, .. }
            | prism_doc::vector::LiveShape::Ellipse { rect }
            | prism_doc::vector::LiveShape::Polygon { rect, .. } => {
                ui.horizontal(|ui| {
                    let (mut w, mut h) = (rect[2] as f32, rect[3] as f32);
                    // Wide enough for sizes like 1250.5 px: narrower fields clip the leading digits.
                    let field = |ui: &mut egui::Ui, label: &str, v: &mut f32| {
                        ui.label(egui::RichText::new(label).color(t.text_dim).size(12.0));
                        crate::widgets::value_field(ui, v, 1.0..=300000.0, "px", 80.0).changed()
                    };
                    let cw = field(ui, "W", &mut w);
                    let ch = field(ui, "H", &mut h);
                    if cw || ch {
                        edit = Some(json!({"rect": [rect[0], rect[1], w.round(), h.round()], "coalesce": key("wh")}));
                    }
                });
            }
            prism_doc::vector::LiveShape::Line { .. } | prism_doc::LiveShape::Arrow { .. } => {}
        }
        ui.vertical(|ui| match live {
            prism_doc::vector::LiveShape::Rect { radii, corners, .. } => {
                // The same gap as between the corner fields, under W/H too.
                ui.spacing_mut().item_spacing.y = ROW_GAP;
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Corners")).color(t.text_dim).size(12.0));
                    let mut style = *corners;
                    if crate::widgets::dropdown(ui, &key("corner-style"), &mut style, &CORNER_STYLES, 120.0) {
                        edit = Some(json!({"corners": style.as_str()}));
                    }
                });
                if let Some(patch) = corner_grid(ui, &key("radius"), *radii, *corners) {
                    edit = Some(patch);
                }
            }
            prism_doc::vector::LiveShape::Polygon { sides, star_ratio, .. } => {
                ui.horizontal(|ui| {
                    let mut n = *sides as f32;
                    if num(ui, tl!("Sides"), &mut n, 3.0..=100.0, "") {
                        edit = Some(json!({"sides": n.round() as u32, "coalesce": key("sides")}));
                    }
                    let mut sr = (*star_ratio * 100.0) as f32;
                    if num(ui, tl!("Star ratio"), &mut sr, 1.0..=100.0, "%") {
                        edit = Some(json!({"starRatio": sr as f64 / 100.0, "coalesce": key("star")}));
                    }
                })
                .inner
            }
            prism_doc::vector::LiveShape::Line { weight, round_tips, .. } => {
                ui.horizontal(|ui| {
                    let mut w = *weight as f32;
                    if num(ui, tl!("Weight"), &mut w, 1.0..=10000.0, "px") {
                        edit = Some(json!({"weight": w, "coalesce": key("weight")}));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(tl!("Line tips"));
                    let mut rounded = *round_tips;
                    if line_tip_picker(ui, "line-property-tips", &mut rounded) {
                        edit = Some(json!({"roundTips":rounded}));
                    }
                })
                .inner
            }
            _ => {}
        });
    }
    if let Some(mut p) = edit {
        p["layer"] = json!(id.0);
        if let Err(e) = app.run("shape.edit", p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Paths panel

fn thumb(ui: &egui::Ui, r: Rect, path: &Path, doc: &Document) {
    let t = Tokens::get(ui.ctx());
    // At the document's aspect ratio, like the Channels thumbnails.
    let r = crate::channels_panel::fit_thumb(r, doc.size.width, doc.size.height).0;
    ui.painter().rect_filled(r, 0.0, Color32::from_gray(if t.pro { 222 } else { 240 }));
    let (w, h) = (doc.size.width.max(1) as f64, doc.size.height.max(1) as f64);
    let s = (r.width() as f64 / w).min(r.height() as f64 / h);
    let off = r.center() - vec2((w * s) as f32 / 2.0, (h * s) as f32 / 2.0);
    let map = |q: [f64; 2]| pos2(off.x + (q[0] * s) as f32, off.y + (q[1] * s) as f32);
    for (pts, closed) in path_lines(path, &map) {
        if closed && pts.len() >= 3 {
            ui.painter().add(egui::Shape::closed_line(pts, Stroke::new(1.0, Color32::from_gray(40))));
        } else {
            ui.painter().add(egui::Shape::line(pts, Stroke::new(1.0, Color32::from_gray(40))));
        }
    }
}

/// What a Paths panel row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRow {
    Saved,
    Work,
    /// The selected layer's shape path or vector mask (temporary, italic).
    Layer,
}

/// A Paths panel row.
#[derive(Clone, Debug)]
pub struct PathEntry {
    pub name: String,
    pub path: Path,
    pub kind: PathRow,
}

/// The Paths panel's rows, top to bottom: saved paths, the work path, then the selected layer's
/// shape path ("<Layer> Shape Path") or vector mask ("<Layer> Vector Mask"), like Photoshop.
pub fn path_rows(doc: &Document, active: Option<prism_doc::LayerId>) -> Vec<PathEntry> {
    let mut rows: Vec<_> = doc.paths.iter().map(|p| PathEntry { name: p.name.clone(), path: p.path.clone(), kind: PathRow::Saved }).collect();
    if let Some(wp) = &doc.work_path {
        rows.push(PathEntry { name: "Work Path".into(), path: wp.clone(), kind: PathRow::Work });
    }
    if let Some(l) = active.and_then(|id| doc.layer(id)) {
        let layer_path = match &l.content {
            LayerContent::Shape(sh) => Some((format!("{} Shape Path", l.name), sh.path.clone())),
            _ => l.vector_mask.as_ref().map(|v| (format!("{} Vector Mask", l.name), v.path.clone())),
        };
        if let Some((name, path)) = layer_path {
            rows.push(PathEntry { name, path, kind: PathRow::Layer });
        }
    }
    rows
}

fn footer_id() -> egui::Id {
    egui::Id::new("paths-footer")
}

fn ctx_data_footer(ctx: &egui::Context, r: Rect) {
    ctx.data_mut(|d| d.insert_temp(footer_id(), r));
}

/// Where the Paths panel's button footer was drawn last frame.
pub fn paths_footer(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(footer_id()))
}

/// Commands offered by a Paths row's context menu. The row's path is passed explicitly so a
/// right-click acts on that row even when another path is selected in the panel.
fn path_context_actions(entry: &PathEntry, doc: &Document) -> Vec<(&'static str, &'static str, Value)> {
    let key = match entry.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => entry.name.clone(),
    };
    let mut actions = vec![
        ("Make Selection", "path.toSelection", json!({"name": key})),
        ("Fill Path", "path.fill", json!({"name": key})),
        ("Stroke Path", "path.stroke", json!({"name": key, "tool": "brush"})),
    ];
    if entry.kind == PathRow::Work {
        let mut n = doc.paths.len().saturating_add(1);
        while doc.paths.iter().any(|p| p.name == format!("Path {n}")) {
            n = n.saturating_add(1);
            if n == usize::MAX {
                break;
            }
        }
        actions.push(("Save Path", "path.rename", json!({"name": "work", "to": format!("Path {n}")})));
    } else {
        let mut n = 1usize;
        while doc.paths.iter().any(|p| p.name == format!("{} copy {n}", entry.name)) {
            n = n.saturating_add(1);
            if n == usize::MAX {
                break;
            }
        }
        let copy_name = format!("{} copy {n}", entry.name);
        actions.push(("Duplicate Path", "path.set", json!({"name": copy_name, "path": prism_engine::vector_cmds::path_json(&entry.path)})));
    }
    if entry.kind != PathRow::Layer {
        actions.push(("Delete Path", "path.delete", json!({"name": key})));
    } else if entry.name.ends_with(" Vector Mask") {
        actions.push(("Delete Vector Mask", "layer.vectorMask.delete", json!({})));
    }
    actions
}

pub fn paths_panel(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint));
        return;
    };
    let doc = st.doc.clone();
    let rows = path_rows(&doc, st.active_layer);
    let has_layer_path = rows.iter().any(|r| r.kind == PathRow::Layer);
    let mut action: Option<(&str, Value)> = None;
    // The buttons sit in a footer at the panel's bottom, like Photoshop's.
    let footer = 34.0;
    let fill = ui.available_height() > footer + 60.0;
    let rows_h = if fill { ui.available_height() - footer } else { f32::INFINITY };
    egui::ScrollArea::vertical().id_salt("path-rows").max_height(rows_h).min_scrolled_height(if fill { rows_h } else { 0.0 }).auto_shrink([false, !fill]).show(
        ui,
        |ui| {
            if rows.is_empty() {
                ui.label(egui::RichText::new(tl!("Draw with the Pen tool (P) or make a work path from a selection.")).color(t.text_faint).size(11.5));
            }
            for PathEntry { name, path, kind } in &rows {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                let key = match kind {
                    PathRow::Work => "work".to_string(),
                    PathRow::Layer => "layer".to_string(),
                    PathRow::Saved => name.clone(),
                };
                // Rows are painted: name them for screen readers and UI tests.
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, true, name));
                let sel = app.ui.selected_path.as_deref() == Some(key.as_str());
                if sel {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
                }
                thumb(ui, Rect::from_min_size(pos2(r.left() + 8.0, r.top() + 4.0), vec2(28.0, 28.0)), path, &doc);
                let mut job = egui::text::LayoutJob::default();
                // Temporary paths (the work path, the selected layer's shape path or vector mask) are italic.
                let italics = *kind != PathRow::Saved;
                job.append(
                    if *kind == PathRow::Work { tl!(name) } else { name },
                    0.0,
                    egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: t.text, italics, ..Default::default() },
                );
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(r.left() + 46.0, r.center().y - g.size().y / 2.0), g, t.text);
                if resp.clicked() {
                    app.ui.selected_path = Some(key.clone());
                    // Picking the work path shows it again.
                    if *kind == PathRow::Work {
                        app.ui.hidden_work_path = None;
                    }
                }
                // Double-clicking the Work Path saves it (Photoshop's "Save Path").
                if resp.double_clicked() && *kind == PathRow::Work {
                    let n = doc.paths.len() + 1;
                    action = Some(("path.rename", json!({"name": "work", "to": format!("Path {n}")})));
                }
                resp.context_menu(|ui| {
                    crate::widgets::menu_scroll(ui, |ui| {
                        ui.set_min_width(190.0);
                        let entry = PathEntry { name: name.clone(), path: path.clone(), kind: *kind };
                        let can_paint = app.session.active().and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id))).is_some_and(|l| l.surface().is_some());
                        for (label, cmd, params) in path_context_actions(&entry, &doc) {
                            let enabled = !matches!(cmd, "path.fill" | "path.stroke") || can_paint;
                            if ui.add_enabled(enabled, egui::Button::new(tl!(label))).clicked() {
                                action = Some((cmd, params));
                                ui.close();
                            }
                        }
                    });
                });
                ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.separator));
            }
            // Clicking the empty area deselects the path, as in Photoshop: new fill and
            // adjustment layers then take the selection as their mask instead (#1419).
            let rest = ui.available_rect_before_wrap();
            let rest = rest.intersect(Rect::from_min_size(rest.min, vec2(rest.width(), rest.height().min(4000.0))));
            if rest.is_positive() && ui.interact(rest, ui.id().with("paths-empty"), Sense::click()).clicked() {
                app.ui.selected_path = None;
            }
        },
    );
    ui.add_space(4.0);
    crate::widgets::hairline(ui);
    ui.add_space(2.0);
    let sel = app.ui.selected_path.clone().filter(|s| s != "layer" || has_layer_path).unwrap_or_else(|| "work".into());
    let footer_rect = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let fg = hex(app.session.tools.foreground);
        let mut n = doc.paths.len() + 1;
        while doc.paths.iter().any(|p| p.name == format!("Path {n}")) {
            n += 1;
        }
        let items: [(&str, &str, &str, Value); 7] = [
            ("paint-bucket", "Fill path with foreground color", "path.fill", json!({"name": sel, "color": fg})),
            ("circle", "Stroke path with brush", "path.stroke", json!({"name": sel, "tool": "brush"})),
            ("square-dashed", "Load path as a selection", "path.toSelection", json!({"name": sel})),
            ("spline", "Make work path from selection", "select.toWorkPath", json!({"tolerance": 2.0})),
            ("square", "Add vector mask", "layer.vectorMask.fromPath", json!({"name": sel})),
            ("square-plus", "Create new path", "path.set", json!({"name": format!("Path {n}"), "path": {"subpaths": []}})),
            ("trash", "Delete path", "path.delete", json!({"name": sel})),
        ];
        // The layer's own path: deleting it deletes the vector mask (a shape keeps its path).
        let shape = rows.iter().any(|r| r.kind == PathRow::Layer && r.name.ends_with(" Shape Path"));
        let delete_layer_path = sel == "layer" && !shape;
        let items = items.map(|(icon, tip, cmd, p)| {
            if cmd == "path.delete" && delete_layer_path { (icon, "Delete vector mask", "layer.vectorMask.delete", json!({})) } else { (icon, tip, cmd, p) }
        });
        for (icon, tip, cmd, p) in items {
            let icon = if crate::icons::exists(icon) { icon } else { "square" };
            if crate::icons::button(ui, icon, 24.0, false, tip).clicked() {
                action = Some((cmd, p));
            }
        }
    });
    ctx_data_footer(ui.ctx(), footer_rect.response.rect);
    if let Some((cmd, p)) = action
        && let Err(e) = app.run(cmd, p)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

fn line_tip_picker(ui: &mut egui::Ui, id: &str, rounded: &mut bool) -> bool {
    let mut value = if *rounded { "round" } else { "square" }.to_string();
    let changed = crate::widgets::dropdown(ui, id, &mut value, &[("square".into(), tl!("Square tips")), ("round".into(), tl!("Round tips"))], 90.0);
    if changed {
        *rounded = value == "round";
    }
    changed
}

fn arrow_ends(rounded: bool) -> (prism_doc::LineCap, prism_doc::LineJoin) {
    if rounded { (prism_doc::LineCap::Round, prism_doc::LineJoin::Round) } else { (prism_doc::LineCap::Butt, prism_doc::LineJoin::Miter) }
}

fn arrow_stroke(app: &PrismApp) -> Value {
    let o = &app.ui.tool_options.arrow;
    json!({"color": app.session.tools.foreground, "width": o.width,
        "cap": if o.rounded { "round" } else { "butt" }, "join": if o.rounded { "round" } else { "miter" }})
}

fn arrow_corner_picker(ui: &mut egui::Ui, id: &str, rounded: &mut bool) -> bool {
    let mut value = if *rounded { "round" } else { "sharp" }.to_string();
    let changed = crate::widgets::dropdown(ui, id, &mut value, &[("sharp".to_string(), tl!("Angular")), ("round".to_string(), tl!("Rounded"))], 80.0);
    if changed {
        *rounded = value == "round";
    }
    changed
}

fn arrow_options(app: &mut PrismApp, ui: &mut egui::Ui) {
    ui.label(tl!("Color"));
    let fg = app.session.tools.foreground;
    if let Some(color) = swatch_with_none(ui, Some(&prism_doc::Fill::Solid(prism_doc::Color::rgba(fg[0], fg[1], fg[2], fg[3]))), tl!("Arrow color"), false)
        && color != "none"
    {
        // This existing command owns the foreground state and text colour synchronization.
        if let Err(e) = app.run("tools.setColors", json!({"foreground": color})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    let o = &mut app.ui.tool_options.arrow;
    ui.label(tl!("Thickness"));
    crate::widgets::value_field(ui, &mut o.width, 0.1..=1000.0, "px", 62.0);
    ui.label(tl!("Arrowhead length"));
    crate::widgets::value_field(ui, &mut o.head_size, 0.1..=10000.0, "px", 62.0);
    ui.label(tl!("Arrowhead angle"));
    crate::widgets::value_field(ui, &mut o.head_angle, 10.0..=160.0, "°", 62.0);
    ui.label(tl!("Corners"));
    arrow_corner_picker(ui, "arrow-corners", &mut o.rounded);
}

// Separate pointer gestures get separate undo steps, while a held drag or typing
// session reuses its key. This is view-only transient state, not document data.
fn arrow_edit_key(ui: &egui::Ui, response: &egui::Response, layer: prism_doc::LayerId, field: &str) -> String {
    let id = response.id.with("arrow-edit");
    let frame = ui.ctx().cumulative_frame_nr();
    // Response focus/drag queries read the Context too: resolve them before its
    // data write lock, otherwise opening Properties would deadlock the UI thread.
    let fresh = response.drag_started() || response.gained_focus() || (response.changed() && !response.dragged() && !response.has_focus());
    let epoch = ui.data_mut(|d| {
        let old = d.get_temp::<u64>(id).unwrap_or(frame);
        let epoch = if fresh { frame } else { old };
        d.insert_temp(id, epoch);
        epoch
    });
    format!("arrow-{}-{field}-{epoch}", layer.0)
}

fn arrow_properties(app: &mut PrismApp, ui: &mut egui::Ui, id: prism_doc::LayerId, sh: &prism_doc::ShapeLayer) {
    let Some(prism_doc::LiveShape::Arrow { head_size, head_angle, .. }) = sh.live else { return };
    let locked = app.session.active().and_then(|st| st.doc.path_of(id).map(|path| st.doc.locks_at(&path))).is_some_and(|locks| locks.all || locks.pixels);
    let mut edit = None;
    ui.add_enabled_ui(!locked, |ui| {
        if !crate::props_layout::section(ui, "arrow-appearance", tl!("Arrow")) {
            return;
        }
        ui.horizontal(|ui| {
            ui.label(tl!("Color"));
            let coalesce = arrow_color_edit_key(ui, id);
            if let Some(c) = swatch_with_none(ui, sh.stroke.as_ref().map(|s| &s.paint), tl!("Arrow color"), false)
                && c != "none"
            {
                edit = Some(json!({"stroke":{"color":c}, "coalesce":coalesce}));
            }
        });
        ui.horizontal(|ui| {
            ui.label(tl!("Thickness"));
            let mut width = sh.stroke.as_ref().map_or(3.0, |s| s.width);
            let response = crate::widgets::value_field(ui, &mut width, 0.1..=1000.0, "px", 64.0);
            let coalesce = arrow_edit_key(ui, &response, id, "width");
            if response.changed() {
                edit = Some(json!({"stroke":{"width":width}, "coalesce":coalesce}));
            }
        });
        ui.horizontal(|ui| {
            ui.label(tl!("Arrowhead length"));
            let mut head = head_size as f32;
            let response = crate::widgets::value_field(ui, &mut head, 0.1..=10000.0, "px", 64.0);
            let coalesce = arrow_edit_key(ui, &response, id, "head");
            if response.changed() {
                edit = Some(json!({"headSize":head, "coalesce":coalesce}));
            }
        });
        ui.horizontal(|ui| {
            ui.label(tl!("Arrowhead angle"));
            let mut angle = head_angle as f32;
            let response = crate::widgets::value_field(ui, &mut angle, 10.0..=160.0, "°", 64.0);
            let coalesce = arrow_edit_key(ui, &response, id, "angle");
            if response.changed() {
                edit = Some(json!({"headAngle":angle, "coalesce":coalesce}));
            }
        });
        ui.horizontal(|ui| {
            ui.label(tl!("Corners"));
            let mut rounded = sh.stroke.as_ref().is_some_and(|s| s.cap == prism_doc::LineCap::Round && s.join == prism_doc::LineJoin::Round);
            if arrow_corner_picker(ui, "arrow-property-corners", &mut rounded) {
                edit = Some(json!({"stroke":{"cap":if rounded { "round" } else { "butt" }, "join":if rounded { "round" } else { "miter" }}}));
            }
        });
    });
    if let Some(mut params) = edit {
        params["layer"] = json!(id.0);
        if let Err(e) = app.run("shape.edit", params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

fn arrow_color_edit_key(ui: &egui::Ui, layer: prism_doc::LayerId) -> String {
    let id = ui.id().with(("arrow-color-gesture", layer.0));
    let frame = ui.ctx().cumulative_frame_nr();
    let fresh = ui.input(|i| i.pointer.any_pressed() || i.events.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Key { pressed: true, .. })));
    let epoch = ui.data_mut(|d| {
        let epoch = if fresh { frame } else { d.get_temp::<u64>(id).unwrap_or(frame) };
        d.insert_temp(id, epoch);
        epoch
    });
    format!("arrow-{}-color-{epoch}", layer.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PrismApp {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn paths_context_menu_uses_the_clicked_row_and_dispatches_commands() {
        let mut app = app();
        let path = json!({"subpaths": [{"closed": true, "knots": [
            {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
            {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
            {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
        ]}]});
        app.run("path.set", json!({"name": "First", "path": path})).unwrap();
        app.run("path.set", json!({"name": "Second", "path": path})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let first = path_rows(doc, None).into_iter().find(|r| r.name == "First").unwrap();
        let entries = path_context_actions(&first, doc);
        assert_eq!(entries[0].1, "path.toSelection");
        assert_eq!(entries[0].2["name"], "First");
        assert!(entries.iter().all(|(_, id, _)| prism_engine::commands::find(id).is_some()));
        let (_, id, params) = entries.into_iter().find(|(_, id, _)| *id == "path.toSelection").unwrap();
        app.run(id, params).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_some());
        assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("path.toSelection"));

        app.run("path.set", json!({"name": "First copy 1", "path": path})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let first = path_rows(doc, None).into_iter().find(|r| r.name == "First").unwrap();
        let (_, _, duplicate) = path_context_actions(&first, doc).into_iter().find(|(_, id, _)| *id == "path.set").unwrap();
        assert_eq!(duplicate["name"], "First copy 2", "duplicate must not overwrite an existing path");
    }

    #[test]
    fn work_path_context_save_and_layer_path_deletion_rules() {
        let mut app = app();
        let path = json!({"subpaths": []});
        app.run("path.set", json!({"name": "work", "path": path})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let work = path_rows(doc, None).into_iter().find(|r| r.kind == PathRow::Work).unwrap();
        let entries = path_context_actions(&work, doc);
        assert!(entries.iter().any(|(_, id, _)| *id == "path.rename"));
        assert!(!entries.iter().any(|(_, id, _)| *id == "path.set"));
        let layer = PathEntry { kind: PathRow::Layer, ..work };
        let entries = path_context_actions(&layer, doc);
        assert!(!entries.iter().any(|(_, id, _)| *id == "path.delete"));
    }

    /// How many shapes the path overlay paints for `app` this frame.
    fn overlay_shapes(app: &PrismApp, ctx: &egui::Context) -> usize {
        let doc = app.session.active().unwrap().doc.clone();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0));
            let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(rect);
            let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0 };
            draw_overlay(app, &painter, &xf, &doc);
        });
        out.textures_delta.clear();
        out.shapes.len()
    }

    #[test]
    fn show_target_path_and_extras_hide_the_path_outlines() {
        // #1119: View › Show › Target Path only flipped its checkmark; the canvas kept drawing the
        // work path and the active shape's path.
        let ctx = egui::Context::default();
        let mut app = app();
        let path = json!({"subpaths": [{"closed": true, "knots": [
            {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
            {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
            {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
        ]}]});
        app.run("path.set", json!({"path": path})).unwrap();
        assert!(app.session.active().unwrap().doc.work_path.is_some());
        finish_shape(&mut app, Tool::Rectangle, [100.0, 100.0], [160.0, 150.0], egui::Modifiers::NONE);
        let toggle = |app: &mut PrismApp, id: &str| crate::menus::invoke(app, &ctx, id, Value::Null).unwrap();
        for tool in [Tool::Pen, Tool::PathSelection, Tool::DirectSelection, Tool::Rectangle] {
            app.ui.tool = tool;
            assert!(app.ui.view.show.target_path && app.ui.view.extras);
            assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: the paths are drawn by default");
            toggle(&mut app, "view.show.targetPath");
            assert!(!app.ui.view.show.target_path);
            assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: Target Path off");
            toggle(&mut app, "view.show.targetPath");
            toggle(&mut app, "view.extras");
            assert!(!app.ui.view.extras);
            assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: Extras off");
            toggle(&mut app, "view.extras");
            assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: shown again");
        }
        // The Pen's path in progress is a gesture, not the target path: it stays visible.
        app.ui.tool = Tool::Pen;
        app.ui.pen = Some(PenPath { knots: vec![[[20.0, 20.0]; 3], [[60.0, 40.0]; 3]], ..Default::default() });
        toggle(&mut app, "view.show.targetPath");
        assert!(overlay_shapes(&app, &ctx) > 0, "the pen path in progress is still drawn");
    }

    #[test]
    fn shape_drag_creates_layers_with_modifiers() {
        let mut app = app();
        finish_shape(&mut app, Tool::Rectangle, [10.0, 10.0], [60.0, 30.0], egui::Modifiers::SHIFT);
        let st = app.session.active().unwrap();
        let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
        assert_eq!((b.width(), b.height()), (50, 50), "shift = square");
        finish_shape(&mut app, Tool::EllipseShape, [100.0, 100.0], [120.0, 110.0], egui::Modifiers::ALT);
        let st = app.session.active().unwrap();
        let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
        assert!(b.x0.abs_diff(80) <= 1 && b.width().abs_diff(40) <= 1, "alt = from centre: {b:?}");
        finish_shape(&mut app, Tool::Line, [10.0, 150.0], [90.0, 152.0], egui::Modifiers::SHIFT);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 4);
    }

    #[test]
    fn shift_pressed_during_a_shape_drag_constrains_it() {
        // #668: ⇧ only counted when held at the press; Photoshop constrains when it is pressed
        // mid-drag (and stops when it is released).
        use crate::canvas::{ToolEvent, tool_event};
        for (tool, sides) in [(Tool::Polygon, 6), (Tool::Rectangle, 4), (Tool::EllipseShape, 0)] {
            let mut app = app();
            app.ui.tool = tool;
            app.ui.tool_options.polygon_sides = 6;
            let none = egui::Modifiers::NONE;
            tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
            tool_event(&mut app, ToolEvent::Move { x: 40.0, y: 30.0, pressure: 1.0 }, none);
            tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::SHIFT);
            tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 40.0 }, egui::Modifiers::SHIFT);
            let st = app.session.active().unwrap();
            let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
            assert!(matches!(l.content, LayerContent::Shape(_)), "{tool:?} ({sides} sides): a shape layer");
            let b = l.surface().unwrap().content_bounds();
            // The drag is 60 × 30; constrained, the shape's box is 60 × 60 (a hexagon is narrower
            // than its box, so the height tells).
            assert!(b.height().abs_diff(60) <= 1 && b.width() <= 61, "{tool:?}: ⇧ mid-drag keeps proportions: {b:?}");
        }
    }

    /// Types `text` + Enter into field `n` of the corner grid (top-left, top-right, bottom-left,
    /// bottom-right), after clicking the link button `toggles` times; returns the patch.
    fn type_into_corner(radii: [f64; 4], toggles: usize, n: usize, text: &str) -> Option<Value> {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        // The grid shows the document's radii, so each patch is applied as `shape.edit` would.
        let mut h = Harness::builder().with_size(egui::vec2(300.0, 80.0)).build_ui_state(
            |ui, (radii, out): &mut ([f64; 4], Option<Value>)| {
                if let Some(p) = corner_grid(ui, "corner-test", *radii, prism_doc::CornerStyle::Round) {
                    match &p["radii"] {
                        Value::Array(a) => *radii = std::array::from_fn(|i| a[i].as_f64().unwrap()),
                        v => *radii = [v.as_f64().unwrap(); 4],
                    }
                    *out = Some(p);
                }
            },
            (radii, None),
        );
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(4);
        for _ in 0..toggles {
            h.get_all_by_role(egui::accesskit::Role::CheckBox).next().unwrap().click();
            h.run_steps(2);
        }
        h.state_mut().1 = None;
        h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(n).unwrap().click();
        h.run();
        for c in text.chars() {
            h.event(egui::Event::Text(c.to_string()));
            h.run();
        }
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        h.state().1.clone()
    }

    /// Properties › Shape corners: a 2 × 2 grid laid out like the rectangle, linked while the
    /// corners match (one field sizes all four) and per corner once unlinked or uneven.
    #[test]
    fn the_corner_grid_links_like_photoshop() {
        // Even corners start linked: any field sets all four.
        let p = type_into_corner([20.0; 4], 0, 3, "30").unwrap();
        assert_eq!(p["radii"], json!(30.0), "{p}");
        // Unlinked, the field is its own corner (the grid's fourth field is bottom-right).
        let p = type_into_corner([20.0; 4], 1, 3, "30").unwrap();
        assert_eq!(p["radii"], json!([20.0, 20.0, 30.0, 20.0]), "{p}");
        // Uneven corners start unlinked; linking evens them to the top-left one.
        let p = type_into_corner([5.0, 6.0, 7.0, 8.0], 0, 2, "9").unwrap();
        assert_eq!(p["radii"], json!([5.0, 6.0, 7.0, 9.0]), "bottom-left: {p}");
        let p = type_into_corner([5.0, 6.0, 7.0, 8.0], 1, 1, "12").unwrap();
        assert_eq!(p["radii"], json!(12.0), "linked after evening out: {p}");
    }

    /// The Rectangle tool with Chamfered corners draws straight cuts, and the preview matches.
    #[test]
    fn the_rectangle_tool_draws_chamfered_corners() {
        let mut app = app();
        app.ui.tool_options.corner_radius = 8.0;
        app.ui.tool_options.corner_style = prism_doc::CornerStyle::Chamfer;
        let none = egui::Modifiers::NONE;
        let shown = shape_geometry(&app.ui.tool_options, Tool::Rectangle, [10.0, 10.0], [60.0, 40.0], none)
            .and_then(|p| prism_engine::vector_cmds::shape_path(&p).ok())
            .unwrap();
        finish_shape(&mut app, Tool::Rectangle, [10.0, 10.0], [60.0, 40.0], none);
        let st = app.session.active().unwrap();
        let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("no shape layer") };
        assert!(matches!(sh.live, Some(prism_doc::LiveShape::Rect { corners: prism_doc::CornerStyle::Chamfer, radii: [8.0, 8.0, 8.0, 8.0], .. })), "{:?}", sh.live);
        assert_eq!(sh.path, shown);
        assert_eq!(sh.path.subpaths[0].knots.len(), 8, "four straight cuts");
        // With no radius it is a sharp rectangle, but it keeps the style: rounding it later
        // (the corner widgets, Properties) cuts chamfers, not arcs.
        app.ui.tool_options.corner_radius = 0.0;
        let sharp = shape_geometry(&app.ui.tool_options, Tool::Rectangle, [0.0, 0.0], [9.0, 9.0], none).unwrap();
        assert_eq!((sharp["kind"].as_str(), sharp["corners"].as_str()), (Some("rect"), Some("chamfer")));
        finish_shape(&mut app, Tool::Rectangle, [100.0, 100.0], [160.0, 140.0], none);
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("shape.edit", json!({"layer": layer.0, "radii": [10, 10, 10, 10]})).unwrap();
        let st = app.session.active().unwrap();
        let LayerContent::Shape(sh) = &st.doc.layer(layer).unwrap().content else { panic!("no shape layer") };
        assert!(matches!(sh.live, Some(prism_doc::LiveShape::Rect { corners: prism_doc::CornerStyle::Chamfer, .. })), "{:?}", sh.live);
        assert!(sh.path.subpaths[0].knots.iter().all(|k| k.out_ctrl == k.anchor), "straight cuts after rounding");
        // The option is remembered with the other tool options (older settings load as round).
        let o: crate::state::ToolOptions = serde_json::from_value(json!({})).unwrap();
        assert_eq!(o.corner_style, prism_doc::CornerStyle::Round);
    }

    #[test]
    fn shape_preview_is_the_committed_path() {
        let mut app = app();
        app.ui.tool_options.corner_radius = 8.0;
        let preview = |app: &PrismApp, tool, end| {
            shape_geometry(&app.ui.tool_options, tool, [10.0, 10.0], end, egui::Modifiers::ALT).and_then(|p| prism_engine::vector_cmds::shape_path(&p).ok())
        };
        for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::Arrow] {
            let shown = preview(&app, tool, [60.0, 30.0]).unwrap();
            finish_shape(&mut app, tool, [10.0, 10.0], [60.0, 30.0], egui::Modifiers::ALT);
            let st = app.session.active().unwrap();
            let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("{tool:?}: no shape layer") };
            assert_eq!(sh.path, shown, "{tool:?}");
        }
        assert!(preview(&app, Tool::Rectangle, [10.2, 40.0]).is_none(), "too thin to draw");
        assert!(prism_engine::vector_cmds::shape_path(&json!({"kind": "nope", "rect": [0, 0, 5, 5]})).is_err());
        assert!(prism_engine::vector_cmds::shape_path(&json!({"kind": "rect"})).is_err());
    }

    #[test]
    fn pen_builds_and_closes_a_work_path() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        for (x, y) in [(20.0, 20.0), (120.0, 20.0)] {
            pen_down(&mut app, x, y);
            pen_up(&mut app);
        }
        pen_down(&mut app, 120.0, 120.0);
        pen_move(&mut app, 140.0, 140.0); // smooth knot
        pen_up(&mut app);
        pen_down(&mut app, 20.5, 20.5); // click the first anchor: close
        assert!(app.ui.pen.is_none());
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        assert!(wp.subpaths[0].closed && wp.subpaths[0].knots.len() == 3);
        assert!(wp.subpaths[0].knots[2].smooth);
        // Path Selection moves it.
        path_selection_finish(&mut app, [50.0, 50.0], [60.0, 55.0]);
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        assert_eq!((wp.subpaths[0].knots[0].anchor.x, wp.subpaths[0].knots[0].anchor.y), (30.0, 25.0));
    }

    #[test]
    fn pen_hovering_the_first_anchor_shows_close_feedback() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
            pen_down(&mut app, x, y);
            pen_up(&mut app);
        }
        let ctx = egui::Context::default();
        let rings = |app: &PrismApp| {
            let doc = app.session.active().unwrap().doc.clone();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0));
                let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(rect);
                let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0 };
                draw_overlay(app, &painter, &xf, &doc);
            });
            out.textures_delta.clear();
            out.shapes.iter().filter(|s| matches!(s.shape, egui::Shape::Circle(_))).count()
        };
        app.hover_doc = Some([60.0, 60.0]);
        assert_eq!(rings(&app), 0);
        assert_eq!(pen_cursor_asset(&app, [60.0, 60.0]), 1030);
        // On the first anchor: a ring around it (plus the closing segment preview).
        app.hover_doc = Some([21.0, 21.0]);
        assert_eq!(rings(&app), 1);
        assert_eq!(pen_cursor_asset(&app, [21.0, 21.0]), 1034);
        // A two-anchor minimum still applies: one anchor never closes.
        let mut one = self::app();
        one.ui.tool = Tool::Pen;
        pen_down(&mut one, 20.0, 20.0);
        pen_up(&mut one);
        assert_eq!(pen_cursor_asset(&one, [20.5, 20.5]), 1066);
    }

    fn pen_triangle(app: &mut PrismApp, x: f64) {
        for (dx, y) in [(0.0, 20.0), (40.0, 20.0), (20.0, 60.0)] {
            pen_down(app, x + dx, y);
            pen_up(app);
        }
        pen_down(app, x, 20.0); // close
        pen_up(app);
    }

    fn work(app: &PrismApp) -> Path {
        app.session.active().unwrap().doc.work_path.clone().unwrap()
    }

    #[test]
    fn pen_subpaths_join_the_selected_work_path_with_the_path_operation() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        app.ui.tool_options.pen_auto_add_delete = false;
        pen_triangle(&mut app, 10.0);
        app.ui.tool_options.pen_path_op = "subtract".into();
        pen_triangle(&mut app, 100.0);
        let wp = work(&app);
        assert_eq!(wp.subpaths.len(), 2);
        assert_eq!(wp.subpaths[1].op, prism_doc::PathOp::Subtract);
        // Deselected in the Paths panel: the next path starts a new work path.
        app.ui.selected_path = None;
        pen_triangle(&mut app, 200.0);
        assert_eq!(work(&app).subpaths.len(), 1);
    }

    #[test]
    fn pen_auto_add_delete_edits_the_shown_path() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        pen_triangle(&mut app, 10.0);
        // A press on the top edge adds an anchor there (and starts no new path).
        pen_down(&mut app, 30.0, 20.0);
        pen_up(&mut app);
        assert!(app.ui.pen.as_ref().is_none_or(|p| p.knots.is_empty()));
        assert_eq!(work(&app).subpaths[0].knots.len(), 4);
        assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("path.addAnchor"));
        // A press on an anchor deletes it.
        pen_down(&mut app, 30.0, 20.0);
        pen_up(&mut app);
        assert_eq!(work(&app).subpaths[0].knots.len(), 3);
        // The overlay shows the + / − badge only when it applies.
        app.hover_doc = Some([30.0, 20.0]);
        assert!(matches!(auto_edit_at(&app, [30.0, 20.0]), Some((_, Hit::Segment(..)))));
        assert!(matches!(auto_edit_at(&app, [10.0, 20.0]), Some((_, Hit::Anchor(_)))));
        assert!(auto_edit_at(&app, [150.0, 150.0]).is_none());
        // Off: the press starts a new subpath instead.
        app.ui.tool_options.pen_auto_add_delete = false;
        assert!(auto_edit_at(&app, [30.0, 20.0]).is_none());
        pen_down(&mut app, 30.0, 20.0);
        pen_up(&mut app);
        assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(1));
    }

    #[test]
    fn pen_shape_mode_aligns_edges_and_adds_to_the_shape_layer() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        app.ui.tool_options.vector_mode = "shape".into();
        app.ui.tool_options.pen_auto_add_delete = false;
        pen_down(&mut app, 10.4, 20.6);
        assert_eq!(app.ui.pen.as_ref().unwrap().knots[0][0], [10.0, 21.0], "Align Edges snaps to pixels");
        pen_up(&mut app);
        app.ui.pen = None;
        pen_triangle(&mut app, 10.0);
        let layers = app.session.active().unwrap().doc.layers.len();
        app.ui.tool_options.pen_shape_op = "exclude".into();
        pen_triangle(&mut app, 30.0);
        let (_, path) = active_shape_path(&app).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "added to the selected shape layer");
        assert_eq!((path.subpaths.len(), path.subpaths[1].op), (2, prism_doc::PathOp::Exclude));
        app.ui.tool_options.pen_shape_op = "new".into();
        pen_triangle(&mut app, 60.0);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    }

    #[test]
    fn pen_rubber_band_previews_the_next_segment_only_when_on() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        pen_down(&mut app, 20.0, 20.0);
        pen_up(&mut app);
        app.hover_doc = Some([90.0, 70.0]);
        let ctx = egui::Context::default();
        let off = overlay_shapes(&app, &ctx);
        app.ui.tool_options.pen_rubber_band = true;
        assert_eq!(overlay_shapes(&app, &ctx), off + 1);
    }

    #[test]
    fn pen_bar_make_buttons_and_component_menus() {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        app.ui.tool_options.pen_auto_add_delete = false;
        assert!(!crate::canvas_tool_menu::pen_bar_enabled(&app, "path.makeShape"), "no path yet");
        pen_triangle(&mut app, 10.0);
        pen_triangle(&mut app, 100.0);
        for cmd in ["path.toSelection", "layer.vectorMask.fromPath", "path.makeShape"] {
            assert!(crate::canvas_tool_menu::pen_bar_enabled(&app, cmd), "{cmd}");
        }
        // The bar acts on the newest component without a Direct Selection pick.
        let (r, _, picked) = pen_bar_target(&app).unwrap();
        assert_eq!((r, picked), (crate::direct_select::PathRef::Work, None));
        run_on_bar_path(&mut app, "path.alignComponents", r, json!({"edge": "top"})).unwrap();
        assert!(work(&app).subpaths.iter().all(|s| s.knots[0].anchor.y == 20.0));
        let ctx = egui::Context::default();
        crate::canvas_tool_menu::pen_bar_run(&mut app, &ctx, "path.makeShape");
        assert!(app.session.active().unwrap().doc.work_path.is_none(), "Shape consumes the work path");
        assert!(active_shape_path(&app).is_some());
        // The bar renders in both modes.
        PrismApp::setup_context(&ctx, Default::default());
        for mode in ["path", "shape"] {
            app.ui.tool_options.vector_mode = mode.into();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.horizontal(|ui| assert!(options_bar(&mut app, ui, Tool::Pen)));
            });
            out.textures_delta.clear();
        }
    }

    /// #1419: Solid Color (and every new fill or adjustment layer) made while a path is selected
    /// in the Paths panel takes it as its vector mask; with the path deselected, the selection
    /// is its layer mask again.
    #[test]
    fn new_fill_layer_takes_the_selected_path_as_its_vector_mask() {
        let mut app = app();
        let layer = |app: &PrismApp, r: Result<Value, String>| {
            let id = prism_doc::LayerId(r.unwrap()["layer"].as_u64().unwrap());
            app.session.active().unwrap().doc.layer(id).unwrap().clone()
        };
        // Drawing a work path with the Pen selects it.
        app.ui.tool = Tool::Pen;
        for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
            pen_down(&mut app, x, y);
            pen_up(&mut app);
        }
        pen_down(&mut app, 20.5, 20.5);
        assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
        let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
        app.run("select.rect", json!({"x": 150, "y": 150, "width": 20, "height": 20})).unwrap();
        for id in ["layer.newFillLayer.solidColor", "layer.newFillLayer.gradient", "layer.newAdjustmentLayer.invert"] {
            app.ui.selected_path = Some("work".into());
            let r = app.run(id, json!({}));
            let l = layer(&app, r);
            assert_eq!(l.vector_mask.as_ref().map(|v| &v.path), Some(&wp), "{id}: the work path is the vector mask");
            assert!(l.mask.is_none(), "{id}: the path wins over the selection");
            // The new layer's vector mask is now the active path, not the work path.
            assert_eq!(app.ui.selected_path.as_deref(), Some("layer"), "{id}");
        }
        // So the next fill layer isn't masked by the work path again.
        let r = app.run("layer.newFillLayer.solidColor", json!({}));
        assert!(layer(&app, r).vector_mask.is_none());
        // A caller can opt out.
        let r = app.run("layer.newFillLayer.solidColor", json!({"path": null}));
        assert!(layer(&app, r).vector_mask.is_none());
        // A selected saved path is used.
        app.run("path.set", json!({"name": "Saved", "path": {"subpaths": [{"closed": true, "knots": [[0, 0], [9, 0], [9, 9]]}]}})).unwrap();
        app.ui.selected_path = Some("Saved".into());
        let saved = app.session.active().unwrap().doc.paths[0].path.clone();
        let r = app.run("layer.newFillLayer.solidColor", json!({}));
        assert_eq!(layer(&app, r).vector_mask.map(|v| v.path), Some(saved));
        // Deselected (or a stale name): the selection is the layer mask, no vector mask.
        for sel in [None, Some("Gone".to_string())] {
            app.ui.selected_path = sel;
            let r = app.run("layer.newFillLayer.solidColor", json!({}));
            let l = layer(&app, r);
            assert!(l.vector_mask.is_none() && l.mask.is_some());
        }
        // Make Work Path selects the new work path.
        app.run("select.toWorkPath", json!({"tolerance": 2.0})).unwrap();
        assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    }

    /// #1482: clicking an existing final Pen anchor breaks its outgoing handle without
    /// deleting the preceding curve or creating another knot; dragging changes only that handle.
    #[test]
    fn pen_last_anchor_can_be_broken_and_reshaped_while_drawing() {
        use crate::canvas::{ToolEvent, tool_event};
        let mut app = app();
        app.ui.tool = Tool::Pen;
        let down = |app: &mut PrismApp, x, y| tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
        let move_to = |app: &mut PrismApp, x, y| tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
        let up = |app: &mut PrismApp, x, y| tool_event(app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);

        down(&mut app, 20.0, 20.0);
        up(&mut app, 20.0, 20.0);
        down(&mut app, 100.0, 20.0);
        move_to(&mut app, 120.0, 40.0);
        up(&mut app, 120.0, 40.0);
        let original = app.ui.pen.as_ref().unwrap().knots[1];
        assert_eq!(original, [[100.0, 20.0], [80.0, 0.0], [120.0, 40.0]]);

        // Bare click breaks the outgoing handle, preserving the preceding segment's tangent.
        down(&mut app, 100.0, 20.0);
        up(&mut app, 100.0, 20.0);
        let pen = app.ui.pen.as_ref().unwrap();
        assert_eq!(pen.knots.len(), 2);
        assert_eq!(pen.knots[1], [[100.0, 20.0], [80.0, 0.0], [100.0, 20.0]]);
        assert!(!pen.dragging && !pen.adjusting_last);

        // Re-dragging the same final anchor moves only its outgoing handle (a cusp).
        down(&mut app, 100.0, 20.0);
        move_to(&mut app, 110.0, 60.0);
        up(&mut app, 110.0, 60.0);
        let pen = app.ui.pen.as_ref().unwrap();
        assert_eq!(pen.knots.len(), 2);
        assert_eq!(pen.knots[1], [[100.0, 20.0], [80.0, 0.0], [110.0, 60.0]]);

        // Drawing on still adds a separate next anchor and keeps the cusp in the work path.
        down(&mut app, 150.0, 100.0);
        up(&mut app, 150.0, 100.0);
        assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 3);
        pen_commit(&mut app, false);
        let work = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
        let k = &work.subpaths[0].knots[1];
        assert_eq!([k.in_ctrl.x, k.in_ctrl.y], [80.0, 0.0]);
        assert_eq!([k.out_ctrl.x, k.out_ctrl.y], [110.0, 60.0]);
        assert!(!k.smooth, "breaking the handle must persist as an unlinked PSD/vector knot");
    }

    #[test]
    fn pen_last_anchor_hit_tolerance_tracks_zoom_and_draft_round_trips() {
        for zoom in [0.25, 1.0, 4.0] {
            let mut app = app();
            app.ui.views[0].zoom = zoom;
            pen_down(&mut app, 20.0, 20.0);
            pen_up(&mut app);
            pen_down(&mut app, 100.0, 100.0);
            pen_move(&mut app, 110.0, 120.0);
            pen_up(&mut app);
            let incoming = app.ui.pen.as_ref().unwrap().knots[1][1];
            pen_down(&mut app, 100.0 + 5.0 / f64::from(zoom), 100.0);
            pen_up(&mut app);
            let pen = app.ui.pen.as_ref().unwrap();
            assert_eq!(pen.knots.len(), 2);
            assert_eq!(pen.knots[1][1], incoming);
            assert_eq!(pen.knots[1][2], pen.knots[1][0]);
            let saved = serde_json::to_value(pen).unwrap();
            let restored: PenPath = serde_json::from_value(saved).unwrap();
            assert_eq!(restored, *pen);
            assert_eq!(pen_to_json(&restored, false)["subpaths"][0]["knots"][1]["smooth"], false);
            pen_down(&mut app, 100.0 + 7.0 / f64::from(zoom), 100.0);
            assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 3);
        }
        let legacy: PenPath = serde_json::from_value(json!({"knots": [[[10, 10], [10, 10], [10, 10]]]})).unwrap();
        assert!(legacy.unlinked.is_empty());
        assert!(!legacy.dragging && !legacy.adjusting_last);
    }

    /// #1520: changing one corner through the Properties command must keep the other three
    /// independent, remain a live rectangle, and undo/redo as one geometry edit.
    #[test]
    fn live_rectangle_corner_radius_edit_preserves_other_corners_and_history() {
        let mut app = app();
        let created = app.run("shape.create", json!({"kind": "roundedRect", "rect": [10, 10, 150, 100], "radii": [5, 10, 15, 20]})).unwrap();
        let id = created["layer"].as_u64().unwrap();
        let get_radii = |app: &PrismApp| {
            let st = app.session.active().unwrap();
            let shape = st.doc.layer(prism_doc::LayerId(id)).unwrap();
            match &shape.content {
                LayerContent::Shape(sh) => match &sh.live {
                    Some(prism_doc::vector::LiveShape::Rect { radii, .. }) => *radii,
                    _ => panic!("rectangle must remain a live shape"),
                },
                _ => panic!("expected shape layer"),
            }
        };
        assert_eq!(get_radii(&app), [5.0, 10.0, 15.0, 20.0]);
        let mut patch = corner_radii_patch(get_radii(&app), 1, 30.0);
        patch["layer"] = json!(id);
        app.run("shape.edit", patch).unwrap();
        assert_eq!(get_radii(&app), [5.0, 30.0, 15.0, 20.0]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(get_radii(&app), [5.0, 10.0, 15.0, 20.0]);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(get_radii(&app), [5.0, 30.0, 15.0, 20.0]);
        for corner in 0..4 {
            let before = get_radii(&app);
            let radius = 35.0 + corner as f32;
            let mut expected = before;
            expected[corner] = f64::from(radius);
            let mut patch = corner_radii_patch(before, corner, radius);
            patch["layer"] = json!(id);
            app.run("shape.edit", patch).unwrap();
            assert_eq!(get_radii(&app), expected);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(get_radii(&app), before);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(get_radii(&app), expected);
        }
        app.run("shape.edit", json!({"layer": id, "radii": 12})).unwrap();
        assert_eq!(get_radii(&app), [12.0; 4]);
    }

    /// #534: dragging in a shape's fill picker, opened from the Properties panel at the right edge
    /// of the window, keeps the picker in place (it flipped from side to side as its width
    /// followed the colour readouts, which moved it under the pointer).
    #[test]
    fn shape_colour_picker_stays_put_while_dragging() {
        use egui::{PointerButton, vec2};
        use egui_kittest::kittest::Queryable;
        let mut app = app();
        let id = app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 100, 60], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            move |ui, app: &mut PrismApp| {
                // Fonts set up after the first frame only apply from the next one.
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::Panel::right("properties").resizable(false).exact_size(285.0).show(ui, |ui| shape_properties(app, ui, prism_doc::LayerId(id)));
            },
            app,
        );
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(4);
        let label = h.query_all(egui_kittest::kittest::by().label("Fill").include_labels()).next().expect("the Fill label").rect();
        // The swatch is right of its label.
        let swatch = egui::pos2(label.right() + 17.0, label.center().y);
        let press = |h: &mut egui_kittest::Harness<'_, PrismApp>, pos, pressed| {
            h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        };
        h.hover_at(swatch);
        h.step();
        press(&mut h, swatch, true);
        press(&mut h, swatch, false);
        h.run_steps(3);
        let popup = |h: &egui_kittest::Harness<'_, PrismApp>| {
            h.ctx.memory(|m| m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)))
        };
        let opened = popup(&h).expect("the picker opened");
        let fill = |h: &egui_kittest::Harness<'_, PrismApp>| {
            let st = h.state().session.active().unwrap();
            match &st.doc.layer(prism_doc::LayerId(id)).unwrap().content {
                LayerContent::Shape(sh) => sh.fill.clone(),
                _ => None,
            }
        };
        let before = fill(&h);
        // Drag across the picker's colour area.
        let start = opened.min + vec2(60.0, opened.height() * 0.5);
        h.hover_at(start);
        h.step();
        press(&mut h, start, true);
        for i in 1..=10 {
            h.event(egui::Event::PointerMoved(start + vec2(i as f32 * 5.0, i as f32)));
            h.step();
            let r = popup(&h).expect("the picker stays open");
            assert_eq!(r.left_top(), opened.left_top(), "step {i}: the picker moved from {opened:?} to {r:?}");
            assert!(h.ctx.content_rect().contains_rect(r), "step {i}: {r:?} runs off the window");
        }
        press(&mut h, start, false);
        assert_ne!(fill(&h), before, "the drag picked a colour");
    }
}

#[cfg(test)]
#[path = "path_selection_tests.rs"]
mod path_selection_tests;

#[cfg(test)]
mod arrow_ui_tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    #[test]
    fn arrow_color_drags_have_distinct_undo_groups() {
        let ctx = egui::Context::default();
        let mut keys = Vec::new();
        for pressed in [None, Some(true), None, Some(false), Some(true), None] {
            let mut input = egui::RawInput::default();
            if let Some(pressed) = pressed {
                input.events.push(egui::Event::PointerButton {
                    pos: egui::pos2(10.0, 10.0),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            ctx.run_ui(input, |ui| keys.push(arrow_color_edit_key(ui, prism_doc::LayerId(1)))).textures_delta.clear();
        }
        assert_eq!(keys[1], keys[2]);
        assert_eq!(keys[2], keys[3]);
        assert_ne!(keys[3], keys[4]);
        assert_eq!(keys[4], keys[5]);
    }

    #[test]
    fn arrow_options_and_properties_render_without_nested_context_locks() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":200,"height":200})).unwrap();
        let id = app.run("shape.create", json!({"kind":"arrow","from":[20,20],"to":[100,100]})).unwrap()["layer"].as_u64().unwrap();
        let ctx = egui::Context::default();
        PrismApp::setup_context(&ctx, Default::default());
        ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| {
                options_bar(&mut app, ui, Tool::Arrow);
            });
            shape_properties(&mut app, ui, prism_doc::LayerId(id));
        })
        .textures_delta
        .clear();
        PrismApp::setup_context(&ctx, crate::theme::ThemeKind::StudioLight);
        app.ui.panels.properties = true;
        app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0));
        for _ in 0..2 {
            ctx.run_ui(egui::RawInput::default(), |ui| crate::panels::properties_window(&mut app, ui.ctx())).textures_delta.clear();
        }
        assert!(crate::props_layout::sections_drawn(&ctx).iter().any(|(name, _)| name == tl!("Arrow")));
    }

    #[test]
    fn arrow_drag_shift_cancel_and_style() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":200,"height":200})).unwrap();
        app.ui.tool = Tool::Arrow;
        let mods = egui::Modifiers::SHIFT;
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, mods);
        tool_event(&mut app, ToolEvent::Move { x: 140.0, y: 50.0, pressure: 1.0 }, mods);
        tool_event(&mut app, ToolEvent::Up { x: 140.0, y: 50.0 }, mods);
        let info = app.run("shape.info", json!({})).unwrap();
        assert_eq!(info["kind"], "arrow");
        assert_eq!(info["live"]["to"][1], 30.0);
        assert_eq!(info["stroke"]["cap"], "round");
        assert!(info["fill"].is_null());
        let before = app.session.active().unwrap().doc.layers.len();
        tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 80.0, pressure: 1.0 }, mods);
        tool_event(&mut app, ToolEvent::Move { x: 120.0, y: 130.0, pressure: 1.0 }, mods);
        assert!(app.drag.is_some());
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(mods),
                egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: mods },
            ],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| crate::shortcuts::handle(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.drag.is_none());
        tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 130.0 }, mods);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), before);
        app.ui.tool_options.arrow.rounded = false;
        finish_shape(&mut app, Tool::Arrow, [20.0, 100.0], [25.0, 100.0], egui::Modifiers::NONE);
        let info = app.run("shape.info", json!({})).unwrap();
        assert_eq!(info["stroke"]["cap"], "butt");
        assert_eq!(info["stroke"]["join"], "miter");
    }
}

#[cfg(test)]
mod line_tip_tests {
    use super::*;
    #[test]
    fn line_tips_option_matches_created_geometry_and_renders_properties() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":200,"height":200})).unwrap();
        assert!(!app.ui.tool_options.line_round_tips);
        app.ui.tool_options.line_round_tips = true;
        finish_shape(&mut app, Tool::Line, [20.0, 30.0], [130.0, 80.0], Default::default());
        let info = app.run("shape.info", json!({})).unwrap();
        assert_eq!(info["live"]["roundTips"], true);
        let id = prism_doc::LayerId(info["layer"].as_u64().unwrap());
        let ctx = egui::Context::default();
        PrismApp::setup_context(&ctx, Default::default());
        ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| options_bar(&mut app, ui, Tool::Line));
            ui.set_width(250.0);
            shape_properties(&mut app, ui, id);
        })
        .textures_delta
        .clear();
    }
}

#[cfg(test)]
#[path = "vector_ui/corner_scrub_tests.rs"]
mod corner_scrub_tests;
