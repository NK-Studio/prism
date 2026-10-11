//! Desktop colour capture starts only on an explicit UI request. No snapshots are written to
//! disk, transmitted or retained after a pick.
use prism_ui_egui::screen_picker::{Capture, MAX_PIXELS, Pending, ScreenImage, Service};

pub fn service(cc: &eframe::CreationContext<'_>) -> Option<Service> {
    if !cfg!(any(target_os = "macos", target_os = "windows")) {
        return None;
    }
    let window = cc.winit_window()?.clone();
    Some(Box::new(move |ctx| {
        let (tx, rx) = std::sync::mpsc::channel();
        let fallback = tx.clone();
        let placements: Vec<Placement> = window
            .available_monitors()
            .enumerate()
            .map(|(index, m)| {
                let p = m.position();
                let size = m.size();
                Placement { index, x: p.x, y: p.y, width: size.width, height: size.height }
            })
            .collect();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = cancelled.clone();
        let scale = ctx.native_pixels_per_point().unwrap_or(1.0);
        let wake = ctx.clone();
        if let Err(e) = std::thread::Builder::new().name("screen-color-picker".into()).spawn(move || {
            let result = capture(scale, &stop, &placements);
            let _ = tx.send(result);
            wake.request_repaint();
        }) {
            let _ = fallback.send(Err(format!("Could not start screen color picker: {e}")));
        }
        Pending { receiver: rx, cancelled }
    }))
}
#[derive(Clone, Copy)]
struct Placement {
    index: usize,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}
fn find_monitor(placements: &[Placement], x: i32, y: i32, w: u32, h: u32) -> Option<usize> {
    placements.iter().find(|p| p.x == x && p.y == y && p.width == w && p.height == h).map(|p| p.index)
}
fn count(width: usize, height: usize) -> Result<usize, String> {
    width.checked_mul(height).filter(|&n| n > 0 && n <= MAX_PIXELS).ok_or_else(|| "Screen capture exceeds the 64 MP memory budget".into())
}
#[cfg(target_os = "windows")]
fn capture(_scale: f32, stop: &std::sync::atomic::AtomicBool, placements: &[Placement]) -> Result<Capture, String> {
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    if monitors.is_empty() || monitors.len() > 16 {
        return Err("No supported screen is available".into());
    }
    let mut budget = 0usize;
    for m in &monitors {
        budget = budget
            .checked_add(count(m.width().map_err(|e| e.to_string())? as usize, m.height().map_err(|e| e.to_string())? as usize)?)
            .filter(|&n| n <= MAX_PIXELS)
            .ok_or("Combined displays exceed the screen picker memory budget")?;
    }
    let mut total = 0usize;
    let mut images = Vec::new();
    for m in monitors {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(Capture::Color(None));
        }
        let (w, h) = (m.width().map_err(|e| e.to_string())?, m.height().map_err(|e| e.to_string())?);
        let scale = m.scale_factor().map_err(|e| e.to_string())?;
        if !scale.is_finite() || scale <= 0.0 || scale > 16.0 {
            return Err("Invalid display scale".into());
        }
        let captured = m.capture_image().map_err(|e| format!("Screen capture failed: {e}"))?;
        let (width, height) = (captured.width() as usize, captured.height() as usize);
        total = total.checked_add(count(width, height)?).filter(|&n| n <= MAX_PIXELS).ok_or("Combined displays exceed the screen picker memory budget")?;
        let (x, y) = (m.x().map_err(|e| e.to_string())?, m.y().map_err(|e| e.to_string())?);
        images.push(ScreenImage {
            monitor: find_monitor(placements, x, y, w, h),
            position: egui::pos2(x as f32, y as f32) / scale,
            size: egui::vec2(w as f32, h as f32) / scale,
            width,
            height,
            rgba: captured.into_raw(),
            profile: None,
        });
    }
    Ok(Capture::Images(images))
}
#[cfg(target_os = "macos")]
fn capture(_scale: f32, stop: &std::sync::atomic::AtomicBool, placements: &[Placement]) -> Result<Capture, String> {
    use core_graphics2::{display::get_active_display_list, image::CGImageAlphaInfo as Alpha, window};
    if !window::preflight_screen_capture_access() && !window::request_screen_capture_access() {
        return Err("Allow Prism in System Settings > Privacy & Security > Screen Recording, then reopen Prism".into());
    }
    let displays = get_active_display_list(17).ok_or("Could not enumerate displays")?;
    if displays.is_empty() || displays.len() > 16 {
        return Err("No supported screen is available".into());
    }
    let mut budget = 0usize;
    for display in &displays {
        let mode = display.copy_display_mode().ok_or("Display mode is unavailable")?;
        budget = budget
            .checked_add(count(mode.pixel_width(), mode.pixel_height())?)
            .filter(|&n| n <= MAX_PIXELS)
            .ok_or("Combined displays exceed the screen picker memory budget")?;
    }
    let mut images = Vec::new();
    let mut total = 0usize;
    for display in displays {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(Capture::Color(None));
        }
        // Capture a composed monitor at native resolution through safe bindings. Keeping the
        // CoreGraphics seam preserves Prism's existing macOS 11 deployment target.
        let bounds = display.bounds();
        let image = window::new_image(
            bounds,
            window::CGWindowListOption::OnScreenOnly,
            window::kCGNullWindowID,
            window::CGWindowImageOption::BestResolution | window::CGWindowImageOption::ShouldBeOpaque,
        )
        .ok_or("Screen capture failed; check Screen Recording permission")?;
        let (width, height) = (image.width(), image.height());
        total = total.checked_add(count(width, height)?).filter(|&n| n <= MAX_PIXELS).ok_or("Combined displays exceed the screen picker memory budget")?;
        let profile = image.color_space().and_then(|space| space.copy_icc_data());
        if profile.as_ref().is_some_and(|p| p.len() > 1024 * 1024) {
            return Err("Screen profile exceeds the memory budget".into());
        }
        let info = image.bitmap_info().bits();
        if image.bits_per_component() != 8 || image.bits_per_pixel() != 32 || info & (1 << 8) != 0 {
            return Err("Unsupported screen capture pixel format".into());
        }
        let (first, premultiplied, opaque) = match image.alpha_info() {
            Alpha::AlphaPremultipliedFirst => (true, true, false),
            Alpha::AlphaPremultipliedLast => (false, true, false),
            Alpha::AlphaFirst => (true, false, false),
            Alpha::AlphaLast => (false, false, false),
            Alpha::AlphaNoneSkipFirst => (true, false, true),
            Alpha::AlphaNoneSkipLast => (false, false, true),
            _ => return Err("Unsupported screen capture alpha layout".into()),
        };
        let little = match info & 0x7000 {
            0 => cfg!(target_endian = "little"),
            0x2000 => true,
            0x4000 => false,
            _ => return Err("Unsupported screen capture byte order".into()),
        };
        let order = match (first, little) {
            (true, true) => [2, 1, 0, 3],
            (true, false) => [1, 2, 3, 0],
            (false, true) => [3, 2, 1, 0],
            (false, false) => [0, 1, 2, 3],
        };
        let stride = image.bytes_per_row();
        checked_stride(width, height, stride)?;
        let bytes = image.data_provider().and_then(|p| p.copy_data()).ok_or("Screen image has no pixels")?;
        let rgba = unpack_rgba(bytes.bytes(), width, height, stride, order, premultiplied, opaque)?;
        let (w, h) = (bounds.size.width as f32, bounds.size.height as f32);
        let (x, y) = (bounds.origin.x as f32, bounds.origin.y as f32);
        if ![w, h, x, y].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
            return Err("Invalid display geometry".into());
        }
        let scale = width as f32 / w;
        images.push(ScreenImage {
            monitor: find_monitor(placements, (x * scale).round() as i32, (y * scale).round() as i32, width as u32, height as u32),
            position: egui::pos2(x, y),
            size: egui::vec2(w, h),
            width,
            height,
            rgba,
            profile: profile.map(|p| p.bytes().to_vec()),
        });
    }
    Ok(Capture::Images(images))
}
// CoreGraphics rows can be padded and their byte order/alpha vary. Kept platform independent
// so platform-independent tests also cover the macOS decoding path against synthetic pixels.
#[cfg(any(target_os = "macos", test))]
fn checked_stride(width: usize, height: usize, stride: usize) -> Result<usize, String> {
    count(width, height)?;
    let row = width.checked_mul(4).ok_or("Invalid screen row")?;
    if stride < row || stride > row.saturating_add(4096) {
        return Err("Invalid screen row stride".into());
    }
    stride.checked_mul(height).filter(|&n| n <= MAX_PIXELS.saturating_mul(4)).ok_or_else(|| "Screen buffer exceeds the memory budget".into())
}
#[cfg(any(target_os = "macos", test))]
fn unpack_rgba(bytes: &[u8], width: usize, height: usize, stride: usize, order: [usize; 4], premultiplied: bool, opaque: bool) -> Result<Vec<u8>, String> {
    let length = checked_stride(width, height, stride)?;
    if bytes.len() < length || order.iter().any(|&i| i > 3) {
        return Err("Invalid screen pixel buffer".into());
    }
    let mut out = Vec::with_capacity(count(width, height)?.saturating_mul(4));
    for row in bytes.chunks_exact(stride).take(height) {
        for p in row[..width * 4].as_chunks::<4>().0 {
            let a = if opaque { 255 } else { p[order[3]] };
            for &i in &order[..3] {
                let v = u32::from(p[i]);
                out.push(if premultiplied && a != 255 {
                    if a == 0 { 0 } else { ((v * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8 }
                } else {
                    v as u8
                });
            }
            out.push(255);
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_capture_decodes_padded_bgra_and_premultiplied_argb() {
        assert_eq!(
            unpack_rgba(&[10, 20, 30, 0, 99, 99, 99, 99, 40, 50, 60, 0, 99, 99, 99, 99], 1, 2, 8, [2, 1, 0, 3], false, true).unwrap(),
            [30, 20, 10, 255, 60, 50, 40, 255]
        );
        assert_eq!(unpack_rgba(&[128, 64, 32, 16], 1, 1, 4, [1, 2, 3, 0], true, false).unwrap(), [128, 64, 32, 255]);
        assert_eq!(unpack_rgba(&[0, 0, 0, 0], 1, 1, 4, [0, 1, 2, 3], true, false).unwrap(), [0, 0, 0, 255]);
    }
    #[test]
    fn screen_capture_rejects_unbounded_or_truncated_buffers() {
        assert!(count(usize::MAX, 2).is_err());
        assert!(checked_stride(1, 1, usize::MAX).is_err());
        assert!(checked_stride(1, MAX_PIXELS, 4096).is_err());
        assert!(unpack_rgba(&[0; 3], 1, 1, 4, [0, 1, 2, 3], false, true).is_err());
        assert!(unpack_rgba(&[0; 4], 1, 1, 4, [4, 1, 2, 3], false, true).is_err());
    }
    #[test]
    fn screen_capture_matches_negative_and_hidpi_monitor_geometry() {
        let m = [Placement { index: 2, x: -3840, y: 0, width: 3840, height: 2160 }];
        assert_eq!(find_monitor(&m, -3840, 0, 3840, 2160), Some(2));
        assert_eq!(find_monitor(&m, 0, 0, 1920, 1080), None);
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn capture(_scale: f32, _stop: &std::sync::atomic::AtomicBool, _placements: &[Placement]) -> Result<Capture, String> {
    Err("Screen color picking is unavailable on this platform".into())
}
