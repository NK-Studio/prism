//! The Save window's Other Location… on macOS: an Open panel that chooses a file *or* a folder.
//!
//! rfd's panels choose files or folders, never both, and its save panel's Go to Folder (⌘⇧G) only
//! goes to folders. In an Open panel that can choose files, ⌘⇧G with a file's path selects that
//! file, so a known path is picked the way Finder users expect; a folder can be chosen too.
//! Shown modeless (`beginWithCompletionHandler:`), so Prism's event loop keeps running, like the
//! rfd dialogs. Safe AppKit calls only.
//!
//! One panel is made and kept: creating an NSOpenPanel blocks the main thread for about 145 ms
//! (the first in a process about 0.5 s), while showing a kept one again takes about 5–15 ms.

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSModalResponse, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSString, NSURL};
use prism_ui_egui::{FileDialogAnswer, FileDialogReply};
use std::cell::Cell;
use std::path::Path;

thread_local! {
    /// The panel, made on first use (AppKit panels live on the main thread).
    static PANEL: std::cell::RefCell<Option<Retained<NSOpenPanel>>> = const { std::cell::RefCell::new(None) };
}

/// Show the panel starting in `suggested`'s folder; `reply` gets the chosen path, or `None`.
pub fn show(suggested: &str, reply: FileDialogReply) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("the location panel must be shown from the main thread");
        reply.send(None);
        return;
    };
    let panel = PANEL.with(|kept| {
        kept.borrow_mut()
            .get_or_insert_with(|| {
                let panel = NSOpenPanel::openPanel(mtm);
                panel.setCanChooseFiles(true);
                panel.setCanChooseDirectories(true);
                panel.setAllowsMultipleSelection(false);
                panel.setCanCreateDirectories(true);
                panel
            })
            .clone()
    });
    // Set each time: the interface language can change between uses.
    panel.setPrompt(Some(&NSString::from_str(prism_ui_egui::i18n::t("Select"))));
    if let Some(dir) = Path::new(suggested).parent().filter(|d| d.is_dir()) {
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()))));
    }
    // The handler is called once; `Fn` blocks can't move out, so the reply waits in a cell.
    let reply = Cell::new(Some(reply));
    let chosen = panel.clone();
    let handler = RcBlock::new(move |result: NSModalResponse| {
        let Some(reply) = reply.take() else { return };
        let path = (result == NSModalResponseOK).then(|| chosen.URL()).flatten().and_then(|url| url.path()).map(|p| p.to_string());
        reply.send(path.map(FileDialogAnswer::SaveTo));
    });
    panel.beginWithCompletionHandler(&handler);
}
