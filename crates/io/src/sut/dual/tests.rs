//! Dual brush: mapping, unsupported settings and hostile values.
use super::super::material::tests::{c2f, database_sized, tar, tile_fixture_color};
use super::super::tests::reference_blob;
use super::super::{AbrImport, Row, read_tool};
use super::*;
use prism_paint::{CspDualAdd, MaskMode};

fn row(fields: &[(&str, Value)]) -> Row {
    fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn int(v: i64) -> Value {
    Value::Integer(v)
}
/// A 2×1 tip material at `path`.
fn material(path: &str) -> Row {
    let pixels = tar(&c2f(&database_sized(1, 1024, tile_fixture_color(2, 1, &[255, 128], &[[0, 0, 0]; 2]))));
    row(&[("OriginalPath", Value::Text(path.into())), ("FileData", Value::Blob(pixels))])
}
fn import_with(fields: &[(&str, Value)], materials: &dyn Fn() -> Result<Vec<Row>, String>) -> AbrImport {
    let node = row(&[("NodeName", Value::Text("Dual".into()))]);
    read_tool(&node, &row(fields), materials, "SUT", &prism_raster::Interrupt::NONE).unwrap()
}
fn import(fields: &[(&str, Value)]) -> AbrImport {
    import_with(fields, &|| Ok(Vec::new()))
}
fn dual(code: i64, more: &[(&str, Value)]) -> AbrImport {
    let mut fields = vec![("UseDualBrush", int(1)), ("DualBrushCompositeMode", int(code))];
    fields.extend(more.iter().cloned());
    import(&fields)
}

#[test]
fn masking_modes_run_with_a_round_second_tip() {
    for (code, mode) in [(1, MaskMode::Multiply), (3, MaskMode::Subtract), (4, MaskMode::Darken), (11, MaskMode::HardMix), (12, MaskMode::LinearHeight)] {
        let imported = dual(
            code,
            &[("DualSize", Value::Number(40.0)), ("DualHardness", int(50)), ("DualAutoIntervalType", int(0)), ("DualInterval", Value::Number(25.0))],
        );
        let d = &imported.presets[0].brush.dual_brush;
        assert!(d.enabled, "{code}");
        assert_eq!(d.mode, mode);
        assert_eq!(d.tip, TipShape::Round);
        assert_eq!((d.size, d.hardness, d.spacing), (40.0, 0.5, 0.25));
        assert!(imported.warnings.is_empty(), "{code}: {:?}", imported.warnings);
    }
}

#[test]
fn adding_modes_run_as_union_dual_brushes() {
    for (code, add) in [(0, CspDualAdd::Normal), (2, CspDualAdd::Add), (5, CspDualAdd::Lighten), (6, CspDualAdd::Screen)] {
        let imported = dual(code, &[("DualSize", Value::Number(30.0))]);
        let d = &imported.presets[0].brush.dual_brush;
        assert!(d.enabled, "{code}");
        assert_eq!(d.csp_add, Some(add), "{code}");
        assert_eq!(d.size, 30.0);
        assert!(imported.warnings.is_empty(), "{code}: {:?}", imported.warnings);
    }
    // Masking modes keep masking.
    assert_eq!(dual(1, &[]).presets[0].brush.dual_brush.csp_add, None);
}

#[test]
fn unknown_modes_leave_the_dual_brush_off() {
    for (code, warning) in [(13, "mode 13 is not supported"), (-4, "mode -4")] {
        let imported = dual(code, &[]);
        assert!(!imported.presets[0].brush.dual_brush.enabled, "{code}");
        assert!(imported.warnings.iter().any(|w| w.contains(warning)), "{code}: {:?}", imported.warnings);
    }
    assert!(!import(&[("DualBrushCompositeMode", int(1))]).presets[0].brush.dual_brush.enabled, "off without UseDualBrush");
}

#[test]
fn spray_thickness_and_rotation_map_onto_scatter_count_and_shape() {
    let imported = dual(
        1,
        &[
            ("DualSize", Value::Number(100.0)),
            ("DualUseSpray", int(1)),
            ("DualSpraySize", Value::Number(10.0)),
            ("DualSprayDensity", int(4)),
            ("DualThickness", int(50)),
            ("DualRotation", Value::Number(30.0)),
            ("DualRotationEffector", int(1)),
        ],
    );
    let d = &imported.presets[0].brush.dual_brush;
    assert_eq!((d.size, d.scatter, d.count, d.both_axes), (10.0, 10.0, 4, true));
    // Half the width: a half-round tip turned a quarter.
    assert_eq!((d.roundness, d.angle), (0.5, 60.0));
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    // A stretched vertical tip: twice as tall.
    let tall = dual(1, &[("DualSize", Value::Number(10.0)), ("DualThickness", int(200)), ("DualVerticalThicknes", int(1))]);
    let d = &tall.presets[0].brush.dual_brush;
    assert_eq!((d.size, d.roundness, d.angle), (20.0, 0.5, 90.0));
}

#[test]
fn unsupported_dual_settings_are_named_in_one_warning() {
    let imported = dual(
        1,
        &[
            ("DualFlow", int(20)),
            ("DualRibbon", int(1)),
            ("DualTextureImage", Value::Blob(vec![0; 8])),
            ("DualRotationEffector", int(0x81)),
            ("DualUseSpray", int(1)),
            ("DualSpraySize", Value::Number(1.0)),
            ("DualSize", Value::Number(500.0)),
            ("DualSprayDensity", int(400)),
            ("DualSprayBias", int(-100)),
        ],
    );
    let d = &imported.presets[0].brush.dual_brush;
    assert!(d.enabled);
    assert_eq!((d.scatter, d.count), (MAX_SCATTER, MAX_COUNT));
    let warning = imported.warnings.iter().find(|w| w.contains("dual brush settings")).expect("one warning");
    for part in ["flow", "ribbon", "texture", "tip rotation inputs", "spray deviation", "limited"] {
        assert!(warning.contains(part), "{part}: {warning}");
    }
}

#[test]
fn image_tips_are_read_and_broken_ones_keep_the_brush() {
    let materials = || Ok(vec![material("a.layer"), material("b.layer")]);
    let fields = [
        ("UseDualBrush", int(1)),
        ("DualBrushCompositeMode", int(1)),
        ("DualUsePatternImage", int(1)),
        ("DualPatternImageArray", Value::Blob(reference_blob(&["a.layer", "b.layer"]))),
    ];
    let imported = import_with(&fields, &materials);
    match &imported.presets[0].brush.dual_brush.tip {
        TipShape::Sequence(tips) => assert_eq!(tips.len(), 2),
        other => panic!("expected two tips, got {other:?}"),
    }
    let one = [
        ("UseDualBrush", int(1)),
        ("DualBrushCompositeMode", int(1)),
        ("DualUsePatternImage", int(1)),
        ("DualPatternImageArray", Value::Blob(reference_blob(&["a.layer"]))),
    ];
    assert!(matches!(import_with(&one, &materials).presets[0].brush.dual_brush.tip, TipShape::Sampled(_)));
    let missing = import_with(&one, &|| Ok(Vec::new()));
    assert!(!missing.presets[0].brush.dual_brush.enabled);
    assert!(missing.warnings.iter().any(|w| w.contains("dual brush tips could not be read")));
    let garbage =
        [("UseDualBrush", int(1)), ("DualBrushCompositeMode", int(1)), ("DualUsePatternImage", int(1)), ("DualPatternImageArray", Value::Blob(vec![1; 3]))];
    assert!(!import_with(&garbage, &materials).presets[0].brush.dual_brush.enabled);
}

#[test]
fn hostile_dual_values_stay_bounded() {
    for value in [Value::Number(f64::NAN), Value::Number(-1e300), Value::Number(1e300), int(i64::MIN), Value::Text("x".into())] {
        let fields: Vec<(&str, Value)> = [
            "DualSize",
            "DualHardness",
            "DualThickness",
            "DualRotation",
            "DualInterval",
            "DualAutoIntervalType",
            "DualSpraySize",
            "DualSprayDensity",
            "DualRotationEffector",
        ]
        .into_iter()
        .map(|k| (k, value.clone()))
        .chain([("DualUseSpray", int(1))])
        .collect();
        let imported = dual(1, &fields);
        let d = &imported.presets[0].brush.dual_brush;
        assert!(d.size.is_finite() && (0.5..=MAX_BRUSH_SIZE).contains(&d.size), "{value:?}: {}", d.size);
        assert!(d.scatter.is_finite() && (0.0..=MAX_SCATTER).contains(&d.scatter));
        assert!((1..=MAX_COUNT).contains(&d.count));
        assert!(d.roundness.is_finite() && d.roundness > 0.0 && d.roundness <= 1.0);
        assert!(d.spacing.is_finite() && d.spacing > 0.0);
        assert!(d.angle.is_finite());
    }
}
