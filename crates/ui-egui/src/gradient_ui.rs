//! Gradient tool, live mode (Photoshop 2023+ "Gradient" vs "Classic gradient"): a drag makes a
//! Gradient Fill layer (`gradient.fill.create`), and with a gradient fill layer selected the canvas
//! shows its widget: start and end handles (drag to move, ⇧ snaps the angle to 45°), the colour
//! stops along the line (drag to move, drag off the line to delete, double-click for the Color
//! Picker), midpoint diamonds between them, a click on the line adds a stop, and dragging the line
//! moves the whole gradient. A drag elsewhere redraws the selected gradient. Radial and Angle
//! gradients show their circle, Diamond its square, Reflected its mirrored half. Over the widget
//! the pointer becomes the move cursor (a plus over the line), and the control last clicked
//! stays selected, ringed in the accent colour.
//!
//! Every gesture previews live through the regular canvas path (a shallow copy of the document
//! with the edited fill, so the GPU canvas just re-runs its gradient pass) and commits one command
//! on release: one history step. The Properties panel's stops editor ([`properties`]) previews
//! through the same path.

use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use prism_compose::gradient_fill as gf;
use prism_doc::{Document, Fill, GradientStyle, Layer, LayerContent, LayerId};
use prism_engine::gradient_fill_cmds as cmds;
use serde_json::{Value, json};

use crate::PrismApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;
use crate::theme::Tokens;
use crate::widgets;

/// Id of the temporary layer previewing a new gradient while it is drawn.
pub const PREVIEW_LAYER: LayerId = LayerId(u64::MAX - 78);
/// Tag of this module's canvas preview keys.
const TAG: u64 = 1 << 58;
/// Screen distance (points) within which a handle, stop or the line is grabbed.
const GRAB: f32 = 8.0;
/// Screen distance from the line past which a dragged stop is deleted.
const TEAR_OFF: f32 = 36.0;
/// Seconds between two clicks of a double-click.
const DOUBLE_CLICK: f64 = 0.45;

/// What a pointer press grabbed on the widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    Start,
    End,
    /// A colour stop (index in sorted order).
    Stop(usize),
    /// The midpoint of the segment after colour stop `i`.
    Mid(usize),
    /// The line at gradient location `t`.
    Line(f32),
}

#[derive(Clone, Debug)]
enum Drag {
    /// Drawing a new gradient, or redrawing the selected one.
    Draw { from: [f32; 2], to: [f32; 2], redraw: Option<LayerId> },
    /// Editing the selected gradient: the command (and params) to commit on release.
    Edit { layer: LayerId, grab: Grab, down: [f32; 2], moved: bool, commit: Option<(&'static str, Value)> },
}

/// Live-gradient interaction state (not saved).
#[derive(Default)]
pub struct LiveGradient {
    drag: Option<Drag>,
    last_click: Option<(Grab, f64)>,
    /// The handle, stop or midpoint last clicked on a gradient fill layer: drawn with a ring
    /// round it, as Photoshop shows the selected control.
    selected: Option<(LayerId, Grab)>,
    /// Pending edit from the Properties panel's stops editor (previewed like a canvas drag).
    panel: Option<(LayerId, &'static str, Value)>,
    /// Cached preview document by key.
    preview: Option<(u64, Arc<Document>)>,
}

/// Whether the Gradient tool is in its live (Gradient Fill layer) mode.
/// Painting into a layer mask, an alpha channel or the Quick Mask stays classic (pixels), except
/// that a selected gradient fill layer is always edited live (selecting it targets its mask).
pub fn live_mode(app: &PrismApp) -> bool {
    if app.ui.tool != Tool::Gradient || app.ui.tool_options.gradient_classic {
        return false;
    }
    match crate::canvas::paint_target(app).as_str() {
        Some("pixels") => true,
        Some("mask") => active_gradient(app).is_some(),
        _ => false,
    }
}

/// The active layer when it is a gradient fill layer: (layer, its frame, the canvas).
fn active_gradient(app: &PrismApp) -> Option<(Layer, Rect32, prism_geom::Rect)> {
    let st = app.session.active()?;
    let l = st.doc.layer(st.active_layer?)?;
    if !matches!(l.content, LayerContent::Fill(Fill::Gradient { .. })) || !l.visible {
        return None;
    }
    let canvas = st.doc.bounds();
    Some((l.clone(), prism_compose::fill_frame(l, canvas), canvas))
}

type Rect32 = prism_geom::Rect;

fn fill_of(l: &Layer) -> Option<&Fill> {
    match &l.content {
        LayerContent::Fill(f @ Fill::Gradient { .. }) => Some(f),
        _ => None,
    }
}

/// The fill an in-progress edit shows (`None` when it can't apply, e.g. a stale index).
fn edited_fill(app: &PrismApp, layer: &Layer, canvas: Rect32, cmd: &str, p: &Value) -> Option<Fill> {
    let f = fill_of(layer)?;
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    match cmd {
        cmds::SET => cmds::apply_set(layer, f, canvas, p, fg, bg).ok(),
        cmds::STOP => cmds::apply_stop(f, p, fg, bg).ok(),
        _ => None,
    }
}

/// Options-bar params of a new live gradient.
fn create_params(app: &PrismApp, from: [f32; 2], to: [f32; 2]) -> Value {
    let o = &app.ui.tool_options;
    json!({"from": from, "to": to, "style": o.gradient_style, "reverse": o.gradient_reverse, "dither": o.gradient_dither, "opacity": o.fill_opacity, "mode": o.gradient_blend_mode.label()})
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Position on the segment `s`→`e` at fraction `u`.
fn lerp(s: [f32; 2], e: [f32; 2], u: f32) -> [f32; 2] {
    [s[0] + (e[0] - s[0]) * u, s[1] + (e[1] - s[1]) * u]
}

/// Fraction along `s`→`e` of `p`'s projection (clamped) and `p`'s distance from the line.
fn project(s: [f32; 2], e: [f32; 2], p: [f32; 2]) -> (f32, f32) {
    let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
    let l2 = dx * dx + dy * dy;
    if l2 <= 1e-9 {
        return (0.0, dist(s, p));
    }
    let u = (((p[0] - s[0]) * dx + (p[1] - s[1]) * dy) / l2).clamp(0.0, 1.0);
    (u, dist(lerp(s, e, u), p))
}

/// The widget's geometry for a gradient fill: handles, stop positions, midpoint positions.
struct Widget {
    style: GradientStyle,
    start: [f32; 2],
    end: [f32; 2],
    reverse: bool,
    /// Sorted colour stops: (location, rgba).
    stops: Vec<(f32, [f32; 4])>,
    mids: Vec<f32>,
}

impl Widget {
    fn new(f: &Fill, frame: Rect32) -> Option<Widget> {
        let Fill::Gradient { stops, midpoints, reverse, style, .. } = f else { return None };
        let (start, end) = gf::fill_handles(f, frame)?;
        let mut st: Vec<(f32, [f32; 4])> = stops
            .iter()
            .map(|(t, c)| {
                let rgb = c.to_rgb();
                (*t, [rgb[0], rgb[1], rgb[2], c.alpha])
            })
            .collect();
        st.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(Widget { style: *style, start, end, reverse: *reverse, stops: st, mids: midpoints.clone() })
    }

    /// Fraction along the line of gradient location `t`.
    fn u_of(&self, t: f32) -> f32 {
        if self.reverse { 1.0 - t } else { t }
    }

    fn at(&self, t: f32) -> [f32; 2] {
        lerp(self.start, self.end, self.u_of(t))
    }

    fn mid_location(&self, i: usize) -> Option<f32> {
        let (a, b) = (self.stops.get(i)?.0, self.stops.get(i + 1)?.0);
        (b - a > 1e-4).then(|| a + (b - a) * self.mids.get(i).copied().unwrap_or(0.5))
    }

    /// What `p` (document pixels) grabs, with `tol` the grab distance in document pixels.
    fn hit(&self, p: [f32; 2], tol: f32) -> Option<Grab> {
        if dist(p, self.end) <= tol {
            return Some(Grab::End);
        }
        if dist(p, self.start) <= tol {
            return Some(Grab::Start);
        }
        let near = |q: [f32; 2]| dist(p, q) <= tol;
        if let Some(i) = self.stops.iter().position(|s| near(self.at(s.0))) {
            return Some(Grab::Stop(i));
        }
        if let Some(i) = (0..self.stops.len().saturating_sub(1)).find(|i| self.mid_location(*i).is_some_and(|t| near(self.at(t)))) {
            return Some(Grab::Mid(i));
        }
        let (u, d) = project(self.start, self.end, p);
        (d <= tol).then_some(Grab::Line(if self.reverse { 1.0 - u } else { u }))
    }
}

/// The selected control of the active gradient fill layer, if it still exists.
fn selected(app: &PrismApp) -> Option<Grab> {
    let (id, grab) = app.gradient.selected?;
    let (l, frame, _) = active_gradient(app).filter(|a| a.0.id == id)?;
    let w = Widget::new(fill_of(&l)?, frame)?;
    match grab {
        Grab::Stop(i) if i >= w.stops.len() => None,
        Grab::Mid(i) if w.mid_location(i).is_none() => None,
        g => Some(g),
    }
}

/// The colour stop of the active gradient fill layer nearest to location `t`.
fn stop_near(app: &PrismApp, t: f32) -> Option<usize> {
    let (l, frame, _) = active_gradient(app)?;
    let w = Widget::new(fill_of(&l)?, frame)?;
    (0..w.stops.len()).min_by(|a, b| (w.stops[*a].0 - t).abs().total_cmp(&(w.stops[*b].0 - t).abs()))
}

/// The pointer over the live gradient's widget (`point` in document pixels): the move cursor
/// on a handle, stop or midpoint and while dragging one, a plus over the line (a click adds a
/// stop there). `None` elsewhere.
pub(crate) fn cursor(app: &PrismApp, point: [f64; 2]) -> Option<egui::CursorIcon> {
    if !live_mode(app) {
        return None;
    }
    match &app.gradient.drag {
        Some(Drag::Edit { .. }) => return Some(egui::CursorIcon::Move),
        Some(Drag::Draw { .. }) => return None,
        None => {}
    }
    let zoom = app.current_zoom().max(0.01);
    let (l, frame, _) = active_gradient(app)?;
    let hit = Widget::new(fill_of(&l)?, frame)?.hit([point[0] as f32, point[1] as f32], GRAB / zoom)?;
    Some(if matches!(hit, Grab::Line(_)) { egui::CursorIcon::Copy } else { egui::CursorIcon::Move })
}

/// Pointer input for the Gradient tool in live mode. Returns true when consumed.
pub fn pointer(app: &mut PrismApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if !live_mode(app) {
        app.gradient.drag = None;
        return false;
    }
    let zoom = app.current_zoom().max(0.01);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = [x as f32, y as f32];
            let active = active_gradient(app);
            let grab = active.as_ref().and_then(|(l, frame, _)| Widget::new(fill_of(l)?, *frame)?.hit(p, GRAB / zoom));
            app.gradient.drag = Some(match (grab, active) {
                (Some(grab), Some((l, ..))) => {
                    // A handle, stop or midpoint becomes the selected control (the line adds a
                    // stop on release, which is selected then).
                    if !matches!(grab, Grab::Line(_)) {
                        app.gradient.selected = Some((l.id, grab));
                    }
                    Drag::Edit { layer: l.id, grab, down: p, moved: false, commit: None }
                }
                (_, active) => {
                    app.gradient.selected = None;
                    Drag::Draw { from: p, to: p, redraw: active.map(|a| a.0.id) }
                }
            });
        }
        ToolEvent::Move { x, y, .. } => drag_to(app, [x as f32, y as f32], mods),
        ToolEvent::Up { x, y } => {
            drag_to(app, [x as f32, y as f32], mods);
            finish(app);
        }
    }
    true
}

fn drag_to(app: &mut PrismApp, p: [f32; 2], mods: egui::Modifiers) {
    let zoom = app.current_zoom().max(0.01);
    let active = active_gradient(app);
    let Some(drag) = app.gradient.drag.as_mut() else { return };
    // ⇧: 45° gradient angles, the same snap as the classic drag (stroke_constraint.rs).
    let snap = |from: [f32; 2], to: [f32; 2]| -> [f32; 2] {
        if !mods.shift {
            return to;
        }
        let c = crate::stroke_constraint::snap45([f64::from(from[0]), f64::from(from[1])], [f64::from(to[0]), f64::from(to[1])]);
        [c[0] as f32, c[1] as f32]
    };
    match drag {
        Drag::Draw { from, to, .. } => *to = snap(*from, p),
        Drag::Edit { layer, grab, down, moved, commit } => {
            if dist(*down, p) * zoom >= 2.0 {
                *moved = true;
            }
            if !*moved {
                return;
            }
            let Some(w) = active.as_ref().filter(|a| a.0.id == *layer).and_then(|(l, frame, _)| Widget::new(fill_of(l)?, *frame)) else { return };
            let id = layer.0;
            *commit = match *grab {
                Grab::Start => Some((cmds::SET, json!({"layer": id, "from": snap(w.end, p)}))),
                Grab::End => Some((cmds::SET, json!({"layer": id, "to": snap(w.start, p)}))),
                Grab::Line(_) => {
                    let (dx, dy) = (p[0] - down[0], p[1] - down[1]);
                    Some((cmds::SET, json!({"layer": id, "from": [w.start[0] + dx, w.start[1] + dy], "to": [w.end[0] + dx, w.end[1] + dy]})))
                }
                Grab::Stop(i) => {
                    let (u, d) = project(w.start, w.end, p);
                    if d * zoom > TEAR_OFF && w.stops.len() > 2 {
                        Some((cmds::STOP, json!({"layer": id, "action": "delete", "index": i})))
                    } else {
                        Some((cmds::STOP, json!({"layer": id, "action": "move", "index": i, "location": w.u_of(u)})))
                    }
                }
                Grab::Mid(i) => {
                    let t = w.u_of(project(w.start, w.end, p).0);
                    match (w.stops.get(i), w.stops.get(i + 1)) {
                        (Some(a), Some(b)) if b.0 - a.0 > 1e-4 => {
                            Some((cmds::STOP, json!({"layer": id, "action": "midpoint", "index": i, "location": ((t - a.0) / (b.0 - a.0)).clamp(0.05, 0.95)})))
                        }
                        _ => None,
                    }
                }
            };
        }
    }
}

fn finish(app: &mut PrismApp) {
    let Some(drag) = app.gradient.drag.take() else { return };
    let zoom = app.current_zoom().max(0.01);
    match drag {
        Drag::Draw { from, to, redraw } => {
            if dist(from, to) * zoom < 2.0 {
                return;
            }
            let _ = match redraw {
                Some(id) => app.run(cmds::SET, json!({"layer": id.0, "from": from, "to": to})),
                None => {
                    let p = create_params(app, from, to);
                    app.run(cmds::CREATE, p)
                }
            };
        }
        Drag::Edit { layer, commit: Some((cmd, p)), moved: true, .. } => {
            let ok = app.run(cmd, p.clone()).is_ok();
            // A stop dragged past another changes its index: keep the one dragged selected; one
            // torn off is gone.
            match p["action"].as_str() {
                Some("move") if ok => {
                    let t = p["location"].as_f64().map(|t| t as f32);
                    app.gradient.selected = t.and_then(|t| stop_near(app, t)).map(|i| (layer, Grab::Stop(i)));
                }
                Some("delete") => app.gradient.selected = None,
                _ => {}
            }
        }
        Drag::Edit { layer, grab, moved: false, .. } => click(app, layer, grab),
        Drag::Edit { .. } => {}
    }
}

/// A click (no drag) on the widget: the line adds a stop; a double-click on a stop (or on an end
/// handle, for the stop there) opens the Color Picker.
fn click(app: &mut PrismApp, layer: LayerId, grab: Grab) {
    let now = app.last_frame_time;
    let double = app.gradient.last_click.is_some_and(|(g, t)| g == grab && now - t <= DOUBLE_CLICK);
    app.gradient.last_click = Some((grab, now));
    match grab {
        Grab::Line(t) => {
            // The new stop is the selected one, as in Photoshop.
            if app.run(cmds::STOP, json!({"layer": layer.0, "action": "add", "location": t})).is_ok() {
                app.gradient.selected = stop_near(app, t).map(|i| (layer, Grab::Stop(i)));
            }
        }
        Grab::Stop(i) if double => edit_stop_color(app, layer, i),
        Grab::Start | Grab::End if double => {
            let Some((l, frame, _)) = active_gradient(app) else { return };
            let Some(w) = fill_of(&l).and_then(|f| Widget::new(f, frame)) else { return };
            // The stop at that end of the line.
            let at_start = (grab == Grab::Start) != w.reverse;
            let i = if at_start { 0 } else { w.stops.len().saturating_sub(1) };
            edit_stop_color(app, layer, i);
        }
        _ => {}
    }
}

/// Opens the Color Picker on colour stop `i`; OK runs `gradient.fill.stop` (one history step).
pub fn edit_stop_color(app: &mut PrismApp, layer: LayerId, i: usize) {
    let Some(st) = app.session.active() else { return };
    let Some(Fill::Gradient { stops, .. }) = st.doc.layer(layer).and_then(fill_of) else { return };
    let mut sorted = stops.clone();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some((_, c)) = sorted.get(i) else { return };
    app.gradient.last_click = None;
    crate::color_picker_ui::open_for_command(
        app,
        "Color Picker (Stop Color)",
        c.to_rgb(),
        cmds::STOP,
        json!({"layer": layer.0, "action": "color", "index": i}),
    );
}

fn preview_key(revision: u64, what: &str) -> u64 {
    let h = what.bytes().fold(revision.wrapping_mul(0x9e37_79b9_7f4a_7c15), |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
    (h & (TAG - 1)) | TAG
}

/// The document to show while a gradient is drawn or edited (`canvas::display_doc`).
pub fn display_doc(app: &mut PrismApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let st = app.session.active()?;
    let (doc, revision) = (st.doc.clone(), st.revision);
    let edit: Option<(LayerId, &'static str, Value)> = match &app.gradient.drag {
        Some(Drag::Draw { from, to, redraw }) if live_mode(app) && dist(*from, *to) >= 1.0 => match redraw {
            Some(id) => Some((*id, cmds::SET, json!({"from": from, "to": to}))),
            None => Some((PREVIEW_LAYER, cmds::CREATE, create_params(app, *from, *to))),
        },
        Some(Drag::Edit { layer, commit: Some((cmd, p)), moved: true, .. }) if live_mode(app) => Some((*layer, *cmd, p.clone())),
        _ => app.gradient.panel.clone(),
    };
    let Some((layer, cmd, p)) = edit else {
        app.gradient.preview = None;
        return None;
    };
    let key = preview_key(revision, &format!("{}{cmd}{p}", layer.0));
    if let Some((k, d)) = &app.gradient.preview
        && *k == key
    {
        return Some((d.clone(), key));
    }
    let mut shown = (*doc).clone();
    if cmd == cmds::CREATE {
        let mut l = cmds::new_layer(&app.session, &doc, &p).ok()?;
        l.id = PREVIEW_LAYER;
        let above = app.session.active().and_then(|s| s.active_layer);
        shown.insert_above(above, l);
    } else {
        let l = doc.layer(layer)?;
        let f = edited_fill(app, l, doc.bounds(), cmd, &p)?;
        shown.layer_mut(layer)?.content = LayerContent::Fill(f);
    }
    let shown = Arc::new(shown);
    app.gradient.preview = Some((key, shown.clone()));
    Some((shown, key))
}

/// The fill and frame the widget shows: the one being drawn or edited, else the selected one.
fn shown_fill(app: &PrismApp) -> Option<(Fill, Rect32, Option<Grab>)> {
    let st = app.session.active()?;
    let canvas = st.doc.bounds();
    match &app.gradient.drag {
        Some(Drag::Draw { from, to, redraw: None }) => {
            let l = cmds::new_layer(&app.session, &st.doc, &create_params(app, *from, *to)).ok()?;
            let f = fill_of(&l)?.clone();
            Some((f, prism_compose::fill_frame(&l, canvas), Some(Grab::End)))
        }
        Some(Drag::Draw { from, to, redraw: Some(id) }) => {
            let l = st.doc.layer(*id)?;
            let f = edited_fill(app, l, canvas, cmds::SET, &json!({"from": from, "to": to}))?;
            Some((f, prism_compose::fill_frame(l, canvas), Some(Grab::End)))
        }
        Some(Drag::Edit { layer, grab, commit, .. }) => {
            let l = st.doc.layer(*layer)?;
            let f = match commit {
                Some((cmd, p)) => match edited_fill(app, l, canvas, cmd, p) {
                    Some(f) => f,
                    None => fill_of(l)?.clone(),
                },
                None => fill_of(l)?.clone(),
            };
            // A torn-off stop no longer exists: highlight nothing.
            let deleting = commit.as_ref().is_some_and(|(_, p)| p["action"] == "delete");
            Some((f, prism_compose::fill_frame(l, canvas), (!deleting).then_some(*grab)))
        }
        None => {
            let (l, frame, _) = active_gradient(app)?;
            Some((fill_of(&l)?.clone(), frame, None))
        }
    }
}

fn c32(c: [f32; 4]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

fn line(painter: &egui::Painter, a: Pos2, b: Pos2) {
    painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(90)));
    painter.line_segment([a, b], Stroke::new(1.5, Color32::WHITE));
}

fn diamond(painter: &egui::Painter, c: Pos2, r: f32, fill: Color32, outline: Color32) {
    let pts = vec![pos2(c.x, c.y - r), pos2(c.x + r, c.y), pos2(c.x, c.y + r), pos2(c.x - r, c.y)];
    painter.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.0, outline)));
}

/// Draws the live gradient widget over the canvas.
pub fn draw_overlay(app: &PrismApp, painter: &egui::Painter, xf: &ViewXform) {
    if !live_mode(app) {
        return;
    }
    let Some((fill, frame, grabbed)) = shown_fill(app) else { return };
    let Some(w) = Widget::new(&fill, frame) else { return };
    // What a drag holds, else the control last clicked.
    let grabbed = match &app.gradient.drag {
        None => selected(app),
        Some(_) => grabbed,
    };
    let accent = Tokens::get(painter.ctx()).accent;
    // The selected control's ring: an accent outline round it, over a soft shadow.
    let ring = |p: Pos2, r: f32| {
        painter.circle_stroke(p, r, Stroke::new(3.5, Color32::from_black_alpha(90)));
        painter.circle_stroke(p, r, Stroke::new(1.5, accent));
    };
    let sp = |p: [f32; 2]| xf.to_screen(p[0], p[1]);
    let (s, e) = (sp(w.start), sp(w.end));
    let r = s.distance(e);
    let shadow = Stroke::new(3.0, Color32::from_black_alpha(90));
    let white = Stroke::new(1.5, Color32::WHITE);
    match w.style {
        GradientStyle::Radial | GradientStyle::Angle => {
            painter.circle_stroke(s, r, shadow);
            painter.circle_stroke(s, r, white);
        }
        GradientStyle::Diamond => {
            let d = e - s;
            let n = vec2(-d.y, d.x);
            let pts = vec![s + d, s + n, s - d, s - n, s + d];
            painter.add(egui::Shape::line(pts.clone(), shadow));
            painter.add(egui::Shape::line(pts, white));
        }
        GradientStyle::Reflected => {
            let m = s - (e - s);
            painter.add(egui::Shape::dashed_line(&[s, m], Stroke::new(1.5, Color32::WHITE), 5.0, 4.0));
        }
        GradientStyle::Linear => {}
    }
    line(painter, s, e);
    // Midpoints.
    for i in 0..w.stops.len().saturating_sub(1) {
        if let Some(t) = w.mid_location(i) {
            let sel = grabbed == Some(Grab::Mid(i));
            diamond(painter, sp(w.at(t)), 4.5, if sel { accent } else { Color32::WHITE }, Color32::from_black_alpha(160));
            if sel {
                ring(sp(w.at(t)), 8.0);
            }
        }
    }
    // End handles, then the colour stops (a stop at an end sits inside its handle).
    for (p, g) in [(s, Grab::Start), (e, Grab::End)] {
        let sel = grabbed == Some(g);
        painter.circle_stroke(p, 8.0, Stroke::new(3.5, Color32::from_black_alpha(90)));
        painter.circle_stroke(p, 8.0, Stroke::new(2.0, if sel { accent } else { Color32::WHITE }));
        if sel {
            ring(p, 12.0);
        }
    }
    for (i, (t, c)) in w.stops.iter().enumerate() {
        let p = sp(w.at(*t));
        let sel = grabbed == Some(Grab::Stop(i));
        painter.circle_filled(p, 5.0, c32(*c));
        painter.circle_stroke(p, 5.5, Stroke::new(1.5, if sel { accent } else { Color32::WHITE }));
        painter.circle_stroke(p, 6.8, Stroke::new(1.0, Color32::from_black_alpha(110)));
        if sel {
            ring(p, 9.5);
        }
    }
}

// ---------------------------------------------------------------- options bar

/// A ramp of RGBA stops over a checkerboard in `r`.
fn paint_ramp(p: &egui::Painter, r: Rect, sample: impl Fn(f32) -> [f32; 4]) {
    widgets::checker(p, r, 5.0);
    let n = 64;
    let mut mesh = egui::Mesh::default();
    for k in 0..=n {
        let u = k as f32 / n as f32;
        let c = sample(u);
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let col = Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]));
        let x = r.left() + u * r.width();
        mesh.colored_vertex(pos2(x, r.top()), col);
        mesh.colored_vertex(pos2(x, r.bottom()), col);
        if k > 0 {
            let i = (2 * k) as u32;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i + 1, i);
        }
    }
    p.add(mesh);
}

/// Side, in pixels, of a gradient fill layer's thumbnail texture.
const THUMB_PX: u32 = 64;

/// Pixels of a gradient fill's Layers-panel thumbnail: the fill as it lays out (style, angle,
/// scale, offset, reverse, stops, midpoints and opacity) in a square, as Photoshop shows it.
/// `None` for other fills.
pub fn thumbnail_image(f: &Fill) -> Option<egui::ColorImage> {
    let Fill::Gradient { .. } = f else { return None };
    let n = THUMB_PX as i32;
    let square = prism_geom::Rect::new(0, 0, n, n);
    // No dither: it is invisible at this size and would only add noise.
    let mut plain = f.clone();
    if let Fill::Gradient { dither, .. } = &mut plain {
        *dither = false;
    }
    let px = gf::render(&plain, square, square);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgba: Vec<Color32> = px.iter().map(|c| Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))).collect();
    Some(egui::ColorImage::new([THUMB_PX as usize; 2], rgba))
}

/// Paints a gradient fill layer's thumbnail into `rect` over a checkerboard (opacity stops show
/// through), cached per layer until its fill changes. Returns `false` for other fills.
pub fn paint_thumbnail(ui: &egui::Ui, layer: LayerId, f: &Fill, rect: Rect) -> bool {
    if !matches!(f, Fill::Gradient { .. }) {
        return false;
    }
    let ctx = ui.ctx();
    let key = egui::Id::new(("gradient-thumb", layer.0));
    let cached: Option<(Fill, egui::TextureHandle)> = ctx.data(|d| d.get_temp(key));
    let tex = match cached {
        Some((cf, t)) if cf == *f => t,
        _ => {
            let Some(img) = thumbnail_image(f) else { return false };
            let t = ctx.load_texture(format!("gradient-thumb-{}", layer.0), img, egui::TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(key, (f.clone(), t.clone())));
            t
        }
    };
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        widgets::checker(p, rect, 5.0);
        p.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    }
    true
}

/// Height of the gradient popup around its list: the current-gradient bar, the footer and margins.
const POPUP_CHROME: f32 = 90.0;

/// Options-bar swatch of the current gradient. Both live and classic modes use the same
/// engine-owned preset, so its popup reuses the Gradients browser instead of keeping an
/// independent copy of the chosen gradient in the UI (#1651).
pub fn preset_swatch(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let stops = app.session.presets.gradient.resolve(app.session.tools.foreground, app.session.tools.background);
    let (r, resp) = ui.allocate_exact_size(vec2(96.0, 20.0), Sense::click());
    paint_ramp(ui.painter(), r, |u| prism_algo::paint::sample_stops(&stops, u));
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    let resp = resp.on_hover_text(tl!("The current gradient (pick one in Window › Gradients)"));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Gradients")));
    // The list's height comes from the room under the swatch, which stays put while the popup
    // is open (the popup's own available height follows its size and would feed back into it).
    let room = ui.ctx().content_rect().bottom() - resp.rect.bottom() - POPUP_CHROME;
    let max_h = room.clamp(90.0, 320.0);
    // Open state of its own rather than egui's one-popup-at-a-time memory, which a preset's
    // context menu (Rename…, Delete, Move to) would take over, closing the picker and the menu
    // with it.
    // It stays open only while drawn every pass: picking another tool hides the swatch, which
    // closes the picker rather than leaving it to reappear with the Gradient tool.
    let open_id = resp.id.with("gradient-picker");
    let pass = ui.ctx().cumulative_pass_nr();
    let shown_at = ui.data(|d| d.get_temp::<u64>(open_id));
    let was_open = shown_at.is_some_and(|p| p.saturating_add(1) >= pass);
    let mut open = was_open ^ resp.clicked();
    // Clicks inside (a group's header, the size slider, a preset, the footer buttons) keep it
    // open, as in Photoshop; so do clicks in a context menu opened from it.
    let close = if egui::Popup::is_any_open(ui.ctx()) { egui::PopupCloseBehavior::IgnoreClicks } else { egui::PopupCloseBehavior::CloseOnClickOutside };
    let chosen = egui::Popup::menu(&resp).open_bool(&mut open).close_behavior(close).show(|ui| {
        ui.set_min_width(260.0);
        ui.set_max_width(330.0);
        crate::preset_panels::gradients_browser(app, ui, crate::preset_panels::GradientsHost::Picker { max_h })
    });
    // A double-clicked preset is chosen: the picker closes.
    if chosen.is_some_and(|r| r.inner) {
        open = false;
    }
    if shown_at.is_some() && !open {
        // A name being typed in the picker goes with it, rather than showing up when it reopens.
        if app.ui.presets_ui.renaming.as_ref().is_some_and(|r| r.0 == "gradients") {
            app.ui.presets_ui.renaming = None;
        }
    }
    ui.data_mut(|d| {
        if open {
            d.insert_temp(open_id, pass);
        } else {
            d.remove::<u64>(open_id);
        }
    });
}

/// Live mode: style, reverse, dither, blend mode and opacity changes in the options bar also edit the selected
/// gradient fill layer (one history step each), as in Photoshop.
pub fn options_changed(app: &mut PrismApp, before: &crate::state::ToolOptions) {
    let o = app.ui.tool_options.clone();
    if app.ui.tool != Tool::Gradient || o.gradient_classic {
        return;
    }
    let Some((l, ..)) = active_gradient(app) else { return };
    if o.gradient_blend_mode != before.gradient_blend_mode {
        let _ = app.run("layer.setProps", json!({"layer": l.id.0, "blend": o.gradient_blend_mode.label()}));
    }
    // Opacity too, as a new live gradient takes it (a scrub of the field is one history step).
    if o.fill_opacity != before.fill_opacity {
        let p = json!({"layer": l.id.0, "opacity": (o.fill_opacity / 100.0).clamp(0.0, 1.0), "coalesce": format!("gradient-tool-opacity-{}", l.id.0)});
        let _ = app.run("layer.setProps", p);
    }
    if o.gradient_style == before.gradient_style && o.gradient_reverse == before.gradient_reverse && o.gradient_dither == before.gradient_dither {
        return;
    }
    let mut p = json!({"layer": l.id.0});
    if o.gradient_style != before.gradient_style {
        p["style"] = json!(o.gradient_style);
    }
    if o.gradient_reverse != before.gradient_reverse {
        p["reverse"] = json!(o.gradient_reverse);
    }
    if o.gradient_dither != before.gradient_dither {
        p["dither"] = json!(o.gradient_dither);
    }
    let _ = app.run(cmds::SET, p);
}

// ---------------------------------------------------------------- Properties panel

/// Width of the Properties label column ("Style", "Angle", "Scale").
const PROP_LABEL_W: f32 = 38.0;

/// Commits a numeric field once (drag released, Enter, focus lost); `None` while editing.
fn field(ui: &mut egui::Ui, id: &str, current: f32, range: std::ops::RangeInclusive<f32>, suffix: &str, width: f32) -> Option<f32> {
    let key = egui::Id::new(("gradient-fill-field", id));
    let mut v = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(current);
    let resp = widgets::value_field(ui, &mut v, range, suffix, width);
    if resp.dragged() || resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, v));
        return None;
    }
    ui.data_mut(|d| d.remove::<f32>(key));
    (resp.drag_stopped() || resp.lost_focus() || resp.changed()).then_some(v).filter(|v| (*v - current).abs() > 1e-3)
}

fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(PROP_LABEL_W, 22.0), Sense::hover());
    ui.painter().text(pos2(r.left(), r.center().y), egui::Align2::LEFT_CENTER, tl!(text), egui::FontId::proportional(12.0), t.text_dim);
}

/// Properties panel sections for a gradient fill layer (`props_layout::section` headers):
/// "Gradient" (the stops editor) and "Gradient Options" (style, angle, scale, reverse, dither,
/// "Align with layer", Reset Alignment). Every change is one `gradient.fill.*` command.
pub fn properties(app: &mut PrismApp, ui: &mut egui::Ui, layer: &Layer) {
    use crate::props_layout::{COL_GAP, LABEL_GAP, field_width, section};
    let Some(Fill::Gradient { angle, scale, style, reverse, dither, align, .. }) = fill_of(layer).cloned() else { return };
    let id = layer.id.0;
    let mut runs: Vec<Value> = Vec::new();
    if section(ui, "gradient-fill", "Gradient") {
        stops_editor(app, ui, layer);
        ui.add_space(crate::theme::ROW_GAP);
    }
    if section(ui, "gradient-fill-options", "Gradient Options") {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = LABEL_GAP;
            label(ui, "Style");
            let mut s = cmds::style_name(style).to_string();
            let opts: Vec<(String, &str)> =
                [("linear", "Linear"), ("radial", "Radial"), ("angle", "Angle"), ("reflected", "Reflected"), ("diamond", "Diamond")]
                    .iter()
                    .map(|(k, l)| (k.to_string(), *l))
                    .collect();
            // Full width, lined up with the fields and Reset Alignment below.
            let w = ui.available_width().max(60.0);
            if widgets::dropdown(ui, "gradient-fill-style", &mut s, &opts, w) {
                runs.push(json!({"style": s}));
            }
        });
        ui.add_space(crate::theme::ROW_GAP);
        // Two label+field columns filling the panel.
        let w = field_width(ui.available_width(), 2, PROP_LABEL_W);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = LABEL_GAP;
            label(ui, "Angle");
            if let Some(v) = field(ui, &format!("angle{id}"), angle, -180.0..=180.0, "°", w) {
                runs.push(json!({"angle": v}));
            }
            ui.add_space(COL_GAP - LABEL_GAP);
            label(ui, "Scale");
            if let Some(v) = field(ui, &format!("scale{id}"), scale * 100.0, 10.0..=150.0, "%", w) {
                runs.push(json!({"scale": v}));
            }
        });
        ui.add_space(crate::theme::ROW_GAP);
        ui.horizontal_wrapped(|ui| {
            let mut r = reverse;
            if widgets::checkbox(ui, &mut r, "Reverse").changed() {
                runs.push(json!({"reverse": r}));
            }
            let mut d = dither;
            if widgets::checkbox(ui, &mut d, "Dither").changed() {
                runs.push(json!({"dither": d}));
            }
            let mut a = align;
            if widgets::checkbox(ui, &mut a, "Align with layer").changed() {
                runs.push(json!({"align": a}));
            }
        });
        ui.add_space(crate::theme::ROW_GAP);
        if widgets::secondary_button(ui, "Reset Alignment", ui.available_width()).on_hover_text(tl!("Centre the gradient (offset 0, 0)")).clicked() {
            runs.push(json!({"offset": [0, 0]}));
        }
        ui.add_space(crate::theme::ROW_GAP);
    }
    for mut p in runs {
        p["layer"] = json!(id);
        let _ = app.run(cmds::SET, p);
    }
}

/// Which marker of the stops editor is being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Marker {
    Color(usize),
    Opacity(usize),
    Mid(usize),
}

/// Photoshop's Gradient Editor strip: opacity stops above the ramp, colour stops and midpoints
/// below. Drag a stop to move it (off the strip to delete it), click above / below to add one,
/// double-click a colour stop for the Color Picker.
fn stops_editor(app: &mut PrismApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(f) = fill_of(layer).cloned() else { return };
    let Fill::Gradient { stops, .. } = &f else { return };
    let t = Tokens::get(ui.ctx());
    let id = layer.id;
    let w = ui.available_width().max(120.0);
    let (outer, resp) = ui.allocate_exact_size(vec2(w, 58.0), Sense::click_and_drag());
    let bar = Rect::from_min_max(pos2(outer.left() + 8.0, outer.top() + 16.0), pos2(outer.right() - 8.0, outer.top() + 40.0));
    let x_of = |loc: f32| bar.left() + loc.clamp(0.0, 1.0) * bar.width();
    let loc_of = |x: f32| ((x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0);
    // The ramp being shown (with a pending drag's edit applied).
    let pending = app.gradient.panel.clone().filter(|(l, ..)| *l == id);
    let shown = pending.as_ref().and_then(|(_, cmd, p)| edited_fill(app, layer, Rect32::new(0, 0, 1, 1), cmd, p)).unwrap_or_else(|| f.clone());
    if let Some(ramp) = gf::Ramp::new(&shown) {
        paint_ramp(ui.painter(), bar, |u| ramp.sample(u));
    }
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    let (ss, os, mids) = match &shown {
        Fill::Gradient { stops, opacity_stops, midpoints, .. } => (stops.clone(), opacity_stops.clone(), midpoints.clone()),
        _ => (Vec::new(), Vec::new(), Vec::new()),
    };
    let mut sorted = ss.clone();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let ops: Vec<(f32, f32)> = if os.is_empty() { vec![(0.0, 1.0), (1.0, 1.0)] } else { os.clone() };
    let key = egui::Id::new(("gradient-stops-drag", id.0));
    let dragging: Option<Marker> = ui.data(|d| d.get_temp(key));
    // Markers.
    let p = ui.painter();
    for (i, (loc, c)) in sorted.iter().enumerate() {
        let x = x_of(*loc);
        let sel = dragging == Some(Marker::Color(i));
        let tip = pos2(x, bar.bottom() + 1.0);
        let body = Rect::from_center_size(pos2(x, bar.bottom() + 10.0), vec2(10.0, 10.0));
        p.add(egui::Shape::convex_polygon(
            vec![tip, pos2(x + 5.0, body.top()), pos2(x - 5.0, body.top())],
            if sel { t.accent } else { t.text_dim },
            Stroke::NONE,
        ));
        p.rect_filled(
            body,
            1.0,
            c32({
                let r = c.to_rgb();
                [r[0], r[1], r[2], 1.0]
            }),
        );
        p.rect_stroke(body, 1.0, Stroke::new(1.0, if sel { t.accent } else { t.text_dim }), StrokeKind::Outside);
    }
    for i in 0..sorted.len().saturating_sub(1) {
        let (a, b) = (sorted[i].0, sorted[i + 1].0);
        if b - a > 1e-3 {
            let m = mids.get(i).copied().unwrap_or(0.5);
            let sel = dragging == Some(Marker::Mid(i));
            diamond(p, pos2(x_of(a + (b - a) * m), bar.bottom() + 6.0), 3.0, if sel { t.accent } else { t.text_faint }, t.text_faint);
        }
    }
    for (i, (loc, a)) in ops.iter().enumerate() {
        let x = x_of(*loc);
        let sel = dragging == Some(Marker::Opacity(i));
        let body = Rect::from_center_size(pos2(x, bar.top() - 10.0), vec2(10.0, 10.0));
        p.add(egui::Shape::convex_polygon(
            vec![pos2(x, bar.top() - 1.0), pos2(x + 5.0, body.bottom()), pos2(x - 5.0, body.bottom())],
            if sel { t.accent } else { t.text_dim },
            Stroke::NONE,
        ));
        p.rect_filled(body, 1.0, Color32::from_gray((a.clamp(0.0, 1.0) * 255.0) as u8));
        p.rect_stroke(body, 1.0, Stroke::new(1.0, if sel { t.accent } else { t.text_dim }), StrokeKind::Outside);
    }
    // Interaction.
    let hit = |pos: Pos2| -> Option<Marker> {
        if pos.y > bar.bottom() {
            if let Some(i) = sorted.iter().position(|(l, _)| (x_of(*l) - pos.x).abs() <= 6.0 && pos.y >= bar.bottom() + 3.0) {
                return Some(Marker::Color(i));
            }
            return (0..sorted.len().saturating_sub(1))
                .find(|i| {
                    let (a, b) = (sorted[*i].0, sorted[*i + 1].0);
                    (x_of(a + (b - a) * mids.get(*i).copied().unwrap_or(0.5)) - pos.x).abs() <= 4.0 && b - a > 1e-3
                })
                .map(Marker::Mid);
        }
        if pos.y < bar.top() {
            return ops.iter().position(|(l, _)| (x_of(*l) - pos.x).abs() <= 6.0).map(Marker::Opacity);
        }
        None
    };
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(m) = hit(pos)
    {
        ui.data_mut(|d| d.insert_temp(key, m));
    }
    let dragging: Option<Marker> = ui.data(|d| d.get_temp(key));
    if let (Some(m), Some(pos)) = (dragging, resp.interact_pointer_pos()) {
        let off = if pos.y > bar.bottom() + 28.0 || pos.y < bar.top() - 26.0 { 1.0 } else { 0.0 };
        let loc = loc_of(pos.x);
        let edit = match m {
            Marker::Color(i) if off > 0.0 && stops.len() > 2 => json!({"action": "delete", "index": i}),
            Marker::Color(i) => json!({"action": "move", "index": i, "location": loc}),
            Marker::Opacity(i) if off > 0.0 && ops.len() > 2 => json!({"action": "delete", "kind": "opacity", "index": i}),
            Marker::Opacity(i) => json!({"action": "move", "kind": "opacity", "index": i, "location": loc}),
            Marker::Mid(i) => {
                let (a, b) = (sorted.get(i).map_or(0.0, |s| s.0), sorted.get(i + 1).map_or(1.0, |s| s.0));
                json!({"action": "midpoint", "index": i, "location": ((loc - a) / (b - a).max(1e-4)).clamp(0.05, 0.95)})
            }
        };
        let mut edit = edit;
        edit["layer"] = json!(id.0);
        if resp.dragged() {
            app.gradient.panel = Some((id, cmds::STOP, edit));
        }
    }
    if resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<Marker>(key));
        if let Some((_, cmd, p)) = app.gradient.panel.take() {
            let _ = app.run(cmd, p);
        }
        return;
    }
    if resp.double_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(Marker::Color(i)) = hit(pos)
    {
        edit_stop_color(app, id, i);
        return;
    }
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && hit(pos).is_none()
    {
        let loc = loc_of(pos.x);
        let p = if pos.y < bar.center().y {
            json!({"layer": id.0, "action": "add", "kind": "opacity", "location": loc})
        } else {
            json!({"layer": id.0, "action": "add", "location": loc})
        };
        let _ = app.run(cmds::STOP, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_gradient(style: &str) -> PrismApp {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app.ui.tool = Tool::Gradient;
        app.ui.tool_options.gradient_style = style.into();
        app
    }

    fn mods() -> egui::Modifiers {
        egui::Modifiers::NONE
    }

    fn drag(app: &mut PrismApp, a: [f64; 2], b: [f64; 2]) {
        crate::canvas::tool_event(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, mods());
        crate::canvas::tool_event(app, ToolEvent::Move { x: (a[0] + b[0]) / 2.0, y: (a[1] + b[1]) / 2.0, pressure: 1.0 }, mods());
        crate::canvas::tool_event(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, mods());
        crate::canvas::tool_event(app, ToolEvent::Up { x: b[0], y: b[1] }, mods());
    }

    fn get(app: &mut PrismApp) -> Value {
        app.run(cmds::GET, json!({})).unwrap()
    }

    #[test]
    fn thumbnails_show_the_gradient() {
        use prism_color::Color;
        // Black to white, left to right: the left edge is dark, the right light, top = bottom.
        let f = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Linear, false);
        let img = thumbnail_image(&f).unwrap();
        let n = img.size[0];
        let at = |x: usize, y: usize| img.pixels[y * n + x];
        assert!(at(0, n / 2).r() < 10 && at(n - 1, n / 2).r() > 245, "{:?} {:?}", at(0, n / 2), at(n - 1, n / 2));
        assert_eq!(at(n / 3, 0), at(n / 3, n - 1));
        // Reverse flips colour and opacity alike; a transparent stop lets the checkerboard through.
        let mut r = f.clone();
        if let Fill::Gradient { reverse, opacity_stops, .. } = &mut r {
            *reverse = true;
            *opacity_stops = vec![(0.0, 1.0), (1.0, 0.0)];
        }
        let img = thumbnail_image(&r).unwrap();
        let (left, right) = (img.pixels[n / 2 * n], img.pixels[n / 2 * n + n - 1]);
        assert!(left.a() < 10 && right.r() < 10 && right.a() > 245, "{left:?} {right:?}");
        // A radial thumbnail is centred.
        let rad = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Radial, false);
        let img = thumbnail_image(&rad).unwrap();
        assert!(img.pixels[n / 2 * n + n / 2].r() < img.pixels[0].r());
        assert!(thumbnail_image(&Fill::Solid(Color::WHITE)).is_none());
    }

    #[test]
    fn options_bar_swatch_opens_the_real_preset_browser_and_selects_presets() {
        use egui_kittest::{Harness, kittest::Queryable};
        let app = app_with_gradient("linear");
        let mut h = Harness::builder().with_size(vec2(440.0, 560.0)).build_ui_state(|ui, app: &mut PrismApp| preset_swatch(app, ui), app);
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(2);
        let original = h.state().session.presets.gradient.name.clone();
        h.get_by_label("Gradients").click();
        h.run_steps(2);
        // The popup uses the existing presets UI, not a detached swatch or an editor-only copy.
        h.get_by_label("Foreground to Transparent").click();
        h.run_steps(2);
        assert_eq!(h.state().session.presets.gradient.name, "Foreground to Transparent");
        assert_ne!(h.state().session.presets.gradient.name, original);
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1, "choosing a tool preset does not edit document layers");
    }

    #[test]
    fn live_drag_creates_a_fill_layer_and_handles_edit_it() {
        let mut app = app_with_gradient("linear");
        drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 2, "a Gradient Fill layer");
        let g = get(&mut app);
        assert!((g["to"][0].as_f64().unwrap() - 180.0).abs() < 1e-3);
        // Drag the end handle: one history step.
        let before = app.session.active().unwrap().history.past_len();
        drag(&mut app, [180.0, 60.0], [150.0, 100.0]);
        let g = get(&mut app);
        assert!((g["to"][0].as_f64().unwrap() - 150.0).abs() < 1e-3 && (g["to"][1].as_f64().unwrap() - 100.0).abs() < 1e-3, "{g}");
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
        // Click on the line adds a stop; dragging it off the line deletes it.
        let (s, e) = ([20.0, 60.0], [150.0, 100.0]);
        // A third of the way (the midpoint diamond sits half-way).
        let mid = [s[0] + (e[0] - s[0]) / 3.0, s[1] + (e[1] - s[1]) / 3.0];
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: mid[0], y: mid[1], pressure: 1.0 }, mods());
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: mid[0], y: mid[1] }, mods());
        assert_eq!(get(&mut app)["stops"].as_array().unwrap().len(), 3);
        drag(&mut app, mid, [mid[0], mid[1] - 80.0]);
        assert_eq!(get(&mut app)["stops"].as_array().unwrap().len(), 2);
        // A drag away from the widget redraws the selected gradient (no new layer).
        drag(&mut app, [100.0, 10.0], [100.0, 110.0]);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
        assert!((get(&mut app)["from"][1].as_f64().unwrap() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn shift_snaps_live_drags_to_45_degrees() {
        let mut app = app_with_gradient("linear");
        let shift = egui::Modifiers::SHIFT;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, shift);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 62.0, pressure: 1.0 }, shift);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 84.0 }, shift);
        let g = get(&mut app);
        let (fx, fy, tx, ty) = (g["from"][0].as_f64().unwrap(), g["from"][1].as_f64().unwrap(), g["to"][0].as_f64().unwrap(), g["to"][1].as_f64().unwrap());
        assert!(((tx - fx) - (ty - fy)).abs() < 1e-2 && tx > 80.0, "45°: {g}");
        // Dragging the end handle with ⇧ snaps too (horizontal here).
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: tx, y: ty, pressure: 1.0 }, shift);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 26.0 }, shift);
        let g = get(&mut app);
        assert!((g["to"][1].as_f64().unwrap() - fy).abs() < 1e-2, "horizontal: {g}");
    }

    #[test]
    fn classic_mode_paints_pixels() {
        let mut app = app_with_gradient("linear");
        app.ui.tool_options.gradient_classic = true;
        drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "no fill layer in classic mode");
    }

    #[test]
    fn classic_drag_preserves_preset_transparency_at_all_depths() {
        for depth in [8, 16, 32] {
            for reverse in [false, true] {
                let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
                app.run("file.new", json!({"width": 200, "height": 120, "depth": depth, "background": "transparent"})).unwrap();
                app.ui.tool = Tool::Gradient;
                app.ui.tool_options.gradient_classic = true;
                app.ui.tool_options.gradient_reverse = reverse;
                app.ui.tool_options.gradient_dither = false;
                app.ui.tool_options.fill_opacity = 50.0;
                app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
                app.run("gradient.presets.select", json!({"preset": "Foreground to Transparent"})).unwrap();
                let before = app.session.active().unwrap().history.past_len();
                drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
                let st = app.session.active().unwrap();
                let surface = st.doc.layers[0].surface().unwrap();
                for x in [0, 100, 199] {
                    let u = ((x as f32 - 20.0) / 160.0).clamp(0.0, 1.0);
                    let expected = 0.5 * if reverse { u } else { 1.0 - u };
                    let px = surface.pixel(x, 60);
                    assert!((px[3] - expected).abs() < 0.01, "depth {depth}, reverse {reverse}, x {x}: {px:?}");
                    if expected > 0.01 {
                        assert!(px[0] > 0.99 && px[1] < 0.01 && px[2] < 0.01, "preset colour: {px:?}");
                    }
                }
                assert_eq!(st.history.past_len(), before + 1);
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(100, 60)[3], 0.0);
                app.run("gradient.presets.select", json!({"stops": [[0, "#00ff00"], [1, "#00ff00"]], "transparency": [[0, 60], [1, 20]]})).unwrap();
                drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
                let px = app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(100, 60);
                assert!((px[3] - 0.2).abs() < 0.01 && px[1] > 0.99 && px[0] < 0.01, "custom semi-transparent colour: {px:?}");
            }
        }
    }

    #[test]
    fn sidebar_transparent_preset_reaches_classic_canvas_over_existing_art() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = app_with_gradient("linear");
        app.run("paint.gradient", json!({"from": [0, 0], "to": [199, 0], "colors": ["#ff0000", "#ff0000"]})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool_options.gradient_classic = true;
        app.ui.tool_options.gradient_dither = false;
        let mut h = Harness::builder().with_size(vec2(420.0, 600.0)).build_ui_state(|ui, app| crate::preset_panels::gradients_panel(app, ui), app);
        h.get_by_label("Foreground to Transparent").click();
        h.run();
        assert_eq!(h.state().session.presets.gradient.name, "Foreground to Transparent");
        drag(h.state_mut(), [20.0, 60.0], [180.0, 60.0]);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), 2);
        let surface = st.doc.layers[1].surface().unwrap();
        assert!(surface.pixel(0, 60)[3] > 0.99);
        assert!((surface.pixel(100, 60)[3] - 0.5).abs() < 0.01);
        assert!(surface.pixel(199, 60)[3] < 0.01, "transparent end reveals existing art");
        assert_eq!(st.doc.layers[0].surface().unwrap().pixel(100, 60), vec![1.0, 0.0, 0.0, 1.0], "existing art is unchanged");
    }

    #[test]
    fn gradient_blend_mode_reaches_live_and_classic_drags() {
        use prism_color::BlendMode;
        for mode in [BlendMode::Difference, BlendMode::Multiply, BlendMode::Screen, BlendMode::Exclusion] {
            let mut live = app_with_gradient("linear");
            live.ui.tool_options.gradient_blend_mode = mode;
            drag(&mut live, [20.0, 60.0], [180.0, 60.0]);
            let layer = live.session.active().unwrap().doc.layers.last().unwrap();
            assert_eq!(layer.blend, mode, "live {mode:?}");

            let mut classic = app_with_gradient("linear");
            classic.ui.tool_options.gradient_classic = true;
            classic.ui.tool_options.gradient_blend_mode = mode;
            classic.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
            classic.session.tools.background = [1.0, 0.0, 0.0, 1.0];
            drag(&mut classic, [20.0, 60.0], [180.0, 60.0]);
            let st = classic.session.active().unwrap();
            let px = st.doc.layers[0].surface().unwrap().pixel(60, 60);
            let expected = match mode {
                BlendMode::Difference | BlendMode::Exclusion => [0.0, 1.0, 1.0],
                BlendMode::Multiply => [1.0, 0.0, 0.0],
                BlendMode::Screen => [1.0, 1.0, 1.0],
                _ => [1.0, 1.0, 1.0],
            };
            for (actual, want) in px.iter().zip(expected) {
                assert!((actual - want).abs() < 0.02, "classic {mode:?}: {px:?}");
            }
        }
    }

    #[test]
    fn changing_live_gradient_mode_updates_selected_fill_layer() {
        use prism_color::BlendMode;
        let mut app = app_with_gradient("linear");
        drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
        let before = app.ui.tool_options.clone();
        app.ui.tool_options.gradient_blend_mode = BlendMode::Difference;
        options_changed(&mut app, &before);
        let layer = app.session.active().unwrap().doc.layers.last().unwrap();
        assert_eq!(layer.blend, BlendMode::Difference);
    }

    #[test]
    fn preview_shows_the_drag_before_commit() {
        let mut app = app_with_gradient("radial");
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 60.0, pressure: 1.0 }, mods());
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 140.0, y: 60.0, pressure: 1.0 }, mods());
        let (doc, key) = display_doc(&mut app, 0).expect("a preview while drawing");
        assert!(key & TAG != 0);
        assert!(doc.layer(PREVIEW_LAYER).is_some(), "the new layer previews");
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "nothing committed yet");
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 140.0, y: 60.0 }, mods());
        assert!(display_doc(&mut app, 0).is_none());
    }

    /// Over the live gradient's widget the pointer changes, as in Photoshop: the move cursor on
    /// the end handles, stops and midpoints (and while dragging one), a plus over the line,
    /// where a click adds a stop; elsewhere and in Classic mode the tool's own cursor stays.
    #[test]
    fn the_widget_changes_the_cursor() {
        use egui::CursorIcon;
        let mut app = app_with_gradient("linear");
        drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
        assert_eq!(cursor(&app, [180.0, 61.0]), Some(CursorIcon::Move), "end handle");
        assert_eq!(cursor(&app, [21.0, 60.0]), Some(CursorIcon::Move), "start handle");
        assert_eq!(cursor(&app, [100.0, 60.0]), Some(CursorIcon::Move), "the midpoint diamond");
        assert_eq!(cursor(&app, [60.0, 62.0]), Some(CursorIcon::Copy), "the line adds a stop");
        assert_eq!(cursor(&app, [100.0, 100.0]), None, "away from the widget");
        // Mid-drag of a handle, wherever the pointer gets to.
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 180.0, y: 60.0, pressure: 1.0 }, mods());
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 100.0, pressure: 1.0 }, mods());
        assert_eq!(cursor(&app, [150.0, 100.0]), Some(CursorIcon::Move));
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 100.0 }, mods());
        app.ui.tool_options.gradient_classic = true;
        assert_eq!(cursor(&app, [150.0, 100.0]), None, "Classic mode has no widget");
    }

    /// A clicked handle, stop or midpoint stays selected, ringed, as in Photoshop: a click on the
    /// line selects the stop it adds, a stop dragged past another stays selected, and a drag off
    /// the widget or another layer drops the selection.
    #[test]
    fn a_clicked_control_stays_selected() {
        let mut app = app_with_gradient("linear");
        drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
        let click_at = |app: &mut PrismApp, p: [f64; 2]| {
            app.last_frame_time += 1.0;
            crate::canvas::tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, mods());
            crate::canvas::tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, mods());
        };
        assert_eq!(selected(&app), None, "nothing until a click");
        click_at(&mut app, [180.0, 60.0]);
        assert_eq!(selected(&app), Some(Grab::End));
        click_at(&mut app, [100.0, 60.0]);
        assert_eq!(selected(&app), Some(Grab::Mid(0)));
        // A click on the line adds a stop a quarter of the way: the stop selected is that one.
        click_at(&mut app, [60.0, 60.0]);
        assert_eq!(get(&mut app)["stops"].as_array().unwrap().len(), 3);
        assert_eq!(selected(&app), Some(Grab::Stop(1)));
        // Another at three quarters; the first one dragged past it becomes the third stop, and
        // stays the one selected.
        click_at(&mut app, [140.0, 60.0]);
        assert_eq!(selected(&app), Some(Grab::Stop(2)));
        drag(&mut app, [60.0, 60.0], [160.0, 60.0]);
        let stops = get(&mut app)["stops"].clone();
        assert_eq!(selected(&app), Some(Grab::Stop(2)), "{stops}");
        assert!((stops[2][0].as_f64().unwrap() - 0.875).abs() < 0.01, "{stops}");
        // Torn off the line, it is gone and so is the selection.
        drag(&mut app, [160.0, 60.0], [160.0, 110.0]);
        assert_eq!(get(&mut app)["stops"].as_array().unwrap().len(), 3);
        assert_eq!(selected(&app), None);
        // A drag away from the widget redraws the gradient and deselects.
        click_at(&mut app, [20.0, 60.0]);
        assert_eq!(selected(&app), Some(Grab::Start));
        drag(&mut app, [100.0, 10.0], [100.0, 110.0]);
        assert_eq!(selected(&app), None);
        // Selecting another layer: nothing of this one shows selected.
        click_at(&mut app, [100.0, 10.0]);
        assert_eq!(selected(&app), Some(Grab::Start));
        app.run("layer.new.layer", json!({})).unwrap();
        assert_eq!(selected(&app), None);
    }

    /// The gradient popup shows fully at once: egui fades popups in, which over the checkerboard
    /// looked like the gradients filling in every time it opened, so the theme turns animations
    /// off. Its first visible frame paints what it paints once settled (colours include the
    /// fade's opacity).
    #[test]
    fn the_gradient_popup_opens_without_fading_in() {
        use egui_kittest::{Harness, kittest::Queryable};
        // Frames 1/60 s apart, as on screen: the harness's default 1/4 s outlasts egui's 0.2 s fade.
        let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PrismApp| {
                ui.horizontal(|ui| preset_swatch(app, ui));
            },
            app_with_gradient("linear"),
        );
        // The harness turns animations off itself: start from egui's default, so it is the
        // theme that turns them off.
        h.ctx.all_styles_mut(|s| s.animation_time = egui::Style::default().animation_time);
        crate::PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(3);
        // The colours painted (fills, strokes, gradient vertices), which carry the fade's opacity.
        let popup = |h: &Harness<'_, PrismApp>| -> Vec<egui::Color32> {
            h.output()
                .shapes
                .iter()
                .flat_map(|s| match &s.shape {
                    egui::Shape::Rect(r) => vec![r.fill, r.stroke.color],
                    egui::Shape::Mesh(m) => m.vertices.iter().map(|v| v.color).collect(),
                    _ => Vec::new(),
                })
                .collect()
        };
        h.get_by_label("Gradients").click();
        // The first pass sizes the popup unseen; the next one is the first it is shown in.
        h.step();
        h.step();
        let first = popup(&h);
        h.run_steps(30);
        assert!(!first.is_empty());
        let settled = popup(&h);
        let diff = first.iter().zip(&settled).position(|(a, b)| a != b);
        assert!(
            diff.is_none() && first.len() == settled.len(),
            "the first visible frame paints other colours ({} vs {}) from {diff:?}: {:?} vs {:?}",
            first.len(),
            settled.len(),
            diff.map(|i| first[i]),
            diff.map(|i| settled[i])
        );
    }

    /// The options bar's gradient popup keeps its size while the pointer moves over it: its list
    /// took its height from the popup's own last size, less a margin, so every repaint shrank it
    /// until the list was at its minimum.
    #[test]
    fn the_gradient_popup_keeps_its_height_while_the_pointer_moves() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
            |ui, app: &mut PrismApp| {
                ui.horizontal(|ui| preset_swatch(app, ui));
            },
            app_with_gradient("linear"),
        );
        crate::PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(3);
        h.get_by_label("Gradients").click();
        h.run_steps(3);
        // The footer's size slider sits at the popup's bottom.
        let footer = |h: &Harness<'_, PrismApp>| h.get_by_role(egui::accesskit::Role::Slider).rect();
        let first = footer(&h);
        for p in [(100.0, 100.0), (200.0, 300.0), (50.0, 200.0), (300.0, 150.0), (120.0, 400.0), (600.0, 600.0)] {
            h.hover_at(egui::pos2(p.0, p.1));
            h.run_steps(3);
            assert_eq!(footer(&h), first, "after the pointer moved to {p:?}");
        }
        assert!(first.top() > 300.0, "the list uses the room under the swatch: {first:?}");
    }
}

#[cfg(test)]
#[path = "gradient_ui/popup_tests.rs"]
mod popup_tests;
