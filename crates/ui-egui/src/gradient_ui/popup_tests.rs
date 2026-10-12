//! The options bar's gradient picker: a popup holding the Gradients browser, driven with real
//! pointer and key input.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

/// The swatch (while the Gradient tool is active) and a button elsewhere, at 60 fps.
fn harness() -> Harness<'static, PrismApp> {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    app.ui.tool = Tool::Gradient;
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PrismApp| {
            if app.ui.tool == Tool::Gradient {
                ui.horizontal(|ui| preset_swatch(app, ui));
            }
            ui.add_space(400.0);
            let _ = ui.button("Elsewhere");
        },
        app,
    );
    PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    h
}

/// The picker is open while its footer's thumbnail size slider is there.
fn open(h: &Harness<'_, PrismApp>) -> bool {
    h.query_all_by_role(Role::Slider).next().is_some()
}

fn layers(h: &Harness<'_, PrismApp>) -> usize {
    h.state().session.active().unwrap().doc.layers.len()
}

fn presets(h: &Harness<'_, PrismApp>) -> Vec<String> {
    h.state().session.presets.gradients.iter().flat_map(|g| g.items.iter().map(|i| i.name.clone())).collect()
}

/// Real pointer input: `n` clicks at `pos`, each press and release a frame apart (2: a
/// double-click), well after any earlier click.
fn clicks_at(h: &mut Harness<'static, PrismApp>, pos: egui::Pos2, n: usize) {
    h.run_steps(40);
    h.hover_at(pos);
    h.run_steps(1);
    for _ in 0..n {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
    }
    h.run_steps(3);
}

fn click(h: &mut Harness<'static, PrismApp>, label: &str) {
    let at = h.get_by_label(label).rect().center();
    clicks_at(h, at, 1);
}

fn type_text(h: &mut Harness<'static, PrismApp>, text: &str) {
    for c in text.chars() {
        h.event(egui::Event::Text(c.to_string()));
        h.run();
    }
}

fn show(h: &mut Harness<'static, PrismApp>) {
    if !open(h) {
        click(h, "Gradients");
    }
    assert!(open(h), "the swatch opens the picker");
}

/// Clicks inside keep the picker open, as in Photoshop: a group's header (it used to close the
/// picker, the group opening unseen), the size slider, a preset (chosen for the tool, no layer
/// made), New and Delete.
#[test]
fn clicks_inside_keep_the_picker_open() {
    let mut h = harness();
    show(&mut h);
    assert!(h.query_by_label("Blue 01").is_none());
    click(&mut h, "Blues");
    assert!(open(&h) && h.query_by_label("Blue 01").is_some(), "the group opens in the picker");
    let slider = h.get_by_role(Role::Slider).rect().center();
    clicks_at(&mut h, slider, 1);
    assert!(open(&h), "size slider");
    click(&mut h, "Blue 01");
    assert!(open(&h), "a preset");
    assert_eq!(h.state().session.presets.gradient.name, "Blue 01");
    assert_eq!(layers(&h), 1, "choosing a gradient for the tool makes no layer");
    let before = presets(&h).len();
    click(&mut h, "Create new gradient");
    assert!(open(&h) && presets(&h).len() == before + 1, "New");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    click(&mut h, "Delete");
    assert!(open(&h) && presets(&h).len() == before, "Delete");
}

/// A double-click picks the gradient and closes the picker; it used to add a Gradient Fill
/// layer, as the Gradients panel's double-click does.
#[test]
fn a_double_click_chooses_and_closes() {
    let mut h = harness();
    show(&mut h);
    let at = h.get_by_label("Black, White").rect().center();
    clicks_at(&mut h, at, 2);
    assert!(!open(&h));
    assert_eq!(h.state().session.presets.gradient.name, "Black, White");
    assert_eq!(layers(&h), 1);
}

/// A preset's context menu works inside the picker: it used to replace the picker in egui's
/// one-popup memory, closing both. Rename… types the new name in place.
#[test]
fn renaming_from_the_context_menu_stays_in_the_picker() {
    let mut h = harness();
    show(&mut h);
    h.get_by_label("Black, White").click_secondary();
    h.run_steps(3);
    assert!(open(&h), "the context menu keeps the picker");
    click(&mut h, "Rename…");
    assert!(open(&h), "choosing in the context menu keeps the picker");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    type_text(&mut h, "Night");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(open(&h));
    assert!(presets(&h).contains(&"Night".to_string()) && !presets(&h).contains(&"Black, White".to_string()), "{:?}", presets(&h));
}

/// Making a group while a new preset's name is being typed renames the group: the two name
/// fields shared an id, so the first one losing focus cancelled the second.
#[test]
fn a_new_group_gets_named_after_a_new_preset() {
    let mut h = harness();
    show(&mut h);
    click(&mut h, "Create new gradient");
    assert!(h.state().ui.presets_ui.renaming.as_ref().is_some_and(|r| r.1 == "Custom"));
    click(&mut h, "Create new group");
    assert!(h.state().ui.presets_ui.renaming.as_ref().is_some_and(|r| r.1 == "group:Group"), "{:?}", h.state().ui.presets_ui.renaming);
    type_text(&mut h, " B");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().session.presets.gradients.iter().any(|g| g.name == "Group B"), "the group takes the typed name");
}

/// Escape, a click elsewhere, the swatch again or another tool close the picker, and a name
/// being typed in it is dropped rather than showing up when it reopens.
#[test]
fn the_picker_closes_and_forgets_a_rename() {
    let mut h = harness();
    show(&mut h);
    click(&mut h, "Create new gradient");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!open(&h), "Escape");
    assert!(h.state().ui.presets_ui.renaming.is_none());
    show(&mut h);
    click(&mut h, "Elsewhere");
    assert!(!open(&h), "a click elsewhere");
    show(&mut h);
    click(&mut h, "Gradients");
    assert!(!open(&h), "the swatch toggles it");
    // Another tool hides the swatch; the picker doesn't come back open with the Gradient tool.
    show(&mut h);
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    h.state_mut().ui.tool = Tool::Gradient;
    h.run_steps(3);
    assert!(!open(&h), "back to the Gradient tool");
}

/// Live mode: the options bar's Opacity changes the selected Gradient Fill layer, as Mode does
/// (a new live gradient takes it already); a scrub is one history step.
#[test]
fn live_opacity_changes_the_selected_fill_layer() {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    app.ui.tool = Tool::Gradient;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 20.0, y: 60.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Move { x: 180.0, y: 60.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 180.0, y: 60.0 }, m);
    let history = app.session.active().unwrap().history.past_len();
    for pct in [40.0, 30.0] {
        let before = app.ui.tool_options.clone();
        app.ui.tool_options.fill_opacity = pct;
        options_changed(&mut app, &before);
    }
    let st = app.session.active().unwrap();
    assert!((st.doc.layers.last().unwrap().opacity - 0.3).abs() < 1e-6);
    assert_eq!(st.history.past_len(), history + 1);
    // Classic mode paints pixels: the option is for the next stroke only.
    app.ui.tool_options.gradient_classic = true;
    let before = app.ui.tool_options.clone();
    app.ui.tool_options.fill_opacity = 80.0;
    options_changed(&mut app, &before);
    assert!((app.session.active().unwrap().doc.layers.last().unwrap().opacity - 0.3).abs() < 1e-6);
}
