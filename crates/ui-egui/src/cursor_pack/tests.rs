use super::*;
use prism_codecs::{ChannelLayout, Format, Image as CodecImage, SampleType};

fn png(side: u32) -> Vec<u8> {
    let image = CodecImage::from_raw(side, side, ChannelLayout::Rgba, SampleType::U8, vec![255; (side * side * 4) as usize]).unwrap();
    prism_codecs::encode(&image, Format::Png, &Default::default()).unwrap()
}
fn repr(side: u16, scale: u16) -> Representation {
    Representation { file: format!("Alpha_1004-{scale}x.png"), scale, size: [side; 2], hotspot: [scale; 2] }
}
fn pack(ctx: &egui::Context) {
    let mut images = BTreeMap::new();
    let ids: std::collections::BTreeSet<u32> = Tool::ALL
        .into_iter()
        .map(|tool| tool_asset(tool, None, false, false))
        .chain([1004, 1451, 1403, 25006, 1045, 1022, 1401, 1402, 1404, 1007, 1411, 1614, 1025, 1046, 1047])
        .collect();
    for id in ids {
        images.insert(
            id,
            vec![
                Image { scale: 1, png: png(32).into(), bitmap: decode(&png(32), &repr(32, 1)).unwrap() },
                Image { scale: 2, png: png(64).into(), bitmap: decode(&png(64), &repr(64, 2)).unwrap() },
            ],
        );
    }
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(PACK_KEY), Arc::new(Pack { images, painter: false })));
    ctx.add_plugin(crate::tool_cursor::CursorLifecycle);
}

#[test]
fn malformed_geometry_and_png_are_errors() {
    assert!(decode(b"not png", &repr(32, 1)).is_err());
    assert!(decode(&png(32), &repr(64, 1)).is_err());
    for (size, hotspot, scale) in [([0, 32], [1, 1], 1), ([32; 2], [32, 1], 1), ([32; 2], [1, 1], 3), ([257; 2], [1, 1], 1)] {
        assert!(decode(&png(32), &Representation { file: "ignored".into(), scale, size, hotspot }).is_err());
    }
}
#[test]
fn load_is_bounded_and_atomic_and_does_not_traverse_paths() {
    let path = std::env::temp_dir().join(format!("prism-cursor-test-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("Alpha_1004-1x.png"), png(32)).unwrap();
    let manifest = serde_json::json!({"version":1,"cursors":[{"id":1004,"images":[{"file":"Alpha_1004-1x.png","scale":1,"size":[32,32],"hotspot":[1,1]}]}]});
    std::fs::write(path.join("manifest.json"), manifest.to_string()).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(install(&ctx, &path, false).unwrap(), 1);
    let mut bad = manifest.clone();
    bad["cursors"][0]["images"][0]["file"] = "../outside.png".into();
    std::fs::write(path.join("manifest.json"), bad.to_string()).unwrap();
    assert!(install(&ctx, &path, false).is_err());
    assert_eq!(ctx.data(|d| d.get_temp::<Arc<Pack>>(egui::Id::new(PACK_KEY))).unwrap().images.len(), 1);
    bad = manifest.clone();
    let duplicate = bad["cursors"][0].clone();
    bad["cursors"].as_array_mut().unwrap().push(duplicate);
    std::fs::write(path.join("manifest.json"), bad.to_string()).unwrap();
    assert!(install(&ctx, &path, false).is_err());
    std::fs::write(path.join("manifest.json"), vec![0; 262145]).unwrap();
    assert!(install(&ctx, &path, false).is_err());
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn dpi_hotspot_geometry_and_fractional_alpha() {
    let image = Image { scale: 2, png: png(64).into(), bitmap: decode(&png(64), &repr(64, 2)).unwrap() };
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let bitmap = rescale(&image, scale).unwrap();
        assert_eq!(bitmap.size, [(32.0 * scale) as u16; 2]);
        assert_eq!(bitmap.hotspot, [scale.round() as u16; 2]);
        assert!(bitmap.rgba.iter().all(|v| *v == 255));
    }
    assert!(rescale(&image, f32::INFINITY).is_none());
}
#[test]
fn roles_cover_every_tool_and_selection_intent_without_new_behavior() {
    assert_eq!(tool_asset(Tool::Move, None, false, false), 1059);
    assert_eq!(tool_asset(Tool::Move, None, false, true), 1004);
    assert_eq!(tool_asset(Tool::Zoom, None, true, false), 1007);
    assert_eq!(tool_asset(Tool::Hand, None, false, true), 25006);
    assert_eq!(tool_asset(Tool::VerticalType, None, false, false), 1474);
    assert_eq!(tool_asset(Tool::Dodge, None, true, false), tool_asset(Tool::Burn, None, false, false));
    assert_eq!(tool_asset(Tool::Blur, None, true, false), tool_asset(Tool::Sharpen, None, false, false));
    assert_eq!(tool_asset(Tool::CloneStamp, None, true, false), 1022);
    assert_eq!(tool_asset(Tool::Healing, None, true, false), 1022);
    assert_eq!(tool_asset(Tool::Count, None, true, false), 1612);
    for tool in [Tool::RectMarquee, Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso, Tool::MagicWand, Tool::ObjectSelection] {
        let base = tool_asset(tool, None, false, false);
        for (badge, delta) in [(Badge::Add, 1), (Badge::Subtract, 2), (Badge::Intersect, 3)] {
            assert_eq!(tool_asset(tool, Some(badge), false, false), base + delta);
        }
    }
    for tool in Tool::ALL {
        assert!(tool_asset(tool, None, false, false) >= 1000);
    }
}
#[test]
fn native_bitmap_cache_and_lifecycle_preserve_widget_priority() {
    let ctx = egui::Context::default();
    pack(&ctx);
    ctx.set_pixels_per_point(2.0);
    let pass = |ctx: &egui::Context, id: u32, override_cursor: bool| {
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            let icon = show(ctx, egui::pos2(50.0, 50.0), id, CursorIcon::Default).unwrap();
            ctx.set_cursor_icon(icon);
            if override_cursor {
                ctx.set_cursor_icon(CursorIcon::Text);
            }
        });
        out.textures_delta.clear();
        out
    };
    let first = pass(&ctx, 1059, false);
    assert_eq!(active(&ctx), Some(1059));
    assert_eq!(last_drawn(&ctx), Some(1059));
    let bitmap = first.platform_output.cursor_image.unwrap();
    assert_eq!(bitmap.size, if cfg!(target_os = "macos") { [32; 2] } else { [64; 2] });
    let again = pass(&ctx, 1059, false).platform_output.cursor_image.unwrap();
    assert!(Arc::ptr_eq(&bitmap.rgba, &again.rgba));
    let arrow = pass(&ctx, 1004, false);
    assert!(arrow.platform_output.cursor_image.is_some());
    assert_eq!(active(&ctx), Some(1004));
    let replaced = pass(&ctx, 1059, true);
    assert!(replaced.platform_output.cursor_image.is_none());
    assert_eq!(replaced.platform_output.cursor_icon, CursorIcon::Text);
    assert_eq!(active(&ctx), None);
    assert_eq!(last_drawn(&ctx), None);
    let mut left = ctx.run_ui(Default::default(), |_| {});
    left.textures_delta.clear();
    assert!(left.platform_output.cursor_image.is_none());
    assert_eq!(active(&ctx), None);
}

fn harness() -> egui_kittest::Harness<'static, crate::PrismApp> {
    let mut app = crate::PrismApp::new(prism_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width":200,"height":200})).unwrap();
    app.run("prefs.set", serde_json::json!({"path":"cursors.painting","value":"standard"})).unwrap();
    app.ui.tool = Tool::Move;
    app.sync_views();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 500.0)).build_ui_state(
        |ui, app: &mut crate::PrismApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    crate::PrismApp::setup_context(&h.ctx, Default::default());
    pack(&h.ctx);
    h.run_steps(4);
    h.event(egui::Event::PointerMoved(h.state().last_canvas_rect.center()));
    h.run_steps(3);
    h
}

#[test]
fn canvas_tools_precision_sampling_and_brush_tip_priority() {
    let mut h = harness();
    for tool in Tool::ALL {
        h.state_mut().ui.tool = tool;
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(h.state().last_canvas_rect.center()));
        h.run_steps(2);
        let expected = if tool == Tool::Crop { 1045 } else { tool_asset(tool, (tool == Tool::QuickSelection).then_some(Badge::Add), false, false) };
        assert_eq!(
            active(&h.ctx),
            Some(expected),
            "tool {tool:?}: output {:?}, position {:?}, rect {:?}, crop {:?}",
            h.output().platform_output.cursor_icon,
            h.ctx.pointer_hover_pos(),
            h.state().last_canvas_rect,
            h.state().ui.crop_rect
        );
    }
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().run("prefs.set", serde_json::json!({"path":"cursors.other","value":"precise"})).unwrap();
    h.run_steps(2);
    assert_eq!(active(&h.ctx), Some(1025));
    h.state_mut().ui.tool = Tool::Brush;
    h.state_mut().run("prefs.set", serde_json::json!({"path":"cursors.painting","value":"normalTip"})).unwrap();
    h.run_steps(2);
    assert_eq!(active(&h.ctx), None, "dynamic brush outline wins");
    h.state_mut().run("prefs.set", serde_json::json!({"path":"cursors.other","value":"standard"})).unwrap();
    h.event(egui::Event::ModifiersChanged(egui::Modifiers { alt: true, ..Default::default() }));
    h.run_steps(2);
    assert_eq!(active(&h.ctx), Some(1017), "Alt eyedropper wins over brush artwork");
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.state_mut().ui.tool = Tool::RectMarquee;
    h.state_mut().ui.selection_mode = 1;
    h.run_steps(2);
    assert_eq!(active(&h.ctx), Some(1451));
    h.event(egui::Event::PointerGone);
    h.run_steps(2);
    assert_eq!(active(&h.ctx), None);
}

#[test]
fn root_and_detached_requests_do_not_clear_each_other() {
    let ctx = egui::Context::default();
    pack(&ctx);
    let draw = |ui: &mut egui::Ui, asset| {
        let icon = show(ui.ctx(), egui::pos2(20.0, 20.0), asset, CursorIcon::Default).unwrap();
        ui.ctx().set_cursor_icon(icon);
    };
    let mut root = ctx.run_ui(Default::default(), |ui| draw(ui, 1059));
    root.textures_delta.clear();
    let child = egui::ViewportId::from_hash_of("detached-document");
    let mut raw = egui::RawInput { viewport_id: child, ..Default::default() };
    raw.viewports.insert(child, egui::ViewportInfo { parent: Some(egui::ViewportId::ROOT), ..Default::default() });
    let mut detached = ctx.run_ui(raw, |ui| draw(ui, 1004));
    detached.textures_delta.clear();
    assert!(root.platform_output.cursor_image.is_some());
    assert!(detached.platform_output.cursor_image.is_some());
    assert_eq!(ctx.data(|d| d.get_temp::<u32>(egui::Id::new(("prism-active-cursor", egui::ViewportId::ROOT)))), Some(1059));
    assert_eq!(ctx.data(|d| d.get_temp::<u32>(egui::Id::new(("prism-active-cursor", child)))), Some(1004));
    ctx.run_ui(Default::default(), |ui| {
        // Control requests run here, after begin-pass clears the new request and before paint.
        assert_eq!(active(ui.ctx()), None);
        assert_eq!(last_drawn(ui.ctx()), Some(1059));
    })
    .textures_delta
    .clear();
    assert_eq!(last_drawn(&ctx), None);
}

#[test]
fn painted_fallback_uses_points_and_clears_when_a_widget_wins() {
    let ctx = egui::Context::default();
    pack(&ctx);
    let previous = ctx.data(|d| d.get_temp::<Arc<Pack>>(egui::Id::new(PACK_KEY))).unwrap();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(PACK_KEY), Arc::new(Pack { images: previous.images.clone(), painter: true })));
    for scale in [1.0, 2.0] {
        ctx.set_pixels_per_point(scale);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let icon = show(ui.ctx(), egui::pos2(20.0, 20.0), 1059, CursorIcon::Default).unwrap();
            ui.ctx().set_cursor_icon(icon);
        });
        output.textures_delta.clear();
        let bounds = output.shapes.iter().find_map(|s| if let egui::Shape::Mesh(m) = &s.shape { Some(m.calc_bounds()) } else { None }).unwrap();
        assert_eq!(bounds.min, egui::pos2(19.0, 19.0));
        assert_eq!(bounds.size(), egui::vec2(32.0, 32.0));
        assert!(output.platform_output.cursor_image.is_none());
    }
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let icon = show(ui.ctx(), egui::pos2(20.0, 20.0), 1059, CursorIcon::Default).unwrap();
        ui.ctx().set_cursor_icon(icon);
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    });
    output.textures_delta.clear();
    assert_eq!(active(&ctx), None);
    assert!(!output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_))));
}

#[test]
fn color_range_sampling_uses_combined_add_subtract_and_precise_artwork() {
    let ctx = egui::Context::default();
    pack(&ctx);
    let mut app = crate::PrismApp::new(prism_engine::Session::new(), Default::default());
    crate::color_range_ui::open(&mut app);
    for (modifiers, resource) in [
        (egui::Modifiers::NONE, 1017),
        (egui::Modifiers { shift: true, ..Default::default() }, 1046),
        (egui::Modifiers { alt: true, ..Default::default() }, 1047),
    ] {
        ctx.run_ui(egui::RawInput { events: vec![egui::Event::ModifiersChanged(modifiers)], ..Default::default() }, |ui| {
            assert!(crate::color_range_ui::canvas_eyedropper(&mut app, ui.ctx(), egui::pos2(30.0, 30.0), None));
        })
        .textures_delta
        .clear();
        assert_eq!(last_drawn(&ctx), Some(resource));
    }
    app.run("prefs.set", serde_json::json!({"path":"cursors.other","value":"precise"})).unwrap();
    ctx.run_ui(egui::RawInput { events: vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)], ..Default::default() }, |ui| {
        assert!(crate::color_range_ui::canvas_eyedropper(&mut app, ui.ctx(), egui::pos2(30.0, 30.0), None));
    })
    .textures_delta
    .clear();
    assert_eq!(last_drawn(&ctx), Some(1025));
}
