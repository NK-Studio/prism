use super::*;
use crate::theme::ThemeKind;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn harness(mode: ColorPanelMode) -> Harness<'static, PrismApp> {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.ui.color_panel.mode = mode;
    app.session.tools.foreground = [0.4, 0.0, 0.0, 1.0];
    let mut h = Harness::builder().with_size(vec2(320.0, 600.0)).build_ui_state(|ui, app: &mut PrismApp| show_mode(app, ui, app.ui.color_panel.mode), app);
    PrismApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'_, PrismApp>, p: Pos2) {
    h.hover_at(p);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
}

fn hsv(h: &Harness<'_, PrismApp>, background: bool) -> [f32; 3] {
    let tools = &h.state().session.tools;
    let [r, g, b, _] = if background { tools.background } else { tools.foreground };
    crate::color_picker_ui::rgb_to_hsv([r, g, b])
}

#[test]
fn triangle_coordinates_round_trip_and_clamp_outside_drags() {
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(240.0, 240.0));
    for s in [0.0, 0.2, 0.7, 1.0] {
        for v in [0.1, 0.4, 1.0] {
            let [s2, v2] = triangle_sv(rect, triangle_point(rect, s, v));
            assert!((s - s2).abs() < 1e-5 && (v - v2).abs() < 1e-5, "{s}/{v} -> {s2}/{v2}");
        }
    }
    let [h, w, b] = triangle(rect);
    assert_eq!(triangle_sv(rect, h), [1.0, 1.0]);
    assert_eq!(triangle_sv(rect, w), [0.0, 1.0]);
    assert_eq!(triangle_sv(rect, b), [0.0, 0.0]);
    for p in [rect.min, pos2(-1000.0, 20.0), pos2(20.0, 1000.0)] {
        assert!(triangle_sv(rect, p).iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    }
    assert!(matches!(wheel_part(rect, rect.center() + vec2(108.0, 0.0)), Some(WheelPart::Ring)));
    assert!(wheel_part(rect, rect.min).is_none());
    assert!(matches!(wheel_part(rect, rect.center()), Some(WheelPart::Triangle)));
}

#[test]
fn brightness_cube_varies_hue_and_saturation_while_slider_varies_only_brightness() {
    let mut h = harness(ColorPanelMode::BrightnessCube);
    let field = h.get_by_label("Brightness cube field").rect();
    click(&mut h, pos2(field.left() + field.width() / 3.0, field.top() + field.height() * 0.2));
    let [hue, s, v] = hsv(&h, false);
    assert!((hue - 120.0).abs() < 2.0 && (s - 0.8).abs() < 0.02 && (v - 0.4).abs() < 0.01);
    let strip = h.get_by_label("Brightness cube slider").rect();
    click(&mut h, pos2(strip.center().x, strip.top() + strip.height() * 0.1));
    let [hue, s, v] = hsv(&h, false);
    assert!((hue - 120.0).abs() < 2.0 && (s - 0.8).abs() < 0.02 && (v - 0.9).abs() < 0.01);
}

#[test]
fn wheel_ring_changes_only_hue_and_triangle_changes_saturation_and_brightness() {
    let mut h = harness(ColorPanelMode::ColorWheel);
    let outer = h.get_by_label("Color wheel").rect();
    let rect = Rect::from_center_size(outer.center(), vec2(outer.height() - 14.0, outer.height() - 14.0));
    click(&mut h, rect.center() + direction(1.0 / 3.0) * rect.width() * 0.45);
    let [hue, s, v] = hsv(&h, false);
    assert!((hue - 120.0).abs() < 2.0 && (s - 1.0).abs() < 0.01 && (v - 0.4).abs() < 0.01);
    click(&mut h, triangle_point(rect, 0.4, 0.8));
    let [hue, s, v] = hsv(&h, false);
    assert!((hue - 120.0).abs() < 2.0 && (s - 0.4).abs() < 0.02 && (v - 0.8).abs() < 0.01);
    let original = h.state().session.tools.foreground;
    click(&mut h, rect.min);
    assert_eq!(h.state().session.tools.foreground, original, "outside the ring and triangle is inert");
}

#[test]
fn wheel_drag_keeps_the_component_chosen_on_press_and_projects_to_the_triangle_edge() {
    let mut h = harness(ColorPanelMode::ColorWheel);
    let outer = h.get_by_label("Color wheel").rect();
    let rect = Rect::from_center_size(outer.center(), vec2(outer.height() - 14.0, outer.height() - 14.0));
    let from = triangle_point(rect, 0.5, 0.8);
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for k in 1..=8 {
        h.hover_at(from + (rect.right_center() - from) * (k as f32 / 8.0));
        h.step();
    }
    let [hue, s, v] = hsv(&h, false);
    assert!(hue.abs() < 1.0 && s > 0.98 && v > 0.98, "dragging onto the ring stays on the triangle: {hue}/{s}/{v}");
    h.event(Event::PointerButton { pos: rect.right_center(), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
}

#[test]
fn every_mode_can_edit_background_without_changing_foreground() {
    for mode in [ColorPanelMode::HueCube, ColorPanelMode::BrightnessCube, ColorPanelMode::ColorWheel, ColorPanelMode::HsbSliders] {
        let mut h = harness(mode);
        let foreground = h.state().session.tools.foreground;
        click(&mut h, pos2(40.0, 40.0));
        assert!(h.state().ui.color_panel.background, "{mode:?}");
        if mode == ColorPanelMode::HueCube {
            let field = h.get_by_label("Hue cube field").rect();
            click(&mut h, field.center());
        } else if mode == ColorPanelMode::BrightnessCube {
            let field = h.get_by_label("Brightness cube field").rect();
            click(&mut h, field.center());
        } else if mode == ColorPanelMode::ColorWheel {
            let outer = h.get_by_label("Color wheel").rect();
            let rect = Rect::from_center_size(outer.center(), vec2(outer.height() - 14.0, outer.height() - 14.0));
            click(&mut h, triangle_point(rect, 0.5, 0.5));
        } else {
            // Hex readout routes to the selected background colour, too.
            h.get_by_role(egui::accesskit::Role::TextInput).click();
            h.run_steps(1);
            h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
            h.event(Event::Text("336699".into()));
            h.run_steps(3);
        }
        assert_eq!(h.state().session.tools.foreground, foreground, "{mode:?}");
        assert_ne!(h.state().session.tools.background, [1.0, 1.0, 1.0, 1.0], "{mode:?}");
    }
}

#[test]
fn changing_modes_preserves_colors_and_remembers_hue_and_saturation_on_black() {
    let mut h = harness(ColorPanelMode::HsbSliders);
    let hsv = egui::ecolor::Hsva::new(0.6, 0.8, 0.0, 1.0);
    let ctx = h.ctx.clone();
    ctx.data_mut(|d| d.insert_temp(hsv_key(false), hsv));
    h.state_mut().session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    for mode in [ColorPanelMode::HueCube, ColorPanelMode::BrightnessCube, ColorPanelMode::ColorWheel, ColorPanelMode::HsbSliders] {
        h.state_mut().ui.color_panel.mode = mode;
        h.run_steps(3);
        assert_eq!(h.state().session.tools.foreground, [0.0, 0.0, 0.0, 1.0]);
        let stored: egui::ecolor::Hsva = h.ctx.data(|d| d.get_temp(hsv_key(false))).unwrap();
        assert_eq!((stored.h, stored.s), (0.6, 0.8));
    }
}

#[test]
fn color_menu_selects_modes_in_every_theme_and_workspace_restores_them() {
    for theme in ThemeKind::ALL {
        for scale in [1.0, 2.0] {
            let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_pixels_per_point(scale).build_eframe(move |cc| {
                PrismApp::setup_context(&cc.egui_ctx, theme);
                let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
                app.ui.dock_tabs.color = usize::from(!matches!(theme, ThemeKind::Pro | ThemeKind::ProMedium));
                app
            });
            h.run_steps(8);
            // The app theme is read from preferences, so select it through the same UI API.
            let ctx = h.ctx.clone();
            let (req, _rx) = crate::control::ControlRequest::new(
                "ui.set",
                json!({"theme":theme.id(),"dockTabs":{"color":usize::from(!matches!(theme, ThemeKind::Pro | ThemeKind::ProMedium))}}),
            );
            crate::control::handle(h.state_mut(), &ctx, &req);
            h.state_mut().ui.dock_tabs.color = usize::from(!matches!(theme, ThemeKind::Pro | ThemeKind::ProMedium));
            h.run_steps(4);
            for (mode, label) in MODES {
                let menu = crate::dock::last_strips(&h.ctx).into_iter().find(|s| s.group == crate::dock::Group::Color).unwrap().menu.center();
                click(&mut h, menu);
                let node = h.get_by_label(label);
                if h.state().ui.color_panel.mode == mode {
                    assert_eq!(node.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
                }
                let height = h.state().ui.dock.height(crate::dock::Group::Color);
                node.click();
                h.run_steps(4);
                assert_eq!(h.state().ui.color_panel.mode, mode, "{theme:?} {scale}");
                if mode == ColorPanelMode::BrightnessCube {
                    assert_eq!(h.state().ui.dock.height(crate::dock::Group::Color), height, "switching cubes retains the resized panel height");
                }
            }
            crate::dock::persist(h.state_mut(), &ctx);
            let prefs = h.state().session.prefs_to_json();
            let mut session = prism_engine::Session::new();
            session.load_prefs_json(&prefs).unwrap();
            let app = PrismApp::new(session, Default::default());
            assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HsbSliders);
        }
    }
}

#[test]
fn named_workspaces_restore_the_selected_color_mode_and_old_state_defaults_to_hue_cube() {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.ui.color_panel.mode = ColorPanelMode::ColorWheel;
    app.ui.color_panel.background = true;
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name":"Wheel workspace"})).unwrap();
    app.ui.color_panel.mode = ColorPanelMode::BrightnessCube;
    app.ui.color_panel.background = false;
    crate::menus::invoke(&mut app, &ctx, "window.workspace.select", json!({"name":"Wheel workspace"})).unwrap();
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::ColorWheel);
    assert!(app.ui.color_panel.background);
    let old: crate::state::ColorPanelState = serde_json::from_value(json!({"background":true})).unwrap();
    assert_eq!(old.mode, ColorPanelMode::HueCube);
    crate::dock::apply(&mut app, &json!({"colorPanel":{"mode":"unsupported"}}));
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::ColorWheel, "unreadable saved state does not replace the current setting");
}

#[test]
fn compact_wheel_fills_remaining_space_without_hex_rgb_readout() {
    for width in [250.0, 320.0, 520.0] {
        for scale in [1.0, 2.0] {
            let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
            app.ui.color_panel.mode = ColorPanelMode::ColorWheel;
            let mut h = Harness::builder()
                .with_size(vec2(width, 240.0))
                .with_pixels_per_point(scale)
                .build_ui_state(|ui, app: &mut PrismApp| show_mode(app, ui, ColorPanelMode::ColorWheel), app);
            PrismApp::setup_context(&h.ctx, ThemeKind::ProMedium);
            h.run_steps(4);
            let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 240.0));
            assert!(screen.contains_rect(h.get_by_label("Color wheel").rect()));
            for field in h.query_all_by_role(egui::accesskit::Role::SpinButton) {
                assert!(screen.contains_rect(field.rect()), "{width} @ {scale}: {:?}", field.rect());
            }
            assert_eq!(h.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 3, "only H/S/B fields remain");
            assert!(h.query_all_by_role(egui::accesskit::Role::TextInput).next().is_none(), "no HEX input");
            let wheel = h.get_by_label("Color wheel").rect();
            assert!(wheel.bottom() > screen.bottom() - 20.0, "wheel fills the space freed by the removed row");
        }
    }
}

#[test]
fn both_cubes_fill_resized_docks_without_numeric_readouts() {
    for theme in ThemeKind::ALL {
        for scale in [1.0, 2.0] {
            for (mode, label, slider) in [
                (ColorPanelMode::HueCube, "Hue cube field", "Hue cube slider"),
                (ColorPanelMode::BrightnessCube, "Brightness cube field", "Brightness cube slider"),
            ] {
                let mut h = Harness::builder().with_size(vec2(1200.0, 1000.0)).with_pixels_per_point(scale).build_eframe(move |cc| {
                    PrismApp::setup_context(&cc.egui_ctx, theme);
                    PrismApp::new(prism_engine::Session::new(), Default::default())
                });
                h.run_steps(4);
                let ctx = h.ctx.clone();
                let (req, _rx) = crate::control::ControlRequest::new(
                    "ui.set",
                    json!({"theme":theme.id(),"dockWidth":360,"dockTabs":{"color":usize::from(!matches!(theme, ThemeKind::Pro | ThemeKind::ProMedium))},
                        "colorPanel":{"mode":mode},"dock":{"collapsed":["properties","history","navigator"]}}),
                );
                crate::control::handle(h.state_mut(), &ctx, &req);
                let mut previous: Option<Rect> = None;
                for height in [160.0, 370.0, 560.0, 160.0] {
                    h.state_mut().ui.dock.heights.insert(crate::dock::Group::Color, height);
                    h.run_steps(8);
                    let group = crate::dock::last_rects(&ctx).into_iter().find(|(g, _)| *g == crate::dock::Group::Color).unwrap().1;
                    let field = h.get_by_label(label).rect();
                    let strip = h.get_by_label(slider).rect();
                    assert!(group.contains_rect(field.expand(5.0)) && group.contains_rect(strip), "{theme:?} {mode:?} {height}: {group:?} {field:?}");
                    assert!(field.height() > group.height() - 100.0, "fills the panel height: {field:?} in {group:?}");
                    assert_eq!((field.top(), field.bottom()), (strip.top(), strip.bottom()));
                    for role in [egui::accesskit::Role::TextInput, egui::accesskit::Role::SpinButton] {
                        assert!(h.query_all_by_role(role).all(|n| !group.intersects(n.rect())), "cubes have no HEX/RGB input row");
                    }
                    if let Some(prev) = previous {
                        assert!((prev.width() - field.width()).abs() < 1.0);
                        assert!((prev.height() - field.height()).abs() > 100.0, "resizing changes height independently of width");
                    }
                    previous = Some(field);
                }
            }
        }
    }
}
