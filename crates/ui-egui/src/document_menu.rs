//! Document-header menu. Targets survive focus changes and document reordering.
use crate::{PrismApp, state::DocumentContextMenu};
use egui::{Context, Response, Ui};
use serde_json::{Value, json};

pub const COMMANDS: [&str; 5] = ["image.duplicate", "image.imageSize", "image.canvasSize", "file.fileInfo", "file.print"];

pub fn available(app: &PrismApp, menu: &DocumentContextMenu, id: &str) -> bool {
    if !crate::menus::modal_allows(app, id) || app.file_dialog_open() {
        return false;
    }
    let Some(st) = app.session.documents().iter().find(|s| s.doc.id == menu.document) else { return false };
    if app.session.job_on(menu.document).is_some() {
        return false;
    }
    if COMMANDS.contains(&id) {
        return st.doc.size.width > 0 && st.doc.size.height > 0;
    }
    matches!(id, "file.close" | "file.closeOthers" | "file.closeAll" | "window.arrange.floatInWindow" | "ui.window.attach")
        && (id != "ui.window.attach" || menu.window.is_some())
        && (id != "file.closeOthers" || app.session.documents().len() > 1)
}

pub fn choose(app: &mut PrismApp, ctx: &Context, id: &str) -> Result<Value, String> {
    let menu = app.ui.document_context_menu.clone().ok_or("no document context menu is open")?;
    if !available(app, &menu, id) {
        return Err("document action is unavailable".into());
    }
    let document = app.session.documents().iter().position(|s| s.doc.id == menu.document).ok_or("document is closed")?;
    crate::doc_windows::activate(app, document, menu.window);
    egui::Popup::close_all(ctx);
    app.ui.document_context_menu = None;
    let result = if id == "ui.window.attach" {
        crate::doc_windows::attach(app, ctx, menu.window.ok_or("no detached window")?, None)
    } else {
        crate::menus::invoke(app, ctx, id, if COMMANDS.contains(&id) { json!({}) } else { json!({"document":document}) })
    };
    // File Info and Print also keep their source even if automation changes active document.
    if result.is_ok() {
        crate::native_dialogs::bind_unbound(app);
    }
    result
}

pub fn response(app: &mut PrismApp, ui: &Ui, response: &Response, document: usize, window: Option<u64>) {
    let state_id = egui::Id::new(("document-context-popup", ui.ctx().viewport_id()));
    if let Some((popup, menu)) = ui.ctx().data(|d| d.get_temp::<(egui::Id, DocumentContextMenu)>(state_id))
        && !egui::Popup::is_id_open(ui.ctx(), popup)
        && app.ui.document_context_menu.as_ref() == Some(&menu)
    {
        app.ui.document_context_menu = None;
    }
    let Some(id) = app.session.documents().get(document).map(|s| s.doc.id) else { return };
    let mut selected = None;
    let popup_id = response.id.with(("document-actions", id.0, window));
    egui::Popup::context_menu(response).id(popup_id).show(|ui| {
        let menu = DocumentContextMenu { document: id, window };
        ui.ctx().data_mut(|d| d.insert_temp(state_id, (popup_id, menu.clone())));
        app.ui.document_context_menu = Some(menu.clone());
        ui.set_min_width(190.0);
        for (i, command) in COMMANDS.into_iter().enumerate() {
            if i == 3 || i == 4 {
                ui.separator();
            }
            let label = prism_engine::commands::find(command).map_or(command, |s| s.label);
            if ui.add_enabled(available(app, &menu, command), egui::Button::new(tl!(label))).clicked() {
                selected = Some(command);
            }
        }
        ui.separator();
        for (label, command) in [("Close", "file.close"), ("Close Others", "file.closeOthers"), ("Close All", "file.closeAll")] {
            if ui.add_enabled(available(app, &menu, command), egui::Button::new(tl!(label))).clicked() {
                selected = Some(command);
            }
        }
        if let Some(w) = window {
            if app.ui.windows.iter().any(|x| x.id == w && x.kind == crate::state::DocWindowKind::Detached)
                && ui.add_enabled(available(app, &menu, "ui.window.attach"), egui::Button::new(tl!("Move to Main Tabs"))).clicked()
            {
                selected = Some("ui.window.attach");
            }
        } else if ui.add_enabled(available(app, &menu, "window.arrange.floatInWindow"), egui::Button::new(tl!("Float in Window"))).clicked() {
            selected = Some("window.arrange.floatInWindow");
        }
    });
    if let Some(command) = selected
        && let Err(e) = choose(app, ui.ctx(), command)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

pub fn inspect(app: &PrismApp) -> Value {
    app.ui.document_context_menu.as_ref().map_or(Value::Null, |menu| json!({"document":menu.document,"window":menu.window,
        "items":COMMANDS.iter().enumerate().map(|(i,id)|json!({"id":id,"label":prism_engine::commands::find(id).map(|s|s.label),"enabled":available(app,menu,id),"separatorBefore":i==3||i==4})).collect::<Vec<_>>()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> PrismApp {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        for name in ["A", "B"] {
            app.session.execute("file.new", json!({"name":name,"width":20,"height":10})).unwrap();
        }
        app.sync_views();
        app
    }
    #[test]
    fn menu_targets_inactive_document_after_reordering() {
        let mut a = app();
        let ctx = Context::default();
        let source = a.session.documents()[0].doc.id;
        a.ui.document_context_menu = Some(DocumentContextMenu { document: source, window: None });
        a.run("document.move", json!({"document":0,"to":1})).unwrap();
        choose(&mut a, &ctx, "image.canvasSize").unwrap();
        let dialog = a.ui.dialogs.last().unwrap().clone();
        assert_eq!(dialog.source_document, Some(source));
        assert_eq!(dialog.presentation, crate::state::DialogPresentation::Native);
        a.session.set_active(0);
        a.ui.dialog_mut(dialog.id).unwrap().fields.insert("width".into(), json!(30));
        crate::dialogs::confirm(&mut a, dialog.id).unwrap();
        assert_eq!(a.session.documents().iter().find(|s| s.doc.id == source).unwrap().doc.size.width, 30);
        assert_eq!(a.session.documents()[0].doc.size.width, 20);
    }
    #[test]
    fn modal_and_closed_document_disable_menu() {
        let mut a = app();
        let menu = DocumentContextMenu { document: a.session.documents()[0].doc.id, window: None };
        assert!(available(&a, &menu, "image.duplicate"));
        crate::duplicate_ui::open(&mut a).unwrap();
        assert!(!available(&a, &menu, "image.duplicate"));
        a.ui.dialogs.clear();
        a.run("file.close", json!({"document":0})).unwrap();
        assert!(!available(&a, &menu, "image.duplicate"));
    }
    #[test]
    fn identical_widget_ids_in_other_document_views_do_not_retarget_the_popup() {
        let mut a = app();
        let ctx = Context::default();
        let position = egui::pos2(30.0, 30.0);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events: vec![egui::Event::PointerMoved(position)],
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 40.0));
            let mut r = ui.interact(rect, egui::Id::new("shared-header-response"), egui::Sense::click());
            // Simulate a context-menu request in one viewport and the same shell ID in
            // another viewport without a click. Popup memory is shared by those views.
            r.flags.insert(egui::response::Flags::LONG_TOUCHED);
            response(&mut a, ui, &r, 0, Some(1));
            r.flags.remove(egui::response::Flags::LONG_TOUCHED);
            response(&mut a, ui, &r, 1, Some(2));
        });
        out.textures_delta.clear();
        assert_eq!(a.ui.document_context_menu.as_ref().map(|m| m.document), Some(a.session.documents()[0].doc.id));
    }
    #[test]
    fn floating_duplicate_keeps_camera_geometry_and_independent_history() {
        let mut a = app();
        let ctx = Context::default();
        let source = crate::doc_windows::detach(&mut a, &ctx, 0, Some([100.0, 100.0])).unwrap()["window"].as_u64().unwrap();
        let window = a.ui.windows.iter_mut().find(|w| w.id == source).unwrap();
        window.size = Some([950.0, 700.0]);
        window.view.zoom = 2.4;
        window.view.rotation = 18.0;
        window.view.center = [3.0, 4.0];
        window.view.fit_pending = false;
        let view = window.view.clone();
        let original = a.session.active().unwrap().doc.clone();
        let id = crate::duplicate_ui::open(&mut a).unwrap();
        a.ui.dialog_mut(id).unwrap().fields.insert("name".into(), json!("Copy"));
        let result = crate::dialogs::confirm(&mut a, id).unwrap();
        let copied = a.ui.windows.iter().find(|w| Some(w.id) == result["window"].as_u64()).unwrap();
        assert_eq!(copied.view, view);
        assert_eq!(copied.size, Some([950.0, 700.0]));
        assert_eq!(copied.position, Some([124.0, 124.0]));
        assert!(std::sync::Arc::ptr_eq(&a.session.documents()[0].doc, &original));
        assert_eq!(a.session.active().unwrap().history.past_len(), 0);
        assert!(a.session.active().unwrap().is_dirty());
        assert!(a.session.active().unwrap().path.is_none());
        a.run("image.canvasSize", json!({"width":40,"height":20})).unwrap();
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(a.session.active().unwrap().doc.size, original.size);
    }
}
