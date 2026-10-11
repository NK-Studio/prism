use super::*;
#[cfg(target_os = "windows")]
use egui::{Event, Modifiers, PointerButton, pos2};
#[cfg(target_os = "windows")]
use egui_kittest::Harness;
#[cfg(target_os = "windows")]
use serde_json::json;

fn alpha(image: &CustomCursorImage, x: u16, y: u16) -> u8 {
    image.rgba[(usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4 + 3]
}

fn frame(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
    let mut output = ctx.run_ui(Default::default(), draw);
    // These tests inspect cursor output without a renderer.
    output.textures_delta.clear();
    output
}

#[test]
fn bitmap_has_a_transparent_hotspot_and_the_correct_radius_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let radius = 20.0;
        let image = rasterize(Shape::Circle { radius, centre: false }, scale).unwrap();
        let [x, y] = image.hotspot;
        assert_eq!(image.size, [2 * x + 1, 2 * y + 1]);
        assert_eq!(image.rgba.len(), usize::from(image.size[0]).pow(2) * 4);
        assert_eq!(alpha(&image, x, y), 0, "the document is visible through the tip");
        assert_eq!(alpha(&image, 0, 0), 0);
        let edge = x + (radius * scale) as u16;
        assert!(alpha(&image, edge, y) > 230);
        assert_eq!(alpha(&image, x - (radius * scale) as u16, y), alpha(&image, edge, y));
        let edge_byte = (usize::from(y) * usize::from(image.size[0]) + usize::from(edge)) * 4;
        assert_eq!(image.rgba[edge_byte], 242, "straight RGBA: 235/255 white over 160/255 black (premultiplied would be 235)");
        assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[0] == 0), "dark outline stays visible on white");
    }
}

#[test]
fn precise_and_tip_centre_crosshairs_have_distinct_hotspots() {
    let ring = rasterize(Shape::Circle { radius: 20.0, centre: true }, 1.0).unwrap();
    assert!(alpha(&ring, ring.hotspot[0], ring.hotspot[1]) > 230);
    let precise = rasterize(Shape::Crosshair { length: 8.0, gap: 2.0 }, 1.0).unwrap();
    let [x, y] = precise.hotspot;
    assert_eq!(alpha(&precise, x, y), 0);
    assert!(alpha(&precise, x + 5, y) > 230);
    let solid = rasterize(Shape::Crosshair { length: 6.0, gap: 0.0 }, 1.0).unwrap();
    assert!(alpha(&solid, solid.hotspot[0], solid.hotspot[1]) > 230);
}

#[test]
fn hostile_sizes_are_rejected_before_allocating() {
    for radius in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 2048.0] {
        assert!(rasterize(Shape::Circle { radius, centre: false }, 1.0).is_none());
    }
    for scale in [f32::NAN, f32::INFINITY, -1.0, 0.0, f32::MAX] {
        assert!(rasterize(Shape::Circle { radius: 20.0, centre: false }, scale).is_none());
    }
    for (length, gap) in [(1.0, 2.0), (8.0, -1.0), (f32::NAN, 0.0), (8.0, f32::NAN)] {
        assert!(rasterize(Shape::Crosshair { length, gap }, 1.0).is_none());
    }
}

#[test]
fn unchanged_geometry_reuses_the_os_upload_and_dpi_invalidates_it() {
    let ctx = egui::Context::default();
    let shape = Shape::Circle { radius: 20.0, centre: false };
    let a = cached_image(&ctx, shape, 1.0).unwrap();
    let b = cached_image(&ctx, shape, 1.0).unwrap();
    assert!(Arc::ptr_eq(&a.rgba, &b.rgba));
    let scaled = cached_image(&ctx, shape, 2.0).unwrap();
    assert!(!Arc::ptr_eq(&a.rgba, &scaled.rgba));
    assert!(scaled.size[0] > a.size[0]);
    let larger = cached_image(&ctx, Shape::Circle { radius: 40.0, centre: false }, 2.0).unwrap();
    assert!(larger.size[0] > scaled.size[0]);
}

#[test]
fn lifecycle_clears_sticky_images_and_respects_later_widgets() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let image = rasterize(Shape::Circle { radius: 20.0, centre: false }, 1.0).unwrap();
    let output = frame(&ctx, |ui| {
        ui.ctx().set_cursor_image(Some(image.clone()));
        ui.ctx().set_cursor_icon(CursorIcon::None);
    });
    assert!(output.platform_output.cursor_image.is_some());
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::Crosshair, "visible fallback if the OS rejects a bitmap");
    let output = frame(&ctx, |_| {});
    assert!(output.platform_output.cursor_image.is_none(), "leaving the canvas cannot keep its bitmap");
    let output = frame(&ctx, |ui| {
        ui.ctx().set_cursor_image(Some(image.clone()));
        ui.ctx().set_cursor_icon(CursorIcon::None);
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    });
    assert!(output.platform_output.cursor_image.is_none());
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::ResizeHorizontal);
}

#[test]
#[cfg(target_os = "windows")]
fn ordinary_brush_cursor_is_not_part_of_the_canvas_draw_list() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let output = frame(&ctx, |ui| {
        let icon = circle(ui.painter(), pos2(100.0, 100.0), 20.0, true);
        ui.ctx().set_cursor_icon(icon);
    });
    assert!(output.platform_output.cursor_image.is_some());
    assert!(output.shapes.iter().all(|s| matches!(s.shape, egui::Shape::Noop)));

    let output = frame(&ctx, |ui| {
        let icon = circle(ui.painter(), pos2(100.0, 100.0), 3000.0, false);
        ui.ctx().set_cursor_icon(icon);
    });
    let image = output.platform_output.cursor_image.unwrap();
    assert!(image.size[0] < 32, "oversized tips retain a small native hotspot");
    assert!(output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Circle(_))), "the full tip is not silently shrunk");
}

/// Run explicitly for visual QA: native cursors are absent from GPU screenshots.
#[test]
#[ignore = "writes target/tool-cursor-preview.png for visual inspection"]
fn write_cursor_preview() {
    let cases = [
        (Shape::Circle { radius: 20.0, centre: false }, 1.0),
        (Shape::Circle { radius: 20.0, centre: false }, 1.5),
        (Shape::Circle { radius: 20.0, centre: true }, 2.0),
        (Shape::Crosshair { length: 8.0, gap: 2.0 }, 2.0),
        (Shape::Crosshair { length: 6.0, gap: 0.0 }, 2.0),
        (Shape::Move, 1.0),
        (Shape::Move, 1.5),
        (Shape::Move, 2.0),
    ];
    let (width, height) = (600_usize, cases.len() * 100);
    let mut rgba = vec![255; width * height * 4];
    for (col, background) in [0_u8, 128, 255].into_iter().enumerate() {
        for y in 0..height {
            for x in col * 200..(col + 1) * 200 {
                rgba[(y * width + x) * 4..(y * width + x) * 4 + 3].fill(background);
            }
        }
        for (row, &(shape, scale)) in cases.iter().enumerate() {
            let image = rasterize(shape, scale).unwrap();
            let side = usize::from(image.size[0]);
            let x0 = col * 200 + (200 - side) / 2;
            let y0 = row * 100 + (100 - side) / 2;
            for (i, pixel) in image.rgba.as_chunks::<4>().0.iter().enumerate() {
                let at = ((y0 + i / side) * width + x0 + i % side) * 4;
                let a = f32::from(pixel[3]) / 255.0;
                let value = (f32::from(pixel[0]) * a + f32::from(background) * (1.0 - a)).round() as u8;
                rgba[at..at + 3].fill(value);
            }
        }
    }
    let image = prism_codecs::Image::from_u8(width as u32, height as u32, prism_codecs::ChannelLayout::Rgba, rgba).unwrap();
    let png = prism_codecs::encode(&image, prism_codecs::Format::Png, &Default::default()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tool-cursor-preview.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, png).unwrap();
}

#[cfg(target_os = "windows")]
fn harness() -> Harness<'static, crate::PrismApp> {
    let mut app = crate::PrismApp::new(prism_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0}})).unwrap();
    app.ui.tool = crate::Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(600.0, 500.0)).build_ui_state(
        |ui, app: &mut crate::PrismApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::Panel::top("cursor-test-toolbar").show(ui, |ui| {
                ui.label("Toolbar");
            });
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            crate::dialogs::show(app, &ctx);
        },
        app,
    );
    crate::PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h.event(Event::PointerMoved(h.state().last_canvas_rect.center()));
    h.run_steps(3);
    h
}

#[test]
#[cfg(target_os = "windows")]
fn brush_hover_uses_an_os_bitmap_that_survives_motion_but_not_tool_or_panel_changes() {
    let mut h = harness();
    let initial = h.output().platform_output.cursor_image.clone().expect("brush hover must reach the OS, not the painter");
    let p = h.state().last_canvas_rect.center();
    h.event(Event::PointerMoved(p + vec2(25.0, 12.0)));
    h.run_steps(2);
    let moved = h.output().platform_output.cursor_image.clone().unwrap();
    assert!(Arc::ptr_eq(&initial.rgba, &moved.rgba), "moving does not rasterize or re-upload the cursor");

    h.event(Event::PointerMoved(pos2(20.0, 12.0)));
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_none(), "toolbar restores the normal pointer");
    h.event(Event::PointerMoved(p));
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_some());

    h.state_mut().ui.tool = crate::Tool::Hand;
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_none());
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Grab);

    h.state_mut().ui.tool = crate::Tool::Brush;
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": "standard"})).unwrap();
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_none());
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Default);
}

#[test]
#[cfg(target_os = "windows")]
fn painting_preference_and_zoom_change_the_native_cursor_without_changing_pointer_input() {
    let mut h = harness();
    let image = h.output().platform_output.cursor_image.clone().unwrap();
    h.state_mut().ui.views[0].zoom *= 2.0;
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.as_ref().unwrap().size[0] > image.size[0]);
    h.state_mut().run("prefs.set", json!({"path": "cursors.showOnlyCrosshairWhilePainting", "value": true})).unwrap();
    let pos = h.state().last_canvas_rect.center();
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(pos + vec2(20.0, 0.0)));
    h.run_steps(2);
    let cross_size = h.output().platform_output.cursor_image.as_ref().unwrap().size[0];
    assert!(cross_size < image.size[0]);
    assert_eq!(h.ctx.pointer_latest_pos(), Some(pos + vec2(20.0, 0.0)));
    h.event(Event::PointerButton { pos: pos + vec2(20.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.as_ref().unwrap().size[0] > cross_size);
}

#[test]
fn move_bitmap_is_bounded_and_hotspot_is_the_arrow_tip_at_fractional_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let img = rasterize(Shape::Move, scale).unwrap();
        assert_eq!(img.rgba.len(), usize::from(img.size[0]) * usize::from(img.size[1]) * 4);
        assert_eq!(img.hotspot, [(2.0 * scale).round() as u16; 2]);
        assert!(alpha(&img, img.hotspot[0], img.hotspot[1]) > 0);
        assert_eq!(alpha(&img, img.size[0] - 1, img.size[1] - 1), 0);
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 10000.0] {
        assert!(rasterize(Shape::Move, scale).is_none());
    }
}

#[test]
fn native_move_cursor_uses_the_backends_coordinate_units_at_each_dpi() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        ctx.set_pixels_per_point(scale);
        let output = frame(&ctx, |ui| {
            let icon = move_tool(ui.painter(), egui::pos2(20.0, 20.0), false);
            ui.ctx().set_cursor_icon(icon);
        });
        assert_eq!(ctx.pixels_per_point(), scale);
        let image = output.platform_output.cursor_image.unwrap();
        let expected = if cfg!(target_os = "macos") { 1.0 } else { scale };
        assert_eq!(image.size, [(32.0 * expected).ceil() as u16; 2]);
        assert_eq!(image.hotspot, [(2.0 * expected).round() as u16; 2]);
    }
}

#[test]
fn painted_move_cursor_keeps_its_point_size_at_each_dpi() {
    let ctx = egui::Context::default();
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        ctx.set_pixels_per_point(scale);
        let output = frame(&ctx, |ui| paint(ui.painter(), egui::pos2(20.0, 20.0), Shape::Move));
        let bounds = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.calc_bounds()),
                _ => None,
            })
            .unwrap();
        assert_eq!(bounds.min, egui::pos2(18.0, 18.0));
        assert_eq!(bounds.size(), vec2(32.0, 32.0));
    }
}

#[test]
fn move_cursor_clears_its_image_on_drag_and_when_a_panel_takes_the_pointer() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let idle = frame(&ctx, |ui| {
        let icon = move_tool(ui.painter(), egui::pos2(20.0, 20.0), false);
        ui.ctx().set_cursor_icon(icon);
    });
    assert!(idle.platform_output.cursor_image.is_some());
    assert_eq!(idle.platform_output.cursor_icon, CursorIcon::Default);
    let moving = frame(&ctx, |ui| {
        let icon = move_tool(ui.painter(), egui::pos2(20.0, 20.0), true);
        ui.ctx().set_cursor_icon(icon);
    });
    assert!(moving.platform_output.cursor_image.is_none());
    assert_eq!(moving.platform_output.cursor_icon, CursorIcon::Default);
    let panel = frame(&ctx, |ui| {
        let icon = move_tool(ui.painter(), egui::pos2(20.0, 20.0), false);
        ui.ctx().set_cursor_icon(icon);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    });
    assert!(panel.platform_output.cursor_image.is_none());
    assert_eq!(panel.platform_output.cursor_icon, CursorIcon::PointingHand);
}
