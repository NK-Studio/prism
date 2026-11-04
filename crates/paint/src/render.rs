//! Dab rasterisation and stroke compositing.
//!
//! A stroke paints into a sparse, tiled *stroke buffer* (coverage plus, with Color Dynamics, a
//! premultiplied colour), exactly like a private stroke layer: dabs build up with flow up to their
//! opacity ceiling, stroke-level masks (Dual Brush, Texture) apply when the buffer is composited,
//! and the whole buffer composites onto the pre-stroke pixels at the stroke opacity. Because the
//! buffer is sparse and compositing always starts from the pre-stroke pixels, strokes can be fed in
//! chunks and re-composited incrementally for interactive painting (see [`StrokeRenderer`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use prism_color::{BlendMode, PixelFormat};
use prism_geom::Rect;
use prism_raster::{Surface, from_rgba_into, to_rgba};

use crate::brush::{BrushSettings, MaskMode, Pattern, TipShape};
use crate::csp_mixing as water_mix;
use crate::csp_water_edge::{self as water, EdgeParams};
use crate::dynamics::DabGenerator;
use crate::retouch::{Footprint, alpha_index, over_native};
use crate::rng::hash2;
use crate::tile::{GrayTile, Mips, PatternImage};
use crate::{Dab, Falloff, StrokePoint};

#[path = "prepared_brush_cache.rs"]
mod prepared_cache;

/// Edge of a stroke-buffer tile.
pub const COV_TILE: i32 = 64;
const NOISE_SALT: u64 = 0x006E_6F69_7365;
/// Prism work limit for the dense blurred watercolor-edge field (pixels); larger strokes commit
/// the unblurred edge instead.
const MAX_WATER_FIELD: usize = 64 << 20;

fn tile_rect(tx: i32, ty: i32) -> Rect {
    Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE)
}

/// Every tile key over `r`.
fn tile_keys(r: Rect) -> impl Iterator<Item = (i32, i32)> {
    let (x0, x1, y0, y1) = if r.is_empty() {
        (0, -1, 0, -1)
    } else {
        (r.x0.div_euclid(COV_TILE), (r.x1 - 1).div_euclid(COV_TILE), r.y0.div_euclid(COV_TILE), (r.y1 - 1).div_euclid(COV_TILE))
    };
    (y0..=y1).flat_map(move |ty| (x0..=x1).map(move |tx| (tx, ty)))
}

/// How the CSP watercolor edge applies to one `composite` call.
enum WaterPass {
    Off,
    /// Computed per tile from the coverage around it.
    Live(EdgeParams),
    /// Blurred 8-bit amounts over `rect`, applied within `halo` (`reach` pixels around the stroke).
    Blurred {
        params: EdgeParams,
        rect: Rect,
        reach: i32,
        field: Vec<u8>,
        halo: Rect,
    },
}

#[inline]
/// Where an aliased (Pencil) dab of `diameter` pixels centred near (`x`, `y`) lands on the pixel
/// grid: the centre of the pixel holding the point for odd diameters, the nearest pixel corner
/// for even ones. Its footprint is then a whole-pixel block around that point, the same square
/// the Pencil cursor shows ([`grid_square`]).
pub fn grid_center(x: f64, y: f64, diameter: f32) -> (f32, f32) {
    let n = if diameter.is_finite() { diameter.round().max(1.0) as i64 } else { 1 };
    let snap = |v: f64| if n % 2 == 1 { v.floor() + 0.5 } else { v.round() };
    (snap(x) as f32, snap(y) as f32)
}

/// The whole-pixel square `[x0, y0, x1, y1]` an aliased dab of `diameter` pixels at (`x`, `y`)
/// can touch (see [`grid_center`]): the Pencil's cursor.
pub fn grid_square(x: f64, y: f64, diameter: f32) -> [f64; 4] {
    let n = if diameter.is_finite() { f64::from(diameter.round().clamp(1.0, 100_000.0)) } else { 1.0 };
    let (cx, cy) = grid_center(x, y, n as f32);
    let h = n / 2.0;
    [f64::from(cx) - h, f64::from(cy) - h, f64::from(cx) + h, f64::from(cy) + h]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Combine brush coverage `v` with a mask value `t` (texture/dual tip) at `depth`.
/// Every mode maps `v = 0` to 0 (a mask never adds paint where the brush has none).
pub fn mask_combine(mode: MaskMode, v: f32, t: f32, d: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let lerp = |a: f32, b: f32| a + (b - a) * d;
    let r = match mode {
        MaskMode::Multiply => v * (1.0 - d + d * t),
        MaskMode::Subtract => (v - d * (1.0 - t)).max(0.0),
        MaskMode::Darken => v.min(1.0 - d + d * t),
        MaskMode::Overlay => {
            let o = if v < 0.5 { 2.0 * v * t } else { 1.0 - 2.0 * (1.0 - v) * (1.0 - t) };
            lerp(v, o)
        }
        MaskMode::ColorDodge => lerp(v, (v / (1.0 - t).max(1e-3)).min(1.0)),
        MaskMode::ColorBurn => lerp(v, 1.0 - ((1.0 - v) / t.max(1e-3)).min(1.0)),
        MaskMode::LinearBurn => lerp(v, (v + t - 1.0).max(0.0)),
        MaskMode::HardMix => lerp(v, if v + t > 1.0 { 1.0 } else { 0.0 }),
        MaskMode::LinearHeight => v * ((t - (1.0 - d * v)) * 4.0 + 0.5).clamp(0.0, 1.0),
        MaskMode::Height => {
            if t >= 1.0 - d * v {
                v
            } else {
                0.0
            }
        }
    };
    r.clamp(0.0, 1.0)
}

/// A stroke-ready brush: the settings plus prepared tip mips and texture.
#[derive(Clone, Debug)]
pub struct BrushContext {
    /// Immutable prepared settings, shared by live/tail previews along with their mip maps.
    pub brush: Arc<BrushSettings>,
    tip: Option<Arc<Mips>>,
    sequence: Vec<Arc<Mips>>,
    dual_tip: Option<Arc<Mips>>,
    dual_sequence: Vec<Arc<Mips>>,
    /// CSP colour tips: premultiplied RGB planes per prepared tip (`tip`, or each of `sequence`);
    /// empty when no tip keeps its own colours or the brush mixes colours.
    tip_colors: Vec<Option<Arc<[Mips; 3]>>>,
    /// The mean colour of each prepared colour tip, for dabs laid in one colour (colour mixing);
    /// empty when no tip keeps its own colours.
    tip_means: Vec<Option<[f32; 3]>>,
    /// `tip_colors` trimmed like `ribbon_tips`, for ribbon slices.
    ribbon_colors: Vec<Option<Arc<[Mips; 3]>>>,
    /// CSP Ribbon: each tip without the transparent padding line at its end along the ribbon,
    /// and the factors from the full tip's normalised coordinates to the trimmed one's.
    ribbon_tips: Vec<(Arc<Mips>, [f32; 2])>,
    texture: Option<Arc<PatternImage>>,
    texture_scale: f32,
}

fn prepare_pattern(b: &BrushSettings) -> (Option<Arc<PatternImage>>, f32) {
    let t = &b.texture;
    if !t.enabled {
        return (None, t.scale);
    }
    let mut img = match &t.pattern {
        Pattern::Procedural { style, size, seed } => crate::procedural::pattern(*style, *size, *seed),
        Pattern::Tile(g) if g.is_valid() => PatternImage { width: g.width as usize, height: g.height as usize, data: g.to_f32() },
        Pattern::Tile(_) => return (None, t.scale),
    };
    // CSP adjusts darkness after bilinear sampling, not independently on source texels.
    if t.csp_mode.is_some() {
        let mut scale = t.scale.max(0.01);
        // The observed brush-pattern factory builds two levels, discarding odd
        // trailing rows/columns. Smaller patterns have no mip chain.
        if img.width > 3 && img.height > 3 {
            for _ in 0..2 {
                if scale > 0.5 {
                    break;
                }
                let (width, height) = (img.width / 2, img.height / 2);
                let mut data = Vec::with_capacity(width * height);
                for y in 0..height {
                    for x in 0..width {
                        let mut sum = 0u32;
                        for dy in 0..2 {
                            for dx in 0..2 {
                                let value = img.data.get((2 * y + dy) * img.width + 2 * x + dx).copied().unwrap_or(1.0);
                                sum += ((1.0 - value).clamp(0.0, 1.0) * 255.0).round() as u32;
                            }
                        }
                        data.push(1.0 - (sum / 4) as f32 / 255.0);
                    }
                }
                img = PatternImage { width, height, data };
                scale *= 2.0;
            }
        }
        return (Some(Arc::new(img)), scale);
    }
    let (br, ct) = (t.brightness.clamp(-1.0, 1.0), t.effective_contrast().clamp(-1.0, 1.0));
    for v in &mut img.data {
        let mut x = (*v - 0.5) * (1.0 + ct) + 0.5 + br;
        x = x.clamp(0.0, 1.0);
        *v = if t.invert { 1.0 - x } else { x };
    }
    (Some(Arc::new(img)), t.scale)
}

/// A computed tip's roundness and minor radius, as the rasterizer uses them.
fn tip_axes(d: &Dab) -> (f32, f32) {
    let ro = d.roundness.max(0.5 / d.radius).min(1.0);
    (ro, (d.radius * ro).max(0.5))
}

/// How far from its centre, in pixels, a computed tip's texels reach: its major radius plus the
/// hard edge's tail, or out to the end of a soft tip's falloff (bilinear placement adds a pixel,
/// which [`rect_around`]'s slack covers).
pub(crate) fn round_reach(d: &Dab, hardness: f32) -> f32 {
    let (ro, rm) = tip_axes(d);
    let major = rm / ro;
    let falloff = Falloff::new(hardness, rm);
    if falloff.is_hard() { major + falloff.reach_past_edge() } else { major * falloff.reach() }
}

/// The pixel rectangle `reach` (plus 1 px of anti-aliasing slack) around a dab's centre. The casts
/// saturate and the arithmetic saturates, so a far or non-finite centre or reach from a direct
/// caller gives a (possibly empty) rectangle instead of an `i32` overflow (#977).
pub(crate) fn rect_around(d: &Dab, reach: f32) -> Rect {
    let rr = (reach.ceil() as i32).saturating_add(1);
    let (cx, cy) = (d.center.x.floor() as i32, d.center.y.floor() as i32);
    Rect::new(cx.saturating_sub(rr), cy.saturating_sub(rr), cx.saturating_add(rr).saturating_add(1), cy.saturating_add(rr).saturating_add(1))
}

impl BrushContext {
    /// Prepare normal-sized sampled tips at preset selection, before the first pen-down.
    /// Larger assets remain lazy so selection cannot introduce an unbounded preparation job.
    pub fn prewarm(brush: &BrushSettings) {
        let samples = |tip: &TipShape| match tip {
            TipShape::Round => 0,
            TipShape::Sampled(g) => g.data.len(),
            TipShape::Sequence(v) => v.iter().fold(0usize, |n, g| n.saturating_add(g.data.len())),
        };
        let texture_samples = if brush.texture.enabled {
            match &brush.texture.pattern {
                Pattern::Tile(g) => g.data.len(),
                Pattern::Procedural { size, .. } => (*size as usize).saturating_mul(*size as usize),
            }
        } else {
            0
        };
        let n = samples(&brush.tip).saturating_add(samples(&brush.dual_brush.tip)).saturating_add(texture_samples);
        let colors = brush.csp_tip_colors.iter().flatten().fold(0usize, |n, c| n.saturating_add(c.rgb.len()));
        if n > 0 && n <= 4 << 20 && colors <= 12 << 20 {
            drop(Self::new(brush));
        }
    }

    pub fn new(brush: &BrushSettings) -> Self {
        let brush = brush.bounded_for_render();
        if let Some(mut context) = prepared_cache::get(&brush) {
            context.brush = Arc::new(brush);
            return context;
        }
        let context = Self::prepare(brush);
        prepared_cache::insert(&context);
        context
    }

    fn prepare(brush: BrushSettings) -> Self {
        let mips = |t: &TipShape| match t {
            TipShape::Sampled(g) if g.is_valid() => Some(Arc::new(Mips::new(g))),
            _ => None,
        };
        let sequence = |t: &TipShape| match t {
            TipShape::Sequence(tips) => tips.iter().take(64).filter(|g| g.is_valid()).map(|g| Arc::new(Mips::new(g))).collect(),
            _ => Vec::new(),
        };
        let (texture, texture_scale) = prepare_pattern(&brush);
        // Colour tips paint their own colours (except when erasing): per pixel, or their mean
        // colour when the brush mixes colours (the mixing reservoir holds one colour).
        let prepared: Vec<(usize, &GrayTile)> = match &brush.tip {
            _ if brush.erase || brush.csp_tip_colors.is_empty() => Vec::new(),
            TipShape::Sampled(g) if g.is_valid() => vec![(0, g)],
            TipShape::Sequence(tips) => tips.iter().enumerate().take(64).filter(|(_, g)| g.is_valid()).collect(),
            _ => Vec::new(),
        };
        let colors = |i: usize| brush.csp_tip_colors.get(i).and_then(Option::as_ref);
        let tip_means: Vec<_> = prepared.iter().map(|(i, g)| colors(*i)?.mean(g)).collect();
        let planes: Vec<_> = if brush.csp_mixing.is_some() { Vec::new() } else { prepared.iter().map(|(i, g)| colors(*i)?.premultiplied(g)).collect() };
        let tip_colors: Vec<_> = planes.iter().map(|p| p.as_ref().map(|p| Arc::new(p.each_ref().map(Mips::new)))).collect();
        let ribbon_tips = brush.csp_ribbon.map_or_else(Vec::new, |r| crate::csp_ribbon::prepare_tips(&brush.tip, r));
        // A ribbon drops a tip's empty last column or row (scale `k` > 1): its colours drop it too.
        let ribbon_colors: Vec<_> = if ribbon_tips.len() == planes.len() {
            let trim = |g: &GrayTile, k: [f32; 2]| {
                let (w, h) = (g.width - u32::from(k[0] > 1.0 && g.width > 1), g.height - u32::from(k[1] > 1.0 && g.height > 1));
                GrayTile::from_fn(w, h, |x, y| g.get(x, y))
            };
            planes.iter().zip(&ribbon_tips).map(|(p, (_, k))| p.as_ref().map(|p| Arc::new(p.each_ref().map(|g| Mips::new(&trim(g, *k)))))).collect()
        } else {
            Vec::new()
        };
        Self {
            tip: mips(&brush.tip),
            sequence: sequence(&brush.tip),
            dual_tip: if brush.dual_brush.enabled { mips(&brush.dual_brush.tip) } else { None },
            dual_sequence: if brush.dual_brush.enabled { sequence(&brush.dual_brush.tip) } else { Vec::new() },
            tip_colors: if tip_colors.iter().any(Option::is_some) { tip_colors } else { Vec::new() },
            tip_means: if tip_means.iter().any(Option::is_some) { tip_means } else { Vec::new() },
            ribbon_colors: if ribbon_colors.iter().any(Option::is_some) { ribbon_colors } else { Vec::new() },
            ribbon_tips,
            texture,
            texture_scale,
            brush: Arc::new(brush),
        }
    }

    /// Texture value at a document pixel. CSP uses integer-coordinate nearest sampling
    /// at or below 100%, and bilinear sampling above it; legacy textures use pixel centres.
    #[inline]
    pub fn texture_at(&self, x: i32, y: i32) -> f32 {
        let Some(p) = &self.texture else { return 1.0 };
        let texture = &self.brush.texture;
        let s = self.texture_scale.max(0.01);
        let value = if s == 1.0 {
            // Pixel centres land exactly on texels at 100 %.
            p.data.get(y.rem_euclid(p.height as i32) as usize * p.width + x.rem_euclid(p.width as i32) as usize).copied().unwrap_or(1.0)
        } else if texture.csp_mode.is_some() && texture.scale <= 1.0 {
            p.sample_wrap(x as f32 / s, y as f32 / s)
        } else {
            let (fx, fy) = if texture.csp_mode.is_some() { (x as f32 / s, y as f32 / s) } else { ((x as f32 + 0.5) / s - 0.5, (y as f32 + 0.5) / s - 0.5) };
            let (x0, y0) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - x0, fy - y0);
            let a = p.sample_wrap(x0, y0) + (p.sample_wrap(x0 + 1.0, y0) - p.sample_wrap(x0, y0)) * tx;
            let b = p.sample_wrap(x0, y0 + 1.0) + (p.sample_wrap(x0 + 1.0, y0 + 1.0) - p.sample_wrap(x0, y0 + 1.0)) * tx;
            a + (b - a) * ty
        };
        if texture.csp_mode.is_some() {
            // GrayTile stores luminance; CSP samples black/alpha coverage.
            crate::csp_texture::luminance(1.0 - value, texture.invert, texture.brightness, texture.effective_contrast())
        } else {
            value
        }
    }

    /// Bounds of the dab's tip square at any rotation.
    pub(crate) fn quad_rect(&self, d: &Dab) -> Rect {
        let scale = self.tip_scale(d, false);
        rect_around(d, d.radius * std::f32::consts::SQRT_2 * scale[0].max(scale[1]))
    }

    /// Whether a point (pixel space) lies in the dab's tip square, rotated and scaled with the
    /// tip: CSP colour mixing picks up every pixel there, transparent ones included.
    pub(crate) fn in_tip_quad(&self, d: &Dab, px: f32, py: f32) -> bool {
        let scale = self.tip_scale(d, false);
        let (ro, rm) = tip_axes(d);
        let (sn, cs) = d.angle.sin_cos();
        let (ux, uy) = (px - d.center.x as f32, -(py - d.center.y as f32));
        let (u, v) = ((ux * cs + uy * sn) / scale[0], (-ux * sn + uy * cs) / scale[1]);
        let (hu, hv) = match self.tip.as_deref().or_else(|| self.sequence.first().map(Arc::as_ref)) {
            Some(m) => {
                let (w, h, _) = &m.levels[0];
                let big = (*w.max(h)).max(1) as f32;
                (d.radius * *w as f32 / big, d.radius * ro * *h as f32 / big)
            }
            None => (rm / ro, rm),
        };
        u.abs() <= hu && v.abs() <= hv
    }

    fn texture_mask(&self, coverage: f32, x: i32, y: i32, density: f32, ceiling: f32) -> f32 {
        let texture = &self.brush.texture;
        let value = self.texture_at(x, y);
        if let Some(mode) = texture.csp_mode {
            crate::csp_texture::combine_with_ceiling(mode, coverage, value, density, ceiling)
        } else {
            mask_combine(texture.mode, coverage, value, density)
        }
    }

    /// Bounding rectangle of a dab (rotated sampled tips need the corner reach; computed tips their
    /// anti-aliased edge or soft tail).
    pub fn dab_rect(&self, d: &Dab, dual: bool) -> Rect {
        if let Some(slice) = d.ribbon.filter(|_| !dual) {
            return slice.rect();
        }
        let b = &self.brush;
        let (sampled, hardness) = if dual {
            (self.dual_tip.is_some() || !self.dual_sequence.is_empty(), b.dual_brush.hardness)
        } else {
            (self.tip.is_some() || !self.sequence.is_empty(), b.hardness)
        };
        let scale = self.tip_scale(d, dual);
        let reach = if sampled { d.radius * std::f32::consts::SQRT_2 } else { round_reach(d, hardness) };
        let reach = reach * scale[0].max(scale[1]);
        rect_around(d, reach)
    }

    fn tip_scale(&self, d: &Dab, dual: bool) -> [f32; 2] {
        if dual {
            return [1.0; 2];
        }
        self.brush.csp_tip_scale.map_or([1.0; 2], |scale| {
            // CSP keeps the thickness axis radius at least one pixel, without
            // restricting expansion to the base radius.
            let floor = (1.0 / d.radius.max(0.5)).min(1.0);
            scale.map(|axis| axis.max(floor))
        })
    }

    /// The prepared tip (index into `sequence`) of dab `d`, by the CSP tip order.
    fn tip_slot(&self, d: &Dab) -> usize {
        self.brush.csp_tip_order.unwrap_or_default().tip_index(d.index, self.sequence.len(), self.brush.seed)
    }

    /// Whether some tip paints its own colours (see [`Self::rasterize_tip_color`]).
    pub fn has_tip_colors(&self) -> bool {
        !self.tip_means.is_empty()
    }

    /// The mean colour of dab `d`'s colour tip (`None`: it paints in the brush colour).
    pub fn tip_mean_color(&self, d: &Dab) -> Option<[f32; 3]> {
        let slot = match (d.ribbon, self.sequence.len()) {
            (_, 0) => 0,
            (Some(slice), n) => slice.tip as usize % n,
            (None, _) => self.tip_slot(d),
        };
        self.tip_means.get(slot).copied().flatten()
    }

    /// The straight RGB of dab `d`'s colour tip at each pixel of `rect` (row-major), sampled exactly
    /// like its shape; `false` (and `out` untouched) when the dab's tip paints in the brush colour.
    pub fn rasterize_tip_color(&self, d: &Dab, rect: Rect, out: &mut Vec<[f32; 3]>) -> bool {
        if let Some(slice) = d.ribbon {
            // Pencil-style thresholds would not divide back into colours: those keep the mean.
            let slot = slice.tip as usize % self.ribbon_tips.len().max(1);
            let (Some((shape, k)), Some(Some(planes)), false) = (self.ribbon_tips.get(slot), self.ribbon_colors.get(slot), self.brush.aliased) else {
                return false;
            };
            let n = rect.width() as usize * rect.height() as usize;
            let mut alpha = vec![0.0; n];
            self.rasterize_ribbon(d, &slice, shape, *k, rect, &mut alpha, (false, false, false));
            out.clear();
            out.resize(n, [0.0; 3]);
            for (c, m) in planes.iter().enumerate() {
                let mut plane = vec![0.0; n];
                self.rasterize_ribbon(d, &slice, m, *k, rect, &mut plane, (false, false, false));
                for ((o, a), p) in out.iter_mut().zip(&alpha).zip(&plane) {
                    if *a > 1e-6 {
                        o[c] = (p / a).clamp(0.0, 1.0);
                    }
                }
            }
            return true;
        }
        if self.tip_colors.is_empty() {
            return false;
        }
        let slot = if self.sequence.is_empty() { 0 } else { self.tip_slot(d) };
        let shape = if self.sequence.is_empty() { self.tip.as_deref() } else { self.sequence.get(slot).map(Arc::as_ref) };
        let (Some(Some(planes)), Some(shape)) = (self.tip_colors.get(slot), shape) else { return false };
        let (mut alpha, mut plane) = (Vec::new(), Vec::new());
        self.rasterize_with(d, false, rect, &mut alpha, Some(shape));
        out.clear();
        out.resize(alpha.len(), [0.0; 3]);
        for (c, m) in planes.iter().enumerate() {
            self.rasterize_with(d, false, rect, &mut plane, Some(m));
            for ((o, a), p) in out.iter_mut().zip(&alpha).zip(&plane) {
                if *a > 1e-6 {
                    o[c] = (p / a).clamp(0.0, 1.0);
                }
            }
        }
        true
    }

    /// Rasterise a dab over `rect` into `out` (tip shape × noise × per-tip texture × wet edges × flow).
    pub fn rasterize(&self, d: &Dab, dual: bool, rect: Rect, out: &mut Vec<f32>) {
        self.rasterize_with(d, dual, rect, out, None);
    }

    /// [`Self::rasterize`], or with `plane` the raw samples of that plane (shaped and placed like
    /// the dab's tip, without effects or flow).
    fn rasterize_with(&self, d: &Dab, dual: bool, rect: Rect, out: &mut Vec<f32>, plane: Option<&Mips>) {
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        out.clear();
        out.resize(w * h, 0.0);
        let b = &self.brush;
        let (hardness, single, sequence) =
            if dual { (b.dual_brush.hardness, &self.dual_tip, &self.dual_sequence) } else { (b.hardness, &self.tip, &self.sequence) };
        let mips = match plane {
            Some(m) => Some(m),
            None if sequence.is_empty() => single.as_deref(),
            None if dual => sequence.get((d.index % sequence.len() as u64) as usize).map(Arc::as_ref),
            None => sequence.get(self.tip_slot(d)).map(Arc::as_ref),
        };
        let aliased = b.aliased && !dual;
        // CSP anti-aliasing (see `csp_tip_value`); level 0 samples the tip once, unfiltered.
        let csp_aa = b.csp_anti_alias.filter(|_| !dual && !aliased);
        let wet = b.wet_edges && !dual;
        let noise = b.noise && !dual;
        let tex_tip = !dual && b.texture.enabled && b.texture.each_tip && self.texture.is_some();
        if let Some(slice) = d.ribbon.filter(|_| !dual && plane.is_none()) {
            if let Some((m, k)) = self.ribbon_tips.get(slice.tip as usize % self.ribbon_tips.len().max(1)) {
                self.rasterize_ribbon(d, &slice, m, *k, rect, out, (noise, tex_tip, wet));
            }
            return;
        }
        let (cx, cy) = if aliased { grid_center(d.center.x, d.center.y, 2.0 * d.radius) } else { (d.center.x as f32, d.center.y as f32) };
        let (sn, cs) = d.angle.sin_cos();
        let (fx, fy) = (if d.flip_x { -1.0 } else { 1.0 }, if d.flip_y { -1.0 } else { 1.0 });
        let scale = self.tip_scale(d, dual);
        // Brush Projection: stretch the sampling coordinates along the tilt direction, which
        // foreshortens the tip there by `proj_scale`.
        let proj = (!dual && d.proj_scale < 0.999).then(|| {
            let (ps, pc) = d.proj_angle.sin_cos();
            (pc, ps, 1.0 / d.proj_scale.max(0.05) - 1.0)
        });
        let r = d.radius;
        let (ro, rm) = tip_axes(d);
        let (inv_major, inv_minor) = (ro / rm, 1.0 / rm);
        let falloff = Falloff::new(hardness, rm);
        // In q² (relative radius squared): beyond `outside_q2` the texels are empty; within
        // `inside_q2` they are full (-1: nowhere, for a tip narrower than its tail).
        let (outside_q2, inside_q2) = (falloff.reach().powi(2), if falloff.full_to() > 0.0 { falloff.full_to().powi(2) } else { -1.0 });
        // Pixel or texel offset from the dab centre (y down) into tip space: y up, projected,
        // rotated by −angle, flipped.
        let to_tip = |dx: f32, dy: f32| {
            let (mut ux, mut uy) = (dx, -dy);
            if let Some((ax, ay, k)) = proj {
                let t = (ux * ax + uy * ay) * k;
                ux += t * ax;
                uy += t * ay;
            }
            ((ux * cs + uy * sn) * fx / scale[0], (-ux * sn + uy * cs) * fy / scale[1])
        };
        // A computed tip's texel at tip-space (u, v): the relative radius on the ellipse (semi-axes
        // `major` along u, `rm` along v), and for a hard tip the signed distance to its edge in
        // pixels, to first order (q − 1) / |∇q| (exact on a circle), so the edge is the same width
        // all round a flattened tip. |∇q| ≤ 1 / rm, so |sd| ≥ |q − 1|·rm: the q² bounds skip the
        // work exactly where it can't change the texel.
        let texel = |u: f32, v: f32| {
            let (qu, qv) = (u * inv_major, v * inv_minor);
            let q2 = qu * qu + qv * qv;
            if q2 >= outside_q2 {
                return 0.0;
            }
            if q2 <= inside_q2 {
                return 1.0;
            }
            let q = q2.sqrt();
            if !falloff.is_hard() {
                return falloff.at(q, 0.0);
            }
            let (gu, gv) = (qu * inv_major, qv * inv_minor);
            let g = if q > 1e-6 { (gu * gu + gv * gv).sqrt() / q } else { 0.0 };
            falloff.at(q, if g > 0.0 { (q - 1.0) / g } else { -rm })
        };
        // Photoshop places a computed tip as a bitmap of texels one pixel apart, centred on the
        // dab, with bilinear interpolation: the texels at whole-pixel offsets from the centre
        // around the rectangle (one more column and row), and the weights of the far texels.
        let (ox, oy) = (rect.x0 as f32 + 0.5 - cx, rect.y0 as f32 + 0.5 - cy);
        let (gx0, gy0) = (ox.floor(), oy.floor());
        let (wx, wy) = (ox - gx0, oy - gy0);
        let computed = mips.is_none() && !aliased && csp_aa.is_none();
        if computed {
            let gw = w + 1;
            let mut texels = Vec::with_capacity(gw * (h + 1));
            for j in 0..=h {
                let ty = gy0 + j as f32;
                for i in 0..gw {
                    let (u, v) = to_tip(gx0 + i as f32, ty);
                    texels.push(texel(u, v));
                }
            }
            let (w00, w10, w01, w11) = ((1.0 - wx) * (1.0 - wy), wx * (1.0 - wy), (1.0 - wx) * wy, wx * wy);
            for (row, (r0, r1)) in out.chunks_exact_mut(w.max(1)).zip(texels.chunks_exact(gw).zip(texels.chunks_exact(gw).skip(1))) {
                for (o, (a, b)) in row.iter_mut().zip(r0.windows(2).zip(r1.windows(2))) {
                    *o = a[0] * w00 + a[1] * w10 + b[0] * w01 + b[1] * w11;
                }
            }
            // Without per-pixel effects the placed texels times flow are the dab.
            if !(noise || tex_tip || wet) {
                if d.alpha != 1.0 {
                    out.iter_mut().for_each(|o| *o *= d.alpha);
                }
                return;
            }
        }
        // Sampled-tip mapping.
        let (level, inv_scale, tw, th) = match mips {
            Some(m) => {
                let (w0, h0, _) = &m.levels[0];
                let big = *w0.max(h0) as f32;
                let display = 2.0 * r * ro.max(0.25);
                let level = match csp_aa {
                    // CSP: weak samples the original tip, medium the level at least as large as the
                    // dab, strong the level at least half as large.
                    Some(0 | 1) => 0,
                    Some(2) => m.level_for(display),
                    Some(_) => m.level_for(display * 0.5),
                    None => m.level_for(display),
                };
                (level, big / (2.0 * r), *w0 as f32, *h0 as f32)
            }
            None => (0, 1.0, 1.0, 1.0),
        };
        // A CSP tip value at a pixel-space offset from the dab centre: the nearest texel of the
        // chosen level (a computed tip is its exact shape, box-filtered for medium/strong).
        let csp_tip_value = |dx: f32, dy: f32| -> f32 {
            let (u, v) = to_tip(dx, dy);
            match mips {
                Some(m) => {
                    let (px, py) = (u * inv_scale + tw / 2.0, -(v / ro) * inv_scale + th / 2.0);
                    m.sample_nearest(level, px / tw, py / th)
                }
                None => {
                    let box_px = match csp_aa {
                        Some(2) => 1.0,
                        Some(3..) => 2.0,
                        _ => 0.0,
                    };
                    if box_px <= 0.0 || !falloff.is_hard() {
                        return if falloff.is_hard() { if (u * inv_major).powi(2) + (v * inv_minor).powi(2) <= 1.0 { 1.0 } else { 0.0 } } else { texel(u, v) };
                    }
                    let mut sum = 0.0;
                    for j in 0..4 {
                        for i in 0..4 {
                            let (ox, oy) = ((i as f32 + 0.5) / 4.0 - 0.5, (j as f32 + 0.5) / 4.0 - 0.5);
                            let (su, sv) = to_tip(dx + ox * box_px, dy + oy * box_px);
                            if (su * inv_major).powi(2) + (sv * inv_minor).powi(2) <= 1.0 {
                                sum += 1.0 / 16.0;
                            }
                        }
                    }
                    sum
                }
            }
        };
        for yy in 0..h {
            let y = rect.y0 + yy as i32;
            let dy = y as f32 + 0.5 - cy;
            for xx in 0..w {
                let x = rect.x0 + xx as i32;
                let dx = x as f32 + 0.5 - cx;
                let (val, rn) = match (csp_aa, mips) {
                    // CSP: one sample at the pixel centre without anti-aliasing; otherwise the mean
                    // of 2×2 samples half a pixel apart around it.
                    (Some(aa), _) => {
                        let val = if aa == 0 {
                            csp_tip_value(dx, dy)
                        } else {
                            let (x0, y0) = (dx - 0.25, dy - 0.25);
                            (csp_tip_value(x0, y0) + csp_tip_value(x0 + 0.5, y0) + csp_tip_value(x0, y0 + 0.5) + csp_tip_value(x0 + 0.5, y0 + 0.5)) * 0.25
                        };
                        if val <= 0.0 {
                            continue;
                        }
                        let rn = if mips.is_some() {
                            1.0 - val
                        } else if wet {
                            let (u, v) = to_tip(dx, dy);
                            (u * inv_major).hypot(v * inv_minor)
                        } else {
                            0.0
                        };
                        (val, rn)
                    }
                    (None, None) if aliased => {
                        let (u, v) = to_tip(dx, dy);
                        let d2 = (u * ro).powi(2) + v * v;
                        if d2 > rm * rm {
                            continue;
                        }
                        (1.0, d2.sqrt() / rm)
                    }
                    (None, None) => {
                        // The placed texels (above); only Wet Edges reads the relative radius.
                        let val = out.get(yy * w + xx).copied().unwrap_or(0.0);
                        if val <= 0.0 {
                            continue;
                        }
                        let rn = if wet {
                            let (u, v) = to_tip(dx, dy);
                            (u * inv_major).hypot(v * inv_minor)
                        } else {
                            0.0
                        };
                        (val, rn)
                    }
                    (None, Some(m)) => {
                        let (u, v) = to_tip(dx, dy);
                        let (px, py) = (u * inv_scale + tw / 2.0, -(v / ro) * inv_scale + th / 2.0);
                        if px < -1.0 || py < -1.0 || px > tw + 1.0 || py > th + 1.0 {
                            continue;
                        }
                        let s = m.sample_clamped(level, px / tw, py / th);
                        let val = if aliased && plane.is_none() { if s >= 0.5 { 1.0 } else { 0.0 } } else { s };
                        (val, 1.0 - s)
                    }
                };
                if val <= 0.0 {
                    continue;
                }
                out[yy * w + xx] = if plane.is_some() { val } else { self.shade(d, val, rn, x, y, (noise, tex_tip, wet)) };
            }
        }
    }

    /// A tip value at pixel (x, y) as the dab's coverage: noise, per-tip texture and wet edges
    /// (`effects`), times flow. `rn` is the relative radius, or 1 − value for sampled tips.
    #[inline]
    fn shade(&self, d: &Dab, mut val: f32, rn: f32, x: i32, y: i32, effects: (bool, bool, bool)) -> f32 {
        let (noise, tex_tip, wet) = effects;
        let b = &self.brush;
        if noise && val < 1.0 {
            let n = hash2(x, y, NOISE_SALT);
            let hard = if n < val { 1.0 } else { 0.0 };
            val += (hard - val) * 0.7;
        }
        if tex_tip && b.texture.csp_mode.is_none() {
            val = self.texture_mask(val, x, y, d.depth, 1.0);
        }
        if wet {
            val *= 0.5 + 0.5 * smoothstep(0.5, 1.0, rn);
        }
        let coverage = val * d.alpha;
        if tex_tip && b.texture.csp_mode.is_some() { self.texture_mask(coverage, x, y, d.depth, d.alpha) } else { coverage }
    }

    /// Rasterise a CSP Ribbon slice: each pixel centre is located in the slice's quadrilateral
    /// and samples the tip there (bilinear on a mip level, nearest for CSP anti-aliasing off).
    /// Pixel centres belong to one slice each, so consecutive slices join without seams.
    #[allow(clippy::too_many_arguments)]
    fn rasterize_ribbon(
        &self,
        d: &Dab,
        slice: &crate::csp_ribbon::RibbonSlice,
        m: &Mips,
        k: [f32; 2],
        rect: Rect,
        out: &mut [f32],
        effects: (bool, bool, bool),
    ) {
        let b = &self.brush;
        let w = rect.width() as usize;
        let Some((w0, h0, _)) = m.levels.first() else { return };
        // The full tip's size (the slice geometry's), before trimming.
        let (tw, th) = (*w0 as f32 * k[0], *h0 as f32 * k[1]);
        let nearest = b.aliased || b.csp_anti_alias == Some(0);
        let level = match b.csp_anti_alias {
            Some(0 | 1) => 0,
            _ => m.level_for(tw.max(th) / slice.texels_per_px(tw, th)),
        };
        let (lw, lh) = m.levels.get(if nearest { 0 } else { level }).map_or((1.0, 1.0), |(w, h, _)| ((*w).max(1) as f32, (*h).max(1) as f32));
        for (yy, row) in out.chunks_exact_mut(w.max(1)).enumerate() {
            let y = rect.y0.saturating_add(yy as i32);
            // Pixel centres within half a pixel of the outline's span on this row.
            let span = [0.0, 0.5, 1.0].into_iter().filter_map(|k| slice.row_span(f64::from(y) + k)).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
            let Some((lo, hi)) = span else { continue };
            let from = ((lo - f64::from(rect.x0) - 1.0).floor().max(0.0) as usize).min(row.len());
            let to = ((hi - f64::from(rect.x0) + 1.0).ceil().max(0.0) as usize).min(row.len());
            for (xx, o) in row.iter_mut().enumerate().take(to).skip(from) {
                let x = rect.x0.saturating_add(xx as i32);
                let Some((s, t)) = slice.locate(f64::from(x) + 0.5, f64::from(y) + 0.5) else { continue };
                let (mut u, mut v) = slice.tip_uv(s, t, tw, th);
                if d.flip_x {
                    u = 1.0 - u;
                }
                if d.flip_y {
                    v = 1.0 - v;
                }
                let (mut u, mut v) = (u * k[0], v * k[1]);
                // Consecutive copies of the tip join: along the ribbon the tip's first and last
                // texels extend to its ends instead of fading into the transparent outside.
                if slice.quarter_turns.is_multiple_of(2) {
                    v = v.clamp(0.5 / lh, 1.0 - 0.5 / lh);
                } else {
                    u = u.clamp(0.5 / lw, 1.0 - 0.5 / lw);
                }
                let val = if nearest { m.sample_nearest(0, u, v) } else { m.sample_clamped(level, u, v) };
                let val = if b.aliased { if val >= 0.5 { 1.0 } else { 0.0 } } else { val };
                if val > 0.0 {
                    *o = self.shade(d, val, 1.0 - val, x, y, effects);
                }
            }
        }
    }

    /// One dab's dense footprint (for the sequential retouch tools).
    pub fn footprint(&self, d: &Dab, index: usize) -> Footprint {
        let rect = self.dab_rect(d, false);
        let mut cov = Vec::new();
        self.rasterize(d, false, rect, &mut cov);
        Footprint { dab: *d, index, rect, cov }
    }

    /// Stroke-level masks applied to accumulated coverage `c` at a pixel (a CSP adding dual
    /// brush adds its coverage instead, also where `c` is 0).
    #[inline]
    pub fn stroke_mask(&self, c: f32, dual: Option<f32>, x: i32, y: i32) -> f32 {
        let b = &self.brush;
        let mut m = c;
        if let Some(dv) = dual {
            m = match b.dual_brush.csp_add {
                Some(add) => add.combine(m, dv),
                None => mask_combine(b.dual_brush.mode, m, dv, 1.0),
            };
        }
        if self.texture.is_some() && !b.texture.each_tip {
            m = self.texture_mask(m, x, y, b.texture.depth.clamp(0.0, 1.0), 1.0);
        }
        m
    }
}

// ---------------------------------------------------------------------------
// Sparse stroke buffer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CovTile {
    cov: Vec<f32>,
    /// Premultiplied native colour channels (`nc` per pixel), empty without per-dab colour.
    col: Vec<f32>,
}

/// A sparse tiled coverage (+ optional colour) buffer.
#[derive(Clone, Debug, Default)]
pub struct CoverageMap {
    tiles: HashMap<(i32, i32), CovTile>,
    nc: usize,
    bounds: Rect,
    dirty: HashSet<(i32, i32)>,
    /// Each dab paints over the colour already stored (by its own tip value), even where the
    /// coverage is full: CSP colour mixing lays every dab onto the layer in turn.
    pub(crate) paint_over: bool,
}

impl CoverageMap {
    pub fn new(color_channels: usize) -> Self {
        Self { nc: color_channels, ..Default::default() }
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        let (tx, ty) = (x.div_euclid(COV_TILE), y.div_euclid(COV_TILE));
        self.tiles.get(&(tx, ty)).map_or(0.0, |t| t.cov[((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize])
    }

    /// Accumulate dab values over `rect`: flow builds up towards the dab's opacity ceiling
    /// (`c ← c + v·(ceil − c)`), or with `max` (wet edges). `color` = native colour for per-dab colour.
    pub fn accumulate(&mut self, rect: Rect, vals: &[f32], ceil: f32, use_max: bool, color: Option<&[f32]>) {
        self.accumulate_with(rect, vals, ceil, use_max, color.map(|c| (c, false)));
    }

    /// [`Self::accumulate`] with a native colour per pixel of `rect` (`colors`: row-major, this
    /// map's colour channels a pixel), as a colour tip paints.
    pub fn accumulate_pixels(&mut self, rect: Rect, vals: &[f32], ceil: f32, use_max: bool, colors: &[f32]) {
        self.accumulate_with(rect, vals, ceil, use_max, Some((colors, true)));
    }

    /// `color`: the dab's colour, and whether it holds one per pixel.
    fn accumulate_with(&mut self, rect: Rect, vals: &[f32], ceil: f32, use_max: bool, color: Option<(&[f32], bool)>) {
        if rect.is_empty() {
            return;
        }
        self.bounds = self.bounds.union(&rect);
        let w = rect.width() as usize;
        let nc = self.nc;
        let (t0x, t0y) = (rect.x0.div_euclid(COV_TILE), rect.y0.div_euclid(COV_TILE));
        let (t1x, t1y) = ((rect.x1 - 1).div_euclid(COV_TILE), (rect.y1 - 1).div_euclid(COV_TILE));
        let want_col = color.is_some() && nc > 0;
        for ty in t0y..=t1y {
            for tx in t0x..=t1x {
                let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&rect);
                if tr.is_empty() {
                    continue;
                }
                // Skip tiles this dab doesn't touch.
                let any = (tr.y0..tr.y1).any(|y| {
                    let row = (y - rect.y0) as usize * w;
                    vals[row + (tr.x0 - rect.x0) as usize..row + (tr.x1 - rect.x0) as usize].iter().any(|&v| v > 0.0)
                });
                if !any {
                    continue;
                }
                let tile = self.tiles.entry((tx, ty)).or_insert_with(|| CovTile {
                    cov: vec![0.0; (COV_TILE * COV_TILE) as usize],
                    col: if want_col { vec![0.0; (COV_TILE * COV_TILE) as usize * nc] } else { Vec::new() },
                });
                if want_col && tile.col.is_empty() {
                    tile.col = vec![0.0; (COV_TILE * COV_TILE) as usize * nc];
                }
                self.dirty.insert((tx, ty));
                for y in tr.y0..tr.y1 {
                    let row = (y - rect.y0) as usize * w;
                    let trow = ((y - ty * COV_TILE) * COV_TILE) as usize;
                    for x in tr.x0..tr.x1 {
                        let v = vals[row + (x - rect.x0) as usize];
                        if v <= 0.0 {
                            continue;
                        }
                        let ti = trow + (x - tx * COV_TILE) as usize;
                        let c = tile.cov[ti];
                        let nv = if use_max {
                            c.max(v * ceil)
                        } else if ceil > c {
                            c + v * (ceil - c)
                        } else {
                            c
                        };
                        if nv <= c && !self.paint_over {
                            continue;
                        }
                        if let (true, Some((all, per_pixel))) = (want_col, color) {
                            let at = (row + (x - rect.x0) as usize) * nc;
                            let Some(col) = (if per_pixel { all.get(at..at + nc) } else { all.get(..nc) }) else { continue };
                            let k = if self.paint_over {
                                (v * ceil).clamp(0.0, 1.0)
                            } else if c < 1.0 {
                                (nv - c) / (1.0 - c)
                            } else {
                                0.0
                            };
                            let p = &mut tile.col[ti * nc..(ti + 1) * nc];
                            for i in 0..nc {
                                p[i] = col[i] * k + p[i] * (1.0 - k);
                            }
                        }
                        tile.cov[ti] = nv;
                    }
                }
            }
        }
    }

    /// Premultiplied native colour at a pixel, when this map keeps per-dab colour.
    pub(crate) fn color_at(&self, x: i32, y: i32) -> Option<&[f32]> {
        let (tx, ty) = (x.div_euclid(COV_TILE), y.div_euclid(COV_TILE));
        let t = self.tiles.get(&(tx, ty))?;
        let i = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
        t.col.get(i * self.nc..(i + 1) * self.nc)
    }

    /// Dense coverage over `rect` (rows), zero where no tile exists.
    fn read_dense(&self, rect: Rect) -> Vec<f32> {
        let w = rect.width() as usize;
        let mut out = vec![0.0f32; w * rect.height() as usize];
        for (tx, ty) in tile_keys(rect) {
            let Some(t) = self.tiles.get(&(tx, ty)) else { continue };
            let tr = tile_rect(tx, ty).intersect(&rect);
            for y in tr.y0..tr.y1 {
                let src = ((y - ty * COV_TILE) * COV_TILE + (tr.x0 - tx * COV_TILE)) as usize;
                let dst = (y - rect.y0) as usize * w + (tr.x0 - rect.x0) as usize;
                let len = tr.width() as usize;
                if let (Some(d), Some(s)) = (out.get_mut(dst..dst + len), t.cov.get(src..src + len)) {
                    d.copy_from_slice(s);
                }
            }
        }
        out
    }

    fn take_dirty(&mut self) -> Vec<(i32, i32)> {
        let mut v: Vec<_> = self.dirty.drain().collect();
        v.sort_unstable();
        v
    }

    /// Union two passes of the same brush stroke without applying opacity twice where they
    /// overlap. The stronger coverage wins, including its per-dab colour when present.
    fn union_max(&mut self, other: &Self) {
        self.bounds = self.bounds.union(&other.bounds);
        for (&key, source) in &other.tiles {
            let Some(target) = self.tiles.get_mut(&key) else {
                self.tiles.insert(key, source.clone());
                self.dirty.insert(key);
                continue;
            };
            for (index, (dst, src)) in target.cov.iter_mut().zip(&source.cov).enumerate() {
                if *src > *dst {
                    *dst = *src;
                    let start = index.saturating_mul(self.nc);
                    let end = start.saturating_add(self.nc);
                    if let (Some(dst_color), Some(src_color)) = (target.col.get_mut(start..end), source.col.get(start..end)) {
                        dst_color.copy_from_slice(src_color);
                    }
                }
            }
            self.dirty.insert(key);
        }
    }
}

// ---------------------------------------------------------------------------
// Stroke renderer
// ---------------------------------------------------------------------------

/// Incremental stroke rendering: feed points, composite dirty tiles from the pre-stroke pixels.
#[derive(Clone, Debug)]
pub struct StrokeRenderer {
    pub ctx: BrushContext,
    generator: DabGenerator,
    cov: CoverageMap,
    dual: Option<CoverageMap>,
    fmt: Option<PixelFormat>,
    per_dab_color: bool,
    dabs_done: usize,
    scratch: Vec<f32>,
    dab_buf: Vec<Dab>,
    dual_buf: Vec<Dab>,
    all_dabs: Option<Vec<Dab>>,
    mixing: Option<Box<MixState>>,
}

/// CSP color mixing state for one stroke (see [`crate::csp_mixing`]).
#[derive(Clone, Debug)]
struct MixState {
    stretch: f32,
    perceptual: bool,
    reservoir: water_mix::Reservoir,
    pending: water_mix::Samples,
    /// The layer the stroke paints on (pre-stroke pixels), when known.
    source: Option<Surface>,
}

impl StrokeRenderer {
    /// Composite the union of this stroke and a mirrored pass as one stroke. This keeps
    /// overlapping dabs on a symmetry axis under one opacity ceiling at every bit depth.
    pub fn composite_union(&self, other: &Self, pre: &Surface, target: &mut Surface, selection: Option<&Surface>, lock_transparency: bool) -> Rect {
        let mut merged = self.clone();
        merged.cov.union_max(&other.cov);
        if let (Some(to), Some(from)) = (&mut merged.dual, &other.dual) {
            to.union_max(from);
        }
        merged.composite(pre, target, selection, lock_transparency, true)
    }
    /// `fmt` = target pixel format (needed for per-dab colour); `zoom` for smoothing.
    pub fn new(brush: &BrushSettings, fmt: Option<PixelFormat>, zoom: f32) -> Self {
        let by_view;
        let brush = match brush.csp_size_by_view_at(zoom) {
            Some(b) => {
                by_view = b;
                &by_view
            }
            None => brush,
        };
        let mixing = brush.csp_mixing.as_ref().filter(|_| !brush.erase && fmt.is_some()).map(|m| {
            Box::new(MixState {
                stretch: m.bounded().stretch,
                perceptual: m.perceptual,
                reservoir: Default::default(),
                pending: water_mix::Samples::new(m.perceptual),
                source: None,
            })
        });
        let ctx = BrushContext::new(brush);
        let per_dab_color =
            (brush.color_dynamics.enabled || brush.csp_sub_color.is_some() || mixing.is_some() || ctx.has_tip_colors()) && !brush.erase && fmt.is_some();
        // Color mixing also keeps each dab's target alpha after the colour channels.
        let nc = if per_dab_color { fmt.map_or(0, |f| f.mode.color_channels()) + usize::from(mixing.is_some()) } else { 0 };
        Self {
            ctx,
            generator: DabGenerator::new(brush, zoom),
            cov: CoverageMap { paint_over: mixing.is_some(), ..CoverageMap::new(nc) },
            dual: brush.dual_brush.enabled.then(|| CoverageMap::new(0)),
            fmt,
            per_dab_color,
            dabs_done: 0,
            scratch: Vec::new(),
            dab_buf: Vec::new(),
            dual_buf: Vec::new(),
            all_dabs: None,
            mixing,
        }
    }

    /// The pre-stroke pixels of the layer being painted: CSP color mixing picks colour up from
    /// them (without it, only from the stroke itself).
    pub fn with_mixing_source(mut self, layer: &Surface) -> Self {
        if let Some(m) = self.mixing.as_mut() {
            m.source = Some(layer.clone());
        }
        self
    }

    /// The stroke's whole input is known up front (committing): enables the CSP end taper.
    pub fn with_full_path(mut self, pts: &[StrokePoint]) -> Self {
        self.generator.set_full_path(pts);
        self
    }

    /// Keep a copy of every primary dab (for tests and sequential tools).
    pub fn record_dabs(mut self) -> Self {
        self.all_dabs = Some(Vec::new());
        self
    }
    pub fn dabs(&self) -> &[Dab] {
        self.all_dabs.as_deref().unwrap_or(&[])
    }
    pub fn dab_count(&self) -> usize {
        self.dabs_done
    }
    /// Union of the dab rectangles so far (the dual brush's too when it adds paint).
    pub fn bounds(&self) -> Rect {
        match self.adding_dual() {
            Some(d) => self.cov.bounds.union(&d.bounds),
            None => self.cov.bounds,
        }
    }

    /// The dual brush's coverage when it adds paint outside the stroke (a CSP adding mode): its
    /// pixels composite even where the stroke has no coverage.
    fn adding_dual(&self) -> Option<&CoverageMap> {
        self.dual.as_ref().filter(|_| self.ctx.brush.dual_brush.csp_add.is_some())
    }

    fn raster_pending(&mut self) {
        let mut dabs = std::mem::take(&mut self.dab_buf);
        let mut duals = std::mem::take(&mut self.dual_buf);
        let wet = self.ctx.brush.wet_edges;
        let mut native = [0.0f32; 8];
        for d in dabs.iter_mut() {
            let rect = self.ctx.dab_rect(d, false);
            self.ctx.rasterize(d, false, rect, &mut self.scratch);
            // A colour tip's dab colour: its mean (mixing and ribbons lay one colour a dab).
            if let Some([r, g, b]) = self.ctx.tip_mean_color(d).filter(|_| self.per_dab_color) {
                d.color = [r, g, b, d.color[3]];
            }
            // Color mixing: the dab's colour and target alpha come from the picked-up paint.
            let mut target_alpha = None;
            if let (Some(m), Some(f)) = (self.mixing.as_mut(), self.fmt) {
                let ctx = &self.ctx;
                if !m.reservoir.picked() && m.pending.is_empty() {
                    // Pen-down: pick up what is under the first dab.
                    sample_under_dab(m, &self.cov, &f, d.color, ctx.quad_rect(d), &|x, y| ctx.in_tip_quad(d, x, y));
                }
                m.reservoir.update(&m.pending, m.stretch);
                m.pending = water_mix::Samples::new(m.perceptual);
                let (color, opacity) = m.reservoir.mix(d.color, d.paint_amount, d.paint_density, m.perceptual);
                d.color = color;
                target_alpha = Some((d.opacity * opacity).clamp(0.0, 1.0));
            }
            let col = match (self.per_dab_color, self.fmt) {
                (true, Some(f)) => {
                    from_rgba_into(&f, d.color, &mut native);
                    let nc = f.mode.color_channels();
                    match target_alpha {
                        // The target alpha rides after the colour channels.
                        Some(t) => {
                            let i = nc.min(native.len() - 1);
                            native[i] = t;
                            Some(&native[..=i])
                        }
                        None => Some(&native[..nc]),
                    }
                }
                _ => None,
            };
            // Mixing dabs blend towards their own colour and alpha: they build up to full weight.
            let ceiling = if target_alpha.is_some() { 1.0 } else { d.opacity };
            // A colour tip lays its own colour at each pixel.
            let mut tip_rgb = Vec::new();
            match self.fmt.filter(|_| self.per_dab_color && self.ctx.rasterize_tip_color(d, rect, &mut tip_rgb)) {
                Some(f) => {
                    let nc = f.mode.color_channels().min(native.len());
                    let mut colors = Vec::with_capacity(tip_rgb.len() * nc);
                    for [r, g, b] in &tip_rgb {
                        from_rgba_into(&f, [*r, *g, *b, 1.0], &mut native);
                        colors.extend_from_slice(&native[..nc]);
                    }
                    self.cov.accumulate_pixels(rect, &self.scratch, ceiling, wet, &colors);
                }
                None => self.cov.accumulate(rect, &self.scratch, ceiling, wet, col),
            }
            if let (Some(m), Some(f)) = (self.mixing.as_mut(), self.fmt) {
                let ctx = &self.ctx;
                // The pixels under this dab, as it leaves them, feed the next dab's pick-up.
                sample_under_dab(m, &self.cov, &f, d.color, ctx.quad_rect(d), &|x, y| ctx.in_tip_quad(d, x, y));
            }
            // Bounds track the full dab rectangle (even fully transparent parts), like the damage.
            self.cov.bounds = self.cov.bounds.union(&rect);
        }
        if let Some(dm) = self.dual.as_mut() {
            for d in &duals {
                let rect = self.ctx.dab_rect(d, true);
                self.ctx.rasterize(d, true, rect, &mut self.scratch);
                dm.accumulate(rect, &self.scratch, 1.0, false, None);
            }
        }
        self.dabs_done += dabs.len();
        if let Some(all) = self.all_dabs.as_mut() {
            all.extend_from_slice(&dabs);
        }
        dabs.clear();
        duals.clear();
        self.dab_buf = dabs;
        self.dual_buf = duals;
    }

    /// Feed input points (any chunking gives the same result).
    pub fn push(&mut self, pts: &[StrokePoint]) {
        self.generator.push(pts, &mut self.dab_buf, &mut self.dual_buf);
        self.raster_pending();
    }

    /// End of stroke (smoothing catch-up, a lone first point).
    pub fn finish(&mut self) {
        self.generator.finish(&mut self.dab_buf, &mut self.dual_buf);
        self.raster_pending();
    }

    /// What finishing the stroke now would add (the smoothing catch-up tail to the last point, or
    /// a lone first dab), for live previews: a renderer holding copies of the coverage tiles the
    /// tail touches with the tail rendered in, so its `composite` draws exactly those tiles as
    /// [`finish`](Self::finish) would leave them. `None` when finishing adds nothing.
    pub fn tail_preview(&self) -> Option<StrokeRenderer> {
        let mut generator = self.generator.clone();
        let (mut dabs, mut duals) = (Vec::new(), Vec::new());
        generator.finish(&mut dabs, &mut duals);
        if dabs.is_empty() && duals.is_empty() {
            return None;
        }
        // The live watercolor edge reads coverage `reach` pixels around the tiles it redraws.
        let reach = self.water_reach(false);
        let (mut keys, mut redraw) = (HashSet::new(), HashSet::new());
        for r in dabs.iter().map(|d| self.ctx.dab_rect(d, false)).chain(duals.iter().map(|d| self.ctx.dab_rect(d, true))) {
            if r.is_empty() {
                continue;
            }
            keys.extend(tile_keys(r.inflate(reach.saturating_mul(2))));
            redraw.extend(tile_keys(r.inflate(reach)));
        }
        let subset = |m: &CoverageMap| CoverageMap {
            tiles: keys.iter().filter_map(|k| m.tiles.get(k).map(|t| (*k, t.clone()))).collect(),
            nc: m.nc,
            bounds: m.bounds,
            dirty: HashSet::new(),
            paint_over: m.paint_over,
        };
        let mut t = StrokeRenderer {
            ctx: self.ctx.clone(),
            generator,
            cov: subset(&self.cov),
            dual: self.dual.as_ref().map(subset),
            fmt: self.fmt,
            per_dab_color: self.per_dab_color,
            dabs_done: self.dabs_done,
            scratch: Vec::new(),
            dab_buf: dabs,
            dual_buf: duals,
            all_dabs: None,
            mixing: self.mixing.clone(),
        };
        t.raster_pending();
        // Tiles whose coverage the tail doesn't change still composite the same: redraw them all.
        let all: Vec<_> = t.cov.tiles.keys().copied().filter(|k| reach == 0 || redraw.contains(k)).collect();
        t.cov.dirty.extend(all);
        if let Some(d) = t.dual.as_mut().filter(|_| t.ctx.brush.dual_brush.csp_add.is_some()) {
            let all: Vec<_> = d.tiles.keys().copied().collect();
            d.dirty.extend(all);
        }
        Some(t)
    }

    /// Mark the coverage tiles over `r` for the next `composite` (e.g. to redraw over a preview).
    pub fn mark_dirty(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        for ty in r.y0.div_euclid(COV_TILE)..=(r.y1 - 1).div_euclid(COV_TILE) {
            for tx in r.x0.div_euclid(COV_TILE)..=(r.x1 - 1).div_euclid(COV_TILE) {
                if self.cov.tiles.contains_key(&(tx, ty)) {
                    self.cov.dirty.insert((tx, ty));
                }
                if let Some(d) = self.dual.as_mut().filter(|d| d.tiles.contains_key(&(tx, ty)))
                    && self.ctx.brush.dual_brush.csp_add.is_some()
                {
                    d.dirty.insert((tx, ty));
                }
            }
        }
    }

    /// The coverage tiles touched since the last call (or `composite`), as one rectangle clipped to
    /// the stroke bounds, and forget them: what a live preview that composites its own paint has
    /// to redraw.
    pub fn take_dirty_rect(&mut self) -> Rect {
        let adds = self.ctx.brush.dual_brush.csp_add.is_some();
        let mut keys = self.cov.take_dirty();
        if let Some(d) = self.dual.as_mut() {
            if adds {
                keys.extend(d.take_dirty());
            }
            d.dirty.clear();
        }
        let r = keys.into_iter().fold(Rect::EMPTY, |acc, (tx, ty)| acc.union(&tile_rect(tx, ty)));
        r.intersect(&self.bounds())
    }

    /// Final coverage over `r`, one value per pixel in rows (as [`dense_coverage`](Self::dense_coverage)
    /// over its bounds).
    pub fn coverage_in(&self, r: Rect) -> Vec<f32> {
        (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| (x, y))).map(|(x, y)| self.coverage_at(x, y)).collect()
    }

    /// Final stroke coverage at a pixel (stroke-level masks applied, before opacity/selection).
    pub fn coverage_at(&self, x: i32, y: i32) -> f32 {
        let c = self.cov.get(x, y);
        let dual = self.dual.as_ref().map(|d| d.get(x, y));
        if c <= 0.0 && (self.adding_dual().is_none() || dual.is_none_or(|t| t <= 0.0)) {
            return 0.0;
        }
        self.ctx.stroke_mask(c, dual, x, y)
    }

    /// Dense final coverage over the stroke bounds.
    pub fn dense_coverage(&self) -> (Rect, Vec<f32>) {
        let b = self.bounds();
        let w = b.width() as usize;
        let mut out = vec![0.0f32; w * b.height() as usize];
        for (&(tx, ty), t) in &self.cov.tiles {
            let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&b);
            for y in tr.y0..tr.y1 {
                for x in tr.x0..tr.x1 {
                    let c = t.cov[((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize];
                    if c > 0.0 {
                        out[(y - b.y0) as usize * w + (x - b.x0) as usize] = self.ctx.stroke_mask(c, self.dual.as_ref().map(|d| d.get(x, y)), x, y);
                    }
                }
            }
        }
        // An adding dual brush also paints the tiles the stroke itself doesn't reach.
        if let Some(d) = self.adding_dual() {
            for &(tx, ty) in d.tiles.keys().filter(|k| !self.cov.tiles.contains_key(k)) {
                let tr = tile_rect(tx, ty).intersect(&b);
                for y in tr.y0..tr.y1 {
                    for x in tr.x0..tr.x1 {
                        if let Some(o) = out.get_mut((y - b.y0) as usize * w + (x - b.x0) as usize) {
                            *o = self.coverage_at(x, y);
                        }
                    }
                }
            }
        }
        (b, out)
    }

    /// Pixels of coverage the CSP watercolor edge reads around a redrawn pixel (0 when it is off
    /// for this kind of composite: "process after drag" only applies when `all`).
    fn water_reach(&self, all: bool) -> i32 {
        match self.ctx.brush.csp_water_edge.as_ref().map(|e| e.bounded()) {
            Some(e) if all || !e.after_drag => EdgeParams::new(&e).map_or(0, |p| p.n),
            _ => 0,
        }
    }

    fn water_pass(&self, all: bool) -> WaterPass {
        let Some(e) = self.ctx.brush.csp_water_edge.as_ref().map(|e| e.bounded()) else { return WaterPass::Off };
        let Some(params) = EdgeParams::new(&e) else { return WaterPass::Off };
        if e.after_drag && !all {
            return WaterPass::Off;
        }
        if !e.blurs() {
            return WaterPass::Live(params);
        }
        let bounds = self.bounds();
        let n = i32::try_from(water::gaussian_taps(e.blur).len().saturating_sub(1)).unwrap_or(i32::MAX);
        // Processed in 64-pixel aligned blocks around the stroke, like the source.
        let grown = bounds.inflate(n);
        let rect = Rect::new(
            grown.x0.div_euclid(COV_TILE).saturating_mul(COV_TILE),
            grown.y0.div_euclid(COV_TILE).saturating_mul(COV_TILE),
            grown.x1.saturating_add(COV_TILE - 1).div_euclid(COV_TILE).saturating_mul(COV_TILE),
            grown.y1.saturating_add(COV_TILE - 1).div_euclid(COV_TILE).saturating_mul(COV_TILE),
        );
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        if bounds.is_empty() || w.checked_mul(h).is_none_or(|a| a > MAX_WATER_FIELD) {
            return WaterPass::Live(params);
        }
        let mut field = vec![0u8; w * h];
        for &(tx, ty) in self.cov.tiles.keys() {
            let tr = tile_rect(tx, ty).intersect(&bounds);
            if tr.is_empty() {
                continue;
            }
            let amounts = water::live_amounts(&params, &self.cov.read_dense(tr.inflate(params.n)), tr);
            let tw = tr.width() as usize;
            for y in tr.y0..tr.y1 {
                for x in tr.x0..tr.x1 {
                    let a = amounts.get((y - tr.y0) as usize * tw + (x - tr.x0) as usize).copied().unwrap_or(0.0);
                    if let Some(slot) = field.get_mut((y - rect.y0) as usize * w + (x - rect.x0) as usize) {
                        *slot = water::to_u8(a);
                    }
                }
            }
        }
        water::blur_u8(&mut field, w, h, e.blur);
        let reach = e.blur.ceil() as i32;
        let halo = bounds.inflate(reach).intersect(&rect);
        WaterPass::Blurred { params, rect, reach, field, halo }
    }

    /// Composite the stroke buffer onto `target`, reading original pixels from `pre` (the
    /// pre-stroke surface; pass a clone of the target taken before the first `push`). With
    /// `all = false` only tiles touched since the last composite are processed (interactive use).
    /// Returns the damaged rectangle.
    pub fn composite(&mut self, pre: &Surface, target: &mut Surface, selection: Option<&Surface>, lock_transparency: bool, all: bool) -> Rect {
        let water = self.water_pass(all);
        let adds = self.ctx.brush.dual_brush.csp_add.is_some();
        let mut dirty: Vec<(i32, i32)> = if all {
            self.cov.dirty.clear();
            self.cov.tiles.keys().copied().collect()
        } else {
            self.cov.take_dirty()
        };
        if let Some(d) = self.dual.as_mut() {
            // An adding dual brush's own tiles composite too.
            if adds {
                if all {
                    dirty.extend(d.tiles.keys().copied());
                } else {
                    dirty.extend(d.take_dirty());
                }
                dirty.sort_unstable();
                dirty.dedup();
            }
            d.dirty.clear();
        }
        let bounds = self.bounds();
        // Tiles to redraw, each clipped to the part that can change.
        let mut jobs: HashMap<(i32, i32), Rect> = dirty.iter().map(|&(tx, ty)| ((tx, ty), tile_rect(tx, ty))).collect();
        let area = match &water {
            WaterPass::Off => bounds,
            WaterPass::Live(p) => {
                // A pixel's edge depends on the coverage within the radius: redraw that strip of
                // the neighbouring tiles too.
                if !all {
                    for &(tx, ty) in &dirty {
                        let reach = tile_rect(tx, ty).inflate(p.n);
                        for k in tile_keys(reach).filter(|k| self.cov.tiles.contains_key(k)) {
                            let strip = tile_rect(k.0, k.1).intersect(&reach);
                            jobs.entry(k).and_modify(|r| *r = r.union(&strip)).or_insert(strip);
                        }
                    }
                }
                bounds
            }
            WaterPass::Blurred { halo, reach, .. } => {
                // The blurred halo reaches at most the blur radius beyond the coverage tiles.
                for &(tx, ty) in &dirty {
                    for k in tile_keys(tile_rect(tx, ty).inflate(*reach)) {
                        jobs.entry(k).or_insert_with(|| tile_rect(k.0, k.1));
                    }
                }
                bounds.union(halo)
            }
        };
        let mut keys: Vec<((i32, i32), Rect)> = jobs.into_iter().collect();
        keys.sort_unstable_by_key(|(k, _)| *k);
        let darkness = match &water {
            WaterPass::Live(p) | WaterPass::Blurred { params: p, .. } => p.darkness,
            WaterPass::Off => 0.0,
        };
        let fmt = target.format();
        let n = fmt.channels();
        let a_idx = alpha_index(&fmt);
        let nc = fmt.mode.color_channels();
        let b = &self.ctx.brush;
        let opacity = b.opacity.clamp(0.0, 1.0);
        let adding = self.adding_dual();
        let erase_compare = b.csp_stroke == Some(crate::CspStrokeMode::EraseCompare);
        // Colour mixing lays paint its own way (CLIP STUDIO paints it in Normal).
        let csp_stroke = b.csp_stroke.filter(|_| self.mixing.is_none());
        let mut base_src = [0.0f32; 8];
        from_rgba_into(&fmt, b.color, &mut base_src);
        if let Some(a) = a_idx {
            base_src[a] = b.color[3];
        }
        let mut dmg = Rect::EMPTY;
        let mut region = Vec::new();
        for ((tx, ty), clip) in keys {
            let tile = self.cov.tiles.get(&(tx, ty));
            let tr = clip.intersect(&area);
            if tr.is_empty() {
                continue;
            }
            let w = tr.width() as usize;
            // Watercolor-edge alpha per pixel of `tr` (empty when there is none).
            let amounts = match &water {
                WaterPass::Live(p) if tile.is_some() => water::live_amounts(p, &self.cov.read_dense(tr.inflate(p.n)), tr),
                WaterPass::Blurred { rect, field, halo, .. } => {
                    let fw = rect.width() as usize;
                    let v: Vec<f32> = (tr.y0..tr.y1)
                        .flat_map(|y| (tr.x0..tr.x1).map(move |x| (x, y)))
                        .map(|(x, y)| {
                            if x < halo.x0 || x >= halo.x1 || y < halo.y0 || y >= halo.y1 {
                                return 0.0;
                            }
                            field.get((y - rect.y0) as usize * fw + (x - rect.x0) as usize).map_or(0.0, |&e| water::from_u8(e))
                        })
                        .collect();
                    if tile.is_none() && v.iter().all(|&a| a <= 0.0) {
                        continue;
                    }
                    v
                }
                _ => Vec::new(),
            };
            let dual_tile = adding.is_some_and(|d| d.tiles.contains_key(&(tx, ty)));
            if tile.is_none() && amounts.is_empty() && !dual_tile {
                continue;
            }
            pre.read_region_into(tr, &mut region);
            let sel = selection.map(|s| (s.channels(), s.read_region(tr)));
            for y in tr.y0..tr.y1 {
                for x in tr.x0..tr.x1 {
                    let ti = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
                    let c = tile.map_or(0.0, |t| t.cov[ti]);
                    let i = (y - tr.y0) as usize * w + (x - tr.x0) as usize;
                    let add = amounts.get(i).copied().unwrap_or(0.0);
                    let dual = if c > 0.0 || dual_tile { self.dual.as_ref().map(|d| d.get(x, y)) } else { None };
                    let dual_adds = dual_tile && dual.is_some_and(|t| t > 0.0);
                    if c <= 0.0 && add <= 0.0 && !dual_adds {
                        continue;
                    }
                    let m = if c > 0.0 || dual_adds { self.ctx.stroke_mask(c, dual, x, y) } else { 0.0 };
                    let alpha = m * opacity + add;
                    let s = sel.as_ref().map_or(1.0, |(sc, v)| v[i * sc]);
                    let mut k = alpha.min(1.0) * s;
                    if k <= 0.0 {
                        continue;
                    }
                    if b.mode == BlendMode::Dissolve {
                        k = if hash2(x, y, b.seed) < k { 1.0 } else { 0.0 };
                        if k == 0.0 {
                            continue;
                        }
                    }
                    let px = &mut region[i * n..(i + 1) * n];
                    if b.erase || erase_compare {
                        if let (Some(a), false) = (a_idx, lock_transparency) {
                            // Erase (compare) keeps whichever is more transparent.
                            px[a] = if erase_compare { px[a].min(1.0 - k) } else { px[a] * (1.0 - k) };
                        }
                        continue;
                    }
                    if lock_transparency && a_idx.is_some_and(|a| px[a] <= 0.0) {
                        continue;
                    }
                    if self.mixing.is_some() {
                        if let Some(t) = tile.filter(|_| c > 0.0)
                            && let Some(p) = t.col.get(ti * (nc + 1)..(ti + 1) * (nc + 1))
                        {
                            let mut paint = base_src;
                            for ch in 0..nc {
                                paint[ch] = p[ch] / c;
                            }
                            if let Some(a) = a_idx {
                                paint[a] = 1.0;
                            }
                            let target = (p[nc] / c).clamp(0.0, 1.0);
                            let d = to_rgba(&fmt, px);
                            let o = crate::csp_mixing::blend_towards(d, to_rgba(&fmt, &paint[..n]), target, k, lock_transparency);
                            let mut enc = [0.0f32; 8];
                            from_rgba_into(&fmt, o, &mut enc);
                            px.copy_from_slice(&enc[..n]);
                        }
                        continue;
                    }
                    let mut src = base_src;
                    if let (true, Some(t)) = (self.per_dab_color && c > 0.0, tile)
                        && !t.col.is_empty()
                    {
                        let p = &t.col[ti * nc..(ti + 1) * nc];
                        for ch in 0..nc {
                            src[ch] = p[ch] / c;
                        }
                    }
                    if darkness > 0.0 && add > 0.0 && alpha > 0.0 {
                        // Edge darkening by the share of the alpha the edge contributed.
                        let dark = water::darken(to_rgba(&fmt, &src[..n]), darkness, add / alpha);
                        let keep = a_idx.map(|a| src[a]);
                        from_rgba_into(&fmt, dark, &mut src);
                        if let (Some(a), Some(v)) = (a_idx, keep) {
                            src[a] = v;
                        }
                    }
                    if let Some(mode) = csp_stroke {
                        crate::csp_blend::stroke_native(mode, &fmt, px, &src[..n], k, opacity, lock_transparency);
                    } else if matches!(b.mode, BlendMode::Normal | BlendMode::Dissolve) {
                        over_native(&fmt, px, &src[..n], k, lock_transparency);
                    } else {
                        let d = to_rgba(&fmt, px);
                        let sr = to_rgba(&fmt, &src[..n]);
                        let mut o = prism_color::blend::composite(b.mode, d, sr, k);
                        if lock_transparency {
                            o[3] = d[3];
                        }
                        let mut enc = [0.0f32; 8];
                        from_rgba_into(&fmt, o, &mut enc);
                        px.copy_from_slice(&enc[..n]);
                    }
                }
            }
            target.write_region(tr, &region);
            dmg = dmg.union(&tr);
        }
        if all { area } else { dmg }
    }
}

/// Sample the layer with the stroke so far composited over it under a dab's footprint (pixels
/// with tip coverage) into the pending color-mixing samples.
fn sample_under_dab(m: &mut MixState, cov: &CoverageMap, fmt: &PixelFormat, brush: [f32; 4], rect: Rect, quad: &dyn Fn(f32, f32) -> bool) {
    let nc = fmt.mode.color_channels();
    let mut native = [0.0f32; 8];
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            if !quad(x as f32 + 0.5, y as f32 + 0.5) {
                continue;
            }
            let layer = m.source.as_ref().map_or([0.0; 4], |s| s.rgba(x, y));
            let c = cov.get(x, y);
            let stroke = match cov.color_at(x, y) {
                Some(p) if c > 0.0 && p.len() == nc + 1 => {
                    for (o, v) in native.iter_mut().zip(&p[..nc]) {
                        *o = v / c;
                    }
                    if let Some(a) = alpha_index(fmt) {
                        native[a] = 1.0;
                    }
                    let mut rgba = to_rgba(fmt, &native[..fmt.channels()]);
                    rgba[3] = (p[nc] / c).clamp(0.0, 1.0);
                    rgba
                }
                _ => brush,
            };
            m.pending.add(layer, c, stroke);
        }
    }
}

/// Render a whole stroke onto `target` (one-shot). Returns the damaged rectangle.
pub fn render_stroke(
    target: &mut Surface,
    brush: &BrushSettings,
    points: &[StrokePoint],
    selection: Option<&Surface>,
    lock_transparency: bool,
    zoom: f32,
) -> Rect {
    if points.is_empty() {
        return Rect::EMPTY;
    }
    let pre = target.clone();
    let mut r = StrokeRenderer::new(brush, Some(target.format()), zoom).with_full_path(points).with_mixing_source(&pre);
    r.push(points);
    r.finish();
    r.composite(&pre, target, selection, lock_transparency, true)
}
