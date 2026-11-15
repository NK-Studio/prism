use egui::{Key, Modifiers, vec2};
use egui_kittest::{
    Harness,
    kittest::{Queryable, Role},
};
use serde_json::json;

use super::*;
use crate::state::DialogKind;

fn app() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    app
}

fn layers(app: &PrismApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn active(app: &PrismApp) -> &prism_doc::Layer {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap()
}

/// The whole app, as the shortcuts reach it.
fn shell(app: PrismApp) -> Harness<'static, PrismApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(8);
    h
}

/// Just the dialogs, for layout and typing, with the New Layer dialog open (opened once the
/// fonts are set up: the harness draws a frame while it is built).
fn dialogs(app: PrismApp) -> Harness<'static, PrismApp> {
    let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut()).unwrap();
    h.run_steps(4);
    h
}

/// A shortcut press with the platform's modifiers (`Cmd` is ⌘ on the Mac, Ctrl elsewhere).
fn press(h: &mut Harness<'_, PrismApp>, sc: &str) {
    let sc = crate::shortcuts::parse(sc).unwrap();
    let mut m = Modifiers { alt: sc.modifiers.alt, shift: sc.modifiers.shift, command: true, ..Default::default() };
    if cfg!(target_os = "macos") {
        m.mac_cmd = true;
    } else {
        m.ctrl = true;
    }
    h.event(egui::Event::ModifiersChanged(m));
    h.event(egui::Event::Key { key: sc.logical_key, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.event(egui::Event::Key { key: sc.logical_key, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

/// Photoshop's three N shortcuts: ⌥⇧⌘N makes a layer straight away, ⇧⌘N asks in the New Layer
/// dialog first, ⌘N opens New Document.
#[test]
fn the_n_shortcuts_make_a_layer_ask_for_one_or_open_new_document() {
    let mut h = shell(app());
    assert_eq!(layers(h.state()), 1);

    press(&mut h, "Cmd+Alt+Shift+N");
    assert_eq!(layers(h.state()), 2, "the layer is there, no questions asked");
    assert!(h.state().ui.dialogs.is_empty(), "no dialog");
    assert_eq!(active(h.state()).name, "Layer 1");

    press(&mut h, "Cmd+Shift+N");
    assert_eq!(layers(h.state()), 2, "nothing is made until OK");
    let d = h.state().ui.dialogs.first().cloned().expect("the New Layer dialog");
    assert!(owns(&d.fields), "{:?}", d.fields);
    assert_eq!(crate::dialogs::title(&d), "New Layer");
    assert_eq!(d.fields["name"], "Layer 2", "the next free name is offered");
    // The dialog owns the keyboard: the same key again makes nothing behind it.
    press(&mut h, "Cmd+Alt+Shift+N");
    assert_eq!(layers(h.state()), 2);
    crate::dialogs::confirm(h.state_mut(), d.id).unwrap();
    assert_eq!(layers(h.state()), 3);
    assert_eq!(active(h.state()).name, "Layer 2");
    h.run_steps(2);

    press(&mut h, "Cmd+N");
    assert_eq!(h.state().ui.dialogs.first().map(|d| d.kind), Some(DialogKind::NewDocument));
    assert_eq!(layers(h.state()), 3);
}

/// The menu item (Layer › New › Layer…) and the canvas context menu's New Layer… open the
/// dialog; a programmatic call and the Layers panel's button keep making the layer directly.
#[test]
fn the_menu_item_asks_and_the_engine_command_does_not() {
    let mut app = app();
    let ctx = egui::Context::default();
    let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert_eq!(r["dialog"], app.ui.dialogs[0].id);
    assert_eq!(layers(&app), 1);
    app.ui.dialogs.clear();
    // With params it is the engine's command (actions, automation).
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"name": "Ink"})).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(active(&app).name, "Ink");
    app.run(COMMAND, json!({})).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(layers(&app), 3);
    crate::menus::invoke(&mut app, &ctx, NO_DIALOG, json!({})).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(layers(&app), 4);
}

#[test]
fn without_a_document_nothing_opens() {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    assert!(!crate::menus::is_enabled(&app, COMMAND));
    assert!(!crate::menus::is_enabled(&app, NO_DIALOG));
    assert!(crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).is_err());
    assert!(crate::menus::invoke(&mut app, &ctx, NO_DIALOG, json!({})).is_err());
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn ok_makes_the_layer_with_every_option_in_one_history_step() {
    let mut app = app();
    let id = open(&mut app).unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    for (k, v) in
        [("name", json!("Shade")), ("color", json!("violet")), ("clipped", json!(true)), ("mode", json!("overlay")), ("opacity", json!(35.0)), ("fillNeutral", json!(true))]
    {
        d.fields.insert(k.into(), v);
    }
    crate::dialogs::confirm(&mut app, id).unwrap();
    let l = active(&app);
    assert_eq!((l.name.as_str(), l.label, l.clipped, l.blend), ("Shade", LabelColor::Violet, true, BlendMode::Overlay));
    assert!((l.opacity - 0.35).abs() < 1e-6);
    let px = l.surface().unwrap().rgba(20, 15);
    assert!(px.iter().zip([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]).all(|(a, b)| (a - b).abs() < 1.5 / 255.0), "50% gray: {px:?}");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(layers(&app), 1, "one undo takes the layer away");
}

/// Fields an agent (or a stale dialog) left in a bad state still make a layer, never an error
/// the user can't act on: a blank name is the default one, a mode without a neutral colour
/// ignores the fill, an unknown mode or colour is Photoshop's default.
#[test]
fn odd_fields_fall_back_to_the_defaults() {
    let mut f = fields(&app());
    for (k, v) in
        [("name", json!("   ")), ("color", json!("plaid")), ("mode", json!("passThrough")), ("opacity", json!(1e300)), ("fillNeutral", json!(true)), ("clipped", json!(7))]
    {
        f.insert(k.into(), v);
    }
    assert_eq!(params(&f), json!({"color": "none", "clipped": false, "blend": "normal", "opacity": 1.0, "fillNeutral": false}));
    f.insert("opacity".into(), json!("lots"));
    f.insert("mode".into(), json!(3));
    assert_eq!(params(&f)["opacity"], 1.0);
    let mut app = app();
    let id = open(&mut app).unwrap();
    app.ui.dialog_mut(id).unwrap().fields = f;
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(active(&app).name, "Layer 1");
}

/// Photoshop's layout: Name and Color on one row, the clipping checkbox, Mode and Opacity, the
/// neutral-colour checkbox, and OK above Cancel in a column at the right.
#[test]
fn the_dialog_is_laid_out_like_photoshops() {
    let mut h = dialogs(app());
    let rect = |h: &Harness<'_, PrismApp>, label: &str| h.get_by_label(label).rect();
    // The row labels (the Opacity field's pop-up slider button carries the same name).
    let row = |h: &Harness<'_, PrismApp>, label: &str| h.get_by_role_and_label(Role::Label, label).rect();
    let (name, color, clip) = (row(&h, "Name"), row(&h, "Color"), rect(&h, "Use Previous Layer to Create Clipping Mask"));
    let (mode, opacity, fill) = (row(&h, "Mode"), row(&h, "Opacity"), rect(&h, "Fill with neutral color"));
    let (ok, cancel) = (rect(&h, "OK"), rect(&h, "Cancel"));
    assert!(h.query_by_label("New Layer").is_some(), "titled New Layer");
    assert!((name.center().y - color.center().y).abs() < 2.0, "Name and Color share a row");
    assert!(color.left() > name.right());
    assert!(clip.top() > name.bottom() - 1.0 && mode.top() > clip.bottom() - 1.0 && fill.top() > mode.bottom() - 1.0, "rows in Photoshop's order");
    assert!((mode.center().y - opacity.center().y).abs() < 2.0, "Mode and Opacity share a row");
    assert!((name.right() - mode.right()).abs() < 1.0, "row labels end on one line");
    assert!(clip.left() > name.right() && (clip.left() - fill.left()).abs() < 1.0, "checkboxes line up under the fields");
    assert!(ok.left() > color.right() && ok.left() > opacity.right(), "the buttons are a column at the right");
    assert!(cancel.top() > ok.bottom() && (cancel.left() - ok.left()).abs() < 1.0 && (cancel.width() - ok.width()).abs() < 1.0);
    assert!((ok.top() - name.top()).abs() < 8.0, "OK starts on the first row");

    // A mode with a neutral colour names it, and the checkbox counts.
    h.state_mut().ui.dialogs[0].fields.insert("mode".into(), json!("overlay"));
    h.run_steps(2);
    assert!(h.query_by_label("Fill with neutral color").is_none());
    h.get_by_label("Fill with Overlay-neutral color (50% gray)").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs[0].fields["fillNeutral"], true);
    // Back on Normal the checkbox is greyed and its tick no longer applies.
    h.state_mut().ui.dialogs[0].fields.insert("mode".into(), json!("normal"));
    h.run_steps(2);
    assert!(h.query_by_label("Fill with neutral color").is_some());
    assert_eq!(params(&h.state().ui.dialogs[0].fields)["fillNeutral"], false);
}

#[test]
fn the_dialog_is_translated() {
    let lang = crate::i18n::Lang::from_code("ko").expect("registered language");
    let _language = crate::i18n::language_scope(lang);
    let mut h = dialogs(app());
    h.state_mut().ui.dialogs[0].fields.insert("mode".into(), json!("multiply"));
    h.run_steps(2);
    for label in ["새 레이어", "이름", "색상", "모드", "불투명도", "이전 레이어를 사용하여 클리핑 마스크 만들기", "곱하기-중성 색상으로 채우기 (흰색)"] {
        assert!(h.query_all_by_label(label).next().is_some(), "{label}");
    }
}

/// The dialog opens with the name focused and selected: typing replaces it, Enter is OK.
#[test]
fn typing_replaces_the_offered_name_and_enter_confirms() {
    let mut h = dialogs(app());
    h.event(egui::Event::Text("Ink".into()));
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs[0].fields["name"], "Ink");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty(), "Enter closed the dialog");
    assert_eq!(active(h.state()).name, "Ink");
    assert_eq!(layers(h.state()), 2);
}

/// Esc (and Cancel) leave the document alone.
#[test]
fn escape_and_cancel_make_nothing() {
    let mut h = dialogs(app());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(layers(h.state()), 1);
    open(h.state_mut()).unwrap();
    h.run_steps(4);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(layers(h.state()), 1);
}
