//! Presets use the live box and its history, never the engine's immediate pixel path.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

const PRESETS: [&str; 5] =
    ["edit.transform.rotate180", "edit.transform.rotate90Cw", "edit.transform.rotate90Ccw", "edit.transform.flipHorizontal", "edit.transform.flipVertical"];

fn app(fmt: PixelFormat) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("asymmetric pixels", |doc, layer| {
            doc.mode = fmt.mode;
            doc.depth = fmt.sample;
            let mut s = Surface::new(fmt);
            let mut pixel = vec![0.25; fmt.channels()];
            *pixel.last_mut().unwrap() = 1.0;
            s.fill_rect(Rect::new(10, 20, 30, 28), &pixel);
            pixel[0] = 0.75;
            s.fill_rect(Rect::new(10, 20, 12, 22), &pixel);
            doc.layer_mut(layer.unwrap()).unwrap().content = LayerContent::Raster(s);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app
}

fn rgba() -> PhotocraftApp {
    app(PixelFormat::RGBA8)
}

#[test]
fn presets_compose_after_move_scale_and_skew_about_a_custom_pivot() {
    let ctx = egui::Context::default();
    let expected = [
        [[10.0, 28.0], [-30.0, 28.0], [-38.0, 12.0], [2.0, 12.0]],
        [[28.0, 30.0], [28.0, 70.0], [12.0, 78.0], [12.0, 38.0]],
        [[12.0, 10.0], [12.0, -30.0], [28.0, -38.0], [28.0, 2.0]],
        [[10.0, 12.0], [-30.0, 12.0], [-38.0, 28.0], [2.0, 28.0]],
        [[30.0, 28.0], [70.0, 28.0], [78.0, 12.0], [38.0, 12.0]],
    ];
    for (id, expected) in PRESETS.into_iter().zip(expected) {
        let mut app = rgba();
        begin(&mut app, &ctx).unwrap();
        let t = app.ui.transform.as_mut().unwrap();
        t.quad = [[30.0, 12.0], [70.0, 12.0], [78.0, 28.0], [38.0, 28.0]];
        t.pivot = [20.0, 20.0];
        t.mode = TransformMode::Skew;
        let before = t.clone();
        let doc = app.session.active().unwrap().doc.clone();
        crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
        let t = app.ui.transform.as_ref().unwrap();
        assert_eq!(t.quad, expected, "{id}");
        let mut unchanged = t.clone();
        unchanged.quad = before.quad;
        assert_eq!(serde_json::to_value(unchanged).unwrap(), serde_json::to_value(before).unwrap());
        assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc), "no resampling");
    }
}

#[test]
fn inverse_presets_restore_the_box_and_each_click_is_one_history_step() {
    for sequence in [vec![PRESETS[1]; 4], vec![PRESETS[3]; 2], vec![PRESETS[4]; 2], vec![PRESETS[0]; 2], vec![PRESETS[1], PRESETS[2]]] {
        let mut app = rgba();
        begin(&mut app, &egui::Context::default()).unwrap();
        let initial = app.ui.transform.as_ref().unwrap().quad;
        let mut states = vec![initial];
        for id in &sequence {
            app.run(id, json!({})).unwrap();
            states.push(app.ui.transform.as_ref().unwrap().quad);
        }
        assert_eq!(states.last().unwrap(), &initial);
        for state in states.iter().rev().skip(1) {
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(&app.ui.transform.as_ref().unwrap().quad, state);
        }
        for state in states.iter().skip(1) {
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(&app.ui.transform.as_ref().unwrap().quad, state);
        }
        let before = app.session.active().unwrap().history.past_len();
        commit(&mut app);
        assert_eq!(app.session.active().unwrap().history.past_len(), before, "net identity is not a document edit");
    }
}

#[test]
fn preset_commit_preserves_formats_and_asymmetric_pixel_direction() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for (id, point) in PRESETS.into_iter().zip([(29, 27), (23, 14), (16, 33), (29, 20), (10, 27)]) {
                let fmt = PixelFormat::new(mode, sample, true);
                let mut app = app(fmt);
                let before = app.session.active().unwrap().doc.clone();
                let history = app.session.active().unwrap().history.past_len();
                begin(&mut app, &egui::Context::default()).unwrap();
                app.run(id, json!({})).unwrap();
                commit(&mut app);
                assert!(!app.ui.status_error, "{mode:?}/{sample:?}/{id}: {}", app.ui.status);
                let st = app.session.active().unwrap();
                let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
                assert_eq!(s.format(), fmt);
                assert!((s.sample_channel(point.0, point.1, 0) - 0.75).abs() < 0.01, "{mode:?}/{sample:?}/{id} at {point:?}");
                assert_eq!(st.history.past_len(), history + 1);
                app.run("edit.undo", json!({})).unwrap();
                assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
            }
        }
    }
}

#[test]
fn presets_reject_drag_stale_session_target_and_nonfinite_input_without_changing_preview() {
    for case in 0..7 {
        let mut app = rgba();
        begin(&mut app, &egui::Context::default()).unwrap();
        match case {
            0 => {
                app.transform_preview.as_mut().unwrap().gesture = Some(Gesture { hit: Hit::Inside, start: [0.0; 2], quad0: [[0.0; 2]; 4], pivot0: [0.0; 2] });
            }
            1 => {
                app.ui.transform.as_mut().unwrap().session += 1;
            }
            2 => {
                app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
            }
            3 => {
                app.run("layer.new.layer", json!({})).unwrap();
            }
            4 => {
                app.ui.mask_target = true;
                app.session
                    .edit("lock", |doc, id| {
                        doc.layer_mut(id.unwrap()).unwrap().locks.all = true;
                        Ok(())
                    })
                    .unwrap();
            }
            5 => {
                app.ui.transform.as_mut().unwrap().pivot = [f64::NAN, 0.0];
            }
            _ => {
                app.ui.transform.as_mut().unwrap().pivot = [f64::MAX, 0.0];
            }
        }
        let before = app.ui.transform.as_ref().unwrap().quad;
        assert!(app.run(PRESETS[3], json!({})).is_err(), "case {case}");
        assert_eq!(app.ui.transform.as_ref().unwrap().quad, before);
    }
    let mut app = rgba();
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(begin(&mut app, &egui::Context::default()).is_err(), "empty target");
}

#[test]
fn selection_outline_and_selected_pixels_keep_their_edit_targets() {
    for outline in [false, true] {
        let mut app = rgba();
        app.run("select.rect", json!({"x": 10, "y": 20, "width": 8, "height": 8})).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        if outline {
            begin_selection(&mut app, &egui::Context::default()).unwrap();
        } else {
            begin(&mut app, &egui::Context::default()).unwrap();
        }
        app.run(PRESETS[3], json!({})).unwrap();
        commit(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let st = app.session.active().unwrap();
        let id = st.active_layer.unwrap();
        assert!((st.doc.layer(id).unwrap().surface().unwrap().sample_channel(10, 20, 0) - if outline { 0.75 } else { 0.25 }).abs() < 0.01);
        app.run("edit.undo", json!({})).unwrap();
        assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
    }
}

#[test]
fn presets_keep_shape_type_and_smart_object_editable() {
    for (command, params) in [
        ("shape.create", json!({"kind": "rect", "rect": [10, 20, 20, 8], "fill": "#ff0000"})),
        ("type.create", json!({"x": 10, "y": 40, "text": "Prism", "size": 16})),
        ("layer.smartObjects.convertToSmartObject", json!({})),
    ] {
        for id in PRESETS {
            let mut app = rgba();
            app.run(command, params.clone()).unwrap();
            begin(&mut app, &egui::Context::default()).unwrap();
            app.run(id, json!({})).unwrap();
            commit(&mut app);
            assert!(!app.ui.status_error, "{command}/{id}: {}", app.ui.status);
            let st = app.session.active().unwrap();
            let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
            assert!(match command {
                "shape.create" => matches!(l.content, LayerContent::Shape(_)),
                "type.create" => matches!(l.content, LayerContent::Text(_)),
                _ => matches!(l.content, LayerContent::Smart(_)),
            });
        }
    }
}

#[test]
fn flipping_unlinked_mask_changes_only_the_mask_and_cancel_preserves_pixels() {
    let mut app = rgba();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.session
        .edit("mask", |doc, id| {
            let m = doc.layer_mut(id.unwrap()).unwrap().mask.as_mut().unwrap();
            m.linked = false;
            m.surface.fill_rect(Rect::new(10, 20, 30, 28), &[0.0]);
            m.surface.fill_rect(Rect::new(10, 20, 12, 22), &[0.5]);
            Ok(())
        })
        .unwrap();
    app.ui.mask_target = true;
    begin(&mut app, &egui::Context::default()).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    app.run(PRESETS[1], json!({})).unwrap();
    cancel(&mut app);
    assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
    begin(&mut app, &egui::Context::default()).unwrap();
    app.ui.mask_target = false;
    assert!(app.run(PRESETS[3], json!({})).is_err(), "target changed");
    app.ui.mask_target = true;
    app.run(PRESETS[3], json!({})).unwrap();
    commit(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!((l.mask.as_ref().unwrap().surface.sample_channel(29, 20, 0) - 0.5).abs() < 0.01);
    assert!((l.surface().unwrap().sample_channel(10, 20, 0) - 0.75).abs() < 0.01);
}

#[test]
fn position_locked_target_is_disabled_and_immediate_commands_still_work() {
    let mut app = rgba();
    app.session
        .edit("lock", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().locks.position = true;
            Ok(())
        })
        .unwrap();
    begin(&mut app, &egui::Context::default()).unwrap();
    for id in PRESETS {
        assert_eq!(is_enabled(&app, id), Some(false));
        assert!(app.run(id, json!({})).is_err());
    }
    let mut app = rgba();
    let before = app.session.active().unwrap().history.past_len();
    app.run(PRESETS[3], json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), before + 1);
    assert!((st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().sample_channel(29, 20, 0) - 0.75).abs() < 0.01);
}

#[test]
fn automation_context_presets_work_in_main_and_detached_views() {
    for detached in [false, true] {
        let mut app = rgba();
        let ctx = egui::Context::default();
        let window = if detached { crate::doc_windows::detach(&mut app, &ctx, 0, None).unwrap()["window"].as_u64() } else { None };
        begin(&mut app, &ctx).unwrap();
        let original = app.ui.transform.as_ref().unwrap().quad;
        for (method, mut params) in [
            ("ui.pointer", json!({"window": window, "button": "right", "events": [{"kind": "down", "x": 15, "y": 23}, {"kind": "up", "x": 15, "y": 23}]})),
            ("ui.context.choose", json!({"id": "edit.transform.rotate90Cw"})),
        ] {
            if !detached {
                params.as_object_mut().unwrap().remove("window");
            }
            let (request, _) = crate::control::ControlRequest::new(method, params);
            let crate::control::Outcome::Done(reply) = crate::control::handle(&mut app, &ctx, &request) else {
                panic!("pending {method}");
            };
            assert_eq!(reply["ok"], true, "{detached}/{method}: {reply}");
        }
        assert!(app.ui.canvas_tool_menu.is_none());
        assert_ne!(app.ui.transform.as_ref().unwrap().quad, original);
        assert_eq!(app.ui.active_window, window);
    }
}
