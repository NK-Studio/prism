use super::*;
use prism_doc::{ColorMode, SampleType, Size};

fn app(mode: ColorMode, depth: SampleType) -> PrismApp {
    let mut s = prism_engine::Session::new();
    s.add_document(Document::with_background("shape colour", Size::new(120, 100), mode, depth, Color::WHITE), None);
    let mut a = PrismApp::new(s, Default::default());
    a.run("shape.create", json!({"kind":"roundedRect","rect":[10,10,80,70],"radii":12,"fill":[0,0,1,0.7],"stroke":{"color":"#000000","width":2}})).unwrap();
    a.sync_views();
    a
}

fn fields(a: &PrismApp, id: u64) -> Map<String, Value> {
    a.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone()
}

fn set(a: &mut PrismApp, id: u64, color: &str) {
    a.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!(color));
}

#[test]
fn live_colour_keeps_geometry_masks_alpha_and_history_and_ok_is_one_edit_at_all_depths_and_modes() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut a = app(mode, depth);
            a.run("layer.layerMask.revealAll", json!({})).unwrap();
            let st = a.session.active().unwrap();
            let (original, history, revision, layer) = (st.doc.clone(), st.history.past_len(), st.revision, st.active_layer.unwrap());
            let id = open(&mut a).unwrap();
            assert!(display_doc(&mut a, 0).is_none());
            let mut shown = original.clone();
            let mut key = 0;
            for color in ["#ff0000", "#00ff00", "#ff8000"] {
                set(&mut a, id, color);
                (shown, key) = display_doc(&mut a, 0).unwrap();
                let (cached, k) = display_doc(&mut a, 0).unwrap();
                assert!(Arc::ptr_eq(&shown, &cached));
                assert_eq!(key, k);
                assert_eq!((shown.mode, shown.depth), (mode, depth));
                let LayerContent::Shape(before) = &original.layer(layer).unwrap().content else { panic!("shape") };
                let LayerContent::Shape(after) = &shown.layer(layer).unwrap().content else { panic!("shape") };
                assert_eq!((&before.path, &before.live, &before.stroke), (&after.path, &after.live, &after.stroke));
                assert_eq!(
                    after.fill,
                    Some(Fill::Solid(Color::rgba(
                        color_picker_ui::parse_hex(color).unwrap()[0],
                        color_picker_ui::parse_hex(color).unwrap()[1],
                        color_picker_ui::parse_hex(color).unwrap()[2],
                        0.7
                    )))
                );
                assert_eq!(shown.layer(layer).unwrap().mask, original.layer(layer).unwrap().mask);
                assert!(Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
                assert_eq!(a.session.active().unwrap().history.past_len(), history);
            }
            let area = damage(&a, original.id.0, revision, 0, key).unwrap();
            assert!(area.contains(40, 40) && area.width() < original.size.width);
            assert_eq!(damage(&a, original.id.0, revision, key, 0), Some(area));
            crate::dialogs::confirm(&mut a, id).unwrap();
            assert_eq!(a.session.active().unwrap().history.past_len(), history + 1);
            assert_eq!(a.session.active().unwrap().doc.layer(layer).unwrap().content, shown.layer(layer).unwrap().content);
            a.run("edit.undo", json!({})).unwrap();
            assert_eq!(a.session.active().unwrap().doc.layer(layer).unwrap().content, original.layer(layer).unwrap().content);
            a.run("edit.redo", json!({})).unwrap();
            assert_eq!(a.session.active().unwrap().doc.layer(layer).unwrap().content, shown.layer(layer).unwrap().content);
        }
    }
}

#[test]
fn cancel_and_unchanged_ok_keep_the_exact_float_colour_and_add_no_history() {
    let mut a = app(ColorMode::Rgb, SampleType::F32);
    a.run("shape.edit", json!({"fill":[1.5,0.234567,0.876543,0.123456]})).unwrap();
    let original = a.session.active().unwrap().doc.clone();
    let history = a.session.active().unwrap().history.past_len();
    let id = open(&mut a).unwrap();
    set(&mut a, id, "#f0a010");
    assert!(display_doc(&mut a, 0).is_some());
    a.ui.close_dialog(id);
    assert!(display_doc(&mut a, 0).is_none());
    assert!(Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
    let id = open(&mut a).unwrap();
    let base = fields(&a, id)["color"].as_str().unwrap().to_ascii_uppercase();
    set(&mut a, id, &base);
    crate::dialogs::confirm(&mut a, id).unwrap();
    assert!(Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
    assert_eq!(a.session.active().unwrap().history.past_len(), history);
}

#[test]
fn rasterized_locked_stale_and_invalid_targets_never_change_a_document() {
    for change in 0..5 {
        let mut a = app(ColorMode::Rgb, SampleType::U8);
        let id = open(&mut a).unwrap();
        set(&mut a, id, "#ff8000");
        display_doc(&mut a, 0).unwrap();
        match change {
            0 => {
                a.run("shape.rasterize", json!({})).unwrap();
            }
            1 => {
                a.run("layer.setProps", json!({"locks":{"all":true}})).unwrap();
            }
            2 => {
                a.run("file.new", json!({"width":30,"height":20})).unwrap();
            }
            3 => {
                a.run("shape.edit", json!({"radii":25})).unwrap();
            }
            _ => set(&mut a, id, "#🤔ab"),
        }
        let original = a.session.active().unwrap().doc.clone();
        let history = a.session.active().unwrap().history.past_len();
        let index = a.session.active_index().unwrap();
        let f = fields(&a, id);
        assert!(display_doc(&mut a, index).is_none());
        assert!(confirm(&mut a, &f).is_err());
        assert!(Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
        assert_eq!(a.session.active().unwrap().history.past_len(), history);
        if change < 2 {
            assert!(open(&mut a).is_err());
        }
    }
    let mut a = PrismApp::new(prism_engine::Session::new(), Default::default());
    assert!(open(&mut a).is_err());
    assert!(confirm(&mut a, &Map::new()).is_err());
}

#[test]
fn all_shape_tool_geometries_and_custom_paths_keep_vector_content() {
    for p in [
        json!({"kind":"rect","rect":[10,10,80,70]}),
        json!({"kind":"ellipse","rect":[10,10,80,70]}),
        json!({"kind":"polygon","rect":[10,10,80,70],"sides":3}),
        json!({"kind":"polygon","rect":[10,10,80,70],"sides":6}),
        json!({"kind":"star","rect":[10,10,80,70]}),
        json!({"kind":"line","from":[10,10],"to":[90,80],"weight":8}),
    ] {
        let mut a = app(ColorMode::Rgb, SampleType::U8);
        a.run("shape.create", p).unwrap();
        // A transformed/custom shape without live geometry is still a shape layer.
        if a.session.active().unwrap().doc.layer(a.session.active().unwrap().active_layer.unwrap()).unwrap().name == "Star" {
            a.run("shape.edit", json!({"transform":[1,0.2,0,1,0,0]})).unwrap();
        }
        let id = open(&mut a).unwrap();
        set(&mut a, id, "#00ff00");
        crate::dialogs::confirm(&mut a, id).unwrap();
        let st = a.session.active().unwrap();
        assert!(matches!(st.doc.layer(st.active_layer.unwrap()).unwrap().content, LayerContent::Shape(_)));
    }
}

#[test]
fn no_fill_can_be_coloured_and_a_restored_dialog_cannot_commit_without_its_source() {
    let mut a = app(ColorMode::Rgb, SampleType::U8);
    a.run("shape.edit", json!({"fill":null})).unwrap();
    let id = open(&mut a).unwrap();
    set(&mut a, id, "#55ccaa");
    assert!(display_doc(&mut a, 0).is_some());
    let f = fields(&a, id);
    a.shape_color_runtime = Default::default();
    assert!(confirm(&mut a, &f).is_err());
    assert!(display_doc(&mut a, 0).is_none());
}

#[test]
fn replacing_a_picker_before_redraw_restores_the_previous_preview_footprint() {
    let mut a = app(ColorMode::Rgb, SampleType::U8);
    let first = open(&mut a).unwrap();
    set(&mut a, first, "#ff0000");
    let (shown, key) = display_doc(&mut a, 0).unwrap();
    let revision = a.session.active().unwrap().revision;
    let area = damage(&a, shown.id.0, revision, 0, key).unwrap();
    let f = fields(&a, first);
    let second = open(&mut a).unwrap();
    assert_ne!(first, second);
    assert!(a.ui.dialogs.iter().all(|d| d.id != first));
    assert!(display_doc(&mut a, 0).is_none());
    assert_eq!(damage(&a, shown.id.0, revision, key, 0), Some(area));
    assert!(confirm(&mut a, &f).is_err());
}
