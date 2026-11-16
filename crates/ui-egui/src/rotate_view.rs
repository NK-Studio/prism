//! Rotate View tool: turn the canvas camera around its centre without rewriting pixels.
//!
//! Drag around the view centre (clockwise increases the angle). Shift snaps to 15°. While the
//! drag lasts a compass stands at the view centre, as in Photoshop: its rose is fixed to the
//! screen and its needle turns with the image, the red half pointing at the document's top.
//! Reset View, Escape, a double-click, or the small corner compass put the angle back to 0.
//! History is not recorded. Trackpad rotate is gated by Preferences › Enhanced Controls ›
//! Rotate View with Trackpad.

use egui::{Align2, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use crate::PrismApp;
use crate::canvas::ViewXform;
use crate::paint_mouse::Buttons;
use crate::state::View;
use crate::theme::Tokens;

/// Snap increment, in degrees, while Shift is held.
pub const SNAP_DEG: f32 = 15.0;

/// A Rotate View drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RotateDrag {
    /// Screen point of the view centre when the drag began.
    pivot: Pos2,
    /// Pointer angle, in degrees, at the press (atan2, Y-down: clockwise is positive).
    start_pointer: f32,
    /// View rotation, in degrees, at the press.
    start_rotation: f32,
}

fn id() -> egui::Id {
    egui::Id::new("pc-rotate-view-drag")
}

fn compass_id() -> egui::Id {
    egui::Id::new("pc-rotate-view-compass")
}

/// Wrap `a` degrees into (−180, 180]. Non-finite values become 0 so a hostile angle never
/// propagates into the camera.
pub fn wrap_deg(a: f32) -> f32 {
    if !a.is_finite() {
        return 0.0;
    }
    let mut x = f64::from(a) % 360.0;
    if x > 180.0 {
        x -= 360.0;
    } else if x <= -180.0 {
        x += 360.0;
    }
    x as f32
}

/// Nearest multiple of `step` degrees, wrapped. A non-positive or non-finite step is ignored.
pub fn snap_deg(a: f32, step: f32) -> f32 {
    if !(step.is_finite() && step > 0.0) {
        return wrap_deg(a);
    }
    wrap_deg((a / step).round() * step)
}

/// Pointer angle in degrees around `pivot` (Y-down, so clockwise is positive).
pub fn pointer_deg(p: Pos2, pivot: Pos2) -> f32 {
    let d = p - pivot;
    if !(d.x.is_finite() && d.y.is_finite()) || d.length_sq() < 1e-8 {
        return 0.0;
    }
    d.y.atan2(d.x).to_degrees()
}

fn finite_number(p: &Value, key: &str) -> Result<Option<f32>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|n| n.is_finite()).map(|n| Some(n as f32)).ok_or_else(|| format!("{key} must be a finite number")),
    }
}

/// Apply `view.rotateView` params to `current`. Empty params, a non-object, or a missing
/// `angle`/`delta`/`reset` are errors (never a silent no-op).
pub fn apply_params(current: f32, p: &Value) -> Result<f32, String> {
    let obj = p.as_object().ok_or("params must be an object")?;
    for k in obj.keys() {
        if !matches!(k.as_str(), "angle" | "delta" | "reset") {
            return Err(format!("unknown field `{k}`"));
        }
    }
    let reset = match obj.get("reset") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| "reset must be a boolean".to_string())?,
    };
    let angle = finite_number(p, "angle")?;
    let delta = finite_number(p, "delta")?;
    if !reset && angle.is_none() && delta.is_none() {
        return Err("expected angle, delta, or reset".into());
    }
    if reset {
        return Ok(0.0);
    }
    let mut r = angle.unwrap_or(current);
    if let Some(d) = delta {
        r += d;
        if !r.is_finite() {
            return Err("delta overflowed the angle".into());
        }
    }
    Ok(wrap_deg(r))
}

/// Trackpad rotate: `delta_rad` is egui's rotation delta (radians, same sign as [`wrap_deg`]).
/// Returns the new angle when the preference is on and the delta is a finite non-zero value.
pub fn apply_trackpad(current: f32, enabled: bool, delta_rad: f32) -> Option<f32> {
    if !enabled || !delta_rad.is_finite() || delta_rad == 0.0 {
        return None;
    }
    let deg = delta_rad.to_degrees();
    if !deg.is_finite() {
        return None;
    }
    let next = current + deg;
    if !next.is_finite() {
        return None;
    }
    Some(wrap_deg(next))
}

/// This frame's trackpad rotation in degrees, if the preference is on and egui reported a delta.
pub fn trackpad_delta(ctx: &egui::Context, enabled: bool) -> Option<f32> {
    if !enabled {
        return None;
    }
    let rad = ctx.input(|i| {
        let from_events: f32 = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Rotate(r) if r.is_finite() => Some(*r),
                _ => None,
            })
            .sum();
        if from_events != 0.0 { from_events } else { i.rotation_delta() }
    });
    apply_trackpad(0.0, true, rad).filter(|_| rad != 0.0)
}

/// `view.rotateView`: set or add degrees on the active document's camera.
pub fn command(app: &mut PrismApp, p: &Value) -> Result<Value, String> {
    let i = app.session.active_index().ok_or("no document open")?;
    let current = app.view(i).map(|v| v.rotation).ok_or("no document open")?;
    let rotation = apply_params(current, p)?;
    let v = app.view_mut(i).ok_or("no document open")?;
    v.rotation = rotation;
    Ok(json!({"rotation": v.rotation}))
}

/// `view.resetView`: angle 0, zoom and centre unchanged.
pub fn reset(app: &mut PrismApp) -> Result<Value, String> {
    let i = app.session.active_index().ok_or("no document open")?;
    let v = app.view_mut(i).ok_or("no document open")?;
    v.rotation = 0.0;
    Ok(json!({"rotation": 0.0, "zoom": v.zoom, "center": v.center}))
}

/// Route canvas buttons for Rotate View. Returns true when the drag is owned (the canvas then
/// ignores the tool underneath).
pub fn drag(ctx: &egui::Context, view: &mut View, xf: &ViewXform, b: &Buttons, pointer: Option<Pos2>) -> bool {
    if b.started {
        let pivot = xf.rect.center();
        let origin = ctx.input(|i| i.pointer.press_origin()).filter(|p| xf.rect.contains(*p)).or(pointer);
        if let Some(p) = origin {
            ctx.data_mut(|d| {
                d.insert_temp(id(), RotateDrag { pivot, start_pointer: pointer_deg(p, pivot), start_rotation: view.rotation });
            });
        }
    }
    let Some(z) = ctx.data(|d| d.get_temp::<RotateDrag>(id())) else {
        return b.started || b.dragged || b.stopped;
    };
    if let Some(p) = pointer.filter(|_| b.dragged || b.stopped) {
        let delta = wrap_deg(pointer_deg(p, z.pivot) - z.start_pointer);
        let mut angle = z.start_rotation + delta;
        if !angle.is_finite() {
            angle = z.start_rotation;
        }
        if ctx.input(|i| i.modifiers.shift) {
            angle = snap_deg(angle, SNAP_DEG);
        }
        view.rotation = wrap_deg(angle);
    }
    if b.stopped {
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
    }
    true
}

/// Screen direction of the document's top (a unit vector, Y-down) in a view turned `rotation`
/// degrees and optionally flipped, as [`ViewXform::to_screen`] maps it. A hostile angle is 0.
pub fn north(rotation: f32, flip: bool) -> Vec2 {
    let rad = wrap_deg(rotation).to_radians();
    let (s, c) = if rad.abs() < 1e-6 { (0.0, 1.0) } else { (rad.sin(), rad.cos()) };
    vec2(if flip { -s } else { s }, -c)
}

/// The drag compass, in points: the rose's long and short points and the ring they stand on,
/// the needle's reach and half-width, and the pivot.
const ROSE_LONG: f32 = 68.0;
const ROSE_SHORT: f32 = 46.0;
const ROSE_RING: f32 = 17.0;
const NEEDLE: f32 = 88.0;
const NEEDLE_HALF: f32 = 11.0;
const HUB: f32 = 5.0;

/// The drag compass's size in a canvas: full size where it fits, smaller in a small view, and
/// nothing in one too small to read it (or with a hostile rect).
pub fn compass_scale(canvas: Rect) -> Option<f32> {
    let (w, h) = (canvas.width(), canvas.height());
    if !(w.is_finite() && h.is_finite() && canvas.center().x.is_finite() && canvas.center().y.is_finite()) {
        return None;
    }
    let k = w.min(h) / (2.0 * NEEDLE + 24.0);
    (k >= 0.2).then_some(k.min(1.0))
}

/// The needle's outline around `centre`: the red tip (the document's top), the right corner,
/// the white tip and the left corner.
pub fn needle(centre: Pos2, rotation: f32, flip: bool, scale: f32) -> [Pos2; 4] {
    let n = north(rotation, flip);
    let left = vec2(n.y, -n.x);
    [centre + n * (NEEDLE * scale), centre - left * (NEEDLE_HALF * scale), centre - n * (NEEDLE * scale), centre + left * (NEEDLE_HALF * scale)]
}

/// A filled triangle or quad, wound the way the tessellator's feathering expects.
fn facet(painter: &egui::Painter, mut points: Vec<Pos2>, fill: egui::Color32) {
    let area: f32 = points.iter().zip(points.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum();
    if area < 0.0 {
        points.reverse();
    }
    painter.add(Shape::convex_polygon(points, fill, Stroke::NONE));
}

/// Draw the drag compass at the centre of `canvas`: the eight-point rose stays put, the needle
/// follows the image (red = the document's top), lit on the needle's left.
fn paint_compass(painter: &egui::Painter, canvas: Rect, rotation: f32, flip: bool) {
    let Some(k) = compass_scale(canvas) else { return };
    let c = canvas.center();
    let col = Tokens::rotate_compass();
    let at = |deg: f32, r: f32| c + vec2(deg.to_radians().sin(), -deg.to_radians().cos()) * (r * k);
    let mut star = Vec::with_capacity(16);
    let mut ring = Vec::with_capacity(8);
    for i in 0..8u8 {
        let a = f32::from(i) * 45.0;
        let tip = at(a, if i % 2 == 0 { ROSE_LONG } else { ROSE_SHORT });
        let (l, r) = (at(a - 22.5, ROSE_RING), at(a + 22.5, ROSE_RING));
        let mid = l + (r - l) * 0.5;
        facet(painter, vec![tip, l, mid], col.rose[0]);
        facet(painter, vec![tip, mid, r], col.rose[1]);
        star.extend([l, tip]);
        ring.push(l);
    }
    facet(painter, ring, col.rose[0]);
    painter.add(Shape::closed_line(star, Stroke::new(1.0, col.outline)));
    painter.circle_stroke(c, (ROSE_RING - 4.5) * k, Stroke::new(2.0 * k, col.rose[1]));

    let [tip_n, right, tip_s, left] = needle(c, rotation, flip, k);
    let drop = vec2(0.0, 1.5 * k);
    facet(painter, vec![tip_n + drop, right + drop, tip_s + drop, left + drop], col.shadow);
    facet(painter, vec![tip_n, left, right], col.north[1]);
    facet(painter, vec![tip_n, left, c], col.north[0]);
    facet(painter, vec![tip_s, right, left], col.south[1]);
    facet(painter, vec![tip_s, c, left], col.south[0]);
    painter.add(Shape::closed_line(vec![tip_n, right, tip_s, left], Stroke::new(1.0, col.outline)));
    painter.circle_filled(c, HUB * k, col.hub[0]);
    painter.circle_stroke(c, HUB * k, Stroke::new(1.0, col.hub[1]));
}

/// The Rotate View drag that turns the view in `canvas`, if one is in progress there. A drag
/// whose release never arrived (the tool changed under it, the window lost the pointer) is
/// dropped here, so its compass cannot outlive the gesture.
fn active_drag(ctx: &egui::Context, canvas: Rect) -> Option<RotateDrag> {
    let z = ctx.data(|d| d.get_temp::<RotateDrag>(id()))?;
    if !ctx.input(|i| i.pointer.any_down()) {
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
        return None;
    }
    (z.pivot.distance(canvas.center()) < 0.5).then_some(z)
}

/// True while a Rotate View drag turns the view in `canvas` (its compass is showing).
pub fn dragging(ctx: &egui::Context, canvas: Rect) -> bool {
    active_drag(ctx, canvas).is_some()
}

/// Small compass in the top-right of the canvas (north = document up). Hidden at 0°.
pub fn compass_rect(canvas: Rect) -> Rect {
    const S: f32 = 36.0;
    const M: f32 = 10.0;
    Rect::from_min_size(pos2(canvas.right() - M - S, canvas.top() + M), vec2(S, S))
}

/// True when the compass is showing (angle is not ~0).
pub fn compass_visible(rotation: f32) -> bool {
    rotation.is_finite() && rotation.abs() > 0.05
}

/// Draw the compass and handle a click / Escape / double-click reset. Returns true when the
/// pointer over the compass should not start a rotate drag.
pub fn overlay(app: &PrismApp, ctx: &egui::Context, painter: &egui::Painter, xf: &ViewXform, view: &mut View, response: &egui::Response) -> bool {
    if app.ui.tool == crate::state::Tool::RotateView && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) && view.rotation != 0.0 {
        view.rotation = 0.0;
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
    }
    if app.ui.tool == crate::state::Tool::RotateView && (response.double_clicked() || response.triple_clicked()) {
        view.rotation = 0.0;
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
    }
    // The drag compass shows at every angle, 0° included, for as long as the button is down.
    if active_drag(ctx, xf.rect).is_some() {
        paint_compass(painter, xf.rect, view.rotation, xf.flip);
    }
    if !compass_visible(view.rotation) {
        ctx.data_mut(|d| d.remove::<Rect>(compass_id()));
        return false;
    }
    let r = compass_rect(xf.rect);
    ctx.data_mut(|d| d.insert_temp(compass_id(), r));
    let t = Tokens::get(ctx);
    let hover = response.hover_pos().is_some_and(|p| r.contains(p));
    painter.circle_filled(r.center(), r.width() * 0.5, if hover { t.hover } else { t.chrome });
    painter.circle_stroke(r.center(), r.width() * 0.5 - 0.5, Stroke::new(1.0, t.separator));
    // Document up: (0, −1) in document space, rotated then flipped like [`ViewXform::to_screen`].
    let tip = r.center() + north(view.rotation, xf.flip) * (r.width() * 0.32);
    painter.line_segment([r.center(), tip], Stroke::new(2.0, t.accent));
    painter.circle_filled(tip, 2.5, t.accent);
    painter.text(r.center() + vec2(0.0, r.width() * 0.18), Align2::CENTER_CENTER, "N", egui::FontId::proportional(9.0), t.text_faint);
    if response.clicked() && response.interact_pointer_pos().is_some_and(|p| r.contains(p)) {
        view.rotation = 0.0;
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
        return true;
    }
    hover
}

/// The stock cursor behind the Rotate View drawing (`tool_cursor::Art::RotateView`), for a
/// backend that cannot upload the bitmap.
pub fn cursor(dragged: bool) -> egui::CursorIcon {
    if dragged { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, pos2};
    use egui_kittest::Harness;
    use serde_json::json;

    #[test]
    fn wrap_deg_folds_huge_and_hostile_angles() {
        assert_eq!(wrap_deg(0.0), 0.0);
        assert_eq!(wrap_deg(180.0), 180.0);
        assert_eq!(wrap_deg(-180.0), 180.0);
        assert!((wrap_deg(190.0) + 170.0).abs() < 1e-4);
        assert!((wrap_deg(-190.0) - 170.0).abs() < 1e-4);
        assert!((wrap_deg(360.0)).abs() < 1e-4);
        assert!((wrap_deg(1.0e10)).abs() <= 180.0);
        assert_eq!(wrap_deg(f32::NAN), 0.0);
        assert_eq!(wrap_deg(f32::INFINITY), 0.0);
        assert_eq!(wrap_deg(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn snap_deg_lands_on_fifteens() {
        assert_eq!(snap_deg(0.0, SNAP_DEG), 0.0);
        assert_eq!(snap_deg(7.0, SNAP_DEG), 0.0);
        assert_eq!(snap_deg(8.0, SNAP_DEG), 15.0);
        assert_eq!(snap_deg(-7.0, SNAP_DEG), 0.0);
        assert_eq!(snap_deg(-8.0, SNAP_DEG), -15.0);
        assert_eq!(snap_deg(40.0, SNAP_DEG), 45.0);
        assert_eq!(snap_deg(f32::NAN, SNAP_DEG), 0.0);
        assert_eq!(snap_deg(40.0, 0.0), wrap_deg(40.0));
        assert_eq!(snap_deg(40.0, f32::NAN), wrap_deg(40.0));
    }

    #[test]
    fn apply_params_sets_adds_resets_and_rejects() {
        assert_eq!(apply_params(0.0, &json!({"angle": 30.0})).unwrap(), 30.0);
        assert_eq!(apply_params(30.0, &json!({"delta": 15.0})).unwrap(), 45.0);
        assert_eq!(apply_params(45.0, &json!({"reset": true})).unwrap(), 0.0);
        assert_eq!(apply_params(170.0, &json!({"delta": 20.0})).unwrap(), -170.0);
        assert!(apply_params(0.0, &json!({})).is_err());
        assert!(apply_params(0.0, &Value::Null).is_err());
        assert!(apply_params(0.0, &json!("30")).is_err());
        assert!(apply_params(0.0, &json!({"angle": f64::NAN})).is_err());
        assert!(apply_params(0.0, &json!({"angle": f64::INFINITY})).is_err());
        assert!(apply_params(0.0, &json!({"delta": f64::INFINITY})).is_err());
        assert!(apply_params(0.0, &json!({"reset": "yes"})).is_err());
        assert!(apply_params(0.0, &json!({"zoom": 1})).is_err());
        let huge = apply_params(0.0, &json!({"angle": 1.0e20})).unwrap();
        assert!(huge.is_finite() && huge.abs() <= 180.0);
    }

    #[test]
    fn trackpad_pref_gates_rotation() {
        assert_eq!(apply_trackpad(10.0, false, 0.5), None);
        assert_eq!(apply_trackpad(10.0, true, 0.0), None);
        assert_eq!(apply_trackpad(10.0, true, f32::NAN), None);
        assert_eq!(apply_trackpad(10.0, true, f32::INFINITY), None);
        let a = apply_trackpad(10.0, true, std::f32::consts::FRAC_PI_2).expect("finite delta");
        assert!((a - 100.0).abs() < 0.05, "{a}");
    }

    #[test]
    fn view_xform_identity_matches_unrotated_mapping() {
        for zoom in [0.5, 1.0, 2.5] {
            for center in [[0.0, 0.0], [320.0, 240.0], [10.0, -4.0]] {
                for flip in [false, true] {
                    let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
                    let a = ViewXform { rect, zoom, center, flip, rotation: 0.0 };
                    let sx = if flip { -1.0 } else { 1.0 };
                    let want = rect.center() + vec2((10.0 - center[0]) * zoom * sx, (20.0 - center[1]) * zoom);
                    let got = a.to_screen(10.0, 20.0);
                    assert!((got.x - want.x).abs() < 1e-4 && (got.y - want.y).abs() < 1e-4, "{got:?} vs {want:?}");
                }
            }
        }
    }

    #[test]
    fn view_xform_roundtrip_and_ninety_degrees() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        for rot in [15.0, 90.0, -45.0, 180.0] {
            for flip in [false, true] {
                let xf = ViewXform { rect, zoom: 2.0, center: [80.0, 40.0], flip, rotation: rot };
                let s = xf.to_screen(10.0, 20.0);
                let d = xf.to_doc(s);
                assert!((d[0] - 10.0).abs() < 1e-3 && (d[1] - 20.0).abs() < 1e-3, "rot={rot} flip={flip} {d:?}");
            }
        }
        // 90°: a point on +X in document space lands on +Y on screen (Y-down = clockwise).
        let xf = ViewXform { rect, zoom: 1.0, center: [0.0, 0.0], flip: false, rotation: 90.0 };
        let s = xf.to_screen(1.0, 0.0);
        let c = rect.center();
        assert!((s.x - c.x).abs() < 1e-3 && (s.y - (c.y + 1.0)).abs() < 1e-3, "{s:?} vs {c:?}");
        // Flip after rotate: 0° +X goes left.
        let xf = ViewXform { rect, zoom: 1.0, center: [0.0, 0.0], flip: true, rotation: 0.0 };
        assert!(xf.to_screen(1.0, 0.0).x < rect.center().x);
        let xf = ViewXform { rect, zoom: 1.0, center: [0.0, 0.0], flip: true, rotation: 90.0 };
        let s = xf.to_screen(1.0, 0.0);
        assert!((s.x - c.x).abs() < 1e-3 && (s.y - (c.y + 1.0)).abs() < 1e-3, "flip then rotate {s:?}");
    }

    fn app_docs(n: usize) -> PrismApp {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        for i in 0..n {
            app.run("file.new", json!({"width": 200 + i as u32 * 10, "height": 100})).unwrap();
        }
        app.sync_views();
        app
    }

    #[test]
    fn commands_rotate_reset_and_reject_without_a_document() {
        let ctx = egui::Context::default();
        let mut empty = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        assert_eq!(crate::menus::invoke(&mut empty, &ctx, "view.rotateView", json!({"angle": 30})).unwrap_err(), "no document open");
        assert_eq!(crate::menus::invoke(&mut empty, &ctx, "view.resetView", json!({})).unwrap_err(), "no document open");

        let mut app = app_docs(1);
        let r = crate::menus::invoke(&mut app, &ctx, "view.rotateView", json!({"angle": 30})).unwrap();
        assert_eq!(r["rotation"], 30.0);
        assert_eq!(app.ui.views[0].rotation, 30.0);
        let r = crate::menus::invoke(&mut app, &ctx, "view.rotateView", json!({"delta": 15})).unwrap();
        assert_eq!(r["rotation"], 45.0);
        let z = app.ui.views[0].zoom;
        let c = app.ui.views[0].center;
        let r = crate::menus::invoke(&mut app, &ctx, "view.resetView", json!({})).unwrap();
        assert_eq!(r["rotation"], 0.0);
        assert_eq!(app.ui.views[0].rotation, 0.0);
        assert_eq!(app.ui.views[0].zoom, z);
        assert_eq!(app.ui.views[0].center, c);
        assert!(crate::menus::invoke(&mut app, &ctx, "view.rotateView", json!({})).is_err());
        assert!(crate::menus::invoke(&mut app, &ctx, "view.rotateView", json!({"angle": f64::NAN})).is_err());
        assert_eq!(app.ui.views[0].rotation, 0.0);
        assert_eq!(app.session.documents()[0].history.past_len(), 0);
    }

    #[test]
    fn ui_set_selects_the_tool_and_sets_rotation() {
        let ctx = egui::Context::default();
        let mut app = app_docs(1);
        let (req, _) = crate::control::ControlRequest::new("ui.set", json!({"tool": "RotateView", "rotation": 30}));
        let out = crate::control::handle(&mut app, &ctx, &req);
        let crate::control::Outcome::Done(v) = out else { panic!("done") };
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(app.ui.tool, crate::state::Tool::RotateView);
        assert_eq!(app.ui.views[0].rotation, 30.0);
        let (req, _) = crate::control::ControlRequest::new("ui.inspect", json!({}));
        let crate::control::Outcome::Done(ins) = crate::control::handle(&mut app, &ctx, &req) else { panic!("inspect") };
        assert_eq!(ins["result"]["tool"], "RotateView");
        assert_eq!(ins["result"]["views"][0]["rotation"], 30.0);
        let (req, _) = crate::control::ControlRequest::new("ui.set", json!({"rotation": f64::NAN}));
        let crate::control::Outcome::Done(bad) = crate::control::handle(&mut app, &ctx, &req) else { panic!("nan") };
        assert_eq!(bad["ok"], false);
        assert_eq!(app.ui.views[0].rotation, 30.0);
        let mut empty = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let (req, _) = crate::control::ControlRequest::new("ui.set", json!({"tool": "RotateView"}));
        let crate::control::Outcome::Done(ok) = crate::control::handle(&mut empty, &ctx, &req) else { panic!("tool") };
        assert_eq!(ok["ok"], true, "{ok}");
        assert_eq!(empty.ui.tool, crate::state::Tool::RotateView);
        let (req, _) = crate::control::ControlRequest::new("ui.set", json!({"rotation": 15}));
        let crate::control::Outcome::Done(err) = crate::control::handle(&mut empty, &ctx, &req) else { panic!("no doc") };
        assert_eq!(err["ok"], false);
        assert!(err["error"].as_str().unwrap().contains("no document open"), "{err}");
    }

    #[test]
    fn match_rotation_copies_the_angle_and_needs_two_documents() {
        let ctx = egui::Context::default();
        let one = app_docs(1);
        assert_eq!(crate::view_cmds::is_enabled(&one, "window.arrange.matchRotation"), Some(false));
        let mut app = app_docs(2);
        assert_eq!(crate::view_cmds::is_enabled(&app, "window.arrange.matchRotation"), Some(true));
        app.ui.views[1].rotation = 30.0;
        let r = crate::menus::invoke(&mut app, &ctx, "window.arrange.matchRotation", json!({})).unwrap();
        assert_eq!(r["rotation"], 30.0);
        assert!(app.ui.views.iter().all(|v| (v.rotation - 30.0).abs() < 1e-4));
        app.ui.views[1].rotation = 12.0;
        app.ui.views[1].zoom = 2.0;
        crate::menus::invoke(&mut app, &ctx, "window.arrange.matchAll", json!({})).unwrap();
        assert!(app.ui.views.iter().all(|v| (v.rotation - 12.0).abs() < 1e-4 && (v.zoom - 2.0).abs() < 1e-4));
    }

    fn harness() -> Harness<'static, PrismApp> {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.ui.extras.rulers = false;
        app.ui.tool = crate::state::Tool::RotateView;
        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
            |ui, app: &mut PrismApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        h
    }

    fn press(h: &mut Harness<'static, PrismApp>, p: Pos2, pressed: bool, shift: bool) {
        let modifiers = egui::Modifiers { shift, ..Default::default() };
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers });
        h.step();
    }

    #[test]
    fn shift_drag_snaps_to_fifteen_degrees() {
        let mut h = harness();
        let r = h.state().last_canvas_rect;
        let c = r.center();
        let p0 = c + vec2(120.0, 0.0);
        h.event(Event::PointerMoved(p0));
        h.step();
        h.event(Event::ModifiersChanged(egui::Modifiers::SHIFT));
        h.step();
        press(&mut h, p0, true, true);
        // ~40° clockwise: Shift rounds to 45°.
        let p1 = c + vec2(120.0 * 40f32.to_radians().cos(), 120.0 * 40f32.to_radians().sin());
        h.event(Event::PointerMoved(p1));
        h.step();
        let rot = h.state().ui.views[0].rotation;
        assert!((rot - 45.0).abs() < 0.75, "{rot}");
        press(&mut h, p1, false, true);
        h.run_steps(2);
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
        assert_eq!(crate::state::Tool::from_name("RotateView"), Some(crate::state::Tool::RotateView));
        assert_eq!(crate::state::Tool::RotateView.key(), 'R');
    }

    /// As measured in Photoshop 2026 (2026-11-16): Shift is read while the drag moves, not at
    /// the press. From −40°, a 20° sweep with Shift gave −15° (absolute multiples of 15°, not
    /// steps from the start); a raw +10° gave 15° (nearest, not floor); Shift pressed mid-drag
    /// snapped (raw 55° → 60°) and Shift released mid-drag freed the angle (raw 80° → 79°).
    #[test]
    fn shift_snaps_while_it_is_down_and_frees_the_angle_when_it_comes_up() {
        let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        let at = |deg: f32| c + vec2(120.0 * deg.to_radians().cos(), 120.0 * deg.to_radians().sin());
        let rot = |h: &Harness<'static, PrismApp>| h.state().ui.views[0].rotation;
        h.state_mut().ui.views[0].rotation = -40.0;
        h.event(Event::PointerMoved(at(0.0)));
        h.step();
        press(&mut h, at(0.0), true, false);
        h.event(Event::PointerMoved(at(20.0)));
        h.step();
        assert!((rot(&h) + 20.0).abs() < 0.75, "free: {}", rot(&h));
        // Shift goes down mid-drag: the nearest absolute multiple of 15°, not −40° + 15°.
        h.event(Event::ModifiersChanged(egui::Modifiers::SHIFT));
        h.event(Event::PointerMoved(at(20.5)));
        h.step();
        assert_eq!(rot(&h), -15.0);
        // Raw +10°: the nearest multiple is 15°, not 0°.
        h.event(Event::PointerMoved(at(50.0)));
        h.step();
        assert_eq!(rot(&h), 15.0);
        // Shift comes up mid-drag: the angle follows the pointer again.
        h.event(Event::ModifiersChanged(egui::Modifiers::NONE));
        h.event(Event::PointerMoved(at(119.0)));
        h.step();
        assert!((rot(&h) - 79.0).abs() < 0.75, "freed: {}", rot(&h));
        press(&mut h, at(119.0), false, false);
        h.run_steps(2);
        assert!((rot(&h) - 79.0).abs() < 0.75, "released: {}", rot(&h));
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
    }

    #[test]
    fn north_follows_the_document_top_and_survives_hostile_angles() {
        let close = |a: Vec2, b: Vec2| (a - b).length() < 1e-4;
        assert!(close(north(0.0, false), vec2(0.0, -1.0)));
        // 90° clockwise: the document's top is at the right of the screen; a flip mirrors it.
        assert!(close(north(90.0, false), vec2(1.0, 0.0)));
        assert!(close(north(90.0, true), vec2(-1.0, 0.0)));
        assert!(close(north(180.0, false), vec2(0.0, 1.0)));
        assert!(close(north(-90.0, false), vec2(-1.0, 0.0)));
        // The same direction `ViewXform` gives the document's up vector.
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        for rot in [0.0, 15.0, 33.0, 168.0, -47.0] {
            for flip in [false, true] {
                let xf = ViewXform { rect, zoom: 1.0, center: [0.0, 0.0], flip, rotation: rot };
                let up = xf.to_screen(0.0, -1.0) - xf.to_screen(0.0, 0.0);
                assert!(close(north(rot, flip), up), "rot={rot} flip={flip}: {:?} vs {up:?}", north(rot, flip));
            }
        }
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(close(north(bad, false), vec2(0.0, -1.0)), "{bad}");
        }
        assert!(north(1.0e30, true).length().is_finite());
    }

    #[test]
    fn needle_points_red_at_the_document_top_and_fits_small_views() {
        let c = pos2(200.0, 150.0);
        let [red, right, white, left] = needle(c, 0.0, false, 1.0);
        assert_eq!(red, c + vec2(0.0, -NEEDLE));
        assert_eq!(white, c + vec2(0.0, NEEDLE));
        assert!(left.x < c.x && right.x > c.x, "lit side on the left at 0°");
        let [red, ..] = needle(c, 90.0, false, 0.5);
        assert!((red - (c + vec2(NEEDLE * 0.5, 0.0))).length() < 1e-3, "{red:?}");
        // Full size where it fits, scaled down in a small view, absent in a tiny or hostile one.
        assert_eq!(compass_scale(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))), Some(1.0));
        let k = compass_scale(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 100.0))).expect("small view");
        assert!(k < 1.0 && 2.0 * NEEDLE * k < 100.0, "{k}");
        assert_eq!(compass_scale(Rect::from_min_size(Pos2::ZERO, vec2(20.0, 20.0))), None);
        assert_eq!(compass_scale(Rect::from_min_size(Pos2::ZERO, vec2(f32::NAN, 500.0))), None);
        assert_eq!(compass_scale(Rect::NOTHING), None);
    }

    /// Shapes the canvas painted this frame that are the drag compass's red needle facets.
    fn red_facets(h: &Harness<'static, PrismApp>) -> usize {
        let red = Tokens::rotate_compass().north;
        h.output().shapes.iter().filter(|s| matches!(&s.shape, Shape::Path(p) if red.contains(&p.fill))).count()
    }

    #[test]
    fn compass_shows_only_while_the_view_is_dragged() {
        let mut h = harness();
        let r = h.state().last_canvas_rect;
        let c = r.center();
        let p0 = c + vec2(120.0, 0.0);
        h.event(Event::PointerMoved(p0));
        h.step();
        assert_eq!(red_facets(&h), 0, "idle: no compass");
        assert!(!dragging(&h.ctx, r));
        press(&mut h, p0, true, false);
        let p1 = c + vec2(120.0 * 30f32.to_radians().cos(), 120.0 * 30f32.to_radians().sin());
        h.event(Event::PointerMoved(p1));
        h.step();
        h.step();
        assert!((h.state().ui.views[0].rotation - 30.0).abs() < 0.75, "{}", h.state().ui.views[0].rotation);
        assert_eq!(red_facets(&h), 2, "dragging: the needle's two red facets");
        assert!(dragging(&h.ctx, r));
        // Another view's canvas does not show this drag's compass.
        assert!(!dragging(&h.ctx, r.translate(vec2(300.0, 0.0))));
        press(&mut h, p1, false, false);
        h.run_steps(2);
        assert_eq!(red_facets(&h), 0, "released: the compass is gone, the angle stays");
        assert!(!dragging(&h.ctx, r));
        assert!((h.state().ui.views[0].rotation - 30.0).abs() < 0.75);
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
    }

    #[test]
    fn a_drag_without_its_release_does_not_leave_a_compass_behind() {
        let mut h = harness();
        let r = h.state().last_canvas_rect;
        h.ctx.data_mut(|d| d.insert_temp(id(), RotateDrag { pivot: r.center(), start_pointer: 0.0, start_rotation: 0.0 }));
        h.run_steps(2);
        assert_eq!(red_facets(&h), 0);
        assert!(h.ctx.data(|d| d.get_temp::<RotateDrag>(id())).is_none(), "the stale drag is dropped");
    }

    #[test]
    fn escape_and_reset_zero_the_angle_without_history() {
        let ctx = egui::Context::default();
        let mut app = app_docs(1);
        crate::menus::invoke(&mut app, &ctx, "view.rotateView", json!({"angle": 33})).unwrap();
        assert_eq!(app.session.documents()[0].history.past_len(), 0);
        crate::menus::invoke(&mut app, &ctx, "view.resetView", json!({})).unwrap();
        assert_eq!(app.ui.views[0].rotation, 0.0);
        assert_eq!(app.session.documents()[0].history.past_len(), 0);

        let mut h = harness();
        h.state_mut().ui.views[0].rotation = 40.0;
        h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.step();
        h.run_steps(2);
        assert!((h.state().ui.views[0].rotation).abs() < 0.05, "{}", h.state().ui.views[0].rotation);
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
    }

    #[test]
    fn rotated_view_keeps_engine_selection_in_document_space() {
        let mut app = app_docs(1);
        app.ui.views[0].rotation = 30.0;
        app.run("select.rect", json!({"x": 10, "y": 20, "width": 40, "height": 10})).unwrap();
        let st = app.session.active().unwrap();
        let b = st.doc.selection.as_ref().and_then(|s| prism_compose::bounds::content_bounds(s).into()).expect("selection");
        // Axis-aligned in the file: the outline is drawn rotated via ViewXform.
        assert!(b.width() >= 40 && b.height() >= 10, "{b:?}");
        let xf = ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), zoom: 1.0, center: [100.0, 50.0], flip: false, rotation: 30.0 };
        let a = xf.to_screen(10.0, 20.0);
        let c = xf.to_screen(50.0, 20.0);
        let v = c - a;
        let ang = v.y.atan2(v.x).to_degrees();
        assert!((wrap_deg(ang) - 30.0).abs() < 0.5, "{ang}");
        assert_eq!(st.history.past_len(), 1, "only the selection is a history step");
    }

    #[test]
    fn gpu_path_stays_on_with_rotation() {
        // Flip is the only CPU-blit fallback; rotation is a shader uniform.
        assert!(crate::gpu_canvas::gpu_view_ok(false, 45.0));
        assert!(!crate::gpu_canvas::gpu_view_ok(true, 45.0));
        assert!(crate::gpu_canvas::gpu_view_ok(false, 0.0));
    }
}
