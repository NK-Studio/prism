//! Brush comparison harness: render the brush test cases in `cases.json` with Prism, generate a
//! Photoshop script that renders the same cases, and diff the two renders (docs/brush-compare.md).
//!
//! Photoshop is studied for behaviour only (AGENTS.md, clean-room): its renders are local
//! observation oracles under the output directory and are never committed. The Photoshop script is
//! generated here at run time so the repository stays Rust-only.
//!
//! ```sh
//! cargo xtask brush-compare jsx      # write <out>/photoshop.jsx; run it in Photoshop (File › Scripts › Browse…)
//! cargo xtask brush-compare render   # Prism renders into <out>/prism/
//! cargo xtask brush-compare diff     # compare <out>/ps/ with <out>/prism/: report.md, diff/, profiles/
//! cargo xtask brush-compare all      # render + diff
//! ```
//!
//! Options: `--out DIR` (default `target/brush-compare`), `--cases FILE` (default: the bundled
//! cases), `--only ID[,ID…]`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use prism_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use prism_engine::Session;
use serde::Deserialize;
use serde_json::{Value, json};

type R<T> = Result<T, String>;

const DEFAULT_CASES: &str = include_str!("cases.json");
const USAGE: &str = "usage: brush_compare <jsx|render|diff|all> [--out DIR] [--cases FILE] [--only ID[,ID...]]";

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// Units follow Photoshop's Brush Settings panel (whole pixels, degrees, whole percentages), so a
/// case means exactly the same thing to both renderers.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Canvas {
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Tool {
    #[default]
    Brush,
    Pencil,
}

/// The line a 1-D profile is sampled along.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Probe {
    Row(u32),
    Column(u32),
}

fn pct100() -> u32 {
    100
}
fn pct25() -> u32 {
    25
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    tool: Tool,
    canvas: Option<Canvas>,
    /// Diameter, px.
    size: u32,
    #[serde(default = "pct100")]
    hardness: u32,
    #[serde(default = "pct25")]
    spacing: u32,
    #[serde(default)]
    angle: i32,
    #[serde(default = "pct100")]
    roundness: u32,
    #[serde(default = "pct100")]
    opacity: u32,
    #[serde(default = "pct100")]
    flow: u32,
    /// Path anchors in document pixels (0,0 = top-left corner of the first pixel).
    points: Vec<[f64; 2]>,
    probe: Option<Probe>,
    /// A single dab centred on the first point: also report its radial edge (every pixel by
    /// distance from the centre), which a one-row probe samples at a single sub-pixel phase.
    #[serde(default)]
    radial: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    canvas: Canvas,
    cases: Vec<Case>,
}

impl Case {
    fn canvas(&self, suite: &Suite) -> Canvas {
        self.canvas.unwrap_or(suite.canvas)
    }

    fn probe(&self) -> Probe {
        // Default: the row through the first point.
        self.probe.unwrap_or(Probe::Row(self.points.first().map_or(0, |p| p[1].max(0.0) as u32)))
    }

    /// Reject anything the two renderers would not read the same way (Photoshop's whole-number
    /// fields and ranges).
    fn validate(&self, suite: &Suite) -> R<()> {
        let id = &self.id;
        let fail = |m: String| Err(format!("case `{id}`: {m}"));
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return fail("id must be non-empty [a-z0-9-]".into());
        }
        let c = self.canvas(suite);
        if !(1..=4000).contains(&c.width) || !(1..=4000).contains(&c.height) {
            return fail("canvas must be 1..=4000 px on each side".into());
        }
        let ranges: [(&str, i64, i64, i64); 7] = [
            ("size", self.size.into(), 1, 5000),
            ("hardness", self.hardness.into(), 0, 100),
            ("spacing", self.spacing.into(), 1, 1000),
            ("angle", self.angle.into(), -180, 180),
            ("roundness", self.roundness.into(), 1, 100),
            ("opacity", self.opacity.into(), 1, 100),
            ("flow", self.flow.into(), 1, 100),
        ];
        for (name, v, lo, hi) in ranges {
            if !(lo..=hi).contains(&v) {
                return fail(format!("{name} {v} is outside {lo}..={hi}"));
            }
        }
        if self.points.len() < 2 {
            return fail("needs at least two points (Photoshop strokes a path)".into());
        }
        if self
            .points
            .iter()
            .any(|p| !(p[0].is_finite() && p[1].is_finite() && p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= c.width.into() && p[1] <= c.height.into()))
        {
            return fail("points must lie inside the canvas".into());
        }
        let in_canvas = match self.probe() {
            Probe::Row(y) => y < c.height,
            Probe::Column(x) => x < c.width,
        };
        if !in_canvas {
            return fail("probe lies outside the canvas".into());
        }
        Ok(())
    }
}

fn load_suite(path: Option<&str>) -> R<Suite> {
    let text = match path {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?,
        None => DEFAULT_CASES.to_string(),
    };
    let suite: Suite = serde_json::from_str(&text).map_err(|e| format!("cases: {e}"))?;
    let mut seen = std::collections::HashSet::new();
    for c in &suite.cases {
        c.validate(&suite)?;
        if !seen.insert(c.id.as_str()) {
            return Err(format!("case `{}` is defined twice", c.id));
        }
    }
    Ok(suite)
}

// ---------------------------------------------------------------------------
// Prism renders
// ---------------------------------------------------------------------------

fn render_prism(case: &Case, suite: &Suite, out: &Path) -> R<()> {
    let c = case.canvas(suite);
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, p).map_err(|e| format!("case `{}`: {id}: {e}", case.id));
    run(&mut s, "file.new", json!({"width": c.width, "height": c.height, "mode": "rgb", "depth": 8, "background": "white"}))?;
    // Constant input: no pressure or tilt (Photoshop's Stroke Path without Simulate Pressure).
    let points: Vec<Value> = case.points.iter().map(|p| json!([p[0], p[1], 1.0])).collect();
    let brush = json!({
        "size": case.size,
        "hardness": f64::from(case.hardness) / 100.0,
        "spacing": f64::from(case.spacing) / 100.0,
        "angle": case.angle,
        "roundness": f64::from(case.roundness) / 100.0,
        "opacity": f64::from(case.opacity) / 100.0,
        "flow": f64::from(case.flow) / 100.0,
        "pressureSize": false,
        "pressureOpacity": false,
        "smoothing": {"amount": 0.0},
    });
    let cmd = match case.tool {
        Tool::Brush => "paint.stroke",
        Tool::Pencil => "paint.pencil",
    };
    run(&mut s, cmd, json!({"points": points, "brush": brush, "color": "#000000", "seed": 1}))?;
    let doc = s.active().map(|d| (*d.doc).clone()).ok_or("no document")?;
    let png = prism_io::export(&doc, "png", &Default::default()).map_err(|e| format!("case `{}`: export: {e}", case.id))?;
    std::fs::write(out.join(format!("{}.png", case.id)), &png.bytes).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Photoshop script
// ---------------------------------------------------------------------------

/// The cases as the script's data: Photoshop units, ready to apply.
fn jsx_cases(suite: &Suite, cases: &[&Case]) -> Value {
    Value::Array(
        cases
            .iter()
            .map(|c| {
                let cv = c.canvas(suite);
                json!({
                    "id": c.id, "width": cv.width, "height": cv.height,
                    "pencil": c.tool == Tool::Pencil,
                    "size": c.size, "hardness": c.hardness, "spacing": c.spacing, "angle": c.angle,
                    "roundness": c.roundness, "opacity": c.opacity, "flow": c.flow,
                    "points": c.points,
                })
            })
            .collect(),
    )
}

/// ExtendScript (ES3) that renders every case into `<out>/ps/<id>.png` and records the tool
/// options Photoshop actually used in `<out>/ps/<id>.json`, so a setting that didn't apply shows
/// up in the report instead of silently skewing the comparison.
fn jsx(suite: &Suite, cases: &[&Case], out: &Path) -> R<String> {
    let dir = std::path::absolute(out.join("ps")).map_err(|e| e.to_string())?;
    let dir = serde_json::to_string(&dir.to_string_lossy().replace('\\', "/")).map_err(|e| e.to_string())?;
    let data = serde_json::to_string(&jsx_cases(suite, cases)).map_err(|e| e.to_string())?;
    Ok(JSX_TEMPLATE.replace("__OUT_DIR__", &dir).replace("__CASES__", &data))
}

const JSX_TEMPLATE: &str = r##"// Generated by `cargo xtask brush-compare jsx` (Prism). Do not edit; regenerate instead.
// Renders each brush test case into a new document, saves it as PNG and records the tool options.
#target photoshop
(function () {
var OUT = __OUT_DIR__;
var CASES = __CASES__;
function c(s) { return charIDToTypeID(s); }
function s(n) { return stringIDToTypeID(n); }
function str(v) {
    if (v === null || v === undefined) return "null";
    if (typeof v === "number") return isFinite(v) ? String(v) : "null";
    if (typeof v === "boolean") return v ? "true" : "false";
    if (typeof v === "string") return '"' + v.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n").replace(/\r/g, "\\r") + '"';
    var i, out = [];
    if (v instanceof Array) { for (i = 0; i < v.length; i++) out.push(str(v[i])); return "[" + out.join(",") + "]"; }
    for (i in v) if (v.hasOwnProperty(i)) out.push(str(i) + ":" + str(v[i]));
    return "{" + out.join(",") + "}";
}
function keyName(id) { try { var n = typeIDToStringID(id); if (n) return n; } catch (e) {} try { return typeIDToCharID(id); } catch (e2) {} return String(id); }
function dumpValue(d, k, depth) {
    var t = d.getType(k);
    if (t === DescValueType.BOOLEANTYPE) return d.getBoolean(k);
    if (t === DescValueType.INTEGERTYPE) return d.getInteger(k);
    if (t === DescValueType.DOUBLETYPE) return d.getDouble(k);
    if (t === DescValueType.UNITDOUBLE) return { unit: keyName(d.getUnitDoubleType(k)), value: d.getUnitDoubleValue(k) };
    if (t === DescValueType.STRINGTYPE) return d.getString(k);
    if (t === DescValueType.ENUMERATEDTYPE) return keyName(d.getEnumerationValue(k));
    if (t === DescValueType.OBJECTTYPE) return depth > 0 ? dump(d.getObjectValue(k), depth - 1) : "<object>";
    if (t === DescValueType.LISTTYPE) return "<list>";
    return "<" + String(t) + ">";
}
function dump(d, depth) {
    var o = {}, i, k;
    for (i = 0; i < d.count; i++) { k = d.getKey(i); try { o[keyName(k)] = dumpValue(d, k, depth); } catch (e) { o[keyName(k)] = "<error " + e + ">"; } }
    return o;
}
function write(path, text) { var f = new File(path); f.encoding = "UTF-8"; f.open("w"); f.write(text); f.close(); }
function selectTool(id) { var r = new ActionReference(); r.putClass(s(id)); var d = new ActionDescriptor(); d.putReference(c("null"), r); executeAction(c("slct"), d, DialogModes.NO); }
function currentTool() {
    var r = new ActionReference(); r.putProperty(c("Prpr"), s("tool")); r.putEnumerated(c("capp"), c("Ordn"), c("Trgt"));
    return typeIDToStringID(executeActionGet(r).getEnumerationType(s("tool")));
}
function optionsDescriptor() {
    var r = new ActionReference(); r.putProperty(c("Prpr"), s("currentToolOptions")); r.putEnumerated(c("capp"), c("Ordn"), c("Trgt"));
    return executeActionGet(r).getObjectValue(s("currentToolOptions"));
}
function putOptions(tool, o) {
    var r = new ActionReference(); r.putClass(s(tool));
    var d = new ActionDescriptor(); d.putReference(c("null"), r);
    d.putObject(c("T   "), s("currentToolOptions"), o);
    executeAction(c("setd"), d, DialogModes.NO);
}
function putBrush(b) {
    var r = new ActionReference(); r.putEnumerated(c("Brsh"), c("Ordn"), c("Trgt"));
    var d = new ActionDescriptor(); d.putReference(c("null"), r);
    d.putObject(c("T   "), c("Brsh"), b);
    executeAction(c("setd"), d, DialogModes.NO);
}
// Every Brush Settings section and stroke option off: Photoshop keeps them in the tool state, so a
// section the user left on would otherwise leak into the render.
var OFF = ["useTipDynamics", "useScatter", "useTexture", "useColorDynamics", "usePaintDynamics", "useBrushPose",
    "wetEdges", "noise", "repeat", "smoothing", "usePressureOverridesSize", "usePressureOverridesOpacity"];
function setToolOptions(tool, k) {
    var o = new ActionDescriptor(), i;
    o.putInteger(s("opacity"), k.opacity);
    if (!k.pencil) o.putInteger(s("flow"), k.flow);
    o.putInteger(s("smooth"), 0);
    for (i = 0; i < OFF.length; i++) o.putBoolean(s(OFF[i]), false);
    var dual = new ActionDescriptor(); dual.putBoolean(s("useDualBrush"), false);
    o.putObject(s("dualBrush"), s("dualBrush"), dual);
    putOptions(tool, o);
}
// `diameter`, not `masterDiameter`: with the master diameter Photoshop 2026 ignores the hardness.
function setBrush(k) {
    var b = new ActionDescriptor();
    b.putUnitDouble(s("diameter"), c("#Pxl"), k.size);
    b.putUnitDouble(s("hardness"), c("#Prc"), k.hardness);
    b.putUnitDouble(s("angle"), c("#Ang"), k.angle);
    b.putUnitDouble(s("roundness"), c("#Prc"), k.roundness);
    b.putUnitDouble(s("spacing"), c("#Prc"), k.spacing);
    b.putBoolean(s("interfaceIconFrameDimmed"), true);
    b.putBoolean(s("flipX"), false);
    b.putBoolean(s("flipY"), false);
    putBrush(b);
}
function toolOptions() { return dump(optionsDescriptor(), 3); }
function strokeLine(doc, k) {
    var pts = [], i, p;
    for (i = 0; i < k.points.length; i++) {
        p = new PathPointInfo(); p.kind = PointKind.CORNERPOINT;
        p.anchor = [k.points[i][0], k.points[i][1]]; p.leftDirection = p.anchor; p.rightDirection = p.anchor;
        pts.push(p);
    }
    var sp = new SubPathInfo(); sp.operation = ShapeOperation.SHAPEXOR; sp.closed = false; sp.entireSubPath = pts;
    var path = doc.pathItems.add("brush-compare", [sp]);
    path.strokePath(k.pencil ? ToolType.PENCIL : ToolType.BRUSH, false);
    path.remove();
}
var units = app.preferences.rulerUnits, fg = app.foregroundColor, done = 0, failed = [];
// The user's tool and both tools' options, restored at the end.
var userTool = currentTool(), saved = {}, TOOLS = ["paintbrushTool", "pencilTool"], t;
for (t = 0; t < TOOLS.length; t++) { selectTool(TOOLS[t]); saved[TOOLS[t]] = optionsDescriptor(); }
app.preferences.rulerUnits = Units.PIXELS;
new Folder(OUT).create();
var black = new SolidColor(); black.rgb.red = 0; black.rgb.green = 0; black.rgb.blue = 0;
try {
    for (var n = 0; n < CASES.length; n++) {
        var k = CASES[n], doc = null, rec = { id: k.id };
        try {
            // 72 ppi so path points and pixels coincide.
            doc = app.documents.add(UnitValue(k.width, "px"), UnitValue(k.height, "px"), 72, "brush-compare " + k.id, NewDocumentMode.RGB, DocumentFill.WHITE, 1, BitsPerChannelType.EIGHT, "sRGB IEC61966-2.1");
            app.foregroundColor = black;
            var tool = k.pencil ? "pencilTool" : "paintbrushTool";
            selectTool(tool);
            setToolOptions(tool, k);
            setBrush(k);
            rec.toolOptions = toolOptions();
            strokeLine(doc, k);
            var o = new PNGSaveOptions(); o.compression = 0; o.interlaced = false;
            doc.saveAs(new File(OUT + "/" + k.id + ".png"), o, true, Extension.LOWERCASE);
            done++;
        } catch (e) {
            rec.error = String(e) + (e.line ? " (line " + e.line + ")" : "");
            failed.push(k.id);
        }
        write(OUT + "/" + k.id + ".json", str(rec));
        if (doc) doc.close(SaveOptions.DONOTSAVECHANGES);
    }
} finally {
    app.preferences.rulerUnits = units;
    app.foregroundColor = fg;
    for (t = 0; t < TOOLS.length; t++) {
        try { selectTool(TOOLS[t]); putOptions(TOOLS[t], saved[TOOLS[t]]); putBrush(saved[TOOLS[t]].getObjectValue(s("brush"))); } catch (e) {}
    }
    try { selectTool(userTool); } catch (e) {}
}
var summary = "brush-compare: " + done + "/" + CASES.length + " cases rendered into " + OUT + (failed.length ? "; failed: " + failed.join(", ") : "");
write(OUT + "/_summary.txt", summary);
return summary;
})();
"##;

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

/// Ink coverage per pixel (black on white: 0 = paper, 1 = full ink).
struct Coverage {
    width: usize,
    height: usize,
    v: Vec<f32>,
}

fn load_coverage(path: &Path) -> R<Option<Coverage>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let img = prism_codecs::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba_f32();
    // Composite over white first so a transparent pixel counts as paper.
    let v = rgba.chunks_exact(4).map(|p| 1.0 - (1.0 - p[3] + p[3] * (p[0] + p[1] + p[2]) / 3.0)).map(|c| c.clamp(0.0, 1.0)).collect();
    Ok(Some(Coverage { width: w as usize, height: h as usize, v }))
}

fn profile(c: &Coverage, probe: Probe) -> Vec<f32> {
    match probe {
        Probe::Row(y) => c.v.chunks(c.width).nth(y as usize).map(<[f32]>::to_vec).unwrap_or_default(),
        Probe::Column(x) => c.v.chunks(c.width).filter_map(|row| row.get(x as usize).copied()).collect(),
    }
}

/// Peak, full width at half the peak (linearly interpolated crossings) and centre of mass.
struct Shape {
    peak: f32,
    fwhm: f32,
    centre: f32,
}

fn shape(p: &[f32]) -> Shape {
    let peak = p.iter().copied().fold(0.0, f32::max);
    let mass: f32 = p.iter().sum();
    let centre = if mass > 0.0 { p.iter().enumerate().map(|(i, v)| (i as f32 + 0.5) * v).sum::<f32>() / mass } else { f32::NAN };
    let half = peak / 2.0;
    let (Some(first), Some(last)) = (p.iter().position(|v| *v >= half && peak > 0.0), p.iter().rposition(|v| *v >= half && peak > 0.0)) else {
        return Shape { peak, fwhm: 0.0, centre };
    };
    // Crossing between sample i-1 (below) and i (at or above), in pixel-centre coordinates.
    let cross = |below: f32, above: f32, at: f32| if above > below { at - (above - half) / (above - below) } else { at };
    let left = if first == 0 { 0.0 } else { cross(p[first - 1], p[first], first as f32 + 0.5) };
    let right = if last + 1 >= p.len() { p.len() as f32 } else { -cross(p[last + 1], p[last], -(last as f32 + 0.5)) };
    Shape { peak, fwhm: right - left, centre }
}

/// Radial bin width, px.
const RADIAL_BIN: f64 = 0.1;

/// Mean coverage by distance from `centre` in [`RADIAL_BIN`] bins out to `max_r` (NaN = no pixel).
fn radial(c: &Coverage, centre: [f64; 2], max_r: f64) -> Vec<f32> {
    let n = (max_r / RADIAL_BIN).ceil() as usize + 1;
    let (mut sum, mut count) = (vec![0.0f64; n], vec![0u32; n]);
    for (y, row) in c.v.chunks(c.width).enumerate() {
        for (x, v) in row.iter().enumerate() {
            let d = (x as f64 + 0.5 - centre[0]).hypot(y as f64 + 0.5 - centre[1]);
            if let (Some(s), Some(k)) = (sum.get_mut((d / RADIAL_BIN) as usize), count.get_mut((d / RADIAL_BIN) as usize)) {
                *s += f64::from(*v);
                *k += 1;
            }
        }
    }
    sum.iter().zip(&count).map(|(s, k)| if *k > 0 { (*s / f64::from(*k)) as f32 } else { f32::NAN }).collect()
}

/// Distance at which the radial mean first falls below `level`, walking out from the centre.
fn crossing(r: &[f32], level: f32) -> Option<f32> {
    let mut prev: Option<(f32, f32)> = None;
    for (i, v) in r.iter().enumerate() {
        if v.is_nan() {
            continue;
        }
        let d = (i as f32 + 0.5) * RADIAL_BIN as f32;
        if *v < level {
            return Some(match prev {
                Some((pd, pv)) if pv > *v => pd + (d - pd) * (pv - level) / (pv - *v),
                _ => d,
            });
        }
        prev = Some((d, *v));
    }
    None
}

/// A dab's edge: the 50 % radius and the 90 % → 10 % width.
struct Edge {
    r50: f32,
    width: f32,
}

fn edge(r: &[f32]) -> Option<Edge> {
    Some(Edge { r50: crossing(r, 0.5)?, width: crossing(r, 0.1)? - crossing(r, 0.9)? })
}

struct Row {
    id: String,
    note: String,
    /// Half the diameter, for radial cases.
    radius: Option<f32>,
    edge_ps: Option<Edge>,
    edge_prism: Option<Edge>,
    status: String,
    ink_ps: f64,
    ink_prism: f64,
    mae: f64,
    max: f32,
    differing: usize,
    ps: Option<Shape>,
    prism: Option<Shape>,
    /// Why the Photoshop render may not match the case (empty = comparable).
    problems: Vec<String>,
}

/// Tool-option switches (as Photoshop 2026 reports them) that must be off for a comparable render:
/// the Brush Settings sections and stroke options the generated script turns off.
const SECTION_KEYS: [&str; 12] = [
    "useTipDynamics",
    "useScatter",
    "useTexture",
    "useColorDynamics",
    "usePaintDynamics",
    "useBrushPose",
    "wetEdges",
    "noise",
    "repeat",
    "smoothing",
    "usePressureOverridesSize",
    "usePressureOverridesOpacity",
];

/// Check what Photoshop reported using against the case: the script's error, a section left on,
/// or a brush value that didn't apply. Empty when the render is comparable; a missing read-back
/// (no Photoshop render yet) is not a problem here.
fn readback_problems(case: &Case, path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return vec!["unreadable read-back".into()] };
    let mut problems = Vec::new();
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        problems.push(format!("script error: {e}"));
    }
    let Some(opts) = v.get("toolOptions").and_then(Value::as_object) else {
        problems.push("no tool options read back".into());
        return problems;
    };
    let on: Vec<&str> = SECTION_KEYS.iter().copied().filter(|k| opts.get(*k).and_then(Value::as_bool) == Some(true)).collect();
    if !on.is_empty() {
        problems.push(format!("left on: {}", on.join(", ")));
    }
    if opts.get("dualBrush").and_then(|d| d.get("useDualBrush")).and_then(Value::as_bool) == Some(true) {
        problems.push("left on: dualBrush".into());
    }
    let unit = |v: &Value| v.get("value").and_then(Value::as_f64).or_else(|| v.as_f64());
    let brush = opts.get("brush");
    let mut expect = vec![
        ("diameter", brush.and_then(|b| b.get("diameter")).and_then(unit), f64::from(case.size)),
        ("hardness", brush.and_then(|b| b.get("hardness")).and_then(unit), f64::from(case.hardness)),
        ("spacing", brush.and_then(|b| b.get("spacing")).and_then(unit), f64::from(case.spacing)),
        ("angle", brush.and_then(|b| b.get("angle")).and_then(unit), f64::from(case.angle)),
        ("roundness", brush.and_then(|b| b.get("roundness")).and_then(unit), f64::from(case.roundness)),
        ("opacity", opts.get("opacity").and_then(unit), f64::from(case.opacity)),
    ];
    if case.tool == Tool::Brush {
        expect.push(("flow", opts.get("flow").and_then(unit), f64::from(case.flow)));
    }
    for (name, got, want) in expect {
        match got {
            Some(g) if (g - want).abs() < 0.01 => {}
            Some(g) => problems.push(format!("{name} {g} (case {want})")),
            None => problems.push(format!("{name} not reported")),
        }
    }
    problems
}

/// Signed difference image: red where Prism lays more ink, blue where Photoshop does (×4), over
/// a faint copy of the Photoshop render.
fn diff_png(a: &Coverage, b: &Coverage) -> R<Vec<u8>> {
    let mut data = Vec::with_capacity(a.v.len() * 3);
    for (pa, pb) in a.v.iter().zip(&b.v) {
        let base = 255.0 - pa * 40.0;
        let d = ((pb - pa) * 4.0).clamp(-1.0, 1.0);
        let (r, g, bl) = if d > 0.0 { (base, base * (1.0 - d), base * (1.0 - d)) } else { (base * (1.0 + d), base * (1.0 + d), base) };
        data.extend([r as u8, g as u8, bl as u8]);
    }
    let img = Image::from_u8(a.width as u32, a.height as u32, ChannelLayout::Rgb, data).map_err(|e| e.to_string())?;
    prism_codecs::encode(&img, Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
}

fn diff_case(case: &Case, out: &Path) -> R<Row> {
    let mut row = Row {
        id: case.id.clone(),
        note: case.note.clone(),
        radius: case.radial.then(|| case.size as f32 / 2.0),
        edge_ps: None,
        edge_prism: None,
        status: String::new(),
        ink_ps: 0.0,
        ink_prism: 0.0,
        mae: 0.0,
        max: 0.0,
        differing: 0,
        ps: None,
        prism: None,
        problems: readback_problems(case, &out.join("ps").join(format!("{}.json", case.id))),
    };
    let ps = load_coverage(&out.join("ps").join(format!("{}.png", case.id)))?;
    let pr = load_coverage(&out.join("prism").join(format!("{}.png", case.id)))?;
    let probe = case.probe();
    let centre = case.points.first().copied().unwrap_or_default();
    let max_r = f64::from(case.size) / 2.0 + 4.0;
    let (rad_ps, rad_prism) =
        (ps.as_ref().filter(|_| case.radial).map(|c| radial(c, centre, max_r)), pr.as_ref().filter(|_| case.radial).map(|c| radial(c, centre, max_r)));
    row.edge_ps = rad_ps.as_deref().and_then(edge);
    row.edge_prism = rad_prism.as_deref().and_then(edge);
    if let (Some(a), Some(b)) = (&rad_ps, &rad_prism) {
        let mut csv = String::from("distance,photoshop,prism\n");
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            let _ = writeln!(csv, "{:.2},{x:.5},{y:.5}", (i as f64 + 0.5) * RADIAL_BIN);
        }
        std::fs::write(out.join("profiles").join(format!("{}.radial.csv", case.id)), csv).map_err(|e| e.to_string())?;
    }
    if let Some(c) = &ps {
        row.ink_ps = c.v.iter().map(|v| f64::from(*v)).sum();
        row.ps = Some(shape(&profile(c, probe)));
    }
    if let Some(c) = &pr {
        row.ink_prism = c.v.iter().map(|v| f64::from(*v)).sum();
        row.prism = Some(shape(&profile(c, probe)));
    }
    let (Some(a), Some(b)) = (&ps, &pr) else {
        row.status = match (&ps, &pr) {
            (None, None) => "no renders",
            (None, _) => "no Photoshop render",
            _ => "no Prism render",
        }
        .into();
        return Ok(row);
    };
    if (a.width, a.height) != (b.width, b.height) {
        row.status = format!("size differs: Photoshop {}×{}, Prism {}×{}", a.width, a.height, b.width, b.height);
        return Ok(row);
    }
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for (pa, pb) in a.v.iter().zip(&b.v) {
        if pa.max(*pb) <= 0.5 / 255.0 {
            continue;
        }
        let d = (pb - pa).abs();
        sum += f64::from(d);
        n += 1;
        row.max = row.max.max(d);
        if d > 2.0 / 255.0 {
            row.differing += 1;
        }
    }
    row.mae = if n > 0 { sum / n as f64 } else { 0.0 };
    row.status = if row.problems.is_empty() { "ok".into() } else { "**PS settings differ**".into() };
    std::fs::write(out.join("diff").join(format!("{}.png", case.id)), diff_png(a, b)?).map_err(|e| e.to_string())?;
    let (pa, pb) = (profile(a, probe), profile(b, probe));
    let mut csv = String::from("pos,photoshop,prism\n");
    for (i, (x, y)) in pa.iter().zip(&pb).enumerate() {
        let _ = writeln!(csv, "{i},{x:.5},{y:.5}");
    }
    std::fs::write(out.join("profiles").join(format!("{}.csv", case.id)), csv).map_err(|e| e.to_string())?;
    Ok(row)
}

fn report(rows: &[Row]) -> String {
    let f = |s: &Option<Shape>, g: fn(&Shape) -> f32| s.as_ref().map_or("–".to_string(), |s| format!("{:.2}", g(s)));
    let mut t = String::from(
        "# Brush comparison: Photoshop vs Prism\n\n\
         Coverage = ink on white (0..1). MAE and max are over pixels either render inked; *differing* counts |Δ| > 2/255.\n\
         Profile columns (peak, FWHM in px, centre of mass in px) are sampled along each case's probe line.\n\
         Diff images: red = Prism lays more ink, blue = Photoshop does (×4).\n\n\
         | case | status | ink PS | ink Prism | ratio | MAE | max | differing | peak PS/Prism | FWHM PS/Prism | Δcentre |\n\
         |---|---|---:|---:|---:|---:|---:|---:|---|---|---:|\n",
    );
    for r in rows {
        let ratio = if r.ink_ps > 0.0 { format!("{:.3}", r.ink_prism / r.ink_ps) } else { "–".into() };
        let dc = match (&r.ps, &r.prism) {
            (Some(a), Some(b)) if a.centre.is_finite() && b.centre.is_finite() => format!("{:+.2}", b.centre - a.centre),
            _ => "–".into(),
        };
        let _ = writeln!(
            t,
            "| `{}` | {} | {:.0} | {:.0} | {} | {:.4} | {:.3} | {} | {} / {} | {} / {} | {} |",
            r.id,
            r.status,
            r.ink_ps,
            r.ink_prism,
            ratio,
            r.mae,
            r.max,
            r.differing,
            f(&r.ps, |s| s.peak),
            f(&r.prism, |s| s.peak),
            f(&r.ps, |s| s.fwhm),
            f(&r.prism, |s| s.fwhm),
            dc
        );
    }
    let radial: Vec<&Row> = rows.iter().filter(|r| r.radius.is_some()).collect();
    if !radial.is_empty() {
        t.push_str(
            "\n## Single-dab edges (radial)\n\n\
             Mean coverage of every pixel by distance from the dab centre (0.1 px bins): the radius where it falls to 50 %, \
             and the 90 % → 10 % edge width.\n\n\
             | case | radius | r50 PS | r50 Prism | width PS | width Prism |\n|---|---:|---:|---:|---:|---:|\n",
        );
        let g = |e: &Option<Edge>, f: fn(&Edge) -> f32| e.as_ref().map_or("–".to_string(), |e| format!("{:.2}", f(e)));
        for r in radial {
            let _ = writeln!(
                t,
                "| `{}` | {:.1} | {} | {} | {} | {} |",
                r.id,
                r.radius.unwrap_or_default(),
                g(&r.edge_ps, |e| e.r50),
                g(&r.edge_prism, |e| e.r50),
                g(&r.edge_ps, |e| e.width),
                g(&r.edge_prism, |e| e.width)
            );
        }
    }
    let flagged: Vec<&Row> = rows.iter().filter(|r| !r.problems.is_empty()).collect();
    if !flagged.is_empty() {
        t.push_str("\n## Photoshop renders that don't match their case\n\nRe-run the script after fixing these; their numbers above are not comparable.\n\n");
        for r in flagged {
            let _ = writeln!(t, "- `{}`: {}", r.id, r.problems.join("; "));
        }
    }
    t.push_str("\n## Case notes\n\n");
    for r in rows.iter().filter(|r| !r.note.is_empty()) {
        let _ = writeln!(t, "- `{}`: {}", r.id, r.note);
    }
    t
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn mkdir(p: &Path) -> R<()> {
    std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> R<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = None;
    let (mut out, mut cases_file, mut only) = (PathBuf::from("target/brush-compare"), None, None::<Vec<String>>);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(it.next().ok_or("--out needs a directory")?),
            "--cases" => cases_file = Some(it.next().ok_or("--cases needs a file")?.clone()),
            "--only" => only = Some(it.next().ok_or("--only needs case ids")?.split(',').map(str::to_string).collect()),
            "jsx" | "render" | "diff" | "all" if mode.is_none() => mode = Some(a.clone()),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unexpected argument `{other}`\n{USAGE}")),
        }
    }
    let mode = mode.ok_or(USAGE)?;
    let suite = load_suite(cases_file.as_deref())?;
    let cases: Vec<&Case> = suite.cases.iter().filter(|c| only.as_ref().is_none_or(|o| o.contains(&c.id))).collect();
    if let Some(o) = &only
        && let Some(missing) = o.iter().find(|id| !suite.cases.iter().any(|c| &c.id == *id))
    {
        return Err(format!("--only: no case `{missing}`"));
    }
    mkdir(&out)?;
    if mode == "jsx" {
        let path = out.join("photoshop.jsx");
        std::fs::write(&path, jsx(&suite, &cases, &out)?).map_err(|e| e.to_string())?;
        println!("wrote {} ({} cases): run it in Photoshop (File > Scripts > Browse...)", path.display(), cases.len());
        return Ok(());
    }
    if mode == "render" || mode == "all" {
        let dir = out.join("prism");
        mkdir(&dir)?;
        for c in &cases {
            render_prism(c, &suite, &dir)?;
        }
        println!("rendered {} cases into {}", cases.len(), dir.display());
    }
    if mode == "diff" || mode == "all" {
        mkdir(&out.join("diff"))?;
        mkdir(&out.join("profiles"))?;
        let rows = cases.iter().map(|c| diff_case(c, &out)).collect::<R<Vec<_>>>()?;
        let path = out.join("report.md");
        std::fs::write(&path, report(&rows)).map_err(|e| e.to_string())?;
        let ok = rows.iter().filter(|r| r.status == "ok").count();
        println!("compared {ok}/{} cases: {}", rows.len(), path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_cases_are_valid() {
        let s = load_suite(None).unwrap();
        assert!(s.cases.len() >= 10);
    }

    #[test]
    fn readback_flags_sections_left_on_and_values_that_did_not_apply() {
        let suite = load_suite(None).unwrap();
        let case = suite.cases.iter().find(|c| c.id == "dab-h50").unwrap();
        let dir = std::env::temp_dir().join(format!("brush-compare-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rb.json");
        let unit = |v: f64| json!({"unit": "percentUnit", "value": v});
        let mut rb = json!({"id": "dab-h50", "toolOptions": {
            "opacity": 100, "flow": 100, "useScatter": false, "smoothing": false, "dualBrush": {"useDualBrush": false},
            "brush": {"diameter": unit(200.0), "hardness": unit(50.0), "spacing": unit(25.0), "angle": unit(0.0), "roundness": unit(100.0)}
        }});
        std::fs::write(&path, rb.to_string()).unwrap();
        assert!(readback_problems(case, &path).is_empty());
        rb["toolOptions"]["useScatter"] = json!(true);
        rb["toolOptions"]["brush"]["hardness"] = unit(0.0);
        std::fs::write(&path, rb.to_string()).unwrap();
        let p = readback_problems(case, &path);
        assert!(p.iter().any(|s| s.contains("useScatter")) && p.iter().any(|s| s.starts_with("hardness 0")), "{p:?}");
        assert!(readback_problems(case, &dir.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn half_max_width_of_a_box() {
        let s = shape(&[0.0, 0.0, 1.0, 1.0, 1.0, 0.0]);
        assert!((s.fwhm - 3.0).abs() < 1e-6, "{}", s.fwhm);
        assert!((s.centre - 3.5).abs() < 1e-6);
    }
}
