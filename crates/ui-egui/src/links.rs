//! Prism project links opened by the Help menu.

use serde_json::{Value, json};

use crate::PrismApp;

pub const APP_PAGE: &str = "https://github.com/NK-Studio/prism";
pub const GITHUB: &str = "https://github.com/NK-Studio/prism";
pub const ISSUES: &str = "https://github.com/NK-Studio/prism/issues";

/// Help-menu link commands: (id, url). Labels live in `menus::UI_COMMANDS`.
pub const COMMANDS: &[(&str, &str)] = &[("help.website", APP_PAGE), ("help.github", GITHUB), ("help.reportIssue", ISSUES)];

pub fn url_for(id: &str) -> Option<&'static str> {
    COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1)
}

/// Open `url` in the system browser and note it in the status bar. Prefers
/// the platform `open_url` service (reliable on Windows/macOS/Linux); falls back to `ctx.open_url`.
pub fn open(app: &mut PrismApp, ctx: &egui::Context, url: &str) -> Value {
    let opened = app.services.open_url.as_ref().map(|f| f(url).is_ok()).unwrap_or(false);
    if !opened {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
    app.ui.status = format!("Opened {url}");
    json!({"url": url})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_links_and_menu_labels_identify_prism() {
        assert_eq!(GITHUB, "https://github.com/NK-Studio/prism");
        assert_eq!(ISSUES, "https://github.com/NK-Studio/prism/issues");
        let app = PrismApp::new(prism_engine::Session::new(), Default::default());
        for item in crate::menus::menu_items(&app).into_iter().filter(|item| matches!(item.id.as_str(), "help.website" | "help.github")) {
            assert!(item.label.starts_with("Prism"), "{}", item.label);
        }
    }

    #[test]
    fn retired_community_commands_cannot_open_external_links() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        for id in ["help.discord", "help.artcraftWebsite"] {
            assert!(url_for(id).is_none());
            assert!(!crate::menus::is_live(id));
            assert!(crate::menus::invoke(&mut app, &ctx, id, json!({})).is_err());
        }
    }

    #[test]
    fn home_screen_identifies_the_studio_without_community_promotions() {
        use egui_kittest::Harness;
        fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(s) => text.push(s.galley.text().to_string()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, text)),
                _ => {}
            }
        }
        let app = PrismApp::new(prism_engine::Session::new(), Default::default());
        let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut PrismApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let _lang = crate::i18n::language_scope(crate::i18n::Lang::EN);
                crate::panels::title_bar(app, ui);
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(5);
        let mut text = Vec::new();
        h.output().shapes.iter().for_each(|s| collect(&s.shape, &mut text));
        assert!(text.iter().any(|s| s == "NK Studio"));
        assert!(text.iter().any(|s| s == "Prism"));
        for retired in ["Discord", "ArtCraft", "PhotoCraft", "open source", "Prism website", "GitHub"] {
            assert!(!text.iter().any(|s| s.contains(retired)), "{retired}: {text:?}");
        }
    }

    #[test]
    fn links_open_through_the_platform_service() {
        // Help-menu links must open in the browser.
        // They route through the `open_url` service rather than the unreliable `ctx.open_url`.
        use std::sync::{Arc, Mutex};
        let opened: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let rec = opened.clone();
        let services = crate::Services {
            open_url: Some(Box::new(move |u: &str| {
                rec.lock().unwrap().push(u.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PrismApp::new(prism_engine::Session::new(), services);
        let ctx = egui::Context::default();
        // Every Help-menu link id reaches the service with its URL.
        for (id, url) in COMMANDS {
            crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
            assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(*url), "{id}");
        }
        // The direct open() helper also uses it.
        open(&mut app, &ctx, APP_PAGE);
        assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(APP_PAGE));
    }

    #[test]
    fn help_menu_lists_links_then_separator_then_system_info_and_about() {
        let app = PrismApp::new(prism_engine::Session::new(), Default::default());
        let help: Vec<String> = crate::menus::menu_items(&app).into_iter().filter(|i| i.path == ["Help"]).map(|i| i.id).collect();
        assert_eq!(help, ["help.website", "help.github", "help.reportIssue", "---", "help.systemInfo", "help.firstRunSetup", "help.checkForUpdates", "help.about"]);
        for (id, _) in COMMANDS {
            assert!(crate::menus::is_live(id) && crate::menus::is_enabled(&app, id), "{id}");
        }
    }

    #[test]
    fn link_commands_open_their_urls() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        for (id, url) in [("help.website", "https://github.com/NK-Studio/prism"), ("help.github", "https://github.com/NK-Studio/prism")] {
            let r = crate::menus::invoke(&mut app, &ctx, id, serde_json::json!({})).unwrap();
            assert_eq!(r["url"], url);
            assert_eq!(app.ui.status, format!("Opened {url}"));
        }
    }
}
