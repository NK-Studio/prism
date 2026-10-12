//! Native document ownership, independent cameras and tab drag-and-drop. No document is cloned
//! when a tab floats: every view dispatches to the same engine Session and history.
use crate::{
    PrismApp,
    state::{DocWindow, DocWindowKind, View},
};
use egui::{Context, Id, Pos2, Rect, ViewportId};
use prism_doc::DocId;
use serde_json::{Value, json};

pub(crate) fn viewport(window: Option<u64>) -> ViewportId {
    window.map_or(ViewportId::ROOT, |id| ViewportId::from_hash_of(("docwin", id)))
}

impl PrismApp {
    /// Camera targeted by the panels and View commands. Other documents retain their main view.
    pub fn view(&self, document: usize) -> Option<&View> {
        self.ui
            .active_window
            .and_then(|id| self.ui.windows.iter().find(|w| w.id == id && w.document == document && w.open))
            .map(|w| &w.view)
            .or_else(|| self.ui.views.get(document))
    }
    pub fn view_mut(&mut self, document: usize) -> Option<&mut View> {
        let window = self.ui.active_window.and_then(|id| self.ui.windows.iter().position(|w| w.id == id && w.document == document && w.open));
        match window {
            Some(i) => self.ui.windows.get_mut(i).map(|w| &mut w.view),
            None => self.ui.views.get_mut(document),
        }
    }
    pub(crate) fn camera(&self, document: usize) -> View {
        self.view(document).cloned().unwrap_or_default()
    }
    pub(crate) fn update_view(&mut self, document: usize, edit: impl FnOnce(&mut View)) {
        if let Some(v) = self.view_mut(document) {
            edit(v);
        }
    }
}

/// Whether Prism is foreground, including focus in any detached window.
pub(crate) fn application_active(app: &PrismApp, ctx: &Context) -> bool {
    app.services.application_active.as_ref().map_or_else(|| ctx.input(|i| i.raw.viewports.values().any(|v| v.focused == Some(true))), |active| active())
}

/// Floating documents stay above the editor while Prism is active. Two exceptions use ordinary
/// stacking: the unsaved-changes prompt's native save picker (a root-window panel an always-on-top
/// canvas could cover; the prompt itself floats above them), and a floating dialog
/// ([`crate::dialogs::floats`]), the one window kept on top so clicking its document to pan the
/// preview cannot bury it.
pub(crate) fn window_level(app: &PrismApp, ctx: &Context) -> egui::viewport::WindowLevel {
    let save_picker = app.discard.is_some() && app.file_dialog_open();
    if !save_picker && !app.ui.dialogs.iter().any(crate::dialogs::floats) && application_active(app, ctx) {
        egui::viewport::WindowLevel::AlwaysOnTop
    } else {
        egui::viewport::WindowLevel::Normal
    }
}

/// The level of a floating modal's own window ([`crate::floating_window`]): above the document
/// windows while Prism is active.
pub(crate) fn dialog_level(app: &PrismApp, ctx: &Context) -> egui::viewport::WindowLevel {
    if application_active(app, ctx) { egui::viewport::WindowLevel::AlwaysOnTop } else { egui::viewport::WindowLevel::Normal }
}

pub(crate) fn sync_window_level(app: &PrismApp, ctx: &Context) {
    let level = window_level(app, ctx);
    let key = Id::new(("document-window-level", ctx.viewport_id()));
    let old = ctx.data(|d| d.get_temp::<egui::viewport::WindowLevel>(key));
    if old != Some(level) {
        ctx.data_mut(|d| d.insert_temp(key, level));
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        ctx.request_repaint();
    }
}

pub(crate) fn docked(app: &PrismApp) -> Vec<usize> {
    (0..app.session.documents().len()).filter(|i| !app.ui.windows.iter().any(|w| w.open && w.document == *i && w.kind == DocWindowKind::Detached)).collect()
}

pub(crate) fn main_index(app: &PrismApp) -> Option<usize> {
    let docked = docked(app);
    let preferred = if app.ui.active_window.is_none() { app.session.active_index() } else { None };
    preferred
        .filter(|i| docked.contains(i))
        .or_else(|| app.ui.main_document.and_then(|id| app.session.documents().iter().position(|d| d.doc.id == id)).filter(|i| docked.contains(i)))
        .or_else(|| docked.first().copied())
}

pub(crate) fn sync(app: &mut PrismApp) {
    if app.ui.active_window.is_some_and(|id| !app.ui.windows.iter().any(|w| w.id == id && w.open && Some(w.document) == app.session.active_index())) {
        app.ui.active_window = None;
    }
    if let Some(i) = main_index(app) {
        app.ui.main_document = app.session.documents().get(i).map(|d| d.doc.id);
    } else {
        app.ui.main_document = None;
    }
}

/// Finish text in its source document and discard held gestures before another view takes input.
pub(crate) fn activate(app: &mut PrismApp, document: usize, window: Option<u64>) {
    if app.session.active_index() != Some(document) {
        if app.ui.text_edit.is_some() {
            crate::type_tool::commit(app);
        }
        crate::transform_tool::cancel(app);
        if app.ui.tool == crate::state::Tool::Crop {
            crate::crop_ui::dismiss_frame(app);
        }
        app.ui.polygon.clear();
    }
    if app.session.active_index() != Some(document) || app.ui.active_window != window {
        app.drag = None;
        app.live_stroke = None;
        app.move_preview = None;
        crate::move_mods::abandon(app);
        crate::type_transform::cancel_drag(app);
        crate::shape_corners_ui::cancel_view_gesture(app);
        crate::arrow_handles_ui::cancel_view_gesture(app);
    }
    app.session.set_active(document);
    app.ui.active_window = window;
    app.jobs.focus = None;
}

pub(crate) fn activate_main(app: &mut PrismApp, document: usize) {
    if !docked(app).contains(&document) {
        return;
    }
    activate(app, document, None);
    if app.session.active_index() == Some(document) {
        app.ui.main_document = app.session.documents().get(document).map(|d| d.doc.id);
        app.jobs.focus = None;
    }
}

#[derive(Clone)]
struct RequestedFocus {
    window: u64,
    until: std::time::Instant,
}
fn focus_key() -> Id {
    Id::new("document-requested-focus")
}
pub(crate) fn requested_focus(ctx: &Context) -> Option<u64> {
    ctx.data(|d| d.get_temp::<RequestedFocus>(focus_key())).filter(|f| f.until > std::time::Instant::now()).map(|f| f.window)
}

pub fn focus(app: &mut PrismApp, ctx: &Context, window: u64) -> Result<Value, String> {
    let document = app.ui.windows.iter().find(|w| w.open && w.id == window).map(|w| w.document).ok_or_else(|| format!("no window {window}"))?;
    if app.session.documents().get(document).is_none() {
        return Err("window's document is closed".into());
    }
    activate(app, document, Some(window));
    ctx.data_mut(|d| d.insert_temp(focus_key(), RequestedFocus { window, until: std::time::Instant::now() + std::time::Duration::from_secs(2) }));
    ctx.send_viewport_cmd_to(viewport(Some(window)), egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd_to(viewport(Some(window)), egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd_to(viewport(Some(window)), egui::ViewportCommand::Focus);
    ctx.request_repaint();
    Ok(json!({"window": window, "document": document}))
}

pub(crate) fn focus_native(app: &mut PrismApp, ctx: &Context) {
    // Focus commands take an OS event-loop turn to arrive. Keep the requested document
    // stable through that turn, rather than letting stale viewport info retarget input.
    if crate::native_dialogs::is_open(app) {
        return;
    }
    if let Some(id) = requested_focus(ctx)
        && let Some(document) = app.ui.windows.iter().find(|w| w.id == id && w.open).map(|w| w.document)
    {
        activate(app, document, Some(id));
        return;
    }
    let target = ctx.input(|i| {
        app.ui.windows.iter().find(|w| i.raw.viewports.get(&viewport(Some(w.id))).is_some_and(|v| v.focused == Some(true))).map(|w| (w.id, w.document))
    });
    if let Some((id, document)) = target
        && app.ui.active_window != Some(id)
        && app.ui.dialogs.is_empty()
        && app.discard.is_none()
    {
        activate(app, document, Some(id));
    }
}

pub fn detach(app: &mut PrismApp, ctx: &Context, document: usize, position: Option<[f32; 2]>) -> Result<Value, String> {
    if app.session.documents().get(document).is_none() {
        return Err(format!("no document {document}"));
    }
    if position.is_some_and(|p| !p.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0)) {
        return Err("invalid window position".into());
    }
    if let Some(id) = app.ui.windows.iter().find(|w| w.open && w.document == document && w.kind == DocWindowKind::Detached).map(|w| w.id) {
        return focus(app, ctx, id);
    }
    app.sync_views();
    let id = app.ui.alloc_id();
    let view = app.ui.views.get(document).cloned().unwrap_or_default();
    app.ui.windows.push(DocWindow { size: None, id, document, view, open: true, kind: DocWindowKind::Detached, position });
    sync(app);
    focus(app, ctx, id)
}

pub fn additional(app: &mut PrismApp, document: usize) -> Result<Value, String> {
    if app.session.documents().get(document).is_none() {
        return Err(format!("no document {document}"));
    }
    app.sync_views();
    let id = app.ui.alloc_id();
    app.ui.windows.push(DocWindow { size: None, id, document, view: app.camera(document), open: true, kind: DocWindowKind::Additional, position: None });
    Ok(json!({"window": id}))
}

pub fn attach(app: &mut PrismApp, ctx: &Context, window: u64, before: Option<DocId>) -> Result<Value, String> {
    let w = app.ui.windows.iter().find(|w| w.open && w.id == window).cloned().ok_or_else(|| format!("no window {window}"))?;
    if w.kind != DocWindowKind::Detached {
        return Err("window is an additional view, not a detached tab".into());
    }
    let doc = app.session.documents().get(w.document).map(|d| d.doc.id).ok_or("window's document is closed")?;
    let slot = before.and_then(|id| app.session.documents().iter().position(|d| d.doc.id == id)).unwrap_or(app.session.documents().len());
    let to = if before.is_some() && w.document < slot { slot.saturating_sub(1) } else { slot };
    if before != Some(doc) {
        app.run("document.move", json!({"document": w.document, "to": to}))?;
    }
    let current = app.ui.windows.iter().find(|w| w.id == window).cloned().ok_or("window closed while attaching")?;
    if let Some(v) = app.ui.views.get_mut(current.document) {
        *v = current.view;
    }
    app.ui.windows.retain(|w| w.id != window);
    ctx.data_mut(|d| d.remove::<RequestedFocus>(focus_key()));
    app.ui.active_window = None;
    let document = app.session.documents().iter().position(|d| d.doc.id == doc).ok_or("document closed while attaching")?;
    activate_main(app, document);
    ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Focus);
    ctx.request_repaint();
    Ok(json!({"document": document}))
}

pub(crate) fn consolidate(app: &mut PrismApp) {
    for w in &app.ui.windows {
        if w.kind == DocWindowKind::Detached
            && let Some(v) = app.ui.views.get_mut(w.document)
        {
            *v = w.view.clone();
        }
    }
    app.ui.windows.clear();
    app.ui.active_window = None;
    app.ui.view.arrange = "tabs".into();
    sync(app);
}

pub fn close(app: &mut PrismApp, ctx: &Context, window: u64) -> Result<Value, String> {
    let w = app.ui.windows.iter().find(|w| w.open && w.id == window).cloned().ok_or_else(|| format!("no window {window}"))?;
    if app.ui.dialogs.iter().any(|d| d.window == Some(window)) || app.file_dialog_open() {
        return Err("close the dialog before closing this document window".into());
    }
    if w.kind == DocWindowKind::Detached {
        app.session.set_active(w.document);
        app.ui.active_window = Some(window);
        crate::menus::invoke(app, ctx, "file.close", json!({"document": w.document}))
    } else {
        app.ui.windows.retain(|w| w.id != window);
        if app.ui.active_window == Some(window) {
            app.ui.active_window = None;
        }
        sync(app);
        Ok(Value::Null)
    }
}

/// egui's immediate viewports bypass eframe's raw_input_hook. This hook feeds automation and
/// normalizes clipboard keys before egui performs hit-testing, in every native viewport.
pub(crate) struct NativeInput;
#[derive(Clone, Default)]
struct InputQueue {
    events: Vec<egui::Event>,
}
fn input_key(id: ViewportId) -> Id {
    Id::new(("document-input", id))
}
fn automated_key(id: ViewportId) -> Id {
    Id::new(("document-automated", id))
}
/// A click's move and press share a pass; motion during a held drag and release need
/// separate passes, as do key press/release, so native hit-testing sees every transition.
pub(crate) fn input_step_len(events: &[egui::Event]) -> usize {
    for (i, event) in events.iter().enumerate() {
        match event {
            egui::Event::PointerMoved(_) if matches!(events.get(i + 1), Some(egui::Event::PointerButton { pressed: true, .. })) => {}
            egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::Key { .. } => return i + 1,
            _ => {}
        }
    }
    events.len()
}

impl egui::plugin::Plugin for NativeInput {
    fn debug_name(&self) -> &'static str {
        "Prism document input"
    }
    fn input_hook(&mut self, ctx: &Context, raw: &mut egui::RawInput) {
        let id = raw.viewport_id;
        if id != ViewportId::ROOT {
            let typing = ctx.data(|d| d.get_temp::<bool>(Id::new("document-typing"))).unwrap_or(false)
                || ctx.data(|d| d.get_temp::<bool>(Id::new(("document-text-focus", id)))).unwrap_or(false);
            crate::shortcuts::clipboard_keys(ctx, typing, raw);
        }
        let events = ctx.data_mut(|d| {
            let mut q = d.get_temp::<InputQueue>(input_key(id)).unwrap_or_default();
            let n = input_step_len(&q.events);
            let events: Vec<_> = q.events.drain(..n).collect();
            d.insert_temp(input_key(id), q);
            d.insert_temp(automated_key(id), !events.is_empty());
            events
        });
        let synthetic = !events.is_empty();
        let native_count = raw.events.len();
        raw.events.extend(events);
        let automated_drag = ctx.data(|d| d.get_temp::<TabDrag>(drag_key())).is_some_and(|d| d.automated);
        if automated_drag && !synthetic && raw.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. })) {
            cancel_drag(ctx);
        }
        if !synthetic && raw.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true) | egui::Event::PointerButton { pressed: true, .. })) {
            ctx.data_mut(|d| d.remove::<RequestedFocus>(focus_key()));
        }
        if raw.viewports.values().all(|v| v.focused == Some(false)) {
            cancel_drag(ctx);
        }
        if raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })) {
            cancel_drag(ctx);
        }
        // Record pointer coordinates in physical desktop pixels, so a tab crosses windows with
        // different DPIs without confusing either viewport's local coordinate system.
        if let Some(info) = raw.viewports.get(&id)
            && let Some(inner) = info.inner_rect
        {
            let scale = info.native_pixels_per_point.unwrap_or(1.0) * ctx.zoom_factor();
            for (index, e) in raw.events.iter().enumerate() {
                if automated_drag && index < native_count {
                    continue;
                }
                match e {
                    egui::Event::PointerMoved(p) => record_pointer(ctx, (*p + inner.min.to_vec2()) * scale, None),
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } => {
                        record_pointer(ctx, (*pos + inner.min.to_vec2()) * scale, Some(*pressed))
                    }
                    egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => ctx.data_mut(|d| d.remove::<TabDrag>(drag_key())),
                    _ => {}
                }
            }
        }
    }
}

/// The unsaved-changes prompt is app-modal and has its own window: automation input aimed at any
/// window (or none) answers it, as the windows behind it take none.
pub(crate) fn prompt_takes_input(app: &PrismApp, ctx: &Context) -> bool {
    app.discard.is_some() && !ctx.embed_viewports() && app.synthetic_dialog.is_none()
}

pub(crate) fn queue_input(app: &mut PrismApp, ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(Id::new("document-typing"), app.ui.text_edit.is_some()));
    let prompt = prompt_takes_input(app, ctx);
    if app.synthetic_window.is_some() || app.synthetic_dialog.is_some() || prompt {
        let prompt = prompt.then(crate::discard_ui::viewport);
        let id = prompt.or(app.synthetic_dialog.map(crate::native_dialogs::viewport)).unwrap_or_else(|| viewport(app.synthetic_window));
        let events = std::mem::take(&mut app.synthetic);
        ctx.data_mut(|d| {
            let mut q = d.get_temp::<InputQueue>(input_key(id)).unwrap_or_default();
            q.events.extend(events);
            d.insert_temp(input_key(id), q);
        });
    }
}
pub(crate) fn queue_drag_input(app: &mut PrismApp, ctx: &Context) {
    let id = viewport(app.synthetic_window);
    let events = std::mem::take(&mut app.synthetic);
    ctx.data_mut(|d| {
        let mut q = d.get_temp::<InputQueue>(input_key(id)).unwrap_or_default();
        q.events.extend(events);
        d.insert_temp(input_key(id), q);
    });
}

pub(crate) fn automated(ctx: &Context) -> bool {
    let id = ctx.viewport_id();
    ctx.data(|d| d.get_temp::<bool>(automated_key(id))).unwrap_or(false)
}
pub(crate) fn input_pending(app: &PrismApp, ctx: &Context) -> bool {
    let queued = |id: ViewportId| ctx.data(|d| d.get_temp::<InputQueue>(input_key(id))).is_some_and(|q| !q.events.is_empty());
    queued(ViewportId::ROOT)
        || (app.discard.is_some() && queued(crate::discard_ui::viewport()))
        || app
            .ui
            .dialogs
            .iter()
            .filter(|d| crate::native_dialogs::own_window(d))
            .any(|d| ctx.data(|m| m.get_temp::<InputQueue>(input_key(crate::native_dialogs::viewport(d.id)))).is_some_and(|q| !q.events.is_empty()))
        || app.ui.windows.iter().any(|w| ctx.data(|d| d.get_temp::<InputQueue>(input_key(viewport(Some(w.id))))).is_some_and(|q| !q.events.is_empty()))
}

#[derive(Clone)]
struct TabDrag {
    document: DocId,
    window: Option<u64>,
    strip: Rect,
    pointer: Pos2,
    released: bool,
    scale: f32,
    start: Pos2,
    window_origin: Option<Pos2>,
    automated: bool,
}
fn drag_key() -> Id {
    Id::new("document-tab-drag")
}
fn record_pointer(ctx: &Context, pointer: Pos2, down: Option<bool>) {
    ctx.data_mut(|d| {
        if let Some(mut drag) = d.get_temp::<TabDrag>(drag_key()) {
            drag.pointer = pointer;
            if let Some(down) = down {
                drag.released = !down;
            }
            d.insert_temp(drag_key(), drag);
        }
    });
}
fn desktop_rect(ctx: &Context, local: Rect) -> Option<Rect> {
    let scale = ctx.pixels_per_point();
    ctx.input(|i| i.viewport().inner_rect.map(|inner| local.translate(inner.min.to_vec2()) * scale))
}

pub(crate) fn tab_drag(app: &PrismApp, ctx: &Context, document: usize, window: Option<u64>, strip: Rect, response: &egui::Response) {
    if !response.drag_started()
        || !app.ui.dialogs.is_empty()
        || app.discard.is_some()
        || app.file_dialog_open()
        || app.drag.is_some()
        || app.guide_drag.is_some()
        || app.ui.shape_corners.is_some()
        || app.ui.arrow_handles.is_some()
        || app.ui.type_transform.is_some()
        || crate::layer_transfer::pointer_if_armed(app, ctx).is_some()
    {
        return;
    }
    let Some(doc) = app.session.documents().get(document) else { return };
    let Some(strip) = desktop_rect(ctx, strip) else { return };
    let start = ctx.input(|i| i.pointer.press_origin()).and_then(|p| desktop_rect(ctx, Rect::from_min_max(p, p)).map(|r| r.min)).unwrap_or(strip.center());
    let window_origin = window.and_then(|_| ctx.input(|i| i.viewport().outer_rect.or(i.viewport().inner_rect))).map(|r| r.min * ctx.pixels_per_point());
    let pointer = response.interact_pointer_pos().and_then(|p| desktop_rect(ctx, Rect::from_min_max(p, p)).map(|r| r.min)).unwrap_or(strip.center());
    let scale = ctx.pixels_per_point();
    let automated = app.automation_input || automated(ctx);
    ctx.data_mut(|d| {
        d.insert_temp(drag_key(), TabDrag { document: doc.doc.id, window, strip, pointer, released: false, scale, start, window_origin, automated })
    });
}

#[derive(Clone)]
struct DropStrip {
    rect: Rect,
    tabs: Vec<(DocId, Rect)>,
    scale: f32,
}
fn strip_key() -> Id {
    Id::new("document-main-drop-strip")
}
pub(crate) fn clear_drop_strip(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<DropStrip>(strip_key()));
}

pub(crate) fn drop_strip(app: &PrismApp, ctx: &Context, rect: Rect, tabs: &[(usize, Rect)]) {
    let Some(rect) = desktop_rect(ctx, rect) else { return };
    let tabs = tabs.iter().filter_map(|(i, r)| Some((app.session.documents().get(*i)?.doc.id, desktop_rect(ctx, *r)?))).collect();
    let scale = ctx.pixels_per_point();
    ctx.data_mut(|d| d.insert_temp(strip_key(), DropStrip { rect, tabs, scale }));
}

/// The same target predicate drives feedback and the final drop, including an empty strip.
fn can_attach(drag: &TabDrag, target: &DropStrip) -> bool {
    drag.window.is_some() && target.rect.expand(8.0 * target.scale).contains(drag.pointer)
}
fn detaching(drag: &TabDrag) -> bool {
    drag.window.is_none() && !drag.strip.expand(24.0 * drag.scale).contains(drag.pointer)
}
fn cancel_drag(ctx: &Context) {
    let Some(drag) = ctx.data_mut(|d| {
        let drag = d.get_temp::<TabDrag>(drag_key());
        d.remove::<TabDrag>(drag_key());
        drag
    }) else {
        return;
    };
    if let (Some(window), Some(origin)) = (drag.window, drag.window_origin) {
        ctx.send_viewport_cmd_to(viewport(Some(window)), egui::ViewportCommand::OuterPosition(origin / drag.scale));
    }
    ctx.request_repaint();
}

pub(crate) fn drag_inspect(ctx: &Context) -> Value {
    let Some(drag) = ctx.data(|d| d.get_temp::<TabDrag>(drag_key())) else { return Value::Null };
    let target = ctx.data(|d| d.get_temp::<DropStrip>(strip_key()));
    json!({"document": drag.document, "window": drag.window,
        "phase": if target.as_ref().is_some_and(|t| can_attach(&drag, t)) { "docking" } else if detaching(&drag) { "detaching" } else { "moving" }})
}

pub(crate) fn dragging(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<TabDrag>(drag_key())).is_some()
}

pub(crate) fn hides_canvas(ctx: &Context, document: DocId) -> bool {
    ctx.viewport_id() == ViewportId::ROOT && ctx.data(|d| d.get_temp::<TabDrag>(drag_key())).is_some_and(|d| d.document == document && detaching(&d))
}

/// Moving the native viewport directly keeps pointer capture in egui. OS StartDrag enters a
/// platform move loop that can swallow the move/up events needed for docking and Escape.
///
/// `screen` is the OS pointer on the desktop (physical pixels) when the platform can read it.
/// It wins over the pointer events: those are relative to the window as it was when they were
/// sent, but placed with its position now, so while the window moves under the pointer each
/// lagging event read as the pointer jumping back and the window shook.
pub(crate) fn move_window(ctx: &Context, window: u64, screen: Option<Pos2>) {
    if let Some(mut drag) = ctx.data(|d| d.get_temp::<TabDrag>(drag_key()))
        && drag.window == Some(window)
        && !drag.released
        && let Some(origin) = drag.window_origin
    {
        if let Some(screen) = screen.filter(|p| !drag.automated && p.x.is_finite() && p.y.is_finite()) {
            drag.pointer = screen;
            ctx.data_mut(|d| d.insert_temp(drag_key(), drag.clone()));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition((origin + (drag.pointer - drag.start)) / drag.scale));
    }
}

/// Whether `window` is being dragged by its tab (read the OS pointer only then).
pub(crate) fn moving(ctx: &Context, window: u64) -> bool {
    ctx.data(|d| d.get_temp::<TabDrag>(drag_key())).is_some_and(|d| d.window == Some(window) && !d.released && !d.automated)
}

/// Fade the entire native window, never overlay black on the document. Called every source
/// frame so leaving the target, cancelling, or releasing restores the OS titlebar as well.
pub(crate) fn fade_window(app: &mut PrismApp, ctx: &Context, window: u64) {
    let drag = ctx.data(|d| d.get_temp::<TabDrag>(drag_key()));
    let target = ctx.data(|d| d.get_temp::<DropStrip>(strip_key()));
    let docking = matches!((drag, target), (Some(drag), Some(target)) if drag.window == Some(window) && can_attach(&drag, &target));
    let opacity = if docking { 0.55 } else { 1.0 };
    let key = Id::new(("document-native-opacity", window));
    let previous = ctx.data(|d| d.get_temp::<f32>(key)).unwrap_or(1.0);
    if previous != opacity
        && let Some(set) = app.services.window_opacity.as_mut()
        && set(ctx, opacity)
    {
        ctx.data_mut(|d| d.insert_temp(key, opacity));
    }
}

pub(crate) fn drag_indicator(app: &PrismApp, ctx: &Context, workspace: Rect) {
    let Some(drag) = ctx.data(|d| d.get_temp::<TabDrag>(drag_key())) else { return };
    let target = ctx.data(|d| d.get_temp::<DropStrip>(strip_key()));
    let t = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("document-docking-preview"))).with_clip_rect(workspace);
    let inner = ctx.input(|i| i.viewport().inner_rect).unwrap_or(Rect::ZERO);
    let local = |p: Pos2| p / ctx.pixels_per_point() - inner.min.to_vec2();
    if let Some(target) = target
        && can_attach(&drag, &target)
    {
        painter.rect_stroke(workspace, 0.0, egui::Stroke::new(2.0, t.docking_outline), egui::StrokeKind::Inside);
        let x = target.tabs.iter().find(|(_, r)| drag.pointer.x < r.center().x).map_or(target.rect.right(), |(_, r)| r.left());
        painter.line_segment([local(egui::pos2(x, target.rect.top())), local(egui::pos2(x, target.rect.bottom()))], egui::Stroke::new(2.0, t.docking_outline));
    } else if detaching(&drag) {
        // A lightweight empty document frame follows the tab. No image composition or camera
        // updates happen until release creates the native viewport.
        let delta = (drag.pointer - drag.start) / ctx.pixels_per_point();
        let ghost = workspace.translate(delta);
        painter.rect_filled(ghost, 0.0, t.docking_placeholder);
        let header = Rect::from_min_size(ghost.min, egui::vec2(ghost.width(), 26.0));
        painter.rect_filled(header, 0.0, t.chrome);
        painter.rect_stroke(ghost, 0.0, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
        if let Some((index, st)) = app.session.documents().iter().enumerate().find(|(_, d)| d.doc.id == drag.document) {
            let zoom = app.ui.views.get(index).map_or(1.0, |v| v.zoom);
            painter.with_clip_rect(header.intersect(workspace)).text(
                header.left_center() + egui::vec2(12.0, 0.0),
                egui::Align2::LEFT_CENTER,
                crate::canvas::document_title(app, index, zoom),
                egui::FontId::proportional(11.5),
                t.text,
            );
            let footer = Rect::from_min_max(ghost.left_bottom() - egui::vec2(0.0, 24.0), ghost.right_bottom());
            painter.rect_filled(footer, 0.0, t.chrome);
            painter.with_clip_rect(footer.intersect(workspace)).text(
                footer.left_center() + egui::vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{}%     {} × {} px ({:.0} ppi)", crate::canvas::fmt_zoom(zoom * 100.0), st.doc.size.width, st.doc.size.height, st.doc.resolution_dpi),
                egui::FontId::proportional(11.5),
                t.text,
            );
        }
    }
}

pub(crate) fn finish_drag(app: &mut PrismApp, ctx: &Context) {
    let Some(drag) = ctx.data(|d| d.get_temp::<TabDrag>(drag_key())) else { return };
    if !app.ui.dialogs.is_empty()
        || app.discard.is_some()
        || app.file_dialog_open()
        || !app.session.documents().iter().any(|d| d.doc.id == drag.document)
        || drag.window.is_some_and(|id| !app.ui.windows.iter().any(|w| w.id == id && w.open))
    {
        cancel_drag(ctx);
        return;
    }
    if !drag.released {
        ctx.request_repaint();
        return;
    }
    ctx.data_mut(|d| d.remove::<TabDrag>(drag_key()));
    let result = if let Some(window) = drag.window {
        let target = ctx.data(|d| d.get_temp::<DropStrip>(strip_key()));
        if let Some(target) = target.filter(|t| can_attach(&drag, t)) {
            let before = target.tabs.iter().find(|(_, r)| drag.pointer.x < r.center().x).map(|(id, _)| *id);
            attach(app, ctx, window, before)
        } else {
            Ok(Value::Null)
        }
    } else if detaching(&drag) {
        let document = app.session.documents().iter().position(|d| d.doc.id == drag.document);
        match document {
            Some(i) => detach(
                app,
                ctx,
                i,
                Some([
                    ((drag.strip.left() + drag.pointer.x - drag.start.x) / drag.scale).max(-100_000.0),
                    ((drag.strip.top() + drag.pointer.y - drag.start.y) / drag.scale).max(-100_000.0),
                ]),
            ),
            None => Ok(Value::Null),
        }
    } else {
        Ok(Value::Null)
    };
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

pub(crate) fn dialogs_here(app: &PrismApp, ctx: &Context) -> bool {
    let host = app.ui.dialogs.last().map_or(app.ui.active_window, |d| d.window);
    !crate::native_dialogs::is_open(app) && viewport(host) == ctx.viewport_id()
}

#[cfg(test)]
mod tests;
