//! Filter › Neural Filters: Super Zoom with Real-ESRGAN (`prism-ml`).
//!
//! Two outputs:
//!
//! - `"document"` (default): the document is enlarged **in place**, as one undoable step (⌘Z/Ctrl+Z
//!   brings it back). Everything is resized as Image Size does it (layers, masks, channels, the
//!   selection, vectors, guides, effect sizes), then the active pixel layer's canvas area is replaced
//!   by the Real-ESRGAN result.
//! - `"newDocument"`: like Photoshop's Super Zoom, the active pixel layer (inside the selection's
//!   bounds, when there is a selection) is enlarged into a new document; the source is untouched.
//!
//! The x4plus models only have a 4× network, so ×2 and ×3 resample their 4× result (Lanczos, edges
//! extended); `realesr-animevideov3` (the fast model) runs its own 2×, 3× and 4× networks.
//!
//! The result reports where the time went (`ms`: runtime install, model run, PNG hand-off, pixel
//! conversion, resampling, document resize).
//!
//! The model works on 8-bit RGB(A): RGB and Grayscale documents at 8 or 16 bits/channel are
//! accepted, and the result is stored at the document's own depth (16-bit results carry 8-bit
//! precision). CMYK, Lab, other modes and 32-bit documents are refused with an error, as in
//! Photoshop.
//!
//! The first run in the app downloads the pinned Real-ESRGAN package (see
//! [`prism_ml::install`]); `"install": false` turns that off, and processes that never called
//! [`configure`] (tests, headless use) never download.

use prism_algo::resample::{Resample, resize_surface_in_canvas};
use prism_color::{ColorMode, PixelFormat, SampleType};
use prism_doc::{Document, Layer, LayerContent, Size};
use prism_geom::Rect;
pub use prism_ml::Model;
use prism_ml::{Rgba8, Runtime};
use prism_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::jobs::JobCtx;
use crate::{EngineError, Result, Session};
use std::time::Instant;

pub const COMMAND: &str = "filter.neuralFilters";
/// Query: what is installed and where runtimes are looked for (no document needed).
pub const STATUS: &str = "filter.neuralFilters.status";
/// The output factors offered (Photoshop's Super Zoom steps up to 4×).
pub const SCALES: [u32; 3] = [2, 3, 4];

/// Tells `prism-ml` where the app keeps per-user data (`<dir>/realesrgan` holds the installed
/// runtime). Called once by the desktop app at start-up.
pub fn configure(user_dir: Option<std::path::PathBuf>) {
    prism_ml::runtime::configure(user_dir);
}

/// Whether `model` can run now without a download.
pub fn installed(model: Model) -> bool {
    Runtime::find(model).is_some()
}

/// Whether a missing runtime can be downloaded in this process.
pub fn can_install() -> bool {
    prism_ml::runtime::user_runtime_dir().is_some() && prism_ml::install::package().is_some()
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: msg.into() }
}

fn ml_err(e: prism_ml::Error) -> EngineError {
    match e {
        prism_ml::Error::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    }
}

/// A pixel layer in an RGB or Grayscale document at 8 or 16 bits/channel.
fn enabled(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if !matches!(l.content, LayerContent::Raster(_)) {
        return Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()));
    }
    let d = s.active().ok_or("no document open")?;
    supported(d.doc.mode, d.doc.depth)
}

fn supported(mode: ColorMode, depth: SampleType) -> std::result::Result<(), String> {
    if !matches!(mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("Neural Filters work on RGB and Grayscale documents; convert with Image › Mode first".into());
    }
    if depth == SampleType::F32 {
        return Err("Neural Filters work on 8 and 16 bits/channel documents; convert with Image › Mode first".into());
    }
    Ok(())
}

/// A [`JobCtx`] seen by `prism-ml`, mapped into `[from, to]` of the job's progress.
struct Ctl<'a> {
    ctx: &'a JobCtx,
    from: f32,
    to: f32,
}

impl prism_ml::Control for Ctl<'_> {
    fn progress(&self, fraction: f32, message: &str) {
        let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
        self.ctx.progress(self.from + (self.to - self.from) * f, message);
    }
    fn cancelled(&self) -> bool {
        self.ctx.cancelled()
    }
}

/// [`STATUS`]: what is installed, and where runtimes are looked for.
fn status(_: &mut Session, _: &Value) -> Result<Value> {
    let models: serde_json::Map<String, Value> = Model::ALL
        .iter()
        .map(|m| (m.id().to_string(), json!({"label": m.label(), "installed": installed(*m), "dir": Runtime::find(*m).map(|r| r.dir.display().to_string())})))
        .collect();
    Ok(json!({
        "filters": ["superZoom"],
        "models": models,
        "canInstall": can_install(),
        "searched": prism_ml::runtime::candidates().iter().map(|d| d.display().to_string()).collect::<Vec<_>>(),
    }))
}

/// Where the result goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    /// Enlarge the document in place (one undo step).
    Document,
    /// A new document from the active layer (within the selection's bounds).
    NewDocument,
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let filter = match p.get("filter") {
        None => "superZoom",
        Some(v) => v.as_str().ok_or_else(|| bad("`filter` must be a string"))?,
    };
    if filter != "superZoom" {
        return Err(bad(format!("unknown neural filter `{filter}` (superZoom)")));
    }
    let model = match p.get("model") {
        None => Model::X4Plus,
        Some(v) => {
            let id = v.as_str().ok_or_else(|| bad("`model` must be a string"))?;
            Model::parse(id).ok_or_else(|| bad(format!("unknown model `{id}` (realesrgan-x4plus | realesrgan-x4plus-anime | realesr-animevideov3)")))?
        }
    };
    let scale = match p.get("scale") {
        None => 4,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| SCALES.contains(n)).ok_or_else(|| bad("`scale` must be 2, 3 or 4"))?,
    };
    let install = match p.get("install") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| bad("`install` must be true or false"))?,
    };
    let output = match p.get("output").map(|v| v.as_str()) {
        None | Some(Some("document")) => Output::Document,
        Some(Some("newDocument")) => Output::NewDocument,
        Some(_) => return Err(bad("`output` must be \"document\" or \"newDocument\"")),
    };
    enabled(s).map_err(|m| EngineError::Disabled(COMMAND.into(), m))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &st.doc;
    let layer_id = st.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    let layer = doc.layer(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
    let LayerContent::Raster(src) = &layer.content else {
        return Err(EngineError::Other("the active layer has no pixels".into()));
    };
    let canvas = doc.bounds();
    // In place the whole canvas grows, so the selection only limits a new document.
    let area = match doc.selection.as_ref().map(Surface::content_bounds).filter(|r| !r.is_empty() && output == Output::NewDocument) {
        Some(sel) => sel.intersect(&canvas),
        None => canvas,
    };
    if area.is_empty() {
        return Err(EngineError::Other("the selection lies outside the canvas".into()));
    }
    let (w, h) = (area.width() as u32, area.height() as u32);
    prism_ml::esrgan::output_size(w, h, model.native_scale(scale)).map_err(ml_err)?;
    let (out_w, out_h) = (w * scale, h * scale);
    if !install && !installed(model) {
        return Err(EngineError::Other(format!("Real-ESRGAN ({}) is not installed", model.id())));
    }
    let src = src.clone();
    let fmt = src.format();
    let done = json!({"width": out_w, "height": out_h, "scale": scale, "model": model.id()});
    if output == Output::Document {
        return crate::jobs::edit_job(
            s,
            "Super Zoom",
            move |doc, _, ctx| {
                let (ai, mut ms) = super_zoom(&src, area, scale, model, install, ctx)?;
                ctx.progress(0.95, "Resizing the document");
                let t = Instant::now();
                let canvas = doc.bounds();
                // The active layer is replaced by the model's pixels, so it isn't resampled with
                // the rest unless it has content beyond the canvas (which keeps Image Size's look).
                let layer = doc.layer_mut(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
                let original = std::mem::replace(crate::pixels_mut(layer)?, Surface::new(fmt));
                let beyond = !canvas.contains_rect(&original.content_bounds()) && !original.content_bounds().is_empty();
                crate::image_cmds::resize_document(doc, out_w, out_h, Resample::Bicubic, true);
                let layer = doc.layer_mut(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
                let surf = crate::pixels_mut(layer)?;
                if beyond {
                    let (sx, sy) = (f64::from(out_w) / f64::from(canvas.width().max(1) as u32), f64::from(out_h) / f64::from(canvas.height().max(1) as u32));
                    *surf = resize_surface_in_canvas(&original, sx, sy, Resample::Bicubic, canvas);
                    // Row bands of encoded bytes keep the copy cheap and its memory bounded.
                    for y0 in (0..out_h).step_by(256) {
                        let r = Rect::new(0, y0 as i32, out_w as i32, (y0 + 256).min(out_h) as i32);
                        surf.write_interleaved(r, &ai.to_interleaved(r));
                    }
                    surf.prune();
                } else {
                    *surf = ai;
                    surf.prune();
                }
                ms.document = elapsed(t);
                Ok(ms)
            },
            move |ms| {
                let mut v = done;
                v["inPlace"] = json!(true);
                v["ms"] = ms.to_json();
                v
            },
        );
    }
    let (mode, depth, icc, dpi) = (doc.mode, doc.depth, doc.icc_profile.clone(), doc.resolution_dpi);
    let base = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(stem, _)| stem).to_string();
    let name = format!("{base} Super Zoom {scale}x");
    let layer_name = if fmt.alpha { layer.name.clone() } else { "Background".to_string() };
    crate::jobs::run(
        s,
        "Super Zoom",
        false,
        move |ctx| super_zoom(&src, area, scale, model, install, ctx),
        move |s, (mut surf, ms)| {
            surf.prune();
            let mut doc = Document::new(name.clone(), Size::new(out_w, out_h), mode, depth);
            doc.icc_profile = icc;
            doc.resolution_dpi = dpi;
            doc.layers.push(Layer::new(layer_name, LayerContent::Raster(surf)));
            let index = s.add_document(doc, None);
            if let Some(st) = s.active_mut() {
                st.saved_revision = 0;
            }
            let mut v = done;
            v["document"] = json!(index);
            v["name"] = json!(name);
            v["ms"] = ms.to_json();
            Ok(v)
        },
    )
}

/// Milliseconds since `t`.
fn elapsed(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

/// Where a Super Zoom spent its time, in milliseconds (reported in the result's `ms`).
#[derive(Clone, Copy, Debug, Default)]
struct Timings {
    install: f64,
    read: f64,
    model: prism_ml::esrgan::Timings,
    convert: f64,
    resample: f64,
    document: f64,
}

impl Timings {
    fn to_json(self) -> Value {
        json!({
            "install": self.install,
            "read": self.read,
            "pngWrite": self.model.encode_ms.round(),
            "model": self.model.run_ms.round(),
            "pngRead": self.model.decode_ms.round(),
            "convert": self.convert,
            "resample": self.resample,
            "document": self.document,
        })
    }
}

/// `area` of `src` enlarged `scale`× with Real-ESRGAN, as a surface in `src`'s format at the
/// origin, and where the time went. Installs the runtime first when it's missing and `install`
/// allows it. The model runs at its own factor when it has one, else at 4× and is resampled.
fn super_zoom(src: &Surface, area: Rect, scale: u32, model: Model, install: bool, ctx: &JobCtx) -> Result<(Surface, Timings)> {
    let mut ms = Timings::default();
    let t = Instant::now();
    let runtime = match Runtime::find(model) {
        Some(rt) => rt,
        None if install => prism_ml::install::install(&Ctl { ctx, from: 0.0, to: 0.3 }).map_err(ml_err)?,
        None => return Err(EngineError::Other(format!("Real-ESRGAN ({}) is not installed", model.id()))),
    };
    ms.install = elapsed(t);
    ctx.check()?;
    ctx.progress(0.3, "Super Zoom");
    let t = Instant::now();
    let rgba = to_rgba8(src, area)?;
    ms.read = elapsed(t);
    let native = model.native_scale(scale);
    let (up, model_ms) = prism_ml::upscale(&runtime, model, native, &rgba, &Ctl { ctx, from: 0.3, to: 0.9 }).map_err(ml_err)?;
    ms.model = model_ms;
    drop(rgba);
    ctx.check()?;
    let t = Instant::now();
    let surf = from_rgba8(&up, src.format())?;
    ms.convert = elapsed(t);
    if native == scale {
        return Ok((surf, ms));
    }
    ctx.progress(0.9, "Resampling");
    let t = Instant::now();
    let f = f64::from(scale) / f64::from(native);
    let out = resize_surface_in_canvas(&surf, f, f, Resample::Lanczos, Rect::new(0, 0, up.width as i32, up.height as i32));
    ms.resample = elapsed(t);
    Ok((out, ms))
}

/// `area` of `src` as 8-bit straight RGBA (gray is repeated into R, G and B; no alpha is opaque).
/// RGBA8 surfaces are copied tile row by tile row without a float round trip.
fn to_rgba8(src: &Surface, area: Rect) -> Result<Rgba8> {
    let fmt = src.format();
    if !matches!(fmt.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err(EngineError::Other(format!("Super Zoom can't read {:?} pixels", fmt.mode)));
    }
    let (w, h) = (area.width() as u32, area.height() as u32);
    let mut px = vec![[0u8; 4]; w as usize * h as usize];
    src.read_rgba8_into(area, &mut px);
    Rgba8::new(w, h, px.into_flattened()).map_err(ml_err)
}

/// An 8-bit RGBA result as a surface in `fmt` at the origin (gray takes the mean of R, G, B; a
/// format without alpha drops it). The samples are encoded straight into the surface's byte
/// layout and copied in by rows; an RGBA8 surface takes the bytes as they are.
fn from_rgba8(img: &Rgba8, fmt: PixelFormat) -> Result<Surface> {
    let color = fmt.channels() - usize::from(fmt.alpha);
    if !matches!(color, 1 | 3) || fmt.sample == SampleType::F32 {
        return Err(EngineError::Other(format!("Super Zoom can't write {:?} {:?} pixels", fmt.mode, fmt.sample)));
    }
    let r = Rect::new(0, 0, img.width as i32, img.height as i32);
    if color == 3 && fmt.alpha && fmt.sample == SampleType::U8 {
        return Ok(Surface::from_interleaved(fmt, r, &img.data));
    }
    let wide = fmt.sample == SampleType::U16;
    let mut bytes = Vec::with_capacity(img.data.len() / 4 * fmt.bytes_per_pixel());
    let mut push = |v: u8| {
        if wide {
            bytes.extend((u16::from(v) * 257).to_ne_bytes());
        } else {
            bytes.push(v);
        }
    };
    for &[r, g, b, a] in img.data.as_chunks::<4>().0 {
        if color == 1 {
            push(((u16::from(r) + u16::from(g) + u16::from(b) + 1) / 3) as u8);
        } else {
            push(r);
            push(g);
            push(b);
        }
        if fmt.alpha {
            push(a);
        }
    }
    Ok(Surface::from_interleaved(fmt, r, &bytes))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: COMMAND,
            label: "Neural Filters…",
            menu: &["Filter"],
            shortcut: None,
            params: r##"{"filter":"superZoom"="superZoom","model":"realesrgan-x4plus"|"realesrgan-x4plus-anime"|"realesr-animevideov3"="realesrgan-x4plus","scale":2|3|4=4,"output":"document"|"newDocument"="document","install":bool=true} — Super Zoom with Real-ESRGAN. "document": enlarges the document in place as one undoable step (everything resized as Image Size does; the active pixel layer gets the model's pixels). "newDocument": the active pixel layer (within the selection's bounds, if any) into a new document, as Photoshop's Super Zoom. x4plus suits photos (best, slowest), x4plus-anime illustrations, realesr-animevideov3 is the fast model (about 6× faster; its own 2×/3×/4× networks). The result's "ms" says where the time went. RGB or Grayscale, 8 or 16 bits/channel (the model works in 8 bits). The first run in the app downloads Real-ESRGAN (~50 MB from github.com/xinntao/Real-ESRGAN) unless "install":false (filter.neuralFilters.status reports what is installed)."##,
            enabled,
            run,
            journal: true,
        },
        CommandSpec {
            id: STATUS,
            label: "Neural Filters Status",
            menu: &[],
            shortcut: None,
            params: r##"{} — reports which Real-ESRGAN models can run without a download (per model: label, installed, dir), whether a missing runtime can be downloaded here, and the folders searched."##,
            enabled: crate::commands::always,
            run: status,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(mode: &str, depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 6, "height": 4, "mode": mode, "depth": depth})).unwrap();
        s
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = session("rgb", 8);
        for p in [
            json!({"filter": "skinSmoothing"}),
            json!({"filter": 3}),
            json!({"model": "realesr-general-x4v3"}),
            json!({"model": null}),
            json!({"scale": 1}),
            json!({"scale": 8}),
            json!({"scale": -4}),
            json!({"scale": "4"}),
            json!({"install": "yes"}),
            json!({"output": "layer"}),
            json!({"output": 1}),
        ] {
            assert!(matches!(s.execute(COMMAND, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
        }
    }

    #[test]
    fn without_a_runtime_nothing_changes() {
        let mut s = session("rgb", 8);
        let before = s.documents().len();
        // Without a runtime (the stand-in test may have configured one meanwhile) nothing runs,
        // and since no test configures an empty folder, nothing is downloaded either.
        if !installed(Model::X4Plus) {
            assert!(s.execute(COMMAND, json!({"install": false})).is_err());
        }
        assert_eq!(s.documents().len(), before);
    }

    #[test]
    fn status_reports_models() {
        let mut s = Session::new();
        let v = s.execute(STATUS, json!({})).unwrap();
        assert!(v["models"]["realesrgan-x4plus"]["installed"].is_boolean());
        assert!(v["models"]["realesrgan-x4plus-anime"]["label"].is_string());
    }

    #[test]
    fn unsupported_documents_are_refused() {
        for (mode, depth) in [("cmyk", 8), ("lab", 8), ("rgb", 32)] {
            let mut s = session(mode, depth);
            assert!(matches!(s.execute(COMMAND, json!({"install": false})), Err(EngineError::Disabled(..))), "{mode} {depth}");
        }
        let mut s = Session::new();
        assert!(s.execute(COMMAND, json!({})).is_err());
    }

    #[test]
    fn rgba8_conversion_round_trips() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::GRAYA8, PixelFormat::GRAY8, PixelFormat::new(ColorMode::Rgb, SampleType::U8, false)] {
            let n = fmt.channels();
            let r = Rect::new(0, 0, 3, 2);
            let vals: Vec<f32> = (0..6 * n).map(|i| ((i * 37) % 256) as f32 / 255.0).collect();
            let mut s = Surface::new(fmt);
            s.write_region(r, &vals);
            let rgba = to_rgba8(&s, r).unwrap();
            assert_eq!((rgba.width, rgba.height, rgba.data.len()), (3, 2, 24));
            let back = from_rgba8(&rgba, fmt).unwrap().read_region(r);
            for (a, b) in vals.iter().zip(&back) {
                assert!((a - b).abs() <= 1.0 / 255.0 + 1e-6, "{fmt:?}: {a} vs {b}");
            }
            if !fmt.alpha {
                assert!(rgba.data.chunks_exact(4).all(|p| p[3] == 255));
            }
        }
    }

    /// End to end with a stand-in executable that copies a prepared 4× image (orange, half
    /// transparent), so the job, resampling and the new document run without a GPU. The stand-in
    /// is installed in a configured user folder and left there, so no other test in this process
    /// can ever find the folder configured but empty (and try to download).
    #[cfg(unix)]
    #[test]
    fn super_zoom_makes_a_new_document_at_each_scale() {
        use std::os::unix::fs::PermissionsExt;
        let user = std::env::temp_dir().join(format!("prism-superzoom-engine-{}", std::process::id()));
        let dir = user.join(prism_ml::runtime::DIR_NAME);
        let _ = std::fs::remove_dir_all(&user);
        std::fs::create_dir_all(dir.join("models")).unwrap();
        for m in Model::ALL {
            for f in m.files() {
                std::fs::write(dir.join(f), b"x").unwrap();
            }
        }
        let exe = dir.join(prism_ml::runtime::executable_name());
        std::fs::write(&exe, "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = -o ] && o=\"$2\"; shift; done\ncp \"$(dirname \"$0\")/result.png\" \"$o\"\n")
            .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The document is 6×4: the stand-in's result is 24×16 RGBA, orange at alpha 128.
        let result: Vec<u8> = (0..24 * 16).flat_map(|_| [255u8, 128, 0, 128]).collect();
        let mut enc = Vec::new();
        {
            let mut e = png_like::encode(24, 16, &result);
            enc.append(&mut e);
        }
        std::fs::write(dir.join("result.png"), enc).unwrap();
        configure(Some(user.clone()));
        assert!(installed(Model::X4Plus));

        for (scale, depth) in [(4u32, 8u32), (2, 16), (3, 8)] {
            let mut s = session("rgb", depth);
            s.execute("layer.new.layer", json!({})).unwrap();
            let src = s.active_index().unwrap();
            let v = s.execute(COMMAND, json!({"scale": scale, "model": "realesrgan-x4plus-anime", "output": "newDocument"})).unwrap();
            assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(6 * scale as u64), Some(4 * scale as u64)), "{v}");
            assert_eq!(s.documents().len(), 2);
            let out = &s.active().unwrap().doc;
            assert_eq!((out.size.width, out.size.height), (6 * scale, 4 * scale));
            assert_eq!(out.depth, if depth == 16 { SampleType::U16 } else { SampleType::U8 });
            let LayerContent::Raster(px) = &out.layers[0].content else { panic!("not pixels") };
            let mid = px.read_region(Rect::new(2, 2, 3, 3));
            assert!((mid[0] - 1.0).abs() < 0.02 && (mid[1] - 0.5).abs() < 0.02 && mid[2] < 0.02, "{mid:?}");
            assert!((mid[3] - 0.5).abs() < 0.02, "{mid:?}");
            // The source document is untouched.
            assert_eq!(s.documents()[src].doc.size.width, 6);
        }
        // A selection limits the area.
        let mut s = session("rgb", 8);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 3, "height": 2})).unwrap();
        let v = s.execute(COMMAND, json!({"scale": 2, "output": "newDocument"})).unwrap_err();
        // The stand-in always returns 24×16, which isn't 4× of 3×2: refused, not stretched.
        assert!(v.to_string().contains("unusable"), "{v}");

        // In place (the default): the document grows as one undo step, and comes back.
        for depth in [8u32, 16] {
            let mut s = session("rgb", depth);
            s.execute("layer.new.layer", json!({})).unwrap();
            // A selection doesn't crop in place; it is resized with the document.
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 3, "height": 2})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let v = s.execute(COMMAND, json!({"scale": 2})).unwrap();
            assert_eq!(v["inPlace"], true, "{v}");
            assert_eq!(s.documents().len(), 1);
            let after = s.active().unwrap().doc.clone();
            assert_eq!((after.size.width, after.size.height), (12, 8));
            let layer = after.layers.last().unwrap();
            let LayerContent::Raster(px) = &layer.content else { panic!("not pixels") };
            let mid = px.read_region(Rect::new(5, 3, 6, 4));
            assert!((mid[0] - 1.0).abs() < 0.02 && (mid[1] - 0.5).abs() < 0.02 && (mid[3] - 0.5).abs() < 0.02, "{mid:?}");
            assert_eq!(after.selection.as_ref().map(Surface::content_bounds), Some(Rect::new(0, 0, 6, 4)));
            s.execute("edit.undo", json!({})).unwrap();
            let undone = s.active().unwrap().doc.clone();
            assert_eq!((undone.size.width, undone.size.height), (6, 4));
            assert!(std::sync::Arc::ptr_eq(&undone, &before) || *undone == *before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.size.width, 12);
        }
    }
}

#[cfg(test)]
mod png_like {
    /// A minimal 8-bit RGBA PNG writer for the stand-in's result (stored deflate blocks).
    pub fn encode(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut raw = Vec::new();
        for row in rgba.chunks_exact(w as usize * 4) {
            raw.push(0u8);
            raw.extend_from_slice(row);
        }
        let mut z = vec![0x78, 0x01];
        let chunks: Vec<&[u8]> = raw.chunks(65_535).collect();
        for (i, c) in chunks.iter().enumerate() {
            z.push(u8::from(i + 1 == chunks.len()));
            z.extend((c.len() as u16).to_le_bytes());
            z.extend((!(c.len() as u16)).to_le_bytes());
            z.extend_from_slice(c);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + x as u32) % 65_521;
            b = (b + a) % 65_521;
        }
        z.extend(((b << 16) | a).to_be_bytes());
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |t: &[u8], d: &[u8]| {
            out.extend((d.len() as u32).to_be_bytes());
            let mut c = flate2::Crc::new();
            out.extend_from_slice(t);
            out.extend_from_slice(d);
            c.update(t);
            c.update(d);
            out.extend(c.sum().to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        chunk(b"IDAT", &z);
        chunk(b"IEND", &[]);
        out
    }
}

/// The real executable through the whole command, when `PRISM_REALESRGAN_DIR` points at an
/// installed runtime: opaque background, transparent layer, 16 bits, every scale.
#[cfg(test)]
mod real_runtime {
    use super::*;

    #[test]
    #[ignore = "needs an installed Real-ESRGAN runtime (PRISM_REALESRGAN_DIR) and a Vulkan GPU"]
    fn super_zoom_with_the_real_executable() {
        assert!(installed(Model::X4Plus), "set PRISM_REALESRGAN_DIR");
        for (depth, new_layer, scale) in [(8u32, false, 4u32), (8, true, 2), (16, false, 3)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 48, "height": 32, "depth": depth, "background": "white"})).unwrap();
            if new_layer {
                s.execute("layer.new.layer", json!({})).unwrap();
                s.execute("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 12})).unwrap();
                s.execute("edit.fill", json!({"use": "black"})).ok();
                s.execute("select.deselect", json!({})).ok();
            }
            let r = s.execute(COMMAND, json!({"scale": scale, "install": false}));
            eprintln!("depth {depth} layer {new_layer} scale {scale}: {r:?}");
            let v = r.unwrap();
            assert_eq!(v["width"].as_u64(), Some(48 * scale as u64));
        }
    }
}
