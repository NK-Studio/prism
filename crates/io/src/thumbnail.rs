//! Small previews of saved documents, for the Home screen's recent files: the thumbnail a
//! `.prism` bundle or a PSD/PSB file embeds, else (a PSD/PSB without one, as Prism writes them)
//! a reduced copy of its merged image. Reading a PSD's thumbnail touches only the start of the
//! file (header, colour mode data and image resources), however large the file is.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use prism_codecs::Format;
use prism_psd::{ColorMode, PsdFile};

use crate::IoError;

/// An 8-bit RGBA preview, at most the requested size on its longer side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    /// Interleaved, unpremultiplied RGBA, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Image resources larger than this are not read for a thumbnail.
const MAX_RESOURCES: u64 = 64 << 20;
/// Files larger than this are not read whole (a `.prism` bundle's zip directory, or a PSD
/// without a thumbnail whose merged image would have to be decoded).
const MAX_WHOLE_FILE: u64 = 512 << 20;

fn io_err(path: &Path, e: std::io::Error) -> IoError {
    IoError::Unsupported(format!("{}: {e}", path.display()))
}

/// The preview of the document at `path`, scaled to at most `max_side` pixels on its longer side:
/// `Ok(None)` when the file has none Prism can show (another format, a `.prism` folder bundle, a
/// PSD in a colour mode the reduced merged image doesn't cover, a file over the size limits).
pub fn read(path: &Path, max_side: u32) -> Result<Option<Thumbnail>, IoError> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    match ext.as_str() {
        "psd" | "psb" => psd(path, max_side),
        "prism" => prism(path, max_side),
        _ => Ok(None),
    }
}

fn prism(path: &Path, max_side: u32) -> Result<Option<Thumbnail>, IoError> {
    if path.is_dir() {
        return Ok(None);
    }
    let len = std::fs::metadata(path).map_err(|e| io_err(path, e))?.len();
    if len > MAX_WHOLE_FILE {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
    let Some(png) = prism_format::read_thumbnail(&bytes)? else { return Ok(None) };
    let img = prism_codecs::decode_as(Format::Png, &png)?;
    Ok(Some(scaled(img.width(), img.height(), &img.to_rgba8(), max_side)))
}

fn psd(path: &Path, max_side: u32) -> Result<Option<Thumbnail>, IoError> {
    let mut f = File::open(path).map_err(|e| io_err(path, e))?;
    if let Some(t) = psd_thumbnail_resource(&mut f, path, max_side)? {
        return Ok(Some(t));
    }
    let len = f.metadata().map_err(|e| io_err(path, e))?.len();
    if len > MAX_WHOLE_FILE {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
    merged(&PsdFile::from_bytes(&bytes)?, max_side)
}

fn be_u32(b: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(0..4)?.try_into().ok()?))
}

/// The JPEG thumbnail resource (1036, or the Photoshop 4 one, 1033), read from the start of the
/// file only.
fn psd_thumbnail_resource(f: &mut File, path: &Path, max_side: u32) -> Result<Option<Thumbnail>, IoError> {
    let mut head = [0u8; 30];
    f.read_exact(&mut head).map_err(|e| io_err(path, e))?;
    if head.get(0..4) != Some(b"8BPS".as_slice()) {
        return Err(IoError::UnknownFormat(format!("{} is not a PSD file", path.display())));
    }
    // 26-byte header, then the colour mode data's length.
    let mode_len = be_u32(head.get(26..).unwrap_or_default()).unwrap_or(0);
    f.seek(SeekFrom::Current(i64::from(mode_len))).map_err(|e| io_err(path, e))?;
    let mut len = [0u8; 4];
    f.read_exact(&mut len).map_err(|e| io_err(path, e))?;
    let res_len = u64::from(u32::from_be_bytes(len));
    if res_len > MAX_RESOURCES {
        return Ok(None);
    }
    let mut body = Vec::new();
    f.take(res_len).read_to_end(&mut body).map_err(|e| io_err(path, e))?;
    if (body.len() as u64) < res_len {
        return Err(IoError::Unsupported(format!("{}: the image resources are cut short", path.display())));
    }
    let resources = prism_psd::resources::parse_section(&body)?;
    // Prefer the current resource over the Photoshop 4 one.
    let mut found: Vec<(&[u8], bool)> = resources.iter().filter_map(|r| r.thumbnail_jfif()).collect();
    found.sort_by_key(|t| t.1);
    let Some((jfif, bgr)) = found.first().copied() else { return Ok(None) };
    let img = prism_codecs::decode_as(Format::Jpeg, jfif)?;
    let mut rgba = img.to_rgba8();
    if bgr {
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    Ok(Some(scaled(img.width(), img.height(), &rgba, max_side)))
}

/// The merged image (planar, big-endian samples) reduced to `max_side`, for RGB and Grayscale at
/// 8, 16 or 32 bits; `None` for other modes.
fn merged(file: &PsdFile, max_side: u32) -> Result<Option<Thumbnail>, IoError> {
    let h = &file.header;
    let color = match h.color_mode {
        ColorMode::Rgb => 3usize,
        ColorMode::Grayscale => 1,
        _ => return Ok(None),
    };
    let bytes = match h.depth {
        8 => 1usize,
        16 => 2,
        32 => 4,
        _ => return Ok(None),
    };
    let channels = usize::from(h.channels);
    if channels < color || h.width == 0 || h.height == 0 {
        return Ok(None);
    }
    let alpha = file.merged_has_alpha();
    let planes = file.decode_merged()?;
    let (w, h_px) = (h.width as usize, h.height as usize);
    let plane = h.row_bytes().checked_mul(h_px).ok_or_else(|| IoError::Unsupported("merged image too large".into()))?;
    let (tw, th) = fit(h.width, h.height, max_side);
    let sample = |c: usize, x: usize, y: usize| -> u8 {
        let at = c.saturating_mul(plane).saturating_add(y.saturating_mul(h.row_bytes())).saturating_add(x.saturating_mul(bytes));
        let Some(s) = planes.get(at..at.saturating_add(bytes)) else { return 0 };
        match bytes {
            1 => s.first().copied().unwrap_or(0),
            2 => s.first().copied().unwrap_or(0),
            _ => {
                // 32-bit documents are linear light: back to display gamma.
                let v = s.try_into().map(f32::from_be_bytes).unwrap_or(0.0);
                let v = if v.is_finite() { v.clamp(0.0, 1.0).powf(1.0 / 2.2) } else { 0.0 };
                (v * 255.0).round() as u8
            }
        }
    };
    let mut rgba = Vec::with_capacity(tw as usize * th as usize * 4);
    for ty in 0..th as usize {
        let y = (ty * h_px / th as usize).min(h_px - 1);
        for tx in 0..tw as usize {
            let x = (tx * w / tw as usize).min(w - 1);
            let (r, g, b) = if color == 3 { (sample(0, x, y), sample(1, x, y), sample(2, x, y)) } else { (sample(0, x, y), sample(0, x, y), sample(0, x, y)) };
            let a = if alpha && channels > color { sample(color, x, y) } else { 255 };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    Ok(Some(Thumbnail { width: tw, height: th, rgba }))
}

/// `w` x `h` fitted into `max_side` (never enlarged, never zero).
fn fit(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    let m = max_side.max(1);
    let (w, h) = (w.max(1), h.max(1));
    if w <= m && h <= m {
        return (w, h);
    }
    if w >= h { (m, ((u64::from(h) * u64::from(m)) / u64::from(w)).max(1) as u32) } else { (((u64::from(w) * u64::from(m)) / u64::from(h)).max(1) as u32, m) }
}

/// Box-filter `rgba` (`w` x `h`) down to fit `max_side`.
fn scaled(w: u32, h: u32, rgba: &[u8], max_side: u32) -> Thumbnail {
    let (tw, th) = fit(w, h, max_side);
    if (tw, th) == (w, h) && rgba.len() == w as usize * h as usize * 4 {
        return Thumbnail { width: w, height: h, rgba: rgba.to_vec() };
    }
    let (w, h, tw_u, th_u) = (w.max(1) as usize, h.max(1) as usize, tw as usize, th as usize);
    let mut out = Vec::with_capacity(tw_u * th_u * 4);
    for ty in 0..th_u {
        let (y0, y1) = (ty * h / th_u, ((ty + 1) * h / th_u).max(ty * h / th_u + 1).min(h));
        for tx in 0..tw_u {
            let (x0, x1) = (tx * w / tw_u, ((tx + 1) * w / tw_u).max(tx * w / tw_u + 1).min(w));
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * w + x) * 4;
                    if let Some(p) = rgba.get(at..at + 4) {
                        for (s, v) in sum.iter_mut().zip(p) {
                            *s += u64::from(*v);
                        }
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            out.extend(sum.iter().map(|s| (s / n) as u8));
        }
    }
    Thumbnail { width: tw, height: th, rgba: out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_psd::{ImageResource, PsdBuilder};

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("prism-thumb-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn jpeg(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let data: Vec<u8> = (0..w * h).flat_map(|_| rgb).collect();
        let img = prism_codecs::Image::from_raw(w, h, prism_codecs::ChannelLayout::Rgb, prism_codecs::SampleType::U8, data).unwrap();
        prism_codecs::encode(&img, Format::Jpeg, &prism_codecs::EncodeOptions::default()).unwrap()
    }

    fn psd_with(resources: Vec<ImageResource>, w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut b = PsdBuilder::new(w, h);
        b.composite(prism_psd::PixelData::Rgba8([rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize)));
        let mut f = b.build().unwrap();
        f.resources.extend(resources);
        f.to_bytes().unwrap()
    }

    #[test]
    fn psd_thumbnail_resource_is_read_and_scaled() {
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(&[0; 24]);
        data.extend(jpeg(64, 32, [250, 10, 10]));
        let p = temp("thumb.psd", &psd_with(vec![ImageResource::new(1036, data)], 8, 8, [0, 0, 255]));
        let t = read(&p, 16).unwrap().unwrap();
        assert_eq!((t.width, t.height), (16, 8));
        let px = &t.rgba[0..4];
        assert!(px[0] > 200 && px[2] < 60, "the embedded (red) thumbnail, not the blue merged image: {px:?}");
    }

    #[test]
    fn psd_without_a_thumbnail_reduces_the_merged_image() {
        let p = temp("merged.psd", &psd_with(Vec::new(), 40, 20, [0, 200, 0]));
        let t = read(&p, 10).unwrap().unwrap();
        assert_eq!((t.width, t.height), (10, 5));
        assert_eq!(&t.rgba[0..4], &[0, 200, 0, 255]);
    }

    #[test]
    fn other_files_and_damaged_psds_never_panic() {
        assert_eq!(read(&temp("a.png", b"png"), 64).unwrap(), None);
        assert!(read(&temp("bad.psd", b"8BPS"), 64).is_err());
        assert!(read(&temp("garbage.psd", &[7; 200]), 64).is_err());
        let mut cut = psd_with(Vec::new(), 4, 4, [1, 2, 3]);
        cut.truncate(40);
        assert!(read(&temp("cut.psd", &cut), 64).is_err());
        assert!(read(&temp("bad.prism", b"not a zip"), 64).is_err());
        assert!(read(Path::new("/definitely/missing.psd"), 64).is_err());
    }

    #[test]
    fn fit_keeps_the_aspect_and_never_reaches_zero() {
        assert_eq!(fit(4000, 1000, 200), (200, 50));
        assert_eq!(fit(10, 4000, 200), (1, 200));
        assert_eq!(fit(20, 10, 200), (20, 10));
        assert_eq!(fit(0, 0, 0), (1, 1));
    }
}
