use super::*;
#[test]
fn legacy_window_defaults_to_additional() {
    let w: DocWindow = serde_json::from_value(json!({"id":1,"document":0,"view":View::default(),"open":true})).unwrap();
    assert_eq!(w.kind, DocWindowKind::Additional);
}

fn app(count: usize) -> PrismApp {
    let mut session = prism_engine::Session::new();
    for i in 0..count {
        session.add_document(
            prism_doc::Document::with_background(
                format!("Document {i}"),
                prism_doc::Size::new(80, 60),
                prism_doc::ColorMode::Rgb,
                prism_doc::SampleType::U8,
                prism_color::Color::WHITE,
            ),
            Some(format!("doc{i}.prism")),
        );
    }
    PrismApp::new(session, Default::default())
}
fn call(a: &mut PrismApp, ctx: &Context, method: &str, p: Value) -> Value {
    let (req, _) = crate::control::ControlRequest::new(method, p);
    match crate::control::handle(a, ctx, &req) {
        crate::control::Outcome::Done(v) => v,
        _ => json!({"ok":true}),
    }
}

#[test]
fn detach_attach_keep_identity_pixels_path_history_and_camera() {
    let ctx = Context::default();
    let mut a = app(2);
    a.session.set_active(0);
    a.run("shape.create", json!({"kind":"rect","rect":[8,8,40,35],"fill":"#ff8000"})).unwrap();
    let original = a.session.active().unwrap().doc.clone();
    let history = a.session.active().unwrap().history.past_len();
    a.ui.views[0] = View { zoom: 2.5, center: [22.0, 17.0], rotation: 25.0, fit_pending: false, ..Default::default() };
    let camera = a.ui.views[0].clone();
    let window = detach(&mut a, &ctx, 0, None).unwrap()["window"].as_u64().unwrap();
    assert_eq!(docked(&a), vec![1]);
    assert_eq!(main_index(&a), Some(1));
    assert_eq!(a.ui.active_window, Some(window));
    assert_eq!(a.view(0), Some(&camera));
    assert!(std::sync::Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
    assert_eq!(a.session.active().unwrap().path.as_deref(), Some("doc0.prism"));
    crate::menus::invoke(&mut a, &ctx, "view.zoomIn", json!({})).unwrap();
    assert!(a.view(0).unwrap().zoom > camera.zoom);
    assert_eq!(a.ui.views[0], camera, "floating camera must not change the docked camera");
    a.run("shape.edit", json!({"fill":"#00ff00"})).unwrap();
    assert_eq!(a.session.active().unwrap().history.past_len(), history + 1);
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(a.session.active().unwrap().doc.layers, original.layers);
    a.run("edit.redo", json!({})).unwrap();
    let edited = a.session.active().unwrap().doc.clone();
    let floating_camera = a.view(0).unwrap().clone();
    attach(&mut a, &ctx, window, None).unwrap();
    assert!(a.ui.windows.is_empty());
    assert_eq!(docked(&a), vec![0, 1]);
    assert_eq!(a.ui.views[a.session.active_index().unwrap()], floating_camera);
    assert!(std::sync::Arc::ptr_eq(&edited, &a.session.active().unwrap().doc));
}

#[test]
fn detached_windows_remap_by_document_identity_after_moves_and_closes() {
    let ctx = Context::default();
    let mut a = app(3);
    let id = a.session.documents()[1].doc.id;
    let window = detach(&mut a, &ctx, 1, None).unwrap()["window"].as_u64().unwrap();
    a.run("document.move", json!({"document":1,"to":3})).unwrap();
    assert_eq!(a.ui.windows[0].document, 2);
    assert_eq!(a.session.documents()[2].doc.id, id);
    a.run("file.close", json!({"document":0})).unwrap();
    assert_eq!(a.ui.windows[0].document, 1);
    assert_eq!(a.session.documents()[1].doc.id, id);
    let before = a.session.documents()[0].doc.id;
    attach(&mut a, &ctx, window, Some(before)).unwrap();
    assert_eq!(a.session.documents()[0].doc.id, id);
    assert_eq!(a.ui.main_document, Some(id));
}

#[test]
fn floating_all_is_idempotent_and_consolidation_keeps_each_camera() {
    let ctx = Context::default();
    let mut a = app(3);
    crate::menus::invoke(&mut a, &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    assert!(docked(&a).is_empty());
    assert_eq!(main_index(&a), None);
    crate::menus::invoke(&mut a, &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    assert_eq!(a.ui.windows.len(), 3);
    for w in &mut a.ui.windows {
        w.view.zoom = w.document as f32 + 2.0;
        w.view.rotation = w.document as f32 * 15.0;
    }
    crate::menus::invoke(&mut a, &ctx, "window.arrange.consolidateAllToTabs", json!({})).unwrap();
    assert_eq!(docked(&a), vec![0, 1, 2]);
    assert_eq!(a.ui.views[1].zoom, 3.0);
    assert_eq!(a.ui.views[2].rotation, 30.0);
}

#[test]
fn detached_close_is_guarded_but_an_additional_view_closes_without_discard() {
    let ctx = Context::default();
    let mut a = app(1);
    a.run("layer.new.layer", json!({})).unwrap();
    let window = detach(&mut a, &ctx, 0, None).unwrap()["window"].as_u64().unwrap();
    close(&mut a, &ctx, window).unwrap();
    assert!(a.discard.is_some());
    assert_eq!(a.ui.windows.len(), 1);
    assert_eq!(a.session.documents().len(), 1);
    a.discard = None; // the same cancellation path as Cancel in the prompt
    let copy = additional(&mut a, 0).unwrap()["window"].as_u64().unwrap();
    close(&mut a, &ctx, copy).unwrap();
    assert!(a.discard.is_none());
    assert_eq!(a.ui.windows.len(), 1);
    a.session.active_mut().unwrap().saved_revision = a.session.active().unwrap().revision;
    close(&mut a, &ctx, window).unwrap();
    assert!(a.session.documents().is_empty());
    assert!(a.ui.windows.is_empty());
}

#[test]
fn invalid_window_control_is_atomic_and_duplicate_detach_reuses_window() {
    let ctx = Context::default();
    let mut a = app(1);
    for (method, params) in [
        ("ui.window.detach", json!({"document":99})),
        ("ui.window.detach", json!({"document":"bad"})),
        ("ui.window.attach", json!({"window":99})),
        ("ui.window.focus", json!({"window":99})),
        ("ui.key", json!({"window":99,"key":"V"})),
    ] {
        assert_eq!(call(&mut a, &ctx, method, params)["ok"], false);
        assert!(a.ui.windows.is_empty());
        assert!(a.synthetic.is_empty());
    }
    let first = call(&mut a, &ctx, "ui.window.detach", json!({}));
    let second = call(&mut a, &ctx, "ui.window.detach", json!({}));
    assert_eq!(first["result"]["window"], second["result"]["window"]);
    assert_eq!(a.ui.windows.len(), 1);
    let state = crate::control::inspect(&a, &ctx);
    assert_eq!(state["dockedDocuments"], json!([]));
    assert_eq!(state["windows"][0]["kind"], "detached");
}

/// Feed separate native viewport passes in the headless harness, using the same input hook and
/// immediate callback as eframe. This catches focus, keyboard routing and camera ownership bugs.
fn native_harness(a: PrismApp) -> egui_kittest::Harness<'static, PrismApp> {
    Context::set_immediate_viewport_renderer(|ctx, mut v| {
        let mut raw = egui::RawInput {
            viewport_id: v.ids.this,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            time: Some(ctx.input_for(v.ids.this, |i| i.time) + 1.0 / 60.0),
            ..Default::default()
        };
        raw.viewports = ctx.input(|i| i.raw.viewports.clone());
        raw.viewports.insert(
            v.ids.this,
            egui::ViewportInfo {
                parent: Some(v.ids.parent),
                native_pixels_per_point: Some(1.0),
                inner_rect: Some(Rect::from_min_size(egui::pos2(200.0, 200.0), egui::vec2(800.0, 600.0))),
                focused: Some(true),
                ..Default::default()
            },
        );
        let mut output = ctx.run_ui(raw, |ui| (v.viewport_ui_cb)(ui));
        output.textures_delta.clear();
    });
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 850.0)).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::Pro);
        cc.egui_ctx.set_embed_viewports(false);
        a
    });
    let root = h.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap();
    root.inner_rect = Some(Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(1200.0, 850.0)));
    h.run_steps(6);
    h
}

#[test]
fn native_child_keyboard_edits_and_navigation_target_only_the_focused_document() {
    let mut a = app(2);
    let doc0 = a.session.documents()[0].doc.clone();
    let window = detach(&mut a, &Context::default(), 1, None).unwrap()["window"].as_u64().unwrap();
    let mut h = native_harness(a);
    let ctx = h.ctx.clone();
    call(h.state_mut(), &ctx, "ui.key", json!({"window":window,"key":"U"}));
    h.run_steps(6);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Rectangle);
    let main_camera = h.state().ui.views[1].clone();
    let zoom_before = h.state().view(1).unwrap().zoom;
    call(h.state_mut(), &ctx, "ui.key", json!({"window":window,"key":"Plus","command":true}));
    h.run_steps(6);
    assert!(h.state().view(1).unwrap().zoom > zoom_before);
    assert_eq!(h.state().ui.views[1], main_camera);
    call(
        h.state_mut(),
        &ctx,
        "ui.pointer",
        json!({"window":window,"events":[{"kind":"down","x":10,"y":10},{"kind":"move","x":40,"y":35},{"kind":"up","x":40,"y":35}]}),
    );
    h.run_steps(3);
    assert!(matches!(h.state().session.active().unwrap().doc.layers.last().unwrap().content, prism_doc::LayerContent::Shape(_)));
    let history = h.state().session.active().unwrap().history.past_len();
    assert_eq!(history, 1);
    call(h.state_mut(), &ctx, "ui.key", json!({"window":window,"key":"Z","command":true}));
    h.run_steps(6);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    assert!(std::sync::Arc::ptr_eq(&doc0, &h.state().session.documents()[0].doc));
}

#[test]
fn a_dialog_opened_for_a_detached_document_keeps_its_window_host() {
    let ctx = Context::default();
    let mut a = app(1);
    let window = detach(&mut a, &ctx, 0, None).unwrap()["window"].as_u64().unwrap();
    let dialog = crate::color_picker_ui::open(&mut a, "foreground");
    assert_eq!(a.ui.dialog_mut(dialog).unwrap().window, Some(window));
    assert!(!dialogs_here(&a, &ctx));
    // It floats in an OS window of its own (not clipped to the document window), above the
    // documents, and automation can address it by id.
    let d = a.ui.dialog_mut(dialog).unwrap().clone();
    assert!(crate::dialogs::floats(&d) && crate::native_dialogs::own_window(&d));
    assert_eq!(window_level(&a, &ctx), egui::viewport::WindowLevel::Normal);
    assert!(crate::native_dialogs::focus(&a, &ctx, dialog).is_ok());
    a.ui.close_dialog(dialog);
    assert!(a.ui.dialogs.is_empty());
}

#[test]
fn native_tab_drag_detaches_then_cross_window_drag_attaches_at_the_root_strip() {
    let mut h = native_harness(app(2));
    let (document, tab) = h.state().tab_strip.as_ref().unwrap().tabs.last().copied().unwrap();
    let id = h.state().session.documents()[document].doc.id;
    let start = tab.center();
    let end = start + egui::vec2(0.0, 100.0);
    let ctx = h.ctx.clone();
    call(h.state_mut(), &ctx, "ui.drag", json!({"from":[start.x,start.y],"to":[end.x,end.y]}));
    loop {
        let events = h.state_mut().take_synthetic_step();
        if events.is_empty() {
            break;
        }
        for e in events {
            h.event(e);
        }
        h.step();
    }
    h.run_steps(4);
    assert_eq!(h.state().ui.windows.len(), 1);
    let window = h.state().ui.windows[0].id;
    assert_eq!(h.state().session.documents()[h.state().ui.windows[0].document].doc.id, id);
    let root_tab = h.state().tab_strip.as_ref().unwrap().rect;
    // Child's client origin is (200,200), root's is (100,100), at 100% DPI.
    let target = root_tab.left_top() + egui::vec2(10.0, 13.0) - egui::vec2(100.0, 100.0);
    call(h.state_mut(), &ctx, "ui.drag", json!({"window":window,"from":[50,45],"to":[target.x,target.y]}));
    h.run_steps(8);
    assert!(h.state().ui.windows.is_empty());
    assert_eq!(h.state().ui.active_window, None);
    assert_eq!(h.state().session.documents()[0].doc.id, id, "drop before the remaining document");
}

#[test]
fn escape_cancels_a_native_tab_drag_without_a_document_or_history_change() {
    let mut h = native_harness(app(1));
    let start = h.state().tab_strip.as_ref().unwrap().tabs[0].1.center();
    let snapshot = h.state().session.active().unwrap().doc.clone();
    h.event(egui::Event::PointerMoved(start));
    h.step();
    h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    let end = start + egui::vec2(0.0, 100.0);
    h.event(egui::Event::PointerMoved(end));
    h.step();
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.step();
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    assert!(h.state().ui.windows.is_empty());
    assert!(std::sync::Arc::ptr_eq(&snapshot, &h.state().session.active().unwrap().doc));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
}

#[test]
fn native_detached_close_save_discard_cancel_and_write_failure() {
    for (key, fails, retained) in [("Escape", false, true), ("D", false, false), ("S", false, false), ("S", true, true)] {
        let mut a = app(1);
        a.services.application_active = Some(Box::new(|| true));
        a.run("layer.new.layer", json!({})).unwrap();
        a.services.export = Some(Box::new(|_, _, _| Ok((vec![1], Vec::new()))));
        a.services.write = Some(Box::new(move |_, _| if fails { Err("disk full".into()) } else { Ok(()) }));
        let window = detach(&mut a, &Context::default(), 0, None).unwrap()["window"].as_u64().unwrap();
        let mut h = native_harness(a);
        h.ctx.set_os(egui::os::OperatingSystem::Mac);
        let ctx = h.ctx.clone();
        call(h.state_mut(), &ctx, "ui.window.close", json!({"window":window}));
        h.run_steps(3);
        assert!(h.state().discard.is_some());
        // The prompt floats above the documents, which stay on top of the editor.
        assert_eq!(window_level(h.state(), &ctx), egui::viewport::WindowLevel::AlwaysOnTop);
        call(h.state_mut(), &ctx, "ui.key", json!({"window":window,"key":key}));
        h.run_steps(8);
        assert_eq!(!h.state().ui.windows.is_empty(), retained, "{key}, failure={fails}");
        assert_eq!(!h.state().session.documents().is_empty(), retained);
        if fails {
            assert!(h.state().discard.is_some());
            assert!(h.state().ui.status.contains("disk full"));
            assert_eq!(window_level(h.state(), &ctx), egui::viewport::WindowLevel::AlwaysOnTop);
        } else if retained {
            assert_eq!(window_level(h.state(), &ctx), egui::viewport::WindowLevel::AlwaysOnTop);
        }
    }
}

#[test]
fn returning_before_a_later_document_uses_the_insertion_slot() {
    let ctx = Context::default();
    let mut a = app(3);
    let source = a.session.documents()[0].doc.id;
    let target = a.session.documents()[2].doc.id;
    let window = detach(&mut a, &ctx, 0, None).unwrap()["window"].as_u64().unwrap();
    attach(&mut a, &ctx, window, Some(target)).unwrap();
    assert_eq!(a.session.documents()[1].doc.id, source);
    assert_eq!(a.session.documents()[2].doc.id, target);
}

#[test]
fn queued_native_input_wins_over_stale_focus_of_another_child() {
    let mut a = app(2);
    let first = detach(&mut a, &Context::default(), 0, None).unwrap()["window"].as_u64().unwrap();
    let _second = detach(&mut a, &Context::default(), 1, None).unwrap()["window"].as_u64().unwrap();
    let mut h = native_harness(a);
    let ctx = h.ctx.clone();
    call(h.state_mut(), &ctx, "ui.key", json!({"window":first,"key":"U"}));
    h.run_steps(3);
    call(h.state_mut(), &ctx, "ui.drag", json!({"window":first,"from":[300,150],"to":[420,250]}));
    h.run_steps(6);
    assert_eq!(h.state().session.documents()[0].history.past_len(), 1);
    assert_eq!(h.state().session.documents()[1].history.past_len(), 0);
    call(h.state_mut(), &ctx, "ui.key", json!({"window":first,"key":"Z","command":true}));
    h.run_steps(3);
    assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
}

#[test]
fn floating_window_level_tracks_application_activation_instead_of_panel_focus() {
    use std::{cell::Cell, rc::Rc};
    let active = Rc::new(Cell::new(true));
    let state = active.clone();
    let mut a = app(1);
    a.services.application_active = Some(Box::new(move || state.get()));
    let ctx = Context::default();
    for (foreground, want) in
        [(true, egui::viewport::WindowLevel::AlwaysOnTop), (false, egui::viewport::WindowLevel::Normal), (true, egui::viewport::WindowLevel::AlwaysOnTop)]
    {
        active.set(foreground);
        assert_eq!(window_level(&a, &ctx), want);
        let mut output = ctx.run_ui(Default::default(), |ui| sync_window_level(&a, ui.ctx()));
        assert!(output.viewport_output[&ViewportId::ROOT].commands.iter().any(|c| matches!(c,egui::ViewportCommand::WindowLevel(l) if *l==want)));
        output.textures_delta.clear();
    }
}

/// Windows: focus moving between Prism's own windows (a layer dropped on another floating
/// document) leaves none focused for a turn. The floating windows keep their level through that
/// gap: dropping it and raising it again put the source window back above the destination.
#[test]
fn focus_passing_between_prism_windows_keeps_the_floating_level() {
    let a = app(1);
    let ctx = Context::default();
    let mut level_commands = Vec::new();
    for (time, focused, want) in [
        (0.0, true, egui::viewport::WindowLevel::AlwaysOnTop),
        // Between the source losing focus and the destination taking it.
        (0.05, false, egui::viewport::WindowLevel::AlwaysOnTop),
        (0.1, true, egui::viewport::WindowLevel::AlwaysOnTop),
        // Another application took the focus.
        (0.2, false, egui::viewport::WindowLevel::AlwaysOnTop),
        (0.8, false, egui::viewport::WindowLevel::Normal),
    ] {
        let mut raw = egui::RawInput { time: Some(time), ..Default::default() };
        raw.viewports.insert(ViewportId::ROOT, egui::ViewportInfo { focused: Some(focused), ..Default::default() });
        let mut output = ctx.run_ui(raw, |ui| {
            assert_eq!(window_level(&a, ui.ctx()), want, "at {time}s");
            sync_window_level(&a, ui.ctx());
        });
        level_commands.extend(output.viewport_output[&ViewportId::ROOT].commands.iter().filter(|c| matches!(c, egui::ViewportCommand::WindowLevel(_))).cloned());
        output.textures_delta.clear();
    }
    assert_eq!(
        level_commands,
        [egui::ViewportCommand::WindowLevel(egui::viewport::WindowLevel::AlwaysOnTop), egui::ViewportCommand::WindowLevel(egui::viewport::WindowLevel::Normal)],
        "set once, lowered only after leaving Prism"
    );
}

/// The main window's File menu dropped over a floating document window: the main window goes on
/// top while the menu is open (its menus are drawn inside it), and back once it closes. A menu
/// that misses every floating window changes nothing.
#[test]
fn a_main_menu_over_a_floating_window_raises_the_main_window_while_open() {
    let ctx = Context::default();
    // Floating windows are OS windows, as in the desktop app.
    ctx.set_embed_viewports(false);
    let mut a = app(1);
    let window = detach(&mut a, &ctx, 0, None).unwrap()["window"].as_u64().unwrap();
    let levels = |output: &egui::FullOutput| -> Vec<egui::viewport::WindowLevel> {
        output.viewport_output[&ViewportId::ROOT]
            .commands
            .iter()
            .filter_map(|c| if let egui::ViewportCommand::WindowLevel(l) = c { Some(*l) } else { None })
            .collect()
    };
    let pass = |open: bool, floating: Rect| {
        ctx.data_mut(|d| d.insert_temp(frames_key(), Frames(vec![(window, floating)])));
        let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
        raw.viewports.insert(
            ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(1.0),
                inner_rect: Some(Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(800.0, 600.0))),
                ..Default::default()
            },
        );
        let mut output = ctx.run_ui(raw, |ui| {
            let file = ui.button("File");
            let id = egui::Popup::default_response_id(&file);
            if open && !egui::Popup::is_id_open(ui.ctx(), id) {
                egui::Popup::open_id(ui.ctx(), id);
            } else if !open {
                egui::Popup::close_all(ui.ctx());
            }
            egui::Popup::menu(&file).show(|ui| {
                for item in ["New…", "Open…", "Close", "Save", "Export", "Print…", "Exit"] {
                    let _ = ui.button(item);
                }
            });
            raise_main_over_popups(&a, ui.ctx());
        });
        output.textures_delta.clear();
        levels(&output)
    };
    // The menu opens at the main window's top left (100, 100 on the desktop).
    let under_menu = Rect::from_min_size(egui::pos2(120.0, 150.0), egui::vec2(400.0, 300.0));
    let elsewhere = Rect::from_min_size(egui::pos2(700.0, 500.0), egui::vec2(150.0, 100.0));
    let mut sent = Vec::new();
    for _ in 0..3 {
        sent.extend(pass(true, elsewhere));
    }
    assert!(sent.is_empty(), "a menu clear of the floating window raises nothing: {sent:?}");
    for _ in 0..3 {
        sent.extend(pass(true, under_menu));
    }
    assert_eq!(sent, [egui::viewport::WindowLevel::AlwaysOnTop]);
    for _ in 0..2 {
        sent.extend(pass(false, under_menu));
    }
    assert_eq!(sent, [egui::viewport::WindowLevel::AlwaysOnTop, egui::viewport::WindowLevel::Normal], "back below once the menu closes");
}

#[test]
fn quit_prompt_floats_above_detached_canvases_and_cancel_keeps_them() {
    let mut a = app(1);
    a.services.application_active = Some(Box::new(|| true));
    a.run("layer.new.layer", json!({})).unwrap();
    let window = detach(&mut a, &Context::default(), 0, None).unwrap()["window"].as_u64().unwrap();
    let mut h = native_harness(a);
    h.ctx.set_os(egui::os::OperatingSystem::Mac);
    let ctx = h.ctx.clone();
    crate::discard_ui::intercept(h.state_mut(), "file.exit", &serde_json::Value::Null);
    h.run_steps(3);
    let out = h.output();
    // Its own always-on-top window; the documents stay visible on top of the editor under it.
    let prompt = &out.viewport_output[&crate::discard_ui::viewport()];
    assert_eq!(prompt.builder.window_level, Some(egui::viewport::WindowLevel::AlwaysOnTop));
    assert!(prompt.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Focus)), "the prompt takes focus");
    assert_eq!(out.viewport_output[&viewport(Some(window))].builder.window_level, Some(egui::viewport::WindowLevel::AlwaysOnTop));
    assert!(!crate::discard_ui::here(h.state(), &ctx), "every Prism window is blocked behind it");
    let document = h.state().session.active().unwrap().doc.clone();
    call(h.state_mut(), &ctx, "ui.key", json!({"key":"Escape"}));
    h.run_steps(6);
    assert!(h.state().discard.is_none());
    assert!(!h.state().allow_close);
    assert_eq!(h.state().ui.windows.len(), 1);
    assert!(std::sync::Arc::ptr_eq(&document, &h.state().session.active().unwrap().doc));
    assert_eq!(window_level(h.state(), &ctx), egui::viewport::WindowLevel::AlwaysOnTop);
}

#[test]
fn detaching_preview_hides_pixels_without_changing_camera_and_escape_restores_them() {
    let mut h = native_harness(app(1));
    let start = h.state().tab_strip.as_ref().unwrap().tabs[0].1.center();
    let id = h.state().session.active().unwrap().doc.id;
    let camera = h.state().ui.views[0].clone();
    let document = h.state().session.active().unwrap().doc.clone();
    h.event(egui::Event::PointerMoved(start));
    h.step();
    h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(egui::Event::PointerMoved(start + egui::vec2(90.0, 80.0)));
    h.step();
    assert!(hides_canvas(&h.ctx, id));
    assert_eq!(h.state().ui.views[0], camera);
    let gray = crate::theme::Tokens::get(&h.ctx).docking_placeholder;
    assert!(h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == gray && r.rect.height() > 200.0)));
    assert!(h.state().ui.windows.is_empty(), "native viewport must wait for release");
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.step();
    assert!(!hides_canvas(&h.ctx, id));
    assert!(!dragging(&h.ctx));
    assert!(std::sync::Arc::ptr_eq(&document, &h.state().session.active().unwrap().doc));
}

#[test]
fn native_titlebar_drag_docks_at_the_same_slot_as_the_document_tab() {
    let mut a = app(2);
    let id = a.session.documents()[1].doc.id;
    let window = detach(&mut a, &Context::default(), 1, None).unwrap()["window"].as_u64().unwrap();
    let mut h = native_harness(a);
    let camera = h.state().ui.windows[0].view.clone();
    let root_tab = h.state().tab_strip.as_ref().unwrap().rect;
    let target = root_tab.left_top() + egui::vec2(10.0, 13.0) - egui::vec2(100.0, 100.0);
    let ctx = h.ctx.clone();
    call(h.state_mut(), &ctx, "ui.drag", json!({"window":window,"from":[300,16],"to":[target.x,target.y]}));
    h.run_steps(8);
    assert!(h.state().ui.windows.is_empty());
    assert_eq!(h.state().session.documents()[0].doc.id, id);
    assert_eq!(h.state().ui.views[0], camera);
}

#[test]
fn docking_border_opacity_and_drop_agree_for_every_theme_and_dpi_even_with_no_tabs() {
    for theme in crate::theme::ThemeKind::ALL {
        for scale in [1.0, 2.0] {
            let ctx = Context::default();
            crate::theme::apply(&ctx, theme);
            ctx.set_pixels_per_point(scale);
            let mut a = app(1);
            let opacity = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let calls = opacity.clone();
            a.services.window_opacity = Some(Box::new(move |_, alpha| {
                calls.lock().unwrap().push(alpha);
                true
            }));
            let document = a.session.active().unwrap().doc.id;
            let workspace = Rect::from_min_size(egui::pos2(40.0, 60.0), egui::vec2(700.0, 500.0));
            let strip = Rect::from_min_size(workspace.min, egui::vec2(700.0, 26.0));
            let target = DropStrip { rect: strip * scale, tabs: Vec::new(), scale };
            let mut drag = TabDrag {
                document,
                window: Some(1),
                strip: strip * scale,
                pointer: strip.center() * scale,
                released: false,
                scale,
                start: Pos2::ZERO,
                window_origin: Some(egui::pos2(200.0, 200.0) * scale),
                automated: false,
            };
            assert!(can_attach(&drag, &target));
            ctx.data_mut(|d| {
                d.insert_temp(strip_key(), target.clone());
                d.insert_temp(drag_key(), drag.clone());
            });
            let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))), ..Default::default() };
            raw.viewports.get_mut(&ViewportId::ROOT).unwrap().inner_rect = Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0)));
            let mut out = ctx.run_ui(raw.clone(), |_| {
                drag_indicator(&a, &ctx, workspace);
                fade_window(&mut a, &ctx, 1);
            });
            out.textures_delta.clear();
            let t = crate::theme::Tokens::for_kind(theme);
            assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == workspace && r.stroke.color == t.docking_outline)));
            assert_eq!(*opacity.lock().unwrap(), [0.55]);
            assert_eq!(t.docking_placeholder, t.chrome);
            drag.pointer.y += 40.0 * scale;
            assert!(!can_attach(&drag, &target));
            ctx.data_mut(|d| d.insert_temp(drag_key(), drag));
            raw.viewport_id = viewport(Some(1));
            raw.viewports.insert(raw.viewport_id, Default::default());
            let mut out = ctx.run_ui(raw, |_| {
                drag_indicator(&a, &ctx, workspace);
                fade_window(&mut a, &ctx, 1);
                cancel_drag(&ctx);
            });
            out.textures_delta.clear();
            assert!(!out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.color == t.docking_outline)));
            assert!(!dragging(&ctx));
            fade_window(&mut a, &ctx, 1);
            assert_eq!(*opacity.lock().unwrap(), [0.55, 1.0]);
            assert!(
                out.viewport_output
                    .get(&viewport(Some(1)))
                    .unwrap()
                    .commands
                    .iter()
                    .any(|c| matches!(c, egui::ViewportCommand::OuterPosition(p) if *p == egui::pos2(200.0,200.0)))
            );
        }
    }
}

#[test]
fn phased_drag_can_capture_docking_before_release_and_cancel_keeps_the_window() {
    let mut a = app(2);
    let window = detach(&mut a, &Context::default(), 1, None).unwrap()["window"].as_u64().unwrap();
    let mut h = native_harness(a);
    let ctx = h.ctx.clone();
    let root_tab = h.state().tab_strip.as_ref().unwrap().rect;
    let target = root_tab.left_top() + egui::vec2(10.0, 13.0) - egui::vec2(100.0, 100.0);
    call(h.state_mut(), &ctx, "ui.drag", json!({"window":window,"phase":"start","from":[300,16],"to":[target.x,target.y]}));
    h.run_steps(5);
    assert_eq!(drag_inspect(&ctx)["phase"], "docking");
    h.event(egui::Event::PointerMoved(egui::pos2(600.0, 500.0)));
    h.step();
    assert_eq!(drag_inspect(&ctx)["phase"], "docking", "native pointer updates must not replace a held synthetic drag");
    assert_eq!(h.state().ui.windows.len(), 1);
    let blue = crate::theme::Tokens::get(&ctx).docking_outline;
    assert!(h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.color == blue && r.rect.height() > 200.0)));
    call(h.state_mut(), &ctx, "ui.key", json!({"window":window,"key":"Escape"}));
    h.run_steps(4);
    assert_eq!(drag_inspect(&ctx), Value::Null);
    call(h.state_mut(), &ctx, "ui.drag", json!({"window":window,"phase":"end","from":[target.x,target.y],"to":[target.x,target.y]}));
    h.run_steps(4);
    assert_eq!(h.state().ui.windows.len(), 1);
    let before = h.state().synthetic.len();
    assert_eq!(call(h.state_mut(), &ctx, "ui.drag", json!({"phase":"invalid","from":[1,2],"to":[3,4]}))["ok"], false);
    assert_eq!(h.state().synthetic.len(), before);
}

/// A floating window dragged by its tab follows the OS pointer, not the pointer events: those
/// lag the window's own move, and placing them with its new position made the window shake.
#[test]
fn a_dragged_window_follows_the_os_pointer() {
    let a = app(1);
    let document = a.session.active().unwrap().doc.id;
    let strip = Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 26.0));
    let drag = |automated| TabDrag {
        document,
        window: Some(1),
        strip,
        // A stale event: read with the moved window's origin, the pointer seems to jump back.
        pointer: egui::pos2(40.0, 10.0),
        released: false,
        scale: 2.0,
        start: egui::pos2(100.0, 20.0),
        window_origin: Some(egui::pos2(400.0, 300.0)),
        automated,
    };
    let moved_to = |automated, screen| {
        let ctx = Context::default();
        ctx.data_mut(|d| d.insert_temp(drag_key(), drag(automated)));
        let mut raw = egui::RawInput { viewport_id: viewport(Some(1)), ..Default::default() };
        raw.viewports.insert(raw.viewport_id, Default::default());
        let mut out = ctx.run_ui(raw, |ui| move_window(ui.ctx(), 1, screen));
        out.textures_delta.clear();
        out.viewport_output.get(&viewport(Some(1))).and_then(|v| {
            v.commands.iter().find_map(|c| if let egui::ViewportCommand::OuterPosition(p) = c { Some(*p) } else { None })
        })
    };
    // The OS pointer 60 px right and 30 px down of the grab: the window moves by exactly that.
    assert_eq!(moved_to(false, Some(egui::pos2(160.0, 50.0))), Some(egui::pos2(230.0, 165.0)));
    // Without one (or under automation) the recorded pointer still drives it.
    assert_eq!(moved_to(false, None), Some(egui::pos2(170.0, 145.0)));
    assert_eq!(moved_to(true, Some(egui::pos2(160.0, 50.0))), Some(egui::pos2(170.0, 145.0)));
    // Non-finite readings are ignored.
    assert_eq!(moved_to(false, Some(egui::pos2(f32::NAN, 0.0))), Some(egui::pos2(170.0, 145.0)));
}

/// A floating window's recorded frame places it when it opens, and never again: fed back every
/// frame, Windows' late moves bounced it between two spots forever.
#[test]
fn an_open_window_is_not_moved_to_its_recorded_frame() {
    let mut window = DocWindow { size: Some([800.0, 600.0]), id: 3, document: 0, view: View::default(), open: true, kind: DocWindowKind::Detached, position: Some([585.0, 534.0]) };
    let built = |window: &DocWindow, open: bool| {
        let ctx = Context::default();
        let mut raw = egui::RawInput::default();
        if open {
            raw.viewports.insert(viewport(Some(window.id)), Default::default());
        }
        let mut builder = None;
        let mut out = ctx.run_ui(raw, |ui| builder = Some(placement(ui.ctx(), window, egui::ViewportBuilder::default())));
        out.textures_delta.clear();
        builder.unwrap()
    };
    let mut opened = built(&window, false);
    assert_eq!((opened.position, opened.inner_size), (Some(egui::pos2(585.0, 534.0)), Some(egui::vec2(800.0, 600.0))));
    // The OS has it a few pixels away (a drag, or a move still on its way): nothing is sent back.
    window.position = Some([587.0, 531.0]);
    window.size = Some([800.0, 599.99994]);
    let (commands, recreate) = opened.patch(built(&window, true));
    assert!(commands.is_empty() && !recreate, "{commands:?}");
    // Reopened after its viewport was dropped, it comes back where it was last seen.
    assert_eq!(built(&window, false).position, Some(egui::pos2(587.0, 531.0)));
}

/// A system file panel (the Save window's Other Location…) must not open under Prism's
/// always-on-top windows: they drop to ordinary stacking while it is up, and it opens on the
/// window that asked for it.
#[test]
fn a_file_panel_is_not_covered_by_floating_windows() {
    let mut a = app(1);
    a.services.application_active = Some(Box::new(|| true));
    let ctx = Context::default();
    assert_eq!(window_level(&a, &ctx), egui::viewport::WindowLevel::AlwaysOnTop);
    a.services.file_dialog = Some(crate::file_dialog::fake(vec![None]).0);
    a.pick_location("/x/a.png", |_, _| Ok(Value::Null)).unwrap();
    a.file_dialog_on(crate::native_dialogs::viewport(7));
    assert_eq!(window_level(&a, &ctx), egui::viewport::WindowLevel::Normal);
    // Answered (cancelled here): back on top.
    let mut out = ctx.run_ui(Default::default(), |ui| a.poll_file_dialog(ui.ctx(), None));
    out.textures_delta.clear();
    assert!(!a.file_dialog_open());
    assert_eq!(window_level(&a, &ctx), egui::viewport::WindowLevel::AlwaysOnTop);
}
