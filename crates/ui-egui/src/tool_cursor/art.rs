//! Fixed cursor artwork: Prism's own SVGs (`assets/cursors`, a 32-point box each), rasterized at
//! the viewport's DPI. The drawings are black and white only; a light halo is added around them
//! here, so every cursor reads on any image.

use std::sync::Arc;

use egui::{CursorIcon, CustomCursorImage};
use resvg::{tiny_skia::Pixmap, usvg};

/// The side of every drawing's box, in points.
pub(super) const SIDE: f32 = 32.0;
/// The halo's width, in points.
const HALO: f32 = 1.25;
/// Scales outside this range get the stock cursor: the halo's cost grows with the fourth power.
const SCALES: std::ops::RangeInclusive<f32> = 0.25..=8.0;

/// One drawing. The names are what it shows, not which tool uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Art {
    /// The black arrow (Path Selection).
    Arrow,
    /// The white arrow (Direct Selection).
    ArrowWhite,
    /// An arrowhead with a four-way badge (Move).
    Move,
    Hand,
    HandClosed,
    ZoomIn,
    ZoomOut,
    /// The crosshair with a centre dot: the marquees, the shapes and every precise cursor.
    Cross,
    /// The crosshair with a dashed square (Object Selection).
    CrossRect,
    Lasso,
    LassoPolygon,
    Wand,
    Crop,
    Eyedropper,
    /// The I-beam in a dotted box: a click starts a new type layer.
    Text,
    TextVertical,
    /// The pen nib alone (the next anchor of a path being drawn).
    Pen,
    /// The nib with an asterisk (a click starts a new path).
    PenNew,
    PenAdd,
    PenDelete,
    PenClose,
    /// The open arrowhead (Convert Point).
    Convert,
}

impl Art {
    #[cfg(test)]
    pub(crate) const ALL: [Art; 22] = [
        Art::Arrow,
        Art::ArrowWhite,
        Art::Move,
        Art::Hand,
        Art::HandClosed,
        Art::ZoomIn,
        Art::ZoomOut,
        Art::Cross,
        Art::CrossRect,
        Art::Lasso,
        Art::LassoPolygon,
        Art::Wand,
        Art::Crop,
        Art::Eyedropper,
        Art::Text,
        Art::TextVertical,
        Art::Pen,
        Art::PenNew,
        Art::PenAdd,
        Art::PenDelete,
        Art::PenClose,
        Art::Convert,
    ];

    fn svg(self) -> &'static [u8] {
        match self {
            Art::Arrow => include_bytes!("../../../../assets/cursors/arrow.svg"),
            Art::ArrowWhite => include_bytes!("../../../../assets/cursors/arrow-white.svg"),
            Art::Move => include_bytes!("../../../../assets/cursors/move.svg"),
            Art::Hand => include_bytes!("../../../../assets/cursors/hand.svg"),
            Art::HandClosed => include_bytes!("../../../../assets/cursors/hand-closed.svg"),
            Art::ZoomIn => include_bytes!("../../../../assets/cursors/zoom-in.svg"),
            Art::ZoomOut => include_bytes!("../../../../assets/cursors/zoom-out.svg"),
            Art::Cross => include_bytes!("../../../../assets/cursors/cross.svg"),
            Art::CrossRect => include_bytes!("../../../../assets/cursors/cross-rect.svg"),
            Art::Lasso => include_bytes!("../../../../assets/cursors/lasso.svg"),
            Art::LassoPolygon => include_bytes!("../../../../assets/cursors/lasso-polygon.svg"),
            Art::Wand => include_bytes!("../../../../assets/cursors/wand.svg"),
            Art::Crop => include_bytes!("../../../../assets/cursors/crop.svg"),
            Art::Eyedropper => include_bytes!("../../../../assets/cursors/eyedropper.svg"),
            Art::Text => include_bytes!("../../../../assets/cursors/text.svg"),
            Art::TextVertical => include_bytes!("../../../../assets/cursors/text-vertical.svg"),
            Art::Pen => include_bytes!("../../../../assets/cursors/pen.svg"),
            Art::PenNew => include_bytes!("../../../../assets/cursors/pen-new.svg"),
            Art::PenAdd => include_bytes!("../../../../assets/cursors/pen-add.svg"),
            Art::PenDelete => include_bytes!("../../../../assets/cursors/pen-delete.svg"),
            Art::PenClose => include_bytes!("../../../../assets/cursors/pen-close.svg"),
            Art::Convert => include_bytes!("../../../../assets/cursors/convert.svg"),
        }
    }

    /// Where a click lands, in points from the box's corner: an arrow's or a nib's tip, a
    /// crosshair's or a lens's centre, the wand's end, the pipette's mouth, the I-beam's baseline.
    pub(super) fn hotspot(self) -> [f32; 2] {
        match self {
            Art::Arrow | Art::Move | Art::Lasso | Art::LassoPolygon => [3.2, 2.5],
            Art::ArrowWhite => [3.2, 2.7],
            Art::Hand | Art::HandClosed => [16.0, 17.0],
            Art::ZoomIn | Art::ZoomOut => [12.0, 12.0],
            Art::Cross => [16.5, 16.5],
            Art::CrossRect => [8.5, 8.5],
            Art::Wand => [9.6, 9.6],
            Art::Crop => [6.5, 6.5],
            Art::Eyedropper => [6.8, 24.8],
            Art::Text => [16.5, 20.5],
            Art::TextVertical => [12.5, 16.5],
            Art::Pen | Art::PenNew | Art::PenAdd | Art::PenDelete | Art::PenClose => [3.0, 3.0],
            Art::Convert => [3.5, 3.5],
        }
    }

    /// The stock cursor a backend shows if it cannot upload the bitmap.
    pub(super) fn fallback(self) -> CursorIcon {
        match self {
            Art::Arrow | Art::ArrowWhite | Art::Move => CursorIcon::Default,
            Art::Hand => CursorIcon::Grab,
            Art::HandClosed => CursorIcon::Grabbing,
            Art::ZoomIn => CursorIcon::ZoomIn,
            Art::ZoomOut => CursorIcon::ZoomOut,
            Art::Text => CursorIcon::Text,
            Art::TextVertical => CursorIcon::VerticalText,
            _ => CursorIcon::Crosshair,
        }
    }

    /// The drawing for a cursor role, by the role's id in an optional local pack
    /// (`cursor_pack::tool_asset`): the pack's own image wins, this is what everyone else sees.
    /// The selection tools' add / subtract / intersect ids share their tool's drawing (the badge
    /// is drawn beside it); roles without a drawing keep the crosshair or the stock cursor.
    pub(crate) fn for_role(id: u32) -> Option<Art> {
        Some(match id {
            1005 | 1630 => Art::Hand,
            25006 => Art::HandClosed,
            1006 => Art::ZoomIn,
            1007 => Art::ZoomOut,
            1017 => Art::Eyedropper,
            1025 | 1450..=1453 => Art::Cross,
            1027 => Art::Crop,
            1030 | 1066 => Art::Pen,
            1031 => Art::PenAdd,
            1032 => Art::PenDelete,
            1034 => Art::PenClose,
            1035 => Art::Convert,
            1057 => Art::Arrow,
            1059 => Art::Move,
            1063 => Art::PenNew,
            1454..=1457 => Art::Wand,
            1458..=1461 => Art::Lasso,
            1462 => Art::Text,
            1474 => Art::TextVertical,
            1480..=1483 => Art::LassoPolygon,
            1500 => Art::ArrowWhite,
            1800..=1803 => Art::CrossRect,
            _ => return None,
        })
    }
}

/// The drawing at `scale` pixels per point, with its halo, as straight RGBA (winit's format).
pub(super) fn rasterize(art: Art, scale: f32) -> Option<CustomCursorImage> {
    if !scale.is_finite() || !SCALES.contains(&scale) {
        return None;
    }
    let side = (SIDE * scale).ceil() as u16;
    let n = usize::from(side);
    let tree = usvg::Tree::from_data(art.svg(), &usvg::Options::default()).ok()?;
    let mut pixmap = Pixmap::new(u32::from(side), u32::from(side))?;
    resvg::render(&tree, usvg::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // Premultiplied RGBA.
    let (pixels, _) = pixmap.data().as_chunks::<4>();
    let alpha: Vec<f32> = pixels.iter().map(|&[_, _, _, a]| f32::from(a) / 255.0).collect();

    // The halo: the drawing's alpha, spread `HALO` points with a one-pixel soft edge. Distances
    // run between pixel centres, half a pixel more than from the drawing's edge on each side.
    let reach = HALO * scale + 1.0;
    let r = reach.ceil() as isize;
    let spread: Vec<(isize, isize, f32)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter_map(|(dx, dy)| {
            let weight = (reach - ((dx * dx + dy * dy) as f32).sqrt()).clamp(0.0, 1.0);
            (weight > 0.0).then_some((dx, dy, weight))
        })
        .collect();
    let [_, light] = crate::theme::Tokens::cursor_outline();
    let light = f32::from(light.a()) / 255.0;
    let side_i = n as isize;
    let mut rgba = Vec::with_capacity(n.checked_mul(n)?.checked_mul(4)?);
    for (i, (&[red, green, blue, _], &a)) in pixels.iter().zip(&alpha).enumerate() {
        let (x, y) = ((i % n) as isize, (i / n) as isize);
        let mut halo = 0.0_f32;
        if a < 1.0 {
            for &(dx, dy, weight) in &spread {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= side_i || ny >= side_i {
                    continue;
                }
                let near = alpha.get(ny as usize * n + nx as usize).copied().unwrap_or(0.0);
                halo = halo.max(near * weight);
            }
        }
        // The drawing over the white halo.
        let under = halo * light * (1.0 - a);
        let out = a + under;
        let straight = |ink: u8| if out > 0.0 { ((f32::from(ink) / 255.0 + under) / out * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        rgba.extend_from_slice(&[straight(red), straight(green), straight(blue), (out * 255.0).round() as u8]);
    }
    let hotspot = art.hotspot().map(|h| ((h * scale).floor() as u16).min(side - 1));
    Some(CustomCursorImage { rgba: Arc::from(rgba), size: [side, side], hotspot })
}
