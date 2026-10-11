//! macOS window lifecycle: title bar dragging and quitting.
//!
//! Dragging: the window draws under a transparent title bar (`main.rs`), so AppKit would
//! otherwise drag the window from anywhere in that strip, menus included: pressing a menu title
//! and dragging down to an item moved the window instead. Turning AppKit's own dragging off leaves
//! only the app's drag region (the free gap between the menus and the controls,
//! `panels::title_bar`), which starts a drag explicitly (`ViewportCommand::StartDrag`, AppKit's
//! `performWindowDragWithEvent:`, which works on an unmovable window).
//!
//! Quitting: see [`terminate_later`]. All safe objc2 calls.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSProcessInfo, NSString};

/// Unbundled Cargo/IDE launches have no CFBundleDisplayName, so AppKit otherwise uses
/// the machine executable name (`prism`) for the Dock tooltip.
pub fn set_process_display_name() {
    NSProcessInfo::processInfo().setProcessName(&NSString::from_str("Prism"));
}

/// Stop AppKit dragging the app's windows by their title bar strip.
pub fn disable_native_title_drag() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("title bar: not on the main thread; AppKit window dragging left on");
        return;
    };
    for window in NSApplication::sharedApplication(mtm).windows().iter() {
        window.setMovable(false);
    }
}

/// App activation, including native sheets: floating documents stay above the main editor
/// only while Prism is active, and rejoin the normal desktop window stack on deactivation.
pub fn application_active() -> bool {
    MainThreadMarker::new().is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive())
}

/// Quit the way a Mac app does, with `-[NSApplication terminate:]` (`Services::quit`), instead of
/// letting eframe close the window. eframe drops the window while AppKit's run loop is still
/// running, and on Macs with a Touch Bar AppKit's next display cycle then tries to stop observing
/// a view that is gone: an uncaught exception that crashes (macOS 27) or leaves a windowless app
/// that only Force Quit ends (macOS 26) (#1458, #1575). `terminate:` ends the process before
/// another display cycle runs; winit's `applicationWillTerminate:` closes the windows and has
/// eframe save the window layout first, as the old ⌘Q (winit's default menu) did.
///
/// Scheduled on the main queue: winit has to be out of its event handler, or eframe's exit
/// handling (and the save) is dropped.
pub fn terminate_later() {
    dispatch2::DispatchQueue::main().exec_async(|| match MainThreadMarker::new() {
        Some(mtm) => NSApplication::sharedApplication(mtm).terminate(None),
        None => log::warn!("quit: the main queue ran off the main thread; not terminating"),
    });
}

/// Resolve a viewport by its native desktop frame once, then retain only AppKit's stable window
/// number. This also distinguishes extra views with identical document titles. No focus changes.
pub fn opacity_service() -> prism_ui_egui::WindowOpacityFn {
    let mut numbers = std::collections::HashMap::<egui::ViewportId, isize>::new();
    Box::new(move |ctx, alpha| {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        let windows = NSApplication::sharedApplication(mtm).windows();
        numbers.retain(|_, number| windows.iter().any(|w| w.windowNumber() == *number));
        let id = ctx.viewport_id();
        let known = numbers.get(&id).and_then(|number| windows.iter().find(|w| w.windowNumber() == *number));
        let window = known.or_else(|| {
            let expected = ctx.input(|i| i.viewport().outer_rect)? * ctx.zoom_factor();
            let screens = objc2_app_kit::NSScreen::screens(mtm);
            let screen = screens.iter().next()?;
            let top = screen.frame().size.height;
            windows.iter().find(|w| {
                let f = w.frame();
                let actual = egui::Rect::from_min_size(
                    egui::pos2(f.origin.x as f32, (top - f.origin.y - f.size.height) as f32),
                    egui::vec2(f.size.width as f32, f.size.height as f32),
                );
                w.isVisible() && (actual.min - expected.min).length() < 3.0 && (actual.size() - expected.size()).length() < 3.0
            })
        });
        let Some(window) = window else { return false };
        numbers.insert(id, window.windowNumber());
        let alpha = if alpha.is_finite() { f64::from(alpha.clamp(0.0, 1.0)) } else { 1.0 };
        if (window.alphaValue() - alpha).abs() > 0.001 {
            window.setAlphaValue(alpha);
        }
        true
    })
}

#[cfg(test)]
mod display_name_tests {
    use super::*;

    #[test]
    fn process_display_name_is_always_english() {
        let _language = prism_ui_egui::i18n::language_scope(prism_ui_egui::i18n::Lang::from_pref("ko-KR"));
        set_process_display_name();
        assert_eq!(NSProcessInfo::processInfo().processName().to_string(), "Prism");
    }
}
