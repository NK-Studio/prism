//! Edit › Content-Aware Fill…: Photoshop's Content-Aware Fill workspace, a full-window mode.
//!
//! Layout as Photoshop 2026: tool options along the top, a tool strip on the left (Sampling
//! Brush, Lasso, Hand, Zoom), the document with the sampling-area overlay in the middle, the
//! Preview panel, and the Content-Aware Fill panel (Sampling Area Overlay, Sampling Area
//! Options, Fill Settings, Output Settings) with Reset, Cancel, Apply and OK at its foot.
//!
//! Everything the workspace edits is plain command data: the Sampling Brush records
//! `samplingStrokes`, the Lasso and Expand / Contract record `fillOps`, and the panel holds the
//! remaining `edit.contentAwareFill` params. A worker thread builds the masks (for the overlay
//! and the fill area's marching ants) and the fill itself (for the Preview) from that data with
//! the engine's own [`prism_engine::caf_cmds`] code, so OK and Apply produce exactly what the
//! Preview showed. The control channel drives it as `edit.contentAwareFill {"ui": {…}}`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use egui::{Align2, Color32, ColorImage, FontId, Pos2, Rect as ERect, Sense, Stroke, TextureHandle, pos2, vec2};
use prism_doc::{Document, LayerId};
use prism_engine::caf_cmds::{self, Combine, FillOp, Output, Prepared, SamplingStroke};
use prism_geom::Rect;
use prism_raster::{Interrupt, Surface};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PrismApp;
use crate::theme::Tokens;
use crate::widgets;

const CMD: &str = "edit.contentAwareFill";
/// Settings kept between sessions (prefs `dialogs`).
const REMEMBERED: &str = "edit.contentAwareFill";
/// Longest side of the canvas and preview textures.
const PROXY_SIDE: usize = 2048;
/// Above this many window pixels the Preview shows only the proxy (no full-resolution crop).
const CROP_MAX_PX: u64 = 6_000_000;
const OPTIONS_H: f32 = 38.0;
const TOOLS_W: f32 = 36.0;
const PREVIEW_W: f32 = 340.0;
const PANEL_W: f32 = 272.0;
/// Expand / Contract step choices (px).
const STEPS: [u32; 7] = [1, 2, 3, 5, 10, 20, 50];

/// The workspace's tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CafTool {
    SamplingBrush,
    Lasso,
    Hand,
    Zoom,
}

impl CafTool {
    const ALL: [CafTool; 4] = [CafTool::SamplingBrush, CafTool::Lasso, CafTool::Hand, CafTool::Zoom];
    fn icon(self) -> &'static str {
        match self {
            CafTool::SamplingBrush => "tool-brush",
            CafTool::Lasso => "tool-lasso",
            CafTool::Hand => "tool-hand",
            CafTool::Zoom => "tool-zoom",
        }
    }
    fn tip(self) -> &'static str {
        match self {
            CafTool::SamplingBrush => "Sampling Brush Tool (B)",
            CafTool::Lasso => "Lasso Tool (L)",
            CafTool::Hand => "Hand Tool (H)",
            CafTool::Zoom => "Zoom Tool (Z)",
        }
    }
}

/// The panel's settings (remembered between sessions, settable over the control channel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CafSettings {
    pub show_overlay: bool,
    /// Overlay opacity, percent.
    pub opacity: f32,
    pub color: [u8; 3],
    /// What the overlay tints: `sampling` (the sampling area) or `excluded` (the rest).
    pub show: String,
    /// The sampling area's starting point: `auto`, `rectangular` or `custom`.
    pub sampling: String,
    pub sample_all_layers: bool,
    pub color_adaptation: String,
    pub rotation_adaptation: String,
    pub scale: bool,
    pub mirror: bool,
    pub output: String,
    pub brush_size: f32,
    /// Expand / Contract step, px.
    pub step: u32,
}

impl Default for CafSettings {
    fn default() -> Self {
        Self {
            show_overlay: true,
            opacity: 60.0,
            color: [126, 194, 66],
            show: "sampling".into(),
            sampling: "auto".into(),
            sample_all_layers: false,
            color_adaptation: "default".into(),
            rotation_adaptation: "none".into(),
            scale: false,
            mirror: false,
            output: "new".into(),
            brush_size: 56.0,
            step: 3,
        }
    }
}

fn one_of(v: &Value, k: &str, allowed: &[&str]) -> Result<Option<String>, String> {
    match v.get(k) {
        None => Ok(None),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{k}` must be one of {}", allowed.join("|"))),
    }
}

impl CafSettings {
    /// Apply the keys present in `v` (validated; nothing changes on an error).
    pub fn apply(&mut self, v: &Value) -> Result<(), String> {
        let mut n = self.clone();
        let flag = |k: &str| -> Result<Option<bool>, String> {
            match v.get(k) {
                None => Ok(None),
                Some(Value::Bool(b)) => Ok(Some(*b)),
                Some(_) => Err(format!("`{k}` must be true or false")),
            }
        };
        let num = |k: &str, lo: f64, hi: f64| -> Result<Option<f32>, String> {
            match v.get(k) {
                None => Ok(None),
                Some(x) => x.as_f64().filter(|f| (lo..=hi).contains(f)).map(|f| Some(f as f32)).ok_or_else(|| format!("`{k}` must be a number in {lo}..{hi}")),
            }
        };
        if let Some(b) = flag("showOverlay")? {
            n.show_overlay = b;
        }
        if let Some(f) = num("opacity", 0.0, 100.0)? {
            n.opacity = f;
        }
        if let Some(c) = v.get("color") {
            let a = c.as_array().filter(|a| a.len() == 3).ok_or("`color` must be [r, g, b] (0..255)")?;
            let mut rgb = [0u8; 3];
            for (o, x) in rgb.iter_mut().zip(a) {
                *o = x.as_u64().filter(|x| *x <= 255).ok_or("`color` must be [r, g, b] (0..255)")? as u8;
            }
            n.color = rgb;
        }
        if let Some(s) = one_of(v, "show", &["sampling", "excluded"])? {
            n.show = s;
        }
        if let Some(s) = one_of(v, "sampling", &["auto", "rectangular", "custom"])? {
            n.sampling = s;
        }
        if let Some(b) = flag("sampleAllLayers")? {
            n.sample_all_layers = b;
        }
        if let Some(s) = one_of(v, "colorAdaptation", &["none", "default", "high", "veryHigh"])? {
            n.color_adaptation = s;
        }
        if let Some(s) = one_of(v, "rotationAdaptation", &["none", "low", "medium", "high", "full"])? {
            n.rotation_adaptation = s;
        }
        if let Some(b) = flag("scale")? {
            n.scale = b;
        }
        if let Some(b) = flag("mirror")? {
            n.mirror = b;
        }
        if let Some(s) = one_of(v, "output", &["current", "new", "duplicate"])? {
            n.output = s;
        }
        if let Some(f) = num("brushSize", 1.0, 5000.0)? {
            n.brush_size = f;
        }
        if let Some(f) = num("step", 1.0, 500.0)? {
            n.step = f.round() as u32;
        }
        *self = n;
        Ok(())
    }

    fn sanitized(mut self) -> Self {
        let d = Self::default();
        let ok = |s: &str, allowed: &[&str]| allowed.contains(&s);
        if !ok(&self.show, &["sampling", "excluded"]) {
            self.show = d.show.clone();
        }
        if !ok(&self.sampling, &["auto", "rectangular", "custom"]) {
            self.sampling = d.sampling.clone();
        }
        if !ok(&self.color_adaptation, &["none", "default", "high", "veryHigh"]) {
            self.color_adaptation = d.color_adaptation.clone();
        }
        if !ok(&self.rotation_adaptation, &["none", "low", "medium", "high", "full"]) {
            self.rotation_adaptation = d.rotation_adaptation.clone();
        }
        if !ok(&self.output, &["current", "new", "duplicate"]) {
            self.output = d.output.clone();
        }
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(0.0, 100.0) } else { d.opacity };
        self.brush_size = if self.brush_size.is_finite() { self.brush_size.clamp(1.0, 5000.0) } else { d.brush_size };
        self.step = self.step.clamp(1, 500);
        self
    }
}

/// What the overlay and the Preview were built from.
struct Masks {
    window: Rect,
    source: Vec<bool>,
    cover: Vec<f32>,
}

enum JobMsg {
    Masks(Masks, Vec<crate::outline::Segment>),
    /// The filled document's proxy image, and a full-resolution crop of the window.
    Done(ColorImage, Option<(Rect, ColorImage)>, f64),
    /// Nothing to sample from (an empty custom area).
    Empty,
    Failed(String),
}

struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<JobMsg>,
}

/// One undo step: the sampling and fill data before a change.
#[derive(Clone)]
struct Snapshot {
    sampling: String,
    strokes: Vec<SamplingStroke>,
    fill_ops: Vec<FillOp>,
}

/// The open workspace.
pub struct CafWorkspace {
    pub layer: LayerId,
    title: String,
    canvas: Rect,
    /// Canvas px per proxy px.
    k: u32,
    pub settings: CafSettings,
    pub tool: CafTool,
    /// Sampling Brush mode: add (true) or subtract.
    pub brush_add: bool,
    /// Lasso mode.
    pub lasso_mode: Combine,
    pub strokes: Vec<SamplingStroke>,
    pub fill_ops: Vec<FillOp>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The Sampling Brush stroke or Lasso polygon being drawn (document px).
    cur_stroke: Option<SamplingStroke>,
    cur_lasso: Option<(Combine, Vec<(f32, f32)>)>,
    base_tex: Option<TextureHandle>,
    overlay_tex: Option<TextureHandle>,
    preview_tex: Option<TextureHandle>,
    crop_tex: Option<(Rect, TextureHandle)>,
    masks: Option<Masks>,
    ants: Vec<crate::outline::Segment>,
    job: Option<Job>,
    generation: u64,
    /// The Preview shows the last result; `false` while a new one is computed.
    pub preview_ready: bool,
    pub preview_empty: bool,
    pub preview_ms: f64,
    pub error: Option<String>,
    overlay_dirty: bool,
    zoom: f32,
    center: [f64; 2],
    preview_zoom: f32,
    preview_center: [f64; 2],
    hover: Option<[f64; 2]>,
}

/// Proxy factor for the canvas textures.
fn proxy_k(canvas: Rect) -> u32 {
    (canvas.width().max(canvas.height()) as usize).div_ceil(PROXY_SIDE).max(1) as u32
}

/// RGBA image of the composite of `doc` over `r`, downsampled to at most `max_side`.
fn render_image(doc: &Document, r: Rect, max_side: usize) -> ColorImage {
    let k = (r.width().max(r.height()) as usize).div_ceil(max_side).max(1) as u32;
    let (doc, r) = if k > 1 {
        let p = crate::proxy::proxy_document(doc, k);
        let pr = Rect::new(r.x0.div_euclid(k as i32), r.y0.div_euclid(k as i32), (r.x1 + k as i32 - 1).div_euclid(k as i32), (r.y1 + k as i32 - 1).div_euclid(k as i32));
        (std::borrow::Cow::Owned(p), pr)
    } else {
        (std::borrow::Cow::Borrowed(doc), r)
    };
    let buf = prism_compose::render(&doc, r);
    let (w, h) = (r.width().max(1) as usize, r.height().max(1) as usize);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let px: Vec<Color32> = buf.px.iter().map(|p| Color32::from_rgba_unmultiplied(byte(p[0]), byte(p[1]), byte(p[2]), byte(p[3]))).collect();
    if px.len() != w * h {
        return ColorImage::new([1, 1], vec![Color32::TRANSPARENT]);
    }
    ColorImage::new([w, h], px)
}

fn combine_name(c: Combine) -> &'static str {
    match c {
        Combine::New => "new",
        Combine::Add => "add",
        Combine::Subtract => "subtract",
        Combine::Intersect => "intersect",
    }
}

fn stroke_json(s: &SamplingStroke) -> Value {
    json!({"mode": if s.add { "add" } else { "subtract" }, "size": s.size, "points": s.points.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>()})
}

fn op_json(op: &FillOp) -> Value {
    match op {
        FillOp::Polygon { mode, points } => json!({"op": combine_name(*mode), "points": points.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>()}),
        FillOp::Grow(px) if *px >= 0 => json!({"op": "expand", "px": px}),
        FillOp::Grow(px) => json!({"op": "contract", "px": -px}),
    }
}

fn points_of(v: &Value) -> Result<Vec<(f32, f32)>, String> {
    let a = v.as_array().ok_or("`points` must be [[x, y], …]")?;
    a.iter()
        .map(|p| {
            let x = p.get(0).and_then(Value::as_f64).filter(|f| f.is_finite());
            let y = p.get(1).and_then(Value::as_f64).filter(|f| f.is_finite());
            x.zip(y).map(|(x, y)| (x as f32, y as f32)).ok_or_else(|| "`points` must be [[x, y], …] of numbers".to_string())
        })
        .collect()
}

impl CafWorkspace {
    /// The command params the workspace stands for (what OK runs).
    pub fn params(&self) -> Value {
        json!({
            "sampling": self.settings.sampling,
            "samplingStrokes": self.strokes.iter().map(stroke_json).collect::<Vec<_>>(),
            "fillOps": self.fill_ops.iter().map(op_json).collect::<Vec<_>>(),
            "sampleAllLayers": self.settings.sample_all_layers,
            "colorAdaptation": self.settings.color_adaptation,
            "rotationAdaptation": self.settings.rotation_adaptation,
            "scale": self.settings.scale,
            "mirror": self.settings.mirror,
            "output": self.settings.output,
        })
    }

    /// The Sampling Area Options button that shows as chosen: brushing turns the area custom.
    fn sampling_shown(&self) -> &str {
        if self.strokes.is_empty() { &self.settings.sampling } else { "custom" }
    }

    pub fn describe(&self) -> Value {
        let (sampled, filled) = self.masks.as_ref().map_or((0, 0), |m| (m.source.iter().filter(|s| **s).count(), m.cover.iter().filter(|c| **c > 0.0).count()));
        json!({
            "layer": self.layer.0,
            "settings": self.settings,
            "samplingShown": self.sampling_shown(),
            "tool": self.tool,
            "brushMode": if self.brush_add { "add" } else { "subtract" },
            "lassoMode": combine_name(self.lasso_mode),
            "strokes": self.strokes.len(),
            "fillOps": self.fill_ops.len(),
            "undo": self.undo.len(),
            "redo": self.redo.len(),
            "busy": self.job.is_some(),
            "previewReady": self.preview_ready,
            "previewEmpty": self.preview_empty,
            "previewMs": self.preview_ms,
            "samplingPixels": sampled,
            "fillPixels": filled,
            "zoom": self.zoom,
            "previewZoom": self.preview_zoom,
            "error": self.error,
            "params": self.params(),
        })
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { sampling: self.settings.sampling.clone(), strokes: self.strokes.clone(), fill_ops: self.fill_ops.clone() }
    }

    /// Record the current data for undo before a change.
    fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn restore(&mut self, s: Snapshot) {
        self.settings.sampling = s.sampling;
        self.strokes = s.strokes;
        self.fill_ops = s.fill_ops;
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.redo.push(self.snapshot());
        self.restore(s);
        self.overlay_dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        self.undo.push(self.snapshot());
        self.restore(s);
        self.overlay_dirty = true;
        true
    }

    /// Choose Auto / Rectangular / Custom: the area starts over from that mode (brush strokes
    /// are dropped), and the Sampling Brush switches to subtract (or add, for an empty custom
    /// area), as in Photoshop.
    pub fn set_sampling(&mut self, mode: &str) {
        if self.settings.sampling == mode && self.strokes.is_empty() {
            return;
        }
        self.push_undo();
        self.settings.sampling = mode.to_string();
        self.strokes.clear();
        self.brush_add = mode == "custom";
        self.overlay_dirty = true;
    }

    pub fn add_stroke(&mut self, s: SamplingStroke) {
        if s.points.is_empty() {
            return;
        }
        self.push_undo();
        self.strokes.push(s);
        self.overlay_dirty = true;
    }

    pub fn add_fill_op(&mut self, op: FillOp) {
        if matches!(&op, FillOp::Polygon { points, .. } if points.len() < 3) {
            return;
        }
        self.push_undo();
        self.fill_ops.push(op);
        self.overlay_dirty = true;
    }

    /// Reset (footer): every panel setting to its default and the sampling / fill data cleared.
    pub fn reset_all(&mut self) {
        self.push_undo();
        let brush = self.settings.brush_size;
        self.settings = CafSettings { brush_size: brush, ..CafSettings::default() };
        self.strokes.clear();
        self.fill_ops.clear();
        self.brush_add = false;
        self.overlay_dirty = true;
    }

    /// Start (or restart) the worker for the current data.
    fn restart(&mut self, doc: Arc<Document>) {
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.generation += 1;
        self.overlay_dirty = false;
        self.preview_ready = false;
        self.preview_empty = false;
        self.error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let txf = tx.clone();
        let (layer, params, canvas, k, flag) = (self.layer, self.params(), self.canvas, self.k, cancel.clone());
        let work = move || {
            let t0 = crate::gpu_canvas::now_ms();
            let stop = || flag.load(Ordering::Relaxed);
            let ignore = |_: f32| {};
            let ctl = Interrupt::new(&stop, &ignore);
            let p = caf_cmds::Params::parse(&doc, &params, CMD).map_err(|e| e.to_string())?;
            let g = caf_cmds::geometry(&doc, &p, CMD).map_err(|e| e.to_string())?;
            let prep: Prepared = caf_cmds::prepare(&doc, layer, &p, &g, &ctl).map_err(|e| e.to_string())?;
            let mut mask = Surface::new(prism_color::PixelFormat::GRAY8);
            let on: Vec<f32> = prep.cover.iter().map(|c| if *c > 0.0 { 1.0 } else { 0.0 }).collect();
            mask.write_region(prep.window, &on);
            let ants = crate::outline::outline_scaled_serial(&mask, prep.window, k);
            let masks = Masks { window: prep.window, source: prep.source.clone(), cover: prep.cover.clone() };
            if tx.send(JobMsg::Masks(masks, ants)).is_err() || stop() {
                return Ok(());
            }
            let Some(filled) = caf_cmds::compute(&p, &prep, &ctl).map_err(|_| "cancelled".to_string())? else {
                let _ = tx.send(JobMsg::Empty);
                return Ok(());
            };
            let mut d = (*doc).clone();
            let mut active = Some(layer);
            caf_cmds::write_result(&mut d, &mut active, Output::Current, &prep, &filled).map_err(|e| e.to_string())?;
            if stop() {
                return Ok(());
            }
            let full = render_image(&d, canvas, PROXY_SIDE);
            let w = prep.window;
            let crop = (k > 1 && u64::from(w.width()) * u64::from(w.height()) <= CROP_MAX_PX).then(|| (w, render_image(&d, w, 4096)));
            let _ = tx.send(JobMsg::Done(full, crop, crate::gpu_canvas::now_ms() - t0));
            Ok::<(), String>(())
        };
        let spawned = std::thread::Builder::new().name("caf-preview".into()).spawn(move || {
            // A failure or a panic in the worker is reported to the workspace, never propagated.
            let msg = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
                Ok(Ok(())) => return,
                Ok(Err(e)) => e,
                Err(_) => "the preview failed unexpectedly".to_string(),
            };
            let _ = txf.send(JobMsg::Failed(msg));
        });
        match spawned {
            Ok(_) => self.job = Some(Job { cancel, rx }),
            Err(e) => self.error = Some(format!("could not start the preview: {e}")),
        }
    }

    /// Take the worker's results (each frame). True when something changed.
    fn poll(&mut self, ctx: &egui::Context) -> bool {
        let Some(job) = &self.job else { return false };
        let mut changed = false;
        let mut finished = false;
        loop {
            match job.rx.try_recv() {
                Ok(JobMsg::Masks(m, ants)) => {
                    self.masks = Some(m);
                    self.ants = ants;
                    self.overlay_tex = None;
                    changed = true;
                }
                Ok(JobMsg::Done(full, crop, ms)) => {
                    self.preview_tex = Some(ctx.load_texture("caf-preview", full, egui::TextureOptions::LINEAR));
                    self.crop_tex = crop.map(|(r, img)| (r, ctx.load_texture("caf-preview-crop", img, egui::TextureOptions::LINEAR)));
                    self.preview_ready = true;
                    self.preview_ms = ms;
                    finished = true;
                }
                Ok(JobMsg::Empty) => {
                    self.preview_tex = None;
                    self.crop_tex = None;
                    self.preview_ready = true;
                    self.preview_empty = true;
                    finished = true;
                }
                Ok(JobMsg::Failed(e)) => {
                    self.error = Some(e);
                    self.preview_ready = true;
                    finished = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            self.job = None;
            changed = true;
        }
        changed
    }

    /// Block until the worker is done (control channel `wait`, tests). At most `ms`.
    pub fn wait(&mut self, ctx: &egui::Context, ms: f64) {
        let t0 = crate::gpu_canvas::now_ms();
        while self.job.is_some() && crate::gpu_canvas::now_ms() - t0 < ms {
            if !self.poll(ctx) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    /// The overlay texture: the sampling area (or what is excluded) tinted, at proxy size.
    fn build_overlay(&mut self, ctx: &egui::Context) {
        let Some(m) = &self.masks else { return };
        let k = self.k as i32;
        let (tw, th) = ((self.canvas.width() as usize).div_ceil(k as usize).max(1), (self.canvas.height() as usize).div_ceil(k as usize).max(1));
        let [r, g, b] = self.settings.color;
        let tint = Color32::from_rgba_unmultiplied(r, g, b, (self.settings.opacity / 100.0 * 255.0).round() as u8);
        let excluded = self.settings.show == "excluded";
        let ww = m.window.width() as usize;
        let mut px = vec![Color32::TRANSPARENT; tw * th];
        for ty in 0..th {
            for tx in 0..tw {
                let (x, y) = (self.canvas.x0 + tx as i32 * k, self.canvas.y0 + ty as i32 * k);
                let (sampled, hole) = if m.window.contains(x, y) {
                    let i = (y - m.window.y0) as usize * ww + (x - m.window.x0) as usize;
                    (m.source.get(i).copied().unwrap_or(false), m.cover.get(i).is_some_and(|c| *c > 0.0))
                } else {
                    (false, false)
                };
                if !hole && sampled != excluded {
                    px[ty * tw + tx] = tint;
                }
            }
        }
        self.overlay_tex = Some(ctx.load_texture("caf-overlay", ColorImage::new([tw, th], px), egui::TextureOptions::NEAREST));
    }

    fn fit(&mut self, area: ERect) {
        let (cw, ch) = (self.canvas.width().max(1) as f32, self.canvas.height().max(1) as f32);
        self.zoom = ((area.width() - 40.0) / cw).min((area.height() - 40.0) / ch).clamp(0.01, 4.0);
        self.center = [f64::from(self.canvas.x0) + f64::from(cw) / 2.0, f64::from(self.canvas.y0) + f64::from(ch) / 2.0];
    }

    /// Preview framing: the fill area, about half the panel across.
    fn fit_preview(&mut self, area: ERect) {
        let fill = self.masks.as_ref().and_then(|m| {
            let b = prism_algo::caf::mask_bounds(m.window.width() as usize, m.window.height() as usize, &m.cover.iter().map(|c| *c > 0.0).collect::<Vec<_>>())?;
            Some((f64::from(m.window.x0) + (b.x0 + b.x1) as f64 / 2.0, f64::from(m.window.y0) + (b.y0 + b.y1) as f64 / 2.0, b.width().max(b.height()).max(1) as f32))
        });
        if let Some((cx, cy, side)) = fill {
            self.preview_center = [cx, cy];
            self.preview_zoom = (area.width().min(area.height()) * 0.55 / side).clamp(0.05, 8.0);
        }
    }
}

/// Opens the workspace on the active layer (needs a pixel layer and a selection).
pub fn open(app: &mut PrismApp, ctx: &egui::Context) -> Result<(), String> {
    if !app.session.is_enabled(CMD) {
        return Err(tl!("Content-Aware Fill needs a selection on a pixel layer").to_string());
    }
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.ok_or("no active layer")?;
    let canvas = st.doc.bounds();
    let layer_name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let name = st.path.as_deref().and_then(|p| std::path::Path::new(p).file_name()).map_or_else(|| tl!("Untitled").to_string(), |n| n.to_string_lossy().into_owned());
    let depth = st.doc.depth;
    let title = format!("{name} ({layer_name}, {}/{})", crate::canvas::mode_label(&st.doc), depth.bits());
    let doc = st.doc.clone();
    let k = proxy_k(canvas);
    let mut settings = app.session.prefs().dialogs.get(REMEMBERED).and_then(|v| serde_json::from_value::<CafSettings>(v.clone()).ok()).unwrap_or_default().sanitized();
    // Every session starts from the automatic sampling area, as in Photoshop.
    settings.sampling = "auto".into();
    let base = render_image(&doc, canvas, PROXY_SIDE);
    let mut w = CafWorkspace {
        layer,
        title,
        canvas,
        k,
        settings,
        tool: CafTool::SamplingBrush,
        brush_add: false,
        lasso_mode: Combine::Add,
        strokes: Vec::new(),
        fill_ops: Vec::new(),
        undo: Vec::new(),
        redo: Vec::new(),
        cur_stroke: None,
        cur_lasso: None,
        base_tex: Some(ctx.load_texture("caf-base", base, egui::TextureOptions::LINEAR)),
        overlay_tex: None,
        preview_tex: None,
        crop_tex: None,
        masks: None,
        ants: Vec::new(),
        job: None,
        generation: 0,
        preview_ready: false,
        preview_empty: false,
        preview_ms: 0.0,
        error: None,
        overlay_dirty: false,
        zoom: 0.0,
        center: [0.0, 0.0],
        preview_zoom: 0.0,
        preview_center: [0.0, 0.0],
        hover: None,
    };
    w.restart(doc);
    app.distort.caf = Some(w);
    Ok(())
}

fn remember(app: &mut PrismApp, s: &CafSettings) {
    if let Ok(v) = serde_json::to_value(s) {
        app.session.prefs.edit(|p| p.dialogs.insert(REMEMBERED.into(), v));
    }
}

/// Cancel (button, Esc, the back arrow, the control channel): the document is untouched.
pub fn cancel(app: &mut PrismApp) {
    if let Some(w) = app.distort.caf.take() {
        if let Some(j) = &w.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
        remember(app, &w.settings);
    }
}

/// Run the fill with the workspace's data (one history step).
fn run_fill(app: &mut PrismApp) -> Result<Value, String> {
    let w = app.distort.caf.as_ref().ok_or("Content-Aware Fill is not open")?;
    let p = w.params();
    let settings = w.settings.clone();
    remember(app, &settings);
    app.run(CMD, p)
}

/// OK: fill and close. On an error the workspace stays open and shows it.
pub fn commit(app: &mut PrismApp) -> Result<Value, String> {
    match run_fill(app) {
        Ok(v) => {
            if let Some(w) = app.distort.caf.take()
                && let Some(j) = &w.job
            {
                j.cancel.store(true, Ordering::Relaxed);
            }
            Ok(v)
        }
        Err(e) => {
            if let Some(w) = app.distort.caf.as_mut() {
                w.error = Some(e.clone());
            }
            Err(e)
        }
    }
}

/// Apply: fill and stay in the workspace, which then works on the result (the new layer, when
/// the output made one) with the sampling and fill data cleared.
pub fn apply(app: &mut PrismApp, ctx: &egui::Context) -> Result<Value, String> {
    let r = run_fill(app);
    let Some(mut w) = app.distort.caf.take() else { return r };
    match &r {
        Ok(_) => {
            let settings = w.settings.clone();
            if open(app, ctx).is_err() {
                // The fill used up the selection (or the layer changed kind): close.
                return r;
            }
            if let Some(n) = app.distort.caf.as_mut() {
                n.settings = settings;
                n.zoom = w.zoom;
                n.center = w.center;
            }
        }
        Err(e) => {
            w.error = Some(e.clone());
            app.distort.caf = Some(w);
        }
    }
    r
}

/// Control channel: `edit.contentAwareFill {"ui": {...}}` while the workspace is open.
pub fn control(app: &mut PrismApp, ctx: &egui::Context, ui: &Value) -> Result<Value, String> {
    let flag = |k: &str| ui.get(k).and_then(Value::as_bool) == Some(true);
    if flag("cancel") {
        cancel(app);
        return Ok(json!({"cancelled": true}));
    }
    let doc = app.session.active().map(|s| s.doc.clone()).ok_or("no document")?;
    let w = app.distort.caf.as_mut().ok_or("Content-Aware Fill is not open")?;
    let before = (w.params(), w.settings.clone());
    w.settings.apply(ui)?;
    if let Some(s) = ui.get("sampling").and_then(Value::as_str) {
        // Choosing a mode starts the area over, like clicking its button.
        w.settings.sampling = before.1.sampling.clone();
        w.set_sampling(s);
    }
    if let Some(t) = ui.get("tool") {
        w.tool = serde_json::from_value(t.clone()).map_err(|_| "`tool` must be samplingBrush|lasso|hand|zoom")?;
    }
    match ui.get("brushMode").and_then(Value::as_str) {
        None => {}
        Some("add") => w.brush_add = true,
        Some("subtract") => w.brush_add = false,
        Some(_) => return Err("`brushMode` must be add|subtract".into()),
    }
    if let Some(m) = ui.get("lassoMode").and_then(Value::as_str) {
        w.lasso_mode = match m {
            "new" => Combine::New,
            "add" => Combine::Add,
            "subtract" => Combine::Subtract,
            "intersect" => Combine::Intersect,
            _ => return Err("`lassoMode` must be new|add|subtract|intersect".into()),
        };
    }
    if let Some(s) = ui.get("stroke") {
        let points = points_of(s.get("points").unwrap_or(&Value::Null))?;
        let add = match s.get("mode").and_then(Value::as_str) {
            None => w.brush_add,
            Some("add") => true,
            Some("subtract") => false,
            Some(_) => return Err("stroke `mode` must be add|subtract".into()),
        };
        let size = s.get("size").and_then(Value::as_f64).map_or(w.settings.brush_size, |f| f as f32);
        if !(1.0..=5000.0).contains(&size) || points.is_empty() {
            return Err("a stroke needs `points` and a `size` of 1..5000".into());
        }
        w.add_stroke(SamplingStroke { add, size, points });
    }
    if let Some(l) = ui.get("lasso") {
        let points = points_of(l.get("points").unwrap_or(&Value::Null))?;
        if points.len() < 3 {
            return Err("a lasso needs at least 3 points".into());
        }
        let mode = match l.get("mode").and_then(Value::as_str) {
            None => w.lasso_mode,
            Some("new") => Combine::New,
            Some("add") => Combine::Add,
            Some("subtract") => Combine::Subtract,
            Some("intersect") => Combine::Intersect,
            Some(_) => return Err("lasso `mode` must be new|add|subtract|intersect".into()),
        };
        w.add_fill_op(FillOp::Polygon { mode, points });
    }
    for (k, sign) in [("expand", 1), ("contract", -1)] {
        if let Some(v) = ui.get(k) {
            let px = v.as_f64().filter(|f| (1.0..=500.0).contains(f)).ok_or_else(|| format!("`{k}` must be 1..500 px"))?;
            w.add_fill_op(FillOp::Grow(sign * px.round() as i32));
        }
    }
    if flag("undo") {
        w.undo();
    }
    if flag("redo") {
        w.redo();
    }
    if flag("reset") {
        w.reset_all();
    }
    if w.params() != before.0 {
        w.restart(doc);
    } else if w.settings != before.1 {
        w.overlay_tex = None;
    }
    // OK and Apply run after the edits in the same call.
    if flag("commit") {
        return commit(app).map(|r| json!({"committed": true, "result": r}));
    }
    if flag("apply") {
        let r = apply(app, ctx)?;
        return Ok(json!({"applied": true, "result": r, "open": app.distort.caf.is_some()}));
    }
    let w = app.distort.caf.as_mut().ok_or("Content-Aware Fill is not open")?;
    if flag("wait") {
        w.wait(ctx, 120_000.0);
    }
    Ok(w.describe())
}

/// Keyboard shortcuts while the workspace is open (Enter / Esc are handled by `distort_ui`).
pub fn keys(app: &mut PrismApp, ctx: &egui::Context) {
    let doc = app.session.active().map(|s| s.doc.clone());
    let Some(w) = app.distort.caf.as_mut() else { return };
    let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    let mut changed = false;
    if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z)) {
        changed |= w.redo();
    } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
        changed |= w.undo();
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket)) {
        w.settings.brush_size = (w.settings.brush_size * 0.9).max(1.0);
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket)) {
        w.settings.brush_size = (w.settings.brush_size * 1.1).min(5000.0);
    }
    for (k, t) in [(egui::Key::B, CafTool::SamplingBrush), (egui::Key::L, CafTool::Lasso), (egui::Key::H, CafTool::Hand), (egui::Key::Z, CafTool::Zoom)] {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k)) {
            w.tool = t;
        }
    }
    if changed && let Some(doc) = doc {
        w.restart(doc);
    }
}

fn to_screen(zoom: f32, center: [f64; 2], area: ERect, p: [f64; 2]) -> Pos2 {
    area.center() + vec2(((p[0] - center[0]) as f32) * zoom, ((p[1] - center[1]) as f32) * zoom)
}

fn to_doc(zoom: f32, center: [f64; 2], area: ERect, p: Pos2) -> [f64; 2] {
    let v = (p - area.center()) / zoom.max(1e-6);
    [center[0] + f64::from(v.x), center[1] + f64::from(v.y)]
}

/// Marching ants along the fill area's outline.
fn draw_ants(painter: &egui::Painter, segs: &[crate::outline::Segment], to: impl Fn([f64; 2]) -> Pos2, time: f64) {
    let dash = 4.0f32;
    let phase = ((time * 10.0) % f64::from(dash * 2.0)) as f32;
    let clip = painter.clip_rect();
    for (a, b) in segs {
        let pa = to([f64::from(a[0]), f64::from(a[1])]);
        let pb = to([f64::from(b[0]), f64::from(b[1])]);
        if !clip.intersects(ERect::from_two_pos(pa, pb).expand(1.0)) {
            continue;
        }
        let (pa, pb) = (pos2(pa.x.round() + 0.5, pa.y.round() + 0.5), pos2(pb.x.round() + 0.5, pb.y.round() + 0.5));
        painter.line_segment([pa, pb], Stroke::new(1.0, Color32::WHITE));
        let len = pa.distance(pb);
        if len < 0.5 {
            continue;
        }
        let dir = (pb - pa) / len;
        let mut t = -(pa.x + pa.y + phase).rem_euclid(dash * 2.0);
        while t < len {
            let (s0, s1) = (t.max(0.0), (t + dash).min(len));
            if s1 > s0 {
                painter.line_segment([pa + dir * s0, pa + dir * s1], Stroke::new(1.0, Color32::BLACK));
            }
            t += dash * 2.0;
        }
    }
}

/// A collapsible panel section header with an optional reset button. Returns (open, reset).
fn section(ui: &mut egui::Ui, id: &str, label: &str, reset: bool) -> (bool, bool) {
    let t = Tokens::get(ui.ctx());
    let key = egui::Id::new(("caf-section", id));
    let mut open = ui.ctx().data_mut(|d| *d.get_persisted_mut_or(key, true));
    let mut clicked_reset = false;
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 18.0), Sense::click());
        crate::icons::paint(ui, r, if open { "chevron-down" } else { "chevron-right" }, 12.0, t.text_dim);
        let text = ui.add(egui::Label::new(egui::RichText::new(label).color(t.text).strong()).sense(Sense::click()));
        if resp.clicked() || text.clicked() {
            open = !open;
        }
        if reset {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked_reset = crate::icons::button(ui, "undo-2", 20.0, false, tl!("Reset to defaults")).clicked();
            });
        }
    });
    ui.ctx().data_mut(|d| d.insert_persisted(key, open));
    (open, clicked_reset)
}

/// A row of equal-width segment buttons; returns the clicked index.
fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: usize, width: f32) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut out = None;
    let w = (width / labels.len().max(1) as f32).floor();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, l) in labels.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
            let on = i == selected;
            let fill = if on { t.pressed } else if resp.hovered() { t.hover } else { t.field };
            ui.painter().rect_filled(r, 0.0, fill);
            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, *l, FontId::proportional(12.0), if on { t.text } else { t.text_dim });
            if resp.clicked() {
                out = Some(i);
            }
        }
    });
    out
}

/// Draws the workspace (a full-window layer over the app).
pub fn show(app: &mut PrismApp, ctx: &egui::Context) {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let doc = app.session.active().map(|s| s.doc.clone());
    let mut action: Option<&str> = None;
    let mut restart = false;
    let time = ctx.input(|i| i.time);
    egui::Area::new(egui::Id::new("caf-workspace")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(w) = app.distort.caf.as_mut() else { return };
        if w.poll(ctx) {
            ctx.request_repaint();
        }
        if w.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if w.overlay_tex.is_none() && w.masks.is_some() {
            w.build_overlay(ctx);
        }
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        let alt = ui.input(|i| i.modifiers.alt);

        // ---------------------------------------------------------------- options bar
        let bar = ERect::from_min_size(full.min, vec2(full.width(), OPTIONS_H));
        painter.rect_filled(bar, 0.0, t.dock);
        painter.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, t.separator));
        let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if crate::icons::button(&mut bui, "chevrons-left", 26.0, false, tl!("Cancel and leave Content-Aware Fill")).clicked() {
            action = Some("cancel");
        }
        widgets::vline(&mut bui, 22.0);
        let (tool_r, _) = bui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
        crate::icons::paint(&bui, tool_r, w.tool.icon(), 16.0, t.text_dim);
        widgets::vline(&mut bui, 22.0);
        match w.tool {
            CafTool::SamplingBrush => {
                let add = w.brush_add != alt;
                if crate::icons::button(&mut bui, "plus", 24.0, add, tl!("Add to the sampling area")).clicked() {
                    w.brush_add = true;
                }
                if crate::icons::button(&mut bui, "minus", 24.0, !add, tl!("Subtract from the sampling area (Alt)")).clicked() {
                    w.brush_add = false;
                }
                bui.add_space(8.0);
                bui.label(egui::RichText::new(tl!("Size")).color(t.text_dim));
                widgets::value_field(&mut bui, &mut w.settings.brush_size, 1.0..=5000.0, "", 56.0);
            }
            CafTool::Lasso => {
                for (mode, icon, tip) in [
                    (Combine::New, "square", "New selection"),
                    (Combine::Add, "plus", "Add to selection"),
                    (Combine::Subtract, "minus", "Subtract from selection"),
                    (Combine::Intersect, "squares-subtract", "Intersect with selection"),
                ] {
                    if crate::icons::button(&mut bui, icon, 24.0, w.lasso_mode == mode, tl!(tip)).clicked() {
                        w.lasso_mode = mode;
                    }
                }
                bui.add_space(10.0);
                let step = w.settings.step as i32;
                if widgets::secondary_button(&mut bui, crate::i18n::tr_ctx(crate::i18n::current(), "caf", "Expand"), 52.0).clicked() {
                    w.add_fill_op(FillOp::Grow(step));
                    restart = true;
                }
                if widgets::secondary_button(&mut bui, crate::i18n::tr_ctx(crate::i18n::current(), "caf", "Contract"), 52.0).clicked() {
                    w.add_fill_op(FillOp::Grow(-step));
                    restart = true;
                }
                bui.label(egui::RichText::new(crate::i18n::tr_ctx(crate::i18n::current(), "caf", "Step")).color(t.text_dim));
                let opts: Vec<(u32, String)> = STEPS.iter().map(|s| (*s, crate::i18n::fmt(tl!("{n} px"), &[("n", &s.to_string())]))).collect();
                let refs: Vec<(u32, &str)> = opts.iter().map(|(v, l)| (*v, l.as_str())).collect();
                widgets::dropdown(&mut bui, "caf-step", &mut w.settings.step, &refs, 76.0);
            }
            CafTool::Hand | CafTool::Zoom => {
                if widgets::secondary_button(&mut bui, tl!("Fit on Screen"), 90.0).clicked() {
                    w.zoom = 0.0;
                }
                if widgets::secondary_button(&mut bui, "100%", 52.0).clicked() {
                    w.zoom = 1.0;
                }
            }
        }

        // ---------------------------------------------------------------- tool strip
        let body = ERect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
        let tools = ERect::from_min_size(body.min, vec2(TOOLS_W, body.height()));
        painter.rect_filled(tools, 0.0, t.dock);
        painter.line_segment([tools.right_top(), tools.right_bottom()], Stroke::new(1.0, t.separator));
        let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(tools.shrink2(vec2(4.0, 8.0))));
        strip.spacing_mut().item_spacing.y = 4.0;
        for tool in CafTool::ALL {
            if crate::icons::button(&mut strip, tool.icon(), 28.0, w.tool == tool, tl!(tool.tip())).clicked() {
                w.tool = tool;
            }
        }

        // ---------------------------------------------------------------- panel
        let panel = ERect::from_min_size(pos2(body.right() - PANEL_W, body.top()), vec2(PANEL_W, body.height()));
        painter.rect_filled(panel, 0.0, t.dock);
        painter.line_segment([panel.left_top(), panel.left_bottom()], Stroke::new(1.0, t.separator));
        let head = ERect::from_min_size(panel.min, vec2(PANEL_W, 26.0));
        painter.text(head.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, tl!("Content-Aware Fill"), FontId::proportional(12.0), t.text);
        painter.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.separator));
        let foot_h = 52.0;
        let props_rect = ERect::from_min_max(pos2(panel.left() + 10.0, head.bottom() + 8.0), pos2(panel.right() - 10.0, panel.bottom() - foot_h));
        let mut props = ui.new_child(egui::UiBuilder::new().max_rect(props_rect));
        let before = (w.params(), w.settings.clone());
        let inner_w = props_rect.width() - 8.0;
        egui::ScrollArea::vertical().id_salt("caf-props").show(&mut props, |ui| {
            ui.set_width(inner_w);
            ui.spacing_mut().item_spacing.y = 6.0;
            let (open, _) = section(ui, "overlay", tl!("Sampling Area Overlay"), false);
            if open {
                widgets::checkbox(ui, &mut w.settings.show_overlay, tl!("Show Sampling Area"));
                widgets::slider_row(ui, tl!("Opacity"), &mut w.settings.opacity, 0.0..=100.0, "%", None);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Color")).color(t.text_dim));
                    widgets::color_edit_button_srgb(ui, &mut w.settings.color);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::dropdown(ui, "caf-show", &mut w.settings.show, &[("sampling".to_string(), tl!("Sampling Area")), ("excluded".to_string(), tl!("Excluded Area"))], 110.0);
                        ui.label(egui::RichText::new(crate::i18n::tr_ctx(crate::i18n::current(), "caf", "Show")).color(t.text_dim));
                    });
                });
            }
            widgets::hairline(ui);
            let (open, reset) = section(ui, "sampling", tl!("Sampling Area Options"), true);
            if reset {
                w.set_sampling("auto");
                w.settings.sample_all_layers = false;
            }
            if open {
                let modes = ["auto", "rectangular", "custom"];
                let sel = modes.iter().position(|m| *m == w.sampling_shown()).unwrap_or(0);
                let ctxs = crate::i18n::current();
                let labels = [crate::i18n::tr_ctx(ctxs, "caf", "Auto"), crate::i18n::tr_ctx(ctxs, "caf", "Rectangular"), crate::i18n::tr_ctx(ctxs, "caf", "Custom")];
                if let Some(i) = segmented(ui, &labels, sel, inner_w) {
                    w.set_sampling(modes[i]);
                }
                let one_layer = doc.as_ref().is_some_and(|d| d.layers.len() < 2);
                ui.add_enabled_ui(!one_layer, |ui| widgets::checkbox(ui, &mut w.settings.sample_all_layers, tl!("Sample All Layers")));
            }
            widgets::hairline(ui);
            let (open, reset) = section(ui, "fill", tl!("Fill Settings"), true);
            if reset {
                let d = CafSettings::default();
                w.settings.color_adaptation = d.color_adaptation;
                w.settings.rotation_adaptation = d.rotation_adaptation;
                w.settings.scale = d.scale;
                w.settings.mirror = d.mirror;
            }
            if open {
                let lang = crate::i18n::current();
                let c = |s: &'static str| crate::i18n::tr_ctx(lang, "caf", s);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(c("Color Adaptation")).color(t.text_dim));
                    widgets::dropdown(
                        ui,
                        "caf-color",
                        &mut w.settings.color_adaptation,
                        &[("none".to_string(), c("None")), ("default".to_string(), c("Default")), ("high".to_string(), c("High")), ("veryHigh".to_string(), c("Very High"))],
                        90.0,
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(c("Rotation Adaptation")).color(t.text_dim));
                    widgets::dropdown(
                        ui,
                        "caf-rotation",
                        &mut w.settings.rotation_adaptation,
                        &[
                            ("none".to_string(), c("None")),
                            ("low".to_string(), c("Low")),
                            ("medium".to_string(), c("Medium")),
                            ("high".to_string(), c("High")),
                            ("full".to_string(), c("Full")),
                        ],
                        90.0,
                    );
                });
                ui.horizontal(|ui| {
                    widgets::checkbox(ui, &mut w.settings.scale, c("Scale"));
                    ui.add_space(18.0);
                    widgets::checkbox(ui, &mut w.settings.mirror, c("Mirror"));
                });
            }
            widgets::hairline(ui);
            let (open, _) = section(ui, "output", tl!("Output Settings"), false);
            if open {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Output To")).color(t.text_dim));
                    widgets::dropdown(
                        ui,
                        "caf-output",
                        &mut w.settings.output,
                        &[("current".to_string(), tl!("Current Layer")), ("new".to_string(), tl!("New Layer")), ("duplicate".to_string(), tl!("Duplicate Layer"))],
                        110.0,
                    );
                });
            }
            if let Some(e) = &w.error {
                widgets::hairline(ui);
                ui.label(egui::RichText::new(e).color(t.danger).size(11.0));
            }
        });
        // Footer: Reset, Cancel, Apply, OK.
        let foot = ERect::from_min_size(pos2(panel.left() + 10.0, panel.bottom() - foot_h + 10.0), vec2(PANEL_W - 20.0, 30.0));
        let mut fb = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if crate::icons::button(&mut fb, "undo-2", 26.0, false, tl!("Reset all settings")).clicked() {
            w.reset_all();
        }
        // Photoshop's order: Cancel, Apply, OK (the default), right-aligned.
        fb.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::primary_button(ui, tl!("OK"), 56.0).clicked() {
                action = Some("ok");
            }
            if widgets::secondary_button(ui, tl!("Apply"), 56.0).clicked() {
                action = Some("apply");
            }
            if widgets::secondary_button(ui, tl!("Cancel"), 56.0).clicked() {
                action = Some("cancel");
            }
        });
        if w.params() != before.0 || w.overlay_dirty {
            restart = true;
        } else if w.settings != before.1 {
            w.overlay_tex = None;
        }

        // ---------------------------------------------------------------- preview panel
        let pv = ERect::from_min_size(pos2(panel.left() - PREVIEW_W, body.top()), vec2(PREVIEW_W, body.height()));
        painter.rect_filled(pv, 0.0, t.dock);
        painter.line_segment([pv.left_top(), pv.left_bottom()], Stroke::new(1.0, t.separator));
        let pv_head = ERect::from_min_size(pv.min, vec2(PREVIEW_W, 26.0));
        painter.text(pv_head.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, tl!("Preview"), FontId::proportional(12.0), t.text);
        painter.line_segment([pv_head.left_bottom(), pv_head.right_bottom()], Stroke::new(1.0, t.separator));
        let pv_foot = ERect::from_min_size(pos2(pv.left(), pv.bottom() - 34.0), vec2(PREVIEW_W, 34.0));
        let pv_area = ERect::from_min_max(pos2(pv.left() + 1.0, pv_head.bottom()), pos2(pv.right(), pv_foot.top()));
        painter.rect_filled(pv_area, 0.0, t.canvas);
        if w.preview_zoom <= 0.0 && w.masks.is_some() {
            w.fit_preview(pv_area);
        }
        let pz = w.preview_zoom.max(0.01);
        let pclip = painter.with_clip_rect(pv_area);
        let img_rect = |zoom: f32, center: [f64; 2], area: ERect, r: Rect| {
            ERect::from_min_max(to_screen(zoom, center, area, [f64::from(r.x0), f64::from(r.y0)]), to_screen(zoom, center, area, [f64::from(r.x1), f64::from(r.y1)]))
        };
        let uv = ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let pimg = img_rect(pz, w.preview_center, pv_area, w.canvas);
        widgets::checker(&pclip, pimg.intersect(pv_area), 8.0);
        if let Some(tex) = w.preview_tex.as_ref().or(w.base_tex.as_ref()) {
            pclip.image(tex.id(), pimg, uv, Color32::WHITE);
        }
        if w.preview_tex.is_some()
            && let Some((r, tex)) = &w.crop_tex
        {
            pclip.image(tex.id(), img_rect(pz, w.preview_center, pv_area, *r), uv, Color32::WHITE);
        }
        if !w.preview_ready {
            pclip.text(pv_area.right_bottom() - vec2(10.0, 10.0), Align2::RIGHT_BOTTOM, tl!("Updating…"), FontId::proportional(11.0), t.text_dim);
        }
        let presp = ui.interact(pv_area, egui::Id::new("caf-preview-area"), Sense::drag());
        if presp.dragged() {
            let dl = presp.drag_delta();
            w.preview_center = [w.preview_center[0] - f64::from(dl.x / pz), w.preview_center[1] - f64::from(dl.y / pz)];
        }
        let mut pfui = ui.new_child(egui::UiBuilder::new().max_rect(pv_foot.shrink2(vec2(10.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        // Until the fill area is known the framing isn't set: show 100% and ignore the field.
        let framed = w.preview_zoom > 0.0;
        let mut pct = if framed { w.preview_zoom * 100.0 } else { 100.0 };
        if widgets::value_field(&mut pfui, &mut pct, 5.0..=800.0, "%", 56.0).changed() && framed {
            w.preview_zoom = pct / 100.0;
        }
        let mut lz = pct.clamp(5.0, 800.0).ln();
        pfui.add_space(6.0);
        pfui.scope(|ui| {
            ui.set_width(PREVIEW_W - 100.0);
            if widgets::slider(ui, &mut lz, 5f32.ln()..=800f32.ln(), None).changed() && framed {
                w.preview_zoom = lz.exp() / 100.0;
            }
        });

        // ---------------------------------------------------------------- canvas
        let area = ERect::from_min_max(pos2(tools.right(), body.top()), pos2(pv.left(), body.bottom()));
        let status = ERect::from_min_size(pos2(area.left(), area.bottom() - 22.0), vec2(area.width(), 22.0));
        let tab = ERect::from_min_size(area.min, vec2(area.width(), 26.0));
        let area = ERect::from_min_max(pos2(area.left(), tab.bottom()), pos2(area.right(), status.top()));
        painter.rect_filled(tab, 0.0, t.tab_strip);
        painter.rect_filled(area, 0.0, t.canvas);
        painter.rect_filled(status, 0.0, t.dock);
        if w.zoom <= 0.0 {
            w.fit(area);
        }
        painter.text(
            tab.left_center() + vec2(12.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{} @ {:.0}%", w.title, w.zoom * 100.0),
            FontId::proportional(12.0),
            t.text,
        );
        painter.text(
            status.left_center() + vec2(10.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{:.0}%    {} × {} px", w.zoom * 100.0, w.canvas.width(), w.canvas.height()),
            FontId::proportional(11.0),
            t.text_dim,
        );
        let resp = ui.interact(area, egui::Id::new("caf-canvas"), Sense::click_and_drag());
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0
                && let Some(hp) = resp.hover_pos()
            {
                let b = to_doc(w.zoom, w.center, area, hp);
                w.zoom = (w.zoom * (scroll / 200.0).exp()).clamp(0.01, 32.0);
                let a = to_doc(w.zoom, w.center, area, hp);
                w.center = [w.center[0] + b[0] - a[0], w.center[1] + b[1] - a[1]];
            }
        }
        let clip = painter.with_clip_rect(area);
        let img = img_rect(w.zoom, w.center, area, w.canvas);
        widgets::checker(&clip, img.intersect(area), 8.0);
        if let Some(tex) = &w.base_tex {
            clip.image(tex.id(), img, uv, Color32::WHITE);
        }
        if w.settings.show_overlay
            && let Some(tex) = &w.overlay_tex
        {
            clip.image(tex.id(), img, uv, Color32::WHITE);
        }
        let (zoom, center) = (w.zoom, w.center);
        let to = |p: [f64; 2]| to_screen(zoom, center, area, p);
        // The stroke being painted: the brush footprint in the overlay colour (add) or grey.
        if let Some(s) = &w.cur_stroke {
            let [r, g, b] = w.settings.color;
            let c = if s.add { Color32::from_rgba_unmultiplied(r, g, b, 150) } else { Color32::from_black_alpha(110) };
            let pts: Vec<Pos2> = s.points.iter().map(|p| to([f64::from(p.0), f64::from(p.1)])).collect();
            let rad = s.size * 0.5 * zoom;
            if pts.len() == 1 {
                clip.circle_filled(pts[0], rad, c);
            } else {
                clip.add(egui::Shape::line(pts, Stroke::new(rad * 2.0, c)));
            }
        }
        draw_ants(&clip, &w.ants, to, time);
        if let Some((mode, pts)) = &w.cur_lasso
            && pts.len() > 1
        {
            let line: Vec<Pos2> = pts.iter().map(|p| to([f64::from(p.0), f64::from(p.1)])).collect();
            clip.add(egui::Shape::line(line, Stroke::new(1.0, if *mode == Combine::Subtract { t.danger } else { t.accent })));
        }
        if !w.ants.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        clip.rect_stroke(img, 0.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Outside);
        if let Some(hp) = resp.hover_pos() {
            w.hover = Some(to_doc(zoom, center, area, hp));
            if w.tool == CafTool::SamplingBrush {
                let r = w.settings.brush_size * 0.5 * zoom;
                clip.circle_stroke(hp, r, Stroke::new(1.5, Color32::BLACK));
                clip.circle_stroke(hp, r, Stroke::new(0.75, Color32::WHITE));
                let sign = if w.brush_add != alt { "+" } else { "−" };
                clip.text(hp, Align2::CENTER_CENTER, sign, FontId::proportional(12.0), Color32::WHITE);
            }
        }
        // Pointer → strokes, lassos, panning, zooming.
        let pan = ui.input(|i| i.key_down(egui::Key::Space) || i.pointer.middle_down()) || w.tool == CafTool::Hand;
        let mods = ui.input(|i| i.modifiers);
        if pan && resp.dragged() {
            let dl = resp.drag_delta();
            w.center = [w.center[0] - f64::from(dl.x / zoom), w.center[1] - f64::from(dl.y / zoom)];
        } else if w.tool == CafTool::Zoom && resp.clicked() {
            if let Some(hp) = resp.interact_pointer_pos() {
                let b = to_doc(w.zoom, w.center, area, hp);
                w.zoom = (w.zoom * if mods.alt { 0.5 } else { 2.0 }).clamp(0.01, 32.0);
                let a = to_doc(w.zoom, w.center, area, hp);
                w.center = [w.center[0] + b[0] - a[0], w.center[1] + b[1] - a[1]];
            }
        } else if let Some(pp) = resp.interact_pointer_pos() {
            let q = to_doc(zoom, center, area, pp);
            let q = (q[0] as f32, q[1] as f32);
            let down = resp.drag_started() || resp.clicked() || resp.is_pointer_button_down_on();
            match w.tool {
                CafTool::SamplingBrush if down => {
                    let add = w.brush_add != mods.alt;
                    let size = w.settings.brush_size;
                    let s = w.cur_stroke.get_or_insert_with(|| SamplingStroke { add, size, points: Vec::new() });
                    if s.points.last().is_none_or(|l| (l.0 - q.0).abs() >= 1.0 || (l.1 - q.1).abs() >= 1.0) {
                        s.points.push(q);
                    }
                    ctx.request_repaint();
                }
                CafTool::Lasso if down => {
                    let mode = if mods.alt && mods.shift {
                        Combine::Intersect
                    } else if mods.alt {
                        Combine::Subtract
                    } else if mods.shift {
                        Combine::Add
                    } else {
                        w.lasso_mode
                    };
                    let (_, pts) = w.cur_lasso.get_or_insert_with(|| (mode, Vec::new()));
                    if pts.last().is_none_or(|l| (l.0 - q.0).abs() >= 1.0 || (l.1 - q.1).abs() >= 1.0) {
                        pts.push(q);
                    }
                    ctx.request_repaint();
                }
                _ => {}
            }
        }
        let primary_down = ui.input(|i| i.pointer.primary_down());
        if !primary_down {
            if let Some(s) = w.cur_stroke.take() {
                w.add_stroke(s);
                restart = true;
            }
            if let Some((mode, pts)) = w.cur_lasso.take() {
                w.add_fill_op(FillOp::Polygon { mode, points: pts });
                restart = true;
            }
        }
        if restart {
            w.overlay_dirty = false;
            if let Some(d) = &doc {
                w.restart(d.clone());
            }
        }
    });
    match action {
        Some("ok") => {
            let _ = commit(app);
        }
        Some("apply") => {
            let _ = apply(app, ctx);
        }
        Some("cancel") => cancel(app),
        _ => {}
    }
}

#[cfg(test)]
mod tests;
