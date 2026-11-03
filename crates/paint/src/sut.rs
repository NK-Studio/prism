//! Imported Clip Studio sub-tool presentation, independent of brush rendering.
use crate::csp_gradient::CspGradient;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Defensive presentation limit, not a claimed CSP format limit.
pub const MAX_SUT_DISPLAY_ITEMS: usize = 512;

/// Verified property IDs grouped for presentation. This is not an availability
/// declaration for every input/output operator. Compound controls can use
/// additional source fields (for example water-edge enable and radius units).
pub const PARAMETER_KEYS: &[(u32, &str)] = &[
    (1000, "BrushSize"),
    (10, "Opacity"),
    (30, "CompositeMode"),
    (1081, "BrushMixColor"),
    (1083, "BrushMixAlpha"),
    (1085, "BrushMixColorExtension"),
    (20, "AntiAlias"),
    (1002, "BrushSizeSyncViewScale"),
    (1020, "BrushFlow"),
    (1030, "BrushHardness"),
    (1060, "BrushRotation"),
    (1131, "BrushSpraySize"),
    (1133, "BrushSprayDensity"),
    (1052, "BrushVerticalThicknes"),
    (1071, "BrushPatternOrderType"),
    (1102, "BrushChangePatternColor"),
    (1103, "BrushHueChange"),
    (1105, "BrushSaturationChange"),
    (1107, "BrushValueChange"),
    (1130, "BrushUseSpray"),
    (1135, "BrushSprayBias"),
    (1061, "BrushRotationInSpray"),
    (1040, "BrushInterval"),
    (1042, "BrushAutoIntervalType"),
    (1050, "BrushThickness"),
    (100, "TextureImage"),
    (110, "TextureDensity"),
    (120, "TextureScale2"),
    (1201, "BrushWaterEdgeRadius"),
    (40, "FlickerReduction"),
    (41, "FlickerReductionBySpeed"),
    (42, "FlickerReductionBySpeedType"),
    (1241, "BrushVectorMagnetPower"),
];

/// CSP automatic spacing (`BrushAutoIntervalType` 1 wide, 2 normal, 3 narrow) as a fraction of
/// the diameter: base 0.12 / 0.08 / 0.04 times `3.5 − 2.5·hardness` (hardness at least 0.2,
/// counted as 1 for image tips and from 0.99999999) and, below 1, times the thickness. `None`
/// for type 0 (the fixed `BrushInterval`) and other types. Evidence: docs/sut-analysis.md ›
/// Automatic spacing; the wide/normal/narrow names follow the bases, they are not traced.
pub fn auto_spacing(kind: i64, hardness: f32, image_tip: bool, thickness: f32) -> Option<f32> {
    let base = match kind {
        1 => 0.12,
        2 => 0.08,
        3 => 0.04,
        _ => return None,
    };
    let hardness = if image_tip || hardness >= 0.999_999_99 { 1.0 } else { hardness.max(0.2) };
    let thickness = if thickness < 1.0 { thickness.max(0.01) } else { 1.0 };
    Some((base * (3.5 - 2.5 * hardness) * thickness).clamp(0.01, 10.0))
}

/// The automatic spacing type (1–3) whose spacing is `spacing`, else 0 (fixed).
pub fn auto_spacing_kind(spacing: f32, hardness: f32, image_tip: bool, thickness: f32) -> i64 {
    (1..=3).find(|k| auto_spacing(*k, hardness, image_tip, thickness).is_some_and(|s| (s - spacing).abs() < 1e-6)).unwrap_or(0)
}

pub fn is_known_parameter(id: u32) -> bool {
    PARAMETER_KEYS.iter().any(|(known, _)| *known == id)
}

/// Imported scalar values kept beside [`PARAMETER_KEYS`]: the extra sources of compound and
/// conditional brush controls. The settings of the other families (fill, auto select, selection,
/// gradient, figure, text, eyedropper, operation) are the numeric keys of the Tool Property
/// catalogue ([`crate::sut_catalog`]). Nothing else is persisted.
pub const RETAINED_KEYS: &[&str] = &[
    "BrushSizeUnit",
    "BrushVerticalThicknes",
    "BrushUseWaterColor",
    "BrushUseWaterColor2",
    "BrushColorMixingMode",
    "BrushUseWaterEdge",
    "BrushWaterEdgeRadiusUnit",
    "TextureCompositeMode",
    "TextureReverseDensity",
    "TextureStressDensity",
    "TextureForPlot",
    "PrismTextureScaleFactor",
];

/// The largest imported setting value kept (see [`crate::sut_catalog::MAX_SETTING_VALUE`]).
pub use crate::sut_catalog::MAX_SETTING_VALUE;

/// Every key an imported sub-tool keeps: [`PARAMETER_KEYS`], [`RETAINED_KEYS`] and the Tool
/// Property catalogue's numeric keys (some appear in more than one).
pub fn retained_keys() -> impl Iterator<Item = &'static str> {
    PARAMETER_KEYS.iter().map(|(_, key)| *key).chain(RETAINED_KEYS.iter().copied()).chain(crate::sut_catalog::numeric_keys())
}

/// Whether an imported value named `key` is kept (see [`retained_keys`]).
pub fn is_retained_key(key: &str) -> bool {
    retained_keys().any(|known| known == key)
}

/// The longest imported text value kept, in characters (font names are far shorter).
pub const MAX_SETTING_CHARS: usize = 256;

/// Whether an imported text value named `key` is kept (see [`crate::sut_catalog::string_keys`]).
pub fn is_string_key(key: &str) -> bool {
    crate::sut_catalog::string_keys().any(|known| known == key)
}

/// An imported text value as kept: control characters removed, trimmed, at most
/// [`MAX_SETTING_CHARS`] characters.
pub fn clean_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_SETTING_CHARS).collect::<String>().trim().to_string()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SutDisplay {
    /// Imported order is retained; known controls render in canonical order.
    pub visible: Vec<u32>,
    pub show_categories: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SutBrushMetadata {
    pub original: SutDisplay,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<SutDisplay>,
    pub input_op: i64,
    pub output_op: i64,
    /// CSP's tool icon for the sub-tool (`NodeIcon`); 0 when absent. It tells apart sub-tools that
    /// share an operation (brush, airbrush, decoration and blend all paint with output 10).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub node_icon: i64,
    /// Allowlisted imported scalar values, including unsupported/disabled rows.
    pub parameters: BTreeMap<String, f64>,
    pub texture_name: String,
    /// Allowlisted imported text values (font names, [`crate::sut_catalog::string_keys`]), each
    /// at most [`MAX_SETTING_CHARS`] characters.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub strings: BTreeMap<String, String>,
    /// The gradient of a gradient sub-tool (`GradationData`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<CspGradient>,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl SutBrushMetadata {
    pub fn effective(&self) -> &SutDisplay {
        self.custom.as_ref().unwrap_or(&self.original)
    }

    /// The CSP tool family this sub-tool belongs to (from its output operation).
    pub fn kind(&self) -> SutToolKind {
        SutToolKind::classify(self.input_op, self.output_op)
    }
}

/// The family of a CSP sub-tool, read from its node's output operation. The table comes from the
/// sub-tools CSP 5.0.4 ships (`Settings/PAINT/Tool/*/EditImageTool.todb`, grouped under their
/// tools); see docs/sut-analysis.md "Sub-tool operation IDs".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SutToolKind {
    /// Output 10: brush, airbrush, decoration, sketch and blend sub-tools.
    Brush,
    /// Output 11: pens, pencils and markers.
    Pen,
    /// Output 12: erasers.
    Eraser,
    /// Output 13: copy stamp.
    CopyStamp,
    /// Output 14: dot pen.
    DotPen,
    /// Output 20: selection (marquee, lasso, selection pen, auto select).
    Selection,
    /// Outputs 31, 50, 51, 53, 54: fill (bucket, enclose and fill, leftover, gradient-group fills).
    Fill,
    /// Output 52: gradient.
    Gradient,
    /// Outputs 30, 32, 33: figures (shapes, lines, curves).
    Figure,
    /// Output 70: text.
    Text,
    /// Outputs 40, 42, 45: operation (move layer, select layer, object).
    Operation,
    /// Output 41: eyedropper.
    Eyedropper,
    /// Outputs 60-65: comic frames, balloons and effect lines.
    Comic,
    /// Outputs 80-85: vector line correction.
    CorrectLine,
    /// Outputs 90-93: rulers and guides.
    Ruler,
    /// Outputs 1-3: zoom, hand, rotate.
    View,
    /// No operation stored (older or synthetic files): treated as a brush, unverified.
    Unknown,
    /// Any other operation (liquify, light table, timeline, unseen values).
    Other,
}

impl SutToolKind {
    pub fn classify(input_op: i64, output_op: i64) -> Self {
        match output_op {
            0 if input_op == 0 => Self::Unknown,
            10 => Self::Brush,
            11 => Self::Pen,
            12 => Self::Eraser,
            13 => Self::CopyStamp,
            14 => Self::DotPen,
            20 => Self::Selection,
            31 | 50 | 51 | 53 | 54 => Self::Fill,
            52 => Self::Gradient,
            30 | 32 | 33 => Self::Figure,
            70 => Self::Text,
            40 | 42 | 45 => Self::Operation,
            41 => Self::Eyedropper,
            60..=65 => Self::Comic,
            80..=85 => Self::CorrectLine,
            90..=93 => Self::Ruler,
            1..=3 => Self::View,
            _ => Self::Other,
        }
    }

    /// Paints with a brush tip, so it belongs in the Photoshop-style brush library.
    pub fn is_brush(self) -> bool {
        matches!(self, Self::Brush | Self::Pen | Self::Eraser | Self::CopyStamp | Self::DotPen | Self::Unknown)
    }

    /// A verified brush family whose tool properties Prism maps (`Unknown` is not verified).
    pub fn is_verified_brush(self) -> bool {
        self.is_brush() && self != Self::Unknown
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Brush => "Brush",
            Self::Pen => "Pen",
            Self::Eraser => "Eraser",
            Self::CopyStamp => "Copy stamp",
            Self::DotPen => "Dot pen",
            Self::Selection => "Selection",
            Self::Fill => "Fill",
            Self::Gradient => "Gradient",
            Self::Figure => "Figure",
            Self::Text => "Text",
            Self::Operation => "Operation",
            Self::Eyedropper => "Eyedropper",
            Self::Comic => "Comic",
            Self::CorrectLine => "Correct line",
            Self::Ruler => "Ruler",
            Self::View => "View",
            Self::Unknown => "Unknown",
            Self::Other => "Other",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushPreset, BrushSettings};

    #[test]
    fn preset_origin_and_display_override_round_trip_without_render_changes() {
        let legacy = serde_json::json!({ "name": "Old", "brush": BrushSettings::default() });
        let mut preset: BrushPreset = serde_json::from_value(legacy).unwrap();
        assert!(preset.sut.is_none());
        let brush = preset.brush.clone();
        let original = SutDisplay { visible: vec![1000, 10, 0xfedcba98], show_categories: true };
        preset.sut = Some(SutBrushMetadata { original: original.clone(), ..Default::default() });
        preset.sut.as_mut().unwrap().custom = Some(SutDisplay::default());
        let copy: BrushPreset = serde_json::from_str(&serde_json::to_string(&preset).unwrap()).unwrap();
        assert_eq!(copy, preset);
        assert!(copy.sut.as_ref().unwrap().effective().visible.is_empty());
        preset.sut.as_mut().unwrap().custom = None;
        assert_eq!(preset.sut.as_ref().unwrap().effective(), &original);
        assert_eq!(preset.brush, brush);
    }
}

#[cfg(test)]
mod spacing_tests {
    use super::*;

    #[test]
    fn automatic_spacing_types_follow_hardness_and_thickness() {
        // Type 2 (normal), hardness 0.5: 0.08 · (3.5 − 1.25) = 0.18; thickness 0.5 halves it.
        assert!((auto_spacing(2, 0.5, false, 1.0).unwrap() - 0.18).abs() < 1e-6);
        assert!((auto_spacing(2, 0.5, false, 0.5).unwrap() - 0.09).abs() < 1e-6);
        // Image tips and full hardness space as hard tips; hardness is at least 0.2.
        assert!((auto_spacing(1, 0.1, true, 1.0).unwrap() - 0.12).abs() < 1e-6);
        assert_eq!(auto_spacing(3, 0.0, false, 1.0), auto_spacing(3, 0.2, false, 1.0));
        assert_eq!((auto_spacing(0, 1.0, false, 1.0), auto_spacing(7, 1.0, false, 1.0)), (None, None));
        // The shown type is the one the spacing matches; anything else is fixed.
        for kind in 1..=3 {
            let s = auto_spacing(kind, 0.7, false, 0.8).unwrap();
            assert_eq!(auto_spacing_kind(s, 0.7, false, 0.8), kind);
        }
        assert_eq!(auto_spacing_kind(0.25, 0.7, false, 0.8), 0);
        // Hostile values stay finite and bounded.
        for v in [f32::NAN, f32::INFINITY, -1e30] {
            let s = auto_spacing(2, v, false, v).unwrap();
            assert!(s.is_finite() && (0.01..=10.0).contains(&s), "{v}: {s}");
        }
        assert!(is_known_parameter(1040) && is_known_parameter(1042));
    }
}

#[cfg(test)]
mod kind_tests {
    use super::*;

    #[test]
    fn shipped_operation_ids_classify_into_tool_families() {
        let k = SutToolKind::classify;
        assert_eq!(
            [k(10, 10), k(11, 11), k(12, 12), k(13, 13), k(14, 14)],
            [SutToolKind::Brush, SutToolKind::Pen, SutToolKind::Eraser, SutToolKind::CopyStamp, SutToolKind::DotPen]
        );
        // The output decides: a selection pen draws freehand but makes a selection.
        assert_eq!(k(10, 20), SutToolKind::Selection);
        assert_eq!([k(50, 50), k(31, 51), k(10, 51), k(50, 54), k(31, 31)], [SutToolKind::Fill; 5]);
        assert_eq!(
            [k(52, 52), k(30, 30), k(70, 70), k(40, 40), k(41, 41)],
            [SutToolKind::Gradient, SutToolKind::Figure, SutToolKind::Text, SutToolKind::Operation, SutToolKind::Eyedropper]
        );
        assert_eq!(
            [k(30, 61), k(10, 83), k(32, 90), k(2, 2), k(120, 120), k(0, 0)],
            [SutToolKind::Comic, SutToolKind::CorrectLine, SutToolKind::Ruler, SutToolKind::View, SutToolKind::Other, SutToolKind::Unknown]
        );
        assert!(k(11, 11).is_verified_brush() && k(0, 0).is_brush() && !k(0, 0).is_verified_brush());
        assert!(!k(50, 50).is_brush() && !k(i64::MIN, i64::MAX).is_brush());
    }
}
