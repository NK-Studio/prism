//! How a layer drag looks: the layers are picked up and carried, in one of two styles
//! (Preferences › Tools › Layer drag look, [`Style`]).
//!
//! **Card**
//! - Carried: a card with a picture of the dragged layers (a group shows its contents) rises
//!   from the pointer, tilted, with a shadow, and follows it; several layers make a stack.
//! - Over another document: the card gives way to the layers themselves, translucent, where they
//!   will land and at the size they will have in that view (its zoom, rotation and flip), inside
//!   that document's window.
//! - Dropped: they settle there, the picture fading into the copies while their outline glows
//!   and fades.
//!
//! **Ghost** (Photoshop)
//! - Carried: the layers' own pixels, translucent and at the size they had in their view, hang
//!   from the pointer by the point that was grabbed, outlined with marching ants; a badge counts
//!   several layers.
//! - Over another document: the same ghost where they will land, at that view's size.
//! - Dropped: the outline flashes and the ghost pops into place as it fades.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Context, Id, Pos2, Rect, Shape, Stroke, TextureHandle, ViewportId, vec2};
use prism_doc::{Document, Layer, LayerId};

use crate::canvas::ViewXform;

pub(super) use prism_engine::prefs::LayerDragLook as Style;

/// Longest side of the picture, in texels.
const SIDE: f32 = 320.0;
/// Layers sampled into the picture at most (a deep group shows its first ones).
const MAX_LAYERS: usize = 64;
/// Group nesting followed at most.
const MAX_DEPTH: usize = 32;
/// How long the card takes to rise, and the copies to settle (seconds).
const LIFT: f64 = 0.16;
const SETTLE: f64 = 0.45;
/// The card's tilt (radians) and largest side (points).
const TILT: f32 = -0.06;
const CARD: f32 = 112.0;
/// A ghost dragged from the Layers panel is at most this big (points).
const GHOST: f32 = 240.0;
/// Marching ants: dash and gap lengths (points), and their speed (points per second).
const ANT: f32 = 4.0;
const MARCH: f64 = 16.0;

/// A picture of the dragged layers and where they are in their document.
#[derive(Clone)]
pub(super) struct Preview {
    pub tex: TextureHandle,
    /// Document pixels.
    pub bounds: Rect,
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview").field("size", &self.tex.size()).field("bounds", &self.bounds).finish()
    }
}

/// The visible pixel layers of `ids` (bottom to top), with the opacity they show at.
fn collect<'a>(layers: &'a [Layer], ids: &[LayerId], chosen: bool, alpha: f32, depth: usize, out: &mut Vec<(&'a prism_raster::Surface, f32)>) {
    if depth > MAX_DEPTH {
        return;
    }
    for l in layers {
        if out.len() >= MAX_LAYERS {
            return;
        }
        let picked = chosen || ids.contains(&l.id);
        if picked && !l.visible {
            continue;
        }
        let a = if picked { alpha * l.opacity.clamp(0.0, 1.0) } else { alpha };
        match l.children() {
            Some(children) => collect(children, ids, picked, a, depth + 1, out),
            None if picked => {
                if let Some(s) = l.surface() {
                    out.push((s, a * l.fill_opacity.clamp(0.0, 1.0)));
                }
            }
            None => {}
        }
    }
}

/// The picture of layers `ids` of `doc`, `None` when they show no pixels.
pub(super) fn preview(ctx: &Context, doc: &Document, ids: &[LayerId]) -> Option<Preview> {
    let mut bounds: Option<prism_geom::Rect> = None;
    for id in ids {
        if let Some(b) = doc.layer(*id).and_then(prism_engine::layer_multi_cmds::layer_bounds) {
            bounds = Some(bounds.map_or(b, |a| a.union(&b)));
        }
    }
    let b = bounds.filter(|b| !b.is_empty())?;
    let mut leaves = Vec::new();
    collect(&doc.layers, ids, false, 1.0, 0, &mut leaves);
    if leaves.is_empty() {
        return None;
    }
    let (bw, bh) = (b.width() as f32, b.height() as f32);
    let scale = (bw.max(bh) / SIDE).max(1.0);
    let (w, h) = (((bw / scale).ceil() as usize).max(1), ((bh / scale).ceil() as usize).max(1));
    let mut pixels = Vec::with_capacity(w * h);
    let mut px = [0.0f32; 16];
    for y in 0..h {
        let dy = b.y0 + ((y as f32 + 0.5) * scale) as i32;
        for x in 0..w {
            let dx = b.x0 + ((x as f32 + 0.5) * scale) as i32;
            // Premultiplied "over", bottom to top.
            let mut acc = [0.0f32; 4];
            for (s, alpha) in &leaves {
                let Some(px) = px.get_mut(..s.channels()) else { continue };
                s.read_pixel(dx, dy, px);
                let c = prism_raster::to_rgba(&s.format(), px);
                let a = (c[3] * alpha).clamp(0.0, 1.0);
                for i in 0..3 {
                    acc[i] = c[i].clamp(0.0, 1.0) * a + acc[i] * (1.0 - a);
                }
                acc[3] = a + acc[3] * (1.0 - a);
            }
            let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels.push(Color32::from_rgba_premultiplied(to(acc[0]), to(acc[1]), to(acc[2]), to(acc[3])));
        }
    }
    let image = egui::ColorImage::new([w, h], pixels);
    let tex = ctx.load_texture("layer-transfer-preview", image, egui::TextureOptions::LINEAR);
    Some(Preview { tex, bounds: Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)) })
}

/// The corners of document rectangle `r` in a view (top left, top right, bottom right, bottom
/// left): rotated and flipped with the view.
pub(super) fn quad(xf: &ViewXform, r: Rect) -> [Pos2; 4] {
    [xf.to_screen(r.min.x, r.min.y), xf.to_screen(r.max.x, r.min.y), xf.to_screen(r.max.x, r.max.y), xf.to_screen(r.min.x, r.max.y)]
}

fn painter(ctx: &Context, name: &str) -> egui::Painter {
    ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new(("layer-transfer", name))))
}

/// `tex` stretched over the quad `q`, tinted (premultiplied) `tint`.
fn picture(tex: &TextureHandle, q: [Pos2; 4], tint: Color32) -> Shape {
    let mut mesh = Mesh::with_texture(tex.id());
    for (pos, uv) in q.into_iter().zip([egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)]) {
        mesh.vertices.push(Vertex { pos, uv, color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

/// `r` turned by `angle` (radians) about its centre, as a quad.
fn turned(r: Rect, angle: f32) -> [Pos2; 4] {
    let (c, rot) = (r.center(), egui::emath::Rot2::from_angle(angle));
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| c + rot * (p - c))
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The carried layers at `pointer`, `since` seconds after the drag began. `anchor`: the picture's
/// top left from the grabbed point (document pixels) and the zoom of the view it came from (a
/// Move-tool drag; `None`: from the Layers panel).
#[allow(clippy::too_many_arguments)]
pub(super) fn carry(ctx: &Context, style: Style, pointer: Pos2, preview: Option<&Preview>, label: &str, count: usize, since: f64, anchor: Option<(egui::Vec2, f32)>) {
    match style {
        Style::Card => card(ctx, pointer, preview, label, count, since),
        Style::Ghost => ghost(ctx, pointer, preview, label, count, since, anchor),
    }
}

/// The carried card at `pointer`, `since` seconds after the drag began.
fn card(ctx: &Context, pointer: Pos2, preview: Option<&Preview>, label: &str, count: usize, since: f64) {
    let t = crate::theme::Tokens::get(ctx);
    let lift = ease_out((since / LIFT) as f32);
    if lift < 1.0 {
        ctx.request_repaint();
    }
    let p = painter(ctx, "card");
    let mut top = pointer + vec2(18.0, 16.0);
    if let Some(pv) = preview {
        let size = pv.bounds.size();
        let fit = (CARD / size.x.max(size.y).max(1.0)).min(1.0) * (0.9 + 0.1 * lift);
        let pic = vec2((size.x * fit).max(28.0), (size.y * fit).max(28.0));
        let frame = Rect::from_min_size(top, pic + vec2(8.0, 8.0));
        let angle = TILT * lift;
        // A stack: one card behind for each further layer, up to two.
        for i in (1..count.min(3)).rev() {
            let k = i as f32;
            let behind = turned(frame.translate(vec2(4.0, 4.0) * k), angle + 0.05 * k);
            p.add(Shape::convex_polygon(behind.to_vec(), t.card.gamma_multiply(0.85 - 0.15 * k), Stroke::new(1.0, t.card_border)));
        }
        let shadow = egui::Shadow { offset: [0, (8.0 * lift) as i8], blur: (6.0 + 14.0 * lift) as u8, spread: 0, color: t.shadow.gamma_multiply(0.4 + 0.6 * lift) };
        p.add(shadow.as_shape(frame, 4.0));
        p.add(Shape::convex_polygon(turned(frame, angle).to_vec(), t.card, Stroke::new(1.0, t.accent)));
        p.add(picture(&pv.tex, turned(frame.shrink(4.0), angle), Color32::WHITE));
        top = frame.left_bottom() + vec2(0.0, 6.0);
    }
    let g = p.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), t.text);
    let chip = Rect::from_min_size(top, g.size() + vec2(16.0, 8.0));
    p.rect_filled(chip, t.radius_sm, t.card.gamma_multiply(0.95));
    p.rect_stroke(chip, t.radius_sm, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    p.galley(chip.min + vec2(8.0, 4.0), g, t.text);
}

/// The marching ants around quad `q`, `phase` points along.
fn ants(p: &egui::Painter, q: [Pos2; 4], phase: f32) {
    let mut path = q.to_vec();
    path.push(q[0]);
    p.add(Shape::line(path.clone(), Stroke::new(1.0, Color32::BLACK)));
    p.extend(Shape::dashed_line_with_offset(&path, Stroke::new(1.0, Color32::WHITE), &[ANT], &[ANT], phase));
}

fn phase(ctx: &Context) -> f32 {
    ((ctx.input(|i| i.time) * MARCH) % f64::from(2.0 * ANT)) as f32
}

/// The carried ghost: the layers hanging from the pointer, at their view's size.
fn ghost(ctx: &Context, pointer: Pos2, preview: Option<&Preview>, label: &str, count: usize, since: f64, anchor: Option<(egui::Vec2, f32)>) {
    let t = crate::theme::Tokens::get(ctx);
    ctx.request_repaint();
    let p = painter(ctx, "ghost");
    let rise = ease_out((since / LIFT) as f32);
    let badge_at = match preview {
        Some(pv) => {
            let size = pv.bounds.size();
            let r = match anchor {
                Some((offset, zoom)) => Rect::from_min_size(pointer + offset * zoom, size * zoom),
                None => {
                    let fit = (GHOST / size.x.max(size.y).max(1.0)).min(1.0);
                    Rect::from_center_size(pointer, size * fit)
                }
            };
            p.add(picture(&pv.tex, [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], Color32::from_white_alpha((150.0 * rise) as u8)));
            ants(&p, [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], phase(ctx));
            pointer
        }
        None => pointer,
    };
    // The count (or the name, with nothing to picture) by the pointer.
    let text = if count > 1 || preview.is_none() { label.to_string() } else { return };
    let g = p.layout_no_wrap(text, egui::FontId::proportional(11.5), Color32::WHITE);
    let pill = Rect::from_min_size(badge_at + vec2(14.0, 14.0), g.size() + vec2(14.0, 6.0));
    p.rect_filled(pill, pill.height() / 2.0, t.accent);
    p.galley(pill.min + vec2(7.0, 3.0), g, Color32::WHITE);
}

/// The layers where they will land (`q`, in this view), and that document's canvas outlined.
pub(super) fn landing(ctx: &Context, style: Style, canvas: Rect, q: [Pos2; 4], preview: &Preview) {
    let t = crate::theme::Tokens::get(ctx);
    let p = painter(ctx, "landing").with_clip_rect(canvas);
    match style {
        Style::Card => {
            p.rect_stroke(canvas, 0.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            p.add(picture(&preview.tex, q, Color32::from_white_alpha(190)));
            p.add(Shape::closed_line(q.to_vec(), Stroke::new(1.5, t.accent)));
        }
        Style::Ghost => {
            ctx.request_repaint();
            p.rect_stroke(canvas, 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            p.add(picture(&preview.tex, q, Color32::from_white_alpha(150)));
            ants(&p, q, phase(ctx));
        }
    }
}

/// Copies settling after a drop: drawn in `viewport` for a moment.
#[derive(Clone)]
struct Settle {
    style: Style,
    viewport: ViewportId,
    quad: [Pos2; 4],
    tex: TextureHandle,
    start: f64,
}

fn settle_key() -> Id {
    Id::new("layer-transfer-settle")
}

/// The dragged layers were copied to where `q` is in `viewport`.
pub(super) fn settle(ctx: &Context, style: Style, viewport: ViewportId, q: [Pos2; 4], preview: &Preview) {
    let start = ctx.input_for(viewport, |i| i.time);
    ctx.data_mut(|d| d.insert_temp(settle_key(), Settle { style, viewport, quad: q, tex: preview.tex.clone(), start }));
    ctx.request_repaint();
}

/// The window the copies are settling in.
#[cfg(test)]
pub(super) fn settling(ctx: &Context) -> Option<ViewportId> {
    ctx.data(|d| d.get_temp::<Settle>(settle_key())).map(|s| s.viewport)
}

/// Every frame in every window: the settling copies, while they settle in this one.
pub(super) fn paint_settle(ctx: &Context) {
    let Some(s) = ctx.data(|d| d.get_temp::<Settle>(settle_key())) else { return };
    if s.viewport != ctx.viewport_id() {
        return;
    }
    let k = ((ctx.input(|i| i.time) - s.start) / SETTLE) as f32;
    if !(0.0..1.0).contains(&k) {
        ctx.data_mut(|d| d.remove::<Settle>(settle_key()));
        return;
    }
    ctx.request_repaint();
    let t = crate::theme::Tokens::get(ctx);
    let fade = 1.0 - ease_out(k);
    let p = painter(ctx, "settle");
    let c = s.quad.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / 4.0;
    let scaled = |by: f32| s.quad.map(|p| (c + (p.to_vec2() - c) * by).to_pos2());
    match s.style {
        Style::Card => {
            p.add(picture(&s.tex, s.quad, Color32::from_white_alpha((190.0 * fade) as u8)));
            // The outline glows outwards as it fades.
            p.add(Shape::closed_line(scaled(1.0 + 0.04 * ease_out(k)).to_vec(), Stroke::new(1.5 + 2.0 * fade, t.accent.gamma_multiply(fade))));
        }
        Style::Ghost => {
            // A pop: the ghost shrinks into place while the outline flashes.
            let q = scaled(1.0 + 0.05 * fade);
            p.add(Shape::convex_polygon(q.to_vec(), t.accent.gamma_multiply(0.35 * fade), Stroke::NONE));
            p.add(picture(&s.tex, q, Color32::from_white_alpha((150.0 * fade) as u8)));
            p.add(Shape::closed_line(q.to_vec(), Stroke::new(1.0 + 2.0 * fade, Color32::WHITE.gamma_multiply(fade))));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_group_pictures_its_visible_contents_where_they_are() {
        let mut s = prism_engine::Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        let red = LayerId(s.execute("layer.new.layer", json!({"name": "red"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let hidden = LayerId(s.execute("layer.new.layer", json!({"name": "hidden"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("select.rect", json!({"x": 20, "y": 6, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.setProps", json!({"layer": hidden.0, "visible": false})).unwrap();
        let group = LayerId(s.execute("layer.groupLayers", json!({"layer": red.0})).unwrap()["layer"].as_u64().unwrap());
        let ctx = Context::default();
        let doc = &s.active().unwrap().doc;
        let p = preview(&ctx, doc, &[group]).unwrap();
        assert_eq!(p.bounds, Rect::from_min_max(egui::pos2(4.0, 6.0), egui::pos2(14.0, 16.0)));
        assert_eq!(p.tex.size(), [10, 10], "small layers at their own size");
        // Hidden or empty: nothing to picture.
        assert!(preview(&ctx, doc, &[hidden]).is_none());
        let bg = doc.layers[0].id;
        assert!(preview(&ctx, doc, &[bg]).is_some());
    }

    #[test]
    fn a_large_layer_is_pictured_small_and_a_quad_follows_the_view() {
        let mut s = prism_engine::Session::new();
        s.execute("file.new", json!({"width": 3000, "height": 1500})).unwrap();
        let ctx = Context::default();
        let doc = &s.active().unwrap().doc;
        let p = preview(&ctx, doc, &[doc.layers[0].id]).unwrap();
        assert_eq!(p.tex.size(), [320, 160]);
        let xf = ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), zoom: 0.5, center: [100.0, 50.0], flip: false, rotation: 0.0 };
        let q = quad(&xf, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 50.0)));
        assert_eq!((q[2] - q[0]), vec2(50.0, 25.0), "at the view's zoom");
    }
}
