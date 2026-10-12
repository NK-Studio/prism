use serde_json::json;

use super::*;
use crate::Services;

/// An app as the desktop builds it: `prefs` is what the preferences file holds (`None`: no file).
fn app_with_prefs(prefs: Option<&'static str>) -> PrismApp {
    let services = Services { load_prefs: Some(Box::new(move || prefs.map(str::to_string))), ..Default::default() };
    PrismApp::new(prism_engine::Session::new(), services)
}

fn setup(app: &PrismApp) -> Option<(u64, Map<String, Value>)> {
    app.ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| (d.id, d.fields.clone()))
}

#[test]
fn it_opens_only_on_a_true_first_launch() {
    assert!(setup(&app_with_prefs(None)).is_some(), "no preferences file: first launch");
    // A file from before the setup existed: its owner already uses Prism.
    assert!(setup(&app_with_prefs(Some(r#"{"interface":{"uiScale":"125"}}"#))).is_none());
    assert!(setup(&app_with_prefs(Some(r#"{"setupComplete":true}"#))).is_none());
    // Tests and headless runs (no preferences loader) never see it.
    assert!(setup(&PrismApp::new(prism_engine::Session::new(), Services::default())).is_none());
    assert!(!prism_engine::prefs::Preferences::default().setup_complete, "fresh preferences start not set up");
}

#[test]
fn skipping_or_finishing_marks_it_done_for_good() {
    for finish_it in [false, true] {
        let mut app = app_with_prefs(None);
        let (id, _) = setup(&app).unwrap();
        if finish_it {
            app.ui.dialog_mut(id).unwrap().fields.insert("step".into(), json!(1));
            crate::dialogs::confirm(&mut app, id).unwrap();
        } else {
            app.ui.close_dialog(id);
            finish(&mut app);
        }
        assert!(app.session.prefs().setup_complete);
        // Saved and loaded again: no setup on the next launch.
        let saved = app.session.prefs_to_json();
        let leaked: &'static str = Box::leak(saved.into_boxed_str());
        assert!(setup(&app_with_prefs(Some(leaked))).is_none(), "finish={finish_it}");
    }
}

#[test]
fn the_steps_apply_language_and_theme_at_once_and_move_with_next_and_back() {
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), PrismApp::new(prism_engine::Session::new(), Services::default()));
    crate::PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    open(h.state_mut());
    h.run_steps(3);
    // Step 1: picking 한국어 switches the interface language now.
    h.get_by_label("한국어").click();
    h.run_steps(2);
    assert_eq!(h.state().session.prefs().interface.language, "ko");
    h.get_by_label("Next").click();
    h.run_steps(2);
    assert_eq!(step(&setup(h.state()).unwrap().1), 1, "Next moves on without closing");
    // Step 2: a theme card applies the theme now; Back returns to the languages.
    h.get_by_label("Studio (Light)").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.theme, ThemeKind::StudioLight);
    h.get_by_label("Back").click();
    h.run_steps(2);
    assert_eq!(step(&setup(h.state()).unwrap().1), 0);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(step(&setup(h.state()).unwrap().1), 1, "Enter is Next on the first step");
    h.get_by_label("Get Started").click();
    h.run_steps(2);
    assert!(setup(h.state()).is_none());
    assert!(h.state().session.prefs().setup_complete);
}

#[test]
fn help_reopens_it_and_every_step_draws_in_every_theme() {
    let mut app = PrismApp::new(prism_engine::Session::new(), Services::default());
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "help.firstRunSetup", json!({})).unwrap();
    let (id, _) = setup(&app).unwrap();
    crate::PrismApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    for kind in ThemeKind::ALL {
        app.set_theme(&ctx, kind);
        for s in [0, 1, 7] {
            app.ui.dialog_mut(id).unwrap().fields.insert("step".into(), json!(s));
            for _ in 0..2 {
                let mut out = ctx.run_ui(Default::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
                out.textures_delta.clear();
            }
        }
    }
}
