//! Programmatic control of the running app, for agents, tests and (soon) MCP.
//!
//! Transport-agnostic: a transport thread (TCP in the desktop app, a channel in tests) sends
//! [`ControlRequest`]s; the UI thread handles them between frames and replies with JSON.
//!
//! Methods:
//! - `engine.execute {command, params}`: run any engine or UI command by id
//! - `engine.commands`: list commands with enablement
//! - `ui.inspect`: full UI state (tool, panels, views, dialogs, windows, window size); the menu
//!   tree is `ui.menu.list`
//! - `ui.set {tool?, panels?, dock?, dockTabs?, dockWidth?, colorPanel?, maskTarget?, vectorMaskTarget?, selectionMode?, zoom?, center?, rotation?, fit?, theme?, brushSection?, brushTab?, brushesView?, brushPicker?, brushPickerView?, brushSize?}`:
//!   change UI state; any other field is an error ([`UI_SET_FIELDS`])
//! - `ui.dialog.open {kind, fields?}` (kinds: newDocument, about, layerStyle {effect?}, colorPicker {target: foreground|background|shapeFill}, command {command}) / `ui.dialog.set {dialog, field, value}` / `ui.dialog.confirm {dialog, wait?}` / `ui.dialog.cancel {dialog}`
//! - `ui.dialog.apply {dialog}`: commit Preferences changes without closing the dialog
//! - `ui.dock {drop?: {payload, target}, toggle?, show?, close?, iconic?}`: panel docking (tear
//!   tabs off, merge them into groups, stack groups, dock columns left/right/bottom, floating
//!   windows, collapse to icons); replies with the layout and the rects of groups, tab strips and
//!   icon rows (see `dock_float::control`)
//! - `ui.window.open {document?}`: additional view; `ui.window.detach {document?}`: float a tab
//! - `ui.window.attach {window}` / `ui.window.focus {window}` / `ui.window.close {window}`
//! - Input, resize and screenshot methods accept optional `window`; omission targets the main window
//! - `ui.drag {from:[x,y],to:[x,y],window?}`: synthetic primary-button screen-point drag
//! - `ui.pointer {events: [{kind: down|move|up|cancel, x?, y?, pressure?, tiltX?, tiltY?, rotation?, time?}], modifiers?, button?}`: drive the active tool in document coordinates (`button: "secondary"` opens the tool's canvas context menu or Brush Preset picker, or erases with Preferences › Tools › Right-click with painting tools = erase)
//! - `ui.penTrace {enabled?, clear?}`: opt-in bounded pen packet diagnostics and replay data.
//! - `ui.click {x, y, button?, count?}` / `ui.move {x, y}`: synthetic pointer input in screen points
//!   (`count` at most [`MAX_CLICKS`])
//! - `ui.key {key, command?, shift?, alt?, ctrl?}` / `ui.type {text}`: synthetic keyboard input
//! - `ui.resize {width, height}`: resize the main window
//! - `ui.gpu.simulateLoss {error?}`: act as if the wgpu device was lost (or, with `error: true`,
//!   reported an error): the app switches to the CPU renderer for the rest of the session, as on a
//!   real loss. For testing the fallback; returns whether a GPU canvas was active
//! - `ui.screenshot {path?, focus?}`: capture the main window (PNG). Raises the window first (default)
//!   because occluded macOS windows stop rendering
//! - `ui.focus`: bring the main window to the front
//! - `app.open {path}` / `app.save {path}`: relative file I/O under the automation roots; reply with `warnings`
//! - `app.quit`
//! - `jobs.list` / `jobs.cancel {job?}`: background jobs (#210) with progress; cancel one (or all).
//!   `engine.execute`, `ui.menu.invoke` and `ui.dialog.confirm` wait for a command that runs as a
//!   job unless `wait: false` (then the reply is `{job, pending: true}`)

use std::sync::mpsc::Sender;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::PrismApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool, UiState};

pub type ControlResponse = Value;

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<ControlResponse>,
    /// Requests still queued at this instant are rejected; already dispatched work continues.
    pub deadline: Option<std::time::Instant>,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<ControlResponse>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx, deadline: None }, rx)
    }
}

pub enum Outcome {
    Done(Value),
    Screenshot {
        token: u64,
        path: Option<String>,
        window: Option<u64>,
        dialog: Option<u64>,
    },
    /// Synthetic input queued: reply once the app has processed all of it (so a following
    /// `ui.inspect`/`engine.execute` observes the effect).
    AfterInput,
    /// A command started a background job and the caller waits for it: reply with its result
    /// once it has been applied (or with its error / "cancelled").
    AfterJob(prism_engine::jobs::JobId),
}

/// The fields `ui.set` reads. Anything else is rejected before a field is applied, and every
/// field's value is validated before the first one is applied, so a typo, an unknown field, a
/// bad value or a bad nested key can't reply with success while nothing — or only half of it —
/// changed (#412).
pub const UI_SET_FIELDS: [&str; 35] = [
    "tool",
    "actions",
    "panels",
    "dock",
    "dockTabs",
    "dockWidth",
    "colorPanel",
    "toolSliders",
    "tools",
    "maskTarget",
    "vectorMaskTarget",
    "selectionMode",
    "zoom",
    "center",
    "rotation",
    "fit",
    "theme",
    "brushSection",
    "brushTab",
    "sutDetails",
    "cspLibraryOpen",
    "cspLibraryView",
    "cspLibraryFilter",
    "cspLibraryFolder",
    "cspLibraryTags",
    "sutTextureScaleExpanded",
    "brushesView",
    "brushPicker",
    "brushPickerView",
    "brushSize",
    "gradientBlendMode",
    "gradientClassic",
    "typeFontFilter",
    "arrow",
    "lineRoundTips",
];

/// Most clicks one `ui.click` may queue (#982). Each click is a press and a release that the app
/// feeds in its own frame before replying, so `count` sizes both the input queue and the wait;
/// the same ceiling as the steps of one batch request.
pub const MAX_CLICKS: u64 = 256;

#[cfg(test)]
mod action_view_tests {
    use super::*;
    #[test]
    fn action_view_state_is_drivable_and_rejects_bad_fields_atomically() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let (request, _) = ControlRequest::new("ui.set", json!({"actions":{"selected":0,"expanded":[true],"collapsed_sets":["Other"]}}));
        let _ = handle(&mut app, &ctx, &request);
        assert_eq!(app.ui.actions.selected, Some(0));
        assert_eq!(app.ui.actions.expanded, [true]);
        let old = app.ui.actions.clone();
        let (request, _) = ControlRequest::new("ui.set", json!({"actions":{"selected":1,"unexpected":true}}));
        let _ = handle(&mut app, &ctx, &request);
        assert_eq!(app.ui.actions, old);
    }
}

fn ok(v: Value) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}
fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}

/// A `ui.set` boolean field: `Ok(None)` when absent, an error when present with another type.
fn bool_field(p: &Value, key: &str) -> std::result::Result<Option<bool>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("{key} must be a boolean")),
    }
}

/// A `ui.set` integer field: `Ok(None)` when absent; whole-number floats (`12.0`, as UIs send
/// them) are accepted, anything else is an error.
fn uint_field(p: &Value, key: &str) -> std::result::Result<Option<u64>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) if v.is_u64() => Ok(v.as_u64()),
        Some(v) => v.as_f64().filter(|f| f.fract() == 0.0 && *f >= 0.0).map(|f| Some(f as u64)).ok_or_else(|| format!("{key} must be a non-negative integer")),
    }
}

/// A `ui.set` numeric field: `Ok(None)` when absent, finite numbers only.
fn num_field(p: &Value, key: &str) -> std::result::Result<Option<f64>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| format!("{key} must be a finite number")),
    }
}

/// A `ui.set` object field merged into the current state (e.g. `panels`): the patch's keys are
/// merged into the serialized current value and parsed back, so a non-object value or an
/// unknown nested key is an error — a nested typo must not reply ok while changing nothing.
fn merged_object<T: serde::Serialize + serde::de::DeserializeOwned>(current: &T, patch: Option<&Value>, field: &str) -> std::result::Result<Option<T>, String> {
    let Some(patch) = patch else { return Ok(None) };
    let Some(patch) = patch.as_object() else { return Err(format!("{field} must be an object")) };
    let mut cur = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let known: std::collections::HashSet<String> = cur.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    for (k, v) in patch {
        if !known.contains(k) {
            return Err(format!("unknown {field} field `{k}`"));
        }
        if let Some(c) = cur.as_object_mut() {
            c.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(cur).map(Some).map_err(|e| format!("{field}: {e}"))
}

/// A `ui.set` object field applied by whole-value replacement (e.g. `dock`, `colorPanel`):
/// unknown nested keys are rejected like [`merged_object`] does, but the value replaces the
/// state instead of merging into it.
fn whole_object<T: serde::Serialize + serde::de::DeserializeOwned>(current: &T, v: Option<&Value>, field: &str) -> std::result::Result<Option<T>, String> {
    let Some(v) = v else { return Ok(None) };
    let Some(patch) = v.as_object() else { return Err(format!("{field} must be an object")) };
    let cur = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let known: std::collections::HashSet<&str> = cur.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    for k in patch.keys() {
        if !known.contains(k.as_str()) {
            return Err(format!("unknown {field} field `{k}`"));
        }
    }
    serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("{field}: {e}"))
}
fn wrap(r: Result<Value, String>) -> Outcome {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

/// Run a command the way automation does (script events off). Long commands may run as
/// background jobs: by default the reply waits for the job's result (backward compatible); with
/// `wait` false it is `{job, pending: true}` at once.
fn run_waiting(app: &mut PrismApp, wait: bool, run: impl FnOnce(&mut PrismApp) -> Result<Value, String>) -> Outcome {
    let events_enabled = app.session.prefs().script_events.enabled;
    if events_enabled {
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
    }
    app.jobs.last_started = None;
    let result = run(app);
    if events_enabled {
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
    }
    match (result, app.jobs.last_started.take()) {
        (Ok(_), Some(job)) if wait => Outcome::AfterJob(job),
        (result, _) => wrap(result),
    }
}

/// Screen position of document point (x, y) on the main canvas, where a `ui.pointer` right-click
/// opens its menu (the mouse's opens at the pointer); the canvas centre when it can't be mapped.
fn screen_point(app: &PrismApp, x: f64, y: f64) -> [f32; 2] {
    let p = crate::canvas::ViewXform::active(app)
        .map(|xf| xf.to_screen(x as f32, y as f32))
        .filter(|p| p.is_finite())
        .unwrap_or_else(|| app.last_canvas_rect.center());
    [p.x, p.y]
}

pub fn handle(app: &mut PrismApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let saved = app.session.authorize;
    if let Some(gate) = app.services.automation_authorize {
        app.session.authorize = Some(gate);
    }
    let outcome = dispatch(app, ctx, req);
    app.session.authorize = saved;
    outcome
}

fn dispatch(app: &mut PrismApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let u = |k: &str| p.get(k).and_then(Value::as_u64);
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
    let window = match p.get("window") {
        Some(value)
            if matches!(req.method.as_str(), "ui.click" | "ui.move" | "ui.drag" | "ui.key" | "ui.type" | "ui.pointer" | "ui.screenshot" | "ui.resize") =>
        {
            let Some(id) = value.as_u64() else { return err("invalid window id") };
            if !app.ui.windows.iter().any(|w| w.open && w.id == id) {
                return err(format!("no window {id}"));
            }
            Some(id)
        }
        _ => None,
    };
    let dialog = if matches!(req.method.as_str(), "ui.click" | "ui.move" | "ui.drag" | "ui.key" | "ui.type" | "ui.pointer" | "ui.screenshot" | "ui.resize") {
        if p.get("window").is_some() && p.get("dialog").is_some() {
            return err("specify either window or dialog");
        }
        match p.get("dialog") {
            Some(v) => {
                let Some(id) = v.as_u64() else { return err("invalid dialog id") };
                if !app.ui.dialogs.iter().any(|d| d.id == id && crate::native_dialogs::own_window(d)) {
                    return err("no native dialog");
                }
                Some(id)
            }
            None => None,
        }
    } else {
        None
    };
    let viewport = dialog.map(crate::native_dialogs::viewport).unwrap_or_else(|| crate::doc_windows::viewport(window));
    if matches!(req.method.as_str(), "ui.click" | "ui.move" | "ui.drag" | "ui.key" | "ui.type" | "ui.pointer") {
        if let Some(dialog) = dialog {
            let _ = crate::native_dialogs::focus(app, ctx, dialog);
        } else if let Some(window) = window {
            if let Err(e) = crate::doc_windows::focus(app, ctx, window) {
                return err(e);
            }
        } else if !crate::native_dialogs::is_open(app)
            && let Some(document) = crate::doc_windows::main_index(app)
        {
            crate::doc_windows::activate_main(app, document);
        }
    }
    match req.method.as_str() {
        "ui.context.choose" => {
            let Some(id) = s("id") else { return err("missing `id`") };
            if app.ui.document_context_menu.is_some() {
                if let Some(authorize) = app.services.automation_command.as_ref()
                    && let Err(e) = authorize(id, &json!({}))
                {
                    return err(e);
                }
                return wrap(crate::document_menu::choose(app, ctx, id));
            }
            let Some(menu) = app.ui.canvas_tool_menu.as_ref() else { return err("no canvas context menu is open") };
            if !crate::canvas_tool_menu::available(app, menu, id) {
                return err("context action is unavailable");
            }
            if let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(id, &json!({}))
            {
                return err(error);
            }
            crate::canvas_tool_menu::choose(app, ctx, id);
            ok(json!({"command": id, "dialog": app.ui.dialogs.last().map(|d| d.id)}))
        }
        "engine.execute" | "ui.menu.invoke" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            // A control-channel menu click must respect the same modal gate as the native menu.
            // engine.execute is deliberately not subject to the UI's menu-click semantics.
            if req.method == "ui.menu.invoke" && !crate::menus::modal_allows(app, id) {
                return err("menu command is unavailable while a modal dialog is open");
            }
            let params = p.get("params").cloned().unwrap_or(json!({}));
            if let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(id, &params)
            {
                return err(error);
            }
            // `engine.execute` is programmatic: engine commands run directly with their default
            // params and never open a dialog (an agent would otherwise get a modal instead of a
            // result). `ui.menu.invoke` behaves like a menu click, so it may open the dialog.
            if req.method == "engine.execute" && prism_engine::commands::find(id).is_some() {
                return run_waiting(app, wait, |app| app.run(id, params));
            }
            run_waiting(app, wait, |app| crate::menus::invoke(app, ctx, id, params))
        }
        "engine.commands" => wrap(app.run("command.list", json!({}))),
        // Background jobs (#210): running ones with progress, then the last few that ended.
        "jobs.list" => wrap(app.session.execute("jobs.list", json!({})).map_err(|e| e.to_string())),
        "jobs.cancel" => {
            let all = p.get("job").is_none_or(Value::is_null);
            match p.get("job").and_then(Value::as_u64) {
                Some(j) if app.session.job(prism_engine::jobs::JobId(j)).is_some() => {
                    crate::jobs_ui::cancel(app, prism_engine::jobs::JobId(j));
                    ok(json!({"cancelled": 1}))
                }
                Some(j) => err(format!("no running job {j}")),
                None if all => {
                    let ids: Vec<_> = app.session.jobs().into_iter().map(|j| j.id).collect();
                    for j in &ids {
                        crate::jobs_ui::cancel(app, *j);
                    }
                    ok(json!({"cancelled": ids.len()}))
                }
                None => err("`job` must be a job id"),
            }
        }
        "ui.menu.list" => ok(serde_json::to_value(crate::menus::menu_items(app)).unwrap_or_default()),
        // Panel docking: read the layout and rects, drop tabs/groups/windows, toggle panels.
        "ui.dock" => wrap(crate::dock_float::control(app, ctx, p)),
        "ui.inspect" => ok(inspect(app, ctx)),
        // Clip Studio workspace: pick a CSP tool (`tool`) and/or one of its sub-tools (`subTool`,
        // a name from the reply's `subTools`); with no fields, report the current ones.
        "ui.cspTool" => {
            if let Some(name) = p.get("tool") {
                let Some(name) = name.as_str() else { return err("`tool` must be a Clip Studio tool name") };
                if let Err(e) = crate::csp_tools::set_from_name(app, name) {
                    return err(e);
                }
            }
            if let Some(name) = p.get("subTool") {
                let Some(name) = name.as_str() else { return err("`subTool` must be a sub-tool name") };
                let tool = app.ui.csp_tool;
                let Some(sub) = crate::sub_tool_panel::sub_tools(app, tool).into_iter().find(|s| s.name == name) else {
                    return err(format!("no sub-tool `{name}` under {}", tool.label()));
                };
                if !sub.usable {
                    return err(format!("sub-tool `{name}` is not supported in Prism yet"));
                }
                crate::sub_tool_panel::apply(app, tool, &sub);
            }
            let subs: Vec<Value> =
                crate::sub_tool_panel::sub_tools(app, app.ui.csp_tool).into_iter().map(|s| json!({"name": s.name, "usable": s.usable})).collect();
            ok(json!({
                "active": crate::csp_tools::active(app),
                "tool": app.ui.csp_tool,
                "subTool": crate::sub_tool_panel::current(app).map(|s| s.name),
                "subTools": subs,
                "prismTool": format!("{:?}", app.ui.tool),
            }))
        }
        "ui.set" => {
            if let Some(field) = p.as_object().and_then(|o| o.keys().find(|k| !UI_SET_FIELDS.contains(&k.as_str()))) {
                return err(format!("unknown field `{field}` (fields: {})", UI_SET_FIELDS.join(", ")));
            }
            let gradient_blend = if let Some(value) = p.get("gradientBlendMode") {
                let Some(name) = value.as_str() else { return err("gradientBlendMode must be a blend mode name") };
                let Some(mode) = prism_engine::commands::blend_from_str(name).filter(|m| prism_color::BlendMode::LAYER_MODES.contains(m)) else {
                    return err(format!("unknown gradient blend mode `{name}`"));
                };
                Some(mode)
            } else {
                None
            };
            // Everything is validated and materialized before the first field is applied, so a
            // rejected call applies none of its fields (#412's guarantee, down to the values
            // and nested keys).
            let applied = (|| -> std::result::Result<Value, String> {
                let tool = match s("tool") {
                    Some(t) => Tool::from_name(t).map(Some).ok_or_else(|| format!("unknown tool `{t}`"))?,
                    None => None,
                };
                let font_filter: Option<crate::state::TypeFontFilter> =
                    p.get("typeFontFilter").map(|v| serde_json::from_value(v.clone()).map_err(|e| format!("typeFontFilter: {e}"))).transpose()?;
                let arrow: Option<crate::state::ArrowOptions> = merged_object(&app.ui.tool_options.arrow, p.get("arrow"), "arrow")?;
                if arrow.as_ref().is_some_and(|o| !o.valid()) {
                    return Err("arrow width must be 0.1–1000 px, headSize 0.1–10000 px and headAngle 10–160 degrees".into());
                }
                let line_round_tips = bool_field(p, "lineRoundTips")?;
                let gradient_classic = bool_field(p, "gradientClassic")?;
                let panels = merged_object(&app.ui.panels, p.get("panels"), "panels")?;
                let actions: Option<crate::actions::ActionsUi> = merged_object(&app.ui.actions, p.get("actions"), "actions")?;
                if actions.as_ref().is_some_and(|a| a.expanded.len() > 4096 || a.collapsed_sets.len() > 4096) {
                    return Err("actions view is limited to 4096 rows".into());
                }
                let mask_target = bool_field(p, "maskTarget")?;
                let vector_mask_target = bool_field(p, "vectorMaskTarget")?;
                let selection_mode = uint_field(p, "selectionMode")?;
                let dock_tabs = merged_object(&app.ui.dock_tabs, p.get("dockTabs"), "dockTabs")?;
                let dock = whole_object(&app.ui.dock, p.get("dock"), "dock")?;
                // Which chip the Color panel edits.
                let color_panel = whole_object(&app.ui.color_panel, p.get("colorPanel"), "colorPanel")?;
                // Tool Sliders placement and sizes (see `tool_sliders::ToolSlidersLayout`).
                let tool_sliders = merged_object(&app.ui.tool_sliders, p.get("toolSliders"), "toolSliders")?;
                // Where the Tools panel sits (see `tools_window::ToolsLayout`).
                let tools = merged_object(&app.ui.tools, p.get("tools"), "tools")?;
                let dock_width = num_field(p, "dockWidth")?;
                let zoom = num_field(p, "zoom")?;
                let rotation = num_field(p, "rotation")?;
                if rotation.is_some() && app.session.active_index().is_none() {
                    return Err("no document open".into());
                }
                let center = match p.get("center") {
                    Some(v) => {
                        let a = v.as_array().ok_or_else(|| "center must be [x, y]".to_string())?;
                        if a.len() != 2 {
                            return Err("center must be exactly [x, y]".into());
                        }
                        let (Some(x), Some(y)) = (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)) else {
                            return Err("center must be [x, y] of two numbers".into());
                        };
                        Some([x as f32, y as f32])
                    }
                    None => None,
                };
                let fit = bool_field(p, "fit")? == Some(true);
                let theme = match s("theme") {
                    Some(name) => {
                        let kind = crate::theme::ThemeKind::from_name(name).ok_or_else(|| {
                            let names: Vec<_> = crate::theme::ThemeKind::ALL.iter().map(|k| k.id()).collect();
                            format!("unknown theme `{name}` ({})", names.join(", "))
                        })?;
                        Some(kind)
                    }
                    None => None,
                };
                let brush_section = uint_field(p, "brushSection")?;
                let brush_tab = uint_field(p, "brushTab")?;
                let sut_details = bool_field(p, "sutDetails")?;
                let csp_library_open = bool_field(p, "cspLibraryOpen")?;
                let csp_library_view: Option<crate::csp_library_ui::View> = match p.get("cspLibraryView") {
                    Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("cspLibraryView: {e} (list, grid)"))?,
                    None => None,
                };
                let csp_library_filter: Option<crate::csp_library_ui::KindFilter> = match p.get("cspLibraryFilter") {
                    Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("cspLibraryFilter: {e} (all, brushes, subTools, layers)"))?,
                    None => None,
                };
                // A CLIP STUDIO folder path ("ダウンロード:Pack-1758445"), "favorite", or null for all.
                let csp_library_folder: Option<Option<String>> = match p.get("cspLibraryFolder") {
                    Some(Value::Null) => Some(None),
                    Some(Value::String(f)) if f == "favorite" => Some(Some(crate::csp_library_ui::tree::FAVORITE.to_string())),
                    Some(Value::String(f)) if !f.trim().is_empty() && f.chars().count() <= 1024 => Some(Some(f.clone())),
                    Some(_) => return Err("cspLibraryFolder: a CLIP STUDIO folder path, \"favorite\" or null".into()),
                    None => None,
                };
                let csp_library_tags: Option<Vec<String>> = match p.get("cspLibraryTags") {
                    Some(v) => Some(
                        serde_json::from_value::<Vec<String>>(v.clone())
                            .ok()
                            .filter(|tags| tags.len() <= 64 && tags.iter().all(|t| !t.trim().is_empty() && t.chars().count() <= 64))
                            .ok_or("cspLibraryTags: an array of at most 64 non-empty tag strings")?,
                    ),
                    None => None,
                };
                let sut_texture_scale_expanded = bool_field(p, "sutTextureScaleExpanded")?;
                let brushes_view = match p.get("brushesView") {
                    Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("brushesView: {e} (list, grid)"))?,
                    None => None,
                };
                let brush_picker_view = match p.get("brushPickerView") {
                    Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("brushPickerView: {e} (list, grid)"))?,
                    None => None,
                };
                // `Some(None)` is an explicit null, which closes the picker (a missing field
                // leaves it alone).
                let brush_picker = match p.get("brushPicker") {
                    Some(v) => {
                        let at = serde_json::from_value::<Option<[f32; 2]>>(v.clone())
                            .map_err(|_| "brushPicker must be [x, y] in screen points, or null to close it".to_string())?;
                        if at.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite()) {
                            return Err("brushPicker must be [x, y] in screen points, or null to close it".into());
                        }
                        Some(at)
                    }
                    None => None,
                };
                // brushSize rides the journaled `tools.setBrush` command (#744). Numeric values
                // are validated here so the command cannot fail after other fields were
                // applied; non-numeric values keep the API's historical silent no-op.
                let brush_size: Option<f64> = match p.get("brushSize").and_then(Value::as_f64) {
                    Some(size) => {
                        if !size.is_finite() || size > f64::from(prism_paint::MAX_BRUSH_SIZE) {
                            return Err(format!("brushSize must be finite and at most {} px", prism_paint::MAX_BRUSH_SIZE));
                        }
                        Some(size)
                    }
                    None => None,
                };

                // Apply (nothing below can fail).
                if let Some(t) = tool {
                    app.ui.tool = t;
                    // Each tool keeps its own brush (#218), so switch it in before `brushSize`
                    // below sets the new tool's size.
                    crate::paint_mouse::sync_tool_brush(app);
                }
                if let Some(rounded) = line_round_tips {
                    app.ui.tool_options.line_round_tips = rounded;
                }
                if let Some(arrow) = arrow {
                    app.ui.tool_options.arrow = arrow;
                }
                if let Some(filter) = font_filter {
                    app.ui.tool_options.type_font_filter = filter;
                }
                let gradient_before = app.ui.tool_options.clone();
                if let Some(mode) = gradient_blend {
                    app.ui.tool_options.gradient_blend_mode = mode;
                }
                if let Some(classic) = gradient_classic {
                    app.ui.tool_options.gradient_classic = classic;
                }
                if gradient_blend.is_some() {
                    crate::gradient_ui::options_changed(app, &gradient_before);
                }
                if let Some(v) = panels {
                    app.ui.panels = v;
                }
                if let Some(v) = actions {
                    app.ui.actions = v;
                }
                if let Some(m) = mask_target {
                    app.ui.mask_target = m;
                    app.ui.vector_mask_target &= !m;
                }
                if let Some(m) = vector_mask_target {
                    app.ui.vector_mask_target = m;
                    app.ui.mask_target &= !m;
                }
                // Selection tools' options-bar mode: 0 New, 1 Add, 2 Subtract, 3 Intersect.
                if let Some(m) = selection_mode {
                    app.ui.selection_mode = m.min(3) as u8;
                }
                if let Some(v) = dock_tabs {
                    app.ui.dock_tabs = v;
                }
                // Dock group order, heights and collapsed groups (see `dock::DockLayout`).
                if let Some(v) = dock {
                    app.ui.dock = v;
                }
                if let Some(v) = color_panel {
                    app.ui.color_panel = v;
                }
                if let Some(v) = tool_sliders {
                    app.ui.tool_sliders = v;
                }
                if let Some(v) = tools {
                    app.ui.tools = v;
                }
                // Right dock width in points (clamped to the dock's 250..=520 range), applied next frame.
                if let Some(w) = dock_width {
                    crate::panels::request_dock_width(ctx, w as f32);
                }
                if let Some(i) = app.session.active_index() {
                    if let Some(z) = zoom {
                        app.update_view(i, |v| v.zoom = (z as f32).clamp(0.01, 64.0));
                        app.update_view(i, |v| v.fit_pending = false);
                        app.update_view(i, |v| v.fill_pending = false);
                    }
                    if let Some(c) = center {
                        app.update_view(i, |v| v.center = c);
                        app.update_view(i, |v| v.fit_pending = false);
                        app.update_view(i, |v| v.fill_pending = false);
                    }
                    if fit {
                        app.update_view(i, |v| v.fit_pending = true);
                        app.update_view(i, |v| v.fill_pending = false);
                    }
                    if let Some(r) = rotation {
                        app.update_view(i, |v| v.rotation = crate::rotate_view::wrap_deg(r as f32));
                    }
                }
                if let Some(k) = theme {
                    app.set_theme(ctx, k);
                }
                if let Some(i) = brush_section {
                    app.ui.brush_section = (i as usize).min(crate::brush_panel::SECTIONS.len() - 1);
                }
                if let Some(i) = brush_tab {
                    app.ui.brush_tab = (i as usize).min(1);
                }
                if let Some(value) = sut_details {
                    app.ui.sut_panel.details = value;
                }
                // Opening shows the CLIP STUDIO Materials panel and lists the downloads again.
                match csp_library_open {
                    Some(true) => crate::csp_library_ui::open(app),
                    Some(false) => crate::csp_library_ui::close(app),
                    None => {}
                }
                if let Some(v) = csp_library_view {
                    app.ui.csp_library.view = v;
                }
                if let Some(tags) = csp_library_tags {
                    app.ui.csp_library.tags = tags;
                }
                if let Some(f) = csp_library_filter {
                    app.ui.csp_library.filter = f;
                }
                if let Some(f) = csp_library_folder {
                    app.ui.csp_library.folder = f;
                }
                if let Some(value) = sut_texture_scale_expanded {
                    app.ui.sut_panel.texture_scale_expanded = value;
                }
                if let Some(v) = brushes_view {
                    app.ui.brushes_panel.view = v;
                }
                if let Some(v) = brush_picker_view {
                    app.ui.brush_picker_list.view = v;
                }
                if let Some(at) = brush_picker {
                    app.ui.brush_picker = at;
                }
                // Which chip the Color panel edits.
                if let Some(c) = color_panel {
                    app.ui.color_panel = c;
                }
                if let Some(size) = brush_size {
                    app.run("tools.setBrush", json!({"brush": {"size": size}})).map_err(|e| e.to_string())?;
                }
                Ok(Value::Null)
            })();
            wrap(applied)
        }
        "ui.dialog.open" => {
            let kind = match s("kind").unwrap_or("") {
                "newDocument" | "NewDocument" => DialogKind::NewDocument,
                "about" | "About" => DialogKind::About,
                "layerStyle" | "LayerStyle" => {
                    return match crate::layer_style::open(app, s("effect")) {
                        Some(id) => ok(json!({"dialog": id})),
                        None => err("no active layer"),
                    };
                }
                "colorPicker" | "ColorPicker" => {
                    if s("target") == Some("shapeFill") {
                        return wrap(crate::shape_color_ui::open(app).map(|dialog| json!({"dialog": dialog})));
                    }
                    let target = if s("target") == Some("background") { "background" } else { "foreground" };
                    return ok(json!({"dialog": crate::color_picker_ui::open(app, target)}));
                }
                "command" | "Command" => {
                    let Some(cmd) = s("command") else { return err("command dialogs need `command`") };
                    let label = prism_engine::commands::find(cmd).map(|c| c.label).unwrap_or(cmd);
                    return ok(json!({"dialog": crate::dialogs::open_command_dialog(app, cmd, label)}));
                }
                other => return err(format!("unknown dialog kind `{other}`")),
            };
            let mut fields = if kind == DialogKind::NewDocument { UiState::new_document_fields() } else { Default::default() };
            if let Some(f) = p.get("fields").and_then(Value::as_object) {
                fields.extend(f.clone());
            }
            ok(json!({"dialog": app.ui.open_dialog(kind, fields)}))
        }
        "ui.dialog.set" => {
            let (Some(id), Some(field)) = (u("dialog"), s("field")) else { return err("need `dialog` and `field`") };
            let value = p.get("value").cloned().unwrap_or(Value::Null);
            match app.ui.dialog_mut(id) {
                Some(d) => {
                    d.fields.insert(field.to_string(), value);
                    ok(Value::Null)
                }
                None => err(format!("no dialog {id}")),
            }
        }
        "ui.dialog.confirm" | "ui.dialog.apply" => match u("dialog") {
            Some(id) => {
                let command = app.ui.dialogs.iter().find(|dialog| dialog.id == id).and_then(|dialog| {
                    if req.method == "ui.dialog.apply" {
                        dialog.fields.get("values").map(|values| ("prefs.set".to_string(), json!({"path": "", "value": values})))
                    } else {
                        dialog.fields.get("__command").and_then(Value::as_str).map(|command| (command.to_string(), Value::Object(dialog.fields.clone())))
                    }
                });
                let command = if req.method == "ui.dialog.confirm"
                    && let Some(dialog) = app.ui.dialogs.iter().find(|dialog| dialog.id == id).filter(|d| crate::shape_color_ui::owns(&d.fields))
                {
                    match crate::shape_color_ui::params(app, &dialog.fields) {
                        Ok(params) => Some(("shape.edit".to_string(), params)),
                        Err(error) => return err(error),
                    }
                } else {
                    command
                };
                if let Some((command, params)) = command
                    && let Some(authorize) = app.services.automation_command.as_ref()
                    && let Err(error) = authorize(&command, &params)
                {
                    return err(error);
                }
                let apply = req.method == "ui.dialog.apply";
                run_waiting(app, wait, |app| if apply { crate::prefs_ui::apply(app, id) } else { crate::dialogs::confirm(app, id) })
            }
            None => err("missing `dialog`"),
        },
        "ui.dialog.focus" => match u("dialog") {
            Some(id) => wrap(crate::native_dialogs::focus(app, ctx, id)),
            None => err("missing dialog"),
        },
        "ui.dialog.cancel" => match u("dialog") {
            Some(id) => wrap(crate::native_dialogs::cancel(app, ctx, id)),
            None => err("missing dialog"),
        },
        "ui.window.open" => {
            if let Some(d) = u("document")
                && !app.session.set_active(d as usize)
            {
                return err(format!("no document {d}"));
            }
            wrap(crate::menus::invoke(app, ctx, "window.newWindowForDocument", json!({})))
        }
        "ui.window.detach" => {
            let document = match p.get("document") {
                Some(v) => match v.as_u64().and_then(|i| usize::try_from(i).ok()) {
                    Some(i) => i,
                    None => return err("invalid document"),
                },
                None => match app.session.active_index() {
                    Some(i) => i,
                    None => return err("no document"),
                },
            };
            wrap(crate::doc_windows::detach(app, ctx, document, None))
        }
        "ui.window.attach" | "ui.window.focus" | "ui.window.close" => {
            let Some(window) = u("window") else { return err("missing `window`") };
            wrap(match req.method.as_str() {
                "ui.window.attach" => crate::doc_windows::attach(app, ctx, window, None),
                "ui.window.focus" => crate::doc_windows::focus(app, ctx, window),
                _ => crate::doc_windows::close(app, ctx, window),
            })
        }
        "ui.penTrace" => {
            if let Some(value) = p.get("enabled") {
                let Some(enabled) = value.as_bool() else { return err("enabled must be a boolean") };
                app.stylus.trace_enabled = enabled;
            }
            if p.get("clear").and_then(Value::as_bool) == Some(true) {
                app.stylus.trace.clear();
            }
            ok(json!({"enabled": app.stylus.trace_enabled, "capacity": 4096, "events": app.stylus.trace}))
        }
        "ui.pointer" => {
            let Some(events) = p.get("events").and_then(Value::as_array) else { return err("missing `events`") };
            if events.len() > 4096 {
                return err("too many pointer events");
            }
            // Validate the whole batch before changing the tool or any preview.
            for event in events {
                if !event.is_object() || event.get("kind").is_some_and(|v| !v.is_string()) {
                    return err("pointer events must be objects with a string kind");
                }
                let kind = event.get("kind").and_then(Value::as_str).unwrap_or("move");
                if !matches!(kind, "down" | "move" | "up" | "cancel") {
                    return err("pointer kind must be down, move, up or cancel");
                }
                for key in ["x", "y", "pressure", "tiltX", "tiltY", "rotation", "time"] {
                    if let Some(value) = event.get(key) {
                        let Some(value) = value.as_f64().filter(|v| v.is_finite()) else { return err(format!("invalid pointer {key}")) };
                        if key == "pressure" && !(0.0..=1.0).contains(&value) || key == "time" && !(0.0..=1e12).contains(&value) {
                            return err(format!("pointer {key} out of range"));
                        }
                    }
                }
            }
            // Modifier flags may be top-level or grouped under "modifiers".
            let m = p.get("modifiers").unwrap_or(p);
            let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
            let mods = egui::Modifiers {
                shift: flag("shift"),
                alt: flag("alt"),
                command: flag("command"),
                mac_cmd: cfg!(target_os = "macos") && flag("command"),
                ctrl: flag("ctrl"),
            };
            if let Some(dialog) = dialog {
                if events.len() > 4096 {
                    return err("too many pointer events");
                }
                let button = match s("button") {
                    Some("right" | "secondary") => egui::PointerButton::Secondary,
                    Some("middle") => egui::PointerButton::Middle,
                    _ => egui::PointerButton::Primary,
                };
                let mut input = Vec::new();
                for event in events {
                    let point = event.get("x").and_then(Value::as_f64).zip(event.get("y").and_then(Value::as_f64));
                    let Some((x, y)) = point.filter(|(x, y)| x.is_finite() && y.is_finite() && x.abs() <= 1_000_000.0 && y.abs() <= 1_000_000.0) else {
                        return err("invalid dialog pointer coordinates");
                    };
                    let pos = egui::pos2(x as f32, y as f32);
                    input.push(egui::Event::PointerMoved(pos));
                    match event.get("kind").and_then(Value::as_str) {
                        Some("down" | "up") => input.push(egui::Event::PointerButton { pos, button, pressed: event["kind"] == "down", modifiers: mods }),
                        Some("move") => {}
                        _ => return err("invalid dialog pointer kind"),
                    }
                }
                app.synthetic.extend(input);
                app.synthetic_dialog = Some(dialog);
                crate::doc_windows::queue_input(app, ctx);
                app.synthetic_dialog = None;
                ctx.request_repaint();
                return Outcome::AfterInput;
            }
            if crate::native_dialogs::is_open(app) {
                return err("a native dialog owns editing input");
            }
            if let Some(t) = s("tool").and_then(Tool::from_name) {
                app.ui.tool = t;
            }
            // Space held: the Crop tool moves the frame being drawn, a marquee, lasso or shape
            // being drawn moves instead of growing (hold_keys.rs).
            let space = flag("space");
            crate::crop_ui::set_space(app, space);
            for e in events {
                if e.get("kind").and_then(Value::as_str) == Some("cancel") {
                    crate::pen_input::cancel(app);
                    continue;
                }
                let x = e.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                let y = e.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                let pr = e.get("pressure").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                app.stylus.time = e.get("time").and_then(Value::as_f64).unwrap_or(0.0);
                let ev = match e.get("kind").and_then(Value::as_str).unwrap_or("move") {
                    "down" => ToolEvent::Down { x, y, pressure: pr },
                    "up" => ToolEvent::Up { x, y },
                    _ => ToolEvent::Move { x, y, pressure: pr },
                };
                // With the Color Picker on top the image is its eyedropper, as for the mouse.
                if crate::color_picker_ui::top(app).is_some() {
                    if !matches!(ev, ToolEvent::Up { .. }) {
                        crate::color_picker_ui::sample_at(app, x, y);
                    }
                    continue;
                }
                // Curves' modal eyedroppers use document coordinates here, matching the native
                // canvas path after its ViewXform conversion. Sample on press only.
                if !matches!(s("button"), Some("secondary" | "right")) && crate::adjust_dialog::picker_armed(app) {
                    if matches!(ev, ToolEvent::Down { .. }) {
                        crate::adjust_dialog::sample_at(app, x, y);
                    }
                    continue;
                }
                // The dialog owns all pointer events, including Up: no underlying brush stroke
                // or context menu may leak through while inspecting a Color Range preview.
                if app.ui.dialogs.last().is_some_and(|d| crate::color_range_ui::owns(&d.fields)) {
                    let down = matches!(ev, ToolEvent::Down { .. });
                    let up = matches!(ev, ToolEvent::Up { .. });
                    let held = app.ui.dialogs.last().and_then(|d| d.fields.get("__pointerDown")).and_then(Value::as_bool).unwrap_or(false);
                    if let Some(d) = app.ui.dialogs.last_mut() {
                        d.fields.insert("__pointerDown".into(), json!(!up && (down || held)));
                    }
                    if !up && (down || held) && !space && !matches!(s("button"), Some("secondary" | "right" | "middle")) {
                        crate::color_range_ui::pick_top(app, [x, y], mods);
                    }
                    continue;
                }
                if matches!(s("button"), Some("secondary" | "right")) {
                    let down = matches!(ev, ToolEvent::Down { .. });
                    if app.ui.transform.as_ref().is_some_and(|t| t.warp.is_none()) {
                        if down {
                            crate::canvas_tool_menu::open_transform(app, screen_point(app, x, y));
                        }
                        continue;
                    }
                    // Right-click with the Move tool, or ⌘/Ctrl+right-click: list the layers there.
                    if crate::layer_pick_ui::is_gesture(app.ui.tool, mods) {
                        if down {
                            app.ui.canvas_tool_menu = None;
                            crate::layer_pick_ui::open(app, screen_point(app, x, y), x, y);
                        }
                        continue;
                    }
                    if crate::canvas_tool_menu::applies(app.ui.tool) {
                        if down {
                            crate::canvas_tool_menu::open(app, app.ui.tool, screen_point(app, x, y));
                        }
                        continue;
                    }
                    if !crate::paint_mouse::pointer_secondary(app, down, mods, screen_point(app, x, y)) {
                        continue;
                    }
                }
                // A simulated pen: tilt/rotation reach the stroke like a real stylus's (see `stylus`).
                let tilt = |k: &str| e.get(k).and_then(Value::as_f64).map(|v| v as f32);
                let pen = (tilt("tiltX"), tilt("tiltY"), tilt("rotation"));
                if pen != (None, None, None) {
                    let (tilt_x, tilt_y, rotation) = (pen.0.unwrap_or(0.0), pen.1.unwrap_or(0.0), pen.2.unwrap_or(0.0));
                    app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: pr, tilt_x, tilt_y, rotation, eraser: false }));
                }
                if let Some(d) = app.drag.as_mut().filter(|d| crate::hold_keys::repositions(d.tool)) {
                    d.reposition = space;
                }
                tool_event(app, ev, mods);
            }
            app.stylus.feed.set(None);
            app.stylus.time = 0.0;
            ok(json!({"status": app.ui.status}))
        }
        "ui.drag" => {
            let point = |key: &str| -> Result<egui::Pos2, String> {
                let values = p.get(key).and_then(Value::as_array).filter(|v| v.len() == 2).ok_or_else(|| format!("{key} must be [x,y]"))?;
                let x = values.first().and_then(Value::as_f64).ok_or("invalid x")?;
                let y = values.get(1).and_then(Value::as_f64).ok_or("invalid y")?;
                if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
                    return Err("invalid drag coordinates".into());
                }
                Ok(egui::pos2(x as f32, y as f32))
            };
            let (from, to) = match (point("from"), point("to")) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => return err(e),
            };
            let phase = match p.get("phase") {
                None => "complete",
                Some(v) => match v.as_str() {
                    Some(s) => s,
                    None => return err("drag phase must be a string"),
                },
            };
            if !matches!(phase, "complete" | "start" | "move" | "end") {
                return err("drag phase must be complete, start, move or end");
            }
            let modifiers = egui::Modifiers::NONE;
            if matches!(phase, "complete" | "start") {
                app.synthetic.extend([
                    egui::Event::PointerMoved(from),
                    egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers },
                ]);
            }
            app.synthetic.push(egui::Event::PointerMoved(to));
            if matches!(phase, "complete" | "end") {
                app.synthetic.push(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers });
            }
            app.synthetic_window = window;
            app.synthetic_dialog = dialog;
            if dialog.is_some() {
                crate::doc_windows::queue_input(app, ctx);
            } else {
                crate::doc_windows::queue_drag_input(app, ctx);
            }
            app.synthetic_window = None;
            app.synthetic_dialog = None;
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.click" | "ui.move" => {
            // Screen coordinates in points (as reported by ui.inspect window size).
            let x = p.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let y = p.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let pos = egui::pos2(x, y);
            let button = match s("button").unwrap_or("left") {
                "right" | "secondary" => egui::PointerButton::Secondary,
                "middle" => egui::PointerButton::Middle,
                _ => egui::PointerButton::Primary,
            };
            let clicks = if req.method == "ui.click" { u("count").unwrap_or(1) } else { 0 };
            if clicks > MAX_CLICKS {
                return err(format!("`count` must be at most {MAX_CLICKS} (got {clicks})"));
            }
            app.synthetic.push(egui::Event::PointerMoved(pos));
            for _ in 0..clicks {
                app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: Default::default() });
                app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers: Default::default() });
            }
            app.synthetic_window = window;
            app.synthetic_dialog = dialog;
            crate::doc_windows::queue_input(app, ctx);
            app.synthetic_window = None;
            app.synthetic_dialog = None;
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.key" => {
            let Some(name) = s("key") else { return err("missing `key`") };
            let Some(key) = egui::Key::from_name(name) else { return err(format!("unknown key `{name}`")) };
            // Modifier flags may be top-level or grouped under "modifiers".
            let m = p.get("modifiers").unwrap_or(p);
            let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
            let modifiers = egui::Modifiers {
                command: flag("command"),
                mac_cmd: cfg!(target_os = "macos") && flag("command"),
                shift: flag("shift"),
                alt: flag("alt"),
                ctrl: flag("ctrl"),
            };
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
            app.synthetic_window = window;
            app.synthetic_dialog = dialog;
            crate::doc_windows::queue_input(app, ctx);
            app.synthetic_window = None;
            app.synthetic_dialog = None;
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.type" => {
            let Some(text) = s("text") else { return err("missing `text`") };
            app.synthetic.push(egui::Event::Text(text.to_string()));
            app.synthetic_window = window;
            app.synthetic_dialog = dialog;
            crate::doc_windows::queue_input(app, ctx);
            app.synthetic_window = None;
            app.synthetic_dialog = None;
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.resize" => {
            let (w, h) = (p.get("width").and_then(Value::as_f64).unwrap_or(1280.0), p.get("height").and_then(Value::as_f64).unwrap_or(800.0));
            if !w.is_finite() || !h.is_finite() || !(160.0..=16384.0).contains(&w) || !(120.0..=16384.0).contains(&h) {
                return err("invalid window size");
            }
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::InnerSize(egui::vec2(w as f32, h as f32)));
            ok(Value::Null)
        }
        "ui.gpu.simulateLoss" => {
            let error = p.get("error").and_then(Value::as_bool).unwrap_or(false);
            let fault = if error {
                prism_gpu::Fault::Error("simulated error (ui.gpu.simulateLoss)".into())
            } else {
                prism_gpu::Fault::Lost("simulated (ui.gpu.simulateLoss)".into())
            };
            let active = match app.gpu_health() {
                Some(h) => {
                    h.mark(fault);
                    true
                }
                None => false,
            };
            app.check_gpu(ctx);
            ctx.request_repaint();
            ok(json!({"wasActive": active, "gpuInfo": app.perf.gpu_info}))
        }
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.screenshot" => {
            // Occluded windows don't render on macOS, so the screenshot would never arrive: raise first.
            if p.get("focus").and_then(Value::as_bool).unwrap_or(true) {
                ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            }
            // The capture itself is issued by `PrismApp::issue_screenshots` once open/close
            // animations (modals, popups) have settled.
            let token = app.ui.alloc_id();
            ctx.request_repaint();
            Outcome::Screenshot { token, path: s("path").map(str::to_string), window, dialog }
        }
        "app.open" => match s("path") {
            Some(path) => {
                let opened = match app.services.automation_read.as_mut() {
                    Some(read) => read(path),
                    None => Err("automation read authority is not configured".into()),
                };
                wrap(opened.and_then(|(name, bytes)| {
                    let name = app.open_name(&name);
                    let warnings = app.open_automation_bytes(&name, &bytes)?;
                    // Brushes/gradients go to the preset libraries: no document, no Open Recent entry.
                    if !crate::preset_files_ui::is_preset_file(&name) {
                        app.opened_from(path);
                    }
                    Ok(json!({"path": path, "name": name, "warnings": warnings}))
                }))
            }
            None => err("missing `path`"),
        },
        "app.save" => wrap(app.save_automation(s("path").map(str::to_string)).map(|(p, w)| json!({"path": p, "warnings": w}))),
        "app.quit" => wrap(crate::menus::invoke(app, ctx, "file.exit", Value::Null)),
        other => err(format!("unknown method `{other}`")),
    }
}

/// Snapshot of everything on screen, addressable by id.
pub fn inspect(app: &PrismApp, ctx: &egui::Context) -> Value {
    let screen = ctx.content_rect();
    let dialogs: Vec<Value> =
        app.ui.dialogs.iter().map(|d| json!({"id": d.id, "kind": d.kind, "title": crate::dialogs::title(d), "fields": d.fields, "window": d.window, "presentation":d.presentation,"sourceDocument":d.source_document,"sourceRevision":d.source_revision})).collect();
    json!({
        "window": {"width": screen.width(), "height": screen.height(), "pixelsPerPoint": ctx.pixels_per_point()},
        "tool": app.ui.tool,
        "cursorResource": crate::cursor_pack::last_drawn(ctx),
        "cursorPackResources": crate::cursor_pack::resource_count(ctx),
        "toolOptions": app.ui.tool_options,
        "magnetic": app.ui.magnetic,
        "textEdit": app.ui.text_edit,
        "typeTransform": app.ui.type_transform,
        "update": app.ui.update,
        "shapeCorners": crate::shape_corners_ui::inspect(app),
        "arrowHandles": crate::arrow_handles_ui::inspect(app),
        "specialRulerHandles": crate::special_ruler_ui::handles::current(app),
        "layerMenu": app.ui.layer_menu,
        "brushPicker": app.ui.brush_picker.map(|pos| json!({
            "pos": pos,
            "list": app.ui.brush_picker_list,
        })),
        "canvasToolMenu": app.ui.canvas_tool_menu.as_ref().map(|menu| {
            json!({
                "pos": menu.pos,
                "tool": menu.tool,
                "entries": crate::canvas_tool_menu::rows(menu).iter().map(|row| match row {
                    Some((label, id)) => json!({"label": label, "id": id, "enabled": crate::canvas_tool_menu::entry_enabled(app, menu, id)}),
                    None => json!({"separator": true}),
                }).collect::<Vec<_>>()
            })
        }),
        "panels": app.ui.panels,
        "colorPanel": app.ui.color_panel,
        "view": app.ui.view,
        "views": app.ui.views,
        "dialogs": dialogs,
        "documentContextMenu":crate::document_menu::inspect(app),
        "documentDrag":crate::doc_windows::drag_inspect(ctx),
        "windows": app.ui.windows,
        "activeWindow": app.ui.active_window,
        "mainDocument": app.ui.main_document,
        "dockedDocuments": crate::doc_windows::docked(app),
        "theme": app.ui.theme,
        "status": app.ui.status,
        "statusError": app.ui.status_error,
        "notices": app.ui.notices,
        "gpuFallbackNotice": app.ui.gpu_fallback_notice,
        "frame": app.frame,
        "session": prism_engine::inspect::session(&app.session),
        "document": app.session.active().map(prism_engine::inspect::document),
        "perf": {"fps": app.fps, "timings": app.perf},
        "brush": {"size": app.session.tools.brush.size, "hardness": app.session.tools.brush.hardness, "opacity": app.session.tools.brush.opacity},
        "distort": app.distort.describe(),
        "jobs": crate::jobs_ui::inspect(app),
        "cameraRaw": app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)),
    })
}

pub fn save_screenshot(app: &mut PrismApp, image: &egui::ColorImage, path: Option<&str>) -> Value {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let png = match app.services.encode_png.as_ref() {
        Some(enc) => enc(w as u32, h as u32, &rgba),
        None => Err("no PNG encoder configured".into()),
    };
    let Some(path) = path else {
        return match png {
            Ok(bytes) => json!({
                "ok": true,
                "result": {
                    "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
                    "mime": "image/png",
                    "width": w,
                    "height": h,
                }
            }),
            Err(error) => json!({"ok": false, "error": error}),
        };
    };
    let result = png.and_then(|bytes| match app.services.automation_write.as_mut() {
        Some(write) => write(path, &bytes),
        None => Err("automation write authority is not configured".into()),
    });
    match result {
        Ok(()) => json!({"ok": true, "result": {"path": path, "width": w, "height": h}}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(app: &mut PrismApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(app, ctx, &req) {
            Outcome::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    }

    #[test]
    fn csp_tool_control_picks_tools_and_sub_tools_and_rejects_bad_input() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.clipStudio", Value::Null).unwrap();
        let r = call(&mut app, &ctx, "ui.cspTool", json!({"tool": "fill", "subTool": "Refer other layers"}));
        let res = &r["result"];
        assert_eq!((res["active"].as_bool(), res["tool"].as_str(), res["subTool"].as_str()), (Some(true), Some("fill"), Some("Refer other layers")), "{r}");
        assert!(app.ui.tool_options.sample_all_layers);
        assert_eq!(res["prismTool"], "PaintBucket");
        for bad in [json!({"tool": 3}), json!({"tool": "lasso"}), json!({"subTool": "nope"}), json!({"subTool": false})] {
            let r = call(&mut app, &ctx, "ui.cspTool", bad.clone());
            assert_eq!(r["ok"], false, "{bad} -> {r}");
        }
    }

    fn pen_control_app() -> PrismApp {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":8,"hardness":1.0,"pressureSize":true,"smoothing":{"amount":0}}})).unwrap();
        app.ui.tool = Tool::Brush;
        app.sync_views();
        app
    }

    fn pen_trace_packet(phase: crate::pen_input::Phase, x: f32, pressure: f32, time: f64) -> crate::pen_input::Packet {
        crate::pen_input::Packet {
            source: "windowsInk",
            id: 7,
            device: 11,
            phase,
            pos: egui::pos2(x, 32.0),
            sample: crate::stylus::PenSample { pressure, tilt_x: 15.0, ..Default::default() },
            pressure_valid: phase != crate::pen_input::Phase::Up,
            pressure_supported: Some(true),
            time,
            barrel: false,
        }
    }

    fn record_pen_packets(app: &mut PrismApp, packets: &[crate::pen_input::Packet]) {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
        let xf = crate::canvas::ViewXform { rect, zoom: 1.0, center: [32.0, 32.0], flip: false, rotation: 0.0 };
        crate::pen_input::paint_packets(app, egui::ViewportId::ROOT, packets, rect, &xf, egui::Modifiers::NONE);
    }

    #[test]
    fn pointer_optional_times_reach_stroke_and_legacy_points_stay_compatible() {
        let mut app = pen_control_app();
        let ctx = egui::Context::default();
        let result = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({"events":[
                {"kind":"down","x":10,"y":32,"pressure":0.1,"time":100},
                {"kind":"move","x":20,"y":32,"pressure":0.7,"time":120},
                {"kind":"up","x":20,"y":32,"time":130}
            ]}),
        );
        assert_eq!(result["ok"], true, "{result}");
        let points = app.session.journal.last().unwrap().1["points"].as_array().unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points.iter().map(|p| p[6].as_f64().unwrap()).collect::<Vec<_>>(), vec![100.0, 120.0]);
        assert_eq!(points[0][2], json!(f64::from(0.1f32)));
        assert_eq!(points[1][2], json!(f64::from(0.7f32)));
        assert_eq!(app.stylus.time, 0.0, "control timestamp must not leak into the next mouse stroke");
        let result = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({"events":[
                {"kind":"down","x":30,"y":32}, {"kind":"up","x":30,"y":32}
            ]}),
        );
        assert_eq!(result["ok"], true, "{result}");
        let points = app.session.journal.last().unwrap().1["points"].as_array().unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].as_array().unwrap().len(), 3, "legacy calls retain the compact point format");
        assert_eq!(points[0][2], 1.0);
    }

    #[test]
    fn pointer_cancel_drops_preview_without_history_or_pressure_leak() {
        let mut app = pen_control_app();
        let ctx = egui::Context::default();
        let history = app.session.active().unwrap().history.past_len();
        let journal = app.session.journal.len();
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "ui.pointer",
                json!({"events":[
                    {"kind":"down","x":10,"y":32,"pressure":0.9,"tiltX":30,"time":100},
                    {"kind":"move","x":40,"y":32,"pressure":0.8,"tiltX":20,"time":120}
                ]})
            )["ok"],
            true
        );
        assert!(app.drag.is_some() && app.live_stroke.is_some());
        assert_eq!(call(&mut app, &ctx, "ui.pointer", json!({"events":[{"kind":"cancel"}]}))["ok"], true);
        assert!(app.drag.is_none() && app.live_stroke.is_none() && app.stylus.contact.is_none());
        assert_eq!(app.stylus.feed.get(), None);
        assert_eq!(app.stylus.time, 0.0);
        assert_eq!(app.session.journal.len(), journal);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "ui.pointer",
                json!({"events":[
                    {"kind":"down","x":32,"y":32,"pressure":0.05}, {"kind":"up","x":32,"y":32}
                ]})
            )["ok"],
            true
        );
        let points = app.session.journal.last().unwrap().1["points"].as_array().unwrap();
        assert_eq!(points[0][2], json!(f64::from(0.05f32)));
        assert_eq!(points[0].as_array().unwrap().len(), 3, "canceled pose/time must not leak into a new contact");
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
    }

    #[test]
    fn pointer_batch_rejects_invalid_late_event_before_changing_tool_or_preview() {
        let mut app = pen_control_app();
        let ctx = egui::Context::default();
        assert_eq!(call(&mut app, &ctx, "ui.pointer", json!({"events":[{"kind":"down","x":12,"y":32,"pressure":0.2}]}))["ok"], true);
        let points = app.drag.as_ref().unwrap().points.clone();
        let journal = app.session.journal.len();
        let history = app.session.active().unwrap().history.past_len();
        let invalid = [
            json!(null),
            json!("move"),
            json!({"kind":4}),
            json!({"kind":"teleport"}),
            json!({"kind":"move","x":null}),
            json!({"kind":"move","y":"bad"}),
            json!({"kind":"move","pressure":-0.1}),
            json!({"kind":"move","pressure":1.1}),
            json!({"kind":"move","pressure":"light"}),
            json!({"kind":"move","time":-1}),
            json!({"kind":"move","time":1_000_000_000_001u64}),
            json!({"kind":"move","time":"now"}),
            json!({"kind":"move","tiltX":"left"}),
            json!({"kind":"cancel","rotation":null}),
        ];
        for bad in invalid {
            let result = call(
                &mut app,
                &ctx,
                "ui.pointer",
                json!({"tool":"pencil","events":[
                    {"kind":"move","x":24,"y":32,"pressure":0.8}, bad
                ]}),
            );
            assert_eq!(result["ok"], false, "{bad}: {result}");
            assert_eq!(app.ui.tool, Tool::Brush);
            assert_eq!(app.drag.as_ref().unwrap().points, points);
            assert_eq!(app.session.journal.len(), journal);
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
        }
        let events = vec![json!({"kind":"move","x":24,"y":32}); 4097];
        let result = call(&mut app, &ctx, "ui.pointer", json!({"tool":"pencil","events":events}));
        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("too many"));
        assert_eq!(app.ui.tool, Tool::Brush);
        assert_eq!(app.drag.as_ref().unwrap().points, points);
    }

    #[test]
    fn pen_trace_is_opt_in_bounded_stoppable_and_clearable() {
        use crate::pen_input::Phase;
        let mut app = pen_control_app();
        let ctx = egui::Context::default();
        let initial = call(&mut app, &ctx, "ui.penTrace", json!({}));
        assert_eq!(initial["result"]["enabled"], false);
        assert_eq!(initial["result"]["capacity"], 4096);
        record_pen_packets(&mut app, &[pen_trace_packet(Phase::Move, 20.0, 0.2, 1.0)]);
        assert!(call(&mut app, &ctx, "ui.penTrace", json!({}))["result"]["events"].as_array().unwrap().is_empty());
        assert_eq!(call(&mut app, &ctx, "ui.penTrace", json!({"enabled":true,"clear":true}))["result"]["enabled"], true);
        let packets: Vec<_> = (0..4105).map(|i| pen_trace_packet(Phase::Move, 20.0, 0.2, f64::from(i))).collect();
        record_pen_packets(&mut app, &packets);
        let trace = call(&mut app, &ctx, "ui.penTrace", json!({}));
        let events = trace["result"]["events"].as_array().unwrap();
        assert_eq!(events.len(), 4096);
        assert_eq!(events.first().unwrap()["time"], 9.0, "only the oldest readings are evicted");
        assert_eq!(events.last().unwrap()["time"], 4104.0);
        assert_eq!(events[0]["source"], "windowsInk");
        assert_eq!(events[0]["device"], 11);
        assert_eq!(events[0]["pressureValid"], true);
        assert_eq!(events[0]["pressureSupported"], true);
        let stopped = call(&mut app, &ctx, "ui.penTrace", json!({"enabled":false}));
        record_pen_packets(&mut app, &[pen_trace_packet(Phase::Move, 20.0, 0.3, 9999.0)]);
        assert_eq!(call(&mut app, &ctx, "ui.penTrace", json!({}))["result"]["events"], stopped["result"]["events"]);
        assert_eq!(call(&mut app, &ctx, "ui.penTrace", json!({"enabled":"yes","clear":true}))["ok"], false);
        assert_eq!(app.stylus.trace.len(), 4096, "invalid enable must not clear the existing trace");
        let cleared = call(&mut app, &ctx, "ui.penTrace", json!({"clear":true}));
        assert_eq!(cleared["result"]["enabled"], false);
        assert!(cleared["result"]["events"].as_array().unwrap().is_empty());
    }

    #[test]
    fn pen_trace_tip_contact_replays_pressure_pose_and_time_through_pointer_control() {
        use crate::pen_input::Phase;
        let mut app = pen_control_app();
        let ctx = egui::Context::default();
        assert_eq!(call(&mut app, &ctx, "ui.penTrace", json!({"enabled":true}))["ok"], true);
        record_pen_packets(
            &mut app,
            &[pen_trace_packet(Phase::Down, 12.0, 0.1, 10.0), pen_trace_packet(Phase::Move, 28.0, 0.6, 20.0), pen_trace_packet(Phase::Up, 28.0, 0.0, 30.0)],
        );
        let original = app.session.journal.last().unwrap().1["points"].clone();
        let trace = call(&mut app, &ctx, "ui.penTrace", json!({"enabled":false}));
        let events = trace["result"]["events"].clone();
        assert_eq!(events.as_array().unwrap().len(), 3);
        assert_eq!(events[0]["x"], 12.0);
        assert_eq!(events[0]["y"], 32.0);
        let replay = call(&mut app, &ctx, "ui.pointer", json!({"events":events}));
        assert_eq!(replay["ok"], true, "{replay}");
        assert_eq!(app.session.journal.last().unwrap().1["points"], original);
    }

    #[test]
    fn expired_queued_edit_does_not_run_and_a_live_retry_runs_once() {
        use std::time::{Duration, Instant};
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let before = app.session.active().unwrap();
        let layers = before.doc.layers.len();
        let steps = before.history.past_len();
        let (tx, rx) = std::sync::mpsc::channel();
        app = app.with_control(rx);
        let (mut expired, expired_reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
        expired.deadline = Some(Instant::now() - Duration::from_secs(1));
        tx.send(expired).unwrap();
        let (mut retry, retry_reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
        retry.deadline = Some(Instant::now() + Duration::from_secs(60));
        tx.send(retry).unwrap();
        app.drain_control(&egui::Context::default());
        assert_eq!(expired_reply.try_recv().unwrap(), json!({"ok": false, "error": "timeout"}));
        assert_eq!(retry_reply.try_recv().unwrap()["ok"], true);
        let after = app.session.active().unwrap();
        assert_eq!(after.doc.layers.len(), layers + 1, "only the live retry adds a layer");
        assert_eq!(after.history.past_len(), steps + 1, "the expired edit adds no undo step");
    }

    #[test]
    fn deadline_free_requests_run_even_after_the_receiver_is_dropped() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let layers = app.session.active().unwrap().doc.layers.len();
        let (tx, rx) = std::sync::mpsc::channel();
        app = app.with_control(rx);
        let (req, reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
        assert!(req.deadline.is_none());
        drop(reply);
        tx.send(req).unwrap();
        app.drain_control(&egui::Context::default());
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    }

    #[test]
    fn right_pointer_opens_agent_visible_selection_menu() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
        let result = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({
                "tool": "RectMarquee", "button": "secondary",
                "events": [{"kind": "down", "x": 4, "y": 4}, {"kind": "up", "x": 4, "y": 4}]
            }),
        );
        assert_eq!(result.get("ok"), Some(&Value::Bool(true)));
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
        assert!(entries.iter().any(|entry| entry.get("id") == Some(&json!("select.inverse")) && entry.get("enabled") == Some(&json!(true))));
        assert!(app.session.active().unwrap().doc.selection.is_some(), "right-click must not edit selection");
    }

    #[test]
    fn agent_can_inspect_and_choose_pen_make_selection_from_full_context_menu() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":true,"knots":[[2,2],[20,2],[20,20]]}]}})).unwrap();
        let expected_pos = screen_point(&app, 8.0, 8.0);
        let result = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({
                "tool": "Pen", "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]
            }),
        );
        assert_eq!(result["ok"], true);
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(app.ui.canvas_tool_menu.as_ref().unwrap().pos, expected_pos);
        let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
        assert_eq!(entries.iter().filter(|row| row.get("separator") == Some(&json!(true))).count(), 9);
        assert!(entries.iter().any(|row| row.get("id") == Some(&json!("path.toSelection")) && row.get("enabled") == Some(&json!(true))));
        assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"file.new"}))["ok"], false);
        let chosen = call(&mut app, &ctx, "ui.context.choose", json!({"id":"path.toSelection"}));
        assert_eq!(chosen["ok"], true);
        let dialog = chosen["result"]["dialog"].as_u64().unwrap();
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":dialog}))["ok"], true);
        assert!(app.session.active().unwrap().doc.selection.is_some());
    }

    #[test]
    fn ui_inspect_reports_the_view_preferences() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let before = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(before.pointer("/result/view/show/selection_edges"), Some(&json!(true)));
        assert_eq!(before.pointer("/result/view/screen_mode"), Some(&json!("standard")));
        assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "view.show.selectionEdges"}))["ok"], true);
        let after = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(after.pointer("/result/view/show/selection_edges"), Some(&json!(false)));
        assert_eq!(after["result"]["view"], json!(app.ui.view));
    }

    #[test]
    fn engine_execute_runs_with_defaults_but_menu_invoke_opens_the_dialog() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let rev = app.session.active().unwrap().revision;
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "filter.blur.gaussianBlur"}));
        assert!(!r.to_string().contains("\"dialog\""), "engine.execute opened a dialog: {r}");
        assert!(app.session.active().unwrap().revision > rev, "engine.execute didn't run the filter");
        let rev = app.session.active().unwrap().revision;
        let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "filter.blur.gaussianBlur"}));
        assert!(r.to_string().contains("dialog"), "ui.menu.invoke should open the dialog: {r}");
        assert_eq!(app.session.active().unwrap().revision, rev, "opening a dialog must not edit the document");
    }

    #[test]
    fn menu_invoke_cannot_edit_document_behind_an_open_dialog() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();

        let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.imageSize"}));
        assert_eq!(opened["ok"], true, "{opened}");
        let dialog = opened["result"]["dialog"].as_u64().unwrap();
        let revision = app.session.active().unwrap().revision;
        let before = call(&mut app, &ctx, "ui.inspect", json!({}));
        let fields = before["result"]["dialogs"][0]["fields"].clone();

        for params in [json!({"id": "image.imageRotation.90cw"}), json!({"id": "image.imageRotation.90cw", "params": {}})] {
            let blocked = call(&mut app, &ctx, "ui.menu.invoke", params);
            assert_eq!(blocked["ok"], false, "{blocked}");
            assert_eq!(app.session.active().unwrap().revision, revision);
            let snapshot = call(&mut app, &ctx, "ui.inspect", json!({}));
            assert_eq!(snapshot["result"]["dialogs"][0]["id"], dialog);
            assert_eq!(snapshot["result"]["dialogs"][0]["fields"], fields);
        }

        // View navigation stays permitted by the same policy as native menus and shortcuts.
        assert!(crate::menus::modal_allows(&app, "view.zoomIn"));
        assert!(!crate::menus::modal_allows(&app, "image.imageRotation.90cw"));

        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
        let rotated = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.imageRotation.90cw"}));
        assert_eq!(rotated["ok"], true, "{rotated}");
        assert!(app.session.active().unwrap().revision > revision);
    }

    #[test]
    fn arrow_control_is_atomic_and_defaults_survive_old_settings() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool":"arrow","arrow":{"width":5,"rounded":false}}))["ok"], true);
        assert_eq!(app.ui.tool, Tool::Arrow);
        assert_eq!(app.ui.tool_options.arrow.head_size, 16.0);
        assert_eq!(app.ui.tool_options.arrow.width, 5.0);
        for arrow in [
            json!({"width":0}),
            json!({"headSize":-3}),
            json!({"headAngle":0}),
            json!({"headAngle":161}),
            json!({"headAngle":"60"}),
            json!({"width":1e300}),
            json!({"rounded":"yes"}),
            json!({"typo":true}),
        ] {
            assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool":"move","arrow":arrow}))["ok"], false);
            assert_eq!(app.ui.tool, Tool::Arrow);
            assert_eq!(app.ui.tool_options.arrow.width, 5.0);
        }
        let old: crate::state::ToolOptions = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old.arrow, crate::state::ArrowOptions::default());
        let copy: crate::state::ToolOptions = serde_json::from_value(serde_json::to_value(&app.ui.tool_options).unwrap()).unwrap();
        assert_eq!(copy.arrow, app.ui.tool_options.arrow);
    }

    #[test]
    fn line_tips_control_is_atomic_and_persisted() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool":"line","lineRoundTips":true}))["ok"], true);
        assert!(app.ui.tool_options.line_round_tips);
        for value in [json!("round"), json!(1), json!(null)] {
            assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool":"move","lineRoundTips":value}))["ok"], false);
            assert_eq!(app.ui.tool, Tool::Line);
            assert!(app.ui.tool_options.line_round_tips);
        }
        let old: crate::state::ToolOptions = serde_json::from_value(json!({})).unwrap();
        assert!(!old.line_round_tips);
        let back: crate::state::ToolOptions = serde_json::from_value(serde_json::to_value(&app.ui.tool_options).unwrap()).unwrap();
        assert!(back.line_round_tips);
    }

    #[test]
    fn font_filter_control_is_validated_and_serialized() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"typeFontFilter":"korean"}))["ok"], true);
        assert_eq!(app.ui.tool_options.type_font_filter, crate::state::TypeFontFilter::Korean);
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"typeFontFilter":"bad"}))["ok"], false);
        assert_eq!(app.ui.tool_options.type_font_filter, crate::state::TypeFontFilter::Korean);
        let restored: crate::state::ToolOptions = serde_json::from_value(serde_json::to_value(&app.ui.tool_options).unwrap()).unwrap();
        assert_eq!(restored.type_font_filter, crate::state::TypeFontFilter::Korean);
    }

    #[test]
    fn ui_set_rejects_unknown_fields_before_changing_anything() {
        use crate::theme::ThemeKind;
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.set_theme(&ctx, ThemeKind::StudioLight);
        // `dark` was advertised to MCP clients but never read; a typo looked like success too.
        for params in [json!({"dark": true}), json!({"thme": "classic"}), json!({"tool": "move", "dark": true})] {
            let r = call(&mut app, &ctx, "ui.set", params.clone());
            assert_eq!(r["ok"], false, "{params}: {r}");
            assert!(r["error"].as_str().unwrap().contains("unknown field"), "{r}");
        }
        assert_eq!(app.ui.theme, ThemeKind::StudioLight);
        assert_eq!(app.ui.tool, Tool::Brush, "a rejected call applies none of its fields");
        // Every field the method reads gets past the check (a bad value is its own error).
        for field in UI_SET_FIELDS {
            let r = call(&mut app, &ctx, "ui.set", json!({ field: null }));
            assert!(!r.to_string().contains("unknown field"), "{field}: {r}");
        }
        assert_eq!(call(&mut app, &ctx, "ui.set", Value::Null)["ok"], true);
    }

    #[test]
    fn ui_set_gradient_blend_mode_validates_and_updates_options() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let good = call(&mut app, &ctx, "ui.set", json!({"tool": "gradient", "gradientBlendMode": "Difference", "gradientClassic": true}));
        assert_eq!(good["ok"], true, "{good}");
        assert_eq!(app.ui.tool_options.gradient_blend_mode, prism_color::BlendMode::Difference);
        assert!(app.ui.tool_options.gradient_classic);
        let bad = call(&mut app, &ctx, "ui.set", json!({"gradientBlendMode": "nonsense", "gradientClassic": false}));
        assert_eq!(bad["ok"], false, "{bad}");
        assert!(app.ui.tool_options.gradient_classic, "invalid mode must not change options");
    }

    #[test]
    fn ui_set_opens_the_brush_preset_picker_and_sets_its_view() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert_eq!(app.ui.brush_picker_list.view, crate::brush_panel::BrushesView::Grid, "tip thumbnails by default");
        let r = call(&mut app, &ctx, "ui.set", json!({"tool": "brush", "brushPicker": [120, 80], "brushPickerView": "list"}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.ui.brush_picker, Some([120.0, 80.0]));
        assert_eq!(app.ui.brush_picker_list.view, crate::brush_panel::BrushesView::List);
        for bad in [json!({"brushPicker": [1]}), json!({"brushPicker": "here"}), json!({"brushPickerView": "tiles"})] {
            assert_eq!(call(&mut app, &ctx, "ui.set", bad.clone())["ok"], false, "{bad}");
        }
        assert_eq!(app.ui.brush_picker, Some([120.0, 80.0]), "a bad value leaves the picker alone");
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPicker": null}))["ok"], true);
        assert_eq!(app.ui.brush_picker, None);
    }

    #[test]
    fn ui_inspect_tracks_brush_picker_open_and_closed() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 128, "height": 128})).unwrap();

        let closed = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert!(closed["result"]["brushPicker"].is_null());

        // A right-click on the canvas with Brush opens the same picker as the options bar.
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool": "brush"}))["ok"], true);
        let click = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({
                "button": "right",
                "events": [{"kind": "down", "x": 64, "y": 64}, {"kind": "up", "x": 64, "y": 64}]
            }),
        );
        assert_eq!(click["ok"], true, "{click}");
        let open = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert!(open["result"]["brushPicker"]["pos"].is_array(), "{open}");
        assert_eq!(open["result"]["brushPicker"]["list"]["view"], "grid");

        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPickerView": "list"}))["ok"], true);
        let changed = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(changed["result"]["brushPicker"]["list"]["view"], "list");

        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPicker": null}))["ok"], true);
        let closed_again = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert!(closed_again["result"]["brushPicker"].is_null());
    }

    #[test]
    fn ui_set_applies_all_fields_or_none() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let before = app.ui.tool;
        let panels_before = serde_json::to_value(&app.ui.panels).unwrap_or_default();
        // A late invalid field must not leave the earlier valid one applied - including the
        // journaled brush command, which is validated up front for exactly this reason.
        for params in
            [json!({"tool": "move", "theme": "nope"}), json!({"tool": "move", "brushSize": 1e300}), json!({"tool": "move", "colorPanel": {"nope": true}})]
        {
            let r = call(&mut app, &ctx, "ui.set", params.clone());
            assert_eq!(r["ok"], false, "{params}: {r}");
            assert_eq!(app.ui.tool, before, "{params}: a rejected call applies none of its fields");
        }
        // Wrong types and unknown nested keys are errors, not silent no-ops with ok:true.
        for params in [
            json!({"panels": "x"}),
            json!({"panels": {"nope": true}}),
            json!({"dockTabs": {"nope": 1}}),
            json!({"dock": {"nope": 1}}),
            json!({"center": [1]}),
            json!({"center": [1, 2, 3]}),
            json!({"center": "here"}),
            json!({"colorPanel": "x"}),
            json!({"selectionMode": "add"}),
        ] {
            let r = call(&mut app, &ctx, "ui.set", params.clone());
            assert_eq!(r["ok"], false, "{params}: {r}");
        }
        assert_eq!(serde_json::to_value(&app.ui.panels).unwrap_or_default(), panels_before, "nothing was applied");
        // Non-numeric brushSize keeps the API's historical silent no-op (see the dispatch test).
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushSize": "large"}))["ok"], true);
    }

    #[test]
    fn ui_set_color_panel_picks_the_edited_chip() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"background": true}}))["ok"], true);
        assert!(app.ui.color_panel.background);
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"background": "yes"}}))["ok"], false);
        assert!(app.ui.color_panel.background, "a bad value changes nothing");
    }

    #[test]
    fn ui_set_color_panel_modes_validate_atomically_and_keep_legacy_defaults() {
        use crate::state::ColorPanelMode;
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        for (name, mode) in [
            ("hueCube", ColorPanelMode::HueCube),
            ("brightnessCube", ColorPanelMode::BrightnessCube),
            ("colorWheel", ColorPanelMode::ColorWheel),
            ("hsbSliders", ColorPanelMode::HsbSliders),
        ] {
            assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel":{"mode":name,"background":true}}))["ok"], true);
            assert_eq!(app.ui.color_panel.mode, mode);
            assert_eq!(call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["colorPanel"]["mode"], name);
        }
        let before = app.ui.color_panel;
        for bad in [json!({"mode":"wheel"}), json!({"mode":3}), json!({"mode":null}), json!({"mode":[]}), json!({"unknown":true})] {
            assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel":bad,"panels":{"color":false}}))["ok"], false);
            assert_eq!(app.ui.color_panel, before);
            assert!(app.ui.panels.color, "invalid mode must not partially apply panel changes");
        }
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel":{"background":true}}))["ok"], true);
        assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HueCube, "old payloads keep the default hue cube");
    }

    #[test]
    fn ui_set_unknown_theme_error_names_every_theme_and_each_name_works() {
        use crate::theme::ThemeKind;
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "ui.set", json!({"theme": "nope"}));
        assert_eq!(r["ok"], false);
        let error = r["error"].as_str().unwrap();
        for kind in ThemeKind::ALL {
            assert!(error.contains(kind.id()), "{error} lacks {}", kind.id());
            assert_eq!(call(&mut app, &ctx, "ui.set", json!({"theme": kind.id()}))["ok"], true);
            assert_eq!(app.ui.theme, kind);
            assert_eq!(ThemeKind::from_name(kind.id()), Some(kind));
        }
    }

    #[test]
    fn ui_window_open_rejects_an_unknown_document_index() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        // An out-of-range index must error instead of silently switching the active document
        // and opening the window for whatever is active now.
        let before = app.session.active_index();
        let r = call(&mut app, &ctx, "ui.window.open", json!({"document": 99}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().unwrap().contains("no document 99"), "{r}");
        assert_eq!(app.session.active_index(), before, "the active document is untouched");
        // A valid index still opens the window.
        assert_eq!(call(&mut app, &ctx, "ui.window.open", json!({"document": 0}))["ok"], true);
    }

    #[test]
    fn ui_set_brush_size_dispatches_a_journaled_brush_command() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();

        let r = call(&mut app, &ctx, "ui.set", json!({"brushSize": 42.5}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.session.tools.brush.size, 42.5);
        let (id, params) = app.session.journal.last().cloned().expect("brush change is journaled");
        assert_eq!(id, "tools.setBrush");
        assert_eq!(params, json!({"brush": {"size": 42.5}}));

        // The control API has historically ignored non-numeric optional values.
        for params in [json!({}), json!({"brushSize": null}), json!({"brushSize": "large"}), json!({"brushSize": true})] {
            let journal_len = app.session.journal.len();
            let r = call(&mut app, &ctx, "ui.set", params);
            assert_eq!(r["ok"], true, "{r}");
            assert_eq!(app.session.tools.brush.size, 42.5);
            assert_eq!(app.session.journal.len(), journal_len);
        }

        // Each tool keeps its own brush (#218), so `tool` + `brushSize` in one call sets the new
        // tool's size rather than the one it was carrying.
        call(&mut app, &ctx, "ui.set", json!({"tool": "eraser", "brushSize": 12.0}));
        assert_eq!(app.session.tools.brush.size, 12.0, "the Eraser's own size");
        call(&mut app, &ctx, "ui.set", json!({"tool": "brush"}));
        assert_eq!(app.session.tools.brush.size, 42.5, "the Brush gets its own back");
        call(&mut app, &ctx, "ui.set", json!({"tool": "eraser"}));
        assert_eq!(app.session.tools.brush.size, 12.0);
        call(&mut app, &ctx, "ui.set", json!({"tool": "brush", "brushSize": 42.5}));

        // Values that cannot be represented by BrushSettings must report the command error and
        // leave both the brush and journal unchanged instead of mutating tool state directly.
        let journal_len = app.session.journal.len();
        let r = call(&mut app, &ctx, "ui.set", json!({"brushSize": f64::MAX}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().is_some(), "{r}");
        assert_eq!(app.session.tools.brush.size, 42.5);
        assert_eq!(app.session.journal.len(), journal_len);
    }

    #[test]
    fn preferences_apply_is_available_over_control_without_closing() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "edit.preferences.interface"}));
        let id = r["result"]["dialog"].as_u64().unwrap();
        let mut values = app.ui.dialog_mut(id).unwrap().fields["values"].clone();
        values["interface"]["uiScale"] = json!("200");
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": id, "field": "values", "value": values}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}))["ok"], true);
        assert_eq!(app.session.prefs().interface.ui_scale, prism_engine::prefs::UiScale::P200);
        assert!(app.ui.dialog_mut(id).is_some());
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], true);
        assert_eq!(app.session.prefs().interface.ui_scale, prism_engine::prefs::UiScale::P200);
        for params in [json!({}), json!({"dialog": "invalid"}), json!({"dialog": u64::MAX})] {
            assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", params)["ok"], false);
        }
    }

    #[test]
    fn preferences_apply_respects_the_automation_command_policy() {
        let services = crate::Services {
            automation_command: Some(Box::new(|id, _| if id == "prefs.set" { Err("preference changes denied".into()) } else { Ok(()) })),
            ..Default::default()
        };
        let mut app = PrismApp::new(prism_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let before = app.session.prefs().to_json();
        let id = crate::prefs_ui::open_preferences(&mut app, "interface");
        app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["interface"]["uiScale"] = json!("200");
        let r = call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}));
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"], "preference changes denied");
        assert_eq!(app.session.prefs().to_json(), before);
        assert!(app.ui.dialog_mut(id).is_some());
    }

    #[test]
    fn shape_picker_confirmation_authorizes_the_actual_layer_and_fill_params() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":40,"height":30})).unwrap();
        let layer = app.run("shape.create", json!({"rect":[2,2,20,20],"fill":"#0000ff"})).unwrap()["layer"].as_u64().unwrap();
        let original = app.session.active().unwrap().doc.clone();
        app.services.automation_command = Some(Box::new(move |id, p| {
            if id == "shape.edit" {
                assert_eq!(p["layer"], layer);
                assert_eq!(p["fill"], json!([1.0, 0.0, 0.0, 1.0]));
                Err("shape edits denied".into())
            } else {
                Ok(())
            }
        }));
        let ctx = egui::Context::default();
        let id = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","target":"shapeFill"}))["result"]["dialog"].as_u64().unwrap();
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog":id,"field":"color","value":"#ff0000"}))["ok"], true);
        // Presentation metadata cannot redirect the edit to a different layer/command.
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog":id,"field":"__params","value":{"layer":u64::MAX}}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog":id,"field":"__command","value":"layer.delete"}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":id}))["ok"], false);
        assert!(std::sync::Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
        assert!(app.ui.dialog_mut(id).is_some(), "a denied edit leaves the picker open");
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog":id}))["ok"], true);
    }

    #[test]
    fn synthetic_shortcuts_use_the_automation_command_policy() {
        let services = crate::Services {
            automation_command: Some(Box::new(|id, _| if id.starts_with("file.") { Err("filesystem command denied".into()) } else { Ok(()) })),
            ..Default::default()
        };
        let mut app = PrismApp::new(prism_engine::Session::new(), services);
        let ctx = egui::Context::default();
        app.automation_input = true;
        let error = crate::menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err();
        assert_eq!(error, "filesystem command denied");
    }

    #[test]
    fn automation_open_and_save_use_the_roots_and_reply_with_warnings() {
        use prism_color::{ColorMode, SampleType};
        use prism_doc::Document;
        use prism_geom::Size;
        use std::cell::RefCell;
        use std::rc::Rc;

        // Without granted roots both fail closed, even with an exporter and an ambient writer.
        let services = crate::Services {
            export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
            write: Some(Box::new(|_p: &str, _b: &[u8]| Err("ambient writer used".into()))),
            ..Default::default()
        };
        let mut app = PrismApp::new(prism_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
        assert!(r.to_string().contains("automation read authority is not configured"), "{r}");
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
        assert!(r.to_string().contains("automation write authority is not configured"), "{r}");

        let written: Rc<RefCell<Vec<String>>> = Rc::default();
        let w = written.clone();
        let services = crate::Services {
            import: Some(Box::new(|name: &str, _b: &[u8]| {
                Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), vec!["Adjustment layer flattened".to_string()]))
            })),
            export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), vec!["Layers were flattened".to_string()])))),
            automation_read: Some(Box::new(|path: &str| Ok((path.rsplit('/').next().unwrap_or(path).to_string(), b"x".to_vec())))),
            automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
                w.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PrismApp::new(prism_engine::Session::new(), services);
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
        assert_eq!(r["result"]["warnings"], json!(["Adjustment layer flattened"]), "{r}");
        assert_eq!(r["result"]["name"], "warn.psd");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("in/warn.psd"));
        assert_eq!(app.ui.recent_files, vec!["in/warn.psd".to_string()]);
        let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
        assert_eq!(r["result"], json!({"path": "out.png", "warnings": ["Layers were flattened"]}), "{r}");
        assert_eq!(*written.borrow(), vec!["out.png".to_string()]);
        // Without `path`, only a layered file is written back, like File › Save (#416).
        call(&mut app, &ctx, "app.open", json!({"path": "in/flat.jpg"}));
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
        assert_eq!(written.borrow().len(), 1, "nothing written over the JPEG");
        call(&mut app, &ctx, "app.open", json!({"path": "in/layered.psd"}));
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert_eq!(r["result"]["path"], "in/layered.psd", "{r}");
        assert_eq!(written.borrow().last().map(String::as_str), Some("in/layered.psd"));
        // A template opens untitled, so a save without `path` never writes over it.
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/card.psdt"}));
        assert_eq!(r["result"]["name"], "Untitled-1", "{r}");
        assert_eq!(app.session.active().unwrap().path, None);
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
    }

    fn deny_ambient_file(id: &str, _: &serde_json::Value) -> prism_engine::Result<()> {
        if id.starts_with("file.") && id != "file.new" {
            Err(prism_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
        } else {
            Ok(())
        }
    }

    #[test]
    fn actions_play_checks_each_step_over_control() {
        let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
        let mut app = PrismApp::new(prism_engine::Session::new(), services);
        app.session.actions.list.push(prism_engine::actions_cmds::Action::new(
            "Open",
            vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        ));
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "actions.play", "params": {"action": "Open"}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["ran"], 0, "{r}");
        assert_eq!(r["result"]["failed"]["id"], "file.open", "{r}");
        assert!(r["result"]["failed"]["error"].as_str().unwrap_or("").contains("disabled"), "{r}");
        assert!(app.session.documents().is_empty());
        assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the request");
    }

    #[test]
    fn actions_play_checks_each_step_for_synthetic_input() {
        let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
        let mut app = PrismApp::new(prism_engine::Session::new(), services);
        app.session.actions.list.push(prism_engine::actions_cmds::Action::new(
            "Open",
            vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        ));
        app.automation_input = true;
        let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
        assert_eq!(r["ran"], 0, "{r}");
        assert_eq!(r["failed"]["id"], "file.open", "{r}");
        assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the command");

        // A local play (no automation input) still runs recorded steps.
        app.automation_input = false;
        app.session.actions.list[0].steps.remove(0);
        app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
        assert_eq!(r["ran"], 1, "{r}");
    }

    #[test]
    fn huge_click_count_is_rejected() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        // #982: a huge count used to queue a press and a release per click before replying.
        for count in [json!(MAX_CLICKS + 1), json!(1_000_000_000u64), json!(u64::MAX)] {
            let r = call(&mut app, &ctx, "ui.click", json!({"x": 10, "y": 10, "count": count}));
            assert_eq!(r["ok"], false, "{r}");
            assert!(r["error"].as_str().unwrap().contains("`count` must be at most 256"), "{r}");
            assert!(app.synthetic.is_empty(), "a rejected click queues nothing");
        }
        // Single, double and the largest allowed clicks still queue a move plus a press and a release each.
        for (count, events) in [(None, 3), (Some(2), 5), (Some(MAX_CLICKS), 2 * MAX_CLICKS as usize + 1)] {
            let params = match count {
                Some(n) => json!({"x": 10, "y": 10, "count": n}),
                None => json!({"x": 10, "y": 10}),
            };
            let (req, _rx) = ControlRequest::new("ui.click", params);
            assert!(matches!(handle(&mut app, &ctx, &req), Outcome::AfterInput));
            assert_eq!(app.synthetic.len(), events);
            app.synthetic.clear();
        }
    }
}
