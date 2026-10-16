//! Window › Tool Sliders: brush Size and Opacity as two long sliders, after Clip Studio Paint's
//! panel of the same name. Each slider is a pill value badge plus a track that runs the length
//! of the panel: side by side and vertical when the panel is taller than wide (docked left or
//! right, or a tall floating window), stacked and horizontal otherwise (docked top or bottom).
//!
//! Edits go through `tools.setBrush` with [`crate::brush_panel::commit_gesture`], like the options
//! bar: one journal entry per drag. Where it sits ([`SlidersDock`]) and how big it is live in
//! [`ToolSlidersLayout`] (`UiState::tool_sliders`, saved with the workspace); whether it is open is
//! `Panels::tool_sliders` (Window › Tool Sliders).
//!
//! Drag the tab strip to move it. While the pointer is within [`SNAP_ZONE`] of a canvas edge,
//! a highlight shows the column or row it would dock as; letting go there docks it. Dragging a
//! docked panel's strip tears it off as a floating window.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, pos2, vec2};
use prism_engine::paint::{BrushSettings, MAX_BRUSH_SIZE};
use serde::{Deserialize, Serialize};

use crate::PrismApp;
use crate::theme::{self, Tokens};

/// Where the panel sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlidersDock {
    /// Its own movable, resizable window.
    #[default]
    Floating,
    /// A column between the toolbar and the canvas.
    Left,
    /// A column between the canvas and the panel dock.
    Right,
    /// A row under the options bar.
    Top,
    /// A row above the status bar.
    Bottom,
    /// A column right beside the panel groups (between them and the icon rail).
    Panels,
}

impl SlidersDock {
    pub const ALL: [SlidersDock; 6] = [SlidersDock::Floating, SlidersDock::Left, SlidersDock::Right, SlidersDock::Panels, SlidersDock::Top, SlidersDock::Bottom];

    fn label(self) -> &'static str {
        match self {
            SlidersDock::Floating => "Float",
            SlidersDock::Left => "Dock Left",
            SlidersDock::Right => "Dock Right",
            SlidersDock::Top => "Dock Top",
            SlidersDock::Bottom => "Dock Bottom",
            SlidersDock::Panels => "Dock Beside Panels",
        }
    }

    fn side(self) -> bool {
        matches!(self, SlidersDock::Left | SlidersDock::Right | SlidersDock::Panels)
    }
}

/// Placement and size of the Tool Sliders panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolSlidersLayout {
    pub dock: SlidersDock,
    /// Width of a left/right column (points).
    pub column: f32,
    /// Height of a top/bottom row (points).
    pub row: f32,
    /// Size of the floating window (points).
    pub floating: [f32; 2],
    /// Top-left corner of the floating window (points); unset = beside the canvas' right edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
}

impl Default for ToolSlidersLayout {
    fn default() -> Self {
        Self { dock: SlidersDock::Floating, column: 128.0, row: 96.0, floating: [128.0, 460.0], pos: None }
    }
}

/// Bounds on stored sizes (guards against absurd values from `ui.set` or a saved workspace).
const MIN_THICK: f32 = 72.0;
const MAX_THICK: f32 = 600.0;
const MAX_LONG: f32 = 4000.0;

fn sane(v: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { fallback }
}

/// Height of the tab strip at the top.
const HEADER: f32 = 24.0;
/// Size of the space a value badge gets: a one-line pill, value then unit, with room around the
/// text (a two-line circle crowded its digits).
const BADGE: egui::Vec2 = egui::vec2(56.0, 22.0);
/// Diameter of a slider's knob.
const KNOB: f32 = 16.0;

/// How close (points) the pointer must bring a dragged panel to a canvas edge to dock it there.
pub const SNAP_ZONE: f32 = 40.0;

/// The edge a drop with the pointer at `p` docks the panel to in the work area `work` (the canvas
/// area): the nearest edge within [`SNAP_ZONE`] of the pointer, inside the area or just past it.
pub fn snap_target(p: Pos2, work: Rect) -> Option<SlidersDock> {
    if !p.is_finite() || !work.is_finite() || work.width() <= 0.0 || work.height() <= 0.0 || !work.expand(SNAP_ZONE).contains(p) {
        return None;
    }
    [
        (SlidersDock::Left, p.x - work.left()),
        (SlidersDock::Right, work.right() - p.x),
        (SlidersDock::Top, p.y - work.top()),
        (SlidersDock::Bottom, work.bottom() - p.y),
    ]
    .into_iter()
    .filter(|(_, gap)| *gap <= SNAP_ZONE)
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(d, _)| d)
}

/// The area the panel takes docked at `dock` in `work`, with its stored column or row size.
pub fn docked_rect(layout: &ToolSlidersLayout, work: Rect, dock: SlidersDock) -> Rect {
    let col = sane(layout.column, MIN_THICK, MAX_THICK, 128.0).min(work.width());
    let row = sane(layout.row, MIN_THICK, MAX_THICK, 96.0).min(work.height());
    match dock {
        SlidersDock::Left => Rect::from_min_max(work.min, pos2(work.left() + col, work.bottom())),
        SlidersDock::Right => Rect::from_min_max(pos2(work.right() - col, work.top()), work.max),
        SlidersDock::Top => Rect::from_min_max(work.min, pos2(work.right(), work.top() + row)),
        SlidersDock::Bottom => Rect::from_min_max(pos2(work.left(), work.bottom() - row), work.max),
        // Drawn beside the panel groups, outside `work` (see `beside_panels`).
        SlidersDock::Floating | SlidersDock::Panels => Rect::NOTHING,
    }
}

/// The blue line shown while a drop would dock the panel at `dock`: 4 pt along that edge of the
/// work area `work` (beside the panel groups: along their left edge `panels_edge`).
pub fn snap_line(work: Rect, dock: SlidersDock, panels_edge: Option<f32>) -> Rect {
    const W: f32 = 4.0;
    match dock {
        SlidersDock::Left => Rect::from_min_max(work.min, pos2(work.left() + W, work.bottom())),
        SlidersDock::Right => Rect::from_min_max(pos2(work.right() - W, work.top()), work.max),
        SlidersDock::Top => Rect::from_min_max(work.min, pos2(work.right(), work.top() + W)),
        SlidersDock::Bottom => Rect::from_min_max(pos2(work.left(), work.bottom() - W), work.max),
        SlidersDock::Panels => {
            let x = panels_edge.filter(|x| x.is_finite()).unwrap_or(work.right());
            Rect::from_min_max(pos2(x - W / 2.0, work.top()), pos2(x + W / 2.0, work.bottom()))
        }
        SlidersDock::Floating => Rect::NOTHING,
    }
}

/// Where a drop at `p` docks the panel: beside the panel groups (left edge `panels_edge`), else
/// the nearest canvas edge of `work` ([`snap_target`]).
pub fn snap_target_at(p: Pos2, work: Rect, panels_edge: Option<f32>) -> Option<SlidersDock> {
    if crate::tools_window::near_panels_edge(p, work, panels_edge) {
        return Some(SlidersDock::Panels);
    }
    snap_target(p, work)
}

/// A drag of the panel by its tab strip: the pointer's offset from the panel's top-left corner,
/// and the edge a drop would dock it to right now.
#[derive(Clone, Copy, Debug)]
struct Drag {
    grab: egui::Vec2,
    snap: Option<SlidersDock>,
}

fn drag_id() -> egui::Id {
    egui::Id::new("tool-sliders-drag")
}

/// `Some(edge a drop would dock to)` while the panel is dragged (for tests and automation).
pub fn dragging(ctx: &egui::Context) -> Option<Option<SlidersDock>> {
    ctx.data(|d| d.get_temp::<Drag>(drag_id())).map(|d| d.snap)
}

/// Start dragging: `grab` is the pointer's offset from the panel's top-left corner. A docked
/// panel tears off and floats under the pointer right away.
fn begin_drag(app: &mut PrismApp, ctx: &egui::Context, grab: egui::Vec2) {
    if app.session.prefs().workspace_locked {
        return;
    }
    let w = sane(app.ui.tool_sliders.floating[0], MIN_THICK, MAX_LONG, 128.0);
    // A grab further right than the floating window is wide would leave the pointer off it.
    let grab = if grab.is_finite() { vec2(grab.x.clamp(0.0, (w - 30.0).max(0.0)), grab.y.clamp(0.0, HEADER)) } else { vec2(20.0, HEADER / 2.0) };
    ctx.data_mut(|d| d.insert_temp(drag_id(), Drag { grab, snap: None }));
    if let Some(p) = ctx.input(|i| i.pointer.latest_pos()).filter(|p| p.is_finite()) {
        let at = p - grab;
        app.ui.tool_sliders.pos = Some([at.x, at.y]);
    }
    app.ui.tool_sliders.dock = SlidersDock::Floating;
    ctx.request_repaint();
}

/// Follow the pointer while a drag is on and, near an edge, highlight where a drop would dock it;
/// on release dock there (or stay floating where it was let go).
fn track_drag(app: &mut PrismApp, ctx: &egui::Context) {
    let Some(mut drag) = ctx.data(|d| d.get_temp::<Drag>(drag_id())) else { return };
    let work = app.last_canvas_rect;
    if ctx.input(|i| i.pointer.primary_down()) {
        if let Some(p) = ctx.input(|i| i.pointer.latest_pos()).filter(|p| p.is_finite()) {
            let at = p - drag.grab;
            // Over a group's tab strip it joins the group (`dock_drag::drop_external`) instead.
            let edge = crate::tools_window::panels_edge(ctx).filter(|_| !crate::dock_drag::over_strip(ctx, p));
            drag.snap = snap_target_at(p, work, edge);
            app.ui.tool_sliders.pos = Some([at.x, at.y]);
        }
        if let Some(d) = drag.snap {
            // Feedback only: a blue line along the edge it would dock at, as for the Tools panel.
            let t = Tokens::get(ctx);
            let line = snap_line(work, d, crate::tools_window::panels_edge(ctx));
            ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, drag_id())).rect_filled(line, 0.0, t.docking_outline);
        } else if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
            // Over a panel group's tab strip (or edge) it would join the dock as a tab.
            crate::dock_drag::preview_external(ctx, p);
        }
        ctx.data_mut(|d| d.insert_temp(drag_id(), drag));
        return;
    }
    if let Some(d) = drag.snap {
        app.ui.tool_sliders.dock = d;
    } else if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
        crate::dock_drag::drop_external(app, ctx, p, crate::dock_model::PanelId::ToolSliders);
    }
    ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
}

/// Where the floating window goes: its stored corner, kept where its strip can still be grabbed.
fn floating_pos(app: &PrismApp, size: egui::Vec2, screen: Rect) -> Pos2 {
    let canvas = app.last_canvas_rect;
    let fallback = pos2((canvas.right() - size.x - 24.0).max(canvas.left() + 8.0), canvas.top() + 24.0);
    let p = app.ui.tool_sliders.pos.map(|[x, y]| pos2(x, y)).filter(|p| p.is_finite()).unwrap_or(fallback);
    if !screen.is_finite() || !p.is_finite() {
        return if fallback.is_finite() { fallback } else { Pos2::ZERO };
    }
    let keep = 40.0;
    let x = p.x.clamp(screen.left() - size.x + keep, (screen.right() - keep).max(screen.left() - size.x + keep));
    let y = p.y.clamp(screen.top(), (screen.bottom() - HEADER).max(screen.top()));
    pos2(x, y)
}

/// Which slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    Size,
    Opacity,
}

impl Slider {
    /// The accessible label (also the hover text).
    pub fn label(self) -> &'static str {
        match self {
            Slider::Size => "Brush Size",
            Slider::Opacity => "Brush Opacity",
        }
    }

    /// The brush value at track position `f` (0 = minimum end, 1 = maximum end).
    pub fn value_at(self, f: f32) -> f32 {
        let f = if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            // Logarithmic, as the Brush Settings Size slider: small sizes get most of the travel.
            Slider::Size => round_size((f * MAX_BRUSH_SIZE.ln()).exp()),
            Slider::Opacity => (f * 100.0).round() / 100.0,
        }
    }

    /// Where `b`'s value sits on the track (0..=1).
    pub fn fraction(self, b: &BrushSettings) -> f32 {
        let f = match self {
            Slider::Size => b.size.max(1.0).ln() / MAX_BRUSH_SIZE.ln(),
            Slider::Opacity => b.opacity,
        };
        if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 }
    }

    fn set(self, b: &mut BrushSettings, f: f32) {
        match self {
            Slider::Size => b.size = self.value_at(f),
            Slider::Opacity => b.opacity = self.value_at(f),
        }
    }

    /// The badge's two lines: value and unit.
    pub fn badge(self, b: &BrushSettings) -> (String, &'static str) {
        match self {
            Slider::Size if b.size >= 1000.0 => (format!("{:.0}", b.size), "px"),
            Slider::Size => (format!("{:.1}", b.size), "px"),
            Slider::Opacity => (format!("{:.0}", (b.opacity * 100.0).clamp(0.0, 100.0)), "%"),
        }
    }
}

/// Sizes below 10 px keep a tenth of a pixel (the low end is where the log track is finest).
fn round_size(s: f32) -> f32 {
    let s = if s.is_finite() { s } else { 1.0 };
    let r = if s < 10.0 { (s * 10.0).round() / 10.0 } else { s.round() };
    r.clamp(1.0, MAX_BRUSH_SIZE)
}

/// Do the sliders run vertically in a body of this size?
pub fn vertical(size: egui::Vec2) -> bool {
    size.y > size.x
}

/// Track rects (minimum end first) and badge centres of both sliders laid out in `body`.
pub fn layout(body: Rect) -> [(Slider, Pos2, Rect); 2] {
    let pad = 6.0;
    let gap = 4.0;
    let sliders = [Slider::Size, Slider::Opacity];
    if vertical(body.size()) {
        let w = (body.width() - 2.0 * pad) / 2.0;
        sliders.map(|s| {
            let i = if s == Slider::Size { 0.0 } else { 1.0 };
            let cx = body.left() + pad + w * (i + 0.5);
            let badge = pos2(cx, body.top() + pad + BADGE.y / 2.0);
            let top = badge.y + BADGE.y / 2.0 + gap + KNOB / 2.0;
            let bottom = (body.bottom() - pad - KNOB / 2.0).max(top);
            (s, badge, Rect::from_min_max(pos2(cx - 3.0, top), pos2(cx + 3.0, bottom)))
        })
    } else {
        let h = (body.height() - 2.0 * pad) / 2.0;
        sliders.map(|s| {
            let i = if s == Slider::Size { 0.0 } else { 1.0 };
            let cy = body.top() + pad + h * (i + 0.5);
            let badge = pos2(body.left() + pad + BADGE.x / 2.0, cy);
            let left = badge.x + BADGE.x / 2.0 + gap + 4.0 + KNOB / 2.0;
            let right = (body.right() - pad - KNOB / 2.0).max(left);
            (s, badge, Rect::from_min_max(pos2(left, cy - 3.0), pos2(right, cy + 3.0)))
        })
    }
}

/// The point on `track` at fraction `f` (the minimum end is the bottom or the left).
fn point_at(track: Rect, f: f32) -> Pos2 {
    if vertical(track.size()) { pos2(track.center().x, track.bottom() - f * track.height()) } else { pos2(track.left() + f * track.width(), track.center().y) }
}

/// The fraction of `track` nearest `p`.
fn fraction_at(track: Rect, p: Pos2) -> f32 {
    let f = if vertical(track.size()) {
        (track.bottom() - p.y) / track.height().max(1.0)
    } else {
        (p.x - track.left()) / track.width().max(1.0)
    };
    if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 }
}

/// Draw one slider and apply a press or drag on it to `b`.
fn slider(ui: &mut egui::Ui, b: &mut BrushSettings, s: Slider, badge: Pos2, track: Rect, enabled: bool) {
    let t = Tokens::get(ui.ctx());
    let vert = vertical(track.size());
    // The hit area: the badge plus the track, widened to the knob.
    let badge_rect = Rect::from_center_size(badge, BADGE);
    let hit = badge_rect.union(track.expand(KNOB / 2.0 + 2.0));
    let sense = if enabled { Sense::click_and_drag() } else { Sense::hover() };
    let resp = ui.interact(hit, ui.id().with(("tool-slider", s.label())), sense);
    // A press on the track jumps there and drags; a press on the badge does nothing.
    let from_badge = ui.input(|i| i.pointer.press_origin()).is_some_and(|o| badge_rect.contains(o));
    if let Some(p) = resp.interact_pointer_pos()
        && resp.is_pointer_button_down_on()
        && !from_badge
    {
        s.set(b, fraction_at(track, p));
    }
    let f = s.fraction(b);
    let resp = resp.on_hover_text(tl!(s.label()));
    resp.widget_info(|| egui::WidgetInfo::slider(enabled, f64::from(s.fraction(b)), tl!(s.label())));
    let p = ui.painter();
    let dim = if enabled { 1.0 } else { 0.45 };
    let line = track.expand2(if vert { vec2(-0.5, KNOB / 2.0) } else { vec2(KNOB / 2.0, -0.5) });
    let r = CornerRadius::same((line.width().min(line.height()) / 2.0) as u8);
    let knob = point_at(track, f);
    match s {
        Slider::Size => {
            p.rect_filled(line, r, t.field.gamma_multiply(dim));
            let filled = if vert { Rect::from_min_max(pos2(line.left(), knob.y), line.max) } else { Rect::from_min_max(line.min, pos2(knob.x, line.bottom())) };
            p.rect_filled(filled, r, t.text_dim.gamma_multiply(dim));
        }
        Slider::Opacity => {
            // Transparent (checkerboard) at the minimum end to the brush colour at the maximum.
            crate::widgets::checker(&p.with_clip_rect(line.intersect(p.clip_rect())), line, 3.0);
            let [cr, cg, cb, _] = b.color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            let solid = Color32::from_rgb(cr, cg, cb);
            let steps = 32;
            for k in 0..steps {
                let (a, z) = (k as f32 / steps as f32, (k + 1) as f32 / steps as f32);
                let seg = if vert {
                    Rect::from_min_max(pos2(line.left(), line.bottom() - z * line.height()), pos2(line.right(), line.bottom() - a * line.height()))
                } else {
                    Rect::from_min_max(pos2(line.left() + a * line.width(), line.top()), pos2(line.left() + z * line.width(), line.bottom()))
                };
                p.rect_filled(seg, 0.0, solid.gamma_multiply((a + z) / 2.0 * dim));
            }
            p.rect_stroke(line, r, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
        }
    }
    let hot = enabled && (resp.hovered() || resp.dragged());
    p.circle_filled(knob, KNOB / 2.0, if hot { t.text } else { t.text_dim.gamma_multiply(dim.max(0.7)) });
    // The value badge: a pill, dark for Size and light for Opacity, the value and a smaller,
    // fainter unit on one line, centred with padding on both sides.
    let (fill, ink) = match s {
        Slider::Size => (t.field, t.text),
        Slider::Opacity => (t.text, t.field),
    };
    let (value, unit) = s.badge(b);
    let vg = p.layout_no_wrap(value, theme::semibold(11.5), ink);
    let ug = p.layout_no_wrap(unit.to_string(), theme::medium(9.5), ink.gamma_multiply(0.7));
    let text_w = vg.size().x + 2.5 + ug.size().x;
    let pill = Rect::from_center_size(badge, vec2((text_w + 14.0).max(BADGE.y + 8.0), BADGE.y));
    p.rect(pill, CornerRadius::same((BADGE.y / 2.0) as u8), fill.gamma_multiply(dim.max(0.7)), Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    let x0 = pill.center().x - text_w / 2.0;
    let base = pill.center().y + vg.size().y / 2.0;
    let ug_h = ug.size().y;
    let vg_w = vg.size().x;
    p.galley(pos2(x0, base - vg.size().y), vg, ink);
    // The unit sits on the value's baseline.
    p.galley(pos2(x0 + vg_w + 2.5, base - ug_h - 1.0), ug, ink);
}

/// What the panel menu asked for.
enum MenuPick {
    Dock(SlidersDock),
    Close,
}

/// Draw the panel in `rect`: the tab strip and both sliders. Returns a menu pick; a drag that
/// starts on the strip moves the panel.
fn contents(app: &mut PrismApp, ui: &mut egui::Ui, rect: Rect) -> Option<MenuPick> {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, t.card);
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    let grip_rect = Rect::from_min_max(strip.min, pos2(strip.right() - 26.0, strip.bottom()));
    let grip = ui.interact(grip_rect, ui.id().with("tool-sliders-grip"), Sense::drag());
    grip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, tl!("Tool Sliders")));
    if !app.session.prefs().workspace_locked {
        let grip = grip.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tl!("Drag to float or dock the Tool Sliders panel"));
        if grip.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = ui.input(|i| i.pointer.press_origin()).or_else(|| grip.interact_pointer_pos())
        {
            begin_drag(app, ui.ctx(), p - rect.min);
        }
    }
    let title = tl!("Tool Sliders");
    let icon = Rect::from_center_size(pos2(strip.left() + 13.0, strip.center().y), egui::Vec2::splat(14.0));
    crate::icons::paint(ui, icon, "sliders-horizontal", 13.0, t.icon);
    let room = strip.width() - 26.0 - 26.0;
    let galley = ui.painter().layout(title.to_string(), theme::medium(11.0), t.text, f32::INFINITY);
    if galley.size().x <= room {
        ui.painter().galley(pos2(icon.right() + 5.0, strip.center().y - galley.size().y / 2.0), galley, t.text);
    }
    let mut pick = None;
    let menu_rect = Rect::from_center_size(pos2(strip.right() - 13.0, strip.center().y), egui::Vec2::splat(20.0));
    let menu = ui.interact(menu_rect, ui.id().with("tool-sliders-menu"), Sense::click()).on_hover_text(tl!("Tool Sliders menu"));
    menu.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Tool Sliders menu")));
    if menu.hovered() {
        ui.painter().rect_filled(menu_rect, t.radius_sm, t.hover);
    }
    crate::icons::paint(ui, menu_rect, "ellipsis", 14.0, t.icon);
    egui::Popup::menu(&menu).show(|ui| {
        ui.set_min_width(140.0);
        let here = app.ui.tool_sliders.dock;
        for d in SlidersDock::ALL {
            if ui.add(egui::Button::selectable(d == here, tl!(d.label()))).clicked() {
                pick = Some(MenuPick::Dock(d));
                ui.close();
            }
        }
        ui.separator();
        if ui.button(tl!("Close")).clicked() {
            pick = Some(MenuPick::Close);
            ui.close();
        }
    });
    let body = Rect::from_min_max(pos2(rect.left(), strip.bottom()), rect.max);
    sliders(app, ui, body);
    pick
}

/// Both sliders laid out in `body` (vertical when it is taller than wide).
fn sliders(app: &mut PrismApp, ui: &mut egui::Ui, body: Rect) {
    let enabled = app.ui.tool.is_brushlike();
    let before = app.session.tools.brush.clone();
    let mut b = before.clone();
    for (s, badge, track) in layout(body) {
        slider(ui, &mut b, s, badge, track, enabled);
    }
    crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &b);
}

/// Tool Sliders as a tab of a dock group (`dock_model::PanelId::ToolSliders`): the sliders fill
/// the group's body, laid out by its shape as in the panel's own window.
pub fn grouped_body(app: &mut PrismApp, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    sliders(app, ui, rect);
    ui.advance_cursor_after_rect(rect);
}

fn apply(app: &mut PrismApp, pick: Option<MenuPick>) {
    match pick {
        Some(MenuPick::Dock(d)) => app.ui.tool_sliders.dock = d,
        Some(MenuPick::Close) => app.ui.panels.tool_sliders = false,
        None => {}
    }
}

/// The docked panel, drawn before the canvas takes the rest of `ui`. Docked beside the panel
/// groups it is drawn by [`beside_panels`] instead, unless ⇧Tab hid them (then at the right).
pub fn docked(app: &mut PrismApp, ui: &mut egui::Ui) {
    if app.ui.tool_sliders.dock == SlidersDock::Panels && app.ui.panels.dock {
        return;
    }
    show_docked(app, ui);
}

/// The panel docked beside the panel groups (called from the panel dock, between its groups and
/// the icon rail).
pub fn beside_panels(app: &mut PrismApp, ui: &mut egui::Ui) {
    if app.ui.tool_sliders.dock == SlidersDock::Panels {
        show_docked(app, ui);
    }
}

fn show_docked(app: &mut PrismApp, ui: &mut egui::Ui) {
    let dock = app.ui.tool_sliders.dock;
    // A tab of a dock group instead (see `dock_model`).
    if !app.ui.panels.tool_sliders || dock == SlidersDock::Floating || crate::dock_model::sliders_grouped(&app.ui) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let layout = &app.ui.tool_sliders;
    let (size, range) = if dock.side() {
        (sane(layout.column, MIN_THICK, MAX_THICK, 128.0), MIN_THICK..=MAX_THICK)
    } else {
        (sane(layout.row, MIN_THICK, MAX_THICK, 128.0), MIN_THICK..=MAX_THICK)
    };
    let id = egui::Id::new(("tool-sliders", dock));
    let panel = match dock {
        SlidersDock::Left => egui::Panel::left(id),
        SlidersDock::Right | SlidersDock::Panels => egui::Panel::right(id),
        SlidersDock::Top => egui::Panel::top(id),
        _ => egui::Panel::bottom(id),
    };
    let mut pick = None;
    let resp = panel.resizable(!app.session.prefs().workspace_locked).default_size(size).size_range(range).frame(egui::Frame::NONE.fill(t.card)).show(ui, |ui| {
        let rect = ui.max_rect();
        pick = contents(app, ui, rect);
        ui.advance_cursor_after_rect(rect);
    });
    let r = resp.response.rect;
    if dock.side() {
        app.ui.tool_sliders.column = sane(r.width(), MIN_THICK, MAX_THICK, 128.0);
    } else {
        app.ui.tool_sliders.row = sane(r.height(), MIN_THICK, MAX_THICK, 128.0);
    }
    apply(app, pick);
}

/// The floating window.
pub fn window(app: &mut PrismApp, ctx: &egui::Context) {
    if !app.ui.panels.tool_sliders || app.ui.tool_sliders.dock != SlidersDock::Floating || crate::dock_model::sliders_grouped(&app.ui) {
        return;
    }
    let t = Tokens::get(ctx);
    let [w, h] = app.ui.tool_sliders.floating;
    let size = vec2(sane(w, MIN_THICK, MAX_LONG, 128.0), sane(h, MIN_THICK, MAX_LONG, 460.0));
    track_drag(app, ctx);
    let pos = floating_pos(app, size, ctx.content_rect());
    let frame = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: t.shadow });
    let mut pick = None;
    let resp = egui::Window::new(tl!("Tool Sliders"))
        .id(egui::Id::new("tool-sliders-window"))
        .title_bar(false)
        .resizable(true)
        // Moved by its tab strip only (see `begin_drag`), which docks it on a drop near an edge.
        .movable(false)
        .constrain(false)
        .min_size(egui::Vec2::splat(MIN_THICK))
        .default_size(size)
        .frame(frame)
        .current_pos(pos)
        .show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            pick = contents(app, ui, rect);
            ui.advance_cursor_after_rect(rect);
        });
    if let Some(r) = resp.map(|r| r.response.rect)
        && r.is_finite()
    {
        app.ui.tool_sliders.floating = [r.width().clamp(MIN_THICK, MAX_LONG), r.height().clamp(MIN_THICK, MAX_LONG)];
        // Resizing from the left or top edge moves the corner.
        if dragging(ctx).is_none() && app.ui.tool_sliders.dock == SlidersDock::Floating {
            app.ui.tool_sliders.pos = Some([r.left(), r.top()]);
        }
    }
    apply(app, pick);
}

#[cfg(test)]
#[path = "tool_sliders_tests.rs"]
mod tests;
