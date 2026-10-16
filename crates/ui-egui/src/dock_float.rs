//! Extra dock columns, floating panel windows and columns collapsed to icons, as in Photoshop.
//!
//! - An extra column (`dock_model::Column`) sits at the left of the work area (inside the
//!   Tools panel), at the right (inside the main dock) or at the bottom (a row whose groups sit
//!   side by side). Its edge resizes it; the bar on top holds « / », which collapses it to icons.
//! - A floating window (`dock_model::Float`) has a small title bar (drag it to move the window
//!   or dock it; « / » collapses it to icons; × closes its panels) and a grip at the bottom that
//!   resizes it (its right edge resizes the width).
//! - A column or window collapsed to icons lists its panels as an icon and a name per tab, the
//!   groups apart; a click opens the panel's group in a flyout beside it.

use egui::{Rect, Sense, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::PrismApp;
use crate::dock::{COLUMN_HEADER, Group, StackOpts, show_stack};
use crate::dock_drag;
use crate::dock_model::{self, DropTarget, FLOAT_H, FLOAT_W, Payload, PanelId, Side, Slot};
use crate::theme::{self, Tokens};

/// Width of a column or window collapsed to icons (icon and name).
pub const ICONIC_W: f32 = 132.0;
/// Height of a floating window's title bar.
pub const TITLE_H: f32 = 14.0;
/// Height of a floating window's bottom resize grip.
pub const GRIP_H: f32 = 8.0;
/// Height of one panel row in an icon list.
const ROW_H: f32 = 26.0;

fn body(app: &mut PrismApp, ui: &mut egui::Ui, panel: PanelId) {
    crate::panels::panel_body(app, ui, panel);
}

fn sane(v: f32, range: std::ops::RangeInclusive<f32>, fallback: f32) -> f32 {
    if v.is_finite() { v.clamp(*range.start(), *range.end()) } else { fallback }
}

/// The bar on top of a dock column with its « / » button. `toward_left`: the column grows
/// toward the left when expanded (a right-hand column). Returns whether the button was clicked.
pub fn header(ui: &mut egui::Ui, rect: Rect, iconic: bool, toward_left: bool, id: impl std::hash::Hash + std::fmt::Debug) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, t.tab_strip.gamma_multiply(0.7));
    let b = Rect::from_min_size(pos2(rect.right() - 18.0, rect.top()), vec2(16.0, rect.height()));
    let b = if toward_left { b } else { Rect::from_min_size(rect.min + vec2(2.0, 0.0), b.size()) };
    let resp = ui.interact(b, egui::Id::new(("dock-column-iconic", id)), Sense::click());
    let label = if iconic { tl!("Expand Panels") } else { tl!("Collapse to Icons") };
    let resp = resp.on_hover_text(label);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    // « expands a right-hand column toward the canvas; » collapses it.
    let icon = if iconic == toward_left { "chevrons-left" } else { "chevrons-right" };
    crate::icons::paint(ui, b, icon, 10.0, if resp.hovered() { t.text } else { t.text_faint });
    resp.clicked()
}

/// A panel's group opened from an icon list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flyout {
    pub group: Group,
    /// The icon row it opened from (screen points).
    pub anchor: Rect,
    /// Open to the left of the anchor.
    pub to_left: bool,
}

fn flyout_id() -> egui::Id {
    egui::Id::new("dock-flyout")
}

/// The flyout open now, if any.
pub fn open_flyout(ctx: &egui::Context) -> Option<Flyout> {
    ctx.data(|d| d.get_temp::<Flyout>(flyout_id()))
}

pub fn close_flyout(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Flyout>(flyout_id()));
}

/// Rects of the icon rows drawn last frame: `(group, panel, rect)`.
pub fn last_icons(ctx: &egui::Context) -> Vec<(Group, PanelId, Rect)> {
    dock_drag::frame_items(ctx, icons_id())
}

fn icons_id() -> egui::Id {
    egui::Id::new("dock-icon-rows")
}

/// Height an icon list of `groups` needs.
pub fn iconic_height(app: &PrismApp, groups: &[Group], pro: bool) -> f32 {
    let rows: usize = groups.iter().map(|g| app.ui.dock.tabs_of(*g, pro).len()).sum();
    rows as f32 * (ROW_H + 2.0) + groups.len().saturating_sub(1) as f32 * 7.0 + 4.0
}

/// Draw `groups` as an icon list in `area`. `to_left`: flyouts open to the left.
pub fn iconic_list(app: &mut PrismApp, ui: &mut egui::Ui, area: Rect, groups: &[Group], to_left: bool, float: Option<u32>) {
    let t = Tokens::get(ui.ctx());
    let pro = t.pro;
    let open = open_flyout(ui.ctx());
    let locked = app.session.prefs().workspace_locked;
    let mut y = area.top() + 2.0;
    for (gi, g) in groups.iter().copied().enumerate() {
        if gi > 0 {
            ui.painter().line_segment([pos2(area.left() + 6.0, y + 2.0), pos2(area.right() - 6.0, y + 2.0)], Stroke::new(1.0, t.separator));
            y += 7.0;
        }
        let top = y;
        for p in app.ui.dock.tabs_of(g, pro) {
            let r = Rect::from_min_size(pos2(area.left() + 2.0, y), vec2((area.width() - 4.0).max(0.0), ROW_H));
            y += ROW_H + 2.0;
            let resp = ui.interact(r, egui::Id::new(("dock-icon", g.key(), p.key())), Sense::click_and_drag());
            let on = open.is_some_and(|f| f.group == g) && app.ui.dock.active.get(&g).copied().or_else(|| {
                let tabs = app.ui.dock.tabs_of(g, pro);
                app.ui.dock.active_panel(g, &tabs, &app.ui.dock_tabs, pro)
            }) == Some(p);
            if on || resp.hovered() {
                ui.painter().rect_filled(r, t.radius_sm, if on { t.hover } else { t.hover.gamma_multiply(0.6) });
            }
            let icon = Rect::from_center_size(pos2(r.left() + 13.0, r.center().y), egui::Vec2::splat(18.0));
            crate::icons::paint(ui, icon, p.icon(), 16.0, if on { t.text } else { t.icon });
            let label = tl!(p.label());
            let painter = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
            painter.text(pos2(icon.right() + 6.0, r.center().y), egui::Align2::LEFT_CENTER, label, theme::semibold(11.5), t.text);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
            dock_drag::frame_push(ui.ctx(), icons_id(), (g, p, r));
            if resp.clicked() {
                if on {
                    close_flyout(ui.ctx());
                } else {
                    let mut dt = app.ui.dock_tabs;
                    app.ui.dock.set_active(g, p, &mut dt, pro);
                    app.ui.dock_tabs = dt;
                    app.ui.dock.set_collapsed(g, false);
                    ui.ctx().data_mut(|d| d.insert_temp(flyout_id(), Flyout { group: g, anchor: r, to_left }));
                }
            }
            if !locked
                && resp.drag_started_by(egui::PointerButton::Primary)
                && let Some(o) = ui.input(|i| i.pointer.press_origin())
            {
                close_flyout(ui.ctx());
                dock_drag::begin(app, ui.ctx(), Payload::Tab { group: g, panel: p }, o - r.min, label.to_string(), vec2(260.0, g.default_height()));
            }
        }
        let block = Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), y));
        dock_drag::add_zone(ui.ctx(), dock_drag::Zone { rect: block, kind: dock_drag::ZoneKind::Body { group: g, horizontal: false }, float });
    }
}

/// The extra dock columns at `side`, as egui side panels (call where that side's panels go).
pub fn columns(app: &mut PrismApp, ui: &mut egui::Ui, side: Side) {
    if !app.ui.panels.dock {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let pro = t.pro;
    let locked = app.session.prefs().workspace_locked;
    let idxs: Vec<usize> = app.ui.dock.columns.iter().enumerate().filter(|(_, c)| c.side == side).map(|(i, _)| i).collect();
    for i in idxs {
        let Some(col) = app.ui.dock.columns.get(i).cloned() else { continue };
        let groups = dock_model::visible_groups(&app.ui, Slot::Column(i), pro);
        if groups.is_empty() {
            continue;
        }
        let id = egui::Id::new(("dock-column", col.id));
        let range = side.size_range();
        let size = sane(col.size, range.clone(), side.default_size());
        let iconic = col.iconic && side != Side::Bottom;
        let panel = match side {
            Side::Left => egui::Panel::left(id),
            Side::Right => egui::Panel::right(id),
            Side::Bottom => egui::Panel::bottom(id),
        };
        let panel = if iconic { panel.resizable(false).exact_size(ICONIC_W) } else { panel.resizable(!locked).default_size(size).size_range(range) };
        let mut toggle = false;
        let resp = panel.frame(egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin::same(2))).show(ui, |ui| {
            let full = ui.max_rect();
            let head = Rect::from_min_size(full.min, vec2(full.width(), COLUMN_HEADER));
            if side == Side::Bottom {
                ui.painter().rect_filled(head, 0.0, t.tab_strip.gamma_multiply(0.7));
            } else {
                toggle = header(ui, head, iconic, side == Side::Right, ("column", col.id));
            }
            let area = Rect::from_min_max(pos2(full.left(), head.bottom() + 2.0), full.max);
            if iconic {
                iconic_list(app, ui, area, &groups, side == Side::Right, None);
            } else {
                show_stack(app, ui, area, &groups, StackOpts { horizontal: side == Side::Bottom, float: None }, &mut body);
            }
            ui.advance_cursor_after_rect(full);
        });
        let r = resp.response.rect;
        if let Some(c) = app.ui.dock.columns.get_mut(i) {
            if toggle {
                c.iconic = !c.iconic;
                close_flyout(ui.ctx());
            }
            if !iconic && r.is_finite() {
                c.size = if side == Side::Bottom { r.height() } else { r.width() };
            }
        }
    }
}

/// What a floating window's chrome asked for this frame.
#[derive(Default)]
struct FloatActions {
    close: bool,
    iconic: bool,
    grow: egui::Vec2,
}

/// The floating panel windows.
pub fn floats(app: &mut PrismApp, ctx: &egui::Context) {
    if !app.ui.panels.dock {
        return;
    }
    let t = Tokens::get(ctx);
    let pro = t.pro;
    let screen = ctx.content_rect();
    let locked = app.session.prefs().workspace_locked;
    for i in 0..app.ui.dock.floats.len() {
        let Some(f) = app.ui.dock.floats.get(i).cloned() else { continue };
        let groups = dock_model::visible_groups(&app.ui, Slot::Float(i), pro);
        if groups.is_empty() {
            continue;
        }
        let w = if f.iconic { ICONIC_W } else { sane(f.size[0], FLOAT_W, 280.0) };
        let h = if f.iconic { TITLE_H + iconic_height(app, &groups, pro) } else { sane(f.size[1], FLOAT_H, 400.0) };
        let pos = crate::tools_window::sane_pos(f.pos, w, screen);
        let frame = egui::Frame::NONE
            .fill(t.dock)
            .stroke(Stroke::new(1.0, t.card_border))
            .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: t.shadow });
        let mut act = FloatActions::default();
        egui::Area::new(egui::Id::new(("dock-float", f.id))).order(egui::Order::Middle).current_pos(pos).movable(false).constrain(false).show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let (strip, grip) = ui.allocate_exact_size(vec2(w, TITLE_H), Sense::click_and_drag());
                ui.painter().rect_filled(strip, 0.0, t.tab_strip);
                let title = groups.iter().flat_map(|g| app.ui.dock.tabs_of(*g, pro)).map(|p| tl!(p.label())).collect::<Vec<_>>().join(", ");
                grip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &title));
                let grip = if locked { grip } else { grip.on_hover_cursor(egui::CursorIcon::Grab) };
                if !locked
                    && grip.drag_started_by(egui::PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin())
                {
                    dock_drag::begin(app, ui.ctx(), Payload::Float(f.id), o - strip.min, title.clone(), vec2(w, h));
                }
                act.iconic |= grip.double_clicked();
                // « collapses to icons (» expands), × closes the window's panels.
                let cell = vec2(TITLE_H, TITLE_H);
                let x_rect = Rect::from_min_size(pos2(strip.right() - TITLE_H - 2.0, strip.top()), cell);
                let c_rect = Rect::from_min_size(pos2(x_rect.left() - TITLE_H - 2.0, strip.top()), cell);
                let chev_label = if f.iconic { tl!("Expand Panels") } else { tl!("Collapse to Icons") };
                let chev = ui.interact(c_rect, egui::Id::new(("dock-float-iconic", f.id)), Sense::click()).on_hover_text(chev_label);
                chev.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, chev_label));
                let shut = ui.interact(x_rect, egui::Id::new(("dock-float-close", f.id)), Sense::click()).on_hover_text(tl!("Close"));
                shut.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Close")));
                let tint = |r: &egui::Response| if r.hovered() { t.text } else { t.text_faint };
                crate::icons::paint(ui, c_rect, if f.iconic { "chevrons-right" } else { "chevrons-left" }, 10.0, tint(&chev));
                crate::icons::paint(ui, x_rect, "x", 10.0, tint(&shut));
                act.iconic |= chev.clicked();
                act.close = shut.clicked();
                let body_h = if f.iconic { (h - TITLE_H).max(0.0) } else { (h - TITLE_H - GRIP_H).max(0.0) };
                let (area, _) = ui.allocate_exact_size(vec2(w, body_h), Sense::hover());
                if f.iconic {
                    let to_left = pos.x + w / 2.0 > screen.center().x;
                    iconic_list(app, ui, area, &groups, to_left, Some(f.id));
                    return;
                }
                show_stack(app, ui, area.shrink(2.0), &groups, StackOpts { horizontal: false, float: Some(f.id) }, &mut body);
                let (g, gresp) = ui.allocate_exact_size(vec2(w, GRIP_H), Sense::drag());
                let gresp = gresp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
                gresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, tl!("Resize")));
                ui.painter().rect_filled(g, 0.0, t.tab_strip.gamma_multiply(0.6));
                for k in -3..=3 {
                    let x = g.center().x + k as f32 * 3.0;
                    ui.painter().line_segment([pos2(x, g.center().y - 1.5), pos2(x, g.center().y + 1.5)], Stroke::new(1.0, t.text_faint));
                }
                if gresp.dragged() {
                    act.grow.y += gresp.drag_delta().y;
                }
                let edge = Rect::from_min_max(pos2(strip.right() - 3.0, strip.bottom()), pos2(strip.right() + 3.0, g.top()));
                let eresp = ui.interact(edge, egui::Id::new(("dock-float-width", f.id)), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                if eresp.dragged() {
                    act.grow.x += eresp.drag_delta().x;
                }
            });
        });
        if act.close {
            for g in &groups {
                dock_model::set_group_visible(&mut app.ui, *g, false);
            }
        }
        if let Some(fm) = app.ui.dock.floats.get_mut(i) {
            if act.iconic {
                fm.iconic = !fm.iconic;
                close_flyout(ctx);
            }
            if act.grow != egui::Vec2::ZERO && act.grow.is_finite() {
                fm.size = [sane(w + act.grow.x, FLOAT_W, w), sane(h + act.grow.y, FLOAT_H, h)];
            }
        }
    }
}

/// Is `g` in a column or window collapsed to icons (where a flyout shows it)?
fn in_iconic(app: &PrismApp, g: Group) -> bool {
    match app.ui.dock.slot_of(g) {
        Some((Slot::Main, _)) => app.ui.dock.main_iconic,
        Some((Slot::Column(i), _)) => app.ui.dock.columns.get(i).is_some_and(|c| c.iconic),
        Some((Slot::Float(i), _)) => app.ui.dock.floats.get(i).is_some_and(|f| f.iconic),
        None => false,
    }
}

/// The flyout of a group opened from an icon list, beside its icon. A click elsewhere, Escape
/// or a drag closes it.
pub fn flyout(app: &mut PrismApp, ctx: &egui::Context) {
    let Some(fl) = open_flyout(ctx) else { return };
    let pro = Tokens::get(ctx).pro;
    if dock_drag::current(ctx).is_some() || !in_iconic(app, fl.group) || !dock_model::group_visible(&app.ui, fl.group, pro) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close_flyout(ctx);
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let w = 290.0;
    let h = app.ui.dock.height(fl.group).clamp(160.0, (screen.height() - 40.0).max(160.0));
    let x = if fl.to_left { fl.anchor.left() - w - 6.0 } else { fl.anchor.right() + 6.0 };
    let y = fl.anchor.top().min(screen.bottom() - h - 8.0).max(screen.top());
    let rect = Rect::from_min_size(pos2(x, y), vec2(w, h));
    let frame = egui::Frame::NONE
        .fill(t.dock)
        .stroke(Stroke::new(1.0, t.card_border))
        .inner_margin(egui::Margin::same(2))
        .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: t.shadow });
    let resp = egui::Area::new(flyout_id()).order(egui::Order::Foreground).fixed_pos(rect.min).constrain(true).show(ctx, |ui| {
        frame.show(ui, |ui| {
            let (area, _) = ui.allocate_exact_size(rect.size() - vec2(4.0, 4.0), Sense::hover());
            show_stack(app, ui, area, &[fl.group], StackOpts::default(), &mut body);
        });
    });
    let shown = resp.response.rect;
    let pressed_outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| !shown.contains(p) && !fl.anchor.contains(p)));
    // Menus and popups opened from the flyout sit outside it but belong to it.
    if pressed_outside && !egui::Popup::is_any_open(ctx) {
        close_flyout(ctx);
    }
}

/// Rects of what agents can drag: groups (with their tab strips), icon rows, floating windows.
fn layout_rects(app: &PrismApp, ctx: &egui::Context) -> Value {
    let r = |r: Rect| json!([r.left(), r.top(), r.width(), r.height()]);
    let groups: Vec<Value> = crate::dock::last_rects(ctx).into_iter().map(|(g, rect)| json!({"group": g, "rect": r(rect)})).collect();
    let strips: Vec<Value> = crate::dock::last_strips(ctx)
        .into_iter()
        .map(|s| {
            let tabs = app.ui.dock.tabs_of(s.group, Tokens::get(ctx).pro);
            let tabs: Vec<Value> = s.tabs.iter().filter_map(|(i, rect)| tabs.get(*i).map(|p| json!({"panel": p, "rect": r(*rect)}))).collect();
            json!({"group": s.group, "tabs": tabs})
        })
        .collect();
    let icons: Vec<Value> = last_icons(ctx).into_iter().map(|(g, p, rect)| json!({"group": g, "panel": p, "rect": r(rect)})).collect();
    json!({"groups": groups, "strips": strips, "icons": icons})
}

/// `ui.dock`: read the dock layout and drive it.
///
/// - `{}`: the layout (`dock`) and last frame's rects (`rects`: groups, tab strips, icon rows)
/// - `{drop: {payload, target}}`: apply a drop (see `dock_model::Payload` / `DropTarget`), e.g.
///   `{"payload": {"tab": {"group": "layers", "panel": "paths"}}, "target": {"float": {"pos": [400, 200]}}}`
/// - `{toggle: <panel>, show?}`: Window › <panel> (`"toolSliders"` included)
/// - `{iconic: {main?: bool, column?: id, float?: id, on: bool}}`: collapse to icons or expand
/// - `{close: <panel>}`: close one tab
pub fn control(app: &mut PrismApp, ctx: &egui::Context, p: &Value) -> Result<Value, String> {
    let pro = Tokens::get(ctx).pro;
    let known = ["drop", "toggle", "show", "iconic", "close"];
    if let Some(k) = p.as_object().and_then(|o| o.keys().find(|k| !known.contains(&k.as_str()))) {
        return Err(format!("unknown field `{k}` (fields: {})", known.join(", ")));
    }
    let mut changed = false;
    if let Some(d) = p.get("drop") {
        let payload: Payload = serde_json::from_value(d.get("payload").cloned().unwrap_or(Value::Null)).map_err(|e| format!("drop.payload: {e}"))?;
        let target: DropTarget = serde_json::from_value(d.get("target").cloned().unwrap_or(Value::Null)).map_err(|e| format!("drop.target: {e}"))?;
        let mut dt = app.ui.dock_tabs;
        changed |= app.ui.dock.apply_drop(payload, target, &mut dt, pro)?;
        app.ui.dock_tabs = dt;
    }
    if let Some(v) = p.get("toggle") {
        let panel = v.as_str().and_then(PanelId::from_key).ok_or("toggle must be a panel name")?;
        let show = match p.get("show") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_bool().ok_or("show must be a boolean")?),
        };
        dock_model::toggle_panel(&mut app.ui, panel, pro, show);
        changed = true;
    }
    if let Some(v) = p.get("close") {
        let panel = v.as_str().and_then(PanelId::from_key).ok_or("close must be a panel name")?;
        let g = app.ui.dock.find_panel(panel, pro).ok_or_else(|| format!("{} is not in a group", panel.key()))?;
        dock_model::close_panel(&mut app.ui, g, panel, pro);
        changed = true;
    }
    if let Some(v) = p.get("iconic") {
        let on = v.get("on").and_then(Value::as_bool).ok_or("iconic.on must be a boolean")?;
        if v.get("main").is_some() {
            app.ui.dock.main_iconic = on;
        } else if let Some(id) = v.get("column").and_then(Value::as_u64) {
            let c = app.ui.dock.columns.iter_mut().find(|c| u64::from(c.id) == id).ok_or_else(|| format!("no dock column {id}"))?;
            c.iconic = on;
        } else if let Some(id) = v.get("float").and_then(Value::as_u64) {
            let f = app.ui.dock.floats.iter_mut().find(|f| u64::from(f.id) == id).ok_or_else(|| format!("no floating window {id}"))?;
            f.iconic = on;
        } else {
            return Err("iconic needs main, column or float".into());
        }
        changed = true;
    }
    Ok(json!({"changed": changed, "dock": app.ui.dock, "dockTabs": app.ui.dock_tabs, "rects": layout_rects(app, ctx)}))
}

#[cfg(test)]
#[path = "dock_float_tests.rs"]
mod tests;
