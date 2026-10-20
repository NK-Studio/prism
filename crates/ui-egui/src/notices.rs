//! Non-blocking notices: a small stack of cards in the lower-right corner of the window for things
//! the user should see but that must not interrupt work (import/export warnings such as "adjustment
//! flattened", files that couldn't be opened). Notices live in [`UiState`](crate::UiState) so the
//! control channel can read them (`ui.inspect`) and clear them (`ui.set`).

use serde::{Deserialize, Serialize};

use crate::PrismApp;

/// At most this many notices are kept; older ones drop off.
pub const MAX_NOTICES: usize = 3;
/// Lines shown per notice before "…and N more".
const MAX_LINES: usize = 8;
/// A full canvas refresh on the CPU compositor slower than this (ms) gets a notice.
const SLOW_CPU_REFRESH_MS: f64 = 1000.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub lines: Vec<String>,
    /// An error (warning colour) rather than an informational notice.
    #[serde(default)]
    pub error: bool,
    /// Preference key to set when this notice is dismissed.
    #[serde(default)]
    pub dismiss_pref: Option<String>,
    /// `{name}` values filled into the title and lines after they are translated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<(String, String)>,
}

impl Notice {
    /// `s` (the title or a line) in the current language, with [`Notice::args`] filled in.
    pub fn text(&self, s: &str) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::i18n::fmt(&translate_line(s), &args)
    }
}

/// `s` in the current language. Import warnings often read "<brush name>: <message>": when the
/// whole line has no translation, the message after the first ": " is translated on its own.
pub fn translate_line(s: &str) -> String {
    let lang = crate::i18n::current();
    if crate::i18n::has(lang, s) {
        return crate::i18n::tr(lang, s).to_string();
    }
    match s.split_once(": ") {
        Some((name, message)) if crate::i18n::has(lang, message) => format!("{name}: {}", crate::i18n::tr(lang, message)),
        _ => s.to_string(),
    }
}

/// How long a toast stays up, in seconds, fades included.
pub const TOAST_SECONDS: f64 = 1.6;
const TOAST_FADE_IN: f64 = 0.12;
const TOAST_FADE_OUT: f64 = 0.4;

/// A short confirmation shown at the bottom of the window and gone after [`TOAST_SECONDS`]: for
/// things that went as expected (a save), where a notice that waits to be dismissed is too much.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Toast {
    pub text: String,
    /// When it was first drawn (egui time); the clock starts then, not when it was posted.
    #[serde(skip)]
    pub shown_at: Option<f64>,
}

/// Show `text` as a toast, replacing the one up.
pub fn toast(app: &mut PrismApp, text: String) {
    app.ui.toast = Some(Toast { text, shown_at: None });
}

/// Confirm a save that had nothing to warn about ("Saved a.png").
pub fn saved(app: &mut PrismApp, name: &str) {
    toast(app, crate::i18n::fmt(tl!("Saved {name}"), &[("name", name)]));
}

/// Draw the toast in `ctx`'s window, bottom centre above the status bar, fading in and out.
pub fn show_toast(app: &mut PrismApp, ctx: &egui::Context) {
    let Some(toast) = app.ui.toast.as_mut() else { return };
    let now = ctx.input(|i| i.time);
    let age = now - *toast.shown_at.get_or_insert(now);
    if !(0.0..TOAST_SECONDS).contains(&age) {
        app.ui.toast = None;
        return;
    }
    let alpha = (age / TOAST_FADE_IN).min((TOAST_SECONDS - age) / TOAST_FADE_OUT).clamp(0.0, 1.0) as f32;
    let text = toast.text.clone();
    let t = crate::theme::Tokens::get(ctx);
    egui::Area::new(egui::Id::new("prism-toast"))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_opacity(alpha);
            egui::Frame::NONE
                .fill(t.card)
                .stroke(egui::Stroke::new(1.0, t.card_border))
                .corner_radius(t.radius_lg)
                .inner_margin(egui::Margin::symmetric(14, 8))
                .shadow(egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: t.shadow })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("✓").strong().color(t.accent));
                        ui.label(egui::RichText::new(text).color(t.text));
                    });
                });
        });
    ctx.request_repaint();
}

/// Show a notice (newest last); returns its id. A notice that says the same as one already shown
/// (saving the same file twice) moves that one to the front instead of stacking a copy.
pub fn post(app: &mut PrismApp, title: impl Into<String>, lines: Vec<String>, error: bool, dismiss_pref: Option<&str>) -> u64 {
    let title = title.into();
    let dismiss_pref = dismiss_pref.map(str::to_owned);
    if let Some(at) =
        app.ui.notices.iter().position(|n| n.title == title && n.lines == lines && n.error == error && n.dismiss_pref == dismiss_pref && n.args.is_empty())
    {
        let notice = app.ui.notices.remove(at);
        let id = notice.id;
        app.ui.notices.push(notice);
        return id;
    }
    let id = app.ui.alloc_id();
    app.ui.notices.push(Notice { id, title, lines, error, dismiss_pref, args: Vec::new() });
    cap_notices(app);
    id
}

fn cap_notices(app: &mut PrismApp) {
    while app.ui.notices.len() > MAX_NOTICES {
        let oldest_temporary = app.ui.notices.iter().position(|notice| notice.dismiss_pref.is_none()).unwrap_or(0);
        app.ui.notices.remove(oldest_temporary);
    }
}

/// A full refresh of document `doc` fell back to the CPU compositor (`reason`, the GPU's) and took
/// `ms`. The canvas doesn't respond while it runs, so once per document say why it was slow and
/// which edits will do it again (#1015).
pub fn slow_cpu_refresh(app: &mut PrismApp, doc: u64, ms: f64, reason: Option<&str>) {
    if ms.is_nan() || ms < SLOW_CPU_REFRESH_MS || app.ui.slow_refresh_noticed.contains(&doc) {
        return;
    }
    app.ui.slow_refresh_noticed.push(doc);
    let seconds = format!("{:.1}", ms / 1000.0);
    let mut lines = vec![
        crate::i18n::fmt(tl!("This redraw took {seconds} s on the CPU."), &[("seconds", &seconds)]),
        tl!("Edits that change the whole document, such as a layer's blend mode, opacity or visibility, redraw all of it.").into(),
    ];
    if reason == Some(crate::gpu_canvas::OVER_BUDGET) {
        lines.push(
            tl!("Its layers don't fit the GPU memory budget. Raising Memory usage in Preferences › Performance raises the budget, up to a quarter of this computer's memory.")
                .into(),
        );
    } else if let Some(r) = reason {
        lines.push(crate::i18n::fmt(tl!("The GPU compositor wasn't used: {reason}"), &[("reason", r)]));
    }
    post(app, tl!("Large document: redrawing on the CPU"), lines, false, None);
}

fn dismiss(app: &mut PrismApp, id: u64) {
    let dismiss_pref = app.ui.notices.iter().find(|notice| notice.id == id).and_then(|notice| notice.dismiss_pref.clone());
    app.ui.notices.retain(|notice| notice.id != id);
    if let Some(key) = dismiss_pref {
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(key, serde_json::Value::Bool(true));
        });
    }
}

/// Report import/export `warnings` for the file operation `what` (e.g. "Opened a.psd"): the status
/// bar says how many there were, and a notice lists them. Nothing happens when there are none.
pub fn io_warnings(app: &mut PrismApp, what: &str, warnings: &[String]) {
    let Some(first) = warnings.first() else { return };
    let n = warnings.len();
    let what = crate::i18n::t(what);
    let count = crate::i18n::trn(crate::i18n::current(), n as u64, "{n} warning", "{n} warnings");
    let first = translate_line(first);
    app.ui.status = if n == 1 { format!("{what}: {first}") } else { format!("{what} — {count}: {first} …") };
    app.ui.status_error = true;
    post(app, format!("{what} — {count}"), warnings.to_vec(), false, None);
}

/// Report a failed file operation: the status bar shows it as an error and a notice keeps it on
/// screen until dismissed.
pub fn error(app: &mut PrismApp, message: String) {
    app.ui.status = message.clone();
    app.ui.status_error = true;
    post(app, message, Vec::new(), true, None);
}

/// Draw the notices; each has a close button.
pub fn show(app: &mut PrismApp, ctx: &egui::Context) {
    if app.ui.notices.is_empty() {
        return;
    }
    let t = crate::theme::Tokens::get(ctx);
    let mut dismiss_id = None;
    // Clear the status bar (~24 px) and leave the dock's edge some air.
    egui::Area::new(egui::Id::new("prism-notices"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -36.0))
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_max_width(360.0);
            for n in &app.ui.notices {
                egui::Frame::NONE
                    .fill(t.card)
                    .stroke(egui::Stroke::new(1.0, if n.error { t.warning } else { t.card_border }))
                    .corner_radius(t.radius)
                    .inner_margin(egui::Margin::same(10))
                    .shadow(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: t.shadow })
                    .show(ui, |ui| {
                        ui.set_width(340.0);
                        ui.horizontal(|ui| {
                            let title = egui::RichText::new(n.text(&n.title)).strong().color(if n.error { t.warning } else { t.text });
                            ui.add(egui::Label::new(title).wrap());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                if ui.add(egui::Button::new(egui::RichText::new("×").color(t.text_dim)).frame(false)).on_hover_text(tl!("Dismiss")).clicked() {
                                    dismiss_id = Some(n.id);
                                }
                            });
                        });
                        for line in n.lines.iter().take(MAX_LINES) {
                            let line = n.text(line);
                            ui.add(egui::Label::new(egui::RichText::new(format!("• {line}")).color(t.text_dim)).wrap());
                        }
                        if n.lines.len() > MAX_LINES {
                            let text = crate::i18n::fmt(tl!("…and {n} more"), &[("n", &(n.lines.len() - MAX_LINES).to_string())]);
                            ui.label(egui::RichText::new(text).color(t.text_faint));
                        }
                    });
                ui.add_space(6.0);
            }
        });
    if let Some(id) = dismiss_id {
        dismiss(app, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PrismApp, Services};
    use prism_engine::Session;

    #[test]
    fn a_slow_cpu_refresh_is_explained_once_per_document() {
        let mut app = PrismApp::new(Session::new(), Services::default());
        slow_cpu_refresh(&mut app, 1, 400.0, Some(crate::gpu_canvas::OVER_BUDGET));
        slow_cpu_refresh(&mut app, 1, f64::NAN, Some(crate::gpu_canvas::OVER_BUDGET));
        assert!(app.ui.notices.is_empty(), "a quick refresh needs no explanation");

        slow_cpu_refresh(&mut app, 1, 55_000.0, Some(crate::gpu_canvas::OVER_BUDGET));
        assert_eq!(app.ui.notices.len(), 1);
        let text = app.ui.notices[0].lines.join(" ");
        assert!(text.contains("55.0 s"), "{text}");
        assert!(text.contains("blend mode, opacity or visibility"), "{text}");
        assert!(text.contains("Memory usage in Preferences › Performance"), "{text}");
        assert!(!app.ui.notices[0].error);

        slow_cpu_refresh(&mut app, 1, 55_000.0, Some(crate::gpu_canvas::OVER_BUDGET));
        assert_eq!(app.ui.notices.len(), 1, "once per document");

        slow_cpu_refresh(&mut app, 2, 3_000.0, Some("Blend If on `Overlay` (composited on the CPU)"));
        assert_eq!(app.ui.notices.len(), 2);
        let text = app.ui.notices[1].lines.join(" ");
        assert!(text.contains("Blend If on `Overlay`"), "{text}");
        assert!(!text.contains("Memory usage"), "the budget hint is only for the budget fallback: {text}");
    }

    #[test]
    fn a_toast_shows_then_goes() {
        let mut app = PrismApp::new(Session::new(), Services::default());
        saved(&mut app, "a.png");
        assert_eq!(app.ui.toast.as_ref().map(|t| t.text.as_str()), Some("Saved a.png"));
        let ctx = egui::Context::default();
        let frame = |app: &mut PrismApp, time: f64| {
            let raw = egui::RawInput { time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show_toast(app, ui.ctx()));
            out.textures_delta.clear();
            out
        };
        let out = frame(&mut app, 10.0);
        assert!(!out.shapes.is_empty(), "drawn");
        assert_eq!(app.ui.toast.as_ref().and_then(|t| t.shown_at), Some(10.0), "the clock starts when it is first drawn");
        frame(&mut app, 10.0 + TOAST_SECONDS / 2.0);
        assert!(app.ui.toast.is_some());
        frame(&mut app, 10.0 + TOAST_SECONDS + 0.01);
        assert!(app.ui.toast.is_none(), "gone");
    }

    #[test]
    fn the_same_notice_twice_shows_once() {
        let mut app = PrismApp::new(Session::new(), Services::default());
        let warn = || vec!["16-bit written as 8-bit".to_string()];
        let first = post(&mut app, "Saved a.png with 1 warning", warn(), false, None);
        post(&mut app, "Opened b.psd", Vec::new(), false, None);
        let again = post(&mut app, "Saved a.png with 1 warning", warn(), false, None);
        assert_eq!(first, again);
        assert_eq!(app.ui.notices.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["Opened b.psd", "Saved a.png with 1 warning"], "moved to the front");
        post(&mut app, "Saved a.png with 1 warning", warn(), true, None);
        assert_eq!(app.ui.notices.len(), 3, "an error is a different notice");
    }

    #[test]
    fn import_warnings_are_titled_and_listed_in_the_ui_language() {
        let mut app = PrismApp::new(Session::new(), Services::default());
        let warnings = vec!["隙間無く囲って塗るツール: SUT tip ordering is approximated by cyclic order".to_string(), "Pen: a message nobody translated".to_string()];
        crate::i18n::with_language(crate::i18n::Lang::from_pref("ko"), || {
            io_warnings(&mut app, "Imported from CLIP STUDIO", &warnings);
            let n = app.ui.notices.last().unwrap().clone();
            assert_eq!(n.text(&n.title), "클립 스튜디오에서 가져옴 — 경고 2개");
            // The brush name stays; the message after it is translated when the catalog has it.
            assert_eq!(
                n.text(&n.lines[0]),
                "隙間無く囲って塗るツール: SUT 팁 순서는 순환 순서로 근사합니다"
            );
            assert_eq!(n.text(&n.lines[1]), "Pen: a message nobody translated");
            assert!(app.ui.status.starts_with("클립 스튜디오에서 가져옴 — 경고 2개: 隙間無く囲って塗るツール: SUT 팁"), "{}", app.ui.status);
        });
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            io_warnings(&mut app, "Saved a.png", &warnings[1..]);
            assert_eq!(app.ui.notices.last().unwrap().title, "Saved a.png — 1 warning");
        });
    }

    #[test]
    fn dismissing_a_notice_writes_its_own_preference_key() {
        let mut app = PrismApp::new(Session::new(), Services::default());
        post(&mut app, "Dismissible", Vec::new(), false, Some("ui.testNoticeDismissed"));
        let id = app.ui.notices[0].id;
        dismiss(&mut app, id);
        assert_eq!(app.session.prefs().dialogs.get("ui.testNoticeDismissed").and_then(serde_json::Value::as_bool), Some(true));
    }
}
