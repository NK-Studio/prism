//! Independently decoded SUT presentation framing; see docs/sut-analysis.md.
use super::{Row, Value};
use prism_paint::csp_gradient::CspGradient;
use prism_paint::sut::{MAX_SETTING_VALUE, MAX_SUT_DISPLAY_ITEMS, SutBrushMetadata, SutDisplay, SutToolKind, clean_text, is_known_parameter, retained_keys};
use prism_paint::sut_catalog;

fn visible(bytes: &[u8]) -> Result<Vec<u32>, &'static str> {
    let word = |at: usize| -> Result<u32, &'static str> {
        let b: [u8; 4] = bytes.get(at..at.saturating_add(4)).and_then(|b| b.try_into().ok()).ok_or("truncated header")?;
        Ok(u32::from_be_bytes(b))
    };
    if word(0)? != 12 || word(8)? != 4 {
        return Err("unsupported header or element width");
    }
    let count = usize::try_from(word(4)?).map_err(|_| "invalid item count")?;
    if count > MAX_SUT_DISPLAY_ITEMS || count.checked_mul(4).and_then(|n| n.checked_add(12)) != Some(bytes.len()) {
        return Err("invalid item count or length");
    }
    bytes
        .get(12..)
        .ok_or("missing payload")?
        .chunks_exact(4)
        .map(|chunk| {
            let b: [u8; 4] = chunk.try_into().map_err(|_| "invalid item")?;
            Ok(u32::from_be_bytes(b))
        })
        .collect()
}

pub(super) fn read(node: &Row, row: &Row, warnings: &mut Vec<String>, notes: &mut Vec<String>) -> SutBrushMetadata {
    let parsed = row.get("VariantShowParam").and_then(Value::blob).ok_or("missing or non-blob display metadata").and_then(visible);
    let visible = match parsed {
        Ok(ids) => ids,
        Err(error) => {
            // Only Tool Property's layout is affected (it falls back to basic controls).
            notes.push(format!("SUT display metadata could not be read ({error}); showing basic brush size and opacity"));
            vec![1000, 10]
        }
    };
    let (input_op, output_op) = (node.get("NodeInputOp").and_then(Value::integer).unwrap_or(0), node.get("NodeOutputOp").and_then(Value::integer).unwrap_or(0));
    // Brush-type sub-tools edit the brush items; the others draw the Tool Property catalogue.
    let brush = SutToolKind::classify(input_op, output_op).is_brush();
    let catalogued = |id: u32| !brush && (sut_catalog::item(id).is_some() || sut_catalog::owner(id).is_some());
    let unknown_count = visible.iter().filter(|id| !is_known_parameter(**id) && !catalogued(**id)).count();
    if unknown_count != 0 {
        notes.push(format!("SUT display metadata contains {unknown_count} unrecognized parameter IDs; they are preserved but have no editable controls"));
    }
    let show_categories = match row.get("VariantShowSeparator") {
        None | Some(Value::Null) => false,
        Some(v) => match v.integer() {
            Some(0) => false,
            Some(1) => true,
            _ => {
                warnings.push("SUT category display flag is invalid; category headings are disabled".into());
                false
            }
        },
    };
    let mut parameters = std::collections::BTreeMap::new();
    // Never copy arbitrary columns into persisted UI metadata. Compound values and
    // conditional controls need these explicit additional source flags/units.
    for key in retained_keys().filter(|key| *key != "TextureImage") {
        if parameters.contains_key(key) {
            continue;
        }
        if let Some(value) = row.get(key).and_then(Value::number) {
            if value.abs() <= MAX_SETTING_VALUE {
                parameters.insert(key.to_string(), value);
            } else {
                warnings.push(format!("SUT display value {key} exceeds the supported range and was omitted"));
            }
        }
    }
    // Font names are text (UTF-8 text, or a UTF-8 blob); anything else is reported and skipped.
    let mut strings = std::collections::BTreeMap::new();
    for key in sut_catalog::string_keys() {
        let text = match row.get(key) {
            None | Some(Value::Null) => continue,
            Some(Value::Text(t)) => Some(t.as_str()),
            Some(Value::Blob(b)) => std::str::from_utf8(b).ok(),
            Some(_) => None,
        };
        match text.map(clean_text) {
            Some(t) if !t.is_empty() => {
                strings.insert(key.to_string(), t);
            }
            Some(_) => {}
            None => warnings.push(format!("SUT text value {key} is not text and was omitted")),
        }
    }
    let gradient = match row.get("GradationData") {
        None | Some(Value::Null) => None,
        Some(v) => match v.blob().ok_or("not a blob").and_then(CspGradient::decode) {
            Ok(g) => Some(g),
            Err(error) => {
                warnings.push(format!("SUT gradient could not be read ({error}); Prism's current gradient is used"));
                None
            }
        },
    };
    SutBrushMetadata {
        original: SutDisplay { visible, show_categories },
        custom: None,
        input_op,
        output_op,
        node_icon: node.get("NodeIcon").and_then(Value::integer).unwrap_or(0),
        parameters,
        // Encoded material catalogue titles are not available to this reader.
        texture_name: String::new(),
        strings,
        gradient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(ids: &[u32]) -> Value {
        Value::Blob([12, ids.len() as u32, 4].into_iter().chain(ids.iter().copied()).flat_map(u32::to_be_bytes).collect())
    }

    #[test]
    fn display_visibility_categories_unknowns_and_empty_are_independent() {
        let node = Row::from([("NodeInputOp".into(), Value::Integer(10)), ("NodeOutputOp".into(), Value::Integer(11))]);
        let mut row = Row::from([
            ("VariantShowParam".into(), blob(&[1000, 10, 0xf1234567])),
            ("VariantShowSeparator".into(), Value::Integer(1)),
            ("BrushUseWaterColor".into(), Value::Integer(0)),
            ("BrushMixColor".into(), Value::Integer(80)),
            ("ArbitraryColumn".into(), Value::Integer(7)),
        ]);
        let (mut warnings, mut notes) = (Vec::new(), Vec::new());
        let m = read(&node, &row, &mut warnings, &mut notes);
        assert_eq!(m.original.visible, [1000, 10, 0xf1234567]);
        assert!(m.original.show_categories);
        assert_eq!((m.input_op, m.output_op), (10, 11));
        assert_eq!(m.parameters.get("BrushMixColor"), Some(&80.0));
        assert_eq!(m.parameters.get("BrushUseWaterColor"), Some(&0.0));
        assert!(!m.parameters.contains_key("ArbitraryColumn"));
        // Unknown ids are kept; that needs no action, so it is a note, not a warning.
        assert!(warnings.is_empty());
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("unrecognized parameter IDs"));
        notes.clear();
        row.insert("VariantShowParam".into(), blob(&[]));
        assert!(read(&node, &row, &mut warnings, &mut notes).original.visible.is_empty());
        assert!(warnings.is_empty() && notes.is_empty());
    }

    #[test]
    fn operator_identity_is_preserved_without_inferring_control_availability() {
        let row = Row::from([
            ("VariantShowParam".into(), blob(&[10])),
            ("Opacity".into(), Value::Integer(90)),
            ("BrushSize".into(), Value::Integer(15)),
            ("BrushUseWaterColor".into(), Value::Integer(1)),
        ]);
        for (input, output) in [(10, 10), (11, 11), (99, 10), (10, 99), (i64::MAX, i64::MIN)] {
            let node = Row::from([("NodeInputOp".into(), Value::Integer(input)), ("NodeOutputOp".into(), Value::Integer(output))]);
            let m = read(&node, &row, &mut Vec::new(), &mut Vec::new());
            assert_eq!((m.input_op, m.output_op), (input, output));
            assert_eq!(m.original.visible, [10], "stored scalar columns are not available/visible control declarations");
            assert_eq!(m.parameters.get("BrushUseWaterColor"), Some(&1.0));
        }
        let m = read(&Row::new(), &row, &mut Vec::new(), &mut Vec::new());
        assert_eq!((m.input_op, m.output_op), (0, 0), "missing operators must not be guessed as a supported brush family");
    }

    #[test]
    fn other_families_keep_their_tool_property_settings() {
        // A text sub-tool: catalogued items are not "unrecognized"; 32-bit colour channels are kept.
        let node = Row::from([("NodeInputOp".into(), Value::Integer(70)), ("NodeOutputOp".into(), Value::Integer(70))]);
        let row = Row::from([
            ("VariantShowParam".into(), blob(&[7000, 7001, 7039, 7025])),
            ("TextFontSize".into(), Value::Integer(30)),
            ("TextEdgeColorRed".into(), Value::Integer(4_294_967_295)),
            ("TextAlignment".into(), Value::Integer(2)),
            ("TextFontName".into(), Value::Null),
            ("FillCloseGap".into(), Value::Integer(5)),
        ]);
        let (mut warnings, mut notes) = (Vec::new(), Vec::new());
        let m = read(&node, &row, &mut warnings, &mut notes);
        assert!(warnings.is_empty() && notes.is_empty(), "{warnings:?} {notes:?}");
        assert_eq!(m.parameters.get("TextEdgeColorRed"), Some(&4_294_967_295.0));
        assert_eq!(m.parameters.get("TextAlignment"), Some(&2.0));
        assert_eq!(m.parameters.get("FillCloseGap"), Some(&5.0));
        assert!(!m.parameters.contains_key("TextFontName"));
        // The same IDs on a brush are not brush controls.
        let pen = Row::from([("NodeInputOp".into(), Value::Integer(11)), ("NodeOutputOp".into(), Value::Integer(11))]);
        read(&pen, &row, &mut warnings, &mut notes);
        assert!(notes.iter().any(|n| n.contains("4 unrecognized")), "{notes:?}");
    }

    #[test]
    fn font_names_and_gradients_are_read() {
        let node = Row::from([("NodeInputOp".into(), Value::Integer(70)), ("NodeOutputOp".into(), Value::Integer(70))]);
        let stop = |rgb: u32, flags: u32, pos: u32| [rgb, rgb, rgb, u32::MAX, flags, pos, 0];
        let gradient: Vec<u8> = [24u32, 28, 2, 16, 1, 3].into_iter().chain(stop(0, 1, 0)).chain(stop(u32::MAX, 0, 0x8000)).flat_map(u32::to_be_bytes).collect();
        let row = Row::from([
            ("VariantShowParam".into(), blob(&[7000, 7014])),
            ("TextFontName".into(), Value::Text(" 源ノ角ゴシック JP\u{0} ".into())),
            ("TextFontUniqueName".into(), Value::Blob("SourceHanSansJP-Regular".into())),
            ("TextRubyFontName".into(), Value::Text("   ".into())),
            ("GradationData".into(), Value::Blob(gradient.clone())),
        ]);
        let (mut warnings, mut notes) = (Vec::new(), Vec::new());
        let m = read(&node, &row, &mut warnings, &mut notes);
        assert!(warnings.is_empty() && notes.is_empty(), "{warnings:?} {notes:?}");
        assert_eq!(m.strings.get("TextFontName").map(String::as_str), Some("源ノ角ゴシック JP"));
        assert_eq!(m.strings.get("TextFontUniqueName").map(String::as_str), Some("SourceHanSansJP-Regular"));
        assert!(!m.strings.contains_key("TextRubyFontName"), "blank names are not kept");
        assert_eq!(m.gradient.as_ref().map(|g| g.stops.len()), Some(2));
        assert_eq!(m.gradient.as_ref().map(|g| g.stops[0].color), Some(prism_paint::csp_gradient::CspStopColor::Foreground));
        // The metadata round-trips through the preset JSON.
        let copy: SutBrushMetadata = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(copy, m);
        // Bad values are reported and skipped, never a failure.
        let row = Row::from([
            ("TextFontName".into(), Value::Integer(3)),
            ("TextFontUniqueName".into(), Value::Blob(vec![0xff, 0xfe])),
            ("GradationData".into(), Value::Blob(gradient[..40].to_vec())),
        ]);
        let mut warnings = Vec::new();
        let m = read(&node, &row, &mut warnings, &mut Vec::new());
        assert!(m.strings.is_empty() && m.gradient.is_none());
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn malformed_display_falls_back_without_losing_brush_data() {
        for value in [
            Value::Null,
            Value::Integer(12),
            Value::Blob(vec![]),
            blob(&vec![10; MAX_SUT_DISPLAY_ITEMS + 1]),
            Value::Blob([12u32, u32::MAX, 4].into_iter().flat_map(u32::to_be_bytes).collect()),
            Value::Blob([12u32, 1, 8, 10].into_iter().flat_map(u32::to_be_bytes).collect()),
            Value::Blob([12u32, 0, 4, 10].into_iter().flat_map(u32::to_be_bytes).collect()),
        ] {
            let row = Row::from([("VariantShowParam".into(), value), ("Opacity".into(), Value::Integer(90))]);
            let mut notes = Vec::new();
            let m = read(&Row::new(), &row, &mut Vec::new(), &mut notes);
            assert_eq!(m.original.visible, [1000, 10]);
            assert_eq!(m.parameters.get("Opacity"), Some(&90.0));
            assert!(!notes.is_empty());
        }
    }
}
