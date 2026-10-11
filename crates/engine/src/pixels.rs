//! Destructive pixel operations on layers (Image → Adjustments, flips, fills).

use prism_compose::{Buffer, adjust};
use prism_doc::{Adjustment, Document, Layer, LayerContent};
use prism_geom::Rect;
use prism_raster::{Surface, to_rgba};

/// Apply an adjustment destructively to a surface, weighted by an optional selection.
/// Applies `adj` to a surface (any colour model and depth, via straight RGBA) through the
/// selection; `mode` is the document's, for the tone transfer (e.g. Exposure in Grayscale).
pub fn adjust_surface(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>, mode: prism_color::ColorMode) {
    let r = s.content_bounds();
    if r.is_empty() {
        return;
    }
    let out = adjusted(s, r, adj, selection, mode);
    s.write_region(r, &out);
}

/// Applies `adj` to a layer mask through the selection. Untouched mask pixels count (they read as
/// the mask's default value), and without a selection the default changes too, since a mask
/// reaches past the canvas: Invert turns a reveal-all mask into a hide-all one (#780).
pub fn adjust_mask(s: &mut Surface, adj: &Adjustment, selection: Option<&Surface>) {
    let mode = prism_color::ColorMode::Grayscale;
    if let Some(sel) = selection {
        let r = sel.content_bounds();
        if !r.is_empty() {
            let out = adjusted(s, r, adj, Some(sel), mode);
            s.write_region(r, &out);
            s.prune();
        }
        return;
    }
    let fmt = s.format();
    let unit = Rect::from_xywh(0, 0, 1, 1);
    let mut out = Surface::with_default(fmt, &adjusted(&Surface::with_default(fmt, &s.default_pixel()), unit, adj, None, mode));
    let r = s.tile_bounds();
    if !r.is_empty() {
        out.write_region(r, &adjusted(s, r, adj, None, mode));
    }
    out.prune();
    *s = out;
}

/// `s`'s pixels over `r` (untouched ones included) with `adj` applied through the selection, encoded.
fn adjusted(s: &Surface, r: Rect, adj: &Adjustment, selection: Option<&Surface>, mode: prism_color::ColorMode) -> Vec<f32> {
    let fmt = s.format();
    let n = fmt.channels();
    let raw = s.read_region(r);
    let mut buf = Buffer { rect: r, px: raw.chunks_exact(n).map(|p| to_rgba(&fmt, p)).collect() };
    let orig = buf.clone();
    // 32-bit documents get the float behaviour of an adjustment layer there (Levels doesn't clip);
    // integer depths keep the unrounded, clipped curves.
    let depth = (fmt.sample == prism_color::SampleType::F32).then_some(fmt.sample);
    adjust::apply_depth(adj, &mut buf, adjust::Transfer::for_document(mode, fmt.sample), depth);
    let w = r.width() as usize;
    let mut out = Vec::with_capacity(raw.len());
    for (i, (a, o)) in buf.px.iter().zip(&orig.px).enumerate() {
        let k = selection.map_or(1.0, |sel| sel.sample_channel(r.x0 + (i % w) as i32, r.y0 + (i / w) as i32, 0));
        let mixed: [f32; 4] = std::array::from_fn(|c| o[c] + (a[c] - o[c]) * k);
        let mut enc = [0.0f32; 8];
        let m = prism_raster::from_rgba_into(&fmt, mixed, &mut enc);
        out.extend_from_slice(&enc[..m]);
    }
    out
}

/// Fill (respecting selection coverage) with a straight RGBA colour.
pub fn fill_surface(s: &mut Surface, area: Rect, color: [f32; 4], selection: Option<&Surface>, lock_transparency: bool) {
    let fmt = s.format();
    let n = fmt.channels();
    let mut region = s.read_region(area);
    let w = area.width() as usize;
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let x = area.x0 + (i % w) as i32;
        let y = area.y0 + (i / w) as i32;
        let k = selection.map_or(1.0, |sel| sel.sample_channel(x, y, 0));
        if k <= 0.0 {
            continue;
        }
        let d = to_rgba(&fmt, px);
        let sa = color[3] * k;
        let oa = sa + d[3] * (1.0 - sa);
        let mut o = [0.0f32; 4];
        if oa > 0.0 {
            for c in 0..3 {
                o[c] = (color[c] * sa + d[c] * d[3] * (1.0 - sa)) / oa;
            }
        }
        o[3] = if lock_transparency { d[3] } else { oa };
        let mut enc = [0.0f32; 8];
        prism_raster::from_rgba_into(&fmt, o, &mut enc);
        px.copy_from_slice(&enc[..n]);
    }
    s.write_region(area, &region);
}

/// Clear pixels (make transparent) within the selection.
pub fn clear_surface(s: &mut Surface, area: Rect, selection: Option<&Surface>) {
    let fmt = s.format();
    let n = fmt.channels();
    if !fmt.alpha {
        return;
    }
    let mut region = s.read_region(area);
    let w = area.width() as usize;
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let k = selection.map_or(1.0, |sel| sel.sample_channel(area.x0 + (i % w) as i32, area.y0 + (i / w) as i32, 0));
        px[n - 1] *= 1.0 - k;
        // Fully cleared pixels become the empty pixel, so bounds and tile pruning see them as gone.
        if px[n - 1] <= 0.0 {
            px.fill(0.0);
        }
    }
    s.write_region(area, &region);
    s.prune();
}

/// Remap a surface through a pixel-coordinate mapping (used for flips/rotations).
pub fn remap_surface(s: &Surface, map: impl Fn(i32, i32) -> (i32, i32)) -> Surface {
    let r = s.content_bounds();
    let mut out = Surface::with_default(s.format(), &s.default_pixel());
    let n = s.channels();
    let raw = s.read_region(r);
    let w = r.width() as usize;
    for (i, px) in raw.chunks_exact(n).enumerate() {
        let (x, y) = (r.x0 + (i % w) as i32, r.y0 + (i / w) as i32);
        let (nx, ny) = map(x, y);
        out.write_pixel(nx, ny, px);
    }
    out
}

/// Merge `upper` onto `lower` producing a raster layer (Layer → Merge Down).
pub fn merge_down(doc_bounds: Rect, lower: &Layer, upper: &Layer, format: prism_color::PixelFormat) -> Layer {
    let area = [lower, upper].iter().map(|l| l.surface().map(|s| s.content_bounds()).unwrap_or(doc_bounds)).fold(Rect::EMPTY, |a, b| a.union(&b));
    let stack = vec![lower.clone(), upper.clone()];
    let mut tmp = Document::new("merge", doc_bounds.size(), format.mode, format.sample);
    tmp.layers = stack;
    let s = merge_sharing_tiles(&tmp, area, lower, upper, format).unwrap_or_else(|| prism_compose::render_to_surface(&tmp, area, format, None));
    let mut merged = Layer::new(lower.name.clone(), LayerContent::Raster(s));
    merged.id = lower.id;
    merged.blend = lower.blend;
    merged.opacity = 1.0;
    merged
}

/// Merge Down without recompositing where it can't change anything: a tile of a plain `lower`
/// layer that `upper` leaves empty composites to exactly its own bytes, so the merged layer shares
/// it (copy-on-write) and only the tiles `upper` touches are rendered. `None` when the layers
/// aren't simple enough for that to be exact; the caller then renders everything.
fn merge_sharing_tiles(tmp: &Document, area: Rect, lower: &Layer, upper: &Layer, format: prism_color::PixelFormat) -> Option<Surface> {
    use prism_color::{ColorMode, SampleType};
    use rayon::prelude::*;
    // Straight RGBA round trips exactly only for RGB / Gray at integer depths (float depths and
    // CMYK / Lab conversions can move the last bits).
    let exact = matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) && matches!(format.sample, SampleType::U8 | SampleType::U16) && format.alpha;
    let (LayerContent::Raster(ls), LayerContent::Raster(us)) = (&lower.content, &upper.content) else { return None };
    if !exact || !plain(lower) || ls.format() != format || !zero(ls.default_bytes()) || !zero(us.default_bytes()) {
        return None;
    }
    // Where `upper` has no pixels it adds nothing, unless effects or advanced blending (knockout,
    // shapeless effects) reach beyond them.
    if upper.video.is_some() || prism_compose::effects::has_effects(upper) || !upper.advanced.is_default() {
        return None;
    }
    let busy: std::collections::BTreeSet<_> = if upper.visible { us.tiles().filter(|(_, t)| !zero(t.bytes())).map(|(c, _)| *c).collect() } else { Default::default() };
    // A lower tile composites to itself if no pixel is fully transparent with leftover colour
    // bytes (the compositor writes those back as zeros).
    let bpp = format.bytes_per_pixel();
    let abytes = format.sample.bytes();
    let mut render: Vec<_> = busy.iter().copied().collect();
    let mut out = Surface::new(format);
    let lower_tiles: Vec<_> = ls.tiles().filter(|(c, _)| !busy.contains(*c)).collect();
    // Per untouched lower tile: `Some(true)` share it, `Some(false)` recomposite it, `None` blank.
    let verdict: Vec<Option<bool>> = lower_tiles
        .par_iter()
        .map(|(_, t)| {
            if zero(t.bytes()) {
                return None;
            }
            Some(t.bytes().chunks_exact(bpp).all(|p| {
                let (color, alpha) = p.split_at(bpp - abytes);
                !zero(alpha) || zero(color)
            }))
        })
        .collect();
    for ((c, t), v) in lower_tiles.into_iter().zip(verdict) {
        match v {
            Some(true) => out.put_tiles([(*c, t.clone())]),
            Some(false) => render.push(*c),
            None => {}
        }
    }
    out.put_tiles(prism_compose::render_tiles(tmp, &render, area, format));
    Some(out)
}

/// A raster layer that composites onto transparency as exactly its own pixels.
fn plain(l: &Layer) -> bool {
    l.visible
        && l.blend == prism_color::blend::BlendMode::Normal
        && l.opacity == 1.0
        && l.fill_opacity == 1.0
        && !l.clipped
        && l.mask.is_none()
        && l.vector_mask.is_none()
        && !prism_compose::effects::has_effects(l)
        && l.excluded_channels == 0
        && l.blend_if.is_default()
        && l.advanced.is_default()
        && l.video.is_none()
        && l.fill_cache.is_none()
}

fn zero(b: &[u8]) -> bool {
    b.iter().all(|&x| x == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_color::blend::BlendMode;
    use prism_color::{ColorMode, PixelFormat, SampleType};
    use prism_doc::LayerMask;

    fn layer(fmt: PixelFormat, r: Rect, seed: f32) -> Layer {
        let n = fmt.channels();
        let mut data = vec![0.0; r.width() as usize * r.height() as usize * n];
        for (i, px) in data.chunks_exact_mut(n).enumerate() {
            let (x, y) = ((i % r.width() as usize) as f32, (i / r.width() as usize) as f32);
            for (c, v) in px.iter_mut().enumerate() {
                *v = ((x * 0.031 + y * 0.017 + c as f32 * 0.7 + seed).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
            }
            px[n - 1] = (x * 0.05 + seed).cos() * 0.4 + 0.6;
        }
        let mut l = Layer::raster("l", fmt);
        let s = l.surface_mut().unwrap();
        s.write_region(r, &data);
        s.prune();
        l
    }

    /// What Merge Down stores without the tile-sharing shortcut.
    fn full(bounds: Rect, lower: &Layer, upper: &Layer, fmt: PixelFormat) -> Surface {
        let area = [lower, upper].iter().map(|l| l.surface().map(|s| s.content_bounds()).unwrap_or(bounds)).fold(Rect::EMPTY, |a, b| a.union(&b));
        let mut tmp = Document::new("merge", bounds.size(), fmt.mode, fmt.sample);
        tmp.layers = vec![lower.clone(), upper.clone()];
        prism_compose::render_to_surface(&tmp, area, fmt, None)
    }

    fn check(lower: &Layer, upper: &Layer, fmt: PixelFormat, what: &str) {
        let bounds = Rect::new(0, 0, 900, 700);
        let got = merge_down(bounds, lower, upper, fmt);
        assert_eq!(got.surface().unwrap(), &full(bounds, lower, upper, fmt), "{what}");
    }

    #[test]
    fn merge_down_sharing_tiles_matches_a_full_composite() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for sample in SampleType::ALL {
                let fmt = PixelFormat::new(mode, sample, true);
                // A large lower layer and a small, unaligned stroke over part of it.
                let lower = layer(fmt, Rect::new(-30, 10, 850, 690), 0.3);
                let upper = layer(fmt, Rect::new(300, 270, 420, 333), 1.9);
                check(&lower, &upper, fmt, &format!("{mode:?} {sample:?}"));
                // Upper reaching outside the lower layer and the canvas.
                check(&lower, &layer(fmt, Rect::new(780, -40, 990, 120), 2.4), fmt, &format!("{mode:?} {sample:?} outside"));
            }
        }
    }

    #[test]
    fn merge_down_sharing_tiles_handles_every_upper_setting() {
        let fmt = PixelFormat::RGBA8;
        let lower = layer(fmt, Rect::new(0, 0, 900, 700), 0.3);
        let stroke = layer(fmt, Rect::new(300, 270, 420, 333), 1.9);
        for mode in BlendMode::LAYER_MODES {
            let mut u = stroke.clone();
            u.blend = mode;
            check(&lower, &u, fmt, &format!("{mode:?}"));
        }
        let mut u = stroke.clone();
        u.visible = false;
        check(&lower, &u, fmt, "hidden upper");
        let mut u = stroke.clone();
        u.opacity = 0.4;
        u.clipped = true;
        check(&lower, &u, fmt, "clipped, 40 %");
        let mut u = stroke.clone();
        u.mask = Some(LayerMask { density: 0.5, ..LayerMask::reveal_all() });
        check(&lower, &u, fmt, "masked upper");
        check(&lower, &Layer::raster("empty", fmt), fmt, "empty upper");
    }

    #[test]
    fn merge_down_sharing_tiles_respects_lower_settings() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U16, true);
        let stroke = layer(fmt, Rect::new(300, 270, 420, 333), 1.9);
        let base = layer(fmt, Rect::new(0, 0, 900, 700), 0.3);
        let mut lowers = Vec::new();
        let mut l = base.clone();
        l.opacity = 0.5;
        lowers.push(l);
        let mut l = base.clone();
        l.blend = BlendMode::Multiply;
        lowers.push(l);
        let mut l = base.clone();
        l.visible = false;
        lowers.push(l);
        // Fully transparent pixels that still carry colour bytes composite to zeros.
        let mut l = base.clone();
        l.surface_mut().unwrap().write_region(Rect::new(10, 10, 12, 11), &[0.5, 0.2, 0.9, 0.0, 0.1, 0.1, 0.1, 0.0]);
        lowers.push(l);
        for (i, l) in lowers.iter().enumerate() {
            check(l, &stroke, fmt, &format!("lower case {i}"));
        }
    }

    #[test]
    fn merge_down_shares_untouched_lower_tiles() {
        let fmt = PixelFormat::RGBA8;
        let mut lower = Layer::raster("photo", fmt);
        lower.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 900, 700), &[0.2, 0.5, 0.7, 1.0]);
        let upper = layer(fmt, Rect::new(300, 270, 420, 333), 1.9);
        let merged = merge_down(Rect::new(0, 0, 900, 700), &lower, &upper, fmt);
        let ls = lower.surface().unwrap();
        let ms = merged.surface().unwrap();
        let shared = ms.tiles().filter(|(c, t)| ls.tiles().any(|(k, lt)| k == *c && std::sync::Arc::ptr_eq(lt, t))).count();
        // The stroke touches one tile; the others are the lower layer's own.
        assert_eq!(shared, ms.tile_count() - 1);
        assert_eq!(ms, &full(Rect::new(0, 0, 900, 700), &lower, &upper, fmt));
    }
}
