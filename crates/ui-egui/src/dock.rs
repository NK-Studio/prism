//! Panel groups and the main dock column (#88), like Photoshop's dock columns.
//!
//! A group's height never follows its content: content taller than the group scrolls inside
//! it. The last expanded group (Layers by default) fills what the others leave. Drag the gap
//! between two groups to resize them, double-click a tab (or use the panel menu) to collapse a
//! group to its tab strip. Drag a tab to tear it off (a floating window, another group's tab
//! strip, above or below a group, or a new dock column at the left, right or bottom: see
//! `dock_drag`), or a tab strip to move the whole group (unless Window › Workspace › Lock
//! Workspace is on). Extra columns, floating windows and columns collapsed to icons are drawn
//! by `dock_float`; the model (tabs, groups, drops) is `dock_model`.
//!
//! The layout is [`DockLayout`] in `UiState::dock` (serialisable, drivable with `ui.set` and
//! `ui.dock`), saved with Window › Workspace › New Workspace…, reset by Reset Workspace, and
//! remembered across launches in the preferences (`panelLayout`) while Remember Workspace Changes
//! is on.

use std::collections::BTreeMap;

use egui::{Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PrismApp;
use crate::dock_drag::{self, Zone, ZoneKind};
use crate::dock_model::{Column, Float, Payload, PanelId, Slot};
use crate::state::{DockTabs, Panels};
use crate::theme::Tokens;
use crate::widgets;

/// A dock panel group (one tab strip). The six Photoshop Essentials groups keep their own
/// names (and `Panels` flags); groups made by tearing tabs off are `Custom(n)`. Serialised as a
/// string ("color", …, "g7"), so it can key JSON maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// Color | Swatches | Gradients | Patterns.
    Color,
    /// Properties | Adjustments.
    Properties,
    /// Character | Paragraph (not in the default workspace; Window › Character opens it, #150).
    Character,
    /// Navigator | Histogram | Info.
    Navigator,
    /// History | Actions | Layer Comps.
    History,
    /// Layers | Channels | Paths.
    Layers,
    /// A group made in the dock (see `dock_model`).
    Custom(u32),
}

impl Serialize for Group {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.key())
    }
}

impl<'de> Deserialize<'de> for Group {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Group::from_key(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown dock group `{s}`")))
    }
}

impl Group {
    /// Photoshop Essentials order, top to bottom.
    pub const ALL: [Group; 6] = [Group::Color, Group::Properties, Group::Character, Group::Navigator, Group::History, Group::Layers];

    pub fn key(self) -> String {
        match self {
            Group::Color => "color".into(),
            Group::Properties => "properties".into(),
            Group::Character => "character".into(),
            Group::Navigator => "navigator".into(),
            Group::History => "history".into(),
            Group::Layers => "layers".into(),
            Group::Custom(n) => format!("g{n}"),
        }
    }

    /// Height (points, tab strip included) a group gets until the user resizes it, when the
    /// column has room (see [`DockLayout::heights_for`]).
    pub fn default_height(self) -> f32 {
        match self {
            Group::Color => 190.0,
            Group::Properties => 250.0,
            Group::Character => 270.0,
            Group::Navigator => 210.0,
            Group::History => 200.0,
            Group::Layers => 320.0,
            Group::Custom(_) => 220.0,
        }
    }

    /// What a group left at its default height gives way down to so the filler (Layers) keeps
    /// [`Group::preferred_fill`] in a short column (#147). Content taller than this scrolls.
    pub fn compact_height(self) -> f32 {
        match self {
            Group::Color => 130.0,
            Group::Properties => 160.0,
            Group::Character => 160.0,
            Group::Navigator => 140.0,
            Group::History => 130.0,
            Group::Layers => 200.0,
            Group::Custom(_) => 120.0,
        }
    }

    /// The height the filling group asks for before default-sized groups above it get their
    /// full defaults: Layers wants room for about ten rows at 900 pt (#147).
    pub fn preferred_fill(self) -> f32 {
        match self {
            Group::Layers => 500.0,
            g => g.min_height(),
        }
    }

    /// The smallest height an expanded group can be dragged or squeezed to.
    pub fn min_height(self) -> f32 {
        match self {
            Group::Layers => 140.0,
            _ => 80.0,
        }
    }

    /// The tabs a group has by default, in order (a custom group has none).
    pub fn default_panels(self, pro: bool) -> Vec<PanelId> {
        use PanelId as P;
        match self {
            // Photoshop Essentials: Color | Swatches | Gradients | Patterns.
            Group::Color if pro => vec![P::Color, P::Swatches, P::Gradients, P::Patterns],
            Group::Color => vec![P::Swatches, P::Color, P::Gradients, P::Patterns],
            Group::Properties => vec![P::Properties, P::Adjustments],
            Group::Character => vec![P::Character, P::Paragraph],
            Group::Navigator => vec![P::Navigator, P::Histogram, P::Info],
            Group::History => vec![P::History, P::Actions, P::LayerComps],
            Group::Layers => vec![P::Layers, P::Channels, P::Paths],
            Group::Custom(_) => Vec::new(),
        }
    }

    /// The default tab labels (English).
    pub fn tabs(self, pro: bool) -> Vec<&'static str> {
        self.default_panels(pro).into_iter().map(PanelId::label).collect()
    }

    /// The `DockTabs` slot of an Essentials group: an index into its default tabs.
    pub fn tab_index(self, tabs: &DockTabs) -> Option<usize> {
        Some(match self {
            Group::Color => tabs.color,
            Group::Properties => tabs.properties,
            Group::Character => tabs.character,
            Group::Navigator => tabs.navigator,
            Group::History => tabs.history,
            Group::Layers => tabs.layers,
            Group::Custom(_) => return None,
        })
    }

    pub fn tab_index_mut(self, tabs: &mut DockTabs) -> Option<&mut usize> {
        Some(match self {
            Group::Color => &mut tabs.color,
            Group::Properties => &mut tabs.properties,
            Group::Character => &mut tabs.character,
            Group::Navigator => &mut tabs.navigator,
            Group::History => &mut tabs.history,
            Group::Layers => &mut tabs.layers,
            Group::Custom(_) => return None,
        })
    }

    /// The group for a `panels` / `dockTabs` key ("color", "properties", …) or a custom group
    /// key ("g7").
    pub fn from_key(key: &str) -> Option<Group> {
        if let Some(n) = key.strip_prefix('g').and_then(|n| n.parse::<u32>().ok()) {
            return Some(Group::Custom(n));
        }
        Group::ALL.into_iter().find(|g| g.key() == key)
    }

    pub fn shown(self, panels: &Panels) -> bool {
        match self {
            Group::Color => panels.color,
            Group::Properties => panels.properties,
            Group::Character => panels.character,
            Group::Navigator => panels.navigator,
            Group::History => panels.history,
            Group::Layers => panels.layers,
            Group::Custom(_) => true,
        }
    }

    /// The `Panels` flag of an Essentials group. A custom group's visibility is
    /// `DockLayout::hidden` (see `dock_model::set_group_visible`); for those this returns a
    /// scratch flag nothing reads.
    pub(crate) fn shown_mut(self, panels: &mut Panels) -> &mut bool {
        match self {
            Group::Color => &mut panels.color,
            Group::Properties => &mut panels.properties,
            Group::Character => &mut panels.character,
            Group::Navigator => &mut panels.navigator,
            Group::History => &mut panels.history,
            Group::Layers => &mut panels.layers,
            Group::Custom(_) => &mut panels.toolbar_double,
        }
    }
}

/// Where the panel groups are: the main column's order, heights and collapsed groups, plus
/// each group's tabs, extra dock columns and floating windows (see `dock_model`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DockLayout {
    /// Top-to-bottom order of the main column; an Essentials group missing here (say one added
    /// after the layout was saved) goes just below the nearest group that precedes it by default
    /// (or last), unless it sits in an extra column or a floating window.
    pub order: Vec<Group>,
    /// Heights the user dragged groups to (points, tab strip included). Unset = default.
    pub heights: BTreeMap<Group, f32>,
    /// Groups collapsed to their tab strip.
    pub collapsed: Vec<Group>,
    /// Tabs of groups whose tabs changed (dragged, merged, closed) and of custom groups. An
    /// Essentials group missing here has its default tabs.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tabs: BTreeMap<Group, Vec<PanelId>>,
    /// The selected tab of a custom group, or a tab moved into an Essentials group (whose own
    /// tabs are selected through `DockTabs`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub active: BTreeMap<Group, PanelId>,
    /// Extra dock columns (left, right, bottom), outermost first per side.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<Column>,
    /// Floating panel windows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub floats: Vec<Float>,
    /// Custom groups closed with × (Essentials groups use their `Panels` flag).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<Group>,
    /// The main dock column is collapsed to icons («).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub main_iconic: bool,
    /// Next id for a custom group, column or floating window.
    #[serde(skip_serializing_if = "is_zero")]
    pub next_id: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Gap between groups; it is also the splitter's grab area.
pub const GAP: f32 = 6.0;
/// Upper bound on a stored height (guards against absurd values from `ui.set`).
const MAX_HEIGHT: f32 = 4000.0;
/// Height of the bar at the top of a dock column that holds its « (collapse to icons) button.
pub const COLUMN_HEADER: f32 = 12.0;

impl DockLayout {
    /// The main column's groups once each, in display order.
    pub fn order(&self) -> Vec<Group> {
        let mut out: Vec<Group> = Vec::with_capacity(Group::ALL.len());
        for g in &self.order {
            let known = !matches!(g, Group::Custom(_)) || self.tabs.contains_key(g);
            if known && !out.contains(g) && !self.placed_elsewhere(*g) {
                out.push(*g);
            }
        }
        // Missing groups slot in below their default predecessor, so a group new to an old
        // saved layout (Character) never lands below Layers and takes over as the filler.
        for (i, g) in Group::ALL.iter().enumerate() {
            if out.contains(g) || self.placed_elsewhere(*g) {
                continue;
            }
            let prev = Group::ALL.iter().take(i).rev().find_map(|p| out.iter().position(|x| x == p));
            match prev {
                Some(at) => out.insert(at + 1, *g),
                None => out.push(*g),
            }
        }
        out
    }

    pub fn is_collapsed(&self, g: Group) -> bool {
        self.collapsed.contains(&g)
    }

    pub fn set_collapsed(&mut self, g: Group, on: bool) {
        self.collapsed.retain(|c| *c != g);
        if on {
            self.collapsed.push(g);
        }
    }

    /// The stored (or default) height, sanitised.
    pub fn height(&self, g: Group) -> f32 {
        match self.heights.get(&g) {
            Some(h) if h.is_finite() => h.clamp(g.min_height(), MAX_HEIGHT),
            _ => g.default_height(),
        }
    }

    /// Move `g` within its column or window so it is drawn just before `before` (or last when
    /// `None`). A `before` in another container leaves `g` where it is.
    pub fn move_group(&mut self, g: Group, before: Option<Group>) {
        if before == Some(g) {
            return;
        }
        let Some((slot, _)) = self.slot_of(g) else { return };
        if before.is_some_and(|b| self.slot_of(b).map(|(s, _)| s) != Some(slot)) {
            return;
        }
        let mut list = self.groups_in(slot);
        list.retain(|x| *x != g);
        let at = before.and_then(|b| list.iter().position(|x| *x == b)).unwrap_or(list.len());
        list.insert(at, g);
        match slot {
            Slot::Main => self.order = list,
            Slot::Column(i) => {
                if let Some(c) = self.columns.get_mut(i) {
                    c.groups = list;
                }
            }
            Slot::Float(i) => {
                if let Some(f) = self.floats.get_mut(i) {
                    f.groups = list;
                }
            }
        }
    }

    /// Lay out the `shown` groups (in display order) in a column `avail` points tall with
    /// `strip`-high tab strips. Returns each group's height. The last expanded group fills the
    /// rest. Groups the user never resized give way first, down to their compact heights, so
    /// the filler gets its preferred height (Layers: ~10 rows, #147); when the column is still
    /// too short every group gives way down to its minimum height.
    pub fn heights_for(&self, shown: &[Group], avail: f32, strip: f32) -> Vec<(Group, f32)> {
        let avail = if avail.is_finite() { avail.max(0.0) } else { 0.0 };
        let filler = shown.iter().rposition(|g| !self.is_collapsed(*g));
        let mut hs: Vec<f32> = shown
            .iter()
            .enumerate()
            .map(|(i, g)| {
                if self.is_collapsed(*g) {
                    strip
                } else if Some(i) == filler {
                    0.0
                } else {
                    self.height(*g)
                }
            })
            .collect();
        if let Some(f) = filler {
            let gaps = GAP * shown.len().saturating_sub(1) as f32;
            let min_fill = shown.get(f).map_or(0.0, |g| g.min_height());
            let pref_fill = shown.get(f).map_or(0.0, |g| g.preferred_fill());
            let used: f32 = hs.iter().sum::<f32>() + gaps;
            let mut deficit = (used + pref_fill - avail).max(0.0);
            for i in (0..f).rev() {
                if deficit <= 0.0 {
                    break;
                }
                let Some(g) = shown.get(i) else { continue };
                if self.is_collapsed(*g) || self.heights.contains_key(g) {
                    continue;
                }
                if let Some(h) = hs.get_mut(i) {
                    let give = (*h - g.compact_height()).max(0.0).min(deficit);
                    *h -= give;
                    deficit -= give;
                }
            }
            let used: f32 = hs.iter().sum::<f32>() + gaps;
            let mut deficit = (used + min_fill - avail).max(0.0);
            // Squeeze the expanded groups nearest the filler first.
            for i in (0..f).rev() {
                if deficit <= 0.0 {
                    break;
                }
                let Some(g) = shown.get(i) else { continue };
                if self.is_collapsed(*g) {
                    continue;
                }
                if let Some(h) = hs.get_mut(i) {
                    let give = (*h - g.min_height()).max(0.0).min(deficit);
                    *h -= give;
                    deficit -= give;
                }
            }
            let rest = avail - hs.iter().sum::<f32>() - gaps;
            if let Some(h) = hs.get_mut(f) {
                *h = rest.max(min_fill);
            }
        }
        shown.iter().copied().zip(hs).collect()
    }
}

/// Show `g` and expand it (Window › <panel>, the icon rail): a panel asked for is always
/// brought back, whatever state it was left in (#129).
pub fn reveal(app: &mut PrismApp, g: Group) {
    crate::dock_model::set_group_visible(&mut app.ui, g, true);
    app.ui.dock.set_collapsed(g, false);
    // The group's own selected tab (`DockTabs`) wins over a tab moved in from elsewhere.
    app.ui.dock.active.remove(&g);
}

/// Icon rail click: a hidden group is shown, a collapsed one expanded and an expanded one
/// collapsed to its tab strip. A docked group is never hidden from the rail (it used to
/// toggle visibility, so one stray click made a panel vanish: #129); `docked` is false for
/// Studio's floating Properties card, which the rail shows and hides.
pub fn rail_click(app: &mut PrismApp, g: Group, docked: bool) {
    if !g.shown(&app.ui.panels) {
        reveal(app, g);
    } else if !docked {
        *g.shown_mut(&mut app.ui.panels) = false;
    } else {
        let collapse = !app.ui.dock.is_collapsed(g);
        app.ui.dock.set_collapsed(g, collapse);
    }
}

/// Per-group interactions collected while drawing, applied afterwards.
enum Action {
    ToggleCollapse(Group),
    Close(Group),
    CloseTab(Group, PanelId),
    Move(Group, Option<Group>),
    /// Float the group in a window of its own at this corner.
    Float(Group, egui::Pos2),
}

/// Rects of the groups drawn last frame (screen points), for tests and automation.
pub fn last_rects(ctx: &egui::Context) -> Vec<(Group, Rect)> {
    dock_drag::frame_items(ctx, rects_id())
}

fn rects_id() -> egui::Id {
    egui::Id::new("dock-group-rects")
}

/// A group's tab strip as drawn last frame (screen points), for tests and automation.
#[derive(Clone, Debug, PartialEq)]
pub struct StripRects {
    pub group: Group,
    /// `(tab index, rect)` of the tabs on the strip (the others are in the chevron menu).
    pub tabs: Vec<(usize, Rect)>,
    /// The panel menu button.
    pub menu: Rect,
    /// The » overflow button, when some tabs didn't fit.
    pub chevron: Option<Rect>,
}

/// The tab strips drawn last frame.
pub fn last_strips(ctx: &egui::Context) -> Vec<StripRects> {
    dock_drag::frame_items(ctx, strips_id())
}

fn strips_id() -> egui::Id {
    egui::Id::new("dock-strip-rects")
}

/// How a stack of groups is laid out.
#[derive(Clone, Copy, Debug, Default)]
pub struct StackOpts {
    /// Side by side (a bottom dock row) instead of top to bottom.
    pub horizontal: bool,
    /// The floating window holding the stack, if any.
    pub float: Option<u32>,
}

/// One group's tab content.
pub type Body<'a> = dyn FnMut(&mut PrismApp, &mut egui::Ui, PanelId) + 'a;

/// Draw the `shown` groups of the main column (any order; the layout decides) filling `ui`.
/// `body` draws one tab's content.
pub fn show(app: &mut PrismApp, ui: &mut egui::Ui, shown: &[Group], mut body: impl FnMut(&mut PrismApp, &mut egui::Ui, PanelId)) {
    let order: Vec<Group> = app.ui.dock.order().into_iter().filter(|g| shown.contains(g)).collect();
    let area = ui.available_rect_before_wrap();
    show_stack(app, ui, area, &order, StackOpts::default(), &mut body);
    ui.advance_cursor_after_rect(area);
}

/// Draw `groups` (in order) in `area`, top to bottom (or side by side).
pub fn show_stack(app: &mut PrismApp, ui: &mut egui::Ui, area: Rect, groups: &[Group], opts: StackOpts, body: &mut Body<'_>) {
    let t = Tokens::get(ui.ctx());
    let strip = if t.pro { 28.0 } else { 40.0 };
    let pro = t.pro;
    let locked = app.session.prefs().workspace_locked;
    let rects: Vec<(Group, Rect)> = if opts.horizontal {
        let n = groups.len().max(1) as f32;
        let w = ((area.width() - GAP * (n - 1.0)) / n).max(0.0);
        groups.iter().enumerate().map(|(i, g)| (*g, Rect::from_min_size(pos2(area.left() + i as f32 * (w + GAP), area.top()), vec2(w, area.height())))).collect()
    } else {
        rects_after_layout(&app.ui.dock.heights_for(groups, area.height(), strip), area)
    };
    let heights: Vec<(Group, f32)> = rects.iter().map(|(g, r)| (*g, r.height())).collect();
    let mut actions: Vec<Action> = Vec::new();
    for (i, (g, rect)) in rects.iter().copied().enumerate() {
        let collapsed = app.ui.dock.is_collapsed(g);
        let key = g.key();
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("dock-group", key.as_str())).max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing.y = if t.pro { 0.0 } else { 6.0 };
        let tabs = app.ui.dock.tabs_of(g, pro);
        let labels: Vec<&str> = tabs.iter().map(|p| p.label()).collect();
        let active = app.ui.dock.active_panel(g, &tabs, &app.ui.dock_tabs, pro);
        let before = active.and_then(|a| tabs.iter().position(|p| *p == a)).unwrap_or(0);
        let before_tabs = app.ui.dock_tabs;
        let mut sel = before;
        let resp = widgets::card_ex(&mut child, &key, &labels, &mut sel, collapsed, |ui, tab| {
            let Some(panel) = tabs.get(tab).copied() else { return };
            let inner = ui.available_height().max(0.0);
            if panel.fills() {
                ui.set_min_height(inner);
                body(app, ui, panel);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt(("dock-scroll", key.as_str(), panel.key()))
                    .max_height(inner)
                    .auto_shrink([false, false])
                    .show(ui, |ui| body(app, ui, panel));
            }
        });
        dock_drag::frame_push(ui.ctx(), rects_id(), (g, rect));
        dock_drag::frame_push(ui.ctx(), strips_id(), StripRects { group: g, tabs: resp.tabs.clone(), menu: resp.menu.rect, chevron: resp.chevron });
        // Drop zones: the group (above or below it) and its tab strip (into the group).
        dock_drag::add_zone(ui.ctx(), Zone { rect, kind: ZoneKind::Body { group: g, horizontal: opts.horizontal }, float: opts.float });
        dock_drag::add_zone(ui.ctx(), Zone { rect: resp.strip.rect, kind: ZoneKind::Strip { group: g, tabs: resp.tabs.clone() }, float: opts.float });
        // A tab click selects it; the body may also switch tabs itself (Adjustments jumps back
        // to Properties through `DockTabs`).
        if sel != before
            && let Some(p) = tabs.get(sel).copied()
        {
            let mut dt = app.ui.dock_tabs;
            app.ui.dock.set_active(g, p, &mut dt, pro);
            app.ui.dock_tabs = dt;
        } else if app.ui.dock_tabs != before_tabs {
            app.ui.dock.active.remove(&g);
        }
        if resp.strip.double_clicked() || resp.tab_double_clicked {
            actions.push(Action::ToggleCollapse(g));
        }
        if !locked && resp.strip.drag_started_by(egui::PointerButton::Primary) {
            begin_drag(app, ui, &resp, g, &tabs, rect, groups.len(), opts);
        }
        let shown_tab = tabs.get(sel).copied();
        egui::Popup::menu(&resp.menu).show(|ui| {
            ui.set_min_width(170.0);
            if shown_tab == Some(PanelId::Layers) {
                crate::layer_row_ui::panel_menu(app, ui);
                ui.separator();
            }
            if shown_tab == Some(PanelId::Swatches) {
                crate::swatches_ui::panel_menu(app, ui);
                ui.separator();
            }
            if shown_tab == Some(PanelId::Color) {
                crate::panels::color_panel_ui::panel_menu(app, ui);
                ui.separator();
            }
            if ui.button(if collapsed { tl!("Expand Panel Group") } else { tl!("Collapse Panel Group") }).clicked() {
                actions.push(Action::ToggleCollapse(g));
                ui.close();
            }
            let pos = groups.iter().position(|x| *x == g).unwrap_or(0);
            let (up, down) = if opts.horizontal { (tl!("Move Group Left"), tl!("Move Group Right")) } else { (tl!("Move Group Up"), tl!("Move Group Down")) };
            if ui.add_enabled(!locked && pos > 0, egui::Button::new(up)).clicked() {
                actions.push(Action::Move(g, groups.get(pos.saturating_sub(1)).copied()));
                ui.close();
            }
            if ui.add_enabled(!locked && pos + 1 < groups.len(), egui::Button::new(down)).clicked() {
                actions.push(Action::Move(g, groups.get(pos + 2).copied()));
                ui.close();
            }
            let floating = opts.float.is_some() && groups.len() == 1;
            if ui.add_enabled(!locked && !floating, egui::Button::new(tl!("Float Panel Group"))).clicked() {
                actions.push(Action::Float(g, rect.min - vec2(24.0, 0.0)));
                ui.close();
            }
            ui.separator();
            if let Some(p) = shown_tab
                && ui.button(tl!("Close")).clicked()
            {
                actions.push(Action::CloseTab(g, p));
                ui.close();
            }
            if ui.button(tl!("Close Tab Group")).clicked() {
                actions.push(Action::Close(g));
                ui.close();
            }
        });
        // Splitter in the gap below this group: resizes it against the next expanded group.
        if !opts.horizontal && i + 1 < rects.len() && !collapsed && rects.iter().skip(i + 1).any(|(n, _)| !app.ui.dock.is_collapsed(*n)) {
            let gap = Rect::from_min_size(pos2(rect.left(), rect.bottom()), vec2(rect.width(), GAP)).expand2(vec2(0.0, 2.0));
            let sresp = ui.interact(gap, ui.id().with(("dock-splitter", key.as_str())), Sense::drag());
            if sresp.hovered() || sresp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                ui.painter().line_segment([gap.left_center(), gap.right_center()], Stroke::new(2.0, t.accent.gamma_multiply(0.7)));
            }
            if sresp.dragged() {
                resize(&mut app.ui.dock, &heights, i, sresp.drag_delta().y);
            }
        }
    }
    for a in actions {
        match a {
            Action::ToggleCollapse(g) => {
                let on = !app.ui.dock.is_collapsed(g);
                app.ui.dock.set_collapsed(g, on);
            }
            Action::Close(g) => crate::dock_model::set_group_visible(&mut app.ui, g, false),
            Action::CloseTab(g, p) => crate::dock_model::close_panel(&mut app.ui, g, p, pro),
            Action::Move(g, before) => app.ui.dock.move_group(g, before),
            Action::Float(g, at) => {
                let mut dt = app.ui.dock_tabs;
                let target = crate::dock_model::DropTarget::Float { pos: [at.x - 280.0, at.y] };
                if let Err(e) = app.ui.dock.apply_drop(Payload::Group(g), target, &mut dt, pro) {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
                app.ui.dock_tabs = dt;
            }
        }
    }
}

/// A drag that started on a group's tab strip: a tab tears off, the strip's background moves
/// the group (or, for the only group of a floating window, the window).
#[allow(clippy::too_many_arguments)]
fn begin_drag(app: &mut PrismApp, ui: &egui::Ui, resp: &widgets::CardResponse, g: Group, tabs: &[PanelId], rect: Rect, siblings: usize, opts: StackOpts) {
    let Some(origin) = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.strip.interact_pointer_pos()) else { return };
    let on_tab = resp.tabs.iter().find(|(_, r)| r.contains(origin)).and_then(|(i, _)| tabs.get(*i).copied());
    let payload = match (on_tab, opts.float) {
        (Some(panel), _) => Payload::Tab { group: g, panel },
        (None, Some(id)) if siblings == 1 => Payload::Float(id),
        (None, _) => Payload::Group(g),
    };
    let label = match on_tab {
        Some(p) => tl!(p.label()).to_string(),
        None => tabs.iter().map(|p| tl!(p.label())).collect::<Vec<_>>().join(" | "),
    };
    dock_drag::begin(app, ui.ctx(), payload, origin - rect.min, label, rect.size());
}

fn rects_after_layout(heights: &[(Group, f32)], area: Rect) -> Vec<(Group, Rect)> {
    let mut y = area.top();
    heights
        .iter()
        .map(|(g, h)| {
            let r = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), h.max(0.0)));
            y = r.bottom() + GAP;
            (*g, r)
        })
        .collect()
}

/// Splitter `i` (below group `i`) dragged by `dy`: group `i` grows or shrinks against the next
/// expanded group (or the filler, which absorbs the difference).
fn resize(layout: &mut DockLayout, heights: &[(Group, f32)], i: usize, dy: f32) {
    if !dy.is_finite() || dy == 0.0 {
        return;
    }
    let Some(&(g, h)) = heights.get(i) else { return };
    let filler = heights.iter().rposition(|(g, _)| !layout.is_collapsed(*g));
    let Some(j) = heights.iter().enumerate().skip(i + 1).find(|(_, (n, _))| !layout.is_collapsed(*n)).map(|(j, _)| j) else { return };
    let Some(&(n, nh)) = heights.get(j) else { return };
    // The first drag pins the other groups at the heights they show, so groups still at their
    // defaults (which give way to the filler) don't shift while this one is resized.
    for (k, (o, oh)) in heights.iter().enumerate() {
        if Some(k) != filler && !layout.is_collapsed(*o) {
            layout.heights.entry(*o).or_insert(*oh);
        }
    }
    let new_h = (h + dy).clamp(g.min_height(), (h + nh - n.min_height()).max(g.min_height()));
    layout.heights.insert(g, new_h);
    if Some(j) != filler {
        layout.heights.insert(n, (nh - (new_h - h)).max(n.min_height()));
    }
}

/// What `prefs.panelLayout` holds: the live layout and open panels.
fn snapshot(app: &PrismApp) -> Value {
    json!({"workspace": app.ui.workspace, "panels": app.ui.panels, "dockTabs": app.ui.dock_tabs, "dock": app.ui.dock, "colorPanel": app.ui.color_panel, "toolSliders": app.ui.tool_sliders, "tools": app.ui.tools})
}

/// Remember the layout in the preferences once the user lets go of the mouse (Workspace ›
/// Remember Workspace Changes). Cheap: a small JSON compare per frame.
pub fn persist(app: &mut PrismApp, ctx: &egui::Context) {
    if !app.session.prefs().workspace.remember_workspace_changes || ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let now = snapshot(app);
    if app.session.prefs().panel_layout != now {
        app.session.prefs.edit(|p| p.panel_layout = now);
    }
}

/// Restore the remembered layout at launch. Unreadable parts keep their defaults.
pub fn restore(app: &mut PrismApp) {
    if !app.session.prefs().workspace.remember_workspace_changes {
        return;
    }
    let saved = app.session.prefs().panel_layout.clone();
    apply(app, &saved);
    if let Some(ws) = saved.get("workspace").and_then(Value::as_str) {
        app.ui.workspace = ws.to_string();
    }
}

/// Apply the `panels`, `dockTabs`, `dock`, `colorPanel`, `toolSliders` and `tools` parts of a saved layout (a
/// workspace or `panelLayout`). Missing or invalid parts are left alone.
pub fn apply(app: &mut PrismApp, v: &Value) {
    if let Some(s) = v.get("toolSliders").and_then(|s| serde_json::from_value(s.clone()).ok()) {
        app.ui.tool_sliders = s;
    }
    if let Some(s) = v.get("tools").and_then(|s| serde_json::from_value(s.clone()).ok()) {
        app.ui.tools = s;
    }
    if let Some(c) = v.get("colorPanel").and_then(|c| serde_json::from_value(c.clone()).ok()) {
        app.ui.color_panel = c;
    }
    if let Some(p) = v.get("panels").and_then(|p| serde_json::from_value(p.clone()).ok()) {
        app.ui.panels = p;
    }
    if let Some(t) = v.get("dockTabs").and_then(|t| serde_json::from_value(t.clone()).ok()) {
        app.ui.dock_tabs = t;
    }
    if let Some(d) = v.get("dock").and_then(|d| serde_json::from_value(d.clone()).ok()) {
        app.ui.dock = d;
    }
}

#[cfg(test)]
#[path = "dock_tests.rs"]
mod tests;
