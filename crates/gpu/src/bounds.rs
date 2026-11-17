//! Layer bounds exactly as `prism-compose` computes them (`layer_bounds`), with the per-tile
//! content scan cached by copy-on-write tile identity, so an effect layer's region costs a hash
//! lookup per tile instead of a pixel scan per frame.

use prism_doc::Layer;
use prism_geom::Rect;
#[cfg(test)]
use prism_raster::Surface;

pub use prism_compose::bounds::content_bounds;

/// Bounds of a layer's own pixels: `prism_compose::layer_bounds`.
pub fn layer_bounds(layer: &Layer, canvas: Rect) -> Rect {
    prism_compose::layer_bounds(layer, canvas)
}

/// The region a layer's effect maps cover: its bounds grown by the effect reach, within the
/// canvas grown likewise (`prism_compose::effect_maps`) when the layer reaches far beyond
/// it. A layer about the canvas's size keeps its whole region even where it overhangs the edge:
/// then the region keeps its size while the layer moves (a Move tool drag), so its maps move with
/// it instead of being rebuilt every frame (#761). The maps are exact either way.
pub fn effect_region(layer: &Layer, canvas: Rect) -> Rect {
    region_with(layer, canvas, effect_margin(layer))
}

/// Where a layer's effects paint: [`effect_region`] with the exact margin, as the CPU compositor
/// draws them. The maps live over the (larger) stepped region, and effects whose contour is not 0
/// at 0 would otherwise paint into its extra pixels.
pub fn effect_paint_region(layer: &Layer, canvas: Rect) -> Rect {
    region_with(layer, canvas, prism_compose::effects::margin(layer))
}

fn region_with(layer: &Layer, canvas: Rect, m: i32) -> Rect {
    let (whole, window) = (layer_bounds(layer, canvas).inflate(m), canvas.inflate(m));
    let area = |r: Rect| u64::from(r.width()) * u64::from(r.height());
    if area(whole) <= area(window).saturating_add(area(window) / 4) { whole } else { whole.intersect(&window) }
}

/// Steps the effect region's margin grows in (px).
pub const MARGIN_STEP: i32 = 32;

/// The effects' reach (`prism_compose::effects::margin`) rounded up to [`MARGIN_STEP`]: the region
/// keeps its size while a drop shadow's distance or a glow's size is dragged a few pixels, so the
/// shape, distance fields and map textures are kept instead of rebuilt every step. A larger
/// region only adds pixels the effects don't reach; the maps stay exact.
pub fn effect_margin(layer: &Layer) -> i32 {
    // Shadow maps are built at distance 0 and read at their offset (`fx::MapProgram::shift`), so
    // the maps only reach the shadows' size: a Distance drag never resizes the region.
    let m = prism_compose::effects::map_margin(layer).max(0);
    // A bevel's Contour or Texture element raises the height everywhere, so its maps slope at the
    // region's edge (on the CPU too): keep the exact region for those.
    let edge_bound =
        layer.effects.items.iter().any(|e| matches!(e, prism_doc::Effect::BevelEmboss(b) if e.enabled() && (b.contour.is_some() || b.texture.is_some())));
    if edge_bound {
        return m;
    }
    (m.unsigned_abs().div_ceil(MARGIN_STEP.unsigned_abs()).saturating_mul(MARGIN_STEP.unsigned_abs())).min(i32::MAX as u32) as i32
}

/// `prism_compose::transparent_outside`.
pub fn transparent_outside(layer: &Layer) -> bool {
    prism_compose::transparent_outside(layer)
}

/// `prism_compose::composite_bounds`: where compositing the layer can change anything.
pub fn composite_bounds(layer: &Layer, canvas: Rect) -> Option<Rect> {
    prism_compose::composite_bounds(layer, canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_color::PixelFormat;

    #[test]
    fn matches_surface_content_bounds() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        assert_eq!(content_bounds(&s), s.content_bounds());
        s.fill_rect(Rect::new(10, 300, 40, 700), &[1.0, 0.0, 0.0, 1.0]);
        s.fill_rect(Rect::new(-50, 5, -3, 9), &[0.0, 0.0, 1.0, 0.5]);
        assert_eq!(content_bounds(&s), s.content_bounds());
        // Cached a second time, and after a change.
        assert_eq!(content_bounds(&s), s.content_bounds());
        s.fill_rect(Rect::new(600, 600, 601, 601), &[0.0, 1.0, 0.0, 1.0]);
        assert_eq!(content_bounds(&s), s.content_bounds());
        let mut g = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
        g.fill_rect(Rect::new(3, 3, 9, 9), &[0.5]);
        assert_eq!(content_bounds(&g), g.content_bounds());
    }

    #[test]
    fn effect_margin_steps_keep_the_region_while_distance_moves() {
        let mut l = Layer::raster("fx", PixelFormat::RGBA8);
        l.effects.items.push(prism_doc::Effect::default_drop_shadow());
        let Some(prism_doc::Effect::DropShadow(s)) = l.effects.items.first_mut() else { panic!() };
        s.distance = 3.0;
        let a = effect_margin(&l);
        assert!(a >= prism_compose::effects::map_margin(&l) && a % MARGIN_STEP == 0);
        let Some(prism_doc::Effect::DropShadow(s)) = l.effects.items.first_mut() else { panic!() };
        s.distance = 9.0;
        assert_eq!(effect_margin(&l), a);
        // Distance doesn't size the maps at all (they're read at the shadow's offset); the paint
        // region still grows with it.
        let Some(prism_doc::Effect::DropShadow(s)) = l.effects.items.first_mut() else { panic!() };
        s.distance = 300.0;
        assert_eq!(effect_margin(&l), a);
        let canvas = Rect::new(0, 0, 100, 100);
        l.surface_mut().unwrap().fill_rect(Rect::new(40, 40, 60, 60), &[1.0, 0.0, 0.0, 1.0]);
        assert!(effect_paint_region(&l, canvas).contains_rect(&Rect::new(40, 40, 60, 60).inflate(300)));
    }
}
