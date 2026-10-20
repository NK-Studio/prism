//! Actions panel: record and replay command sequences.
//!
//! The list and the recording flag live on the engine session (`actions.record` / `stop` /
//! `play` / `list` / `get` / `delete` / `move`), so the panel, the CLI and MCP share them. This module
//! keeps only which row is selected and which rows are expanded.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use prism_engine::actions_cmds::{self, Action, ActionStep};

use crate::PrismApp;
use crate::theme::Tokens;

/// Which row is selected and which rows are expanded. The action list itself is
/// [`prism_engine::Session::actions`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionsUi {
    pub selected: Option<usize>,
    pub selected_step: Option<usize>,
    pub expanded: Vec<bool>,
    pub collapsed_sets: Vec<String>,
    #[serde(skip)]
    reveal_recording: Option<(usize, usize)>,
}

fn label_of(id: &str) -> String {
    prism_engine::commands::find(id)
        .map(|c| c.label)
        .or_else(|| crate::menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1))
        .map(|label| crate::i18n::tr_id(crate::i18n::current(), id, label).trim_end_matches('…').to_string())
        .unwrap_or_else(|| id.to_string())
}

/// The selected action, or the first one when nothing is selected.
pub fn selected_action(app: &PrismApp) -> Option<&Action> {
    let list = &app.session.actions.list;
    let idx = app.ui.actions.selected.filter(|i| *i < list.len());
    idx.and_then(|i| list.get(i)).or_else(|| list.first())
}

/// Steps in the `[[id, params], …]` shape batch and droplets already accept.
pub fn action_steps(action: &Action) -> Vec<Value> {
    prism_engine::action_model::batch_steps(action)
}

/// Named action bindings use the existing persisted shortcut overrides. Keeping the
/// name in the key means selecting or deleting another row cannot retarget F6.
pub(crate) const SHORTCUT_PREFIX: &str = "actions.play:";

pub(crate) fn shortcut_id(name: &str) -> String {
    format!("{SHORTCUT_PREFIX}{name}")
}

pub(crate) fn assign_shortcut(app: &mut PrismApp, name: &str, shortcut: &str) -> Result<Value, String> {
    let id = shortcut_id(name);
    let mut set = serde_json::Map::new();
    // A function key belongs to only one action. Ordinary menu bindings remain
    // intact and become available again when the action binding is removed.
    for (other, assigned) in &app.session.prefs().shortcuts {
        if other.starts_with(SHORTCUT_PREFIX) && other != &id && assigned == shortcut && !shortcut.is_empty() {
            set.insert(other.clone(), json!(""));
        }
    }
    set.insert(id, json!(shortcut));
    app.run("edit.keyboardShortcuts", json!({"set": set, "allowUnknown": true, "removeConflicts": false}))
}

/// Replay edits synchronously, with view steps handled by the shell in the same order.
/// Engine steps keep the headless action semantics (no dialogs or background jobs).
impl prism_engine::action_playback::Host for PrismApp {
    fn session(&mut self) -> &mut prism_engine::Session {
        &mut self.session
    }
    fn command(&mut self, id: &str, p: Value) -> prism_engine::Result<Value> {
        if actions_cmds::shell_view_command(id) {
            self.sync_views();
            self.run(id, p).map_err(prism_engine::EngineError::Other)
        } else {
            self.session.execute(id, p)
        }
    }
}
pub(crate) fn play(app: &mut PrismApp, params: &Value) -> Result<Value, String> {
    let value = prism_engine::action_playback::run(app, params, false).map_err(|e| e.to_string())?;
    show_pause(app, &value);
    Ok(value)
}
pub(crate) fn resume(app: &mut PrismApp, params: &Value) -> Result<Value, String> {
    let value = prism_engine::action_playback::run(app, params, true).map_err(|e| e.to_string())?;
    show_pause(app, &value);
    Ok(value)
}

/// Execute the actual menu command, not a synthetic keystroke. Fit is deferred
/// until canvas layout, so it uses the dimensions after all preceding action steps.
pub(crate) fn run_view(app: &mut PrismApp, id: &str, params: Value) -> Result<Value, String> {
    app.sync_views();
    let i = app.session.active_index().ok_or("no document")?;
    let v = app.view_mut(i).ok_or("no document view")?;
    v.fill_pending = false;
    match id {
        "view.fitOnScreen" => v.fit_pending = true,
        "view.zoomIn" => {
            v.fit_pending = false;
            v.zoom = crate::canvas::zoom_step(v.zoom, 1);
        }
        "view.zoomOut" => {
            v.fit_pending = false;
            v.zoom = crate::canvas::zoom_step(v.zoom, -1);
        }
        "view.actualPixels" => {
            v.fit_pending = false;
            v.zoom = 1.0;
        }
        _ => return Err(format!("unsupported view command: {id}")),
    }
    app.session.journal.push((id.into(), params));
    Ok(Value::Null)
}

fn begin_recording(app: &mut PrismApp, append: bool) {
    if app.session.actions.recording.is_some() {
        return;
    }
    let params = if append { app.ui.actions.selected.map_or(json!({}), |i| json!({"action": i})) } else { json!({}) };
    let Ok(v) = app.run("actions.record", params) else { return };
    let Some(i) = v.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()) else { return };
    app.ui.actions.selected = Some(i);
    app.ui.actions.selected_step = None;
    if app.ui.actions.expanded.len() <= i {
        app.ui.actions.expanded.resize(i + 1, false);
    }
    app.ui.actions.expanded[i] = true;
}

pub(crate) fn report_play(app: &mut PrismApp, v: &Value) {
    if v.get("paused").is_some() {
        app.ui.status = tl!("Action paused").into();
        app.ui.status_error = false;
        return;
    }
    let Some(failed) = v.get("failed").filter(|f| f.is_object()) else {
        let ran = v.get("ran").and_then(Value::as_u64).unwrap_or(0);
        app.ui.status = format!("Played {ran} steps");
        app.ui.status_error = false;
        return;
    };
    let step = failed.get("step").and_then(Value::as_u64).unwrap_or(0);
    let id = failed.get("id").and_then(Value::as_str).unwrap_or("");
    let error = failed.get("error").and_then(Value::as_str).unwrap_or("");
    app.ui.status = format!("Step {} ({id}) failed: {error}", step + 1);
    app.ui.status_error = true;
}

fn visible_steps(app: &PrismApp, action: usize) -> Vec<ActionStep> {
    app.session
        .actions
        .list
        .get(action)
        .into_iter()
        .flat_map(|a| a.steps.iter().cloned())
        .chain(actions_cmds::pending_steps(&app.session, action).cloned().map(ActionStep::from))
        .collect()
}

fn step_label(id: &str, params: &Value) -> String {
    if id == "actions.unsupportedAtnStep" {
        let event = params.get("event").and_then(Value::as_str).unwrap_or("");
        let name = params.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()).unwrap_or(event);
        if name == event || event.is_empty() {
            format!("{}: {name}", tl!("Unsupported Photoshop step"))
        } else {
            format!("{}: {name} ({event})", tl!("Unsupported Photoshop step"))
        }
    } else if id == "actions.play" {
        let target = params.get("action").map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        format!("{}: {target}", label_of(id))
    } else {
        label_of(id)
    }
}

fn unsupported_step_tooltip(id: &str, params: &Value) -> Option<String> {
    (id == "actions.unsupportedAtnStep").then(|| {
        let reason = params.get("reason").and_then(Value::as_str).unwrap_or("");
        format!("{}\n{reason}\n{}", step_label(id, params), tl!("This enabled step prevents action playback."))
    })
}

#[derive(Clone, Copy)]
struct DragRow {
    action: usize,
    step: Option<usize>,
}

/// Drops use an insertion line, converted to the final index expected by actions.move.
fn drag_row(ui: &egui::Ui, response: &egui::Response, row: DragRow, t: &Tokens) -> Option<(DragRow, usize)> {
    response.dnd_set_drag_payload(row);
    let source = response.dnd_hover_payload::<DragRow>()?;
    if source.step.is_some() != row.step.is_some() || (source.step.is_some() && source.action != row.action) {
        return None;
    }
    let after = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y >= response.rect.center().y));
    let index = row.step.unwrap_or(row.action);
    let from = source.step.unwrap_or(source.action);
    let slot = index.saturating_add(usize::from(after));
    let to = if from < slot { slot.saturating_sub(1) } else { slot };
    let y = if after { response.rect.bottom() } else { response.rect.top() };
    ui.painter().line_segment([pos2(response.rect.left() + 22.0, y), pos2(response.rect.right(), y)], Stroke::new(2.0, t.accent));
    response.dnd_release_payload::<DragRow>().map(|source| (*source, to))
}

fn move_row(app: &mut PrismApp, source: DragRow, to: usize) {
    let mut params = json!({"action": source.action, "to": to});
    if let Some(step) = source.step {
        params["step"] = json!(step);
    } else if let Some(target) = app.session.actions.list.get(to) {
        params["set"] = json!(target.set);
    }
    if app.run("actions.move", params).is_err() {
        return;
    }
    if source.step.is_some() {
        app.ui.actions.selected = Some(source.action);
        app.ui.actions.selected_step = Some(to);
    } else {
        app.ui.actions.expanded.resize(app.session.actions.list.len(), false);
        if source.action < app.ui.actions.expanded.len() && to < app.ui.actions.expanded.len() {
            let expanded = app.ui.actions.expanded.remove(source.action);
            app.ui.actions.expanded.insert(to, expanded);
        }
        app.ui.actions.selected = Some(to);
        app.ui.actions.selected_step = None;
    }
}

fn delete_selection(app: &mut PrismApp) {
    let Some(action) = app.ui.actions.selected else { return };
    let step = app.ui.actions.selected_step;
    let mut params = json!({"action": action});
    if let Some(step) = step {
        params["step"] = json!(step);
    }
    if app.run("actions.delete", params).is_err() {
        return;
    }
    // Clear selection after deletion so a second click cannot delete a different
    // step or unexpectedly fall back to deleting the entire action.
    app.ui.actions.selected = None;
    app.ui.actions.selected_step = None;
    if step.is_none() && action < app.ui.actions.expanded.len() {
        app.ui.actions.expanded.remove(action);
    }
}

pub fn panel(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let recording = app.session.actions.recording;
    let rows: Vec<_> = app.session.actions.list.iter().enumerate().map(|(i, a)| (a.name.clone(), visible_steps(app, i))).collect();
    let sets: Vec<_> = app.session.actions.list.iter().map(|a| a.set.clone()).collect();
    if app
        .ui
        .actions
        .selected
        .is_some_and(|i| i >= rows.len() || app.ui.actions.selected_step.is_some_and(|step| rows.get(i).is_none_or(|row| step >= row.1.len())))
    {
        app.ui.actions.selected = None;
        app.ui.actions.selected_step = None;
    }
    if app.ui.actions.expanded.len() > rows.len() {
        app.ui.actions.expanded.truncate(rows.len());
    }
    // The dock bounds this panel. Scroll only the rows, keeping transport controls visible.
    let footer = 32.0 + ui.spacing().item_spacing.y;
    let max_h = (ui.available_height() - footer).max(0.0);
    let mut play_idx = None;
    let mut moved = None;
    let recording_row = recording.and_then(|(_, i)| rows.get(i).map(|(_, steps)| (i, steps.len())));
    let reveal = recording_row.filter(|row| app.ui.actions.reveal_recording != Some(*row));
    app.ui.actions.reveal_recording = recording_row;
    if let Some((i, _)) = reveal {
        app.ui.actions.expanded.resize(rows.len(), false);
        if let Some(expanded) = app.ui.actions.expanded.get_mut(i) {
            *expanded = true;
        }
    }
    egui::ScrollArea::vertical().id_salt("actions-rows").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        if rows.is_empty() {
            ui.label(egui::RichText::new(tl!("Record ● a sequence of edits, then play ▶ it on any document.")).color(t.text_faint).size(11.5));
        }
        if egui::DragAndDrop::has_payload_of_type::<DragRow>(ui.ctx()) {
            let clip = ui.clip_rect();
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|p| clip.contains(*p)) {
                let delta = if p.y < clip.top() + 22.0 {
                    8.0
                } else if p.y > clip.bottom() - 22.0 {
                    -8.0
                } else {
                    0.0
                };
                if delta != 0.0 {
                    ui.scroll_with_delta(vec2(0.0, delta));
                    ui.ctx().request_repaint();
                }
            }
        }
        let mut groups = Vec::<String>::new();
        for set in &sets {
            if !groups.contains(set) {
                groups.push(set.clone());
            }
        }
        for set in groups {
            let closed = app.ui.actions.collapsed_sets.contains(&set);
            let heading = if set == "Default Actions" { tl!("Default Actions").to_string() } else { set.clone() };
            let header = ui.selectable_label(false, format!("     {heading}"));
            let arrow = Rect::from_center_size(pos2(header.rect.left() + 9.0, header.rect.center().y), vec2(7.0, 7.0));
            let points = if closed {
                vec![arrow.left_top(), arrow.right_center(), arrow.left_bottom()]
            } else {
                vec![arrow.left_top(), arrow.right_top(), arrow.center_bottom()]
            };
            ui.painter().add(egui::Shape::convex_polygon(points, t.text_dim, Stroke::NONE));
            if header.clicked() {
                if closed {
                    app.ui.actions.collapsed_sets.retain(|s| s != &set);
                } else {
                    app.ui.actions.collapsed_sets.push(set.clone());
                }
            }
            if closed && recording.is_none_or(|(_, i)| sets.get(i) != Some(&set)) {
                continue;
            }
            for (i, (name, steps)) in rows.iter().enumerate().filter(|(i, _)| sets.get(*i) == Some(&set)) {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
                if reveal == Some((i, 0)) {
                    ui.scroll_to_rect(rect, Some(egui::Align::BOTTOM));
                }
                let sel = app.ui.actions.selected == Some(i) && app.ui.actions.selected_step.is_none();
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, sel, name));
                if sel {
                    ui.painter().rect_filled(rect, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
                }
                if let Some(change) = drag_row(ui, &resp, DragRow { action: i, step: None }, &t) {
                    moved = Some(change);
                }
                let expanded = app.ui.actions.expanded.get(i).copied().unwrap_or(false);
                let tri = Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(8.0, 8.0));
                let pts = if expanded {
                    vec![tri.left_top(), tri.right_top(), tri.center_bottom()]
                } else {
                    vec![tri.left_top(), tri.right_center(), tri.left_bottom()]
                };
                ui.painter().add(egui::Shape::convex_polygon(pts, t.text_dim, Stroke::NONE));
                crate::icons::paint(ui, Rect::from_center_size(pos2(rect.left() + 30.0, rect.center().y), vec2(16.0, 16.0)), "play", 11.0, t.icon);
                let recording_this = recording.is_some_and(|(_, r)| r == i);
                let nsteps = steps.len();
                ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), t.text);
                ui.painter().text(
                    pos2(rect.right() - 8.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    app.session
                        .prefs()
                        .shortcuts
                        .get(&shortcut_id(name))
                        .filter(|s| !s.is_empty())
                        .map_or_else(|| format!("{nsteps} steps"), |key| format!("{key} · {nsteps} steps")),
                    egui::FontId::proportional(11.0),
                    t.text_faint,
                );
                if recording_this {
                    ui.painter().circle_filled(pos2(rect.right() - 64.0, rect.center().y), 4.0, Color32::from_rgb(230, 60, 60));
                }
                if resp.clicked() {
                    app.ui.actions.selected = Some(i);
                    app.ui.actions.selected_step = None;
                    if resp.interact_pointer_pos().is_some_and(|p| p.x < rect.left() + 20.0) {
                        if app.ui.actions.expanded.len() <= i {
                            app.ui.actions.expanded.resize(i + 1, false);
                        }
                        app.ui.actions.expanded[i] = !expanded;
                    }
                }
                resp.context_menu(|ui| {
                    if ui.button(tl!("Action Set…")).clicked() {
                        form(app, "actions.configure", tl!("Action Set…"), json!({"set":set,"__action":i}));
                        ui.close();
                    }
                    if ui.button(tl!("Insert Stop…")).clicked() {
                        form(app, "actions.insertStop", tl!("Insert Stop…"), json!({"message":"","allowContinue":false,"__action":i}));
                        ui.close();
                    }
                    if ui.button(tl!("Duplicate Action…")).clicked() {
                        form(app, "actions.duplicate", tl!("Duplicate Action…"), json!({"name":format!("{name} copy"),"set":set,"__action":i}));
                        ui.close();
                    }
                    ui.separator();
                    ui.label(tl!("Keyboard Shortcuts"));
                    let current = app.session.prefs().shortcuts.get(&shortcut_id(name)).cloned().unwrap_or_default();
                    for key in std::iter::once(String::new()).chain((1..=12).map(|n| format!("F{n}"))) {
                        let label = if key.is_empty() { tl!("None").to_string() } else { key.clone() };
                        if ui.selectable_label(current == key, label).clicked() {
                            let _ = assign_shortcut(app, name, &key);
                            ui.close();
                        }
                    }
                });
                if resp.double_clicked() {
                    play_idx = Some(i);
                }
                if expanded {
                    for (step, item) in steps.iter().enumerate() {
                        let (id, params) = (&item.id, &item.params);
                        let (r, response) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click_and_drag());
                        let unsupported = unsupported_step_tooltip(id, params);
                        let response = if let Some(tooltip) = &unsupported { response.on_hover_text(tooltip) } else { response };
                        if reveal == Some((i, step + 1)) {
                            ui.scroll_to_rect(r, Some(egui::Align::BOTTOM));
                        }
                        let label = step_label(id, params);
                        let selected = app.ui.actions.selected == Some(i) && app.ui.actions.selected_step == Some(step);
                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &label));
                        if selected {
                            ui.painter().rect_filled(r, 0.0, t.row_selected);
                        } else if response.hovered() {
                            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
                        }
                        if let Some(change) = drag_row(ui, &response, DragRow { action: i, step: Some(step) }, &t) {
                            moved = Some(change);
                        }
                        let mut enabled = item.enabled;
                        let mut dialog = item.dialog;
                        if ui
                            .put(Rect::from_min_size(pos2(r.left() + 3.0, r.top()), vec2(19.0, 20.0)), egui::Checkbox::without_text(&mut enabled))
                            .on_hover_text(tl!("Enable action step"))
                            .changed()
                        {
                            let _ = app.run("actions.configure", json!({"action":i,"step":step,"enabled":enabled}));
                        }
                        if ui
                            .put(Rect::from_min_size(pos2(r.left() + 25.0, r.top()), vec2(19.0, 20.0)), egui::Checkbox::without_text(&mut dialog))
                            .on_hover_text(tl!("Show settings during playback"))
                            .changed()
                        {
                            let _ = app.run("actions.configure", json!({"action":i,"step":step,"dialog":dialog}));
                        }
                        ui.painter().text(
                            pos2(r.left() + 54.0, r.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(11.5),
                            if !enabled {
                                t.text_faint
                            } else if unsupported.is_some() {
                                t.warning
                            } else {
                                t.text_dim
                            },
                        );
                        if response.double_clicked() {
                            let mut fields = Value::Object(params.as_object().cloned().unwrap_or_default());
                            fields["__action"] = json!(i);
                            fields["__step"] = json!(step);
                            fields["__actionParams"] = json!(true);
                            form(app, "actions.configure", &label_of(id), fields);
                        }
                        response.context_menu(|ui| {
                            if ui.button(tl!("Duplicate Step")).clicked() {
                                let _ = app.run("actions.duplicate", json!({"action":i,"step":step}));
                                ui.close();
                            }
                            if ui.button(tl!("Insert Stop…")).clicked() {
                                form(app, "actions.insertStop", tl!("Insert Stop…"), json!({"message":"","allowContinue":false,"after":step,"__action":i}));
                                ui.close();
                            }
                        });
                        if response.clicked() {
                            app.ui.actions.selected = Some(i);
                            app.ui.actions.selected_step = Some(step);
                        }
                    }
                }
            }
        }
    });
    if let Some((source, to)) = moved {
        move_row(app, source, to);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let recording_now = app.session.actions.recording.is_some();
        if crate::icons::button(ui, "square", 24.0, false, tl!("Stop playing/recording")).clicked() {
            if recording_now {
                let _ = app.run("actions.stop", json!({}));
            } else if app.session.actions.paused.is_some() {
                let _ = app.run("actions.cancel", json!({}));
            }
        }
        let (r, rec) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
        if rec.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        ui.painter().circle_filled(r.center(), 5.5, if recording_now { Color32::from_rgb(230, 60, 60) } else { t.icon });
        if rec.on_hover_text(tl!("Begin recording")).clicked() && !recording_now {
            begin_recording(app, true);
        }
        if crate::icons::button(ui, "play", 24.0, false, tl!("Play selection")).clicked() {
            if app.session.actions.paused.is_some() {
                if let Ok(v) = app.run("actions.resume", json!({})) {
                    report_play(app, &v);
                }
            } else {
                play_idx = app.ui.actions.selected;
            }
        }
        if crate::icons::button(ui, "plus", 24.0, false, tl!("Create new action")).clicked() && !recording_now {
            let set = selected_action(app).map(|a| a.set.clone()).unwrap_or_else(prism_engine::action_model::default_set);
            form(app, "actions.record", tl!("Create new action"), json!({"name":format!("Action {}", rows.len()+1),"set":set}));
        }
        let can_delete = app.ui.actions.selected.is_some() && (!recording_now || app.ui.actions.selected_step.is_some());
        ui.add_enabled_ui(can_delete, |ui| {
            let response = crate::icons::button(ui, "trash", 24.0, false, tl!("Delete"));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, can_delete, tl!("Delete")));
            if response.clicked() {
                delete_selection(app);
            }
        });
        if let Some(paused) = app.session.actions.paused.as_ref() {
            // Play resumes and Stop cancels; long messages belong in the notice/tooltip.
            ui.label(tl!("Action paused")).on_hover_text(paused.reason.get("message").and_then(Value::as_str).unwrap_or(""));
        }
    });
    if let Some(i) = play_idx
        && let Ok(v) = app.run("actions.play", json!({"action": i,"from":app.ui.actions.selected_step.unwrap_or(0)}))
    {
        report_play(app, &v);
    }
}

fn form(app: &mut PrismApp, command: &str, title: &str, fields: Value) {
    let mut fields = fields.as_object().cloned().unwrap_or_default();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(title));
    fields.insert("__form".into(), json!(true));
    fields.insert("__actions".into(), json!(true));
    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}
fn show_pause(app: &mut PrismApp, value: &Value) {
    let Some(p) = value.get("paused") else { return };
    if p["kind"] == "dialog" {
        form(app, "actions.resume", &label_of(p["id"].as_str().unwrap_or("")), p["params"].clone());
    } else {
        form(app, "actions.resume", tl!("Stop"), json!({"__actionStop":true,"__message":p["message"],"__allowContinue":p["allowContinue"]}));
    }
}
pub(crate) fn body(ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>) {
    if fields.contains_key("__actionStop") {
        ui.label(fields.get("__message").and_then(Value::as_str).unwrap_or(""));
    } else {
        crate::view_cmds::form_body(ui, fields);
        if fields.iter().any(|(k, v)| !k.starts_with("__") && (v.is_array() || v.is_object())) {
            ui.label(tl!("Additional settings retain their recorded values."));
        }
    }
}
pub(crate) fn confirm(app: &mut PrismApp, fields: &serde_json::Map<String, Value>) -> Result<Value, String> {
    if fields.contains_key("__actionStop") && fields.get("__allowContinue").and_then(Value::as_bool) != Some(true) {
        return Ok(json!({"paused":true}));
    }
    let command = fields.get("__command").and_then(Value::as_str).ok_or("missing action command")?;
    let fields_value = crate::filter_dialog::params_of(fields);
    let mut params = if command == "actions.resume" || fields.contains_key("__actionParams") { json!({"params":fields_value}) } else { fields_value };
    if let Some(action) = fields.get("__action") {
        params["action"] = action.clone();
    }
    if let Some(step) = fields.get("__step") {
        params["step"] = step.clone();
    }
    let value = app.run(command, params)?;
    if command == "actions.record" {
        app.ui.actions.selected = value.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
        app.ui.actions.selected_step = None;
    }
    if command == "actions.resume" {
        report_play(app, &value);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_atn_rows_preserve_original_identity_and_explain_playback_failure() {
        use egui_kittest::{Harness, kittest::Queryable};
        let params = json!({"event":"vendorPlugin", "name":"Original Plugin", "reason":"Plugin command unavailable", "descriptor":{}});
        let label = step_label("actions.unsupportedAtnStep", &params);
        assert!(label.contains("Original Plugin") && label.contains("vendorPlugin"));
        let tooltip = unsupported_step_tooltip("actions.unsupportedAtnStep", &params).unwrap();
        assert!(tooltip.contains("Plugin command unavailable"));
        assert!(unsupported_step_tooltip("view.zoomIn", &json!({})).is_none());
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action::new("Imported", vec![("actions.unsupportedAtnStep".into(), params)]));
        app.ui.actions.expanded = vec![true];
        let mut h = Harness::builder().with_size(vec2(600.0, 220.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        h.get_by_label(&label);
        assert!(h.state().session.actions.list[0].steps[0].enabled, "showing an unsupported step never disables it");
    }

    #[test]
    fn modal_playback_uses_native_dialog_and_resumes_with_edited_settings() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4})).unwrap();
        app.session.actions.list.push(Action::new("Settings", vec![("layer.new.layer".into(), json!({"name":"recorded"}))]));
        app.run("actions.configure", json!({"action":0,"step":0,"dialog":true})).unwrap();
        let value = app.run("actions.play", json!({"action":0})).unwrap();
        assert_eq!(value["paused"]["kind"], "dialog");
        let dialog = app.ui.dialogs.last().unwrap().id;
        app.ui.dialog_mut(dialog).unwrap().fields.insert("name".into(), json!("edited once"));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert!(app.ui.dialogs.is_empty());
        assert!(app.session.actions.paused.is_none());
        assert_eq!(app.session.active().unwrap().doc.layers[1].name, "edited once");
        assert_eq!(app.session.actions.list[0].steps[0].params["name"], "recorded");
    }

    #[test]
    fn stop_notice_continue_policy_does_not_restart_prior_steps() {
        for allow in [false, true] {
            let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width":4,"height":4})).unwrap();
            app.session.actions.list.push(Action::new("Pause", vec![("layer.new.layer".into(), json!({}))]));
            app.run("actions.insertStop", json!({"action":0,"message":"Inspect","allowContinue":allow})).unwrap();
            app.run("actions.play", json!({"action":0})).unwrap();
            let dialog = app.ui.dialogs.last().unwrap().id;
            crate::dialogs::confirm(&mut app, dialog).unwrap();
            assert_eq!(app.session.actions.paused.is_some(), !allow);
            if !allow {
                app.run("actions.resume", json!({})).unwrap();
            }
            assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
        }
    }

    #[test]
    fn small_panel_keeps_transport_visible_with_long_rows_and_pause_message() {
        use egui_kittest::{Harness, kittest::Queryable};
        assert!(crate::dock_model::PanelId::Actions.fills());
        for paused in [false, true] {
            let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
            app.session.actions.list.push(Action::new("Long", vec![("view.zoomOut".into(), json!({})); 30]));
            app.ui.actions.selected = Some(0);
            app.ui.actions.expanded = vec![true];
            if paused {
                app.run("actions.insertStop", json!({"action":0,"message":"Inspect ".repeat(200)})).unwrap();
                app.run("actions.move", json!({"action":0,"step":30,"to":0})).unwrap();
                app.run("actions.play", json!({"action":0})).unwrap();
            }
            let mut h = Harness::builder().with_size(vec2(280.0, 120.0)).build_ui_state(|ui, app| panel(app, ui), app);
            h.run_steps(4);
            let footer = h.get_by_label("Delete").rect();
            assert!(footer.top() >= 0.0 && footer.bottom() <= 120.0, "transport visible: {footer:?}");
            if paused {
                assert!(h.get_by_label("Action paused").rect().bottom() <= 120.0);
            }
        }
    }

    #[test]
    fn recording_reveals_new_action_and_latest_step_in_a_long_panel() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4})).unwrap();
        for i in 0..30 {
            app.session.actions.list.push(Action::new(format!("Existing {i}"), vec![]));
        }
        let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(4);
        begin_recording(h.state_mut(), false);
        h.run_steps(20);
        let rect = h.get_by_label("Action 31").rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= h.get_by_label("Delete").rect().top(), "new action visible: {rect:?}");
        for _ in 0..20 {
            h.state_mut().run("view.zoomOut", json!({})).unwrap();
        }
        h.state_mut().run("view.fitOnScreen", json!({})).unwrap();
        h.run_steps(20);
        let rect = h.get_by_label("Fit on Screen").rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= h.get_by_label("Delete").rect().top(), "latest step visible: {rect:?}");
    }

    #[test]
    fn drag_steps_and_action_headers_reorders_and_keeps_selection() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action::new(
            "First",
            vec![("view.zoomOut".into(), json!({})), ("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
        ));
        app.session.actions.list.push(Action::new("Second", vec![]));
        app.ui.actions.expanded = vec![true, false];
        let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(4);
        fn drag(h: &mut Harness<'_, PrismApp>, from: egui::Pos2, to: egui::Pos2) {
            h.event(egui::Event::PointerMoved(from));
            h.run_steps(1);
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
            h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
            h.run_steps(2);
            h.event(egui::Event::PointerMoved(to));
            h.run_steps(2);
            h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(3);
        }
        let from = h.get_by_label("Zoom Out").rect().center();
        let target = h.get_by_label("100%").rect();
        drag(&mut h, from, pos2(target.center().x, target.bottom() - 2.0));
        assert_eq!(
            h.state().session.actions.list[0].steps.iter().map(|st| st.id.as_str()).collect::<Vec<_>>(),
            ["view.fitOnScreen", "view.actualPixels", "view.zoomOut"]
        );
        assert_eq!(h.state().ui.actions.selected_step, Some(2));
        let from = h.get_by_label("Second").rect().center();
        let target = h.get_by_label("First").rect();
        drag(&mut h, from, pos2(target.center().x, target.top() + 2.0));
        assert_eq!(h.state().session.actions.list[0].name, "Second");
        assert_eq!(h.state().ui.actions.expanded, [false, true]);
        assert_eq!(h.state().ui.actions.selected, Some(0));
    }

    #[test]
    fn nested_shell_actions_include_view_steps_and_propagate_failures() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":40,"height":40})).unwrap();
        app.session.actions.list.push(Action::new("Child", vec![("layer.new.layer".into(), json!({})), ("view.fitOnScreen".into(), json!({}))]));
        app.run("actions.record", json!({"name":"Parent"})).unwrap();
        app.run("actions.play", json!({"action":"Child"})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, [("actions.play".into(), json!({"action":"Child"})).into()]);
        assert_eq!(step_label("actions.play", &json!({"action":"Child"})), "Play Action: Child");
        app.run("view.actualPixels", json!({})).unwrap();
        let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        assert!(app.ui.views[0].fit_pending);
        app.session.authorize =
            Some(|id, _| if id == "view.fitOnScreen" { Err(prism_engine::EngineError::Other("denied nested view".into())) } else { Ok(()) });
        let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        assert!(r["failed"]["error"].as_str().unwrap().contains("denied nested view"));
        assert!(app.session.actions.playback_stack.is_empty());
        assert_eq!(app.session.actions.playing, 0);
    }

    #[test]
    fn panel_shows_pending_steps_and_trash_deletes_only_the_clicked_step() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        begin_recording(&mut app, false);
        app.run("view.fitOnScreen", json!({})).unwrap();
        app.run("view.zoomOut", json!({})).unwrap();
        let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(2);
        assert!(h.state().session.actions.recording.is_some());
        h.get_by_label("Fit on Screen");
        h.get_by_label("Zoom Out").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.actions.selected_step, Some(1));
        h.get_by_label("Delete").click();
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen"]);
        assert_eq!(h.state().session.actions.list.len(), 1);
        assert!(h.state().session.actions.recording.is_some());
        assert!(h.state().ui.actions.selected.is_none());
        delete_selection(h.state_mut());
        assert_eq!(h.state().session.actions.list.len(), 1, "repeated trash cannot delete the action");
        h.state_mut().run("view.zoomIn", json!({})).unwrap();
        h.run_steps(2);
        h.get_by_label("Zoom In");
        h.state_mut().run("actions.stop", json!({})).unwrap();
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen", "view.zoomIn"]);
        h.get_by_label("Zoom In").click();
        h.run_steps(2);
        h.get_by_label("Delete").click();
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen"]);
    }

    #[test]
    fn view_menu_commands_record_append_and_replay_after_resize() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 160, "height": 100})).unwrap();
        begin_recording(&mut app, false);
        app.run("image.canvasSize", json!({"width": 80, "height": 60})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        begin_recording(&mut app, true);
        crate::menus::invoke(&mut app, &ctx, "view.fitOnScreen", json!({})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list.len(), 1);
        assert_eq!(app.session.actions.list[0].steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["image.canvasSize", "view.fitOnScreen"]);
        assert_eq!(label_of("view.fitOnScreen"), "Fit on Screen");
        // Persisted actions retain the menu step, and playback fits the new document.
        let saved = serde_json::to_string(&app.session.actions.list).unwrap();
        app.session.actions.list = serde_json::from_str(&saved).unwrap();
        app.run("file.new", json!({"width": 200, "height": 140})).unwrap();
        app.run("view.actualPixels", json!({})).unwrap();
        let result = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(result["ran"], 2);
        assert!(result.get("failed").is_none(), "{result}");
        let idx = app.session.active_index().unwrap();
        assert!(app.ui.views[idx].fit_pending);
        let doc = &app.session.active().unwrap().doc;
        crate::canvas::fit_view(&mut app.ui.views[idx], doc, vec2(100.0, 100.0));
        assert_eq!(app.ui.views[idx].zoom, 0.75);
        assert_eq!(app.ui.views[idx].center, [40.0, 30.0]);
        assert!(!app.ui.views[idx].fit_pending);
    }

    #[test]
    fn view_action_failure_resume_and_authorization() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action::new("Fit", vec![("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))]));
        let result = app.run("actions.play", json!({"action": "Fit"})).unwrap();
        assert_eq!(result["ran"], 0);
        assert_eq!(result["failed"]["step"], 0);
        assert_eq!(app.session.actions.playing, 0);
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.session.authorize =
            Some(
                |id, _| {
                    if id == "view.fitOnScreen" { Err(prism_engine::EngineError::BadParams { cmd: id.into(), msg: "denied view".into() }) } else { Ok(()) }
                },
            );
        let result = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(result["ran"], 0);
        assert!(result["failed"]["error"].as_str().unwrap().contains("denied view"));
        let result = app.run("actions.play", json!({"action": 0, "from": 1})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(!app.ui.views[0].fit_pending);
        assert!(app.run("actions.play", json!({"action": 0, "from": -1})).is_err());
        app.session.actions.playing = 1;
        assert!(app.run("actions.play", json!({"action": 0})).is_err());
    }

    #[test]
    fn record_and_replay_on_another_document() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.sync_views();
        app.run("actions.record", json!({})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let _ = app.run("document.pixel", json!({"x": 1, "y": 1}));
        app.run("actions.stop", json!({})).unwrap();
        let a = &app.session.actions.list[0];
        assert_eq!(a.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["layer.new.layer", "select.rect", "edit.fill"]);
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.sync_views();
        let played = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(played["ran"], 3);
        assert!(played.get("failed").is_none(), "{played}");
        let d = &app.session.active().unwrap().doc;
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[1].surface().unwrap().pixel(5, 5), vec![1.0, 0.0, 0.0, 1.0]);
    }
}
