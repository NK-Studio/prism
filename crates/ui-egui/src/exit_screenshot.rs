//! `prism --screenshot <file.png>`: start normally, render about [`FRAMES`] frames, save the
//! main window's picture as a PNG and quit. Used by build checks that need the real window,
//! without the control channel (no port, token or automation roots involved).
use crate::PrismApp;

/// Frames rendered before the picture is taken (fonts, panels and the first layout settle).
pub const FRAMES: u32 = 120;
/// The screenshot's user-data token; control screenshots use small increasing tokens.
const TOKEN: u64 = u64::MAX - 0x5c;
/// Frames to wait for the window to close after saving before the process exits.
const CLOSE_GRACE_FRAMES: u32 = 30;

#[derive(Debug)]
pub struct ExitScreenshot {
    path: std::path::PathBuf,
    frames: u32,
    requested: bool,
    /// Frames since the picture was saved and the window asked to close.
    closing: Option<u32>,
}

impl ExitScreenshot {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path, frames: 0, requested: false, closing: None }
    }
}

/// One frame: count, ask the root viewport for its picture once, save it when it arrives and
/// close the window. Errors are reported on standard error; the app still quits.
pub(crate) fn tick(app: &mut PrismApp, ctx: &egui::Context) {
    let Some(mut shot) = app.exit_screenshot.take() else { return };
    shot.frames = shot.frames.saturating_add(1);
    ctx.request_repaint();
    if let Some(waited) = shot.closing.as_mut() {
        // A close the shell holds back (an unsaved-changes prompt for a restored document) must
        // not leave a screenshot run open: the picture is saved, nothing else is written.
        *waited += 1;
        if *waited >= CLOSE_GRACE_FRAMES {
            log::info!("--screenshot: the window did not close; exiting");
            std::process::exit(0);
        }
        app.exit_screenshot = Some(shot);
        return;
    }
    if shot.frames >= FRAMES && !shot.requested {
        shot.requested = true;
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Screenshot(egui::UserData::new(TOKEN)));
    }
    let image = ctx.input(|i| {
        i.raw.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { user_data, image, .. } if user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()) == Some(&TOKEN) => {
                Some(image.clone())
            }
            _ => None,
        })
    });
    let Some(image) = image else {
        app.exit_screenshot = Some(shot);
        return;
    };
    if let Err(error) = save(app, &image, &shot.path) {
        log::error!("--screenshot {}: {error}", shot.path.display());
        eprintln!("prism: --screenshot {}: {error}", shot.path.display());
    }
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
    shot.closing = Some(0);
    app.exit_screenshot = Some(shot);
}

fn save(app: &PrismApp, image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let [w, h] = image.size;
    let (w32, h32) = (u32::try_from(w).map_err(|_| "image too wide")?, u32::try_from(h).map_err(|_| "image too tall")?);
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let encode = app.services.encode_png.as_ref().ok_or("no PNG encoder configured")?;
    let png = encode(w32, h32, &rgba)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, png).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_the_frames_then_requests_once() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.exit_screenshot = Some(ExitScreenshot::new(std::env::temp_dir().join("prism-exit-shot-test.png")));
        let ctx = egui::Context::default();
        for _ in 0..FRAMES + 5 {
            ctx.run_ui(egui::RawInput::default(), |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
        }
        // No picture arrives in a headless context: still waiting, requested exactly once.
        let shot = app.exit_screenshot.as_ref().unwrap();
        assert!(shot.requested);
        assert_eq!(shot.frames, FRAMES + 5);
    }
}
