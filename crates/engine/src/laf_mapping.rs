//! CLIP STUDIO auto-action steps (`actionItemKind` + `actionItemParameter`) onto Prism
//! commands. A step either maps completely or returns why it cannot; nothing is guessed from an
//! unknown parameter. Evidence for each value: docs/sut-analysis.md › Auto-action step mapping.
use prism_paint::csp_blend::{CspBlend, csp_blend};
use serde_json::{Map, Value, json};

/// Prism commands for one CLIP STUDIO step, and what differs from CLIP STUDIO when it does.
#[derive(Debug, PartialEq)]
pub(crate) struct Mapped {
    pub steps: Vec<(String, Value)>,
    pub note: Option<String>,
}

type Params = Map<String, Value>;
type R<T> = std::result::Result<T, String>;

fn one(id: &str, params: Value) -> R<Mapped> {
    Ok(Mapped { steps: vec![(id.into(), params)], note: None })
}

fn noted(id: &str, params: Value, note: impl Into<String>) -> R<Mapped> {
    Ok(Mapped { steps: vec![(id.into(), params)], note: Some(note.into()) })
}

/// Reject parameters this mapping does not read: an unknown one may change the result.
fn only(p: &Params, keys: &[&str]) -> R<()> {
    match p.keys().find(|k| k.as_str() != "typename" && !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown parameter `{k}`")),
        None => Ok(()),
    }
}

fn int(p: &Params, key: &str) -> R<i64> {
    p.get(key).and_then(Value::as_i64).ok_or_else(|| format!("missing or invalid `{key}`"))
}

/// An optional integer that must hold `want` (or be absent) for the mapping to apply.
fn expect(p: &Params, key: &str, want: i64) -> R<()> {
    match p.get(key) {
        None => Ok(()),
        Some(v) if v.as_i64() == Some(want) => Ok(()),
        Some(v) => Err(format!("`{key}` {v} is not supported")),
    }
}

fn flag(p: &Params, key: &str) -> R<bool> {
    match int(p, key)? {
        0 => Ok(false),
        1 => Ok(true),
        v => Err(format!("`{key}` {v} is not supported")),
    }
}

fn number(v: Option<&Value>, what: &str) -> R<f64> {
    v.and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| format!("missing or invalid `{what}`"))
}

fn text<'a>(p: &'a Params, key: &str) -> R<&'a str> {
    p.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty() && s.chars().count() <= 255).ok_or_else(|| format!("missing or invalid `{key}`"))
}

/// A `length` value in pixels (unit 0). Other units depend on the document resolution.
fn pixels(v: Option<&Value>, what: &str) -> R<f64> {
    let v = v.and_then(Value::as_object).filter(|o| o.get("typename").and_then(Value::as_str) == Some("length")).ok_or_else(|| format!("missing `{what}`"))?;
    if v.get("unit").and_then(Value::as_i64) != Some(0) {
        return Err(format!("`{what}` is not in pixels"));
    }
    let n = number(v.get("value"), what)?;
    if n.abs() > 1e6 { Err(format!("`{what}` is out of range")) } else { Ok(n) }
}

/// One colour channel: a 32-bit value where all ones is full intensity. Stored signed, so −1
/// is all ones (white is `-1, -1, -1`, 0.8 gray `-858993460` = 0xCCCCCCCC).
fn channel(v: Option<&Value>) -> R<f64> {
    let v = v.and_then(Value::as_i64).ok_or("invalid colour channel")?;
    Ok(v.rem_euclid(1 << 32) as f64 / f64::from(u32::MAX))
}

fn rgb(v: Option<&Value>, what: &str) -> R<[f64; 3]> {
    let o = v.and_then(Value::as_object).filter(|o| o.get("typename").and_then(Value::as_str) == Some("rgbcolor")).ok_or_else(|| format!("missing `{what}`"))?;
    Ok([channel(o.get("red"))?, channel(o.get("green"))?, channel(o.get("blue"))?])
}

fn hex(c: [f64; 3]) -> String {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

/// A CLIP STUDIO blending mode code (the brush and layer codes are one list) as a
/// `layer.setProps` / `edit.fill` mode, and a note when Prism's formula differs.
fn blend(code: i64, group: bool) -> R<(String, Option<String>)> {
    let Some((label, run)) = csp_blend(code as f64) else { return Err(format!("unknown blending mode {code}")) };
    match run {
        CspBlend::Mode { mode, exact } => Ok((mode.label().into(), (!exact).then(|| format!("blending mode {label} is approximated by {}", mode.label())))),
        _ if group && label == "Through" => Ok(("Pass Through".into(), None)),
        _ => Err(format!("blending mode {label} has no layer equivalent")),
    }
}

/// A tone curve channel (`[{x, y}]` in 0..65535) as Curves points in 0..255.
fn curve(v: Option<&Value>, what: &str) -> R<Value> {
    let points = v.and_then(Value::as_array).filter(|a| (2..=19).contains(&a.len())).ok_or_else(|| format!("invalid `{what}`"))?;
    let scale = 255.0 / 65535.0;
    let mut out = Vec::with_capacity(points.len());
    for p in points {
        let x = number(p.get("x"), what)?;
        let y = number(p.get("y"), what)?;
        if !(0.0..=65535.0).contains(&x) || !(0.0..=65535.0).contains(&y) {
            return Err(format!("`{what}` is out of range"));
        }
        out.push(json!([x * scale, y * scale]));
    }
    Ok(Value::Array(out))
}

/// Tone curve channels: CLIP STUDIO's composite (`channelmaster`) and per-channel curves.
fn curves(p: &Params) -> R<Value> {
    only(p, &["channelmaster", "channelred", "channelgreen", "channelblue"])?;
    let mut out = json!({});
    for (key, name) in [("channelmaster", "points"), ("channelred", "red"), ("channelgreen", "green"), ("channelblue", "blue")] {
        if p.contains_key(key) {
            out[name] = curve(p.get(key), key)?;
        }
    }
    Ok(out)
}

/// Hue / Saturation / Luminosity: hue −180..180, saturation and luminosity −100..100.
fn hsv(p: &Params) -> R<Value> {
    only(p, &["hue", "saturation", "value"])?;
    let get = |k: &str, lim: i64| int(p, k).and_then(|v| if v.abs() <= lim { Ok(v) } else { Err(format!("`{k}` is out of range")) });
    Ok(json!({"hue": get("hue", 180)?, "saturation": get("saturation", 100)?, "lightness": get("value", 100)?}))
}

/// One Level Correction channel: input `low`/`mid`/`high` and output `low`/`high`, 0..65535 as in
/// the tone curve (a normal-map action multiplies two maps, then sets the input high to 32638,
/// 127/255, to bring the neutral 0.25 back to 0.5). `mid` is the input that maps to the middle of
/// the output; as a Photoshop gamma, `ln r / ln 0.5` with `r` its place between low and high
/// (inferred; exact at the middle, where every local action has it).
fn level(v: Option<&Value>, what: &str) -> R<(Value, bool)> {
    let c = v.and_then(Value::as_object).ok_or_else(|| format!("invalid `{what}`"))?;
    only(c, &["input", "output"])?;
    let part = |k: &str, keys: &[&str]| -> R<Vec<f64>> {
        let o = c.get(k).and_then(Value::as_object).ok_or_else(|| format!("invalid `{what}` {k}"))?;
        only(o, keys)?;
        keys.iter().map(|key| int(o, key).and_then(|v| if (0..=65535).contains(&v) { Ok(v as f64) } else { Err(format!("`{what}` {k} {key} is out of range")) })).collect()
    };
    let (input, output) = (part("input", &["low", "mid", "high"])?, part("output", &["low", "high"])?);
    let ([low, mid, high], [out_low, out_high]) = (
        <[f64; 3]>::try_from(input).map_err(|_| format!("invalid `{what}` input"))?,
        <[f64; 2]>::try_from(output).map_err(|_| format!("invalid `{what}` output"))?,
    );
    // Prism's input black and white stay two levels apart.
    if high - low < 2.0 * 257.0 || !(low < mid && mid < high) {
        return Err(format!("`{what}` input levels are not supported"));
    }
    let r = (mid - low) / (high - low);
    // Stored middles sit up to one unit low (16318 for 0..32638).
    let exact = (mid - (low + high) / 2.0).abs() <= 1.5;
    let gamma = if exact { 1.0 } else { (r.ln() / 0.5f64.ln()).clamp(0.01, 9.99) };
    let level = |v: f64| (v * 255.0 / 65535.0 * 100.0).round() / 100.0;
    Ok((json!({"inBlack": level(low), "inWhite": level(high), "gamma": (gamma * 1000.0).round() / 1000.0, "outBlack": level(out_low), "outWhite": level(out_high)}), exact))
}

/// Level Correction: the composite (`channelmaster`) and red, green, blue, every channel written
/// (identity where the step has none) so a settings step replaces the whole correction.
fn levels(p: &Params) -> R<(Value, Option<String>)> {
    only(p, &["channelmaster", "channelred", "channelgreen", "channelblue"])?;
    let identity = json!({"inBlack": 0.0, "inWhite": 255.0, "gamma": 1.0, "outBlack": 0.0, "outWhite": 255.0});
    let mut inferred = false;
    let mut channel = |key: &str| -> R<Value> {
        match p.get(key) {
            None => Ok(identity.clone()),
            v => {
                let (v, exact) = level(v, key)?;
                inferred |= !exact;
                Ok(v)
            }
        }
    };
    let mut out = channel("channelmaster")?;
    for (key, name) in [("channelred", "red"), ("channelgreen", "green"), ("channelblue", "blue")] {
        out[name] = channel(key)?;
    }
    Ok((out, inferred.then(|| "CLIP STUDIO's midtone input is converted to a gamma (inferred)".to_string())))
}

fn level_step(id: &str, p: &Params) -> R<Mapped> {
    only(p, &["adjustment"])?;
    let a = p.get("adjustment").and_then(Value::as_object).ok_or("missing `adjustment`")?;
    let (q, note) = levels(a)?;
    Ok(Mapped { steps: vec![(id.into(), q)], note })
}

/// A Gradient Map layer's `gradation` bytes as Prism stops. Stops that follow the drawing colours,
/// are not fully opaque or curve between colours have no Gradient Map equivalent and are refused.
fn gradient_map(p: &Params) -> R<Value> {
    use prism_paint::csp_gradient::{CspGradient, CspStopColor};
    only(p, &["gradation"])?;
    let hexed = p.get("gradation").and_then(|v| v.get(prism_io::laf::BYTES_KEY)).and_then(Value::as_str).ok_or("missing `gradation`")?;
    if hexed.len() > 64 * 1024 || hexed.len() % 2 != 0 {
        return Err("invalid `gradation`".into());
    }
    let bytes: Option<Vec<u8>> = (0..hexed.len()).step_by(2).map(|i| hexed.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
    let g = CspGradient::decode(&bytes.ok_or("invalid `gradation`")?).map_err(|e| format!("`gradation`: {e}"))?;
    let stops = g.prism_stops();
    if stops.approximated || stops.opacity.iter().any(|(_, a)| *a < 1.0) {
        return Err("gradient map stops with transparency are not supported".into());
    }
    let colors = stops
        .colors
        .iter()
        .map(|(t, c)| match c {
            CspStopColor::Rgb(c) => Ok(json!([t, c])),
            _ => Err("gradient map stops that follow the drawing colours are not supported".to_string()),
        })
        .collect::<R<Vec<Value>>>()?;
    if !(2..=64).contains(&colors.len()) {
        return Err("the gradient map has too many stops".into());
    }
    Ok(json!({"stops": colors}))
}

const HSV_NOTE: &str = "CLIP STUDIO's Luminosity is approximated by Photoshop's Lightness";

/// The canvas anchor of a 1-based reference point (1 top left … 5 centre … 9 bottom right).
fn anchor(point: i64) -> R<&'static str> {
    const ANCHORS: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];
    usize::try_from(point - 1).ok().and_then(|i| ANCHORS.get(i).copied()).ok_or_else(|| format!("reference point {point} is not supported"))
}

/// The transform reference point of a 0-based position (0 top left … 4 centre … 8 bottom right).
fn reference(position: i64) -> R<&'static str> {
    anchor(position + 1)
}

pub(crate) fn map(kind: &str, p: &Params) -> R<Mapped> {
    match kind {
        // ---- selection ----
        "selectactionselectall" => only(p, &[]).and_then(|_| one("select.all", json!({}))),
        // Pixel Perfect Outline deselects before anything is selected: nothing to do then.
        "selectactiondeselect" => only(p, &[]).and_then(|_| one("select.deselect", json!({"ifAny": true}))),
        "selectactionexpand" | "selectactionreduce" => {
            let (id, size, edge) = if kind == "selectactionexpand" { ("select.modify.expand", "sizelength", None) } else { ("select.modify.contract", "sizelength", Some("reduceatpageedge")) };
            only(p, &["expandtype", "reducetype", "sizelength", "reduceatpageedge"])?;
            let radius = pixels(p.get(size), size)?;
            if !(1.0..=500.0).contains(&radius) {
                return Err(format!("radius {radius} px is out of range"));
            }
            let mut q = json!({"radius": radius.round()});
            if let Some(edge) = edge {
                q["applyAtCanvasBounds"] = json!(flag(p, edge)?);
            }
            let shape = p.get("expandtype").or_else(|| p.get("reducetype")).and_then(Value::as_i64).unwrap_or(0);
            if shape == 0 { one(id, q) } else { noted(id, q, "CLIP STUDIO's square expand or shrink shape is approximated by a round one") }
        }
        "layeractionselectfromlayerstatusnew" | "layeractionselectfromlayerstatusmultiply" => {
            only(p, &["referselected", "layername"])?;
            if int(p, "referselected")? != 0 {
                return Err("loading transparency from several selected layers is not supported".into());
            }
            let operation = if kind.ends_with("new") { "new" } else { "intersect" };
            match p.get("layername") {
                None => one("select.loadSelection", json!({"channel": "transparency", "operation": operation})),
                Some(_) => one("select.loadSelection", json!({"channel": "transparency", "layerName": text(p, "layername")?, "operation": operation})),
            }
        }

        // ---- edit ----
        "editactioncopy" => only(p, &[]).and_then(|_| one("edit.copy", json!({}))),
        "editactionpaste" => only(p, &[]).and_then(|_| one("edit.paste", json!({}))),
        "editactioncut" => {
            only(p, &["cutlayer"])?;
            expect(p, "cutlayer", 0)?;
            one("edit.cut", json!({}))
        }
        "editactionclearoutofselectarea" => only(p, &[]).and_then(|_| one("edit.clearOutsideSelection", json!({}))),
        "editactionbrightnesstoalpha" => only(p, &[]).and_then(|_| one("edit.convertBrightnessToOpacity", json!({}))),
        "editactionpaint" => only(p, &[]).and_then(|_| one("edit.fillForeground", json!({}))),
        "editactiondetailpaint" => {
            only(p, &[
                "antialias", "colormargin", "compositemode", "expandsize", "isrefermultilayer", "isshrink", "opacity", "paintexpandtype", "rgbcolor", "targetcolor", "useflame",
                "vectorcenterline",
            ])?;
            for k in ["isrefermultilayer", "isshrink", "useflame", "vectorcenterline"] {
                expect(p, k, 0)?;
            }
            if pixels(p.get("expandsize"), "expandsize")? != 0.0 {
                return Err("filling with an expanded area is not supported".into());
            }
            let opacity = int(p, "opacity")?;
            if !(0..=100).contains(&opacity) {
                return Err("`opacity` is out of range".into());
            }
            let (mode, note) = blend(int(p, "compositemode")?, false)?;
            let [r, g, b] = rgb(p.get("rgbcolor"), "rgbcolor")?;
            let q = json!({"contents": "color", "color": [r, g, b, 1.0], "opacity": opacity, "mode": mode});
            let fill = "CLIP STUDIO fills the whole area; its colour-margin and target-colour options do not apply";
            noted("edit.fill", q, note.map_or_else(|| fill.to_string(), |n| format!("{fill}; {n}")))
        }
        "editactionchangecanvassize" => {
            only(p, &["basepoint", "width", "height"])?;
            let size = |k: &str| pixels(p.get(k), k).and_then(|v| if (1.0..=300_000.0).contains(&v) { Ok(v.round()) } else { Err(format!("`{k}` is out of range")) });
            one("image.canvasSize", json!({"width": size("width")?, "height": size("height")?, "anchor": anchor(int(p, "basepoint")?)?, "extensionColor": "transparent"}))
        }
        "editactiontransformstretch" => {
            only(p, &[
                "anchortype", "changevectorwidth", "fixaspect", "move", "positiontype", "quality", "remainoriginal", "rotation", "scalex", "scaley", "transformoperation", "transformtype",
            ])?;
            expect(p, "transformoperation", 1)?;
            expect(p, "anchortype", 0)?;
            // 0 and 1 are the tool property's scale and rotate modes; both store an affine
            // scale-then-rotate. Other modes (free, distort, mesh) are refused.
            if !(0..=1).contains(&int(p, "transformtype")?) {
                return Err("only scaling and rotating transforms are supported".into());
            }
            // Positive is clockwise on screen, as in Prism (observed in CLIP STUDIO, 2026-11-15).
            let angle = match p.get("rotation") {
                None => 0.0,
                v => number(v, "rotation").and_then(|v| if v.abs() <= 360.0 { Ok(v) } else { Err("`rotation` is out of range".into()) })?,
            };
            let scale = |k: &str| match p.get(k) {
                None => Ok(100.0),
                v => number(v, k).and_then(|v| if v > 0.0 && v <= 10_000.0 { Ok(v) } else { Err(format!("`{k}` is out of range")) }),
            };
            let (dx, dy) = match p.get("move").and_then(Value::as_object) {
                Some(m) => (pixels(m.get("x"), "move x")?, pixels(m.get("y"), "move y")?),
                None if p.contains_key("move") => return Err("invalid `move`".into()),
                None => (0.0, 0.0),
            };
            let interpolation = if p.get("quality").and_then(Value::as_i64) == Some(1) { "nearest" } else { "bicubic" };
            let keep = p.get("remainoriginal").map(|_| flag(p, "remainoriginal")).transpose()?.unwrap_or(false);
            let q = json!({
                "scaleX": scale("scalex")?, "scaleY": scale("scaley")?, "angle": angle, "dx": dx, "dy": dy,
                "reference": reference(int(p, "positiontype")?)?, "interpolation": interpolation, "keepOriginal": keep,
            });
            noted("edit.transform.numeric", q, "CLIP STUDIO's interpolation quality is approximated")
        }
        "toolactionmovelayer" => {
            only(p, &["inselectarea", "movegrid", "movelayerx", "movelayery", "movetarget", "remainoriginal", "selectlayer"])?;
            for k in ["inselectarea", "movegrid", "movetarget", "remainoriginal", "selectlayer"] {
                expect(p, k, 0)?;
            }
            let (dx, dy) = (pixels(p.get("movelayerx"), "movelayerx")?, pixels(p.get("movelayery"), "movelayery")?);
            // A recorded drag stores fractions (511.87 for 512); pixel layers move whole pixels.
            let q = json!({"dx": dx.round() as i64, "dy": dy.round() as i64});
            if dx.fract() == 0.0 && dy.fract() == 0.0 { one("layer.translate", q) } else { noted("layer.translate", q, "layer moves are rounded to whole pixels") }
        }

        // ---- adjustments on the layer ----
        "adjustmentactioninvert" => only(p, &[]).and_then(|_| one("image.adjustments.invert", json!({}))),
        "adjustmentactionhsv" => noted("image.adjustments.hueSaturation", hsv(p)?, HSV_NOTE),
        "adjustmentactionToneCurve" => one("image.adjustments.curves", curves(p)?),
        "adjustmentactionposterize" => {
            only(p, &["steps"])?;
            let levels = int(p, "steps")?;
            if !(2..=255).contains(&levels) {
                return Err("`steps` is out of range".into());
            }
            one("image.adjustments.posterize", json!({"levels": levels}))
        }
        "adjustmentactionbrightnessandcontrast" => {
            only(p, &["brightness", "contrast"])?;
            let (b, c) = (int(p, "brightness")?, int(p, "contrast")?);
            if b.abs() > 100 || c.abs() > 100 {
                return Err("brightness or contrast is out of range".into());
            }
            noted("image.adjustments.brightnessContrast", json!({"brightness": b, "contrast": c.clamp(-50, 100)}), "CLIP STUDIO's Brightness/Contrast scale is approximated")
        }
        "adjustmentactionLevel" if p.keys().all(|k| k == "typename") => one("image.adjustments.levels", json!({})),
        "layeractionlevelnew" => level_step("layer.newAdjustmentLayer.levels", p),
        // Changes the current Level Correction layer: every channel, as its dialog sets them.
        "layeractionlevelsettings" => level_step("layer.setAdjustment", p),
        // Layer › Convert Layer. `layerkind` 0 is raster (the CMY/K separation actions run tone
        // curves on the result) and 2 image material ("画像素材化", "make an image material";
        // the misregistration set tiles them by hand), which becomes a smart object. The colour
        // type every colour action stores ([0,1,1], channel order 33) is the only one accepted.
        "layeractionconvert" => {
            only(p, &["addnumbertoname", "channelorder", "colortype", "compositemode", "copytimeline", "layerkind", "layername", "leaveoriginal"])?;
            expect(p, "copytimeline", 0)?;
            let color = p.get("colortype").and_then(|v| v.get(prism_io::laf::BYTES_KEY)).and_then(Value::as_str);
            if color != Some("000000000000000100000001") || int(p, "channelorder")? != 33 {
                return Err("only conversions that keep colour are supported".into());
            }
            let (kind, mut note) = match int(p, "layerkind")? {
                0 => ("pixel", None),
                2 => ("smartObject", Some("CLIP STUDIO's image material layer becomes a smart object (no tiling)".to_string())),
                k => return Err(format!("conversion to layer kind {k} is not supported")),
            };
            let (mode, blend_note) = blend(int(p, "compositemode")?, false)?;
            let mut q = json!({"kind": kind, "keepOriginal": flag(p, "leaveoriginal")?, "blend": mode});
            match p.get("layername") {
                Some(_) => q["name"] = json!(text(p, "layername")?),
                None if p.get("addnumbertoname").map(|_| flag(p, "addnumbertoname")).transpose()?.unwrap_or(false) => {
                    let n = "the converted copy keeps Prism's copy name, not CLIP STUDIO's numbered name";
                    note = Some(note.map_or_else(|| n.to_string(), |m| format!("{m}; {n}")));
                }
                None => {}
            }
            if let Some(b) = blend_note {
                note = Some(note.map_or(b.clone(), |m| format!("{m}; {b}")));
            }
            Ok(Mapped { steps: vec![("layer.convert".into(), q)], note })
        }
        // Layer Color recolours how a layer shows; Prism layers have none, so turning it off is
        // already true and the step has nothing to do. Turning it on is refused.
        "layeractionsetuselayercolor" => {
            only(p, &["use"])?;
            if flag(p, "use")? {
                return Err("Layer Color is not supported".into());
            }
            Ok(Mapped { steps: Vec::new(), note: Some("turning Layer Color off is skipped: Prism layers have no layer colour".into()) })
        }
        "layeractiongradientmapnew" => {
            only(p, &["adjustment"])?;
            let a = p.get("adjustment").and_then(Value::as_object).ok_or("missing `adjustment`")?;
            one("layer.newAdjustmentLayer.gradientMap", gradient_map(a)?)
        }

        // ---- new layers ----
        // With a folder selected CLIP STUDIO creates the layer inside it, at the top: the CMY/K
        // separation actions make a folder, add a layer, then select the layer below it inside.
        "layeractionrasternew" => only(p, &[]).and_then(|_| one("layer.new.layer", json!({"into": true}))),
        "layeractionfoldernew" => only(p, &[]).and_then(|_| one("layer.new.group", json!({"into": true}))),
        "layeractionfoldernewandinsert" => {
            only(p, &["duplicate"])?;
            expect(p, "duplicate", 0)?;
            one("layer.groupLayers", json!({}))
        }
        "layeractioncopy" => only(p, &[]).and_then(|_| one("layer.duplicate", json!({}))),
        "layeractionmonochromefillnew" => {
            only(p, &["fillcolor"])?;
            one("layer.newFillLayer.solidColor", json!({"color": hex(rgb(p.get("fillcolor"), "fillcolor")?)}))
        }
        "layeractionhsvnew" => {
            only(p, &["adjustment"])?;
            let a = p.get("adjustment").and_then(Value::as_object).ok_or("missing `adjustment`")?;
            noted("layer.newAdjustmentLayer.hueSaturation", hsv(a)?, HSV_NOTE)
        }
        "layeractiontonecurvenew" => {
            only(p, &["adjustment"])?;
            let a = p.get("adjustment").and_then(Value::as_object).ok_or("missing `adjustment`")?;
            one("layer.newAdjustmentLayer.curves", curves(a)?)
        }

        // ---- layer order, merging, deleting ----
        "layeractionmergelowerlayer" => {
            only(p, &["keepmask"])?;
            expect(p, "keepmask", 0)?;
            one("layer.mergeDown", json!({}))
        }
        "layeractionmergeselectedlayer" => only(p, &[]).and_then(|_| one("layer.mergeLayers", json!({}))),
        "layeractionmergevisiblelayercopy" => only(p, &[]).and_then(|_| one("layer.stampVisible", json!({}))),
        "layeractionmergealllayer" => only(p, &[]).and_then(|_| one("layer.flattenImage", json!({}))),
        "layeractiondelete" => only(p, &[]).and_then(|_| one("layer.delete", json!({}))),
        "layeractiondeletemask" => only(p, &[]).and_then(|_| one("layer.layerMask.delete", json!({}))),
        "layeractionorderup" => only(p, &[]).and_then(|_| one("layer.arrange.bringForward", json!({}))),
        "layeractionorderdown" => only(p, &[]).and_then(|_| one("layer.arrange.sendBackward", json!({}))),

        // ---- layer selection ----
        "layeractionselectupperlayer" => only(p, &[]).and_then(|_| one("layer.selectAbove", json!({}))),
        "layeractionselectlowerlayer" => only(p, &[]).and_then(|_| one("layer.selectBelow", json!({}))),
        "layeractionsetcurrentlayer" => {
            only(p, &["layername"])?;
            one("layer.selectByName", json!({"name": text(p, "layername")?}))
        }
        "layeractionselectcontinuouslayer" => {
            only(p, &["layername"])?;
            one("layer.selectByName", json!({"name": text(p, "layername")?, "mode": "range"}))
        }

        // ---- layer properties ----
        "layeractionchangename" => {
            only(p, &["layername"])?;
            one("layer.renameLayer", json!({"name": text(p, "layername")?}))
        }
        "layeractionsetcompositemode" => {
            only(p, &["compositemode"])?;
            let code = int(p, "compositemode")?;
            // Through (30) is a folder mode; on a layer CLIP STUDIO shows it only for folders.
            let (mode, note) = blend(code, true)?;
            Ok(Mapped { steps: vec![("layer.setProps".into(), json!({"blend": mode}))], note })
        }
        "layeractionsetopacity" => {
            only(p, &["opacity"])?;
            let o = int(p, "opacity")?;
            if !(0..=100).contains(&o) {
                return Err("`opacity` is out of range".into());
            }
            one("layer.setProps", json!({"opacity": o as f64 / 100.0}))
        }
        "layeractionchangevisiblelayer" => {
            only(p, &["visiblelayer"])?;
            one("layer.setProps", json!({"visible": flag(p, "visiblelayer")?}))
        }
        "layeractionchangelockalpha" => {
            only(p, &["lockalpha"])?;
            one("layer.setProps", json!({"locks": {"transparency": flag(p, "lockalpha")?}}))
        }
        "layeractionchangebelowclip" => {
            only(p, &["belowclip"])?;
            one(if flag(p, "belowclip")? { "layer.createClippingMask" } else { "layer.releaseClippingMask" }, json!({}))
        }

        // ---- application and files ----
        "applicationactionchangecurrentcolor" => {
            only(p, &["currentcolor"])?;
            one("tools.setColors", json!({"foreground": hex(rgb(p.get("currentcolor"), "currentcolor")?)}))
        }
        "fileactionnewfromclipboard" => only(p, &[]).and_then(|_| one("file.newFromClipboard", json!({}))),
        "fileactionnew" => {
            only(p, &[
                "canvasheight", "canvaswidth", "defaultcolor", "defaultcolor122", "filename", "papercolor", "presetid", "resolution", "settingtype", "usecropframe", "uselayertemplate",
                "usemulticanvas", "usepaperlayer",
            ])?;
            for k in ["usecropframe", "uselayertemplate", "usemulticanvas"] {
                expect(p, k, 0)?;
            }
            let size = |k: &str| pixels(p.get(k), k).and_then(|v| if (1.0..=300_000.0).contains(&v) { Ok(v.round()) } else { Err(format!("`{k}` is out of range")) });
            let resolution = number(p.get("resolution"), "resolution")?;
            if !(1.0..=10_000.0).contains(&resolution) {
                return Err("`resolution` is out of range".into());
            }
            let background = if flag(p, "usepaperlayer")? { hex(rgb(p.get("papercolor"), "papercolor")?) } else { "transparent".into() };
            let mut q = json!({"width": size("canvaswidth")?, "height": size("canvasheight")?, "resolution": resolution, "background": background});
            if let Ok(name) = text(p, "filename") {
                q["name"] = json!(name);
            }
            noted("file.new", q, "CLIP STUDIO's paper layer becomes the Background layer")
        }
        _ => Err("this CLIP STUDIO step has no Prism equivalent yet".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(v: Value) -> Params {
        match v {
            Value::Object(mut m) => {
                m.insert("typename".into(), json!(""));
                m
            }
            _ => panic!("object expected"),
        }
    }
    fn mapped(kind: &str, v: Value) -> (String, Value) {
        let m = map(kind, &params(v)).unwrap();
        assert_eq!(m.steps.len(), 1);
        m.steps.into_iter().next().unwrap()
    }

    #[test]
    fn colours_are_signed_32_bit_channels() {
        let c = |r: i64, g: i64, b: i64| json!({"typename":"rgbcolor","red":r,"green":g,"blue":b});
        assert_eq!(rgb(Some(&c(-1, -1, -1)), "c").unwrap(), [1.0, 1.0, 1.0]);
        assert_eq!(rgb(Some(&c(4_294_967_295, 0, 0)), "c").unwrap(), [1.0, 0.0, 0.0]);
        assert_eq!(hex(rgb(Some(&c(-858_993_460, -858_993_460, -858_993_460)), "c").unwrap()), "#cccccc");
        assert_eq!(hex(rgb(Some(&c(0x8080_8080, 0x4040_4040, -0x4040_4041)), "c").unwrap()), "#8040bf");
        assert!(rgb(Some(&json!({"typename":"rgbcolor","red":0,"green":0})), "c").is_err());
    }

    #[test]
    fn layer_steps_map_to_prism_commands() {
        assert_eq!(mapped("layeractionsetcompositemode", json!({"compositemode": 2})), ("layer.setProps".into(), json!({"blend": "Multiply"})));
        assert_eq!(mapped("layeractionsetcompositemode", json!({"compositemode": 30})), ("layer.setProps".into(), json!({"blend": "Pass Through"})));
        let add = map("layeractionsetcompositemode", &params(json!({"compositemode": 11}))).unwrap();
        assert!(add.note.unwrap().contains("Add"), "approximated modes are noted");
        assert!(map("layeractionsetcompositemode", &params(json!({"compositemode": 27}))).is_err(), "Erase is a brush mode");
        assert!(map("layeractionsetcompositemode", &params(json!({"compositemode": 99}))).is_err());
        assert_eq!(mapped("layeractionsetopacity", json!({"opacity": 50})).1, json!({"opacity": 0.5}));
        assert!(map("layeractionsetopacity", &params(json!({"opacity": 101}))).is_err());
        assert_eq!(mapped("layeractionchangelockalpha", json!({"lockalpha": 0})).1, json!({"locks": {"transparency": false}}));
        assert_eq!(mapped("layeractionchangebelowclip", json!({"belowclip": 1})).0, "layer.createClippingMask");
        assert_eq!(mapped("layeractionchangebelowclip", json!({"belowclip": 0})).0, "layer.releaseClippingMask");
        assert_eq!(mapped("layeractionsetcurrentlayer", json!({"layername": "_M"})), ("layer.selectByName".into(), json!({"name": "_M"})));
        assert_eq!(mapped("layeractionselectcontinuouslayer", json!({"layername": "u_Kg"})).1, json!({"name": "u_Kg", "mode": "range"}));
        assert_eq!(mapped("layeractionselectfromlayerstatusmultiply", json!({"referselected": 0, "layername": "_C"})).1, json!({"channel": "transparency", "layerName": "_C", "operation": "intersect"}));
        assert_eq!(mapped("layeractionselectfromlayerstatusnew", json!({"referselected": 0})).1, json!({"channel": "transparency", "operation": "new"}));
        assert!(map("layeractionmergelowerlayer", &params(json!({"keepmask": 1}))).is_err());
        assert!(map("layeractionfoldernewandinsert", &params(json!({"duplicate": 1}))).is_err());
    }

    #[test]
    fn values_convert_to_prism_units() {
        let len = |v: f64| json!({"typename":"length","unit":0,"value":v});
        let canvas = mapped("editactionchangecanvassize", json!({"basepoint": 5, "width": len(1024.0), "height": len(512.0)}));
        assert_eq!(canvas.1, json!({"width": 1024.0, "height": 512.0, "anchor": "center", "extensionColor": "transparent"}));
        assert_eq!(mapped("editactionchangecanvassize", json!({"basepoint": 1, "width": len(8.0), "height": len(8.0)})).1["anchor"], "topLeft");
        let mm = json!({"typename":"length","unit":2,"value":14.1});
        assert!(map("editactionchangecanvassize", &params(json!({"basepoint": 5, "width": mm, "height": len(8.0)}))).is_err(), "only pixels");
        let mv = mapped("toolactionmovelayer", json!({"inselectarea":0,"movegrid":0,"movetarget":0,"remainoriginal":0,"selectlayer":0,"movelayerx":len(20.0),"movelayery":len(-40.0)}));
        assert_eq!(mv, ("layer.translate".into(), json!({"dx": 20, "dy": -40})));
        let curve = mapped("adjustmentactionToneCurve", json!({"channelred":[{"x":0,"y":65535,"typename":""},{"x":65535,"y":0,"typename":""}]}));
        assert_eq!(curve.1, json!({"red": [[0.0, 255.0], [255.0, 0.0]]}));
        let master = mapped("adjustmentactionToneCurve", json!({"channelmaster":[{"x":0,"y":0,"typename":""},{"x":65535,"y":32767.5,"typename":""}]}));
        assert_eq!(master.1, json!({"points": [[0.0, 0.0], [255.0, 127.5]]}));
        let drag = map("toolactionmovelayer", &params(json!({"movelayerx":len(511.87),"movelayery":len(-0.4)}))).unwrap();
        assert_eq!(drag.steps[0].1, json!({"dx": 512, "dy": 0}));
        assert!(drag.note.is_some());
        assert_eq!(mapped("adjustmentactionhsv", json!({"hue":0,"saturation":-100,"value":0})).1, json!({"hue":0,"saturation":-100,"lightness":0}));
        let fill = mapped("layeractionmonochromefillnew", json!({"fillcolor":{"typename":"rgbcolor","red":-1,"green":-1,"blue":-1}}));
        assert_eq!(fill.1, json!({"color": "#ffffff"}));
        let scale = mapped(
            "editactiontransformstretch",
            json!({"anchortype":0,"changevectorwidth":0,"fixaspect":0,"positiontype":4,"quality":4,"scalex":101.0,"scaley":102.0,"transformoperation":1,"transformtype":0}),
        );
        assert_eq!(scale.1, json!({"scaleX":101.0,"scaleY":102.0,"angle":0.0,"dx":0.0,"dy":0.0,"reference":"center","interpolation":"bicubic","keepOriginal":false}));
        // Isometric 30° L's second step: scale, then rotate counter-clockwise.
        let rotate = mapped(
            "editactiontransformstretch",
            json!({"anchortype":0,"changevectorwidth":1,"fixaspect":0,"positiontype":4,"quality":4,"rotation":-30.0,"scalex":58.0,"scaley":100.0,"transformoperation":1,"transformtype":1}),
        );
        assert_eq!((rotate.1["angle"].as_f64(), rotate.1["scaleX"].as_f64()), (Some(-30.0), Some(58.0)));
        for bad in [json!({"rotation": 400.0, "transformtype": 1}), json!({"rotation": "45", "transformtype": 1}), json!({"transformtype": 2})] {
            let mut q = json!({"anchortype":0,"positiontype":4,"transformoperation":1});
            q.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
            assert!(map("editactiontransformstretch", &params(q)).is_err());
        }
    }

    #[test]
    fn level_and_gradient_map_layers() {
        let ch = |low: i64, mid: i64, high: i64| json!({"input":{"high":high,"low":low,"mid":mid,"typename":""},"output":{"high":65535,"low":0,"typename":""},"typename":""});
        // The normal-map action: red and green stretched from 0..127.
        let new = map("layeractionlevelnew", &params(json!({"adjustment":{"channelred":ch(0, 16318, 32638),"channelgreen":ch(0, 16189, 32379),"typename":""}}))).unwrap();
        assert_eq!(new.steps[0].0, "layer.newAdjustmentLayer.levels");
        let q = &new.steps[0].1;
        assert_eq!(q["red"], json!({"inBlack": 0.0, "inWhite": 127.0, "gamma": 1.0, "outBlack": 0.0, "outWhite": 255.0}));
        assert_eq!(q["green"]["inWhite"], json!(125.99));
        assert_eq!((q["blue"]["inWhite"].as_f64(), q["inWhite"].as_f64()), (Some(255.0), Some(255.0)), "untouched channels are identity");
        assert!(new.note.is_none(), "a middle midtone is exact");
        // A midtone off the middle becomes an (inferred) gamma: a third of the way brightens.
        let off = map("layeractionlevelsettings", &params(json!({"adjustment":{"channelmaster":ch(0, 21845, 65535),"typename":""}}))).unwrap();
        assert_eq!(off.steps[0].0, "layer.setAdjustment");
        assert!((off.steps[0].1["gamma"].as_f64().unwrap() - 1.585).abs() < 1e-3);
        assert!(off.note.is_some());
        for bad in [ch(100, 50, 200), ch(0, 10, 100), ch(0, 40000, 70000), json!({"input":{"low":0,"mid":1,"high":9999}}), json!(3)] {
            assert!(map("layeractionlevelnew", &params(json!({"adjustment":{"channelred":bad}}))).is_err());
        }
        assert!(map("layeractionlevelnew", &params(json!({"adjustment":{"channelalpha":ch(0, 1000, 2000)}}))).is_err());
        let off = map("layeractionsetuselayercolor", &params(json!({"use": 0}))).unwrap();
        assert!(off.steps.is_empty() && off.note.is_some());
        assert!(map("layeractionsetuselayercolor", &params(json!({"use": 1}))).is_err());

        // Gradient Map: a four-word header, 28-byte stops, straight curves.
        let word = |v: u32| format!("{v:08x}");
        let stop = |rgb: [u32; 3], flags: u32, pos: u32, points: u32| [rgb[0], rgb[1], rgb[2], u32::MAX, flags, pos, points].map(word).concat();
        let straight = ["0000000000000000", "0000000000000000", "3ff0000000000000", "3ff0000000000000"].concat();
        let blob = |first: String, last: String, curve: &str| json!({"gradation":{"$bytes": format!("{}{first}{last}{curve}", [16, 28, 2, 16].map(word).concat())}});
        let g = mapped("layeractiongradientmapnew", json!({"adjustment": blob(stop([0x2425_2425, 0, 0], 0x10, 0, 2), stop([u32::MAX; 3], 0, 0x8000, 0), &straight)}));
        assert_eq!(g.0, "layer.newAdjustmentLayer.gradientMap");
        let stops = g.1["stops"].as_array().unwrap();
        assert_eq!(stops.len(), 2);
        assert!((stops[0][1][0].as_f64().unwrap() - 0x24 as f64 / 255.0).abs() < 1e-3);
        assert_eq!(stops[1][0].as_f64(), Some(1.0));
        let live = map("layeractiongradientmapnew", &params(json!({"adjustment": blob(stop([0; 3], 1, 0, 0), stop([0; 3], 0, 0x8000, 0), "")})));
        assert!(live.is_err(), "drawing-colour stops");
        for bad in [json!({"gradation":{"$bytes":"zz"}}), json!({"gradation":{"$bytes":"0000"}}), json!({"gradation":"00"}), json!({})] {
            assert!(map("layeractiongradientmapnew", &params(json!({"adjustment": bad}))).is_err());
        }
    }

    #[test]
    fn convert_layer_steps() {
        let color = json!({"$bytes": "000000000000000100000001"});
        let step = |kind: i64, extra: Value| {
            let mut q = json!({"addnumbertoname":1,"channelorder":33,"colortype":color,"compositemode":0,"copytimeline":0,"layerkind":kind,"leaveoriginal":1});
            q.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            map("layeractionconvert", &params(q))
        };
        // CMY/K separation: a raster copy above the source.
        let raster = step(0, json!({})).unwrap();
        assert_eq!(raster.steps[0], ("layer.convert".into(), json!({"kind": "pixel", "keepOriginal": true, "blend": "Normal"})));
        assert!(raster.note.unwrap().contains("copy name"));
        // Misregistration set: named image material layers in place, Darken.
        let material = step(2, json!({"addnumbertoname":0,"leaveoriginal":0,"layername":"RorC","compositemode":1})).unwrap();
        assert_eq!(material.steps[0].1, json!({"kind": "smartObject", "keepOriginal": false, "blend": "Darken", "name": "RorC"}));
        assert!(material.note.unwrap().contains("smart object"));
        // Gray (the brush-material variant), vector and timeline copies are refused.
        assert!(step(2, json!({"channelorder":1,"colortype":{"$bytes":"000000010000000100000000"}})).is_err());
        assert!(step(1, json!({})).is_err());
        assert!(step(0, json!({"copytimeline":1})).is_err());
        assert!(step(0, json!({"leaveoriginal":2})).is_err());
        assert!(step(0, json!({"layername":""})).is_err());
    }

    #[test]
    fn unknown_kinds_and_parameters_are_refused_not_guessed() {
        assert!(map("layeractionsettoneline", &params(json!({"line": 30.0}))).is_err());
        assert!(map("selectactiondeselect", &params(json!({"extra": 1}))).is_err());
        assert!(map("adjustmentactionLevel", &params(json!({"channelred": {}}))).is_err());
        assert_eq!(mapped("adjustmentactionLevel", json!({})).0, "image.adjustments.levels");
        for kind in ["layeractionchangename", "layeractionsetcurrentlayer"] {
            for bad in [json!({}), json!({"layername": ""}), json!({"layername": 3}), json!({"layername": "x".repeat(300)})] {
                assert!(map(kind, &params(bad)).is_err());
            }
        }
        // Every mapping refuses hostile values without panicking.
        let hostile = [json!(null), json!(-1), json!(i64::MAX), json!(1e308), json!("x"), json!([]), json!({}), json!({"typename":"length","unit":0,"value":1e300})];
        for kind in [
            "selectactionexpand", "selectactionreduce", "editactiondetailpaint", "editactionchangecanvassize", "editactiontransformstretch", "toolactionmovelayer",
            "adjustmentactionToneCurve", "adjustmentactionposterize", "adjustmentactionbrightnessandcontrast", "layeractionhsvnew", "layeractiontonecurvenew",
            "layeractionmonochromefillnew", "applicationactionchangecurrentcolor", "fileactionnew", "layeractionsetcompositemode",
            "layeractionlevelnew", "layeractionlevelsettings", "layeractiongradientmapnew", "layeractionconvert",
        ] {
            for v in &hostile {
                for key in ["sizelength", "opacity", "width", "scalex", "movelayerx", "channelred", "steps", "adjustment", "fillcolor", "currentcolor", "canvaswidth", "compositemode"] {
                    let mut m = Params::new();
                    m.insert(key.into(), v.clone());
                    let _ = map(kind, &m);
                }
            }
        }
    }
}
