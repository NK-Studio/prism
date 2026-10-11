use super::*;
use prism_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};

// Synthetic SQLite fixtures built from the public spec; no third-party brush assets.
fn vi(mut n: u64) -> Vec<u8> {
    let mut v = vec![(n & 127) as u8];
    n >>= 7;
    while n > 0 {
        v.push((n & 127) as u8 | 128);
        n >>= 7;
    }
    v.reverse();
    v
}
fn rec(values: &[Value]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut body = Vec::new();
    for v in values {
        let t = match v {
            Value::Null => 0,
            Value::Number(v) => {
                body.extend_from_slice(&v.to_be_bytes());
                7
            }
            Value::Text(s) => {
                body.extend_from_slice(s.as_bytes());
                13 + s.len() as u64 * 2
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
fn leaf(db: &mut Vec<u8>, id: usize, rows: &[Vec<Value>]) {
    const P: usize = 4096;
    let mut cells = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let r = rec(row);
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
fn text(s: &str) -> Value {
    Value::Text(s.into())
}
fn number(n: f64) -> Value {
    Value::Number(n)
}
fn fixture() -> Vec<u8> {
    fixture_with(true, None)
}
/// `texture`: the reference blob for TextureImage (NULL when `None`).
fn fixture_with(with_refs: bool, texture: Option<Vec<u8>>) -> Vec<u8> {
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
                text("CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL, BrushUsePatternImage INTEGER, BrushPatternImageArray BLOB, BrushInterval REAL, TextureImage BLOB, TextureScale2 REAL, TextureReverseDensity INTEGER, TextureForPlot INTEGER, TextureDensity INTEGER, TextureCompositeMode INTEGER)"),
            ],
            vec![text("table"), text("MaterialFile"), text("MaterialFile"), number(4.0), text("CREATE TABLE MaterialFile (OriginalPath TEXT, FileData BLOB)")],
        ],
    );
    leaf(&mut db, 2, &[vec![text("Synthetic SUT"), number(818.0)]]);
    let paths = ["first.layer", "second.layer"];
    let mut refs = vec![0, 0, 0, 8, 0, 0, 0, 2];
    for p in paths {
        refs.extend(p.encode_utf16().flat_map(u16::to_le_bytes));
    }
    let refs = if with_refs { Value::Blob(refs) } else { Value::Null };
    let texture = texture.map_or(Value::Null, Value::Blob);
    leaf(
        &mut db,
        3,
        &[vec![number(818.0), number(17.0), number(1.0), refs, number(10.0), texture, number(50.0), number(1.0), number(1.0), number(40.0), number(2.0)]],
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
    let mut refs = vec![0, 0, 0, 8];
    refs.extend(count.to_be_bytes());
    refs.extend(path.encode_utf16().flat_map(u16::to_le_bytes));
    refs
}
#[test]
fn maps_texture_material_and_settings() {
    let imp = read_sut(&fixture_with(true, Some(texture_refs("second.layer", 1))), "SUT", &prism_raster::Interrupt::NONE).unwrap();
    let t = &imp.presets[0].brush.texture;
    assert!(t.enabled);
    // White, black, black, transparent (reads as white): luminance, not coverage.
    assert_eq!(t.pattern, Pattern::Tile(GrayTile { width: 2, height: 2, data: vec![65535, 0, 0, 65535] }));
    assert_eq!((t.scale, t.invert, t.each_tip, t.depth), (0.5, true, true, 0.4));
    assert!(!imp.warnings.iter().any(|w| w.contains("texture could not be read")));
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
