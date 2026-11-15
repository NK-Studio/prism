//! Editing 3D figure layers (CLIP STUDIO's 3D drawing figures) on the canvas and in Properties.
//!
//! With the Move tool on a figure layer (View › Extras on, no transform box), the figure's main
//! joints show as dots. Drag a dot: turn the bone ending there in the picture plane (⇧: twist it
//! about itself; ⌘/Ctrl on a hand or foot: reach it there with two-bone IK). Drag anywhere else:
//! orbit the camera (⇧: move the figures, ⌥: dolly). Every move runs one `layer.figure.*` command
//! with `draft: true` under one `coalesce` key, and the release re-renders at full quality into
//! the same history step, so a gesture undoes in one go and replays like any command.
use egui::{Color32, Stroke};
use prism_algo::figure3d::JointPoint;
use prism_doc::{Document, Layer, LayerId};
use serde::Serialize;
use serde_json::{Value, json};

use crate::PrismApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;
use crate::widgets;

/// Grab radius in screen points.
pub const HIT: f64 = 8.0;
/// Camera turn per screen point dragged (degrees).
const ORBIT_PER_POINT: f64 = 0.4;
/// Dolly factor per screen point dragged (vertical).
const DOLLY_PER_POINT: f64 = 0.005;
/// Twist per screen point dragged (degrees).
const TWIST_PER_POINT: f64 = 0.6;

/// One grabbable joint, in document pixels.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handle {
    pub figure: usize,
    pub joint: String,
    /// The joint whose bone ends here: what a drag turns.
    pub parent: Option<String>,
    pub x: f64,
    pub y: f64,
    /// A hand or foot: ⌘/Ctrl-drag reaches it with IK.
    pub end: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Mode {
    Orbit,
    Pan,
    Dolly,
    /// Turn `parent` about the view axis around `pivot` (document pixels).
    Turn { figure: usize, parent: String, pivot: [f64; 2] },
    Twist { figure: usize, parent: String },
    Reach { figure: usize, joint: String },
}

/// A gesture in progress.
#[derive(Clone, Debug)]
pub struct FigureDrag {
    mode: Mode,
    layer: LayerId,
    last: [f64; 2],
    key: String,
    moved: bool,
}

/// Joints that get a handle: the body's main joints, not finger segments, tips or the root.
fn shown(j: &JointPoint) -> bool {
    let n = j.name.to_ascii_lowercase();
    let finger = ["thumb", "index", "middle", "ring", "pinky"].iter().any(|f| n.contains(f));
    j.parent.is_some() && !finger && !n.contains("end") && !n.contains("root") && !n.starts_with("spine_")
}

fn is_end(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.ends_with("hand_bb_") || n.ends_with("foot_bb_")) && !n.contains("toe")
}

/// The figure layer `layer` of `doc`, if it is one.
pub fn figure_layer(doc: &Document, layer: LayerId) -> Option<&Layer> {
    doc.layer(layer).filter(|l| l.figure.is_some() && l.visible)
}

/// The handles of figure layer `layer`.
pub fn handles(doc: &Document, layer: LayerId) -> Vec<Handle> {
    if figure_layer(doc, layer).is_none() {
        return Vec::new();
    }
    prism_engine::figure_cmds::joints(doc, layer)
        .into_iter()
        .filter(shown)
        .map(|j| Handle { end: is_end(&j.name), figure: j.figure, x: j.doc[0], y: j.doc[1], parent: j.parent, joint: j.name })
        .collect()
}

/// The editor is live: Move tool, no transform box, View › Extras on, a figure layer active.
pub fn active(app: &PrismApp) -> bool {
    app.ui.tool == Tool::Move
        && app.ui.view.extras
        && !crate::transform_tool::modal(app)
        && app.session.active().and_then(|st| st.active_layer.and_then(|l| figure_layer(&st.doc, l))).is_some()
}

/// The handles shown now (empty when the editor is not live).
pub fn current(app: &PrismApp) -> Vec<Handle> {
    let Some(st) = app.session.active().filter(|_| active(app)) else { return Vec::new() };
    st.active_layer.map(|l| handles(&st.doc, l)).unwrap_or_default()
}

/// The handle within [`HIT`] screen points of `p`, nearest first.
pub fn at(handles: &[Handle], p: [f64; 2], zoom: f64) -> Option<Handle> {
    let reach = HIT / zoom.max(1e-3);
    handles
        .iter()
        .map(|h| (h, (h.x - p[0]).hypot(h.y - p[1])))
        .filter(|(_, d)| d.is_finite() && *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h.clone())
}

/// Screen-point angle (degrees, clockwise) from `pivot` to `p`.
fn angle(pivot: [f64; 2], p: [f64; 2]) -> Option<f64> {
    let (dx, dy) = (p[0] - pivot[0], p[1] - pivot[1]);
    (dx.hypot(dy) > 1e-6).then(|| dy.atan2(dx).to_degrees())
}

/// The command a gesture step runs; `None` for no change.
fn step(drag: &FigureDrag, p: [f64; 2], zoom: f64, draft: bool) -> Option<(&'static str, Value)> {
    let zoom = zoom.max(1e-3);
    let (dx, dy) = ((p[0] - drag.last[0]) * zoom, (p[1] - drag.last[1]) * zoom);
    let mut v = match &drag.mode {
        Mode::Orbit => json!({ "orbitBy": [-dx * ORBIT_PER_POINT, dy * ORBIT_PER_POINT] }),
        Mode::Pan => json!({ "panBy": [p[0] - drag.last[0], p[1] - drag.last[1]] }),
        Mode::Dolly => json!({ "dollyBy": (dy * DOLLY_PER_POINT).exp() }),
        Mode::Turn { figure, parent, pivot } => {
            let mut d = angle(*pivot, p)? - angle(*pivot, drag.last)?;
            if d > 180.0 {
                d -= 360.0;
            } else if d < -180.0 {
                d += 360.0;
            }
            json!({ "figure": figure, "joint": parent, "rotateBy": {"axis": "view", "degrees": d} })
        }
        Mode::Twist { figure, parent } => json!({ "figure": figure, "joint": parent, "rotateBy": {"axis": "bone", "degrees": dx * TWIST_PER_POINT} }),
        Mode::Reach { figure, joint } => json!({ "figure": figure, "joint": joint, "point": p }),
    };
    let cmd = match drag.mode {
        Mode::Orbit | Mode::Pan | Mode::Dolly => "layer.figure.setCamera",
        Mode::Turn { .. } | Mode::Twist { .. } => "layer.figure.setJoint",
        Mode::Reach { .. } => "layer.figure.reachTo",
    };
    v["layer"] = json!(drag.layer.0);
    v["coalesce"] = json!(drag.key);
    v["draft"] = json!(draft);
    Some((cmd, v))
}

/// The pointer path. `true` when the event was the figure editor's.
pub fn pointer(app: &mut PrismApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if app.drag.is_some() || app.ruler_drag.is_some() || app.figure_drag.is_some() || !active(app) {
                return false;
            }
            let Some(layer) = app.session.active().and_then(|st| st.active_layer) else { return false };
            let zoom = f64::from(app.current_zoom());
            let all = current(app);
            let mode = match at(&all, [x, y], zoom) {
                Some(h) if h.end && mods.command => Mode::Reach { figure: h.figure, joint: h.joint },
                Some(h) => {
                    let Some(parent) = h.parent.clone() else { return false };
                    if mods.shift {
                        Mode::Twist { figure: h.figure, parent }
                    } else {
                        let pivot = app
                            .session
                            .active()
                            .map(|st| prism_engine::figure_cmds::joints(&st.doc, layer))
                            .and_then(|js| js.into_iter().find(|j| j.figure == h.figure && j.name == parent).map(|j| j.doc))
                            .unwrap_or([h.x, h.y]);
                        Mode::Turn { figure: h.figure, parent, pivot }
                    }
                }
                None if mods.shift => Mode::Pan,
                None if mods.alt => Mode::Dolly,
                None => Mode::Orbit,
            };
            app.figure_seq = app.figure_seq.wrapping_add(1);
            app.figure_drag = Some(FigureDrag { mode, layer, last: [x, y], key: format!("figure-{}", app.figure_seq), moved: false });
            true
        }
        ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => {
            let Some(drag) = app.figure_drag.clone() else { return false };
            let up = matches!(ev, ToolEvent::Up { .. });
            let zoom = f64::from(app.current_zoom());
            let moved = (x - drag.last[0]).abs() > 1e-9 || (y - drag.last[1]).abs() > 1e-9;
            if moved && let Some((cmd, p)) = step(&drag, [x, y], zoom, !up) {
                match app.run(cmd, p) {
                    Ok(_) => {
                        if let Some(d) = app.figure_drag.as_mut() {
                            d.moved = true;
                            d.last = [x, y];
                        }
                    }
                    Err(e) => {
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                }
            } else if up && drag.moved {
                // The last move was a draft: re-render at full quality into the same step.
                let _ = app.run("layer.figure.setCamera", json!({"layer": drag.layer.0, "coalesce": drag.key}));
            }
            if up {
                app.figure_drag = None;
            }
            true
        }
    }
}

/// The cursor over the editor: a hand on a joint, the move cross elsewhere.
pub fn cursor(app: &PrismApp, p: [f64; 2]) -> Option<egui::CursorIcon> {
    if app.figure_drag.is_some() {
        return Some(egui::CursorIcon::Grabbing);
    }
    if !active(app) {
        return None;
    }
    Some(if at(&current(app), p, f64::from(app.current_zoom())).is_some() { egui::CursorIcon::Grab } else { egui::CursorIcon::AllScroll })
}

/// Bones between handles and the joint dots, while the editor is live.
pub fn draw(app: &PrismApp, painter: &egui::Painter, xf: &crate::canvas::ViewXform) {
    if !active(app) {
        return;
    }
    let t = crate::theme::Tokens::get(painter.ctx());
    let color = t.accent;
    let all = current(app);
    for h in &all {
        if let Some(p) = h.parent.as_ref().and_then(|n| all.iter().find(|o| o.figure == h.figure && &o.joint == n)) {
            painter.line_segment([xf.to_screen(p.x as f32, p.y as f32), xf.to_screen(h.x as f32, h.y as f32)], Stroke::new(1.0, color.gamma_multiply(0.7)));
        }
    }
    let hot = app.figure_drag.as_ref().and_then(|d| match &d.mode {
        Mode::Turn { parent, figure, .. } | Mode::Twist { parent, figure } => Some((*figure, parent.clone(), true)),
        Mode::Reach { figure, joint } => Some((*figure, joint.clone(), false)),
        _ => None,
    });
    for h in &all {
        let at = xf.to_screen(h.x as f32, h.y as f32);
        let grabbed = hot.as_ref().is_some_and(|(f, n, by_parent)| *f == h.figure && if *by_parent { h.parent.as_ref() == Some(n) } else { &h.joint == n });
        let fill = if grabbed { color } else { Color32::WHITE };
        let r = if h.end { 5.0 } else { 4.0 };
        painter.circle(at, r, fill, Stroke::new(1.5, color));
    }
}

/// What agents see of the editor (`ui.inspect` › `figureHandles`).
pub fn inspect(app: &PrismApp) -> Value {
    json!(current(app))
}

/// Run a Properties edit: live drafts while a slider drags, the final render on release.
fn run_edit(app: &mut PrismApp, cmd: &str, mut p: Value, r: &egui::Response, key: &str) {
    if r.dragged() && r.changed() {
        p["draft"] = json!(true);
        p["coalesce"] = json!(key);
        let _ = app.run(cmd, p);
    } else if r.drag_stopped() || (r.changed() && !r.dragged()) {
        p["coalesce"] = json!(key);
        let _ = app.run(cmd, p);
    }
}

/// Properties section of a figure layer: the figures, camera and look, and its actions.
pub fn properties(app: &mut PrismApp, ui: &mut egui::Ui, layer: &Layer) {
    use crate::props_layout::section;
    let Some(scene) = layer.figure.clone() else { return };
    let id = layer.id.0;
    if section(ui, "figure-figures", "3D Figures") {
        for f in &scene.figures {
            ui.label(egui::RichText::new(&f.name).small());
        }
        ui.add_space(crate::theme::ROW_GAP);
        ui.horizontal_wrapped(|ui| {
            let w = (ui.available_width() / 3.0 - 4.0).max(40.0);
            if widgets::secondary_button(ui, "Fit Figures", w).on_hover_text(tl!("Aim at every figure and fit them to the canvas")).clicked() {
                let _ = app.run("layer.figure.frame", json!({"layer": id, "faceFront": true}));
            }
            if widgets::secondary_button(ui, "Reset Pose", w).on_hover_text(tl!("Prism's standing pose, where each figure stands")).clicked() {
                let _ = app.run("layer.figure.resetPose", json!({"layer": id}));
            }
            if widgets::secondary_button(ui, "Rasterize", w).on_hover_text(tl!("Keep the pixels and drop the 3D figures, to paint on them")).clicked() {
                let _ = app.run("layer.rasterize.figure", json!({"layer": id}));
            }
        });
        ui.add_space(crate::theme::ROW_GAP);
    }
    if section(ui, "figure-camera", "Camera") {
        let c = scene.camera;
        let mut yaw = c.yaw as f32;
        let r = widgets::slider_row(ui, tl!("Turn"), &mut yaw, 0.0..=360.0, "°", None);
        run_edit(app, "layer.figure.setCamera", json!({"layer": id, "yaw": yaw}), &r, &format!("figure-yaw-{id}"));
        let mut pitch = c.pitch as f32;
        let r = widgets::slider_row(ui, tl!("Tilt"), &mut pitch, -89.0..=89.0, "°", None);
        run_edit(app, "layer.figure.setCamera", json!({"layer": id, "pitch": pitch}), &r, &format!("figure-pitch-{id}"));
        let mut fov = c.fov_deg as f32;
        let r = widgets::slider_row(ui, tl!("Field of view"), &mut fov, 5.0..=120.0, "°", None);
        run_edit(app, "layer.figure.setCamera", json!({"layer": id, "fov": fov}), &r, &format!("figure-fov-{id}"));
        ui.label(egui::RichText::new(tl!("Drag on the canvas with the Move tool: orbit; ⇧ move; ⌥ zoom; drag a joint to pose it")).small());
        ui.add_space(crate::theme::ROW_GAP);
    }
    if section(ui, "figure-look", "Look") {
        let l = scene.look;
        let mut outline = l.outline;
        if widgets::checkbox(ui, &mut outline, "Outline").changed() {
            let _ = app.run("layer.figure.setLook", json!({"layer": id, "outline": outline}));
        }
        let mut w = l.outline_width as f32;
        let r = widgets::slider_row(ui, tl!("Line width"), &mut w, 0.0..=8.0, "px", None);
        run_edit(app, "layer.figure.setLook", json!({"layer": id, "outlineWidth": w}), &r, &format!("figure-line-{id}"));
        ui.add_space(crate::theme::ROW_GAP);
    }
}

#[cfg(test)]
mod tests;
