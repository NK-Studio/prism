//! Canvas cursors must move with the OS pointer, independently of the canvas frame rate.
//! Windows' stock crosshair inverts grey into grey, so use our black/white bitmap there too.
//!
//! Each tool's own cursor is a drawing (`art.rs`, from `assets/cursors/*.svg`); the brush tips
//! and their crosshairs are sized by the brush and the zoom, so they are computed here.

mod art;
pub(crate) use art::Art;

use std::sync::Arc;

use egui::{CursorIcon, CustomCursorImage, Painter, Pos2, Stroke, vec2};

/// winit's cursor limit. Keep allocations bounded even for hostile brush sizes or zooms.
const MAX_SIDE: u16 = 2048;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Circle {
        radius: f32,
        centre: bool,
    },
    Crosshair {
        length: f32,
        gap: f32,
    },
    /// Fixed artwork: an OS bitmap on every platform, never sized by the document or the brush.
    Art(Art),
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
    show(painter, at, Shape::Circle { radius, centre }, CursorIcon::Crosshair)
}

pub(crate) fn crosshair(painter: &Painter, at: Pos2, length: f32, gap: f32) -> CursorIcon {
    if gap == 0.0
        && (5.0..=8.0).contains(&length)
        && let Some(icon) = crate::cursor_pack::show(painter.ctx(), at, 1025, CursorIcon::Crosshair)
    {
        return icon;
    }
    show(painter, at, Shape::Crosshair { length, gap }, CursorIcon::Crosshair)
}

/// The cursor of a role (`cursor_pack::tool_asset`'s id): the optional local pack's image, else
/// Prism's own drawing; `None` for a role with neither, which keeps `stock`. `stock` is also what
/// a backend shows if it cannot upload the bitmap.
pub(crate) fn standard(painter: &Painter, at: Pos2, role: u32, stock: CursorIcon) -> Option<CursorIcon> {
    crate::cursor_pack::show(painter.ctx(), at, role, stock).or_else(|| Art::for_role(role).map(|art| show(painter, at, Shape::Art(art), stock)))
}

/// The crosshair with a centre dot, where a tool has no drawing of its own.
pub(crate) fn cross(painter: &Painter, at: Pos2) -> CursorIcon {
    art(painter, at, Art::Cross)
}

/// The path tools' arrows, as in Photoshop: black for Path Selection, white for Direct
/// Selection (`direct`). The hotspot is the tip.
pub(crate) fn path_tool(painter: &Painter, at: Pos2, direct: bool) -> CursorIcon {
    if let Some(icon) = crate::cursor_pack::show(painter.ctx(), at, if direct { 1500 } else { 1057 }, CursorIcon::Default) {
        return icon;
    }
    art(painter, at, if direct { Art::ArrowWhite } else { Art::Arrow })
}

/// An arrowhead with a four-way move badge. The hotspot is the arrow's tip.
pub(crate) fn move_tool(painter: &Painter, at: Pos2, dragging: bool) -> CursorIcon {
    if let Some(icon) = crate::cursor_pack::show(painter.ctx(), at, if dragging { 1004 } else { 1059 }, CursorIcon::Default) {
        return icon;
    }
    if dragging { CursorIcon::Default } else { art(painter, at, Art::Move) }
}

/// The Zoom tool's magnifying glass (also the temporary zoom keys'): a plus, or a minus when a
/// click zooms `out`. The hotspot is the centre of the lens.
pub(crate) fn zoom(painter: &Painter, at: Pos2, out: bool) -> CursorIcon {
    let stock = if out { CursorIcon::ZoomOut } else { CursorIcon::ZoomIn };
    if let Some(icon) = crate::cursor_pack::show(painter.ctx(), at, if out { 1007 } else { 1006 }, stock) {
        return icon;
    }
    art(painter, at, if out { Art::ZoomOut } else { Art::ZoomIn })
}

/// The Hand tool's hand (also Space and the middle button's temporary hand), closed while it
/// drags. winit's stock grab cursor is a four-way arrow on Windows.
pub(crate) fn hand(painter: &Painter, at: Pos2, dragging: bool) -> CursorIcon {
    let stock = if dragging { CursorIcon::Grabbing } else { CursorIcon::Grab };
    if let Some(icon) = crate::cursor_pack::show(painter.ctx(), at, if dragging { 25006 } else { 1005 }, stock) {
        return icon;
    }
    art(painter, at, if dragging { Art::HandClosed } else { Art::Hand })
}

fn art(painter: &Painter, at: Pos2, art: Art) -> CursorIcon {
    show(painter, at, Shape::Art(art), art.fallback())
}

fn show(painter: &Painter, at: Pos2, shape: Shape, fallback: CursorIcon) -> CursorIcon {
    let ctx = painter.ctx();
    let fallback_key = fallback_key(ctx);
    ctx.data_mut(|d| d.insert_temp(fallback_key, fallback));
    // The native backend uploads cursor images in root and immediate document viewports.
    // Unsupported/oversize artwork retains the painter fallback below.
    if cfg!(target_os = "windows") || matches!(shape, Shape::Art(_)) {
        let scale = native_raster_scale(ctx.pixels_per_point());
        if let Some(image) = cached_image(ctx, shape, scale) {
            ctx.set_cursor_image(Some(image));
            ctx.data_mut(|d| d.insert_temp(fallback_key, fallback));
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
    if let Shape::Art(art) = shape {
        let ctx = painter.ctx();
        let scale = ctx.pixels_per_point();
        let id = egui::Id::new(("arrow-cursor-texture", format!("{shape:?}")));
        let cached = ctx.data(|d| d.get_temp::<(f32, egui::TextureHandle)>(id));
        let texture = cached.filter(|(s, _)| *s == scale).map(|(_, t)| t).or_else(|| {
            let image = rasterize(shape, scale)?;
            let tex = ctx.load_texture(
                "arrow-cursor",
                egui::ColorImage::from_rgba_unmultiplied([usize::from(image.size[0]), usize::from(image.size[1])], &image.rgba),
                egui::TextureOptions::LINEAR,
            );
            ctx.data_mut(|d| d.insert_temp(id, (scale, tex.clone())));
            Some(tex)
        });
        if let Some(tex) = texture {
            let [x, y] = art.hotspot();
            let min = at - vec2(x.floor(), y.floor());
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
            Shape::Art(_) => {}
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
        Shape::Art(_) => art::SIDE,
    })
}

fn rasterize(shape: Shape, scale: f32) -> Option<CustomCursorImage> {
    if let Shape::Art(art) = shape {
        return art::rasterize(art, scale);
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
                Shape::Art(_) => 0.0,
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
