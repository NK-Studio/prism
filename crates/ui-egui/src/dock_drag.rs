//! Dragging panels around the dock, as in Photoshop: a torn-off tab or a dragged group follows
//! the pointer as a ghost while a blue outline (a tab strip: join that group), a blue line (above
//! or below a group) or a blue bar (a work-area edge: a new dock column) shows where it would
//! land. Let go elsewhere and it becomes a floating window. A floating window dragged by its
//! title bar moves with the pointer and docks the same way.
//!
//! While drawing, every group registers drop [`Zone`]s for the frame; [`finish`] runs after
//! everything is drawn, resolves the zone under the pointer (the last one registered wins, so
//! floating windows beat what's under them) and applies the drop
//! (`DockLayout::apply_drop`). Escape cancels a drag.

use egui::{Pos2, Rect, Stroke, Vec2, pos2, vec2};

use crate::PrismApp;
use crate::dock::Group;
use crate::dock_model::{DragKind, DropTarget, Payload, PanelId, Side, allowed};
use crate::theme::{self, Tokens};

/// A dock drag in progress.
#[derive(Clone, Debug)]
pub struct Drag {
    pub payload: Payload,
    /// The pointer's offset from the dragged thing's top-left corner.
    pub grab: Vec2,
    /// What the ghost says ("Paths", "Layers | Channels | Paths").
    pub label: String,
    /// Size of what is dragged (the ghost is a smaller copy).
    pub size: Vec2,
}

fn drag_id() -> egui::Id {
    egui::Id::new("dock-drag")
}

/// The drag in progress, if any.
pub fn current(ctx: &egui::Context) -> Option<Drag> {
    ctx.data(|d| d.get_temp::<Drag>(drag_id()))
}

/// Start a drag (ignored while the workspace is locked).
pub fn begin(app: &mut PrismApp, ctx: &egui::Context, payload: Payload, grab: Vec2, label: String, size: Vec2) {
    if app.session.prefs().workspace_locked {
        return;
    }
    // A floating window is held where it was grabbed: its corner stays under the same spot.
    let grab = match payload {
        Payload::Float(id) => match (app.ui.dock.floats.iter().find(|f| f.id == id), ctx.input(|i| i.pointer.press_origin())) {
            (Some(f), Some(o)) => o - pos2(f.pos[0], f.pos[1]),
            _ => grab,
        },
        _ => grab,
    };
    let grab = if grab.is_finite() { grab } else { Vec2::ZERO };
    ctx.data_mut(|d| d.insert_temp(drag_id(), Drag { payload, grab, label, size }));
    ctx.request_repaint();
}

/// Drop the drag without applying it.
pub fn cancel(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
}

/// What a drop zone accepts.
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneKind {
    /// A tab strip: the payload joins the group; `tabs` are the strip's `(tab index, rect)`.
    Strip { group: Group, tabs: Vec<(usize, Rect)> },
    /// A group: the payload stacks above or below it (left or right of it in a bottom row).
    Body { group: Group, horizontal: bool },
    /// An edge of the work area or of a dock column: a new dock column.
    Edge { side: Side, index: usize },
}

/// A drop zone registered this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub rect: Rect,
    pub kind: ZoneKind,
    /// The floating window it belongs to (a dragged window never drops on itself).
    pub float: Option<u32>,
}

fn zones_id() -> egui::Id {
    egui::Id::new("dock-zones")
}

/// Append `item` to this pass's list under `id` (the list starts afresh each pass).
pub(crate) fn frame_push<T: Clone + Send + Sync + 'static>(ctx: &egui::Context, id: egui::Id, item: T) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let e = d.get_temp_mut_or_default::<(u64, Vec<T>)>(id);
        if e.0 != pass {
            *e = (pass, Vec::new());
        }
        e.1.push(item);
    });
}

/// The list under `id` from the latest pass that pushed to it.
pub(crate) fn frame_items<T: Clone + Send + Sync + 'static>(ctx: &egui::Context, id: egui::Id) -> Vec<T> {
    ctx.data(|d| d.get_temp::<(u64, Vec<T>)>(id)).map(|(_, v)| v).unwrap_or_default()
}

pub fn add_zone(ctx: &egui::Context, zone: Zone) {
    if zone.rect.is_finite() && zone.rect.is_positive() {
        frame_push(ctx, zones_id(), zone);
    }
}

/// The zones registered so far this pass (or the last one).
pub fn zones(ctx: &egui::Context) -> Vec<Zone> {
    frame_items(ctx, zones_id())
}

/// A resolved drop: where it lands, and the mark that shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub target: DropTarget,
    /// Outline (`outline`) or fill this rect.
    pub mark: Rect,
    pub outline: bool,
    /// The insertion caret on a tab strip.
    pub caret: Option<Rect>,
}

/// How thick the edge zones and marks are (points).
pub const EDGE: f32 = 28.0;

/// The zone under `p` that takes a `kind` drag (`float`: the window being dragged, whose own
/// zones don't count). Later zones win.
pub fn resolve(zones: &[Zone], p: Pos2, kind: DragKind, float: Option<u32>) -> Option<Hit> {
    if !p.is_finite() {
        return None;
    }
    for z in zones.iter().rev() {
        if !z.rect.contains(p) || (float.is_some() && z.float == float) {
            continue;
        }
        let hit = match &z.kind {
            ZoneKind::Strip { group, tabs } => {
                let next = tabs.iter().find(|(_, r)| p.x < r.center().x);
                let index = match next {
                    Some((i, _)) => *i,
                    None => tabs.iter().map(|(i, _)| i + 1).max().unwrap_or(0),
                };
                let x = match next {
                    Some((_, r)) => r.left(),
                    None => tabs.last().map_or(z.rect.left() + 4.0, |(_, r)| r.right()),
                };
                let caret = Rect::from_min_max(pos2(x - 1.0, z.rect.top() + 2.0), pos2(x + 1.0, z.rect.bottom() - 2.0));
                Hit { target: DropTarget::Tabs { group: *group, index }, mark: z.rect, outline: true, caret: Some(caret) }
            }
            ZoneKind::Body { group, horizontal } => {
                let r = z.rect;
                let after = if *horizontal { p.x > r.center().x } else { p.y > r.center().y };
                let mark = match (*horizontal, after) {
                    (false, false) => Rect::from_min_max(pos2(r.left(), r.top() - 3.0), pos2(r.right(), r.top() + 1.0)),
                    (false, true) => Rect::from_min_max(pos2(r.left(), r.bottom() - 1.0), pos2(r.right(), r.bottom() + 3.0)),
                    (true, false) => Rect::from_min_max(pos2(r.left() - 3.0, r.top()), pos2(r.left() + 1.0, r.bottom())),
                    (true, true) => Rect::from_min_max(pos2(r.right() - 1.0, r.top()), pos2(r.right() + 3.0, r.bottom())),
                };
                Hit { target: DropTarget::Stack { anchor: *group, after }, mark, outline: false, caret: None }
            }
            ZoneKind::Edge { side, index } => {
                let r = z.rect;
                let mark = match side {
                    Side::Left => Rect::from_min_max(r.min, pos2(r.left() + 4.0, r.bottom())),
                    Side::Right => Rect::from_min_max(pos2(r.right() - 4.0, r.top()), r.max),
                    Side::Bottom => Rect::from_min_max(pos2(r.left(), r.bottom() - 4.0), r.max),
                };
                Hit { target: DropTarget::Column { side: *side, index: *index }, mark, outline: false, caret: None }
            }
        };
        if allowed(kind, &hit.target) {
            return Some(hit);
        }
    }
    None
}

/// The work-area edges that take a new dock column: left, right and bottom of the canvas area
/// (`canvas`), each adding the innermost column on its side. Never the top (Photoshop docks
/// panels at the sides and bottom only).
pub fn edge_zones(app: &PrismApp, canvas: Rect) -> Vec<Zone> {
    if !canvas.is_finite() || canvas.width() < 3.0 * EDGE || canvas.height() < 3.0 * EDGE {
        return Vec::new();
    }
    let count = |s: Side| app.ui.dock.columns.iter().filter(|c| c.side == s).count();
    vec![
        Zone {
            rect: Rect::from_min_max(pos2(canvas.left(), canvas.bottom() - EDGE), canvas.max),
            kind: ZoneKind::Edge { side: Side::Bottom, index: count(Side::Bottom) },
            float: None,
        },
        Zone { rect: Rect::from_min_max(canvas.min, pos2(canvas.left() + EDGE, canvas.bottom())), kind: ZoneKind::Edge { side: Side::Left, index: count(Side::Left) }, float: None },
        Zone {
            rect: Rect::from_min_max(pos2(canvas.right() - EDGE, canvas.top()), canvas.max),
            kind: ZoneKind::Edge { side: Side::Right, index: count(Side::Right) },
            float: None,
        },
    ]
}

fn paint_hit(ctx: &egui::Context, hit: &Hit) {
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, drag_id().with("mark")));
    if hit.outline {
        painter.rect_filled(hit.mark, 2.0, t.docking_outline.gamma_multiply(0.15));
        painter.rect_stroke(hit.mark, 2.0, Stroke::new(2.0, t.docking_outline), egui::StrokeKind::Inside);
    } else {
        painter.rect_filled(hit.mark, 0.0, t.docking_outline);
    }
    if let Some(c) = hit.caret {
        painter.rect_filled(c, 0.0, t.docking_outline);
    }
}

fn paint_ghost(ctx: &egui::Context, drag: &Drag, p: Pos2) {
    let t = Tokens::get(ctx);
    let size = vec2(drag.size.x.clamp(60.0, 220.0), drag.size.y.clamp(24.0, 120.0));
    let grab = vec2(drag.grab.x.clamp(0.0, size.x), drag.grab.y.clamp(0.0, 20.0));
    let r = Rect::from_min_size(p - grab, size);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, drag_id().with("ghost")));
    painter.rect_filled(r, t.radius_sm, t.card.gamma_multiply(0.75));
    let strip = Rect::from_min_size(r.min, vec2(r.width(), 20.0));
    painter.rect_filled(strip, t.radius_sm, t.tab_strip.gamma_multiply(0.9));
    painter.rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    painter.text(strip.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, &drag.label, theme::medium(11.0), t.text);
}

/// Follow, preview and apply the drag in progress. Runs once per frame, after the dock, its
/// columns and floating windows have registered their zones.
pub fn finish(app: &mut PrismApp, ctx: &egui::Context) {
    let Some(drag) = current(ctx) else { return };
    if app.session.prefs().workspace_locked || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel(ctx);
        return;
    }
    let p = ctx.input(|i| i.pointer.latest_pos()).filter(|p| p.is_finite());
    let float = match drag.payload {
        Payload::Float(id) => Some(id),
        _ => None,
    };
    if let (Some(id), Some(p)) = (float, p)
        && let Some(f) = app.ui.dock.floats.iter_mut().find(|f| f.id == id)
    {
        let at = p - drag.grab;
        f.pos = [at.x, at.y];
    }
    let mut zs = edge_zones(app, app.last_canvas_rect);
    zs.extend(zones(ctx));
    let hit = p.and_then(|p| resolve(&zs, p, drag.payload.kind(), float));
    if ctx.input(|i| i.pointer.primary_down()) {
        if let Some(h) = &hit {
            paint_hit(ctx, h);
        }
        if let (None, Some(p)) = (float, p) {
            paint_ghost(ctx, &drag, p);
        }
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        ctx.request_repaint();
        return;
    }
    cancel(ctx);
    let pro = Tokens::get(ctx).pro;
    let target = match (hit, float, p) {
        (Some(h), _, _) => h.target,
        // A floating window let go away from any zone stays where it was dropped.
        (None, Some(_), _) | (None, None, None) => return,
        (None, None, Some(p)) => {
            let at = p - drag.grab;
            DropTarget::Float { pos: [at.x, at.y] }
        }
    };
    let mut dt = app.ui.dock_tabs;
    if let Err(e) = app.ui.dock.apply_drop(drag.payload, target, &mut dt, pro) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    app.ui.dock_tabs = dt;
    // A floating window that got its size from a docked group fits the dragged group.
    if let (None, DropTarget::Float { .. }) = (float, target)
        && let Some(f) = app.ui.dock.floats.last_mut()
    {
        f.size[0] = drag.size.x.clamp(*crate::dock_model::FLOAT_W.start(), *crate::dock_model::FLOAT_W.end());
    }
}

/// Tool Sliders dragged by its own window's strip, from outside the dock: the group strip or
/// group edge under `p` it would join, if any (its own window handles the canvas edges).
fn external_hit(ctx: &egui::Context, p: Pos2) -> Option<Hit> {
    let hit = resolve(&zones(ctx), p, DragKind::Panel, None)?;
    matches!(hit.target, DropTarget::Tabs { .. } | DropTarget::Stack { .. }).then_some(hit)
}

/// Is `p` over a group's tab strip (where a panel from outside the dock would join the group)?
pub fn over_strip(ctx: &egui::Context, p: Pos2) -> bool {
    external_hit(ctx, p).is_some_and(|h| matches!(h.target, DropTarget::Tabs { .. }))
}

/// Show where a panel dragged from outside the dock would land.
pub fn preview_external(ctx: &egui::Context, p: Pos2) -> bool {
    match external_hit(ctx, p) {
        Some(h) => {
            paint_hit(ctx, &h);
            true
        }
        None => false,
    }
}

/// Drop a panel dragged from outside the dock at `p`. Returns whether the dock took it.
pub fn drop_external(app: &mut PrismApp, ctx: &egui::Context, p: Pos2, panel: PanelId) -> bool {
    let Some(hit) = external_hit(ctx, p) else { return false };
    let pro = Tokens::get(ctx).pro;
    let mut dt = app.ui.dock_tabs;
    let ok = app.ui.dock.apply_drop(Payload::Panel(panel), hit.target, &mut dt, pro).is_ok();
    app.ui.dock_tabs = dt;
    ok
}
