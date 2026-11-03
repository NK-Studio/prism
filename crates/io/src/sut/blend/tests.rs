//! Blending mode, colour jitter and brush options: mapping and hostile values.
use super::super::{AbrImport, Row, read_tool};
use super::*;
use prism_color::BlendMode;
use prism_paint::CspStrokeMode;

fn row(fields: &[(&str, Value)]) -> Row {
    fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn import(fields: &[(&str, Value)]) -> AbrImport {
    let node = row(&[("NodeName", Value::Text("Synthetic".into()))]);
    read_tool(&node, &row(fields), &|| Ok(Vec::new()), "SUT", &prism_raster::Interrupt::NONE).unwrap()
}
fn int(v: i64) -> Value {
    Value::Integer(v)
}

#[test]
fn constant_sub_color_is_imported_without_requiring_random_input() {
    for controls in [None, Some(effector(0, -100))] {
        let mut fields = vec![("BrushChangePatternColor", int(1)), ("BrushSubColor", int(25))];
        if let Some(controls) = controls {
            fields.push(("BrushSubColorEffector", controls));
        }
        let imported = import(&fields);
        let b = &imported.presets[0].brush;
        assert!(b.csp_correction);
        assert_eq!(b.csp_sub_color, Some(0.25));
        assert!(!imported.warnings.iter().any(|w| w.contains("sub color blend")));
    }
    let off = import(&[("BrushSubColor", int(25))]);
    assert_eq!(off.presets[0].brush.csp_sub_color, None);
    let erase = import(&[("CompositeMode", int(27)), ("BrushChangePatternColor", int(1)), ("BrushSubColor", int(25))]);
    assert_eq!(erase.presets[0].brush.csp_sub_color, None);
}
/// An effector blob with the `active` controls and the random minimum `random_min` %.
fn effector(active: u32, random_min: i32) -> Value {
    let mut blob = vec![0; 48];
    blob[..4].copy_from_slice(&48u32.to_be_bytes());
    blob[4..8].copy_from_slice(&0x1f0u32.to_be_bytes());
    blob[8..12].copy_from_slice(&active.to_be_bytes());
    blob[24..28].copy_from_slice(&random_min.to_be_bytes());
    Value::Blob(blob)
}

#[test]
fn blending_modes_map_onto_prism_modes_and_erasing() {
    let cases = [(2, BlendMode::Multiply, false), (10, BlendMode::ColorDodge, false), (26, BlendMode::Luminosity, false), (36, BlendMode::Divide, false)];
    for (code, mode, erase) in cases {
        let imported = import(&[("CompositeMode", int(code))]);
        let b = &imported.presets[0].brush;
        assert_eq!((b.mode, b.csp_erase), (mode, erase), "code {code}");
        assert!(!imported.warnings.iter().any(|w| w.contains("blending mode")), "{code}: {:?}", imported.warnings);
    }
    let imported = import(&[("CompositeMode", Value::Number(27.0))]);
    let b = &imported.presets[0].brush;
    assert!(b.csp_erase);
    assert_eq!(b.mode, BlendMode::Normal);
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
}

#[test]
fn approximated_and_csp_only_modes_warn() {
    for (code, mode, warning) in [
        (9, BlendMode::ColorDodge, "Color dodge is approximated"),
        (11, BlendMode::LinearDodge, "Add is approximated"),
        (30, BlendMode::Normal, "Through is not supported"),
        (32, BlendMode::Normal, "Black burn is not supported"),
        (33, BlendMode::Normal, "White burn is not supported"),
    ] {
        let imported = import(&[("CompositeMode", int(code))]);
        assert_eq!(imported.presets[0].brush.mode, mode, "code {code}");
        assert_eq!(imported.presets[0].brush.csp_stroke, None, "code {code}");
        assert!(imported.warnings.iter().any(|w| w.contains(warning)), "{code}: {:?}", imported.warnings);
    }
}

#[test]
fn csp_only_stroke_modes_run_without_warnings() {
    for (code, mode) in [(29, CspStrokeMode::Behind), (31, CspStrokeMode::ReplaceAlpha), (34, CspStrokeMode::CompareDensity), (35, CspStrokeMode::EraseCompare)]
    {
        let imported = import(&[("CompositeMode", int(code))]);
        let b = &imported.presets[0].brush;
        assert_eq!((b.mode, b.csp_stroke), (BlendMode::Normal, Some(mode)), "code {code}");
        // Erase (compare) erases with any tool, like Erase.
        assert_eq!(b.csp_erase, code == 35, "code {code}");
        assert!(imported.warnings.is_empty(), "{code}: {:?}", imported.warnings);
    }
    // CLIP STUDIO's builder resets the mode while colour mixing (other than smear) is on.
    let mixed = import(&[("CompositeMode", int(34)), ("BrushUseWaterColor", int(1)), ("BrushWaterColor", int(0))]);
    assert_eq!(mixed.presets[0].brush.csp_stroke, None);
}

#[test]
fn hostile_blending_mode_values_paint_normally() {
    for value in
        [int(-1), int(37), int(i64::MAX), Value::Number(2.5), Value::Number(f64::NAN), Value::Number(1e300), Value::Text("2".into()), Value::Blob(vec![2])]
    {
        let imported = import(&[("CompositeMode", value.clone())]);
        let b = &imported.presets[0].brush;
        assert_eq!((b.mode, b.csp_erase), (BlendMode::Normal, false), "{value:?}");
    }
    let imported = import(&[("CompositeMode", int(37))]);
    assert!(imported.warnings.iter().any(|w| w.contains("blending mode 37 is not supported")));
}

#[test]
fn colour_mixing_paints_in_normal_except_smear() {
    let mixed = |kind: i64| import(&[("CompositeMode", int(2)), ("BrushUseWaterColor", int(1)), ("BrushWaterColor", int(kind))]);
    assert_eq!(mixed(0).presets[0].brush.mode, BlendMode::Normal);
    assert_eq!(mixed(1).presets[0].brush.mode, BlendMode::Normal);
    assert_eq!(mixed(2).presets[0].brush.mode, BlendMode::Multiply);
}

#[test]
fn per_tip_colour_jitter_maps_its_random_spread() {
    let random = effector(0x80, -100);
    let imported = import(&[
        ("BrushChangePatternColor", int(1)),
        ("BrushHueChange", int(18)),
        ("BrushHueChangeEffector", random.clone()),
        ("BrushSaturationChange", int(-10)),
        ("BrushSaturationChangeEffector", random.clone()),
        ("BrushValueChange", int(25)),
        ("BrushValueChangeEffector", random.clone()),
        ("BrushSubColor", int(40)),
        ("BrushSubColorEffector", random),
    ]);
    let cd = &imported.presets[0].brush.color_dynamics;
    assert!(cd.enabled && cd.per_tip);
    // ±18° is a tenth of Prism's ±180° hue range.
    assert!((cd.hue_jitter - 0.1).abs() < 1e-6, "{}", cd.hue_jitter);
    assert!((cd.saturation_jitter - 0.1).abs() < 1e-6);
    assert!((cd.brightness_jitter - 0.25).abs() < 1e-6);
    assert!((cd.fg_bg.jitter - 0.4).abs() < 1e-6);
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
}

#[test]
fn colour_jitter_needs_its_switch_and_warns_what_it_cannot_run() {
    // Values without the switch do nothing (as CLIP STUDIO's builder).
    let off = import(&[("BrushHueChange", int(30)), ("BrushHueChangeEffector", effector(0x80, -100))]);
    assert!(!off.presets[0].brush.color_dynamics.enabled);
    // A constant shift (no random input) is dropped with a warning.
    let constant = import(&[("BrushChangePatternColor", int(1)), ("BrushValueChange", int(-3)), ("BrushValueChangeEffector", effector(0, -100))]);
    assert!(!constant.presets[0].brush.color_dynamics.enabled);
    assert!(constant.warnings.iter().any(|w| w.contains("constant")));
    // A one-sided random range keeps its width; pressure on top warns.
    let one_sided = import(&[("BrushChangePatternColor", int(1)), ("BrushHueChange", int(36)), ("BrushHueChangeEffector", effector(0x90, 0))]);
    let cd = &one_sided.presets[0].brush.color_dynamics;
    assert!((cd.hue_jitter - 0.1).abs() < 1e-6, "{}", cd.hue_jitter);
    assert!(one_sided.warnings.iter().any(|w| w.contains("one side")));
    assert!(one_sided.warnings.iter().any(|w| w.contains("other than random")));
    // Erasing brushes skip colour jitter.
    let erase = import(&[
        ("CompositeMode", int(27)),
        ("BrushChangePatternColor", int(1)),
        ("BrushHueChange", int(30)),
        ("BrushHueChangeEffector", effector(0x80, -100)),
    ]);
    assert!(!erase.presets[0].brush.color_dynamics.enabled);
}

#[test]
fn per_stroke_colour_jitter_varies_once_a_stroke() {
    let imported = import(&[("BrushChangeStrokeColor", int(1)), ("BrushStrokeHueChange", int(9)), ("BrushStrokeSaturationChange", int(-20))]);
    let cd = &imported.presets[0].brush.color_dynamics;
    assert!(cd.enabled && !cd.per_tip);
    assert!((cd.hue_jitter - 0.05).abs() < 1e-6);
    assert!((cd.saturation_jitter - 0.2).abs() < 1e-6);
    let both = import(&[
        ("BrushChangePatternColor", int(1)),
        ("BrushHueChange", int(18)),
        ("BrushHueChangeEffector", effector(0x80, -100)),
        ("BrushChangeStrokeColor", int(1)),
        ("BrushStrokeHueChange", int(9)),
        ("BrushChangeColorTarget", int(2)),
    ]);
    assert!(both.presets[0].brush.color_dynamics.per_tip);
    assert!(both.warnings.iter().any(|w| w.contains("per stroke and per brush tip")));
    assert!(both.warnings.iter().any(|w| w.contains("sub color only")));
}

#[test]
fn hostile_colour_jitter_values_stay_in_range() {
    let huge = effector(0x80, i32::MIN);
    let imported = import(&[
        ("BrushChangePatternColor", int(1)),
        ("BrushHueChange", Value::Number(1e300)),
        ("BrushHueChangeEffector", huge.clone()),
        ("BrushSaturationChange", Value::Number(f64::NAN)),
        ("BrushSaturationChangeEffector", Value::Blob(vec![0, 0, 0])),
        ("BrushValueChange", int(i64::MIN)),
        ("BrushValueChangeEffector", Value::Integer(5)),
        ("BrushChangeStrokeColor", int(1)),
        ("BrushStrokeHueChange", Value::Number(f64::INFINITY)),
    ]);
    let cd = &imported.presets[0].brush.color_dynamics;
    for j in [cd.hue_jitter, cd.saturation_jitter, cd.brightness_jitter, cd.fg_bg.jitter] {
        assert!((0.0..=1.0).contains(&j), "{j}");
    }
}

#[test]
fn continuous_spraying_view_scale_and_unsupported_options() {
    let imported = import(&[
        ("BrushContinuousPlot", int(1)),
        ("BrushSizeSyncViewScale", int(1)),
        ("BrushAtLeast1Pixel", int(1)),
        ("BrushUseRevision", int(1)),
        ("BrushRevision", int(10)),
        ("BrushUseReferLayer", int(1)),
    ]);
    let b = &imported.presets[0].brush;
    assert!(b.build_up && b.csp_size_by_view && b.csp_at_least_1px);
    assert!(imported.warnings.iter().any(|w| w.contains("post correction")));
    assert!(imported.warnings.iter().any(|w| w.contains("reference-layer")));
    // Post correction's value alone (off) is silent; so is a plain brush.
    let plain = import(&[("BrushRevision", int(10))]);
    assert!(!plain.presets[0].brush.build_up && !plain.presets[0].brush.csp_size_by_view && !plain.presets[0].brush.csp_at_least_1px);
    assert!(plain.warnings.is_empty(), "{:?}", plain.warnings);
}
