//! Bounded, Rust-only Clip Studio Paint `.sut` import.
//! Layout observations: CSP2PC (https://github.com/Leon-Schoenbrunn/CSP2PC),
//! plus locally inspected files. No converter code or brush assets are bundled.
mod sqlite;
use crate::abr_map::AbrImport;
use prism_paint::{BrushPreset, BrushSettings, Control, Dynamic, GrayTile, MaskMode, Pattern, TipShape};
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
/// Decode a material PNG into `(width, height, [luminance, alpha])` samples.
fn gray_alpha(blob: &[u8]) -> Result<(u32, u32, Vec<[f32; 2]>), String> {
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
    Ok((im.width(), im.height(), data))
}
fn tip(blob: &[u8]) -> Result<GrayTile, String> {
    let (width, height, px) = gray_alpha(blob)?;
    let data = px.iter().map(|[g, a]| ((1.0 - g) * a * 65535.0).round() as u16).collect();
    Ok(GrayTile { width, height, data })
}
/// Texture tiles keep luminance (white leaves paint untouched); transparency reads as white.
/// Returns the tile and the factor its larger edge was reduced by.
fn texture_tile(blob: &[u8]) -> Result<(GrayTile, f32), String> {
    let (w, h, px) = gray_alpha(blob)?;
    let gray = px.iter().map(|[g, a]| g * a + (1.0 - a)).collect();
    let (nw, nh, gray) = crate::abr_map::fit(w, h, gray, crate::abr_map::MAX_PATTERN_EDGE);
    let shrink = w.max(h) as f32 / nw.max(nh).max(1) as f32;
    let data = gray.iter().map(|x| (x.clamp(0.0, 1.0) * 65535.0).round() as u16).collect();
    Ok((GrayTile { width: nw, height: nh, data }, shrink))
}
/// Materials named by a CSP reference array (`u32` header size, `u32` count, UTF-16LE paths),
/// in reference order. Every declared reference must resolve.
fn referenced<'a>(refs: &[u8], materials: &'a [Row], what: &str) -> Result<Vec<&'a Row>, String> {
    let mut ordered = Vec::new();
    for mat in materials {
        let Some(path) = mat.get("OriginalPath").and_then(Value::text) else { continue };
        let encoded: Vec<u8> = path.encode_utf16().flat_map(u16::to_le_bytes).collect();
        if encoded.is_empty() {
            continue;
        }
        if let Some(pos) = refs.windows(encoded.len()).position(|w| w == encoded) {
            ordered.push((pos, mat));
        }
    }
    ordered.sort_by_key(|(pos, _)| *pos);
    if ordered.is_empty() || ordered.len() > 64 {
        return Err(format!("SUT has no readable referenced {what} (or more than 64)"));
    }
    let declared = refs.get(4..8).and_then(|v| <[u8; 4]>::try_from(v).ok()).map(u32::from_be_bytes).ok_or(format!("invalid SUT {what} reference header"))?;
    if declared as usize != ordered.len() {
        return Err(format!("some referenced SUT {what} are missing"));
    }
    Ok(ordered.into_iter().map(|(_, m)| m).collect())
}
/// Observed effector header: bit 0x10 enables pen pressure.
fn pressure_effector(r: &Row, k: &str) -> bool {
    r.get(k).and_then(Value::blob).and_then(|e| e.get(4..8)).and_then(|v| <[u8; 4]>::try_from(v).ok()).is_some_and(|v| u32::from_be_bytes(v) & 0x10 != 0)
}

/// Read one sub-tool into a preset, retaining all referenced primary tips in file order.
/// Unsupported behaviour is reported through warnings; unreadable tips fail the entire import.
pub fn read_sut(bytes: &[u8], group: &str, ctl: &prism_raster::Interrupt) -> Result<AbrImport, String> {
    ctl.check().map_err(|e| e.to_string())?;
    let db = Database::new(bytes)?;
    let nodes = db.table("Node")?;
    let node = nodes.iter().find(|r| r.get("NodeName").and_then(Value::text).is_some()).ok_or("SUT has no named sub-tool")?;
    let variants = db.table("Variant")?;
    let variant_id = num(node, "NodeVariantID", -1.0);
    let r = variants.iter().find(|r| num(r, "VariantID", -2.0) == variant_id).ok_or("SUT current brush settings are missing")?;
    let name = node.get("NodeName").and_then(Value::text).filter(|n| !n.trim().is_empty()).unwrap_or(group).to_string();
    if name.chars().count() > 255 {
        return Err("SUT brush name exceeds 255 characters".into());
    }
    let mut b = BrushSettings {
        size: num(r, "BrushSize", 20.0).clamp(0.5, prism_paint::MAX_BRUSH_SIZE),
        opacity: (num(r, "Opacity", 100.0) / 100.0).clamp(0.0, 1.0),
        flow: (num(r, "BrushFlow", 100.0) / 100.0).clamp(0.0, 1.0),
        hardness: (num(r, "BrushHardness", 100.0) / 100.0).clamp(0.0, 1.0),
        spacing: (num(r, "BrushInterval", 10.0) / 100.0).clamp(0.01, 10.0),
        roundness: (num(r, "BrushThickness", 100.0) / 100.0).clamp(0.01, 1.0),
        angle: -num(r, "BrushRotation", 0.0),
        pressure_size: false,
        flip_x: enabled(r, "BrushPatternReverseHorizontal"),
        flip_y: enabled(r, "BrushPatternReverseVertical"),
        ..Default::default()
    };
    let mut warnings = vec!["SUT pressure curves, automatic spacing and stabilisation are approximated; CSP rendering parity is not guaranteed".into()];
    // Custom curves remain a warning.
    b.pressure_size = pressure_effector(r, "BrushSizeEffector");
    if num(r, "BrushRotationEffector", 0.0) == 3.0 {
        b.shape_dynamics.enabled = true;
        b.shape_dynamics.angle = Dynamic::controlled(Control::Direction);
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
        let materials = db.table("MaterialFile")?;
        let ordered = referenced(refs, &materials, "tips")?;
        let mut tips = Vec::new();
        let mut pixels = 0usize;
        for mat in ordered {
            ctl.check().map_err(|e| e.to_string())?;
            let t = tip(mat.get("FileData").and_then(Value::blob).ok_or("SUT tip material is missing")?)?;
            pixels = pixels.checked_add(t.data.len()).filter(|n| *n <= 16_000_000).ok_or("SUT tips exceed pixel budget")?;
            tips.push(t);
        }
        b.tip = if tips.len() == 1 { TipShape::Sampled(tips.remove(0)) } else { TipShape::Sequence(tips) };
        if num(r, "BrushPatternOrderType", 0.0) != 0.0 {
            warnings.push("SUT tip ordering is approximated by cyclic order".into());
        }
    }
    for k in [
        "BrushUseWaterColor",
        "BrushUseWaterColor2",
        "UseDualBrush",
        "BrushUseSpray",
        "BrushRibbon",
        "BrushUseIn",
        "BrushUseOut",
        "BrushUseWaterEdge",
        "BrushChangePatternColor",
    ] {
        if enabled(r, k) {
            warnings.push(format!("SUT {k} is not supported"));
        }
    }
    match r.get("TextureImage") {
        None | Some(Value::Null) => {}
        Some(v) => {
            ctl.check().map_err(|e| e.to_string())?;
            // A broken texture drops the texture, not the brush.
            if let Err(e) = texture(&db, r, v.blob().ok_or("SUT texture reference is invalid")?, &mut b, &mut warnings) {
                warnings.push(format!("SUT texture could not be read ({e}); the brush is imported without it"));
            }
        }
    }
    ctl.progress(1.0);
    Ok(AbrImport { presets: vec![BrushPreset { name, group: group.into(), brush: b, builtin: false }], warnings, version: 0 })
}

/// Map the Texture category onto [`prism_paint::Texture`] (CSP manual: density, invert,
/// emphasize density, scale, rotation, brightness, contrast, mode, apply by each plot).
fn texture(db: &Database, r: &Row, refs: &[u8], b: &mut BrushSettings, warnings: &mut Vec<String>) -> Result<(), String> {
    let materials = db.table("MaterialFile")?;
    let mats = referenced(refs, &materials, "textures")?;
    if mats.len() > 1 {
        warnings.push("SUT brush references several textures; only the first is used".into());
    }
    let mat = mats.first().ok_or("SUT texture is missing")?;
    let (tile, shrink) = texture_tile(mat.get("FileData").and_then(Value::blob).ok_or("SUT texture material is missing")?)?;
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
    if pressure_effector(r, "TextureDensityEffector") {
        if t.each_tip {
            t.depth_jitter = Dynamic::controlled(Control::PenPressure);
        } else {
            warnings.push("SUT pen-pressure texture density needs Apply by each plot; it is ignored".into());
        }
    }
    if num(r, "TextureRotate", 0.0) != 0.0 {
        warnings.push("SUT texture rotation is not supported".into());
    }
    let code = num(r, "TextureCompositeMode", 0.0);
    t.mode = texture_mode(code).unwrap_or_else(|| {
        warnings.push(format!("SUT texture mode {code} is approximated by Multiply"));
        MaskMode::Multiply
    });
    Ok(())
}
/// Stored `TextureCompositeMode` codes confirmed against CSP's Texture mode menu. The codes don't
/// follow the menu order, so only verified ones are listed; others fall back with a warning.
const TEXTURE_MODES: &[(f32, MaskMode)] = &[];
fn texture_mode(code: f32) -> Option<MaskMode> {
    TEXTURE_MODES.iter().find(|(c, _)| *c == code).map(|(_, m)| *m)
}

#[cfg(test)]
mod tests;
