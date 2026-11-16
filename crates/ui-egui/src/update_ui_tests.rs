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

/// An app whose automatic check isn't due unless `auto` is set.
fn app(auto: bool) -> (PrismApp, Rc<RefCell<Log>>, egui::Context) {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    let log = Rc::new(RefCell::new(Log::default()));
    app.services.updater = Some(Box::new(Fake(log.clone())));
    if !auto {
        app.ui.update.next_auto = Some(std::time::Instant::now() + RECHECK_EVERY);
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

/// Makes the automatic check due now, as if `RECHECK_EVERY` had passed.
fn time_passes(a: &mut PrismApp) {
    a.ui.update.next_auto = Some(std::time::Instant::now());
}

#[test]
fn checks_at_every_launch_and_while_open() {
    let (mut a, log, ctx) = app(true);
    // An earlier launch today already checked: this launch checks again anyway.
    a.session.prefs.edit(|p| p.updates.last_check = today());
    tick(&mut a, &ctx);
    tick(&mut a, &ctx);
    assert_eq!(log.borrow().calls, ["check"]);
    assert_eq!(a.session.prefs().updates.last_check, today());
    assert!(a.ui.update.next_auto.is_some_and(|at| at > std::time::Instant::now()), "next check scheduled");

    push(&log, UpdateEvent::Checked(Ok(None)));
    tick(&mut a, &ctx);
    time_passes(&mut a);
    tick(&mut a, &ctx);
    assert_eq!(log.borrow().calls, ["check", "check"], "checks again while open");

    let (mut a, log, ctx) = app(false);
    tick(&mut a, &ctx);
    assert!(log.borrow().calls.is_empty(), "not due yet");

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
fn later_isnt_nagged_until_the_next_launch() {
    let (mut a, log, ctx) = app(true);
    tick(&mut a, &ctx);
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    tick(&mut a, &ctx);
    let id = update_dialog(&a).unwrap();
    // Later.
    a.ui.close_dialog(id);
    cancel(&mut a);

    time_passes(&mut a);
    tick(&mut a, &ctx);
    assert_eq!(log.borrow().calls, ["check", "check"]);
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    tick(&mut a, &ctx);
    assert!(update_dialog(&a).is_none(), "the same version stays quiet this session");

    // A newer one is announced.
    time_passes(&mut a);
    tick(&mut a, &ctx);
    push(&log, UpdateEvent::Checked(Ok(Some(Offer { version: "9.1.0".into(), ..offer(true) }))));
    tick(&mut a, &ctx);
    assert!(update_dialog(&a).is_some(), "a newer version pops up");

    // A manual check always shows it.
    let (mut a, log, ctx) = app(false);
    a.ui.update.announced = "9.0.0".into();
    check_now(&mut a).unwrap();
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    tick(&mut a, &ctx);
    assert!(update_dialog(&a).is_some());
}

#[test]
fn no_automatic_check_over_a_download() {
    let (mut a, log, ctx) = app(false);
    check_now(&mut a).unwrap();
    push(&log, UpdateEvent::Checked(Ok(Some(offer(true)))));
    tick(&mut a, &ctx);
    let id = update_dialog(&a).unwrap();
    crate::dialogs::confirm(&mut a, id).unwrap();
    time_passes(&mut a);
    tick(&mut a, &ctx);
    assert_eq!(log.borrow().calls, ["check", "download"], "the download isn't interrupted");
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

#[test]
fn icon_downscale_averages_blocks_and_survives_bad_input() {
    // 4×2 → 2×1: an opaque white block, and a block that is half transparent black.
    let white = [255, 255, 255, 255];
    let clear = [0, 0, 0, 0];
    let rgba: Vec<u8> = [white, white, white, clear, white, white, clear, white].concat();
    let image = downscale(&rgba, 4, 2, 2).unwrap();
    assert_eq!(image.size, [2, 1]);
    assert_eq!(image.pixels[0], egui::Color32::WHITE);
    // Alpha-weighted: the clear pixels thin the block out, they don't darken it.
    assert_eq!(image.pixels[1], egui::Color32::from_rgba_premultiplied(127, 127, 127, 127));

    assert!(downscale(&rgba, 4, 3, 2).is_none(), "fewer bytes than the size says");
    assert!(downscale(&[], 0, 0, 256).is_none());
    assert!(downscale(&rgba, usize::MAX, 2, 2).is_none(), "an absurd size doesn't overflow");
    assert!(downscale(&rgba, 4, 2, 0).is_none(), "a zero target doesn't divide by zero");
}

#[test]
fn headline_names_the_version_and_asks() {
    let (title, sub) = headline(&Status::Available { offer: offer(true) });
    assert!(title.contains("9.0.0"), "{title}");
    assert_eq!(sub, tl!("Do you want to update now?"));

    // Why Prism can't install it replaces the question.
    let manual = Offer { note: Some("Portable copy.".into()), ..offer(false) };
    assert_eq!(headline(&Status::Available { offer: manual }).1, "Portable copy.");
    assert_eq!(headline(&Status::Failed { message: "offline".into() }).1, "offline");
    assert!(headline(&Status::UpToDate).1.contains(prism_engine::build_info::VERSION));
}

#[test]
fn the_window_draws_in_every_state() {
    let states = [
        Status::Checking,
        Status::UpToDate,
        Status::Available { offer: offer(true) },
        Status::Available { offer: Offer { note: Some("Portable copy.".into()), ..offer(false) } },
        Status::Downloading { offer: offer(true), received: 0, total: 0 },
        Status::Downloading { offer: offer(true), received: u64::MAX, total: 1 },
        Status::Ready { offer: offer(true) },
        Status::Failed { message: "x".repeat(4000) },
    ];
    for status in states {
        let (mut a, _log, ctx) = app(false);
        PrismApp::setup_context(&ctx, Default::default());
        a.ui.update.status = status;
        open_dialog(&mut a);
        for _ in 0..3 {
            ctx.run_ui(Default::default(), |ui| crate::dialogs::show(&mut a, ui.ctx())).textures_delta.clear();
        }
        assert!(update_dialog(&a).is_some());
    }
}
