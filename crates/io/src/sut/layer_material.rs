//! CLIP STUDIO layer materials: the `data/material_0.layer` of an `image` material is a small
//! canvas (C2F-framed SQLite, the `.clip` table family) whose layers carry pixels and special
//! rulers. Read-only and bounded; layout and confidence in `docs/csp-special-rulers.md`.

use super::{
    Row,
    material::{MaterialRaster, mipmap_raster},
    sqlite::{Database, Value},
};
use prism_doc::special_ruler::{
    MAX_GUIDES, MAX_RULERS, MAX_VANISHING_POINTS, PerspectiveRuler, RulerKind, SpecialRuler, SpecialRulers, VanishTarget, VanishingPoint, normalize_angle,
};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, String>;

/// Layers read from one material.
pub const MAX_LAYERS: usize = 64;
/// Folder nesting followed.
const MAX_DEPTH: usize = 16;
/// Pixels one layer may hold (the decoder's own source limit).
const MAX_LAYER_PIXELS: usize = 64 << 20;
/// Largest `icedata/layerData.xml` read.
pub const INFO_LIMIT: u64 = 64 << 10;

/// Ruler kinds the model does not hold yet: the `SpecialRulerManager` head column, its table,
/// and the name used in warnings.
const SKIPPED_KINDS: [(&str, &str, &str); 8] = [
    ("FirstParallel", "RulerParallel", "parallel line"),
    ("FirstCurveParallel", "RulerCurveParallel", "parallel curve"),
    ("FirstMultiCurve", "RulerMultiCurve", "multi curve"),
    ("FirstEmit", "RulerEmit", "radial line"),
    ("FirstCurveEmit", "RulerCurveEmit", "radial curve"),
    ("FirstConcentricCircle", "RulerConcentricCircle", "concentric circle"),
    ("FirstGuide", "RulerGuide", "guide"),
    ("FirstSymmetry", "RulerSymmetry", "symmetry"),
];

fn invalid() -> String {
    "invalid or unsupported CLIP STUDIO layer material".into()
}

/// One decoded layer material.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerMaterial {
    /// The material's canvas, in pixels.
    pub width: u32,
    pub height: u32,
    /// Leaf layers in CLIP STUDIO's sibling order (folders are flattened).
    pub layers: Vec<MaterialLayer>,
    /// What could not be carried over.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialLayer {
    pub name: String,
    pub visible: bool,
    /// 0..=1.
    pub opacity: f32,
    /// Where the pixels' top-left sits on the material canvas.
    pub offset: [i32; 2],
    /// `None` when the layer has no visible pixels.
    pub image: Option<MaterialImage>,
    /// In material canvas pixels.
    pub rulers: Option<SpecialRulers>,
}

/// Straight 8-bit RGBA pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl MaterialImage {
    /// The repeating tile of a tiling material. CLIP STUDIO's pixel blocks cover one pixel less
    /// than the stored size, so the last column and row read transparent; a tile keeps them
    /// out (the downloaded "Isometric grid" only joins up at 428 × 258 of its 429 × 259).
    /// Only a wholly transparent last column or row is dropped.
    pub fn tile(&self) -> MaterialImage {
        let (w, h) = (self.width as usize, self.height as usize);
        let alpha = |x: usize, y: usize| self.rgba.get((y * w + x) * 4 + 3).copied().unwrap_or(0);
        let tw = if w > 1 && (0..h).all(|y| alpha(w - 1, y) == 0) { w - 1 } else { w };
        let th = if h > 1 && (0..w).all(|x| alpha(x, h - 1) == 0) { h - 1 } else { h };
        let mut rgba = Vec::with_capacity(tw * th * 4);
        for y in 0..th {
            rgba.extend_from_slice(self.rgba.get(y * w * 4..(y * w + tw) * 4).unwrap_or(&[]));
        }
        if rgba.len() != tw * th * 4 {
            return self.clone();
        }
        MaterialImage { width: tw as u32, height: th as u32, rgba }
    }
}

/// What a material's `icedata/layerData.xml` says about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialInfo {
    /// CLIP STUDIO's system tags (`Ruler`, `PersRuler`, `Resizable`, …).
    pub tags: Vec<String>,
    /// The image repeats across the canvas when placed (`isTiling`).
    pub tiling: bool,
}

impl MaterialInfo {
    /// A material that carries a special ruler.
    pub fn ruler(&self) -> bool {
        self.tags.iter().any(|t| t == "SpecialRuler" || t == "Ruler")
    }
}

/// The text of every `<data>` in the `<datalist key="…">` element.
fn datalist<'a>(xml: &'a str, key: &str) -> Vec<&'a str> {
    let open = format!("<datalist key=\"{key}\">");
    let Some(start) = xml.find(&open) else { return Vec::new() };
    let rest = xml.get(start + open.len()..).unwrap_or("");
    let body = rest.find("</datalist>").and_then(|end| rest.get(..end)).unwrap_or("");
    body.split("<data>").skip(1).filter_map(|d| d.split("</data>").next()).map(str::trim).take(64).collect()
}

/// Read `icedata/layerData.xml`; unknown or missing keys keep their defaults.
pub fn parse_info(xml: &str) -> MaterialInfo {
    let tags = datalist(xml, "systemtag").into_iter().filter(|t| !t.is_empty() && t.len() <= 64).map(str::to_string).collect();
    MaterialInfo { tags, tiling: datalist(xml, "isTiling").first() == Some(&"1") }
}

fn int(row: &Row, key: &str) -> Option<i64> {
    row.get(key).and_then(Value::integer)
}
fn num(row: &Row, key: &str) -> Result<f64> {
    row.get(key).and_then(Value::number).filter(|v| v.is_finite()).ok_or_else(|| format!("CLIP STUDIO ruler value {key} is missing or not a number"))
}
fn point(row: &Row, x: &str, y: &str) -> Result<[f64; 2]> {
    Ok([num(row, x)?, num(row, y)?])
}

/// The rows of a `NextIndex` chain starting at `first` (0 = empty), following `MainId`.
fn chain(rows: &[Row], first: i64, cap: usize) -> Result<Vec<&Row>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut at = first;
    while at != 0 {
        if !seen.insert(at) || out.len() >= cap {
            return Err("CLIP STUDIO ruler chain loops or is too long".into());
        }
        let row = rows.iter().find(|r| int(r, "MainId") == Some(at)).ok_or("CLIP STUDIO ruler chain points to a missing row")?;
        out.push(row);
        at = int(row, "NextIndex").unwrap_or(0);
    }
    Ok(out)
}

/// `GuideNumber` records of `GuideDataSize` (16) bytes: a big-endian f64 (x, y) handle each.
fn guides(row: &Row) -> Result<Vec<[f64; 2]>> {
    let n = usize::try_from(int(row, "GuideNumber").unwrap_or(0)).map_err(|_| invalid())?;
    if n == 0 {
        return Ok(Vec::new());
    }
    if int(row, "GuideDataSize") != Some(16) || n > MAX_GUIDES {
        return Err("unsupported CLIP STUDIO vanishing-point guide data".into());
    }
    let blob = row.get("Guide").and_then(Value::blob).ok_or("CLIP STUDIO vanishing-point guides are missing")?;
    if blob.len() != n * 16 {
        return Err("CLIP STUDIO vanishing-point guide data has the wrong size".into());
    }
    blob.chunks_exact(16)
        .map(|g| {
            let f = |b: &[u8]| b.try_into().map(f64::from_be_bytes).map_err(|_| invalid());
            let p = [f(g.get(..8).ok_or_else(invalid)?)?, f(g.get(8..).ok_or_else(invalid)?)?];
            if p.iter().all(|v| v.is_finite()) { Ok(p) } else { Err("CLIP STUDIO vanishing-point guide is not a number".into()) }
        })
        .collect()
}

/// One `RulerPerspective` row and its vanishing points.
fn perspective(row: &Row, points: &[Row], layer_id: i64) -> Result<SpecialRuler> {
    let mut out = Vec::new();
    for vp in chain(points, int(row, "FirstVanishIndex").unwrap_or(0), MAX_VANISHING_POINTS)? {
        if int(vp, "LayerId") != Some(layer_id) {
            return Err(invalid());
        }
        let flag = int(vp, "Flag").unwrap_or(0);
        let handle = point(vp, "VanishPointX", "VanishPointY")?;
        // 0x10: the point was made parallel; its handle is far away and only the angle counts.
        let target = if flag & 0x10 != 0 { VanishTarget::Parallel { angle_deg: normalize_angle(num(vp, "ParallelAngle")?) } } else { VanishTarget::Point(handle) };
        out.push(VanishingPoint { snap: true, target, handle, guides: guides(vp)? });
    }
    if out.is_empty() {
        return Err("CLIP STUDIO perspective ruler has no vanishing points".into());
    }
    let eye_level = point(row, "EyeLevelHandleX", "EyeLevelHandleY").ok();
    let ruler = SpecialRuler { snap: true, kind: RulerKind::Perspective(PerspectiveRuler { points: out, eye_level }) };
    ruler.validate()?;
    Ok(ruler)
}

/// The special rulers of `layer`, or `None` when it has none. A ruler that cannot be read is
/// left out with a warning; the rest still import.
fn rulers(db: &Database<'_>, layer: &Row, name: &str, warnings: &mut Vec<String>) -> Option<SpecialRulers> {
    let manager_id = int(layer, "SpecialRulerManager").filter(|id| *id != 0)?;
    let layer_id = int(layer, "MainId")?;
    let manager = match db.table("SpecialRulerManager") {
        Ok(rows) => rows.into_iter().find(|r| int(r, "MainId") == Some(manager_id)),
        Err(e) => {
            warnings.push(format!("{name}: special rulers could not be read ({e})"));
            return None;
        }
    };
    let Some(manager) = manager else {
        warnings.push(format!("{name}: special rulers could not be read (missing manager)"));
        return None;
    };
    let mut set = SpecialRulers::default();
    let first = int(&manager, "FirstPerspective").unwrap_or(0);
    if first != 0 {
        match (db.table("RulerPerspective"), db.table("RulerVanishPoint")) {
            (Ok(rows), Ok(points)) => match chain(&rows, first, MAX_RULERS) {
                Ok(list) => {
                    for row in list {
                        match perspective(row, &points, layer_id) {
                            Ok(r) => set.rulers.push(r),
                            Err(e) => warnings.push(format!("{name}: a perspective ruler was skipped ({e})")),
                        }
                    }
                }
                Err(e) => warnings.push(format!("{name}: perspective rulers were skipped ({e})")),
            },
            (Err(e), _) | (_, Err(e)) => warnings.push(format!("{name}: perspective rulers were skipped ({e})")),
        }
    }
    for (head, table, kind) in SKIPPED_KINDS {
        let first = int(&manager, head).unwrap_or(0);
        if first == 0 {
            continue;
        }
        let n = db.table(table).ok().and_then(|rows| chain(&rows, first, MAX_RULERS).ok().map(|c| c.len())).unwrap_or(1);
        warnings.push(format!("{name}: {n} {kind} ruler(s) skipped; Prism does not read that kind yet"));
    }
    // LayerVisibility bit 2 (4): the layer's rulers are shown (inferred: set on every ruler layer).
    set.shown = int(layer, "LayerVisibility").is_none_or(|v| v & 4 != 0);
    match int(layer, "RulerRange").unwrap_or(0) {
        0 => {}
        other => warnings.push(format!("{name}: ruler show range {other} is not understood yet; the rulers apply to all layers")),
    }
    (!set.rulers.is_empty()).then_some(set)
}

/// Pixels of a decoded raster as straight RGBA, `None` when nothing is visible.
fn rgba(r: MaterialRaster) -> Option<MaterialImage> {
    if r.alpha.iter().all(|a| *a == 0) {
        return None;
    }
    let mut rgba = Vec::with_capacity(r.alpha.len().saturating_mul(4));
    for (i, a) in r.alpha.iter().enumerate() {
        let [cr, cg, cb] = match (&r.rgb, &r.gray) {
            (Some(rgb), _) => [rgb.get(i * 3).copied().unwrap_or(0), rgb.get(i * 3 + 1).copied().unwrap_or(0), rgb.get(i * 3 + 2).copied().unwrap_or(0)],
            (None, Some(gray)) => [gray.get(i).copied().unwrap_or(0); 3],
            // Alpha-only material: black.
            (None, None) => [0; 3],
        };
        rgba.extend_from_slice(&[cr, cg, cb, *a]);
    }
    Some(MaterialImage { width: r.width, height: r.height, rgba })
}

/// The leaf layers below `folder` in sibling order, depth first.
fn leaves<'a>(layers: &'a [Row], folder: &'a Row, depth: usize, seen: &mut BTreeSet<i64>, out: &mut Vec<&'a Row>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err("CLIP STUDIO layer material folders are nested too deep".into());
    }
    let mut at = int(folder, "LayerFirstChildIndex").unwrap_or(0);
    while at != 0 {
        if !seen.insert(at) {
            return Err("CLIP STUDIO layer material layers loop".into());
        }
        let row = layers.iter().find(|r| int(r, "MainId") == Some(at)).ok_or("CLIP STUDIO layer material points to a missing layer")?;
        if int(row, "LayerFolder").unwrap_or(0) != 0 {
            leaves(layers, row, depth + 1, seen, out)?;
        } else {
            if out.len() >= MAX_LAYERS {
                return Err(format!("CLIP STUDIO layer materials with more than {MAX_LAYERS} layers are unsupported"));
            }
            out.push(row);
        }
        at = int(row, "LayerNextIndex").unwrap_or(0);
    }
    Ok(())
}

/// Decode a `material_0.layer` file.
pub fn read(bytes: &[u8], ctl: &prism_raster::Interrupt) -> Result<LayerMaterial> {
    let full = crate::csp_container::decode(bytes, ctl)?;
    if !full.starts_with(b"SQLite format 3\0") {
        return Err(invalid());
    }
    let db = Database::new(&full)?;
    let canvas = db.table("Canvas")?.into_iter().next().ok_or_else(invalid)?;
    let size = |key: &str| int(&canvas, key).and_then(|v| u32::try_from(v).ok()).filter(|v| (1..=65_535).contains(v)).ok_or_else(invalid);
    let (width, height) = (size("CanvasWidth")?, size("CanvasHeight")?);
    let layers = db.table("Layer")?;
    let root = layers.iter().find(|r| int(r, "MainId") == int(&canvas, "CanvasRootFolder")).ok_or_else(invalid)?;
    let mut rows = Vec::new();
    leaves(&layers, root, 0, &mut BTreeSet::new(), &mut rows)?;
    let mut warnings = Vec::new();
    let mut out = Vec::new();
    for row in rows {
        ctl.check().map_err(|e| e.to_string())?;
        let name = row.get("LayerName").and_then(Value::text).map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Layer").chars().take(255).collect::<String>();
        // A resizable image keeps its original pixels apart from the rendered ones.
        let mipmap = int(row, "ResizableOriginalMipmap").filter(|m| *m != 0).or_else(|| int(row, "LayerRenderMipmap")).unwrap_or(0);
        let image = if mipmap == 0 {
            None
        } else {
            match mipmap_raster(&db, row, mipmap, MAX_LAYER_PIXELS) {
                Ok(r) => rgba(r),
                Err(e) => {
                    warnings.push(format!("{name}: pixels could not be read ({e})"));
                    None
                }
            }
        };
        let offset = |key: &str| int(row, key).and_then(|v| i32::try_from(v).ok()).unwrap_or(0);
        let opacity = int(row, "LayerOpacity").unwrap_or(256).clamp(0, 256) as f32 / 256.0;
        let rulers = rulers(&db, row, &name, &mut warnings);
        out.push(MaterialLayer {
            visible: int(row, "LayerVisibility").is_none_or(|v| v & 1 != 0),
            opacity,
            offset: [offset("LayerOffsetX"), offset("LayerOffsetY")],
            image,
            rulers,
            name,
        });
    }
    if out.is_empty() {
        return Err("the CLIP STUDIO layer material holds no layers".into());
    }
    Ok(LayerMaterial { width, height, layers: out, warnings })
}

#[cfg(test)]
#[path = "layer_material_tests.rs"]
mod tests;
