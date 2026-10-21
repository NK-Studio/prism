//! CLIP STUDIO special rulers on the canvas: brush strokes snap to the rulers that apply to the
//! layer being painted (View › Snap To › Special Ruler), and the rulers are drawn over the image.
//!
//! The snap happens where ⇧ constrains strokes (`stroke_constraint`): pointer positions are
//! snapped before they enter the drag, so the live stroke and the committed `paint.stroke` get
//! the same points. ⇧ wins over the ruler. Geometry: `prism_engine::special_ruler_snap`.

use egui::{Color32, Pos2, Stroke, vec2};
use prism_doc::special_ruler::{RulerKind, VanishTarget, angle_dir};
use prism_doc::{Document, LayerId};
use prism_engine::special_ruler_cmds::LOCK_SCREEN_DISTANCE;
use prism_engine::special_ruler_snap::{StrokeSnapper, candidates, rulers_for};

use crate::PrismApp;
use crate::canvas::{Drag, ViewXform};
use crate::state::Tool;

/// Ruler lines over the image, in CLIP STUDIO's violet. A canvas overlay colour like the guide
/// and smart-guide defaults (`rulers.rs`, `snap_ui.rs`), not a chrome token.
const RULER: Color32 = Color32::from_rgb(0x9b, 0x5c, 0xf6);
/// The line a stroke has locked to.
const LOCKED: Color32 = Color32::from_rgb(0xd9, 0x46, 0xef);
/// Handles further away than this are not drawn (parallel points keep far-off handles).
const MAX_DRAWN: f64 = 1e6;

/// Strokes of `tool` snap to special rulers right now.
pub fn snaps(app: &PrismApp, tool: Tool) -> bool {
    tool.is_brushlike() && app.ui.extras.snap && app.ui.view.snap_to.special_ruler
}

/// The snapper for a stroke of `tool` starting at `start` on the active layer; `None` when
/// snapping is off or no ruler applies.
pub fn snapper(app: &PrismApp, tool: Tool, start: [f64; 2]) -> Option<StrokeSnapper> {
    if !snaps(app, tool) {
        return None;
    }
    let st = app.session.active()?;
    let lines = candidates(&st.doc, st.active_layer?, start);
    let zoom = f64::from(app.current_zoom()).max(1e-3);
    (!lines.is_empty()).then(|| StrokeSnapper::new(lines, start, LOCK_SCREEN_DISTANCE / zoom))
}

/// Pointer position `p` of drag `d`, snapped to its ruler unless ⇧ constrains the stroke.
pub fn snap(d: &mut Drag, p: [f64; 2], shift: bool) -> [f64; 2] {
    match &mut d.ruler {
        Some(r) if !shift => r.push(p),
        _ => p,
    }
}

fn near(p: [f64; 2]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_DRAWN)
}

/// A line through document point `p` along pixel direction `dir`, across the visible canvas.
fn line(painter: &egui::Painter, xf: &ViewXform, p: [f64; 2], dir: [f64; 2], stroke: Stroke) {
    if !near(p) {
        return;
    }
    let a = xf.to_screen(p[0] as f32, p[1] as f32);
    let b = xf.to_screen((p[0] + dir[0]) as f32, (p[1] + dir[1]) as f32);
    let d = b - a;
    let len = d.length();
    if !(len.is_finite() && len > 1e-6) {
        return;
    }
    let clip = painter.clip_rect();
    // Long enough to cross the view from any point on it.
    let reach = clip.size().length() + a.distance(clip.center());
    let u = d / len;
    painter.line_segment([a - u * reach, a + u * reach], stroke);
}

fn handle(painter: &egui::Painter, at: Pos2, color: Color32) {
    painter.circle_stroke(at, 4.5, Stroke::new(1.5, color));
    painter.line_segment([at - vec2(3.0, 0.0), at + vec2(3.0, 0.0)], Stroke::new(1.0, color));
    painter.line_segment([at - vec2(0.0, 3.0), at + vec2(0.0, 3.0)], Stroke::new(1.0, color));
}

/// Draw every ruler that applies to `layer`, and the line the current stroke follows.
pub fn draw(app: &PrismApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document, layer: Option<LayerId>) {
    let Some(layer) = layer else { return };
    for (_, set) in rulers_for(doc, layer) {
        for r in &set.rulers {
            let color = if r.snap { RULER } else { RULER.gamma_multiply(0.45) };
            let stroke = Stroke::new(1.0, color);
            match &r.kind {
                RulerKind::Parallel { angle_deg, center } => {
                    line(painter, xf, *center, angle_dir(*angle_deg), stroke);
                    if near(*center) {
                        handle(painter, xf.to_screen(center[0] as f32, center[1] as f32), color);
                    }
                }
                RulerKind::Perspective(per) => {
                    if let Some(e) = per.eye_level.filter(|_| per.points.iter().any(|v| matches!(v.target, VanishTarget::Point(_)))) {
                        line(painter, xf, e, [1.0, 0.0], stroke);
                    }
                    for vp in &per.points {
                        let color = if r.snap && vp.snap { color } else { RULER.gamma_multiply(0.45) };
                        let stroke = Stroke::new(1.0, color);
                        match vp.target {
                            VanishTarget::Parallel { angle_deg } => {
                                let dir = angle_dir(angle_deg);
                                for g in vp.guides.iter().chain(std::iter::once(&vp.handle)) {
                                    line(painter, xf, *g, dir, stroke);
                                }
                            }
                            VanishTarget::Point(q) => {
                                for g in &vp.guides {
                                    line(painter, xf, *g, [q[0] - g[0], q[1] - g[1]], stroke);
                                }
                                if near(q) {
                                    handle(painter, xf.to_screen(q[0] as f32, q[1] as f32), color);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    handles::draw(app, painter, xf, doc, layer, RULER);
    if let Some(l) = app.drag.as_ref().and_then(|d| d.ruler.as_ref()).and_then(StrokeSnapper::locked) {
        line(painter, xf, l.origin, l.dir, Stroke::new(1.5, LOCKED));
    }
}

pub mod handles;

#[cfg(test)]
mod tests;
