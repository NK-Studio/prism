use super::*;
use crate::canvas::tool_event;
use prism_doc::{Color, ColorMode, SampleType, Size};

fn app(depth: SampleType) -> PrismApp {
    let mut session = prism_engine::Session::new();
    session.add_document(Document::with_background("corners", Size::new(300, 200), ColorMode::Rgb, depth, Color::WHITE), None);
    let mut a = PrismApp::new(session, Default::default());
    a.run("shape.create", json!({"kind":"roundedRect", "rect":[20,30,200,100], "radii":20, "fill":"#0000ff"})).unwrap();
    a.ui.tool = Tool::Rectangle;
    a.ui.extras.snap = false;
    a.ui.extras.rulers = false;
    a.ui.view.show.smart_guides = false;
    a.sync_views();
    a.ui.views[0].zoom = 1.0;
    a.ui.views[0].center = [150.0, 100.0];
    a
}

fn live(doc: &Document, id: LayerId) -> [f64; 4] {
    let LayerContent::Shape(sh) = &doc.layer(id).unwrap().content else { panic!("shape") };
    let Some(LiveShape::Rect { radii, .. }) = sh.live else { panic!("rectangle") };
    radii
}

fn event(a: &mut PrismApp, kind: &str, p: [f64; 2], mods: Modifiers) {
    let [x, y] = p;
    tool_event(
        a,
        match kind {
            "down" => ToolEvent::Down { x, y, pressure: 1.0 },
            "move" => ToolEvent::Move { x, y, pressure: 1.0 },
            _ => ToolEvent::Up { x, y },
        },
        mods,
    );
}

#[test]
fn linked_preview_preserves_the_document_then_commits_one_undoable_edit_at_every_depth() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for tool in [Tool::Rectangle, Tool::PathSelection, Tool::DirectSelection] {
            let mut a = app(depth);
            a.ui.tool = tool;
            // Corner widgets take precedence over Snap and Direct Selection's path nodes.
            a.ui.extras.snap = true;
            let st = a.session.active().unwrap();
            let (original, id, history, rev) = (st.doc.clone(), st.active_layer.unwrap(), st.history.past_len(), st.revision);
            event(&mut a, "down", [40.0, 50.0], Modifiers::NONE);
            event(&mut a, "move", [60.0, 70.0], Modifiers::NONE);
            assert!(a.drag.is_none());
            assert!(Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
            assert_eq!(a.session.active().unwrap().history.past_len(), history);
            let (shown, key) = display_doc(&mut a, 0).unwrap();
            assert_eq!(live(&shown, id), [40.0; 4]);
            let LayerContent::Shape(shape) = &shown.layer(id).unwrap().content else { panic!("preview shape") };
            assert_eq!(outline(&a, id).as_ref(), Some(&shape.path));
            assert_eq!(shown.depth, depth);
            let (same, same_key) = display_doc(&mut a, 0).unwrap();
            assert!(Arc::ptr_eq(&shown, &same));
            assert_eq!(key, same_key, "unchanged radii reuse the rendered shape");
            let area = damage(&a, original.id.0, rev, 0, key).unwrap();
            assert!(area.contains(20, 30) && area.contains(219, 129));
            assert_eq!(damage(&a, original.id.0, rev, key, 0), Some(area));
            assert!(damage(&a, original.id.0, rev + 1, key, 0).is_none());
            // A real pixel changes during preview, rather than only its path outline.
            let sample = Rect::new(28, 38, 29, 39);
            let before = prism_compose::render(&original, sample).px[0];
            let preview = prism_compose::render(&shown, sample).px[0];
            assert!(before[2] > 0.9 && before[0] < 0.1, "blue: {before:?}");
            assert!(preview[0] > 0.9 && preview[1] > 0.9, "white: {preview:?}");
            event(&mut a, "up", [60.0, 70.0], Modifiers::NONE);
            let st = a.session.active().unwrap();
            assert_eq!(st.history.past_len(), history + 1);
            assert_eq!(live(&st.doc, id), [40.0; 4]);
            assert_eq!(st.doc.layer_count(), original.layer_count());
            assert_eq!(st.doc.layer(id).unwrap().content, shown.layer(id).unwrap().content);
            a.run("edit.undo", json!({})).unwrap();
            assert_eq!(live(&a.session.active().unwrap().doc, id), [20.0; 4]);
            a.run("edit.redo", json!({})).unwrap();
            assert_eq!(live(&a.session.active().unwrap().doc, id), [40.0; 4]);
        }
    }
}

#[test]
fn option_edits_each_corner_independently_and_clamps_without_changing_its_neighbours() {
    for (corner, direction) in DIRECTIONS.iter().enumerate() {
        for (delta, radius) in [(15.0, 35.0), (-200.0, 0.0), (200.0, 80.0)] {
            let mut a = app(SampleType::U8);
            let (id, rect, base) = rectangle(&a).unwrap();
            let start = handles(rect, base, 1.0)[corner];
            let end = [start[0] + delta * direction[0], start[1] + delta * direction[1]];
            event(&mut a, "down", start, Modifiers::ALT);
            // The modifier is captured at the press: releasing Option mid-drag keeps it independent.
            event(&mut a, "up", end, Modifiers::NONE);
            let mut expected = base;
            expected[corner] = radius;
            assert_eq!(live(&a.session.active().unwrap().doc, id), expected);
        }
    }
}

#[test]
fn clicks_and_a_drag_back_to_the_start_leave_no_history_or_preview() {
    let mut a = app(SampleType::U8);
    let history = a.session.active().unwrap().history.past_len();
    for moving in [false, true] {
        event(&mut a, "down", [40.0, 50.0], Modifiers::NONE);
        if moving {
            event(&mut a, "move", [60.0, 70.0], Modifiers::NONE);
        }
        event(&mut a, "move", [40.1, 50.1], Modifiers::NONE);
        assert!(display_doc(&mut a, 0).is_none());
        event(&mut a, "up", [40.0, 50.0], Modifiers::NONE);
        assert_eq!(a.session.active().unwrap().history.past_len(), history);
        assert!(a.ui.shape_corners.is_none());
    }
}

#[test]
fn sharp_rectangles_can_be_rounded_and_handles_remain_distinct_at_maximum_radius() {
    let mut a = app(SampleType::U8);
    a.run("shape.edit", json!({"radii":0})).unwrap();
    let (id, rect, _) = rectangle(&a).unwrap();
    let start = handles(rect, [0.0; 4], 1.0)[0];
    event(&mut a, "down", start, Modifiers::NONE);
    event(&mut a, "up", [start[0] + 80.0, start[1] + 80.0], Modifiers::NONE);
    assert_eq!(live(&a.session.active().unwrap().doc, id), [50.0; 4]);
    for zoom in [0.1, 1.0, 8.0] {
        let points = handles([0.0, 0.0, 10.0, 10.0], [100.0; 4], zoom);
        for i in 0..4 {
            for j in i + 1..4 {
                assert_ne!(points[i], points[j]);
            }
        }
    }
}

#[test]
fn stale_tool_document_revision_or_selection_never_commits_to_another_target() {
    for change in 0..4 {
        let mut a = app(SampleType::U8);
        let id = rectangle(&a).unwrap().0;
        event(&mut a, "down", [40.0, 50.0], Modifiers::NONE);
        event(&mut a, "move", [60.0, 70.0], Modifiers::NONE);
        display_doc(&mut a, 0).unwrap();
        match change {
            0 => a.ui.tool = Tool::Brush,
            1 => {
                a.run("file.new", json!({"width":100,"height":100})).unwrap();
            }
            2 => {
                a.run("shape.edit", json!({"radii":10})).unwrap();
            }
            _ => {
                let background = a.session.active().unwrap().doc.layers[0].id;
                a.run("layer.select", json!({"layer":background.0})).unwrap();
            }
        }
        cancel_stale(&mut a);
        assert!(a.ui.shape_corners.is_none());
        assert!(a.shape_corner_runtime.preview.is_none());
        assert!(!pointer(&mut a, ToolEvent::Up { x: 60.0, y: 70.0 }, Modifiers::NONE));
        assert_eq!(live(&a.session.documents()[0].doc, id), if change == 2 { [10.0; 4] } else { [20.0; 4] });
    }
}

#[test]
fn unavailable_shapes_masks_hidden_paths_and_locked_groups_have_no_handles() {
    for mode in 0..9 {
        let mut a = app(SampleType::U8);
        let id = rectangle(&a).unwrap().0;
        match mode {
            0 => a.ui.mask_target = true,
            1 => a.ui.vector_mask_target = true,
            2 => a.ui.view.extras = false,
            3 => a.ui.tool = Tool::EllipseShape,
            4 => {
                a.run("shape.edit", json!({"kind":"ellipse"})).unwrap();
            }
            5 => {
                Arc::make_mut(&mut a.session.active_mut().unwrap().doc).layer_mut(id).unwrap().locks.pixels = true;
            }
            6 => {
                Arc::make_mut(&mut a.session.active_mut().unwrap().doc).layer_mut(id).unwrap().visible = false;
            }
            7 | 8 => {
                a.run("layer.groupLayers", json!({})).unwrap();
                let group = a.session.active().unwrap().active_layer.unwrap();
                let st = a.session.active_mut().unwrap();
                let g = Arc::make_mut(&mut st.doc).layer_mut(group).unwrap();
                if mode == 7 {
                    g.locks.all = true;
                } else {
                    g.visible = false;
                }
                a.run("layer.select", json!({"layer":id.0})).unwrap();
            }
            _ => panic!("unknown test mode"),
        }
        assert!(rectangle(&a).is_none(), "mode {mode}");
        assert!(inspect(&a).is_null());
    }
}

#[test]
fn saved_ui_state_does_not_restore_a_held_corner_gesture() {
    let mut a = app(SampleType::U8);
    event(&mut a, "down", [40.0, 50.0], Modifiers::NONE);
    let value = serde_json::to_value(&a.ui).unwrap();
    assert!(!value["shape_corners"].is_null());
    let restored: crate::state::UiState = serde_json::from_value(value).unwrap();
    assert!(restored.shape_corners.is_none());
}

type Harness = egui_kittest::Harness<'static, PrismApp>;

fn harness(zoom: f32, rotation: f32, flip: bool) -> Harness {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, Default::default());
        let mut a = app(SampleType::U8);
        a.ui.views[0].zoom = zoom;
        a.ui.views[0].rotation = rotation;
        a.ui.view.flip_horizontal = flip;
        a
    });
    h.run_steps(4);
    h
}

fn screen(h: &Harness, p: [f64; 2]) -> egui::Pos2 {
    let a = h.state();
    let v = &a.ui.views[0];
    ViewXform {
        rect: crate::rulers::content_rect(a, a.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: a.ui.view.flip_horizontal,
        rotation: v.rotation,
    }
    .to_screen(p[0] as f32, p[1] as f32)
}

fn native_button(h: &mut Harness, p: [f64; 2], pressed: bool) {
    let pos = screen(h, p);
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

#[test]
fn native_press_precedes_drag_threshold_and_uses_zoom_rotation_and_flip() {
    for (zoom, rotation, flip) in [(0.5, 0.0, false), (1.0, 30.0, false), (2.0, 30.0, true)] {
        let mut h = harness(zoom, rotation, flip);
        let (id, rect, base) = rectangle(h.state()).unwrap();
        let at = handles(rect, base, zoom)[0];
        let end = [at[0] + 15.0, at[1] + 15.0];
        let history = h.state().session.active().unwrap().history.past_len();
        native_button(&mut h, at, true);
        assert!(h.state().ui.shape_corners.is_some(), "press at zoom={zoom}");
        assert!(h.state().drag.is_none());
        h.event(Event::PointerMoved(screen(&h, end)));
        h.run_steps(2);
        assert_eq!(h.state().ui.shape_corners.as_ref().unwrap().radii, [35.0; 4]);
        native_button(&mut h, end, false);
        let st = h.state().session.active().unwrap();
        assert_eq!(live(&st.doc, id), [35.0; 4]);
        assert_eq!(st.history.past_len(), history + 1);
        assert_eq!(st.doc.layer_count(), 2);
    }
}

#[test]
fn escape_enter_and_focus_loss_own_the_release_without_drawing_an_extra_rectangle() {
    for mode in 0..3 {
        let mut h = harness(1.0, 0.0, false);
        let history = h.state().session.active().unwrap().history.past_len();
        let id = rectangle(h.state()).unwrap().0;
        native_button(&mut h, [40.0, 50.0], true);
        h.event(Event::PointerMoved(screen(&h, [60.0, 70.0])));
        h.run_steps(2);
        match mode {
            0 => h.key_press(egui::Key::Escape),
            1 => h.key_press(egui::Key::Enter),
            _ => h.event(Event::WindowFocused(false)),
        }
        h.run_steps(3);
        assert!(h.state().ui.shape_corners.is_none());
        // Movement after cancellation and the eventual Up must remain consumed.
        h.event(Event::PointerMoved(screen(&h, [90.0, 100.0])));
        h.step();
        native_button(&mut h, [90.0, 100.0], false);
        h.run_steps(3);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layer_count(), 2);
        assert_eq!(live(&st.doc, id), if mode == 1 { [40.0; 4] } else { [20.0; 4] });
        assert_eq!(st.history.past_len(), history + usize::from(mode == 1));
        assert!(h.state().drag.is_none());
    }
}

#[test]
fn control_inspection_and_pointer_can_drive_the_same_independent_corner_edit() {
    use crate::control::{ControlRequest, Outcome, handle};
    let mut a = app(SampleType::U8);
    let ctx = egui::Context::default();
    let value = crate::control::inspect(&a, &ctx);
    assert_eq!(value["shapeCorners"]["handles"], json!([[40.0, 50.0], [200.0, 50.0], [200.0, 110.0], [40.0, 110.0]]));
    let (request, _) = ControlRequest::new(
        "ui.pointer",
        json!({"modifiers":{"alt":true},"events":[{"kind":"down","x":200,"y":50},{"kind":"move","x":185,"y":65},{"kind":"up","x":185,"y":65}]}),
    );
    let Outcome::Done(response) = handle(&mut a, &ctx, &request) else { panic!("pointer result") };
    assert_eq!(response["ok"], true);
    assert_eq!(inspect(&a)["radii"], json!([20.0, 35.0, 20.0, 20.0]));
}

/// The Move tool shows the corner widgets too, as it does the arrow handles: hidden while it
/// drags the layer, back on release, and a press on one still edits the corner.
#[test]
fn the_move_tool_shows_the_widgets_but_hides_them_while_moving() {
    let mut a = app(SampleType::U8);
    a.ui.tool = Tool::Move;
    let (id, rect, base) = rectangle(&a).expect("widgets with the Move tool");
    assert!(!inspect(&a).is_null());
    // Drag the rectangle by its middle: no widgets meanwhile.
    event(&mut a, "down", [120.0, 80.0], Modifiers::NONE);
    event(&mut a, "move", [150.0, 100.0], Modifiers::NONE);
    assert!(a.drag.as_ref().is_some_and(|d| d.tool == Tool::Move));
    assert!(rectangle(&a).is_none() && inspect(&a).is_null(), "hidden while moving");
    assert!(cursor(&a, handles(rect, base, 1.0)[0]).is_none());
    event(&mut a, "up", [150.0, 100.0], Modifiers::NONE);
    assert!(a.drag.is_none());
    let (_, moved, _) = rectangle(&a).expect("back after the move");
    assert_eq!(moved, [rect[0] + 30.0, rect[1] + 20.0, rect[2], rect[3]]);
    // A press on a widget edits that corner rather than moving the layer.
    let corner = handles(moved, base, 1.0)[0];
    event(&mut a, "down", corner, Modifiers::NONE);
    assert!(a.ui.shape_corners.is_some() && a.drag.is_none());
    event(&mut a, "move", [corner[0] + 10.0, corner[1] + 10.0], Modifiers::NONE);
    event(&mut a, "up", [corner[0] + 10.0, corner[1] + 10.0], Modifiers::NONE);
    let after = live(&a.session.active().unwrap().doc, id);
    assert!(after[0] > base[0], "{after:?}");
    assert_eq!(rectangle(&a).unwrap().1, moved, "the layer stayed put");
}
