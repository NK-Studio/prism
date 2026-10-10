//! Native modal command windows; document identity is independent of OS focus.
use crate::{
    PhotocraftApp,
    state::{Dialog, DialogPresentation},
};
use egui::{Context, ViewportId, vec2};
use serde_json::{Value, json};

pub fn viewport(id: u64) -> ViewportId {
    ViewportId::from_hash_of(("command-dialog", id))
}
pub fn bind_unbound(app: &mut PhotocraftApp) {
    let source = app.session.active().map(|s| (s.doc.id, s.revision));
    for d in &mut app.ui.dialogs {
        if d.source_document.is_none()
            && d.fields.get("__command").and_then(Value::as_str).is_some_and(|c| crate::document_menu::COMMANDS.contains(&c))
            && let Some((id, revision)) = source
        {
            d.source_document = Some(id);
            d.source_revision = Some(revision);
        }
    }
}
pub fn make_native(app: &mut PhotocraftApp, id: u64) {
    bind_unbound(app);
    if let Some(d) = app.ui.dialog_mut(id) {
        d.presentation = DialogPresentation::Native;
    }
}
pub fn is_open(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.iter().any(|d| d.presentation == DialogPresentation::Native)
}
pub fn focus(app: &PhotocraftApp, ctx: &Context, id: u64) -> Result<Value, String> {
    if !app.ui.dialogs.iter().any(|d| d.id == id && d.presentation == DialogPresentation::Native) {
        return Err(format!("no native dialog {id}"));
    }
    ctx.send_viewport_cmd_to(viewport(id), egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd_to(viewport(id), egui::ViewportCommand::Focus);
    ctx.request_repaint();
    Ok(json!({"dialog":id}))
}
pub fn source(app: &mut PhotocraftApp, d: &Dialog) -> Result<usize, String> {
    let index = if let Some(id) = d.source_document {
        app.session.documents().iter().position(|s| s.doc.id == id).ok_or("source document is closed")?
    } else {
        app.session.active_index().ok_or("no source document")?
    };
    let st = app.session.documents().get(index).ok_or("source document is closed")?;
    if d.source_revision.is_some_and(|r| r != st.revision) {
        return Err("source document changed while the dialog was open".into());
    }
    if let Some(window) = d.window
        && !app.ui.windows.iter().any(|w| w.open && w.id == window && w.document == index)
    {
        return Err("source window is closed".into());
    }
    crate::doc_windows::activate(app, index, d.window);
    Ok(index)
}
pub fn cancel(app: &mut PhotocraftApp, ctx: &Context, id: u64) -> Result<Value, String> {
    let d = app.ui.close_dialog(id).ok_or_else(|| format!("no dialog {id}"))?;
    if d.presentation == DialogPresentation::Native {
        ctx.send_viewport_cmd_to(crate::doc_windows::viewport(d.window), egui::ViewportCommand::Focus);
    }
    app.filter_preview = None;
    app.color_range = None;
    Ok(Value::Null)
}
fn size(d: &Dialog) -> egui::Vec2 {
    match d.fields.get("__command").and_then(Value::as_str) {
        Some("image.imageSize") => vec2(860.0, 460.0),
        Some("image.canvasSize") => vec2(500.0, 480.0),
        _ => vec2(520.0, 220.0),
    }
}
fn minimum_size(d: &Dialog) -> egui::Vec2 {
    match d.fields.get("__command").and_then(Value::as_str) {
        Some("image.imageSize") => vec2(480.0, 240.0),
        Some("image.canvasSize") => vec2(440.0, 240.0),
        _ => vec2(380.0, 180.0),
    }
}
fn fitted_geometry(wanted: egui::Vec2, source: Option<egui::Rect>, screen: egui::Rect) -> (egui::Vec2, egui::Pos2) {
    let wanted = wanted.min((screen.size() - vec2(32.0, 80.0)).max(vec2(160.0, 120.0)));
    let centered = source.filter(|r| r.is_finite()).unwrap_or(screen).center() - wanted / 2.0;
    let position = egui::pos2(
        centered.x.clamp(screen.left() + 16.0, (screen.right() - wanted.x - 16.0).max(screen.left() + 16.0)),
        centered.y.clamp(screen.top() + 32.0, (screen.bottom() - wanted.y - 48.0).max(screen.top() + 32.0)),
    );
    (wanted, position)
}
pub fn show(app: &mut PhotocraftApp, ctx: &Context) {
    let dialogs: Vec<_> = app
        .ui
        .dialogs
        .iter()
        .filter(|d| d.presentation == DialogPresentation::Native && crate::doc_windows::viewport(d.window) == ctx.viewport_id())
        .cloned()
        .collect();
    for d in dialogs {
        let extent = ctx.input(|i| i.viewport().monitor_size).filter(|v| v.is_finite() && v.x > 200.0 && v.y > 200.0).unwrap_or(vec2(1920.0, 1080.0));
        let source = ctx.input(|i| i.viewport().inner_rect);
        let screen = crate::monitor_status::view_display(app, ctx)
            .and_then(|id| app.session.color.displays.iter().find(|display| display.id == id))
            .map(|display| {
                let scale = ctx.input(|i| i.viewport().native_pixels_per_point).filter(|v| *v > 0.0).unwrap_or(ctx.pixels_per_point()) / ctx.pixels_per_point();
                let [x, y, w, h] = display.frame;
                egui::Rect::from_min_size(egui::pos2(x as f32, y as f32) * scale, vec2(w as f32, h as f32) * scale)
            })
            .filter(|r| r.is_finite() && r.is_positive())
            .unwrap_or_else(|| {
                let center = source.filter(|r| r.is_finite()).map_or(egui::Pos2::ZERO, |r| r.center());
                egui::Rect::from_min_size(egui::pos2((center.x / extent.x).floor() * extent.x, (center.y / extent.y).floor() * extent.y), extent)
            });
        let (wanted, position) = fitted_geometry(size(&d), source, screen);
        let builder = egui::ViewportBuilder::default()
            .with_title(crate::i18n::tr(crate::i18n::current(), &crate::dialogs::title(&d)).to_string())
            .with_inner_size(wanted)
            .with_min_inner_size(wanted.min(minimum_size(&d)))
            .with_max_inner_size(screen.size() - vec2(32.0, 80.0))
            .with_position(position)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_window_level(crate::doc_windows::window_level(app, ctx));
        let id = viewport(d.id);
        ctx.show_viewport_immediate(id, builder, |ui, _| {
            let child = ui.ctx().clone();
            crate::doc_windows::sync_window_level(app, &child);
            app.collect_screenshots(&child);
            let mut fields = app.ui.dialogs.iter().find(|x| x.id == d.id).map_or_else(|| d.fields.clone(), |x| x.fields.clone());
            let popup_open = egui::Popup::is_any_open(&child);
            let mut outcome = None;
            if child.input(|i| i.viewport().close_requested()) {
                child.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                outcome = Some(false);
            }
            egui::Panel::bottom(egui::Id::new(("native-dialog-footer", d.id)))
                .exact_size(64.0)
                .frame(egui::Frame::new().inner_margin(16).fill(crate::theme::Tokens::get(&child).card))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::widgets::primary_button(ui, tl!("OK"), 110.0).clicked() {
                            outcome = Some(true);
                        }
                        if crate::widgets::secondary_button(ui, tl!("Cancel"), 110.0).clicked() {
                            outcome = Some(false);
                        }
                    });
                });
            egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(20).fill(crate::theme::Tokens::get(&child).card)).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(error) = fields.get("__error").and_then(Value::as_str) {
                        ui.colored_label(crate::theme::Tokens::get(&child).text, error);
                    }
                    if fields.contains_key("__duplicate") {
                        crate::duplicate_ui::body(ui, &mut fields);
                    } else {
                        crate::sizing::body(app, ui, &mut fields);
                    }
                });
            });
            if !popup_open && !egui::Popup::is_any_open(&child) && (child.input(|i| i.focused) || crate::doc_windows::automated(&child)) {
                if child.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                    outcome = Some(true);
                }
                if child.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    outcome = Some(false);
                }
            }
            if let Some(dialog) = app.ui.dialog_mut(d.id) {
                dialog.fields = fields;
            }
            match outcome {
                Some(true) => {
                    if crate::dialogs::confirm(app, d.id).is_ok() {
                        child.send_viewport_cmd_to(crate::doc_windows::viewport(app.ui.active_window), egui::ViewportCommand::Focus);
                    }
                }
                Some(false) => {
                    let _ = cancel(app, &child, d.id);
                }
                None => {}
            }
            // Disabled document windows can still receive OS title-bar focus. Reassert modality
            // only within this application, never while another app is foreground.
            if crate::doc_windows::window_level(app, &child) == egui::viewport::WindowLevel::AlwaysOnTop
                && child.input(|i| i.raw.viewports.iter().any(|(v, info)| *v != id && info.focused == Some(true)))
            {
                child.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dialog_geometry_fits_small_and_negative_origin_displays() {
        for screen in
            [egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(800.0, 600.0)), egui::Rect::from_min_size(egui::pos2(-1920.0, -500.0), vec2(1920.0, 1080.0))]
        {
            let source = egui::Rect::from_min_size(screen.max - vec2(100.0, 60.0), vec2(800.0, 600.0));
            let (size, position) = fitted_geometry(vec2(860.0, 560.0), Some(source), screen);
            assert!(screen.contains_rect(egui::Rect::from_min_size(position, size)));
            assert!(position.y + size.y <= screen.bottom() - 48.0);
        }
    }
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":12,"height":10})).unwrap();
        app.sync_views();
        app
    }
    #[test]
    fn failed_confirm_keeps_fields_and_document() {
        let mut a = app();
        let id = crate::sizing::open(&mut a, "image.imageSize").unwrap();
        let original = a.session.active().unwrap().doc.clone();
        a.ui.dialog_mut(id).unwrap().fields.insert("width".into(), json!(0));
        assert!(crate::dialogs::confirm(&mut a, id).is_err());
        assert_eq!(a.ui.dialogs[0].fields["width"], 0);
        assert!(std::sync::Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
        a.ui.dialog_mut(id).unwrap().fields.insert("width".into(), json!(24));
        crate::dialogs::confirm(&mut a, id).unwrap();
        assert!(a.ui.dialogs.is_empty());
    }
    #[test]
    fn stale_or_closed_source_is_rejected_and_cancel_is_non_mutating() {
        let ctx = Context::default();
        let mut a = app();
        let id = crate::sizing::open(&mut a, "image.canvasSize").unwrap();
        a.run("image.canvasSize", json!({"width":14,"height":10})).unwrap();
        let original = a.session.active().unwrap().doc.clone();
        assert!(crate::dialogs::confirm(&mut a, id).is_err());
        cancel(&mut a, &ctx, id).unwrap();
        assert!(std::sync::Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
        let id = crate::duplicate_ui::open(&mut a).unwrap();
        a.run("file.close", json!({})).unwrap();
        assert!(crate::dialogs::confirm(&mut a, id).is_err());
        assert!(a.ui.dialog_mut(id).is_some());
    }
    #[test]
    fn old_dialogs_remain_embedded() {
        let d: Dialog = serde_json::from_value(json!({"id":7,"kind":"Command","fields":{}})).unwrap();
        assert_eq!(d.presentation, DialogPresentation::Embedded);
        assert!(d.source_document.is_none());
    }
    #[test]
    fn dialog_input_selectors_validate_and_keep_document_window_default() {
        let mut a = app();
        let ctx = Context::default();
        let id = crate::duplicate_ui::open(&mut a).unwrap();
        for (method, params) in
            [("ui.click", json!({"dialog":id,"window":1})), ("ui.key", json!({"dialog":999,"key":"Enter"})), ("ui.resize", json!({"dialog":"bad"}))]
        {
            let (req, _) = crate::control::ControlRequest::new(method, params);
            assert!(matches!(crate::control::handle(&mut a,&ctx,&req),crate::control::Outcome::Done(v) if v["ok"]==false));
        }
        assert!(focus(&a, &ctx, id).is_ok());
        let original = a.session.active().unwrap().doc.clone();
        let (request, _) = crate::control::ControlRequest::new("ui.pointer", json!({"events":[{"kind":"down","x":1,"y":1}],"tool":"brush"}));
        assert!(matches!(crate::control::handle(&mut a,&ctx,&request), crate::control::Outcome::Done(v) if v["ok"]==false));
        assert!(std::sync::Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
        cancel(&mut a, &ctx, id).unwrap();
        assert!(focus(&a, &ctx, id).is_err());
    }
}
