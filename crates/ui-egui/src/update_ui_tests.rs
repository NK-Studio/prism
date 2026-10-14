use std::cell::RefCell;
use std::rc::Rc;

use super::*;

#[derive(Default)]
struct Log {
    calls: Vec<&'static str>,
    queued: Vec<UpdateEvent>,
}

struct Fake(Rc<RefCell<Log>>);

impl Updater for Fake {
    fn check(&mut self) {
        self.0.borrow_mut().calls.push("check");
    }
    fn download(&mut self) {
        self.0.borrow_mut().calls.push("download");
    }
    fn cancel(&mut self) {
        self.0.borrow_mut().calls.push("cancel");
    }
    fn poll(&mut self) -> Vec<UpdateEvent> {
        std::mem::take(&mut self.0.borrow_mut().queued)
    }
    fn install_on_exit(&mut self) -> Result<(), String> {
        self.0.borrow_mut().calls.push("install");
        Ok(())
    }
}

fn offer(can_install: bool) -> Offer {
    Offer { version: "9.0.0".into(), current: "1.0.0".into(), can_install, download_url: Some("https://example.com".into()), ..Default::default() }
}

/// An app whose automatic check already ran today unless `auto` is set.
fn app(auto: bool) -> (PrismApp, Rc<RefCell<Log>>, egui::Context) {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    let log = Rc::new(RefCell::new(Log::default()));
    app.services.updater = Some(Box::new(Fake(log.clone())));
    if !auto {
        let d = today();
        app.session.prefs.edit(|p| p.updates.last_check = d);
    }
    (app, log, egui::Context::default())
}

fn push(log: &Rc<RefCell<Log>>, e: UpdateEvent) {
    log.borrow_mut().queued.push(e);
}

fn update_dialog(app: &PrismApp) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| d.id)
}

#[test]
fn civil_dates() {
    assert_eq!(civil_date(0), "1970-01-01");
    assert_eq!(civil_date(19_723), "2024-01-01");
    assert_eq!(civil_date(20_739), "2026-10-13");
    assert_eq!(civil_date(-1), "1969-12-31");
}

#[test]
fn checks_once_a_day_and_only_when_enabled() {
    let (mut a, log, ctx) = app(true);
    tick(&mut a, &ctx);
    tick(&mut a, &ctx);
    assert_eq!(log.borrow().calls, ["check"]);
    assert_eq!(a.session.prefs().updates.last_check, today());

    let (mut a, log, ctx) = app(false);
    tick(&mut a, &ctx);
    assert!(log.borrow().calls.is_empty(), "already checked today");

    let (mut a, log, ctx) = app(true);
    a.session.prefs.edit(|p| p.integrations.check_for_updates = false);
    tick(&mut a, &ctx);
    assert!(log.borrow().calls.is_empty(), "turned off in Preferences");
}

#[test]
fn quiet_automatic_checks() {
    let (mut a, log, ctx) = app(true);
    tick(&mut a, &ctx);
    push(&log, UpdateEvent::Checked(Err("offline".into())));
    tick(&mut a, &ctx);
    assert!(update_dialog(&a).is_none(), "no dialog for a failed automatic check");

    let (mut a, log, ctx) = app(true);
    a.session.prefs.edit(|p| p.updates.skipped_version = "9.0.0".into());
    tick(&mut a, &ctx);
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    tick(&mut a, &ctx);
    assert!(update_dialog(&a).is_none(), "skipped version stays quiet");

    let (mut a, log, ctx) = app(true);
    tick(&mut a, &ctx);
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    tick(&mut a, &ctx);
    assert!(update_dialog(&a).is_some(), "a new version is announced");
}

#[test]
fn download_then_install_on_quit() {
    let (mut a, log, ctx) = app(false);
    check_now(&mut a).unwrap();
    assert!(matches!(a.ui.update.status, Status::Checking));
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    tick(&mut a, &ctx);
    let id = update_dialog(&a).unwrap();
    crate::dialogs::confirm(&mut a, id).unwrap();
    assert!(log.borrow().calls.contains(&"download"));
    assert!(matches!(a.ui.update.status, Status::Downloading { .. }));
    assert!(update_dialog(&a).is_some(), "the dialog shows the progress");

    push(&log, UpdateEvent::Progress { received: 5, total: 10 });
    tick(&mut a, &ctx);
    assert!(matches!(a.ui.update.status, Status::Downloading { received: 5, total: 10, .. }));
    push(&log, UpdateEvent::Ready);
    tick(&mut a, &ctx);
    assert!(matches!(a.ui.update.status, Status::Ready { .. }));

    // Later: armed for the next quit, not installed yet.
    let id = update_dialog(&a).unwrap();
    a.ui.close_dialog(id);
    cancel(&mut a);
    assert!(a.ui.update.install_on_exit);
    assert!(!log.borrow().calls.contains(&"install"));
    on_exit(&mut a);
    assert!(log.borrow().calls.contains(&"install"));
    assert!(!a.ui.update.install_on_exit, "only once");
}

#[test]
fn restart_to_update_quits() {
    let (mut a, log, ctx) = app(false);
    check_now(&mut a).unwrap();
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    push(&log, UpdateEvent::Ready);
    tick(&mut a, &ctx);
    let id = update_dialog(&a).unwrap();
    crate::dialogs::confirm(&mut a, id).unwrap();
    assert!(a.ui.update.install_on_exit && a.ui.update.quit_requested);
    tick(&mut a, &ctx);
    assert!(!a.ui.update.quit_requested, "quit was sent");
}

#[test]
fn cancelling_a_download_goes_back_to_the_offer() {
    let (mut a, log, ctx) = app(false);
    check_now(&mut a).unwrap();
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    tick(&mut a, &ctx);
    let id = update_dialog(&a).unwrap();
    crate::dialogs::confirm(&mut a, id).unwrap();
    cancel(&mut a);
    assert!(log.borrow().calls.contains(&"cancel"));
    assert!(matches!(a.ui.update.status, Status::Available { .. }));
    on_exit(&mut a);
    assert!(!log.borrow().calls.contains(&"install"));
}

#[test]
fn manual_only_offers_open_the_download_page() {
    let (mut a, log, ctx) = app(false);
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let o = opened.clone();
    a.services.open_url = Some(Box::new(move |u: &str| {
        o.borrow_mut().push(u.to_string());
        Ok(())
    }));
    check_now(&mut a).unwrap();
    push(&log, UpdateEvent::Checked(Ok(Some(offer(false)))));
    tick(&mut a, &ctx);
    let id = update_dialog(&a).unwrap();
    crate::dialogs::confirm(&mut a, id).unwrap();
    assert_eq!(opened.borrow().as_slice(), ["https://example.com"]);
    assert!(!log.borrow().calls.contains(&"download"));
}

#[test]
fn without_an_updater_the_command_explains() {
    let mut a = PrismApp::new(prism_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    assert!(crate::menus::invoke(&mut a, &ctx, "help.checkForUpdates", Value::Null).is_err());
    tick(&mut a, &ctx);
    on_exit(&mut a);
    assert!(matches!(a.ui.update.status, Status::Idle));
}

#[test]
fn buttons_follow_the_state() {
    let f = |state: &str, can: bool| {
        let mut m = Map::new();
        m.insert("__state".into(), json!(state));
        m.insert("__canInstall".into(), json!(can));
        buttons(&m)
    };
    assert!(!f("checking", false).ok_enabled);
    assert!(f("available", true).ok_enabled);
    assert!(!f("downloading", true).ok_enabled);
    assert_eq!(f("upToDate", false).cancel, None);
    assert_eq!(f("failed", false).cancel, None);
}
