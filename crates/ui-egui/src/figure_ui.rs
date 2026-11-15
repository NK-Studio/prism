//! Editing 3D figure layers (CLIP STUDIO's 3D drawing figures) on the canvas and in Properties.
//!
//! With the Move tool on a figure layer (View › Extras on, no transform box), the figure's main
//! joints show as dots, and a hand's finger joints too once that hand is [`FINGER_HAND_MIN`]
//! screen points long (zoom in on it). Drag a dot: turn the bone ending there in the picture plane
//! (⇧: twist it about itself; ⌘/Ctrl on a hand, a foot or a fingertip: reach it there with
//! two-bone IK). Drag anywhere else:
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
/// Dots this many screen points further than the nearest still count as under the pointer, and
/// the one in front wins.
pub const OVERLAP: f64 = 3.0;
/// Camera turn per screen point dragged (degrees).
const ORBIT_PER_POINT: f64 = 0.4;
/// Dolly factor per screen point dragged (vertical).
const DOLLY_PER_POINT: f64 = 0.005;
/// Twist per screen point dragged (degrees).
const TWIST_PER_POINT: f64 = 0.6;
/// A hand's finger joints get handles once the hand (wrist to the farthest fingertip) is this
/// many screen points long: then neighbouring knuckles sit about 6 points apart.
pub const FINGER_HAND_MIN: f64 = 90.0;
/// The digits of a CLIP STUDIO / Prism hand, as they appear in joint names.
const DIGITS: [&str; 5] = ["thumb", "index", "middle", "ring", "pinky"];

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
    /// A hand, a foot or a fingertip: ⌘/Ctrl-drag reaches it with IK.
    pub end: bool,
    /// A finger joint (shown only on hands zoomed in far enough).
    pub finger: bool,
    /// Distance from the camera (cm): where dots overlap, the nearest one is grabbed.
    pub depth: f64,
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

/// `Some((hand, segment))` for a finger joint such as `lefthandindex3_bb_`: the hand joint's name
/// (`lefthand_bb_`) and the segment number (1 at the knuckle, 4 at the tip).
fn finger_joint(name: &str) -> Option<(String, u32)> {
    let n = name.to_ascii_lowercase();
    let at = n.find("hand")?;
    let rest = n.get(at + 4..)?.strip_suffix("_bb_")?;
    let digit = DIGITS.iter().find(|d| rest.starts_with(**d))?;
    let segment = rest.get(digit.len()..)?.parse::<u32>().ok().filter(|s| (1..=9).contains(s))?;
    Some((format!("{}hand_bb_", n.get(..at)?), segment))
}

/// Joints that get a handle: the body's main joints, not tips, the root or finger joints (those
/// come per hand, see [`handles`]).
fn shown(j: &JointPoint) -> bool {
    let n = j.name.to_ascii_lowercase();
    j.parent.is_some() && finger_joint(&n).is_none() && !n.contains("end") && !n.contains("root") && !n.starts_with("spine_")
}

fn is_end(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ((n.ends_with("hand_bb_") || n.ends_with("foot_bb_")) && !n.contains("toe")) || finger_joint(&n).is_some_and(|(_, s)| s >= 4)
}

/// The hands (figure, hand joint) long enough on screen at `zoom` to show their finger joints.
fn open_hands(joints: &[JointPoint], zoom: f64) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for j in joints {
        let Some((hand, _)) = finger_joint(&j.name) else { continue };
        if out.iter().any(|(f, h)| *f == j.figure && *h == hand) {
            continue;
        }
        let Some(wrist) = joints.iter().find(|w| w.figure == j.figure && w.name.eq_ignore_ascii_case(&hand)) else { continue };
        let reach = joints
            .iter()
            .filter(|t| t.figure == j.figure && finger_joint(&t.name).is_some_and(|(h, _)| h == hand))
            .map(|t| (t.doc[0] - wrist.doc[0]).hypot(t.doc[1] - wrist.doc[1]))
            .filter(|d| d.is_finite())
            .fold(0.0, f64::max);
        if reach * zoom >= FINGER_HAND_MIN {
            out.push((j.figure, hand));
        }
    }
    out
}

/// The figure layer `layer` of `doc`, if it is one.
pub fn figure_layer(doc: &Document, layer: LayerId) -> Option<&Layer> {
    doc.layer(layer).filter(|l| l.figure.is_some() && l.visible)
}

/// The handles of figure layer `layer` seen at `zoom`: the main joints, plus the finger joints
/// of hands at least [`FINGER_HAND_MIN`] screen points long.
pub fn handles(doc: &Document, layer: LayerId, zoom: f64) -> Vec<Handle> {
    if figure_layer(doc, layer).is_none() {
        return Vec::new();
    }
    let joints = prism_engine::figure_cmds::joints(doc, layer);
    let open = open_hands(&joints, zoom.max(1e-3));
    joints
        .into_iter()
        .filter_map(|j| {
            let finger = match finger_joint(&j.name) {
                Some((hand, _)) if open.iter().any(|(f, h)| *f == j.figure && *h == hand) => true,
                Some(_) => return None,
                None if shown(&j) => false,
                None => return None,
            };
            Some(Handle { end: is_end(&j.name), finger, depth: j.depth, figure: j.figure, x: j.doc[0], y: j.doc[1], parent: j.parent, joint: j.name })
        })
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
    st.active_layer.map(|l| handles(&st.doc, l, f64::from(app.current_zoom()))).unwrap_or_default()
}

/// The handle within [`HIT`] screen points of `p`. Of dots that overlap there (within
/// [`OVERLAP`] points of the nearest), the one nearest the camera: the finger in front, not the
/// one hidden behind it, as clicking the model picks the part in front.
pub fn at(handles: &[Handle], p: [f64; 2], zoom: f64) -> Option<Handle> {
    let zoom = zoom.max(1e-3);
    let near: Vec<(&Handle, f64)> = handles.iter().map(|h| (h, (h.x - p[0]).hypot(h.y - p[1]))).filter(|(_, d)| d.is_finite() && *d <= HIT / zoom).collect();
    let best = near.iter().map(|(_, d)| *d).fold(f64::INFINITY, f64::min);
    near.into_iter().filter(|(_, d)| *d <= best + OVERLAP / zoom).min_by(|a, b| a.0.depth.total_cmp(&b.0.depth).then(a.1.total_cmp(&b.1))).map(|(h, _)| h.clone())
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
    // Far dots first, so the ones in front are drawn over them.
    let mut order: Vec<&Handle> = all.iter().collect();
    order.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    for h in order {
        let at = xf.to_screen(h.x as f32, h.y as f32);
        let grabbed = hot.as_ref().is_some_and(|(f, n, by_parent)| *f == h.figure && if *by_parent { h.parent.as_ref() == Some(n) } else { &h.joint == n });
        let fill = if grabbed { color } else { Color32::WHITE };
        let r = match (h.finger, h.end) {
            (true, true) => 3.0,
            (true, false) => 2.5,
            (false, true) => 5.0,
            (false, false) => 4.0,
        };
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
        ui.label(egui::RichText::new(tl!("Zoom in on a hand to pose its fingers")).small());
        ui.add_space(crate::theme::ROW_GAP);
    }
    hand_section(app, ui, id);
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

/// The closed shapes at the triangle's bottom corner, as `layer.figure.setHand` names them.
pub const CLOSED: [(&str, &str); 4] = [("fist", "Fist"), ("catPaw", "Cat Paw"), ("tube", "Tube"), ("grip", "Grip")];
/// Which hands the setup poses.
pub const SIDES: [(&str, &str); 3] = [("both", "Both Hands"), ("left", "Left Hand"), ("right", "Right Hand")];
/// The fingers, thumb first, as `layer.figure.setHand` names them.
pub const FINGERS: [(&str, &str); 5] = [("thumb", "Thumb"), ("index", "Index"), ("middle", "Middle"), ("ring", "Ring"), ("pinky", "Little")];

/// Hand setup state of one layer (kept in the UI, as CLIP STUDIO keeps its pad): blend weights
/// (flat, spread, closed), the closed shape, the locked fingers, the side, and a gesture counter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandPad {
    pub weights: [f32; 3],
    pub closed: usize,
    pub locked: [bool; 5],
    pub side: usize,
    seq: u64,
}

impl Default for HandPad {
    fn default() -> Self {
        Self { weights: [1.0, 0.0, 0.0], closed: 0, locked: [false; 5], side: 0, seq: 0 }
    }
}

impl HandPad {
    /// The `layer.figure.setHand` params for layer `layer`.
    pub fn params(&self, layer: u64) -> Value {
        let locked: Vec<&str> = FINGERS.iter().zip(self.locked).filter(|(_, l)| *l).map(|((id, _), _)| *id).collect();
        json!({
            "layer": layer,
            "weights": self.weights,
            "closed": CLOSED.get(self.closed).map_or("fist", |c| c.0),
            "side": SIDES.get(self.side).map_or("both", |c| c.0),
            "locked": locked,
        })
    }
}

/// Room above and below the pad's triangle for the corner names.
const PAD_LABEL: f32 = 16.0;

/// The pad's corners: flat hand top left, spread hand top right, closed hand at the bottom
/// (the names sit above and below them, outside the triangle).
pub fn pad_corners(r: egui::Rect) -> [egui::Pos2; 3] {
    let inset = 8.0;
    [
        egui::pos2(r.left() + inset, r.top() + PAD_LABEL + inset),
        egui::pos2(r.right() - inset, r.top() + PAD_LABEL + inset),
        egui::pos2(r.center().x, r.bottom() - PAD_LABEL - inset),
    ]
}

/// Blend weights for a point on the pad: its barycentric coordinates, clamped into the triangle.
pub fn pad_weights(c: [egui::Pos2; 3], p: egui::Pos2) -> [f32; 3] {
    let (a, b, d) = (c[0], c[1], c[2]);
    let det = (b.y - d.y) * (a.x - d.x) + (d.x - b.x) * (a.y - d.y);
    if det.abs() < 1e-6 {
        return [1.0, 0.0, 0.0];
    }
    let w0 = ((b.y - d.y) * (p.x - d.x) + (d.x - b.x) * (p.y - d.y)) / det;
    let w1 = ((d.y - a.y) * (p.x - d.x) + (a.x - d.x) * (p.y - d.y)) / det;
    let w = [w0.max(0.0), w1.max(0.0), (1.0 - w0 - w1).max(0.0)];
    let sum: f32 = w.iter().sum();
    if sum > 1e-6 && sum.is_finite() { w.map(|v| v / sum) } else { [1.0, 0.0, 0.0] }
}

/// Where weights `w` sit on the pad.
pub fn pad_point(c: [egui::Pos2; 3], w: [f32; 3]) -> egui::Pos2 {
    let sum: f32 = w.iter().sum();
    let w = if sum > 1e-6 && sum.is_finite() { w.map(|v| v / sum) } else { [1.0, 0.0, 0.0] };
    egui::pos2(c[0].x * w[0] + c[1].x * w[1] + c[2].x * w[2], c[0].y * w[0] + c[1].y * w[1] + c[2].y * w[2])
}

/// A label of the hand setup, translated in its own context ("Ring" is a finger here).
fn hand_label(s: &'static str) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "hand", s)
}

fn pad_id(layer: u64) -> egui::Id {
    egui::Id::new(("figure-hand-pad", layer))
}

/// The hand setup state of layer `layer` (defaults until the pad is used).
pub fn pad_state(ctx: &egui::Context, layer: u64) -> HandPad {
    ctx.data(|d| d.get_temp::<HandPad>(pad_id(layer))).unwrap_or_default()
}

/// Run the hand setup; `draft` while the pad drags, under the gesture's one history step.
fn run_hand(app: &mut PrismApp, pad: &HandPad, layer: u64, draft: bool) {
    let mut p = pad.params(layer);
    p["coalesce"] = json!(format!("figure-hand-{layer}-{}", pad.seq));
    p["draft"] = json!(draft);
    if let Err(e) = app.run("layer.figure.setHand", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Properties › Hand Pose: CLIP STUDIO's hand setup. Drag in the triangle to blend the hand
/// between flat, spread and the closed shape picked below it; locked fingers keep their pose.
fn hand_section(app: &mut PrismApp, ui: &mut egui::Ui, layer: u64) {
    use crate::props_layout::section;
    if !section(ui, "figure-hands", "Hand Pose") {
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut pad = pad_state(ui.ctx(), layer);
    let before = pad;
    ui.horizontal_wrapped(|ui| {
        for (k, (_, label)) in SIDES.iter().enumerate() {
            if widgets::pill_tab(ui, hand_label(label), pad.side == k).clicked() {
                pad.side = k;
            }
        }
    });
    ui.label(egui::RichText::new(tl!("Lock Fingers")).small());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (k, (_, label)) in FINGERS.iter().enumerate() {
            let on = pad.locked.get(k).copied().unwrap_or(false);
            let r = widgets::pill_tab(ui, hand_label(label), on).on_hover_text(tl!("A locked finger keeps its pose"));
            if r.clicked()
                && let Some(l) = pad.locked.get_mut(k)
            {
                *l = !on;
            }
        }
    });
    // The triangle pad, centred: CLIP STUDIO's is about as wide as it is tall.
    ui.add_space(crate::theme::ROW_GAP);
    let avail = ui.available_width();
    let w = avail.min(200.0);
    let (row, _) = ui.allocate_exact_size(egui::vec2(avail, 150.0), egui::Sense::hover());
    let r = egui::Rect::from_center_size(row.center(), egui::vec2(w, 150.0));
    let resp = ui.interact(r, pad_id(layer).with("pad"), egui::Sense::click_and_drag());
    let c = pad_corners(r);
    let painter = ui.painter_at(r.expand(6.0));
    painter.add(egui::Shape::convex_polygon(c.to_vec(), t.field, Stroke::new(1.0, t.text_dim)));
    let font = egui::FontId::proportional(11.0);
    let closed_label = CLOSED.get(pad.closed).map_or("Fist", |c| c.1);
    painter.text(egui::pos2(c[0].x - 4.0, r.top()), egui::Align2::LEFT_TOP, hand_label("Flat"), font.clone(), t.text_dim);
    painter.text(egui::pos2(c[1].x + 4.0, r.top()), egui::Align2::RIGHT_TOP, hand_label("Spread"), font.clone(), t.text_dim);
    painter.text(egui::pos2(c[2].x, r.bottom()), egui::Align2::CENTER_BOTTOM, hand_label(closed_label), font, t.text_dim);
    if resp.drag_started() || resp.clicked() {
        pad.seq = pad.seq.wrapping_add(1);
    }
    let mut run = None;
    if (resp.dragged() || resp.clicked() || resp.drag_started())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let nw = pad_weights(c, p);
        if nw != pad.weights || resp.clicked() {
            pad.weights = nw;
            run = Some(!resp.clicked());
        }
    }
    if resp.drag_stopped() {
        run = Some(false);
    }
    let at = pad_point(c, pad.weights);
    painter.circle(at, 5.0, t.accent, Stroke::new(1.5, Color32::WHITE));
    let label = format!("{} {:.0}% · {} {:.0}% · {} {:.0}%", hand_label("Flat"), pad.weights[0] * 100.0, hand_label("Spread"), pad.weights[1] * 100.0, hand_label(closed_label), pad.weights[2] * 100.0);
    resp.on_hover_text(tl!("Drag in the triangle to blend the hands between flat, spread and closed")).widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, &label));
    ui.horizontal_wrapped(|ui| {
        for (k, (_, label)) in CLOSED.iter().enumerate() {
            if widgets::pill_tab(ui, hand_label(label), pad.closed == k).clicked() && pad.closed != k {
                pad.closed = k;
                pad.seq = pad.seq.wrapping_add(1);
                run = Some(false);
            }
        }
    });
    if pad != before {
        ui.data_mut(|d| d.insert_temp(pad_id(layer), pad));
    }
    if let Some(draft) = run {
        run_hand(app, &pad, layer, draft);
    }
    ui.add_space(crate::theme::ROW_GAP);
}

#[cfg(test)]
mod tests;
