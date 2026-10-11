//! Optional PNG cursors with explicit point geometry on AppKit. The ordinary egui bitmap
//! remains the fallback and keeps egui-winit's cursor restoration/cache bookkeeping intact.
#![forbid(unsafe_code)]

use std::sync::Arc;

#[derive(Clone)]
struct Request {
    #[cfg(target_os = "macos")]
    png: Arc<[u8]>,
    bitmap: egui::CustomCursorImage,
    #[cfg(target_os = "macos")]
    size: [f32; 2],
    #[cfg(target_os = "macos")]
    hotspot: [f32; 2],
}

fn key(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::new(("eframe-cursor-png", viewport))
}

/// Request a PNG representation in addition to an ordinary cursor bitmap. `size` and
/// `hotspot` are points, independent of PNG pixel dimensions. No native objects enter egui data.
pub fn request(ctx: &egui::Context, png: Arc<[u8]>, bitmap: egui::CustomCursorImage, size: [f32; 2], hotspot: [f32; 2]) {
    if png.len() > 65536 || !valid_geometry(size, hotspot) {
        return;
    }
    let id = key(ctx.viewport_id());
    ctx.data_mut(|d| {
        d.insert_temp(
            id,
            Request {
                bitmap,
                #[cfg(target_os = "macos")]
                png,
                #[cfg(target_os = "macos")]
                size,
                #[cfg(target_os = "macos")]
                hotspot,
            },
        )
    });
}

/// Clear a viewport's request each pass, so leaving a canvas never leaves a sticky PNG.
pub fn clear(ctx: &egui::Context) {
    let id = key(ctx.viewport_id());
    ctx.data_mut(|d| d.remove::<Request>(id));
}

fn valid_geometry(size: [f32; 2], hotspot: [f32; 2]) -> bool {
    size.into_iter().zip(hotspot).all(|(s, h)| s.is_finite() && (0.0..=256.0).contains(&s) && s > 0.0 && h.is_finite() && h >= 0.0 && h < s)
}

fn matching(ctx: &egui::Context, viewport: egui::ViewportId, output: &egui::PlatformOutput) -> Option<Request> {
    let bitmap = output.cursor_image.as_ref()?;
    let request = ctx.data(|d| d.get_temp::<Request>(key(viewport)))?;
    (Arc::ptr_eq(&bitmap.rgba, &request.bitmap.rgba) && bitmap.size == request.bitmap.size && bitmap.hotspot == request.bitmap.hotspot).then_some(request)
}

/// Backend-only prepared cursor; preparation precedes egui-winit's output, application follows
/// it. An image supplied by another widget cannot accidentally reuse the previous tool's PNG.
pub(crate) struct Prepared {
    #[cfg(target_os = "macos")]
    cursor: Option<objc2::rc::Retained<objc2_app_kit::NSCursor>>,
}

impl Prepared {
    pub(crate) fn new(ctx: &egui::Context, viewport: egui::ViewportId, output: &egui::PlatformOutput) -> Self {
        let request = matching(ctx, viewport, output);
        #[cfg(target_os = "macos")]
        {
            Self { cursor: request.and_then(native::cached) }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = request;
            Self {}
        }
    }

    pub(crate) fn apply(self, window: &winit::window::Window) {
        #[cfg(target_os = "macos")]
        if window.has_focus()
            && let Some(cursor) = self.cursor
        {
            cursor.set();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = window;
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::{AnyThread, rc::Retained};
    use objc2_app_kit::{NSCursor, NSImage};
    use objc2_foundation::{NSData, NSPoint, NSSize};
    use std::cell::RefCell;

    // Native objects stay on the event-loop thread; retaining the bytes also prevents pointer
    // reuse from making cache keys alias a newly loaded pack. Bound the cache independently.
    thread_local! { static CACHE: RefCell<Vec<(Request, Retained<NSCursor>)>> = const { RefCell::new(Vec::new()) }; }

    pub(super) fn cached(request: Request) -> Option<Retained<NSCursor>> {
        // Bound native decoder allocations too, even if a caller bypasses Prism's pack loader.
        if request.png.get(..8)? != b"\x89PNG\r\n\x1a\n" || request.png.get(12..16)? != b"IHDR" {
            return None;
        }
        let width = u32::from_be_bytes(request.png.get(16..20)?.try_into().ok()?);
        let height = u32::from_be_bytes(request.png.get(20..24)?.try_into().ok()?);
        if width == 0 || height == 0 || width > 256 || height > 256 {
            return None;
        }
        CACHE.with(|cell| {
            let mut cache = cell.try_borrow_mut().ok()?;
            if let Some((_, cursor)) = cache.iter().find(|(r, _)| Arc::ptr_eq(&r.png, &request.png) && r.size == request.size && r.hotspot == request.hotspot) {
                return Some(cursor.clone());
            }
            let data = NSData::with_bytes(&request.png);
            let image = NSImage::initWithData(NSImage::alloc(), &data)?;
            image.setSize(NSSize::new(f64::from(request.size[0]), f64::from(request.size[1])));
            let cursor = NSCursor::initWithImage_hotSpot(NSCursor::alloc(), &image, NSPoint::new(f64::from(request.hotspot[0]), f64::from(request.hotspot[1])));
            if cache.len() >= 128 {
                cache.remove(0);
            }
            cache.push((request, cursor.clone()));
            Some(cursor)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn points_are_bounded_and_hotspots_inside() {
        assert!(valid_geometry([32.0; 2], [1.0; 2]));
        for (size, hot) in [([0.0, 32.0], [1.0; 2]), ([f32::NAN, 32.0], [1.0; 2]), ([32.0; 2], [32.0, 1.0]), ([32.0; 2], [-1.0, 1.0])] {
            assert!(!valid_geometry(size, hot));
        }
    }
    #[test]
    fn requests_are_viewport_scoped_and_match_the_output_bitmap() {
        let ctx = egui::Context::default();
        let image = egui::CustomCursorImage { rgba: Arc::from(vec![255; 16]), size: [2, 2], hotspot: [0, 0] };
        request(&ctx, Arc::from(vec![0; 8]), image.clone(), [2.0; 2], [0.0; 2]);
        let mut output = egui::PlatformOutput::default();
        output.cursor_image = Some(image);
        assert!(matching(&ctx, egui::ViewportId::ROOT, &output).is_some());
        assert!(matching(&ctx, egui::ViewportId::from_hash_of("other"), &output).is_none());
        output.cursor_image.as_mut().unwrap().rgba = Arc::from(vec![255; 16]);
        assert!(matching(&ctx, egui::ViewportId::ROOT, &output).is_none());
        clear(&ctx);
        assert!(ctx.data(|d| d.get_temp::<Request>(key(egui::ViewportId::ROOT))).is_none());
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn retina_representation_keeps_point_size_and_hotspot_and_reuses_native_object() {
        // Synthetic white RGBA PNG; no external artwork or app installation is needed.
        let png: Arc<[u8]> = Arc::from(vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 64, 0, 0, 0, 64, 8, 6, 0, 0, 0, 170, 105, 113, 222, 0, 0, 0, 146, 73, 68,
            65, 84, 120, 156, 237, 208, 65, 17, 0, 0, 8, 128, 48, 251, 151, 214, 24, 123, 200, 18, 112, 204, 62, 55, 58, 64, 107, 128, 14, 208, 26, 160, 3,
            180, 6, 232, 0, 173, 1, 58, 64, 107, 128, 14, 208, 26, 160, 3, 180, 6, 232, 0, 173, 1, 58, 64, 107, 128, 14, 208, 26, 160, 3, 180, 6, 232, 0, 173,
            1, 58, 64, 107, 128, 14, 208, 26, 160, 3, 180, 6, 232, 0, 173, 1, 58, 64, 107, 128, 14, 208, 26, 160, 3, 180, 6, 232, 0, 173, 1, 58, 64, 107, 128,
            14, 208, 26, 160, 3, 180, 6, 232, 0, 173, 1, 58, 64, 107, 128, 14, 208, 26, 160, 3, 180, 6, 232, 0, 173, 1, 58, 64, 107, 128, 14, 208, 26, 160, 3,
            180, 6, 232, 0, 237, 0, 70, 208, 195, 178, 165, 222, 223, 188, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ]);
        let bitmap = egui::CustomCursorImage { rgba: Arc::from(vec![255; 32 * 32 * 4]), size: [32; 2], hotspot: [1; 2] };
        let request = Request { png, bitmap, size: [32.0; 2], hotspot: [1.0; 2] };
        let cursor = native::cached(request.clone()).unwrap();
        let again = native::cached(request).unwrap();
        assert_eq!(cursor.image().size(), objc2_foundation::NSSize::new(32.0, 32.0));
        assert_eq!(cursor.hotSpot(), objc2_foundation::NSPoint::new(1.0, 1.0));
        assert!(std::ptr::eq(&*cursor, &*again));
    }
}
