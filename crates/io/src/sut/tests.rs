use super::*;
use prism_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};

// Synthetic SQLite fixtures built from the public spec; no third-party brush assets.
pub(super) fn vi(mut n: u64) -> Vec<u8> {
    let mut v = vec![(n & 127) as u8];
    n >>= 7;
    while n > 0 {
        v.push((n & 127) as u8 | 128);
        n >>= 7;
    }
    v.reverse();
    v
}
/// A record; `utf16` stores text as UTF-16LE (database text encoding 2).
pub(super) fn rec_with(values: &[Value], utf16: bool) -> Vec<u8> {
    let mut types = Vec::new();
    let mut body = Vec::new();
    for v in values {
        let t = match v {
            Value::Null => 0,
            Value::Integer(v) => {
                body.extend_from_slice(&v.to_be_bytes());
                6
            }
            Value::Number(v) => {
                body.extend_from_slice(&v.to_be_bytes());
                7
            }
            Value::Text(s) => {
                let bytes: Vec<u8> = if utf16 { s.encode_utf16().flat_map(u16::to_le_bytes).collect() } else { s.as_bytes().to_vec() };
                body.extend_from_slice(&bytes);
                13 + bytes.len() as u64 * 2
            }
            Value::Blob(b) => {
                body.extend_from_slice(b);
                12 + b.len() as u64 * 2
            }
        };
        types.extend(vi(t));
    }
    let mut size = types.len() + 1;
    while size != types.len() + vi(size as u64).len() {
        size = types.len() + vi(size as u64).len();
    }
    let mut out = vi(size as u64);
    out.extend(types);
    out.extend(body);
    out
}
pub(super) fn leaf(db: &mut Vec<u8>, id: usize, rows: &[Vec<Value>]) {
    leaf_with(db, id, rows, false);
}
pub(super) fn leaf_with(db: &mut Vec<u8>, id: usize, rows: &[Vec<Value>], utf16: bool) {
    const P: usize = 4096;
    let mut cells = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let r = rec_with(row, utf16);
        let mut cell = vi(r.len() as u64);
        cell.extend(vi(i as u64 + 1));
        let min = (P - 12) * 32 / 255 - 23;
        let max = P - 35;
        let local = if r.len() <= max {
            r.len()
        } else {
            let k = min + (r.len() - min) % (P - 4);
            if k <= max { k } else { min }
        };
        cell.extend_from_slice(&r[..local]);
        if local < r.len() {
            let first = db.len() / P + 1;
            cell.extend_from_slice(&(first as u32).to_be_bytes());
            let chunks: Vec<_> = r[local..].chunks(P - 4).collect();
            for (j, chunk) in chunks.iter().enumerate() {
                let start = db.len();
                db.resize(start + P, 0);
                let next = if j + 1 == chunks.len() { 0 } else { (first + j + 1) as u32 };
                db[start..start + 4].copy_from_slice(&next.to_be_bytes());
                db[start + 4..start + 4 + chunk.len()].copy_from_slice(chunk);
            }
        }
        cells.push(cell);
    }
    let start = (id - 1) * P;
    let p = &mut db[start..start + P];
    let h = if id == 1 { 100 } else { 0 };
    p[h] = 13;
    p[h + 3..h + 5].copy_from_slice(&(rows.len() as u16).to_be_bytes());
    let mut at = P;
    for (i, cell) in cells.iter().enumerate() {
        at -= cell.len();
        p[at..at + cell.len()].copy_from_slice(cell);
        p[h + 8 + i * 2..h + 10 + i * 2].copy_from_slice(&(at as u16).to_be_bytes());
    }
    p[h + 5..h + 7].copy_from_slice(&(at as u16).to_be_bytes());
}
pub(super) fn text(s: &str) -> Value {
    Value::Text(s.into())
}
pub(super) fn number(n: f64) -> Value {
    Value::Number(n)
}
fn fixture() -> Vec<u8> {
    fixture_with(true, None)
}
pub(super) fn tool_fixture() -> Vec<u8> {
    let mut db = vec![0u8; 8192];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&4096u16.to_be_bytes());
    db[56..60].copy_from_slice(&1u32.to_be_bytes());
    leaf(&mut db, 1, &[vec![text("table"), text("Material"), text("Material"), number(2.0), text("CREATE TABLE Material (MaterialToolVariantManager BLOB)")]]);
    leaf(&mut db, 2, &[vec![Value::Blob(fixture_with(false, None))]]);
    crate::csp_container::plain_fixture(&db)
}
#[test]
fn downloaded_tool_reads_nested_sqlite_without_scanning_for_headers() {
    let tool = read_sut(&tool_fixture(), "Test", &prism_raster::Interrupt::NONE).unwrap();
    let sut = read_sut(&fixture_with(false, None), "Test", &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(tool.presets, sut.presets);
}
#[test]
fn downloaded_tool_reads_tips_from_its_tool_pattern_table() {
    // Downloaded `.tool` containers keep tip packages in the outer `ToolPatternInMaterial`
    // (`RelativePath`, `FileData`), not in a `MaterialFile` table of the sub-tool database.
    let sut = fixture();
    let tips: Vec<Vec<Value>> = Database::new(&sut)
        .unwrap()
        .table("MaterialFile")
        .unwrap()
        .into_iter()
        .map(|r| vec![r["OriginalPath"].clone(), r["FileData"].clone(), text("unused")])
        .collect();
    let mut inner = sut.clone();
    let schema = &mut inner[..4096];
    for at in (0..schema.len() - 12).rev() {
        if &schema[at..at + 12] == b"MaterialFile" {
            schema[at..at + 12].copy_from_slice(b"MaterialFilX");
        }
    }
    let mut db = vec![0u8; 3 * 4096];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&4096u16.to_be_bytes());
    db[56..60].copy_from_slice(&1u32.to_be_bytes());
    leaf(
        &mut db,
        1,
        &[
            vec![text("table"), text("Material"), text("Material"), number(2.0), text("CREATE TABLE Material (MaterialToolVariantManager BLOB)")],
            vec![
                text("table"),
                text("ToolPatternInMaterial"),
                text("ToolPatternInMaterial"),
                number(3.0),
                text("CREATE TABLE ToolPatternInMaterial (RelativePath TEXT, FileData BLOB, MaterialUuid TEXT)"),
            ],
        ],
    );
    leaf(&mut db, 2, &[vec![Value::Blob(inner.clone())]]);
    leaf(&mut db, 3, &tips);
    let tool = read_sut(&crate::csp_container::plain_fixture(&db), "Test", &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(tool.presets, read_sut(&sut, "Test", &prism_raster::Interrupt::NONE).unwrap().presets, "the same tips as the SUT's own table");
    // Without either table the tips are still required.
    let err = read_sut(&inner, "Test", &prism_raster::Interrupt::NONE).unwrap_err();
    assert!(err.contains("MaterialFile"), "{err}");
}
#[test]
#[ignore = "reads PRISM_TEST_CSP_TOOL, a local material never bundled"]
fn local_downloaded_tool_imports_without_a_cache() {
    let path = std::env::var_os("PRISM_TEST_CSP_TOOL").unwrap();
    let before = std::fs::read(&path).unwrap();
    let imp = read_sut(&before, "Local", &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(imp.presets.len(), 1);
    assert!(imp.presets[0].sut.is_some());
    assert_eq!(std::fs::read(path).unwrap(), before);
    eprintln!(
        "{}: {:?}, size={}, {} warnings",
        imp.presets[0].name,
        imp.presets[0].sut.as_ref().unwrap().kind(),
        imp.presets[0].brush.size,
        imp.warnings.len()
    );
}
/// `texture`: the reference blob for TextureImage (NULL when `None`).
fn fixture_with(with_refs: bool, texture: Option<Vec<u8>>) -> Vec<u8> {
    fixture_with_texture_value(with_refs, texture.map_or(Value::Null, Value::Blob))
}
fn fixture_with_texture_value(with_refs: bool, texture: Value) -> Vec<u8> {
    fixture_with_effectors(with_refs, texture, Value::Null, Value::Null)
}
fn fixture_with_effectors(with_refs: bool, texture: Value, size_effector: Value, density_effector: Value) -> Vec<u8> {
    fixture_with_all_effectors(with_refs, texture, [size_effector, density_effector, Value::Null, Value::Null])
}
fn fixture_with_all_effectors(with_refs: bool, texture: Value, effectors: [Value; 4]) -> Vec<u8> {
    let [size_effector, density_effector, opacity_effector, flow_effector] = effectors;
    let mut db = vec![0; 4 * 4096];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&4096u16.to_be_bytes());
    db[56..60].copy_from_slice(&1u32.to_be_bytes());
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
                text(
                    "CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL, BrushUsePatternImage INTEGER, BrushPatternImageArray BLOB, BrushInterval REAL, TextureImage BLOB, TextureScale2 REAL, TextureReverseDensity INTEGER, TextureForPlot INTEGER, TextureDensity INTEGER, TextureCompositeMode INTEGER, BrushSizeEffector BLOB, TextureDensityEffector BLOB, BrushOpacityEffector BLOB, BrushFlowEffector BLOB)",
                ),
            ],
            vec![text("table"), text("MaterialFile"), text("MaterialFile"), number(4.0), text("CREATE TABLE MaterialFile (OriginalPath TEXT, FileData BLOB)")],
        ],
    );
    leaf(&mut db, 2, &[vec![text("Synthetic SUT"), number(818.0)]]);
    let paths = ["first.layer", "second.layer"];
    let refs = reference_blob(&paths);
    let refs = if with_refs { Value::Blob(refs) } else { Value::Null };
    leaf(
        &mut db,
        3,
        &[vec![
            number(818.0),
            number(17.0),
            number(1.0),
            refs,
            number(10.0),
            texture,
            number(50.0),
            number(1.0),
            number(1.0),
            number(40.0),
            number(2.0),
            size_effector,
            density_effector,
            opacity_effector,
            flow_effector,
        ]],
    );
    let mut rows = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        // Include black, white and transparent pixels; second tip is inverse of the first.
        let pixels = if i == 0 {
            vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 255]
        } else {
            vec![255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 0]
        };
        let im = Image::from_raw(2, 2, ChannelLayout::Rgba, SampleType::U8, pixels).unwrap();
        let mut blob = vec![0; 9000]; // Exercise SQLite overflow chains.
        blob.extend(prism_codecs::encode(&im, Format::Png, &EncodeOptions::default()).unwrap());
        rows.push(vec![text(p), Value::Blob(blob)]);
    }
    leaf(&mut db, 4, &rows);
    db
}
#[test]
fn imports_referenced_tips_in_order_and_maps_masks() {
    let f = fixture();
    let imp = read_sut(&f, "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let p = &imp.presets[0];
    assert_eq!(p.name, "Synthetic SUT");
    assert_eq!(p.brush.size, 17.0);
    assert_eq!(p.brush.spacing, 0.1);
    let TipShape::Sequence(tips) = &p.brush.tip else { panic!("sequence expected") };
    assert_eq!(tips[0].data, vec![65535, 0, 0, 65535]);
    assert_eq!(tips[1].data, vec![0, 65535, 65535, 0]);
    let mut wrapped = b"SUT prefix".to_vec();
    wrapped.extend(&f);
    assert_eq!(read_sut(&wrapped, "SUT", &prism_raster::Interrupt::NONE).unwrap().presets, imp.presets);
}
#[test]
fn tip_flag_without_references_falls_back_to_round() {
    // Shipped brushes set BrushUsePatternImage with a NULL reference column; this was an error.
    let imp = read_sut(&fixture_with(false, None), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(imp.presets[0].brush.tip, TipShape::Round);
    assert!(imp.warnings.iter().any(|w| w.contains("round tip")));
}
fn texture_refs(path: &str, count: u32) -> Vec<u8> {
    let mut refs = reference_blob(&[path]);
    refs[4..8].copy_from_slice(&count.to_be_bytes());
    refs
}
pub(super) fn reference_blob(paths: &[&str]) -> Vec<u8> {
    let mut refs = 8u32.to_be_bytes().to_vec();
    refs.extend((paths.len() as u32).to_be_bytes());
    for path in paths {
        let encoded: Vec<_> = path.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let metadata = [0xab, 0xcd, 0xef]; // Opaque fields need not resemble a path.
        refs.extend((8 + encoded.len() as u32 + metadata.len() as u32).to_be_bytes());
        refs.extend((encoded.len() as u32).to_be_bytes());
        refs.extend(encoded);
        refs.extend(metadata);
    }
    refs
}
#[test]
fn material_reference_paths_are_exact_and_preserve_repetitions() {
    let materials: Vec<Row> = ["tip.layer", "folder/tip.layer", "한글/🖌.layer"].into_iter().map(|path| [("OriginalPath".into(), text(path))].into()).collect();
    let paths = ["folder/tip.layer", "tip.layer", "folder/tip.layer", "한글/🖌.layer"];
    let resolved = referenced(&reference_blob(&paths), &materials, "tips").unwrap();
    let actual: Vec<_> = resolved.iter().map(|r| r["OriginalPath"].text().unwrap()).collect();
    assert_eq!(actual, paths);
    assert!(referenced(&reference_blob(&["missing/tip.layer"]), &materials, "tips").is_err());
    assert!(referenced(&reference_blob(&["tip.layer"]), &materials[1..], "tips").is_err());
    let ambiguous = vec![materials[0].clone(), materials[0].clone()];
    assert!(referenced(&reference_blob(&["tip.layer"]), &ambiguous, "tips").is_err());
}
#[test]
fn material_reference_framing_and_utf16_are_validated() {
    let materials: Vec<Row> = vec![[("OriginalPath".into(), text("a"))].into()];
    let valid = reference_blob(&["a"]);
    for (offset, value) in [(0, 7), (4, 0), (4, 2), (4, 65), (8, 0), (8, 7), (8, u32::MAX), (12, 0), (12, 1), (12, u32::MAX - 1)] {
        let mut bad = valid.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert!(referenced(&bad, &materials, "tips").is_err(), "offset {offset}, value {value}");
    }
    for len in 0..valid.len() {
        assert!(referenced(&valid[..len], &materials, "tips").is_err());
    }
    let mut surrogate = valid.clone();
    surrogate[16..18].copy_from_slice(&0xd800u16.to_le_bytes());
    assert!(referenced(&surrogate, &materials, "tips").is_err());
    let mut trailing = valid;
    trailing.push(0);
    assert!(referenced(&trailing, &materials, "tips").is_err());
}
#[test]
fn maps_texture_material_and_settings() {
    let imp = read_sut(&fixture_with(true, Some(texture_refs("second.layer", 1))), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let t = &imp.presets[0].brush.texture;
    assert!(t.enabled);
    // White, black, black, transparent (reads as white): luminance, not coverage.
    assert_eq!(t.pattern, Pattern::Tile(GrayTile { width: 2, height: 2, data: vec![65535, 0, 0, 65535] }));
    assert_eq!((t.scale, t.invert, t.each_tip, t.depth), (0.5, true, true, 0.4));
    assert_eq!(t.csp_mode, Some(CspTextureMode::Subtract));
    assert!(!imp.warnings.iter().any(|w| w.contains("texture could not be read")));
}

#[test]
fn verified_texture_modes_keep_csp_semantics_and_unknown_codes_warn() {
    let bytes = fixture();
    let db = Database::new(&bytes).unwrap();
    let mut row: Row = [("TextureImage".into(), Value::Blob(texture_refs("second.layer", 1)))].into();
    let modes = [
        CspTextureMode::Normal,
        CspTextureMode::Multiply,
        CspTextureMode::Subtract,
        CspTextureMode::Compare,
        CspTextureMode::Outline,
        CspTextureMode::Overlay,
        CspTextureMode::ColorDodge,
        CspTextureMode::ColorBurn,
        CspTextureMode::HardMix,
        CspTextureMode::Height,
    ];
    for (code, mode) in modes.into_iter().enumerate() {
        row.insert("TextureCompositeMode".into(), Value::Integer(code as i64));
        let mut brush = BrushSettings::default();
        let mut warnings = Vec::new();
        texture(&|| db.table("MaterialFile"), &row, &texture_refs("second.layer", 1), &mut brush, &mut warnings).unwrap();
        assert_eq!(brush.texture.csp_mode, Some(mode));
        assert!(!warnings.iter().any(|w| w.contains("texture mode")));
    }
    for code in [-1.0, 10.0, 1.00000001, 1e20] {
        row.insert("TextureCompositeMode".into(), Value::Number(code));
        let mut brush = BrushSettings::default();
        let mut warnings = Vec::new();
        texture(&|| db.table("MaterialFile"), &row, &texture_refs("second.layer", 1), &mut brush, &mut warnings).unwrap();
        assert_eq!(brush.texture.csp_mode, Some(CspTextureMode::Multiply));
        assert!(warnings.iter().any(|w| w.contains("approximated by Multiply")));
    }
}
fn effector_blob(supported: u32, active: u32) -> Vec<u8> {
    let mut blob = vec![0; 48];
    blob[..4].copy_from_slice(&48u32.to_be_bytes());
    blob[4..8].copy_from_slice(&supported.to_be_bytes());
    blob[8..12].copy_from_slice(&active.to_be_bytes());
    blob
}
#[test]
fn pressure_effectors_require_supported_and_active_flags() {
    for (supported, active, expected) in [(0x1f0, 0, false), (0x1f0, 0x10, true), (0x1e0, 0x10, false), (0x1f0, 0x20, false)] {
        let blob = Value::Blob(effector_blob(supported, active));
        let bytes = fixture_with_effectors(true, Value::Blob(texture_refs("second.layer", 1)), blob.clone(), blob);
        let imp = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
        let brush = &imp.presets[0].brush;
        assert!(!brush.pressure_size);
        assert_eq!(brush.shape_dynamics.enabled, expected);
        assert_eq!(brush.shape_dynamics.size.control == Control::PenPressure, expected);
        assert!(brush.texture.enabled && brush.texture.each_tip);
        assert_eq!(brush.texture.depth_jitter.control, if expected { Control::PenPressure } else { Control::Off });
    }
    // Each column controls its own parameter, even when the other is enabled.
    for size_active in [false, true] {
        let bytes = fixture_with_effectors(
            true,
            Value::Blob(texture_refs("second.layer", 1)),
            Value::Blob(effector_blob(0x1f0, if size_active { 0x10 } else { 0 })),
            Value::Blob(effector_blob(0x1f0, if size_active { 0 } else { 0x10 })),
        );
        let imp = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
        assert_eq!(imp.presets[0].brush.shape_dynamics.size.control == Control::PenPressure, size_active);
        assert_eq!(imp.presets[0].brush.texture.depth_jitter.control == Control::PenPressure, !size_active);
    }
}
#[test]
fn pressure_effectors_reject_missing_truncated_and_invalid_headers() {
    let valid = effector_blob(0x1f0, 0x10);
    let mut invalid = vec![Value::Null, text("pressure"), number(0x10 as f64)];
    for len in 0..valid.len() {
        invalid.push(Value::Blob(valid[..len].to_vec()));
    }
    for header_len in [0u32, 8, 12, 39, 49, u32::MAX] {
        let mut blob = valid.clone();
        blob[..4].copy_from_slice(&header_len.to_be_bytes());
        invalid.push(Value::Blob(blob));
    }
    for value in invalid {
        let row: Row = [("effector".into(), value)].into();
        assert!(pressure_effector(&row, "effector", &mut Vec::new()).is_none());
    }
    assert!(pressure_effector(&Row::new(), "effector", &mut Vec::new()).is_none());
    // The mandatory 40-byte header is sufficient; the observed 48-byte extension is optional.
    let mut fixed = valid[..40].to_vec();
    fixed[..4].copy_from_slice(&40u32.to_be_bytes());
    let row: Row = [("effector".into(), Value::Blob(fixed))].into();
    assert!(pressure_effector(&row, "effector", &mut Vec::new()).is_some());
}

fn curve_effector(points: &[[f64; 2]], minimum: u32) -> Vec<u8> {
    let mut blob = effector_blob(0x1f0, 0x10);
    blob[12..16].copy_from_slice(&minimum.to_be_bytes());
    let length = 12 + points.len() as u32 * 16;
    blob[32..36].copy_from_slice(&length.to_be_bytes());
    blob.extend(12u32.to_be_bytes());
    blob.extend((points.len() as u32).to_be_bytes());
    blob.extend(16u32.to_be_bytes());
    for point in points {
        for coordinate in point {
            blob.extend(coordinate.to_be_bytes());
        }
    }
    blob
}

fn curve_block(points: &[[f64; 2]]) -> Vec<u8> {
    let mut block = Vec::new();
    block.extend(12u32.to_be_bytes());
    block.extend((points.len() as u32).to_be_bytes());
    block.extend(16u32.to_be_bytes());
    for point in points {
        for coordinate in point {
            block.extend(coordinate.to_be_bytes());
        }
    }
    block
}

/// A 48-byte-header effector with optional pressure and tilt blocks (original synthetic values).
fn tilt_effector(active: u32, pressure: Option<&[[f64; 2]]>, tilt: Option<&[[f64; 2]]>, at_zero: u32, at_one: u32) -> Vec<u8> {
    let mut blob = effector_blob(0x1f0, active);
    blob[12..16].copy_from_slice(&7u32.to_be_bytes());
    blob[16..20].copy_from_slice(&at_zero.to_be_bytes());
    blob[40..44].copy_from_slice(&at_one.to_be_bytes());
    let (p, t) = (pressure.map(curve_block).unwrap_or_default(), tilt.map(curve_block).unwrap_or_default());
    blob[32..36].copy_from_slice(&(p.len() as u32).to_be_bytes());
    blob[36..40].copy_from_slice(&(t.len() as u32).to_be_bytes());
    blob.extend(p);
    blob.extend(t);
    blob
}

#[test]
fn tilt_effector_control_maps_to_an_independent_csp_tilt_response() {
    let pressure = [[0.0, 0.0], [1.0, 1.0]];
    let tilt = [[0.0, 1.0], [0.4, 1.0], [0.9, 0.0], [1.0, 0.0]];
    let size = |blob: Vec<u8>| {
        let bytes = fixture_with_effectors(true, Value::Blob(texture_refs("second.layer", 1)), Value::Blob(blob), Value::Null);
        let imported = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
        (imported.presets[0].brush.shape_dynamics.clone(), imported.warnings)
    };
    // Pressure and tilt together: both survive, tilt keeps its own values and curve.
    let (dynamics, warnings) = size(tilt_effector(0x30, Some(&pressure), Some(&tilt), 40, 100));
    assert!(dynamics.enabled && dynamics.size.control == Control::PenPressure);
    let response = dynamics.size.csp_tilt.clone().unwrap();
    assert_eq!((response.minimum, response.maximum), (0.4, 1.0));
    assert_eq!(response.curve.points(), tilt);
    assert!((response.multiplier(0.0, 0.0) - 0.4).abs() < 1e-6, "mouse / vertical pen: input 1");
    assert!((response.multiplier(90.0, 0.0) - 1.0).abs() < 1e-6, "horizontal pen: input 0");
    assert!(!warnings.iter().any(|w| w.contains("additional input controls")), "{warnings:?}");
    // Tilt alone is still a dynamic, without pen pressure.
    let (dynamics, _) = size(tilt_effector(0x20, None, Some(&tilt), 150, 20));
    assert_eq!(dynamics.size.control, Control::Off);
    let response = dynamics.size.csp_tilt.unwrap();
    assert_eq!((response.minimum, response.maximum), (1.5, 0.2), "the source does not order the two values");
    // Out-of-range values warn and drop only the tilt; no tilt block means no tilt response.
    let (dynamics, warnings) = size(tilt_effector(0x30, Some(&pressure), Some(&tilt), 900, 100));
    assert!(dynamics.size.csp_tilt.is_none() && dynamics.size.control == Control::PenPressure);
    assert!(warnings.iter().any(|w| w.contains("tilt response could not be read")), "{warnings:?}");
    let (dynamics, _) = size(tilt_effector(0x30, Some(&pressure), None, 40, 100));
    assert!(dynamics.size.csp_tilt.is_none());
    // Speed and random controls still warn.
    let (_, warnings) = size(tilt_effector(0x70, Some(&pressure), Some(&tilt), 40, 100));
    assert!(warnings.iter().any(|w| w.contains("additional input controls (0x40)")), "{warnings:?}");
}

#[test]
fn independent_pressure_curves_and_minimums_survive_sut_import() {
    // Original synthetic curves, deliberately different for each parameter.
    let points =
        [vec![[0.0, 0.0], [0.25, 0.75], [1.0, 1.0]], vec![[0.0, 1.0], [1.0, 0.0]], vec![[0.0, 0.2], [0.5, 0.1], [1.0, 0.8]], vec![[0.0, 0.0], [1.0, 1.0]]];
    let minimums = [13, 23, 28, 0];
    let effectors = std::array::from_fn(|i| Value::Blob(curve_effector(&points[i], minimums[i])));
    let bytes = fixture_with_all_effectors(true, Value::Blob(texture_refs("second.layer", 1)), effectors);
    let imp = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let b = &imp.presets[0].brush;
    assert!(b.shape_dynamics.enabled && b.transfer.enabled && b.texture.enabled);
    for (i, dynamic) in [&b.shape_dynamics.size, &b.texture.depth_jitter, &b.transfer.opacity, &b.transfer.flow].into_iter().enumerate() {
        assert_eq!(dynamic.control, Control::PenPressure);
        assert_eq!(dynamic.minimum, minimums[i] as f32 / 100.0);
        assert_eq!(dynamic.pressure_curve.as_ref().unwrap().points(), points[i]);
    }
    // This legacy fixture has no presentation blob; its fallback warning does not
    // imply a pressure-curve or material decoding failure.
    assert!(!imp.warnings.iter().any(|w| w.contains("could not be read") && !w.starts_with("SUT display metadata ")));
}

#[test]
fn malformed_pressure_curves_fail_safely_and_warn_without_losing_the_brush() {
    let valid = curve_effector(&[[0.0, 0.0], [0.5, 0.8], [1.0, 1.0]], 25);
    let mut invalid = Vec::new();
    for len in 0..valid.len() {
        invalid.push(valid[..len].to_vec());
    }
    for (offset, value) in [(0, u32::MAX), (12, 101), (32, u32::MAX), (36, 1), (44, 1), (48, 0), (52, 0), (52, 1), (52, 17), (56, 0), (56, u32::MAX)] {
        let mut bad = valid.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        invalid.push(bad);
    }
    for points in [
        vec![[0.0, 0.0], [f64::NAN, 1.0]],
        vec![[0.0, 0.0], [1.0, f64::INFINITY]],
        vec![[0.5, 0.0], [0.5, 1.0]],
        vec![[1.0, 0.0], [0.0, 1.0]],
        vec![[0.0, -0.1], [1.0, 1.0]],
    ] {
        invalid.push(curve_effector(&points, 0));
    }
    for blob in invalid {
        assert!(effector::read(&blob).is_err());
    }
    let bytes = fixture_with_effectors(true, Value::Null, Value::Blob(curve_effector(&[[0.0, 0.0], [0.0, 1.0]], 0)), Value::Null);
    let imp = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
    assert!(!imp.presets[0].brush.shape_dynamics.enabled);
    assert!(matches!(imp.presets[0].brush.tip, TipShape::Sequence(_)));
    assert!(imp.warnings.iter().any(|w| w.contains("BrushSizeEffector could not be read")));
}
#[test]
fn broken_texture_reference_keeps_the_brush() {
    for refs in [texture_refs("absent.layer", 1), texture_refs("second.layer", 3), vec![1, 2]] {
        let imp = read_sut(&fixture_with(true, Some(refs)), "SUT", &prism_raster::Interrupt::NONE).unwrap();
        assert!(!imp.presets[0].brush.texture.enabled);
        assert!(imp.warnings.iter().any(|w| w.contains("texture could not be read")));
    }
}
#[test]
fn wrong_texture_value_type_keeps_the_imported_tips() {
    let expected = read_sut(&fixture(), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    for value in [text("not a reference blob"), number(1.0), Value::Integer(1)] {
        let imp = read_sut(&fixture_with_texture_value(true, value), "SUT", &prism_raster::Interrupt::NONE).unwrap();
        assert_eq!(imp.presets, expected.presets);
        assert!(imp.warnings.iter().any(|w| w.contains("texture could not be read") && w.contains("reference is invalid")));
    }
}

/// Minimal round-brush database with independently controlled identifiers and schema root.
fn identity_fixture(node: Value, variants: &[Vec<Value>], node_root: Value) -> Vec<u8> {
    let mut db = vec![0; 3 * 4096];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&4096u16.to_be_bytes());
    db[56..60].copy_from_slice(&1u32.to_be_bytes());
    leaf(
        &mut db,
        1,
        &[
            vec![text("table"), text("Node"), text("Node"), node_root, text("CREATE TABLE Node (NodeName TEXT, NodeVariantID INTEGER)")],
            vec![text("table"), text("Variant"), text("Variant"), Value::Integer(3), text("CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL)")],
        ],
    );
    leaf(&mut db, 2, &[vec![text("Exact identifier"), node]]);
    leaf(&mut db, 3, variants);
    db
}

#[test]
fn variant_selection_retains_exact_integer_identifiers() {
    for id in [1_000_001, 16_777_216, 9_007_199_254_740_992, i64::MAX - 1, i64::MIN] {
        for reverse in [false, true] {
            let mut variants = vec![vec![Value::Integer(id), number(17.0)], vec![Value::Integer(id + 1), number(93.0)]];
            if reverse {
                variants.reverse();
            }
            let bytes = identity_fixture(Value::Integer(id), &variants, Value::Integer(2));
            let imp = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
            assert_eq!(imp.presets[0].brush.size, 17.0, "id {id}, reversed {reverse}");
        }
    }
    // SQLite REAL storage used by older synthetic fixtures still interoperates with INTEGER.
    for (node, variant) in [(number(818.0), Value::Integer(818)), (Value::Integer(818), number(818.0))] {
        let bytes = identity_fixture(node, &[vec![variant, Value::Integer(17)]], number(2.0));
        assert_eq!(read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap().presets[0].brush.size, 17.0);
    }
}

#[test]
fn malformed_identifiers_cannot_match_or_truncate_to_a_root_page() {
    for invalid in [
        Value::Null,
        text("818"),
        number(818.25),
        number(f64::NAN),
        number(f64::INFINITY),
        number(f64::NEG_INFINITY),
        number(-(i64::MIN as f64)),
        number(-1e30),
    ] {
        let bytes = identity_fixture(invalid.clone(), &[vec![invalid, number(17.0)]], number(2.0));
        assert!(read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).is_err());
    }
    for root in [number(2.5), number(f64::NAN), number(f64::INFINITY), Value::Integer(0), Value::Integer(i64::MAX), text("2")] {
        let bytes = identity_fixture(Value::Integer(818), &[vec![Value::Integer(818), number(17.0)]], root);
        assert!(read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).is_err());
    }
    let bytes = identity_fixture(Value::Integer(818), &[vec![number(818.25), number(17.0)]], number(2.0));
    assert!(read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).is_err());
}
#[test]
fn truncated_corrupt_and_cyclic_databases_fail_without_panicking() {
    let f = fixture();
    for n in (0..f.len()).step_by(127) {
        assert!(read_sut(&f[..n], "", &prism_raster::Interrupt::NONE).is_err());
    }
    let mut corrupt = f.clone();
    corrupt[16..18].fill(0);
    assert!(read_sut(&corrupt, "", &prism_raster::Interrupt::NONE).is_err());
    let mut cyclic = f.clone();
    cyclic[4 * 4096..4 * 4096 + 4].copy_from_slice(&5u32.to_be_bytes());
    assert!(read_sut(&cyclic, "", &prism_raster::Interrupt::NONE).is_err());
    for i in (0..f.len()).step_by(71) {
        let mut bad = f.clone();
        bad[i] ^= 255;
        let _ = read_sut(&bad, "", &prism_raster::Interrupt::NONE);
    }
}
#[test]
fn png_chunk_lengths_and_cancellation_are_checked() {
    assert!(png(b"no PNG").is_err());
    assert!(png(b"\x89PNG\r\n\x1a\n\xff\xff\xff\xffIDAT").is_err());
    let cancel = || true;
    let progress = |_| {};
    assert!(read_sut(&fixture(), "", &prism_raster::Interrupt::new(&cancel, &progress)).is_err());
}

fn variant_fields_fixture(fields: &[(&str, Value)]) -> Vec<u8> {
    sub_tool_fixture(None, fields)
}

/// A one-node SUT; `ops` adds the node's input/output operations.
fn sub_tool_fixture(ops: Option<(i64, i64)>, fields: &[(&str, Value)]) -> Vec<u8> {
    let mut db = vec![0; 3 * 4096];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&4096u16.to_be_bytes());
    db[56..60].copy_from_slice(&1u32.to_be_bytes());
    let columns = fields.iter().map(|(key, _)| format!("{key} BLOB")).collect::<Vec<_>>().join(", ");
    leaf(
        &mut db,
        1,
        &[
            vec![
                text("table"),
                text("Node"),
                text("Node"),
                Value::Integer(2),
                text(if ops.is_some() {
                    "CREATE TABLE Node(NodeName TEXT, NodeVariantID INTEGER, NodeInputOp INTEGER, NodeOutputOp INTEGER)"
                } else {
                    "CREATE TABLE Node(NodeName TEXT, NodeVariantID INTEGER)"
                }),
            ],
            vec![text("table"), text("Variant"), text("Variant"), Value::Integer(3), text(&format!("CREATE TABLE Variant(VariantID INTEGER, {columns})"))],
        ],
    );
    let mut node = vec![text("Synthetic spray"), Value::Integer(1)];
    if let Some((input, output)) = ops {
        node.extend([Value::Integer(input), Value::Integer(output)]);
    }
    leaf(&mut db, 2, &[node]);
    let row = std::iter::once(Value::Integer(1)).chain(fields.iter().map(|(_, value)| value.clone())).collect();
    leaf(&mut db, 3, &[row]);
    db
}

#[test]
fn sut_presentation_uses_current_variant_and_does_not_change_render_settings() {
    let display = |ids: &[u32]| Value::Blob([12, ids.len() as u32, 4].into_iter().chain(ids.iter().copied()).flat_map(u32::to_be_bytes).collect());
    let fields = [
        ("VariantShowParam", display(&[1000, 10, 0xf1234567])),
        ("VariantShowSeparator", Value::Integer(1)),
        ("BrushSize", number(15.0)),
        ("Opacity", number(90.0)),
    ];
    let mut bytes = variant_fields_fixture(&fields);
    // An initial/different variant appears first: display provenance must still
    // come from NodeVariantID=1, including a valid deliberately empty list.
    leaf(
        &mut bytes,
        3,
        &[
            vec![Value::Integer(2), display(&[]), Value::Integer(0), number(100.0), number(50.0)],
            std::iter::once(Value::Integer(1)).chain(fields.iter().map(|(_, value)| value.clone())).collect(),
        ],
    );
    let imported = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let preset = &imported.presets[0];
    let metadata = preset.sut.as_ref().unwrap();
    assert_eq!(metadata.original.visible, [1000, 10, 0xf1234567]);
    assert!(metadata.original.show_categories);
    assert_eq!((preset.brush.size, preset.brush.opacity), (15.0, 0.9));
    let mut hidden = fields.clone();
    hidden[0].1 = display(&[]);
    let hidden = read_sut(&variant_fields_fixture(&hidden), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    assert!(hidden.presets[0].sut.as_ref().unwrap().original.visible.is_empty());
    assert_eq!(preset.brush, hidden.presets[0].brush);
    let mut broken = fields;
    broken[0].1 = Value::Blob(vec![0, 1, 2]);
    let broken = read_sut(&variant_fields_fixture(&broken), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(broken.presets[0].brush, preset.brush);
    assert_eq!(broken.presets[0].sut.as_ref().unwrap().original.visible, [1000, 10]);
    assert!(broken.notes.iter().any(|w| w.contains("display metadata")));
}

#[test]
#[ignore = "requires local CSP-exported differential files; set PRISM_SUT_DISPLAY_DIR"]
fn local_csp_display_exports_keep_visibility_category_and_indicator_separate() {
    let dir = std::path::PathBuf::from(std::env::var("PRISM_SUT_DISPLAY_DIR").expect("set PRISM_SUT_DISPLAY_DIR"));
    let imports: Vec<_> = ["display-baseline.sut", "display-hide-size.sut", "display-opacity-indicator.sut", "display-category.sut"]
        .iter()
        .map(|name| read_sut(&std::fs::read(dir.join(name)).unwrap(), "SUT", &prism_raster::Interrupt::NONE).unwrap())
        .collect();
    let presets: Vec<_> = imports.iter().map(|i| &i.presets[0]).collect();
    let base = presets[0].sut.as_ref().unwrap();
    let hidden = presets[1].sut.as_ref().unwrap();
    assert_eq!(base.original.visible, [1000, 10, 30, 1081, 1083, 1085, 100, 110, 120, 1201, 40, 41]);
    assert_eq!(hidden.original.visible, base.original.visible.iter().copied().filter(|id| *id != 1000).collect::<Vec<_>>());
    assert_eq!(hidden, presets[2].sut.as_ref().unwrap());
    let category = presets[3].sut.as_ref().unwrap();
    assert!(!hidden.original.show_categories);
    assert!(category.original.show_categories);
    assert_eq!(hidden.original.visible, category.original.visible);
    for p in &presets[1..] {
        assert_eq!(p.brush, presets[0].brush);
    }
}

#[test]
fn csp_spray_uses_native_fields_and_keeps_area_particle_size_count_bias_independent() {
    let fields = [
        ("BrushUseSpray", Value::Integer(1)),
        ("BrushSize", number(15.0)),
        ("Opacity", number(90.0)),
        ("BrushSpraySize", number(0.74)),
        ("BrushSprayDensity", Value::Integer(22)),
        ("BrushSprayBias", Value::Integer(-30)),
        ("BrushRotation", number(65.1)),
        ("BrushRotationInSpray", number(12.5)),
        ("AntiAlias", Value::Integer(0)),
        ("BrushRotationEffectorInSpray", Value::Integer(3)),
        ("BrushRotationRandomInSpray", number(100.0)),
    ];
    let imported = read_sut(&variant_fields_fixture(&fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let b = &imported.presets[0].brush;
    let spray = b.csp_spray.as_ref().unwrap();
    assert_eq!((b.size, b.opacity, b.angle, b.csp_anti_alias), (15.0, 0.9, -65.1, Some(0)));
    assert!(!b.aliased, "CSP AA OFF must not enable Pencil thresholding");
    assert_eq!((spray.size, spray.count, spray.bias, spray.angle), (0.74, 22, -0.3, -12.5));
    assert!(!b.scattering.enabled, "CSP particle placement must not also enable Photoshop scattering");
    assert!(!imported.warnings.iter().any(|w| w.contains("particle angle input controls")));
    let mut no_random = fields.clone();
    no_random.iter_mut().find(|(k, _)| *k == "BrushRotationRandomInSpray").unwrap().1 = number(0.0);
    let other = read_sut(&variant_fields_fixture(&no_random), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(other.presets[0].brush, *b, "stored Random=100 must remain inactive when angle control flags are only 3");
}

#[test]
fn csp_spray_pressure_effectors_do_not_replace_the_area_curve_or_each_other() {
    let area = [[0.0, 0.0], [1.0, 1.0]];
    let particle = [[0.0, 0.0], [0.5, 1.0], [1.0, 1.0]];
    let count = [[0.0, 1.0], [1.0, 0.0]];
    let fields = [
        ("BrushUseSpray", Value::Integer(1)),
        ("BrushSizeEffector", Value::Blob(curve_effector(&area, 13))),
        ("BrushSpraySizeEffector", Value::Blob(curve_effector(&particle, 27))),
        ("BrushSprayDensityEffector", Value::Blob(curve_effector(&count, 42))),
    ];
    let imported = read_sut(&variant_fields_fixture(&fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let b = &imported.presets[0].brush;
    let spray = b.csp_spray.as_ref().unwrap();
    for (dynamic, points, minimum) in
        [(&b.shape_dynamics.size, area.as_slice(), 0.13), (&spray.size_dynamic, particle.as_slice(), 0.27), (&spray.count_dynamic, count.as_slice(), 0.42)]
    {
        assert_eq!(dynamic.control, Control::PenPressure);
        assert_eq!(dynamic.pressure_curve.as_ref().unwrap().points(), points);
        assert!((dynamic.minimum - minimum).abs() < 1e-6);
    }
}

#[test]
fn csp_spray_disabled_fields_stay_inactive_and_particle_budget_warns() {
    let inactive = read_sut(
        &variant_fields_fixture(&[("BrushUseSpray", Value::Integer(0)), ("BrushSprayDensity", Value::Integer(123)), ("AntiAlias", Value::Integer(2))]),
        "SUT",
        &prism_raster::Interrupt::NONE,
    )
    .unwrap();
    assert!(inactive.presets[0].brush.csp_spray.is_none());
    assert!(!inactive.presets[0].brush.aliased);
    assert_eq!(inactive.presets[0].brush.csp_anti_alias, Some(2));
    let imported = read_sut(
        &variant_fields_fixture(&[
            ("BrushUseSpray", Value::Integer(1)),
            ("BrushSprayDensity", Value::Integer(i64::MAX)),
            ("BrushSpraySizeUnit", Value::Integer(1)),
            ("BrushRotationEffectorInSpray", Value::Integer(0x80)),
        ]),
        "SUT",
        &prism_raster::Interrupt::NONE,
    )
    .unwrap();
    assert_eq!(imported.presets[0].brush.csp_spray.as_ref().unwrap().count, prism_paint::csp_spray::MAX_PARTICLES);
    assert!(!imported.presets[0].brush.aliased, "missing AA must not imply explicit disabled AA");
    assert_eq!(imported.presets[0].brush.csp_anti_alias, None);
    for message in ["per-step budget", "BrushSpraySizeUnit", "particle angle input controls"] {
        assert!(imported.warnings.iter().any(|w| w.contains(message)));
    }
}

fn in_out_target_blob(entries: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut blob = Vec::new();
    for word in [12, entries.len() as u32, 12] {
        blob.extend(word.to_be_bytes());
    }
    for (id, on, minimum) in entries {
        for word in [*id, *on, *minimum] {
            blob.extend(word.to_be_bytes());
        }
    }
    blob
}

#[test]
fn csp_start_and_end_maps_lengths_percentages_and_targets() {
    use prism_paint::{CspInOutMode, CspInOutParam, CspInOutTarget};
    let read = |fields: &[(&str, Value)]| read_sut(&variant_fields_fixture(fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    // Original synthetic targets: size to 0, flow to 40 %, thickness (unsupported) and a disabled one.
    let targets = Value::Blob(in_out_target_blob(&[(0x70, 0, 50), (0x3e9, 1, 0), (0x3fd, 1, 40), (0x41b, 1, 57)]));
    let base = [
        ("BrushUseIn", Value::Integer(1)),
        ("BrushInLength", number(20.0)),
        ("BrushInRatio", number(30.0)),
        ("BrushUseOut", Value::Integer(1)),
        ("BrushOutLength", number(7.0)),
        ("BrushOutRatio", number(10.0)),
        ("BrushInOutType", Value::Integer(0)),
        ("BrushInOutTarget", targets),
    ];
    let imported = read(&base);
    let in_out = imported.presets[0].brush.csp_in_out.clone().unwrap();
    assert_eq!((in_out.mode, in_out.start, in_out.end, in_out.by_speed), (CspInOutMode::Length, Some(20.0), Some(7.0), false));
    assert_eq!(in_out.targets, vec![CspInOutTarget { param: CspInOutParam::Size, minimum: 0.0 }, CspInOutTarget { param: CspInOutParam::Flow, minimum: 0.4 }]);
    assert!(imported.warnings.iter().any(|w| w.contains("target 0x41b")), "{:?}", imported.warnings);
    assert!(!imported.warnings.iter().any(|w| w.contains("BrushUseIn")));
    let mut percent = base.clone();
    percent[6].1 = Value::Integer(1);
    let in_out = read(&percent).presets[0].brush.csp_in_out.clone().unwrap();
    assert_eq!((in_out.mode, in_out.start, in_out.end), (CspInOutMode::Percent, Some(0.3), Some(0.1)));
    let mut fade = base.clone();
    fade[6].1 = Value::Integer(2);
    let faded = read(&fade);
    assert!(faded.presets[0].brush.csp_in_out.is_none() && faded.warnings.iter().any(|w| w.contains("fade")));
    let mut off = base.clone();
    off[0].1 = Value::Integer(0);
    off[3].1 = Value::Integer(0);
    assert!(read(&off).presets[0].brush.csp_in_out.is_none());
    let mut broken = base.clone();
    broken[7].1 = Value::Blob(vec![0, 0, 0, 12, 0, 0, 0, 9]);
    let broken = read(&broken);
    assert!(broken.presets[0].brush.csp_in_out.is_none() && broken.warnings.iter().any(|w| w.contains("targets could not be read")));
}

#[test]
fn sub_tool_kinds_import_and_non_brushes_are_marked_for_the_clip_studio_workspace() {
    use prism_paint::sut::SutToolKind;
    let read = |ops, fields: &[(&str, Value)]| read_sut(&sub_tool_fixture(Some(ops), fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    // A pen keeps its brush settings and verified brush family.
    let pen = read((11, 11), &[("BrushSize", number(7.0))]);
    let meta = pen.presets[0].sut.as_ref().unwrap();
    assert_eq!((meta.kind(), pen.presets[0].brush.size), (SutToolKind::Pen, 7.0));
    assert!(!pen.warnings.iter().any(|w| w.contains("not a brush")));
    // A fill sub-tool is imported but flagged: it is not a brush.
    let fill = read((50, 50), &[("FillReferMultiLayer", Value::Integer(1))]);
    assert_eq!(fill.presets[0].sut.as_ref().unwrap().kind(), SutToolKind::Fill);
    assert!(fill.notes.iter().any(|w| w.contains("Clip Studio fill sub-tool, not a brush")), "{:?}", fill.notes);
    // A runnable sub-tool needs no warning; the general approximation note is not one either.
    assert!(fill.warnings.is_empty(), "{:?}", fill.warnings);
    assert!(pen.notes.iter().any(|w| w.contains("rendering is approximated")));
    // Enclose-and-fill runs in the Clip Studio workspace: no warning, its fill settings kept.
    let enclose = read((31, 51), &[("FillUseCloseGap", Value::Integer(1)), ("FillCloseGap", number(5.0)), ("FillExpandLength", number(-1.0))]);
    assert!(enclose.warnings.is_empty(), "{:?}", enclose.warnings);
    let params = &enclose.presets[0].sut.as_ref().unwrap().parameters;
    assert_eq!((params.get("FillUseCloseGap"), params.get("FillCloseGap"), params.get("FillExpandLength")), (Some(&1.0), Some(&5.0), Some(&-1.0)));
    // The dot pen stores no size: one aliased pixel, not the generic 20 px default.
    let dot = read((14, 14), &[("Opacity", number(100.0))]);
    assert_eq!((dot.presets[0].brush.size, dot.presets[0].brush.csp_anti_alias), (1.0, Some(0)));
    // A stored size still wins.
    assert_eq!(read((14, 14), &[("BrushSize", number(3.0))]).presets[0].brush.size, 3.0);
}

#[test]
fn csp_rotation_flags_map_random_turn_direction_and_base_angle() {
    let brush = |flags: i64| {
        let imported = read_sut(
            &variant_fields_fixture(&[
                ("BrushRotation", Value::Number(45.0)),
                ("BrushRotationEffector", Value::Integer(flags)),
                ("BrushRotationRandomScale", Value::Integer(100)),
            ]),
            "SUT",
            &prism_raster::Interrupt::NONE,
        )
        .unwrap();
        (imported.presets[0].brush.clone(), imported.warnings)
    };
    // 131 = enabled | random (the Grainy pens): a random turn of up to ±180°, no direction.
    let (b, _) = brush(131);
    assert_eq!((b.angle, b.shape_dynamics.angle.control, b.shape_dynamics.angle.jitter), (-45.0, Control::Off, 0.5));
    assert!(b.shape_dynamics.enabled);
    // 3 has neither the direction nor the random bit: the base angle only.
    let (b, _) = brush(3);
    assert_eq!((b.angle, b.shape_dynamics.angle.control, b.shape_dynamics.angle.jitter), (-45.0, Control::Off, 0.0));
    // 19 (Gouache) follows the pen's direction, 0x41 the line direction (see tip_tests).
    assert_eq!(brush(19).0.shape_dynamics.angle.control, Control::PenTilt);
    assert_eq!(brush(0x41).0.shape_dynamics.angle.control, Control::Direction);
    // Without the enable bit there is no rotation at all.
    assert_eq!(brush(0x82).0.angle, 0.0);
}

#[test]
fn csp_automatic_spacing_follows_type_hardness_and_thickness() {
    let spacing = |fields: &[(&str, Value)]| {
        let imported = read_sut(&variant_fields_fixture(fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
        (imported.presets[0].brush.spacing, imported.warnings)
    };
    let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
    let row = |kind: i64, hardness: f64, thickness: f64| {
        vec![
            ("BrushAutoIntervalType", Value::Integer(kind)),
            ("BrushInterval", Value::Number(10.0)),
            ("BrushHardness", Value::Number(hardness)),
            ("BrushThickness", Value::Number(thickness)),
        ]
    };
    // Fixed spacing uses the interval; automatic types ignore it.
    assert!(close(spacing(&row(0, 100.0, 100.0)).0, 0.1));
    assert!(close(spacing(&row(1, 100.0, 100.0)).0, 0.12));
    assert!(close(spacing(&row(2, 100.0, 100.0)).0, 0.08));
    assert!(close(spacing(&row(3, 100.0, 100.0)).0, 0.04));
    // Softer tips space wider (hardness floors at 20%); thinner tips space closer.
    assert!(close(spacing(&row(3, 50.0, 100.0)).0, 0.04 * 2.25));
    assert!(close(spacing(&row(3, 0.0, 100.0)).0, 0.04 * 3.0));
    assert!(close(spacing(&row(2, 100.0, 50.0)).0, 0.04));
    let (unknown, warnings) = spacing(&row(9, 100.0, 100.0));
    assert!(close(unknown, 0.08));
    assert!(warnings.iter().any(|w| w.contains("spacing type 9")), "{warnings:?}");
}

#[test]
fn csp_color_mixing_maps_amount_density_stretch_and_effectors() {
    let read = |fields: &[(&str, Value)]| read_sut(&variant_fields_fixture(fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let curve = [[0.0, 0.0], [0.5, 1.0], [1.0, 1.0]];
    let base = [
        ("BrushUseWaterColor", Value::Integer(1)),
        ("BrushWaterColor", Value::Integer(0)),
        ("BrushMixColor", Value::Integer(80)),
        ("BrushMixAlpha", Value::Integer(64)),
        ("BrushMixColorExtension", Value::Integer(42)),
        ("BrushColorMixingMode", Value::Integer(0)),
        ("BrushMixColorEffector", Value::Blob(curve_effector(&curve, 20))),
    ];
    let imported = read(&base);
    let mixing = imported.presets[0].brush.csp_mixing.clone().unwrap();
    assert_eq!((mixing.amount, mixing.density, mixing.stretch), (0.8, 0.64, 0.42));
    assert_eq!(mixing.amount_dynamic.control, Control::PenPressure);
    assert!((mixing.amount_dynamic.minimum - 0.2).abs() < 1e-6);
    assert_eq!(mixing.density_dynamic.control, Control::Off);
    assert!(!imported.warnings.iter().any(|w| w.contains("WaterColor") || w.contains("color mixing")), "{:?}", imported.warnings);
    // Stored values without the enable flag stay inactive (the Grainy Pen case).
    let mut off = base.clone();
    off[0].1 = Value::Integer(0);
    assert!(read(&off).presets[0].brush.csp_mixing.is_none());
    let mut other = base.clone();
    other[1].1 = Value::Integer(1);
    other[5].1 = Value::Integer(1);
    let other = read(&other);
    assert!(other.presets[0].brush.csp_mixing.as_ref().unwrap().perceptual, "BrushColorMixingMode 1 is perceptual mixing");
    assert!(other.warnings.iter().any(|w| w.contains("color mixing type 1")), "{:?}", other.warnings);
    assert!(!other.warnings.iter().any(|w| w.contains("perceptual") || w.contains("WaterColor2")), "{:?}", other.warnings);
    // The duplicate flag alone (no BrushUseWaterColor field) also enables mixing.
    let mut mirrored = base[1..].to_vec();
    mirrored.push(("BrushUseWaterColor2", Value::Integer(1)));
    assert!(read(&mirrored).presets[0].brush.csp_mixing.is_some());
}

#[test]
fn csp_stabilization_maps_level_speed_adjustment_and_stickiness() {
    let read = |fields: &[(&str, Value)]| read_sut(&variant_fields_fixture(fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let imported = read(&[
        ("FlickerReduction", Value::Integer(10)),
        ("FlickerReductionBySpeed", Value::Integer(1)),
        ("FlickerReductionBySpeedType", Value::Integer(0)),
        ("Stickness", Value::Integer(6)),
    ]);
    assert_eq!(imported.presets[0].brush.csp_stabilizer, Some(prism_paint::CspStabilizer { level: 10, adjust_by_speed: true, speed_type: 0, stickiness: 6 }));
    assert!(!imported.warnings.iter().any(|w| w.contains("stabiliz")), "{:?}", imported.warnings);
    assert!(read(&[("FlickerReduction", Value::Integer(0)), ("FlickerReductionBySpeed", Value::Integer(1))]).presets[0].brush.csp_stabilizer.is_none());
    let odd =
        read(&[("FlickerReduction", Value::Integer(5000)), ("FlickerReductionBySpeed", Value::Integer(1)), ("FlickerReductionBySpeedType", Value::Integer(2))]);
    assert_eq!(odd.presets[0].brush.csp_stabilizer.as_ref().unwrap().level, prism_paint::csp_stabilizer::MAX_LEVEL);
    assert!(odd.warnings.iter().any(|w| w.contains("exceeds 100")), "{:?}", odd.warnings);
    assert!(!odd.warnings.iter().any(|w| w.contains("speed type")), "speed types are mapped: {:?}", odd.warnings);
    assert_eq!(odd.presets[0].brush.csp_stabilizer.as_ref().unwrap().speed_type, 2);
}

#[test]
fn csp_water_edge_maps_native_fields_and_blur_needs_after_drag() {
    let fields = [
        ("BrushUseWaterEdge", Value::Integer(1)),
        ("BrushWaterEdgeRadius", number(2.0)),
        ("BrushWaterEdgeRadiusUnit", Value::Integer(0)),
        ("BrushWaterEdgeAlphaPower", Value::Integer(15)),
        ("BrushWaterEdgeValuePower", Value::Integer(40)),
        ("BrushWaterEdgeAfterDrag", Value::Integer(1)),
        ("BrushWaterEdgeBlur", number(10.0)),
        ("BrushWaterEdgeBlurUnit", Value::Integer(0)),
        ("BrushWaterEdgeStyle", Value::Integer(0)),
    ];
    let imported = read_sut(&variant_fields_fixture(&fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let edge = imported.presets[0].brush.csp_water_edge.clone().unwrap();
    assert_eq!(edge, prism_paint::CspWaterEdge { radius: 2.0, opacity: 0.15, darkness: 0.4, blur: 10.0, after_drag: true });
    assert!(!imported.warnings.iter().any(|w| w.contains("WaterEdge") || w.contains("watercolor edge")), "{:?}", imported.warnings);
    // The source reads the blur only with Process after drag.
    let mut live = fields;
    live[5].1 = Value::Integer(0);
    let edge = read_sut(&variant_fields_fixture(&live), "SUT", &prism_raster::Interrupt::NONE).unwrap().presets[0].brush.csp_water_edge.clone().unwrap();
    assert_eq!((edge.after_drag, edge.blur), (false, 0.0));
    // Disabled edges stay off even with stored values; units, style and huge radii warn.
    let off = read_sut(
        &variant_fields_fixture(&[("BrushUseWaterEdge", Value::Integer(0)), ("BrushWaterEdgeRadius", number(5.0))]),
        "SUT",
        &prism_raster::Interrupt::NONE,
    )
    .unwrap();
    assert!(off.presets[0].brush.csp_water_edge.is_none());
    let odd = read_sut(
        &variant_fields_fixture(&[
            ("BrushUseWaterEdge", Value::Integer(1)),
            ("BrushWaterEdgeRadius", number(1e9)),
            ("BrushWaterEdgeRadiusUnit", Value::Integer(1)),
            ("BrushWaterEdgeStyle", Value::Integer(2)),
            ("BrushWaterEdgeAlphaPower", Value::Integer(-50)),
        ]),
        "SUT",
        &prism_raster::Interrupt::NONE,
    )
    .unwrap();
    let edge = odd.presets[0].brush.csp_water_edge.clone().unwrap();
    assert_eq!((edge.radius, edge.opacity), (prism_paint::csp_water_edge::MAX_RADIUS, 0.0));
    for message in ["BrushWaterEdgeRadiusUnit", "edge style", "work limit"] {
        assert!(odd.warnings.iter().any(|w| w.contains(message)), "{message}: {:?}", odd.warnings);
    }
}

#[test]
fn unsupported_original_material_falls_back_with_an_explicit_thumbnail_warning() {
    let mut bytes = fixture();
    let rows = Database::new(&bytes).unwrap().table("MaterialFile").unwrap();
    let rebuilt: Vec<_> = rows
        .into_iter()
        .map(|mut row| {
            let mut blob = row.remove("FileData").unwrap().blob().unwrap().to_vec();
            blob[257..262].copy_from_slice(b"ustar"); // Recognized but corrupt container; its PNG remains readable.
            vec![row.remove("OriginalPath").unwrap(), Value::Blob(blob)]
        })
        .collect();
    leaf(&mut bytes, 4, &rebuilt);
    let imported = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
    assert!(matches!(imported.presets[0].brush.tip, TipShape::Sequence(_)));
    assert!(imported.warnings.iter().any(|w| w.contains("using its thumbnail") && w.contains("change the brush grain")));
}

#[test]
#[ignore = "local-only material oracle; set PRISM_GRAINY_SUT_PATH to the maintainer-provided SUT"]
fn local_grainy_sut_uses_original_501_pixel_texture() {
    let path = std::env::var_os("PRISM_GRAINY_SUT_PATH").expect("set PRISM_GRAINY_SUT_PATH for this opt-in local test");
    let bytes = std::fs::read(path).unwrap();
    let imported = read_sut(&bytes, "Local SUT verification", &prism_raster::Interrupt::NONE).unwrap();
    let brush = &imported.presets[0].brush;
    assert_eq!(brush.csp_tip_scale, Some([1.0, 2.4]));
    let Pattern::Tile(tile) = &brush.texture.pattern else { panic!("original texture tile is required") };
    assert_eq!((tile.width, tile.height), (501, 501));
    assert_eq!(brush.texture.scale, 0.25);
    assert!(!imported.warnings.iter().any(|w| w.contains("thumbnail")), "{:?}", imported.warnings);
    let TipShape::Sampled(tip) = &brush.tip else { panic!("original sampled tip is required") };
    assert_eq!((tip.width, tip.height), (52, 72));
}

#[test]
fn csp_tip_thickness_preserves_expansion_and_axis() {
    for (vertical, expected) in [(0, [2.4, 1.0]), (1, [1.0, 2.4])] {
        let bytes = variant_fields_fixture(&[("BrushThickness", number(240.0)), ("BrushVerticalThicknes", Value::Integer(vertical))]);
        let imported = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
        let b = &imported.presets[0].brush;
        assert_eq!(b.csp_tip_scale, Some(expected));
        assert_eq!(b.roundness, 1.0);
    }
}

#[test]
fn colour_tips_paint_their_shape_in_their_own_colours() {
    use super::material::tests::{c2f, database_sized, tar, tile_fixture_color};
    let mut bytes = fixture();
    // White ink with a transparent corner, as in downloaded highlight brushes.
    let package = tar(&c2f(&database_sized(1, 1024, tile_fixture_color(2, 2, &[255, 128, 255, 0], &[[255, 255, 255]; 4]))));
    let rows: Vec<_> = ["first.layer", "second.layer"].iter().map(|p| vec![text(p), Value::Blob(package.clone())]).collect();
    leaf(&mut bytes, 4, &rows);
    let imported = read_sut(&bytes, "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let TipShape::Sequence(tips) = &imported.presets[0].brush.tip else { panic!("sequence expected") };
    assert_eq!(tips[0].data, [65535, 32896, 65535, 0], "alpha is the density, not the white luminance");
    let white = imported.presets[0].brush.csp_tip_colors.first().cloned().flatten().map(|c| c.rgb);
    assert_eq!(white, Some(vec![255; 12]));
    assert!(!imported.warnings.iter().any(|w| w == COLOUR_TIP), "{:?}", imported.warnings);
    assert!(!imported.warnings.iter().any(|w| w.contains("thumbnail")));
}

#[test]
fn spray_size_synchronized_with_the_brush_keeps_its_ratio() {
    let fields = [
        ("BrushUseSpray", Value::Integer(1)),
        ("BrushSize", number(500.0)),
        ("BrushSpraySize", number(100.0)),
        ("BrushSpraySizeSyncBrushSize", Value::Integer(1)),
    ];
    let imported = read_sut(&variant_fields_fixture(&fields), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let spray = imported.presets[0].brush.csp_spray.as_ref().unwrap();
    assert_eq!(spray.size_ratio, Some(0.2));
    assert!(!imported.warnings.iter().any(|w| w.contains("SyncBrushSize")), "{:?}", imported.warnings);
    let unsynced = read_sut(&variant_fields_fixture(&fields[..3]), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(unsynced.presets[0].brush.csp_spray.as_ref().unwrap().size_ratio, None);
}

#[test]
fn random_effector_control_maps_to_a_csp_random_factor() {
    let mut random = effector_blob(0x1f0, 0x80);
    random[24..28].copy_from_slice(&40u32.to_be_bytes());
    let imported = read_sut(&variant_fields_fixture(&[("BrushSizeEffector", Value::Blob(random.clone()))]), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let sd = &imported.presets[0].brush.shape_dynamics;
    assert!(sd.enabled);
    assert_eq!(sd.size.csp_random, Some(0.4));
    assert!(!imported.warnings.iter().any(|w| w.contains("additional input controls")), "{:?}", imported.warnings);
    // With pressure as well, both apply; an out-of-range minimum is capped at 100 %.
    let mut both = effector_blob(0x1f0, 0x90);
    both[24..28].copy_from_slice(&250u32.to_be_bytes());
    let imported = read_sut(&variant_fields_fixture(&[("BrushSizeEffector", Value::Blob(both))]), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let size = &imported.presets[0].brush.shape_dynamics.size;
    assert_eq!((size.control, size.csp_random), (prism_paint::Control::PenPressure, Some(1.0)));
}

#[test]
fn ribbon_maps_quarter_turns_and_needs_tips_without_spray() {
    let bytes = fixture();
    let db = Database::new(&bytes).unwrap();
    let node: Row = [("NodeName".into(), text("Ribbon")), ("NodeVariantID".into(), Value::Integer(1))].into();
    let import = |fields: &[(&str, Value)]| {
        let mut row: Row = [
            ("BrushRibbon".into(), Value::Integer(1)),
            ("BrushUsePatternImage".into(), Value::Integer(1)),
            ("BrushPatternImageArray".into(), Value::Blob(reference_blob(&["first.layer"]))),
            // A ribbon follows the rotation even when rotation is off (flag 0x01 clear).
            ("BrushRotationEffector".into(), Value::Integer(0)),
        ]
        .into();
        for (k, v) in fields {
            row.insert((*k).into(), v.clone());
        }
        read_tool(&node, &row, &|| db.table("MaterialFile"), "SUT", &prism_raster::Interrupt::NONE).unwrap()
    };
    let turns = |rotation: f64| import(&[("BrushRotation", number(rotation))]).presets[0].brush.csp_ribbon.map(|r| r.quarter_turns);
    for (rotation, expected) in [(0.0, 0), (44.9, 0), (90.0, 1), (100.0, 1), (180.0, 2), (270.0, 3), (315.0, 0), (360.0, 0), (-90.0, 3), (195.8, 2)] {
        assert_eq!(turns(rotation), Some(expected), "rotation {rotation}");
    }
    let imported = import(&[("BrushRotation", number(90.0))]);
    assert!(!imported.warnings.iter().any(|w| w.contains("Ribbon")), "{:?}", imported.warnings);
    assert!(matches!(imported.presets[0].brush.tip, TipShape::Sampled(_)));
    // Without tips (or with spraying) CSP draws no ribbon.
    assert_eq!(import(&[("BrushUsePatternImage", Value::Integer(0))]).presets[0].brush.csp_ribbon, None);
    assert_eq!(import(&[("BrushPatternImageArray", Value::Null)]).presets[0].brush.csp_ribbon, None);
    assert_eq!(import(&[("BrushUseSpray", Value::Integer(1))]).presets[0].brush.csp_ribbon, None);
    assert_eq!(import(&[("BrushRibbon", Value::Integer(0))]).presets[0].brush.csp_ribbon, None);
    // Hostile rotations still give a quarter turn.
    for rotation in [f64::NAN, f64::INFINITY, -1e300, 1e300] {
        assert!(turns(rotation).is_some_and(|t| t < 4));
    }
    // Ribbons reset colour mixing type 1 to type 0, so it is not approximated.
    let mixing = [("BrushUseWaterColor", Value::Integer(1)), ("BrushWaterColor", Value::Integer(1))];
    assert!(!import(&mixing).warnings.iter().any(|w| w.contains("color mixing type")));
    let mut no_ribbon = mixing.to_vec();
    no_ribbon.push(("BrushRibbon", Value::Integer(0)));
    assert!(import(&no_ribbon).warnings.iter().any(|w| w.contains("color mixing type 1")));
}
