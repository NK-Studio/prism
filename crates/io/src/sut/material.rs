//! Limited original-raster decoding for the observed SUT material containers.
//! Reads ustar/C2F framing and linked SQLite raster tables, never a thumbnail.
//! The whole layer file is decoded with the C2F container cipher when it can be; otherwise
//! the plaintext pages after the encoded prefix are recovered. Supported tiles: 8-bit
//! black/alpha, monochrome, gray+alpha and colour+alpha (read as its luminance).
use super::{
    Row,
    sqlite::{Database, Value},
};
use std::io::Read;

type Result<T> = std::result::Result<T, String>;
const LIMIT: usize = 64 << 20;
/// Largest material read, in pixels: downloaded tips reach 14418 × 1244 and 5703².
const MAX_EDGE: u32 = 16384;
const MAX_SOURCE_PIXELS: usize = 64 << 20;
/// Size a material is reduced to fit unless the caller asks otherwise (see [`raster_within`]).
pub(super) const DEFAULT_MAX_PIXELS: usize = 4 << 20;
fn invalid() -> String {
    "invalid or unsupported SUT original material raster".into()
}
fn part(b: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    b.get(at..at.checked_add(n).ok_or_else(invalid)?).ok_or_else(invalid)
}
fn be(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(part(b, at, 4)?.try_into().map_err(|_| invalid())?))
}
fn le(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(part(b, at, 4)?.try_into().map_err(|_| invalid())?))
}
fn octal(b: &[u8]) -> Result<usize> {
    let s = std::str::from_utf8(b).map_err(|_| invalid())?.trim_matches(['\0', ' ']);
    usize::from_str_radix(s, 8).map_err(|_| invalid())
}
fn crc32(b: &[u8]) -> u32 {
    let mut c = !0u32;
    for byte in b {
        c ^= u32::from(*byte);
        for _ in 0..8 {
            c = (c >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(c & 1));
        }
    }
    !c
}

pub(super) struct MaterialRaster {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
    /// Gray level per pixel for gray materials (alpha-only materials are black).
    pub gray: Option<Vec<u8>>,
    /// How much the material's larger edge was reduced to fit (1 = original size).
    pub reduced: f32,
    /// A colour material: `gray` is its luminance, and CLIP STUDIO draws such a brush tip in
    /// its own colours unless the brush changes the tip colour.
    pub color: bool,
    /// A colour material's straight red, green and blue, three bytes a pixel.
    pub rgb: Option<Vec<u8>>,
}

/// Non-tar fixtures retain the legacy PNG path. A recognized but unsupported
/// material returns an error so the caller can explicitly report thumbnail fallback.
#[cfg(test)]
pub(super) fn raster(blob: &[u8]) -> Result<Option<MaterialRaster>> {
    raster_within(blob, DEFAULT_MAX_PIXELS)
}

/// The original raster of a material package (`None` for a non-tar fixture), area-averaged
/// down (aspect kept) when it has more than `max_pixels` pixels.
pub(super) fn raster_within(blob: &[u8], max_pixels: usize) -> Result<Option<MaterialRaster>> {
    if blob.get(257..262) != Some(b"ustar") {
        return Ok(None);
    }
    if blob.len() > LIMIT {
        return Err(invalid());
    }
    let layer = tar_layer(blob)?;
    // The complete database, when the container decodes (its prefix is enciphered).
    let full_error = match crate::csp_container::decode(layer, &prism_raster::Interrupt::NONE) {
        Ok(full) if full.starts_with(b"SQLite format 3\0") => match Database::new(&full).and_then(|db| linked_raster_within(&db, max_pixels)) {
            Ok(image) => return Ok(Some(image)),
            Err(e) => Some(e),
        },
        Ok(_) => None,
        Err(e) => Some(e),
    };
    let pages = match material_pages(layer) {
        Ok(pages) => pages,
        Err(e) => return Err(full_error.unwrap_or(e)),
    };
    let mut result = None;
    // SQLite's encoded header is unavailable. Both observed text encodings must
    // satisfy the complete recovered schema and all image-reference checks.
    for (encoding, page_size) in [(1, 1024), (2, 1024), (1, 4096), (2, 4096)] {
        if let Ok(db) = Database::partial_material_pages_sized(&pages, encoding, page_size)
            && let Ok(image) = linked_raster_within(&db, max_pixels)
        {
            if result.is_some() {
                return Err("ambiguous SUT material text encoding".into());
            }
            result = Some(image);
        }
    }
    result.map(Some).ok_or_else(|| full_error.unwrap_or_else(|| "SUT original raster is unavailable or uses unsupported material tables/pixels".into()))
}

fn tar_layer(b: &[u8]) -> Result<&[u8]> {
    let mut at = 0;
    let mut layer = None;
    for _ in 0..64 {
        let h = part(b, at, 512)?;
        if h.iter().all(|v| *v == 0) {
            if b.get(at..).is_some_and(|tail| tail.iter().all(|v| *v == 0)) {
                return layer.ok_or_else(invalid);
            }
            return Err(invalid());
        }
        if h.get(257..262) != Some(b"ustar") {
            return Err(invalid());
        }
        let checksum = h.iter().enumerate().map(|(i, v)| if (148..156).contains(&i) { 32usize } else { usize::from(*v) }).sum::<usize>();
        if octal(part(h, 148, 8)?)? != checksum {
            return Err("invalid SUT material tar checksum".into());
        }
        let name_bytes = part(h, 0, 100)?;
        let end = name_bytes.iter().position(|v| *v == 0).unwrap_or(name_bytes.len());
        let name = std::str::from_utf8(part(name_bytes, 0, end)?).map_err(|_| invalid())?;
        let n = octal(part(h, 124, 12)?)?;
        let data = part(b, at.checked_add(512).ok_or_else(invalid)?, n)?;
        if name.starts_with("data/") && name.ends_with(".layer") {
            if layer.is_some() || !matches!(h.get(156), Some(0 | b'0')) || part(h, 345, 155)?.iter().any(|v| *v != 0) {
                return Err(invalid());
            }
            layer = Some(data);
        }
        let padded = n.checked_add(511).ok_or_else(invalid)? / 512 * 512;
        at = at.checked_add(512).and_then(|v| v.checked_add(padded)).ok_or_else(invalid)?;
    }
    Err(invalid())
}

fn material_pages(b: &[u8]) -> Result<Vec<u8>> {
    if b.len() > LIMIT || b.get(..8) != Some(b"\x89C2F\r\n\x1a\n") {
        return Err(invalid());
    }
    let mut at = 8;
    let mut pages = Vec::new();
    for expected in [b"HEAD", b"dATA", b"dATA", b"TAIL"] {
        let n = le(b, at)? as usize;
        let tagged = part(b, at + 4, n.checked_add(4).ok_or_else(invalid)?)?;
        if tagged.get(..4) != Some(expected.as_slice()) || crc32(tagged) != le(b, at.checked_add(8).and_then(|p| p.checked_add(n)).ok_or_else(invalid)?)? {
            return Err(invalid());
        }
        let data = tagged.get(4..).ok_or_else(invalid)?;
        match expected {
            b"HEAD" | b"TAIL" if !data.is_empty() => return Err(invalid()),
            b"dATA" if pages.is_empty() => {
                if n != 5130 || data.get(..2) != Some(&[1, 0]) {
                    return Err("unsupported SUT C2F encoded prefix".into());
                }
                pages.resize(5120, 0);
            }
            b"dATA" => {
                if data.get(..2) != Some(&[0, 0]) {
                    return Err("unsupported SUT C2F data encoding".into());
                }
                let plain = data.get(2..).ok_or_else(invalid)?;
                if !plain.len().is_multiple_of(1024) || plain.len() > LIMIT - pages.len() {
                    return Err(invalid());
                }
                pages.extend_from_slice(plain);
            }
            _ => {}
        }
        at = at.checked_add(12).and_then(|p| p.checked_add(n)).ok_or_else(invalid)?;
    }
    if at != b.len() {
        return Err(invalid());
    }
    Ok(pages)
}

fn integer(row: &Row, key: &str) -> Result<i64> {
    row.get(key).and_then(Value::integer).ok_or_else(invalid)
}
fn unique<'a>(rows: &'a [Row], key: &str, id: i64) -> Result<&'a Row> {
    let mut found = rows.iter().filter(|r| r.get(key).and_then(Value::integer) == Some(id));
    let row = found.next().ok_or_else(invalid)?;
    if found.next().is_some() {
        return Err("ambiguous SUT material reference".into());
    }
    Ok(row)
}
#[cfg(test)]
fn linked_raster(db: &Database<'_>) -> Result<MaterialRaster> {
    linked_raster_within(db, DEFAULT_MAX_PIXELS)
}
fn linked_raster_within(db: &Database<'_>, max_pixels: usize) -> Result<MaterialRaster> {
    let layers = db.table("Layer")?;
    let mut candidates = layers.iter().filter(|r| r.get("LayerFolder").and_then(Value::integer) == Some(0));
    let layer = candidates.next().ok_or_else(invalid)?;
    if candidates.next().is_some() {
        return Err("multi-layer SUT materials are unsupported".into());
    }
    for key in ["LayerOffsetX", "LayerOffsetY", "LayerRenderOffscrOffsetX", "LayerRenderOffscrOffsetY", "LayerComposite", "LayerMasking"] {
        if integer(layer, key)? != 0 {
            return Err(invalid());
        }
    }
    // Layer opacity is 0..256; a translucent material layer makes a lighter tip.
    let opacity = integer(layer, "LayerOpacity")?;
    if !(0..=256).contains(&opacity) || integer(layer, "LayerVisibility")? != 1 {
        return Err(invalid());
    }
    let mut raster = mipmap_raster(db, layer, integer(layer, "ResizableOriginalMipmap")?, max_pixels)?;
    if opacity < 256 {
        for a in &mut raster.alpha {
            *a = ((u32::from(*a) * opacity as u32 + 128) / 256) as u8;
        }
    }
    Ok(raster)
}

/// The pixels of `layer`'s mipmap `mipmap` at 100 % (one level), without the layer's opacity.
pub(super) fn mipmap_raster(db: &Database<'_>, layer: &Row, mipmap: i64, max_pixels: usize) -> Result<MaterialRaster> {
    let layer_id = integer(layer, "MainId")?;
    let canvas = integer(layer, "CanvasId")?;
    let mipmaps = db.table("Mipmap")?;
    let mip = unique(&mipmaps, "MainId", mipmap)?;
    if integer(mip, "MipmapCount")? != 1 || integer(mip, "LayerId")? != layer_id || integer(mip, "CanvasId")? != canvas {
        return Err(invalid());
    }
    let infos = db.table("MipmapInfo")?;
    let info = unique(&infos, "MainId", integer(mip, "BaseMipmapInfo")?)?;
    if integer(info, "ThisScale")? != 100 || integer(info, "NextIndex")? != 0 || integer(info, "LayerId")? != layer_id || integer(info, "CanvasId")? != canvas {
        return Err(invalid());
    }
    let offscreens = db.table("Offscreen")?;
    let off = unique(&offscreens, "MainId", integer(info, "Offscreen")?)?;
    if integer(off, "LayerId")? != layer_id || integer(off, "CanvasId")? != canvas {
        return Err(invalid());
    }
    decode_tiles(off.get("Attribute").and_then(Value::blob).ok_or_else(invalid)?, off.get("BlockData").and_then(Value::blob).ok_or_else(invalid)?, max_pixels)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let b = part(self.bytes, self.at, n)?;
        self.at += n;
        Ok(b)
    }
    fn uint(&mut self) -> Result<u32> {
        be(self.take(4)?, 0)
    }
    fn name(&mut self, expected: &str) -> Result<()> {
        let n = self.uint()? as usize;
        if n != expected.len() {
            return Err(invalid());
        }
        let bytes = self.take(n.checked_mul(2).ok_or_else(invalid)?)?;
        if !bytes.chunks_exact(2).zip(expected.bytes()).all(|(pair, byte)| pair == [0, byte]) {
            return Err(invalid());
        }
        Ok(())
    }
    fn done(&self) -> bool {
        self.at == self.bytes.len()
    }
}
/// Where decoded rows go: the material at full size, or area-averaged into a smaller grid.
struct Sink {
    width: usize,
    height: usize,
    /// Output size (the source size when it fits).
    out_w: usize,
    out_h: usize,
    alpha: Vec<u8>,
    gray: Option<Vec<u8>>,
    /// Colour materials: red, green, blue a pixel (full size).
    rgb: Option<Vec<u8>>,
    /// Reduced colour materials: alpha-weighted sums of red, green and blue a pixel.
    rgb_sums: Option<Vec<u64>>,
    /// Reduced output: per output pixel, Σ alpha, Σ alpha·gray and the source pixel count.
    sums: Option<(Vec<u64>, Vec<u64>, Vec<u32>)>,
}
impl Sink {
    fn new(width: u32, height: u32, gray: bool, rgb: bool, max_pixels: usize) -> Result<Self> {
        let (w, h) = (width as usize, height as usize);
        let pixels = w.checked_mul(h).filter(|n| *n <= MAX_SOURCE_PIXELS).ok_or("SUT material is too large")?;
        let max = max_pixels.max(1);
        let (out_w, out_h, sums) = if pixels <= max {
            (w, h, None)
        } else {
            let k = (max as f64 / pixels as f64).sqrt();
            let (out_w, out_h) = (((w as f64 * k) as usize).clamp(1, w), ((h as f64 * k) as usize).clamp(1, h));
            let n = out_w * out_h;
            (out_w, out_h, Some((vec![0; n], vec![0; n], vec![0; n])))
        };
        let n = out_w * out_h;
        let full = sums.is_none();
        Ok(Self {
            width: w,
            height: h,
            out_w,
            out_h,
            alpha: if full { vec![0; n] } else { Vec::new() },
            gray: gray.then(|| if full { vec![0; n] } else { Vec::new() }),
            rgb: (rgb && full).then(|| vec![0; n * 3]),
            rgb_sums: (rgb && !full).then(|| vec![0; n * 3]),
            sums,
        })
    }
    /// One source row segment starting at `(x, y)`; `rgb` holds three bytes a pixel.
    fn row(&mut self, x: usize, y: usize, alpha: &[u8], gray: Option<&[u8]>, rgb: Option<&[u8]>) -> Result<()> {
        match &mut self.sums {
            None => {
                let dest = y * self.width + x;
                self.alpha.get_mut(dest..dest + alpha.len()).ok_or_else(invalid)?.copy_from_slice(alpha);
                if let (Some(out), Some(gray)) = (&mut self.gray, gray) {
                    out.get_mut(dest..dest + gray.len()).ok_or_else(invalid)?.copy_from_slice(gray);
                }
                if let (Some(out), Some(rgb)) = (&mut self.rgb, rgb) {
                    out.get_mut(dest * 3..dest * 3 + rgb.len()).ok_or_else(invalid)?.copy_from_slice(rgb);
                }
            }
            Some((sa, sga, count)) => {
                let oy = y * self.out_h / self.height;
                for (dx, a) in alpha.iter().enumerate() {
                    let o = oy * self.out_w + (x + dx) * self.out_w / self.width;
                    let a = u64::from(*a);
                    *sa.get_mut(o).ok_or_else(invalid)? += a;
                    *sga.get_mut(o).ok_or_else(invalid)? += a * gray.and_then(|g| g.get(dx)).map_or(0, |g| u64::from(*g));
                    *count.get_mut(o).ok_or_else(invalid)? += 1;
                    if let (Some(out), Some(rgb)) = (&mut self.rgb_sums, rgb) {
                        for c in 0..3 {
                            *out.get_mut(o * 3 + c).ok_or_else(invalid)? += a * rgb.get(dx * 3 + c).map_or(0, |v| u64::from(*v));
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn finish(self) -> MaterialRaster {
        let (width, height) = (self.out_w as u32, self.out_h as u32);
        let reduced = self.width.max(self.height) as f32 / self.out_w.max(self.out_h).max(1) as f32;
        match self.sums {
            None => {
                MaterialRaster { width, height, alpha: self.alpha, gray: self.gray, reduced, color: false, rgb: self.rgb }
            }
            Some((sa, sga, count)) => {
                let alpha = sa.iter().zip(&count).map(|(a, n)| (a / u64::from((*n).max(1))) as u8).collect();
                let gray = self.gray.map(|_| sga.iter().zip(&sa).map(|(ga, a)| if *a == 0 { 0 } else { (ga / a) as u8 }).collect());
                // Colour is weighted by alpha, like gray.
                let rgb = self.rgb_sums.map(|v| {
                    v.chunks_exact(3).zip(&sa).flat_map(|(c, a)| c.iter().map(move |c| if *a == 0 { 0 } else { (c / a).min(255) as u8 })).collect()
                });
                MaterialRaster { width, height, alpha, gray, reduced, color: false, rgb }
            }
        }
    }
}
fn decode_tiles(attr: &[u8], blocks: &[u8], max_pixels: usize) -> Result<MaterialRaster> {
    if be(attr, 0)? != 16 {
        return Err(invalid());
    }
    let sizes = [be(attr, 4)? as usize, be(attr, 8)? as usize, be(attr, 12)? as usize];
    let mut a = Cursor::new(attr.get(16..).ok_or_else(invalid)?);
    let mut param = Cursor::new(a.take(sizes[0])?);
    param.name("Parameter")?;
    let width = param.uint()?.checked_add(1).filter(|v| *v <= MAX_EDGE).ok_or_else(invalid)?;
    let height = param.uint()?.checked_add(1).filter(|v| *v <= MAX_EDGE).ok_or_else(invalid)?;
    let cols = param.uint()?;
    let rows = param.uint()?;
    // The tiles cover the stored extent (width − 1), or the whole width: a downloaded material
    // stores 2048 with 8 columns, its last pixel column outside every tile (left transparent).
    let covers = |tiles: u32, size: u32| tiles == size.div_ceil(256) || tiles == (size - 1).div_ceil(256).max(1);
    if !covers(cols, width) || !covers(rows, height) {
        return Err(invalid());
    }
    // Supported layouts: a black material stores only alpha, one 8-bit 256² plane per tile; a
    // gray material stores an alpha plane, then a gray plane; a colour material an alpha plane,
    // then four bytes a pixel (blue, green, red, unused).
    let mut layout = [0u32; 14];
    for v in &mut layout {
        *v = param.uint()?;
    }
    // A monochrome material packs its alpha one bit per pixel (32 bytes a row, most
    // significant bit first) and marks it in the trailing word.
    let (planes, mono, color) = match layout {
        [1, 1, 0, 1, 0, 0, 0, 1, 256, 0, 256, 256, 8, 8] => (1, false, false),
        [1, 1, 0, 1, 0, 0, 0, 1, 32, 0, 256, 256, 8, 8] => (1, true, false),
        [17, 1, 1, 2, 65536, 1, 256, 1, 256, 65536, 256, 256, 8, 8] => (2, false, false),
        [33, 1, 4, 5, 65536, 4, 1024, 1, 256, 65536, 256, 256, 8, 8] => (5, false, true),
        _ => return Err("unsupported SUT material channel layout".into()),
    };
    let tile_len = if mono { 8192 } else { 65536 * planes };
    let trailing = if param.done() { [0, 0] } else { [param.uint()?, param.uint()?] };
    if !param.done() || trailing != [u32::from(mono), 0] {
        return Err(invalid());
    }
    let mut init = Cursor::new(a.take(sizes[1])?);
    init.name("InitColor")?;
    for expected in [20, 0, 0, 0, 4] {
        if init.uint()? != expected {
            return Err("unsupported SUT material initial color".into());
        }
    }
    if !init.done() {
        return Err(invalid());
    }
    let mut size = Cursor::new(a.take(sizes[2])?);
    size.name("BlockSize")?;
    let count = cols.checked_mul(rows).filter(|v| *v <= 4096).ok_or_else(invalid)?;
    if size.uint()? != 12 || size.uint()? != count || size.uint()? != 4 {
        return Err(invalid());
    }
    let mut block_sizes = Vec::new();
    for _ in 0..count {
        block_sizes.push(size.uint()? as usize);
    }
    if !size.done() || !a.done() {
        return Err(invalid());
    }
    let mut sink = Sink::new(width, height, planes >= 2, color, max_pixels)?;
    let (mut alpha_row, mut gray_row, mut rgb_row) = (vec![0u8; 256], vec![0u8; 256], vec![0u8; 768]);
    let mut input = Cursor::new(blocks);
    let mut checksums = Vec::new();
    for (i, len) in block_sizes.into_iter().enumerate() {
        let mut block = Cursor::new(input.take(len)?);
        if block.uint()? as usize != len {
            return Err(invalid());
        }
        block.name("BlockDataBeginChunk")?;
        // Index, the expanded tile size, then the fixed 256 × 256 tile shape.
        if block.uint()? as usize != i || block.uint()? as usize != tile_len || block.take(8)? != [0, 0, 1, 0, 0, 0, 1, 0] {
            return Err(invalid());
        }
        let present = block.uint()?;
        let tile = match present {
            0 => {
                checksums.push(0);
                vec![0; tile_len]
            }
            1 => {
                let outer = block.uint()? as usize;
                let payload = block.take(outer)?;
                let n = le(payload, 0)? as usize;
                if n.checked_add(4) != Some(payload.len()) {
                    return Err(invalid());
                }
                let mut decoder = flate2::read::ZlibDecoder::new(payload.get(4..).ok_or_else(invalid)?);
                let mut tile = Vec::new();
                decoder.by_ref().take(tile_len as u64 + 1).read_to_end(&mut tile).map_err(|e| e.to_string())?;
                if tile.len() != tile_len || decoder.total_in() != n as u64 {
                    return Err("invalid SUT material tile expansion length".into());
                }
                checksums.push(adler32(payload));
                tile
            }
            _ => return Err(invalid()),
        };
        block.name("BlockDataEndChunk")?;
        if !block.done() {
            return Err(invalid());
        }
        let x = (i % cols as usize) * 256;
        let y = (i / cols as usize) * 256;
        for dy in 0..256usize.min(height as usize - y) {
            let len = 256usize.min(width as usize - x);
            let row = alpha_row.get_mut(..len).ok_or_else(invalid)?;
            if mono {
                let bits = part(&tile, dy * 32, 32)?;
                for (dx, out) in row.iter_mut().enumerate() {
                    let byte = bits.get(dx / 8).ok_or_else(invalid)?;
                    *out = if byte & (0x80 >> (dx % 8)) != 0 { 255 } else { 0 };
                }
            } else {
                row.copy_from_slice(part(&tile, dy * 256, len)?);
            }
            let gray = if planes >= 2 {
                let out = gray_row.get_mut(..len).ok_or_else(invalid)?;
                if color {
                    // Rec. 601 luma of the colour plane, as for colour thumbnails.
                    for (dx, (o, px)) in out.iter_mut().zip(rgb_row.chunks_exact_mut(3)).enumerate() {
                        let [b, g, r, _] = <[u8; 4]>::try_from(part(&tile, 65536 + dy * 1024 + dx * 4, 4)?).map_err(|_| invalid())?;
                        *o = ((299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) + 500) / 1000) as u8;
                        px.copy_from_slice(&[r, g, b]);
                    }
                } else {
                    out.copy_from_slice(part(&tile, 65536 + dy * 256, len)?);
                }
                Some(&*out)
            } else {
                None
            };
            let rgb = if color { Some(rgb_row.get(..len * 3).ok_or_else(invalid)?) } else { None };
            sink.row(x, y + dy, alpha_row.get(..len).ok_or_else(invalid)?, gray, rgb)?;
        }
    }
    array_tail(&mut input, "BlockStatus", count, None)?;
    if !input.done() {
        array_tail(&mut input, "BlockCheckSum", count, Some(&checksums))?;
    }
    if !input.done() {
        return Err(invalid());
    }
    Ok(MaterialRaster { color, ..sink.finish() })
}
fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for x in bytes {
        a = (a + u32::from(*x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
fn array_tail(input: &mut Cursor<'_>, name: &str, count: u32, expected: Option<&[u32]>) -> Result<()> {
    input.name(name)?;
    if input.uint()? != 12 || input.uint()? != count || input.uint()? != 4 {
        return Err(invalid());
    }
    for i in 0..count as usize {
        let value = input.uint()?;
        if let Some(expected) = expected {
            if value != 0 && expected.get(i) != Some(&value) {
                return Err("invalid SUT material block checksum".into());
            }
        } else if value > 1 {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "material_tests.rs"]
pub(super) mod tests;
