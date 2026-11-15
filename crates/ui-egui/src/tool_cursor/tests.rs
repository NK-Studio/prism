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
    let mut cases = vec![
        (Shape::Circle { radius: 20.0, centre: false }, 1.0),
        (Shape::Circle { radius: 20.0, centre: false }, 1.5),
        (Shape::Circle { radius: 20.0, centre: true }, 2.0),
        (Shape::Crosshair { length: 8.0, gap: 2.0 }, 2.0),
        (Shape::Crosshair { length: 6.0, gap: 0.0 }, 2.0),
    ];
    for scale in [1.0, 1.5, 2.0] {
        cases.extend(Art::ALL.map(|art| (Shape::Art(art), scale)));
    }
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
                for c in 0..3 {
                    rgba[at + c] = (f32::from(pixel[c]) * a + f32::from(background) * (1.0 - a)).round() as u8;
                }
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
    let hand = h.output().platform_output.cursor_image.clone().expect("the Hand tool's own hand, not the brush tip");
    assert!(!Arc::ptr_eq(&hand.rgba, &initial.rgba));
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
        let img = rasterize(Shape::Art(Art::Move), scale).unwrap();
        assert_eq!(img.rgba.len(), usize::from(img.size[0]) * usize::from(img.size[1]) * 4);
        assert_eq!(img.hotspot, [(3.2 * scale).floor() as u16, (2.5 * scale).floor() as u16]);
        assert!(alpha(&img, img.hotspot[0], img.hotspot[1]) > 0);
        assert_eq!(alpha(&img, img.size[0] - 1, img.size[1] - 1), 0);
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 10000.0] {
        assert!(rasterize(Shape::Art(Art::Move), scale).is_none());
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
        assert_eq!(image.hotspot, [(3.2 * expected).floor() as u16, (2.5 * expected).floor() as u16]);
    }
}

#[test]
fn painted_move_cursor_keeps_its_point_size_at_each_dpi() {
    let ctx = egui::Context::default();
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        ctx.set_pixels_per_point(scale);
        let output = frame(&ctx, |ui| paint(ui.painter(), egui::pos2(20.0, 20.0), Shape::Art(Art::Move)));
        let bounds = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.calc_bounds()),
                _ => None,
            })
            .unwrap();
        assert_eq!(bounds.min, egui::pos2(17.0, 18.0), "the arrow's tip on the pointer");
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

fn ink(image: &CustomCursorImage, x: u16, y: u16) -> bool {
    let at = (usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4;
    image.rgba[at + 3] > 230 && image.rgba[at] < 40
}

#[test]
fn zoom_bitmap_is_a_glass_with_a_plus_or_minus_about_its_hotspot_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let plus = rasterize(Shape::Art(Art::ZoomIn), scale).unwrap();
        let minus = rasterize(Shape::Art(Art::ZoomOut), scale).unwrap();
        for img in [&plus, &minus] {
            assert_eq!(img.size, [(32.0 * scale).ceil() as u16; 2]);
            assert_eq!(img.rgba.len(), usize::from(img.size[0]) * usize::from(img.size[1]) * 4);
            assert_eq!(img.hotspot, [(12.0 * scale).floor() as u16; 2]);
            let [x, y] = img.hotspot;
            assert!(ink(img, x, y), "the sign crosses the lens centre");
            assert!(ink(img, x + (2.0 * scale) as u16, y), "both signs have the horizontal bar");
            assert!(ink(img, x + (7.2 * scale) as u16, y), "the lens ring");
            assert!(ink(img, x + (9.0 * scale) as u16, y + (9.0 * scale) as u16), "the handle");
            // The corners off the glass's diagonal stay clear.
            assert_eq!(alpha(img, img.size[0] - 1, 0), 0);
            assert_eq!(alpha(img, 0, img.size[1] - 1), 0);
            // The lens is light, so the sign reads on a dark image.
            let lens = (usize::from(y + (4.0 * scale) as u16) * usize::from(img.size[0]) + usize::from(x + (3.0 * scale) as u16)) * 4;
            assert!(img.rgba[lens] > 230 && img.rgba[lens + 3] > 230);
        }
        let [x, y] = plus.hotspot;
        let up = y - (2.5 * scale) as u16;
        assert!(ink(&plus, x, up), "zoom in: the vertical bar");
        assert!(!ink(&minus, x, up), "zoom out: no vertical bar");
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 10000.0] {
        assert!(rasterize(Shape::Art(Art::ZoomIn), scale).is_none());
    }
}

#[test]
fn zoom_cursor_is_an_os_bitmap_with_the_stock_zoom_cursor_as_its_fallback() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    for (out, stock) in [(false, CursorIcon::ZoomIn), (true, CursorIcon::ZoomOut)] {
        let output = frame(&ctx, |ui| {
            let icon = zoom(ui.painter(), egui::pos2(40.0, 40.0), out);
            ui.ctx().set_cursor_icon(icon);
        });
        assert!(output.platform_output.cursor_image.is_some());
        assert_eq!(output.platform_output.cursor_icon, stock);
    }
    let panel = frame(&ctx, |ui| {
        let icon = zoom(ui.painter(), egui::pos2(40.0, 40.0), false);
        ui.ctx().set_cursor_icon(icon);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    });
    assert!(panel.platform_output.cursor_image.is_none());
    assert_eq!(panel.platform_output.cursor_icon, CursorIcon::PointingHand);
}

#[test]
fn hand_bitmap_is_an_open_hand_that_closes_while_dragging_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let open = rasterize(Shape::Art(Art::Hand), scale).unwrap();
        let closed = rasterize(Shape::Art(Art::HandClosed), scale).unwrap();
        for img in [&open, &closed] {
            assert_eq!(img.size, [(32.0 * scale).ceil() as u16; 2]);
            assert_eq!(img.rgba.len(), usize::from(img.size[0]) * usize::from(img.size[1]) * 4);
            assert_eq!(img.hotspot, [(16.0 * scale).floor() as u16, (17.0 * scale).floor() as u16]);
            // The palm under the hotspot is light, so the hand reads on a dark image.
            let [x, y] = img.hotspot;
            let palm = (usize::from(y + (3.0 * scale) as u16) * usize::from(img.size[0]) + usize::from(x)) * 4;
            assert!(img.rgba[palm] > 230 && img.rgba[palm + 3] > 230, "light palm at {scale}");
            assert_eq!(alpha(img, img.size[0] - 1, 0), 0, "the corner stays clear");
        }
        // Inside the middle finger: up when open, curled into the fist when closed.
        let (x, tip) = ((15.55 * scale) as u16, (7.5 * scale) as u16);
        assert!(alpha(&open, x, tip) > 230, "open: the raised finger at {scale}");
        assert_eq!(alpha(&closed, x, tip), 0, "closed: the finger curled down at {scale}");
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 10000.0] {
        assert!(rasterize(Shape::Art(Art::Hand), scale).is_none());
    }
}

/// Every drawing, at every DPI: it parses, its hotspot is inside it, it has dark ink with a light
/// halo beside it (so it reads on black and on white), and the box's edge cuts nothing but the
/// halo's soft tail.
#[test]
fn every_drawing_rasterizes_with_ink_a_halo_and_a_hotspot_at_each_dpi() {
    for art in Art::ALL {
        for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let img = rasterize(Shape::Art(art), scale).unwrap_or_else(|| panic!("{art:?} at {scale}"));
            let side = img.size[0];
            assert_eq!(img.size, [(32.0 * scale).ceil() as u16; 2], "{art:?}");
            assert_eq!(img.rgba.len(), usize::from(side).pow(2) * 4, "{art:?}");
            assert!(img.hotspot[0] < side && img.hotspot[1] < side, "{art:?}");
            let pixels = img.rgba.as_chunks::<4>().0;
            let dark = pixels.iter().filter(|p| p[3] > 230 && p[0] < 40).count();
            let light = pixels.iter().filter(|p| p[3] > 120 && p[0] > 200).count();
            assert!(dark as f32 > 12.0 * scale * scale, "{art:?} at {scale}: {dark} dark pixels");
            assert!(light > dark / 3, "{art:?} at {scale}: {light} light pixels around {dark} dark ones");
            // Colour is straight, never premultiplied: no grey fringe where the halo fades out.
            assert!(pixels.iter().filter(|p| p[3] > 0 && p[3] < 60).all(|p| p[0] > 200), "{art:?} at {scale}");
            for i in 0..side {
                for (x, y) in [(i, 0), (i, side - 1), (0, i), (side - 1, i)] {
                    assert!(alpha(&img, x, y) < 64, "{art:?} at {scale}: clipped at ({x}, {y})");
                }
            }
        }
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX, 10000.0, 64.0] {
            assert!(rasterize(Shape::Art(art), scale).is_none(), "{art:?} at {scale}");
        }
    }
}

#[test]
fn hotspots_are_on_the_part_of_the_drawing_that_points() {
    let at = |art: Art, scale: f32| {
        let img = rasterize(Shape::Art(art), scale).unwrap();
        let [x, y] = img.hotspot;
        let i = (usize::from(y) * usize::from(img.size[0]) + usize::from(x)) * 4;
        [img.rgba[i], img.rgba[i + 3]]
    };
    for scale in [1.0, 2.0] {
        // Tips, centres and the baseline tick are ink (a tip covers part of its pixel at 1x).
        for art in [Art::Arrow, Art::Move, Art::Lasso, Art::LassoPolygon, Art::Cross, Art::CrossRect, Art::Crop, Art::Text, Art::TextVertical, Art::Convert] {
            let [value, alpha] = at(art, scale);
            assert!(alpha > 200 && value < 140, "{art:?} at {scale}: {value} {alpha}");
        }
        // The nib's, the white arrow's, the wand's and the pipette's ends are opaque (line or fill).
        for art in [Art::ArrowWhite, Art::Pen, Art::PenNew, Art::PenAdd, Art::PenDelete, Art::PenClose, Art::Wand, Art::Eyedropper] {
            assert!(at(art, scale)[1] > 200, "{art:?} at {scale}");
        }
    }
}

#[test]
fn roles_map_to_their_drawings_and_selection_modes_share_one() {
    use crate::{state::Tool, tool_feedback::Badge};
    let role = |tool, badge| Art::for_role(crate::cursor_pack::tool_asset(tool, badge, false, false));
    for badge in [None, Some(Badge::Add), Some(Badge::Subtract), Some(Badge::Intersect)] {
        assert_eq!(role(Tool::RectMarquee, badge), Some(Art::Cross));
        assert_eq!(role(Tool::EllipseMarquee, badge), Some(Art::Cross));
        assert_eq!(role(Tool::Lasso, badge), Some(Art::Lasso));
        assert_eq!(role(Tool::PolygonLasso, badge), Some(Art::LassoPolygon));
        assert_eq!(role(Tool::MagicWand, badge), Some(Art::Wand));
        assert_eq!(role(Tool::ObjectSelection, badge), Some(Art::CrossRect));
    }
    for (tool, art) in [
        (Tool::Move, Art::Move),
        (Tool::Crop, Art::Crop),
        (Tool::Eyedropper, Art::Eyedropper),
        (Tool::Type, Art::Text),
        (Tool::VerticalType, Art::TextVertical),
        (Tool::Pen, Art::PenNew),
        (Tool::PathSelection, Art::Arrow),
        (Tool::DirectSelection, Art::ArrowWhite),
        (Tool::Rectangle, Art::Cross),
        (Tool::Hand, Art::Hand),
        (Tool::RotateView, Art::Hand),
        (Tool::Zoom, Art::ZoomIn),
    ] {
        assert_eq!(role(tool, None), Some(art), "{tool:?}");
    }
    assert_eq!(Art::for_role(crate::cursor_pack::tool_asset(Tool::Zoom, None, true, false)), Some(Art::ZoomOut));
    assert_eq!(Art::for_role(crate::cursor_pack::tool_asset(Tool::Hand, None, false, true)), Some(Art::HandClosed));
    // Tools without a drawing keep the crosshair; hostile ids are nothing.
    for id in [crate::cursor_pack::tool_asset(Tool::Gradient, None, false, false), 0, 1, u32::MAX] {
        assert_eq!(Art::for_role(id), None, "{id}");
    }
}

#[test]
fn a_role_with_a_drawing_is_an_os_bitmap_with_the_tools_stock_cursor_as_its_fallback() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    for (role, stock) in [(1450, CursorIcon::Crosshair), (1462, CursorIcon::Text), (1063, CursorIcon::Crosshair), (1017, CursorIcon::Crosshair)] {
        let output = frame(&ctx, |ui| {
            let icon = standard(ui.painter(), egui::pos2(40.0, 40.0), role, stock).expect("a drawing");
            ui.ctx().set_cursor_icon(icon);
        });
        let image = output.platform_output.cursor_image.expect("carried by the OS on every platform");
        assert_eq!(image.size, [32, 32]);
        assert_eq!(output.platform_output.cursor_icon, stock, "role {role}");
        assert!(output.shapes.iter().all(|s| matches!(s.shape, egui::Shape::Noop)), "not in the canvas draw list");
    }
    // No drawing: the caller keeps its stock cursor, and no stale bitmap is left behind.
    let output = frame(&ctx, |ui| {
        assert_eq!(standard(ui.painter(), egui::pos2(40.0, 40.0), 1009, CursorIcon::Crosshair), None);
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    });
    assert!(output.platform_output.cursor_image.is_none());
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::Crosshair);
    // A widget over the canvas still wins.
    let panel = frame(&ctx, |ui| {
        let icon = standard(ui.painter(), egui::pos2(40.0, 40.0), 1450, CursorIcon::Crosshair).unwrap();
        ui.ctx().set_cursor_icon(icon);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    });
    assert!(panel.platform_output.cursor_image.is_none());
    assert_eq!(panel.platform_output.cursor_icon, CursorIcon::PointingHand);
}

/// Each tool's drawing reaches the OS from the real canvas, with the tool's stock cursor behind it.
#[test]
#[cfg(target_os = "windows")]
fn tools_show_their_drawings_on_the_canvas() {
    use crate::Tool;
    let mut h = harness();
    let bitmap = |art: Art| rasterize(Shape::Art(art), 1.0).unwrap().rgba;
    for (tool, art, stock) in [
        (Tool::RectMarquee, Art::Cross, CursorIcon::Crosshair),
        (Tool::Lasso, Art::Lasso, CursorIcon::Crosshair),
        (Tool::PolygonLasso, Art::LassoPolygon, CursorIcon::Crosshair),
        (Tool::MagicWand, Art::Wand, CursorIcon::Crosshair),
        (Tool::ObjectSelection, Art::CrossRect, CursorIcon::Crosshair),
        (Tool::Eyedropper, Art::Eyedropper, CursorIcon::Crosshair),
        (Tool::Type, Art::Text, CursorIcon::Text),
        (Tool::Pen, Art::PenNew, CursorIcon::Crosshair),
        (Tool::PathSelection, Art::Arrow, CursorIcon::Default),
        (Tool::DirectSelection, Art::ArrowWhite, CursorIcon::Default),
        (Tool::Rectangle, Art::Cross, CursorIcon::Crosshair),
        // No drawing of its own: the same crosshair, not Windows' inverting one.
        (Tool::Gradient, Art::Cross, CursorIcon::Crosshair),
        (Tool::Move, Art::Move, CursorIcon::Default),
        (Tool::Hand, Art::Hand, CursorIcon::Grab),
        (Tool::Zoom, Art::ZoomIn, CursorIcon::ZoomIn),
    ] {
        h.state_mut().ui.tool = tool;
        h.run_steps(3);
        let image = h.output().platform_output.cursor_image.clone().unwrap_or_else(|| panic!("{tool:?}: no OS bitmap"));
        assert_eq!(image.size, [32, 32], "{tool:?}");
        assert!(image.rgba == bitmap(art), "{tool:?} shows {art:?}");
        assert_eq!(image.hotspot, art.hotspot().map(|h| h.floor() as u16), "{tool:?}");
        assert_eq!(h.output().platform_output.cursor_icon, stock, "{tool:?}");
    }
    // Preferences > Cursors > Other Cursors > Precise: the crosshair for every tool but the views'.
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    for tool in [Tool::Lasso, Tool::MagicWand, Tool::Eyedropper, Tool::Type, Tool::Move, Tool::Pen] {
        h.state_mut().ui.tool = tool;
        h.run_steps(3);
        let image = h.output().platform_output.cursor_image.clone().unwrap_or_else(|| panic!("{tool:?}: no OS bitmap"));
        assert!(image.rgba == bitmap(Art::Cross), "{tool:?}: precise");
    }
    h.state_mut().ui.tool = Tool::Zoom;
    h.run_steps(3);
    assert!(h.output().platform_output.cursor_image.clone().unwrap().rgba == bitmap(Art::ZoomIn), "the glass stays");
}

#[test]
fn hand_cursor_is_an_os_bitmap_with_the_stock_grab_cursor_as_its_fallback() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    for (dragging, stock) in [(false, CursorIcon::Grab), (true, CursorIcon::Grabbing)] {
        let output = frame(&ctx, |ui| {
            let icon = hand(ui.painter(), egui::pos2(40.0, 40.0), dragging);
            ui.ctx().set_cursor_icon(icon);
        });
        assert!(output.platform_output.cursor_image.is_some(), "not winit's four-way Grab arrow");
        assert_eq!(output.platform_output.cursor_icon, stock);
    }
}

#[test]
#[cfg(target_os = "windows")]
fn hand_tool_shows_the_hand_on_the_canvas() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::Tool::Hand;
    h.run_steps(2);
    let open = h.output().platform_output.cursor_image.clone().expect("the Hand tool draws its own cursor");
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Grab);
    assert_eq!(open.size, [32, 32]);
    // Pressed and dragged: the closed hand, all the way through the drag.
    let p = h.state().last_canvas_rect.center();
    let button = |pressed: bool, pos| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.event(button(true, p));
    h.run_steps(1);
    for i in 1..=6 {
        h.event(Event::PointerMoved(p + vec2(8.0 * i as f32, 5.0 * i as f32)));
        h.run_steps(1);
        let closed = h.output().platform_output.cursor_image.clone().unwrap_or_else(|| panic!("step {i}: no hand while dragging: {:?}", h.output().platform_output.cursor_icon));
        assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Grabbing, "step {i}");
        assert_ne!(closed.rgba, open.rgba, "step {i}: the closed hand");
    }
    h.event(button(false, p + vec2(48.0, 30.0)));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_image.clone().map(|c| c.rgba), Some(open.rgba), "open again once released");
}

#[test]
#[cfg(target_os = "windows")]
fn zoom_tool_and_the_temporary_zoom_keys_show_the_glass_on_the_canvas() {
    let key = |pressed: bool, modifiers: Modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let mut h = harness();
    let brush = h.output().platform_output.cursor_image.clone().unwrap();
    h.state_mut().ui.tool = crate::Tool::Zoom;
    h.run_steps(2);
    let plus = h.output().platform_output.cursor_image.clone().expect("the Zoom tool draws its own cursor");
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::ZoomIn);
    assert_eq!(plus.size, [32, 32]);

    // Ctrl+Space over the Brush: the glass; with Alt as well, the minus.
    h.state_mut().ui.tool = crate::Tool::Brush;
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    h.event(Event::ModifiersChanged(ctrl));
    h.event(key(true, ctrl));
    h.run_steps(2);
    let held = h.output().platform_output.cursor_image.clone().unwrap();
    assert_eq!(held.rgba, plus.rgba);
    let alt = Modifiers { alt: true, ..ctrl };
    h.event(Event::ModifiersChanged(alt));
    h.event(key(true, alt));
    h.run_steps(2);
    let minus = h.output().platform_output.cursor_image.clone().unwrap();
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::ZoomOut);
    assert_ne!(minus.rgba, plus.rgba);

    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.event(key(false, Modifiers::NONE));
    h.run_steps(2);
    let back = h.output().platform_output.cursor_image.clone().unwrap();
    assert_eq!(back.size, brush.size, "releasing the keys gives the brush tip back");
}
