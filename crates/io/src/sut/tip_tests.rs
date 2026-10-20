//! SUT tip order, colour tips and tip rotation input: mapping and hostile values.

use super::material::tests::{c2f, database_sized, tar, tile_fixture_color};
use super::tests::{leaf, number, reference_blob, text};
use super::*;

/// A sub-tool with two colour tips (`first.layer` red and blue, `second.layer` white) and the
/// variant `fields`.
fn tipped(fields: &[(&str, Value)]) -> AbrImport {
    tipped_with(fields, [[0, 0, 255], [255, 0, 0]])
}

/// [`tipped`] with the first tip's two pixels in blue, green, red order.
fn tipped_with(fields: &[(&str, Value)], first: [[u8; 3]; 2]) -> AbrImport {
    let mut db = vec![0; 4 * 4096];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&4096u16.to_be_bytes());
    db[56..60].copy_from_slice(&1u32.to_be_bytes());
    let columns: String = fields.iter().map(|(key, _)| format!(", {key} BLOB")).collect();
    leaf(
        &mut db,
        1,
        &[
            vec![text("table"), text("Node"), text("Node"), number(2.0), text("CREATE TABLE Node (NodeName TEXT, NodeVariantID INTEGER)")],
            vec![
                text("table"),
                text("Variant"),
                text("Variant"),
                number(3.0),
                text(&format!("CREATE TABLE Variant (VariantID INTEGER, BrushUsePatternImage INTEGER, BrushPatternImageArray BLOB{columns})")),
            ],
            vec![text("table"), text("MaterialFile"), text("MaterialFile"), number(4.0), text("CREATE TABLE MaterialFile (OriginalPath TEXT, FileData BLOB)")],
        ],
    );
    leaf(&mut db, 2, &[vec![text("Colour tips"), Value::Integer(1)]]);
    let paths = ["first.layer", "second.layer"];
    let mut row = vec![Value::Integer(1), Value::Integer(1), Value::Blob(reference_blob(&paths))];
    row.extend(fields.iter().map(|(_, v)| v.clone()));
    leaf(&mut db, 3, &[row]);
    // A tip with a half-transparent pixel, then a white one.
    let red = tar(&c2f(&database_sized(1, 1024, tile_fixture_color(2, 1, &[255, 128], &first))));
    let white = tar(&c2f(&database_sized(1, 1024, tile_fixture_color(2, 1, &[255, 255], &[[255, 255, 255]; 2]))));
    leaf(&mut db, 4, &[vec![text(paths[0]), Value::Blob(red)], vec![text(paths[1]), Value::Blob(white)]]);
    read_sut(&db, "SUT", &prism_raster::Interrupt::NONE).unwrap()
}

#[test]
fn colour_tips_keep_their_colours_unless_the_brush_changes_them() {
    let imported = tipped(&[]);
    let b = &imported.presets[0].brush;
    let colors: Vec<_> = b.csp_tip_colors.iter().map(|c| c.as_ref().map(|c| c.rgb.clone())).collect();
    assert_eq!(colors, [Some(vec![255, 0, 0, 0, 0, 255]), Some(vec![255; 6])]);
    let TipShape::Sequence(tips) = &b.tip else { panic!("sequence expected") };
    assert_eq!(tips[0].data, [65535, 32896], "alpha is the shape");
    assert!(!imported.warnings.iter().any(|w| w.contains("colour")), "{:?}", imported.warnings);
    // Change brush tip colour paints them in the brush colour, as Prism's gray tips.
    assert!(tipped(&[("BrushChangePatternColor", Value::Integer(1))]).presets[0].brush.csp_tip_colors.is_empty());
    // Colour mixing lays a tip's mean colour, which only matters for several colours; a ribbon
    // keeps every pixel's colour.
    let mixed = tipped(&[("BrushUseWaterColor", Value::Integer(1))]);
    assert!(mixed.presets[0].brush.csp_tip_colors.len() == 2 && mixed.warnings.iter().any(|w| w == COLOUR_TIP), "{:?}", mixed.warnings);
    let single = tipped_with(&[("BrushUseWaterColor", Value::Integer(1))], [[255, 255, 255]; 2]);
    assert!(!single.warnings.iter().any(|w| w == COLOUR_TIP), "{:?}", single.warnings);
    let ribbon = tipped(&[("BrushRibbon", Value::Integer(1))]);
    assert!(ribbon.presets[0].brush.csp_ribbon.is_some() && !ribbon.warnings.iter().any(|w| w == COLOUR_TIP), "{:?}", ribbon.warnings);
}

#[test]
fn tip_order_maps_every_csp_order_and_prefers_the_newer_key() {
    let order = |fields: &[(&str, Value)]| {
        let imported = tipped(fields);
        (imported.presets[0].brush.csp_tip_order, imported.warnings.iter().any(|w| w.contains("tip order")))
    };
    assert_eq!(order(&[]), (None, false));
    assert_eq!(order(&[("BrushPatternOrderType", Value::Integer(0))]), (None, false));
    assert_eq!(order(&[("BrushPatternOrderType", Value::Integer(1))]), (Some(CspTipOrder::Reverse), false));
    assert_eq!(order(&[("BrushPatternOrderType", Value::Integer(2))]), (Some(CspTipOrder::Once), false));
    assert_eq!(order(&[("BrushPatternOrderType", Value::Integer(3))]), (Some(CspTipOrder::Random), false));
    // CLIP STUDIO reads `BrushPatternOrderType2` when it is stored.
    assert_eq!(order(&[("BrushPatternOrderType", Value::Integer(1)), ("BrushPatternOrderType2", Value::Integer(3))]), (Some(CspTipOrder::Random), false));
    assert_eq!(order(&[("BrushPatternOrderType", Value::Integer(2)), ("BrushPatternOrderType2", Value::Null)]), (Some(CspTipOrder::Once), false));
    // Hostile or unknown values repeat the tips and warn; nothing panics.
    for v in [Value::Integer(4), Value::Integer(-1), Value::Integer(i64::MAX), number(1.5), number(f64::NAN), text("1"), Value::Blob(vec![1])] {
        assert_eq!(order(&[("BrushPatternOrderType2", v.clone())]), (None, true), "{v:?}");
    }
}

#[test]
fn tip_rotation_follows_the_selected_pen_or_line_angle() {
    let rotation = |flags: i64| {
        let imported = tipped(&[("BrushRotationEffector", Value::Integer(flags)), ("BrushRotationRandomScale", Value::Integer(50))]);
        let b = imported.presets[0].brush.clone();
        (b.shape_dynamics.angle.control, b.shape_dynamics.angle.jitter, imported.warnings)
    };
    // 19 Gouache, pencil: the pen's direction; 35 flat brushes: its axis rotation; 67 and 195
    // decorations, crayon: the line direction (195 also turns at random).
    assert_eq!(rotation(19).0, Control::PenTilt);
    assert_eq!(rotation(35).0, Control::Rotation);
    assert_eq!(rotation(67).0, Control::Direction);
    let (control, jitter, _) = rotation(195);
    assert_eq!((control, jitter), (Control::Direction, 0.25));
    for (flags, control) in [(0x51, Control::PenTilt), (0x61, Control::Rotation)] {
        assert_eq!(rotation(flags).0, control, "0x10/0x20 take precedence over 0x40");
    }
    let (control, _, warnings) = rotation(0x31);
    assert!(control == Control::PenTilt && warnings.iter().any(|w| w.contains("pen rotation")), "{warnings:?}");
    // Without the enable bit nothing turns; no flag value panics or warns about rotation input.
    for flags in [0x40, 0x70, -1, i64::MIN, i64::MAX] {
        let (control, _, warnings) = rotation(flags);
        assert!(!warnings.iter().any(|w| w.contains("rotation by pen input")), "{flags}: {warnings:?}");
        if flags & 1 == 0 {
            assert_eq!(control, Control::Off);
        }
    }
}
