//! Window › Tools: the toolbar as a panel that tears off, as in Photoshop. Drag its header off a
//! dock edge and it floats over the canvas in a small window of its own (a title strip with «,
//! which switches between one and two columns, and ×, which closes it); drag that strip back to
//! the left or right edge, or right beside the panel groups (between them and the icon rail), and it
//! docks there again. It docks vertically only: the top and bottom
//! of the work area are never drop targets.
//!
//! Where it sits is [`ToolsLayout`] (`UiState::tools`, drivable with `ui.set {"tools": …}`, saved
//! with the workspace and remembered in `panelLayout`); whether it is open is `Panels::toolbar`
//! (Window › Tools) and its column count `Panels::toolbar_double`.

use egui::{Pos2, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::PrismApp;
use crate::theme::Tokens;

/// Where the Tools panel sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolsDock {
    /// A column at the left edge of the work area (Photoshop's default).
    #[default]
    Left,
    /// A column between the canvas and the panel dock.
    Right,
    /// A column right beside the panel dock's groups (between them and the icon rail).
    Panels,
    /// Its own small window over the canvas.
    Floating,
}

/// Placement of the Tools panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolsLayout {
    pub dock: ToolsDock,
    /// Top-left corner of the floating window (points, window coordinates).
    pub pos: [f32; 2],
}

impl Default for ToolsLayout {
    fn default() -> Self {
        Self { dock: ToolsDock::Left, pos: [220.0, 180.0] }
    }
}

/// How far from a dock edge a drop still docks (points).
pub const DOCK_ZONE: f32 = 40.0;
/// Height of the floating window's title strip.
pub const TITLE_H: f32 = 14.0;
/// How much of the floating window stays on screen when it is dragged or restored off it.
const KEEP_VISIBLE: f32 = 40.0;

/// Where a drop at `p` docks the panel: the left edge of the work area `work` (below the options
/// bar, above the status bar) or the canvas' right edge `right_edge` (the panel dock's left side)
/// through to the window's right edge. Only these vertical docks exist: a drop near the top or
/// bottom of the work area, or anywhere else, floats.
pub fn drop_target(p: Pos2, work: Rect, right_edge: f32) -> Option<ToolsDock> {
    // Past the left or right edge still docks; above or below the work area never does.
    if !p.is_finite() || !work.is_finite() || !work.y_range().contains(p.y) {
        return None;
    }
    if p.x <= work.left() + DOCK_ZONE {
        return Some(ToolsDock::Left);
    }
    let right_edge = if right_edge.is_finite() { right_edge.clamp(work.left(), work.right()) } else { work.right() };
    (p.x >= right_edge - DOCK_ZONE).then_some(ToolsDock::Right)
}

/// How far a drop may land toward the canvas from the panel dock's edge and still dock beside it.
const PANELS_ZONE_IN: f32 = 14.0;
/// How far into the panel dock a drop still docks beside it.
const PANELS_ZONE_OUT: f32 = 48.0;

fn panels_edge_id() -> egui::Id {
    egui::Id::new("tools-panels-edge")
}

/// Remember where the panel dock's groups start this pass (their left edge, screen points).
pub fn set_panels_edge(ctx: &egui::Context, x: f32) {
    if x.is_finite() {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| d.insert_temp(panels_edge_id(), (pass, x)));
    }
}

/// The panel dock's left edge, when the dock was drawn this pass.
pub fn panels_edge(ctx: &egui::Context) -> Option<f32> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<(u64, f32)>(panels_edge_id())).filter(|(p, _)| *p == pass).map(|(_, x)| x)
}

/// [`drop_target`], plus the column beside the panel dock's groups at `panels_edge` (their left
/// edge): a drop just left of it or a little into the dock docks there. Still vertical only.
pub fn drop_target_at(p: Pos2, work: Rect, right_edge: f32, panels_edge: Option<f32>) -> Option<ToolsDock> {
    if near_panels_edge(p, work, panels_edge) {
        return Some(ToolsDock::Panels);
    }
    drop_target(p, work, right_edge)
}

/// Is `p` where a vertical panel (Tools, Tool Sliders) docks beside the panel groups, whose left
/// edge is `panels_edge`: just left of it or a little into the groups, within the work area?
pub fn near_panels_edge(p: Pos2, work: Rect, panels_edge: Option<f32>) -> bool {
    panels_edge.filter(|x| x.is_finite()).is_some_and(|x| {
        p.is_finite() && work.is_finite() && work.y_range().contains(p.y) && (x - PANELS_ZONE_IN..=x + PANELS_ZONE_OUT).contains(&p.x)
    })
}

/// The floating window's top-left `pos`, kept where its title strip can still be grabbed in
/// `screen` (a stored position may come from a bigger display, or from `ui.set`).
pub fn sane_pos(pos: [f32; 2], width: f32, screen: Rect) -> Pos2 {
    let fallback = screen.left_top() + vec2(60.0, 80.0);
    let p = pos2(pos[0], pos[1]);
    if !p.is_finite() || !screen.is_finite() {
        return if fallback.is_finite() { fallback } else { Pos2::ZERO };
    }
    let w = if width.is_finite() { width.max(0.0) } else { 0.0 };
    let x = p.x.clamp(screen.left() - w + KEEP_VISIBLE, (screen.right() - KEEP_VISIBLE).max(screen.left() - w + KEEP_VISIBLE));
    let y = p.y.clamp(screen.top(), (screen.bottom() - TITLE_H).max(screen.top()));
    pos2(x, y)
}

/// A drag of the panel in progress: where the pointer holds it, relative to its top-left corner.
#[derive(Clone, Copy, Debug)]
struct Drag {
    grab: egui::Vec2,
}

fn drag_id() -> egui::Id {
    egui::Id::new("tools-panel-drag")
}

/// Is the panel being dragged?
pub fn dragging(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Drag>(drag_id())).is_some()
}

/// Start moving the panel: `grab` is the pointer's offset from the panel's top-left corner. A
/// docked panel tears off and floats right away, so it follows the pointer.
pub fn begin_drag(app: &mut PrismApp, ctx: &egui::Context, grab: egui::Vec2) {
    if app.session.prefs().workspace_locked {
        return;
    }
    let grab = if grab.is_finite() { grab } else { egui::Vec2::ZERO };
    ctx.data_mut(|d| d.insert_temp(drag_id(), Drag { grab }));
    if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
        let at = p - grab;
        app.ui.tools.pos = [at.x, at.y];
    }
    app.ui.tools.dock = ToolsDock::Floating;
    ctx.request_repaint();
}

/// Follow the pointer while a drag is on, show where a drop would dock, and dock on release.
/// `work` and `right_edge` as in [`drop_target`].
pub fn track_drag(app: &mut PrismApp, ctx: &egui::Context, work: Rect, right_edge: f32) {
    let Some(drag) = ctx.data(|d| d.get_temp::<Drag>(drag_id())) else { return };
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    if ctx.input(|i| i.pointer.primary_down()) {
        if let Some(p) = pointer.filter(|p| p.is_finite()) {
            let at = p - drag.grab;
            app.ui.tools.pos = [at.x, at.y];
            if let Some(target) = drop_target_at(p, work, right_edge, panels_edge(ctx)) {
                let t = Tokens::get(ctx);
                let bar = match target {
                    ToolsDock::Panels => {
                        let x = panels_edge(ctx).unwrap_or(right_edge);
                        Rect::from_min_max(pos2(x - 2.0, work.top()), pos2(x + 2.0, work.bottom()))
                    }
                    ToolsDock::Right => {
                        let x = right_edge.clamp(work.left(), work.right());
                        Rect::from_min_max(pos2(x - 4.0, work.top()), pos2(x, work.bottom()))
                    }
                    _ => Rect::from_min_max(work.left_top(), pos2(work.left() + 4.0, work.bottom())),
                };
                ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, drag_id())).rect_filled(bar, 0.0, t.docking_outline);
            }
        }
        return;
    }
    if let Some(target) = pointer.and_then(|p| drop_target_at(p, work, right_edge, panels_edge(ctx))) {
        app.ui.tools.dock = target;
    }
    ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
}

/// The docked panel's header row: drag it to tear the panel off. `rect` is the row; returns its
/// response so the caller can lay its own buttons over it.
pub fn docked_grip(app: &mut PrismApp, ui: &mut egui::Ui, rect: Rect, panel: Rect) -> egui::Response {
    let resp = ui.interact(rect, egui::Id::new("tools-docked-grip"), Sense::drag());
    let locked = app.session.prefs().workspace_locked;
    let resp = if locked { resp } else { resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tl!("Drag to float or dock the Tools panel")) };
    tear_off(app, ui, &resp, panel.min);
    resp
}

/// A drag that starts on `resp` (a header widget of the docked panel whose top-left corner is
/// `panel_min`) tears the panel off.
pub fn tear_off(app: &mut PrismApp, ui: &egui::Ui, resp: &egui::Response, panel_min: Pos2) {
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos())
    {
        begin_drag(app, ui.ctx(), p - panel_min);
    }
}

/// The floating window.
pub fn floating(app: &mut PrismApp, ctx: &egui::Context) {
    if !app.ui.panels.toolbar || app.ui.tools.dock != ToolsDock::Floating {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let geo = crate::panels::toolbar_geometry(app, &t, (screen.height() - TITLE_H - 40.0).max(0.0));
    let pos = sane_pos(app.ui.tools.pos, geo.width, screen);
    let frame = egui::Frame::NONE
        .fill(t.chrome)
        .stroke(Stroke::new(1.0, t.card_border))
        .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: t.shadow });
    let mut close = false;
    egui::Area::new(egui::Id::new("tools-floating"))
        .order(egui::Order::Middle)
        .current_pos(pos)
        .movable(false)
        .constrain(false)
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let (strip, grip) = ui.allocate_exact_size(vec2(geo.width, TITLE_H), Sense::drag());
                ui.painter().rect_filled(strip, 0.0, t.tab_strip);
                grip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, tl!("Tools")));
                let locked = app.session.prefs().workspace_locked;
                let grip = if locked { grip } else { grip.on_hover_cursor(egui::CursorIcon::Grab) };
                if grip.drag_started_by(egui::PointerButton::Primary)
                    && let Some(p) = grip.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.press_origin()))
                {
                    begin_drag(app, ui.ctx(), p - strip.min);
                }
                // « switches between one and two columns; × closes (Window › Tools reopens).
                let cell = vec2(TITLE_H, TITLE_H);
                let x_rect = Rect::from_min_size(pos2(strip.right() - TITLE_H - 1.0, strip.top()), cell);
                let c_rect = Rect::from_min_size(pos2(x_rect.left() - TITLE_H, strip.top()), cell);
                let chev = ui.interact(c_rect, egui::Id::new("tools-floating-columns"), Sense::click()).on_hover_text(tl!("Toolbar"));
                chev.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Toolbar")));
                let shut = ui.interact(x_rect, egui::Id::new("tools-floating-close"), Sense::click()).on_hover_text(tl!("Close"));
                shut.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Close")));
                let tint = |r: &egui::Response| if r.hovered() { t.text } else { t.text_faint };
                crate::icons::paint(ui, c_rect, if geo.double { "chevrons-left" } else { "chevrons-right" }, 10.0, tint(&chev));
                crate::icons::paint(ui, x_rect, "x", 10.0, tint(&shut));
                if chev.clicked() {
                    app.ui.panels.toolbar_double = !geo.double;
                }
                close = shut.clicked();
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(geo.margin, 6)).show(ui, |ui| {
                    ui.set_width(geo.width - 2.0 * f32::from(geo.margin));
                    crate::panels::toolbar_body(app, ui, &geo, false);
                });
            });
        });
    if close {
        app.ui.panels.toolbar = false;
    }
}

#[cfg(test)]
#[path = "tools_window_tests.rs"]
mod tests;
