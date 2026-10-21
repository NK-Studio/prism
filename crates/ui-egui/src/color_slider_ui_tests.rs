use super::*;
use crate::state::ColorPanelMode;
use crate::theme::ThemeKind;
use egui::Modifiers;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

fn harness(mode: ColorSliderMode) -> Harness<'static, PrismApp> {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.ui.color_panel.slider = mode;
    app.session.tools.foreground = [190.0 / 255.0, 100.0 / 255.0, 100.0 / 255.0, 1.0];
    let mut h = Harness::builder().with_size(vec2(300.0, 200.0)).build_ui_state(
        |ui, app: &mut PrismApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                body(app, ui);
            }
        },
        app,
    );
    PrismApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

/// Type `text` into the `index`th number field and leave it.
fn type_field(h: &mut Harness<'static, PrismApp>, index: usize, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(index).unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn bytes(c: [f32; 4]) -> [u8; 3] {
    srgb_bytes(c)
}

#[test]
fn hls_round_trips_and_survives_hostile_values() {
    for rgb in [[0.75, 0.39, 0.39], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.2, 0.6, 0.4], [0.5, 0.5, 0.5]] {
        let [h, l, s] = rgb_to_hls(rgb);
        let back = hls_to_rgb(h, l, s);
        for i in 0..3 {
            assert!((back[i] - rgb[i]).abs() < 1e-4, "{rgb:?} -> {back:?}");
        }
    }
    // Pure red is hue 0, lightness ½, full saturation; grey has none.
    assert_eq!(rgb_to_hls([1.0, 0.0, 0.0]), [0.0, 0.5, 1.0]);
    assert_eq!(rgb_to_hls([0.5, 0.5, 0.5])[2], 0.0);
    for bad in [f32::NAN, f32::INFINITY, -3.0, 9.0] {
        assert!(rgb_to_hls([bad, 0.2, bad]).iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        assert!(hls_to_rgb(bad, bad, bad).iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    }
}

#[test]
fn each_model_shows_three_fields_and_edits_the_foreground() {
    // RGB: the fields read the colour's bytes; G set to 200.
    let mut h = harness(ColorSliderMode::Rgb);
    let values: Vec<_> = h.query_all_by_role(egui::accesskit::Role::SpinButton).map(|n| n.value().unwrap_or_default()).collect();
    assert_eq!(values.len(), 3);
    assert!(values[0].starts_with("190") && values[1].starts_with("100"), "{values:?}");
    type_field(&mut h, 1, "200");
    assert_eq!(bytes(h.state().session.tools.foreground), [190, 200, 100]);
    // CMY is the complement: C = 255 - R.
    let mut h = harness(ColorSliderMode::Cmy);
    assert!(h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().value().unwrap_or_default().starts_with("65"));
    type_field(&mut h, 0, "255");
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 100, 100]);
    // HLS: lightness 100 is white whatever the hue.
    let mut h = harness(ColorSliderMode::Hls);
    type_field(&mut h, 1, "100");
    assert_eq!(bytes(h.state().session.tools.foreground), [255, 255, 255]);
    // Back down, the hue and saturation come back (they are remembered through white).
    type_field(&mut h, 1, "50");
    let [r, g, b] = bytes(h.state().session.tools.foreground);
    assert!(r > g && g == b, "the red hue survives white: {r} {g} {b}");
}

#[test]
fn model_tabs_switch_and_chips_pick_the_colour_edited() {
    let mut h = harness(ColorSliderMode::Rgb);
    for (label, mode) in [("HLS", ColorSliderMode::Hls), ("CMY", ColorSliderMode::Cmy), ("RGB", ColorSliderMode::Rgb)] {
        h.get_by_label(label).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.color_panel.slider, mode);
        assert_eq!(h.get_by_label(label).accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    }
    let foreground = h.state().session.tools.foreground;
    h.get_by_label("Background Color").click();
    h.run_steps(2);
    assert!(h.state().ui.color_panel.background);
    type_field(&mut h, 0, "0");
    assert_eq!(h.state().session.tools.foreground, foreground, "the background chip routes edits away from the foreground");
    assert_eq!(bytes(h.state().session.tools.background), [0, 255, 255]);
    h.get_by_label("Foreground Color").click();
    h.run_steps(2);
    assert!(!h.state().ui.color_panel.background);
}

#[test]
fn clip_studio_workspace_docks_color_slider_and_strips_the_color_panel_sliders() {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.ui.color_panel.mode = ColorPanelMode::HsbSliders;
    crate::menus::invoke(&mut app, &ctx, "window.workspace.clipStudio", json!({})).unwrap();
    assert!(crate::csp_tools::active(&app));
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::ColorWheel);
    let pro = crate::prefs_ui::layout(&app) == crate::theme::Layout::Compact;
    let group = app.ui.dock.find_panel(crate::dock_model::PanelId::ColorSlider, pro).expect("Color Slider is docked");
    assert!(app.ui.dock.columns.iter().any(|c| c.groups.contains(&group)));
    // The Color panel shows the wheel and chips, with no H/S/B sliders, even in HSB Sliders mode.
    for mode in [ColorPanelMode::ColorWheel, ColorPanelMode::HsbSliders] {
        app.ui.color_panel.mode = mode;
        let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(
            |ui, app: &mut PrismApp| crate::panels::color_panel_ui::show_mode(app, ui, app.ui.color_panel.mode),
            app,
        );
        PrismApp::setup_context(&h.ctx, ThemeKind::ProMedium);
        h.run_steps(4);
        assert!(h.query_by_label("Color wheel").is_some(), "{mode:?}");
        assert_eq!(h.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 0, "{mode:?}: no slider fields");
        app = h.into_state();
    }
    // Window › Color Slider toggles the panel off and back on.
    crate::menus::invoke(&mut app, &ctx, "window.panel.colorSlider", json!({})).unwrap();
    assert_eq!(crate::view_cmds::checked(&app, "window.panel.colorSlider"), Some(false));
    crate::menus::invoke(&mut app, &ctx, "window.panel.colorSlider", json!({})).unwrap();
    assert_eq!(crate::view_cmds::checked(&app, "window.panel.colorSlider"), Some(true));
    // Back in a Photoshop workspace the Color panel has its sliders again.
    app.ui.workspace = "Essentials".into();
    crate::menus::apply_workspace(&mut app);
    assert!(!crate::csp_tools::active(&app));
    app.ui.color_panel.mode = ColorPanelMode::HsbSliders;
    let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(
        |ui, app: &mut PrismApp| crate::panels::color_panel_ui::show_mode(app, ui, app.ui.color_panel.mode),
        app,
    );
    PrismApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(4);
    assert!(h.query_all_by_role(egui::accesskit::Role::SpinButton).count() >= 3);
}
