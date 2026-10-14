//! Help › Check for Updates… and the once-a-day automatic check.
//!
//! The network, signature checks and installation live in the desktop app (`prism-updater`),
//! injected as [`Services::updater`](crate::Services::updater): this crate stays free of I/O.
//! The state machine is plain data in [`UiState::update`](crate::state::UiState::update) so
//! automation can read it:
//!
//! idle → checking → upToDate | available | failed; available → downloading → ready → (quit)
//! → the helper swaps the app and relaunches it.
//!
//! "Later" on a downloaded update keeps it armed: it installs the next time Prism quits.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::state::DialogKind;
use crate::theme::Tokens;

/// An update found by a check, as the UI shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub version: String,
    pub current: String,
    pub notes_url: Option<String>,
    /// Page for a manual download.
    pub download_url: Option<String>,
    /// Prism can download and install it itself.
    pub can_install: bool,
    /// Why it can't (portable copy, read-only location, too old a version…).
    pub note: Option<String>,
}

/// What the desktop updater reports back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateEvent {
    Checked(Result<Option<Offer>, String>),
    Progress { received: u64, total: u64 },
    /// Downloaded and verified; [`Updater::install_on_exit`] can run.
    Ready,
    Failed(String),
}

/// The desktop app's updater (see `apps/prism/src/updates.rs`). Every call returns at once;
/// results arrive through [`Updater::poll`].
pub trait Updater {
    /// Starts looking for a new version.
    fn check(&mut self);
    /// Downloads and verifies the update found by the last check.
    fn download(&mut self);
    /// Stops a download in progress.
    fn cancel(&mut self);
    fn poll(&mut self) -> Vec<UpdateEvent>;
    /// Starts the helper that installs the downloaded update once Prism has exited.
    fn install_on_exit(&mut self) -> Result<(), String>;
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available { offer: Offer },
    Downloading { offer: Offer, received: u64, total: u64 },
    Ready { offer: Offer },
    Failed { message: String },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUi {
    pub status: Status,
    /// The current check was asked for (Help menu): its outcome is always shown.
    pub manual: bool,
    /// The downloaded update installs when Prism quits.
    pub install_on_exit: bool,
    /// The automatic check ran (or was skipped) this session.
    #[serde(skip)]
    auto_done: bool,
    /// Quit now to install (set by "Restart to Update"; acted on in [`tick`], which has the context).
    #[serde(skip)]
    quit_requested: bool,
    /// Close the update dialog this frame ("Skip This Version").
    #[serde(skip)]
    close_requested: bool,
}

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__update")
}

fn dialog_open(app: &PrismApp) -> bool {
    app.ui.dialogs.iter().any(|d| owns(&d.fields))
}

fn open_dialog(app: &mut PrismApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields)) {
        return d.id;
    }
    let mut f = Map::new();
    f.insert("__update".into(), json!(true));
    f.insert("__label".into(), json!("Software Update"));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Help › Check for Updates…
pub fn check_now(app: &mut PrismApp) -> Result<Value, String> {
    let Some(updater) = app.services.updater.as_mut() else {
        return Err(tl!("Updates aren't available in this build").into());
    };
    match app.ui.update.status {
        // A download in progress or finished: show it rather than starting over.
        Status::Downloading { .. } | Status::Ready { .. } => {}
        _ => {
            updater.check();
            app.ui.update.status = Status::Checking;
        }
    }
    app.ui.update.manual = true;
    Ok(json!({"dialog": open_dialog(app), "update": app.ui.update.status}))
}

/// Days since 1970-01-01 → `YYYY-MM-DD` (proleptic Gregorian, UTC).
pub fn civil_date(days: i64) -> String {
    // Howard Hinnant's days-from-civil inverse.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    civil_date(i64::try_from(secs / 86_400).unwrap_or(0))
}

/// Once per frame: the daily automatic check, updater events, and quitting to install.
pub fn tick(app: &mut PrismApp, ctx: &egui::Context) {
    if !app.ui.update.auto_done {
        app.ui.update.auto_done = true;
        let prefs = app.session.prefs();
        let due = prefs.integrations.check_for_updates && prefs.updates.last_check != today();
        if due && let Some(updater) = app.services.updater.as_mut() {
            updater.check();
            app.ui.update.status = Status::Checking;
            app.ui.update.manual = false;
            let date = today();
            app.session.prefs.edit(|p| p.updates.last_check = date);
        }
    }
    let events = app.services.updater.as_mut().map(|u| u.poll()).unwrap_or_default();
    for event in events {
        apply(app, event);
    }
    if std::mem::take(&mut app.ui.update.close_requested) {
        let ids: Vec<u64> = app.ui.dialogs.iter().filter(|d| owns(&d.fields)).map(|d| d.id).collect();
        for id in ids {
            app.ui.close_dialog(id);
        }
    }
    if std::mem::take(&mut app.ui.update.quit_requested)
        && let Err(e) = crate::menus::invoke(app, ctx, "file.exit", Value::Null)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if matches!(app.ui.update.status, Status::Checking | Status::Downloading { .. }) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    } else if dialog_open(app) {
        // Its buttons may be pressed by automation later in this frame (after this tick): keep a
        // frame coming so "Restart to Update" and "Skip This Version" are acted on.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn apply(app: &mut PrismApp, event: UpdateEvent) {
    let manual = app.ui.update.manual;
    match event {
        UpdateEvent::Checked(Ok(Some(offer))) => {
            let skipped = app.session.prefs().updates.skipped_version == offer.version;
            app.ui.update.status = Status::Available { offer };
            if manual || !skipped {
                open_dialog(app);
            }
        }
        UpdateEvent::Checked(Ok(None)) => app.ui.update.status = Status::UpToDate,
        UpdateEvent::Checked(Err(message)) => {
            // An automatic check that can't reach the server stays quiet.
            if !manual {
                log::info!("update check: {message}");
            }
            app.ui.update.status = Status::Failed { message };
        }
        UpdateEvent::Progress { received, total } => {
            if let Status::Downloading { offer, .. } | Status::Available { offer } = &app.ui.update.status {
                app.ui.update.status = Status::Downloading { offer: offer.clone(), received, total };
            }
        }
        UpdateEvent::Ready => {
            if let Status::Downloading { offer, .. } | Status::Available { offer } = &app.ui.update.status {
                app.ui.update.status = Status::Ready { offer: offer.clone() };
                open_dialog(app);
            }
        }
        UpdateEvent::Failed(message) => {
            app.ui.update.status = Status::Failed { message };
            app.ui.update.manual = true;
            open_dialog(app);
        }
    }
}

/// Starts the installer if a downloaded update is armed. Called as Prism leaves (after the
/// unsaved-changes prompt is settled).
pub fn on_exit(app: &mut PrismApp) {
    if !app.ui.update.install_on_exit || !matches!(app.ui.update.status, Status::Ready { .. }) {
        return;
    }
    app.ui.update.install_on_exit = false;
    if let Some(updater) = app.services.updater.as_mut()
        && let Err(e) = updater.install_on_exit()
    {
        log::error!("couldn't start the update installer: {e}");
    }
}

/// Which buttons the dialog shows in its current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub ok: &'static str,
    pub ok_enabled: bool,
    /// `None`: OK only.
    pub cancel: Option<&'static str>,
}

pub fn buttons(f: &Map<String, Value>) -> Buttons {
    let state = f.get("__state").and_then(Value::as_str).unwrap_or("checking");
    let can_install = f.get("__canInstall").and_then(Value::as_bool).unwrap_or(false);
    match state {
        "checking" => Buttons { ok: tl!("Update"), ok_enabled: false, cancel: Some(tl!("Close")) },
        "available" if can_install => Buttons { ok: tl!("Update"), ok_enabled: true, cancel: Some(tl!("Later")) },
        "available" => Buttons { ok: tl!("Download…"), ok_enabled: true, cancel: Some(tl!("Later")) },
        "downloading" => Buttons { ok: tl!("Restart to Update"), ok_enabled: false, cancel: Some(tl!("Cancel")) },
        "ready" => Buttons { ok: tl!("Restart to Update"), ok_enabled: true, cancel: Some(tl!("Later")) },
        _ => Buttons { ok: tl!("OK"), ok_enabled: true, cancel: None },
    }
}

fn state_name(s: &Status) -> &'static str {
    match s {
        Status::Idle | Status::Checking => "checking",
        Status::UpToDate => "upToDate",
        Status::Available { .. } => "available",
        Status::Downloading { .. } => "downloading",
        Status::Ready { .. } => "ready",
        Status::Failed { .. } => "failed",
    }
}

pub fn body(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let status = app.ui.update.status.clone();
    f.insert("__state".into(), json!(state_name(&status)));
    let can_install = matches!(&status, Status::Available { offer } if offer.can_install);
    f.insert("__canInstall".into(), json!(can_install));
    let heading = |ui: &mut egui::Ui, text: &str| {
        ui.label(egui::RichText::new(text).font(crate::theme::semibold(14.0)).color(t.text));
        ui.add_space(4.0);
    };
    let dim = |ui: &mut egui::Ui, text: &str| {
        ui.label(egui::RichText::new(text).color(t.text_dim));
    };
    match &status {
        Status::Idle | Status::Checking => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label(tl!("Checking for updates…"));
            });
        }
        Status::UpToDate => {
            heading(ui, tl!("Prism is up to date"));
            dim(ui, &tl!("You have the latest version ({version}).").replace("{version}", prism_engine::build_info::VERSION));
        }
        Status::Failed { message } => {
            heading(ui, tl!("Couldn't update Prism"));
            dim(ui, message);
        }
        Status::Available { offer } => {
            heading(ui, &tl!("Prism {version} is available").replace("{version}", &offer.version));
            dim(ui, &tl!("You have version {version}.").replace("{version}", &offer.current));
            if let Some(note) = &offer.note {
                ui.add_space(4.0);
                dim(ui, note);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if let Some(url) = offer.notes_url.clone()
                    && ui.link(tl!("Release Notes")).clicked()
                {
                    crate::links::open(app, ui.ctx(), &url);
                }
                if ui.link(tl!("Skip This Version")).clicked() {
                    let v = offer.version.clone();
                    app.session.prefs.edit(|p| p.updates.skipped_version = v);
                    app.ui.update.close_requested = true;
                }
            });
        }
        Status::Downloading { offer, received, total } => {
            heading(ui, &tl!("Downloading Prism {version}…").replace("{version}", &offer.version));
            let frac = if *total > 0 { (*received as f64 / *total as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).show_percentage());
            dim(ui, &format!("{:.1} / {:.1} MB", *received as f64 / 1e6, *total as f64 / 1e6));
        }
        Status::Ready { offer } => {
            heading(ui, &tl!("Prism {version} is ready to install").replace("{version}", &offer.version));
            dim(ui, tl!("Prism will quit, update and open again. Choose Later to install it the next time you quit."));
        }
    }
    if let Some(e) = f.get("__error").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.colored_label(t.warning, e);
    }
}

/// OK: the state's main action. The dialog reopens for states that continue.
pub fn confirm(app: &mut PrismApp, _f: &Map<String, Value>) -> Result<Value, String> {
    match app.ui.update.status.clone() {
        Status::Available { offer } if offer.can_install => {
            let updater = app.services.updater.as_mut().ok_or("updates aren't available in this build")?;
            updater.download();
            app.ui.update.status = Status::Downloading { offer, received: 0, total: 0 };
            open_dialog(app);
        }
        Status::Available { offer } => {
            let url = offer.download_url.or(offer.notes_url).unwrap_or_else(|| crate::links::APP_PAGE.to_string());
            let opened = app.services.open_url.as_ref().map(|f| f(&url));
            if !matches!(opened, Some(Ok(()))) {
                app.ui.status = url.clone();
            }
        }
        Status::Ready { .. } => {
            app.ui.update.install_on_exit = true;
            app.ui.update.quit_requested = true;
        }
        Status::Checking | Status::Downloading { .. } => {
            open_dialog(app);
        }
        Status::Idle | Status::UpToDate | Status::Failed { .. } => {}
    }
    Ok(json!({"update": app.ui.update.status}))
}

/// Cancel / Later / Esc.
pub fn cancel(app: &mut PrismApp) {
    match app.ui.update.status.clone() {
        Status::Downloading { offer, .. } => {
            if let Some(u) = app.services.updater.as_mut() {
                u.cancel();
            }
            app.ui.update.status = Status::Available { offer };
        }
        // Later: install on the next quit.
        Status::Ready { .. } => app.ui.update.install_on_exit = true,
        _ => {}
    }
    if !dialog_open(app) {
        app.ui.update.manual = false;
    }
}

#[cfg(test)]
#[path = "update_ui_tests.rs"]
mod tests;
