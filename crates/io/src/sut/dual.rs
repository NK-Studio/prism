//! Dual brush (`UseDualBrush` 1910, `DualBrushCompositeMode` and `Dual*` 50000–51141) onto
//! Prism's Dual Brush. Evidence and inferences: docs/sut-analysis.md › Blending modes, colour
//! jitter and dual brush.
use super::sqlite::Value;
use super::{Row, TipKeys, effector, enabled, material, referenced, spacing_of, tip};
use prism_paint::csp_blend::{CspDualMode, csp_dual_mode};
use prism_paint::{BrushSettings, DualBrush, MAX_BRUSH_SIZE, MAX_SCATTER, TipShape};

/// A setting's number, or `default` when it is missing or not finite.
fn num(r: &Row, k: &str, default: f64) -> f32 {
    Some(super::num(r, k, default)).filter(|v| v.is_finite()).unwrap_or(default as f32)
}

/// Pixels the dual brush's tips may hold together.
const DUAL_TIP_PIXEL_BUDGET: usize = 4_000_000;
/// Prism's dual brush lays at most this many tips a step.
const MAX_COUNT: u32 = 16;

/// Map the dual brush onto `b.dual_brush` (masking modes onto `mode`, adding modes onto
/// `csp_add`). A mode Prism can't run, or tips that can't be read,
/// leave the dual brush off with a warning; the brush itself is kept.
pub(super) fn dual_brush(
    r: &Row,
    materials: &dyn Fn() -> Result<Vec<Row>, String>,
    ctl: &prism_raster::Interrupt,
    b: &mut BrushSettings,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    if !enabled(r, "UseDualBrush") {
        return Ok(());
    }
    let code = r.get("DualBrushCompositeMode").and_then(Value::number).unwrap_or(0.0);
    let (mode, csp_add) = match csp_dual_mode(code) {
        Some((_, CspDualMode::Mask(mode))) => (mode, None),
        // The second brush adds its opacity outside the first.
        Some((_, CspDualMode::Adds(add))) => (DualBrush::default().mode, Some(add)),
        None => {
            warnings.push(format!("SUT dual brush mode {code} is not supported; the dual brush is not applied"));
            return Ok(());
        }
    };
    let mut d = DualBrush { enabled: true, mode, csp_add, ..DualBrush::default() };
    let refs = match r.get("DualPatternImageArray") {
        None | Some(Value::Null) => None,
        Some(v) => v.blob(),
    };
    match refs.filter(|_| enabled(r, "DualUsePatternImage")) {
        Some(refs) => match dual_tips(refs, materials, ctl, warnings) {
            Ok(tip) => d.tip = tip,
            Err(e) => {
                ctl.check().map_err(|e| e.to_string())?;
                warnings.push(format!("SUT dual brush tips could not be read ({e}); the dual brush is not applied"));
                return Ok(());
            }
        },
        None => d.hardness = (num(r, "DualHardness", 100.0) / 100.0).clamp(0.0, 1.0),
    }
    d.size = num(r, "DualSize", 20.0).clamp(0.5, MAX_BRUSH_SIZE);
    d.spacing = spacing_of(r, TipKeys::DUAL, warnings);
    // Thickness scales the tip's width (its height when vertical); Prism's roundness squashes
    // the height of a `size` tip, so a squashed width turns the tip a quarter.
    let thickness = (num(r, "DualThickness", 100.0) / 100.0).clamp(0.01, 10.0);
    let vertical = enabled(r, "DualVerticalThicknes");
    d.angle = -num(r, "DualRotation", 0.0);
    let flags = r.get("DualRotationEffector").and_then(Value::integer).unwrap_or(1);
    if flags & 1 == 0 {
        d.angle = 0.0;
    }
    let (roundness, turned) = if thickness <= 1.0 { (thickness, !vertical) } else { (1.0 / thickness, vertical) };
    if thickness > 1.0 {
        d.size = (d.size * thickness).min(MAX_BRUSH_SIZE);
    }
    d.roundness = roundness;
    if turned {
        d.angle += 90.0;
    }
    // Flip (0 off, otherwise on; Prism flips the dual tip at random).
    d.flip = num(r, "DualPatternReverseHorizontal", 0.0) != 0.0;
    let mut unsupported = Vec::new();
    if enabled(r, "DualUseSpray") {
        // The dual size is the spray area; its particles are `DualSpraySize`.
        let particle = num(r, "DualSpraySize", 2.0).clamp(0.5, MAX_BRUSH_SIZE);
        let area = d.size;
        d.size = particle;
        d.both_axes = true;
        let scatter = area / particle;
        if scatter > MAX_SCATTER {
            unsupported.push("spray area over 10 particle sizes (limited)");
        }
        d.scatter = scatter.clamp(0.0, MAX_SCATTER);
        let count = num(r, "DualSprayDensity", 1.0).round().max(1.0);
        if count > MAX_COUNT as f32 {
            unsupported.push("spray density over 16 (limited)");
        }
        d.count = (count as u32).min(MAX_COUNT);
        if num(r, "DualSprayBias", 0.0) != 0.0 {
            unsupported.push("spray deviation");
        }
        if enabled(r, "DualSprayUseFixedPoint") {
            unsupported.push("fixed spray");
        }
    }
    for (key, what) in [("DualSizeUnit", "size units"), ("DualSpraySizeUnit", "particle size units")] {
        if enabled(r, key) {
            unsupported.push(what);
        }
    }
    if num(r, "DualFlow", 100.0) < 100.0 {
        unsupported.push("flow");
    }
    let dynamic = |key: &str| r.get(key).and_then(Value::blob).is_some_and(effector::is_dynamic);
    if ["DualSizeEffector", "DualFlowEffector", "DualIntervalEffector", "DualThicknessEffector", "DualSpraySizeEffector", "DualSprayDensityEffector"]
        .into_iter()
        .any(dynamic)
    {
        unsupported.push("pen input and random dynamics");
    }
    if flags & 0xf0 != 0 {
        unsupported.push("tip rotation inputs");
    }
    if !matches!(r.get("DualTextureImage"), None | Some(Value::Null)) {
        unsupported.push("texture");
    }
    for (key, what) in [
        ("DualRibbon", "ribbon"),
        ("DualContinuousPlot", "continuous spraying"),
        ("DualBlendPatternByDarken", "blend tips with darken"),
        ("DualPatternReverseVertical", "vertical flip"),
    ] {
        if enabled(r, key) {
            unsupported.push(what);
        }
    }
    if matches!(d.tip, TipShape::Sequence(_)) && num(r, "DualPatternOrderType", 0.0) != 0.0 {
        unsupported.push("tip order");
    }
    if !unsupported.is_empty() {
        warnings.push(format!("SUT dual brush settings are not supported and are ignored: {}", unsupported.join(", ")));
    }
    b.dual_brush = d;
    Ok(())
}

/// The dual brush's tips in file order (their colours are not used: the dual brush masks or adds
/// opacity in the brush colour).
fn dual_tips(
    refs: &[u8],
    materials: &dyn Fn() -> Result<Vec<Row>, String>,
    ctl: &prism_raster::Interrupt,
    warnings: &mut Vec<String>,
) -> Result<TipShape, String> {
    let materials = materials()?;
    let ordered = referenced(refs, &materials, "dual brush tips")?;
    let each = (DUAL_TIP_PIXEL_BUDGET / ordered.len().max(1)).min(material::DEFAULT_MAX_PIXELS);
    let mut tips = Vec::with_capacity(ordered.len());
    let mut pixels = 0usize;
    for mat in ordered {
        ctl.check().map_err(|e| e.to_string())?;
        let (t, _) = tip(mat.get("FileData").and_then(Value::blob).ok_or("SUT dual brush tip material is missing")?, each, warnings)?;
        pixels = pixels.checked_add(t.data.len()).filter(|n| *n <= DUAL_TIP_PIXEL_BUDGET).ok_or("SUT dual brush tips exceed pixel budget")?;
        tips.push(t);
    }
    match (tips.pop(), tips.is_empty()) {
        (Some(only), true) => Ok(TipShape::Sampled(only)),
        (Some(last), false) => {
            tips.push(last);
            Ok(TipShape::Sequence(tips))
        }
        (None, _) => Err("SUT dual brush has no tips".into()),
    }
}

#[cfg(test)]
mod tests;
