//! Brush blending mode, colour jitter and the smaller brush options of a sub-tool. Evidence and
//! inferences: docs/sut-analysis.md › Blending modes, colour jitter and dual brush.
use super::sqlite::Value;
use super::{Row, effector, enabled, num};
use prism_paint::BrushSettings;
use prism_paint::csp_blend::{CspBlend, csp_blend};

/// Colour mixing is on (`BrushUseWaterColor`, else `BrushUseWaterColor2`).
pub(super) fn mixing_on(r: &Row) -> bool {
    match r.get("BrushUseWaterColor").and_then(Value::number) {
        Some(v) => v != 0.0,
        None => enabled(r, "BrushUseWaterColor2"),
    }
}

/// Map the brush blending mode (`CompositeMode`, property 30) onto the brush's blend mode or
/// erasing. CLIP STUDIO's brush builder paints in Normal while colour mixing is on, except with
/// Smear (`BrushWaterColor` 2).
pub(super) fn blend_mode(r: &Row, b: &mut BrushSettings, warnings: &mut Vec<String>) {
    let Some(code) = r.get("CompositeMode").and_then(Value::number).filter(|c| *c != 0.0) else { return };
    if mixing_on(r) && num(r, "BrushWaterColor", 0.0) != 2.0 {
        return;
    }
    match csp_blend(code) {
        Some((_, CspBlend::Mode { mode, exact })) => {
            b.mode = mode;
            if !exact {
                warnings.push(format!("SUT blending mode {} is approximated by {}", label(code), mode.label()));
            }
        }
        Some((_, CspBlend::Erase { exact })) => {
            b.csp_erase = true;
            if !exact {
                warnings.push(format!("SUT blending mode {} is approximated by erasing", label(code)));
            }
        }
        // Background, Replace alpha, Compare density, Erase (compare): Prism's own compositing.
        Some((_, CspBlend::Stroke(mode))) => {
            b.csp_stroke = Some(mode);
            b.csp_erase = mode.erases();
        }
        Some((_, CspBlend::Unsupported)) | None => {
            warnings.push(format!("SUT blending mode {} is not supported; the brush paints in Normal", label(code)));
        }
    }
}

/// A blending mode code as CLIP STUDIO names it, or the number when it is not a known code.
fn label(code: f64) -> String {
    csp_blend(code).map_or_else(|| format!("{code}"), |(label, _)| label.to_string())
}

/// One colour jitter amount: `value` (a hue fraction of the wheel, or a saturation, luminosity
/// or sub-colour fraction) scaled per dab by its effector `key`. Returns the half-width of the
/// random spread (Prism's jitter is symmetric about the drawing colour).
fn jitter(r: &Row, value: f32, key: &str, what: &str, warnings: &mut Vec<String>) -> f32 {
    if value == 0.0 || !value.is_finite() {
        return 0.0;
    }
    let controls = r.get(key).and_then(Value::blob).map(effector::controls);
    let (active, minimum) = match controls {
        Some(Ok(c)) => c,
        // No effector: CLIP STUDIO applies the value as a constant shift.
        None | Some(Err(_)) => (0, 0),
    };
    if active & effector::RANDOM_CONTROL == 0 {
        warnings.push(format!(
            "SUT colour jitter {what} without the random input shifts the colour by a constant; Prism varies colours only randomly, so it is ignored"
        ));
        return 0.0;
    }
    if active & !effector::RANDOM_CONTROL != 0 {
        warnings.push(format!("SUT colour jitter {what} input controls other than random are not supported"));
    }
    // The factor is random between minimum % and 100 %: a spread of |value| × (1 − minimum).
    let minimum = minimum as f32 / 100.0;
    if minimum > -1.0 {
        warnings.push(format!("SUT colour jitter {what} with a random minimum above -100% varies to one side; Prism's jitter is centred on the colour"));
    }
    value.abs() * (1.0 - minimum) / 2.0
}

/// Map Color jitter onto Color Dynamics: per dab (`BrushChangePatternColor` 0x44e enables
/// `BrushHueChange` 0x44f, `BrushSaturationChange` 0x451, `BrushValueChange` 0x453 and
/// `BrushSubColor` 0x44d with their effectors) or per stroke (`BrushChangeStrokeColor` 0x4f6
/// and `BrushStroke*` 0x4f7–0x4fa). Hue is in degrees, the others in percent.
pub(super) fn color_jitter(r: &Row, b: &mut BrushSettings, warnings: &mut Vec<String>) {
    // CLIP STUDIO's brush builder skips colour jitter for the erasing modes.
    if b.csp_erase {
        return;
    }
    let cd = &mut b.color_dynamics;
    let mut any = false;
    if enabled(r, "BrushChangePatternColor") {
        let hue = jitter(r, num(r, "BrushHueChange", 0.0) / 360.0, "BrushHueChangeEffector", "hue", warnings);
        let saturation = jitter(r, num(r, "BrushSaturationChange", 0.0) / 100.0, "BrushSaturationChangeEffector", "saturation", warnings);
        let value = jitter(r, num(r, "BrushValueChange", 0.0) / 100.0, "BrushValueChangeEffector", "luminosity", warnings);
        let sub = jitter(r, num(r, "BrushSubColor", 0.0) / 100.0, "BrushSubColorEffector", "sub color blend", warnings);
        // Prism's hue jitter j spreads ±180°·j.
        cd.hue_jitter = (hue * 2.0).clamp(0.0, 1.0);
        cd.saturation_jitter = saturation.clamp(0.0, 1.0);
        cd.brightness_jitter = value.clamp(0.0, 1.0);
        // Prism blends towards the background colour by 0..j; CLIP STUDIO's blend is the amount
        // times a factor of -1..1, kept at 0 or more, so it reaches the same 0..amount.
        cd.fg_bg.jitter = sub.clamp(0.0, 1.0);
        cd.per_tip = true;
        any = [cd.hue_jitter, cd.saturation_jitter, cd.brightness_jitter, cd.fg_bg.jitter].iter().any(|j| *j > 0.0);
    }
    if enabled(r, "BrushChangeStrokeColor") {
        let amounts = [
            num(r, "BrushStrokeHueChange", 0.0) / 180.0,
            num(r, "BrushStrokeSaturationChange", 0.0) / 100.0,
            num(r, "BrushStrokeValueChange", 0.0) / 100.0,
            num(r, "BrushStrokeSubColor", 0.0) / 100.0,
        ]
        .map(|v| if v.is_finite() { v.abs().clamp(0.0, 1.0) } else { 0.0 });
        if amounts.iter().any(|v| *v > 0.0) {
            if any {
                warnings.push("SUT colour jitter per stroke and per brush tip together is not supported; the per-tip jitter is used".into());
            } else {
                let [hue, saturation, value, sub] = amounts;
                cd.hue_jitter = hue;
                cd.saturation_jitter = saturation;
                cd.brightness_jitter = value;
                cd.fg_bg.jitter = sub;
                cd.per_tip = false;
                any = true;
            }
        }
    }
    if !any {
        return;
    }
    cd.enabled = true;
    if num(r, "BrushChangeColorTarget", 1.0) == 2.0 {
        warnings.push("SUT colour jitter of the sub color only is approximated by varying the drawing color".into());
    }
}

/// Map the smaller brush options: continuous spraying (`BrushContinuousPlot` 0x413) onto
/// Build-up, size synchronized with the view scale (`BrushSizeSyncViewScale` 0x3ea), and warn
/// for post correction (`BrushUseRevision` 0x47e) and drawing beside reference-layer lines
/// (`BrushUseReferLayer` 0x4e2). The running-colour blur (`BrushBlur` 0x444) is part of colour
/// mixing type 1, which already warns.
pub(super) fn options(r: &Row, b: &mut BrushSettings, warnings: &mut Vec<String>) {
    if enabled(r, "BrushContinuousPlot") {
        b.build_up = true;
    }
    b.csp_size_by_view = enabled(r, "BrushSizeSyncViewScale");
    if enabled(r, "BrushUseRevision") && num(r, "BrushRevision", 0.0) > 0.0 {
        warnings.push("SUT post correction is not supported; strokes are not smoothed after drawing".into());
    }
    if enabled(r, "BrushUseReferLayer") {
        warnings.push("SUT drawing beside reference-layer lines is not supported (Prism has no reference layer)".into());
    }
}

#[cfg(test)]
mod tests;
