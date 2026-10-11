//! Optional local cursor artwork. Documents, preferences and distributions do not embed the
//! pack. A missing pack retains Prism's original cursors; a malformed pack is rejected atomically.
use crate::{state::Tool, tool_feedback::Badge};
use egui::{CursorIcon, CustomCursorImage};
use serde::Deserialize;
use std::{collections::BTreeMap, io::Read, path::Path, sync::Arc};

const MAX_FILE: u64 = 65536;
const MAX_ENTRIES: usize = 512;
const PACK_KEY: &str = "prism-local-cursor-pack";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    cursors: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: u32,
    images: Vec<Representation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Representation {
    file: String,
    scale: u16,
    size: [u16; 2],
    hotspot: [u16; 2],
}
#[derive(Clone)]
struct Image {
    scale: u16,
    png: Arc<[u8]>,
    bitmap: CustomCursorImage,
}
struct Pack {
    images: BTreeMap<u32, Vec<Image>>,
    painter: bool,
}
#[derive(Clone)]
struct Cached {
    asset: u32,
    scale: f32,
    bitmap: CustomCursorImage,
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds the cursor pack size limit", path.display()));
    }
    Ok(bytes)
}

fn decode(png: &[u8], repr: &Representation) -> Result<CustomCursorImage, String> {
    use prism_codecs::{ChannelLayout, DecodeOptions, Format, Limits, SampleType};
    if !matches!(repr.scale, 1 | 2) || repr.size.into_iter().zip(repr.hotspot).any(|(s, h)| s == 0 || s > 256 || h >= s) {
        return Err("invalid cursor size, scale or hotspot".into());
    }
    let opts = DecodeOptions { limits: Limits { max_width: 256, max_height: 256, max_pixels: 65536, max_alloc: 262144 }, keep_orientation: true };
    let decoded = prism_codecs::decode_as_with(Format::Png, png, &opts).map_err(|e| e.to_string())?;
    if [decoded.width(), decoded.height()] != repr.size.map(u32::from) || decoded.sample_type() != SampleType::U8 {
        return Err("cursor PNG dimensions/depth disagree with its manifest".into());
    }
    let mut rgba = Vec::with_capacity(usize::from(repr.size[0]) * usize::from(repr.size[1]) * 4);
    // Cursor packs deliberately contain display RGBA8, independent of document depth/profile.
    for pixel in decoded.data().chunks_exact(decoded.layout().channels()) {
        match (decoded.layout(), pixel) {
            (ChannelLayout::Rgba, [r, g, b, a]) => rgba.extend_from_slice(&[*r, *g, *b, *a]),
            (ChannelLayout::Rgb, [r, g, b]) => rgba.extend_from_slice(&[*r, *g, *b, 255]),
            (ChannelLayout::GrayA, [g, a]) => rgba.extend_from_slice(&[*g, *g, *g, *a]),
            (ChannelLayout::Gray, [g]) => rgba.extend_from_slice(&[*g, *g, *g, 255]),
            _ => return Err("cursor PNG must contain grayscale or RGB display pixels".into()),
        }
    }
    Ok(CustomCursorImage { rgba: Arc::from(rgba), size: repr.size, hotspot: repr.hotspot })
}

fn load(path: &Path, painter: bool) -> Result<Pack, String> {
    let manifest: Manifest = serde_json::from_slice(&read_bounded(&path.join("manifest.json"), 262144)?).map_err(|e| e.to_string())?;
    if manifest.version != 1 || manifest.cursors.is_empty() || manifest.cursors.len() > MAX_ENTRIES {
        return Err("unsupported or oversized cursor pack manifest".into());
    }
    let mut images = BTreeMap::new();
    let mut total = 0usize;
    for entry in manifest.cursors {
        if entry.images.is_empty() || entry.images.len() > 2 || images.contains_key(&entry.id) {
            return Err("empty or duplicate cursor entry".into());
        }
        let mut variants: Vec<Image> = Vec::new();
        for repr in entry.images {
            if repr.file != format!("Alpha_{}-{}x.png", entry.id, repr.scale) || variants.iter().any(|v| v.scale == repr.scale) {
                return Err("invalid cursor filename or duplicate scale".into());
            }
            let png: Arc<[u8]> = read_bounded(&path.join(&repr.file), MAX_FILE)?.into();
            let bitmap = decode(&png, &repr).map_err(|e| format!("{}: {e}", repr.file))?;
            total = total.checked_add(bitmap.rgba.len() + png.len()).ok_or("cursor pack allocation overflow")?;
            if total > 32 * 1024 * 1024 {
                return Err("cursor pack exceeds its decoded memory budget".into());
            }
            variants.push(Image { scale: repr.scale, png, bitmap });
        }
        variants.sort_by_key(|v| v.scale);
        // A point-sized fallback is mandatory on AppKit, whose winit bitmap path has no DPI metadata.
        if variants.first().is_none_or(|v| v.scale != 1) {
            return Err(format!("cursor {} has no 1x fallback", entry.id));
        }
        images.insert(entry.id, variants);
    }
    Ok(Pack { images, painter })
}

/// Load an optional per-user pack once. Returns its number of resource IDs; no partial install
/// occurs on failure. `painter` is for non-native snapshots; the native app uses OS cursors.
pub fn install(ctx: &egui::Context, path: &Path, painter: bool) -> Result<usize, String> {
    let pack = load(path, painter)?;
    let count = pack.images.len();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(PACK_KEY), Arc::new(pack)));
    Ok(count)
}

/// Number of validated local resource IDs loaded for this process.
pub fn resource_count(ctx: &egui::Context) -> usize {
    ctx.data(|d| d.get_temp::<Arc<Pack>>(egui::Id::new(PACK_KEY))).map_or(0, |p| p.images.len())
}

/// Resource currently requested in this viewport, for automation and regression checks.
pub fn active(ctx: &egui::Context) -> Option<u32> {
    let id = egui::Id::new(("prism-active-cursor", ctx.viewport_id()));
    ctx.data(|d| d.get_temp::<u32>(id))
}
/// Last completed pass, available while automation runs before the current canvas is drawn.
pub fn last_drawn(ctx: &egui::Context) -> Option<u32> {
    let id = egui::Id::new(("prism-rendered-cursor", ctx.viewport_id()));
    ctx.data(|d| d.get_temp::<Option<u32>>(id)).flatten()
}
pub(crate) fn finish_pass(ctx: &egui::Context) {
    let value = active(ctx);
    let id = egui::Id::new(("prism-rendered-cursor", ctx.viewport_id()));
    ctx.data_mut(|d| d.insert_temp(id, value));
}
pub(crate) fn clear(ctx: &egui::Context) {
    let id = egui::Id::new(("prism-active-cursor", ctx.viewport_id()));
    ctx.data_mut(|d| d.remove::<u32>(id));
    eframe::cursor_png::clear(ctx);
    ctx.graphics_mut(|graphics| {
        if let Some(list) = graphics.get_mut(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("local cursor preview"))) {
            *list = Default::default();
        }
    });
}

pub(crate) fn show(ctx: &egui::Context, at: egui::Pos2, asset: u32, fallback: CursorIcon) -> Option<CursorIcon> {
    let pack = ctx.data(|d| d.get_temp::<Arc<Pack>>(egui::Id::new(PACK_KEY)))?;
    let variants = pack.images.get(&asset)?;
    let low = variants.first()?;
    let ppp = ctx.pixels_per_point();
    if !ppp.is_finite() || !(0.5..=4.0).contains(&ppp) {
        return None;
    }
    let best = if ppp > 1.0 { variants.last()? } else { low };
    let points = best.bitmap.size.map(|s| f32::from(s) / f32::from(best.scale));
    let hotspot = best.bitmap.hotspot.map(|h| f32::from(h) / f32::from(best.scale));
    let id = egui::Id::new(("prism-active-cursor", ctx.viewport_id()));
    ctx.data_mut(|d| d.insert_temp(id, asset));
    if pack.painter {
        let key = egui::Id::new(("prism-cursor-texture", asset, best.scale));
        let texture = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)).unwrap_or_else(|| {
            let texture = ctx.load_texture(
                "local cursor",
                egui::ColorImage::from_rgba_unmultiplied(best.bitmap.size.map(usize::from), &best.bitmap.rgba),
                egui::TextureOptions::LINEAR,
            );
            ctx.data_mut(|d| d.insert_temp(key, texture.clone()));
            texture
        });
        ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("local cursor preview"))).image(
            texture.id(),
            egui::Rect::from_min_size(at - egui::vec2(hotspot[0], hotspot[1]), egui::vec2(points[0], points[1])),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        return Some(CursorIcon::None);
    }
    let bitmap = if cfg!(target_os = "macos") { low.bitmap.clone() } else { scaled(ctx, asset, best, ppp)? };
    ctx.set_cursor_image(Some(bitmap.clone()));
    let fallback_key = crate::tool_cursor::fallback_key(ctx);
    ctx.data_mut(|d| d.insert_temp(fallback_key, fallback));
    eframe::cursor_png::request(ctx, best.png.clone(), bitmap, points, hotspot);
    Some(CursorIcon::None)
}

fn scaled(ctx: &egui::Context, asset: u32, image: &Image, scale: f32) -> Option<CustomCursorImage> {
    let key = egui::Id::new(("prism-scaled-cursor", ctx.viewport_id()));
    let mut cache = ctx.data(|d| d.get_temp::<Vec<Cached>>(key)).unwrap_or_default();
    if let Some(c) = cache.iter().find(|c| c.asset == asset && c.scale == scale) {
        return Some(c.bitmap.clone());
    }
    let bitmap = rescale(image, scale)?;
    if cache.len() >= 128 {
        cache.remove(0);
    }
    cache.push(Cached { asset, scale, bitmap: bitmap.clone() });
    ctx.data_mut(|d| d.insert_temp(key, cache));
    Some(bitmap)
}

fn rescale(image: &Image, scale: f32) -> Option<CustomCursorImage> {
    if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
        return None;
    }
    if scale == f32::from(image.scale) {
        return Some(image.bitmap.clone());
    }
    let ratio = scale / f32::from(image.scale);
    let size = image.bitmap.size.map(|n| (f32::from(n) * ratio).round().max(1.0) as u16);
    if size.into_iter().any(|s| s > 1024) {
        return None;
    }
    let hotspot = std::array::from_fn(|i| (f32::from(image.bitmap.hotspot[i]) * ratio).round().min(f32::from(size[i] - 1)) as u16);
    let mut rgba = Vec::with_capacity(usize::from(size[0]) * usize::from(size[1]) * 4);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let sx = ((f32::from(x) + 0.5) / ratio - 0.5).clamp(0.0, f32::from(image.bitmap.size[0] - 1));
            let sy = ((f32::from(y) + 0.5) / ratio - 0.5).clamp(0.0, f32::from(image.bitmap.size[1] - 1));
            let (x0, y0) = (sx.floor() as u16, sy.floor() as u16);
            let (dx, dy) = (sx.fract(), sy.fract());
            let mut premul = [0.0f32; 4];
            for (px, py, weight) in [
                (x0, y0, (1.0 - dx) * (1.0 - dy)),
                ((x0 + 1).min(image.bitmap.size[0] - 1), y0, dx * (1.0 - dy)),
                (x0, (y0 + 1).min(image.bitmap.size[1] - 1), (1.0 - dx) * dy),
                ((x0 + 1).min(image.bitmap.size[0] - 1), (y0 + 1).min(image.bitmap.size[1] - 1), dx * dy),
            ] {
                let offset = (usize::from(py) * usize::from(image.bitmap.size[0]) + usize::from(px)) * 4;
                let pixel = image.bitmap.rgba.get(offset..offset + 4)?;
                let a = f32::from(*pixel.get(3)?) / 255.0;
                for (c, value) in premul.iter_mut().take(3).zip(pixel) {
                    *c += f32::from(*value) * a * weight;
                }
                premul[3] += a * weight;
            }
            let a = premul[3];
            rgba.extend(premul.into_iter().take(3).map(|c| if a > 0.0 { (c / a).round() as u8 } else { 0 }));
            rgba.push((a * 255.0).round() as u8);
        }
    }
    Some(CustomCursorImage { rgba: rgba.into(), size, hotspot })
}

/// Only roles Prism can perform are mapped. Cursor resources for 3D/unsupported modes remain
/// in the personal pack without inventing tool behavior. Modifier variants follow engine intent.
pub(crate) fn tool_asset(tool: Tool, badge: Option<Badge>, alt: bool, dragging: bool) -> u32 {
    let tool = crate::retouch_ui::alt_flipped(tool, alt);
    let variant = |base: u32| {
        base + match badge {
            None => 0,
            Some(Badge::Add) => 1,
            Some(Badge::Subtract) => 2,
            Some(Badge::Intersect) => 3,
        }
    };
    match tool {
        Tool::Move => {
            if dragging {
                1004
            } else {
                1059
            }
        }
        Tool::RectMarquee | Tool::EllipseMarquee => variant(1450),
        Tool::Lasso => variant(1458),
        Tool::PolygonLasso => variant(1480),
        Tool::MagneticLasso => variant(1490),
        Tool::MagicWand => variant(1454),
        Tool::QuickSelection => match badge {
            Some(Badge::Subtract) => 1615,
            Some(Badge::Add) => 1614,
            _ => 1613,
        },
        Tool::ObjectSelection => variant(1800),
        Tool::Crop => 1027,
        Tool::Eyedropper => 1017,
        Tool::Ruler => 1510,
        Tool::Note => 1540,
        Tool::Count => {
            if alt {
                1612
            } else {
                1610
            }
        }
        Tool::Brush => 1012,
        Tool::Pencil => 1011,
        Tool::MixerBrush => 1601,
        Tool::Eraser => 1048,
        Tool::BackgroundEraser => 1049,
        Tool::MagicEraser => 1010,
        Tool::Gradient => 1009,
        Tool::PaintBucket => 1018,
        Tool::Type => 1462,
        Tool::VerticalType => 1474,
        Tool::Hand => {
            if dragging {
                25006
            } else {
                1005
            }
        }
        Tool::RotateView => {
            if dragging {
                25006
            } else {
                1630
            }
        }
        Tool::Zoom => {
            if alt {
                1007
            } else {
                1006
            }
        }
        Tool::SpotHealing => 1565,
        Tool::Healing => {
            if alt {
                1022
            } else {
                1571
            }
        }
        Tool::Patch => 1572,
        Tool::ContentAwareMove => 1578,
        Tool::RedEye => 1562,
        Tool::CloneStamp => {
            if alt {
                1022
            } else {
                1020
            }
        }
        Tool::PatternStamp => 1021,
        Tool::HistoryBrush => 1023,
        Tool::Blur => 1014,
        Tool::Sharpen => 1015,
        Tool::Smudge => 1016,
        Tool::Dodge => 1022,
        Tool::Burn => 1202,
        Tool::Sponge => 1037,
        Tool::Pen => 1063,
        Tool::PathSelection => 1057,
        Tool::DirectSelection => 1500,
        Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Line | Tool::Arrow | Tool::CustomShape => 1450,
        Tool::Slice => 1550,
        Tool::SliceSelect => 1551,
    }
}

pub(crate) fn stock_asset(icon: CursorIcon) -> Option<u32> {
    Some(match icon {
        CursorIcon::Default => 1004,
        CursorIcon::Crosshair => 1450,
        CursorIcon::Move => 1045,
        CursorIcon::ResizeHorizontal => 1402,
        CursorIcon::ResizeVertical => 1401,
        CursorIcon::ResizeNwSe => 1403,
        CursorIcon::ResizeNeSw => 1404,
        CursorIcon::Alias => 1411,
        CursorIcon::NotAllowed => 1042,
        CursorIcon::Grab => 1005,
        CursorIcon::Grabbing => 25006,
        CursorIcon::ZoomIn => 1006,
        CursorIcon::ZoomOut => 1007,
        CursorIcon::Text => 1462,
        CursorIcon::VerticalText => 1474,
        _ => return None,
    })
}

pub(crate) fn stock(ctx: &egui::Context, at: egui::Pos2, icon: CursorIcon) -> CursorIcon {
    stock_asset(icon).and_then(|asset| show(ctx, at, asset, icon)).unwrap_or(icon)
}

#[cfg(test)]
mod tests;
