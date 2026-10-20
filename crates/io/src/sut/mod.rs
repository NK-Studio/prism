//! Bounded, Rust-only Clip Studio Paint `.sut` import.
//! Layout observations: CSP2PC (https://github.com/Leon-Schoenbrunn/CSP2PC),
//! plus locally inspected files. No converter code or brush assets are bundled.
mod display;
mod effector;
pub mod library;
mod material;
mod sqlite;
use crate::abr_map::AbrImport;
use prism_paint::{
    BrushPreset, BrushSettings, Control, CspColorMixing, CspInOut, CspInOutMode, CspInOutParam, CspInOutTarget, CspSpray, CspStabilizer, CspTextureMode,
    CspTipColor, CspTipOrder, CspWaterEdge, Dynamic, GrayTile, Pattern, TipShape,
};
use sqlite::{Database, Value};
use std::collections::BTreeMap;

type Row = BTreeMap<String, Value>;
fn num(r: &Row, k: &str, default: f64) -> f32 {
    r.get(k).and_then(Value::number).unwrap_or(default).clamp(-1e6, 1e6) as f32
}
fn enabled(r: &Row, k: &str) -> bool {
    num(r, k, 0.0) != 0.0
}

fn png(blob: &[u8]) -> Result<&[u8], String> {
    let start = blob.windows(8).rposition(|b| b == b"\x89PNG\r\n\x1a\n").ok_or("SUT material has no PNG tip")?;
    let mut at = start + 8;
    for _ in 0..4096 {
        let header = blob.get(at..at.saturating_add(8)).ok_or("truncated SUT PNG")?;
        let len = u32::from_be_bytes(header.get(..4).ok_or("truncated PNG length")?.try_into().map_err(|_| "invalid PNG length")?) as usize;
        let end = at.checked_add(12).and_then(|n| n.checked_add(len)).filter(|n| *n <= blob.len()).ok_or("truncated SUT PNG chunk")?;
        if header.get(4..8) == Some(b"IEND") {
            if len != 0 {
                return Err("invalid PNG IEND".into());
            }
            return blob.get(start..end).ok_or_else(|| "invalid PNG range".into());
        }
        at = end;
    }
    Err("too many SUT PNG chunks".into())
}
/// A colour tip of several colours on a brush that mixes colours: Prism's mixing holds one colour
/// a dab, so such dabs mix the tip's mean colour.
pub const COLOUR_TIP: &str = "SUT colour brush tips with colour mixing paint in each tip's mean colour; CLIP STUDIO keeps every pixel's colour";
/// Pixels all of a brush's tips may hold together.
const TIP_PIXEL_BUDGET: usize = 16_000_000;
/// A decoded material: `[luminance, alpha]` per pixel.
struct Samples {
    width: u32,
    height: u32,
    px: Vec<[f32; 2]>,
    /// The factor the larger edge was reduced by to fit.
    reduced: f32,
    /// An original colour material (thumbnails are not).
    color: bool,
    /// A colour material's straight RGB, three bytes a pixel.
    rgb: Option<Vec<u8>>,
}
/// Decode a material, reduced to at most `max_pixels` (original rasters; thumbnails are small).
fn gray_alpha(blob: &[u8], max_pixels: usize, warnings: &mut Vec<String>) -> Result<Samples, String> {
    match material::raster_within(blob, max_pixels) {
        Ok(Some(raster)) => {
            let gray = |i: usize| raster.gray.as_ref().and_then(|g| g.get(i)).map_or(0.0, |g| f32::from(*g) / 255.0);
            let px = raster.alpha.iter().enumerate().map(|(i, a)| [gray(i), f32::from(*a) / 255.0]).collect();
            let rgb = raster.rgb.filter(|_| raster.color);
            return Ok(Samples { width: raster.width, height: raster.height, px, reduced: raster.reduced, color: raster.color, rgb });
        }
        Ok(None) => {}
        Err(error) => warnings.push(format!("SUT original material could not be read ({error}); using its thumbnail, which can change the brush grain")),
    }
    use prism_codecs::{DecodeOptions, Format, Limits};
    let opts = DecodeOptions { limits: Limits { max_width: 2500, max_height: 2500, max_pixels: 6_250_000, max_alloc: 64 << 20 }, ..Default::default() };
    let im = prism_codecs::decode_as_with(Format::Png, png(blob)?, &opts).map_err(|e| e.to_string())?;
    let ch = im.layout().channels();
    if !im.layout().is_gray() && !im.layout().is_rgb() {
        return Err("unsupported SUT material colour model".into());
    }
    let mut data = Vec::with_capacity(im.pixel_count());
    for i in 0..im.pixel_count() {
        let base = i * ch;
        let gray = if im.layout().is_gray() {
            im.sample_normalized(base)
        } else {
            0.299 * im.sample_normalized(base) + 0.587 * im.sample_normalized(base + 1) + 0.114 * im.sample_normalized(base + 2)
        };
        let alpha = if im.layout().has_alpha() { im.sample_normalized(base + ch - 1) } else { 1.0 };
        data.push([gray.clamp(0.0, 1.0), alpha.clamp(0.0, 1.0)]);
    }
    Ok(Samples { width: im.width(), height: im.height(), px: data, reduced: 1.0, color: false, rgb: None })
}
/// A brush tip of at most `max_pixels` pixels, with its colours when its material is a colour
/// one. A gray tip's darkness is its density; a colour tip is its shape (alpha), in its own
/// colours, or in the brush colour when the brush changes the tip colour.
fn tip(blob: &[u8], max_pixels: usize, warnings: &mut Vec<String>) -> Result<(GrayTile, Option<CspTipColor>), String> {
    let Samples { width, height, px, color, rgb, .. } = gray_alpha(blob, max_pixels, warnings)?;
    let density = |[g, a]: &[f32; 2]| if color { *a } else { (1.0 - g) * a };
    let data = px.iter().map(|p| (density(p) * 65535.0).round() as u16).collect();
    let tile = GrayTile { width, height, data };
    let colors = rgb.map(|rgb| CspTipColor { width, height, rgb }).filter(|c| color && c.fits(&tile));
    if color && colors.is_none() {
        warnings.push("SUT colour brush tip colours could not be read; the tip paints in the brush colour".into());
    }
    Ok((tile, colors))
}
/// Texture tiles keep luminance; transparency reads as white before mode-specific blending.
/// Returns the tile and the factor its larger edge was reduced by.
fn texture_tile(blob: &[u8], warnings: &mut Vec<String>) -> Result<(GrayTile, f32), String> {
    let Samples { width: w, height: h, px, reduced, .. } = gray_alpha(blob, material::DEFAULT_MAX_PIXELS, warnings)?;
    let gray = px.iter().map(|[g, a]| g * a + (1.0 - a)).collect();
    let (nw, nh, gray) = crate::abr_map::fit(w, h, gray, crate::abr_map::MAX_PATTERN_EDGE);
    let shrink = reduced * w.max(h) as f32 / nw.max(nh).max(1) as f32;
    let data = gray.iter().map(|x| (x.clamp(0.0, 1.0) * 65535.0).round() as u16).collect();
    Ok((GrayTile { width: nw, height: nh, data }, shrink))
}
/// Material references: BE u32 header size (8), BE u32 count, then length-prefixed records.
/// Each record starts with its BE u32 total length and BE u32 UTF-16LE path byte length;
/// remaining record metadata is opaque. Every declared path must resolve exactly once.
fn referenced<'a>(refs: &[u8], materials: &'a [Row], what: &str) -> Result<Vec<&'a Row>, String> {
    let mut ordered = Vec::new();
    for path in reference_paths(refs, what)? {
        let mut matches = materials.iter().filter(|m| m.get("OriginalPath").and_then(Value::text) == Some(path.as_str()));
        let mat = matches.next().ok_or_else(|| format!("some referenced SUT {what} are missing"))?;
        if matches.next().is_some() {
            return Err(format!("referenced SUT {what} have ambiguous material paths"));
        }
        ordered.push(mat);
    }
    Ok(ordered)
}
/// The declared material paths of a reference blob, in order.
fn reference_paths(refs: &[u8], what: &str) -> Result<Vec<String>, String> {
    let invalid = || format!("invalid SUT {what} references");
    let read_u32 = |bytes: Option<&[u8]>| -> Result<usize, String> {
        let bytes: [u8; 4] = bytes.and_then(|v| v.try_into().ok()).ok_or_else(invalid)?;
        usize::try_from(u32::from_be_bytes(bytes)).map_err(|_| invalid())
    };
    if read_u32(refs.get(..4))? != 8 {
        return Err(invalid());
    }
    let count = read_u32(refs.get(4..8))?;
    if count == 0 || count > 64 {
        return Err(format!("SUT has no readable referenced {what} (or more than 64)"));
    }
    let mut ordered = Vec::with_capacity(count);
    let mut at = 8usize;
    for _ in 0..count {
        let len = read_u32(refs.get(at..at.checked_add(4).ok_or_else(invalid)?))?;
        let end = at.checked_add(len).ok_or_else(invalid)?;
        let record = refs.get(at..end).filter(|r| r.len() >= 8).ok_or_else(invalid)?;
        let path_len = read_u32(record.get(4..8))?;
        if path_len == 0 || !path_len.is_multiple_of(2) {
            return Err(invalid());
        }
        let path_bytes = record.get(8..8usize.checked_add(path_len).ok_or_else(invalid)?).ok_or_else(invalid)?;
        let units =
            path_bytes.chunks_exact(2).map(|unit| <[u8; 2]>::try_from(unit).map(u16::from_le_bytes).map_err(|_| invalid())).collect::<Result<Vec<_>, _>>()?;
        ordered.push(String::from_utf16(&units).map_err(|_| invalid())?);
        at = end;
    }
    if at != refs.len() {
        return Err(invalid());
    }
    Ok(ordered)
}
fn pressure_effector(r: &Row, k: &str, warnings: &mut Vec<String>) -> Option<Dynamic> {
    let value = match r.get(k) {
        None | Some(Value::Null) => return None,
        Some(value) => value,
    };
    let result = value.blob().ok_or_else(|| "effector is not a blob".to_string()).and_then(effector::read);
    match result {
        Ok(e) => {
            if e.other_controls != 0 {
                warnings.push(format!("SUT {k} has additional input controls ({:#x}) that are not supported", e.other_controls));
            }
            if let Some(error) = e.tilt_error {
                warnings.push(format!("SUT {k} tilt response could not be read ({error}); it is disabled"));
            }
            e.dynamic
        }
        Err(error) => {
            warnings.push(format!("SUT {k} could not be read ({error}); its pressure dynamics are disabled"));
            None
        }
    }
}

/// Read one sub-tool into a preset, retaining all referenced primary tips in file order.
/// Unsupported behaviour is reported through warnings (things that need no action through
/// notes); unreadable tips fail the entire import.
pub fn read_sut(bytes: &[u8], group: &str, ctl: &prism_raster::Interrupt) -> Result<AbrImport, String> {
    ctl.check().map_err(|e| e.to_string())?;
    if bytes.starts_with(crate::csp_container::MAGIC) {
        let decoded = crate::csp_container::decode(bytes, ctl)?;
        if !decoded.starts_with(b"SQLite format 3\0") {
            return Err("CLIP STUDIO tool payload is not a SQLite database".into());
        }
        let outer = Database::new(&decoded)?;
        let materials = outer.table("Material")?;
        if materials.len() != 1 {
            return Err("CLIP STUDIO tool must contain exactly one material".into());
        }
        let material = materials.first().ok_or("CLIP STUDIO tool has no material")?;
        let tool = material.get("MaterialToolVariantManager").and_then(Value::blob).ok_or("CLIP STUDIO material is not a sub-tool")?;
        if !tool.starts_with(b"SQLite format 3\0") {
            return Err("CLIP STUDIO sub-tool settings are not a SQLite database".into());
        }
        // Downloaded tools keep their tips and textures beside the settings, in the outer
        // database's `ToolPatternInMaterial` (same package format as a SUT `MaterialFile`).
        return read_database(tool, group, ctl, Some(&|| outer.table("ToolPatternInMaterial").map(tool_patterns)));
    }
    read_database(bytes, group, ctl, None)
}
/// `ToolPatternInMaterial` rows as `MaterialFile` rows: the reference path (`RelativePath`) and
/// the material package (`FileData`).
fn tool_patterns(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter()
        .filter_map(|r| {
            let path = r.get("RelativePath")?.clone();
            let data = r.get("FileData")?.clone();
            Some(Row::from([("OriginalPath".to_string(), path), ("FileData".to_string(), data)]))
        })
        .collect()
}
/// Read the sub-tool database `bytes`. Its own `MaterialFile` table holds the referenced
/// materials; without one, `fallback` (a container's material table) is used.
fn read_database(bytes: &[u8], group: &str, ctl: &prism_raster::Interrupt, fallback: Option<&dyn Fn() -> Result<Vec<Row>, String>>) -> Result<AbrImport, String> {
    let db = Database::new(bytes)?;
    let nodes = db.table("Node")?;
    let node = nodes.iter().find(|r| r.get("NodeName").and_then(Value::text).is_some()).ok_or("SUT has no named sub-tool")?;
    let variants = db.table("Variant")?;
    let r = active_variant(node, &variants)?;
    read_tool(
        node,
        r,
        &|| match (db.table("MaterialFile"), fallback) {
            (Err(_), Some(outer)) => outer(),
            (own, _) => own,
        },
        group,
        ctl,
    )
}
/// The variant a sub-tool node currently uses (`NodeVariantID`).
fn active_variant<'a>(node: &Row, variants: &'a [Row]) -> Result<&'a Row, String> {
    let variant_id = node.get("NodeVariantID").and_then(Value::integer).ok_or("SUT current brush identifier is invalid")?;
    variants.iter().find(|r| r.get("VariantID").and_then(Value::integer) == Some(variant_id)).ok_or_else(|| "SUT current brush settings are missing".into())
}
/// Map one sub-tool node and its active variant. `materials` yields `MaterialFile`-shaped rows
/// (`OriginalPath`, `FileData`) and is only consulted when the brush references tips or textures.
fn read_tool(node: &Row, r: &Row, materials: &dyn Fn() -> Result<Vec<Row>, String>, group: &str, ctl: &prism_raster::Interrupt) -> Result<AbrImport, String> {
    let name = node.get("NodeName").and_then(Value::text).filter(|n| !n.trim().is_empty()).unwrap_or(group).to_string();
    if name.chars().count() > 255 {
        return Err("SUT brush name exceeds 255 characters".into());
    }
    let op = |key: &str| node.get(key).and_then(Value::integer).unwrap_or(0);
    let kind = prism_paint::sut::SutToolKind::classify(op("NodeInputOp"), op("NodeOutputOp"));
    // The dot pen stores no size (also in CSP's own sub-tool library): it draws single pixels.
    let default_size = if kind == prism_paint::sut::SutToolKind::DotPen { 1.0 } else { 20.0 };
    let mut b = BrushSettings {
        size: num(r, "BrushSize", default_size).clamp(0.5, prism_paint::MAX_BRUSH_SIZE),
        opacity: (num(r, "Opacity", 100.0) / 100.0).clamp(0.0, 1.0),
        flow: (num(r, "BrushFlow", 100.0) / 100.0).clamp(0.0, 1.0),
        hardness: (num(r, "BrushHardness", 100.0) / 100.0).clamp(0.0, 1.0),
        spacing: 0.1,
        roundness: 1.0,
        csp_tip_scale: Some({
            let thickness = (num(r, "BrushThickness", 100.0) / 100.0).clamp(0.01, 10.0);
            if enabled(r, "BrushVerticalThicknes") { [1.0, thickness] } else { [thickness, 1.0] }
        }),
        angle: -num(r, "BrushRotation", 0.0),
        pressure_size: false,
        flip_x: enabled(r, "BrushPatternReverseHorizontal"),
        flip_y: enabled(r, "BrushPatternReverseVertical"),
        csp_anti_alias: r
            .get("AntiAlias")
            .and_then(Value::number)
            .map(|level| level.clamp(0.0, f64::from(prism_paint::CSP_ANTI_ALIAS_MAX)) as u8)
            .or((kind == prism_paint::sut::SutToolKind::DotPen).then_some(0)),
        ..Default::default()
    };
    let mut warnings = Vec::new();
    let mut notes = vec![String::from("SUT rendering is approximated; CSP rendering parity is not guaranteed")];
    if !kind.is_brush() {
        // Enclose-and-fill (output 51) included: the Clip Studio workspace runs it as a lasso fill.
        notes.push(format!(
            "This SUT is a Clip Studio {} sub-tool, not a brush; it is kept for the Clip Studio workspace and hidden from the brush list",
            kind.label().to_lowercase()
        ));
    }
    b.spacing = spacing(r, &mut warnings);
    b.csp_stabilizer = stabilizer(r, &mut warnings);
    b.csp_in_out = in_out(r, &mut warnings);
    // Ribbon (property 0x456) applies only to a brush with tip images and without spraying;
    // CSP snaps its tip to quarter turns of `BrushRotation`, whether rotation is on or not.
    let ribbon = enabled(r, "BrushRibbon")
        && enabled(r, "BrushUsePatternImage")
        && !matches!(r.get("BrushPatternImageArray"), None | Some(Value::Null))
        && !enabled(r, "BrushUseSpray");
    b.csp_ribbon = ribbon.then(|| prism_paint::csp_ribbon::CspRibbon::from_rotation(f64::from(num(r, "BrushRotation", 0.0))));
    b.csp_mixing = color_mixing(r, ribbon, &mut warnings);
    if let Some(dynamic) = pressure_effector(r, "BrushSizeEffector", &mut warnings) {
        b.shape_dynamics.enabled = true;
        b.shape_dynamics.size = dynamic;
    }
    if let Some(dynamic) = pressure_effector(r, "BrushOpacityEffector", &mut warnings) {
        b.transfer.enabled = true;
        b.transfer.opacity = dynamic;
    }
    if let Some(dynamic) = pressure_effector(r, "BrushFlowEffector", &mut warnings) {
        b.transfer.enabled = true;
        b.transfer.flow = dynamic;
    }
    rotation(r, &mut b, &mut warnings);
    if enabled(r, "BrushUseSpray") {
        let count = num(r, "BrushSprayDensity", 8.0).round().max(1.0) as u32;
        let mut spray = CspSpray {
            size: num(r, "BrushSpraySize", 2.0).clamp(0.5, prism_paint::MAX_BRUSH_SIZE),
            count: count.min(prism_paint::csp_spray::MAX_PARTICLES),
            bias: (num(r, "BrushSprayBias", 0.0) / 100.0).clamp(-1.0, 1.0),
            angle: -num(r, "BrushRotationInSpray", 0.0),
            ..Default::default()
        };
        if r.get("BrushRotationEffector").and_then(Value::integer).is_some_and(|flags| flags & 1 == 0) {
            spray.angle = 0.0;
        }
        if let Some(dynamic) = pressure_effector(r, "BrushSpraySizeEffector", &mut warnings) {
            spray.size_dynamic = dynamic;
        }
        if let Some(dynamic) = pressure_effector(r, "BrushSprayDensityEffector", &mut warnings) {
            spray.count_dynamic = dynamic;
        }
        if count > prism_paint::csp_spray::MAX_PARTICLES {
            warnings.push("SUT spray particle count exceeds the supported per-step budget and was limited".into());
        }
        // Synchronized with the brush size: the stored particle size keeps its ratio to the
        // stored brush size when the brush is resized.
        if enabled(r, "BrushSpraySizeSyncBrushSize") && b.size > 0.0 {
            spray.size_ratio = Some(spray.size / b.size);
        }
        for key in ["BrushSpraySizeUnit", "BrushSprayUseFixedPoint"] {
            if enabled(r, key) {
                warnings.push(format!("SUT {key} is not supported; spray uses a fixed particle size in pixels"));
            }
        }
        let angle_controls = num(r, "BrushRotationEffectorInSpray", 0.0) as u32;
        if angle_controls & !3 != 0 {
            warnings.push("SUT particle angle input controls are not supported; using the base particle angle".into());
        }
        b.csp_spray = Some(spray);
    }
    // Observed in shipped brushes: the tip flag set with a NULL reference column and no
    // MaterialFile table. CSP draws these with its default round tip.
    let refs = match r.get("BrushPatternImageArray") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.blob().ok_or("SUT brush tip references are invalid")?),
    };
    if enabled(r, "BrushUsePatternImage") && refs.is_none() {
        warnings.push("SUT brush enables a tip image without referencing one; a round tip is used".into());
    }
    if enabled(r, "BrushUsePatternImage")
        && let Some(refs) = refs
    {
        let materials = materials()?;
        let ordered = referenced(refs, &materials, "tips")?;
        let mut tips = Vec::new();
        let mut pixels = 0usize;
        // Large original tips are reduced so the whole set fits the tip pixel budget.
        let each = (TIP_PIXEL_BUDGET / ordered.len().max(1)).min(material::DEFAULT_MAX_PIXELS);
        let mut colors = Vec::new();
        for mat in ordered {
            ctl.check().map_err(|e| e.to_string())?;
            let (t, color) = tip(mat.get("FileData").and_then(Value::blob).ok_or("SUT tip material is missing")?, each, &mut warnings)?;
            pixels = pixels.checked_add(t.data.len()).filter(|n| *n <= TIP_PIXEL_BUDGET).ok_or("SUT tips exceed pixel budget")?;
            tips.push(t);
            colors.push(color);
        }
        // Change brush tip colour (`BrushChangePatternColor`, property 0x44e) paints every tip in
        // the brush colour; without it CLIP STUDIO keeps the colours of a colour tip.
        if colors.iter().any(Option::is_some) && !enabled(r, "BrushChangePatternColor") {
            // Single-colour tips (white highlights) lose nothing by being laid in one colour.
            let uniform = tips.iter().zip(&colors).all(|(t, c)| c.as_ref().is_none_or(|c| c.is_uniform(t, 2)));
            if b.csp_mixing.is_some() && !uniform {
                warnings.push(COLOUR_TIP.into());
            }
            b.csp_tip_colors = colors;
        }
        if tips.len() > 1 {
            b.csp_tip_order = tip_order(r, &mut warnings).filter(|o| *o != CspTipOrder::Repeat);
        }
        b.tip = if tips.len() == 1 { TipShape::Sampled(tips.remove(0)) } else { TipShape::Sequence(tips) };
    }
    if enabled(r, "UseDualBrush") {
        warnings.push("SUT UseDualBrush is not supported".into());
    }
    if enabled(r, "BrushUseWaterEdge") {
        b.csp_water_edge = Some(water_edge(r, &mut warnings));
    }
    let mut texture_scale_factor = 1.0;
    match r.get("TextureImage") {
        None | Some(Value::Null) => {}
        Some(v) => {
            ctl.check().map_err(|e| e.to_string())?;
            // A broken texture drops the texture, not the brush.
            let result =
                v.blob().ok_or_else(|| "SUT texture reference is invalid".to_string()).and_then(|refs| texture(materials, r, refs, &mut b, &mut warnings));
            match result {
                Ok(factor) => texture_scale_factor = factor,
                Err(e) => warnings.push(format!("SUT texture could not be read ({e}); the brush is imported without it")),
            }
        }
    }
    let mut metadata = display::read(node, r, &mut warnings, &mut notes);
    if b.texture.enabled {
        // Presentation conversion only: original texture dimensions can exceed
        // the bounded rendered tile. Keep the source percent in CSP units.
        metadata.parameters.insert("PrismTextureScaleFactor".into(), f64::from(texture_scale_factor));
    }
    ctl.progress(1.0);
    Ok(AbrImport { presets: vec![BrushPreset { name, group: group.into(), brush: b, builtin: false, sut: Some(metadata) }], warnings, notes, version: 0 })
}

/// The tip order of a multi-tip brush (`BrushPatternOrderType2`, else `BrushPatternOrderType`;
/// property 0x42f): 0 repeat, 1 reverse, 2 do not repeat, 3 random. Evidence in
/// docs/sut-analysis.md.
fn tip_order(r: &Row, warnings: &mut Vec<String>) -> Option<CspTipOrder> {
    let value = ["BrushPatternOrderType2", "BrushPatternOrderType"].iter().find_map(|k| r.get(*k).filter(|v| !matches!(v, Value::Null)))?;
    let order = value.integer().and_then(CspTipOrder::from_code);
    if order.is_none() {
        warnings.push("SUT tip order is not supported; the tips repeat in order".into());
    }
    order
}

/// Map the tip rotation controls (`BrushRotationEffector` flags, property 0x426): 0x01 enables
/// the rotation; one input angle is added: 0x10 the pen's direction (where it leans), 0x20 its
/// axis rotation, else 0x40 the line direction; 0x80 adds a random turn of
/// `(r - 0.5) * BrushRotationRandomScale% * 360°`. Evidence in docs/sut-analysis.md.
fn rotation(r: &Row, b: &mut BrushSettings, warnings: &mut Vec<String>) {
    let Some(flags) = r.get("BrushRotationEffector").and_then(Value::integer) else { return };
    if flags & 1 == 0 {
        b.angle = 0.0;
        return;
    }
    let input = if flags & 0x10 != 0 {
        // 0x10 and 0x20 read the same pen angle in CLIP STUDIO; the pen direction wins.
        if flags & 0x20 != 0 {
            warnings.push("SUT tip rotation by both pen direction and pen rotation is approximated by the pen direction".into());
        }
        Some(Control::PenTilt)
    } else if flags & 0x20 != 0 {
        Some(Control::Rotation)
    } else if flags & 0x40 != 0 {
        Some(Control::Direction)
    } else {
        None
    };
    if let Some(control) = input {
        b.shape_dynamics.enabled = true;
        b.shape_dynamics.angle.control = control;
    }
    if flags & 0x80 != 0 {
        // Prism's angle jitter j spreads ±360°·j; CSP's random scale s spreads ±180°·s.
        let scale = (num(r, "BrushRotationRandomScale", 0.0) / 100.0).clamp(0.0, 2.0);
        b.shape_dynamics.enabled = true;
        b.shape_dynamics.angle.jitter = scale / 2.0;
    }
}

/// Dab spacing as a fraction of the diameter. Fixed spacing (`BrushAutoIntervalType` 0) is
/// `BrushInterval`; the automatic types scale a per-type base by hardness and thickness.
/// Evidence in docs/sut-analysis.md.
fn spacing(r: &Row, warnings: &mut Vec<String>) -> f32 {
    let kind = num(r, "BrushAutoIntervalType", 0.0) as i64;
    let base = match kind {
        0 => return (num(r, "BrushInterval", 10.0) / 100.0).clamp(0.01, 10.0),
        1 => 0.12,
        3 => 0.04,
        2 => 0.08,
        _ => {
            warnings.push(format!("SUT automatic spacing type {kind} is approximated by the normal spacing"));
            0.08
        }
    };
    // Image tips space as fully hard ones.
    let hardness = if enabled(r, "BrushUsePatternImage") { 1.0 } else { num(r, "BrushHardness", 100.0) / 100.0 };
    let hardness = if hardness >= 0.999_999_99 { 1.0 } else { hardness.max(0.2) };
    let thickness = num(r, "BrushThickness", 100.0) / 100.0;
    let thickness = if thickness < 1.0 { thickness.max(0.01) } else { 1.0 };
    (base * (3.5 - 2.5 * hardness) * thickness).clamp(0.01, 10.0)
}

/// Map Color mixing (BrushUseWaterColor 0x43e; amount 0x439, density 0x43b, stretch 0x43d with
/// their effectors). Evidence in docs/sut-analysis.md.
fn color_mixing(r: &Row, ribbon: bool, warnings: &mut Vec<String>) -> Option<CspColorMixing> {
    // CSP writes the one enable flag under both names; either is enough when the other is absent.
    let on = match r.get("BrushUseWaterColor").and_then(Value::number) {
        Some(v) => v != 0.0,
        None => enabled(r, "BrushUseWaterColor2"),
    };
    if !on {
        return None;
    }
    let kind = num(r, "BrushWaterColor", 0.0) as i64;
    // A ribbon brush mixes with type 0 where type 1 is set (the CSP settings builder resets it).
    if kind != 0 && !(ribbon && kind == 1) {
        warnings.push(format!("SUT color mixing type {kind} is approximated by blending"));
    }
    let percent = |k: &str| (num(r, k, 100.0) / 100.0).clamp(0.0, 1.0);
    let mut mixing = CspColorMixing {
        amount: percent("BrushMixColor"),
        density: percent("BrushMixAlpha"),
        stretch: (num(r, "BrushMixColorExtension", 0.0) / 100.0).clamp(0.0, 1.0),
        perceptual: enabled(r, "BrushColorMixingMode"),
        ..Default::default()
    };
    if let Some(dynamic) = pressure_effector(r, "BrushMixColorEffector", warnings) {
        mixing.amount_dynamic = dynamic;
    }
    if let Some(dynamic) = pressure_effector(r, "BrushMixAlphaEffector", warnings) {
        mixing.density_dynamic = dynamic;
    }
    Some(mixing)
}

/// Map Start and end (properties 0x488–0x49e; targets keyed by effector property IDs).
/// Evidence in docs/sut-analysis.md.
fn in_out(r: &Row, warnings: &mut Vec<String>) -> Option<CspInOut> {
    let (use_in, use_out) = (enabled(r, "BrushUseIn"), enabled(r, "BrushUseOut"));
    if !use_in && !use_out {
        return None;
    }
    let mode = match num(r, "BrushInOutType", 0.0) as i64 {
        0 => CspInOutMode::Length,
        1 => CspInOutMode::Percent,
        other => {
            warnings.push(format!("SUT start and end type {other} (fade) is not supported"));
            return None;
        }
    };
    for key in ["BrushInLengthUnit", "BrushOutLengthUnit"] {
        if mode == CspInOutMode::Length && enabled(r, key) {
            warnings.push(format!("SUT {key} is not supported; the length is used in pixels"));
        }
    }
    let length = |used: bool, length_key: &str, ratio_key: &str| {
        used.then(|| match mode {
            CspInOutMode::Length => f64::from(num(r, length_key, 0.0)),
            CspInOutMode::Percent => f64::from(num(r, ratio_key, 0.0)) / 100.0,
        })
        .filter(|v| *v > 0.0)
    };
    let mut targets = Vec::new();
    match r.get("BrushInOutTarget").map(|v| v.blob().ok_or("not a blob").and_then(in_out_targets)) {
        None => {}
        Some(Ok(entries)) => {
            for (id, minimum) in entries {
                let param = match id {
                    0x3e9 => CspInOutParam::Size,
                    0x3fd => CspInOutParam::Flow,
                    0x70 => CspInOutParam::TextureDensity,
                    0x46c => CspInOutParam::SpraySize,
                    0x46e => CspInOutParam::SprayDensity,
                    other => {
                        warnings.push(format!("SUT start and end target {other:#x} is not supported"));
                        continue;
                    }
                };
                targets.push(CspInOutTarget { param, minimum });
            }
        }
        Some(Err(error)) => warnings.push(format!("SUT start and end targets could not be read ({error})")),
    }
    let cfg = CspInOut {
        mode,
        start: length(use_in, "BrushInLength", "BrushInRatio"),
        end: length(use_out, "BrushOutLength", "BrushOutRatio"),
        by_speed: enabled(r, "BrushInOutBySpeed"),
        targets,
    }
    .bounded();
    (!cfg.targets.is_empty() && (cfg.start.is_some() || cfg.end.is_some())).then_some(cfg)
}

/// Enabled `(effector property, minimum)` entries of a start/end target array: big-endian u32
/// header length, count and entry size, then `(id, enabled, minimum %)` entries.
fn in_out_targets(blob: &[u8]) -> Result<Vec<(u32, f32)>, &'static str> {
    let word = |at: usize| blob.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_be_bytes).ok_or("truncated");
    let (header, count, size) = (word(0)? as usize, word(4)? as usize, word(8)? as usize);
    if header < 12 || size < 12 || count > 256 {
        return Err("invalid framing");
    }
    let mut out = Vec::new();
    for i in 0..count {
        let at = i.checked_mul(size).and_then(|n| n.checked_add(header)).ok_or("overflow")?;
        let (id, on, minimum) = (word(at)?, word(at + 4)?, word(at + 8)?);
        if on != 0 {
            out.push((id, (minimum.min(100) as f32) / 100.0));
        }
    }
    Ok(out)
}

/// Map Stabilization (`FlickerReduction`, property 0x28), Adjust by speed (0x29), its type (0x2a)
/// and `Stickness` (0x32). Evidence in docs/sut-analysis.md.
fn stabilizer(r: &Row, warnings: &mut Vec<String>) -> Option<CspStabilizer> {
    let level = num(r, "FlickerReduction", 0.0);
    if level < 1.0 {
        return None;
    }
    let max = prism_paint::csp_stabilizer::MAX_LEVEL;
    if level > max as f32 {
        warnings.push("SUT stabilization exceeds 100 and was limited".into());
    }
    let adjust_by_speed = enabled(r, "FlickerReductionBySpeed");
    let speed_type = num(r, "FlickerReductionBySpeedType", 0.0).max(0.0) as u32;
    Some(CspStabilizer {
        level: (level.round() as u32).min(max),
        adjust_by_speed,
        speed_type,
        stickiness: (num(r, "Stickness", 0.0).max(0.0).round() as u32).min(max),
    })
}

/// Map the Watercolor edge settings (property reader and renderer evidence in docs/sut-analysis.md).
fn water_edge(r: &Row, warnings: &mut Vec<String>) -> CspWaterEdge {
    let after_drag = enabled(r, "BrushWaterEdgeAfterDrag");
    let radius = num(r, "BrushWaterEdgeRadius", 0.0).max(0.0);
    let blur = if after_drag { num(r, "BrushWaterEdgeBlur", 0.0).max(0.0) } else { 0.0 };
    for key in ["BrushWaterEdgeRadiusUnit", "BrushWaterEdgeBlurUnit"] {
        if enabled(r, key) {
            warnings.push(format!("SUT {key} is not supported; the value is used in pixels"));
        }
    }
    if enabled(r, "BrushWaterEdgeStyle") {
        warnings.push("SUT watercolor edge style is not supported; the standard edge is used".into());
    }
    if radius > prism_paint::csp_water_edge::MAX_RADIUS {
        warnings.push("SUT watercolor edge radius exceeds Prism's work limit and was limited".into());
    }
    CspWaterEdge {
        radius: radius.min(prism_paint::csp_water_edge::MAX_RADIUS),
        opacity: (num(r, "BrushWaterEdgeAlphaPower", 0.0) / 100.0).clamp(0.0, 1.0),
        darkness: (num(r, "BrushWaterEdgeValuePower", 0.0) / 100.0).clamp(0.0, 1.0),
        blur: blur.min(prism_paint::csp_water_edge::MAX_BLUR),
        after_drag,
    }
}

/// Map the Texture category onto [`prism_paint::Texture`] (CSP manual: density, invert,
/// emphasize density, scale, rotation, brightness, contrast, mode, apply by each plot).
fn texture(materials: &dyn Fn() -> Result<Vec<Row>, String>, r: &Row, refs: &[u8], b: &mut BrushSettings, warnings: &mut Vec<String>) -> Result<f32, String> {
    let materials = materials()?;
    let mats = referenced(refs, &materials, "textures")?;
    if mats.len() > 1 {
        warnings.push("SUT brush references several textures; only the first is used".into());
    }
    let mat = mats.first().ok_or("SUT texture is missing")?;
    let (tile, shrink) = texture_tile(mat.get("FileData").and_then(Value::blob).ok_or("SUT texture material is missing")?, warnings)?;
    let t = &mut b.texture;
    t.enabled = true;
    t.pattern = Pattern::Tile(tile);
    t.scale = (num(r, "TextureScale2", 100.0) / 100.0 * shrink).clamp(0.01, 10.0);
    t.invert = enabled(r, "TextureReverseDensity");
    t.brightness = (num(r, "TextureBrightness", 0.0) / 100.0).clamp(-1.0, 1.0);
    t.contrast = (num(r, "TextureContrast", 0.0) / 100.0).clamp(-1.0, 1.0);
    t.each_tip = enabled(r, "TextureForPlot");
    t.depth = (num(r, "TextureDensity", 100.0) / 100.0).clamp(0.0, 1.0);
    if enabled(r, "TextureStressDensity") {
        t.contrast = (t.contrast + 0.5).clamp(-1.0, 2.0);
        warnings.push("SUT emphasize texture density is approximated by extra contrast".into());
    }
    if let Some(dynamic) = pressure_effector(r, "TextureDensityEffector", warnings) {
        if t.each_tip {
            t.depth_jitter = dynamic;
        } else {
            warnings.push("SUT pen-pressure texture density needs Apply by each plot; it is ignored".into());
        }
    }
    if num(r, "TextureRotate", 0.0) != 0.0 {
        warnings.push("SUT texture rotation is not supported".into());
    }
    let code = r.get("TextureCompositeMode").and_then(Value::number).unwrap_or(0.0);
    t.csp_mode = Some(texture_mode(code).unwrap_or_else(|| {
        warnings.push(format!("SUT texture mode {code} is approximated by Multiply"));
        CspTextureMode::Multiply
    }));
    Ok(shrink)
}
/// Codes linked through the property reader, ABR enum conversion and texture renderer dispatch.
/// See docs/sut-analysis.md; label order alone is not evidence for a serialized value.
const TEXTURE_MODES: &[(f64, CspTextureMode)] = &[
    (0.0, CspTextureMode::Normal),
    (1.0, CspTextureMode::Multiply),
    (2.0, CspTextureMode::Subtract),
    (3.0, CspTextureMode::Compare),
    (4.0, CspTextureMode::Outline),
    (5.0, CspTextureMode::Overlay),
    (6.0, CspTextureMode::ColorDodge),
    (7.0, CspTextureMode::ColorBurn),
    (8.0, CspTextureMode::HardMix),
    (9.0, CspTextureMode::Height),
];
fn texture_mode(code: f64) -> Option<CspTextureMode> {
    TEXTURE_MODES.iter().find(|(c, _)| *c == code).map(|(_, m)| *m)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tip_tests;
