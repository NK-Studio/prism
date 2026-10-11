//! A modal that floats in a borderless OS window of its own, above Prism's document windows,
//! instead of inside one window (where it would be clipped to that window and hidden behind
//! floating documents). Sized to its contents and opened centred over the window it came from.

use crate::PrismApp;

/// Shows `viewport` from its host window's `ctx`. `add` draws the contents into the new window's
/// context and returns the area they cover once laid out (`None` during a sizing pass). Headless
/// and embedded contexts have no OS windows: there `add` draws into `ctx` itself.
pub fn show(
    app: &mut PrismApp,
    ctx: &egui::Context,
    key: egui::Id,
    viewport: egui::ViewportId,
    title: String,
    mut add: impl FnMut(&mut PrismApp, &egui::Context) -> Option<egui::Rect>,
) {
    if ctx.embed_viewports() {
        add(app, ctx);
        return;
    }
    let measured: Option<egui::Vec2> = ctx.data(|m| m.get_temp(key.with("size")));
    let position = match ctx.data(|m| m.get_temp::<egui::Pos2>(key.with("pos"))) {
        Some(p) => p,
        None => {
            let host = ctx
                .input(|i| i.viewport().outer_rect)
                .filter(|r| r.is_finite())
                .unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
            match measured {
                // Centred once its real size is known; until then it is hidden.
                Some(size) => {
                    let p = (host.center() - size / 2.0).max(egui::pos2(host.left().min(0.0), 24.0));
                    ctx.data_mut(|m| m.insert_temp(key.with("pos"), p));
                    p
                }
                None => host.center(),
            }
        }
    };
    // macOS: a titled window with its title bar and buttons hidden and the content under it, for
    // the system's rounded corners and shadow. Elsewhere a plain borderless window.
    let mac = cfg!(target_os = "macos");
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_decorations(mac)
        .with_fullsize_content_view(mac)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_inner_size(measured.unwrap_or(egui::vec2(380.0, 200.0)))
        .with_position(position)
        .with_visible(measured.is_some())
        .with_window_level(crate::doc_windows::dialog_level(app, ctx));
    ctx.show_viewport_immediate(viewport, builder, |ui, _| {
        let child = ui.ctx().clone();
        app.collect_screenshots(&child);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |_| {});
        if child.input(|i| i.viewport().close_requested()) {
            child.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if let Some(size) = add(app, &child).map(|r| r.size().round()).filter(|s| s.is_finite() && s.x >= 1.0 && s.y >= 1.0)
            && measured != Some(size)
        {
            child.data_mut(|m| m.insert_temp(key.with("size"), size));
            child.request_repaint();
        }
    });
}

/// Whether the window shown under `key` has been laid out (and so is visible).
pub fn measured(ctx: &egui::Context, key: egui::Id) -> bool {
    ctx.data(|m| m.get_temp::<egui::Vec2>(key.with("size"))).is_some()
}

/// The frame of a modal drawn in its own window: the OS window is its edge, so no shadow or
/// rounding to clip. On macOS the window draws its own rounded border too.
pub fn frame(ctx: &egui::Context) -> egui::Frame {
    let frame = egui::Frame::popup(&ctx.global_style()).shadow(egui::Shadow::NONE).corner_radius(0);
    if cfg!(target_os = "macos") { frame.stroke(egui::Stroke::NONE) } else { frame }
}

/// The modal's area pinned to the window's top-left corner.
pub fn area(id: egui::Id) -> egui::Area {
    egui::Modal::default_area(id).anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
}

/// A drag that started on the title row moves the whole OS window.
pub fn drag_window(ctx: &egui::Context, title: &egui::Response) {
    if title.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

/// While a floating modal owns the app, a window behind it takes no pointer or keyboard input:
/// a full-window catcher above everything else, and no widget keeps text focus. `dim` shades the
/// window, so the modal reads as the one thing to answer.
pub fn block(ctx: &egui::Context, dim: bool) {
    let rect = ctx.content_rect();
    egui::Area::new(egui::Id::new("floating-modal-blocker")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        let r = egui::Rect::from_min_size(rect.min, rect.size());
        ui.allocate_rect(r, egui::Sense::click_and_drag());
        if dim {
            ui.painter().rect_filled(r, 0.0, egui::Color32::from_black_alpha(90));
        }
    });
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
}

/// Turns `ctx`'s window title-bar buttons (close, minimise, zoom) on or off, sending the command
/// only when that changes (a builder change would recreate the window).
pub fn enable_buttons(ctx: &egui::Context, enabled: bool) {
    let key = egui::Id::new(("window-buttons-enabled", ctx.viewport_id()));
    if ctx.data(|m| m.get_temp::<bool>(key)).unwrap_or(true) != enabled {
        ctx.data_mut(|m| m.insert_temp(key, enabled));
        ctx.send_viewport_cmd(egui::ViewportCommand::EnableButtons { close: enabled, minimized: enabled, maximize: enabled });
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    #[test]
    fn a_blocked_window_ignores_clicks_even_in_its_floating_panels() {
        for blocked in [false, true] {
            let mut clicks = 0;
            let mut h = egui_kittest::Harness::new_ui(|ui| {
                if ui.button("Edit layer").clicked() {
                    clicks += 1;
                }
                // Panels in their own egui windows too, which `ui.disable` would not reach.
                egui::Window::new("Layers").default_pos([300.0, 200.0]).show(ui.ctx(), |ui| {
                    if ui.button("Delete layer").clicked() {
                        clicks += 1;
                    }
                });
                if blocked {
                    super::block(ui.ctx(), true);
                }
            });
            h.get_by_label("Edit layer").click();
            h.run();
            h.get_by_label("Delete layer").click();
            h.run();
            drop(h);
            assert_eq!(clicks, if blocked { 0 } else { 2 }, "blocked={blocked}");
        }
    }
}
