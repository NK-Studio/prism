//! Hover feedback on the actual dock and Layers hit targets, including nested controls.

use egui::{CursorIcon, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::{PrismApp, dock::Group, theme::ThemeKind};

fn hover<T>(h: &mut Harness<'_, T>, at: Pos2) -> CursorIcon {
    h.hover_at(at);
    h.run_steps(2);
    h.output().platform_output.cursor_icon
}

#[test]
fn layer_rows_tabs_panel_menu_and_footer_have_click_cursors() {
    for kind in ThemeKind::ALL {
        let mut session = prism_engine::Session::new();
        session.execute("prefs.set", json!({"values":{"interface.theme":kind.id()}})).unwrap();
        session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        let layer = session.execute("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap();
        session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        session.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
        let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
            PrismApp::setup_context(&cc.egui_ctx, kind);
            let mut app = PrismApp::new(session, crate::Services::default());
            app.ui.theme = kind;
            for group in [Group::Color, Group::Properties, Group::History, Group::Navigator] {
                app.ui.dock.set_collapsed(group, true);
            }
            app
        });
        h.run_steps(8);
        let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == layer).unwrap();
        let strip = crate::dock::last_strips(&h.ctx).into_iter().find(|s| s.group == Group::Layers).unwrap();
        let steps = h.state().session.active().unwrap().history.past_len();
        for at in
            [row.name.unwrap().center(), pos2(row.row.left() + 17.0, row.row.center().y), pos2(row.row.left() + 45.0, row.row.center().y), strip.menu.center()]
                .into_iter()
                .chain(strip.tabs.iter().map(|(_, r)| r.center()))
        {
            assert_eq!(hover(&mut h, at), CursorIcon::PointingHand, "{kind:?} at {at:?}");
        }
        let (masks, chains) = crate::mask_thumbs_ui::recorded(&h.ctx, layer).unwrap();
        for rect in masks
            .iter()
            .chain(&chains)
            .map(|(_, rect)| rect)
            .chain(row.indicators.iter().filter(|(kind, _)| *kind == crate::layer_row_ui::Indicator::FxTriangle).map(|(_, rect)| rect))
        {
            assert_eq!(hover(&mut h, rect.center()), CursorIcon::PointingHand, "nested layer control: {kind:?}");
        }
        for label in ["Delete layer", "Create a new layer", "Create a new group"] {
            let at = h.get_by_label(crate::i18n::t(label)).rect().center();
            assert_eq!(hover(&mut h, at), CursorIcon::PointingHand, "{kind:?}: {label}");
        }
        let empty = pos2(row.row.center().x, row.row.bottom() + 150.0);
        assert_eq!(hover(&mut h, empty), CursorIcon::Default, "unused panel space: {kind:?}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "hover leaves the document unchanged");
    }
}

#[test]
fn disabled_buttons_and_numeric_fields_keep_their_appropriate_cursors() {
    let mut h = Harness::new_ui_state(
        |ui, rects: &mut Vec<Rect>| {
            crate::theme::apply(ui.ctx(), ThemeKind::Pro);
            rects.clear();
            rects.push(crate::icons::button(ui, "link", 26.0, false, "Link").rect);
            rects.push(ui.add_enabled_ui(false, |ui| crate::icons::button(ui, "trash", 26.0, false, "Disabled")).inner.rect);
            rects.push(ui.button("Standard button").rect);
            let mut value = 50.0;
            rects.push(crate::widgets::value_field(ui, &mut value, 0.0..=100.0, "%", 80.0).rect);
        },
        Vec::new(),
    );
    h.run_steps(4);
    let rects = h.state().clone();
    assert_eq!(hover(&mut h, rects[0].center()), CursorIcon::PointingHand);
    assert_eq!(hover(&mut h, rects[1].center()), CursorIcon::Default);
    assert_eq!(hover(&mut h, rects[2].center()), CursorIcon::PointingHand);
    assert_eq!(hover(&mut h, rects[3].center()), CursorIcon::ResizeHorizontal);
}

#[test]
fn layer_hover_is_blue_over_name_and_thumbnail_and_selection_keeps_its_style() {
    for kind in ThemeKind::ALL {
        let mut session = prism_engine::Session::new();
        session.execute("prefs.set", json!({"values":{"interface.theme":kind.id()}})).unwrap();
        session.execute("file.new", json!({"width":64,"height":48})).unwrap();
        let background = session.active().unwrap().active_layer.unwrap().0;
        session.execute("layer.new.layer", json!({"name":"Paint"})).unwrap();
        let capture = std::env::var_os("PRISM_WINDOW_SNAPSHOTS");
        let scale = std::env::var("PRISM_WINDOW_SCALE").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0);
        let builder = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_pixels_per_point(scale).with_max_steps(64);
        let builder = if capture.is_some() { builder.wgpu() } else { builder };
        let mut h = builder.build_eframe(move |cc| {
            PrismApp::setup_context(&cc.egui_ctx, kind);
            let mut app = PrismApp::new(session, Default::default());
            app.ui.theme = kind;
            for group in [Group::Color, Group::Properties, Group::History, Group::Navigator] {
                app.ui.dock.set_collapsed(group, true);
            }
            app
        });
        h.run_steps(6);
        assert_eq!(h.state().ui.theme, kind);
        let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == background).unwrap();
        let t = crate::theme::Tokens::for_kind(kind);
        let painted_blue = |h: &Harness<'_, PrismApp>| {
            h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == t.layer_hover && r.rect.contains(row.row.center())))
        };
        let history = h.state().session.active().unwrap().history.past_len();
        for at in [row.name.unwrap().center(), pos2(row.row.left() + 45.0, row.row.center().y)] {
            h.hover_at(at);
            h.run_steps(3);
            assert!(painted_blue(&h), "{kind:?} at {at:?}");
            assert_ne!(h.state().session.active().unwrap().active_layer.unwrap().0, background);
        }
        if let Some(dir) = &capture {
            std::fs::create_dir_all(dir).unwrap();
            h.render().unwrap().save(std::path::PathBuf::from(dir).join(format!("hover-{}-{}x.png", kind.id(), scale))).unwrap();
        }
        let pos = row.name.unwrap().center();
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(4);
        assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, background);
        assert!(!painted_blue(&h), "selection must replace hover, {kind:?}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    }
}
