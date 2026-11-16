//! Help › Check for Updates… and the automatic check: at every launch, then every
//! [`RECHECK_EVERY`] while Prism stays open, so a release published mid-session still pops up.
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

/// How often a running Prism looks again (people leave an editor open for days).
pub const RECHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(4 * 60 * 60);

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
    /// When the next automatic check is due; `None` before the first (at launch).
    #[serde(skip)]
    next_auto: Option<std::time::Instant>,
    /// The version an automatic check already announced this session: a later automatic check
    /// that finds it again stays quiet ("Later" means not now, not every few hours).
    #[serde(skip)]
    announced: String,
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

/// The automatic check is due and nothing is in the way of it.
fn auto_check_due(app: &PrismApp, now: std::time::Instant) -> bool {
    let due = app.ui.update.next_auto.is_none_or(|at| now >= at);
    // Not over a check, download or install in progress, nor under an open update dialog.
    let busy = matches!(app.ui.update.status, Status::Checking | Status::Downloading { .. } | Status::Ready { .. }) || dialog_open(app);
    due && !busy
}

/// Once per frame: the automatic check, updater events, and quitting to install.
pub fn tick(app: &mut PrismApp, ctx: &egui::Context) {
    let now = std::time::Instant::now();
    if auto_check_due(app, now) {
        app.ui.update.next_auto = Some(now + RECHECK_EVERY);
        let enabled = app.session.prefs().integrations.check_for_updates;
        if enabled && let Some(updater) = app.services.updater.as_mut() {
            updater.check();
            app.ui.update.status = Status::Checking;
            app.ui.update.manual = false;
            let date = today();
            app.session.prefs.edit(|p| p.updates.last_check = date);
        }
    }
    if let Some(at) = app.ui.update.next_auto {
        ctx.request_repaint_after(at.saturating_duration_since(now));
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
            let announced = app.ui.update.announced == offer.version;
            if manual || !(skipped || announced) {
                if !manual {
                    app.ui.update.announced = offer.version.clone();
                }
                open_dialog(app);
            }
            app.ui.update.status = Status::Available { offer };
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

/// Content width of the update window.
pub const WIDTH: f32 = 404.0;
/// The app icon beside the headline (points).
const ICON: f32 = 56.0;
/// The icon file keeps the macOS template's clear margin (100 of 1024 px a side); it is cropped
/// away so the artwork lines up with the window's left edge.
const ICON_MARGIN: f32 = 100.0 / 1024.0;
const ICON_GAP: f32 = 16.0;

/// The app icon, box-filtered down once (the 1024 px original shimmers when drawn this small).
fn icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("prism-update-icon");
    if let Some(cached) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return cached;
    }
    let texture = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/app-icon/prism-1024.png"))
        .inspect_err(|e| log::warn!("Prism update icon: {e}"))
        .ok()
        .and_then(|icon| downscale(&icon.rgba, icon.width as usize, icon.height as usize, 256))
        .map(|image| ctx.load_texture("prism-update-icon", image, egui::TextureOptions::LINEAR));
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

/// Averages whole blocks of an unmultiplied RGBA image down to about `target` pixels wide
/// (alpha-weighted, so transparent pixels don't darken the edge).
fn downscale(rgba: &[u8], w: usize, h: usize, target: usize) -> Option<egui::ColorImage> {
    let factor = (w / target.max(1)).max(1);
    let (ow, oh) = (w / factor, h / factor);
    if ow == 0 || oh == 0 || rgba.len() < w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let n = u64::try_from(factor.checked_mul(factor)?).ok()?;
    let mut out = Vec::with_capacity(ow * oh * 4);
    for oy in 0..oh {
        for ox in 0..ow {
            let mut sum = [0u64; 4];
            for dy in 0..factor {
                let at = ((oy * factor + dy) * w + ox * factor) * 4;
                for &[r, g, b, a] in rgba.get(at..at + factor * 4)?.as_chunks::<4>().0 {
                    let a = u64::from(a);
                    sum = [sum[0] + u64::from(r) * a, sum[1] + u64::from(g) * a, sum[2] + u64::from(b) * a, sum[3] + a];
                }
            }
            let premultiplied = |c: u64| u8::try_from(c / (255 * n)).unwrap_or(u8::MAX);
            out.extend([premultiplied(sum[0]), premultiplied(sum[1]), premultiplied(sum[2]), u8::try_from(sum[3] / n).unwrap_or(u8::MAX)]);
        }
    }
    Some(egui::ColorImage::from_rgba_premultiplied([ow, oh], &out))
}

/// The headline and the line under it, per state.
fn headline(status: &Status) -> (String, String) {
    let version = |text: &str, v: &str| text.replace("{version}", v);
    match status {
        Status::Idle | Status::Checking => {
            (tl!("Checking for updates…").to_string(), version(tl!("You have version {version}."), prism_engine::build_info::VERSION))
        }
        Status::UpToDate => (tl!("Prism is up to date").to_string(), version(tl!("You have the latest version ({version})."), prism_engine::build_info::VERSION)),
        Status::Failed { message } => (tl!("Couldn't update Prism").to_string(), message.clone()),
        Status::Available { offer } => {
            let sub = match &offer.note {
                Some(note) => tl!(note.as_str()).to_string(),
                None => tl!("Do you want to update now?").to_string(),
            };
            (version(tl!("Prism {version} is available"), &offer.version), sub)
        }
        Status::Downloading { offer, .. } => (version(tl!("Downloading Prism {version}…"), &offer.version), tl!("You can keep working while it downloads.").to_string()),
        Status::Ready { offer } => (
            version(tl!("Prism {version} is ready to install"), &offer.version),
            tl!("Prism will quit, update and open again. Choose Later to install it the next time you quit.").to_string(),
        ),
    }
}

/// The top of the window: the app icon beside the headline. Returns its rectangle (the window's
/// drag handle).
pub fn header(app: &PrismApp, ui: &mut egui::Ui) -> egui::Rect {
    let t = Tokens::get(ui.ctx());
    ui.set_width(WIDTH);
    let (title, sub) = headline(&app.ui.update.status);
    let checking = matches!(app.ui.update.status, Status::Idle | Status::Checking);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = ICON_GAP;
        match icon(ui.ctx()) {
            Some(icon) => {
                let art = egui::Rect::from_min_max(egui::pos2(ICON_MARGIN, ICON_MARGIN), egui::pos2(1.0 - ICON_MARGIN, 1.0 - ICON_MARGIN));
                ui.add(egui::Image::new(&icon).uv(art).fit_to_exact_size(egui::vec2(ICON, ICON)).alt_text("Prism"));
            }
            None => {
                ui.allocate_space(egui::vec2(ICON, ICON));
            }
        }
        ui.vertical(|ui| {
            ui.set_width(WIDTH - ICON - ICON_GAP);
            ui.spacing_mut().item_spacing.y = 6.0;
            // The two lines sit centred against the icon when they are shorter than it.
            let title_font = crate::theme::semibold(17.0);
            let lines = ui.fonts_mut(|f| f.row_height(&title_font)) + 6.0 + ui.text_style_height(&egui::TextStyle::Body);
            ui.add_space(((ICON - lines) / 2.0 - 1.0).max(0.0));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if checking {
                    ui.add(egui::Spinner::new().size(14.0));
                }
                ui.add(egui::Label::new(egui::RichText::new(title).font(title_font.clone()).color(t.text)).selectable(false));
            });
            ui.add(egui::Label::new(egui::RichText::new(sub).color(t.text_dim)).wrap().selectable(false));
        });
    })
    .response
    .rect
}

/// "1.4.2 → 1.4.3": what is installed and what the update brings.
fn versions(ui: &mut egui::Ui, t: &Tokens, offer: &Offer) {
    let half = (WIDTH - 36.0 - 28.0) / 2.0;
    let cell = |ui: &mut egui::Ui, caption: &str, version: &str, new: bool| {
        ui.vertical(|ui| {
            ui.set_width(half);
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.add(egui::Label::new(egui::RichText::new(caption).size(11.0).color(if new { t.accent } else { t.text_faint })).selectable(false));
            ui.add(egui::Label::new(egui::RichText::new(version).font(crate::theme::semibold(16.0)).color(if new { t.text } else { t.text_dim })).truncate());
        });
    };
    egui::Frame::new().fill(t.field).stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius).inner_margin(egui::Margin::symmetric(18, 11)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            cell(ui, tl!("Current version"), &offer.current, false);
            let (arrow, _) = ui.allocate_exact_size(egui::vec2(28.0, 34.0), egui::Sense::hover());
            let (tip, stroke) = (arrow.center() + egui::vec2(-4.0, 0.0), egui::Stroke::new(1.5, t.text_faint));
            ui.painter().line_segment([tip - egui::vec2(14.0, 0.0), tip], stroke);
            ui.painter().line_segment([tip - egui::vec2(5.0, 4.5), tip], stroke);
            ui.painter().line_segment([tip - egui::vec2(5.0, -4.5), tip], stroke);
            cell(ui, tl!("New version"), &offer.version, true);
        });
    });
}

/// A slim download bar with the amounts under it.
fn progress(ui: &mut egui::Ui, t: &Tokens, received: u64, total: u64) {
    let frac = if total > 0 { (received as f64 / total as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let (bar, _) = ui.allocate_exact_size(egui::vec2(WIDTH, 6.0), egui::Sense::hover());
    ui.painter().rect_filled(bar, 3.0, t.field);
    if frac > 0.0 {
        let done = egui::Rect::from_min_size(bar.min, egui::vec2((bar.width() * frac).max(bar.height()), bar.height()));
        ui.painter().rect_filled(done, 3.0, t.accent);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let amounts = if total > 0 { format!("{:.1} / {:.1} MB", received as f64 / 1e6, total as f64 / 1e6) } else { tl!("Starting…").to_string() };
        ui.label(egui::RichText::new(amounts).size(11.5).color(t.text_faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(format!("{:.0}%", frac * 100.0)).size(11.5).color(t.text_dim));
        });
    });
}

pub fn body(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let status = app.ui.update.status.clone();
    f.insert("__state".into(), json!(state_name(&status)));
    let can_install = matches!(&status, Status::Available { offer } if offer.can_install);
    f.insert("__canInstall".into(), json!(can_install));
    ui.set_width(WIDTH);
    match &status {
        Status::Available { offer } => {
            versions(ui, &t, offer);
            if let Some(url) = offer.notes_url.clone() {
                ui.add_space(6.0);
                if ui.link(tl!("Release Notes")).clicked() {
                    crate::links::open(app, ui.ctx(), &url);
                }
            }
        }
        Status::Downloading { received, total, .. } => progress(ui, &t, *received, *total),
        Status::Idle | Status::Checking | Status::UpToDate | Status::Failed { .. } | Status::Ready { .. } => {}
    }
    if let Some(e) = f.get("__error").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.colored_label(t.warning, e);
    }
}

/// "Skip This Version", at the left of the button row while an update is offered.
pub fn footer(app: &mut PrismApp, ui: &mut egui::Ui) {
    let Status::Available { offer } = &app.ui.update.status else { return };
    let version = offer.version.clone();
    let t = Tokens::get(ui.ctx());
    let row = egui::vec2(ui.available_width(), ui.min_rect().height().max(1.0));
    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let quiet = ui.add(egui::Label::new(egui::RichText::new(tl!("Skip This Version")).color(t.text_faint)).selectable(false).sense(egui::Sense::click()));
        if quiet.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            let y = quiet.rect.bottom() - 1.0;
            ui.painter().line_segment([egui::pos2(quiet.rect.left(), y), egui::pos2(quiet.rect.right(), y)], egui::Stroke::new(1.0, t.text_faint));
        }
        if quiet.clicked() {
            app.session.prefs.edit(|p| p.updates.skipped_version = version);
            app.ui.update.close_requested = true;
        }
    });
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
