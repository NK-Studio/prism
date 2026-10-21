use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Map, Value, json};

use super::*;
use crate::Services;

type Written = Rc<RefCell<Vec<String>>>;

/// An app with one document bound to an existing `.psd` on disk, and a writer that records paths.
fn app(name: &str) -> (PrismApp, Written, String) {
    let dir = std::env::temp_dir().join(format!("prism-overwrite-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("art.psd").to_string_lossy().into_owned();
    std::fs::write(&path, b"old").unwrap();
    let written: Written = Rc::default();
    let w = written.clone();
    let services = Services {
        export: Some(Box::new(|_doc: &prism_doc::Document, _path: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |p: &str, _b: &[u8]| {
            w.borrow_mut().push(p.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PrismApp::new(prism_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.session.active_mut().unwrap().path = Some(path.clone());
    app.sync_views();
    (app, written, path)
}

fn prompt(app: &PrismApp) -> Option<(u64, Map<String, Value>)> {
    app.ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| (d.id, d.fields.clone()))
}

#[test]
fn save_over_the_documents_own_file_asks_first() {
    let (mut app, written, path) = app("own");
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    let (id, f) = prompt(&app).expect("the Replace prompt");
    assert!(written.borrow().is_empty(), "nothing is written before the answer");
    assert_eq!(f.get(MARK).and_then(Value::as_str), Some(path.as_str()));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(*written.borrow(), vec![path.clone()]);
    assert!(prompt(&app).is_none());
}

#[test]
fn cancel_writes_nothing() {
    let (mut app, written, _) = app("cancel");
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    let (id, _) = prompt(&app).unwrap();
    crate::native_dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert!(written.borrow().is_empty());
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn dont_show_again_turns_the_preference_off() {
    let (mut app, written, path) = app("dont-ask");
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    let (id, _) = prompt(&app).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("dontAsk".into(), json!(true));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(!app.session.prefs().file_handling.ask_before_overwriting);
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert!(prompt(&app).is_none());
    assert_eq!(*written.borrow(), vec![path.clone(), path]);
}

#[test]
fn a_scripted_save_naming_the_path_does_not_ask() {
    let (mut app, written, path) = app("explicit");
    crate::menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({"path": path})).unwrap();
    assert!(prompt(&app).is_none());
    assert_eq!(*written.borrow(), vec![path]);
}

#[test]
fn the_save_window_asks_and_cancel_returns_to_it() {
    let (mut app, written, path) = app("window");
    let ctx = egui::Context::default();
    crate::save_ui::open(&mut app, false).unwrap();
    let window = app.ui.dialogs.iter().find(|d| crate::save_ui::owns(&d.fields)).unwrap().id;
    crate::dialogs::confirm(&mut app, window).unwrap();
    let (id, _) = prompt(&app).expect("the Replace prompt");
    assert!(written.borrow().is_empty());
    crate::native_dialogs::cancel(&mut app, &ctx, id).unwrap();
    let window = app.ui.dialogs.iter().find(|d| crate::save_ui::owns(&d.fields)).expect("the Save window is back").id;
    crate::dialogs::confirm(&mut app, window).unwrap();
    let (id, _) = prompt(&app).unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert!(crate::save_ui::same_file(&w[0], &path), "{w:?}");
    drop(w);
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn a_new_file_does_not_ask() {
    let (mut app, written, path) = app("new");
    let fresh = path.replace("art.psd", "fresh.psd");
    let _ = std::fs::remove_file(&fresh);
    app.session.active_mut().unwrap().path = Some(fresh.clone());
    crate::menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({})).unwrap();
    assert!(prompt(&app).is_none());
    assert_eq!(*written.borrow(), vec![fresh]);
}

#[test]
fn malformed_prompt_fields_fail_without_panicking() {
    let (mut app, _, _) = app("malformed");
    for f in [json!({MARK: 3}), json!({MARK: "x.psd"}), json!({MARK: "x.psd", "doc": 999_999}), json!({MARK: "x.psd", "doc": "a", "window": 1})] {
        assert!(confirm(&mut app, f.as_object().unwrap()).is_err(), "{f}");
    }
}
