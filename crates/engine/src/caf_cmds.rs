//! Edit › Content-Aware Fill… (and the selection tools' Delete and Fill Selection).
//!
//! The command carries everything Photoshop's Content-Aware Fill workspace edits, so the
//! workspace, the CLI, the control channel and MCP all drive the same fill:
//!
//! * the **fill area** is the selection, changed by `fillOps` (the workspace's Lasso: add,
//!   subtract, intersect or replace with a polygon; Expand / Contract by pixels);
//! * the **sampling area** is `sampling` (`auto`, `rectangular` or `custom`), changed by
//!   `samplingStrokes` (the Sampling Brush: round strokes that add or subtract);
//! * **fill settings** (colour, rotation, scale, mirror adaptation) and the **output**
//!   (current layer, new layer, duplicate layer) as in the workspace's panel.
//!
//! [`prepare`] builds the masks and reads the pixels (the workspace's sampling overlay uses it
//! directly), [`compute`] runs [`prism_algo::caf::fill_with`], [`write_result`] writes into a
//! document. The command runs the last two as a background job.

use prism_algo::caf::{self, Bounds};
use prism_color::PixelFormat;
use prism_doc::{Document, Layer, LayerContent, LayerId};
use prism_geom::Rect;
use prism_raster::{Interrupt, Surface, from_rgba_into, to_rgba};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "edit.contentAwareFill";
/// Caps on hostile params: points per polygon or stroke, strokes and fill operations.
const MAX_POINTS: usize = 200_000;
const MAX_OPS: usize = 10_000;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Where the sampling area starts from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SamplingMode {
    Auto,
    Rectangular,
    Custom,
}

/// Where the result goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Current,
    New,
    Duplicate,
}

/// How a fill-area polygon combines with the area so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    New,
    Add,
    Subtract,
    Intersect,
}

/// One change to the fill area.
#[derive(Clone, Debug, PartialEq)]
pub enum FillOp {
    Polygon { mode: Combine, points: Vec<(f32, f32)> },
    /// Expand (positive) or contract (negative) by pixels.
    Grow(i32),
}

/// One Sampling Brush stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct SamplingStroke {
    pub add: bool,
    /// Brush diameter in pixels.
    pub size: f32,
    pub points: Vec<(f32, f32)>,
}

/// Parsed command params.
#[derive(Clone, Debug)]
pub struct Params {
    pub sampling: SamplingMode,
    /// Search-bounds factor for Auto / Rectangular.
    pub factor: f32,
    /// Rectangular: a fixed margin around the fill area instead of the factor.
    pub margin: Option<i32>,
    /// Custom: a rectangle and/or an alpha channel to sample from.
    pub area: Option<Rect>,
    pub channel: Option<Surface>,
    pub strokes: Vec<SamplingStroke>,
    pub fill_ops: Vec<FillOp>,
    pub sample_all: bool,
    pub output: Output,
    pub opts: caf::Options,
}

fn points(v: &Value, cmd: &str, what: &str) -> Result<Vec<(f32, f32)>> {
    let a = v.as_array().ok_or_else(|| bad(cmd, format!("{what} `points` must be [[x, y], …]")))?;
    if a.len() > MAX_POINTS {
        return Err(bad(cmd, format!("{what} has more than {MAX_POINTS} points")));
    }
    a.iter()
        .map(|q| {
            let x = q.get(0).and_then(Value::as_f64).filter(|f| f.is_finite());
            let y = q.get(1).and_then(Value::as_f64).filter(|f| f.is_finite());
            match (x, y) {
                (Some(x), Some(y)) => Ok((x.clamp(-1e9, 1e9) as f32, y.clamp(-1e9, 1e9) as f32)),
                _ => Err(bad(cmd, format!("{what} `points` must be [[x, y], …] of finite numbers"))),
            }
        })
        .collect()
}

/// `key` as an `[x, y, w, h]` rectangle (saturating: `[1e30, 0, 1e30, 10]` is a whole-canvas
/// window, not an overflow).
fn rect_param(p: &Value, key: &str, cmd: &str) -> Result<Rect> {
    let a = p.get(key).and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("`{key}` must be [x, y, w, h]")))?;
    let n = |i: usize| {
        a.get(i)
            .and_then(Value::as_f64)
            .filter(|f| f.is_finite())
            .map(|v| v.round() as i32)
            .ok_or_else(|| bad(cmd, format!("`{key}` must be four finite numbers")))
    };
    let (x, y, w, h) = (n(0)?, n(1)?, n(2)?, n(3)?);
    Ok(Rect::new(x, y, x.saturating_add(w.max(0)), y.saturating_add(h.max(0))))
}

/// Mask surface (gray) of an alpha channel by index or name.
fn channel_mask(doc: &Document, v: &Value) -> Option<Surface> {
    match v {
        Value::Number(n) => doc.channels.get(usize::try_from(n.as_u64()?).ok()?).map(|c| c.surface.clone()),
        Value::String(name) => doc.channels.iter().find(|c| c.name == *name).map(|c| c.surface.clone()),
        _ => None,
    }
}

impl Params {
    /// Parse and validate. `defaults_only` ignores every param (Delete and Fill Selection).
    pub fn parse(doc: &Document, p: &Value, cmd: &str) -> Result<Params> {
        let s = |k: &str, d: &'static str| -> Result<String> {
            match p.get(k) {
                None | Some(Value::Null) => Ok(d.to_string()),
                Some(Value::String(v)) => Ok(v.clone()),
                Some(_) => Err(bad(cmd, format!("`{k}` must be a string"))),
            }
        };
        let flag = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
        let sampling = match s("sampling", "auto")?.as_str() {
            "auto" => SamplingMode::Auto,
            "rectangular" => SamplingMode::Rectangular,
            "custom" => SamplingMode::Custom,
            o => return Err(bad(cmd, format!("unknown sampling `{o}` (auto|rectangular|custom)"))),
        };
        let factor = match p.get("factor") {
            None | Some(Value::Null) => if sampling == SamplingMode::Rectangular { caf::RECT_FACTOR } else { caf::AUTO_FACTOR },
            Some(v) => v.as_f64().filter(|f| (1.0..=10.0).contains(f)).ok_or_else(|| bad(cmd, "`factor` must be a number in 1..10"))? as f32,
        };
        let margin = match p.get("margin") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad(cmd, "`margin` must be a number of pixels"))?.clamp(0.0, 100_000.0) as i32),
        };
        let area = if p.get("area").is_some() { Some(rect_param(p, "area", cmd)?) } else { None };
        let channel = match p.get("channel") {
            None | Some(Value::Null) => None,
            Some(v) => Some(channel_mask(doc, v).ok_or_else(|| bad(cmd, "`channel` must name an alpha channel (index or name)"))?),
        };
        let mut strokes = Vec::new();
        if let Some(v) = p.get("samplingStrokes").filter(|v| !v.is_null()) {
            let a = v.as_array().ok_or_else(|| bad(cmd, "`samplingStrokes` must be a list"))?;
            if a.len() > MAX_OPS {
                return Err(bad(cmd, format!("more than {MAX_OPS} sampling strokes")));
            }
            for st in a {
                let add = match st.get("mode").and_then(Value::as_str).unwrap_or("add") {
                    "add" => true,
                    "subtract" => false,
                    o => return Err(bad(cmd, format!("sampling stroke mode `{o}` (add|subtract)"))),
                };
                let size = st.get("size").and_then(Value::as_f64).filter(|f| (1.0..=5000.0).contains(f)).ok_or_else(|| bad(cmd, "sampling stroke `size` must be 1..5000 px"))? as f32;
                let pts = points(st.get("points").unwrap_or(&Value::Null), cmd, "sampling stroke")?;
                if pts.is_empty() {
                    return Err(bad(cmd, "a sampling stroke needs at least one point"));
                }
                strokes.push(SamplingStroke { add, size, points: pts });
            }
        }
        let mut fill_ops = Vec::new();
        if let Some(v) = p.get("fillOps").filter(|v| !v.is_null()) {
            let a = v.as_array().ok_or_else(|| bad(cmd, "`fillOps` must be a list"))?;
            if a.len() > MAX_OPS {
                return Err(bad(cmd, format!("more than {MAX_OPS} fill-area operations")));
            }
            for op in a {
                let name = op.get("op").and_then(Value::as_str).ok_or_else(|| bad(cmd, "each fill-area operation needs `op`"))?;
                let px = || op.get("px").and_then(Value::as_f64).filter(|f| (0.0..=500.0).contains(f)).map(|f| f.round() as i32).ok_or_else(|| bad(cmd, "expand/contract `px` must be 0..500"));
                fill_ops.push(match name {
                    "expand" => FillOp::Grow(px()?),
                    "contract" => FillOp::Grow(-px()?),
                    "new" | "add" | "subtract" | "intersect" => {
                        let mode = match name {
                            "new" => Combine::New,
                            "add" => Combine::Add,
                            "subtract" => Combine::Subtract,
                            _ => Combine::Intersect,
                        };
                        let pts = points(op.get("points").unwrap_or(&Value::Null), cmd, "fill-area polygon")?;
                        if pts.len() < 3 {
                            return Err(bad(cmd, "a fill-area polygon needs at least 3 points"));
                        }
                        FillOp::Polygon { mode, points: pts }
                    }
                    o => return Err(bad(cmd, format!("unknown fill-area operation `{o}` (new|add|subtract|intersect|expand|contract)"))),
                });
            }
        }
        let color = s("colorAdaptation", "default")?;
        let color_limit = caf::color_limit(&color).ok_or_else(|| bad(cmd, "`colorAdaptation` must be none|default|high|veryHigh"))?;
        let membrane = caf::membrane_strength(&color).unwrap_or(0.0);
        let rot = s("rotationAdaptation", "none")?;
        if !matches!(rot.as_str(), "none" | "low" | "medium" | "high" | "full") {
            return Err(bad(cmd, "`rotationAdaptation` must be none|low|medium|high|full"));
        }
        let output = match s("output", "current")?.as_str() {
            "current" => Output::Current,
            "new" => Output::New,
            "duplicate" => Output::Duplicate,
            _ => return Err(bad(cmd, "`output` must be current|new|duplicate")),
        };
        let opts = caf::Options {
            color_limit,
            membrane,
            rotations: prism_algo::content_aware::rotation_level(&rot),
            scale: flag("scale"),
            mirror: flag("mirror"),
            seed: p.get("seed").and_then(Value::as_u64).unwrap_or(1),
            ..caf::Options::default()
        };
        // An empty custom area is valid (the workspace starts Custom empty); the fill then
        // reports that there is nothing to copy from.
        Ok(Params { sampling, factor, margin, area, channel, strokes, fill_ops, sample_all: flag("sampleAllLayers"), output, opts })
    }
}

fn to_bounds(r: Rect, origin: Rect) -> Bounds {
    let c = |v: i32, o: i32| (i64::from(v) - i64::from(o)).max(0) as usize;
    Bounds { x0: c(r.x0, origin.x0), y0: c(r.y0, origin.y0), x1: c(r.x1, origin.x0), y1: c(r.y1, origin.y0) }
}

fn from_bounds(b: Bounds, origin: Rect) -> Rect {
    let c = |v: usize, o: i32| i32::try_from(i64::from(o).saturating_add(v as i64)).unwrap_or(i32::MAX);
    Rect::new(c(b.x0, origin.x0), c(b.y0, origin.y0), c(b.x1, origin.x0), c(b.y1, origin.y0))
}

fn points_rect(pts: &[(f32, f32)], pad: f32) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in pts {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    let c = |v: f32| v.clamp(-1e9, 1e9) as i32;
    Rect::new(c((x0 - pad).floor()), c((y0 - pad).floor()), c((x1 + pad).ceil()), c((y1 + pad).ceil()))
}

/// The cheap part: the fill area's and sampling area's extents and the window to read, checked
/// against the size cap before anything is allocated.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    /// Where the fill area can be.
    pub fill: Rect,
    /// Auto / Rectangular base rectangle (empty for Custom).
    pub base: Rect,
    /// Everything that is read and masked.
    pub window: Rect,
}

pub fn geometry(doc: &Document, p: &Params, cmd: &str) -> Result<Geometry> {
    let canvas = doc.bounds();
    let sel = doc.selection.as_ref().map(|s| s.content_bounds()).unwrap_or(Rect::EMPTY);
    // The fill area can reach as far as any added polygon, plus what Expand adds.
    let mut fill = sel;
    let mut grow = 0i32;
    for op in &p.fill_ops {
        match op {
            FillOp::Polygon { mode: Combine::Add | Combine::New, points } => fill = fill.union(&points_rect(points, 1.0)),
            FillOp::Polygon { .. } => {}
            FillOp::Grow(px) => grow = grow.saturating_add((*px).max(0)),
        }
    }
    let fill = fill.inflate(grow).intersect(&canvas);
    if fill.is_empty() {
        return Err(bad(cmd, "the fill area is empty or outside the canvas"));
    }
    let base = match (p.sampling, p.margin) {
        (SamplingMode::Custom, _) => Rect::EMPTY,
        (SamplingMode::Rectangular, Some(m)) => fill.inflate(m).intersect(&canvas),
        _ => from_bounds(caf::search_bounds(canvas.width() as usize, canvas.height() as usize, to_bounds(fill, canvas), p.factor), canvas),
    };
    let mut window = fill.union(&base);
    if p.sampling == SamplingMode::Custom {
        if let Some(a) = p.area {
            window = window.union(&a.intersect(&canvas));
        }
        if let Some(c) = &p.channel {
            window = window.union(&c.content_bounds().intersect(&canvas));
        }
    }
    for s in p.strokes.iter().filter(|s| s.add) {
        window = window.union(&points_rect(&s.points, s.size / 2.0 + 1.0));
    }
    let window = window.intersect(&canvas);
    crate::fill_cmds::window_size(window)?;
    Ok(Geometry { fill, base, window })
}

/// Masks and pixels over the window.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub layer: LayerId,
    pub fmt: PixelFormat,
    pub window: Rect,
    /// Fill-area coverage (0..1) per window pixel.
    pub cover: Vec<f32>,
    /// Sampling area per window pixel (never inside the fill area).
    pub source: Vec<bool>,
    /// Pixels the fill reads (the layer, or the composite with Sample All Layers), in `fmt`.
    pub img: Vec<f32>,
}

/// Pixels of `rect` from the composite of all visible layers, in `fmt`.
fn composite(doc: &Document, rect: Rect, fmt: PixelFormat) -> Vec<f32> {
    let buf = prism_compose::render(doc, rect);
    let n = fmt.channels();
    let mut out = vec![0.0f32; buf.px.len() * n];
    let mut enc = [0.0f32; 8];
    for (o, px) in out.chunks_exact_mut(n.max(1)).zip(&buf.px) {
        from_rgba_into(&fmt, *px, &mut enc);
        o.copy_from_slice(&enc[..n]);
    }
    out
}

/// Distance from `(px, py)` to the segment `a–b`, squared.
fn seg_dist2(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (a.0 + t * dx - px, a.1 + t * dy - py);
    qx * qx + qy * qy
}

/// Paint a round stroke into `m` (window-sized) with `value`.
fn paint_stroke(m: &mut [bool], window: Rect, s: &SamplingStroke, value: bool) {
    let (w, r) = (window.width() as usize, s.size / 2.0);
    let segs: Vec<((f32, f32), (f32, f32))> = if s.points.len() == 1 { vec![(s.points[0], s.points[0])] } else { s.points.windows(2).map(|p| (p[0], p[1])).collect() };
    for (a, b) in segs {
        let bb = points_rect(&[a, b], r + 1.0).intersect(&window);
        for y in bb.y0..bb.y1 {
            for x in bb.x0..bb.x1 {
                if seg_dist2(x as f32 + 0.5, y as f32 + 0.5, a, b) <= r * r {
                    let i = (y - window.y0) as usize * w + (x - window.x0) as usize;
                    if let Some(v) = m.get_mut(i) {
                        *v = value;
                    }
                }
            }
        }
    }
}

/// Build the masks and read the pixels (the expensive part; `ctl` is checked per stage).
pub fn prepare(doc: &Document, layer: LayerId, p: &Params, g: &Geometry, ctl: &Interrupt) -> Result<Prepared> {
    let cancelled = || EngineError::Cancelled;
    let surf = doc.layer(layer).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(layer))?;
    let fmt = surf.format();
    let n = fmt.channels();
    let window = g.window;
    let (w, h) = crate::fill_cmds::window_size(window)?;
    // Fill area: the selection, then the Lasso's operations in order.
    let mut cover = vec![0.0f32; w * h];
    if let Some(sel) = &doc.selection {
        for y in 0..h {
            if y % 64 == 0 {
                ctl.check().map_err(|_| cancelled())?;
            }
            for x in 0..w {
                cover[y * w + x] = sel.sample_channel(window.x0 + x as i32, window.y0 + y as i32, 0);
            }
        }
    }
    for op in &p.fill_ops {
        ctl.check().map_err(|_| cancelled())?;
        match op {
            FillOp::Polygon { mode, points } => {
                let c = prism_algo::selection::polygon(points, window, true);
                for (a, b) in cover.iter_mut().zip(c) {
                    *a = match mode {
                        Combine::New => b,
                        Combine::Add => a.max(b),
                        Combine::Subtract => *a * (1.0 - b),
                        Combine::Intersect => a.min(b),
                    };
                }
            }
            FillOp::Grow(px) => {
                let on: Vec<bool> = cover.iter().map(|c| *c > 0.0).collect();
                let r = px.unsigned_abs() as usize;
                let grown = if *px >= 0 { caf::dilate(w, h, &on, r) } else { caf::dilate(w, h, &on.iter().map(|v| !v).collect::<Vec<_>>(), r).iter().map(|v| !v).collect() };
                for (c, g) in cover.iter_mut().zip(grown) {
                    *c = if g { c.max(if *px >= 0 { 1.0 } else { *c }) } else { 0.0 };
                }
            }
        }
    }
    ctl.check().map_err(|_| cancelled())?;
    let img = if p.sample_all { composite(doc, window, fmt) } else { surf.read_region(window) };
    if img.len() != w * h * n {
        return Err(EngineError::Other("could not read the layer's pixels".into()));
    }
    ctl.check().map_err(|_| cancelled())?;
    let hole: Vec<bool> = cover.iter().map(|c| *c > 0.0).collect();
    // Sampling area: the mode's base, then the Sampling Brush strokes.
    let mut source = vec![false; w * h];
    match p.sampling {
        SamplingMode::Custom => {
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = (window.x0 + x as i32, window.y0 + y as i32);
                    let in_area = p.area.is_some_and(|a| a.contains(dx, dy));
                    let in_chan = p.channel.as_ref().is_some_and(|c| c.sample_channel(dx, dy, 0) > 0.5);
                    source[y * w + x] = in_area || in_chan;
                }
            }
        }
        mode => {
            let b = to_bounds(g.base, window);
            for y in b.y0..b.y1.min(h) {
                for x in b.x0..b.x1.min(w) {
                    source[y * w + x] = true;
                }
            }
            if mode == SamplingMode::Auto {
                caf::exclude_unlike_with(w, h, n, &img, &hole, &mut source, ctl).map_err(|_| cancelled())?;
            }
        }
    }
    for s in &p.strokes {
        paint_stroke(&mut source, window, s, s.add);
    }
    for (s, h) in source.iter_mut().zip(&hole) {
        *s &= !h;
    }
    Ok(Prepared { layer, fmt, window, cover, source, img })
}

/// Run the fill. `None` when nothing in the sampling area can be copied from.
pub fn compute(p: &Params, prep: &Prepared, ctl: &Interrupt) -> std::result::Result<Option<Vec<f32>>, prism_raster::Cancelled> {
    let (w, h) = (prep.window.width() as usize, prep.window.height() as usize);
    let hole: Vec<bool> = prep.cover.iter().map(|c| *c > 0.0).collect();
    let alpha = if prep.fmt.alpha { Some(prep.fmt.channels().saturating_sub(1)) } else { None };
    let opts = caf::Options { alpha, ..p.opts.clone() };
    caf::fill_with(w, h, prep.fmt.channels(), &prep.img, &hole, &prep.source, &opts, ctl)
}

/// Write a result into `doc` (the layer itself, a new layer holding only the fill, or a
/// duplicate). Returns the layer written.
pub fn write_result(doc: &mut Document, active: &mut Option<LayerId>, output: Output, prep: &Prepared, filled: &[f32]) -> Result<LayerId> {
    let (fmt, window) = (prep.fmt, prep.window);
    let n = fmt.channels();
    let (w, h) = (window.width() as usize, window.height() as usize);
    if filled.len() != w * h * n || prep.img.len() != filled.len() || prep.cover.len() != w * h {
        return Err(EngineError::Other("the fill result does not match its window".into()));
    }
    let id = prep.layer;
    let target = match output {
        Output::New => {
            let mut l = Layer::raster(doc.next_layer_name("Content-Aware Fill"), PixelFormat { alpha: true, ..fmt });
            let lf = l.surface().map(|s| s.format()).unwrap_or(fmt);
            let ln = lf.channels();
            let mut px = vec![0.0f32; w * h * ln];
            let mut enc = [0.0f32; 8];
            for (i, k) in prep.cover.iter().enumerate() {
                if *k > 0.0 {
                    let rgba = to_rgba(&fmt, &filled[i * n..(i + 1) * n]);
                    from_rgba_into(&lf, [rgba[0], rgba[1], rgba[2], rgba[3] * k], &mut enc);
                    px[i * ln..(i + 1) * ln].copy_from_slice(&enc[..ln]);
                }
            }
            if let Some(s) = l.surface_mut() {
                s.write_region(window, &px);
                s.prune();
            }
            let nid = doc.insert_above(Some(id), l);
            *active = Some(nid);
            return Ok(nid);
        }
        Output::Duplicate => {
            let mut dup = doc.layer(id).ok_or(EngineError::NoLayer(id))?.duplicate();
            dup.name = doc.copy_name(&dup.name);
            let nid = doc.insert_above(Some(id), dup);
            *active = Some(nid);
            nid
        }
        Output::Current => id,
    };
    let locks = doc.effective_locks(target);
    let l = doc.layer_mut(target).ok_or(EngineError::NoLayer(target))?;
    if locks.all || locks.pixels {
        return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
    }
    let surf = l.surface_mut().ok_or_else(|| EngineError::Other("not a pixel layer".into()))?;
    // With Sample All Layers the fill came from the composite; the rest of the layer stays.
    let mut out = surf.read_region(window);
    if out.len() != filled.len() {
        return Err(EngineError::Other("could not read the layer's pixels".into()));
    }
    for (i, &k) in prep.cover.iter().enumerate() {
        if k > 0.0 {
            for c in 0..n {
                let j = i * n + c;
                out[j] += (filled[j] - out[j]) * k;
            }
        }
    }
    surf.write_region(window, &out);
    Ok(target)
}

fn pixel_layer(s: &Session) -> std::result::Result<LayerId, String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    let l = d.doc.layer(id).ok_or("no active layer")?;
    if !matches!(l.content, LayerContent::Raster(_)) {
        return Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()));
    }
    let locks = d.doc.effective_locks(id);
    if locks.all || locks.pixels {
        return Err(format!("the layer \"{}\" is locked", l.name));
    }
    Ok(id)
}

fn can_caf(s: &Session) -> std::result::Result<(), String> {
    pixel_layer(s)?;
    let d = s.active().ok_or("no document open")?;
    if d.doc.selection.as_ref().is_none_or(|m| m.content_bounds().is_empty()) {
        return Err("make a selection around what to remove".into());
    }
    Ok(())
}

fn content_aware_fill(s: &mut Session, p: &Value, cmd: &'static str, label: &'static str) -> Result<Value> {
    let id = pixel_layer(s).map_err(EngineError::Other)?;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    if doc.selection.is_none() && !p.get("fillOps").is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
        return Err(bad(cmd, "no selection"));
    }
    let params = Params::parse(&doc, p, cmd)?;
    // Cheap checks first: a window too large to read is refused before allocating (#963).
    let g = geometry(&doc, &params, cmd)?;
    let output = params.output;
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            ctx.progress(0.0, label);
            let prep = ctx.stage(0.0, 0.1, label, |ctl| prepare(&doc, id, &params, &g, ctl))?;
            if !prep.cover.iter().any(|c| *c > 0.0) {
                return Err(bad(cmd, "the fill area is empty"));
            }
            ctx.check()?;
            let filled = ctx.stage(0.1, 1.0, label, |ctl| compute(&params, &prep, ctl)).map_err(|_| EngineError::Cancelled)?;
            let filled = filled.ok_or_else(|| bad(cmd, "the sampling area has nothing to copy from: add to it with the Sampling Brush"))?;
            Ok((prep, filled))
        },
        move |s, (prep, filled)| {
            let n_hole = prep.cover.iter().filter(|c| **c > 0.0).count();
            let window = prep.window;
            let layer = s.edit(label, |doc, active| write_result(doc, active, output, &prep, &filled))?;
            Ok(json!({"layer": layer.0, "filled": n_hole, "window": [window.x0, window.y0, window.width(), window.height()]}))
        },
    )
}

/// Content-Aware Fill command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "edit.contentAwareFill",
            label: "Content-Aware Fill…",
            menu: &["Edit"],
            shortcut: None,
            params: r##"{"sampling":"auto|rectangular|custom"="auto","factor":1..10=3 (auto/rectangular search bounds: area factor² × fill area),"margin":px? (rectangular: fixed margin instead),"area":[x,y,w,h]?,"channel":index|name? (custom),"samplingStrokes":[{"mode":"add|subtract","size":px,"points":[[x,y],…]}]? (Sampling Brush),"fillOps":[{"op":"new|add|subtract|intersect","points":[[x,y],…]}|{"op":"expand|contract","px":0..500}]? (Lasso on the fill area),"sampleAllLayers":bool=false,"colorAdaptation":"none|default|high|veryHigh"="default","rotationAdaptation":"none|low|medium|high|full"="none","scale":bool=false,"mirror":bool=false,"output":"current|new|duplicate"="current","seed":u64=1}"##,
            enabled: can_caf,
            run: |s, p| content_aware_fill(s, p, CMD, "Content-Aware Fill"),
            journal: true,
        },
        CommandSpec {
            id: "edit.deleteAndFillSelection",
            label: "Delete and Fill Selection",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: can_caf,
            // Photoshop's one-click removal from the selection tools' context menu (#1286): the
            // fill's defaults into the layer, no workspace. Params are ignored.
            run: |s, _| content_aware_fill(s, &json!({}), "edit.deleteAndFillSelection", "Delete and Fill Selection"),
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;
