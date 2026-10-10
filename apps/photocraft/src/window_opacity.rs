//! Safe Win32 window opacity for the document docking preview (including caption buttons).
use photocraft_ui_egui::WindowOpacityFn;
use winsafe::{HWND, co};

pub fn service() -> WindowOpacityFn {
    // Only faded windows are retained; restoring releases the association. Preserve pre-existing
    // extended styles, and never enumerate or change another process's windows.
    let mut windows = std::collections::HashMap::<egui::ViewportId, (HWND, bool)>::new();
    Box::new(move |ctx, opacity| {
        let id = ctx.viewport_id();
        let known = windows.remove(&id).filter(|(h, _)| h.IsWindow() && h.GetWindowThreadProcessId().1 == std::process::id());
        let found = known.or_else(|| {
            let expected = ctx.input(|i| i.viewport().outer_rect)? * ctx.pixels_per_point();
            let mut found = None;
            let _ = winsafe::EnumWindows(|h| {
                if h.GetWindowThreadProcessId().1 == std::process::id()
                    && h.IsWindowVisible()
                    && let Ok(r) = h.GetWindowRect()
                {
                    let actual = egui::Rect::from_min_max(egui::pos2(r.left as f32, r.top as f32), egui::pos2(r.right as f32, r.bottom as f32));
                    if (actual.min - expected.min).length() < 3.0 && (actual.size() - expected.size()).length() < 3.0 {
                        let was_layered = h.style_ex().has(co::WS_EX::LAYERED);
                        found = Some((h, was_layered));
                        return false;
                    }
                }
                true
            });
            found
        });
        let Some((window, was_layered)) = found else { return false };
        let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
        window.set_style_ex(window.style_ex() | co::WS_EX::LAYERED);
        if let Err(e) = window.SetLayeredWindowAttributes(winsafe::COLORREF::from_rgb(0, 0, 0), (opacity * 255.0).round() as u8, co::LWA::ALPHA) {
            if !was_layered {
                window.set_style_ex(window.style_ex() & !co::WS_EX::LAYERED);
            }
            log::warn!("document window opacity: {e}");
            return false;
        }
        if opacity == 1.0 {
            if !was_layered {
                window.set_style_ex(window.style_ex() & !co::WS_EX::LAYERED);
            }
            windows.remove(&id);
        } else {
            windows.insert(id, (window, was_layered));
        }
        true
    })
}
