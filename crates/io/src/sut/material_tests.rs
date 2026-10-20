use super::*;
use std::io::Write;

fn u32be(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn name(out: &mut Vec<u8>, value: &str) {
    u32be(out, value.len() as u32);
    for c in value.encode_utf16() {
        out.extend_from_slice(&c.to_be_bytes());
    }
}
fn tile_fixture(width: u32, height: u32, values: &[u8]) -> (Vec<u8>, Vec<u8>) {
    tile_fixture_planes(width, height, values, None)
}
fn tile_fixture_planes(width: u32, height: u32, values: &[u8], gray: Option<&[u8]>) -> (Vec<u8>, Vec<u8>) {
    tile_fixture_kind(width, height, values, gray, false)
}
/// One 256² tile: an alpha plane, then a gray plane for gray materials; a monochrome tile
/// packs alpha one bit per pixel (non-zero values set the bit).
fn tile_fixture_kind(width: u32, height: u32, values: &[u8], gray: Option<&[u8]>, mono: bool) -> (Vec<u8>, Vec<u8>) {
    let planes = if gray.is_some() { 2 } else { 1 };
    let mut plane = vec![0; if mono { 8192 } else { 65536 * planes }];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let v = values[y * width as usize + x];
            if mono {
                if v != 0 {
                    plane[y * 32 + x / 8] |= 0x80 >> (x % 8);
                }
            } else {
                plane[y * 256 + x] = v;
            }
            if let Some(gray) = gray {
                plane[65536 + y * 256 + x] = gray[y * width as usize + x];
            }
        }
    }
    let layout: &[u32] = if gray.is_some() {
        &[17, 1, 1, 2, 65536, 1, 256, 1, 256, 65536, 256, 256, 8, 8]
    } else if mono {
        &[1, 1, 0, 1, 0, 0, 0, 1, 32, 0, 256, 256, 8, 8, 1, 0]
    } else {
        &[1, 1, 0, 1, 0, 0, 0, 1, 256, 0, 256, 256, 8, 8]
    };
    wrap_tile(width, height, &plane, layout)
}
/// A colour tile: an alpha plane, then blue, green, red and an unused byte per pixel.
pub(in crate::sut) fn tile_fixture_color(width: u32, height: u32, alpha: &[u8], bgr: &[[u8; 3]]) -> (Vec<u8>, Vec<u8>) {
    let mut plane = vec![0; 65536 * 5];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let i = y * width as usize + x;
            plane[y * 256 + x] = alpha[i];
            plane[65536 + (y * 256 + x) * 4..][..3].copy_from_slice(&bgr[i]);
        }
    }
    wrap_tile(width, height, &plane, &[33, 1, 4, 5, 65536, 4, 1024, 1, 256, 65536, 256, 256, 8, 8])
}
/// One compressed 256² tile (`plane`, already laid out) with its attribute header.
fn wrap_tile(width: u32, height: u32, plane: &[u8], layout: &[u32]) -> (Vec<u8>, Vec<u8>) {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(plane).unwrap();
    let compressed = z.finish().unwrap();
    let mut block = vec![0; 4];
    name(&mut block, "BlockDataBeginChunk");
    u32be(&mut block, 0);
    u32be(&mut block, plane.len() as u32);
    block.extend_from_slice(&[0, 0, 1, 0, 0, 0, 1, 0]);
    u32be(&mut block, 1);
    u32be(&mut block, compressed.len() as u32 + 4);
    let payload_at = block.len();
    block.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    block.extend_from_slice(&compressed);
    let checksum = adler32(&block[payload_at..]);
    name(&mut block, "BlockDataEndChunk");
    let block_len = block.len();
    block[..4].copy_from_slice(&(block_len as u32).to_be_bytes());
    for (label, value) in [("BlockStatus", 1), ("BlockCheckSum", checksum)] {
        name(&mut block, label);
        for n in [12, 1, 4, value] {
            u32be(&mut block, n);
        }
    }
    let mut param = Vec::new();
    name(&mut param, "Parameter");
    for n in [width - 1, height - 1, 1, 1].iter().chain(layout) {
        u32be(&mut param, *n);
    }
    let mut init = Vec::new();
    name(&mut init, "InitColor");
    for n in [20, 0, 0, 0, 4] {
        u32be(&mut init, n);
    }
    let mut sizes = Vec::new();
    name(&mut sizes, "BlockSize");
    for n in [12, 1, 4, block_len as u32] {
        u32be(&mut sizes, n);
    }
    let mut attr = Vec::new();
    for n in [16, param.len() as u32, init.len() as u32, sizes.len() as u32] {
        u32be(&mut attr, n);
    }
    attr.extend(param);
    attr.extend(init);
    attr.extend(sizes);
    (attr, block)
}
fn varint(mut n: u64) -> Vec<u8> {
    let mut b = vec![(n & 127) as u8];
    while {
        n >>= 7;
        n != 0
    } {
        b.push((n & 127) as u8 | 128);
    }
    b.reverse();
    b
}
fn record(values: &[Value], encoding: u8) -> Vec<u8> {
    let (mut types, mut body) = (Vec::new(), Vec::new());
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
            Value::Blob(v) => {
                body.extend_from_slice(v);
                12 + 2 * v.len() as u64
            }
            Value::Text(v) => {
                let bytes = if encoding == 2 { v.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>() } else { v.as_bytes().to_vec() };
                body.extend_from_slice(&bytes);
                13 + 2 * bytes.len() as u64
            }
        };
        types.extend(varint(t));
    }
    let mut h = types.len() + 1;
    while h != types.len() + varint(h as u64).len() {
        h = types.len() + varint(h as u64).len();
    }
    let mut out = varint(h as u64);
    out.extend(types);
    out.extend(body);
    out
}
fn leaf(bytes: &mut Vec<u8>, id: usize, rows: &[Vec<Value>], encoding: u8) {
    leaf_sized(bytes, id, rows, encoding, 1024);
}
fn leaf_sized(bytes: &mut Vec<u8>, id: usize, rows: &[Vec<Value>], encoding: u8, page: usize) {
    // SQLite's local payload limits for a leaf table page of `page` usable bytes.
    let (max_local, min_local, chunk) = (page - 35, (page - 12) * 32 / 255 - 23, page - 4);
    let mut cells = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let r = record(row, encoding);
        let mut cell = varint(r.len() as u64);
        cell.extend(varint(i as u64 + 1));
        let local = if r.len() <= max_local {
            r.len()
        } else {
            let k = min_local + (r.len() - min_local) % chunk;
            if k <= max_local { k } else { min_local }
        };
        cell.extend_from_slice(&r[..local]);
        if local < r.len() {
            let first = bytes.len() / page + 1;
            cell.extend_from_slice(&(first as u32).to_be_bytes());
            let chunks: Vec<_> = r[local..].chunks(chunk).collect();
            for (j, chunk) in chunks.iter().enumerate() {
                let at = bytes.len();
                bytes.resize(at + page, 0);
                let next = if j + 1 == chunks.len() { 0 } else { (first + j + 1) as u32 };
                bytes[at..at + 4].copy_from_slice(&next.to_be_bytes());
                bytes[at + 4..at + 4 + chunk.len()].copy_from_slice(chunk);
            }
        }
        cells.push(cell);
    }
    let size = page;
    let page = &mut bytes[(id - 1) * size..id * size];
    page[0] = 13;
    page[3..5].copy_from_slice(&(rows.len() as u16).to_be_bytes());
    let mut at = size;
    for (i, cell) in cells.iter().enumerate() {
        at -= cell.len();
        page[at..at + cell.len()].copy_from_slice(cell);
        page[8 + i * 2..10 + i * 2].copy_from_slice(&(at as u16).to_be_bytes());
    }
    page[5..7].copy_from_slice(&(at as u16).to_be_bytes());
}
fn text(v: &str) -> Value {
    Value::Text(v.into())
}
fn int(v: i64) -> Value {
    Value::Integer(v)
}
fn database(encoding: u8) -> Vec<u8> {
    database_sized(encoding, 1024, tile_fixture(2, 2, &[0, 64, 128, 255]))
}
pub(in crate::sut) fn database_sized(encoding: u8, page: usize, tiles: (Vec<u8>, Vec<u8>)) -> Vec<u8> {
    database_with_opacity(encoding, page, tiles, 256)
}
fn database_with_opacity(encoding: u8, page: usize, (attr, blocks): (Vec<u8>, Vec<u8>), opacity: i64) -> Vec<u8> {
    let specs = [
        (
            "Layer",
            vec![
                "_PW_ID",
                "MainId",
                "CanvasId",
                "LayerFolder",
                "LayerOffsetX",
                "LayerOffsetY",
                "LayerRenderOffscrOffsetX",
                "LayerRenderOffscrOffsetY",
                "LayerComposite",
                "LayerMasking",
                "LayerOpacity",
                "LayerVisibility",
                "ResizableOriginalMipmap",
            ],
            vec![Value::Null, int(3), int(0), int(0), int(0), int(0), int(0), int(0), int(0), int(0), int(opacity), int(1), int(4)],
        ),
        ("Mipmap", vec!["_PW_ID", "MainId", "CanvasId", "LayerId", "MipmapCount", "BaseMipmapInfo"], vec![Value::Null, int(4), int(0), int(3), int(1), int(6)]),
        (
            "MipmapInfo",
            vec!["_PW_ID", "MainId", "CanvasId", "LayerId", "ThisScale", "Offscreen", "NextIndex"],
            vec![Value::Null, int(6), int(0), int(3), int(100), int(9), int(0)],
        ),
        (
            "Offscreen",
            vec!["_PW_ID", "MainId", "CanvasId", "LayerId", "Attribute", "BlockData"],
            vec![Value::Null, int(9), int(0), int(3), Value::Blob(attr), Value::Blob(blocks)],
        ),
    ];
    let mut db = vec![0; 13 * page];
    for (i, (table, columns, row)) in specs.into_iter().enumerate() {
        let sql = format!("CREATE TABLE {table}({})", columns.iter().map(|c| format!("{c} BLOB DEFAULT NULL")).collect::<Vec<_>>().join(", "));
        leaf_sized(&mut db, 6 + i, &[vec![text("table"), text(table), text(table), int((10 + i) as i64), text(&sql)]], encoding, page);
        leaf_sized(&mut db, 10 + i, &[row], encoding, page);
    }
    db
}
pub(in crate::sut) fn c2f(pages: &[u8]) -> Vec<u8> {
    let mut b = b"\x89C2F\r\n\x1a\n".to_vec();
    let mut encoded = vec![0; 5130];
    encoded[0] = 1;
    let mut plain = vec![0; 2];
    plain.extend_from_slice(&pages[5120..]);
    for (tag, payload) in [(b"HEAD", &[][..]), (b"dATA", encoded.as_slice()), (b"dATA", plain.as_slice()), (b"TAIL", &[][..])] {
        b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        let at = b.len();
        b.extend_from_slice(tag);
        b.extend_from_slice(payload);
        let crc = crc32(&b[at..]);
        b.extend_from_slice(&crc.to_le_bytes());
    }
    b
}
pub(in crate::sut) fn tar(layer: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    for (name, body) in [("data/material.layer", layer), ("thumbnail/thumbnail.png", b"\x89PNG\r\n\x1a\nnot the raster".as_slice())] {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[124..136].copy_from_slice(format!("{:011o}\0", body.len()).as_bytes());
        h[148..156].fill(b' ');
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        let checksum: usize = h.iter().map(|b| *b as usize).sum();
        h[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        b.extend(h);
        b.extend_from_slice(body);
        b.resize(b.len().div_ceil(512) * 512, 0);
    }
    b.resize(b.len() + 1024, 0);
    b
}

#[test]
fn original_raster_is_linked_from_both_schema_encodings_instead_of_thumbnail() {
    for encoding in [1, 2] {
        let image = raster(&tar(&c2f(&database(encoding)))).unwrap().unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.alpha, [0, 64, 128, 255]);
    }
}
#[test]
fn non_tar_keeps_explicit_legacy_path_and_recognized_corrupt_tar_errors() {
    assert!(raster(b"\x89PNG\r\n\x1a\nfixture").unwrap().is_none());
    let valid = tar(&c2f(&database(1)));
    let mut broken = valid.clone();
    broken[0] ^= 1;
    assert!(raster(&broken).is_err());
    for n in [262, 512, valid.len() - 1025] {
        assert!(raster(&valid[..n]).is_err());
    }
    let mut encoded = c2f(&database(1));
    encoded[33] ^= 1;
    assert!(raster(&tar(&encoded)).is_err());
}
#[test]
fn tile_channels_extents_compression_and_checksums_are_validated() {
    let (attr, blocks) = tile_fixture(2, 2, &[0, 64, 128, 255]);
    assert_eq!(decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).unwrap().alpha, [0, 64, 128, 255]);
    let mut bad = attr.clone();
    bad[38..42].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(decode_tiles(&bad, &blocks, DEFAULT_MAX_PIXELS).is_err());
    let mut bad = attr.clone();
    bad[54..58].copy_from_slice(&5u32.to_be_bytes());
    assert!(decode_tiles(&bad, &blocks, DEFAULT_MAX_PIXELS).is_err());
    let mut bad = blocks.clone();
    bad[80] ^= 127;
    assert!(decode_tiles(&attr, &bad, DEFAULT_MAX_PIXELS).is_err());
    let mut bad = blocks.clone();
    let end = bad.len();
    bad[end - 1] ^= 1;
    assert!(decode_tiles(&attr, &bad, DEFAULT_MAX_PIXELS).is_err());
    for len in [0, 1, 16, attr.len() - 1] {
        assert!(decode_tiles(&attr[..len], &blocks, DEFAULT_MAX_PIXELS).is_err());
    }
}
#[test]
fn partial_schema_rejects_text_impostors_unavailable_pages_and_cycles() {
    let mut db = database(1);
    leaf(&mut db, 6, &[vec![text("CREATE TABLE Layer(dummy)")]], 1);
    assert!(Database::partial_material_pages_sized(&db, 1, 1024).is_err());
    let mut db = database(1);
    leaf(&mut db, 6, &[vec![text("table"), text("Layer"), text("Layer"), int(4), text("CREATE TABLE Layer(dummy)")]], 1);
    assert!(Database::partial_material_pages_sized(&db, 1, 1024).is_err());
    let mut db = database(1);
    let p = &mut db[12 * 1024..13 * 1024];
    p.fill(0);
    p[0] = 5;
    p[8..12].copy_from_slice(&13u32.to_be_bytes());
    let parsed = Database::partial_material_pages_sized(&db, 1, 1024).unwrap();
    assert!(linked_raster(&parsed).is_err());
    assert!(Database::partial_material_pages_sized(&vec![0; LIMIT + 1024], 1, 1024).is_err());
}

#[test]
fn older_folder_rows_use_only_explicit_trailing_null_defaults() {
    let mut db = database(1);
    let original = Database::partial_material_pages_sized(&db, 1, 1024).unwrap().table("Layer").unwrap();
    let order = [
        "_PW_ID",
        "MainId",
        "CanvasId",
        "LayerFolder",
        "LayerOffsetX",
        "LayerOffsetY",
        "LayerRenderOffscrOffsetX",
        "LayerRenderOffscrOffsetY",
        "LayerComposite",
        "LayerMasking",
        "LayerOpacity",
        "LayerVisibility",
        "ResizableOriginalMipmap",
    ];
    let image: Vec<_> = order.iter().map(|key| original[0][*key].clone()).collect();
    leaf(&mut db, 10, &[vec![Value::Null, int(2), int(0), int(1)], image], 1);
    let parsed = Database::partial_material_pages_sized(&db, 1, 1024).unwrap();
    assert_eq!(linked_raster(&parsed).unwrap().alpha, [0, 64, 128, 255]);
}
#[test]
fn schema_duplicates_and_wrong_reference_ownership_are_rejected() {
    let mut db = database(1);
    let id = db.len() / 1024 + 1;
    db.resize(db.len() + 1024, 0);
    leaf(&mut db, id, &[vec![text("table"), text("Layer"), text("Layer"), int(10), text("CREATE TABLE Layer(dummy)")]], 1);
    assert!(Database::partial_material_pages_sized(&db, 1, 1024).is_err());
    let mut db = database(1);
    leaf(&mut db, 11, &[vec![Value::Null, int(4), int(0), int(99), int(1), int(6)]], 1);
    let parsed = Database::partial_material_pages_sized(&db, 1, 1024).unwrap();
    assert!(linked_raster(&parsed).is_err());
}

#[test]
fn four_tiles_keep_native_resolution_and_crop_only_the_canvas_edges() {
    let (mut attr, _) = tile_fixture(1, 1, &[0]);
    attr[38..42].copy_from_slice(&256u32.to_be_bytes());
    attr[42..46].copy_from_slice(&256u32.to_be_bytes());
    attr[46..50].copy_from_slice(&2u32.to_be_bytes());
    attr[50..54].copy_from_slice(&2u32.to_be_bytes());
    let old_section = be(&attr, 12).unwrap() as usize;
    attr.truncate(attr.len() - old_section);
    let mut sizes = Vec::new();
    name(&mut sizes, "BlockSize");
    for n in [12, 4, 4] {
        u32be(&mut sizes, n);
    }
    let mut blocks = Vec::new();
    for (i, value) in [11, 22, 33, 44].into_iter().enumerate() {
        let (_, mut tile) = tile_fixture(256, 256, &vec![value; 65536]);
        let len = be(&tile, 0).unwrap() as usize;
        tile[46..50].copy_from_slice(&(i as u32).to_be_bytes());
        blocks.extend_from_slice(&tile[..len]);
        u32be(&mut sizes, len as u32);
    }
    attr[12..16].copy_from_slice(&(sizes.len() as u32).to_be_bytes());
    attr.extend(sizes);
    name(&mut blocks, "BlockStatus");
    for n in [12, 4, 4, 1, 1, 1, 1] {
        u32be(&mut blocks, n);
    }
    let raster = decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).unwrap();
    assert_eq!((raster.width, raster.height), (257, 257));
    assert_eq!([raster.alpha[0], raster.alpha[256], raster.alpha[256 * 257], raster.alpha[257 * 257 - 1]], [11, 22, 33, 44]);
}

#[test]
fn tile_expansion_must_be_exactly_one_bounded_plane() {
    for expanded in [65535, 65537] {
        let (mut attr, original) = tile_fixture(1, 1, &[0]);
        let old_len = be(&original, 0).unwrap() as usize;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        z.write_all(&vec![0; expanded]).unwrap();
        let compressed = z.finish().unwrap();
        let mut blocks = original[..66].to_vec();
        u32be(&mut blocks, compressed.len() as u32 + 4);
        blocks.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        blocks.extend(compressed);
        name(&mut blocks, "BlockDataEndChunk");
        let len = blocks.len();
        blocks[..4].copy_from_slice(&(len as u32).to_be_bytes());
        let end = attr.len();
        attr[end - 4..].copy_from_slice(&(len as u32).to_be_bytes());
        blocks.extend_from_slice(&original[old_len..]);
        assert!(decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).is_err());
    }
}

#[test]
fn gray_materials_decode_the_alpha_plane_then_the_gray_plane() {
    let (attr, blocks) = tile_fixture_planes(2, 2, &[0, 64, 128, 255], Some(&[10, 20, 30, 40]));
    let raster = decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).unwrap();
    assert_eq!((raster.width, raster.height), (2, 2));
    assert_eq!(raster.alpha, [0, 64, 128, 255]);
    assert_eq!(raster.gray.as_deref(), Some(&[10, 20, 30, 40][..]));
    assert_eq!(decode_tiles(&tile_fixture(1, 1, &[9]).0, &tile_fixture(1, 1, &[9]).1, DEFAULT_MAX_PIXELS).unwrap().gray, None);
    // A gray tile must expand to exactly two planes, and its block header must say so.
    let mut bad = blocks.clone();
    bad[50..54].copy_from_slice(&65536u32.to_be_bytes());
    assert!(decode_tiles(&attr, &bad, DEFAULT_MAX_PIXELS).is_err());
    let (alpha_attr, _) = tile_fixture(2, 2, &[0, 64, 128, 255]);
    assert!(decode_tiles(&alpha_attr, &blocks, DEFAULT_MAX_PIXELS).is_err(), "an alpha-only layout cannot read gray tiles");
}

#[test]
fn monochrome_materials_unpack_one_bit_alpha_most_significant_bit_first() {
    let values: Vec<u8> = (0..9 * 3).map(|i| if i % 3 == 0 || i == 8 { 255 } else { 0 }).collect();
    let (attr, blocks) = tile_fixture_kind(9, 3, &values, None, true);
    let raster = decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).unwrap();
    assert_eq!((raster.width, raster.height, raster.gray.is_none()), (9, 3, true));
    assert_eq!(raster.alpha, values);
    // The trailing mono marker must match the 32-byte rows.
    let mut bad = attr.clone();
    let at = 16 + 4 + 18 + 4 * 18;
    bad[at..at + 4].copy_from_slice(&0u32.to_be_bytes());
    assert!(decode_tiles(&bad, &blocks, DEFAULT_MAX_PIXELS).is_err());
}

#[test]
fn materials_with_4096_byte_pages_are_recovered_past_the_encoded_prefix() {
    let db = database_sized(1, 4096, tile_fixture_kind(2, 2, &[0, 255, 255, 0], None, true));
    let blob = tar(&c2f(&db));
    let raster = raster(&blob).unwrap().unwrap();
    assert_eq!((raster.width, raster.height, raster.alpha.as_slice()), (2, 2, &[0, 255, 255, 0][..]));
    // The same pages read as 1024-byte pages have no recoverable schema.
    assert!(Database::partial_material_pages_sized(&db, 1, 1024).is_err());
    assert!(Database::partial_material_pages_sized(&db, 1, 4096).is_ok());
    assert!(Database::partial_material_pages_sized(&db, 1, 2048).is_err(), "only observed page sizes");
    assert!(Database::partial_material_pages_sized(&db[..4096 * 2], 1, 4096).is_err());
}

/// Every tip and texture package of every downloaded `.tool` below `PRISM_TEST_CSP_MATERIALS`
/// (a CLIP STUDIO `Material` folder) decodes from its original raster.
#[test]
#[ignore = "reads the local CLIP STUDIO material folder"]
fn local_tool_materials_decode() {
    let Some(root) = std::env::var_os("PRISM_TEST_CSP_MATERIALS") else { return };
    let mut stack = vec![std::path::PathBuf::from(root)];
    let (mut ok, mut failed) = (0, Vec::new());
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "material_0.tool") {
                let decoded = crate::csp_container::decode(&std::fs::read(&path).unwrap(), &prism_raster::Interrupt::NONE).unwrap();
                let db = Database::new(&decoded).unwrap();
                for row in db.table("ToolPatternInMaterial").unwrap_or_default() {
                    let Some(blob) = row.get("FileData").and_then(Value::blob) else { continue };
                    match raster(blob) {
                        Ok(Some(_)) => ok += 1,
                        other => failed.push(format!("{}: {:?}", path.display(), other.err())),
                    }
                }
            }
        }
    }
    eprintln!("{ok} decoded");
    assert!(failed.is_empty(), "{failed:#?}");
}


#[test]
fn colour_tiles_read_as_luminance_and_say_so() {
    let (attr, blocks) = tile_fixture_color(2, 2, &[255, 255, 128, 0], &[[0, 0, 255], [255, 255, 255], [0, 0, 0], [9, 9, 9]]);
    let r = decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).unwrap();
    assert_eq!(r.alpha, [255, 255, 128, 0]);
    // Blue, green, red order: pure red has Rec. 601 luma 76.
    assert_eq!(r.gray.as_deref(), Some(&[76, 255, 0, 9][..]));
    assert!(r.color);
    assert!(!decode_tiles(&tile_fixture(2, 2, &[1, 2, 3, 4]).0, &tile_fixture(2, 2, &[1, 2, 3, 4]).1, DEFAULT_MAX_PIXELS).unwrap().color);
}

#[test]
fn colour_tiles_keep_their_rgb_and_average_it_by_alpha() {
    let (attr, blocks) = tile_fixture_color(2, 2, &[255, 255, 128, 0], &[[0, 0, 255], [255, 255, 255], [0, 0, 0], [9, 9, 9]]);
    let full = decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).unwrap();
    assert_eq!(full.rgb.as_deref(), Some(&[255, 0, 0, 255, 255, 255, 0, 0, 0, 9, 9, 9][..]));
    // Reduced to one pixel: (255·red + 255·white + 128·black) / 638, truncated like gray; the
    // clear pixel counts nothing.
    let one = decode_tiles(&attr, &blocks, 1).unwrap();
    assert_eq!(one.rgb.as_deref(), Some(&[203, 101, 101][..]));
    assert_eq!(decode_tiles(&tile_fixture(2, 2, &[1, 2, 3, 4]).0, &tile_fixture(2, 2, &[1, 2, 3, 4]).1, DEFAULT_MAX_PIXELS).unwrap().rgb, None);
}

#[test]
fn layer_opacity_scales_the_alpha() {
    let blob = tar(&c2f(&database_with_opacity(1, 1024, tile_fixture(2, 2, &[0, 64, 128, 255]), 128)));
    assert_eq!(raster(&blob).unwrap().unwrap().alpha, [0, 32, 64, 128]);
    assert!(raster(&tar(&c2f(&database_with_opacity(1, 1024, tile_fixture(2, 2, &[0, 64, 128, 255]), 257)))).is_err());
}

#[test]
fn large_materials_are_area_averaged_to_the_pixel_budget() {
    let (attr, blocks) = tile_fixture_planes(2, 2, &[0, 64, 128, 255], Some(&[0, 0, 200, 100]));
    let r = decode_tiles(&attr, &blocks, 1).unwrap();
    assert_eq!((r.width, r.height, r.reduced), (1, 1, 2.0));
    assert_eq!(r.alpha, [111]);
    // Gray is weighted by alpha: (128·200 + 255·100) / 447.
    assert_eq!(r.gray.as_deref(), Some(&[114][..]));
    let full = decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).unwrap();
    assert_eq!((full.width, full.reduced), (2, 1.0));
}

#[test]
fn tiles_may_cover_only_the_stored_extent() {
    // 257 wide in one 256-pixel column (stored extent 256): the last column stays transparent.
    let values = vec![200u8; 257];
    let (attr, blocks) = tile_fixture(257, 1, &values);
    let r = decode_tiles(&attr, &blocks, DEFAULT_MAX_PIXELS).unwrap();
    assert_eq!((r.width, r.alpha[255], r.alpha[256]), (257, 200, 0));
}
