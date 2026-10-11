//! Dialogs rendered from `UiState::dialogs`. Field values live in the dialog data, so automation can
//! set them (`ui.dialog.set`) and confirm (`ui.dialog.confirm`) exactly like a user.

use serde_json::{Value, json};

use crate::PrismApp;
use crate::state::{Dialog, DialogKind};
use crate::widgets::{ButtonRole, DialogButton, dialog_buttons};

/// Where this frame's dialogs are on screen (the canvas reads last frame's: it draws first).
const RECTS: &str = "pc-dialog-rects";

fn rects(ctx: &egui::Context) -> Vec<egui::Rect> {
    ctx.data(|m| m.get_temp(egui::Id::new(RECTS))).unwrap_or_default()
}

/// With a dialog open the rest of the window is inert (egui's modal layer), but like Photoshop the
/// image can still be panned and zoomed under it. The pointer position when it is over `canvas`
/// and not over a dialog or one of its popups.
pub fn free_pointer_over(ctx: &egui::Context, canvas: egui::Rect) -> Option<egui::Pos2> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let p = ctx.pointer_hover_pos()?;
    let rects = rects(ctx);
    (canvas.contains(p) && !rects.iter().any(|r| r.contains(p))).then_some(p)
}

/// Pan drag under an open dialog: Space-drag, middle-drag, or a drag with the Hand tool, started on
/// the free canvas. Returns this frame's pointer movement.
pub fn pan_delta(ctx: &egui::Context, canvas: egui::Rect, hand: bool) -> Option<egui::Vec2> {
    let rects = rects(ctx);
    let (origin, panning, delta) = ctx.input(|i| {
        let p = &i.pointer;
        (p.press_origin(), p.middle_down() || (p.primary_down() && (hand || i.key_down(egui::Key::Space))), p.delta())
    });
    let origin = origin?;
    (panning && canvas.contains(origin) && !rects.iter().any(|r| r.contains(origin))).then_some(delta)
}

/// A click or drag with the primary button started on the free canvas under an open dialog, Space
/// not held: where the pointer is this frame. The press itself counts even when it is released in
/// the same frame (a quick click, `ui.click`); the drag only while it stays on the free canvas.
pub fn free_press(ctx: &egui::Context, canvas: egui::Rect) -> Option<egui::Pos2> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let rects = rects(ctx);
    let free = |p: egui::Pos2| canvas.contains(p) && !rects.iter().any(|r| r.contains(p));
    ctx.input(|i| {
        if i.key_down(egui::Key::Space) {
            return None;
        }
        let pressed = i.events.iter().rev().find_map(|e| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
            _ => None,
        });
        if let Some(p) = pressed {
            return free(p).then_some(p);
        }
        let held = i.pointer.primary_down() && i.pointer.press_origin().is_some_and(free);
        i.pointer.latest_pos().filter(|p| held && free(*p))
    })
}

pub fn show(app: &mut PrismApp, ctx: &egui::Context) {
    crate::native_dialogs::show(app, ctx);
    let dialogs = app.ui.dialogs.clone();
    let mut shown = Vec::new();
    for d in dialogs {
        if d.presentation == crate::state::DialogPresentation::Native || crate::doc_windows::viewport(d.window) != ctx.viewport_id() {
            continue;
        }
        if floats(&d) {
            floating(app, ctx, &d);
        } else if let Some(rect) = show_one(app, ctx, &d, false) {
            shown.push(rect);
        }
    }
    ctx.data_mut(|m| m.insert_temp(egui::Id::new(RECTS), shown));
}

/// A dialog opened from a detached document window gets an OS window of its own, above the
/// document windows, like Photoshop's floating dialogs: drawn inside the document window it would
/// be clipped to that window and could not be dragged past its edges.
pub fn floats(d: &Dialog) -> bool {
    d.presentation == crate::state::DialogPresentation::Embedded && d.window.is_some()
}

/// [`floats`]: the dialog in its own borderless window, dragged by the title row.
fn floating(app: &mut PrismApp, ctx: &egui::Context, d: &Dialog) {
    let key = egui::Id::new(("floating-dialog", d.id));
    crate::floating_window::show(app, ctx, key, crate::native_dialogs::viewport(d.id), display_title(d), |app, child| {
        show_one(app, child, d, !child.embed_viewports())
    });
}

/// Draws one dialog in `ctx`'s window. `own_window`: `ctx` is the dialog's own viewport
/// ([`floating`]). Returns the area it covers.
fn show_one(app: &mut PrismApp, ctx: &egui::Context, d: &Dialog, own_window: bool) -> Option<egui::Rect> {
    {
        if let Some(source) = d.source_document.and_then(|id| app.session.documents().iter().position(|s| s.doc.id == id)) {
            crate::doc_windows::activate(app, source, d.window);
        }
        let lang = if crate::prefs_ui::is_preferences(&d.fields) {
            crate::i18n::Lang::from_pref(d.fields.get("values").and_then(|v| v.pointer("/interface/language")).and_then(Value::as_str).unwrap_or("auto"))
        } else {
            crate::i18n::current()
        };
        let _language = crate::i18n::language_scope(lang);
        let mut fields = d.fields.clone();
        let mut outcome: Option<bool> = None; // Some(true)=OK, Some(false)=Cancel
        let mut apply_requested = false;
        let title = display_title(d);
        let id = egui::Id::new(("dialog", d.id));
        // Opens centred, then its top-left stays put (offset from the window's top-left, moved by
        // dragging the title bar; view state only, so egui memory): a dialog whose body grows, like
        // Layer Style switching effects, extends down and right instead of re-centring.
        let pinned: Option<egui::Vec2> = ctx.data(|m| m.get_temp(id));
        let mut drag = egui::Vec2::ZERO;
        let mut drag_started = false;
        let mut sizing = false;
        let area = match pinned {
            _ if own_window => crate::floating_window::area(id),
            Some(offset) => egui::Modal::default_area(id).anchor(egui::Align2::LEFT_TOP, offset),
            None => egui::Modal::default_area(id),
        };
        let color_picker = crate::color_picker_ui::owns(&fields);
        let about_card = d.kind == DialogKind::About && fields.get("systemInfo").and_then(Value::as_bool) != Some(true);
        let mut modal = egui::Modal::new(id).area(area).backdrop_color(egui::Color32::TRANSPARENT);
        if color_picker {
            modal = modal.frame(egui::Frame::popup(&ctx.global_style()).inner_margin(16));
        }
        if own_window && !color_picker && !about_card {
            // The OS window is the frame's edge: no shadow or rounding to clip.
            modal = modal.frame(crate::floating_window::frame(ctx));
        }
        if about_card {
            let t = crate::about_ui::palette();
            modal = modal.frame(
                egui::Frame::popup(&ctx.global_style())
                    .fill(t.card)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius((t.radius_lg * 2.0).round() as u8)
                    .inner_margin(crate::about_ui::margins(ctx)),
            );
        }
        // Photoshop doesn't dim the window behind dialogs: previews must be judged at true contrast.
        let modal = modal.show(ctx, |ui| {
            sizing = ui.is_sizing_pass();
            ui.set_min_width(380.0);
            let wide = crate::prefs_ui::width(&d.fields);
            if let Some(w) = wide {
                ui.set_min_width(w.min(460.0));
            }
            if d.kind == DialogKind::NewDocument {
                ui.set_min_width(800.0);
            }
            // A single studio-branded About card, constrained to the window.
            let about_overview = about_card;
            if about_overview {
                ui.set_min_width(crate::about_ui::width(ctx));
                ui.style_mut().visuals = egui::Visuals::light();
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.style_mut().visuals.override_text_color = Some(crate::about_ui::palette().text);
            }
            ui.set_max_width(wide.unwrap_or(if d.kind == DialogKind::NewDocument {
                800.0
            } else if about_overview {
                crate::about_ui::width(ctx)
            } else if d.kind == DialogKind::LayerStyle || d.fields.contains_key("__export") || crate::color_picker_ui::owns(&d.fields) {
                600.0
            } else {
                440.0
            }));
            if let Some(w) = crate::file_ui::dialog_width(&d.fields) {
                ui.set_min_width(w);
                ui.set_max_width(w);
            }
            if color_picker {
                ui.set_width(crate::color_picker_ui::CONTENT_WIDTH);
            }
            if about_card {
                let header = crate::about_ui::header(ui);
                let r = ui.interact(header.with_max_x(ui.max_rect().right()), id.with("title"), egui::Sense::drag());
                drag = r.drag_delta();
                drag_started |= r.drag_started_by(egui::PointerButton::Primary);
                ui.add_space(if crate::about_ui::compact(ctx) { 16.0 } else { 42.0 });
            } else {
                let label = egui::Label::new(egui::RichText::new(&title).font(crate::theme::semibold(15.0))).selectable(false);
                let t = if color_picker { ui.add_sized(egui::vec2(ui.available_width(), 22.0), label).rect } else { ui.add(label).rect };
                let bar = egui::Rect::from_min_max(t.min, egui::pos2(ui.max_rect().right(), t.bottom()));
                let r = ui.interact(bar, id.with("title"), egui::Sense::drag());
                drag = r.drag_delta();
                drag_started |= r.drag_started_by(egui::PointerButton::Primary);
                ui.add_space(4.0);
                crate::widgets::hairline(ui);
                ui.add_space(8.0);
            }
            match d.kind {
                DialogKind::NewDocument => crate::new_doc_ui::body(app, ui, &mut fields),
                DialogKind::About if fields.get("systemInfo").and_then(Value::as_bool) == Some(true) => {
                    let lines = crate::gpu_status::system_info(app);
                    for l in &lines {
                        ui.add(egui::Label::new(egui::RichText::new(l).font(crate::theme::mono(12.0))).selectable(true));
                    }
                    ui.add_space(8.0);
                    if crate::widgets::secondary_button(ui, tl!("Copy"), 84.0).clicked() {
                        ui.ctx().copy_text(lines.join("\n"));
                    }
                }
                DialogKind::About => crate::about_ui::body(ui),
                DialogKind::Command if crate::fill_ui::owns(&fields) => crate::fill_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::rasterize_prompt::owns(&fields) => crate::rasterize_prompt::body(ui, &fields),
                DialogKind::Command if crate::variables_ui::owns(&fields) => crate::variables_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::file_ui::owns(&fields) => crate::file_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::color_picker_ui::owns(&fields) => {
                    outcome = crate::color_picker_ui::body(ui, &mut fields);
                    crate::color_picker_ui::take_add_swatch(app, &mut fields);
                }
                DialogKind::Command if crate::color_range_ui::owns(&fields) => crate::color_range_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::prefs_ui::owns(&fields) => crate::prefs_ui::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__export") => crate::export_dialog::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__sizing") => crate::sizing::body(app, ui, &mut fields),
                DialogKind::Command if crate::adjust_dialog::owns(&fields) => crate::adjust_dialog::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__filter") => {
                    // Color Settings: the monitor profile in use can change while it is open.
                    if fields.get("__command").and_then(Value::as_str) == Some("edit.colorSettings") {
                        fields.insert("__note".into(), Value::String(crate::monitor_status::note(app)));
                    }
                    crate::filter_dialog::body(ui, &mut fields)
                }
                DialogKind::Command if fields.contains_key("__form") => crate::view_cmds::form_body(ui, &mut fields),
                DialogKind::Command => {}
                DialogKind::LayerStyle => crate::layer_style::body(app, ui, &mut fields),
                DialogKind::Error => {
                    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or("Error"));
                }
            }
            if crate::color_picker_ui::owns(&fields) {
                if outcome.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    outcome = Some(true);
                }
            } else {
                ui.add_space(if about_card && !crate::about_ui::compact(ctx) { 34.0 } else { 8.0 });
                // Align::Min, not Center: a centred row fills the height left over from last frame's
                // (larger) size, so a dialog whose body gets shorter would never shrink back.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if about_card {
                        if crate::about_ui::ok_button(ui) {
                            outcome = Some(false);
                        }
                    } else if matches!(d.kind, DialogKind::About | DialogKind::Error) {
                        if dialog_buttons(ui, &[DialogButton::new(ButtonRole::Default, tl!("OK"), 84.0)]).is_some() {
                            outcome = Some(false);
                        }
                    } else {
                        let ok_label = if d.kind == DialogKind::NewDocument {
                            tl!("Create")
                        } else if d.fields.contains_key("__export") {
                            tl!("Export")
                        } else {
                            crate::file_ui::ok_label(&d.fields).unwrap_or(tl!("OK"))
                        };
                        let ok = DialogButton::new(ButtonRole::Default, ok_label, 84.0);
                        // Photoshop: holding Alt turns Cancel into Reset (the dialog stays open).
                        let reset = ui.input(|i| i.modifiers.alt) && crate::adjust_dialog::resets(&fields);
                        let cancel_label = if reset {
                            tl!("Reset")
                        } else if d.kind == DialogKind::NewDocument {
                            tl!("Close")
                        } else {
                            tl!("Cancel")
                        };
                        let cancel = DialogButton::new(ButtonRole::Cancel, cancel_label, 84.0);
                        let clicked = if d.kind == DialogKind::Command && crate::prefs_ui::is_preferences(&fields) {
                            let changed = crate::prefs_ui::preferences_changed(app, &fields);
                            dialog_buttons(ui, &[ok, cancel, DialogButton::new(ButtonRole::Apply, tl!("Apply"), 84.0).enabled(changed)])
                        } else {
                            dialog_buttons(ui, &[ok, cancel])
                        };
                        match clicked {
                            Some(ButtonRole::Cancel) if reset => crate::adjust_dialog::reset(ui.ctx(), &mut fields),
                            Some(ButtonRole::Cancel) => outcome = Some(false),
                            Some(ButtonRole::Apply) => apply_requested = true,
                            Some(_) => outcome = Some(true),
                            None if ui.input(|i| i.key_pressed(egui::Key::Enter)) => outcome = Some(true),
                            None => {}
                        }
                    }
                });
            }
            // The frame around the content (Frame::popup's margin and stroke).
            ui.min_rect().expand(ui.spacing().menu_margin.sum().max_elem() + 2.0)
        });
        // The picker has a larger frame margin; include that whole frame in the canvas hit guard.
        let covered = if color_picker || about_card || own_window { modal.response.rect } else { modal.inner };
        if own_window && drag_started {
            // The OS moves its own window (egui would see the pointer move with the window).
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        // Pin once laid out at its real size (the first frame is an invisible sizing pass).
        // About stays centred while its font/layout metrics settle; pin only after a drag.
        // Re-clamp dragged About cards on resize so the fixed footer remains onscreen.
        if !own_window
            && !sizing
            && (pinned.is_none() || drag != egui::Vec2::ZERO || about_card)
            && (!about_card || pinned.is_some() || drag != egui::Vec2::ZERO)
        {
            let screen = ctx.content_rect();
            // Keep the whole dialog (and so its title bar) on screen.
            let room = (screen.size() - modal.response.rect.size()).max(egui::Vec2::ZERO);
            let offset = pinned.unwrap_or(modal.response.rect.min - screen.min) + drag;
            ctx.data_mut(|m| m.insert_temp(id, offset.clamp(egui::Vec2::ZERO, room)));
        }
        // Esc cancels (topmost dialog, no popup open). A click outside does nothing: Photoshop keeps
        // the dialog, and the pointer may be panning or zooming the canvas under it.
        if outcome.is_none()
            && (modal.response.should_close()
                || (modal.is_top_modal && !modal.any_popup_open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))))
        {
            outcome = Some(false);
        }
        if let Some(dm) = app.ui.dialog_mut(d.id) {
            dm.fields = fields;
        }
        if apply_requested && outcome.is_none() {
            let _ = crate::prefs_ui::apply(app, d.id);
        }
        match outcome {
            Some(true) => {
                let _ = confirm(app, d.id);
            }
            Some(false) => {
                app.ui.close_dialog(d.id);
                app.filter_preview = None;
                app.color_range = None;
            }
            None => {}
        }
        // A floating dialog's first frame is an invisible sizing pass: not its real size yet.
        (!(own_window && sizing)).then_some(covered)
    }
}

/// The dialog title as shown: [`title`] in the UI language.
fn display_title(d: &Dialog) -> String {
    match d.kind {
        DialogKind::Command => {
            let label = d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command");
            // Dialog labels are catalogued with their "…" ("Export As…"); some commands omit it.
            let with_dots = format!("{}…", label.trim_end_matches('…'));
            let shown = if crate::i18n::has(crate::i18n::current(), label) { tl!(label) } else { tl!(&with_dots) };
            shown.trim_end_matches('…').to_string()
        }
        _ => tl!(&title(d)).to_string(),
    }
}

pub fn title(d: &Dialog) -> String {
    match d.kind {
        DialogKind::NewDocument => "New Document".into(),
        DialogKind::About if d.fields.get("systemInfo").and_then(Value::as_bool) == Some(true) => "System Info".into(),
        DialogKind::About => "About Prism".into(),
        DialogKind::LayerStyle => "Layer Style".into(),
        DialogKind::Command => d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command").trim_end_matches('…').to_string(),
        DialogKind::Error => "Error".into(),
    }
}

/// Confirm a dialog: run its action and close it. Used by the OK button and by automation.
pub fn confirm(app: &mut PrismApp, id: u64) -> Result<Value, String> {
    let d = app.ui.dialogs.iter().find(|d| d.id == id).cloned().ok_or_else(|| format!("no dialog {id}"))?;
    if d.source_document.is_none() && d.presentation != crate::state::DialogPresentation::Native {
        return confirm_inner(app, id);
    }
    let mut result = (|| {
        crate::native_dialogs::source(app, &d)?;
        if d.fields.contains_key("__sizing") {
            crate::sizing::validate(&d.fields)?;
        }
        if d.fields.contains_key("__duplicate") && d.fields.get("name").and_then(Value::as_str).is_none_or(|n| n.trim().is_empty()) {
            return Err("enter a document name".into());
        }
        confirm_inner(app, id)
    })();
    if d.fields.contains_key("__duplicate")
        && let Ok(value) = result
    {
        result = Ok(crate::duplicate_ui::finish(app, &d, value));
    }
    if let Err(ref error) = result {
        if app.ui.dialog_mut(id).is_none() {
            app.ui.dialogs.push(d);
        }
        if let Some(dialog) = app.ui.dialog_mut(id) {
            dialog.fields.insert("__error".into(), json!(error));
        }
    }
    result
}
fn confirm_inner(app: &mut PrismApp, id: u64) -> Result<Value, String> {
    let d = app.ui.close_dialog(id).ok_or_else(|| format!("no dialog {id}"))?;
    match d.kind {
        DialogKind::NewDocument => {
            let r = app.run("file.new", crate::new_doc_ui::command_params(&d.fields));
            if let Some(i) = app.session.active_index() {
                app.update_view(i, |v| v.fit_pending = true);
            }
            r
        }
        DialogKind::Command if crate::fill_ui::owns(&d.fields) => crate::fill_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::rasterize_prompt::owns(&d.fields) => crate::rasterize_prompt::confirm(app, &d.fields),
        DialogKind::Command if crate::variables_ui::owns(&d.fields) => crate::variables_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::file_ui::owns(&d.fields) => crate::file_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_picker_ui::owns(&d.fields) => crate::color_picker_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_range_ui::owns(&d.fields) => crate::color_range_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::prefs_ui::owns(&d.fields) => crate::prefs_ui::confirm(app, &d.fields),
        DialogKind::Command if d.fields.contains_key("__export") => crate::export_dialog::confirm(app, &d.fields),
        DialogKind::Command => {
            app.filter_preview = None;
            let cmd = d.fields.get("__command").and_then(|v| v.as_str().map(str::to_string)).ok_or("dialog has no command")?;
            let (cmd, params) = crate::smart_ui::confirm_command(&d.fields, cmd, crate::filter_dialog::params_of(&d.fields));
            let result = app.run(&cmd, params.clone());
            if result.is_ok() && cmd == "view.newGuideLayout" {
                app.ui.view.guide_layout = params;
            }
            result
        }
        DialogKind::LayerStyle => crate::layer_style::confirm(app, &d.fields),
        DialogKind::About | DialogKind::Error => Ok(Value::Null),
    }
}

/// Open the parameter dialog of `command`: the adjustment editor for `image.adjustments.*`, the
/// schema dialog for filters, the Color Range dialog for `select.colorRange`, otherwise a bare
/// confirm dialog.
pub fn open_command_dialog(app: &mut PrismApp, command: &str, label: &str) -> u64 {
    if command == "image.duplicate"
        && let Ok(id) = crate::duplicate_ui::open(app)
    {
        return id;
    }
    if crate::sizing::is_sizing(command)
        && let Some(id) = crate::sizing::open(app, command)
    {
        return id;
    }
    if command == crate::color_range_ui::COMMAND {
        return crate::color_range_ui::open(app);
    }
    if let Some(id) = crate::adjust_dialog::open(app, command) {
        return id;
    }
    if crate::filter_dialog::has_dialog(command)
        && let Some(id) = crate::filter_dialog::open(app, command)
    {
        return id;
    }
    let mut fields = serde_json::Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    app.ui.open_dialog(DialogKind::Command, fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_overview_uses_the_prism_product_name() {
        use egui_kittest::{Harness, kittest::Queryable};

        let app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(egui::vec2(1100.0, 740.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().ui.open_dialog(DialogKind::About, serde_json::Map::new());
        h.run_steps(4);
        assert!(h.query_by_label("Prism").is_some());
        assert!(h.query_by_label("NK Studio").is_some());
        assert!(h.query_by_label("© 2026 NK Studio. All rights reserved.").is_some());
        assert!(h.query_by_label(&format!("Version {}", prism_engine::build_info::VERSION)).is_some());
    }

    #[test]
    fn color_picker_has_inset_content_and_actions_at_the_right_edge() {
        use egui_kittest::{Harness, kittest::Queryable};

        for size in [egui::vec2(760.0, 480.0), egui::vec2(960.0, 640.0)] {
            let app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
            let mut h = Harness::builder().with_size(size).build_ui_state(|ui, app| show(app, ui.ctx()), app);
            PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            crate::color_picker_ui::open(h.state_mut(), "foreground");
            h.run_steps(4);
            let frame = rects(&h.ctx)[0];
            let title = h.get_by_label("Color Picker (Foreground Color)").rect();
            let ok = h.get_by_label("OK").rect();
            let cancel = h.get_by_label("Cancel").rect();
            let web = h.get_by_label("Only Web Colors").rect();
            assert!((ok.right() - title.right()).abs() < 1.0, "OK aligns with the right content edge: {ok:?} vs {title:?}");
            assert!((cancel.right() - ok.right()).abs() < 1.0);
            assert!(frame.right() - ok.right() >= 16.0, "buttons retain the window's padding");
            assert!(web.left() - frame.left() >= 16.0, "checkbox has comfortable left padding");
            assert!(frame.bottom() - web.bottom() >= 16.0, "checkbox has comfortable bottom padding");
            assert!(title.top() - frame.top() >= 16.0, "title has top padding");
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(frame), "padded picker fits {size:?}: {frame:?}");
            // A click anywhere inside the added margin must stay out of the canvas eyedropper.
            let margin = egui::pos2(frame.left() + 8.0, web.center().y);
            h.hover_at(margin);
            h.run_steps(1);
            assert!(free_pointer_over(&h.ctx, egui::Rect::from_min_size(egui::Pos2::ZERO, size)).is_none());
        }
    }

    #[test]
    fn dragging_the_title_bar_moves_the_dialog() {
        use egui_kittest::{Harness, kittest::Queryable};

        let app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PrismApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
        harness.run_steps(3);
        let before = harness.get_by_label("Layer Style").rect();

        // Grab the title text itself: it must move the dialog, not select the text.
        let from = before.center();
        harness.hover_at(from);
        harness.drag_at(from);
        harness.run_steps(2);
        for i in 1..=10 {
            harness.hover_at(from + egui::vec2(-12.0, 8.0) * i as f32);
            harness.run_steps(1);
        }
        harness.drop_at(from + egui::vec2(-120.0, 80.0));
        harness.run_steps(3);

        let moved = harness.get_by_label("Layer Style").rect().min - before.min;
        assert!((moved - egui::vec2(-120.0, 80.0)).length() < 1.0, "dialog moved by {moved:?}");
        assert_eq!(harness.state().ui.dialogs.len(), 1, "dragging must not close the dialog");
    }
}
