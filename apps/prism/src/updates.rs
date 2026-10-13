//! The desktop [`Updater`]: `prism-updater` on background threads, reporting to the UI through a
//! channel that `update_ui::tick` polls every frame.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use prism_ui_egui::update_ui::{Offer, UpdateEvent, Updater};
use prism_updater::{InstallTarget, Prepared, Update};

pub struct DesktopUpdater {
    tx: Sender<UpdateEvent>,
    rx: Receiver<UpdateEvent>,
    /// The update the last check found.
    found: Arc<Mutex<Option<Update>>>,
    /// The verified download, ready to install.
    prepared: Arc<Mutex<Option<Prepared>>>,
    cancel: Arc<AtomicBool>,
    target: InstallTarget,
    /// Wakes the UI when a background step finishes.
    ctx: egui::Context,
}

impl DesktopUpdater {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            found: Arc::default(),
            prepared: Arc::default(),
            cancel: Arc::default(),
            target: prism_updater::detect_install_target(),
            ctx,
        }
    }
}

fn offer(update: &Update, target: &InstallTarget) -> Offer {
    let note = if update.manual_only {
        Some("This version is too old to update in place. Download the new installer instead.".to_string())
    } else if update.asset.is_none() {
        Some("There's no automatic update for this platform. Download it instead.".to_string())
    } else if let InstallTarget::Manual(why) = target {
        Some(format!("Prism can't update itself here: {why}."))
    } else {
        None
    };
    Offer {
        version: update.version.clone(),
        current: update.current.clone(),
        notes_url: update.notes_url.clone(),
        download_url: update.download_url.clone(),
        can_install: update.asset.is_some() && target.can_install(),
        note,
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Updater for DesktopUpdater {
    fn check(&mut self) {
        let (tx, found, target, ctx) = (self.tx.clone(), self.found.clone(), self.target.clone(), self.ctx.clone());
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let result = crate::crash_guard::guard("Update check", || {
                let config = prism_updater::Config::from_build().map_err(|e| e.to_string())?;
                prism_updater::check(&config, prism_engine::build_info::VERSION).map_err(|e| e.to_string())
            });
            let event = UpdateEvent::Checked(result.map(|found_update| {
                let shown = found_update.as_ref().map(|u| offer(u, &target));
                *lock(&found) = found_update;
                shown
            }));
            let _ = tx.send(event);
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            let _ = self.tx.send(UpdateEvent::Checked(Err(e.to_string())));
        }
    }

    fn download(&mut self) {
        let Some(update) = lock(&self.found).clone() else {
            let _ = self.tx.send(UpdateEvent::Failed("check for updates first".into()));
            return;
        };
        let Some(asset) = update.asset.clone() else {
            let _ = self.tx.send(UpdateEvent::Failed("no package to download for this platform".into()));
            return;
        };
        self.cancel.store(false, Ordering::Relaxed);
        let (tx, prepared, cancel, target, ctx) = (self.tx.clone(), self.prepared.clone(), self.cancel.clone(), self.target.clone(), self.ctx.clone());
        let spawned = std::thread::Builder::new().name("update-download".into()).spawn(move || {
            let dir = std::env::temp_dir().join("prism-updates");
            let mut last_pct = u64::MAX;
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let result = crate::crash_guard::guard("Update download", || {
                let file = prism_updater::download(&asset, &dir, &cancel, |received, total| {
                    let pct = received.saturating_mul(100) / total.max(1);
                    if pct != last_pct {
                        last_pct = pct;
                        let _ = progress_tx.send(UpdateEvent::Progress { received, total });
                        progress_ctx.request_repaint();
                    }
                })
                .map_err(|e| e.to_string())?;
                prism_updater::prepare(&target, &file, &update.version).map_err(|e| e.to_string())
            });
            match result {
                Ok(p) => {
                    *lock(&prepared) = Some(p);
                    let _ = tx.send(UpdateEvent::Ready);
                }
                // The user cancelled: the UI already went back to the offer.
                Err(_) if cancel.load(Ordering::Relaxed) => {}
                Err(e) => {
                    let _ = tx.send(UpdateEvent::Failed(e));
                }
            }
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            let _ = self.tx.send(UpdateEvent::Failed(e.to_string()));
        }
    }

    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn poll(&mut self) -> Vec<UpdateEvent> {
        self.rx.try_iter().collect()
    }

    fn install_on_exit(&mut self) -> Result<(), String> {
        let prepared = lock(&self.prepared).take().ok_or("no update has been downloaded")?;
        prism_updater::install(&prepared).map_err(|e| e.to_string())
    }
}
