//! Canvas cursors must move with the OS pointer, independently of the canvas frame rate.
//! Windows' stock crosshair inverts grey into grey, so use our black/white bitmap there too.

use std::sync::Arc;

use egui::{CursorIcon, CustomCursorImage, Painter, Pos2, Stroke, vec2};

/// winit's cursor limit. Keep allocations bounded even for hostile brush sizes or zooms.
const MAX_SIDE: u16 = 2048;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Circle { radius: f32, centre: bool },
    Crosshair { length: f32, gap: f32 },
    Move,
}

#[derive(Clone)]
struct Cached {
    shape: Shape,
    scale: f32,
    image: CustomCursorImage,
}

/// The image is sticky in egui's platform output. Clear it every pass, including when a
/// document closes or the pointer leaves the canvas; a later widget's cursor takes precedence.
pub(crate) struct CursorLifecycle;

pub(crate) fn fallback_key(ctx: &egui::Context) -> egui::Id {
    egui::Id::new(("tool-cursor-fallback", ctx.viewport_id()))
}

impl egui::plugin::Plugin for CursorLifecycle {
    fn debug_name(&self) -> &'static str {
        "Prism tool cursor"
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        ui.ctx().set_cursor_image(None);
        crate::cursor_pack::clear(ui.ctx());
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let id = fallback_key(ui.ctx());
        let fallback = ui.ctx().data(|d| d.get_temp::<CursorIcon>(id)).unwrap_or(CursorIcon::Crosshair);
        let packed = crate::cursor_pack::active(ui.ctx()).is_some();
        let overridden = ui.ctx().output(|o| (o.cursor_image.is_some() || packed) && o.cursor_icon != CursorIcon::None);
        if overridden {
            crate::cursor_pack::clear(ui.ctx());
        }
        crate::cursor_pack::finish_pass(ui.ctx());
        ui.ctx().output_mut(|o| {
            if o.cursor_image.is_some() {
                if o.cursor_icon == CursorIcon::None {
                    // None marks our request until all widgets have had their say. A backend
                    // that cannot upload the bitmap must still leave a visible pointer.
                    o.cursor_icon = fallback;
                } else {
                    o.cursor_image = None;
                }
            }
        });
    }
}

pub(crate) fn circle(painter: &Painter, at: Pos2, radius: f32, centre: bool) -> CursorIcon {
    show(painter, at, Shape::Circle { radius, centre })
}

pub(crate) fn crosshair(painter: &Painter, at: Pos2, length: f32, gap: f32) -> CursorIcon {
    if gap == 0.0
        && (5.0..=8.0).contains(&length)
        && let Some(icon) = crate::cursor_pack::show(painter.ctx(), at, 1025, CursorIcon::Crosshair)
    {
        return icon;
    }
    show(painter, at, Shape::Crosshair { length, gap })
}

/// A clean-room arrow with a four-way move badge. The hotspot is the arrow's tip.
pub(crate) fn move_tool(painter: &Painter, at: Pos2, dragging: bool) -> CursorIcon {
    if let Some(icon) = crate::cursor_pack::show(painter.ctx(), at, if dragging { 1004 } else { 1059 }, CursorIcon::Default) {
        return icon;
    }
    if dragging { CursorIcon::Default } else { show(painter, at, Shape::Move) }
}

fn show(painter: &Painter, at: Pos2, shape: Shape) -> CursorIcon {
    let ctx = painter.ctx();
    let fallback_key = fallback_key(ctx);
    ctx.data_mut(|d| d.insert_temp(fallback_key, if shape == Shape::Move { CursorIcon::Default } else { CursorIcon::Crosshair }));
    // The native backend uploads cursor images in root and immediate document viewports.
    // Unsupported/oversize artwork retains the painter fallback below.
    if cfg!(target_os = "windows") || shape == Shape::Move {
        let scale = native_raster_scale(ctx.pixels_per_point());
        if let Some(image) = cached_image(ctx, shape, scale) {
            ctx.set_cursor_image(Some(image));
            ctx.data_mut(|d| d.insert_temp(fallback_key, if shape == Shape::Move { CursorIcon::Default } else { CursorIcon::Crosshair }));
            return CursorIcon::None;
        }
        // An enormous tip cannot fit an OS cursor. Preserve its full-size outline, with an OS
        // crosshair at the hotspot so the user still gets immediate position feedback.
        paint(painter, at, shape);
        if let Some(image) = cached_image(ctx, Shape::Crosshair { length: 6.0, gap: 2.0 }, scale) {
            ctx.set_cursor_image(Some(image));
        }
        return CursorIcon::None;
    }
    paint(painter, at, shape);
    CursorIcon::None
}

/// winit's AppKit backend uses bitmap dimensions for NSImage's size in points,
/// including its hotspot. AppKit then applies the screen's backing scale itself.
/// Windows expects physical pixels instead. The canvas fallback below still needs
/// pixels_per_point for its texture, since the painter converts it back to points.
fn native_raster_scale(pixels_per_point: f32) -> f32 {
    if cfg!(target_os = "macos") { 1.0 } else { pixels_per_point }
}

fn cached_image(ctx: &egui::Context, shape: Shape, scale: f32) -> Option<CustomCursorImage> {
    let id = egui::Id::new("prism-tool-cursor");
    if let Some(cached) = ctx.data(|d| d.get_temp::<Cached>(id))
        && cached.shape == shape
        && cached.scale == scale
    {
        return Some(cached.image);
    }
    let image = rasterize(shape, scale)?;
    ctx.data_mut(|d| d.insert_temp(id, Cached { shape, scale, image: image.clone() }));
    Some(image)
}

fn paint(painter: &Painter, at: Pos2, shape: Shape) {
    if shape == Shape::Move {
        let ctx = painter.ctx();
        let scale = ctx.pixels_per_point();
        let id = egui::Id::new("move-cursor-texture");
        let cached = ctx.data(|d| d.get_temp::<(f32, egui::TextureHandle)>(id));
        let texture = cached.filter(|(s, _)| *s == scale).map(|(_, t)| t).or_else(|| {
            let image = rasterize_move(scale)?;
            let tex = ctx.load_texture(
                "move-cursor",
                egui::ColorImage::from_rgba_unmultiplied([usize::from(image.size[0]), usize::from(image.size[1])], &image.rgba),
                egui::TextureOptions::LINEAR,
            );
            ctx.data_mut(|d| d.insert_temp(id, (scale, tex.clone())));
            Some(tex)
        });
        if let Some(tex) = texture {
            let min = at - vec2(2.0, 2.0);
            painter.image(
                tex.id(),
                egui::Rect::from_min_size(min, tex.size_vec2() / scale),
                egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        return;
    }
    if extent(shape).is_none() {
        return;
    }
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    for (width, color) in [(3.0, dark), (1.0, light)] {
        let stroke = Stroke::new(width, color);
        match shape {
            Shape::Circle { radius, centre } => {
                painter.circle_stroke(at, radius, stroke);
                if centre {
                    paint_crosshair(painter, at, 3.0, 0.0, stroke);
                }
            }
            Shape::Crosshair { length, gap } => paint_crosshair(painter, at, length, gap, stroke),
            Shape::Move => {}
        }
    }
}

fn paint_crosshair(painter: &Painter, at: Pos2, length: f32, gap: f32, stroke: Stroke) {
    for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
        painter.line_segment([at + d * gap, at + d * length], stroke);
    }
}

/// Distance to the nearest arm (round caps), in physical pixels from the hotspot.
fn cross_distance(x: f32, y: f32, length: f32, gap: f32) -> f32 {
    let horizontal = (x - x.clamp(gap, length)).hypot(y);
    let vertical = x.hypot(y - y.clamp(gap, length));
    horizontal.min(vertical)
}

fn extent(shape: Shape) -> Option<f32> {
    Some(match shape {
        Shape::Circle { radius, centre } => {
            if !radius.is_finite() || radius <= 0.0 {
                return None;
            }
            if centre { radius.max(3.0) } else { radius }
        }
        Shape::Crosshair { length, gap } => {
            if !length.is_finite() || !gap.is_finite() || gap < 0.0 || length < gap {
                return None;
            }
            length
        }
        Shape::Move => 30.0,
    })
}

fn rasterize(shape: Shape, scale: f32) -> Option<CustomCursorImage> {
    if shape == Shape::Move {
        return rasterize_move(scale);
    }
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let extent = extent(shape)?;
    // Odd dimensions put the hotspot on the centre pixel at every DPI. Include the outline
    // and its antialiasing fringe, then check before converting or allocating.
    let half = ((extent + 1.5) * scale + 1.0).ceil();
    if !half.is_finite() || half > f32::from((MAX_SIDE - 1) / 2) {
        return None;
    }
    let hot = half as u16;
    let side = hot * 2 + 1;
    let bytes = usize::from(side).checked_mul(usize::from(side))?.checked_mul(4)?;
    let mut rgba = Vec::with_capacity(bytes);
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    for row in 0..side {
        let y = (f32::from(row) - f32::from(hot)).abs();
        for col in 0..side {
            let x = (f32::from(col) - f32::from(hot)).abs();
            let distance = match shape {
                Shape::Circle { radius, centre } => {
                    let ring = (x.hypot(y) - radius * scale).abs();
                    if centre { ring.min(cross_distance(x, y, 3.0 * scale, 0.0)) } else { ring }
                }
                Shape::Crosshair { length, gap } => cross_distance(x, y, length * scale, gap * scale),
                Shape::Move => 0.0,
            };
            let a_dark = (1.5 * scale + 0.5 - distance).clamp(0.0, 1.0) * f32::from(dark.a()) / 255.0;
            let a_light = (0.5 * scale + 0.5 - distance).clamp(0.0, 1.0) * f32::from(light.a()) / 255.0;
            let alpha = a_light + a_dark * (1.0 - a_light);
            // Straight RGBA, as required by winit, not egui's premultiplied representation.
            let white = if alpha > 0.0 { (255.0 * a_light / alpha).round() as u8 } else { 0 };
            rgba.extend_from_slice(&[white, white, white, (255.0 * alpha).round() as u8]);
        }
    }
    Some(CustomCursorImage { rgba: Arc::from(rgba), size: [side, side], hotspot: [hot, hot] })
}

#[cfg(test)]
mod tests;

/// Signed distance to a small fixed polygon (negative inside). No input-sized allocation.
fn polygon_distance(p: [f32; 2], vertices: &[[f32; 2]]) -> f32 {
    let mut inside = false;
    let mut nearest = f32::MAX;
    for (a, b) in vertices.iter().zip(vertices.iter().cycle().skip(1)).take(vertices.len()) {
        let ab = [b[0] - a[0], b[1] - a[1]];
        let ap = [p[0] - a[0], p[1] - a[1]];
        let len = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if len > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1]) / len).clamp(0.0, 1.0) } else { 0.0 };
        nearest = nearest.min((ap[0] - t * ab[0]).hypot(ap[1] - t * ab[1]));
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * ab[0] / ab[1] {
            inside = !inside;
        }
    }
    if inside { -nearest } else { nearest }
}

fn rasterize_move(scale: f32) -> Option<CustomCursorImage> {
    if !scale.is_finite() || scale <= 0.0 || 32.0 * scale > f32::from(MAX_SIDE) {
        return None;
    }
    let side = (32.0 * scale).ceil() as u16;
    let hotspot = (2.0 * scale).round() as u16;
    let mut rgba = Vec::with_capacity(usize::from(side).checked_mul(usize::from(side))?.checked_mul(4)?);
    let arrow = [[0.0, 0.0], [0.0, 17.0], [4.0, 13.0], [8.0, 21.0], [11.0, 19.5], [7.0, 11.5], [14.0, 11.5]];
    let badge = [
        [19.0, 15.0],
        [15.5, 18.5],
        [18.0, 18.5],
        [18.0, 21.0],
        [15.5, 21.0],
        [15.5, 18.5],
        [12.0, 22.0],
        [15.5, 25.5],
        [15.5, 23.0],
        [18.0, 23.0],
        [18.0, 25.5],
        [15.5, 25.5],
        [19.0, 29.0],
        [22.5, 25.5],
        [20.0, 25.5],
        [20.0, 23.0],
        [22.5, 23.0],
        [22.5, 25.5],
        [26.0, 22.0],
        [22.5, 18.5],
        [22.5, 21.0],
        [20.0, 21.0],
        [20.0, 18.5],
        [22.5, 18.5],
    ];
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    for y in 0..side {
        for x in 0..side {
            let p = [(f32::from(x) - f32::from(hotspot)) / scale, (f32::from(y) - f32::from(hotspot)) / scale];
            let distance = polygon_distance(p, &arrow).min(polygon_distance(p, &badge));
            let alpha = ((1.25 - distance) * scale + 0.5).clamp(0.0, 1.0);
            let edge = ((0.55 - distance.abs()) * scale + 0.5).clamp(0.0, 1.0);
            let value = |a: u8, b: u8| (f32::from(a) * (1.0 - edge) + f32::from(b) * edge).round() as u8;
            rgba.extend_from_slice(&[value(dark.r(), light.r()), value(dark.g(), light.g()), value(dark.b(), light.b()), (alpha * 255.0).round() as u8]);
        }
    }
    Some(CustomCursorImage { rgba: Arc::from(rgba), size: [side, side], hotspot: [hotspot, hotspot] })
}
