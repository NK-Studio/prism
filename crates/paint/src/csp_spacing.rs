//! CSP dab spacing (Gap) as a fraction of the diameter, shared by SUT import and by editing an
//! imported brush's Gap. Fixed spacing (`BrushAutoIntervalType` 0) is `BrushInterval`; the
//! automatic types scale a per-type base by hardness and thickness. Evidence in
//! docs/sut-analysis.md.

/// The keys of one tip's spacing settings: the main tip's or the dual brush's.
#[derive(Clone, Copy, Debug)]
pub struct TipKeys {
    pub auto_interval: &'static str,
    pub interval: &'static str,
    pub use_image: &'static str,
    pub hardness: &'static str,
    pub thickness: &'static str,
}

impl TipKeys {
    pub const MAIN: Self = Self {
        auto_interval: "BrushAutoIntervalType",
        interval: "BrushInterval",
        use_image: "BrushUsePatternImage",
        hardness: "BrushHardness",
        thickness: "BrushThickness",
    };
    pub const DUAL: Self = Self {
        auto_interval: "DualAutoIntervalType",
        interval: "DualInterval",
        use_image: "DualUsePatternImage",
        hardness: "DualHardness",
        thickness: "DualThickness",
    };
}

/// The spacing of tip `k` from the stored settings `get` (missing values take CSP's defaults);
/// an untraced automatic type is approximated by the normal one and reported in `warnings`.
pub fn spacing(get: impl Fn(&str) -> Option<f64>, k: TipKeys, warnings: &mut Vec<String>) -> f32 {
    let num = |key: &str, default: f64| get(key).filter(|v| v.is_finite()).unwrap_or(default).clamp(-1e6, 1e6) as f32;
    let kind = num(k.auto_interval, 0.0) as i64;
    let base = match kind {
        0 => return (num(k.interval, 10.0) / 100.0).clamp(0.01, 10.0),
        1 => 0.12,
        3 => 0.04,
        2 => 0.08,
        _ => {
            warnings.push(format!("SUT automatic spacing type {kind} is approximated by the normal spacing"));
            0.08
        }
    };
    // Image tips space as fully hard ones.
    let hardness = if num(k.use_image, 0.0) != 0.0 { 1.0 } else { num(k.hardness, 100.0) / 100.0 };
    let hardness = if hardness >= 0.999_999_99 { 1.0 } else { hardness.max(0.2) };
    let thickness = num(k.thickness, 100.0) / 100.0;
    let thickness = if thickness < 1.0 { thickness.max(0.01) } else { 1.0 };
    let spacing = (base * (3.5 - 2.5 * hardness) * thickness).clamp(0.01, 10.0);
    if spacing.is_finite() { spacing } else { 0.1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn of(pairs: &[(&str, f64)]) -> f32 {
        let m: BTreeMap<&str, f64> = pairs.iter().copied().collect();
        spacing(|k| m.get(k).copied(), TipKeys::MAIN, &mut Vec::new())
    }

    #[test]
    fn fixed_and_automatic_types() {
        assert!((of(&[("BrushInterval", 25.0)]) - 0.25).abs() < 1e-6);
        assert!((of(&[("BrushAutoIntervalType", 1.0), ("BrushUsePatternImage", 1.0)]) - 0.12).abs() < 1e-6);
        assert!(of(&[("BrushAutoIntervalType", 3.0)]) < of(&[("BrushAutoIntervalType", 2.0)]));
        let mut w = Vec::new();
        assert!(spacing(|k| (k == "BrushAutoIntervalType").then_some(9.0), TipKeys::MAIN, &mut w).is_finite());
        assert_eq!(w.len(), 1);
        assert!(of(&[("BrushInterval", f64::NAN)]).is_finite());
        assert_eq!(of(&[("BrushInterval", 1e300)]), 10.0);
    }
}
